/* SPDX-License-Identifier: Apache-2.0 */
/* TOTP application glue — see totp_app.h.
 *
 * Time base: the STM32 RTC (LSE, GNSS-disciplined) is the single source of
 * truth for TOTP. The GNSS sets the RTC when a valid UTC fix is seen; between
 * fixes the RTC free-runs. A staleness guard refuses codes if the clock has
 * not been disciplined recently (anti-replay).
 *
 * Note: we deliberately do NOT include smalltotp's stm32_rtc_time.h here — it
 * typedefs RTC_TimeTypeDef / RTC_DateTypeDef, which collide with the STM32 HAL
 * types of the same name. We do the civil-date -> Unix conversion locally
 * against the HAL types instead, and reuse smalltotp only for the TOTP core
 * (totp.c / hmac_sha1.c / sha1.c / base32.c / totp_time.c). */

#include "totp_app.h"
#include "ephemerkey_config.h"
#include "board.h"          /* pulls in stm32u0xx_hal.h (HAL RTC/UART types) */

#include "totp.h"
#include "base32.h"
#include "totp_time.h"

#include <string.h>
#include <stdio.h>

extern RTC_HandleTypeDef  hrtc;
extern UART_HandleTypeDef hlock;   /* USART2 to companion lock */

static uint8_t        s_secret[64];
static int            s_secret_len;
static totp_config_t  s_cfg;
static uint64_t       s_last_discipline_unix;  /* RTC time at last GNSS sync */

/* Days since 1970-01-01 for a proleptic-Gregorian civil date.
 * (Howard Hinnant's days_from_civil; valid for any reasonable y/m/d.) */
static int64_t days_from_civil(int y, unsigned m, unsigned d)
{
    y -= (m <= 2);
    int64_t era = (y >= 0 ? y : y - 399) / 400;
    unsigned yoe = (unsigned)(y - era * 400);
    unsigned doy = (153u * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1;
    unsigned doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return era * 146097 + (int64_t)doe - 719468;
}

/* smalltotp time source: read the HAL RTC (UTC) and return Unix seconds. */
static uint64_t ek_rtc_unix(void)
{
    RTC_TimeTypeDef t;
    RTC_DateTypeDef d;
    HAL_RTC_GetTime(&hrtc, &t, RTC_FORMAT_BIN);
    HAL_RTC_GetDate(&hrtc, &d, RTC_FORMAT_BIN);   /* must read date after time */

    int64_t days = days_from_civil(2000 + d.Year, d.Month, d.Date);
    return (uint64_t)(days * 86400 +
                      t.Hours * 3600 + t.Minutes * 60 + t.Seconds);
}

bool totp_app_init(void)
{
    s_secret_len = base32_decode(EK_TOTP_SECRET_B32, s_secret, sizeof(s_secret));
    if (s_secret_len <= 0) {
        return false;
    }
    s_cfg.secret     = s_secret;
    s_cfg.secret_len = (size_t)s_secret_len;
    s_cfg.time_step  = EK_TOTP_TIME_STEP;
    s_cfg.digits     = (uint8_t)EK_TOTP_DIGITS;

    totp_set_time_func(ek_rtc_unix);
    s_last_discipline_unix = 0;
    return true;
}

void totp_app_discipline_rtc(const gnss_fix_t *fix)
{
    if (fix == NULL || !fix->time_valid) {
        return;
    }
    RTC_TimeTypeDef t = {0};
    RTC_DateTypeDef d = {0};
    t.Hours   = fix->hour;
    t.Minutes = fix->minute;
    t.Seconds = fix->second;
    d.Date    = fix->day;
    d.Month   = fix->month;
    d.Year    = (uint8_t)(fix->year - 2000);   /* HAL: years since 2000 */

    HAL_RTC_SetTime(&hrtc, &t, RTC_FORMAT_BIN);
    HAL_RTC_SetDate(&hrtc, &d, RTC_FORMAT_BIN);

    s_last_discipline_unix = totp_get_time();   /* now, post-set */
}

bool totp_app_clock_fresh(void)
{
    if (s_last_discipline_unix == 0) {
        return false;
    }
    uint64_t now = totp_get_time();
    return (now >= s_last_discipline_unix) &&
           (now - s_last_discipline_unix <= EK_CLOCK_MAX_STALENESS_S);
}

uint32_t totp_app_generate(void)
{
    return totp_generate_current(&s_cfg);
}

void totp_app_emit(uint32_t code)
{
    char line[16];
    int  n = snprintf(line, sizeof(line), "CODE %0*lu\n",
                      (int)EK_TOTP_DIGITS, (unsigned long)code);
    if (n <= 0) {
        return;
    }
    /* Assert CODE_VALID strobe (open-drain low = asserted), send the line. */
    HAL_GPIO_WritePin(CODE_VALID_PORT, CODE_VALID_PIN, GPIO_PIN_RESET);
    HAL_UART_Transmit(&hlock, (uint8_t *)line, (uint16_t)n, 100);
    HAL_GPIO_WritePin(CODE_VALID_PORT, CODE_VALID_PIN, GPIO_PIN_SET);
}
