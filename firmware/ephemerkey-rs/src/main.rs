//! ephemerkey — GPS-geofenced TOTP (RFC 6238) code generator.
//!
//! STM32U083KCU6 (Cortex-M0+, UFQFPN-32), Embassy async runtime.
//! The pin assignments below mirror DESIGN.md "Pin Budget" exactly; the
//! 32-pin package has no spare GPIO, and every binding here is type-checked
//! against the chip's AF table by embassy-stm32 at compile time.
//!
//! One firmware, two PERSONALITIES selected by persistent config (see
//! `config::Role`): a geofenced TOTP **Generator**, or a **LockController**
//! that validates codes and drives the companion lock board over I2C. Both
//! are provisioned by files delivered over USB or WiFi (`provision`).
//!
//! Architecture: one async task per subsystem instead of the C superloop.
//! Sleep is the executor's WFI; Stop-mode entry is a later step (see README).

#![no_std]
#![no_main]

use defmt::info;
use defmt_rtt as _;
use panic_probe as _;

use embassy_executor::Spawner;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_time::Timer;

mod buzzer;
mod config;
mod gnss;
mod lock;
mod policy;
mod provision;
mod sensors;
mod wifi;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let config = embassy_stm32::Config::default();
    // TODO(clocks): LSE 32.768 kHz -> RTC (the TOTP timebase, disciplined from
    // GNSS PPS); HSI48 + CRS (SOF-trimmed) for the crystal-less USB FS device.
    // Default config boots on HSI16 which is fine for bringup.
    let p = embassy_stm32::init(config);

    let cfg = config::load();
    info!("ephemerkey-rs boot, role: {}", cfg.role);

    // PA6 green = in-fence / code-valid, PA7 red = out-of-fence / fault.
    let led_green = Output::new(p.PA6, Level::Low, Speed::Low);
    let _led_red = Output::new(p.PA7, Level::Low, Speed::Low);

    // PA0 = GNSS PPS. Held as a plain input until the TIM2_CH1 input-capture
    // RTC-discipline path lands.
    let _pps = Input::new(p.PA0, Pull::None);

    // PA11/PA12 = USB_DM/DP — intentionally unclaimed until the USB
    // provisioning console lands. NOTE (DESIGN.md): confirm the SYSCFG
    // PA11/PA12 pin-pair remap stays DISABLED so pins 21/22 carry USB while
    // PA9/PA10 keep USART1.

    // Subsystems common to both personalities.
    spawner.spawn(wifi::task(p.LPUART1, p.PA2, p.PA3, p.PB5).unwrap());
    spawner.spawn(sensors::task(p.I2C1, p.PB6, p.PB7, p.PB3, p.PA8, p.EXTI3, p.EXTI8).unwrap());
    spawner.spawn(buzzer::task(p.TIM3, p.PB4).unwrap());

    // Role-specific pipeline. The GNSS module is only meaningful on the
    // Generator; the lock link only on the LockController. (Hardware is
    // identical — unused subsystems stay unpowered/parked.)
    match cfg.role {
        config::Role::Generator => {
            spawner.spawn(gnss::task(p.USART1, p.PA9, p.PA10, p.PA4, p.PA1, p.DMA1_CH1).unwrap());
        }
        config::Role::LockController => {
            // Bit-banged open-drain master — PB0/PB1 have no I2C AF on the
            // U083 (see lock.rs).
            spawner.spawn(lock::task(p.PB1, p.PB0).unwrap());
        }
    }
    spawner.spawn(buttons(Input::new(p.PA5, Pull::Up), Input::new(p.PA15, Pull::Up)).unwrap());

    heartbeat(led_green).await;
}

/// 1 Hz green-LED heartbeat — bringup placeholder for the real
/// in-fence / code-valid indication.
async fn heartbeat(mut led: Output<'static>) -> ! {
    loop {
        led.set_high();
        Timer::after_millis(50).await;
        led.set_low();
        Timer::after_millis(950).await;
    }
}

/// SW1 (PA5) / SW2 (PA15), active-low with internal pull-ups.
/// Polled for now; EXTI5/EXTI15 are free if edge wakes are wanted later.
#[embassy_executor::task]
async fn buttons(sw1: Input<'static>, sw2: Input<'static>) {
    let mut last = (false, false);
    loop {
        let now = (sw1.is_low(), sw2.is_low());
        if now.0 && !last.0 {
            info!("SW1 pressed");
        }
        if now.1 && !last.1 {
            info!("SW2 pressed");
        }
        last = now;
        Timer::after_millis(20).await;
    }
}
