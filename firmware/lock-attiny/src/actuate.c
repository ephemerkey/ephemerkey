/*
 * ephemerkey lock board — non-blocking actuator state machine (ATtiny1616)
 * See actuate.h.
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

/* --- phases --- */
enum { PH_IDLE, PH_SERVO, PH_BOOST, PH_STRIKE, PH_HOLD, PH_DRAIN };
static uint8_t  s_phase = PH_IDLE;
static uint16_t s_end;
static uint8_t  s_unlock;    /* target of the current cycle (for servo->solenoid chain) */

static void status_set(uint8_t bit, uint8_t on)
{
    if (on) twi_status |= bit; else twi_status &= (uint8_t)~bit;
}

void actuate_init(void) { s_phase = PH_IDLE; }
uint8_t actuate_busy(void) { return s_phase != PH_IDLE; }

/* Everything off, safe, idle. TCD is already stopped before we get here. */
static void finish(void)
{
    servo_power(false);
    servo_pwm_stop();
    sol_off();
    boost_disable();
    status_set(ST_RAIL_12V, 0);
    status_set(ST_BUSY, 0);
    s_phase = PH_IDLE;
    tick_stop();
}

/* Clean up whatever is currently active (for an abort). */
static void teardown(void)
{
    switch (s_phase) {
    case PH_SERVO:
        servo_power(false);
        servo_pwm_stop();
        break;
    case PH_HOLD:
        sol_hold_stop();          /* stop TCD before the DC cleanup */
        /* FALLTHROUGH */
    case PH_BOOST:
    case PH_STRIKE:
    case PH_DRAIN:
        boost_disable();
        sol_off();
        status_set(ST_RAIL_12V, 0);
        break;
    default:
        break;
    }
    s_phase = PH_IDLE;
}

void actuate_begin(uint8_t unlock)
{
    const config_t *c = config_get();

    teardown();                   /* abort any in-progress cycle */
    tick_start();
    status_set(ST_BUSY, 1);
    s_unlock = unlock;

    if (c->flags & (CFG_SERVO1_EN | CFG_SERVO2_EN)) {
        boost_disable();          /* servos run on Vbat */
        status_set(ST_RAIL_12V, 0);
        if (c->flags & CFG_SERVO1_EN)
            servo1_set_us(cfg_pos_to_us(unlock ? c->s1_unlock : c->s1_lock));
        if (c->flags & CFG_SERVO2_EN)
            servo2_set_us(cfg_pos_to_us(unlock ? c->s2_unlock : c->s2_lock));
        servo_pwm_start();
        servo_power(true);
        s_phase = PH_SERVO;
        s_end = ms_now() + cfg_primary_ms();
    } else if ((c->flags & CFG_SOLENOID_EN) && unlock) {
        boost_12v_enable();
        status_set(ST_RAIL_12V, 1);
        s_phase = PH_BOOST;
        s_end = ms_now() + SOL_BOOST_RAMP_MS;
    } else {
        finish();                 /* nothing to do (e.g. solenoid + LOCK) */
    }
}

void actuate_tick(void)
{
    if (s_phase == PH_IDLE) return;
    if (ms_now() < s_end) return;                 /* phase not elapsed yet */

    const config_t *c = config_get();
    uint16_t t = ms_now();

    switch (s_phase) {
    case PH_SERVO:
        servo_power(false);                       /* drive time up -> release */
        servo_pwm_stop();
        if ((c->flags & CFG_SOLENOID_EN) && s_unlock) {
            boost_12v_enable();                   /* then chain into the solenoid */
            status_set(ST_RAIL_12V, 1);
            s_phase = PH_BOOST;
            s_end = t + SOL_BOOST_RAMP_MS;
        } else {
            finish();
        }
        break;
    case PH_BOOST:                                /* rail up -> strike */
        sol_on();
        s_phase = PH_STRIKE;
        s_end = t + cfg_primary_ms();
        break;
    case PH_STRIKE:
        if (cfg_hold_ms() > 0) {                  /* -> economizer hold */
            sol_hold_start(c->hold_duty);
            s_phase = PH_HOLD;
            s_end = t + cfg_hold_ms();
        } else {                                  /* -> drain immediately */
            boost_disable();
            status_set(ST_RAIL_12V, 0);
            s_phase = PH_DRAIN;
            s_end = t + SOL_DRAIN_MS;
        }
        break;
    case PH_HOLD:                                 /* hold done -> drain */
        sol_hold_stop();                          /* -> PA5 DC high, coil conducts */
        boost_disable();
        status_set(ST_RAIL_12V, 0);
        s_phase = PH_DRAIN;
        s_end = t + SOL_DRAIN_MS;
        break;
    case PH_DRAIN:                                /* VSOL bled -> release */
        sol_off();
        finish();
        break;
    }
}
