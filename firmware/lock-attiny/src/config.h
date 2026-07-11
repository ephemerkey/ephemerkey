/*
 * ephemerkey lock board — programmable configuration (ATtiny1616)
 *
 * A bit-packed config blob, provisioned over I2C with its own HMAC secret
 * (REG_CONFIG), persisted in EEPROM, loaded at boot. Falls back to compile-time
 * defaults when EEPROM is unprogrammed. Chooses the actuator(s), servo lock/
 * unlock positions, drive time, and the solenoid economizer hold.
 */
#ifndef CONFIG_H
#define CONFIG_H

#include <stdint.h>

#define CONFIG_MAGIC     0xE1
#define CONFIG_LEN       9        /* wire + EEPROM size of the packed blob */

/* flags byte */
#define CFG_SERVO1_EN    0x01
#define CFG_SERVO2_EN    0x02
#define CFG_SOLENOID_EN  0x04

/* Packed blob — all uint8_t, so struct layout == wire layout (no padding). */
typedef struct {
    uint8_t magic;       /* CONFIG_MAGIC */
    uint8_t flags;       /* CFG_* */
    uint8_t s1_lock;     /* servo1 positions: 0..255 -> 500..2500 us */
    uint8_t s1_unlock;
    uint8_t s2_lock;     /* servo2 positions */
    uint8_t s2_unlock;
    uint8_t primary_cs;  /* primary drive time, x10 ms (servo hold / sol strike) */
    uint8_t hold_ds;     /* solenoid economizer hold, x100 ms (0 = none) */
    uint8_t hold_duty;   /* solenoid hold PWM duty, 0..255 -> 0..100 % */
} config_t;

void            config_init(void);                  /* load EEPROM or defaults */
const config_t *config_get(void);
void            config_to_blob(uint8_t out[CONFIG_LEN]);

/* Validate a received blob (magic), persist to EEPROM, reload. Returns 1 on ok. */
uint8_t         config_apply_blob(const uint8_t blob[CONFIG_LEN]);

/* Encoding helpers. */
uint16_t        cfg_pos_to_us(uint8_t pos);         /* 0..255 -> 500..2500 us */
uint16_t        cfg_primary_ms(void);               /* primary_cs * 10 */
uint16_t        cfg_hold_ms(void);                  /* hold_ds * 100 */

#endif /* CONFIG_H */
