//! Authenticated I2C master link to the lock board (ATtiny1616 target @0x60).
//!
//! PA6 = LOCK_SDA, PA7 = LOCK_SCL — hardware **I2C3** (AF4), on J2, a
//! dedicated bus separate from the I2C1 sensor bus. (Rev 0.2 pin swap: the
//! link originally sat on PB0/PB1, which carry no I2C silicon on the U083;
//! the LEDs took PB0/PB1 instead.)
//!
//! HARD REQUIREMENT (see firmware/lock-attiny/README.md "bus transients"):
//! lock actuation glitches SDA/SCL for ~0.4 s (boost converter driving
//! stalled servos). This master MUST recover from bus errors / NACK bursts:
//! on any error, re-init the peripheral (drop + reconstruct), issue a bus
//! clear, and retry — the ATmega bridge needed exactly that treatment.
//!
//! Protocol (proven against the lock firmware):
//!   REG_STATUS 0x00 (read 1B) | REG_NONCE 0x01 (read 16B, arms the nonce) |
//!   REG_COMMAND 0x10 (write: cmd ‖ HMAC-SHA1(pairing, nonce ‖ cmd)) |
//!   CMD_UNLOCK 0x01, CMD_LOCK 0x02, CMD_ABORT 0x03.
//! The lock sleeps in power-down and wakes on the first START; send a dummy
//! wake transfer and retry once it is up.
//!
//! Scaffold: claims the bus and parks; no lock attached to the bench rig yet.

use defmt::info;
use embassy_stm32::i2c::{self, I2c};
use embassy_stm32::peripherals::{I2C3, PA6, PA7};
use embassy_stm32::time::Hertz;
use embassy_stm32::Peri;
use embassy_time::Timer;

pub const LOCK_ADDR: u8 = 0x60;

#[embassy_executor::task]
pub async fn task(i2c: Peri<'static, I2C3>, scl: Peri<'static, PA7>, sda: Peri<'static, PA6>) {
    // 100 kHz — short cable, and the ATtiny target clock-stretches freely.
    let mut cfg = i2c::Config::default();
    cfg.frequency = Hertz::khz(100);
    cfg.timeout = embassy_time::Duration::from_millis(50);
    let _bus = I2c::new_blocking(i2c, scl, sda, cfg);

    info!("lock: I2C3 up (target {:#04x})", LOCK_ADDR);
    loop {
        // Placeholder until the nonce/HMAC command flow is ported. The real
        // loop: wake transfer -> STATUS poll -> policy engine decides ->
        // NONCE read -> COMMAND write; on any bus error re-init + bus-clear
        // + retry (expected during the lock's actuation windows).
        Timer::after_secs(3600).await;
    }
}
