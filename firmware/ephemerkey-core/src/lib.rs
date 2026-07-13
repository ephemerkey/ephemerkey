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

#![cfg_attr(not(test), no_std)]

pub mod engine;
pub mod policy;
pub mod reveal;
pub mod totp;
