//! Device configuration: the integer-keyed CBOR policy schema → the
//! [`ephemerkey_core`] engine. One decoder + one builder, shared by the STM32
//! firmware (parses the sealed config off flash) and the emulator (its device
//! twin), so on-device behavior and the simulator can never diverge.
//!
//! [`parse`] turns the CBOR config bytes into a [`DeviceModel`] (fixed-size,
//! no-alloc); [`build_generator`] / [`build_lock`] / [`build_validator`]
//! instantiate the core engine from it — the exact mapping the emulator used to
//! do inline.
//!
//! ## Wire schema (integer-keyed CBOR)
//!
//! Top-level map — keys 1-3 and 8 are also read by the geofence-only
//! [`ephemerkey_envelope::config`] parser:
//!
//! | key | field | type |
//! |----|--------|------|
//! | 1 | role         | uint (1 generator, 2 lock-controller) |
//! | 2 | staleness_s  | uint |
//! | 3 | zones        | `[[lat_e7, lon_e7, radius_m]]` |
//! | 4 | keys         | `[key]` |
//! | 5 | slots        | `[slot]` |
//! | 6 | calendars    | `[cal-window]` |
//! | 7 | confirm      | confirm-map |
//! | 8 | crit         | `[tstr]` — refused unless every entry is in `known_features` |
//! | 9 | unlock_window_s | uint — reveal window a ritual `Fired(unlock)` opens (generator; default 30) |
//!
//! **key**: 1 secret(bstr) · 2 digits · 3 decoy(idx) · 4 display · 5 chain ·
//! 6 zone(idx, reserved) · 7 gated(bool — cascade: reveal real only while
//! unlocked, else decoy/refuse). **display**: 1 mode(0 plain/1 scatter) · 2 dwell_ms ·
//! 3 reveal_s · 4 once(0 unlimited/1 refuse/2 decoy) · 5 gap_min_s. **chain**:
//! 1 secret · 2 digits · 3 mode(0 seq/1 time/2 both) · 4 action · 5
//! min_elapsed_s · 6 max_age_s. **slot**: 1 key · 2 action · 3 policy · 4
//! progress(bool) · 5 reset_on_invalid(bool) · 6 negative · 7 gates · 8
//! veto_delay_s · 9 veto_key · 10 budget. **policy** (field 1 = type FIRST):
//! 0 always; 1 sequence{2 n,3 window_s,4 gap_min_s,5 gap_max_s,6 delay_min_s,
//! 7 delay_max_s,8 jitter_s}; 2 path{2 leg_keys[],3 leg_deadline_s,4
//! delay_max_s}; 3 deadman{2 beat_s}; 4 quorum{2 m,3 keys[],4 window_s,5
//! alternating(bool),6 gap_min_s,7 gap_max_s}. **gates**: 1 fence · 2
//! stillness_s · 3 calendar. **negative**: `[0]` reset / `[1]` silent /
//! `[2, secs]` lockout. **confirm**: 1 secret · 2 digits · 3 mode.
//!
//! **action** everywhere: 0 unlock · 1 lock · 2 duress.

#![cfg_attr(not(test), no_std)]

use ephemerkey_core::engine::{KeyDef, LockEngine, MAX_SECRET};
use ephemerkey_core::policy::{
    Action, Gates, NegativeAction, Policy, Slot, MAX_KEYS, MAX_PATH_LEGS, MAX_QUORUM, MAX_SLOTS,
};
use ephemerkey_core::receipt::{ReceiptMode, Receipts, Validator};
use ephemerkey_core::reveal::{ChainSpec, DisplayMode, DisplaySpec, GenKey, Generator, OnceMode};
use ephemerkey_envelope::cbor::Dec;
pub use ephemerkey_envelope::config::{Role, Zone, DEFAULT_STALENESS_S, MAX_RADIUS_M, MAX_ZONES};

/// The feature tags this crate's `build` fully honors. A config whose `crit`
/// list (key 8) names anything outside the caller's known set is refused —
/// the firmware and emulator each pass their own capability list to [`parse`].
pub const SUPPORTED_FEATURES: &[&str] =
    &["seq-jitter", "quorum-pace", "chain", "veto", "budget", "cascade"];

/// Default reveal window (seconds) a ritual unlock opens when a generator
/// config omits `unlock_window_s`.
pub const DEFAULT_UNLOCK_WINDOW_S: u32 = 30;

/// Most calendar windows a single config may carry (fixed RAM footprint).
pub const MAX_CALENDARS: usize = 8;

/// A recurring time-of-week window. `days_mask` bit `i` = weekday `i` is active
/// (0 = Sunday … 6 = Saturday); `[start_min, end_min)` is the open interval in
/// minutes from midnight (UTC). Non-wrapping — a window that crosses midnight is
/// two entries.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct CalWindow {
    pub days_mask: u8,
    pub start_min: u16,
    pub end_min: u16,
}

impl CalWindow {
    /// Whether this window is open at `now_unix` (UTC). 1970-01-01 was a
    /// Thursday, so `weekday = (days + 4) mod 7` with 0 = Sunday.
    pub fn open_at(&self, now_unix: u64) -> bool {
        let weekday = (((now_unix / 86_400) % 7 + 4) % 7) as u8;
        if self.days_mask & (1 << weekday) == 0 {
            return false;
        }
        let minute = ((now_unix % 86_400) / 60) as u16;
        self.start_min <= minute && minute < self.end_min
    }
}

