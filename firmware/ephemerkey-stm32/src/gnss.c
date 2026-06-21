/* SPDX-License-Identifier: Apache-2.0 */
/* Minimal NMEA RMC/GGA parser — see gnss.h. */

#include "gnss.h"
#include <string.h>
#include <stdlib.h>

#define NMEA_MAX 96

static char       s_line[NMEA_MAX];
static uint8_t    s_len;
static gnss_fix_t s_fix;

void gnss_reset(void)
{
    s_len = 0;
    memset(&s_fix, 0, sizeof(s_fix));
}

/* Convert NMEA "ddmm.mmmm" + hemisphere to signed decimal degrees. */
static double nmea_to_deg(const char *field, char hemi, int deg_digits)
{
    if (field == NULL || field[0] == '\0') {
        return 0.0;
    }
    double v = atof(field);
    int    d = (int)(v / 100.0);          /* leading ddd or dd */
    double m = v - (double)d * 100.0;     /* remaining mm.mmmm */
    (void)deg_digits;
    double deg = (double)d + m / 60.0;
    if (hemi == 'S' || hemi == 'W') {
        deg = -deg;
    }
    return deg;
}

/* Split a comma-separated sentence body into fields (in place). */
static int split_fields(char *body, char *fields[], int max)
{
    int n = 0;
    char *p = body;
    fields[n++] = p;
    while (*p && n < max) {
        if (*p == ',') {
            *p = '\0';
            fields[n++] = p + 1;
        }
        p++;
    }
    return n;
}

static void parse_rmc(char *fields[], int n)
{
    /* $xxRMC,time,status,lat,N,lon,E,speed,course,date,... */
    if (n < 10) {
        return;
    }
    s_fix.valid = (fields[2][0] == 'A');
    if (s_fix.valid) {
        s_fix.lat = nmea_to_deg(fields[3], fields[4][0], 2);
        s_fix.lon = nmea_to_deg(fields[5], fields[6][0], 3);
    }
    /* time hhmmss(.sss) */
    const char *t = fields[1];
    if (strlen(t) >= 6) {
        s_fix.hour   = (uint8_t)((t[0]-'0')*10 + (t[1]-'0'));
        s_fix.minute = (uint8_t)((t[2]-'0')*10 + (t[3]-'0'));
        s_fix.second = (uint8_t)((t[4]-'0')*10 + (t[5]-'0'));
    }
    /* date ddmmyy */
    const char *d = fields[9];
    if (strlen(d) >= 6) {
        s_fix.day   = (uint8_t)((d[0]-'0')*10 + (d[1]-'0'));
        s_fix.month = (uint8_t)((d[2]-'0')*10 + (d[3]-'0'));
        s_fix.year  = (uint16_t)(2000 + (d[4]-'0')*10 + (d[5]-'0'));
        s_fix.time_valid = (s_fix.month >= 1 && s_fix.month <= 12);
    }
}

static void parse_gga(char *fields[], int n)
{
    /* $xxGGA,time,lat,N,lon,E,fixq,sats,hdop,alt,... */
    if (n < 9) {
        return;
    }
    s_fix.satellites = (uint8_t)atoi(fields[7]);
    s_fix.hdop       = atof(fields[8]);
}

/* XOR checksum of everything between '$' and '*'. */
static bool checksum_ok(const char *line, uint8_t len)
{
    /* line excludes leading '$'; find '*' */
    uint8_t cs = 0;
    uint8_t i = 0;
    for (; i < len; i++) {
        if (line[i] == '*') {
            break;
        }
        cs ^= (uint8_t)line[i];
    }
    if (i + 2 >= len) {
        return false;               /* no/short checksum */
    }
    char hex[3] = { line[i+1], line[i+2], '\0' };
    uint8_t want = (uint8_t)strtol(hex, NULL, 16);
    return cs == want;
}

bool gnss_feed_byte(uint8_t b)
{
    if (b == '$') {                 /* start of sentence */
        s_len = 0;
        return false;
    }
    if (b == '\r' || b == '\n') {
        if (s_len == 0) {
            return false;
        }
        s_line[s_len] = '\0';
        bool ok = checksum_ok(s_line, s_len);
        uint8_t parsed_len = s_len;
        s_len = 0;
        if (!ok) {
            return false;
        }
        /* strip "*HH" before splitting */
        char *star = strchr(s_line, '*');
        if (star) {
            *star = '\0';
        }
        char *fields[24];
        int   n = split_fields(s_line, fields, 24);
        (void)parsed_len;
        if (n >= 1 && strlen(fields[0]) >= 5) {
            const char *type = fields[0] + 2;   /* skip talker id (GP/GN/...) */
            if (strncmp(type, "RMC", 3) == 0) {
                parse_rmc(fields, n);
            } else if (strncmp(type, "GGA", 3) == 0) {
                parse_gga(fields, n);
            }
        }
        return true;
    }
    if (s_len < NMEA_MAX - 1) {
        s_line[s_len++] = (char)b;
    }
    return false;
}

const gnss_fix_t *gnss_get_fix(void)
{
    return &s_fix;
}
