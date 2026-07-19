//! Code-slot policy model + per-slot state machines.
//!
//! Moved from the firmware scaffold; shared verbatim by the STM32 build and
//! the host emulator so behavior can never diverge. See DESIGN-policies.md.

use crate::totp::PERIOD_S;

pub const MAX_SLOTS: usize = 8;
pub const MAX_KEYS: usize = 8;
pub const MAX_PATH_LEGS: usize = 4;
pub const MAX_QUORUM: usize = 4;

/// Gates that must hold for codes to count toward a slot (composable).
/// The engine never touches hardware — it asks the caller-supplied
/// [`Sensors`] the questions below; the caller owns GNSS/accel/RTC state.
#[derive(Copy, Clone, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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

/// The first gate found shut, for telemetry and lock-side UX. A gate never
/// silently swallows a valid code — the caller reports exactly why the slot
/// wouldn't accept it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum GateBlock {
    /// Lock's own position is outside the required geofence.
    Fence,
    /// The lock has not been still long enough (motion/tamper).
    Stillness,
    /// Outside the permitted calendar window.
    Calendar,
}

/// The runtime environment gates evaluate against. Firmware implements this
/// over the real GNSS fix / LIS3DH activity / RTC; the emulator fakes it.
/// Keeping it a trait is what lets the exact same engine run on-device and
/// on a laptop without any hardware in the loop.
pub trait Sensors {
    /// Is the lock currently inside fence-table entry `fence`?
    fn inside_fence(&self, fence: u8) -> bool;
    /// Seconds the lock has been continuously still (accelerometer quiet).
    fn still_for_s(&self) -> u32;
    /// Is calendar-window `window` open at the current time?
    fn calendar_open(&self, window: u8) -> bool;
}

/// Every gate open — the environment for ungated call sites and unit tests
/// that don't model position/motion/time-of-day.
pub struct AllGatesOpen;
impl Sensors for AllGatesOpen {
    fn inside_fence(&self, _: u8) -> bool {
        true
    }
    fn still_for_s(&self) -> u32 {
        u32::MAX
    }
    fn calendar_open(&self, _: u8) -> bool {
        true
    }
}

impl Gates {
    /// The first gate that is currently shut, or `None` if the slot is live.
    /// Evaluated left-to-right (fence, then stillness, then calendar) so the
    /// reported reason is stable.
    pub fn block(&self, env: &impl Sensors) -> Option<GateBlock> {
        if let Some(f) = self.own_fence {
            if !env.inside_fence(f) {
                return Some(GateBlock::Fence);
            }
        }
        if self.stillness_s > 0 && env.still_for_s() < u32::from(self.stillness_s) {
            return Some(GateBlock::Stillness);
        }
        if let Some(w) = self.calendar {
            if !env.calendar_open(w) {
                return Some(GateBlock::Calendar);
            }
        }
        None
    }
}

#[derive(Copy, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
    /// `pace_jitter_s`: 0 = the classic fixed `[gap_min, gap_max]` accept
    /// window every step. Non-zero = RANDOMIZED pacing — after each accepted
    /// code the device draws the next step's effective window by tightening
    /// both bounds by up to `pace_jitter_s` (clamped so the window never
    /// empties): entries before the drawn minimum are a reject (reset),
    /// entries after the drawn maximum likewise. The drawn window is device-
    /// internal — the rhythm cannot be rehearsed, only felt out live.
    Sequence {
        n: u8,
        window_s: u16,
        gap_min_s: u16,
        gap_max_s: u16,
        delay_min_s: u16,
        delay_max_s: u16,
        pace_jitter_s: u16,
    },
    /// Ordered zone-keyed legs (walk-the-path / walk-away): leg i's code
    /// must mint from `leg_keys[i]`, legs strictly in minting order, each
    /// leg minted within `leg_deadline_s` of the previous one. All codes
    /// are entered at the end of the journey — any code minted up to
    /// `delay_max_s` ago is searchable, so the whole route replays from
    /// the notebook while the ORDER and PACE are proven by the counters.
    Path {
        legs: u8,
        leg_keys: [u8; MAX_PATH_LEGS],
        leg_deadline_s: u16,
        delay_max_s: u16,
    },
    /// First valid code fires the action; after that a fresh code must
    /// keep arriving every <= beat_s or the engine emits a re-lock event
    /// (see `LockEngine::take_relocks`) and the slot re-arms.
    DeadMan { beat_s: u16 },
    /// M of the listed keys (distinct generators), interleaved within
    /// `window_s` of the first contribution. Each key counts once.
    /// `alternating`: the same key twice in a row resets the round.
    /// `gap_min_s..gap_max_s` paces the CONTRIBUTIONS by generation
    /// cadence, exactly like Sequence (0..=65535 = unpaced): a paced
    /// quorum proves the parties acted in a deliberate shared rhythm,
    /// not that one courier replayed a stack of pre-minted codes.
    Quorum {
        m: u8,
        n_keys: u8,
        keys: [u8; MAX_QUORUM],
        window_s: u16,
        alternating: bool,
        gap_min_s: u16,
        gap_max_s: u16,
    },
}

