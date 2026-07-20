//! MAX-M10S GNSS: USART1 NMEA link + reset / EXTINT control + 1 PPS.
//!
//! Parses RMC sentences off the UART and disciplines the RTC ([`crate::clock`])
//! whenever the receiver reports a valid fix, so the TOTP time base tracks GNSS
//! UTC. The 1 PPS TIMEPULSE (PA0) is watched concurrently as a liveness/second
//! marker.
//!
//! Coarse for now: the clock is set from the RMC timestamp (good to well under
//! the 30 s TOTP window). Sub-second alignment — applying the pending UTC
//! exactly on the PPS edge, or nudging RTC SHIFTR — is the next refinement; the
//! PPS edge is already surfaced here for it.

use defmt::{info, warn};
use embassy_futures::select::{select, Either};
use embassy_stm32::exti::{self, ExtiInput};
use embassy_stm32::gpio::{Level, Output, OutputOpenDrain, Pull, Speed};
use embassy_stm32::interrupt::typelevel;
use embassy_stm32::peripherals::{DMA1_CH1, EXTI0, PA0, PA1, PA10, PA4, PA9, USART1};
use embassy_stm32::usart::{self, UartRx};
use embassy_stm32::{bind_interrupts, dma, interrupt, Peri};
use embassy_time::Timer;

use crate::{clock, gate};

bind_interrupts!(struct Irqs {
    USART1 => usart::InterruptHandler<USART1>;
    DMA1_CHANNEL1 => dma::InterruptHandler<DMA1_CH1>;
    EXTI0_1 => exti::InterruptHandler<typelevel::EXTI0_1>; // PA0 = 1 PPS
});

// Silence the "unused typelevel interrupt" path — bind_interrupts consumes it.
#[allow(unused_imports)]
use interrupt::typelevel::Interrupt as _;

// An NMEA sentence is at most 82 chars incl. the leading '$' and trailing CRLF.
const LINE_MAX: usize = 96;

#[embassy_executor::task]
pub async fn task(
    uart: Peri<'static, USART1>,
    tx: Peri<'static, PA9>,
    rx: Peri<'static, PA10>,
    reset_n: Peri<'static, PA4>,
    extint: Peri<'static, PA1>,
    rx_dma: Peri<'static, DMA1_CH1>,
    pps: Peri<'static, PA0>,
    pps_exti: Peri<'static, EXTI0>,
) {
    // RESET_N: open-drain into the module (it has its own pull-up).
    // EXTINT: wake / time-mark output to the module.
    let mut reset_n = OutputOpenDrain::new(reset_n, Level::Low, Speed::Low);
    let _extint = Output::new(extint, Level::Low, Speed::Low);

    // TX (PA9) is parked until we send UBX config; claiming it here keeps the
    // pin owned by this task.
    let _tx = tx;

    // 1 PPS TIMEPULSE on PA0 (external signal; no internal pull).
    let mut pps = ExtiInput::new(pps, pps_exti, Pull::None, Irqs);

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
    let mut line = [0u8; LINE_MAX];
    let mut ll = 0usize;
    loop {
        match select(rx.read_until_idle(&mut buf), pps.wait_for_rising_edge()).await {
            Either::First(Ok(n)) => feed(&buf[..n], &mut line, &mut ll),
            Either::First(Err(e)) => warn!("gnss: rx error {}", e),
            Either::Second(()) => {
                // PPS rising edge: the receiver is alive and marking the UTC
                // second boundary. TODO: apply the pending fix exactly here for
                // sub-second accuracy. Coarse RMC discipline is already active.
            }
        }
    }
}

/// Accumulate UART bytes into NMEA lines; on each complete line, parse RMC and
/// discipline the clock on a valid fix.
fn feed(chunk: &[u8], line: &mut [u8; LINE_MAX], ll: &mut usize) {
    for &b in chunk {
        if b == b'\n' || b == b'\r' {
            if *ll > 0 {
                if let Some(fix) = ephemerkey_nmea::parse_rmc(&line[..*ll]) {
                    gate::set_fix_valid(fix.valid);
                    if fix.valid {
                        clock::discipline_utc(fix.year, fix.month, fix.day, fix.hour, fix.min, fix.sec);
                    }
                }
                *ll = 0;
            }
        } else if *ll < LINE_MAX {
            line[*ll] = b;
            *ll += 1;
        } else {
            *ll = 0; // oversized/garbled line — drop it
        }
    }
}
