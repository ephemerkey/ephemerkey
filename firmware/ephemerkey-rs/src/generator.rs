//! Generator I/O loop (bench stub).
//!
//! Holds the [`Generator`] built from the sealed config and, on a button press,
//! reveals key 0's code — but only when [`gate::may_emit`] holds (a fresh,
//! in-fence GNSS fix). With no display wired yet, the reveal is logged over RTT;
//! the real device drives the code onto its display here. Everything up to that
//! last step — the policy, the reveal scheduler, the emission gate — is the
//! shipping logic, exercised live.

use defmt::info;
use embassy_stm32::gpio::Input;
use embassy_time::{Instant, Timer};
use ephemerkey_core::reveal::Generator;

use crate::{clock, gate};

/// Poll interval for the (debounced) reveal button.
const POLL_MS: u64 = 40;

#[embassy_executor::task]
pub async fn task(mut gen: Generator, button: Input<'static>) {
    info!("generator: ready (press the button to reveal key 0)");
    let mut was_pressed = false;
    loop {
        Timer::after_millis(POLL_MS).await;
        let pressed = button.is_low(); // active-low
        if pressed && !was_pressed {
            reveal_key0(&mut gen);
        }
        was_pressed = pressed;
    }
}

fn reveal_key0(gen: &mut Generator) {
    if !gate::may_emit() {
        info!("generator: refused — no fresh in-fence fix");
        return;
    }
    let Some(now) = clock::now_unix() else {
        info!("generator: refused — clock undisciplined");
        return;
    };
    // Entropy for the scatter-reveal digit order; a monotonic tick is plenty
    // for the presentation shuffle (it gates nothing security-relevant).
    let entropy = Instant::now().as_ticks() as u32;
    match gen.reveal(0, now, entropy) {
        Ok(reveal) => {
            let mut buf = [0u8; 10];
            info!("generator: code {}", reveal.code.render(&mut buf));
        }
        Err(_) => info!("generator: key 0 has no reveal (unconfigured or refused)"),
    }
}
