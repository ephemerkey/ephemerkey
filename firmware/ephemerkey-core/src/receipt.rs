//! Confirm-TOTP — the lock talks back.
//!
//! On completing an action (unlock / lock / duress / relock) the lock mints
//! its own code from a per-lock confirm secret, so a remote party holding the
//! same secret can verify the event actually happened. Two orthogonal proofs,
//! independently selectable ([`ReceiptMode`]):
//!
//! - **Sequence** (HOTP over a monotonic event counter): proves *which* event
//!   and *in what order* — detects skips, replays, and re-ordering. It is an
//!   event receipt, not a time proof, and never collides on two events inside
//!   one TOTP period.
//! - **Time** (TOTP over the event's unix time): proves *when* — the verifier
//!   searches a drift window around its own clock and reports the offset.
//! - **Both**: emit both codes; each is independently verifiable, and a
//!   validator can require the pair (time AND order) to agree.
//!
//! Every code binds a domain tag (sequence vs time) and the action, so an
//! "unlock" receipt can never be replayed as a "lock", nor a sequence code
//! passed off as a time code. See DESIGN-policies.md "Confirm-TOTP".

use crate::engine::MAX_SECRET;
use crate::policy::Action;
use crate::totp::{hotp, Code, PERIOD_S};

/// Which proof(s) a lock issues on each event.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ReceiptMode {
    /// HOTP over the event counter — order/skip/replay proof.
    #[default]
    Sequence,
    /// TOTP over the event time — "it happened at HH:MM" proof.
    Time,
    /// Both codes, each independently verifiable.
    Both,
}

impl ReceiptMode {
    fn has_seq(self) -> bool {
        matches!(self, ReceiptMode::Sequence | ReceiptMode::Both)
    }
    fn has_time(self) -> bool {
        matches!(self, ReceiptMode::Time | ReceiptMode::Both)
    }
}

// Domain tags fold into the HMAC message so the two code kinds can never be
// confused for one another and receipts don't cross-replay between actions.
const KIND_SEQ: u16 = 0x51; // 'Q'
const KIND_TIME: u16 = 0x54; // 'T'

/// HMAC message: `[kind:8][action:8][zero:16][counter:32]`. `counter` is the
/// event counter (sequence) or the TOTP time counter — both fit in u32 for
/// any realistic device lifetime.
fn msg(kind: u16, action: Action, counter: u32) -> u64 {
    (u64::from(kind) << 56) | (u64::from(action as u8) << 48) | u64::from(counter)
}

/// A minted receipt for one event. Carries the metadata a human relays
/// alongside the code(s): the action, the sequence number (shown as
/// `code+seq`), and the event time.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Receipt {
    pub action: Action,
    /// Monotonic event counter (always assigned; only *proven* in seq modes).
    pub seq: u32,
    /// Unix time of the event (seconds).
    pub time_s: u32,
    /// HOTP over `seq` — present iff the mode proves sequence.
    pub seq_code: Option<Code>,
    /// TOTP over `time_s` — present iff the mode proves time.
    pub time_code: Option<Code>,
}

/// Per-lock receipt minter. Owns the confirm secret and the monotonic event
/// counter — which the firmware MUST persist (flash journal), since a receipt
/// stream that resets its counter on power-cycle would replay.
#[derive(Copy, Clone)]
pub struct Receipts {
    secret: [u8; MAX_SECRET],
    secret_len: u8,
    pub digits: u8,
    pub mode: ReceiptMode,
    /// Next event counter to assign. Persist across power-cycles.
    pub next_seq: u32,
}

impl Receipts {
    pub fn new(secret: &[u8], digits: u8, mode: ReceiptMode) -> Self {
        let mut s = [0u8; MAX_SECRET];
        s[..secret.len()].copy_from_slice(secret);
        Self {
            secret: s,
            secret_len: secret.len() as u8,
            digits,
            mode,
            next_seq: 0,
        }
    }

    fn secret(&self) -> &[u8] {
        &self.secret[..self.secret_len as usize]
    }

    /// Mint the receipt for one event, consuming the next event counter.
    pub fn mint(&mut self, action: Action, time_s: u32) -> Receipt {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        let digits = self.digits;
        let seq_code = self
            .mode
            .has_seq()
            .then(|| hotp(self.secret(), msg(KIND_SEQ, action, seq), digits));
        let time_code = self.mode.has_time().then(|| {
            hotp(
                self.secret(),
                msg(KIND_TIME, action, (time_s / PERIOD_S) as u32),
                digits,
            )
        });
        Receipt {
            action,
            seq,
            time_s,
            seq_code,
            time_code,
        }
    }
}

/// Result of verifying a relayed receipt.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ReceiptCheck {
    /// Genuine. `skipped` = events the validator missed before this one
    /// (sequence modes); `drift_s` = event time minus the validator's clock,
    /// negative = in the past (time modes; 0 otherwise).
    Valid { skipped: u32, drift_s: i32 },
    /// HMAC mismatch — forged, wrong key, or corrupted in relay.
    BadCode,
    /// Sequence counter already consumed (replay).
    Replay,
    /// Sequence counter beyond the resync look-ahead — too many missed events.
    TooFarAhead,
    /// Time code outside the accepted drift window.
    OutOfWindow,
    /// The relayed receipt didn't carry the code kind this validator needs.
    ModeMismatch,
}

