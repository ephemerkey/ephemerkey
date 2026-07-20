//! Generator-side reveal scheduler: what the display is allowed to show.
//!
//! HARD REQUIREMENT (DESIGN-policies.md "poison mode"): real and decoy
//! reveals share this ONE code path, parameterized only by which secret they
//! mint from. The returned [`Reveal`] carries no is-decoy information unless
//! the `introspect` feature is on (emulator/tests only — never firmware).

use crate::engine::{KeyDef, Outcome};
use crate::policy::{Action, MAX_KEYS};
use crate::receipt::{ReceiptCheck, Validator};
use crate::totp::{totp_at, Code, MAX_DIGITS};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DisplayMode {
    /// Whole code at once.
    Plain,
    /// One digit at a time, correct position, random order.
    Scatter,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum OnceMode {
    /// No reveal limiting.
    Unlimited,
    /// After one reveal, refuse until the next legitimate cadence window.
    Refuse,
    /// After one reveal, further reveals mint from the DECOY twin.
    Decoy,
}

#[derive(Copy, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DisplaySpec {
    pub mode: DisplayMode,
    /// Per-digit dwell for Scatter, ms.
    pub dwell_ms: u16,
    /// How long a revealed code stays on screen, seconds.
    pub reveal_s: u16,
    pub once: OnceMode,
    /// The slot cadence this key feeds — the generator mirrors the lock's
    /// gap window so its countdown UX matches what will be accepted.
    pub gap_min_s: u16,
}

/// Receipt chain: the generator refuses to mint this key's codes until it
/// has been fed (witnessed) the lock's receipt from the previous session,
/// and a minimum time has passed since. The human carries the lock's code
/// BACK to the generator — continuity both ways. The lock's attest button
/// can mint a fresh state receipt any time one goes missing.
#[derive(Copy, Clone)]
pub struct ChainSpec {
    /// Validator over the lock's confirm secret.
    pub validator: Validator,
    /// Which receipt action feeds the chain. Default `Lock`: "I witnessed
    /// it locked" — exactly what the attest button re-mints on demand.
    pub action: Action,
    /// Codes mint only this long after a receipt was fed (cooling-off).
    pub min_elapsed_s: u32,
}

#[derive(Copy, Clone)]
pub struct GenKey {
    pub key: KeyDef,
    /// Decoy twin secret (poison mode). MUST be display-indistinguishable:
    /// same digits, same spec — enforced by construction here.
    pub decoy: Option<KeyDef>,
    pub display: DisplaySpec,
    /// Optional receipt chain gating this key's REAL reveals.
    pub chain: Option<ChainSpec>,
}

