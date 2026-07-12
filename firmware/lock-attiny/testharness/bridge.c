/*
 * RedBoard (ATmega328P @ 16 MHz) — serial <-> I2C-master bridge.
 *
 * A tiny test harness for the ephemerkey lock (ATtiny1616 TWI target @ 0x60):
 * the host drives I2C transactions over USB-serial, so lock_test.py can run the
 * NONCE / COMMAND / STATUS protocol without the STM32 master.
 *
 * Serial: 57600 8N1. All numbers are HEX. One response line per command:
 *   W <addr> <b0> <b1> ...   I2C write         -> "OK"  or "ERR <tw_status>"
 *   R <addr> <n>             I2C read n bytes   -> "D <b0> <b1> ..." or "ERR <s>"
 *   ?                        help
 *
 * I2C is open-drain: this bridge only ever pulls SDA/SCL low (internal pull-ups
 * are DISABLED), so a 5 V RedBoard is safe against a ~3.3 V lock — PROVIDED the
 * bus pull-ups go to the LOCK's Vdd, not 5 V, and grounds are common.
 */
#include <avr/io.h>
#include <stdint.h>

#define BAUD 57600
#include <util/setbaud.h>

/* ---- UART ---- */
static void uart_init(void)
{
    UBRR0H = UBRRH_VALUE;
    UBRR0L = UBRRL_VALUE;
#if USE_2X
    UCSR0A |= (1 << U2X0);
#else
    UCSR0A &= ~(1 << U2X0);
#endif
    UCSR0C = (1 << UCSZ01) | (1 << UCSZ00);   /* 8N1 */
    UCSR0B = (1 << RXEN0) | (1 << TXEN0);
}
static void uart_tx(char c)      { while (!(UCSR0A & (1 << UDRE0))); UDR0 = c; }
static uint8_t uart_rx(void)     { while (!(UCSR0A & (1 << RXC0))); return UDR0; }
static void uart_puts(const char *s) { while (*s) uart_tx(*s++); }
static void uart_hex8(uint8_t b)
{
    static const char h[] = "0123456789ABCDEF";
    uart_tx(h[b >> 4]); uart_tx(h[b & 0xF]);
}

/* ---- I2C master (TWI) ---- */
static void twi_init(void)
{
    PORTC &= ~((1 << PC4) | (1 << PC5));   /* no internal pull-ups (would be 5 V) */
    TWSR = 0;                              /* prescaler 1 */
    TWBR = 72;                             /* 100 kHz @ 16 MHz */
    TWCR = (1 << TWEN);
}
static uint8_t twi_wait(void)
{
    uint16_t t = 0;
    while (!(TWCR & (1 << TWINT))) if (++t == 0) return 1;   /* ~16 ms timeout */
    return 0;
}
static uint8_t twi_start(void)
{
    TWCR = (1 << TWINT) | (1 << TWSTA) | (1 << TWEN);
    if (twi_wait()) return 0xFF;
    return TWSR & 0xF8;
}
static uint8_t twi_wr(uint8_t d)
{
    TWDR = d;
    TWCR = (1 << TWINT) | (1 << TWEN);
    if (twi_wait()) return 0xFF;
    return TWSR & 0xF8;
}
static uint8_t twi_rd(uint8_t ack, uint8_t *out)
{
    TWCR = (1 << TWINT) | (1 << TWEN) | (ack ? (1 << TWEA) : 0);
    if (twi_wait()) return 1;
    *out = TWDR;
    return 0;
}
static void twi_stop(void)
{
    TWCR = (1 << TWINT) | (1 << TWSTO) | (1 << TWEN);
    uint16_t t = 0;
    while (TWCR & (1 << TWSTO)) if (++t == 0) break;
}

static void err(uint8_t st) { uart_puts("ERR "); uart_hex8(st); uart_tx('\n'); }

/* ---- line parsing ---- */
/* Big enough for the longest command: "W 60 20 " + 30 hex bytes (config write)
 * ~= 98 chars, plus headroom. */
static char line[320];   /* CONFIG write = "W 60 " + 86 bytes*3 ~= 263 chars */

static uint8_t hexnib(char c)
{
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return 0xFF;
}
/* parse space-separated hex bytes from line[start..]; return count. */
static uint8_t parse_bytes(uint16_t start, uint8_t *out, uint8_t max)
{
    uint8_t n = 0;
    uint16_t i = start;         /* line can be >255 chars (uint16 index) */
    while (line[i] && n < max) {
        while (line[i] == ' ') i++;
        if (!line[i]) break;
        uint8_t hi = hexnib(line[i]);
        if (hi == 0xFF) break;
        i++;
        uint8_t v = hi, lo = hexnib(line[i]);
        if (lo != 0xFF) { v = (uint8_t)((hi << 4) | lo); i++; }
        out[n++] = v;
        while (line[i] && line[i] != ' ') i++;
    }
    return n;
}

static void do_write(const uint8_t *b, uint8_t n)   /* b[0]=addr, b[1..]=data */
{
    uint8_t st = twi_start();
    if (st != 0x08 && st != 0x10) { twi_stop(); err(st); return; }
    st = twi_wr((uint8_t)((b[0] << 1) | 0));
    if (st != 0x18) { twi_stop(); err(st); return; }        /* SLA+W ACK */
    for (uint8_t i = 1; i < n; i++) {
        st = twi_wr(b[i]);
        if (st != 0x28) { twi_stop(); err(st); return; }    /* data ACK */
    }
    twi_stop();
    uart_puts("OK\n");
}

static void do_read(uint8_t addr, uint8_t n)
{
    uint8_t st = twi_start();
    if (st != 0x08 && st != 0x10) { twi_stop(); err(st); return; }
    st = twi_wr((uint8_t)((addr << 1) | 1));
    if (st != 0x40) { twi_stop(); err(st); return; }        /* SLA+R ACK */
    uart_tx('D');
    for (uint8_t i = 0; i < n; i++) {
        uint8_t v = 0;
        if (twi_rd(i < (uint8_t)(n - 1), &v)) { uart_puts(" TIMEOUT"); break; }
        uart_tx(' '); uart_hex8(v);
    }
    twi_stop();
    uart_tx('\n');
}

int main(void)
{
    uart_init();
    twi_init();
    uart_puts("BRIDGE ready\n");

    for (;;) {
        uint16_t li = 0;        /* line index — must exceed 255 (uint16) */
        for (;;) {
            char c = (char)uart_rx();
            if (c == '\r') continue;
            if (c == '\n') break;
            if (li < sizeof(line) - 1) line[li++] = c;
        }
        line[li] = 0;

        if (line[0] == 'W' || line[0] == 'w') {
            uint8_t buf[100];   /* addr + reg + 85-byte CONFIG payload */
            uint8_t n = parse_bytes(1, buf, sizeof(buf));
            if (n < 1) uart_puts("ERR args\n"); else do_write(buf, n);
        } else if (line[0] == 'R' || line[0] == 'r') {
            uint8_t buf[2];
            uint8_t n = parse_bytes(1, buf, 2);
            if (n < 2) uart_puts("ERR args\n"); else do_read(buf[0], buf[1]);
        } else if (line[0] == '?') {
            uart_puts("BRIDGE: W addr b..; R addr n  (all hex)\n");
        } else if (li) {
            uart_puts("ERR cmd\n");
        }
    }
}
