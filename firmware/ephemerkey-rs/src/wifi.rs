//! ESP32-C3 WiFi co-processor link: LPUART1 (PA2 TX / PA3 RX) + PB5 power gate.
//!
//! LPUART1 is deliberate — it can wake the MCU from Stop on RX. PB5 drives the
//! AP2112K EN pin (100k pulldown on the board): the ESP is UNPOWERED by
//! default and only brought up for OTA / provisioning sessions.
//!
//! Scaffold: claims the pins, keeps the rail off, and parks.

use defmt::info;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::mode::Blocking;
use embassy_stm32::peripherals::{LPUART1, PA2, PA3, PB5};
use embassy_stm32::usart::{self, Uart};
use embassy_stm32::Peri;
use embassy_time::Timer;

#[embassy_executor::task]
pub async fn task(
    uart: Peri<'static, LPUART1>,
    tx: Peri<'static, PA2>,
    rx: Peri<'static, PA3>,
    pwr: Peri<'static, PB5>,
) {
    // WiFi off by default — matches the board's 100k pulldown on EN.
    let _pwr = Output::new(pwr, Level::Low, Speed::Low);

    let mut cfg = usart::Config::default();
    cfg.baudrate = 115_200;
    let _uart: Option<Uart<'static, Blocking>> = Uart::new_blocking(uart, rx, tx, cfg).ok();

    info!("wifi: link claimed, ESP rail off");
    loop {
        // Placeholder until the OTA/provisioning protocol lands.
        Timer::after_secs(3600).await;
    }
}
