/*
 * ephemerkey lock board — HMAC secrets (ATtiny1616)
 */
#include <avr/io.h>
#include <string.h>
#include "secret.h"

/* DEV DEFAULTS — bench only, exactly SECRET_LEN bytes (no NUL). REPLACE by
 * provisioning USERROW over UPDI. */
static const uint8_t k_pairing_fallback[SECRET_LEN] __attribute__((nonstring)) = "ephemerkey-dev01";
static const uint8_t k_config_fallback[SECRET_LEN]  __attribute__((nonstring)) = "ephemerkey-cfg01";

static void get_secret(uint8_t offset, const uint8_t *fallback, uint8_t out[SECRET_LEN])
{
    const uint8_t *ur = (const uint8_t *)&USERROW + offset;   /* USERROW @ 0x1300 */

    uint8_t blank = 1;
    for (uint8_t i = 0; i < SECRET_LEN; i++) {
        if (ur[i] != 0xFF) { blank = 0; break; }
    }
    memcpy(out, blank ? fallback : ur, SECRET_LEN);
}

void secret_get_pairing(uint8_t out[SECRET_LEN]) { get_secret(0,  k_pairing_fallback, out); }
void secret_get_config(uint8_t out[SECRET_LEN])  { get_secret(16, k_config_fallback,  out); }
