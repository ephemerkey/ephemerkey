//! Stillness latch.
//!
//! The [`sensors`](crate::sensors) task samples the LIS3DH and publishes how
//! long the device has been continuously still here; the policy engines read it
//! for their stillness gate ([`Sensors::still_for_s`]). Decoupled through an
//! atomic — like [`gate`](crate::gate)'s in-fence latch — so the accel driver
//! stays a dumb source and every reader (generator ritual, lock) sees one
//! number.
//!
//! [`Sensors::still_for_s`]: ephemerkey_core::policy::Sensors::still_for_s

use core::sync::atomic::{AtomicU32, Ordering};

/// Seconds the device has been continuously still (0 = moving / unknown).
static STILL_S: AtomicU32 = AtomicU32::new(0);

/// Publish the current stillness duration (called by the accel sampler).
pub fn set_still_s(secs: u32) {
    STILL_S.store(secs, Ordering::Relaxed);
}

/// Seconds of continuous stillness — the engine's `still_for_s` gate input.
pub fn still_for_s() -> u32 {
    STILL_S.load(Ordering::Relaxed)
}
