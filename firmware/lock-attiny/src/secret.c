/*
 * ephemerkey lock board — pairing-secret access (ATtiny1616)
 */
#include <avr/io.h>
#include <string.h>
#include "secret.h"

/* DEV DEFAULT — bench only. REPLACE by provisioning USERROW over UPDI, then
 * set lockbits. If USERROW holds a real secret this is never used. */
static const uint8_t k_fallback[LOCK_SECRET_LEN] = {
    0x65, 0x70, 0x68, 0x65, 0x6D, 0x65, 0x72, 0x6B, 0x65, 0x79,
    0x2D, 0x64, 0x65, 0x76, 0x2D, 0x73, 0x65, 0x63, 0x72, 0x74,
};

void secret_get(uint8_t out[LOCK_SECRET_LEN])
{
    const uint8_t *ur = (const uint8_t *)&USERROW;   /* mapped at 0x1300 */

    uint8_t blank = 1;
    for (uint8_t i = 0; i < LOCK_SECRET_LEN; i++) {
        if (ur[i] != 0xFF) { blank = 0; break; }
    }
    memcpy(out, blank ? k_fallback : ur, LOCK_SECRET_LEN);
}
