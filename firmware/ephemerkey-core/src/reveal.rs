//! Generator-side reveal scheduler: what the display is allowed to show.
//!
//! HARD REQUIREMENT (DESIGN-policies.md "poison mode"): real and decoy
//! reveals share this ONE code path, parameterized only by which secret they
//! mint from. The returned [`Reveal`] carries no is-decoy information unless
//! the `introspect` feature is on (emulator/tests only — never firmware).

use crate::engine::KeyDef;
use crate::policy::MAX_KEYS;
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

#[derive(Copy, Clone)]
pub struct GenKey {
    pub key: KeyDef,
    /// Decoy twin secret (poison mode). MUST be display-indistinguishable:
    /// same digits, same spec — enforced by construction here.
    pub decoy: Option<KeyDef>,
    pub display: DisplaySpec,
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
}

#[derive(Copy, Clone, Default)]
struct GenKeyState {
    /// Unix time of the last REAL reveal (0 = never).
    last_real: u64,
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

        // Is a real reveal permitted right now?
        let real_ok = match gk.display.once {
            OnceMode::Unlimited => true,
            _ => st.last_real == 0 || now - st.last_real >= u64::from(gk.display.gap_min_s),
        };

        let (secret_key, is_decoy) = if real_ok {
            st.last_real = now;
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
}
