//! Generator I/O loop (bench stub): geofence-gated reveal + the 3-button
//! cascade ritual dial.
//!
//! Holds the [`Generator`] built from the sealed config, plus its cascade
//! ritual engine (a [`LockEngine`] over the config's slots). Two UI modes:
//!
//!   * **DIAL** (locked) — the three buttons dial a 4–8 digit ritual code:
//!     `●` decrements the current digit, `■` increments it, `◆` accepts a digit
//!     (tap), backspaces (double-tap), or submits the code (hold). A submitted
//!     code drives the ritual engine; a `Fired(unlock)` opens the reveal window
//!     (`Fired(duress)` opens a poison-only one) via [`apply_ritual_outcome`].
//!   * **UNLOCKED** — `●`/`■` select the key, `◆` REVEALs it. The window closes
//!     on its own after `unlock_window_s`, dropping back to DIAL.
//!
//! A generator with no ritual (no slots) starts UNLOCKED and behaves like the
//! original press-to-reveal generator; per-key gating still routes gated keys
//! through poison/refuse (see [`Generator::reveal`]).
//!
//! Each state renders through the shared [`ephemerkey_ui`] model and is blitted
//! to the SSD1306 OLED (feature `display`; see [`crate::display`]); the same
//! events are also logged over RTT for bench visibility. The bench `hw-aes`
//! build omits the panel for SRAM headroom (the `oled` cfg — see `build.rs`).

use defmt::info;
use embassy_stm32::gpio::Input;
use embassy_time::{Instant, Timer};
use ephemerkey_config::Calendars;
use ephemerkey_core::engine::{LockEngine, Outcome};
use ephemerkey_core::policy::Sensors;
use ephemerkey_core::reveal::{apply_ritual_outcome, Generator, RevealErr};

use crate::{clock, gate};
#[cfg(oled)]
use ephemerkey_ui::{render, Screen, View};

/// The OLED handle the task carries: the real display when built with `display`,
/// or nothing (unit) otherwise — so the task signature is stable across builds.
#[cfg(oled)]
type OledArg = Option<crate::display::Oled<'static>>;
#[cfg(not(oled))]
type OledArg = ();

const POLL_MS: u64 = 20;
/// Center-button gesture thresholds (ms): a press held this long is a submit;
/// a second tap arriving within the double window is a backspace.
const HOLD_MS: u64 = 600;
const DOUBLE_MS: u64 = 300;
/// Longest ritual code the dial accepts (schema allows 4–10 digits).
const MAX_DIAL: usize = 10;

/// The gate environment the ritual engine evaluates dialed codes against. The
/// generator has its own GNSS fence (via [`gate`]) and calendar table; the
/// stillness gate needs an accelerometer not wired to this task yet, so it
/// reads "still" (documented non-enforcement, matching the lock console).
struct GenSensors {
    calendars: Calendars,
    now: u64,
}
impl Sensors for GenSensors {
    fn inside_fence(&self, _fence: u8) -> bool {
        gate::in_fence()
    }
    fn still_for_s(&self) -> u32 {
        u32::MAX
    }
    fn calendar_open(&self, window: u8) -> bool {
        self.calendars.open(window, self.now)
    }
}

/// The three dial buttons in their design roles (`● ◆ ■`). SW1/PA5 and SW2/PA15
/// are active-low (internal pull-up); SW3/PF3 is active-high (pull-down, also
/// the BOOT0 strap).
pub struct DialButtons {
    pub left: Input<'static>,   // ●
    pub center: Input<'static>, // ◆
    pub right: Input<'static>,  // ■
}

