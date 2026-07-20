//! Device configuration: the sealed CBOR config, decoded by the shared
//! [`ephemerkey_config`] crate (host-tested; the emulator runs the identical
//! decoder) into a [`Config`] — role, geofence zones, freshness window, and the
//! full policy tables (keys/slots/confirm). [`load`] reads the newest committed
//! journal record and parses it; a factory-fresh or unparseable config falls
//! back to a **shut** default (Generator, no fences/keys/slots ⇒ emits nothing).
//!
//! The engine itself is built from a [`Config`] with
//! [`ephemerkey_config::build_generator`] / [`build_lock`](ephemerkey_config::build_lock)
//! / [`build_validator`](ephemerkey_config::build_validator) in `main`.

// `Role` is only named by the role-dispatch match, which is feature-gated
// (gnss / lock) — unused on a bare Nucleo build.
#[allow(unused_imports)]
pub use ephemerkey_config::{DeviceModel as Config, Role};
pub use ephemerkey_envelope::config::{Zone, DEFAULT_STALENESS_S, MAX_ZONES};

/// Policy features this firmware actually honors. A config whose `crit` list
/// names anything else is refused at parse (never silently drop a protection).
/// `zones` = the geofence gate is implemented; `calendars` is not yet, so it is
/// deliberately absent.
pub const FIRMWARE_FEATURES: &[&str] =
    &["seq-jitter", "quorum-pace", "chain", "veto", "budget", "zones"];

/// Parse the committed config out of the flash journal. Empty (factory-fresh)
/// or unparseable ⇒ the shut default: the device links and runs, but emits
/// nothing until a valid config is provisioned.
pub fn load(journal: &crate::provision::DeviceJournal) -> Config {
    let bytes = journal.config();
    if bytes.is_empty() {
        return Config::shut_default();
    }
    match ephemerkey_config::parse(bytes, FIRMWARE_FEATURES) {
        Ok(cfg) => cfg,
        Err(_) => {
            defmt::warn!("config: stored blob rejected; running shut default");
            Config::shut_default()
        }
    }
}
