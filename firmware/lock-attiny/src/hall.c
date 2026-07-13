/*
 * ephemerkey lock board — hall sensor read (ATtiny1616)
 */
#include <avr/io.h>
#include <util/delay.h>
#include "hall.h"
#include "config.h"     /* SENSOR_SRC_* */

#define HALL_PWR_bm    PIN4_bm    /* PORTA: PA4 */
#define HALL_J6_bm     PIN7_bm    /* PORTA: PA7 (J6 sensor) */
#define HALL_J7_bm     PIN3_bm    /* PORTB: PB3 (J7 sensor) */

void hall_init(void)
{
    PORTA.OUTCLR = HALL_PWR_bm;
    PORTA.DIRSET = HALL_PWR_bm;    /* HALL_PWR output, off */
    PORTA.DIRCLR = HALL_J6_bm;     /* sensor inputs */
    PORTB.DIRCLR = HALL_J7_bm;
}

void hall_power(uint8_t on)
{
    if (on) PORTA.OUTSET = HALL_PWR_bm;
    else    PORTA.OUTCLR = HALL_PWR_bm;
}

uint8_t hall_sample(void)
{
    /* Open-drain outputs pulled to HALL_PWR: magnet present -> LOW. Assumes the
     * sensors are already powered + settled (see hall_power / hall_read).
     * TODO: confirm polarity against the actual hall part; flip if needed. */
    uint8_t r = 0;
    if (!(PORTA.IN & HALL_J6_bm)) r |= HALL_J6;
    if (!(PORTB.IN & HALL_J7_bm)) r |= HALL_J7;
    return r;
}

uint8_t hall_read(void)
{
    hall_power(1);
    _delay_ms(1);                  /* settle: sensor turn-on + RC on OUT nets */
    uint8_t r = hall_sample();
    hall_power(0);                 /* back to ~0 uA */
    return r;
}

uint8_t hall_src(uint8_t src, uint8_t raw)
{
    switch (src) {
    case SENSOR_SRC_J6: return raw & HALL_J6;
    case SENSOR_SRC_J7: return raw & HALL_J7;
    default:            return 0;   /* SENSOR_SRC_OFF */
    }
}
