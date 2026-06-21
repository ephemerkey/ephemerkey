/* SPDX-License-Identifier: Apache-2.0 */
/* Minimal NMEA 0183 parser for the MAX-M10S (RMC + GGA).
 * Self-contained (no external deps). For a fuller parser consider minmea
 * (MIT, github.com/kosma/minmea). */

#ifndef EPHEMERKEY_GNSS_H
#define EPHEMERKEY_GNSS_H

#include <stdint.h>
#include <stdbool.h>

typedef struct {
    bool     valid;        /* RMC status 'A' (active) */
    double   lat;          /* decimal degrees, +N */
    double   lon;          /* decimal degrees, +E */
    uint8_t  satellites;   /* from GGA */
    double   hdop;         /* from GGA */
    /* UTC date/time from RMC, for RTC discipline */
    uint16_t year;         /* full year, e.g. 2026 */
    uint8_t  month;        /* 1..12 */
    uint8_t  day;          /* 1..31 */
    uint8_t  hour;         /* 0..23 */
    uint8_t  minute;       /* 0..59 */
    uint8_t  second;       /* 0..59 */
    bool     time_valid;   /* date+time fields populated this fix */
} gnss_fix_t;

/* Reset the line accumulator and fix state. */
void gnss_reset(void);

/* Feed one received UART byte. Returns true when a full, checksum-valid
 * sentence was just parsed (call gnss_get_fix() to read the merged state). */
bool gnss_feed_byte(uint8_t b);

/* Most recent merged fix (RMC ∧ GGA fields). */
const gnss_fix_t *gnss_get_fix(void);

#endif /* EPHEMERKEY_GNSS_H */
