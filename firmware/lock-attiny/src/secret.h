/*
 * ephemerkey lock board — pairing-secret access (ATtiny1616)
 * Reads the HMAC pairing secret from USERROW; falls back to a compile-time DEV
 * default when USERROW is blank. See lock_config.h.
 */
#ifndef SECRET_H
#define SECRET_H

#include <stdint.h>
#include "lock_config.h"

/* Copy the LOCK_SECRET_LEN-byte pairing secret into out[]. */
void secret_get(uint8_t out[LOCK_SECRET_LEN]);

#endif /* SECRET_H */