/// One reveal, ready for the display driver. Identical shape for real and
/// decoy codes — the display layer cannot tell, by design.
#[derive(Copy, Clone)]
pub struct Reveal {
    pub code: Code,
    /// Digit positions in presentation order (first `code.digits` entries).
    pub order: [u8; MAX_DIGITS as usize],
    pub mode: DisplayMode,
    pub dwell_ms: u16,
    pub reveal_s: u16,
    /// Ground truth for the emulator/tests ONLY.
    #[cfg(feature = "introspect")]
    pub decoy: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum RevealErr {
    /// Show-once refusal: next legitimate reveal at this unix time.
    RefusedUntil(u64),
    NoSuchKey,
    /// Receipt chain: feed the lock's receipt code before this key mints.
    ChainRequired,
    /// Receipt fed; cooling-off runs until this unix time.
    ChainWait(u64),
    /// Ritual-gated key with no unlock window open and no decoy twin to fall
    /// back on: reveal nothing (the firmware blanks — no distinct signal, so
    /// an observer can't tell a failed ritual from an idle device).
    Locked,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ChainErr {
    NoSuchKey,
    /// Key has no chain configured.
    NoChain,
    /// The code did not verify (wrong/replayed/desynced).
    Rejected(ReceiptCheck),
}

#[derive(Copy, Clone, Default)]
struct GenKeyState {
    /// Unix time of the last REAL reveal (0 = never).
    last_real: u64,
    /// Chain: unix time the last receipt was fed (0 = never).
    chain_fed_at: u64,
    /// Chain: a real reveal happened since the last feed — locked again.
    chain_spent: bool,
}

pub struct Generator {
    pub keys: [Option<GenKey>; MAX_KEYS],
    state: [GenKeyState; MAX_KEYS],
    /// Ritual gate (cascading generation): a key with `gated[idx]` set reveals
    /// its REAL code only while `now < unlocked_until`; otherwise it mints the
    /// poison twin (or refuses). Non-cascade generators leave `gated` all-false
    /// and these windows unused, so behavior is exactly as before.
    gated: [bool; MAX_KEYS],
    /// Real reveals are permitted until this unix time (set by a ritual
    /// `Fired(unlock)` via [`apply_ritual_outcome`]). 0 = locked.
    unlocked_until: u64,
    /// A duress ritual opened a window: the device LOOKS unlocked but every
    /// gated key mints poison until this unix time. Checked before
    /// `unlocked_until`, so it wins.
    duress_until: u64,
}

impl Default for Generator {
    fn default() -> Self {
        Self::new()
    }
}

impl Generator {
    pub fn new() -> Self {
        Self {
            keys: [None; MAX_KEYS],
            state: [GenKeyState::default(); MAX_KEYS],
            gated: [false; MAX_KEYS],
            unlocked_until: 0,
            duress_until: 0,
        }
    }

    /// Mark key `idx` as ritual-gated (built from the config's `gated` flag).
    pub fn set_gated(&mut self, idx: usize, gated: bool) {
        if idx < MAX_KEYS {
            self.gated[idx] = gated;
        }
    }

    /// Open the real-reveal window until `until` (unix). Called by
    /// [`apply_ritual_outcome`] on a ritual `Fired(unlock)`.
    pub fn set_unlocked(&mut self, until: u64) {
        self.unlocked_until = until;
    }

    /// Open a DURESS window until `until` (unix): the device presents as
    /// unlocked but only poison flows. Also raises `unlocked_until` so the UI
    /// state ("unlocked") is itself indistinguishable.
    pub fn set_duress(&mut self, until: u64) {
        self.duress_until = until;
        if until > self.unlocked_until {
            self.unlocked_until = until;
        }
    }

    /// Whether the reveal window is currently open (UI countdown helper).
    pub fn is_unlocked(&self, now: u64) -> bool {
        now < self.unlocked_until
    }

    /// Request a code reveal for key `idx` at unix `now`. `entropy` feeds
    /// the scatter order (firmware: TRNG; emulator: seeded PRNG).
    pub fn reveal(&mut self, idx: usize, now: u64, entropy: u32) -> Result<Reveal, RevealErr> {
        let gk = self
            .keys
            .get(idx)
            .copied()
            .flatten()
            .ok_or(RevealErr::NoSuchKey)?;

        // Ritual gate (outermost): a duress window forces poison regardless; a
        // gated key with no open unlock window mints poison too. Both route
        // through the shared decoy branch so the reveal is indistinguishable
        // from a real one — and neither touches `last_real`/chain state, so the
        // legitimate cadence is unperturbed. `Locked` (no twin) reveals nothing.
        let in_duress = now < self.duress_until;
        let ritual_locked = self.gated[idx] && now >= self.unlocked_until;
        if in_duress || ritual_locked {
            let d = gk.decoy.ok_or(RevealErr::Locked)?;
            let code = totp_at(d.secret(), now, d.digits);
            return Ok(Reveal {
                code,
                order: scatter_order(code.digits, entropy),
                mode: gk.display.mode,
                dwell_ms: gk.display.dwell_ms,
                reveal_s: gk.display.reveal_s,
                #[cfg(feature = "introspect")]
                decoy: true,
            });
        }

        let st = &mut self.state[idx];

        // Receipt chain: real reveals need a fresh witnessed receipt plus
        // the cooling-off. (Poison-mode decoys below stay unaffected — an
        // observer can't distinguish a chain-blocked generator.)
        if let Some(ch) = gk.chain {
            if st.chain_spent {
                return Err(RevealErr::ChainRequired);
            }
            if st.chain_fed_at > 0 {
                let ready_at = st.chain_fed_at + u64::from(ch.min_elapsed_s);
                if now < ready_at {
                    return Err(RevealErr::ChainWait(ready_at));
                }
            }
        }

        // Is a real reveal permitted right now?
        let real_ok = match gk.display.once {
            OnceMode::Unlimited => true,
            _ => st.last_real == 0 || now - st.last_real >= u64::from(gk.display.gap_min_s),
        };

        let (secret_key, is_decoy) = if real_ok {
            st.last_real = now;
            if gk.chain.is_some() {
                st.chain_spent = true; // next session needs a new witness
            }
            (gk.key, false)
        } else {
            match gk.display.once {
                OnceMode::Refuse => {
                    return Err(RevealErr::RefusedUntil(
                        st.last_real + u64::from(gk.display.gap_min_s),
                    ))
                }
                OnceMode::Decoy => match gk.decoy {
                    // Poison: mint from the twin, identical presentation.
                    Some(d) => (d, true),
                    None => {
                        return Err(RevealErr::RefusedUntil(
                            st.last_real + u64::from(gk.display.gap_min_s),
                        ))
                    }
                },
                OnceMode::Unlimited => unreachable!(),
            }
        };

        let code = totp_at(secret_key.secret(), now, secret_key.digits);
        let _ = is_decoy; // consumed only under introspect
        Ok(Reveal {
            code,
            order: scatter_order(code.digits, entropy),
            mode: gk.display.mode,
            dwell_ms: gk.display.dwell_ms,
            reveal_s: gk.display.reveal_s,
            #[cfg(feature = "introspect")]
            decoy: is_decoy,
        })
    }

    /// Feed a lock receipt code into key `idx`'s chain. On success the
    /// chain re-arms and returns the unix time reveals resume.
    pub fn feed_chain(&mut self, idx: usize, code: Code, now: u64) -> Result<u64, ChainErr> {
        let Some(gk) = self.keys.get_mut(idx).and_then(|k| k.as_mut()) else {
            return Err(ChainErr::NoSuchKey);
        };
        let Some(ch) = gk.chain.as_mut() else {
            return Err(ChainErr::NoChain);
        };
        match ch.validator.verify_code(ch.action, code, now as u32) {
            ReceiptCheck::Valid { .. } => {
                let min_elapsed = u64::from(ch.min_elapsed_s);
                let st = &mut self.state[idx];
                st.chain_fed_at = now;
                st.chain_spent = false;
                Ok(now + min_elapsed)
            }
            e => Err(ChainErr::Rejected(e)),
        }
    }

    /// Seconds until the next real reveal for the key (0 = now). UX helper
    /// for the "next code in N s" countdown.
    pub fn next_real_in(&self, idx: usize, now: u64) -> Option<u64> {
        let gk = self.keys.get(idx).copied().flatten()?;
        let st = self.state[idx];
        Some(match gk.display.once {
            OnceMode::Unlimited => 0,
            _ if st.last_real == 0 => 0,
            _ => (st.last_real + u64::from(gk.display.gap_min_s)).saturating_sub(now),
        })
    }
}

/// Apply a ritual engine's [`Outcome`] to the generator's reveal window, the
/// single glue point between the cascade ritual ([`LockEngine`]) and the
/// reveal side. Firmware and emulator both route their `enter_code_with`
/// result through here, so on-device and simulated cascades can't diverge.
///
/// A `Fired(unlock)` opens a real window for `window_s`; a `Fired(duress)`
/// opens an indistinguishable poison-only window; everything else
/// (progress, gated, invalid) leaves the generator locked.
///
/// [`LockEngine`]: crate::engine::LockEngine
pub fn apply_ritual_outcome(gen: &mut Generator, out: &Outcome, now: u64, window_s: u32) {
    match out {
        Outcome::Fired(_, Action::Unlock) => gen.set_unlocked(now + window_s as u64),
        Outcome::Fired(_, Action::DuressUnlock) => gen.set_duress(now + window_s as u64),
        _ => {}
    }
}

/// Fisher-Yates over digit positions with a tiny xorshift PRNG. Presentation
/// order only — no security weight beyond shoulder-surf resistance.
fn scatter_order(digits: u8, seed: u32) -> [u8; MAX_DIGITS as usize] {
    let mut order = [0u8; MAX_DIGITS as usize];
    for (i, o) in order.iter_mut().enumerate() {
        *o = i as u8;
    }
    let mut s = seed | 1;
    let n = digits as usize;
    for i in (1..n).rev() {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        order.swap(i, (s as usize) % (i + 1));
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn genkey(once: OnceMode) -> GenKey {
        let mut key = KeyDef::new(b"12345678901234567890", 6);
        key.decoy = Some(1);
        GenKey {
            key,
            decoy: Some(KeyDef::new(b"decoy-secret-00000000", 6)),
            display: DisplaySpec {
                mode: DisplayMode::Scatter,
                dwell_ms: 800,
                reveal_s: 5,
                once,
                gap_min_s: 90,
            },
            chain: None,
        }
    }

    #[test]
    fn scatter_order_is_a_permutation() {
        for seed in [1u32, 42, 0xdeadbeef] {
            let o = scatter_order(6, seed);
            let mut seen = [false; 6];
            for &p in &o[..6] {
                assert!(!seen[p as usize]);
                seen[p as usize] = true;
            }
        }
    }

    #[test]
    fn show_once_refuses_then_allows() {
        let mut g = Generator::new();
        g.keys[0] = Some(genkey(OnceMode::Refuse));
        let t = 1_750_000_000u64;
        assert!(g.reveal(0, t, 7).is_ok());
        match g.reveal(0, t + 10, 7) {
            Err(RevealErr::RefusedUntil(u)) => assert_eq!(u, t + 90),
            _ => panic!("expected refusal"),
        }
        assert!(g.reveal(0, t + 90, 7).is_ok());
    }

    #[cfg(feature = "introspect")]
    #[test]
    fn poison_mode_mints_decoys_indistinguishably() {
        let mut g = Generator::new();
        g.keys[0] = Some(genkey(OnceMode::Decoy));
        let t = 1_750_000_000u64;
        let real = g.reveal(0, t, 7).unwrap();
        let poison = g.reveal(0, t + 10, 7).unwrap();
        assert!(!real.decoy);
        assert!(poison.decoy);
        // Identical presentation surface:
        assert_eq!(real.code.digits, poison.code.digits);
        assert_eq!(real.dwell_ms, poison.dwell_ms);
        assert_eq!(real.reveal_s, poison.reveal_s);
        // And the real stream resumes on cadence:
        let real2 = g.reveal(0, t + 95, 7).unwrap();
        assert!(!real2.decoy);
    }

    #[cfg(feature = "introspect")]
    #[test]
    fn ritual_gate_locked_reveals_decoy_until_unlocked() {
        let mut g = Generator::new();
        g.keys[0] = Some(genkey(OnceMode::Unlimited));
        g.set_gated(0, true);
        let t = 1_750_000_000u64;

        // Locked: a gated key mints the (indistinguishable) poison twin.
        let poison = g.reveal(0, t, 7).unwrap();
        assert!(poison.decoy);

        // A ritual unlock opens the window; reveals go real for its duration.
        apply_ritual_outcome(&mut g, &Outcome::Fired(1, Action::Unlock), t, 30);
        assert!(g.is_unlocked(t + 10));
        assert!(!g.reveal(0, t + 10, 7).unwrap().decoy);

        // Window closes → back to poison, no separate signal.
        assert!(!g.is_unlocked(t + 30));
        assert!(g.reveal(0, t + 30, 7).unwrap().decoy);
    }

    #[test]
    fn ritual_gate_without_twin_refuses_then_reveals() {
        let mut g = Generator::new();
        let mut gk = genkey(OnceMode::Unlimited);
        gk.decoy = None; // no poison twin → locked reveal yields nothing
        g.keys[0] = Some(gk);
        g.set_gated(0, true);
        let t = 1_750_000_000u64;

        assert_eq!(g.reveal(0, t, 7).err(), Some(RevealErr::Locked));
        apply_ritual_outcome(&mut g, &Outcome::Fired(0, Action::Unlock), t, 30);
        assert!(g.reveal(0, t + 5, 7).is_ok());
    }

    #[cfg(feature = "introspect")]
    #[test]
    fn duress_ritual_looks_unlocked_but_mints_poison() {
        let mut g = Generator::new();
        g.keys[0] = Some(genkey(OnceMode::Unlimited));
        g.set_gated(0, true);
        let t = 1_750_000_000u64;

        apply_ritual_outcome(&mut g, &Outcome::Fired(2, Action::DuressUnlock), t, 30);
        // Presents as unlocked (indistinguishable UI state)...
        assert!(g.is_unlocked(t + 10));
        // ...but every reveal in the window is poison.
        assert!(g.reveal(0, t + 10, 7).unwrap().decoy);
    }

    #[test]
    fn ungated_key_ignores_ritual_window() {
        // A non-cascade key reveals real regardless of the (unused) windows.
        let mut g = Generator::new();
        g.keys[0] = Some(genkey(OnceMode::Unlimited));
        // gated stays false
        let t = 1_750_000_000u64;
        assert!(g.reveal(0, t, 7).is_ok());
    }

    #[test]
    fn receipt_chain_gates_real_reveals() {
        use crate::receipt::{ReceiptMode, Receipts, Validator};
        const CONFIRM: &[u8] = b"confirm-secret-000000";
        let mut lock_receipts = Receipts::new(CONFIRM, 6, ReceiptMode::Sequence);

        let mut g = Generator::new();
        let mut gk = genkey(OnceMode::Unlimited);
        gk.display.gap_min_s = 0;
        gk.chain = Some(ChainSpec {
            validator: Validator::new(CONFIRM, 6, ReceiptMode::Sequence),
            action: Action::Lock,
            min_elapsed_s: 600,
        });
        g.keys[0] = Some(gk);
        let t0 = 1_750_000_000u64;

        // Genesis reveal is free; it spends the chain.
        assert!(g.reveal(0, t0, 1).is_ok());
        assert_eq!(g.reveal(0, t0 + 40, 2).err(), Some(RevealErr::ChainRequired));

        // Garbage codes don't re-arm it.
        assert!(matches!(
            g.feed_chain(0, crate::totp::Code { value: 123456, digits: 6 }, t0 + 50),
            Err(ChainErr::Rejected(_))
        ));

        // A real lock receipt re-arms; the cooling-off holds until elapsed.
        let r = lock_receipts.mint(Action::Lock, (t0 + 60) as u32);
        let unlock_at = g.feed_chain(0, r.seq_code.unwrap(), t0 + 60).unwrap();
        assert_eq!(unlock_at, t0 + 660);
        assert_eq!(g.reveal(0, t0 + 120, 3).err(), Some(RevealErr::ChainWait(t0 + 660)));
        assert!(g.reveal(0, t0 + 660, 4).is_ok());
        assert_eq!(g.reveal(0, t0 + 700, 5).err(), Some(RevealErr::ChainRequired));

        // Attest re-mints (skipping counters is fine): a LATER receipt still
        // verifies via look-ahead — the missing-code recovery path.
        let _lost = lock_receipts.mint(Action::Lock, (t0 + 800) as u32);
        let r2 = lock_receipts.mint(Action::Lock, (t0 + 900) as u32);
        assert!(g.feed_chain(0, r2.seq_code.unwrap(), t0 + 900).is_ok());
    }
}
