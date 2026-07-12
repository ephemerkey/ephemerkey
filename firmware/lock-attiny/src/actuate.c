/*
 * ephemerkey lock board — non-blocking actuator STEP-SEQUENCE engine (ATtiny1616)
 * See actuate.h.
 *
 * A cycle runs the configured UNLOCK or LOCK sequence: an ordered list of steps,
 * each firing any combination of {servo1, servo2, solenoid} together. Each step
 * runs through a small sub-state-machine (ramp -> strike -> run -> drain), all
 * advanced by the TCB0 ms tick so I2C never blocks. A step may end early when its
 * configured hall sensor reaches the wanted state; a new lock/unlock aborts.
 *
 * Per-step rail selection (the boost makes VSOL, which also feeds the servo):
 *   solenoid + servo  -> 6 V,  solenoid FULL DC (no PWM), servo powered (combined)
 *   solenoid only     -> 12 V, solenoid strike + economizer PWM hold, servo locked out
 *   servo + boost flag -> 6 V, servo powered
 *   servo only        -> Vbat, servo powered
 */
#include <avr/io.h>
#include <avr/interrupt.h>
#include "actuate.h"
#include "lock_config.h"
#include "lock_twi.h"          /* twi_status + ST_* */
#include "config.h"
#include "servo.h"
#include "power.h"
#include "solenoid.h"
#include "hall.h"

/* --- millisecond tick (TCB0), enabled only during a cycle --- */
static volatile uint16_t g_ms;

ISR(TCB0_INT_vect)
{
    TCB0.INTFLAGS = TCB_CAPT_bm;
    g_ms++;
}

static void tick_start(void)
{
    g_ms = 0;
    TCB0.CNT = 0;
    TCB0.CCMP = (uint16_t)(F_CPU / 1000UL) - 1u;   /* 1 ms */
    TCB0.INTFLAGS = TCB_CAPT_bm;
    TCB0.INTCTRL = TCB_CAPT_bm;
    TCB0.CTRLB = TCB_CNTMODE_INT_gc;
    TCB0.CTRLA = TCB_CLKSEL_CLKDIV1_gc | TCB_ENABLE_bm;
}
static void tick_stop(void)
{
    TCB0.CTRLA = 0;
    TCB0.INTCTRL = 0;
}
static uint16_t ms_now(void)
{
    uint8_t s = SREG; cli();
    uint16_t m = g_ms;
    SREG = s;
    return m;
}

/* --- rail modes --- */
enum { RAIL_VBAT, RAIL_6V, RAIL_12V };

/* --- per-step sub-phases --- */
enum { SB_RAMP, SB_STRIKE, SB_RUN, SB_DRAIN };

#define EOFF_CONFIRM_MS  500u    /* sensor must hold the wanted state this long */

static uint8_t         s_active;       /* 0 = idle */
static const step_t   *s_seq;          /* seq_unlock or seq_lock */
static uint8_t         s_idx;          /* current step index */
static uint8_t         s_sub;          /* SB_* within the current step */
static uint16_t        s_end;          /* ms deadline for the current sub-phase */
static uint8_t         s_rail;         /* RAIL_* for the current step */
static uint8_t         s_sol_pwm;      /* 1 = economizer PWM hold, 0 = full DC */
static uint8_t         s_eoff_pending; /* early-off confirm in progress */
static uint16_t        s_eoff_at;      /* ms when the early-off condition was first met */

static void status_set(uint8_t bit, uint8_t on)
{
    if (on) twi_status |= bit; else twi_status &= (uint8_t)~bit;
}

/* Blunt all-off — safe from any state; used for abort and finish. */
static void all_off(void)
{
    servo_power(false);
    servo_pwm_stop();
    sol_hold_stop();
    sol_off();
    boost_disable();
    status_set(ST_RAIL_12V, 0);
}

void actuate_init(void) { s_active = 0; }
uint8_t actuate_busy(void) { return s_active; }

static void finish(void)
{
    all_off();
    hall_power(0);                /* drop hall power; idle reads re-pulse it */
    status_set(ST_BUSY, 0);
    s_active = 0;
    tick_stop();
}

/* Begin step `idx`: pick the rail, program the servos, and start the ramp/run. */
static void step_enter(uint8_t idx)
{
    const config_t *c = config_get();
    const step_t *st = &s_seq[idx];
    uint8_t has_sv  = st->act & (STEP_SERVO1 | STEP_SERVO2);
    uint8_t has_sol = st->act & STEP_SOLENOID;
    uint16_t t = ms_now();

    s_idx = idx;
    s_eoff_pending = 0;

    /* rail + solenoid drive mode for this step */
    if (has_sol && has_sv)      { s_rail = RAIL_6V;  s_sol_pwm = 0; }  /* combined */
    else if (has_sol)           { s_rail = RAIL_12V; s_sol_pwm = 1; }  /* economizer */
    else if (has_sv && (c->flags & CFG_SERVO_BOOST)) { s_rail = RAIL_6V; s_sol_pwm = 0; }
    else                        { s_rail = RAIL_VBAT; s_sol_pwm = 0; }

    if (has_sv) {                          /* commanded pulse(s) on before power */
        if (st->act & STEP_SERVO1) servo1_set_us(cfg_pos_to_us(st->s1_pos));
        if (st->act & STEP_SERVO2) servo2_set_us(cfg_pos_to_us(st->s2_pos));
        servo_pwm_start();
    }

    if (s_rail == RAIL_12V) {
        boost_12v_enable();                /* VSEL high: servo interlocked out */
        status_set(ST_RAIL_12V, 1);
        s_sub = SB_RAMP;
        s_end = t + SOL_BOOST_RAMP_MS;
    } else if (s_rail == RAIL_6V) {
        boost_servo_enable();              /* VSEL low -> 6 V, interlock clear */
        status_set(ST_RAIL_12V, 0);
        if (has_sv) servo_power(true);     /* soft-start: ride the rail up */
        s_sub = SB_RAMP;
        s_end = t + SOL_BOOST_RAMP_MS;
    } else {                               /* RAIL_VBAT (servo only) */
        boost_disable();
        status_set(ST_RAIL_12V, 0);
        if (has_sv) servo_power(true);
        s_sub = SB_RUN;                    /* no ramp needed */
        s_end = t + (uint16_t)st->dur_ds * 100u;
    }
}

