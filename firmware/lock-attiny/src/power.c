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
