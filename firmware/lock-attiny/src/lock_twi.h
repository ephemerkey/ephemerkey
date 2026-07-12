/*
 * ephemerkey lock board — I2C target + register protocol (ATtiny1616 TWI0)
 *
 *   0x00 STATUS  (read)  -> twi_status byte
 *   0x01 NONCE   (read)  -> 16 bytes; reading ARMS the challenge
 *   0x10 COMMAND (write) -> cmd(1) ‖ HMAC(pairing_secret, nonce ‖ cmd)      (21B)
 *   0x20 CONFIG  (write) -> blob(CONFIG_LEN) ‖ HMAC(config_secret, nonce ‖ blob)
 *                (read)  -> current config blob (unauthenticated)
 *
 * COMMAND and CONFIG writes share one RX buffer (only one is in flight at a time)
 * and each sets its own *_pending flag on STOP. The ISR is pure protocol I/O.
 */
#ifndef LOCK_TWI_H
#define LOCK_TWI_H

#include <stdint.h>
#include "nonce.h"      /* NONCE_LEN */
#include "config.h"     /* CONFIG_LEN */

#ifndef LOCK_DEBUG
#define LOCK_DEBUG      0
#endif

#define REG_STATUS      0x00
#define REG_NONCE       0x01
#define REG_COMMAND     0x10
#define REG_CONFIG      0x20
#if LOCK_DEBUG
#define REG_DEBUG       0x11
#endif

#define CMD_UNLOCK      0x01
#define CMD_LOCK        0x02
#define CMD_ABORT       0x03   /* stop any in-flight cycle, everything off */

#define ST_DOOR_CLOSED  0x01
#define ST_BOLT_LOCKED  0x02
#define ST_ACTUATOR     0x04   /* 1 = servo, 0 = solenoid */
#define ST_RAIL_12V     0x08
#define ST_BUSY         0x10
#define ST_LAST_CMD_OK  0x20

#define HMAC_LEN        20u                    /* SHA1_DIGEST_SIZE */
#define CMD_LEN         (1u + HMAC_LEN)        /* COMMAND payload */
#define CFG_LEN         (CONFIG_LEN + HMAC_LEN)/* CONFIG payload  */
#define RX_MAX          CFG_LEN                /* larger of the two writes */

/* --- shared state (ISR <-> main) --- */
extern volatile uint8_t twi_status;
extern volatile uint8_t twi_cmd_pending;            /* COMMAND write complete   */
extern volatile uint8_t twi_cfg_pending;            /* CONFIG write complete    */
extern volatile uint8_t twi_rx_len;                 /* bytes in twi_rx_buf      */
extern uint8_t          twi_rx_buf[RX_MAX];         /* COMMAND or CONFIG payload*/
extern uint8_t          twi_next_nonce[NONCE_LEN];
extern uint8_t          twi_armed_nonce[NONCE_LEN];
extern volatile uint8_t twi_nonce_armed;
extern volatile uint8_t twi_nonce_consumed;
#if LOCK_DEBUG
extern uint8_t          twi_dbg[32];                /* armed(16) ‖ verified(16) */
#endif

void twi_target_init(uint8_t addr7);

#endif /* LOCK_TWI_H */
