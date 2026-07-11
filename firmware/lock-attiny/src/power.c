/*
 * ephemerkey lock board — boost-rail power control (ATtiny1616)
 * See power.h for the rail/interlock rationale.
 */
#include <avr/io.h>
#include "power.h"

#define BOOST_EN_bm     PIN6_bm   /* PORTA: PA6 (MT3608 EN)   */
#define BOOST_VSEL_bm   PIN1_bm   /* PORTA: PA1 (6/12V + Q5)  */

void power_init(void)
{
    PORTA.OUTCLR = BOOST_EN_bm | BOOST_VSEL_bm;   /* boost off, 6V, interlock clear */
    PORTA.DIRSET = BOOST_EN_bm | BOOST_VSEL_bm;
}

void boost_12v_enable(void)
{
    PORTA.OUTSET = BOOST_VSEL_bm;   /* 12V FB select first (also locks out servo) */
    PORTA.OUTSET = BOOST_EN_bm;     /* enable the converter */
}

void boost_disable(void)
{
    PORTA.OUTCLR = BOOST_EN_bm;     /* converter off */
    PORTA.OUTCLR = BOOST_VSEL_bm;   /* back to 6V select / clear interlock */
}

void boost_servo_enable(void)
{
    /* 6 V boosted servo: enable the converter with BOOST_VSEL LOW (6 V FB
     * select). VSEL low keeps the Q5 interlock CLEAR, so servo power can be
     * applied — VSOL rises to ~6 V and feeds the servo. (VSEL high / 12 V would
     * fire the interlock and cut servo power, so it is never used for servos.)
     * Requires the servo strapped to VSOL (R13) and a 6 V-rated servo — behind
     * CFG_SERVO_BOOST, off by default. */
    PORTA.OUTCLR = BOOST_VSEL_bm;   /* 6 V select, interlock clear */
    PORTA.OUTSET = BOOST_EN_bm;     /* enable the converter */
}
