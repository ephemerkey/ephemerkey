//! I2C1 sensor bus (PB6 SCL / PB7 SDA): LIS3DH accel @0x18, OLED, M24M02E-F
//! log EEPROM @0x50-0x53 — plus the LIS3DH interrupt pins.
//!
//! PB3 = INT1 (wake-on-motion, EXTI3), PA8 = INT2 (tamper/free-fall, EXTI8).
//!
//! Scaffold: probes the LIS3DH WHO_AM_I register, then logs interrupt edges.

use defmt::{info, warn};
use embassy_futures::select::{select, Either};
use embassy_stm32::exti::{self, ExtiInput};
use embassy_stm32::gpio::Pull;
use embassy_stm32::i2c::I2c;
use embassy_stm32::interrupt::typelevel;
use embassy_stm32::peripherals::{EXTI3, EXTI8, I2C1, PA8, PB3, PB6, PB7};
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, Peri};

bind_interrupts!(struct Irqs {
    EXTI2_3 => exti::InterruptHandler<typelevel::EXTI2_3>;   // PB3 = INT1
    EXTI4_15 => exti::InterruptHandler<typelevel::EXTI4_15>; // PA8 = INT2
});

const LIS3DH_ADDR: u8 = 0x18;
const LIS3DH_WHO_AM_I: u8 = 0x0F; // reads 0x33

#[embassy_executor::task]
pub async fn task(
    i2c: Peri<'static, I2C1>,
    scl: Peri<'static, PB6>,
    sda: Peri<'static, PB7>,
    int1: Peri<'static, PB3>,
    int2: Peri<'static, PA8>,
    exti3: Peri<'static, EXTI3>,
    exti8: Peri<'static, EXTI8>,
) {
    let mut i2c_cfg = embassy_stm32::i2c::Config::default();
    i2c_cfg.frequency = Hertz::khz(100);
    let mut bus = I2c::new_blocking(i2c, scl, sda, i2c_cfg);

    let mut who = [0u8; 1];
    match bus.blocking_write_read(LIS3DH_ADDR, &[LIS3DH_WHO_AM_I], &mut who) {
        Ok(()) => info!("lis3dh: WHO_AM_I = {:#04x}", who[0]),
        Err(e) => warn!("lis3dh: probe failed: {}", e),
    }

    // LIS3DH INTs push-pull active-high by default config.
    let mut int1 = ExtiInput::new(int1, exti3, Pull::Down, Irqs);
    let mut int2 = ExtiInput::new(int2, exti8, Pull::Down, Irqs);
    loop {
        match select(int1.wait_for_rising_edge(), int2.wait_for_rising_edge()).await {
            Either::First(()) => info!("lis3dh: INT1 (motion)"),
            Either::Second(()) => info!("lis3dh: INT2 (tamper/free-fall)"),
        }
    }
}
