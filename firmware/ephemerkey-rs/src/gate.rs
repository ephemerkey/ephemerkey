//! Code-emission readiness gate.
//!
//! A generator may emit a TOTP only when the **last GNSS fix was valid, inside
//! an authorized geofence, and recent enough** — DESIGN's "valid fix ∧ in-fence
//! ∧ fresh clock" guard. On this device the GNSS is powered on-demand (the user
//! presses a button, we acquire one fix, then the receiver powers back down),
//! so there is no continuous PPS to lean on: freshness is measured from that
//! last acquired fix via the RTC's last-discipline time ([`clock::is_fresh`]).
//! When the fix ages past the staleness window, the gate closes on its own — a
//! frozen or rolled-back clock, or a device carried out of the fence and left
//! sitting, must never keep yielding codes.
//!
//! Both inputs are latched here by the GNSS task ([`crate::gnss`]): the RTC is
//! disciplined and [`set_in_fence`] is called together on each fix. The gate
//! itself just reads the latch and asks the clock whether it is still fresh.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use embassy_time::Duration;

use crate::clock;
use crate::config;

/// Whether the *last* GNSS fix was both valid and inside an authorized fence.
/// A void fix, or a valid fix outside every fence, clears this immediately.
static IN_FENCE: AtomicBool = AtomicBool::new(false);

/// Emission-freshness window (seconds): the maximum age of the last fix before
/// codes are refused. Loaded from the sealed config at boot ([`configure`]);
/// defaults to the config crate's default until then.
static STALENESS_S: AtomicU32 = AtomicU32::new(config::DEFAULT_STALENESS_S);

/// Apply the sealed config to the gate — currently just the staleness window.
pub fn configure(cfg: &config::Config) {
    STALENESS_S.store(cfg.staleness_s, Ordering::Relaxed);
}

/// Record whether the latest fix places the device inside a fence. Called by
/// the GNSS task with `valid_fix && config.in_any_fence(lat, lon)`.
pub fn set_in_fence(in_fence: bool) {
    IN_FENCE.store(in_fence, Ordering::Relaxed);
}

/// Whether the device may currently emit a TOTP: the last fix was in-fence and
/// the clock (disciplined at that fix) is still fresh.
pub fn may_emit() -> bool {
    IN_FENCE.load(Ordering::Relaxed)
        && clock::is_fresh(Duration::from_secs(STALENESS_S.load(Ordering::Relaxed) as u64))
}
