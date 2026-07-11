/*
 * ephemerkey lock board — ATtiny1616 firmware
 * Authenticated I2C lock: HMAC-SHA1 challenge-response over TWI0 (target @0x60).
 *
 * Architecture (per hardware/lock/README.md):
 *   POWER-DOWN sleep  --(I2C START wakes on TWI address match; verified to wake
 *   from power-down)-->  ISR services the register protocol (STATUS/NONCE/
 *   COMMAND)  -->  main verifies the HMAC and actuates  -->  back to sleep.
 *
 * Protocol:
 *   1. Master reads NONCE (0x01) -> lock arms a fresh 16-byte nonce.
 *   2. Master writes COMMAND (0x10) = cmd ‖ HMAC-SHA1(secret, nonce ‖ cmd).
 *   3. Lock recomputes, constant-time compares, burns the nonce (replay-proof),
 *      and actuates (UNLOCK/LOCK). STATUS reports door/bolt/busy/last-cmd-ok.
 *
 * The heavy work (HMAC ~hundreds of us, actuation seconds) runs in main, not the
 * ISR, so the bus isn't stretched. The LED gives brief activity feedback.
 *
 * NOTE: compile+flash verified; live protocol verification is deferred to the
 * STM32 master (its I2C/HMAC side isn't implemented yet).
 */
#include <avr/io.h>
#include <avr/interrupt.h>
#include <avr/sleep.h>
#include <util/delay.h>
#include <string.h>

#include "lock_config.h"
#include "lock_twi.h"
#include "nonce.h"
#include "secret.h"
#include "hall.h"
#include "actuate.h"
#include "servo.h"
#include "power.h"
#include "solenoid.h"
#include "sha1.h"
#include "hmac_sha1.h"

/* The COMMAND payload is exactly cmd(1) + HMAC-SHA1(20); keep the RX buffer and
 * the length check in lockstep with the crypto so neither can overflow. */
_Static_assert(CMD_MAXLEN == 1u + SHA1_DIGEST_SIZE, "CMD_MAXLEN must equal cmd+HMAC");

#define LED_PIN   PIN3_bm       /* PC3 */

/* Sleep mode. IDLE for bringup: the TWI clock keeps running so the target
 * answers instantly — no wake-from-power-down first-NACK and no bus-wedge/UPDI
 * trap. POWER-DOWN (~0.1 uA) is the deployment target, to re-enable once the
 * protocol is proven and the wake path is hardened. */
#ifndef LOCK_SLEEP_MODE
#define LOCK_SLEEP_MODE   SLEEP_MODE_IDLE
#endif

static void led_init(void) { PORTC.OUTCLR = LED_PIN; PORTC.DIRSET = LED_PIN; }

static void led_pulse(uint8_t n)
{
    while (n--) {
        PORTC.OUTSET = LED_PIN; _delay_ms(60);
        PORTC.OUTCLR = LED_PIN; _delay_ms(120);
    }
}

/* twi_status is written only by main, so this RMW is safe against the ISR
 * (which only reads it). */
static void status_bit(uint8_t bit, uint8_t on)
{
    if (on) twi_status |= bit; else twi_status &= (uint8_t)~bit;
}

static void refresh_hall(void)
{
    uint8_t h = hall_read();
    uint8_t s = twi_status & (uint8_t)~(ST_DOOR_CLOSED | ST_BOLT_LOCKED);
    if (h & HALL_DOOR_CLOSED) s |= ST_DOOR_CLOSED;
    if (h & HALL_BOLT_LOCKED) s |= ST_BOLT_LOCKED;
    twi_status = s;
}