/// A device's calendar table, snapshotted for a
/// [`Sensors`](ephemerkey_core::policy::Sensors) implementation: `open(window,
/// now)` answers the engine's `calendar_open` gate from the config windows plus
/// the current clock.
#[derive(Copy, Clone, Default)]
pub struct Calendars {
    wins: [Option<CalWindow>; MAX_CALENDARS],
}

impl Calendars {
    pub fn open(&self, window: u8, now_unix: u64) -> bool {
        self.wins
            .get(window as usize)
            .and_then(|w| *w)
            .map(|w| w.open_at(now_unix))
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Malformed CBOR, wrong type, or a missing required field.
    Malformed,
    /// A value out of range (role, digits, policy type, action).
    BadField,
    /// A secret longer than [`MAX_SECRET`], or more keys/slots/legs than the
    /// fixed tables hold.
    TooLong,
    /// `crit` names a feature the caller doesn't implement (never silently
    /// weaken a protection).
    Unsupported,
}

impl From<ephemerkey_envelope::Error> for Error {
    fn from(_: ephemerkey_envelope::Error) -> Self {
        Error::Malformed
    }
}

/// One key's build inputs: the secret/digits/decoy, plus the generator-side
/// display + receipt-chain when present.
#[derive(Copy, Clone)]
struct KeyEntry {
    def: KeyDef,
    display: Option<DisplaySpec>,
    chain: Option<ChainSpec>,
    /// Had a display or chain in the config, i.e. it's a generator key.
    is_gen: bool,
    /// Cascade: this reveal key is ritual-gated (key field 7). Real codes flow
    /// only while a ritual unlock window is open; otherwise poison/refuse.
    gated: bool,
}

/// The lock's confirm-TOTP identity (secret copied out of the config).
#[derive(Copy, Clone)]
struct ConfirmEntry {
    secret: [u8; MAX_SECRET],
    secret_len: u8,
    digits: u8,
    mode: ReceiptMode,
}

/// A fully-parsed device configuration, ready for the `build_*` functions. No
/// borrows of the source bytes (secrets are copied into fixed arrays), so it
/// can outlive the config buffer.
pub struct DeviceModel {
    pub role: Role,
    pub staleness_s: u32,
    /// Reveal window (seconds) a generator's ritual unlock opens. See
    /// [`build_ritual`] / [`ephemerkey_core::reveal::apply_ritual_outcome`].
    pub unlock_window_s: u32,
    zones: [Zone; MAX_ZONES],
    zone_count: usize,
    keys: [Option<KeyEntry>; MAX_KEYS],
    slots: [Option<Slot>; MAX_SLOTS],
    calendars: [Option<CalWindow>; MAX_CALENDARS],
    confirm: Option<ConfirmEntry>,
}

impl DeviceModel {
    /// The "shut" fail-safe: a Generator with no zones, keys, or slots — emits
    /// nothing. Used on a factory-fresh device and when a stored config fails
    /// to parse, so the firmware always has a valid (inert) model.
    pub fn shut_default() -> Self {
        Self::empty()
    }

    fn empty() -> Self {
        DeviceModel {
            role: Role::Generator,
            staleness_s: DEFAULT_STALENESS_S,
            unlock_window_s: DEFAULT_UNLOCK_WINDOW_S,
            zones: [Zone::default(); MAX_ZONES],
            zone_count: 0,
            keys: [None; MAX_KEYS],
            slots: [None; MAX_SLOTS],
            calendars: [None; MAX_CALENDARS],
            confirm: None,
        }
    }

    /// A snapshot of the calendar table for a `Sensors` impl to evaluate the
    /// `calendar_open` gate against the clock.
    pub fn calendars(&self) -> Calendars {
        Calendars { wins: self.calendars }
    }

    /// The configured geofence zones.
    pub fn zones(&self) -> &[Zone] {
        &self.zones[..self.zone_count]
    }

