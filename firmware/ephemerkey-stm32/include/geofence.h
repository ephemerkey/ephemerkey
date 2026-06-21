/* SPDX-License-Identifier: Apache-2.0 */
/* Geofence test: is a lat/lon fix inside any authorized circle? */

#ifndef EPHEMERKEY_GEOFENCE_H
#define EPHEMERKEY_GEOFENCE_H

#include <stdbool.h>

/* Great-circle distance in meters between two WGS-84 points (haversine). */
double geofence_distance_m(double lat1, double lon1, double lat2, double lon2);

/* True if (lat, lon) lies within radius of any configured geofence. */
bool geofence_contains(double lat, double lon);

#endif /* EPHEMERKEY_GEOFENCE_H */
