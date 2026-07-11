/*
 * ephemerkey lock board — non-blocking actuator state machine (ATtiny1616)
 *
 * A config-driven state machine drives the servo(s) or solenoid+boost and turns
 * them off when done, WITHOUT blocking — it is advanced by actuate_tick() from
 * the main loop, timed by a TCB0 millisecond tick that runs only during a cycle.
 * So I2C stays responsive throughout, and a new lock/unlock aborts the current
 * cycle (actuate_begin re-targets the machine).
 *
 * Servos: full power for the configured drive time, then released.
 * Solenoid (unlock only): boost -> strike -> economizer hold (TCD0 PWM at the
 *   configured duty) -> drain -> release. LOCK on a solenoid is a no-op.
 */
#ifndef ACTUATE_H
#define ACTUATE_H

#include <stdint.h>

void    actuate_init(void);
void    actuate_begin(uint8_t unlock);  /* start or ABORT toward unlock(1)/lock(0) */
void    actuate_tick(void);             /* advance the machine; call every loop */
uint8_t actuate_busy(void);             /* 1 while a cycle is in progress */

#endif /* ACTUATE_H */