    /// Whether a point lies inside any configured fence (empty ⇒ false).
    pub fn in_any_fence(&self, lat_e7: i32, lon_e7: i32) -> bool {
        self.zones().iter().any(|z| z.contains(lat_e7, lon_e7))
    }
}

/// Parse the integer-keyed CBOR config. `known_features` is the caller's
/// implemented-feature list; a `crit` entry outside it fails with
/// [`Error::Unsupported`].
pub fn parse(cbor: &[u8], known_features: &[&str]) -> Result<DeviceModel, Error> {
    let mut d = Dec::new(cbor);
    let n = d.map()?;
    let mut m = DeviceModel::empty();
    let mut have_role = false;

    for _ in 0..n {
        match d.uint()? {
            1 => {
                m.role = role_from(d.uint()?)?;
                have_role = true;
            }
            2 => m.staleness_s = d.uint()? as u32,
            3 => parse_zones(&mut d, &mut m)?,
            4 => parse_keys(&mut d, &mut m)?,
            5 => parse_slots(&mut d, &mut m)?,
            6 => parse_calendars(&mut d, &mut m)?,
            7 => m.confirm = Some(parse_confirm(&mut d)?),
            8 => check_crit(&mut d, known_features)?,
            9 => m.unlock_window_s = d.uint()? as u32,
            _ => d.skip()?,
        }
    }
    if !have_role {
        return Err(Error::BadField);
    }
    Ok(m)
}

/// Instantiate the lock-side engine: key table, slot table, and (if a confirm
/// identity is set) the receipt minter.
pub fn build_lock(m: &DeviceModel) -> LockEngine {
    let mut lock = LockEngine::new();
    for (i, k) in m.keys.iter().enumerate() {
        if let Some(k) = k {
            lock.keys[i] = Some(k.def);
        }
    }
    for (i, s) in m.slots.iter().enumerate() {
        lock.slots[i] = *s;
    }
    if let Some(c) = &m.confirm {
        lock.receipts = Some(Receipts::new(c.secret(), c.digits, c.mode));
    }
    lock
}

/// Instantiate the generator: each key that carries a display or a chain
/// becomes a [`GenKey`] (with its decoy twin's secret resolved).
pub fn build_generator(m: &DeviceModel) -> Generator {
    let mut gen = Generator::new();
    for (i, k) in m.keys.iter().enumerate() {
        let Some(k) = k else { continue };
        if !k.is_gen {
            continue;
        }
        let decoy = k
            .def
            .decoy
            .and_then(|di| m.keys.get(di as usize).and_then(|o| o.as_ref()))
            .map(|dk| dk.def);
        let display = k.display.unwrap_or(DisplaySpec {
            mode: DisplayMode::Plain,
            dwell_ms: 800,
            reveal_s: 5,
            once: OnceMode::Unlimited,
            gap_min_s: 0,
        });
        gen.keys[i] = Some(GenKey { key: k.def, decoy, display, chain: k.chain });
        gen.set_gated(i, k.gated);
    }
    gen
}

/// The cascade ritual engine for a generator: the [`LockEngine`] over the same
/// key + slot table that validates dialed unlock codes. A `Fired(unlock)` from
/// it opens the reveal window (see
/// [`ephemerkey_core::reveal::apply_ritual_outcome`]); a generator with no
/// slots yields an inert engine that never fires. Structurally identical to
/// [`build_lock`] — the generator just consumes the outcome differently.
pub fn build_ritual(m: &DeviceModel) -> LockEngine {
    build_lock(m)
}

/// The validator a remote party (or the generator's chain) holds over the
/// lock's confirm secret, if one is configured.
pub fn build_validator(m: &DeviceModel) -> Option<Validator> {
    m.confirm
        .as_ref()
        .map(|c| Validator::new(c.secret(), c.digits, c.mode))
}

impl ConfirmEntry {
    fn secret(&self) -> &[u8] {
        &self.secret[..self.secret_len as usize]
    }
}

// ---- field parsers -------------------------------------------------------

fn parse_zones(d: &mut Dec, m: &mut DeviceModel) -> Result<(), Error> {
    let zn = d.array()?;
    m.zone_count = 0;
    for _ in 0..zn {
        // The single zone decoder lives in envelope alongside Zone + the
        // geofence math — reused here so there is only one zone parser.
        let zone = ephemerkey_envelope::config::parse_zone(d)?;
        if m.zone_count < MAX_ZONES {
            m.zones[m.zone_count] = zone;
            m.zone_count += 1;
        }
    }
    Ok(())
}

fn parse_keys(d: &mut Dec, m: &mut DeviceModel) -> Result<(), Error> {
    let kn = d.array()?;
    for i in 0..kn as usize {
        let entry = parse_key(d)?;
        if i >= MAX_KEYS {
            return Err(Error::TooLong);
        }
        m.keys[i] = Some(entry);
    }
    Ok(())
}

fn parse_key(d: &mut Dec) -> Result<KeyEntry, Error> {
    let n = d.map()?;
    let mut secret = [0u8; MAX_SECRET];
    let mut secret_len = 0u8;
    let mut have_secret = false;
    let mut digits = 6u8;
    let mut decoy: Option<u8> = None;
    let mut display: Option<DisplaySpec> = None;
    let mut chain: Option<ChainSpec> = None;
    let mut gated = false;

    for _ in 0..n {
        match d.uint()? {
            1 => {
                secret_len = read_secret(d, &mut secret)?;
                have_secret = true;
            }
            2 => digits = digits_from(d.uint()?)?,
            3 => decoy = Some(d.uint()? as u8),
            4 => display = Some(parse_display(d)?),
            5 => chain = Some(parse_chain(d)?),
            7 => gated = d.bool()?,
            _ => d.skip()?, // 6 zone-binding (reserved) + forward-compat
        }
    }
    if !have_secret {
        return Err(Error::BadField);
    }
    let def = KeyDef { secret, secret_len, digits, decoy };
    let is_gen = display.is_some() || chain.is_some();
    Ok(KeyEntry { def, display, chain, is_gen, gated })
}

fn parse_display(d: &mut Dec) -> Result<DisplaySpec, Error> {
    let n = d.map()?;
    let mut s = DisplaySpec {
        mode: DisplayMode::Plain,
        dwell_ms: 800,
        reveal_s: 5,
        once: OnceMode::Unlimited,
        gap_min_s: 0,
    };
    for _ in 0..n {
        match d.uint()? {
            1 => s.mode = if d.uint()? == 1 { DisplayMode::Scatter } else { DisplayMode::Plain },
            2 => s.dwell_ms = d.uint()? as u16,
            3 => s.reveal_s = d.uint()? as u16,
            4 => {
                s.once = match d.uint()? {
                    1 => OnceMode::Refuse,
                    2 => OnceMode::Decoy,
                    _ => OnceMode::Unlimited,
                }
            }
            5 => s.gap_min_s = d.uint()? as u16,
            _ => d.skip()?,
        }
    }
    Ok(s)
}

fn parse_chain(d: &mut Dec) -> Result<ChainSpec, Error> {
    let n = d.map()?;
    let mut secret = [0u8; MAX_SECRET];
    let mut secret_len = 0u8;
    let mut have_secret = false;
    let mut digits = 6u8;
    let mut mode = ReceiptMode::Sequence;
    let mut action = Action::Lock;
    let mut min_elapsed_s = 0u32;
    let mut max_age_s = 3600u32;

    for _ in 0..n {
        match d.uint()? {
            1 => {
                secret_len = read_secret(d, &mut secret)?;
                have_secret = true;
            }
            2 => digits = digits_from(d.uint()?)?,
            3 => mode = mode_from(d.uint()?)?,
            4 => action = action_from(d.uint()?)?,
            5 => min_elapsed_s = d.uint()? as u32,
            6 => max_age_s = d.uint()? as u32,
            _ => d.skip()?,
        }
    }
    if !have_secret {
        return Err(Error::BadField);
    }
    let mut validator = Validator::new(&secret[..secret_len as usize], digits, mode);
    validator.time_window_s = max_age_s;
    Ok(ChainSpec { validator, action, min_elapsed_s })
}

fn parse_slots(d: &mut Dec, m: &mut DeviceModel) -> Result<(), Error> {
    let sn = d.array()?;
    for i in 0..sn as usize {
        let slot = parse_slot(d)?;
        if i >= MAX_SLOTS {
            return Err(Error::TooLong);
        }
        m.slots[i] = Some(slot);
    }
    Ok(())
}

fn parse_slot(d: &mut Dec) -> Result<Slot, Error> {
    let n = d.map()?;
    let mut key = 0u8;
    let mut action = Action::Unlock;
    let mut policy = Policy::AlwaysValid;
    let mut have_policy = false;
    let mut show_progress = false;
    let mut reset_on_invalid = true;
    let mut negative = NegativeAction::Reset;
    let mut gates = Gates { own_fence: None, stillness_s: 0, calendar: None };
    let mut veto_delay_s = 0u16;
    let mut veto_key: Option<u8> = None;
    let mut budget = 0u16;

    for _ in 0..n {
        match d.uint()? {
            1 => key = d.uint()? as u8,
            2 => action = action_from(d.uint()?)?,
            3 => {
                policy = parse_policy(d)?;
                have_policy = true;
            }
            4 => show_progress = d.bool()?,
            5 => reset_on_invalid = d.bool()?,
            6 => negative = parse_negative(d)?,
            7 => gates = parse_gates(d)?,
            8 => veto_delay_s = d.uint()? as u16,
            9 => veto_key = Some(d.uint()? as u8),
            10 => budget = d.uint()? as u16,
            _ => d.skip()?,
        }
    }
    if !have_policy {
        return Err(Error::BadField);
    }
    Ok(Slot {
        key,
        policy,
        gates,
        action,
        show_progress,
        reset_on_invalid,
        negative,
        veto_delay_s,
        veto_key,
        budget,
    })
}

fn parse_policy(d: &mut Dec) -> Result<Policy, Error> {
    let n = d.map()?;
    if n == 0 || d.uint()? != 1 {
        return Err(Error::BadField); // the `type` field must come first
    }
    let ptype = d.uint()?;
    let rem = n - 1;
    match ptype {
        0 => {
            for _ in 0..rem {
                let _ = d.uint()?;
                d.skip()?;
            }
            Ok(Policy::AlwaysValid)
        }
        1 => parse_sequence(d, rem),
        2 => parse_path(d, rem),
        3 => parse_deadman(d, rem),
        4 => parse_quorum(d, rem),
        _ => Err(Error::BadField),
    }
}

fn parse_sequence(d: &mut Dec, rem: u64) -> Result<Policy, Error> {
    let (mut n, mut window_s, mut gap_min_s, mut gap_max_s) = (1u8, 0u16, 0u16, 0u16);
    let (mut delay_min_s, mut delay_max_s, mut jitter_s) = (0u16, 60u16, 0u16);
    for _ in 0..rem {
        match d.uint()? {
            2 => n = d.uint()? as u8,
            3 => window_s = d.uint()? as u16,
            4 => gap_min_s = d.uint()? as u16,
            5 => gap_max_s = d.uint()? as u16,
            6 => delay_min_s = d.uint()? as u16,
            7 => delay_max_s = d.uint()? as u16,
            8 => jitter_s = d.uint()? as u16,
            _ => d.skip()?,
        }
    }
    Ok(Policy::Sequence {
        n,
        window_s,
        gap_min_s,
        gap_max_s,
        delay_min_s,
        delay_max_s,
        pace_jitter_s: jitter_s,
    })
}

fn parse_path(d: &mut Dec, rem: u64) -> Result<Policy, Error> {
    let mut leg_keys = [0u8; MAX_PATH_LEGS];
    let mut legs = 0usize;
    let mut leg_deadline_s = 0u16;
    let mut delay_max_s = 60u16;
    for _ in 0..rem {
        match d.uint()? {
            2 => legs = read_idx_array(d, &mut leg_keys)?,
            3 => leg_deadline_s = d.uint()? as u16,
            4 => delay_max_s = d.uint()? as u16,
            _ => d.skip()?,
        }
    }
    Ok(Policy::Path { legs: legs as u8, leg_keys, leg_deadline_s, delay_max_s })
}

fn parse_deadman(d: &mut Dec, rem: u64) -> Result<Policy, Error> {
    let mut beat_s = 0u16;
    for _ in 0..rem {
        match d.uint()? {
            2 => beat_s = d.uint()? as u16,
            _ => d.skip()?,
        }
    }
    Ok(Policy::DeadMan { beat_s })
}

fn parse_quorum(d: &mut Dec, rem: u64) -> Result<Policy, Error> {
    let mut m = 0u8;
    let mut keys = [0u8; MAX_QUORUM];
    let mut n_keys = 0usize;
    let mut window_s = 0u16;
    let mut alternating = false;
    let mut gap_min_s = 0u16;
    let mut gap_max_s = u16::MAX;
    for _ in 0..rem {
        match d.uint()? {
            2 => m = d.uint()? as u8,
            3 => n_keys = read_idx_array(d, &mut keys)?,
            4 => window_s = d.uint()? as u16,
            5 => alternating = d.bool()?,
            6 => gap_min_s = d.uint()? as u16,
            7 => gap_max_s = d.uint()? as u16,
            _ => d.skip()?,
        }
    }
    Ok(Policy::Quorum {
        m,
        n_keys: n_keys as u8,
        keys,
        window_s,
        alternating,
        gap_min_s,
        gap_max_s,
    })
}

fn parse_gates(d: &mut Dec) -> Result<Gates, Error> {
    let n = d.map()?;
    let mut g = Gates { own_fence: None, stillness_s: 0, calendar: None };
    for _ in 0..n {
        match d.uint()? {
            1 => g.own_fence = Some(d.uint()? as u8),
            2 => g.stillness_s = d.uint()? as u16,
            3 => g.calendar = Some(d.uint()? as u8),
            _ => d.skip()?,
        }
    }
    Ok(g)
}

fn parse_negative(d: &mut Dec) -> Result<NegativeAction, Error> {
    let n = d.array()?;
    if n == 0 {
        return Err(Error::BadField);
    }
    let (neg, read) = match d.uint()? {
        0 => (NegativeAction::Reset, 1),
        1 => (NegativeAction::Silent, 1),
        2 => {
            if n < 2 {
                return Err(Error::BadField);
            }
            (NegativeAction::Lockout(d.uint()? as u16), 2)
        }
        _ => return Err(Error::BadField),
    };
    for _ in read..n {
        d.skip()?;
    }
    Ok(neg)
}

fn parse_calendars(d: &mut Dec, m: &mut DeviceModel) -> Result<(), Error> {
    let cn = d.array()?;
    for i in 0..cn as usize {
        let win = parse_calendar(d)?;
        if i >= MAX_CALENDARS {
            return Err(Error::TooLong);
        }
        m.calendars[i] = Some(win);
    }
    Ok(())
}

fn parse_calendar(d: &mut Dec) -> Result<CalWindow, Error> {
    let n = d.map()?;
    let mut w = CalWindow::default();
    for _ in 0..n {
        match d.uint()? {
            1 => w.days_mask = d.uint()? as u8,
            2 => w.start_min = d.uint()? as u16,
            3 => w.end_min = d.uint()? as u16,
            _ => d.skip()?,
        }
    }
    Ok(w)
}

fn parse_confirm(d: &mut Dec) -> Result<ConfirmEntry, Error> {
    let n = d.map()?;
    let mut secret = [0u8; MAX_SECRET];
    let mut secret_len = 0u8;
    let mut have_secret = false;
    let mut digits = 6u8;
    let mut mode = ReceiptMode::Sequence;
    for _ in 0..n {
        match d.uint()? {
            1 => {
                secret_len = read_secret(d, &mut secret)?;
                have_secret = true;
            }
            2 => digits = digits_from(d.uint()?)?,
            3 => mode = mode_from(d.uint()?)?,
            _ => d.skip()?,
        }
    }
    if !have_secret {
        return Err(Error::BadField);
    }
    Ok(ConfirmEntry { secret, secret_len, digits, mode })
}

fn check_crit(d: &mut Dec, known: &[&str]) -> Result<(), Error> {
    let n = d.array()?;
    for _ in 0..n {
        let name = d.tstr()?;
        if !known.contains(&name) {
            return Err(Error::Unsupported);
        }
    }
    Ok(())
}

// ---- leaf helpers --------------------------------------------------------

fn read_secret(d: &mut Dec, out: &mut [u8; MAX_SECRET]) -> Result<u8, Error> {
    let b = d.bstr()?;
    if b.len() > MAX_SECRET {
        return Err(Error::TooLong);
    }
    out[..b.len()].copy_from_slice(b);
    Ok(b.len() as u8)
}

/// Read a CBOR array of small unsigned integers (key/leg indices) into `out`,
/// returning the count. More than `out.len()` entries is [`Error::TooLong`].
fn read_idx_array(d: &mut Dec, out: &mut [u8]) -> Result<usize, Error> {
    let n = d.array()?;
    let mut count = 0usize;
    for _ in 0..n {
        let v = d.uint()? as u8;
        if count >= out.len() {
            return Err(Error::TooLong);
        }
        out[count] = v;
        count += 1;
    }
    Ok(count)
}

fn role_from(v: u64) -> Result<Role, Error> {
    match v {
        1 => Ok(Role::Generator),
        2 => Ok(Role::LockController),
        _ => Err(Error::BadField),
    }
}

fn action_from(v: u64) -> Result<Action, Error> {
    match v {
        0 => Ok(Action::Unlock),
        1 => Ok(Action::Lock),
        2 => Ok(Action::DuressUnlock),
        _ => Err(Error::BadField),
    }
}

fn mode_from(v: u64) -> Result<ReceiptMode, Error> {
    match v {
        0 => Ok(ReceiptMode::Sequence),
        1 => Ok(ReceiptMode::Time),
        2 => Ok(ReceiptMode::Both),
        _ => Err(Error::BadField),
    }
}

fn digits_from(v: u64) -> Result<u8, Error> {
    if (4..=10).contains(&v) {
        Ok(v as u8)
    } else {
        Err(Error::BadField)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ephemerkey_core::engine::Outcome;
    use ephemerkey_core::totp::totp_at;
    use ephemerkey_envelope::cbor::Enc;

    const FW: &[&str] = SUPPORTED_FEATURES;
    const NOW: u64 = 1_750_000_000;

    fn code_str<'a>(secret: &[u8], now: u64, digits: u8, buf: &'a mut [u8; 10]) -> &'a str {
        totp_at(secret, now, digits).render(buf)
    }

