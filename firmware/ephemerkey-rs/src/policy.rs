//! Code-slot policy engine (LockController personality) — typed sketch.
//!
//! Design doc: `DESIGN-policies.md` (repo root). Summary: a lock holds
//! parallel, independent CODE SLOTS; each slot has its own secret (or
//! zone-key), its own policy state machine, and its own action. Codes are
//! tried against every slot; timing violations and invalid codes reset
//! per-slot state; each TOTP period counts at most once per slot.
//!
//! Scaffold: types + state-machine skeleton, no TOTP validation yet (that
//! lands with the RustCrypto hmac/sha1 port of smalltotp).

#![allow(dead_code)]

use defmt::Format;

pub const MAX_SLOTS: usize = 8;

/// Gates that must hold for codes to count toward a slot (composable).
#[derive(Copy, Clone, Format)]
pub struct Gates {
    /// Lock's own GNSS must place it inside this fence (portable locks).
    pub own_fence: Option<u8>, // fence table index
    /// LIS3DH quiet for at least this many seconds (0 = no stillness gate).
    pub stillness_s: u16,
    /// RTC window (calendar gate) — index into a provisioned window table.
    pub calendar: Option<u8>,
}
// (Split-epoch freshness is not a gate: it is `delay_min_s..delay_max_s =
// 0..X` on the Sequence policy — a slot has exactly one arrival window.)

/// TOTP period (RFC 6238 default).
pub const PERIOD_S: u32 = 30;

#[derive(Copy, Clone, Format)]
pub enum Policy {
    /// One valid code fires immediately (master slot).
    AlwaysValid,
    /// N codes within `window_s` (arrival), with GENERATION cadence inside
    /// [gap_min_s, gap_max_s] — spacing is checked on the TOTP counters the
    /// codes matched, not on arrival times, so hoard-and-burst fails by
    /// construction. gap_min == gap_max ± tol = rhythm lock.
    ///
    /// `delay_min_s..delay_max_s` is the WALK-TIME window: a code minted at
    /// T is accepted only when `now - T` falls inside it (the verifier
    /// searches counters [(now-delay_max)/P .. (now-delay_min)/P]). 0..~60
    /// = instant slot; 1800..2100 = "generated 30-35 min ago".
    Sequence {
        n: u8,
        window_s: u16,
        gap_min_s: u16,
        gap_max_s: u16,
        delay_min_s: u16,
        delay_max_s: u16,
    },
    /// Ordered zone-keyed legs: key index per leg, per-leg deadline.
    /// (Walk-the-path / walk-away — legs reference different zone keys.)
    Path { legs: u8, leg_deadline_s: u16 },
    /// Once fired, must keep receiving a code every <= beat_s or re-lock.
    DeadMan { beat_s: u16 },
}

#[derive(Copy, Clone, PartialEq, Eq, Format)]
pub enum Action {
    Unlock,
    Lock,
    /// Unlocks normally, flags the audit log, distinct confirm code.
    DuressUnlock,
}

#[derive(Copy, Clone, Format)]
pub struct Slot {
    /// Secret / zone-key-set reference in the provisioned key table.
    pub key: u8,
    pub policy: Policy,
    pub gates: Gates,
    pub action: Action,
    /// Show sequence progress on the display, or stay indistinguishable
    /// from idle (decoy).
    pub show_progress: bool,
    /// A code matching NO slot resets this slot's sequence state.
    pub reset_on_invalid: bool,
}

/// Runtime state — RAM only, deliberately lost on power-cycle.
#[derive(Copy, Clone, Format, Default)]
pub struct SlotState {
    pub count: u8,
    /// TOTP period counter of the last accepted code (replay dedupe).
    pub last_period: u32,
    /// Uptime seconds of first / most recent accepted code.
    pub window_start_s: u32,
    pub last_code_s: u32,
}

pub enum Verdict {
    /// Code accepted, sequence advanced (progress = count/needed).
    Progress(u8, u8),
    /// Policy satisfied — perform `Action` and emit the confirm code.
    Fire(Action),
    /// Timing/gate violation — this slot reset.
    Reset,
    /// Not for this slot / replayed period — no state change.
    Ignored,
    /// Matched a DECOY key (`K_decoy`, generator poison mode): a definite
    /// squeezed-generator signal, not noise. Caller applies the configured
    /// severity (reset / lockout / silent duress telemetry) and always logs.
    Negative,
}

impl SlotState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Feed a validated code event at uptime `now_s`. `counter` is the TOTP
    /// counter the code matched — i.e. its generation time in periods; the
    /// caller has already verified the code AND that it fell inside the
    /// slot's delay window (it searched exactly that counter range). Pure
    /// state machine — crypto and gate evaluation happen in the caller.
    pub fn on_code(&mut self, slot: &Slot, counter: u32, now_s: u32) -> Verdict {
        if counter <= self.last_period && self.count > 0 {
            return Verdict::Ignored; // counter burned (replay) or out of order
        }
        match slot.policy {
            Policy::AlwaysValid => Verdict::Fire(slot.action),
            Policy::Sequence {
                n,
                window_s,
                gap_min_s,
                gap_max_s,
                ..
            } => {
                if self.count > 0 {
                    // Generation cadence: spacing between minting times,
                    // recovered from the matched counters.
                    let gen_gap = (counter - self.last_period) * PERIOD_S;
                    let in_window = now_s - self.window_start_s <= window_s as u32;
                    if gen_gap < gap_min_s as u32 || gen_gap > gap_max_s as u32 || !in_window {
                        self.reset();
                        return Verdict::Reset;
                    }
                } else {
                    self.window_start_s = now_s;
                }
                self.count += 1;
                self.last_period = counter;
                self.last_code_s = now_s;
                if self.count >= n {
                    self.reset();
                    Verdict::Fire(slot.action)
                } else {
                    Verdict::Progress(self.count, n)
                }
            }
            // TODO: Path legs (per-leg key check in caller), DeadMan re-arm
            // (needs a tick() driven by the actuation state).
            Policy::Path { .. } | Policy::DeadMan { .. } => Verdict::Ignored,
        }
    }

    /// Time-driven expiry: called periodically; resets a stale sequence even
    /// if no further code ever arrives (so `show_progress` displays decay).
    /// Expiry is on ARRIVAL time, so it allows the generation gap plus the
    /// delay-window spread (a +30..35 min code can arrive 5 min "late"
    /// relative to its minting cadence).
    pub fn tick(&mut self, slot: &Slot, now_s: u32) {
        if let Policy::Sequence {
            gap_max_s,
            delay_min_s,
            delay_max_s,
            ..
        } = slot.policy
        {
            let slack = (delay_max_s - delay_min_s) as u32;
            if self.count > 0 && now_s - self.last_code_s > gap_max_s as u32 + slack {
                self.reset();
            }
        }
    }
}
