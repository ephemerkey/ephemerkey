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
enum { SB_PREDRAIN, SB_RAMP, SB_STRIKE, SB_RUN, SB_DRAIN };

/* Early-off deglitching — two layers, because actuation disturbs the sensors
 * (bus/rail transients per testharness/README; on the bench the hall reads a
 * sustained false-absent whenever actuators are loaded):
 *   1. ARM: the step must first see the sensor in the OPPOSITE state for
 *      EOFF_ARM_N consecutive samples (e.g. for eoff=door- the door must read
 *      PRESENT for 50 ms). A sensor that is wrong-from-the-start, broken, or
 *      disconnected never arms — the step just runs its full time.
 *   2. FIRE: after arming, an INTEGRATING counter sampled at a fixed cadence:
 *      a wanted sample adds 1, an opposite sample subtracts EOFF_MISS_PENALTY
 *      (floor 0) — a few transients only slow it down, but sustained opposite
 *      readings drain it. Fires at EOFF_CONFIRM_N (~500 ms clean dwell).
 * Both fail toward "keep driving". */
#define EOFF_SAMPLE_MS      10u  /* sampling cadence */
#define EOFF_ARM_N           5u  /* consecutive opposite-state samples to arm */
#define EOFF_CONFIRM_N      50u  /* integrator level that fires (~500 ms) */
#define EOFF_MISS_PENALTY    5u  /* integrator cost of one opposite sample */
/* Passive VSOL bleed after an abort (coil off, only the FB divider loads the
 * rail — slow). TODO: measure the actual decay from 12 V and trim. */
#define PREDRAIN_MS     2000u

static uint8_t         s_active;       /* 0 = idle */
static const step_t   *s_seq;          /* seq_unlock or seq_lock */
static uint8_t         s_idx;          /* current step index */
static uint8_t         s_sub;          /* SB_* within the current step */
static uint16_t        s_t0;           /* ms when the current sub-phase began */
static uint16_t        s_dur;          /* sub-phase length (delta — wrap-safe) */
static uint8_t         s_rail;         /* RAIL_* for the current step */
static uint8_t         s_sol_pwm;      /* 1 = economizer PWM hold, 0 = full DC */
static uint8_t         s_eoff_armed;   /* opposite state confirmed this step */
static uint8_t         s_eoff_arm_cnt; /* consecutive opposite-state samples */
static uint8_t         s_eoff_cnt;     /* firing integrator (0..EOFF_CONFIRM_N) */
static uint16_t        s_eoff_last;    /* ms of the last early-off sample */

/* Rail-hot: set whenever the boost is enabled (VSOL above ~Vbat), cleared only
 * after a completed drain window. Persists across finish()/abort so a new cycle
 * never powers a servo on a still-charged rail (see SB_PREDRAIN). */
static uint8_t         s_rail_hot;

static void status_set(uint8_t bit, uint8_t on)
{
    if (on) twi_status |= bit; else twi_status &= (uint8_t)~bit;
}

/* Enter a sub-phase for `dur` ms. Elapsed time is measured as a 16-bit DELTA
 * from s_t0, so it stays correct when g_ms wraps (65.5 s) mid-cycle — any
 * single sub-phase is < 65.5 s even if the whole sequence is much longer. */
