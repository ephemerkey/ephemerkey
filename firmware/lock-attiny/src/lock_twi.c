/*
 * ephemerkey lock board — I2C target + register protocol (ATtiny1616 TWI0)
 * See lock_twi.h. TWI0 client on PB0(SCL)/PB1(SDA), default PORTMUX. External
 * bus pull-ups assumed (the lock has none on-board).
 */
#include <avr/io.h>
#include <avr/interrupt.h>
#include <string.h>
#include "lock_twi.h"
#include "config.h"

/* --- shared state definitions --- */
volatile uint8_t twi_status;
volatile uint8_t twi_cmd_pending;
volatile uint8_t twi_cfg_pending;
volatile uint8_t twi_rx_len;
uint8_t          twi_rx_buf[RX_MAX];
uint8_t          twi_next_nonce[NONCE_LEN];
uint8_t          twi_armed_nonce[NONCE_LEN];
volatile uint8_t twi_nonce_armed;
volatile uint8_t twi_nonce_consumed;
#if LOCK_DEBUG
uint8_t          twi_dbg[32];
#define TX_BUF_SIZE  32
#else
#define TX_BUF_SIZE  NONCE_LEN
#endif

/* --- ISR-private transaction state --- */
enum { RX_EXPECT_REG, RX_EXPECT_DATA };
static uint8_t s_reg = REG_STATUS;   /* current register pointer (sticky) */
static uint8_t s_rx_state;
static uint8_t s_rx_count;

static uint8_t s_tx_buf[TX_BUF_SIZE]; /* bytes to stream on a read */
static uint8_t s_tx_len;
static uint8_t s_tx_idx;

void twi_target_init(uint8_t addr7)
{
    s_reg = REG_STATUS;
    s_rx_state = RX_EXPECT_REG;

    TWI0.SADDR = (uint8_t)(addr7 << 1);
    TWI0.SADDRMASK = 0;
    TWI0.SCTRLA = TWI_DIEN_bm | TWI_APIEN_bm | TWI_PIEN_bm | TWI_ENABLE_bm;
}

static uint8_t is_write_reg(uint8_t r)
{
    return r == REG_COMMAND || r == REG_CONFIG;
}

ISR(TWI0_TWIS_vect)
{
    uint8_t s = TWI0.SSTATUS;

    if (s & (TWI_BUSERR_bm | TWI_COLL_bm)) {   /* clear + release, never wedge */
        TWI0.SSTATUS = TWI_BUSERR_bm | TWI_COLL_bm;
        TWI0.SCTRLB = TWI_SCMD_COMPTRANS_gc;
        s_rx_state = RX_EXPECT_REG;
        return;
    }

    if (s & TWI_APIF_bm) {
        if (s & TWI_AP_bm) {                    /* --- address match --- */
            if (s & TWI_DIR_bm) {               /* master READ: prepare tx */
                if (s_reg == REG_NONCE) {
                    memcpy(twi_armed_nonce, twi_next_nonce, NONCE_LEN);
                    twi_nonce_armed = 1;
                    twi_nonce_consumed = 1;
                    memcpy(s_tx_buf, twi_armed_nonce, NONCE_LEN);
                    s_tx_len = NONCE_LEN;
#if LOCK_DEBUG
                    memcpy(twi_dbg, twi_armed_nonce, NONCE_LEN);
                } else if (s_reg == REG_DEBUG) {
                    memcpy(s_tx_buf, twi_dbg, sizeof(twi_dbg));
                    s_tx_len = sizeof(twi_dbg);
#endif
                } else if (s_reg == REG_CONFIG) {
                    config_to_blob(s_tx_buf);   /* CONFIG_LEN <= TX_BUF_SIZE */
                    s_tx_len = CONFIG_LEN;
                } else {                        /* STATUS (default) */
                    s_tx_buf[0] = twi_status;
                    s_tx_len = 1;
                }
                s_tx_idx = 0;
            } else {                            /* master WRITE */
                s_rx_state = RX_EXPECT_REG;
                s_rx_count = 0;
            }
            TWI0.SCTRLB = TWI_SCMD_RESPONSE_gc; /* ACK the address */
        } else {                                /* --- STOP --- */
            if (s_rx_count >= 1) {
                if (s_reg == REG_COMMAND)      { twi_rx_len = s_rx_count; twi_cmd_pending = 1; }
                else if (s_reg == REG_CONFIG)  { twi_rx_len = s_rx_count; twi_cfg_pending = 1; }
            }
            TWI0.SCTRLB = TWI_SCMD_COMPTRANS_gc;
        }
        return;
    }

    if (s & TWI_DIF_bm) {
        if (s & TWI_DIR_bm) {                   /* master READ: we transmit */
            if (s_tx_idx && (s & TWI_RXACK_bm)) {
                TWI0.SCTRLB = TWI_SCMD_COMPTRANS_gc;
            } else {
                uint8_t b = (s_tx_idx < s_tx_len) ? s_tx_buf[s_tx_idx] : 0x00;
                s_tx_idx++;
                TWI0.SDATA = b;
                TWI0.SCTRLB = TWI_SCMD_RESPONSE_gc;
            }
        } else {                                /* master WRITE: we received */
            uint8_t d = TWI0.SDATA;
            if (s_rx_state == RX_EXPECT_REG) {
                s_reg = d;
                s_rx_state = RX_EXPECT_DATA;
                s_rx_count = 0;
            } else {
                if (is_write_reg(s_reg) && s_rx_count < RX_MAX)
                    twi_rx_buf[s_rx_count] = d;   /* bounded to [0,RX_MAX) */
                if (s_rx_count <= RX_MAX)
                    s_rx_count++;                /* saturate: never wraps */
            }
            TWI0.SCTRLB = TWI_SCMD_RESPONSE_gc;
        }
    }
}