    #[test]
    fn lock_always_valid_fires() {
        let mut buf = [0u8; 256];
        let mut e = Enc::new(&mut buf);
        e.map(3).unwrap();
        e.uint(1).unwrap();
        e.uint(2).unwrap(); // role: lock
        e.uint(4).unwrap(); // keys
        e.array(1).unwrap();
        e.map(2).unwrap();
        e.uint(1).unwrap();
        e.bstr(b"MYSECRET").unwrap();
        e.uint(2).unwrap();
        e.uint(6).unwrap();
        e.uint(5).unwrap(); // slots
        e.array(1).unwrap();
        e.map(3).unwrap();
        e.uint(1).unwrap();
        e.uint(0).unwrap(); // key 0
        e.uint(2).unwrap();
        e.uint(0).unwrap(); // action unlock
        e.uint(3).unwrap(); // policy
        e.map(1).unwrap();
        e.uint(1).unwrap();
        e.uint(0).unwrap(); // type always
        let n = e.len();

        let m = parse(&buf[..n], FW).unwrap();
        assert_eq!(m.role, Role::LockController);
        let mut lock = build_lock(&m);
        let mut cb = [0u8; 10];
        let out = lock.enter_code(code_str(b"MYSECRET", NOW, 6, &mut cb), NOW);
        assert!(matches!(out, Outcome::Fired(0, Action::Unlock)), "got {:?}", out);
        // a wrong code matches nothing
        assert_eq!(lock.enter_code("000000", NOW), Outcome::Invalid);
    }