#[embassy_executor::task]
pub async fn task(
    mut gen: Generator,
    mut ritual: LockEngine,
    calendars: Calendars,
    unlock_window_s: u32,
    buttons: DialButtons,
    #[cfg_attr(not(oled), allow(unused_variables, unused_mut))] mut oled: OledArg,
) {
    let DialButtons { left, center, right } = buttons;
    let has_ritual = ritual.slots.iter().any(|s| s.is_some());
    if has_ritual {
        info!("generator: cascade ritual armed — dial to unlock (●- ◆ok ■+, ◆hold submit)");
    } else {
        info!("generator: ready (◆ reveals, ●/■ select key)");
    }

    // A ritual-less generator has nothing to unlock: start ready-to-reveal.
    let mut unlocked = !has_ritual;
    let mut dial: heapless::Vec<u8, MAX_DIAL> = heapless::Vec::new();
    let mut cur: u8 = 0; // digit currently being adjusted
    let mut sel: usize = 0; // selected key (UNLOCKED mode)

    // Edge / gesture tracking.
    let (mut l_prev, mut c_prev, mut r_prev) = (false, false, false);
    let mut c_press: Option<Instant> = None; // center held since…
    let mut c_fired = false; // hold already actioned this press
    let mut pending_tap: Option<Instant> = None; // a tap awaiting a possible double

    // Display: the last screen blitted (so we only push the OLED on change) and
    // a revealed code held on screen for its `reveal_s`.
    #[cfg(oled)]
    let mut last_screen: Option<Screen> = None;
    #[cfg(oled)]
    let mut reveal_hold: Option<(heapless::String<10>, u16, Instant)> = None;

    loop {
        Timer::after_millis(POLL_MS).await;
        let now = clock::now_unix().unwrap_or(0);

        // The reveal window is authoritative: a ritual device follows it in and
        // out of UNLOCKED. (A ritual-less device stays UNLOCKED.)
        if has_ritual {
            let open = gen.is_unlocked(now);
            if open && !unlocked {
                unlocked = true;
                sel = 0;
            } else if !open && unlocked {
                unlocked = false;
                info!("generator: reveal window closed — locked");
            }
        }

        let l = left.is_low();
        let c = center.is_low();
        let r = right.is_high();

        // ● and ■: rising-edge actions. DIAL → adjust digit; UNLOCKED → select.
        if l && !l_prev {
            if unlocked {
                sel = sel.wrapping_sub(1) % gen.keys.len();
                info!("generator: key {}", sel);
            } else {
                cur = (cur + 9) % 10;
                info!("generator: dial [{}] {}", dial.len(), cur);
            }
        }
        if r && !r_prev {
            if unlocked {
                sel = (sel + 1) % gen.keys.len();
                info!("generator: key {}", sel);
            } else {
                cur = (cur + 1) % 10;
                info!("generator: dial [{}] {}", dial.len(), cur);
            }
        }

        // ◆ hold: fire as soon as the threshold passes (submit in DIAL mode).
        if c && !c_prev {
            c_press = Some(Instant::now());
            c_fired = false;
        }
        if c && !c_fired {
            if let Some(t) = c_press {
                if t.elapsed().as_millis() >= HOLD_MS {
                    c_fired = true;
                    if !unlocked {
                        submit(&mut gen, &mut ritual, &calendars, unlock_window_s, &dial);
                        dial.clear();
                        cur = 0;
                    }
                }
            }
        }
        // ◆ release: a short press is a tap; a tap within DOUBLE_MS is a double.
        if !c && c_prev {
            if !c_fired {
                match pending_tap {
                    Some(t0) if t0.elapsed().as_millis() < DOUBLE_MS => {
                        pending_tap = None;
                        center_double(unlocked, &mut dial); // backspace (DIAL)
                    }
                    _ => pending_tap = Some(Instant::now()),
                }
            }
            c_press = None;
        }
        // A tap that outlives the double window resolves as a single tap.
        if let Some(t0) = pending_tap {
            if t0.elapsed().as_millis() >= DOUBLE_MS {
                pending_tap = None;
                if unlocked {
                    let shown = reveal(&mut gen, sel, now);
                    #[cfg(oled)]
                    if let Some((code, secs)) = shown {
                        reveal_hold = Some((code, secs, Instant::now()));
                    }
                    #[cfg(not(oled))]
                    let _ = shown;
                } else if dial.push(cur).is_ok() {
                    info!("generator: dial accept {} (len {})", cur, dial.len());
                    cur = 0;
                } else {
                    info!("generator: dial full");
                }
            }
        }

        // Paint the OLED from the current state, only when the screen changes.
        #[cfg(oled)]
        {
            if let Some((_, secs, at)) = &reveal_hold {
                if at.elapsed().as_secs() as u16 >= *secs {
                    reveal_hold = None; // reveal duration elapsed
                }
            }
            let view = match &reveal_hold {
                Some((code, secs, at)) => View::Reveal {
                    code: code.as_str(),
                    secs_left: secs.saturating_sub(at.elapsed().as_secs() as u16),
                },
                None if unlocked && gate::may_emit() => {
                    View::Unlocked { key: sel as u8, secs_left: gen.unlock_secs_left(now) }
                }
                None if unlocked => View::Searching, // ready but no fresh fix
                None if has_ritual => View::Dialing { entered: dial.as_slice(), cur },
                None => View::Locked,
            };
            let screen = render(&view);
            if last_screen.as_ref() != Some(&screen) {
                if let Some(o) = oled.as_mut() {
                    o.show(&screen);
                }
                last_screen = Some(screen);
            }
        }

        (l_prev, c_prev, r_prev) = (l, c, r);
    }
}

