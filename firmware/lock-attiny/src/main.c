/*
 * ephemerkey lock board — ATtiny1616 firmware
 * Authenticated I2C lock: HMAC-SHA1 challenge-response over TWI0 (target @0x60),
 * with a non-blocking actuator state machine and programmable configuration.
 *
 *   1. Master reads NONCE (0x01) -> lock arms a fresh 16-byte nonce.
 *   2a. UNLOCK/LOCK: write COMMAND (0x10) = cmd ‖ HMAC(pairing_secret, nonce‖cmd).
 *   2b. CONFIG:      write CONFIG (0x20)  = blob ‖ HMAC(config_secret, nonce‖blob).
 *   3. Lock verifies (constant-time), burns the nonce, and either kicks off the
 *      actuator state machine (COMMAND) or persists the config (CONFIG).
 *
 * Actuation NEVER blocks the main loop: a TCB0 ms tick advances a state machine
 * that keeps the right rails powered then turns them off, so I2C stays live and
 * a new lock/unlock aborts the in-flight cycle. LED = BUSY.
 *
 * Sleep is IDLE for bringup (LOCK_SLEEP_MODE); an ~8 s boot window keeps every
 * reset reprogrammable. See README.
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
#include "config.h"
#include "hall.h"
#include "actuate.h"
#include "servo.h"
#include "power.h"
#include "solenoid.h"
#include "sha1.h"
#include "hmac_sha1.h"

/* Keep the wire sizes locked to the crypto so nothing can overflow. */
_Static_assert(HMAC_LEN == SHA1_DIGEST_SIZE,          "HMAC length mismatch");
_Static_assert(CMD_LEN  == 1u + SHA1_DIGEST_SIZE,     "COMMAND length");
_Static_assert(CFG_LEN  == CONFIG_LEN + SHA1_DIGEST_SIZE, "CONFIG length");

#define LED_PIN   PIN3_bm       /* PC3 */

#ifndef LOCK_SLEEP_MODE
#define LOCK_SLEEP_MODE   SLEEP_MODE_IDLE   /* see README; POWER-DOWN is the target */
#endif

static void led_init(void) { PORTC.OUTCLR = LED_PIN; PORTC.DIRSET = LED_PIN; }
static void led_set(uint8_t on) { if (on) PORTC.OUTSET = LED_PIN; else PORTC.OUTCLR = LED_PIN; }

/* twi_status is written only by main/actuate (main context); the ISR only reads it. */
static void status_bit(uint8_t bit, uint8_t on)
{
    if (on) twi_status |= bit; else twi_status &= (uint8_t)~bit;
}

/* Reflect the configured actuator type into STATUS bit2 (1=servo, 0=solenoid).
 * With programmable sequences, "servo" means any step drives a servo. */
static void status_reflect_config(void)
{
    status_bit(ST_ACTUATOR, cfg_any_servo() ? 1 : 0);
}

static void hall_update(uint8_t raw)
{
    uint8_t s = twi_status & (uint8_t)~(ST_DOOR_CLOSED | ST_BOLT_LOCKED);
    if (hall_src(cfg_door_src(), raw)) s |= ST_DOOR_CLOSED;  /* config-mapped */
    if (hall_src(cfg_bolt_src(), raw)) s |= ST_BOLT_LOCKED;
    twi_status = s;
}

/* During an actuation the sensors are already powered (actuate holds HALL_PWR),
 * so sample them live and non-blocking; when idle, do a one-shot pulsed read. */
static void refresh_hall(void)
{
    hall_update(actuate_busy() ? hall_sample() : hall_read());
}

/* Snapshot the armed nonce + RX payload, burning the nonce. */
static uint8_t take_challenge(uint8_t *buf, uint8_t bufcap, uint8_t nonce[NONCE_LEN])
{
    uint8_t rawlen, n, armed;
    cli();
    rawlen = twi_rx_len;
    n = (rawlen > bufcap) ? bufcap : rawlen;
    memcpy(buf, twi_rx_buf, n);
    armed = twi_nonce_armed;
    memcpy(nonce, twi_armed_nonce, NONCE_LEN);
    twi_nonce_armed = 0;                     /* single-use */
    sei();
    return armed ? rawlen : 0;               /* 0 = no live challenge */
}

