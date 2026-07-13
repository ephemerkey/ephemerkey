/*
 * ephemerkey lock board — hall sensor read (ATtiny1616)
 *
 *   PA4  HALL_PWR  -> powers both hall sensors (J6.1/J7.1) + pull-ups R22/R23
 *   PA7  (J6)      <- J6.3 sensor
 *   PB3  (J7)      <- J7.3 sensor
 *
 * hall_sample/hall_read return RAW per-port readings (magnet present). The
 * door/bolt ROLES are assigned in config (sensor_map): either port can be the
 * door or the bolt indicator. Outputs pull up to HALL_PWR (open-drain), so a
 * present magnet reads LOW.
 */
#ifndef HALL_H
#define HALL_H

#include <stdint.h>

#define HALL_J6  0x01u   /* sensor on J6 (PA7): magnet present */
#define HALL_J7  0x02u   /* sensor on J7 (PB3): magnet present */

void hall_init(void);

/* Hold HALL_PWR on/off explicitly — used to keep the sensors powered across a
 * whole lock/unlock so they can be sampled live during actuation. */
void hall_power(uint8_t on);

/* Read both sensor inputs (no power control / settle) — requires them already
 * powered. Returns HALL_J6 | HALL_J7 (raw magnet-present bits). */
uint8_t hall_sample(void);

/* One-shot: power, settle ~1 ms, sample, drop power (~0 uA between reads). */
uint8_t hall_read(void);

/* Resolve a SENSOR_SRC_* selector against a raw reading: non-zero if the chosen
 * source's magnet is present (SENSOR_SRC_OFF -> always 0). */
uint8_t hall_src(uint8_t src, uint8_t raw);

#endif /* HALL_H */
