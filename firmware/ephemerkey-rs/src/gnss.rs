//! MAX-M10S GNSS: USART1 NMEA/UBX link + reset / EXTINT control pins.
//!
//! Scaffold: releases reset, then logs received chunk sizes. The real task
//! will parse RMC/GGA/GSA, run the geofence test, and hand PPS-disciplined
//! time to the RTC.

use defmt::{info, warn};
use embassy_stm32::gpio::{Level, Output, OutputOpenDrain, Speed};
use embassy_stm32::peripherals::{DMA1_CH1, PA1, PA10, PA4, PA9, USART1};
use embassy_stm32::usart::{self, UartRx};
use embassy_stm32::{bind_interrupts, dma, interrupt, Peri};
use embassy_time::Timer;

bind_interrupts!(struct Irqs {
    USART1 => usart::InterruptHandler<USART1>;
    DMA1_CHANNEL1 => dma::InterruptHandler<DMA1_CH1>;
});

// Silence the "unused typelevel interrupt" path — bind_interrupts consumes it.
#[allow(unused_imports)]
use interrupt::typelevel::Interrupt as _;

#[embassy_executor::task]
pub async fn task(
    uart: Peri<'static, USART1>,
    tx: Peri<'static, PA9>,
    rx: Peri<'static, PA10>,
    reset_n: Peri<'static, PA4>,
    extint: Peri<'static, PA1>,
    rx_dma: Peri<'static, DMA1_CH1>,
) {
    // RESET_N: open-drain into the module (it has its own pull-up).
    // EXTINT: wake / time-mark output to the module.
    let mut reset_n = OutputOpenDrain::new(reset_n, Level::Low, Speed::Low);
    let _extint = Output::new(extint, Level::Low, Speed::Low);

    // TX (PA9) is parked until we send UBX config; claiming it here keeps the
    // pin owned by this task.
    let _tx = tx;

    // Hold the module in reset briefly, then release.
    Timer::after_millis(10).await;
    reset_n.set_high();
    info!("gnss: reset released");

    // M10 default: 9600 baud NMEA, RX on DMA.
    let mut cfg = usart::Config::default();
    cfg.baudrate = 9600;
    let mut rx: UartRx<'static, _> = match UartRx::new(uart, rx, rx_dma, Irqs, cfg) {
        Ok(r) => r,
        Err(_) => {
            warn!("gnss: uart init failed");
            return;
        }
    };

    let mut buf = [0u8; 128];
    loop {
        match rx.read_until_idle(&mut buf).await {
            Ok(n) => info!("gnss: {} bytes", n),
            Err(e) => warn!("gnss: rx error {}", e),
        }
    }
}
