/*
 * ephemerkey lock board — challenge nonce + anti-replay (ATtiny1616)
 *
 * The nonce is derived from a monotonic counter persisted in EEPROM, so nonces
 * never repeat across reboots/power loss (replay-proof even without a TRNG).
 * The counter feeds SHA1 for whitening; the armed nonce is single-use.
 */
#ifndef NONCE_H
#define NONCE_H

#include <stdint.h>

#define NONCE_LEN   16u

/* Seed the counter from EEPROM and reserve a range for this power cycle. */
void nonce_init(void);

/* Produce the next fresh 16-byte nonce (advances the monotonic counter, and
 * persists a new EEPROM watermark once per range). */
void nonce_next(uint8_t out[NONCE_LEN]);

/* Constant-time equality (1 = equal). Use for HMAC comparison. */
uint8_t ct_equal(const uint8_t *a, const uint8_t *b, uint8_t len);

#endif /* NONCE_H */