/// Remote validator holding the same confirm secret. Tracks the expected next
/// event counter (RFC 4226 §7.4-style resync with a look-ahead window) and a
/// drift window for time codes.
#[derive(Copy, Clone)]
pub struct Validator {
    secret: [u8; MAX_SECRET],
    secret_len: u8,
    pub digits: u8,
    pub mode: ReceiptMode,
    /// Next event counter expected. Advances past accepted receipts.
    pub expected_seq: u32,
    /// How many missed events to tolerate before declaring desync.
    pub look_ahead: u32,
    /// Accepted lateness for a time code, seconds (searched into the past).
    pub time_window_s: u32,
}

impl Validator {
    pub fn new(secret: &[u8], digits: u8, mode: ReceiptMode) -> Self {
        let mut s = [0u8; MAX_SECRET];
        s[..secret.len()].copy_from_slice(secret);
        Self {
            secret: s,
            secret_len: secret.len() as u8,
            digits,
            mode,
            expected_seq: 0,
            look_ahead: 16,
            time_window_s: 3600,
        }
    }

    fn secret(&self) -> &[u8] {
        &self.secret[..self.secret_len as usize]
    }

    /// Verify a relayed receipt against the validator's clock `now_s`.
    /// Mutates `expected_seq` on an accepted sequence proof.
    pub fn verify(&mut self, r: &Receipt, now_s: u32) -> ReceiptCheck {
        let mut skipped = 0;
        let mut drift_s = 0;

        if self.mode.has_seq() {
            let Some(code) = r.seq_code else {
                return ReceiptCheck::ModeMismatch;
            };
            match self.check_seq(r.action, r.seq, code) {
                Ok(s) => skipped = s,
                Err(e) => return e,
            }
        }
        if self.mode.has_time() {
            let Some(code) = r.time_code else {
                return ReceiptCheck::ModeMismatch;
            };
            match self.check_time(r.action, code, now_s) {
                Ok(d) => drift_s = d,
                Err(e) => return e,
            }
        }

        // Commit the sequence advance only once both required proofs pass.
        if self.mode.has_seq() {
            self.expected_seq = r.seq.wrapping_add(1);
        }
        ReceiptCheck::Valid { skipped, drift_s }
    }

    /// Verify a receipt from its CODE ALONE — what a human can actually
    /// carry from the lock's display to a generator keypad. Sequence modes
    /// resync by searching the look-ahead window (RFC 4226 §7.4); pure Time
    /// mode searches the drift window. `Both` verifies the sequence proof.
    pub fn verify_code(&mut self, action: Action, code: Code, now_s: u32) -> ReceiptCheck {
        if self.mode.has_seq() {
            let mut seq = self.expected_seq;
            while seq < self.expected_seq.saturating_add(self.look_ahead) {
                if hotp(self.secret(), msg(KIND_SEQ, action, seq), self.digits) == code {
                    let skipped = seq - self.expected_seq;
                    self.expected_seq = seq.wrapping_add(1);
                    return ReceiptCheck::Valid { skipped, drift_s: 0 };
                }
                seq += 1;
            }
            ReceiptCheck::BadCode
        } else {
            match self.check_time(action, code, now_s) {
                Ok(drift_s) => ReceiptCheck::Valid { skipped: 0, drift_s },
                Err(e) => e,
            }
        }
    }

    fn check_seq(&self, action: Action, seq: u32, code: Code) -> Result<u32, ReceiptCheck> {
        if seq < self.expected_seq {
            return Err(ReceiptCheck::Replay);
        }
        if seq >= self.expected_seq.saturating_add(self.look_ahead) {
            return Err(ReceiptCheck::TooFarAhead);
        }
        if hotp(self.secret(), msg(KIND_SEQ, action, seq), self.digits) != code {
            return Err(ReceiptCheck::BadCode);
        }
        Ok(seq - self.expected_seq)
    }

