//! ekemu — local emulator for an ephemerkey generator + lock pair.
//!
//! Runs the EXACT `ephemerkey-core` engines the firmware ships, under a
//! virtual clock, with the display/key-entry surfaces mimicked in text.
//! Scriptable: commands from stdin (or a file via shell redirection), one
//! per line; `expect` makes scripts self-checking (non-zero exit on miss).
//!
//!   ekemu scenario.json < script.txt
//!
//! Commands:
//!   time +<n>[s|m|h]        advance the virtual clock
//!   time set <unix>         jump the clock
//!   gen reveal [key]        request a code reveal (default key 0)
//!   gen unlock <code|@N>    cascade: dial a code into the generator's unlock
//!                           ritual (@N = a predecessor device's code)
//!   gen dial <d…>|bs|clr|go simulate the 3-button dial (shows the display);
//!                           `go` submits the dialed code to the ritual
//!   gen key <n>             select key n in the UNLOCKED display
//!   gen screen              paint the generator's current OLED face
//!   gen next [key]          seconds until the next REAL reveal
//!   lock enter <code|@N>    type a code at the lock (@N = Nth revealed
//!                           code, 1-based — the "written down" notebook)
//!   lock status             slot states
//!   validate <@R>           relay the Rth emitted receipt to the remote
//!                           validator (confirm-TOTP verification)
//!   env ...                 drive the virtual GNSS/accel/RTC (see cmd_env)
//!   expect <substring>      assert the last event line contains this
//!   echo <text> | # ...     narration / comments
//!   quit

use ephemerkey_core::engine::{LockEngine, Outcome};
use ephemerkey_core::policy::{Action, Sensors};
use ephemerkey_core::receipt::{Receipt, ReceiptCheck, ReceiptMode, Validator};
use ephemerkey_core::totp::Code;
use ephemerkey_core::reveal::{ChainErr, DisplayMode, Generator, RevealErr};
use ephemerkey_config::Calendars;
use ephemerkey_ui::{render, Align, Screen, Size, View};
use serde::Deserialize;
use std::io::BufRead;

mod serial;

// ---------------------------------------------------------------- scenario

#[derive(Deserialize)]
struct Scenario {
    #[serde(default = "default_start")]
    start_time: u64,
    #[serde(default = "default_seed")]
    seed: u32,
    keys: Vec<KeyCfg>,
    slots: Vec<SlotCfg>,
    /// Cascade: seconds a ritual unlock keeps the generator's real reveals open
    /// (the slots double as the generator's unlock ritual — dial codes with
    /// `gen unlock`). Only meaningful when a key is `gated`.
    #[serde(default = "default_unlock_window")]
    unlock_window_s: u32,
    /// Critical features this config depends on: every entry must be in
    /// KNOWN_FEATURES or the scenario/config is refused outright.
    #[serde(default)]
    crit: Vec<String>,
    /// The lock's own confirm-TOTP identity (receipts it mints on every
    /// fire/relock). Absent = the lock stays silent.
    confirm: Option<ConfirmCfg>,
    /// Calendar windows the slot `calendar` gates reference (time-of-week).
    #[serde(default)]
    calendars: Vec<CalendarCfg>,
}

/// A recurring time-of-week window: `days` (0 = Sunday … 6 = Saturday) plus a
/// `HH:MM`–`HH:MM` interval. Mirrors the console's CalendarWindow.
#[derive(Deserialize)]
struct CalendarCfg {
    #[serde(default)]
    days: Vec<u8>,
    #[serde(default)]
    start: String,
    #[serde(default)]
    end: String,
}
fn default_start() -> u64 {
    1_750_000_000
}
fn default_unlock_window() -> u32 {
    30
}

