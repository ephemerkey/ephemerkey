/* SPDX-License-Identifier: Apache-2.0 */
/* ephemerkey application configuration.
 *
 * In production these values (secret, geofence) are provisioned over USB into
 * a protected flash page and read at boot — the compile-time defaults here are
 * for bench bring-up only. Do NOT ship a device with the example secret. */

#ifndef EPHEMERKEY_CONFIG_H
#define EPHEMERKEY_CONFIG_H

#include <stdint.h>
#include <stddef.h>

/* ---- TOTP (RFC 6238) ------------------------------------------------------ */
/* Base32 shared secret. Example vector from RFC test material — REPLACE. */
#define EK_TOTP_SECRET_B32   "JBSWY3DPEHPK3PXP"
#define EK_TOTP_TIME_STEP    30U     /* seconds per code */
#define EK_TOTP_DIGITS       6U      /* code length */

/* ---- Geofence ------------------------------------------------------------- */
/* Authorized circles: a fix inside ANY circle (haversine <= radius) passes. */
typedef struct {
    double   lat;        /* decimal degrees, +N */
    double   lon;        /* decimal degrees, +E */
    double   radius_m;   /* meters */
} ek_geofence_t;

/* Example: replace with your authorized location(s). */
static const ek_geofence_t EK_GEOFENCES[] = {
    { 37.7749, -122.4194, 150.0 },   /* example: SF, 150 m */
};
#define EK_GEOFENCE_COUNT  (sizeof(EK_GEOFENCES) / sizeof(EK_GEOFENCES[0]))

/* ---- Fix-quality gating --------------------------------------------------- */
#define EK_MIN_SATELLITES    4U      /* require >= N sats in solution */
#define EK_MAX_HDOP          5.0     /* reject fixes with HDOP above this */

/* ---- Anti-replay clock staleness ------------------------------------------ */
/* Refuse to emit a code if the RTC has not been disciplined by GNSS within
 * this many seconds (a frozen/rolled-back clock must not yield valid codes). */
#define EK_CLOCK_MAX_STALENESS_S   3600U

/* ---- Duty cycle ----------------------------------------------------------- */
#define EK_GNSS_ACQUIRE_TIMEOUT_S  120U   /* give up acquiring a fix after this */
#define EK_SLEEP_INTERVAL_S        300U   /* periodic wake even without motion */

#endif /* EPHEMERKEY_CONFIG_H */