/// ◆ double-tap = backspace (DIAL mode only).
fn center_double(unlocked: bool, dial: &mut heapless::Vec<u8, MAX_DIAL>) {
    if !unlocked {
        dial.pop();
        info!("generator: dial backspace (len {})", dial.len());
    }
}

/// Feed the dialed code to the ritual engine and apply its outcome to the
/// reveal window.
fn submit(
    gen: &mut Generator,
    ritual: &mut LockEngine,
    calendars: &Calendars,
    unlock_window_s: u32,
    dial: &heapless::Vec<u8, MAX_DIAL>,
) {
    if dial.is_empty() {
        return;
    }
    let mut s: heapless::String<MAX_DIAL> = heapless::String::new();
    for &d in dial.iter() {
        let _ = s.push((b'0' + d) as char);
    }
    let now = clock::now_unix().unwrap_or(0);
    let sensors = GenSensors { calendars: *calendars, now };
    let out = ritual.enter_code_with(s.as_str(), now, &sensors);
    apply_ritual_outcome(gen, &out, now, unlock_window_s);
    match out {
        Outcome::Fired(_, _) if gen.is_unlocked(now) => {
            info!("generator: RITUAL OK — reveal window open {}s", unlock_window_s)
        }
        Outcome::Progress(_, h, n) => info!("generator: ritual progress {}/{}", h, n),
        _ => info!("generator: ritual code rejected"),
    }
}

/// REVEAL the selected key, honoring the emission gate (fresh in-fence fix) and
/// the ritual gate inside [`Generator::reveal`] (real / poison / blank). Returns
/// the code + how long to show it, so the display can hold it (`None` = nothing
/// to show — refused, blank, or no fix).
fn reveal(gen: &mut Generator, sel: usize, now: u64) -> Option<(heapless::String<10>, u16)> {
    if !gate::may_emit() {
        info!("generator: refused — no fresh in-fence fix");
        return None;
    }
    if now == 0 {
        info!("generator: refused — clock undisciplined");
        return None;
    }
    let entropy = Instant::now().as_ticks() as u32;
    match gen.reveal(sel, now, entropy) {
        Ok(reveal) => {
            let mut buf = [0u8; 10];
            let code = reveal.code.render(&mut buf);
            info!("generator: key {} code {}", sel, code);
            let mut s: heapless::String<10> = heapless::String::new();
            let _ = s.push_str(code);
            Some((s, reveal.reveal_s))
        }
        Err(RevealErr::Locked) => {
            info!("generator: key {} locked (blank)", sel);
            None
        }
        Err(_) => {
            info!("generator: key {} has no reveal (unconfigured or refused)", sel);
            None
        }
    }
}
