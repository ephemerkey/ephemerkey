/*
 * ephemerkey lock board — 12 V solenoid driver (ATtiny1616)
 *
 *   PA5  SOL_PWM -> R5(100R) -> Q1(AO3400) gate; Q1 low-side switches the coil
 *   coil: VSOL (J3.1) -> SOL_DRV (Q1 drain); D3 flyback across it.
 *
 * Requires VSOL at +12 V (see power.h). Uses a firmware peak-and-hold profile:
 * full drive to pull the armature in (strike), then 50 %-duty PWM to hold it
 * with far less current (economizer). The hold PWM is generated in hardware by
 * TCD0/WOB (PA5) at ~31 kHz — inaudible and CPU-free. (TCA0 is reserved for the
 * servos and cannot reach PA5.)
 */
#ifndef SOLENOID_H
#define SOLENOID_H

#include <stdint.h>

/* PA5 low, Q1 off. Call once at boot. */
void sol_init(void);

/* Q1 fully on / off (DC). */
void sol_on(void);
void sol_off(void);

/* Peak-and-hold: drive full power for strike_ms, then 50%-duty PWM hold for
 * hold_ms. Blocks for (strike_ms + hold_ms). Leaves Q1 ON at the end so the
 * caller can drain VSOL through the coil before releasing. */
void sol_peak_and_hold(uint16_t strike_ms, uint16_t hold_ms);

#endif /* SOLENOID_H */