/* Verify the pending COMMAND and, if authentic, actuate. */
static void service_command(void)
{
    uint8_t buf[CMD_MAXLEN], rawlen, n, armed, nonce[NONCE_LEN];

    cli();
    rawlen = twi_cmd_len;                            /* true count, saturated <= CMD_MAXLEN+1 */
    n = (rawlen > CMD_MAXLEN) ? CMD_MAXLEN : rawlen; /* clamp the copy length */
    memcpy(buf, twi_cmd_buf, n);                     /* n <= CMD_MAXLEN == sizeof(buf) */
    armed = twi_nonce_armed;
    memcpy(nonce, twi_armed_nonce, NONCE_LEN);
    twi_cmd_pending = 0;
    twi_nonce_armed = 0;               /* single-use: burn regardless of outcome */
    sei();

    /* Require EXACTLY cmd(1) + HMAC(20) — reject short or over-long writes. */
    if (!armed || rawlen != 1u + SHA1_DIGEST_SIZE) {
        status_bit(ST_LAST_CMD_OK, 0);
        led_pulse(3);
        return;
    }

    uint8_t cmd = buf[0];
    uint8_t secret[LOCK_SECRET_LEN];
    secret_get(secret);

    uint8_t msg[NONCE_LEN + 1];
    memcpy(msg, nonce, NONCE_LEN);
    msg[NONCE_LEN] = cmd;

    uint8_t mac[SHA1_DIGEST_SIZE];
    hmac_sha1(secret, LOCK_SECRET_LEN, msg, NONCE_LEN + 1, mac);

#if LOCK_DEBUG
    memcpy(twi_dbg + NONCE_LEN, nonce, NONCE_LEN);   /* nonce this verify used */
#endif

    if (!ct_equal(mac, &buf[1], SHA1_DIGEST_SIZE) ||
        (cmd != CMD_UNLOCK && cmd != CMD_LOCK)) {
        status_bit(ST_LAST_CMD_OK, 0);
        led_pulse(3);                  /* reject */
        return;
    }

    /* Authentic: actuate. */
    status_bit(ST_BUSY, 1);
#if LOCK_ACTUATOR == ACTUATOR_SOLENOID
    if (cmd == CMD_UNLOCK) {
        status_bit(ST_RAIL_12V, 1);
        actuate_unlock();
        status_bit(ST_RAIL_12V, 0);
    }                                  /* LOCK: momentary solenoid no-op */
#else
    if (cmd == CMD_UNLOCK) actuate_unlock(); else actuate_lock();
#endif
    status_bit(ST_BUSY, 0);
    status_bit(ST_LAST_CMD_OK, 1);
    refresh_hall();
    led_pulse(1);                      /* ack */
}

int main(void)
{
    led_init();
    power_init();                      /* boost off, interlock clear (FIRST) */
    servo_init();
    sol_init();
    hall_init();
    actuate_init();
    nonce_init();

    twi_status = 0;
#if LOCK_ACTUATOR == ACTUATOR_SERVO
    twi_status |= ST_ACTUATOR;
#endif

    nonce_next(twi_next_nonce);        /* pre-arm the first challenge */
    twi_target_init(LOCK_I2C_ADDR);
    refresh_hall();

    sei();

    /* Programming/recovery window: stay AWAKE (UPDI reachable) for ~8 s at every
     * reset before entering continuous power-down — a chip in uninterrupted
     * power-down is hard to re-init over UPDI. LED flutters ~2.5 Hz to show the
     * window; I2C is already live and serviced here too. */
    for (uint8_t i = 0; i < 40; i++) {
        PORTC.OUTTGL = LED_PIN;
        _delay_ms(200);
        if (twi_nonce_consumed) { twi_nonce_consumed = 0; nonce_next(twi_next_nonce); }
        if (twi_cmd_pending)    { service_command(); }
    }
    PORTC.OUTCLR = LED_PIN;

    set_sleep_mode(LOCK_SLEEP_MODE);   /* IDLE (bringup) — see LOCK_SLEEP_MODE */
    sleep_enable();

    for (;;) {
        if (twi_nonce_consumed) {
            twi_nonce_consumed = 0;
            nonce_next(twi_next_nonce); /* prepare the next challenge */
        }
        if (twi_cmd_pending) {
            service_command();
        }
        refresh_hall();                /* keep door/bolt fresh for next STATUS */

        /* Race-free sleep: only sleep if no work arrived while we were busy
         * (e.g. a STOP interrupt during refresh_hall). sei() + sleep is atomic
         * on AVR — an interrupt can't fire between them. */
        cli();
        if (!twi_cmd_pending && !twi_nonce_consumed) {
            sei();
            sleep_cpu();
        } else {
            sei();
        }
    }
}