    #[test]
    fn lock_sequence_paces_and_fires() {
        // A 2-step sequence, generation gap 30..90 s, 10-min window.
        let mut buf = [0u8; 256];
        let mut e = Enc::new(&mut buf);
        e.map(3).unwrap();
        e.uint(1).unwrap();
        e.uint(2).unwrap();
        e.uint(4).unwrap();
        e.array(1).unwrap();
        e.map(2).unwrap();
        e.uint(1).unwrap();
        e.bstr(b"SEQKEY").unwrap();
        e.uint(2).unwrap();
        e.uint(6).unwrap();
        e.uint(5).unwrap();
        e.array(1).unwrap();
        e.map(2).unwrap();
        e.uint(1).unwrap();
        e.uint(0).unwrap();
        e.uint(3).unwrap(); // policy
        e.map(6).unwrap();
        e.uint(1).unwrap();
        e.uint(1).unwrap(); // type sequence
        e.uint(2).unwrap();
        e.uint(2).unwrap(); // n = 2
        e.uint(3).unwrap();
        e.uint(600).unwrap(); // window_s
        e.uint(4).unwrap();
        e.uint(30).unwrap(); // gap_min
        e.uint(5).unwrap();
        e.uint(90).unwrap(); // gap_max
        e.uint(7).unwrap();
        e.uint(120).unwrap(); // delay_max (accept codes minted up to 120s ago)
        let n = e.len();

        let m = parse(&buf[..n], FW).unwrap();
        let mut lock = build_lock(&m);
        let mut cb = [0u8; 10];
        // first code (minted 60 s ago), then a second minted 30 s later.
        let out1 = lock.enter_code(code_str(b"SEQKEY", NOW - 60, 6, &mut cb), NOW);
        assert!(matches!(out1, Outcome::Progress(0, 1, 2)), "got {:?}", out1);
        let out2 = lock.enter_code(code_str(b"SEQKEY", NOW - 30, 6, &mut cb), NOW);
        assert!(matches!(out2, Outcome::Fired(0, Action::Unlock)), "got {:?}", out2);
    }

