//! ephemerkey — GPS-geofenced TOTP (RFC 6238) code generator.
//!
//! One firmware, two build targets sharing the same STM32U0x3 die:
//!   - `board-ephemerkey`  → STM32U083KCU6 (UFQFPN-32), the product board.
//!   - `board-nucleo-u083` → STM32U083RCT6 (NUCLEO-U083RC), for bench bring-up
//!     of the flash-journal / identity / USB-provisioning path with no GNSS,
//!     ESP32, lock, or sensors attached.
//!
//! Board selection (a Cargo feature) picks the embassy chip feature and the two
//! board-specific pins below; the on-board subsystems (`gnss`, `wifi`, `lock`,
//! `sensors`, `buzzer`) are each behind their own feature so a bare Nucleo
//! compiles and links without them. The store/identity/USB code is identical
//! across both.
//!
//! Architecture: one async task per subsystem. Sleep is the executor's WFI;
//! Stop-mode entry is a later step (see README).

#![no_std]
#![no_main]

use defmt::info;
use defmt_rtt as _;
use panic_probe as _;

use embassy_executor::Spawner;
use embassy_stm32::flash::Flash;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::rng::Rng;
use embassy_stm32::{bind_interrupts, peripherals, rng};
use embassy_time::Timer;

// Policy engine + TOTP + reveal scheduler live in the shared no_std core,
// exercised on-host by the emulator (../ephemerkey-emu) and tests.
use ephemerkey_core as _;

// `discipline_from_unix` / `is_fresh` are the clock API the GNSS pipeline will
// call once NMEA UTC parsing lands; keep them even though unused today.
#[allow(dead_code)]
mod clock;
mod config;
mod provision;
#[cfg(feature = "usb-provision")]
mod usbprov;

#[cfg(feature = "buzzer")]
mod buzzer;
#[cfg(feature = "gnss")]
mod gnss;
#[cfg(feature = "lock")]
mod lock;
#[cfg(feature = "sensors")]
mod sensors;
#[cfg(feature = "wifi")]
mod wifi;

bind_interrupts!(struct Irqs {
    RNG_CRYP => rng::InterruptHandler<peripherals::RNG>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    #[allow(unused_mut)]
    let mut config = embassy_stm32::Config::default();
    // TODO(clocks): LSE 32.768 kHz -> RTC (the TOTP timebase, disciplined from
    // GNSS PPS). For USB we run the crystal-less path: HSI48 trimmed by CRS off
    // the USB SOF. embassy's U0 Config::default() already enables HSI48; ask CRS
    // to sync from USB so it stays within the ±0.25% FS spec without a crystal.
    #[cfg(feature = "usb-provision")]
    {
        config.rcc.hsi48 = Some(embassy_stm32::rcc::Hsi48Config { sync_from_usb: true });
    }
    // RTC clock source (TOTP time base). The product board has the 32.768 kHz
    // LSE crystal (PC14/PC15); the Nucleo has no guaranteed crystal, so fall
    // back to the LSI. DESIGN.md Open Question 8 tracks LSE-vs-LSI accuracy.
    #[cfg(feature = "board-ephemerkey")]
    {
        config.rcc.ls = embassy_stm32::rcc::LsConfig::default_lse();
    }
    #[cfg(feature = "board-nucleo-u083")]
    {
        config.rcc.ls = embassy_stm32::rcc::LsConfig::default_lsi();
    }
    let p = embassy_stm32::init(config);

    // TOTP time base. Undisciplined until the GNSS provides UTC; `now()` reads 0
    // until then (see clock::now_unix / is_fresh).
    clock::init(p.RTC);

    let cfg = config::load();
    info!("ephemerkey-rs boot, role: {}", cfg.role);

    // --- Board-specific pins -------------------------------------------------
    // status LED (heartbeat / provisioning-mode indicator) and the provisioning
    // button (active-low). Everything else is board-independent.
    #[cfg(feature = "board-ephemerkey")]
    let (status_led, prov_button) = (
        Output::new(p.PB0, Level::Low, Speed::Low), // green, in-fence/code-valid
        Input::new(p.PA5, Pull::Up),                // SW1
    );
    #[cfg(feature = "board-nucleo-u083")]
    let (status_led, prov_button) = (
        Output::new(p.PA5, Level::Low, Speed::Low), // LD2 (green)
        Input::new(p.PC13, Pull::Up),               // B1 user button
    );

    // --- Persistent store + device identity ---------------------------------
    // Mount the flash journal and load (or, first boot, mint from the TRNG and
    // persist) the device identity. Provisioning stays gated until this holds a
    // real, durable identity — no more zero-key placeholders.
    let flash = Flash::new_blocking(p.FLASH);
    let (journal, identity) = provision::mount_and_identity(flash, |buf| {
        let mut rng = Rng::new(p.RNG, Irqs);
        rng.fill_bytes(buf);
    });
    info!("identity ready");

    // --- Provisioning mode (button held at boot) ----------------------------
    // "Hold the provisioning button while connecting": only in that case do we
    // bring up the USB device and accept a sealed config. Otherwise the device
    // is not even a USB peripheral — it cannot be silently rewritten.
    #[cfg(feature = "usb-provision")]
    let provisioning = prov_button.is_low();
    #[cfg(not(feature = "usb-provision"))]
    let _ = prov_button;

    // Subsystems common to both personalities (product board only).
    #[cfg(feature = "wifi")]
    spawner.spawn(wifi::task(p.LPUART1, p.PA2, p.PA3, p.PB5).unwrap());
    #[cfg(feature = "sensors")]
    spawner.spawn(sensors::task(p.I2C1, p.PB6, p.PB7, p.PB3, p.PA8, p.EXTI3, p.EXTI8).unwrap());
    #[cfg(feature = "buzzer")]
    spawner.spawn(buzzer::task(p.TIM3, p.PB4).unwrap());

    // Role-specific pipeline (product board only; the Nucleo has neither GNSS
    // nor the lock link wired, so both features are off there).
    match cfg.role {
        #[cfg(feature = "gnss")]
        config::Role::Generator => {
            spawner.spawn(gnss::task(p.USART1, p.PA9, p.PA10, p.PA4, p.PA1, p.DMA1_CH1).unwrap());
        }
        #[cfg(feature = "lock")]
        config::Role::LockController => {
            spawner.spawn(lock::task(p.I2C3, p.PA7, p.PA6).unwrap());
        }
        #[allow(unreachable_patterns)]
        _ => {}
    }

    #[cfg(feature = "usb-provision")]
    if provisioning {
        info!("entering USB provisioning mode");
        spawner.spawn(usbprov::task(p.USB, p.PA12, p.PA11, journal, identity).unwrap());
        // Solid LED = provisioning; park here (the USB task owns the work). This
        // branch diverges, so `status_led` is never needed by the heartbeat
        // below on this path.
        let mut led = status_led;
        led.set_high();
        loop {
            Timer::after_secs(3600).await;
        }
    }

    // Normal run. The identity/journal aren't consumed by a running task in this
    // build path — drop them explicitly so there's no unused-binding noise.
    let _ = (journal, identity);
    heartbeat(status_led).await;
}

/// 1 Hz status-LED heartbeat — bring-up placeholder for the real
/// in-fence / code-valid indication.
async fn heartbeat(mut led: Output<'static>) -> ! {
    loop {
        led.set_high();
        Timer::after_millis(50).await;
        led.set_low();
        Timer::after_millis(950).await;
    }
}
