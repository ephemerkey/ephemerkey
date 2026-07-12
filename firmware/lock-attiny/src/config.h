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

#define CONFIG_MAGIC     0xE3       /* bump on layout change */
#define CONFIG_LEN       11       /* wire + EEPROM size of the packed blob */

/* flags byte */
#define CFG_SERVO1_EN    0x01
#define CFG_SERVO2_EN    0x02
#define CFG_SOLENOID_EN  0x04
/* 6 V boosted servo: drive the servo phase from the boost rail at 6 V
 * (SOL_BOOST_EN on, BOOST_VSEL low = interlock clear) instead of Vbat, for a
 * 6 V servo. Requires the servo strapped to VSOL (R13, the default strap). Off by default — do NOT
 * set it unless the board is wired for a boosted servo. */
#define CFG_SERVO_BOOST  0x08
/* Door-open early-off: during a solenoid hold, if the "door closed" hall sensor
 * loses its magnet (door opened), end the hold early after DOOR_OFF_DELAY_MS and
 * go to drain — stop energizing the coil once the door is actually open. */
#define CFG_DOOR_EARLYOFF   0x10

/* sensor_map: each STATUS role picks its source sensor, so either physical port
 * (J6 or J7) can be the "door" or the "bolt" indicator (or a role can be off).
 *   bits 0-1: DOOR_CLOSED source   bits 2-3: BOLT_LOCKED source
 * The door-open early-off uses the DOOR_CLOSED source. */
#define SENSOR_SRC_J6    0u   /* HALL_DOOR pin, PA7 */
#define SENSOR_SRC_J7    1u   /* HALL_BOLT pin, PB3 */
#define SENSOR_SRC_OFF   2u   /* role disabled -> bit reads 0 */
#define SENSOR_DOOR_SHIFT 0
#define SENSOR_BOLT_SHIFT 2
#define SENSOR_SRC_MASK  0x03u

/* Packed blob — all uint8_t, so struct layout == wire layout (no padding). */
typedef struct {
    uint8_t magic;       /* CONFIG_MAGIC */
    uint8_t flags;       /* CFG_* */
    uint8_t s1_lock;     /* servo1 positions: 0..255 -> 500..2500 us */
    uint8_t s1_unlock;
    uint8_t s2_lock;     /* servo2 positions */
    uint8_t s2_unlock;
    uint8_t servo_cs;    /* servo full-power drive time, x10 ms */
    uint8_t strike_cs;   /* solenoid strike (full pull-in) time, x10 ms */
    uint8_t hold_ds;     /* solenoid economizer hold, x100 ms (0 = none) */
    uint8_t hold_duty;   /* solenoid hold PWM duty, 0..255 -> 0..100 % */
    uint8_t sensor_map;  /* door/bolt role -> J6/J7/off (see SENSOR_*) */
} config_t;

void            config_init(void);                  /* load EEPROM or defaults */
const config_t *config_get(void);
void            config_to_blob(uint8_t out[CONFIG_LEN]);

/* Validate a received blob (magic), persist to EEPROM, reload. Returns 1 on ok. */
uint8_t         config_apply_blob(const uint8_t blob[CONFIG_LEN]);

/* Encoding helpers. */
uint16_t        cfg_pos_to_us(uint8_t pos);         /* 0..255 -> 500..2500 us */
uint16_t        cfg_servo_ms(void);                 /* servo_cs * 10 */
uint16_t        cfg_strike_ms(void);                /* strike_cs * 10 */
uint16_t        cfg_hold_ms(void);                  /* hold_ds * 100 */
uint8_t         cfg_door_src(void);                 /* SENSOR_SRC_* for DOOR_CLOSED */
uint8_t         cfg_bolt_src(void);                 /* SENSOR_SRC_* for BOLT_LOCKED */

#endif /* CONFIG_H */
