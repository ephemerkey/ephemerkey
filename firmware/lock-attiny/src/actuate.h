/*
 * ephemerkey lock board — lock/unlock actuation (ATtiny1616)
 * Wraps the servo OR solenoid+boost drivers per LOCK_ACTUATOR (lock_config.h),
 * including the boost<->servo mutual-exclusion + VSOL drain sequencing.
 * Blocking; call from the main loop (not an ISR).
 */
#ifndef ACTUATE_H
#define ACTUATE_H

void actuate_init(void);   /* safe state for whichever actuator is built */
void actuate_unlock(void);
void actuate_lock(void);   /* servo: lock angle; momentary solenoid: no-op */

#endif /* ACTUATE_H */
