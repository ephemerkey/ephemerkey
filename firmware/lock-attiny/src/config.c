/*
 * ephemerkey lock board — programmable configuration (ATtiny1616)
 */
#include <avr/eeprom.h>
#include <string.h>
#include "config.h"

/* EEPROM map: 0..3 = nonce counter (nonce.c); 16.. = config blob. */
#define EE_CONFIG_ADDR   ((void *)16)

static config_t s_cfg;

/* Compile-time defaults: servo1 only, 1.0 ms lock / 2.0 ms unlock, 600 ms drive,
 * 200 ms / 50 % solenoid hold (unused unless solenoid is enabled). */
static const config_t k_defaults = {
    .magic      = CONFIG_MAGIC,
    .flags      = CFG_SERVO1_EN,
    .s1_lock    = 64,    /* ~1000 us */
    .s1_unlock  = 191,   /* ~2000 us */
    .s2_lock    = 64,
    .s2_unlock  = 191,
    .primary_cs = 60,    /* 600 ms */
    .hold_ds    = 2,     /* 200 ms */
    .hold_duty  = 128,   /* ~50 %   */
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

uint16_t cfg_primary_ms(void) { return (uint16_t)s_cfg.primary_cs * 10u; }
uint16_t cfg_hold_ms(void)    { return (uint16_t)s_cfg.hold_ds * 100u; }