    #[test]
    fn generator_reveals_and_confirm_builds_receipts() {
        // Generator key with a display; lock-style confirm identity present.
        let mut buf = [0u8; 256];
        let mut e = Enc::new(&mut buf);
        e.map(3).unwrap();
        e.uint(1).unwrap();
        e.uint(1).unwrap(); // role generator
        e.uint(4).unwrap();
        e.array(1).unwrap();
        e.map(3).unwrap();
        e.uint(1).unwrap();
        e.bstr(b"GENSECRET").unwrap();
        e.uint(2).unwrap();
        e.uint(6).unwrap();
        e.uint(4).unwrap(); // display
        e.map(1).unwrap();
        e.uint(1).unwrap();
        e.uint(1).unwrap(); // mode scatter
        e.uint(7).unwrap(); // confirm
        e.map(2).unwrap();
        e.uint(1).unwrap();
        e.bstr(b"CONFIRM").unwrap();
        e.uint(2).unwrap();
        e.uint(6).unwrap();
        let n = e.len();

        let m = parse(&buf[..n], FW).unwrap();
        assert_eq!(m.role, Role::Generator);
        let mut gen = build_generator(&m);
        let r = gen.reveal(0, NOW, 0x1234).expect("reveal");
        // the revealed code matches the key's TOTP
        assert_eq!(r.code, totp_at(b"GENSECRET", NOW, 6));
        assert!(build_validator(&m).is_some());
    }

