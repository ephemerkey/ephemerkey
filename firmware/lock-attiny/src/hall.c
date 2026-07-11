/*
 * ephemerkey lock board — hall sensor read (ATtiny1616)
 */
#include <avr/io.h>
#include <util/delay.h>
#include "hall.h"

#define HALL_PWR_bm    PIN4_bm    /* PORTA: PA4 */
#define HALL_DOOR_bm   PIN7_bm    /* PORTA: PA7 */
#define HALL_BOLT_bm   PIN3_bm    /* PORTB: PB3 */

void hall_init(void)
{
    PORTA.OUTCLR = HALL_PWR_bm;
    PORTA.DIRSET = HALL_PWR_bm;    /* HALL_PWR output, off */
    PORTA.DIRCLR = HALL_DOOR_bm;   /* sensor inputs */
    PORTB.DIRCLR = HALL_BOLT_bm;
}

uint8_t hall_read(void)
{
    PORTA.OUTSET = HALL_PWR_bm;    /* power the sensors */
    _delay_ms(1);                  /* settle: sensor turn-on + RC on OUT nets */

    /* Open-drain outputs pulled to HALL_PWR: magnet present -> LOW.
     * TODO: confirm polarity against the actual hall part; flip if needed. */
    uint8_t door_present = (PORTA.IN & HALL_DOOR_bm) ? 0 : 1;
    uint8_t bolt_present = (PORTB.IN & HALL_BOLT_bm) ? 0 : 1;

    PORTA.OUTCLR = HALL_PWR_bm;    /* back to ~0 uA */

    uint8_t r = 0;
    if (door_present) r |= HALL_DOOR_CLOSED;
    if (bolt_present) r |= HALL_BOLT_LOCKED;
    return r;
}
