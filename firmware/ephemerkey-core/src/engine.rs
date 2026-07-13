//! Lock-side validation engine.
//!
//! Owns the key table, slot table, and per-slot runtime state. For each
//! entered code it searches every slot's delay window (the counter range
//! `[(now-delay_max)/P .. (now-delay_min)/P]`), burns accepted counters
//! against replay, matches decoy keys as NEGATIVE, and feeds the slot state
//! machines. Gate evaluation (GNSS/accel/RTC) is the caller's job — a gated
//! slot is simply passed as `enabled = false` for now.

use crate::policy::{Action, NegativeAction, Slot, SlotState, Verdict, MAX_KEYS, MAX_SLOTS};
use crate::totp::{hotp, Code, PERIOD_S};

pub const MAX_SECRET: usize = 32;

#[derive(Copy, Clone)]
pub struct KeyDef {
    pub secret: [u8; MAX_SECRET],
    pub secret_len: u8,
    pub digits: u8,
    /// Key table index of this key's DECOY twin (poison mode), if any.
    pub decoy: Option<u8>,
}

impl KeyDef {
    pub fn new(secret: &[u8], digits: u8) -> Self {
        let mut s = [0u8; MAX_SECRET];
        s[..secret.len()].copy_from_slice(secret);
        Self {
            secret: s,
            secret_len: secret.len() as u8,
            digits,
            decoy: None,
        }
    }
    pub fn secret(&self) -> &[u8] {
        &self.secret[..self.secret_len as usize]
    }
}

/// What happened to an entered code — the emulator/CLI surface.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Outcome {
    /// Advanced slot's sequence: (slot, have, need).
    Progress(u8, u8, u8),
    /// Slot satisfied: perform the action.
    Fired(u8, Action),
    /// Timing violation reset the slot it matched.
    Reset(u8),
    /// Replay of an already-burned counter — silently ignored.
    Replay(u8),
    /// DECOY match on a slot: the squeezed-generator tripwire.
    Negative(u8, NegativeAction),
    /// Slot is in negative-lockout; code ignored.
    LockedOut(u8),
    /// Matched nothing: `reset_on_invalid` slots were reset.
    Invalid,
}

pub struct LockEngine {
    pub keys: [Option<KeyDef>; MAX_KEYS],
    pub slots: [Option<Slot>; MAX_SLOTS],
    pub state: [SlotState; MAX_SLOTS],
    /// Highest accepted counter per slot (replay burn watermark). Counters
    /// only move forward in time, so a watermark suffices.
    burned: [u32; MAX_SLOTS],
    /// Unix time (u32) until which the slot ignores codes (negative lockout).
    lockout_until: [u32; MAX_SLOTS],
}

impl Default for LockEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl LockEngine {
    pub fn new() -> Self {
        Self {
            keys: [None; MAX_KEYS],
            slots: [None; MAX_SLOTS],
            state: [SlotState::default(); MAX_SLOTS],
            burned: [0; MAX_SLOTS],
            lockout_until: [0; MAX_SLOTS],
        }
    }

    /// Search a delay window for a code match against one key.
    /// Returns the matched counter, newest first.
    fn match_key(key: &KeyDef, code: Code, now: u64, delay: (u32, u32)) -> Option<u32> {
        if code.digits != key.digits {
            return None;
        }
        let hi = (now.saturating_sub(u64::from(delay.0)) / u64::from(PERIOD_S)) as u32;
        let lo = (now.saturating_sub(u64::from(delay.1)) / u64::from(PERIOD_S)) as u32;
        let mut c = hi;
        loop {
            // NOTE(firmware): make this compare constant-time per candidate.
            if hotp(key.secret(), u64::from(c), key.digits) == code {
                return Some(c);
            }
            if c == lo {
                return None;
            }
            c -= 1;
        }
    }

