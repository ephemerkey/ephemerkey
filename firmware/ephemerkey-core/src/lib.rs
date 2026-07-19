//! ephemerkey shared core — used by the STM32 firmware (no_std) and the
//! host emulator/tests (std). Design docs: DESIGN-policies.md.
//!
//! Modules:
//! - [`totp`]   — RFC 4226/6238 HOTP/TOTP, 4-10 digit codes
//! - [`policy`] — code-slot model + per-slot sequence state machines
//! - [`engine`] — lock-side validation: delay-window counter search,
//!                replay burn, decoy (negative) matching, slot dispatch
//! - [`reveal`] — generator-side display scheduler: scatter reveal,
//!                show-once refusal windows, decoy (poison) minting
//! - [`receipt`] — confirm-TOTP: the lock's own HOTP/TOTP event receipts and
//!                the remote validator that verifies them

#![cfg_attr(not(test), no_std)]

pub mod engine;
pub mod policy;
pub mod receipt;
pub mod reveal;
pub mod totp;

/// Feature tags this engine build implements. A config's `crit` list names
/// the features it depends on for its security promises; a consumer MUST
/// reject a config whose crit entry it doesn't recognize (silent
/// non-enforcement of a protection is the failure mode this prevents —
/// same idea as X.509 critical extensions / COSE `crit`).
pub const SUPPORTED_POLICY_FEATURES: &[&str] = &["seq-jitter", "quorum-pace", "chain", "veto", "budget"];