    fn check_time(&self, action: Action, code: Code, now_s: u32) -> Result<i32, ReceiptCheck> {
        // Search from a little future slack (clock skew) back over the window.
        let hi = (now_s + PERIOD_S) / PERIOD_S;
        let lo = now_s.saturating_sub(self.time_window_s) / PERIOD_S;
        let mut c = hi;
        loop {
            if hotp(self.secret(), msg(KIND_TIME, action, c), self.digits) == code {
                let drift = c as i64 * PERIOD_S as i64 - now_s as i64;
                return Ok(drift as i32);
            }
            if c == lo {
                return Err(ReceiptCheck::OutOfWindow);
            }
            c -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: &[u8] = b"confirm-secret-000000";

    fn pair(mode: ReceiptMode) -> (Receipts, Validator) {
        (Receipts::new(K, 6, mode), Validator::new(K, 6, mode))
    }

    #[test]
    fn sequence_verifies_and_advances() {
        let (mut m, mut v) = pair(ReceiptMode::Sequence);
        let r0 = m.mint(Action::Unlock, 1_750_000_000);
        assert_eq!(r0.seq, 0);
        assert!(r0.time_code.is_none());
        assert_eq!(
            v.verify(&r0, 1_750_000_005),
            ReceiptCheck::Valid {
                skipped: 0,
                drift_s: 0
            }
        );
        // Relaying it again is a replay: counter already consumed.
        assert_eq!(v.verify(&r0, 1_750_000_010), ReceiptCheck::Replay);
        // Next event verifies in order.
        let r1 = m.mint(Action::Lock, 1_750_000_100);
        assert_eq!(r1.seq, 1);
        assert_eq!(
            v.verify(&r1, 1_750_000_105),
            ReceiptCheck::Valid {
                skipped: 0,
                drift_s: 0
            }
        );
    }

    #[test]
    fn sequence_resyncs_over_missed_events() {
        let (mut m, mut v) = pair(ReceiptMode::Sequence);
        // Validator misses events 0..2; event 3 still verifies, reports skip.
        let _ = m.mint(Action::Unlock, 1_000_000);
        let _ = m.mint(Action::Unlock, 1_000_100);
        let _ = m.mint(Action::Unlock, 1_000_200);
        let r3 = m.mint(Action::Unlock, 1_000_300);
        assert_eq!(
            v.verify(&r3, 1_000_305),
            ReceiptCheck::Valid {
                skipped: 3,
                drift_s: 0
            }
        );
        // Now the old missed ones are stale (replays).
        assert!(v.expected_seq == 4);
    }

    #[test]
    fn sequence_too_far_ahead_is_desync() {
        let (mut m, mut v) = pair(ReceiptMode::Sequence);
        v.look_ahead = 4;
        let mut r = m.mint(Action::Unlock, 1_000_000);
        for _ in 0..9 {
            r = m.mint(Action::Unlock, 1_000_000);
        }
        assert_eq!(r.seq, 9);
        assert_eq!(v.verify(&r, 1_000_000), ReceiptCheck::TooFarAhead);
    }

    #[test]
    fn action_binding_prevents_cross_replay() {
        // A code minted for Unlock must not verify when relayed as Lock.
        let (mut m, mut v) = pair(ReceiptMode::Sequence);
        let mut r = m.mint(Action::Unlock, 1_000_000);
        r.action = Action::Lock; // attacker relabels the event
        assert_eq!(v.verify(&r, 1_000_005), ReceiptCheck::BadCode);
    }

    #[test]
    fn time_verifies_with_drift_and_expires() {
        let (mut m, mut v) = pair(ReceiptMode::Time);
        v.time_window_s = 3600;
        let mint_t = 1_750_000_000;
        let r = m.mint(Action::Lock, mint_t);
        assert!(r.seq_code.is_none());
        // Relayed 20 min later: valid, drift ~ -1200 s (rounded to the period).
        match v.verify(&r, mint_t + 1200) {
            ReceiptCheck::Valid { drift_s, .. } => assert!((-1230..=-1170).contains(&drift_s)),
            other => panic!("expected Valid, got {other:?}"),
        }
        // Relayed 2 h later: outside the window.
        assert_eq!(v.verify(&r, mint_t + 7200), ReceiptCheck::OutOfWindow);
    }

    #[test]
    fn both_requires_time_and_sequence() {
        let (mut m, mut v) = pair(ReceiptMode::Both);
        let mint_t = 1_750_000_000;
        let r = m.mint(Action::Unlock, mint_t);
        assert!(r.seq_code.is_some() && r.time_code.is_some());
        match v.verify(&r, mint_t + 60) {
            ReceiptCheck::Valid { skipped, drift_s } => {
                assert_eq!(skipped, 0);
                assert!(drift_s <= 0 && drift_s >= -90);
            }
            other => panic!("expected Valid, got {other:?}"),
        }
    }

    #[test]
    fn both_rejects_when_a_proof_is_missing() {
        // A Both-validator handed a sequence-only receipt refuses it rather
        // than silently accepting half a proof.
        let mut m = Receipts::new(K, 6, ReceiptMode::Sequence);
        let mut v = Validator::new(K, 6, ReceiptMode::Both);
        let r = m.mint(Action::Unlock, 1_750_000_000);
        assert_eq!(v.verify(&r, 1_750_000_000), ReceiptCheck::ModeMismatch);
    }

    #[test]
    fn both_does_not_advance_seq_when_time_fails() {
        // If the time proof is out of window, the sequence counter must NOT
        // advance — otherwise a stale-but-in-order receipt would burn a seq.
        let (mut m, mut v) = pair(ReceiptMode::Both);
        v.time_window_s = 600;
        let mint_t = 1_750_000_000;
        let r = m.mint(Action::Unlock, mint_t);
        assert_eq!(v.verify(&r, mint_t + 7200), ReceiptCheck::OutOfWindow);
        assert_eq!(v.expected_seq, 0); // not advanced
        // The same receipt, relayed promptly, still verifies.
        assert!(matches!(
            v.verify(&r, mint_t + 30),
            ReceiptCheck::Valid { .. }
        ));
    }
}
