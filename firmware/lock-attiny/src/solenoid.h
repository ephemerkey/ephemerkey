/*
 * ephemerkey lock board — 12 V solenoid driver (ATtiny1616)
 *
 *   PA5  SOL_PWM -> R5 -> Q1 (low-side); coil VSOL -> SOL_DRV, D3 flyback.
 *
 * Peak-and-hold economizer, non-blocking: the caller's state machine sequences
 * strike (full DC) -> hold (TCD0/WOB hardware PWM at a configurable duty, ~31 kHz
 * inaudible) -> release. Requires VSOL at +12 V (see power.h).
 */
#ifndef SOLENOID_H
#define SOLENOID_H

#include <stdint.h>

void sol_init(void);

void sol_on(void);                    /* full DC drive (TCD0 off) */
void sol_off(void);

void sol_hold_start(uint8_t duty);    /* start TCD0 PWM on PA5 at duty 0..255 */
void sol_hold_stop(void);             /* stop TCD0; PA5 back to GPIO high */

#endif /* SOLENOID_H */
