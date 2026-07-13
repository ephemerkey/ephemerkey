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
    /// Code only counts in the first X seconds of its TOTP period
    /// (split-epoch freshness; 0 = whole period).
    pub epoch_head_s: u8,
}

#[derive(Copy, Clone, Format)]
pub enum Policy {
    /// One valid code fires immediately (master slot).
    AlwaysValid,
    /// N codes within `window_s`, inter-arrival inside [gap_min_s, gap_max_s].
    /// The canonical pedantic sequence; gap_min == gap_max ± tol = rhythm lock.
    Sequence {
        n: u8,
        window_s: u16,
        gap_min_s: u16,
        gap_max_s: u16,
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
}

impl SlotState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Feed a validated code event (slot key already matched, period given)
    /// at uptime `now_s`. Pure state machine — TOTP validation and gate
    /// evaluation happen in the caller.
    pub fn on_code(&mut self, slot: &Slot, period: u32, now_s: u32) -> Verdict {
        if period == self.last_period && self.count > 0 {
            return Verdict::Ignored; // same TOTP period: counts once
        }
        match slot.policy {
            Policy::AlwaysValid => Verdict::Fire(slot.action),
            Policy::Sequence {
                n,
                window_s,
                gap_min_s,
                gap_max_s,
            } => {
                if self.count > 0 {
                    let gap = now_s - self.last_code_s;
                    let in_window = now_s - self.window_start_s <= window_s as u32;
                    if gap < gap_min_s as u32 || gap > gap_max_s as u32 || !in_window {
                        self.reset();
                        return Verdict::Reset;
                    }
                } else {
                    self.window_start_s = now_s;
                }
                self.count += 1;
                self.last_period = period;
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
    pub fn tick(&mut self, slot: &Slot, now_s: u32) {
        if let Policy::Sequence { gap_max_s, .. } = slot.policy {
            if self.count > 0 && now_s - self.last_code_s > gap_max_s as u32 {
                self.reset();
            }
        }
    }
}
