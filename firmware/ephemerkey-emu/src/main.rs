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

use ephemerkey_core::engine::{KeyDef, LockEngine, Outcome};
use ephemerkey_core::policy::{
    Action, Gates, NegativeAction, Policy, Sensors, Slot, MAX_PATH_LEGS, MAX_QUORUM,
};
use ephemerkey_core::receipt::{Receipt, ReceiptCheck, ReceiptMode, Receipts, Validator};
use ephemerkey_core::totp::Code;
use ephemerkey_core::reveal::{ChainErr, ChainSpec, DisplayMode, DisplaySpec, GenKey, Generator, OnceMode, RevealErr};
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
    /// Critical features this config depends on: every entry must be in
    /// KNOWN_FEATURES or the scenario/config is refused outright.
    #[serde(default)]
    crit: Vec<String>,
    /// The lock's own confirm-TOTP identity (receipts it mints on every
    /// fire/relock). Absent = the lock stays silent.
    confirm: Option<ConfirmCfg>,
}
fn default_start() -> u64 {
    1_750_000_000
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
    f.extend(["zones", "calendars"]);
    f
}

fn build(scn: &Scenario) -> (Generator, LockEngine, Option<Validator>) {
    for c in &scn.crit {
        assert!(
            known_features().contains(&c.as_str()),
            "config requires unsupported critical feature '{c}' — refusing"
        );
    }
    let mut lock = LockEngine::new();
    let mut gen = Generator::new();

    for (i, k) in scn.keys.iter().enumerate() {
        let mut kd = KeyDef::new(k.secret.as_bytes(), k.digits);
        kd.decoy = k.decoy;
        lock.keys[i] = Some(kd);
        if k.display.is_some() || k.chain.is_some() {
            let decoy_kd = k.decoy.map(|di| {
                let dk = &scn.keys[di as usize];
                KeyDef::new(dk.secret.as_bytes(), dk.digits)
            });
            let display = match &k.display {
                Some(d) => DisplaySpec {
                    mode: match d.mode {
                        ModeCfg::Plain => DisplayMode::Plain,
                        ModeCfg::Scatter => DisplayMode::Scatter,
                    },
                    dwell_ms: d.dwell_ms,
                    reveal_s: d.reveal_s,
                    once: match d.once {
                        OnceCfg::Unlimited => OnceMode::Unlimited,
                        OnceCfg::Refuse => OnceMode::Refuse,
                        OnceCfg::Decoy => OnceMode::Decoy,
                    },
                    gap_min_s: d.gap_min_s,
                },
                None => DisplaySpec {
                    mode: DisplayMode::Plain,
                    dwell_ms: 800,
                    reveal_s: 5,
                    once: OnceMode::Unlimited,
                    gap_min_s: 0,
                },
            };
            let chain = k.chain.as_ref().map(|c| {
                let mut validator =
                    Validator::new(c.secret.as_bytes(), c.digits, ReceiptMode::from(c.mode));
                validator.time_window_s = c.max_age_s;
                ChainSpec {
                    validator,
                    action: match c.action.as_str() {
                        "unlock" => Action::Unlock,
                        "duress" => Action::DuressUnlock,
                        _ => Action::Lock,
                    },
                    min_elapsed_s: c.min_elapsed_s,
                }
            });
            gen.keys[i] = Some(GenKey {
                key: kd,
                decoy: decoy_kd,
                display,
                chain,
            });
        }
    }

    for (i, s) in scn.slots.iter().enumerate() {
        let negative = if s.negative == "silent" {
            NegativeAction::Silent
        } else if let Some(secs) = s.negative.strip_prefix("lockout:") {
            NegativeAction::Lockout(secs.parse().expect("lockout secs"))
        } else {
            NegativeAction::Reset
        };
        lock.slots[i] = Some(Slot {
            key: s.key,
            policy: match &s.policy {
                PolicyCfg::Always => Policy::AlwaysValid,
                &PolicyCfg::Sequence {
                    n,
                    window_s,
                    gap_min_s,
                    gap_max_s,
                    delay_min_s,
                    delay_max_s,
                    jitter_s,
                } => Policy::Sequence {
                    n,
                    window_s,
                    gap_min_s,
                    gap_max_s,
                    delay_min_s,
                    delay_max_s,
                    pace_jitter_s: jitter_s,
                },
                PolicyCfg::Path {
                    leg_keys,
                    leg_deadline_s,
                    delay_max_s,
                } => {
                    assert!(leg_keys.len() >= 2 && leg_keys.len() <= MAX_PATH_LEGS);
                    let mut lk = [0u8; MAX_PATH_LEGS];
                    lk[..leg_keys.len()].copy_from_slice(leg_keys);
                    Policy::Path {
                        legs: leg_keys.len() as u8,
                        leg_keys: lk,
                        leg_deadline_s: *leg_deadline_s,
                        delay_max_s: *delay_max_s,
                    }
                }
                &PolicyCfg::DeadMan { beat_s } => Policy::DeadMan { beat_s },
                PolicyCfg::Quorum {
                    m,
                    keys,
                    window_s,
                    alternating,
                    gap_min_s,
                    gap_max_s,
                } => {
                    assert!(keys.len() >= *m as usize && keys.len() <= MAX_QUORUM);
                    let mut ks = [0u8; MAX_QUORUM];
                    ks[..keys.len()].copy_from_slice(keys);
                    Policy::Quorum {
                        m: *m,
                        n_keys: keys.len() as u8,
                        keys: ks,
                        window_s: *window_s,
                        alternating: *alternating,
                        gap_min_s: *gap_min_s,
                        gap_max_s: *gap_max_s,
                    }
                }
            },
            veto_delay_s: s.veto_delay_s,
            veto_key: s.veto_key,
            budget: s.budget,
            gates: Gates {
                own_fence: s.gates.fence,
                stillness_s: s.gates.stillness_s,
                calendar: s.gates.calendar,
            },
            action: match s.action.as_str() {
                "lock" => Action::Lock,
                "duress" => Action::DuressUnlock,
                _ => Action::Unlock,
            },
            show_progress: s.progress,
            reset_on_invalid: s.reset_on_invalid,
            negative,
        });
    }

    // The lock's confirm identity: the engine mints, a matching validator
    // (a remote party holding the same secret) verifies what gets relayed.
    let validator = scn.confirm.as_ref().map(|c| {
        let mode: ReceiptMode = c.mode.into();
        lock.receipts = Some(Receipts::new(c.secret.as_bytes(), c.digits, mode));
        Validator::new(c.secret.as_bytes(), c.digits, mode)
    });

    (gen, lock, validator)
}