    #[test]
    fn cascade_ritual_gates_generator() {
        // A generator whose reveal key 0 (with decoy twin key 1) is ritual-
        // gated; dialing the unlock code (key 2) via the ritual engine opens
        // the reveal window. crit:["cascade"] guards the downgrade.
        let mut buf = [0u8; 320];
        let mut e = Enc::new(&mut buf);
        e.map(5).unwrap();
        e.uint(1).unwrap();
        e.uint(1).unwrap(); // role generator
        e.uint(9).unwrap();
        e.uint(30).unwrap(); // unlock_window_s
        e.uint(4).unwrap(); // keys
        e.array(3).unwrap();
        // key 0: gated reveal key with display + decoy twin (idx 1)
        e.map(5).unwrap();
        e.uint(1).unwrap();
        e.bstr(b"REVEALSEC").unwrap();
        e.uint(2).unwrap();
        e.uint(6).unwrap();
        e.uint(3).unwrap();
        e.uint(1).unwrap(); // decoy = key 1
        e.uint(4).unwrap(); // display
        e.map(1).unwrap();
        e.uint(1).unwrap();
        e.uint(0).unwrap(); // mode plain
        e.uint(7).unwrap();
        e.bool(true).unwrap(); // gated
        // key 1: the poison twin
        e.map(2).unwrap();
        e.uint(1).unwrap();
        e.bstr(b"DECOYSEC0").unwrap();
        e.uint(2).unwrap();
        e.uint(6).unwrap();
        // key 2: the ritual unlock secret (dialed in)
        e.map(2).unwrap();
        e.uint(1).unwrap();
        e.bstr(b"UNLOCKSEC").unwrap();
        e.uint(2).unwrap();
        e.uint(6).unwrap();
        e.uint(5).unwrap(); // slots
        e.array(1).unwrap();
        e.map(3).unwrap();
        e.uint(1).unwrap();
        e.uint(2).unwrap(); // ritual over key 2
        e.uint(2).unwrap();
        e.uint(0).unwrap(); // action unlock
        e.uint(3).unwrap(); // policy
        e.map(1).unwrap();
        e.uint(1).unwrap();
        e.uint(0).unwrap(); // always
        e.uint(8).unwrap(); // crit
        e.array(1).unwrap();
        e.tstr("cascade").unwrap();
        let n = e.len();

        // Safety: a device that doesn't implement cascade must REFUSE, not
        // silently reveal ungated.
        assert!(matches!(parse(&buf[..n], &["seq-jitter"]), Err(Error::Unsupported)));

        let m = parse(&buf[..n], FW).unwrap();
        assert_eq!(m.role, Role::Generator);
        assert_eq!(m.unlock_window_s, 30);
        let mut gen = build_generator(&m);
        let mut ritual = build_ritual(&m);
        let mut cb = [0u8; 10];

        // Locked: reveal mints the indistinguishable poison twin.
        assert!(gen.reveal(0, NOW, 9).unwrap().decoy);

        // Dial the unlock code → ritual fires → window opens.
        let out = ritual.enter_code(code_str(b"UNLOCKSEC", NOW, 6, &mut cb), NOW);
        assert!(matches!(out, Outcome::Fired(0, Action::Unlock)), "got {:?}", out);
        ephemerkey_core::reveal::apply_ritual_outcome(&mut gen, &out, NOW, m.unlock_window_s);

        // Now real, and it is the reveal key's TOTP (not the decoy's).
        let r = gen.reveal(0, NOW + 1, 9).unwrap();
        assert!(!r.decoy);
        assert_eq!(r.code, totp_at(b"REVEALSEC", NOW + 1, 6));

        // After the window, poison again.
        assert!(gen.reveal(0, NOW + 31, 9).unwrap().decoy);
    }

    #[test]
    fn generator_zones_parse_and_gate() {
        // A generator with one fence; the model's in_any_fence uses the shared
        // zone decoder + geofence math.
        let mut buf = [0u8; 128];
        let mut e = Enc::new(&mut buf);
        e.map(2).unwrap();
        e.uint(1).unwrap();
        e.uint(1).unwrap(); // role generator
        e.uint(3).unwrap(); // zones
        e.array(1).unwrap();
        e.array(3).unwrap();
        e.int(473_766_000).unwrap();
        e.int(85_417_000).unwrap();
        e.uint(500).unwrap();
        let n = e.len();

        let m = parse(&buf[..n], FW).unwrap();
        assert_eq!(m.zones().len(), 1);
        assert!(m.in_any_fence(473_766_000, 85_417_000)); // dead centre
        assert!(!m.in_any_fence(473_766_000 + 179_660, 85_417_000)); // ~2 km north
    }