impl Policy {
    /// The slot's arrival window relative to a code's minting time.
    pub fn delay_window(&self) -> (u32, u32) {
        match *self {
            Policy::Sequence {
                delay_min_s,
                delay_max_s,
                ..
            } => (u32::from(delay_min_s), u32::from(delay_max_s)),
            Policy::Path { delay_max_s, .. } => (0, u32::from(delay_max_s)),
            // Instant-entry default: current or previous period.
            _ => (0, 2 * PERIOD_S),
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Action {
    Unlock,
    Lock,
    /// Unlocks normally, flags the audit log, distinct confirm code.
    DuressUnlock,
}

/// Response to a decoy (negative) match — see `Verdict::Negative`.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum NegativeAction {
    /// Reset this slot's sequence state (and log).
    Reset,
    /// Hard lockout for N seconds (and log).
    Lockout(u16),
    /// No externally visible effect — duress telemetry only.
    Silent,
}

#[derive(Copy, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Slot {
    /// Key table index of the secret this slot validates against.
    pub key: u8,
    pub policy: Policy,
    pub gates: Gates,
    pub action: Action,
    /// Show sequence progress on the display, or stay indistinguishable
    /// from idle (decoy).
    pub show_progress: bool,
    /// A code matching NO slot resets this slot's sequence state.
    pub reset_on_invalid: bool,
    /// What a decoy match does to this slot.
    pub negative: NegativeAction,
}

/// Runtime state — RAM only, deliberately lost on power-cycle.
#[derive(Copy, Clone, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SlotState {
    pub count: u8,
    /// TOTP counter of the last accepted code (generation-cadence anchor).
    pub last_counter: u32,
    /// Arrival times (unix seconds truncated to u32) of window start / last code.
    pub window_start_s: u32,
    pub last_code_s: u32,
    /// DeadMan: fired and being kept alive by beats.
    pub sustained: bool,
    /// Quorum: bit i = quorum key index i has contributed this round.
    pub seen_mask: u8,
    /// Quorum: quorum key index of the last contribution (alternating rule).
    pub last_kidx: u8,
    /// Randomized pacing (Sequence with `pace_jitter_s > 0`): xorshift state
    /// and the currently drawn `[lo, hi]` accept window for the NEXT step.
    /// Device-internal; deliberately not part of any wire format.
    pub pace_rng: u32,
    pub pace_lo_s: u16,
    pub pace_hi_s: u16,
}

impl SlotState {
    fn pace_next(&mut self) -> u32 {
        // xorshift32; seeded per round in `on_code`
        let mut x = self.pace_rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.pace_rng = x;
        x
    }

    /// Draw the next step's effective accept window. Both bounds tighten by
    /// up to `jitter`, clamped to half the span each so the window can
    /// never empty.
    fn pace_draw(&mut self, gap_min_s: u16, gap_max_s: u16, jitter_s: u16) {
        if jitter_s == 0 {
            self.pace_lo_s = gap_min_s;
            self.pace_hi_s = gap_max_s;
            return;
        }
        let span = gap_max_s.saturating_sub(gap_min_s);
        let j = jitter_s.min(span / 2);
        let r1 = (self.pace_next() % (u32::from(j) + 1)) as u16;
        let r2 = (self.pace_next() % (u32::from(j) + 1)) as u16;
        self.pace_lo_s = gap_min_s + r1;
        self.pace_hi_s = gap_max_s - r2;
    }

