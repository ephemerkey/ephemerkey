/*
 * ephemerkey lock board — hall sensor read (ATtiny1616)
 *
 *   PA4  HALL_PWR   -> powers both hall sensors (J6.1/J7.1) + pull-ups R22/R23
 *   PA7  HALL_DOOR  <- J6.3 (door sensor)
 *   PB3  HALL_BOLT  <- J7.3 (bolt sensor)
 *
 * Sensors are powered only during a read (~0 uA otherwise). Outputs pull up to
 * HALL_PWR through R22/R23 (open-drain style), so a present magnet reads LOW.
 */
#ifndef HALL_H
#define HALL_H

#include <stdint.h>

#define HALL_DOOR_CLOSED  0x01
#define HALL_BOLT_LOCKED  0x02

void hall_init(void);

/* Pulse HALL_PWR, settle, sample both sensors, drop power. Returns
 * HALL_DOOR_CLOSED | HALL_BOLT_LOCKED bits. */
uint8_t hall_read(void);

#endif /* HALL_H */
