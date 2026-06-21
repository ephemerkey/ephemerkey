/* SPDX-License-Identifier: Apache-2.0 */
/* Application glue between the STM32 RTC, the GNSS fix, and smalltotp. */

#ifndef EPHEMERKEY_TOTP_APP_H
#define EPHEMERKEY_TOTP_APP_H

#include <stdint.h>
#include <stdbool.h>
#include "gnss.h"

/* Decode the configured base32 secret and register the RTC time source.
 * Call once at boot. Returns false if the secret fails to decode. */
bool totp_app_init(void);

/* Set the RTC from a GNSS UTC fix and mark the clock freshly disciplined.
 * Call whenever a valid fix with valid date/time is obtained. */
void totp_app_discipline_rtc(const gnss_fix_t *fix);

/* True if the RTC has been GNSS-disciplined within the staleness window. */
bool totp_app_clock_fresh(void);

/* Generate the current TOTP code from the RTC time base. */
uint32_t totp_app_generate(void);

/* Emit a code to the companion lock (UART line + CODE_VALID strobe). */
void totp_app_emit(uint32_t code);

#endif /* EPHEMERKEY_TOTP_APP_H */
