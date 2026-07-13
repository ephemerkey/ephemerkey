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
//!   expect <substring>      assert the last event line contains this
//!   echo <text> | # ...     narration / comments
//!   quit

use ephemerkey_core::engine::{KeyDef, LockEngine, Outcome};
use ephemerkey_core::policy::{
    Action, Gates, NegativeAction, Policy, Slot, MAX_PATH_LEGS, MAX_QUORUM,
};
use ephemerkey_core::reveal::{DisplayMode, DisplaySpec, GenKey, Generator, OnceMode, RevealErr};
use serde::Deserialize;
use std::io::BufRead;

// ---------------------------------------------------------------- scenario

#[derive(Deserialize)]
struct Scenario {
    #[serde(default = "default_start")]
    start_time: u64,
    #[serde(default = "default_seed")]
    seed: u32,
    keys: Vec<KeyCfg>,
    slots: Vec<SlotCfg>,
}
fn default_start() -> u64 {
    1_750_000_000
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
    },
}
fn default_delay_max() -> u16 {
    60
}

fn build(scn: &Scenario) -> (Generator, LockEngine) {
    let mut lock = LockEngine::new();
    let mut gen = Generator::new();

    for (i, k) in scn.keys.iter().enumerate() {
        let mut kd = KeyDef::new(k.secret.as_bytes(), k.digits);
        kd.decoy = k.decoy;
        lock.keys[i] = Some(kd);
        if let Some(d) = &k.display {
            let decoy_kd = k.decoy.map(|di| {
                let dk = &scn.keys[di as usize];
                KeyDef::new(dk.secret.as_bytes(), dk.digits)
            });
            gen.keys[i] = Some(GenKey {
                key: kd,
                decoy: decoy_kd,
                display: DisplaySpec {
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
                } => Policy::Sequence {
                    n,
                    window_s,
                    gap_min_s,
                    gap_max_s,
                    delay_min_s,
                    delay_max_s,
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
                    }
                }
            },
            gates: Gates::default(),
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
    (gen, lock)
}

// ------------------------------------------------------------------- emu

struct Emu {
    now: u64,
    seed: u32,
    gen: Generator,
    lock: LockEngine,
    last_event: String,
    failures: u32,
    /// Every code the generator has shown, in order — the user's notebook.
    notebook: Vec<String>,
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
                let out = self.lock.enter_code(&code, self.now);
                let s = describe(out);
                self.event(format!("lock: {s}"));
                // Relock consequences print (and become expectable) after
                // the outcome that caused them.
                self.drain_relocks();
            }
            ["lock", "status"] => self.cmd_status(),

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
            }
            _ => println!("? time +<n>[s|m|h] | time set <unix>"),
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
        Outcome::Invalid => "INVALID (armed slots reset)".into(),
    }
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: ekemu <scenario.json>  (commands on stdin)");
        std::process::exit(2);
    });
    let scn: Scenario =
        serde_json::from_str(&std::fs::read_to_string(&arg).expect("read scenario"))
            .expect("parse scenario");
    let (gen, lock) = build(&scn);
    let mut emu = Emu {
        now: scn.start_time,
        seed: scn.seed,
        gen,
        lock,
        last_event: String::new(),
        failures: 0,
        notebook: Vec::new(),
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
