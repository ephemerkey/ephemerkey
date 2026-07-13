/*
 * ephemerkey lock board — non-blocking actuator state machine (ATtiny1616)
 *
 * A config-driven STEP SEQUENCE engine: actuate_begin(unlock) runs the UNLOCK or
 * LOCK step list from config, firing each step's actuators in order and turning
 * them off when done, WITHOUT blocking — advanced by actuate_tick() from the main
 * loop, timed by a TCB0 millisecond tick that runs only during a cycle. So I2C
 * stays responsive throughout, and a new lock/unlock aborts the current cycle.
 *
 * Each step drives any combination of {servo1, servo2, solenoid} with per-step
 * servo targets, a run time, and an optional hall early-off that ends the step
 * and advances to the next. See config.h (step_t) and actuate.c for the per-step
 * rail selection (Vbat / 6 V / 12 V economizer).
 */
#ifndef ACTUATE_H
#define ACTUATE_H

#include <stdint.h>

void    actuate_init(void);
void    actuate_begin(uint8_t unlock);  /* start or ABORT toward unlock(1)/lock(0) */
void    actuate_tick(void);             /* advance the machine; call every loop */
uint8_t actuate_busy(void);             /* 1 while a cycle is in progress */
void    actuate_abort(void);            /* stop NOW: everything off, cycle ended */

#endif /* ACTUATE_H */
