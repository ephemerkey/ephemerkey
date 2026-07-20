//! I2C1 sensor bus (PB6 SCL / PB7 SDA): LIS3DH accel @0x18, OLED @0x3C,
//! M24M02E-F log EEPROM @0x50-0x53, MAX17048 fuel gauge @0x36 — plus the
//! LIS3DH interrupt pins. The bus is a shared blocking bus (see `main`), so the
//! generator's OLED lives on the same wires; this task holds a second
//! [`I2cDevice`](crate::I2c1Dev) handle to it.
//!
//! PB3 = INT1 (wake-on-motion, EXTI3), PA8 = INT2 (tamper/free-fall, EXTI8).
//!
//! Samples the accelerometer and publishes a **stillness** duration to
//! [`crate::motion`]; the policy engines read it for their `still_for_s` gate.
//! Motion (an INT edge or a supra-threshold sample delta) resets the timer.

use defmt::{info, warn};
use embassy_futures::select::{select3, Either3};
use embassy_stm32::exti::{self, ExtiInput};
use embassy_stm32::gpio::Pull;
use embassy_stm32::interrupt::typelevel;
use embassy_stm32::peripherals::{EXTI3, EXTI8, PA8, PB3};
use embassy_stm32::{bind_interrupts, Peri};
use embassy_time::Timer;
use embedded_hal::i2c::I2c;

use crate::motion;

bind_interrupts!(struct Irqs {
    EXTI2_3 => exti::InterruptHandler<typelevel::EXTI2_3>;   // PB3 = INT1
    EXTI4_15 => exti::InterruptHandler<typelevel::EXTI4_15>; // PA8 = INT2
});

const LIS3DH_ADDR: u8 = 0x18;
const LIS3DH_WHO_AM_I: u8 = 0x0F; // reads 0x33
const LIS3DH_CTRL_REG1: u8 = 0x20;
const LIS3DH_OUT: u8 = 0x28; // OUT_X_L..OUT_Z_H; MSB set = auto-increment
                             // 10 Hz, normal mode, X+Y+Z enabled
const LIS3DH_ODR_10HZ_XYZ: u8 = 0b0010_0111;

const MAX17048_ADDR: u8 = 0x36;
const MAX17048_VERSION: u8 = 0x08; // 16-bit BE, reads 0x001x
#[allow(dead_code)]
const MAX17048_VCELL: u8 = 0x02; // 78.125 uV/LSB
#[allow(dead_code)]
const MAX17048_SOC: u8 = 0x04; // 1/256 %/LSB

/// Accelerometer sample cadence.
const SAMPLE_MS: u64 = 250;
/// Sum-of-per-axis |Δ| (high byte, ~16 mg/LSB) at or below which the device is
/// "still". Coarse and bench-tunable — the intent is "sitting on a surface",
/// not metrology.
const STILL_DELTA: u16 = 3;

#[embassy_executor::task]
pub async fn task(
    mut dev: crate::I2c1Dev,
    int1: Peri<'static, PB3>,
    int2: Peri<'static, PA8>,
    exti3: Peri<'static, EXTI3>,
    exti8: Peri<'static, EXTI8>,
) {
    let mut who = [0u8; 1];
    match dev.write_read(LIS3DH_ADDR, &[LIS3DH_WHO_AM_I], &mut who) {
        Ok(()) => info!("lis3dh: WHO_AM_I = {:#04x}", who[0]),
        Err(e) => warn!("lis3dh: probe failed: {}", e),
    }
    // Bring the accel out of power-down so OUT_* update.
    if let Err(e) = dev.write(LIS3DH_ADDR, &[LIS3DH_CTRL_REG1, LIS3DH_ODR_10HZ_XYZ]) {
        warn!("lis3dh: enable failed: {}", e);
    }

    let mut ver = [0u8; 2];
    match dev.write_read(MAX17048_ADDR, &[MAX17048_VERSION], &mut ver) {
        Ok(()) => info!("max17048: VERSION = {:#06x}", u16::from_be_bytes(ver)),
        Err(e) => warn!("max17048: probe failed: {}", e),
    }

    // LIS3DH INTs push-pull active-high by default config.
    let mut int1 = ExtiInput::new(int1, exti3, Pull::Down, Irqs);
    let mut int2 = ExtiInput::new(int2, exti8, Pull::Down, Irqs);

    let mut prev: Option<[i8; 3]> = None;
    let mut still_ms: u32 = 0;
    loop {
        match select3(
            Timer::after_millis(SAMPLE_MS),
            int1.wait_for_rising_edge(),
            int2.wait_for_rising_edge(),
        )
        .await
        {
            // Periodic sample: accumulate stillness, or reset on movement.
            Either3::First(()) => {
                if let Some(s) = read_xyz(&mut dev) {
                    if let Some(p) = prev {
                        let d = axis_delta(s[0], p[0]) + axis_delta(s[1], p[1]) + axis_delta(s[2], p[2]);
                        if d <= STILL_DELTA {
                            still_ms += SAMPLE_MS as u32;
                        } else {
                            still_ms = 0;
                        }
                        motion::set_still_s(still_ms / 1000);
                    }
                    prev = Some(s);
                }
            }
            // Either interrupt is motion/tamper — the device is not still.
            Either3::Second(()) => {
                info!("lis3dh: INT1 (motion)");
                still_ms = 0;
                motion::set_still_s(0);
            }
            Either3::Third(()) => {
                info!("lis3dh: INT2 (tamper/free-fall)");
                still_ms = 0;
                motion::set_still_s(0);
            }
        }
    }
}

/// Read the three high-byte acceleration values (signed, ~16 mg/LSB).
fn read_xyz(dev: &mut crate::I2c1Dev) -> Option<[i8; 3]> {
    let mut b = [0u8; 6];
    dev.write_read(LIS3DH_ADDR, &[LIS3DH_OUT | 0x80], &mut b).ok()?;
    Some([b[1] as i8, b[3] as i8, b[5] as i8])
}

/// |a − b| for one axis, in high-byte counts (widened so i8::MIN can't wrap).
fn axis_delta(a: i8, b: i8) -> u16 {
    (a as i16 - b as i16).unsigned_abs()
}
