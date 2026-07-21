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
#[cfg(feature = "usb-provision")]
use embassy_stm32::usb;
use embassy_time::Timer;

// Policy engine + TOTP + reveal scheduler live in the shared no_std core,
// exercised on-host by the emulator (../ephemerkey-emu) and tests.
use ephemerkey_core as _;

// Some clock/gate API is only exercised on the generator (gnss) build; keep it
// across configurations even when a given board doesn't call it.
#[allow(dead_code)]
mod clock;
mod config;
#[allow(dead_code)]
mod gate;
#[cfg(feature = "hw-aes")]
mod pacaes;
mod provision;
#[cfg(feature = "usb-provision")]
mod usbcdc;
#[cfg(feature = "usb-provision")]
mod usbprov;

#[cfg(feature = "buzzer")]
mod buzzer;
#[cfg(oled)]
mod display;
#[cfg(feature = "gnss")]
mod generator;
#[cfg(feature = "gnss")]
mod gnss;
#[cfg(feature = "lock")]
mod lock;
#[cfg(any(feature = "gnss", feature = "lock", feature = "sensors"))]
mod motion;
#[cfg(all(feature = "lock", feature = "usb-provision"))]
mod lockconsole;
#[cfg(feature = "sensors")]
mod sensors;
#[cfg(feature = "wifi")]
mod wifi;

// Shared blocking I2C1 bus (PB6/PB7): the generator's OLED and the accel
// sampler each hold an `I2c1Dev` handle to it. Single-executor, cooperative —
// a blocking transaction never awaits — so a `NoopRawMutex` is sound and, unlike
// a critical-section mutex, doesn't disable interrupts across the (multi-ms)
// OLED flush.
#[cfg(feature = "sensors")]
use embassy_embedded_hal::shared_bus::blocking::i2c::I2cDevice;
#[cfg(feature = "sensors")]
use embassy_sync::blocking_mutex::{raw::NoopRawMutex, Mutex as BlockingMutex};
#[cfg(feature = "sensors")]
use static_cell::StaticCell;
#[cfg(feature = "sensors")]
type I2c1Raw =
    embassy_stm32::i2c::I2c<'static, embassy_stm32::mode::Blocking, embassy_stm32::i2c::mode::Master>;
#[cfg(feature = "sensors")]
pub type I2c1Bus = BlockingMutex<NoopRawMutex, core::cell::RefCell<I2c1Raw>>;
#[cfg(feature = "sensors")]
pub type I2c1Dev = I2cDevice<'static, NoopRawMutex, I2c1Raw>;

