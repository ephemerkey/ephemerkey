/*
 * ephemerkey lock board — programmable configuration (ATtiny1616)
 */
#include <avr/eeprom.h>
#include <string.h>
#include "config.h"

_Static_assert(sizeof(config_t) == CONFIG_LEN, "config_t packing != CONFIG_LEN");

/* EEPROM map: 0..3 = nonce counter (nonce.c); 16.. = config blob. */
#define EE_CONFIG_ADDR   ((void *)16)

static config_t s_cfg;

/* Compile-time defaults: single servo1 phase each way (matches the old default),
 * 1.0 ms lock / 2.0 ms unlock, 600 ms drive, no early-off. Solenoid economizer
 * knobs are set but unused until a step drives the solenoid. */
static const config_t k_defaults = {
    .magic      = CONFIG_MAGIC,
    .flags      = 0,
    .strike_cs  = 5,     /* 50 ms solenoid strike (12 V economizer) */
    .hold_duty  = 128,   /* ~50 % */
    .sensor_map = (SENSOR_SRC_J6 << SENSOR_DOOR_SHIFT)   /* door <- J6 */
                | (SENSOR_SRC_J7 << SENSOR_BOLT_SHIFT),  /* bolt <- J7 */
    .seq_unlock = { { STEP_SERVO1, 191, 0, 6, EOFF_NONE } },  /* ~2000 us, 600 ms */
    .seq_lock   = { { STEP_SERVO1,  64, 0, 6, EOFF_NONE } },  /* ~1000 us, 600 ms */
};

void config_init(void)
{
    eeprom_read_block(&s_cfg, EE_CONFIG_ADDR, CONFIG_LEN);
    if (s_cfg.magic != CONFIG_MAGIC)
        memcpy(&s_cfg, &k_defaults, CONFIG_LEN);
}

const config_t *config_get(void) { return &s_cfg; }

void config_to_blob(uint8_t out[CONFIG_LEN]) { memcpy(out, &s_cfg, CONFIG_LEN); }

uint8_t config_apply_blob(const uint8_t blob[CONFIG_LEN])
{
    if (blob[0] != CONFIG_MAGIC) return 0;       /* magic guards a bad write */
    memcpy(&s_cfg, blob, CONFIG_LEN);
    eeprom_update_block(&s_cfg, EE_CONFIG_ADDR, CONFIG_LEN);
    return 1;
}

uint16_t cfg_pos_to_us(uint8_t pos)
{
    return (uint16_t)(500u + ((uint32_t)pos * 2000u) / 255u);
}

uint16_t cfg_strike_ms(void) { return (uint16_t)s_cfg.strike_cs * 10u; }
uint8_t cfg_door_src(void)   { return (s_cfg.sensor_map >> SENSOR_DOOR_SHIFT) & SENSOR_SRC_MASK; }
uint8_t cfg_bolt_src(void)   { return (s_cfg.sensor_map >> SENSOR_BOLT_SHIFT) & SENSOR_SRC_MASK; }

uint8_t cfg_any_servo(void)
{
    for (uint8_t i = 0; i < SEQ_STEPS; i++) {
        if (s_cfg.seq_unlock[i].act & (STEP_SERVO1 | STEP_SERVO2)) return 1;
        if (s_cfg.seq_lock[i].act   & (STEP_SERVO1 | STEP_SERVO2)) return 1;
    }
    return 0;
}
