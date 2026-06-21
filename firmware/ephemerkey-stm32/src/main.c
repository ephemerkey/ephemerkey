/* SPDX-License-Identifier: Apache-2.0 */
/* ephemerkey — GPS-geofenced TOTP generator, STM32U083 application.
 *
 * Flow (see DESIGN.md "Geofence + TOTP Logic"):
 *   wake -> power GNSS -> acquire fix -> parse NMEA -> discipline RTC from
 *   UTC -> gate on fix quality + clock freshness + geofence -> generate &
 *   emit TOTP -> sleep (Stop mode) until motion or duty timer.
 *
 * Peripheral init bodies marked TODO need finalizing against the STM32U083
 * datasheet / a CubeMX-generated reference (clock tree, AF numbers). The
 * application logic below is complete. */

#include "board.h"
#include "ephemerkey_config.h"
#include "gnss.h"
#include "geofence.h"
#include "totp_app.h"

/* ---- HAL peripheral handles (referenced by totp_app.c via extern) -------- */
RTC_HandleTypeDef  hrtc;
UART_HandleTypeDef hgnss;   /* USART1 <-> MAX-M10S */
UART_HandleTypeDef hlock;   /* USART2  -> companion lock */
I2C_HandleTypeDef  hacc;    /* I2C1   <-> LIS3DH */

/* ---- forward declarations ------------------------------------------------- */
static void SystemClock_Config(void);
static void MX_GPIO_Init(void);
static void MX_RTC_Init(void);
static void MX_USART1_GNSS_Init(void);
static void MX_USART2_LOCK_Init(void);
static void MX_I2C1_Init(void);
static void gnss_power(int on);
static int  acquire_fix(uint32_t timeout_s);
static void enter_stop_until_event(void);
void        Error_Handler(void);

/* ---- application ---------------------------------------------------------- */
int main(void)
{
    HAL_Init();
    SystemClock_Config();

    MX_GPIO_Init();
    MX_RTC_Init();
    MX_USART1_GNSS_Init();
    MX_USART2_LOCK_Init();
    MX_I2C1_Init();

    if (!totp_app_init()) {
        Error_Handler();            /* bad/missing secret */
    }
    /* TODO: lis3dh_init(&hacc): configure INT1 wake-on-motion, INT2 tamper. */

    for (;;) {
        gnss_power(1);
        int got = acquire_fix(EK_GNSS_ACQUIRE_TIMEOUT_S);
        const gnss_fix_t *fix = gnss_get_fix();

        bool authorized = false;
        if (got && fix->valid) {
            totp_app_discipline_rtc(fix);

            bool quality_ok = (fix->satellites >= EK_MIN_SATELLITES) &&
                              (fix->hdop > 0.0 && fix->hdop <= EK_MAX_HDOP);
            bool in_fence   = geofence_contains(fix->lat, fix->lon);
            bool clock_ok   = totp_app_clock_fresh();

            authorized = quality_ok && in_fence && clock_ok;
        }

        gnss_power(0);

        if (authorized) {
            uint32_t code = totp_app_generate();
            totp_app_emit(code);
            HAL_GPIO_WritePin(LED_GREEN_PORT, LED_GREEN_PIN, GPIO_PIN_SET);
            HAL_GPIO_WritePin(LED_RED_PORT,   LED_RED_PIN,   GPIO_PIN_RESET);
        } else {
            HAL_GPIO_WritePin(LED_GREEN_PORT, LED_GREEN_PIN, GPIO_PIN_RESET);
            HAL_GPIO_WritePin(LED_RED_PORT,   LED_RED_PIN,   GPIO_PIN_SET);
        }

        HAL_Delay(2000);            /* show status briefly */
        HAL_GPIO_WritePin(LED_GREEN_PORT, LED_GREEN_PIN, GPIO_PIN_RESET);
        HAL_GPIO_WritePin(LED_RED_PORT,   LED_RED_PIN,   GPIO_PIN_RESET);

        enter_stop_until_event();   /* wake on LIS3DH motion or RTC alarm */
    }
}

/* Power/duty-cycle the GNSS module (load switch or module backup mode). */
static void gnss_power(int on)
{
    HAL_GPIO_WritePin(GNSS_EN_PORT, GNSS_EN_PIN, on ? GPIO_PIN_SET : GPIO_PIN_RESET);
    if (on) {
        HAL_GPIO_WritePin(GNSS_RESET_PORT, GNSS_RESET_PIN, GPIO_PIN_SET); /* release reset */
        gnss_reset();
        HAL_Delay(50);
    }
}

/* Poll the GNSS UART until a fix is acquired or the timeout elapses.
 * Returns 1 if at least one complete sentence was parsed. */
static int acquire_fix(uint32_t timeout_s)
{
    uint32_t start = HAL_GetTick();
    int      any   = 0;
    uint8_t  b;

    while ((HAL_GetTick() - start) < timeout_s * 1000U) {
        if (HAL_UART_Receive(&hgnss, &b, 1, 50) == HAL_OK) {
            if (gnss_feed_byte(b)) {
                any = 1;
                const gnss_fix_t *f = gnss_get_fix();
                if (f->valid && f->satellites >= EK_MIN_SATELLITES) {
                    return 1;       /* good enough; stop early */
                }
            }
        }
    }
    return any;
}

/* ---- low power ------------------------------------------------------------ */
static void enter_stop_until_event(void)
{
    /* TODO: arm LIS3DH INT1 (EXTI) and an RTC wakeup timer
     * (EK_SLEEP_INTERVAL_S), then enter Stop 2. Placeholder uses a delay. */
    HAL_Delay(EK_SLEEP_INTERVAL_S * 1000U);
}

