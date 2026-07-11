/*
 * ephemerkey lock board — boost-rail power control (ATtiny1616)
 *
 *   PA6  SOL_BOOST_EN  -> MT3608 EN
 *   PA1  BOOST_VSEL    -> 6V/12V FB select (Q2) + servo interlock (Q5)
 *
 * The MT3608 makes VSOL. Disabled, its L1+D2 path passes ~Vbat-0.3 V to VSOL
 * (the servo supply). Enabled with VSEL high it regulates VSOL to +12 V for the
 * solenoid. VSEL high also engages the Q5 interlock that force-disables servo
 * power — so 12 V and servo power can never coexist.
 *
 * IMPORTANT: VSOL feeds the servo. After a 12 V solenoid strike, VSOL must be
 * drained back to ~Vbat (boost off + solenoid conducting) BEFORE re-powering the
 * servo. See the main loop's drain phase.
 */
#ifndef POWER_H
#define POWER_H

/* Boost off, VSEL low (6V/interlock clear), pins driven. Call once at boot,
 * before servo/solenoid init. */
void power_init(void);

/* Bring VSOL up to +12 V: select 12 V first (engages servo interlock), then
 * enable the boost. Caller must wait for the rail to ramp (~500 ms). */
void boost_12v_enable(void);

/* Disable the boost and return VSEL low. VSOL then decays toward ~Vbat (drain
 * it through the solenoid before the next servo phase). */
void boost_disable(void);

/* Higher-voltage servos (CFG_SERVO_BOOST): raise the servo rail via the boost.
 * ** Not usable on current hardware ** — asserting BOOST_VSEL engages the Q5
 * interlock that disables servo power. Kept behind the config flag for a future
 * hardware rev; identical signal assertion to boost_12v_enable today. */
void boost_servo_enable(void);

#endif /* POWER_H */
