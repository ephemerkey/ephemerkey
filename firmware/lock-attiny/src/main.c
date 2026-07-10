/*
 * ephemerkey lock board — ATtiny1616 firmware
 * Bringup stage 3: servo + gated 12 V solenoid, mutually exclusive rails.
 *
 * One cycle, repeated forever:
 *   1. SERVO   — both servos actuate on battery voltage (boost off, VSOL~Vbat).
 *   2.           servo power off.
 *   3. BOOST    — enable the MT3608 to +12 V; wait 500 ms to ramp + charge C5.
 *   4. SOLENOID — drive the coil for ~10 s: full-power strike, then 50 % hold.
 *   5. DRAIN    — disable the boost with the solenoid STILL conducting, so the
 *                 12 V on VSOL bleeds through the coil back to ~Vbat; only THEN
 *                 release the solenoid. This is mandatory: VSOL is the servo's
 *                 supply, so it must be at Vbat before the servo runs again.
 *
 * The boost and the servo supply are never energised at once — enforced in
 * firmware by this ordering and in hardware by the Q5 interlock (BOOST_VSEL).
 *
 * Status LED D1 (PC3) blinks at 1 Hz throughout via the RTC PIT as an
 * "alive" beacon, independent of the phase sequence.
 *
 * Clock: reset default (20 MHz / 6 = 3.33 MHz); F_CPU must match.
 */

#include <avr/io.h>
#include <avr/interrupt.h>
#include <util/delay.h>

#include "servo.h"
#include "power.h"
#include "solenoid.h"

#define LED_PIN         PIN3_bm      /* PC3 */

#define SERVO_MOVE_MS   500          /* dwell at each servo endpoint      */
#define BOOST_RAMP_MS   500          /* boost settle before the strike    */
#define SOL_STRIKE_MS   50           /* full-power pull-in                 */
#define SOL_HOLD_MS     9950         /* 50 % hold -> ~10 s total actuation */
#define VSOL_DRAIN_MS   500          /* bleed 12 V off VSOL before release */
#define CYCLE_GAP_MS    500          /* settle before the next servo phase */

/* LED heartbeat, fully interrupt-driven so it survives the blocking phases. */
ISR(RTC_PIT_vect)
{
    RTC.PITINTFLAGS = RTC_PI_bm;
    PORTC.OUTTGL    = LED_PIN;
}

static void led_init(void)
{
    PORTC.OUTCLR = LED_PIN;
    PORTC.DIRSET = LED_PIN;
}

static void pit_init(void)
{
    RTC.CLKSEL     = RTC_CLKSEL_INT32K_gc;
    RTC.PITINTCTRL = RTC_PI_bm;
    while (RTC.PITSTATUS & RTC_CTRLBUSY_bm) { }
    RTC.PITCTRLA   = RTC_PERIOD_CYC16384_gc | RTC_PITEN_bm;
}

/* One dual-servo move: valid pulses first, then power the shared VSERVO rail,
 * let both travel, then cut power and stop the signals. */
static void servo_move(uint16_t s1_us, uint16_t s2_us)
{
    servo1_set_us(s1_us);
    servo2_set_us(s2_us);
    servo_pwm_start();
    servo_power(true);          /* VSERVO on (battery voltage), both connectors */
    _delay_ms(SERVO_MOVE_MS);
    servo_power(false);
    servo_pwm_stop();
}

int main(void)
{
    led_init();
    power_init();               /* boost off, VSEL low, interlock clear — FIRST */
    servo_init();               /* servo power off, PWM configured */
    sol_init();                 /* Q1 off */
    pit_init();

    sei();                      /* enable the LED heartbeat */

    for (;;) {
        /* 1. SERVO phase — both servos, opposite phase, on battery voltage. */
        servo_move(SERVO_MAX_US, SERVO_MIN_US);
        servo_move(SERVO_MIN_US, SERVO_MAX_US);

        /* 2. servo power is already off (servo_move leaves it off). */

        /* 3. BOOST up to +12 V and let the rail settle. */
        boost_12v_enable();
        _delay_ms(BOOST_RAMP_MS);

        /* 4. SOLENOID: strike then hold (~10 s). Leaves Q1 conducting. */
        sol_peak_and_hold(SOL_STRIKE_MS, SOL_HOLD_MS);

        /* 5. DRAIN: boost off while the solenoid still conducts, so 12 V bleeds
         *    off VSOL down to ~Vbat; only then release the coil. */
        boost_disable();
        _delay_ms(VSOL_DRAIN_MS);
        sol_off();

        _delay_ms(CYCLE_GAP_MS);
    }
}
