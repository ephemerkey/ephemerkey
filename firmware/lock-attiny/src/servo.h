/*
 * ephemerkey lock board — dual servo actuator driver (ATtiny1616)
 *
 * Two servos share one VSERVO supply and one TCA0 frame (50 Hz, 1-2 ms):
 *   servo1  SERVO_SIG  (PB2) — hardware TCA0/WO2 (CMP2)
 *   servo2  SERVO_SIG2 (PB4) — software pulse, phase-locked to TCA0
 *                             (OVF raises it, CMP1 match lowers it)
 *
 * Both run on BATTERY voltage: the MT3608 boost is kept OFF and VSOL passes
 * ~Vbat-0.3 V through its L1+D2 path. See servo.c for pin/interlock rationale.
 */
#ifndef SERVO_H
#define SERVO_H

#include <stdint.h>
#include <stdbool.h>

#define SERVO_MID_US   1500u   /* center — the power-on default pulse (not a clamp) */

/* Put boost + servo-power pins in their safe (off) state and configure the
 * TCA0 PWM generator for both channels. Does NOT power or move the servos. */
void servo_init(void);

/* Set commanded pulse width per channel (clamped); glitch-free, takes effect
 * on the next frame. */
void servo1_set_us(uint16_t pulse_us);   /* PB2, hardware WO2 */
void servo2_set_us(uint16_t pulse_us);   /* PB4, software     */

/* Start / stop the shared PWM frame (both channels). When stopped both signal
 * lines idle low (R16 / R25 pulldowns). */
void servo_pwm_start(void);
void servo_pwm_stop(void);

/* Enable / disable the high-side servo supply (PA2 -> Q3), shared by both
 * connectors. Requires boost off and BOOST_VSEL low (set by servo_init). */
void servo_power(bool on);

#endif /* SERVO_H */
