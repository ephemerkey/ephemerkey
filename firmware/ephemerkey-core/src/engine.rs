//! Lock-side validation engine.
//!
//! Owns the key table, slot table, and per-slot runtime state. For each
//! entered code it searches every slot's delay window (the counter range
//! `[(now-delay_max)/P .. (now-delay_min)/P]`), burns accepted counters
//! against replay, matches decoy keys as NEGATIVE, and feeds the slot state
//! machines. Gate evaluation (GNSS/accel/RTC) is delegated to a caller-owned
//! [`Sensors`] environment: [`enter_code_with`] consults each slot's
//! [`Gates`] before letting a code advance it. A correct code that arrives
//! while a gate is shut yields [`Outcome::Gated`] and is NOT burned, so it
//! still works once the gate opens.
//!
//! [`enter_code_with`]: LockEngine::enter_code_with
//! [`Gates`]: crate::policy::Gates

use crate::policy::{
    Action, AllGatesOpen, GateBlock, NegativeAction, Policy, Sensors, Slot, SlotState, Verdict,
    MAX_KEYS, MAX_QUORUM, MAX_SLOTS,
};
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
    /// DeadMan sustain refreshed.
    Beat(u8),
    /// DECOY match on a slot: the squeezed-generator tripwire.
    Negative(u8, NegativeAction),
    /// Slot is in negative-lockout; code ignored.
    LockedOut(u8),
    /// A correct code matched this slot but one of its gates is shut
    /// (position / stillness / calendar). Not burned, no state change — it
    /// works once the gate opens.
    Gated(u8, GateBlock),
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
    /// Bit i set: slot i's DeadMan sustain expired — caller must re-lock.
    pending_relock: u8,
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
            pending_relock: 0,
        }
    }

    /// Candidate (key-table index, kidx) pairs an entered code may match
    /// for this slot right now: the single key, the current Path leg's
    /// key, or every Quorum member.
    fn candidates(slot: &Slot, st: &SlotState) -> ([(u8, u8); MAX_QUORUM], usize) {
        let mut out = [(0u8, 0u8); MAX_QUORUM];
        match slot.policy {
            Policy::Path { legs, leg_keys, .. } => {
                let leg = st.count.min(legs.saturating_sub(1));
                out[0] = (leg_keys[leg as usize], leg);
                (out, 1)
            }
            Policy::Quorum { n_keys, keys, .. } => {
                let n = (n_keys as usize).min(MAX_QUORUM);
                for (i, o) in out[..n].iter_mut().enumerate() {
                    *o = (keys[i], i as u8);
                }
                (out, n)
            }
            _ => {
                out[0] = (slot.key, 0);
                (out, 1)
            }
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

    /// Advance time-driven state (sequence/quorum expiry, DeadMan beats).
    /// Call on every time advance and before entry. Expired DeadMan slots
    /// accumulate in the relock queue — drain with [`take_relocks`].
    ///
    /// [`take_relocks`]: LockEngine::take_relocks
    pub fn tick(&mut self, now: u64) {
        let now_s = now as u32;
        for i in 0..MAX_SLOTS {
            if let Some(slot) = self.slots[i] {
                if self.state[i].tick(&slot, now_s) {
                    self.pending_relock |= 1 << i;
                }
            }
        }
    }

    /// Slots whose DeadMan sustain ended since the last call (bit i = slot
    /// i) — beat missed, invalid-code reset, or decoy reset. The caller
    /// must actuate the re-lock — the opposite of the slot's fire action —
    /// and log it. The engine's state and the physical lock must never
    /// disagree about "sustained".
    pub fn take_relocks(&mut self) -> u8 {
        core::mem::take(&mut self.pending_relock)
    }

    /// Forced reset (invalid code / decoy): a sustained DeadMan slot that
    /// gets reset here still owes the world a re-lock event.
    fn reset_slot(&mut self, i: usize) {
        if let Some(slot) = self.slots[i] {
            if matches!(slot.policy, Policy::DeadMan { .. }) && self.state[i].sustained {
                self.pending_relock |= 1 << i;
            }
        }
        self.state[i].reset();
    }

    /// Feed one entered code at unix time `now`, with every gate open.
    /// Convenience wrapper over [`enter_code_with`](Self::enter_code_with)
    /// for callers/tests that don't model position, motion, or time-of-day.
    pub fn enter_code(&mut self, entered: &str, now: u64) -> Outcome {
        self.enter_code_with(entered, now, &AllGatesOpen)
    }

    /// Feed one entered code at unix time `now`, evaluating each slot's gates
    /// against `env`. A code that would advance a slot whose gate is shut
    /// returns [`Outcome::Gated`] without burning the counter or touching
    /// slot state — the decoy tripwire, however, stays armed regardless of
    /// gates (an out-of-fence poison code is still a squeezed-generator
    /// signal worth catching).
    pub fn enter_code_with(&mut self, entered: &str, now: u64, env: &impl Sensors) -> Outcome {
        self.tick(now);
        let now_s = now as u32;

        for i in 0..MAX_SLOTS {
            let Some(slot) = self.slots[i] else { continue };
            let delay = slot.policy.delay_window();
            let gate = slot.gates.block(env);
            let (cands, n_cands) = Self::candidates(&slot, &self.state[i]);

            for &(ktab, kidx) in &cands[..n_cands] {
                let Some(key) = self.keys[ktab as usize] else {
                    continue;
                };
                let Some(code) = Code::parse(entered, key.digits) else {
                    continue;
                };

                if now_s < self.lockout_until[i] {
                    // Even a correct code is ignored during lockout — but
                    // check the match so lockout is observable.
                    if Self::match_key(&key, code, now, delay).is_some() {
                        return Outcome::LockedOut(i as u8);
                    }
                } else if let Some(counter) = Self::match_key(&key, code, now, delay) {
                    // A shut gate stops the code cold: no burn, no advance,
                    // so it stays valid once the lock is in position / still /
                    // in-window.
                    if let Some(reason) = gate {
                        return Outcome::Gated(i as u8, reason);
                    }
                    if counter <= self.burned[i] {
                        return Outcome::Replay(i as u8);
                    }
                    self.burned[i] = counter;
                    return match self.state[i].on_code(&slot, kidx, counter, now_s) {
                        Verdict::Progress(h, n) => Outcome::Progress(i as u8, h, n),
                        Verdict::Fire(a) => Outcome::Fired(i as u8, a),
                        Verdict::Beat => Outcome::Beat(i as u8),
                        Verdict::Reset => Outcome::Reset(i as u8),
                        Verdict::Ignored => Outcome::Replay(i as u8),
                        Verdict::Negative => unreachable!("decoys matched below"),
                    };
                }

                // Decoy twin: same delay window — poison codes are minted
                // where real ones would have been.
                if let Some(dk) = key.decoy.and_then(|d| self.keys[d as usize]) {
                    if let Some(code) = Code::parse(entered, dk.digits) {
                        if Self::match_key(&dk, code, now, delay).is_some() {
                            match slot.negative {
                                NegativeAction::Reset => self.reset_slot(i),
                                NegativeAction::Lockout(s) => {
                                    self.reset_slot(i);
                                    self.lockout_until[i] = now_s + u32::from(s);
                                }
                                NegativeAction::Silent => {}
                            }
                            return Outcome::Negative(i as u8, slot.negative);
                        }
                    }
                }
            }
        }

        // Matched nothing anywhere.
        for i in 0..MAX_SLOTS {
            if let Some(slot) = self.slots[i] {
                if slot.reset_on_invalid {
                    self.reset_slot(i);
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

    fn slot_with(policy: Policy) -> Slot {
        Slot {
            key: 0,
            policy,
            gates: Gates::default(),
            action: Action::Unlock,
            show_progress: true,
            reset_on_invalid: true,
            negative: NegativeAction::Reset,
        }
    }

    const ZONE_A: &[u8] = b"zone-a-secret-0000000";
    const ZONE_B: &[u8] = b"zone-b-secret-0000000";
    const ZONE_C: &[u8] = b"zone-c-secret-0000000";

    fn path_engine() -> LockEngine {
        let mut e = LockEngine::new();
        e.keys[0] = Some(KeyDef::new(ZONE_A, 6));
        e.keys[1] = Some(KeyDef::new(ZONE_B, 6));
        e.keys[2] = Some(KeyDef::new(ZONE_C, 6));
        e.slots[0] = Some(slot_with(Policy::Path {
            legs: 3,
            leg_keys: [0, 1, 2, 0],
            leg_deadline_s: 600,
            delay_max_s: 3600,
        }));
        e
    }

    #[test]
    fn path_walks_in_order() {
        let mut e = path_engine();
        let t0 = 1_750_000_000u64;
        // Mint along the route: A at t0, B at +8 min, C at +16 min.
        let (ca, cb, cc) = (
            code_at(ZONE_A, t0),
            code_at(ZONE_B, t0 + 480),
            code_at(ZONE_C, t0 + 960),
        );
        // Arrive 30 min after departure; enter in order from the notebook.
        let arr = t0 + 1800;
        assert_eq!(e.enter_code(&ca, arr), Outcome::Progress(0, 1, 3));
        assert_eq!(e.enter_code(&cb, arr + 10), Outcome::Progress(0, 2, 3));
        assert_eq!(
            e.enter_code(&cc, arr + 20),
            Outcome::Fired(0, Action::Unlock)
        );
    }

    #[test]
    fn path_wrong_order_is_invalid() {
        let mut e = path_engine();
        let t0 = 1_750_000_000u64;
        let cb = code_at(ZONE_B, t0 + 480);
        // Leg 1 expects zone A — a zone B code matches nothing.
        assert_eq!(e.enter_code(&cb, t0 + 1800), Outcome::Invalid);
    }

    #[test]
    fn path_dawdling_between_legs_resets() {
        let mut e = path_engine();
        let t0 = 1_750_000_000u64;
        let ca = code_at(ZONE_A, t0);
        let cb = code_at(ZONE_B, t0 + 900); // 15 min > 10 min leg deadline
        let arr = t0 + 1800;
        assert_eq!(e.enter_code(&ca, arr), Outcome::Progress(0, 1, 3));
        assert_eq!(e.enter_code(&cb, arr + 10), Outcome::Reset(0));
    }

    #[test]
    fn deadman_fires_beats_and_relocks() {
        let mut e = LockEngine::new();
        e.keys[0] = Some(KeyDef::new(SECRET, 6));
        e.slots[0] = Some(slot_with(Policy::DeadMan { beat_s: 120 }));
        let t0 = 1_750_000_000u64;
        assert_eq!(
            e.enter_code(&code_at(SECRET, t0), t0),
            Outcome::Fired(0, Action::Unlock)
        );
        // Fresh beat inside the window.
        let t1 = t0 + 90;
        assert_eq!(e.enter_code(&code_at(SECRET, t1), t1), Outcome::Beat(0));
        assert_eq!(e.take_relocks(), 0);
        // Miss a beat: relock event surfaces on tick.
        e.tick(t1 + 121);
        assert_eq!(e.take_relocks(), 0b1);
        assert_eq!(e.take_relocks(), 0); // drained
                                         // Re-arms: a fresh code fires again.
        let t2 = t1 + 180;
        assert_eq!(
            e.enter_code(&code_at(SECRET, t2), t2),
            Outcome::Fired(0, Action::Unlock)
        );
    }

    #[test]
    fn deadman_forced_reset_still_owes_a_relock() {
        // A stale/invalid code while sustained resets via reset_on_invalid —
        // the physical lock MUST still get a re-lock event, or engine state
        // and the actuator diverge.
        let mut e = LockEngine::new();
        e.keys[0] = Some(KeyDef::new(SECRET, 6));
        e.slots[0] = Some(slot_with(Policy::DeadMan { beat_s: 120 }));
        let t0 = 1_750_000_000u64;
        assert_eq!(
            e.enter_code(&code_at(SECRET, t0), t0),
            Outcome::Fired(0, Action::Unlock)
        );
        // A 90s-old code is outside the instant delay window -> Invalid.
        let t1 = t0 + 90;
        assert_eq!(e.enter_code(&code_at(SECRET, t0), t1), Outcome::Invalid);
        assert_eq!(e.take_relocks(), 0b1);
    }

    #[test]
    fn quorum_two_person_alternating() {
        const ALICE: &[u8] = b"alice-secret-00000000";
        const BOB: &[u8] = b"bob-secret-0000000000";
        let mut e = LockEngine::new();
        e.keys[0] = Some(KeyDef::new(ALICE, 6));
        e.keys[1] = Some(KeyDef::new(BOB, 6));
        let quorum = Policy::Quorum {
            m: 2,
            n_keys: 2,
            keys: [0, 1, 0, 0],
            window_s: 300,
            alternating: true,
        };
        e.slots[0] = Some(slot_with(quorum));
        let t0 = 1_750_000_000u64;

        // Alice then Alice again: alternating violation resets.
        assert_eq!(
            e.enter_code(&code_at(ALICE, t0), t0),
            Outcome::Progress(0, 1, 2)
        );
        let t1 = t0 + 40;
        assert_eq!(e.enter_code(&code_at(ALICE, t1), t1), Outcome::Reset(0));

        // Alice then Bob completes.
        let t2 = t0 + 90;
        assert_eq!(
            e.enter_code(&code_at(ALICE, t2), t2),
            Outcome::Progress(0, 1, 2)
        );
        let t3 = t0 + 130;
        assert_eq!(
            e.enter_code(&code_at(BOB, t3), t3),
            Outcome::Fired(0, Action::Unlock)
        );
    }

    #[test]
    fn quorum_window_expires() {
        const ALICE: &[u8] = b"alice-secret-00000000";
        const BOB: &[u8] = b"bob-secret-0000000000";
        let mut e = LockEngine::new();
        e.keys[0] = Some(KeyDef::new(ALICE, 6));
        e.keys[1] = Some(KeyDef::new(BOB, 6));
        e.slots[0] = Some(slot_with(Policy::Quorum {
            m: 2,
            n_keys: 2,
            keys: [0, 1, 0, 0],
            window_s: 300,
            alternating: false,
        }));
        let t0 = 1_750_000_000u64;
        assert_eq!(
            e.enter_code(&code_at(ALICE, t0), t0),
            Outcome::Progress(0, 1, 2)
        );
        // Bob shows up 6 minutes later: round expired, he starts a new one.
        let t1 = t0 + 360;
        assert_eq!(
            e.enter_code(&code_at(BOB, t1), t1),
            Outcome::Progress(0, 1, 2)
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

    // ------------------------------------------------------------- gates

    use crate::policy::{GateBlock, Sensors};

    /// A fake, script-settable environment for the gate tests.
    #[derive(Default)]
    struct FakeEnv {
        in_fence: bool,
        still_s: u32,
        cal_open: bool,
    }
    impl Sensors for FakeEnv {
        fn inside_fence(&self, _: u8) -> bool {
            self.in_fence
        }
        fn still_for_s(&self) -> u32 {
            self.still_s
        }
        fn calendar_open(&self, _: u8) -> bool {
            self.cal_open
        }
    }

    fn gated_master(gates: Gates) -> LockEngine {
        let mut e = LockEngine::new();
        e.keys[0] = Some(KeyDef::new(SECRET, 6));
        e.slots[0] = Some(Slot {
            key: 0,
            policy: Policy::AlwaysValid,
            gates,
            action: Action::Unlock,
            show_progress: true,
            reset_on_invalid: false,
            negative: NegativeAction::Reset,
        });
        e
    }

    #[test]
    fn fence_gate_blocks_then_opens() {
        let mut e = gated_master(Gates {
            own_fence: Some(0),
            ..Gates::default()
        });
        let t0 = 1_750_000_000u64;
        let mut env = FakeEnv {
            in_fence: false,
            ..Default::default()
        };
        let c = code_at(SECRET, t0);
        // Out of fence: correct code is gated, not burned.
        assert_eq!(
            e.enter_code_with(&c, t0, &env),
            Outcome::Gated(0, GateBlock::Fence)
        );
        // Step inside: the SAME code (never burned) now fires.
        env.in_fence = true;
        assert_eq!(
            e.enter_code_with(&c, t0, &env),
            Outcome::Fired(0, Action::Unlock)
        );
    }

    #[test]
    fn stillness_gate_reports_stillness() {
        let mut e = gated_master(Gates {
            stillness_s: 20,
            ..Gates::default()
        });
        let t0 = 1_750_000_000u64;
        let c = code_at(SECRET, t0);
        let moving = FakeEnv {
            still_s: 5,
            ..Default::default()
        };
        assert_eq!(
            e.enter_code_with(&c, t0, &moving),
            Outcome::Gated(0, GateBlock::Stillness)
        );
        let settled = FakeEnv {
            still_s: 25,
            ..Default::default()
        };
        assert_eq!(
            e.enter_code_with(&c, t0, &settled),
            Outcome::Fired(0, Action::Unlock)
        );
    }

    #[test]
    fn gates_evaluate_fence_before_calendar() {
        // With fence + calendar both shut, the reported reason is the fence
        // (left-to-right evaluation), so the UX message is deterministic.
        let mut e = gated_master(Gates {
            own_fence: Some(0),
            calendar: Some(0),
            ..Gates::default()
        });
        let t0 = 1_750_000_000u64;
        let env = FakeEnv::default(); // out of fence, calendar closed
        assert_eq!(
            e.enter_code_with(&code_at(SECRET, t0), t0, &env),
            Outcome::Gated(0, GateBlock::Fence)
        );
    }

    #[test]
    fn decoy_tripwire_fires_through_a_shut_gate() {
        // Out-of-fence is no excuse to ignore a poison code: the tripwire is
        // always armed even though the real path is gated.
        let mut e = gated_master(Gates {
            own_fence: Some(0),
            ..Gates::default()
        });
        // give the master key a decoy twin
        if let Some(k) = e.keys[0].as_mut() {
            k.decoy = Some(1);
        }
        e.keys[1] = Some(KeyDef::new(DECOY, 6));
        let t0 = 1_750_000_000u64;
        let env = FakeEnv::default(); // out of fence
        assert_eq!(
            e.enter_code_with(&code_at(DECOY, t0), t0, &env),
            Outcome::Negative(0, NegativeAction::Reset)
        );
    }
}
