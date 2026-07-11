/*
 * ephemerkey lock board — HMAC secrets (ATtiny1616)
 *
 * Two independent secrets, split across USERROW (0x1300), each SECRET_LEN bytes:
 *   pairing secret  = USERROW[0 .. 15]   — authorizes UNLOCK / LOCK
 *   config secret   = USERROW[16 .. 31]  — authorizes REG_CONFIG writes (admin)
 * Compile-time DEV fallbacks are used when USERROW is blank. Provision over UPDI
 * and set lockbits in production.
 */
#ifndef SECRET_H
#define SECRET_H

#include <stdint.h>

#define SECRET_LEN   16u

void secret_get_pairing(uint8_t out[SECRET_LEN]);
void secret_get_config(uint8_t out[SECRET_LEN]);

#endif /* SECRET_H */