/* Advance to the next step, or finish if the sequence is done. */
static void step_advance(void)
{
    uint8_t n = s_idx + 1u;
    if (n >= SEQ_STEPS || s_seq[n].act == 0) { finish(); return; }
    step_enter(n);
}

void actuate_begin(uint8_t unlock)
{
    const config_t *c = config_get();

    all_off();                    /* abort any in-progress cycle */
    tick_start();
    status_set(ST_BUSY, 1);
    hall_power(1);                /* keep the hall sensors powered for the cycle */
    s_seq = unlock ? c->seq_unlock : c->seq_lock;
    s_active = 1;

    if (s_seq[0].act == 0) { finish(); return; }   /* empty sequence -> nothing */
    step_enter(0);
}

/* End the current RUN sub-phase: release this step's actuators, then drain (if a
 * boosted rail was up) or advance straight to the next step. */
static void end_run(uint16_t t, uint8_t has_sv, uint8_t has_sol)
{
    if (has_sv) { servo_power(false); servo_pwm_stop(); }
    if (has_sol && s_sol_pwm) sol_hold_stop();   /* -> PA5 DC high; coil drains VSOL */
    /* full-DC solenoid (combined) stays energized through the drain, then sol_off */

    if (s_rail == RAIL_VBAT) {
        step_advance();
    } else {
        boost_disable();
        status_set(ST_RAIL_12V, 0);
        s_sub = SB_DRAIN;
        s_end = t + SOL_DRAIN_MS;
    }
}

void actuate_tick(void)
{
    if (!s_active) return;
    const config_t *c = config_get();
    const step_t *st = &s_seq[s_idx];
    uint8_t has_sv  = st->act & (STEP_SERVO1 | STEP_SERVO2);
    uint8_t has_sol = st->act & STEP_SOLENOID;
    uint16_t dur = (uint16_t)st->dur_ds * 100u;

    /* Per-step early-off: during the RUN window, if the chosen logical sensor
     * reaches the wanted state for EOFF_CONFIRM_MS, end the step early. */
    if (s_sub == SB_RUN) {
        uint8_t sel = st->eoff & EOFF_SENSOR_MASK;
        if (sel != EOFF_NONE) {
            uint8_t src = (sel == EOFF_DOOR) ? cfg_door_src() : cfg_bolt_src();
            uint8_t present = hall_src(src, hall_sample()) ? 1 : 0;
            uint8_t want = (st->eoff & EOFF_EDGE_ABSENT) ? !present : present;
            if (want) {
                if (!s_eoff_pending) { s_eoff_pending = 1; s_eoff_at = ms_now(); }
                else if ((uint16_t)(ms_now() - s_eoff_at) >= EOFF_CONFIRM_MS)
                    s_end = ms_now();          /* force the RUN window to end now */
            } else {
                s_eoff_pending = 0;            /* condition cleared -> reset confirm */
            }
        }
    }

    if (ms_now() < s_end) return;             /* sub-phase not elapsed yet */
    uint16_t t = ms_now();

    switch (s_sub) {
    case SB_RAMP:                             /* rail reached target */
        if (has_sol) {
            sol_on();                         /* full DC (strike for both modes) */
            if (s_sol_pwm) { s_sub = SB_STRIKE; s_end = t + cfg_strike_ms(); }
            else           { s_sub = SB_RUN;    s_end = t + dur; }  /* 6 V combined */
        } else {                              /* boosted servo, already powered */
            s_sub = SB_RUN;
            s_end = t + dur;
        }
        break;
    case SB_STRIKE:                           /* 12 V economizer: strike done */
        if (dur > 0) {
            sol_hold_start(c->hold_duty);
            s_sub = SB_RUN;
            s_end = t + dur;
        } else {                              /* no hold -> drain */
            boost_disable();
            status_set(ST_RAIL_12V, 0);
            s_sub = SB_DRAIN;
            s_end = t + SOL_DRAIN_MS;
        }
        break;
    case SB_RUN:
        end_run(t, has_sv, has_sol);
        break;
    case SB_DRAIN:                            /* VSOL bled -> release, next step */
        if (has_sol) sol_off();
        step_advance();
        break;
    }
}