    #[test]
    fn calendar_window_evaluation() {
        // Mon–Fri (bits 1..=5), 09:00–17:00.
        let w = CalWindow { days_mask: 0b0111_1110, start_min: 540, end_min: 1020 };
        // 2024-01-01 00:00 UTC was a Monday (unix 1_704_067_200).
        let mon = 1_704_067_200u64;
        assert!(w.open_at(mon + 10 * 3600)); // Mon 10:00 — open
        assert!(!w.open_at(mon + 8 * 3600)); // Mon 08:00 — before window
        assert!(!w.open_at(mon + 18 * 3600)); // Mon 18:00 — after window
        assert!(!w.open_at(mon + 6 * 86_400 + 10 * 3600)); // Sunday 10:00 — wrong day
    }

    #[test]
    fn calendar_gates_a_slot() {
        // A lock whose always-valid slot is gated on calendar window 0.
        let mut buf = [0u8; 256];
        let mut e = Enc::new(&mut buf);
        e.map(4).unwrap();
        e.uint(1).unwrap();
        e.uint(2).unwrap(); // role lock
        e.uint(4).unwrap(); // keys
        e.array(1).unwrap();
        e.map(2).unwrap();
        e.uint(1).unwrap();
        e.bstr(b"CALKEY").unwrap();
        e.uint(2).unwrap();
        e.uint(6).unwrap();
        e.uint(5).unwrap(); // slots
        e.array(1).unwrap();
        e.map(4).unwrap();
        e.uint(1).unwrap();
        e.uint(0).unwrap(); // key 0
        e.uint(3).unwrap(); // policy
        e.map(1).unwrap();
        e.uint(1).unwrap();
        e.uint(0).unwrap(); // always
        e.uint(6).unwrap(); // negative
        e.array(1).unwrap();
        e.uint(0).unwrap(); // reset
        e.uint(7).unwrap(); // gates
        e.map(1).unwrap();
        e.uint(3).unwrap();
        e.uint(0).unwrap(); // calendar window 0
        e.uint(6).unwrap(); // calendars
        e.array(1).unwrap();
        e.map(3).unwrap();
        e.uint(1).unwrap();
        e.uint(0b0111_1110).unwrap(); // Mon–Fri
        e.uint(2).unwrap();
        e.uint(540).unwrap(); // 09:00
        e.uint(3).unwrap();
        e.uint(1020).unwrap(); // 17:00
        let n = e.len();

        let m = parse(&buf[..n], FW).unwrap();
        let cals = m.calendars();
        let mut lock = build_lock(&m);
        let mon = 1_704_067_200u64;

        struct Env(Calendars, u64);
        impl ephemerkey_core::policy::Sensors for Env {
            fn inside_fence(&self, _: u8) -> bool {
                true
            }
            fn still_for_s(&self) -> u32 {
                u32::MAX
            }
            fn calendar_open(&self, w: u8) -> bool {
                self.0.open(w, self.1)
            }
        }

        let mut cb = [0u8; 10];
        // Monday 10:00 — window open → fires.
        let open = mon + 10 * 3600;
        let out = lock.enter_code_with(code_str(b"CALKEY", open, 6, &mut cb), open, &Env(cals, open));
        assert!(matches!(out, Outcome::Fired(0, Action::Unlock)), "open: {:?}", out);
        // Monday 20:00 — window shut → gated (and NOT burned).
        let shut = mon + 20 * 3600;
        let out = lock.enter_code_with(code_str(b"CALKEY", shut, 6, &mut cb), shut, &Env(cals, shut));
        assert!(
            matches!(out, Outcome::Gated(0, ephemerkey_core::policy::GateBlock::Calendar)),
            "shut: {:?}",
            out
        );
    }

    #[test]
    fn unknown_crit_refused() {
        let mut buf = [0u8; 64];
        let mut e = Enc::new(&mut buf);
        e.map(2).unwrap();
        e.uint(1).unwrap();
        e.uint(2).unwrap();
        e.uint(8).unwrap(); // crit
        e.array(1).unwrap();
        e.tstr("time-travel").unwrap();
        let n = e.len();
        assert!(matches!(parse(&buf[..n], FW), Err(Error::Unsupported)));

        // a known feature is accepted
        let mut buf2 = [0u8; 64];
        let mut e = Enc::new(&mut buf2);
        e.map(2).unwrap();
        e.uint(1).unwrap();
        e.uint(2).unwrap();
        e.uint(8).unwrap();
        e.array(1).unwrap();
        e.tstr("veto").unwrap();
        let n = e.len();
        assert!(parse(&buf2[..n], FW).is_ok());
    }

    #[test]
    fn missing_role_and_oversized_secret_rejected() {
        // no role
        let mut buf = [0u8; 32];
        let mut e = Enc::new(&mut buf);
        e.map(1).unwrap();
        e.uint(2).unwrap();
        e.uint(90).unwrap();
        let n = e.len();
        assert!(matches!(parse(&buf[..n], FW), Err(Error::BadField)));

        // secret longer than MAX_SECRET
        let mut buf = [0u8; 128];
        let mut e = Enc::new(&mut buf);
        e.map(2).unwrap();
        e.uint(1).unwrap();
        e.uint(2).unwrap();
        e.uint(4).unwrap();
        e.array(1).unwrap();
        e.map(1).unwrap();
        e.uint(1).unwrap();
        e.bstr(&[b'x'; MAX_SECRET + 1]).unwrap();
        let n = e.len();
        assert!(matches!(parse(&buf[..n], FW), Err(Error::TooLong)));
    }
}
