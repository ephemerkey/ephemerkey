//! Generator-side reveal scheduler: what the display is allowed to show.
//!
//! HARD REQUIREMENT (DESIGN-policies.md "poison mode"): real and decoy
//! reveals share this ONE code path, parameterized only by which secret they
//! mint from. The returned [`Reveal`] carries no is-decoy information unless
//! the `introspect` feature is on (emulator/tests only — never firmware).

use crate::engine::KeyDef;
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
        }
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