/* COMMAND: verify pairing HMAC, then kick the (non-blocking) actuator. */
static void service_command(void)
{
    uint8_t buf[CMD_LEN], nonce[NONCE_LEN];
    twi_cmd_pending = 0;
    uint8_t rawlen = take_challenge(buf, sizeof(buf), nonce);

    if (rawlen != CMD_LEN) { status_bit(ST_LAST_CMD_OK, 0); return; }
    uint8_t cmd = buf[0];
    if (cmd != CMD_UNLOCK && cmd != CMD_LOCK) { status_bit(ST_LAST_CMD_OK, 0); return; }

    uint8_t secret[SECRET_LEN];
    secret_get_pairing(secret);
    uint8_t msg[NONCE_LEN + 1];
    memcpy(msg, nonce, NONCE_LEN);
    msg[NONCE_LEN] = cmd;
    uint8_t mac[SHA1_DIGEST_SIZE];
    hmac_sha1(secret, SECRET_LEN, msg, NONCE_LEN + 1, mac);
#if LOCK_DEBUG
    memcpy(twi_dbg + NONCE_LEN, nonce, NONCE_LEN);
#endif
    if (!ct_equal(mac, &buf[1], SHA1_DIGEST_SIZE)) { status_bit(ST_LAST_CMD_OK, 0); return; }

    status_bit(ST_LAST_CMD_OK, 1);
    actuate_begin(cmd == CMD_UNLOCK);        /* non-blocking; aborts any in-flight */
}

/* CONFIG: verify config HMAC, then persist the blob. */
static void service_config(void)
{
    uint8_t buf[CFG_LEN], nonce[NONCE_LEN];
    twi_cfg_pending = 0;
    uint8_t rawlen = take_challenge(buf, sizeof(buf), nonce);

    if (rawlen != CFG_LEN) { status_bit(ST_LAST_CMD_OK, 0); return; }

    uint8_t secret[SECRET_LEN];
    secret_get_config(secret);
    uint8_t msg[NONCE_LEN + CONFIG_LEN];
    memcpy(msg, nonce, NONCE_LEN);
    memcpy(msg + NONCE_LEN, buf, CONFIG_LEN);
    uint8_t mac[SHA1_DIGEST_SIZE];
    hmac_sha1(secret, SECRET_LEN, msg, NONCE_LEN + CONFIG_LEN, mac);

    if (!ct_equal(mac, &buf[CONFIG_LEN], SHA1_DIGEST_SIZE) || !config_apply_blob(buf)) {
        status_bit(ST_LAST_CMD_OK, 0);
        return;
    }
    status_reflect_config();
    status_bit(ST_LAST_CMD_OK, 1);
}

/* Process any completed transaction + advance the actuator. Non-blocking. */
static void service_pending(void)
{
    if (twi_nonce_consumed) { twi_nonce_consumed = 0; nonce_next(twi_next_nonce); }
    if (twi_cmd_pending) service_command();
    if (twi_cfg_pending) service_config();
    actuate_tick();
}

int main(void)
{
    led_init();
    power_init();
    servo_init();
    sol_init();
    hall_init();
    config_init();
    actuate_init();
    nonce_init();

    twi_status = 0;
    status_reflect_config();
    nonce_next(twi_next_nonce);
    twi_target_init(LOCK_I2C_ADDR);
    refresh_hall();

    set_sleep_mode(LOCK_SLEEP_MODE);
    sleep_enable();
    sei();

    /* ~8 s awake boot/reprogram window (LED flutters), still servicing I2C. */
    for (uint8_t i = 0; i < 40; i++) {
        PORTC.OUTTGL = LED_PIN;
        _delay_ms(200);
        service_pending();
    }
    PORTC.OUTCLR = LED_PIN;

    for (;;) {
        service_pending();
        refresh_hall();                        /* live sample while busy, one-shot when idle */
        led_set(actuate_busy());

        /* Race-free sleep: don't sleep if work arrived while we were busy. */
        cli();
        if (!twi_cmd_pending && !twi_cfg_pending && !twi_nonce_consumed) {
            sei();
            sleep_cpu();                       /* TCB tick or I2C wakes us */
        } else {
            sei();
        }
    }
}
