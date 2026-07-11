/*
 * ephemerkey lock board — build/config constants (ATtiny1616)
 * Actuator selection, servo positions, and drive timing are now runtime
 * config (see config.h). What remains here is fixed board/bring-up constants.
 */
#ifndef LOCK_CONFIG_H
#define LOCK_CONFIG_H

/* I2C target address (7-bit) — per hardware/lock/README.md. */
#define LOCK_I2C_ADDR       0x60

/* Fixed boost sequencing for the solenoid path (not user-configurable). */
#define SOL_BOOST_RAMP_MS   500u    /* let VSOL reach +12 V */
#define SOL_DRAIN_MS        500u    /* bleed VSOL back to ~Vbat after a strike */

#endif /* LOCK_CONFIG_H */
