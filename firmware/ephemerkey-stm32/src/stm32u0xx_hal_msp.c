/* SPDX-License-Identifier: Apache-2.0 */
/* HAL MSP: low-level peripheral pin/clock setup (called by HAL_*_Init). */

#include "board.h"

void HAL_UART_MspInit(UART_HandleTypeDef *huart)
{
    GPIO_InitTypeDef gi = {0};

    if (huart->Instance == GNSS_UART) {            /* USART1: PA9 TX, PA10 RX */
        __HAL_RCC_USART1_CLK_ENABLE();
        __HAL_RCC_GPIOA_CLK_ENABLE();
        gi.Pin       = GNSS_UART_TX_PIN | GNSS_UART_RX_PIN;
        gi.Mode      = GPIO_MODE_AF_PP;
        gi.Pull      = GPIO_PULLUP;
        gi.Speed     = GPIO_SPEED_FREQ_LOW;
        gi.Alternate = GNSS_UART_AF;
        HAL_GPIO_Init(GNSS_UART_PORT, &gi);
    } else if (huart->Instance == LOCK_UART) {     /* USART2: PB0 TX */
        __HAL_RCC_USART2_CLK_ENABLE();
        __HAL_RCC_GPIOB_CLK_ENABLE();
        gi.Pin       = LOCK_UART_TX_PIN;
        gi.Mode      = GPIO_MODE_AF_PP;
        gi.Pull      = GPIO_NOPULL;
        gi.Speed     = GPIO_SPEED_FREQ_LOW;
        gi.Alternate = GPIO_AF7_USART2;            /* TODO: verify AF for PB0 */
        HAL_GPIO_Init(LOCK_UART_PORT, &gi);
    }
}

void HAL_I2C_MspInit(I2C_HandleTypeDef *hi2c)
{
    GPIO_InitTypeDef gi = {0};

    if (hi2c->Instance == ACC_I2C) {               /* I2C1: PB6 SCL, PB7 SDA */
        __HAL_RCC_GPIOB_CLK_ENABLE();
        gi.Pin       = ACC_I2C_SCL_PIN | ACC_I2C_SDA_PIN;
        gi.Mode      = GPIO_MODE_AF_OD;
        gi.Pull      = GPIO_PULLUP;                /* plus external 4.7k */
        gi.Speed     = GPIO_SPEED_FREQ_LOW;
        gi.Alternate = ACC_I2C_AF;
        HAL_GPIO_Init(ACC_I2C_PORT, &gi);
        __HAL_RCC_I2C1_CLK_ENABLE();
    }
}

void HAL_RTC_MspInit(RTC_HandleTypeDef *hrtc)
{
    (void)hrtc;
    /* RTC clock source (LSE) is selected in SystemClock_Config /
     * RCC_OscConfig + RCC_PeriphCLKConfig. Enable the RTC APB clock here. */
    __HAL_RCC_RTCAPB_CLK_ENABLE();
}