#[derive(Deserialize)]
struct ConfirmCfg {
    secret: String,
    #[serde(default = "default_digits")]
    digits: u8,
    /// "sequence" | "time" | "both"
    #[serde(default)]
    mode: ReceiptModeCfg,
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum ReceiptModeCfg {
    #[default]
    Sequence,
    Time,
    Both,
}
impl From<ReceiptModeCfg> for ReceiptMode {
    fn from(m: ReceiptModeCfg) -> Self {
        match m {
            ReceiptModeCfg::Sequence => ReceiptMode::Sequence,
            ReceiptModeCfg::Time => ReceiptMode::Time,
            ReceiptModeCfg::Both => ReceiptMode::Both,
        }
    }
}
fn default_seed() -> u32 {
    0x5EED_C0DE
}

#[derive(Deserialize)]
struct KeyCfg {
    secret: String,
    #[serde(default = "default_digits")]
    digits: u8,
    /// index of this key's decoy twin in `keys`
    decoy: Option<u8>,
    display: Option<DisplayCfg>,
    /// Receipt chain: this key's REAL reveals require feeding the lock's
    /// receipt code back into the generator, then a cooling-off.
    chain: Option<ChainCfg>,
    /// Cascade: this reveal key is ritual-gated. Its real code flows only while
    /// a ritual unlock window is open (see `gen unlock`); otherwise it mints the
    /// poison twin (if `decoy` set) or refuses.
    #[serde(default)]
    gated: bool,
}

#[derive(Deserialize)]
struct ChainCfg {
    /// The lock's confirm secret (the generator holds a Validator over it).
    secret: String,
    #[serde(default = "default_digits")]
    digits: u8,
    #[serde(default)]
    mode: ReceiptModeCfg,
    /// Which receipt action feeds the chain ("lock" = witnessed it locked).
    #[serde(default = "default_chain_action")]
    action: String,
    /// Reveals resume this long after a receipt is fed.
    #[serde(default)]
    min_elapsed_s: u32,
    /// Accepted receipt age for TIME-mode proofs (travel time to a far
    /// generator). Sequence proofs are ageless by construction.
    #[serde(default = "default_max_age")]
    max_age_s: u32,
}
fn default_chain_action() -> String {
    "lock".into()
}
fn default_max_age() -> u32 {
    3600
}
fn default_digits() -> u8 {
    6
}

#[derive(Deserialize)]
struct DisplayCfg {
    #[serde(default)]
    mode: ModeCfg,
    #[serde(default = "default_dwell")]
    dwell_ms: u16,
    #[serde(default = "default_reveal")]
    reveal_s: u16,
    #[serde(default)]
    once: OnceCfg,
    #[serde(default)]
    gap_min_s: u16,
}
fn default_dwell() -> u16 {
    800
}
fn default_reveal() -> u16 {
    5
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum ModeCfg {
    #[default]
    Plain,
    Scatter,
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum OnceCfg {
    #[default]
    Unlimited,
    Refuse,
    Decoy,
}

#[derive(Deserialize)]
struct SlotCfg {
    key: u8,
    #[serde(default = "default_action")]
    action: String,
    policy: PolicyCfg,
    #[serde(default)]
    progress: bool,
    #[serde(default = "default_true")]
    reset_on_invalid: bool,
    /// "reset" | "lockout:<secs>" | "silent"
    #[serde(default = "default_negative")]
    negative: String,
    #[serde(default)]
    gates: GatesCfg,
    /// Veto window: 0 = fire immediately; >0 = arm, fire after this many
    /// seconds unless a valid code from veto_key cancels.
    #[serde(default)]
    veto_delay_s: u16,
    veto_key: Option<u8>,
    /// 0 = unlimited; otherwise the slot dies after this many fires.
    #[serde(default)]
    budget: u16,
}

/// Position / motion / calendar gates on a slot. Evaluated against the
/// emulator's virtual `Env` (driven by `env` REPL commands).
#[derive(Deserialize, Default)]
struct GatesCfg {
    /// fence-table index the lock must be inside (portable locks)
    fence: Option<u8>,
    /// seconds of stillness required (0 = no stillness gate)
    #[serde(default)]
    stillness_s: u16,
    /// calendar-window index that must be open
    calendar: Option<u8>,
}
fn default_action() -> String {
    "unlock".into()
}
fn default_true() -> bool {
    true
}
fn default_negative() -> String {
    "reset".into()
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum PolicyCfg {
    Always,
    Sequence {
        n: u8,
        window_s: u16,
        gap_min_s: u16,
        gap_max_s: u16,
        #[serde(default)]
        delay_min_s: u16,
        #[serde(default = "default_delay_max")]
        delay_max_s: u16,
        /// 0 = fixed pacing; >0 = each step's accept window is randomly
        /// tightened by up to this many seconds (device-internal draw).
        #[serde(default)]
        jitter_s: u16,
    },
    Path {
        leg_keys: Vec<u8>,
        leg_deadline_s: u16,
        delay_max_s: u16,
    },
    DeadMan {
        beat_s: u16,
    },
    Quorum {
        m: u8,
        keys: Vec<u8>,
        window_s: u16,
        #[serde(default)]
        alternating: bool,
        /// Paced quorum: contributions must keep this generation cadence
        /// (defaults = unpaced).
        #[serde(default)]
        gap_min_s: u16,
        #[serde(default = "default_gap_max")]
        gap_max_s: u16,
    },
}
fn default_gap_max() -> u16 {
    u16::MAX
}
fn default_delay_max() -> u16 {
    60
}

/// Everything this emulated device implements (policy features from the
/// shared engine + the platform gates the virtual env provides).
pub fn known_features() -> Vec<&'static str> {
    let mut f = ephemerkey_core::SUPPORTED_POLICY_FEATURES.to_vec();
    f.extend(["zones", "calendars", "cascade"]);
    f
}

/// Build the generator + lock + remote validator from a scenario — by
/// ENCODING the scenario to the pinned integer-keyed CBOR config and running it
/// back through the shared `ephemerkey-config` decoder/builder, exactly as the
/// firmware does on a sealed config. So the emulator exercises the same CBOR
/// path the device ships (no separate mapping to drift out of sync), and its
/// simulation genuinely runs on CBOR data.
fn build(scn: &Scenario) -> (Generator, LockEngine, LockEngine, Option<Validator>, Calendars, u32) {
    for c in &scn.crit {
        assert!(
            known_features().contains(&c.as_str()),
            "config requires unsupported critical feature '{c}' — refusing"
        );
    }
    let cbor = scenario_to_cbor(scn);
    let model = ephemerkey_config::parse(&cbor, &known_features())
        .expect("scenario → cbor → model");
    (
        ephemerkey_config::build_generator(&model),
        ephemerkey_config::build_lock(&model),
        // The generator's cascade ritual: the same slot table, consumed as an
        // unlock ritual rather than a lock-consumer (see `gen unlock`).
        ephemerkey_config::build_ritual(&model),
        ephemerkey_config::build_validator(&model),
        model.calendars(),
        model.unlock_window_s,
    )
}

// ---- scenario → integer-keyed CBOR config (see ephemerkey_config schema) ---

/// Encode a scenario as the sealed config the firmware parses. The role field
/// is a placeholder (the emulator simulates both sides; the `build_*` functions
/// ignore role), so only keys/slots/confirm/crit carry meaning here.
fn scenario_to_cbor(scn: &Scenario) -> Vec<u8> {
    use ephemerkey_envelope::cbor::Enc;
    let mut buf = [0u8; 4096];
    let mut e = Enc::new(&mut buf);
    // A generator cascade is in play when any key is gated; only then emit the
    // unlock window (and the config must carry crit:["cascade"] to be accepted).
    let has_cascade = scn.keys.iter().any(|k| k.gated);
    let mut top = 1u64; // role
    top += !scn.keys.is_empty() as u64;
    top += !scn.slots.is_empty() as u64;
    top += !scn.calendars.is_empty() as u64;
    top += scn.confirm.is_some() as u64;
    top += !scn.crit.is_empty() as u64;
    top += has_cascade as u64;
    e.map(top).unwrap();
    e.uint(1).unwrap();
    e.uint(1).unwrap(); // placeholder role
    if has_cascade {
        e.uint(9).unwrap();
        e.uint(scn.unlock_window_s as u64).unwrap();
    }
    if !scn.keys.is_empty() {
        e.uint(4).unwrap();
        e.array(scn.keys.len() as u64).unwrap();
        for k in &scn.keys {
            enc_key(&mut e, k);
        }
    }
    if !scn.slots.is_empty() {
        e.uint(5).unwrap();
        e.array(scn.slots.len() as u64).unwrap();
        for s in &scn.slots {
            enc_slot(&mut e, s);
        }
    }
    if !scn.calendars.is_empty() {
        e.uint(6).unwrap();
        e.array(scn.calendars.len() as u64).unwrap();
        for c in &scn.calendars {
            e.map(3).unwrap();
            e.uint(1).unwrap();
            e.uint(days_mask(&c.days)).unwrap();
            e.uint(2).unwrap();
            e.uint(hhmm_min(&c.start)).unwrap();
            e.uint(3).unwrap();
            e.uint(hhmm_min(&c.end)).unwrap();
        }
    }
    if let Some(c) = &scn.confirm {
        e.uint(7).unwrap();
        e.map(3).unwrap();
        e.uint(1).unwrap();
        e.bstr(c.secret.as_bytes()).unwrap();
        e.uint(2).unwrap();
        e.uint(c.digits as u64).unwrap();
        e.uint(3).unwrap();
        e.uint(mode_u(c.mode)).unwrap();
    }
    if !scn.crit.is_empty() {
        e.uint(8).unwrap();
        e.array(scn.crit.len() as u64).unwrap();
        for c in &scn.crit {
            e.tstr(c).unwrap();
        }
    }
    let n = e.len();
    buf[..n].to_vec()
}

fn enc_key(e: &mut ephemerkey_envelope::cbor::Enc<'_>, k: &KeyCfg) {
    let mut cnt = 2u64; // secret + digits
    cnt += k.decoy.is_some() as u64;
    cnt += k.display.is_some() as u64;
    cnt += k.chain.is_some() as u64;
    cnt += k.gated as u64;
    e.map(cnt).unwrap();
    e.uint(1).unwrap();
    e.bstr(k.secret.as_bytes()).unwrap();
    e.uint(2).unwrap();
    e.uint(k.digits as u64).unwrap();
    if let Some(d) = k.decoy {
        e.uint(3).unwrap();
        e.uint(d as u64).unwrap();
    }
    if let Some(d) = &k.display {
        e.uint(4).unwrap();
        e.map(5).unwrap();
        e.uint(1).unwrap();
        e.uint(match d.mode {
            ModeCfg::Plain => 0,
            ModeCfg::Scatter => 1,
        })
        .unwrap();
        e.uint(2).unwrap();
        e.uint(d.dwell_ms as u64).unwrap();
        e.uint(3).unwrap();
        e.uint(d.reveal_s as u64).unwrap();
        e.uint(4).unwrap();
        e.uint(match d.once {
            OnceCfg::Unlimited => 0,
            OnceCfg::Refuse => 1,
            OnceCfg::Decoy => 2,
        })
        .unwrap();
        e.uint(5).unwrap();
        e.uint(d.gap_min_s as u64).unwrap();
    }
    if let Some(c) = &k.chain {
        e.uint(5).unwrap();
        e.map(6).unwrap();
        e.uint(1).unwrap();
        e.bstr(c.secret.as_bytes()).unwrap();
        e.uint(2).unwrap();
        e.uint(c.digits as u64).unwrap();
        e.uint(3).unwrap();
        e.uint(mode_u(c.mode)).unwrap();
        e.uint(4).unwrap();
        e.uint(action_u(&c.action)).unwrap();
        e.uint(5).unwrap();
        e.uint(c.min_elapsed_s as u64).unwrap();
        e.uint(6).unwrap();
        e.uint(c.max_age_s as u64).unwrap();
    }
    if k.gated {
        e.uint(7).unwrap();
        e.bool(true).unwrap();
    }
}

fn enc_slot(e: &mut ephemerkey_envelope::cbor::Enc<'_>, s: &SlotCfg) {
    let mut cnt = 9u64; // fields 1-8 + 10
    cnt += s.veto_key.is_some() as u64;
    e.map(cnt).unwrap();
    e.uint(1).unwrap();
    e.uint(s.key as u64).unwrap();
    e.uint(2).unwrap();
    e.uint(action_u(&s.action)).unwrap();
    e.uint(3).unwrap();
    enc_policy(e, &s.policy);
    e.uint(4).unwrap();
    e.bool(s.progress).unwrap();
    e.uint(5).unwrap();
    e.bool(s.reset_on_invalid).unwrap();
    e.uint(6).unwrap();
    enc_negative(e, &s.negative);
    e.uint(7).unwrap();
    enc_gates(e, &s.gates);
    e.uint(8).unwrap();
    e.uint(s.veto_delay_s as u64).unwrap();
    if let Some(vk) = s.veto_key {
        e.uint(9).unwrap();
        e.uint(vk as u64).unwrap();
    }
    e.uint(10).unwrap();
    e.uint(s.budget as u64).unwrap();
}

fn enc_policy(e: &mut ephemerkey_envelope::cbor::Enc<'_>, p: &PolicyCfg) {
    match p {
        PolicyCfg::Always => {
            e.map(1).unwrap();
            e.uint(1).unwrap();
            e.uint(0).unwrap();
        }
        PolicyCfg::Sequence {
            n,
            window_s,
            gap_min_s,
            gap_max_s,
            delay_min_s,
            delay_max_s,
            jitter_s,
        } => {
            e.map(8).unwrap();
            e.uint(1).unwrap();
            e.uint(1).unwrap();
            for (k, v) in [
                (2, *n as u64),
                (3, *window_s as u64),
                (4, *gap_min_s as u64),
                (5, *gap_max_s as u64),
                (6, *delay_min_s as u64),
                (7, *delay_max_s as u64),
                (8, *jitter_s as u64),
            ] {
                e.uint(k).unwrap();
                e.uint(v).unwrap();
            }
        }
        PolicyCfg::Path {
            leg_keys,
            leg_deadline_s,
            delay_max_s,
        } => {
            e.map(4).unwrap();
            e.uint(1).unwrap();
            e.uint(2).unwrap();
            e.uint(2).unwrap();
            e.array(leg_keys.len() as u64).unwrap();
            for k in leg_keys {
                e.uint(*k as u64).unwrap();
            }
            e.uint(3).unwrap();
            e.uint(*leg_deadline_s as u64).unwrap();
            e.uint(4).unwrap();
            e.uint(*delay_max_s as u64).unwrap();
        }
        PolicyCfg::DeadMan { beat_s } => {
            e.map(2).unwrap();
            e.uint(1).unwrap();
            e.uint(3).unwrap();
            e.uint(2).unwrap();
            e.uint(*beat_s as u64).unwrap();
        }
        PolicyCfg::Quorum {
            m,
            keys,
            window_s,
            alternating,
            gap_min_s,
            gap_max_s,
        } => {
            e.map(7).unwrap();
            e.uint(1).unwrap();
            e.uint(4).unwrap();
            e.uint(2).unwrap();
            e.uint(*m as u64).unwrap();
            e.uint(3).unwrap();
            e.array(keys.len() as u64).unwrap();
            for k in keys {
                e.uint(*k as u64).unwrap();
            }
            e.uint(4).unwrap();
            e.uint(*window_s as u64).unwrap();
            e.uint(5).unwrap();
            e.bool(*alternating).unwrap();
            e.uint(6).unwrap();
            e.uint(*gap_min_s as u64).unwrap();
            e.uint(7).unwrap();
            e.uint(*gap_max_s as u64).unwrap();
        }
    }
}

fn enc_gates(e: &mut ephemerkey_envelope::cbor::Enc<'_>, g: &GatesCfg) {
    let mut cnt = 1u64; // stillness always
    cnt += g.fence.is_some() as u64;
    cnt += g.calendar.is_some() as u64;
    e.map(cnt).unwrap();
    if let Some(f) = g.fence {
        e.uint(1).unwrap();
        e.uint(f as u64).unwrap();
    }
    e.uint(2).unwrap();
    e.uint(g.stillness_s as u64).unwrap();
    if let Some(c) = g.calendar {
        e.uint(3).unwrap();
        e.uint(c as u64).unwrap();
    }
}

fn enc_negative(e: &mut ephemerkey_envelope::cbor::Enc<'_>, s: &str) {
    if s == "silent" {
        e.array(1).unwrap();
        e.uint(1).unwrap();
    } else if let Some(secs) = s.strip_prefix("lockout:") {
        e.array(2).unwrap();
        e.uint(2).unwrap();
        e.uint(secs.parse::<u64>().expect("lockout secs")).unwrap();
    } else {
        e.array(1).unwrap();
        e.uint(0).unwrap();
    }
}

fn action_u(s: &str) -> u64 {
    match s {
        "lock" => 1,
        "duress" => 2,
        _ => 0, // unlock
    }
}

fn mode_u(m: ReceiptModeCfg) -> u64 {
    match m {
        ReceiptModeCfg::Sequence => 0,
        ReceiptModeCfg::Time => 1,
        ReceiptModeCfg::Both => 2,
    }
}

/// Days list (0 = Sunday … 6 = Saturday) → the bit-`i`-per-day mask.
fn days_mask(days: &[u8]) -> u64 {
    days.iter().filter(|&&d| d < 7).fold(0u64, |m, &d| m | (1 << d))
}

/// `"HH:MM"` → minutes from midnight.
fn hhmm_min(s: &str) -> u64 {
    let mut it = s.split(':');
    let h: u64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let m: u64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    h * 60 + m
}

// ------------------------------------------------------------------- env

/// Virtual GNSS / accelerometer / RTC the gates evaluate against. Firmware
/// implements `Sensors` over real hardware; here the `env` commands drive the
/// position/motion inputs. The CALENDAR gate is NOT faked — like the firmware,
/// it is evaluated from the config's windows against the (virtual) clock, so
/// `time set` moves in and out of a window. Position/motion default "shut" (out
/// of every fence, moving) so a gated slot genuinely gates until the script
/// puts the lock where it needs to be.
#[derive(Default)]
struct Env {
    /// bit f set => lock is inside fence f
    fences: u64,
    /// seconds the lock has been continuously still
    still_s: u32,
    /// calendar windows from the config, evaluated against `now`
    calendars: Calendars,
    /// current virtual unix time (synced from the emu clock before each entry)
    now: u64,
}
impl Sensors for Env {
    fn inside_fence(&self, fence: u8) -> bool {
        self.fences & (1 << fence) != 0
    }
    fn still_for_s(&self) -> u32 {
        self.still_s
    }
    fn calendar_open(&self, window: u8) -> bool {
        self.calendars.open(window, self.now)
    }
}

// ------------------------------------------------------------------- emu

struct Emu {
    now: u64,
    seed: u32,
    gen: Generator,
    lock: LockEngine,
    /// Cascade: the generator's unlock ritual engine (dialed codes → reveal
    /// window). Distinct from `lock`, which simulates a downstream lock consuming
    /// generated codes. Same slot table, different consumer.
    ritual: LockEngine,
    /// Seconds a ritual unlock keeps real reveals open.
    unlock_window_s: u32,
    env: Env,
    /// A remote party holding the lock's confirm secret (None = silent lock).
    validator: Option<Validator>,
    last_event: String,
    failures: u32,
    /// The digits dialed so far on the (simulated) 3-button keypad — feeds the
    /// `Dialing` display view; submitted with `gen dial go`.
    dial: Vec<u8>,
    /// Selected key in the UNLOCKED display view (`gen key <n>`).
    sel: u8,
    /// Every code the generator has shown, in order — the user's notebook.
    notebook: Vec<String>,
    /// Every receipt the lock has emitted, in order — relay with `validate @R`.
    receipts: Vec<Receipt>,
}

impl Emu {
    fn event(&mut self, s: String) {
        println!("[t={}] {}", self.now, s);
        self.last_event = s;
    }

    /// Preview a [`Screen`] in a wide, short bezel matching the 128×32 panel.
    /// This is the SAME `ephemerkey-ui` render the firmware blits, so the layout
    /// you see is the device's; a terminal can't scale fonts, so BIG lines are
    /// drawn wider (spaced glyphs) and one row taller to stand in for the font.
    fn paint(&self, screen: &Screen) {
        const W: usize = 32; // ~4 px per column across the 128 px face
        let mut rows: Vec<String> = Vec::new();
        for line in screen.lines() {
            let big = line.size() == Size::Big;
            let text: String = if big {
                // spaced glyphs ≈ the ~1.7× width of the big font
                line.text().chars().map(|c| format!("{c} ")).collect::<String>().trim_end().into()
            } else {
                line.text().into()
            };
            let text: String = text.chars().take(W).collect();
            let pad = W - text.chars().count();
            let left = match line.align() {
                Align::Left => 0,
                Align::Center => pad / 2,
                Align::Right => pad,
            };
            rows.push(format!("{}{text}{}", " ".repeat(left), " ".repeat(pad - left)));
            if big {
                rows.push(" ".repeat(W)); // the big line is twice as tall
            }
        }
        println!("    ┌{}┐", "─".repeat(W));
        for r in &rows {
            println!("    │{r}│");
        }
        println!("    └{}┘", "─".repeat(W));
    }

    /// Paint the generator's current display: a pending dial in progress, an
    /// open reveal window, or the idle/locked face.
    fn paint_screen(&self) {
        let view = if !self.dial.is_empty() {
            let (entered, cur) = self.dial.split_at(self.dial.len() - 1);
            View::Dialing { entered, cur: cur[0] }
        } else if self.gen.is_unlocked(self.now) {
            View::Unlocked { key: self.sel, secs_left: self.gen.unlock_secs_left(self.now) }
        } else {
            View::Locked
        };
        self.paint(&render(&view));
    }

    fn cmd(&mut self, line: &str) -> bool {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            if !line.is_empty() {
                println!("{line}");
            }
            return true;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["quit"] => return false,
            ["echo", rest @ ..] => println!("{}", rest.join(" ")),

            ["time", ..] => self.cmd_time(&words),
            ["env", ..] => self.cmd_env(&words),

            ["gen", "chain", code] => self.cmd_chain(code, 0),
            ["gen", "chain", code, k] => {
                let k: usize = k.parse().unwrap_or(0);
                self.cmd_chain(code, k)
            }
            ["gen", "dial", rest @ ..] => self.cmd_dial(rest),
            ["gen", "key", n] => {
                self.sel = n.parse().unwrap_or(0);
                self.paint_screen();
            }
            ["gen", "screen"] => self.paint_screen(),
            ["gen", "unlock", code] => self.cmd_unlock(code),
            ["gen", "reveal"] => self.cmd_reveal(0),
            ["gen", "reveal", k] => self.cmd_reveal(k.parse().unwrap_or(0)),
            ["gen", "next"] => self.cmd_next(0),
            ["gen", "next", k] => self.cmd_next(k.parse().unwrap_or(0)),

            ["lock", "enter", code] => {
                let code = if let Some(n) = code.strip_prefix('@') {
                    let i: usize = n.parse().unwrap_or(0);
                    match self.notebook.get(i.wrapping_sub(1)) {
                        Some(c) => c.clone(),
                        None => {
                            self.event(format!("lock: no notebook entry @{n}"));
                            return true;
                        }
                    }
                } else {
                    (*code).to_string()
                };
                self.env.now = self.now; // the calendar gate reads the clock
                let out = self.lock.enter_code_with(&code, self.now, &self.env);
                let s = describe(out);
                self.event(format!("lock: {s}"));
                // Relock + receipt consequences print (and become expectable)
                // after the outcome that caused them.
                self.drain_relocks();
                self.drain_receipts();
            }
            ["lock", "status"] => self.cmd_status(),

            // Attest button: mint a fresh receipt for the current state.
            ["lock", "attest"] => {
                self.lock.attest(Action::Lock, self.now);
                self.event("lock: ATTEST (state=locked)".into());
                self.drain_receipts();
            }
            ["lock", "attest", which] => {
                let a = match *which {
                    "unlock" => Action::Unlock,
                    _ => Action::Lock,
                };
                self.lock.attest(a, self.now);
                self.event(format!("lock: ATTEST (state={which})"));
                self.drain_receipts();
            }

            ["validate", r] => self.cmd_validate(r),

            ["expect", rest @ ..] => {
                let want = rest.join(" ");
                if self.last_event.contains(&want) {
                    println!("  ok: expect \"{want}\"");
                } else {
                    println!("  FAIL: expected \"{want}\" in \"{}\"", self.last_event);
                    self.failures += 1;
                }
            }
            _ => println!("? unknown: {line} (try: time/gen/lock/expect/quit)"),
        }
        true
    }

    fn drain_relocks(&mut self) {
        let mask = self.lock.take_relocks();
        for i in 0..8u8 {
            if mask & (1 << i) != 0 {
                self.event(format!("lock: RELOCK slot={i} (dead-man expired)"));
            }
        }
        let fires = self.lock.take_fires();
        for i in 0..8u8 {
            if fires & (1 << i) != 0 {
                self.event(format!("lock: FIRE slot={i} (veto window elapsed)"));
            }
        }
    }

    /// Surface any confirm-TOTP receipts the lock minted, and file them in
    /// the receipt book so `validate @R` can relay them to the validator.
    fn drain_receipts(&mut self) {
        while let Some(r) = self.lock.take_receipt() {
            self.receipts.push(r);
            let n = self.receipts.len();
            let mut buf = [0u8; 10];
            let seq_code = r
                .seq_code
                .map(|c| format!(" seq_code={}", c.render(&mut buf)))
                .unwrap_or_default();
            let mut buf2 = [0u8; 10];
            let time_code = r
                .time_code
                .map(|c| format!(" time_code={}", c.render(&mut buf2)))
                .unwrap_or_default();
            self.event(format!(
                "lock: RECEIPT @{n} action={:?} seq={}{seq_code}{time_code}",
                r.action, r.seq
            ));
        }
    }

    /// Relay a receipt from the book (`@R`, 1-based) to the remote validator.
    fn cmd_validate(&mut self, arg: &str) {
        let idx: Option<usize> = arg.strip_prefix('@').unwrap_or(arg).parse().ok();
        let Some(i) = idx else {
            return self.event(format!("validate: bad receipt ref {arg}"));
        };
        let Some(r) = self.receipts.get(i.wrapping_sub(1)).copied() else {
            return self.event(format!("validate: no receipt @{i}"));
        };
        let now_s = self.now as u32;
        let Some(v) = self.validator.as_mut() else {
            return self.event("validate: no validator (scenario has no confirm secret)".into());
        };
        let check = v.verify(&r, now_s);
        self.event(format!("validator: {} (receipt @{i})", describe_check(check)));
    }

    fn cmd_time(&mut self, words: &[&str]) {
        match words {
            ["time", "set", v] => {
                self.now = v.parse().expect("unix time");
                println!("[t={}] clock set", self.now);
            }
            ["time", adv] if adv.starts_with('+') => {
                let a = &adv[1..];
                let (num, mul) = match a.as_bytes().last() {
                    Some(b's') => (&a[..a.len() - 1], 1u64),
                    Some(b'm') => (&a[..a.len() - 1], 60),
                    Some(b'h') => (&a[..a.len() - 1], 3600),
                    _ => (a, 1),
                };
                let d: u64 = num.parse().expect("duration");
                self.now += d * mul;
                self.lock.tick(self.now);
                println!("[t={}] +{}s", self.now, d * mul);
                self.drain_relocks();
                self.drain_receipts();
            }
            _ => println!("? time +<n>[s|m|h] | time set <unix>"),
        }
    }

    /// Drive the virtual GNSS/accel the slot gates read. (The calendar gate is
    /// time-driven from the config — move it with `time set`/`time +`.)
    ///   env fence <idx> <in|out>   move the lock in/out of a geofence
    ///   env still <secs>           set how long it's been motionless
    ///   env show                   print the environment + open calendar windows
    fn cmd_env(&mut self, words: &[&str]) {
        match words {
            ["env", "fence", idx, state] => {
                let b = 1u64 << idx.parse::<u8>().expect("fence idx");
                match *state {
                    "in" => self.env.fences |= b,
                    "out" => self.env.fences &= !b,
                    _ => return println!("? env fence <idx> <in|out>"),
                }
                self.event(format!("env: fence {idx} {state}"));
            }
            ["env", "still", secs] => {
                self.env.still_s = secs.parse().expect("stillness secs");
                self.event(format!("env: still for {}s", self.env.still_s));
            }
            ["env", "show"] => {
                // Calendar windows are time-driven from the config; list which
                // are open right now (use `time set`/`time +` to move windows).
                let mut open = String::new();
                for w in 0..ephemerkey_config::MAX_CALENDARS as u8 {
                    if self.env.calendars.open(w, self.now) {
                        open.push_str(&format!("{w} "));
                    }
                }
                let s = format!(
                    "env: fences=0x{:x} still={}s calendars-open=[{}]",
                    self.env.fences,
                    self.env.still_s,
                    open.trim_end()
                );
                self.event(s);
            }
            _ => println!("? env fence <i> <in|out> | env still <s> | env show (calendars are time-driven)"),
        }
    }

    /// Cascade: dial an unlock code into the generator's ritual engine.
    /// `gen unlock <code|@N>` — `@N` pulls the Nth revealed code from the
    /// notebook, i.e. a code carried over from a predecessor device (A→B).
    /// A `Fired(unlock)` opens the reveal window; `Fired(duress)` opens a
    /// poison-only window, both via the shared `apply_ritual_outcome`.
    fn cmd_unlock(&mut self, code: &str) {
        let code = if let Some(n) = code.strip_prefix('@') {
            let i: usize = n.parse().unwrap_or(0);
            match self.notebook.get(i.wrapping_sub(1)) {
                Some(c) => c.clone(),
                None => return self.event(format!("gen: no notebook entry @{n}")),
            }
        } else {
            code.to_string()
        };
        self.env.now = self.now; // the ritual's own gates read the clock
        let out = self.ritual.enter_code_with(&code, self.now, &self.env);
        ephemerkey_core::reveal::apply_ritual_outcome(&mut self.gen, &out, self.now, self.unlock_window_s);
        let s = describe(out);
        let win = if self.gen.is_unlocked(self.now) {
            format!(" — reveal window open until t={}", self.now + self.unlock_window_s as u64)
        } else {
            String::new()
        };
        self.event(format!("gen unlock: {s}{win}"));
        match out {
            Outcome::Progress(_, h, n) => self.paint(&render(&View::Progress { have: h, need: n })),
            Outcome::Fired(..) => {
                // the ritual completed — the dial is spent
                self.dial.clear();
                self.paint_screen();
            }
            _ => self.paint_screen(),
        }
    }

    /// Simulate the 3-button dial for the display:
    ///   gen dial <digits…>   append digits (e.g. `gen dial 4 7 1`)
    ///   gen dial bs | clr    backspace / clear
    ///   gen dial go          submit the dialed code to the ritual
    fn cmd_dial(&mut self, args: &[&str]) {
        match args {
            ["bs"] => {
                self.dial.pop();
            }
            ["clr"] => self.dial.clear(),
            ["go"] => {
                if self.dial.is_empty() {
                    return self.event("gen: dial empty".into());
                }
                let code: String = self.dial.iter().map(|&d| (b'0' + d) as char).collect();
                self.dial.clear();
                return self.cmd_unlock(&code);
            }
            digits => {
                for tok in digits {
                    for ch in tok.chars() {
                        if let Some(d) = ch.to_digit(10) {
                            self.dial.push(d as u8);
                        }
                    }
                }
            }
        }
        self.paint_screen();
    }

    fn cmd_reveal(&mut self, k: usize) {
        self.seed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
        match self.gen.reveal(k, self.now, self.seed) {
            Ok(r) => {
                let mut buf = [0u8; 10];
                let code = r.code.render(&mut buf).to_string();
                self.notebook.push(code.clone());
                // Ground truth for scripts (introspect build) on a # line —
                // the "display" below is what a shoulder-surfer sees.
                self.event(format!(
                    "gen: reveal key={k} code={code}{}",
                    if r.decoy { " DECOY" } else { "" }
                ));
                match r.mode {
                    // Plain reveal = the OLED face, through the shared render.
                    DisplayMode::Plain => {
                        self.paint(&render(&View::Reveal { code: &code, secs_left: r.reveal_s }))
                    }
                    DisplayMode::Scatter => {
                        let n = r.code.digits as usize;
                        for (frame, &pos) in r.order[..n].iter().enumerate() {
                            let mut mask = vec!['·'; n];
                            mask[pos as usize] = code.as_bytes()[pos as usize] as char;
                            let mask: String = mask.into_iter().collect();
                            println!("  frame {}/{n}: {mask}  ({}ms)", frame + 1, r.dwell_ms);
                        }
                    }
                }
            }
            Err(RevealErr::RefusedUntil(u)) => {
                let in_s = u.saturating_sub(self.now);
                self.event(format!("gen: REFUSED key={k} next in {in_s}s"));
            }
            Err(RevealErr::NoSuchKey) => self.event(format!("gen: no key {k}")),
            Err(RevealErr::ChainRequired) => {
                self.event(format!("gen: CHAINLOCKED key={k} — feed the lock's receipt code first"))
            }
            Err(RevealErr::ChainWait(u)) => {
                let in_s = u.saturating_sub(self.now);
                self.event(format!("gen: CHAINWAIT key={k} — reveals resume in {in_s}s"))
            }
            Err(RevealErr::Locked) => {
                self.event(format!("gen: LOCKED key={k} — ritual not satisfied (blank display)"))
            }
        }
    }

    /// Feed a lock receipt code into the generator's chain: `gen chain <code|@R> [key]`.
    fn cmd_chain(&mut self, arg: &str, k: usize) {
        let code_str = if let Some(n) = arg.strip_prefix('@') {
            let i: usize = n.parse().unwrap_or(0);
            let Some(r) = self.receipts.get(i.wrapping_sub(1)) else {
                return self.event(format!("gen: no receipt @{n}"));
            };
            let Some(c) = r.seq_code.or(r.time_code) else {
                return self.event(format!("gen: receipt @{n} carries no code"));
            };
            let mut buf = [0u8; 10];
            c.render(&mut buf).to_string()
        } else {
            arg.to_string()
        };
        let digits = code_str.len() as u8;
        let Some(code) = Code::parse(&code_str, digits) else {
            return self.event(format!("gen: bad chain code '{code_str}'"));
        };
        match self.gen.feed_chain(k, code, self.now) {
            Ok(at) => {
                let in_s = at.saturating_sub(self.now);
                self.event(format!("gen: CHAIN ACCEPTED key={k} — reveals resume in {in_s}s"))
            }
            Err(ChainErr::NoSuchKey) => self.event(format!("gen: no key {k}")),
            Err(ChainErr::NoChain) => self.event(format!("gen: key {k} has no chain")),
            Err(ChainErr::Rejected(c)) => {
                self.event(format!("gen: CHAIN REJECTED key={k} — {}", describe_check(c)))
            }
        }
    }

    fn cmd_next(&mut self, k: usize) {
        match self.gen.next_real_in(k, self.now) {
            Some(s) => self.event(format!("gen: next real reveal key={k} in {s}s")),
            None => self.event(format!("gen: no key {k}")),
        }
    }

    fn cmd_status(&mut self) {
        let mut lines = Vec::new();
        for i in 0..ephemerkey_core::policy::MAX_SLOTS {
            if let Some(slot) = self.lock.slots[i] {
                let st = self.lock.state[i];
                let progress = if slot.show_progress {
                    format!("{}", st.count)
                } else {
                    "hidden".into()
                };
                lines.push(format!("slot {i}: key={} count={progress}", slot.key));
            }
        }
        let s = format!("lock: status {}", lines.join(" | "));
        self.event(s);
    }
}

fn describe(o: Outcome) -> String {
    match o {
        Outcome::Progress(s, h, n) => format!("PROGRESS slot={s} {h}/{n}"),
        Outcome::Fired(s, a) => format!("FIRED slot={s} action={a:?}"),
        Outcome::Reset(s) => format!("RESET slot={s} (timing violation)"),
        Outcome::Replay(s) => format!("REPLAY slot={s} (ignored)"),
        Outcome::Beat(s) => format!("BEAT slot={s} (sustain refreshed)"),
        Outcome::Negative(s, n) => format!("NEGATIVE slot={s} {n:?} (decoy tripwire)"),
        Outcome::LockedOut(s) => format!("LOCKEDOUT slot={s}"),
        Outcome::Gated(s, b) => format!("GATED slot={s} {b:?} (not burned)"),
        Outcome::Invalid => "INVALID (matched no ritual; reset_on_invalid rituals wiped)".into(),
        Outcome::Armed(s, at) => format!("ARMED slot={s} fires at t={at} unless vetoed"),
        Outcome::Vetoed(s) => format!("VETOED slot={s} (pending action canceled)"),
        Outcome::Exhausted(s) => format!("EXHAUSTED slot={s} (usage budget spent)"),
    }
}

fn describe_check(c: ReceiptCheck) -> String {
    match c {
        ReceiptCheck::Valid { skipped, drift_s } => {
            format!("VALID (skipped={skipped} drift={drift_s}s)")
        }
        ReceiptCheck::BadCode => "BADCODE (forged / wrong key / relabeled)".into(),
        ReceiptCheck::Replay => "REPLAY (seq already consumed)".into(),
        ReceiptCheck::TooFarAhead => "DESYNC (seq beyond look-ahead)".into(),
        ReceiptCheck::OutOfWindow => "OUTOFWINDOW (time drift too large)".into(),
        ReceiptCheck::ModeMismatch => "MODEMISMATCH (wrong proof kind relayed)".into(),
    }
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: ekemu <scenario.json>  (commands on stdin)");
        eprintln!("       ekemu serial <state.json> [listen_addr]");
        std::process::exit(2);
    });
    if arg == "serial" {
        let state = std::env::args().nth(2).unwrap_or_else(|| {
            eprintln!("usage: ekemu serial <state.json> [listen_addr]");
            std::process::exit(2);
        });
        let listen = std::env::args().nth(3).unwrap_or_else(|| "127.0.0.1:8422".into());
        serial::run(&state, &listen);
        return;
    }
    let scn: Scenario =
        serde_json::from_str(&std::fs::read_to_string(&arg).expect("read scenario"))
            .expect("parse scenario");
    let (gen, lock, ritual, validator, calendars, unlock_window_s) = build(&scn);
    let mut emu = Emu {
        now: scn.start_time,
        seed: scn.seed,
        gen,
        lock,
        ritual,
        unlock_window_s,
        env: Env { calendars, ..Default::default() },
        validator,
        last_event: String::new(),
        failures: 0,
        dial: Vec::new(),
        sel: 0,
        notebook: Vec::new(),
        receipts: Vec::new(),
    };
    println!("ekemu: scenario {arg}, t={}", emu.now);

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line.expect("stdin");
        if !emu.cmd(&line) {
            break;
        }
    }
    if emu.failures > 0 {
        eprintln!("ekemu: {} expectation(s) FAILED", emu.failures);
        std::process::exit(1);
    }
}
