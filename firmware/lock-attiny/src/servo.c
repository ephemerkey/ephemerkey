/*
 * ephemerkey lock board — dual servo actuator driver (ATtiny1616)
 *
 * Pin map (from hardware/lock, MCU/PWR/DRV sheet notes):
 *   PB2  SERVO_SIG     -> TCA0/WO2 -> R15(1k) -> J5.1   (servo1 signal, HW PWM)
 *   PB4  SERVO_SIG2    -> R24(1k) -> J8.1               (servo2 signal, SW PWM)
 *   PA2  SERVO_PWR_EN  -> R21 -> ENNODE -> Q4 -> Q3(P-FET) high-side VSERVO
 *   PA6  SOL_BOOST_EN  -> MT3608 EN                     (KEEP LOW: boost unused)
 *   PA1  BOOST_VSEL    -> boost 6/12V select + servo interlock (Q5) (KEEP LOW)
 *
 * Power rationale — servos on BATTERY voltage, not the boost:
 *   VSERVO_SRC is strap-selected: R13 -> VSOL (fitted) or R14 -> VCC/VSYS (DNP).
 *   With the MT3608 DISABLED, its L1+D2 Schottky path passes Vin to VSOL, so
 *   VSOL settles at ~Vbat-0.3 V. Either strap gives ~Vbat as long as the boost
 *   stays off. BOOST_VSEL must also be low: Q5 interlocks servo power OFF
 *   whenever it is high (12 V mode). Both boost pins live in power.c, which
 *   power_init() drives low before any servo_power() call.
 *
 * PWM: TCA0 single-slope, 50 Hz. WO2 (PB2) is the hardware servo1 output.
 *   PB4 (WO4) is only a HW output in SPLIT mode, which is 8-bit — too coarse
 *   for a 20 ms frame. So servo2 is generated in software, phase-locked to the
 *   same TCA0 frame: the OVF ISR raises PB4 at frame start, and the CMP1 match
 *   ISR lowers it after the pulse width. CMP1 maps to WO1/PB1(SDA), but only
 *   its INTERRUPT is enabled — the WO1 output stays off, so I2C is untouched.
 */

#include <avr/io.h>
#include <avr/interrupt.h>
#include "servo.h"

/* --- pin bitmasks (mind the port!) --- */
#define SERVO_SIG_bm    PIN2_bm   /* PORTB: PB2 (servo1, WO2) */
#define SERVO_SIG2_bm   PIN4_bm   /* PORTB: PB4 (servo2, SW)  */
#define PWR_EN_bm       PIN2_bm   /* PORTA: PA2 */
/* Boost pins (PA6/PA1) are owned by power.c; power_init() must run first so the
 * Q5 interlock is clear before servo_power() is used. */

/* --- PWM timing --- */
/* Servo frame rate ("PWM frequency"). 50 Hz suits analog servos; digital
 * servos accept up to ~300 Hz and hold stiffer/quieter there. Pulse widths are
 * unaffected (US_TO_CMP is frame-rate independent); just keep the frame period
 * longer than SERVO_MAX_US (e.g. <=300 Hz for a 2.4 ms max pulse). */
#define SERVO_FRAME_HZ  50u
#define TCA_PRESCALE    8u
#define F_TCA           (F_CPU / TCA_PRESCALE)
#define PWM_PER         ((uint16_t)(F_TCA / SERVO_FRAME_HZ - 1u))
#define US_TO_CMP(us)   ((uint16_t)(((uint32_t)(us) * F_TCA) / 1000000UL))

static uint16_t clamp_cmp(uint16_t pulse_us)
{
    if (pulse_us < SERVO_MIN_US) pulse_us = SERVO_MIN_US;
    if (pulse_us > SERVO_MAX_US) pulse_us = SERVO_MAX_US;
    return US_TO_CMP(pulse_us);
}

/* servo2 software pulse on PB4, phase-locked to the TCA0 frame. */
ISR(TCA0_OVF_vect)
{
    TCA0.SINGLE.INTFLAGS = TCA_SINGLE_OVF_bm;
    PORTB.OUTSET = SERVO_SIG2_bm;    /* rising edge at frame start */
}

ISR(TCA0_CMP1_vect)
{
    TCA0.SINGLE.INTFLAGS = TCA_SINGLE_CMP1_bm;
    PORTB.OUTCLR = SERVO_SIG2_bm;    /* falling edge after pulse width */
}

void servo_init(void)
{
    /* Servo high-side supply -> off. (Boost pins handled by power_init.) */
    PORTA.OUTCLR = PWR_EN_bm;
    PORTA.DIRSET = PWR_EN_bm;

    /* Both servo signal pins low + output. */
    PORTB.OUTCLR = SERVO_SIG_bm | SERVO_SIG2_bm;
    PORTB.DIRSET = SERVO_SIG_bm | SERVO_SIG2_bm;

    /* TCA0 single-slope PWM, 50 Hz. WO2 output not yet enabled. */
    TCA0.SINGLE.CTRLB = TCA_SINGLE_WGMODE_SINGLESLOPE_gc;
    TCA0.SINGLE.PER   = PWM_PER;
    TCA0.SINGLE.CMP2  = US_TO_CMP(SERVO_MID_US);   /* servo1 (HW WO2)     */
    TCA0.SINGLE.CMP1  = US_TO_CMP(SERVO_MID_US);   /* servo2 (SW timing)  */
    TCA0.SINGLE.CTRLA = TCA_SINGLE_CLKSEL_DIV8_gc; /* configured, disabled */
}

void servo1_set_us(uint16_t pulse_us)
{
    TCA0.SINGLE.CMP2BUF = clamp_cmp(pulse_us);     /* buffered update */
}

void servo2_set_us(uint16_t pulse_us)
{
    TCA0.SINGLE.CMP1BUF = clamp_cmp(pulse_us);     /* buffered update */
}

void servo_pwm_start(void)
{
    TCA0.SINGLE.CTRLB  |= TCA_SINGLE_CMP2EN_bm;    /* drive WO2 (PB2), servo1 */
    TCA0.SINGLE.INTCTRL = TCA_SINGLE_OVF_bm | TCA_SINGLE_CMP1_bm; /* servo2 SW */
    TCA0.SINGLE.CTRLA  |= TCA_SINGLE_ENABLE_bm;    /* run timer */
}

void servo_pwm_stop(void)
{
    TCA0.SINGLE.CTRLA  &= ~TCA_SINGLE_ENABLE_bm;
    TCA0.SINGLE.CTRLB  &= ~TCA_SINGLE_CMP2EN_bm;
    TCA0.SINGLE.INTCTRL = 0;
    PORTB.OUTCLR = SERVO_SIG_bm | SERVO_SIG2_bm;   /* idle low (pulldowns hold) */
}

void servo_power(bool on)
{
    if (on) PORTA.OUTSET = PWR_EN_bm;
    else    PORTA.OUTCLR = PWR_EN_bm;
}
