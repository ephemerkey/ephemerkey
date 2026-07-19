//! Persistent device configuration — including the device PERSONALITY.
//!
//! One firmware image serves two device roles, selected by config:
//!   - **Generator**: GNSS-geofenced TOTP code generator (the classic
//!     ephemerkey). Needs fix + fence + fresh RTC to emit codes.
//!   - **LockController**: receives/validates TOTP codes and drives the
//!     companion lock board (ATtiny1616) over the LOCK I2C bus (J2).
//!
//! Storage plan (DESIGN.md "Storage, Logging & OTA"). Two append regions at
//! the top of internal flash, both exploiting NOR's program-`1->0` /
//! bulk-erase asymmetry so a routine update costs one double-word write, not a
//! page erase:
//!
//!   - **Config/secret map** (2x2KB pages, ping-pong + CRC): role, secrets
//!     (TOTP, lock-pairing, device Ed25519/X25519, log key, `K_confirm`), zone
//!     table, slots, policy blobs, device-opts, and the config `seq`
//!     (anti-rollback). Rewritten only on provisioning — rare.
//!   - **Event-counter queue** (2x2KB pages, append + erase-when-full): the
//!     confirm-TOTP monotonic counter, bumped once per fire/relock.
//!
//! Why the counter gets its own region: STM32U0 flash programs in 64-bit
//! double-words with ECC that forbids re-writing a unit before erase, so each
//! bump appends a *fresh* double-word (never re-flips bits in place). 2KB/8B =
//! 256 bumps per page; across the two-page queue that's ~5M bumps before the
//! erase-cycle limit (verify the U0 spec) — >100 years even at a heavy 100
//! unlock/lock events/day, so no reserve-ahead is needed for wear. A reboot
//! resumes from the last persisted counter; the small skip is absorbed by the
//! receipt `Validator`'s RFC-4226 look-ahead, so no counter is ever reused.
//!
//! Implementation: the `sequential-storage` crate over `embedded-storage`
//! gives the ECC-safe single-write-per-word log (map for config, queue for the
//! counter) — don't hand-roll it. Hidden behind RDP/HDP (verify RM0503).
//! Provisioning writes arrive as FILES over USB or the WiFi (ESP32-C3) link —
//! see `provision.rs`.
//!
//! Scaffold: compile-time defaults only; the flash journal is the next step.

use defmt::Format;

#[derive(Copy, Clone, PartialEq, Eq, Format)]
#[allow(dead_code)] // LockController is constructed by provisioning, not defaults
pub enum Role {
    /// GNSS-geofenced TOTP code generator.
    Generator,
    /// TOTP receiver driving the lock board over the LOCK I2C bus.
    LockController,
}

#[derive(Copy, Clone, Format)]
pub struct Config {
    pub role: Role,
    // TODO: geofence table (Generator), TOTP secret slot refs, lock pairing
    // secret slot ref (LockController), RTC staleness window, log key.
}

impl Config {
    const fn default() -> Self {
        Self {
            role: Role::Generator,
        }
    }
}

/// Load config. Scaffold: defaults. Real version reads the newest valid
/// journal entry from the last 2x2KB flash pages (CRC + sequence).
pub fn load() -> Config {
    Config::default()
}
