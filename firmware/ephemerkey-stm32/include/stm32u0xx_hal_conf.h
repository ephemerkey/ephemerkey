/* SPDX-License-Identifier: Apache-2.0 */
/* Minimal STM32CubeU0 HAL configuration for ephemerkey.
 * Enables only the modules this firmware uses. */

#ifndef STM32U0xx_HAL_CONF_H
#define STM32U0xx_HAL_CONF_H

#ifdef __cplusplus
extern "C" {
#endif

/* ---- Module selection ----------------------------------------------------- */
#define HAL_MODULE_ENABLED
#define HAL_CORTEX_MODULE_ENABLED
#define HAL_RCC_MODULE_ENABLED
#define HAL_FLASH_MODULE_ENABLED
#define HAL_GPIO_MODULE_ENABLED
#define HAL_PWR_MODULE_ENABLED
#define HAL_DMA_MODULE_ENABLED
#define HAL_RTC_MODULE_ENABLED
#define HAL_UART_MODULE_ENABLED
#define HAL_I2C_MODULE_ENABLED
#define HAL_EXTI_MODULE_ENABLED

/* ---- Oscillator values ---------------------------------------------------- */
#if !defined (HSE_VALUE)
  #define HSE_VALUE              16000000UL
#endif
#if !defined (HSE_STARTUP_TIMEOUT)
  #define HSE_STARTUP_TIMEOUT    100UL
#endif
#if !defined (MSI_VALUE)
  #define MSI_VALUE              4000000UL
#endif
#if !defined (HSI_VALUE)
  #define HSI_VALUE              16000000UL
#endif
#if !defined (HSI48_VALUE)
  #define HSI48_VALUE            48000000UL
#endif
#if !defined (LSI_VALUE)
  #define LSI_VALUE              32000UL
#endif
#if !defined (LSE_VALUE)
  #define LSE_VALUE              32768UL          /* RTC time base */
#endif
#if !defined (LSE_STARTUP_TIMEOUT)
  #define LSE_STARTUP_TIMEOUT    5000UL
#endif

/* ---- System config -------------------------------------------------------- */
#define VDD_VALUE                     3300UL
#define TICK_INT_PRIORITY             0UL
#define USE_RTOS                      0U
#define PREFETCH_ENABLE               1U
#define INSTRUCTION_CACHE_ENABLE      1U

#define USE_HAL_UART_REGISTER_CALLBACKS  0U
#define USE_HAL_I2C_REGISTER_CALLBACKS   0U
#define USE_HAL_RTC_REGISTER_CALLBACKS   0U

/* ---- assert --------------------------------------------------------------- */
/* #define USE_FULL_ASSERT  1U */
#ifdef USE_FULL_ASSERT
  #define assert_param(expr) ((expr) ? (void)0U : assert_failed((uint8_t *)__FILE__, __LINE__))
  void assert_failed(uint8_t *file, uint32_t line);
#else
  #define assert_param(expr) ((void)0U)
#endif

/* ---- Module headers ------------------------------------------------------- */
#ifdef HAL_RCC_MODULE_ENABLED
  #include "stm32u0xx_hal_rcc.h"
#endif
#ifdef HAL_GPIO_MODULE_ENABLED
  #include "stm32u0xx_hal_gpio.h"
#endif
#ifdef HAL_DMA_MODULE_ENABLED
  #include "stm32u0xx_hal_dma.h"
#endif
#ifdef HAL_CORTEX_MODULE_ENABLED
  #include "stm32u0xx_hal_cortex.h"
#endif
#ifdef HAL_FLASH_MODULE_ENABLED
  #include "stm32u0xx_hal_flash.h"
#endif
#ifdef HAL_PWR_MODULE_ENABLED
  #include "stm32u0xx_hal_pwr.h"
#endif
#ifdef HAL_RTC_MODULE_ENABLED
  #include "stm32u0xx_hal_rtc.h"
#endif
#ifdef HAL_UART_MODULE_ENABLED
  #include "stm32u0xx_hal_uart.h"
#endif
#ifdef HAL_I2C_MODULE_ENABLED
  #include "stm32u0xx_hal_i2c.h"
#endif
#ifdef HAL_EXTI_MODULE_ENABLED
  #include "stm32u0xx_hal_exti.h"
#endif

#ifdef __cplusplus
}
#endif

#endif /* STM32U0xx_HAL_CONF_H */
