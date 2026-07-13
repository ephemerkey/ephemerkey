//! Authenticated I2C master link to the lock board (ATtiny1616 target @0x60).
//!
//! PB0 = LOCK_SDA, PB1 = LOCK_SCL (J2) — a dedicated bus, separate from the
//! I2C1 sensor bus.
//!
//! **These pins have NO hardware I2C function on the STM32U083** (verified
//! against the AF table: PB0/PB1 only carry LCD/LPTIM3/SPI1_CS/UART-flow
//! AFs), so this is a BIT-BANGED open-drain master. That is a feature, not a
//! workaround: lock actuation glitches SDA/SCL for ~0.4 s (boost converter
//! driving stalled servos — see firmware/lock-attiny/README.md "bus
//! transients"), and a software master owns its own error recovery outright
//! (no peripheral error-state machine to unwedge). If hardware I2C is ever
//! wanted here, the schematic must re-pin (e.g. swap LEDs PA6/PA7 <-> PB0/PB1
//! to free I2C3).
//!
//! Protocol (proven against the lock firmware):
//!   REG_STATUS 0x00 (read 1B) | REG_NONCE 0x01 (read 16B, arms the nonce) |
//!   REG_COMMAND 0x10 (write: cmd ‖ HMAC-SHA1(pairing, nonce ‖ cmd)) |
//!   CMD_UNLOCK 0x01, CMD_LOCK 0x02, CMD_ABORT 0x03.
//! The lock sleeps in power-down and wakes on the first START; send a dummy
//! wake transfer and retry once it is up.

// Scaffold: the byte-level ops are exercised once the command flow lands.
#![allow(dead_code)]

use defmt::info;
use embassy_stm32::gpio::{Level, OutputOpenDrain, Speed};
use embassy_stm32::peripherals::{PB0, PB1};
use embassy_stm32::Peri;
use embassy_time::Timer;

pub const LOCK_ADDR: u8 = 0x60;

/// Half-bit busy-wait, in CPU cycles. ~5 us at the 16 MHz HSI boot clock
/// -> ~100 kHz SCL. TODO: derive from the live sysclk once clock config lands.
const HALF_BIT_CYCLES: u32 = 80;

/// Software open-drain I2C master. Blocking ops are fine: a full
/// nonce+command exchange is ~40 bytes ≈ 4 ms at 100 kHz.
pub struct BitbangI2c {
    scl: OutputOpenDrain<'static>,
    sda: OutputOpenDrain<'static>,
}

#[derive(Debug, defmt::Format)]
pub enum Error {
    Nack,
    /// SCL held low past the clock-stretch budget.
    SclStuck,
}

impl BitbangI2c {
    pub fn new(scl: Peri<'static, PB1>, sda: Peri<'static, PB0>) -> Self {
        Self {
            scl: OutputOpenDrain::new(scl, Level::High, Speed::Low),
            sda: OutputOpenDrain::new(sda, Level::High, Speed::Low),
        }
    }

    fn delay(&self) {
        cortex_m::asm::delay(HALF_BIT_CYCLES);
    }

    /// Release SCL and wait out target clock stretching (bounded).
    fn scl_release(&mut self) -> Result<(), Error> {
        self.scl.set_high();
        for _ in 0..10_000u16 {
            if self.scl.is_high() {
                return Ok(());
            }
        }
        Err(Error::SclStuck)
    }

    /// Bus clear: 9 SCL pulses + STOP. Run after actuation-window glitches
    /// leave the target mid-byte — the software equivalent of the TWI
    /// re-init the ATmega bridge needed.
    pub fn bus_clear(&mut self) -> Result<(), Error> {
        self.sda.set_high();
        for _ in 0..9 {
            self.scl.set_low();
            self.delay();
            self.scl_release()?;
            self.delay();
        }
        self.stop()
    }

    fn start(&mut self) -> Result<(), Error> {
        self.sda.set_high();
        self.scl_release()?;
        self.delay();
        self.sda.set_low();
        self.delay();
        self.scl.set_low();
        Ok(())
    }

    fn stop(&mut self) -> Result<(), Error> {
        self.sda.set_low();
        self.delay();
        self.scl_release()?;
        self.delay();
        self.sda.set_high();
        self.delay();
        Ok(())
    }

    /// Clock out one byte, return Ok on target ACK.
    fn write_byte(&mut self, byte: u8) -> Result<(), Error> {
        for i in (0..8).rev() {
            if byte & (1 << i) != 0 {
                self.sda.set_high();
            } else {
                self.sda.set_low();
            }
            self.delay();
            self.scl_release()?;
            self.delay();
            self.scl.set_low();
        }
        // ACK bit
        self.sda.set_high();
        self.delay();
        self.scl_release()?;
        let ack = self.sda.is_low();
        self.delay();
        self.scl.set_low();
        if ack {
            Ok(())
        } else {
            Err(Error::Nack)
        }
    }

    fn read_byte(&mut self, ack: bool) -> Result<u8, Error> {
        let mut byte = 0u8;
        self.sda.set_high();
        for _ in 0..8 {
            self.delay();
            self.scl_release()?;
            byte = (byte << 1) | (self.sda.is_high() as u8);
            self.delay();
            self.scl.set_low();
        }
        if ack {
            self.sda.set_low();
        } else {
            self.sda.set_high();
        }
        self.delay();
        self.scl_release()?;
        self.delay();
        self.scl.set_low();
        self.sda.set_high();
        Ok(byte)
    }

    /// write reg pointer + payload
    pub fn write(&mut self, addr: u8, bytes: &[u8]) -> Result<(), Error> {
        let r = (|| {
            self.start()?;
            self.write_byte(addr << 1)?;
            for &b in bytes {
                self.write_byte(b)?;
            }
            Ok(())
        })();
        let stop = self.stop();
        r.and(stop)
    }

    /// write reg pointer, repeated-start, read payload
    pub fn write_read(&mut self, addr: u8, wr: &[u8], rd: &mut [u8]) -> Result<(), Error> {
        let r = (|| {
            self.start()?;
            self.write_byte(addr << 1)?;
            for &b in wr {
                self.write_byte(b)?;
            }
            self.start()?; // repeated start
            self.write_byte((addr << 1) | 1)?;
            let last = rd.len() - 1;
            for (i, slot) in rd.iter_mut().enumerate() {
                *slot = self.read_byte(i != last)?;
            }
            Ok(())
        })();
        let stop = self.stop();
        r.and(stop)
    }
}

#[embassy_executor::task]
pub async fn task(scl: Peri<'static, PB1>, sda: Peri<'static, PB0>) {
    let mut bus = BitbangI2c::new(scl, sda);
    let _ = bus.bus_clear();

    info!("lock: bit-bang bus up (target {:#04x})", LOCK_ADDR);
    loop {
        // Placeholder until the nonce/HMAC command flow is ported. The real
        // loop: wake transfer -> STATUS poll -> policy engine decides ->
        // NONCE read -> COMMAND write, with bus_clear() + retry on any Error
        // (expected during the lock's actuation windows).
        Timer::after_secs(3600).await;
    }
}