// ------------------------------------------------------------------- env

/// Virtual GNSS / accelerometer / RTC the gates evaluate against. Firmware
/// implements `Sensors` over real hardware; here the `env` commands drive it.
/// Defaults are deliberately "shut": out of every fence, moving, every
/// calendar window closed — so a gated slot genuinely gates until the script
/// puts the lock where it needs to be.
#[derive(Default)]
struct Env {
    /// bit f set => lock is inside fence f
    fences: u64,
    /// seconds the lock has been continuously still
    still_s: u32,
    /// bit w set => calendar window w is open
    calendars: u64,
}
impl Sensors for Env {
    fn inside_fence(&self, fence: u8) -> bool {
        self.fences & (1 << fence) != 0
    }
    fn still_for_s(&self) -> u32 {
        self.still_s
    }
    fn calendar_open(&self, window: u8) -> bool {
        self.calendars & (1 << window) != 0
    }
}

// ------------------------------------------------------------------- emu

struct Emu {
    now: u64,
    seed: u32,
    gen: Generator,
    lock: LockEngine,
    env: Env,
    /// A remote party holding the lock's confirm secret (None = silent lock).
    validator: Option<Validator>,
    last_event: String,
    failures: u32,
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

    /// Drive the virtual GNSS/accel/RTC the slot gates read.
    ///   env fence <idx> <in|out>   move the lock in/out of a geofence
    ///   env still <secs>           set how long it's been motionless
    ///   env cal <idx> <open|shut>  open/close a calendar window
    ///   env show                   print the current environment
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
            ["env", "cal", idx, state] => {
                let b = 1u64 << idx.parse::<u8>().expect("calendar idx");
                match *state {
                    "open" => self.env.calendars |= b,
                    "shut" | "closed" => self.env.calendars &= !b,
                    _ => return println!("? env cal <idx> <open|shut>"),
                }
                self.event(format!("env: calendar {idx} {state}"));
            }
            ["env", "show"] => {
                let s = format!(
                    "env: fences=0x{:x} still={}s calendars=0x{:x}",
                    self.env.fences, self.env.still_s, self.env.calendars
                );
                self.event(s);
            }
            _ => println!("? env fence <i> <in|out> | env still <s> | env cal <i> <open|shut> | env show"),
        }
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
                    DisplayMode::Plain => {
                        println!("  ┌{}┐", "─".repeat(code.len() + 2));
                        println!("  │ {code} │  ({}s)", r.reveal_s);
                        println!("  └{}┘", "─".repeat(code.len() + 2));
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
        Outcome::Invalid => "INVALID (armed slots reset)".into(),
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
    let (gen, lock, validator) = build(&scn);
    let mut emu = Emu {
        now: scn.start_time,
        seed: scn.seed,
        gen,
        lock,
        env: Env::default(),
        validator,
        last_event: String::new(),
        failures: 0,
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
