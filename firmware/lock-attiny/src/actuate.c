/*
 * ephemerkey lock board — lock/unlock actuation (ATtiny1616)
 */
#include <util/delay.h>
#include "lock_config.h"
#include "actuate.h"
#include "servo.h"
#include "power.h"
#include "solenoid.h"

static void delay_ms_n(uint16_t ms) { while (ms--) _delay_ms(1); }

#if LOCK_ACTUATOR == ACTUATOR_SERVO

void actuate_init(void) { /* servo_init()/power_init() done in main */ }

static void servo_to(uint16_t us)
{
    servo1_set_us(us);
    servo_pwm_start();
    servo_power(true);            /* battery voltage; boost stays off */
    delay_ms_n(SERVO_TRAVEL_MS);
    servo_power(false);
    servo_pwm_stop();
}

void actuate_unlock(void) { servo_to(SERVO_UNLOCK_US); }
void actuate_lock(void)   { servo_to(SERVO_LOCK_US); }

#else  /* ACTUATOR_SOLENOID */

void actuate_init(void) { /* sol_init()/power_init() done in main */ }

void actuate_unlock(void)
{
    boost_12v_enable();               /* +12 V for the coil */
    delay_ms_n(SOL_BOOST_RAMP_MS);
    sol_peak_and_hold(SOL_STRIKE_MS, SOL_HOLD_MS);   /* leaves Q1 conducting */
    boost_disable();                  /* drain VSOL through the coil... */
    delay_ms_n(SOL_DRAIN_MS);
    sol_off();                        /* ...then release */
}

/* Momentary solenoid re-latches mechanically (fail-secure): LOCK is a no-op. */
void actuate_lock(void) { }

#endif
