/*
 * ephemerkey lock board — challenge nonce + anti-replay (ATtiny1616)
 */
#include <avr/eeprom.h>
#include <string.h>
#include "sha1.h"
#include "nonce.h"

/* 32-bit monotonic counter at EEPROM offset 0. */
#define EE_CTR_ADDR   ((uint32_t *)0)
#define CTR_STRIDE    64u        /* EEPROM writes amortised: 1 per 64 nonces */

static uint32_t s_ctr;           /* current counter (RAM) */
static uint32_t s_watermark;     /* value currently persisted in EEPROM */

void nonce_init(void)
{
    uint32_t stored = eeprom_read_dword(EE_CTR_ADDR);
    if (stored == 0xFFFFFFFFUL) stored = 0;   /* blank EEPROM */

    s_ctr = stored;
    s_watermark = stored + CTR_STRIDE;
    /* Reserve a range so a reset mid-cycle can't reuse counter values. */
    eeprom_update_dword(EE_CTR_ADDR, s_watermark);
}

void nonce_next(uint8_t out[NONCE_LEN])
{
    if (++s_ctr >= s_watermark) {
        s_watermark = s_ctr + CTR_STRIDE;
        eeprom_update_dword(EE_CTR_ADDR, s_watermark);
    }

    const uint8_t in[4] = {
        (uint8_t)s_ctr, (uint8_t)(s_ctr >> 8),
        (uint8_t)(s_ctr >> 16), (uint8_t)(s_ctr >> 24),
    };
    uint8_t digest[SHA1_DIGEST_SIZE];
    sha1_ctx_t c;
    sha1_init(&c);
    sha1_update(&c, in, sizeof(in));
    sha1_final(&c, digest);

    memcpy(out, digest, NONCE_LEN);   /* first 16 of 20 */
}

uint8_t ct_equal(const uint8_t *a, const uint8_t *b, uint8_t len)
{
    uint8_t diff = 0;
    for (uint8_t i = 0; i < len; i++) diff |= (uint8_t)(a[i] ^ b[i]);
    return diff == 0;
}
