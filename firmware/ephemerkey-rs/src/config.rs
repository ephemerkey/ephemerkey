//! Persistent device configuration — including the device PERSONALITY.
//!
//! One firmware image serves two device roles, selected by config:
//!   - **Generator**: GNSS-geofenced TOTP code generator (the classic
//!     ephemerkey). Needs fix + fence + fresh RTC to emit codes.
//!   - **LockController**: receives/validates TOTP codes and drives the
//!     companion lock board (ATtiny1616) over the LOCK I2C bus (J2).
//!
//! Storage plan (DESIGN.md "Storage, Logging & OTA"): config + secrets live in
//! the last 2x2KB internal-flash pages as a ping-pong journal with CRC,
//! hidden behind RDP/HDP. Provisioning writes arrive as FILES over USB or the
//! WiFi (ESP32-C3) link — see `provision.rs`.
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
