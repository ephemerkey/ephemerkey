/*
 * ephemerkey lock board — programmable configuration (ATtiny1616)
 *
 * A bit-packed config blob, provisioned over I2C with its own HMAC secret
 * (REG_CONFIG), persisted in EEPROM, loaded at boot. Falls back to compile-time
 * defaults when EEPROM is unprogrammed.
 *
 * Actuation is a programmable STEP SEQUENCE: an ordered list of phases run on
 * UNLOCK, and an independent list run on LOCK. Each step fires any combination
 * of {servo1, servo2, solenoid} together, with per-step servo target positions
 * (so a servo can be driven either direction), a run time, and an optional
 * early-off sensor that ends the step early and advances to the next one.
 */
#ifndef CONFIG_H
#define CONFIG_H

#include <stdint.h>

#define CONFIG_MAGIC     0xE4       /* bump on layout change */

#define SEQ_STEPS        6          /* max phases per sequence */
#define STEP_BYTES       5          /* wire size of one step_t */
#define CONFIG_HDR       5          /* header bytes before the sequences */
#define CONFIG_LEN       (CONFIG_HDR + STEP_BYTES * SEQ_STEPS * 2)  /* 65 */

/* --- flags byte --- */
/* 6 V boosted servo: servo-only steps run off the boost rail at 6 V (BOOST_VSEL
 * low = interlock clear) instead of Vbat, for a 6 V servo. Requires the servo
 * strapped to VSOL (R13, the default). Off by default — do NOT set it unless the
 * board is wired for a boosted servo. Combined servo+solenoid steps force 6 V
 * regardless (see step.act below). */
#define CFG_SERVO_BOOST  0x01

/* --- step.act bits (which actuators the step drives) --- */
#define STEP_SERVO1      0x01
#define STEP_SERVO2      0x02
#define STEP_SOLENOID    0x04
/* act == 0 marks the end of the sequence (remaining steps ignored). */

/* --- step.eoff: per-step early-off selector --- */
/* bits 0-1: which LOGICAL sensor gates the step (resolved through sensor_map) */
#define EOFF_NONE        0u
#define EOFF_DOOR        1u         /* the DOOR_CLOSED logical sensor */
#define EOFF_BOLT        2u         /* the BOLT_LOCKED logical sensor */
#define EOFF_SENSOR_MASK 0x03u
/* bit 2: trigger edge. Clear = advance when the magnet is PRESENT (sensor
 * active); set = advance when the magnet is ABSENT (e.g. door opened).
 * Deglitched (see actuate.c): the step must first CONFIRM the opposite state
 * (arm), then an integrating counter of fixed-cadence samples must reach the
 * firing level — a few transients only delay it; a sensor that never shows the
 * opposite state (broken/disturbed) never fires and the step runs full time. */
#define EOFF_EDGE_ABSENT 0x04u

/* sensor_map: each STATUS role picks its source sensor, so either physical port
 * (J6 or J7) can be the "door" or the "bolt" indicator (or a role can be off).
 * The per-step early-off DOOR/BOLT selectors resolve through this map too.
 *   bits 0-1: DOOR_CLOSED source   bits 2-3: BOLT_LOCKED source */
#define SENSOR_SRC_J6    0u   /* HALL pin on J6, PA7 */
#define SENSOR_SRC_J7    1u   /* HALL pin on J7, PB3 */
#define SENSOR_SRC_OFF   2u   /* role disabled -> reads 0 */
#define SENSOR_DOOR_SHIFT 0
#define SENSOR_BOLT_SHIFT 2
#define SENSOR_SRC_MASK  0x03u

/* One phase of a sequence. All uint8_t, so struct layout == wire layout. */
typedef struct {
    uint8_t act;      /* STEP_* actuator bits; 0 = end of sequence */
    uint8_t s1_pos;   /* servo1 target 0..255 -> 500..2500 us (if STEP_SERVO1) */
    uint8_t s2_pos;   /* servo2 target (if STEP_SERVO2) */
    uint8_t dur_ds;   /* run time, x100 ms (servo drive / solenoid hold), 0..25.5 s */
    uint8_t eoff;     /* early-off selector (EOFF_*) */
} step_t;

/* Packed blob — all uint8_t, so struct layout == wire layout (no padding). */
typedef struct {
    uint8_t magic;       /* CONFIG_MAGIC */
    uint8_t flags;       /* CFG_* */
    uint8_t strike_cs;   /* solenoid strike (full pull-in) time, x10 ms (12 V economizer) */
    uint8_t hold_duty;   /* solenoid economizer hold PWM duty, 0..255 -> 0..100 % */
    uint8_t sensor_map;  /* door/bolt role -> J6/J7/off (see SENSOR_*) */
    step_t  seq_unlock[SEQ_STEPS];
    step_t  seq_lock[SEQ_STEPS];
} config_t;

void            config_init(void);                  /* load EEPROM or defaults */
const config_t *config_get(void);
void            config_to_blob(uint8_t out[CONFIG_LEN]);

/* Validate a received blob (magic), persist to EEPROM, reload. Returns 1 on ok. */
uint8_t         config_apply_blob(const uint8_t blob[CONFIG_LEN]);

/* Encoding helpers. */
uint16_t        cfg_pos_to_us(uint8_t pos);         /* 0..255 -> 500..2500 us */
uint16_t        cfg_strike_ms(void);                /* strike_cs * 10 */
uint8_t         cfg_door_src(void);                 /* SENSOR_SRC_* for DOOR_CLOSED */
uint8_t         cfg_bolt_src(void);                 /* SENSOR_SRC_* for BOLT_LOCKED */
uint8_t         cfg_any_servo(void);                /* 1 if any step drives a servo */

#endif /* CONFIG_H */
