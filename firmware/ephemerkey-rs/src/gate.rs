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

use core::cell::RefCell;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex};
use embassy_time::Duration;

use crate::clock;
use crate::config::{self, Zone, MAX_ZONES};

/// Whether the *last* GNSS fix was both valid and inside an authorized fence.
/// A void fix, or a valid fix outside every fence, clears this immediately.
static IN_FENCE: AtomicBool = AtomicBool::new(false);

/// Emission-freshness window (seconds): the maximum age of the last fix before
/// codes are refused. Loaded from the sealed config at boot ([`configure`]);
/// defaults to the config crate's default until then.
static STALENESS_S: AtomicU32 = AtomicU32::new(config::DEFAULT_STALENESS_S);

/// The authorized geofence zones (copied from the sealed config at boot). The
/// GNSS task tests each fix against these; the gate owns them so the task
/// stays a dumb position source.
static ZONES: Mutex<CriticalSectionRawMutex, RefCell<([Zone; MAX_ZONES], usize)>> = Mutex::new(
    RefCell::new(([Zone { lat_e7: 0, lon_e7: 0, radius_m: 0 }; MAX_ZONES], 0)),
);

/// Apply the sealed config to the gate: the freshness window and the geofence
/// zone table.
pub fn configure(cfg: &config::Config) {
    STALENESS_S.store(cfg.staleness_s, Ordering::Relaxed);
    ZONES.lock(|z| {
        let mut z = z.borrow_mut();
        let zones = cfg.zones();
        let count = zones.len().min(MAX_ZONES);
        z.0[..count].copy_from_slice(&zones[..count]);
        z.1 = count;
    });
}

/// Update the fence latch from a fix. `valid` = the receiver reported a fix;
/// only then is the position tested against the configured zones. An invalid
/// (void) fix, or a valid fix outside every fence, closes the gate at once.
pub fn update_from_fix(valid: bool, lat_e7: i32, lon_e7: i32) {
    let in_fence = valid
        && ZONES.lock(|z| {
            let z = z.borrow();
            z.0[..z.1].iter().any(|zone| zone.contains(lat_e7, lon_e7))
        });
    IN_FENCE.store(in_fence, Ordering::Relaxed);
}

/// Whether the last GNSS fix was inside an authorized fence (the raw latch,
/// without the freshness check). Feeds the generator ritual's `own_fence`
/// gate — the generator's own position gate reuses the emission fence.
pub fn in_fence() -> bool {
    IN_FENCE.load(Ordering::Relaxed)
}

/// Whether the device may currently emit a TOTP: the last fix was in-fence and
/// the clock (disciplined at that fix) is still fresh.
pub fn may_emit() -> bool {
    IN_FENCE.load(Ordering::Relaxed)
        && clock::is_fresh(Duration::from_secs(STALENESS_S.load(Ordering::Relaxed) as u64))
}
