/*
 * ephemerkey lock board — 12 V solenoid driver (ATtiny1616)
 * See solenoid.h. Non-blocking primitives; the actuator state machine sequences
 * strike/hold/release with its own timer.
 *
 * Hold PWM is TCD0 on WOB (= PA5, fixed mux). ~31 kHz (inaudible). Duty is
 * configurable; WOB is high in [CMPBSET, CMPBCLR], so on-time = TOP - CMPBSET.
 * TCD runs CPU-free, so I2C is undisturbed during the hold.
 *
 * TCD0 quirks: FAULTCTRL (output-enable) is CCP-protected and only writable
 * while disabled; CTRLA.ENABLE only when STATUS.ENRDY is set. TCD stays disabled
 * (PA5 = GPIO) for the DC strike/drain phases, enabled only for the hold.
 */
#include <avr/io.h>
#include <avr/cpufunc.h>          /* _PROTECTED_WRITE */
#include "solenoid.h"

#define SOL_PWM_bm      PIN5_bm    /* PORTA: PA5 = TCD0 WOB (Q1 gate) */

/* TCD0 clocked from the internal 20 MHz osc: 20e6 / (639+1) = 31.25 kHz. */
#define SOL_TCD_TOP     639u       /* CMPBCLR: period (TOP) */

void sol_init(void)
{
    PORTA.OUTCLR = SOL_PWM_bm;     /* GPIO output, low (Q1 off) */
    PORTA.DIRSET = SOL_PWM_bm;

    TCD0.CTRLB   = TCD_WGMODE_ONERAMP_gc;
    TCD0.CMPASET = 0;              /* WOA unused (PA4 = HALL_PWR) */
    TCD0.CMPACLR = 0;
    TCD0.CMPBCLR = SOL_TCD_TOP;    /* period; CMPBSET set per-hold */
}

void sol_on(void)  { PORTA.OUTSET = SOL_PWM_bm; }   /* DC (TCD disabled) */
void sol_off(void) { PORTA.OUTCLR = SOL_PWM_bm; }

/* BOUNDED wait for TCD's enable-sync (it has its own 20 MHz clock domain).
 * Normally ready in a few cycles; the bound (~100 ms worst case) guarantees a
 * stuck sync can never hang the main loop with an actuator energized — on
 * timeout we proceed and let the FAULTCTRL/GPIO cleanup take effect anyway. */
static void tcd_wait_enrdy(void)
{
    uint16_t t = 0;
    while (!(TCD0.STATUS & TCD_ENRDY_bm))
        if (++t == 0) break;
}

void sol_hold_start(uint8_t duty)
{
    /* on-time = TOP - CMPBSET, so CMPBSET = TOP*(1 - duty/255). */
    TCD0.CMPBSET = (uint16_t)(SOL_TCD_TOP - ((uint32_t)SOL_TCD_TOP * duty) / 255u);
    _PROTECTED_WRITE(TCD0.FAULTCTRL, TCD_CMPBEN_bm);   /* connect WOB -> PA5 */
    tcd_wait_enrdy();
    TCD0.CTRLA = TCD_CLKSEL_20MHZ_gc | TCD_CNTPRES_DIV1_gc | TCD_ENABLE_bm;
}

void sol_hold_stop(void)
{
    tcd_wait_enrdy();
    TCD0.CTRLA = TCD_CLKSEL_20MHZ_gc | TCD_CNTPRES_DIV1_gc;   /* disable */
    _PROTECTED_WRITE(TCD0.FAULTCTRL, 0);               /* release PA5 -> GPIO */
    PORTA.OUTSET = SOL_PWM_bm;                          /* GPIO high (drain) */
}
