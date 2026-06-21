/* SPDX-License-Identifier: Apache-2.0 */
/* ephemerkey board pin map — STM32U083KCU6 (UFQFPN-32)
 * Mirrors DESIGN.md "Pin Budget". Verify AF numbers against the
 * STM32U083 datasheet for the UFQFPN-32 package before relying on them. */

#ifndef EPHEMERKEY_BOARD_H
#define EPHEMERKEY_BOARD_H

#include "stm32u0xx_hal.h"

/* ---- Status LEDs ---------------------------------------------------------- */
#define LED_GREEN_PORT      GPIOA
#define LED_GREEN_PIN       GPIO_PIN_6   /* in-fence / code valid */
#define LED_RED_PORT        GPIOA
#define LED_RED_PIN         GPIO_PIN_7   /* out-of-fence / fault */

/* ---- User button ---------------------------------------------------------- */
#define BTN_PORT            GPIOA
#define BTN_PIN             GPIO_PIN_5   /* provision / show-code, internal pull-up */

/* ---- GNSS (MAX-M10S) ------------------------------------------------------ */
#define GNSS_UART           USART1       /* PA9 TX -> RXD, PA10 RX <- TXD */
#define GNSS_UART_TX_PIN    GPIO_PIN_9
#define GNSS_UART_RX_PIN    GPIO_PIN_10
#define GNSS_UART_PORT      GPIOA
#define GNSS_UART_AF        GPIO_AF1_USART1
#define GNSS_BAUD           9600U

#define GNSS_PPS_PORT       GPIOA        /* 1PPS -> TIM2_CH1 input capture */
#define GNSS_PPS_PIN        GPIO_PIN_0
#define GNSS_EXTINT_PORT    GPIOA        /* MCU -> GNSS EXTINT (wake/time-mark) */
#define GNSS_EXTINT_PIN     GPIO_PIN_1
#define GNSS_RESET_PORT     GPIOA        /* MCU -> GNSS RESET_N (open-drain) */
#define GNSS_RESET_PIN      GPIO_PIN_4
#define GNSS_EN_PORT        GPIOA        /* optional load-switch power gate */
#define GNSS_EN_PIN         GPIO_PIN_8

/* ---- Accelerometer (LIS3DH) ---------------------------------------------- */
#define ACC_I2C             I2C1         /* PB6 SCL, PB7 SDA */
#define ACC_I2C_SCL_PIN     GPIO_PIN_6
#define ACC_I2C_SDA_PIN     GPIO_PIN_7
#define ACC_I2C_PORT        GPIOB
#define ACC_I2C_AF          GPIO_AF6_I2C1
#define ACC_I2C_ADDR        (0x18U << 1) /* SA0=0 -> 0x18; SA0=1 -> 0x19 */
#define ACC_INT1_PORT       GPIOA        /* wake-on-motion */
#define ACC_INT1_PIN        GPIO_PIN_2
#define ACC_INT2_PORT       GPIOA        /* tamper / free-fall */
#define ACC_INT2_PIN        GPIO_PIN_3

/* ---- Companion lock interface -------------------------------------------- */
#define LOCK_UART           USART2       /* PB0 TX (code line out) */
#define LOCK_UART_TX_PIN    GPIO_PIN_0
#define LOCK_UART_PORT      GPIOB
#define LOCK_BAUD           9600U
#define CODE_VALID_PORT     GPIOB        /* open-drain strobe */
#define CODE_VALID_PIN      GPIO_PIN_1

#endif /* EPHEMERKEY_BOARD_H */