bind_interrupts!(pub struct Irqs {
    RNG_CRYP => rng::InterruptHandler<peripherals::RNG>;
    #[cfg(feature = "usb-provision")]
    USB_DRD_FS => usb::InterruptHandler<peripherals::USB>;
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

    // Device configuration (role, geofence zones, freshness window) from the
    // newest committed journal record; the shut default on a fresh/unparseable
    // config. Apply the freshness window to the emission gate.
    let cfg = config::load(&journal);
    info!(
        "ephemerkey-rs boot, role: {}, zones: {}, staleness: {}s",
        cfg.role,
        cfg.zones().len(),
        cfg.staleness_s
    );
    gate::configure(&cfg);

    // --- Provisioning mode (button held at boot) ----------------------------
    // "Hold the provisioning button while connecting": only in that case do we
    // bring up the USB device and accept a sealed config. Otherwise the device
    // is not even a USB peripheral — it cannot be silently rewritten. Checked
    // FIRST so this branch can diverge, leaving the USB peripheral free for the
    // lock console on the normal path.
    #[cfg(feature = "usb-provision")]
    if prov_button.is_low() {
        info!("entering USB provisioning mode");
        spawner.spawn(usbprov::task(p.USB, p.PA12, p.PA11, p.AES, journal, identity).unwrap());
        let mut led = status_led;
        led.set_high(); // solid = provisioning
        loop {
            Timer::after_secs(3600).await;
        }
    }

    // --- Normal run ---------------------------------------------------------
    // The identity/journal aren't consumed by a running task here — drop them.
    let _ = (journal, identity);

    // Peripheral power gate (PERI_EN, PA8, active-high): the load switch (Q3)
    // that feeds +3V3_SW to the OLED / EEPROM / I2C1 pull-ups. Assert it before
    // the I2C1 bus so those peripherals are powered; the always-on rail keeps the
    // accel (wake source) and the fuel gauge alive regardless. Held for the
    // program's life (main never returns). PA8 was the accel's INT2 — that 2nd
    // alarm channel was dropped to free this pin; tamper now rides INT1.
    #[cfg(feature = "sensors")]
    let _peri_en = Output::new(p.PA8, Level::High, Speed::Low);

    // Subsystems common to both personalities (product board only). The I2C1
    // sensor bus is mounted once here and SHARED: the accel sampler runs for
    // both roles (publishing stillness to `motion`), and the generator also puts
    // its OLED on the same bus (below).
    #[cfg(feature = "sensors")]
    let i2c1_bus: &'static I2c1Bus = {
        let mut ic = embassy_stm32::i2c::Config::default();
        ic.frequency = embassy_stm32::time::Hertz::khz(400);
        let i2c = embassy_stm32::i2c::I2c::new_blocking(p.I2C1, p.PB6, p.PB7, ic);
        static BUS: StaticCell<I2c1Bus> = StaticCell::new();
        BUS.init(BlockingMutex::new(core::cell::RefCell::new(i2c)))
    };
    #[cfg(feature = "sensors")]
    spawner.spawn(sensors::task(I2cDevice::new(i2c1_bus), p.PB3, p.EXTI3).unwrap());
    #[cfg(feature = "wifi")]
    spawner.spawn(wifi::task(p.LPUART1, p.PA2, p.PA3, p.PB5).unwrap());
    #[cfg(feature = "buzzer")]
    spawner.spawn(buzzer::task(p.TIM3, p.PB4).unwrap());

    // Role-specific pipeline. The engine is built from the sealed config by the
    // shared `ephemerkey-config` crate — the exact `Generator` / `LockEngine`
    // the emulator runs. (Nucleo has neither GNSS nor the lock link, so both
    // arms are off there and the device just blinks "searching".)
    match cfg.role {
        #[cfg(feature = "gnss")]
        config::Role::Generator => {
            // GNSS disciplines the clock + drives the geofence gate; the
            // generator task reveals key 0 on a button press when the gate is open.
            spawner.spawn(
                gnss::task(p.USART1, p.PA9, p.PA10, p.PA4, p.PA1, p.DMA1_CH1, p.PA0, p.EXTI0)
                    .unwrap(),
            );
            let gen = ephemerkey_config::build_generator(&cfg);
            let ritual = ephemerkey_config::build_ritual(&cfg);
            // The 3-button cascade dial: ● SW1 (PA5, the prov button on the
            // normal path), ◆ SW2 (PA15), ■ SW3 (PF3, active-high / BOOT0).
            let buttons = generator::DialButtons {
                left: prov_button,
                center: Input::new(p.PA15, Pull::Up),
                right: Input::new(p.PF3, Pull::Down),
            };
            // The dial/reveal OLED shares the I2C1 bus (0x3C). `None` if the
            // panel isn't populated — the generator still runs, logging.
            #[cfg(oled)]
            let oled = display::Oled::new(I2cDevice::new(i2c1_bus));
            #[cfg(not(oled))]
            let oled = ();
            spawner.spawn(
                generator::task(gen, ritual, cfg.calendars(), cfg.unlock_window_s, buttons, oled)
                    .unwrap(),
            );
        }
        #[cfg(feature = "lock")]
        config::Role::LockController => {
            // The actuator bus (stub) + the validation engine, fed codes over
            // the USB console (stand-in for the LOCK I2C code path). The accel
            // (stillness / tamper) runs commonly for both roles, above.
            spawner.spawn(lock::task(p.I2C3, p.PA7, p.PA6).unwrap());
            #[cfg(feature = "usb-provision")]
            {
                let engine = ephemerkey_config::build_lock(&cfg);
                let _validator = ephemerkey_config::build_validator(&cfg);
                spawner.spawn(
                    lockconsole::task(p.USB, p.PA12, p.PA11, engine, cfg.calendars()).unwrap(),
                );
            }
        }
        #[allow(unreachable_patterns)]
        _ => {}
    }

    status_indicator(status_led).await;
}

/// Status LED: solid while the device is ready to emit codes (valid fix + fresh
/// clock, per `gate::may_emit`), otherwise a 1 Hz "searching" blink. On a board
/// without GNSS the gate never opens, so it simply blinks.
async fn status_indicator(mut led: Output<'static>) -> ! {
    loop {
        if gate::may_emit() {
            led.set_high();
            Timer::after_millis(500).await;
        } else {
            led.set_high();
            Timer::after_millis(50).await;
            led.set_low();
            Timer::after_millis(950).await;
        }
    }
}
