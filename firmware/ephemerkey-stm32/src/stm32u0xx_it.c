/* SPDX-License-Identifier: Apache-2.0 */
/* Interrupt handlers. Default/fault vectors come from the startup file as weak
 * Default_Handler; only the ones we use are defined here. */

#include "board.h"

/* HAL time base. */
void SysTick_Handler(void)
{
    HAL_IncTick();
}

/* Accelerometer INT1 (PA2) / INT2 (PA3) -> EXTI lines 2,3 (grouped on M0+). */
void EXTI2_3_IRQHandler(void)
{
    HAL_GPIO_EXTI_IRQHandler(ACC_INT1_PIN);
    HAL_GPIO_EXTI_IRQHandler(ACC_INT2_PIN);
}

/* Rising-edge callback (HAL U0). Wake from Stop happens automatically; this is
 * where motion vs tamper policy is applied. */
void HAL_GPIO_EXTI_Rising_Callback(uint16_t pin)
{
    if (pin == ACC_INT1_PIN) {
        /* motion: just wake — main loop re-runs the GNSS/TOTP pipeline */
    } else if (pin == ACC_INT2_PIN) {
        /* tamper: TODO apply policy (e.g. zeroize secret) per DESIGN.md */
    }
}