    /// The currently drawn accept window (testing / emulator introspection).
    #[cfg(any(test, feature = "introspect"))]
    pub fn pace_window(&self) -> (u16, u16) {
        (self.pace_lo_s, self.pace_hi_s)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Verdict {
    /// Code accepted, sequence advanced (progress = count/needed).
    Progress(u8, u8),
    /// Policy satisfied — perform `Action` and emit the confirm code.
    Fire(Action),
    /// Timing/gate violation — this slot reset.
    Reset,
    /// Not for this slot / replayed counter — no state change.
    Ignored,
    /// DeadMan: sustain refreshed; nothing to actuate.
    Beat,
    /// Matched a DECOY key (`K_decoy`, generator poison mode): a definite
    /// squeezed-generator signal, not noise. Caller applies the configured
    /// severity (reset / lockout / silent duress telemetry) and always logs.
    Negative,
}

impl SlotState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Feed a validated code event at time `now_s`. `counter` is the TOTP
    /// counter the code matched — i.e. its generation time in periods; the
    /// caller has already verified the code AND that it fell inside the
    /// slot's delay window (it searched exactly that counter range).
    /// `kidx` identifies WHICH of the slot's candidate keys matched: the
    /// leg index for Path, the quorum key index for Quorum, 0 otherwise.
    /// Pure state machine — crypto and gate evaluation happen in the caller.
    pub fn on_code(&mut self, slot: &Slot, kidx: u8, counter: u32, now_s: u32) -> Verdict {
        if self.count > 0 && counter <= self.last_counter {
            return Verdict::Ignored; // counter burned (replay) or out of order
        }
        match slot.policy {
            Policy::AlwaysValid => Verdict::Fire(slot.action),
            Policy::Sequence {
                n,
                window_s,
                gap_min_s,
                gap_max_s,
                pace_jitter_s,
                ..
            } => {
                if self.count > 0 {
                    // Generation cadence: spacing between minting times,
                    // recovered from the matched counters, checked against
                    // the (possibly randomized) drawn accept window.
                    let gen_gap = (counter - self.last_counter) * PERIOD_S;
                    let in_window = now_s - self.window_start_s <= u32::from(window_s);
                    if gen_gap < u32::from(self.pace_lo_s)
                        || gen_gap > u32::from(self.pace_hi_s)
                        || !in_window
                    {
                        self.reset();
                        return Verdict::Reset;
                    }
                } else {
                    self.window_start_s = now_s;
                    // Seed the round's pacing RNG from the first code's
                    // minting counter + arrival time.
                    self.pace_rng = counter.wrapping_mul(0x9e37_79b9) ^ now_s | 1;
                }
                self.count += 1;
                self.last_counter = counter;
                self.last_code_s = now_s;
                if self.count >= n {
                    self.reset();
                    Verdict::Fire(slot.action)
                } else {
                    self.pace_draw(gap_min_s, gap_max_s, pace_jitter_s);
                    Verdict::Progress(self.count, n)
                }
            }
            Policy::Path {
                legs,
                leg_deadline_s,
                ..
            } => {
                // The caller only offers the CURRENT leg's key, so kidx ==
                // self.count by construction; legs advance strictly in
                // minting order (counter > last is already enforced above).
                debug_assert_eq!(kidx, self.count);
                if self.count > 0 {
                    let gen_gap = (counter - self.last_counter) * PERIOD_S;
                    if gen_gap > u32::from(leg_deadline_s) {
                        self.reset();
                        return Verdict::Reset; // dawdled between legs
                    }
                }
                self.count += 1;
                self.last_counter = counter;
                self.last_code_s = now_s;
                if self.count >= legs {
                    self.reset();
                    Verdict::Fire(slot.action)
                } else {
                    Verdict::Progress(self.count, legs)
                }
            }
            Policy::DeadMan { .. } => {
                self.last_counter = counter;
                self.last_code_s = now_s;
                if self.sustained {
                    Verdict::Beat
                } else {
                    self.sustained = true;
                    self.count = 1; // engage the replay/out-of-order guard
                    Verdict::Fire(slot.action)
                }
            }
            Policy::Quorum {
                m,
                window_s,
                alternating,
                gap_min_s,
                gap_max_s,
                ..
            } => {
                if self.count > 0 {
                    let in_window = now_s - self.window_start_s <= u32::from(window_s);
                    // Paced quorum: contributions must keep the configured
                    // generation cadence (0..=65535 = unpaced).
                    let gen_gap = (counter - self.last_counter) * PERIOD_S;
                    let paced_ok =
                        gen_gap >= u32::from(gap_min_s) && gen_gap <= u32::from(gap_max_s);
                    if !in_window || !paced_ok || (alternating && kidx == self.last_kidx) {
                        self.reset();
                        return Verdict::Reset;
                    }
                    if self.seen_mask & (1 << kidx) != 0 {
                        // This generator already contributed; nothing new.
                        self.last_counter = counter;
                        return Verdict::Ignored;
                    }
                } else {
                    self.window_start_s = now_s;
                }
                self.seen_mask |= 1 << kidx;
                self.last_kidx = kidx;
                self.count += 1;
                self.last_counter = counter;
                self.last_code_s = now_s;
                if self.count >= m {
                    self.reset();
                    Verdict::Fire(slot.action)
                } else {
                    Verdict::Progress(self.count, m)
                }
            }
        }
    }

    /// Time-driven expiry: called periodically; resets a stale sequence even
    /// if no further code ever arrives (so `show_progress` displays decay).
    /// Expiry is on ARRIVAL time, so it allows the generation gap plus the
    /// delay-window spread (a +30..35 min code can arrive 5 min "late"
    /// relative to its minting cadence).
    ///
    /// Returns `true` when a DeadMan sustain just expired — the caller must
    /// perform the re-lock (the opposite of the slot's action).
    pub fn tick(&mut self, slot: &Slot, now_s: u32) -> bool {
        match slot.policy {
            Policy::Sequence {
                gap_max_s,
                delay_min_s,
                delay_max_s,
                ..
            } => {
                let slack = u32::from(delay_max_s - delay_min_s);
                if self.count > 0 && now_s - self.last_code_s > u32::from(gap_max_s) + slack {
                    self.reset();
                }
                false
            }
            Policy::Quorum { window_s, .. } => {
                if self.count > 0 && now_s - self.window_start_s > u32::from(window_s) {
                    self.reset();
                }
                false
            }
            Policy::DeadMan { beat_s } => {
                if self.sustained && now_s - self.last_code_s > u32::from(beat_s) {
                    self.reset();
                    return true; // re-lock due
                }
                false
            }
            // Path pace is generation-time; nothing arrival-driven to expire.
            Policy::AlwaysValid | Policy::Path { .. } => false,
        }
    }
}
