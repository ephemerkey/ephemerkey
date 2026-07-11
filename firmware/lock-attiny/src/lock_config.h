/*
 * ephemerkey lock board — build/config constants (ATtiny1616)
 */
#ifndef LOCK_CONFIG_H
#define LOCK_CONFIG_H

/* I2C target address (7-bit) — per hardware/lock/README.md. */
#define LOCK_I2C_ADDR       0x60

/* Which actuator is populated on this board (DRV sheet is "build one"). */
#define ACTUATOR_SOLENOID   0
#define ACTUATOR_SERVO      1
#ifndef LOCK_ACTUATOR
#define LOCK_ACTUATOR       ACTUATOR_SERVO   /* override with -DLOCK_ACTUATOR=... */
#endif

/* Servo lock/unlock pulse widths (within the verified 600–2400 us range). */
#define SERVO_LOCK_US       1000u
#define SERVO_UNLOCK_US     2000u
#define SERVO_TRAVEL_MS     600u    /* power window for a full swing */

/* Solenoid actuation profile (shorter than the 10 s bench demo). */
#define SOL_BOOST_RAMP_MS   500u
#define SOL_STRIKE_MS       50u
#define SOL_HOLD_MS         200u
#define SOL_DRAIN_MS        500u

/* Pairing secret: LOCK_SECRET_LEN bytes read from USERROW (0x1300); if USERROW
 * is blank (all 0xFF) a compile-time DEV fallback is used. Distinct from the
 * ephemerkey TOTP secret. Provision via UPDI + set lockbits in production. */
#define LOCK_SECRET_LEN     20u

#endif /* LOCK_CONFIG_H */
