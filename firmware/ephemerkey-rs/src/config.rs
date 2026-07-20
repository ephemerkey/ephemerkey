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
//! The sealed config is an integer-keyed CBOR map, decoded by the shared
//! [`ephemerkey_envelope::config`] parser (host-tested there) into a fixed-size
//! [`DeviceConfig`]: role, the emission-freshness window, and the geofence zone
//! table. We re-export those types so the rest of the firmware speaks one
//! vocabulary; [`load`] reads the newest committed record out of the flash
//! journal and parses it, falling back to a **shut** default (Generator, no
//! fences ⇒ never emits) on a factory-fresh or unparseable config.

// `Role` is only named by the role-dispatch match, which is entirely
// feature-gated (gnss / lock) — unused on a bare Nucleo build.
#[allow(unused_imports)]
pub use ephemerkey_envelope::config::{DeviceConfig as Config, Role, DEFAULT_STALENESS_S};

/// Parse the committed config out of the flash journal. A factory-fresh device
/// (empty record) or a config that fails to parse yields the shut default —
/// the device links and runs, but a Generator emits nothing until a valid,
/// fenced config is provisioned.
pub fn load(journal: &crate::provision::DeviceJournal) -> Config {
    let bytes = journal.config();
    if bytes.is_empty() {
        return Config::shut_default();
    }
    match ephemerkey_envelope::config::parse(bytes) {
        Ok(cfg) => cfg,
        Err(_) => {
            defmt::warn!("config: stored blob failed to parse; running shut default");
            Config::shut_default()
        }
    }
}
