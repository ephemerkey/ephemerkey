/* SPDX-License-Identifier: Apache-2.0 */
/* Geofence test — see geofence.h. */

#include "geofence.h"
#include "ephemerkey_config.h"
#include <math.h>

#define EARTH_RADIUS_M  6371000.0
#define DEG2RAD         (M_PI / 180.0)

double geofence_distance_m(double lat1, double lon1, double lat2, double lon2)
{
    double phi1 = lat1 * DEG2RAD;
    double phi2 = lat2 * DEG2RAD;
    double dphi = (lat2 - lat1) * DEG2RAD;
    double dlmb = (lon2 - lon1) * DEG2RAD;

    double a = sin(dphi / 2.0) * sin(dphi / 2.0) +
               cos(phi1) * cos(phi2) * sin(dlmb / 2.0) * sin(dlmb / 2.0);
    double c = 2.0 * atan2(sqrt(a), sqrt(1.0 - a));
    return EARTH_RADIUS_M * c;
}

bool geofence_contains(double lat, double lon)
{
    for (size_t i = 0; i < EK_GEOFENCE_COUNT; i++) {
        double d = geofence_distance_m(lat, lon,
                                       EK_GEOFENCES[i].lat,
                                       EK_GEOFENCES[i].lon);
        if (d <= EK_GEOFENCES[i].radius_m) {
            return true;
        }
    }
    return false;
}