    /// Advance time-driven state (sequence expiry). Call before/with entry.
    pub fn tick(&mut self, now: u64) {
        let now_s = now as u32;
        for i in 0..MAX_SLOTS {
            if let Some(slot) = self.slots[i] {
                self.state[i].tick(&slot, now_s);
            }
        }
    }

    /// Feed one entered code at unix time `now`.
    pub fn enter_code(&mut self, entered: &str, now: u64) -> Outcome {
        self.tick(now);
        let now_s = now as u32;

        for i in 0..MAX_SLOTS {
            let Some(slot) = self.slots[i] else { continue };
            let Some(key) = self.keys[slot.key as usize] else {
                continue;
            };
            let Some(code) = Code::parse(entered, key.digits) else {
                continue;
            };
            let delay = slot.policy.delay_window();

            if now_s < self.lockout_until[i] {
                // Even a correct code is ignored during lockout — but check
                // the match first so lockout is observable in the outcome.
                if Self::match_key(&key, code, now, delay).is_some() {
                    return Outcome::LockedOut(i as u8);
                }
            } else if let Some(counter) = Self::match_key(&key, code, now, delay) {
                if counter <= self.burned[i] {
                    return Outcome::Replay(i as u8);
                }
                self.burned[i] = counter;
                return match self.state[i].on_code(&slot, counter, now_s) {
                    Verdict::Progress(h, n) => Outcome::Progress(i as u8, h, n),
                    Verdict::Fire(a) => Outcome::Fired(i as u8, a),
                    Verdict::Reset => Outcome::Reset(i as u8),
                    Verdict::Ignored => Outcome::Replay(i as u8),
                    Verdict::Negative => unreachable!("decoys matched below"),
                };
            }

            // Decoy twin: same delay window — poison codes are minted where
            // real ones would have been.
            if let Some(dk) = key.decoy.and_then(|d| self.keys[d as usize]) {
                if let Some(code) = Code::parse(entered, dk.digits) {
                    if Self::match_key(&dk, code, now, delay).is_some() {
                        match slot.negative {
                            NegativeAction::Reset => self.state[i].reset(),
                            NegativeAction::Lockout(s) => {
                                self.state[i].reset();
                                self.lockout_until[i] = now_s + u32::from(s);
                            }
                            NegativeAction::Silent => {}
                        }
                        return Outcome::Negative(i as u8, slot.negative);
                    }
                }
            }
        }

        // Matched nothing anywhere.
        for i in 0..MAX_SLOTS {
            if let Some(slot) = self.slots[i] {
                if slot.reset_on_invalid {
                    self.state[i].reset();
                }
            }
        }
        Outcome::Invalid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{Gates, Policy};
    use crate::totp::totp_at;

    const SECRET: &[u8] = b"12345678901234567890";
    const DECOY: &[u8] = b"decoy-secret-00000000";

    fn seq_slot(key: u8, delay: (u16, u16)) -> Slot {
        Slot {
            key,
            policy: Policy::Sequence {
                n: 3,
                window_s: 600,
                gap_min_s: 90,
                gap_max_s: 240,
                delay_min_s: delay.0,
                delay_max_s: delay.1,
            },
            gates: Gates::default(),
            action: Action::Unlock,
            show_progress: true,
            reset_on_invalid: true,
            negative: NegativeAction::Lockout(300),
        }
    }

    fn engine(delay: (u16, u16)) -> LockEngine {
        let mut e = LockEngine::new();
        let mut k = KeyDef::new(SECRET, 6);
        k.decoy = Some(1);
        e.keys[0] = Some(k);
        e.keys[1] = Some(KeyDef::new(DECOY, 6));
        e.slots[0] = Some(seq_slot(0, delay));
        e
    }

    fn code_at(secret: &[u8], t: u64) -> String {
        let mut buf = [0u8; 10];
        totp_at(secret, t, 6).render(&mut buf).into()
    }

    #[test]
    fn instant_sequence_fires() {
        let mut e = engine((0, 60));
        let t0 = 1_750_000_000u64;
        assert_eq!(
            e.enter_code(&code_at(SECRET, t0), t0),
            Outcome::Progress(0, 1, 3)
        );
        let t1 = t0 + 120;
        assert_eq!(
            e.enter_code(&code_at(SECRET, t1), t1),
            Outcome::Progress(0, 2, 3)
        );
        let t2 = t1 + 120;
        assert_eq!(
            e.enter_code(&code_at(SECRET, t2), t2),
            Outcome::Fired(0, Action::Unlock)
        );
    }

    #[test]
    fn replay_is_ignored_not_reset() {
        let mut e = engine((0, 60));
        let t0 = 1_750_000_000u64;
        let c = code_at(SECRET, t0);
        assert_eq!(e.enter_code(&c, t0), Outcome::Progress(0, 1, 3));
        assert_eq!(e.enter_code(&c, t0 + 5), Outcome::Replay(0));
        // state intact: a properly spaced second code still progresses
        let t1 = t0 + 120;
        assert_eq!(
            e.enter_code(&code_at(SECRET, t1), t1),
            Outcome::Progress(0, 2, 3)
        );
    }

    #[test]
    fn walk_delay_window() {
        // Codes minted 30-35 min ago are the only valid ones.
        let mut e = engine((1800, 2100));
        let mint = 1_750_000_000u64;
        let c = code_at(SECRET, mint);
        // Entered immediately: not yet valid -> Invalid (and resets nothing armed).
        assert_eq!(e.enter_code(&c, mint), Outcome::Invalid);
        // Entered 32 min later: valid.
        assert_eq!(e.enter_code(&c, mint + 1920), Outcome::Progress(0, 1, 3));
        // Entered 40 min later (fresh engine): expired.
        let mut e2 = engine((1800, 2100));
        assert_eq!(e2.enter_code(&c, mint + 2400), Outcome::Invalid);
    }

    #[test]
    fn generation_cadence_not_arrival() {
        // Two codes minted 30s apart (gap_min = 90s) but entered 120s apart:
        // hoard-and-burst must fail even though ARRIVAL spacing looks fine.
        let mut e = engine((1800, 2100));
        let mint0 = 1_750_000_000u64;
        let c0 = code_at(SECRET, mint0);
        let c1 = code_at(SECRET, mint0 + 30);
        assert_eq!(e.enter_code(&c0, mint0 + 1900), Outcome::Progress(0, 1, 3));
        assert_eq!(e.enter_code(&c1, mint0 + 2020), Outcome::Reset(0));
    }

    #[test]
    fn decoy_negative_lockout() {
        let mut e = engine((0, 60));
        let t0 = 1_750_000_000u64;
        assert_eq!(
            e.enter_code(&code_at(SECRET, t0), t0),
            Outcome::Progress(0, 1, 3)
        );
        // Poison code: minted from the decoy twin.
        assert_eq!(
            e.enter_code(&code_at(DECOY, t0 + 30), t0 + 30),
            Outcome::Negative(0, NegativeAction::Lockout(300))
        );
        // Real codes are now ignored until lockout expires.
        let t1 = t0 + 150;
        assert_eq!(
            e.enter_code(&code_at(SECRET, t1), t1),
            Outcome::LockedOut(0)
        );
        let t2 = t0 + 400;
        assert_eq!(
            e.enter_code(&code_at(SECRET, t2), t2),
            Outcome::Progress(0, 1, 3)
        );
    }

    #[test]
    fn invalid_resets_armed_state() {
        let mut e = engine((0, 60));
        let t0 = 1_750_000_000u64;
        assert_eq!(
            e.enter_code(&code_at(SECRET, t0), t0),
            Outcome::Progress(0, 1, 3)
        );
        assert_eq!(e.enter_code("000000", t0 + 10), Outcome::Invalid);
        // Sequence restarted: next good code is 1/3 again.
        let t1 = t0 + 120;
        assert_eq!(
            e.enter_code(&code_at(SECRET, t1), t1),
            Outcome::Progress(0, 1, 3)
        );
    }
}
