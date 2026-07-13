//! Provisioning: config/secret FILES delivered over USB or WiFi.
//!
//! Both personalities are programmed the same way — a signed config file
//! (role, geofence table, TOTP secrets, lock pairing secret, policies) is
//! pushed to the device and committed to the internal-flash config journal:
//!
//!   - **USB** (PA11/PA12, FS device): provisioning mode is explicit —
//!     button + USB enumeration (DESIGN.md "Security Considerations"), never
//!     silently writable. Planned as a vendor-class/console `embassy-usb`
//!     interface carrying the file.
//!   - **WiFi**: the ESP32-C3 (LPUART1 link, powered only on demand) fetches
//!     the file and streams it across; same verification path as USB.
//!
//! File format sketch: TLV body ‖ HMAC-SHA1 tag under a provisioning secret
//! (mirrors the lock board's authenticated-config scheme, which is proven).
//! Verification is constant-time; a bad tag leaves the journal untouched.
//!
//! Scaffold: interface sketch only — no transport is wired up yet.

#![allow(dead_code)]

use crate::config::Config;

pub enum ProvisionError {
    BadTag,
    BadFormat,
    FlashWrite,
}

/// Verify a received provisioning file and commit it to the config journal.
/// Called by whichever transport (USB / WiFi task) received the file.
pub fn apply(_file: &[u8]) -> Result<Config, ProvisionError> {
    // TODO: HMAC verify -> TLV parse -> journal commit -> return new Config.
    Err(ProvisionError::BadFormat)
}