static void sub_enter(uint8_t sub, uint16_t dur)
{
    s_sub = sub;
    s_t0 = ms_now();
    s_dur = dur;
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

/* Emergency stop (CMD_ABORT): unconditional — even if the engine thinks it is
 * idle, force every actuator/rail off so nothing can stay energized. */
void actuate_abort(void)
{
    finish();
}

/* Begin step `idx`: pick the rail, program the servos, and start the ramp/run. */
static void step_enter(uint8_t idx)
{
    const config_t *c = config_get();
    const step_t *st = &s_seq[idx];
    uint8_t has_sv  = st->act & (STEP_SERVO1 | STEP_SERVO2);
    uint8_t has_sol = st->act & STEP_SOLENOID;

    s_idx = idx;
    s_eoff_armed = 0;
    s_eoff_arm_cnt = 0;
    s_eoff_cnt = 0;
    s_eoff_last = ms_now();

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
        servo_power(false);                /* servo off BEFORE raising the rail */
        servo_pwm_stop();                  /* (Q5 interlock backs this up in HW) */
        boost_12v_enable();                /* VSEL high: servo interlocked out */
        s_rail_hot = 1;
        status_set(ST_RAIL_12V, 1);
        sub_enter(SB_RAMP, SOL_BOOST_RAMP_MS);
    } else if (s_rail == RAIL_6V) {
        boost_servo_enable();              /* VSEL low -> 6 V, interlock clear */
        s_rail_hot = 1;
        status_set(ST_RAIL_12V, 0);
        if (has_sv) servo_power(true);     /* soft-start: ride the rail up */
        sub_enter(SB_RAMP, SOL_BOOST_RAMP_MS);
    } else {                               /* RAIL_VBAT (servo only) */
        boost_disable();
        status_set(ST_RAIL_12V, 0);
        if (has_sv) servo_power(true);
        sub_enter(SB_RUN, (uint16_t)st->dur_ds * 100u);   /* no ramp needed */
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
    if (s_rail_hot) {             /* aborted mid-boost: VSOL may still be charged.
                                   * Bleed passively (coil off) before step 0 so a
                                   * servo is never powered on a hot rail. */
        s_idx = 0;
        sub_enter(SB_PREDRAIN, PREDRAIN_MS);
        return;
    }
    step_enter(0);
}

/* End the current RUN sub-phase: release this step's actuators, then drain (if a
 * boosted rail was up) or advance straight to the next step. */
static void end_run(uint8_t has_sv, uint8_t has_sol)
{
    if (has_sv) { servo_power(false); servo_pwm_stop(); }
    if (has_sol && s_sol_pwm) sol_hold_stop();   /* -> PA5 DC high; coil drains VSOL */
    /* full-DC solenoid (combined) stays energized through the drain, then sol_off */

    if (s_rail == RAIL_VBAT) {
        step_advance();
    } else {
        boost_disable();
        status_set(ST_RAIL_12V, 0);
        sub_enter(SB_DRAIN, SOL_DRAIN_MS);
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

    /* Per-step early-off: during the RUN window, sample the chosen logical
     * sensor every EOFF_SAMPLE_MS. ARM on EOFF_ARM_N consecutive opposite-state
     * samples, then FIRE when the integrator (+1 wanted / -EOFF_MISS_PENALTY
     * opposite) reaches EOFF_CONFIRM_N. See the deglitching note above. */
    if (s_sub == SB_RUN) {
        uint8_t sel = st->eoff & EOFF_SENSOR_MASK;
        if (sel != EOFF_NONE &&
            (uint16_t)(ms_now() - s_eoff_last) >= EOFF_SAMPLE_MS) {
            s_eoff_last = ms_now();
            uint8_t src = (sel == EOFF_DOOR) ? cfg_door_src() : cfg_bolt_src();
            uint8_t present = hall_src(src, hall_sample()) ? 1 : 0;
            uint8_t want = (st->eoff & EOFF_EDGE_ABSENT) ? !present : present;
            if (!s_eoff_armed) {               /* must see the opposite state first */
                if (!want && ++s_eoff_arm_cnt >= EOFF_ARM_N)
                    s_eoff_armed = 1;
                else if (want)
                    s_eoff_arm_cnt = 0;
            } else if (want) {
                if (++s_eoff_cnt >= EOFF_CONFIRM_N)
                    s_dur = 0;                 /* force the RUN window to end now */
            } else {
                s_eoff_cnt = (s_eoff_cnt > EOFF_MISS_PENALTY)
                           ? (uint8_t)(s_eoff_cnt - EOFF_MISS_PENALTY) : 0;
            }
        }
    }

    if ((uint16_t)(ms_now() - s_t0) < s_dur)   /* wrap-safe delta compare */
        return;                                /* sub-phase not elapsed yet */

    switch (s_sub) {
    case SB_PREDRAIN:                         /* post-abort bleed done */
        s_rail_hot = 0;
        step_enter(0);
        break;
    case SB_RAMP:                             /* rail reached target */
        if (has_sol) {
            sol_on();                         /* full DC (strike for both modes) */
            if (s_sol_pwm) sub_enter(SB_STRIKE, cfg_strike_ms());
            else           sub_enter(SB_RUN, dur);   /* 6 V combined */
        } else {                              /* boosted servo, already powered */
            sub_enter(SB_RUN, dur);
        }
        break;
    case SB_STRIKE:                           /* 12 V economizer: strike done */
        if (dur > 0) {
            sol_hold_start(c->hold_duty);
            sub_enter(SB_RUN, dur);
        } else {                              /* no hold -> drain */
            boost_disable();
            status_set(ST_RAIL_12V, 0);
            sub_enter(SB_DRAIN, SOL_DRAIN_MS);
        }
        break;
    case SB_RUN:
        end_run(has_sv, has_sol);
        break;
    case SB_DRAIN:                            /* VSOL bled -> release, next step */
        if (has_sol) sol_off();
        s_rail_hot = 0;                       /* completed drain: rail back at ~Vbat */
        step_advance();
        break;
    }
}
