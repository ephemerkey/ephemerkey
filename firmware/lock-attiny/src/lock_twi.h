/*
 * ephemerkey lock board — I2C target + register protocol (ATtiny1616 TWI0)
 *
 * Implements the spec in hardware/lock/README.md: target @ 0x60 with three
 * registers. The ISR is pure protocol I/O (fast, no crypto/actuation); the main
 * loop does HMAC verification and actuation. Shared state is exposed as extern
 * volatiles below.
 *
 *   0x00 STATUS  (read)  -> twi_status byte (door/bolt/actuator/rail/busy/ok)
 *   0x01 NONCE   (read)  -> 16 bytes; reading ARMS the challenge
 *   0x10 COMMAND (write) -> cmd(1) ‖ HMAC-SHA1(secret, nonce ‖ cmd) (20)
 */
#ifndef LOCK_TWI_H
#define LOCK_TWI_H

#include <stdint.h>
#include "nonce.h"      /* NONCE_LEN */

/* Build with -DLOCK_DEBUG=1 to expose a debug register (see below). Off by
 * default — never ship it. */
#ifndef LOCK_DEBUG
#define LOCK_DEBUG      0
#endif

/* Register addresses. */
#define REG_STATUS      0x00
#define REG_NONCE       0x01
#define REG_COMMAND     0x10
#if LOCK_DEBUG
#define REG_DEBUG       0x11   /* read 32B: armed nonce(16) ‖ last-verified nonce(16) */
#endif

/* Commands. */
#define CMD_UNLOCK      0x01
#define CMD_LOCK        0x02

/* STATUS bits. */
#define ST_DOOR_CLOSED  0x01
#define ST_BOLT_LOCKED  0x02
#define ST_ACTUATOR     0x04   /* 1 = servo, 0 = solenoid */
#define ST_RAIL_12V     0x08
#define ST_BUSY         0x10
#define ST_LAST_CMD_OK  0x20

/* COMMAND payload = cmd(1) + HMAC-SHA1(20). */
#define CMD_MAXLEN      21u

/* --- shared state (ISR <-> main) --- */
extern volatile uint8_t twi_status;                 /* main writes, ISR serves  */
extern volatile uint8_t twi_cmd_pending;            /* ISR sets on COMMAND STOP */
extern volatile uint8_t twi_cmd_len;
extern uint8_t          twi_cmd_buf[CMD_MAXLEN];    /* cmd ‖ hmac               */
extern uint8_t          twi_next_nonce[NONCE_LEN];  /* main pre-generates       */
extern uint8_t          twi_armed_nonce[NONCE_LEN]; /* ISR freezes on NONCE read*/
extern volatile uint8_t twi_nonce_armed;            /* a challenge is live      */
extern volatile uint8_t twi_nonce_consumed;         /* ISR sets -> main regens  */
#if LOCK_DEBUG
extern uint8_t          twi_dbg[32];                /* armed(16) ‖ verified(16) */
#endif

void twi_target_init(uint8_t addr7);

#endif /* LOCK_TWI_H */
