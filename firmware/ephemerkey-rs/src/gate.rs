//! Code-emission readiness gate.
//!
//! A generator may emit a TOTP only when **the clock is fresh** (disciplined by
//! GNSS within the staleness window — a frozen or rolled-back clock must never
//! yield valid codes, DESIGN §Security) **and the GNSS reports a valid fix**.
//! This is the software half of DESIGN's "valid fix ∧ in-fence ∧ fresh clock"
//! guard; the in-fence (geofence) test joins [`may_emit`] once the zone table
//! is parsed from the sealed config.

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_time::Duration;

use crate::clock;

/// Maximum age of the last GNSS clock discipline before codes are refused —
/// 3× the 30 s TOTP period. (DESIGN calls this a configurable window; a config
/// field can override it once integer-keyed configs land.)
pub const STALENESS: Duration = Duration::from_secs(90);

/// Last GNSS fix validity (RMC status). Updated by the GNSS task.
static FIX_VALID: AtomicBool = AtomicBool::new(false);

/// Record the receiver's fix validity (RMC 'A' vs 'V').
pub fn set_fix_valid(valid: bool) {
    FIX_VALID.store(valid, Ordering::Relaxed);
}

/// Whether the device may currently emit a TOTP: a valid fix and a
/// recently-disciplined clock. Geofence membership is still TODO.
pub fn may_emit() -> bool {
    FIX_VALID.load(Ordering::Relaxed) && clock::is_fresh(STALENESS)
}