/* ---- peripheral init (skeletons) ----------------------------------------- */
static void SystemClock_Config(void)
{
    /* TODO: enable LSE (32.768kHz) for the RTC, select MSI/HSI for the core,
     * and (if USB used) configure the HSI48 + CRS for crystal-less USB.
     * Generate with CubeMX for the STM32U083 and paste here. */
}

static void MX_RTC_Init(void)
{
    __HAL_RCC_RTC_ENABLE();
    hrtc.Instance = RTC;
    hrtc.Init.HourFormat     = RTC_HOURFORMAT_24;
    hrtc.Init.AsynchPrediv   = 127;        /* 32768 / (127+1) / (255+1) = 1 Hz */
    hrtc.Init.SynchPrediv    = 255;
    hrtc.Init.OutPut         = RTC_OUTPUT_DISABLE;
    if (HAL_RTC_Init(&hrtc) != HAL_OK) {
        Error_Handler();
    }
}

static void MX_USART1_GNSS_Init(void)
{
    hgnss.Instance        = GNSS_UART;
    hgnss.Init.BaudRate   = GNSS_BAUD;
    hgnss.Init.WordLength = UART_WORDLENGTH_8B;
    hgnss.Init.StopBits   = UART_STOPBITS_1;
    hgnss.Init.Parity     = UART_PARITY_NONE;
    hgnss.Init.Mode       = UART_MODE_TX_RX;
    hgnss.Init.HwFlowCtl  = UART_HWCONTROL_NONE;
    if (HAL_UART_Init(&hgnss) != HAL_OK) {
        Error_Handler();
    }
}

static void MX_USART2_LOCK_Init(void)
{
    hlock.Instance        = LOCK_UART;
    hlock.Init.BaudRate   = LOCK_BAUD;
    hlock.Init.WordLength = UART_WORDLENGTH_8B;
    hlock.Init.StopBits   = UART_STOPBITS_1;
    hlock.Init.Parity     = UART_PARITY_NONE;
    hlock.Init.Mode       = UART_MODE_TX;
    hlock.Init.HwFlowCtl  = UART_HWCONTROL_NONE;
    if (HAL_UART_Init(&hlock) != HAL_OK) {
        Error_Handler();
    }
}

static void MX_I2C1_Init(void)
{
    hacc.Instance        = ACC_I2C;
    hacc.Init.Timing     = 0x00303D5B;     /* TODO: recompute for actual I2C clk */
    hacc.Init.AddressingMode = I2C_ADDRESSINGMODE_7BIT;
    hacc.Init.DualAddressMode = I2C_DUALADDRESS_DISABLE;
    hacc.Init.GeneralCallMode = I2C_GENERALCALL_DISABLE;
    hacc.Init.NoStretchMode   = I2C_NOSTRETCH_DISABLE;
    if (HAL_I2C_Init(&hacc) != HAL_OK) {
        Error_Handler();
    }
}

static void MX_GPIO_Init(void)
{
    GPIO_InitTypeDef gi = {0};

    __HAL_RCC_GPIOA_CLK_ENABLE();
    __HAL_RCC_GPIOB_CLK_ENABLE();

    /* LEDs + GNSS control outputs (push-pull) */
    HAL_GPIO_WritePin(LED_GREEN_PORT, LED_GREEN_PIN, GPIO_PIN_RESET);
    HAL_GPIO_WritePin(LED_RED_PORT,   LED_RED_PIN,   GPIO_PIN_RESET);
    gi.Mode  = GPIO_MODE_OUTPUT_PP;
    gi.Pull  = GPIO_NOPULL;
    gi.Speed = GPIO_SPEED_FREQ_LOW;
    gi.Pin   = LED_GREEN_PIN | LED_RED_PIN | GNSS_EXTINT_PIN | GNSS_EN_PIN;
    HAL_GPIO_Init(GPIOA, &gi);

    /* GNSS RESET_N + CODE_VALID strobe: open-drain */
    gi.Mode  = GPIO_MODE_OUTPUT_OD;
    gi.Pin   = GNSS_RESET_PIN;
    HAL_GPIO_Init(GNSS_RESET_PORT, &gi);
    gi.Pin   = CODE_VALID_PIN;
    HAL_GPIO_Init(CODE_VALID_PORT, &gi);
    HAL_GPIO_WritePin(CODE_VALID_PORT, CODE_VALID_PIN, GPIO_PIN_SET); /* deasserted */

    /* Button: input pull-up */
    gi.Mode = GPIO_MODE_INPUT;
    gi.Pull = GPIO_PULLUP;
    gi.Pin  = BTN_PIN;
    HAL_GPIO_Init(BTN_PORT, &gi);

    /* Accelerometer INT1/INT2: EXTI rising (wake) */
    gi.Mode = GPIO_MODE_IT_RISING;
    gi.Pull = GPIO_NOPULL;
    gi.Pin  = ACC_INT1_PIN | ACC_INT2_PIN;
    HAL_GPIO_Init(GPIOA, &gi);

    /* USART/I2C AF pins are configured in HAL_*_MspInit (msp.c). */
}

void Error_Handler(void)
{
    __disable_irq();
    for (;;) {
        /* fast-blink red to signal a fatal init error */
    }
}

#ifdef USE_FULL_ASSERT
void assert_failed(uint8_t *file, uint32_t line)
{
    (void)file; (void)line;
}
#endif
