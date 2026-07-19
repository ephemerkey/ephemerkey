//! Provisioning: sealed config envelopes delivered over USB or WiFi.
//!
//! The protocol/crypto engine lives in the shared `ephemerkey-provision`
//! crate (framed link per ephemerkey-control docs/serial-protocol.md;
//! `COSE_Encrypt0(COSE_Sign1(config, owner), device_kx)` envelopes; owner
//! TOFU via the inner Sign1 kid; signed acks/events; wifi handoff). The
//! emulator's `ekemu serial` runs the same engine semantics and is the
//! behavioral reference; this module owns only the platform pieces:
//!
//!   - **Transport**: USB FS CDC on PA11/PA12, button-gated ("provisioning
//!     mode" — never silently writable). The USB task pumps raw bytes into
//!     `Provisioner::feed` and writes the response frames back. The WiFi
//!     path (ESP32-C3 over LPUART1) pumps the exact same engine.
//!   - **Identity**: device_id + Ed25519/X25519 secrets, minted from the
//!     TRNG on first boot and persisted; never leave the device.
//!   - **[`FlashStore`]**: owner binding, config seq, and the verified
//!     config payload in the internal-flash journal (DESIGN.md §Storage).
//!
//! Status: engine wired, transports and flash journal still TODO — `feed`
//! is reachable from tests/emulator today, from hardware once the USB task
//! lands.

#![allow(dead_code)]

use ephemerkey_provision::{Identity, Provisioner, Store, CONFIG_MAX};

/// Flash-journal-backed store. TODO: back with the config journal pages
/// (append-counter region + wear budget per DESIGN.md); this in-RAM version
/// gives the engine correct semantics until then — but loses state on
/// power-down, so hardware provisioning stays gated off until the journal
/// lands (a lost owner binding would re-open TOFU).
pub struct FlashStore {
    owner: Option<[u8; 32]>,
    seq: u64,
    config: [u8; CONFIG_MAX],
    config_len: usize,
    wifi_ssid: heapless::String<32>,
    wifi_psk: heapless::String<64>,
}

impl FlashStore {
    pub const fn new() -> Self {
        FlashStore {
            owner: None,
            seq: 0,
            config: [0; CONFIG_MAX],
            config_len: 0,
            wifi_ssid: heapless::String::new(),
            wifi_psk: heapless::String::new(),
        }
    }
}

impl Store for FlashStore {
    fn owner_pub(&self) -> Option<[u8; 32]> {
        self.owner
    }
    fn seq(&self) -> u64 {
        self.seq
    }
    fn commit(&mut self, owner_pub: &[u8; 32], seq: u64, config: &[u8]) -> Result<(), ()> {
        if config.len() > CONFIG_MAX {
            return Err(());
        }
        // TODO: journal write (owner+seq page, then config pages, then the
        // commit marker) so a torn write can't lose the owner binding.
        self.owner = Some(*owner_pub);
        self.seq = seq;
        self.config[..config.len()].copy_from_slice(config);
        self.config_len = config.len();
        Ok(())
    }
    fn wifi_set(&mut self, ssid: &str, psk: &str) -> Result<(), ()> {
        self.wifi_ssid = heapless::String::try_from(ssid).map_err(|_| ())?;
        self.wifi_psk = heapless::String::try_from(psk).map_err(|_| ())?;
        Ok(())
    }
    fn wifi_clear(&mut self) -> Result<(), ()> {
        self.wifi_ssid.clear();
        self.wifi_psk.clear();
        Ok(())
    }
    fn wifi_ssid(&self) -> Option<&str> {
        if self.wifi_ssid.is_empty() {
            None
        } else {
            Some(self.wifi_ssid.as_str())
        }
    }
    fn now(&self) -> u64 {
        // TODO: RTC (GNSS-disciplined) once the clock task exposes it.
        0
    }
}

pub type DeviceProvisioner = Provisioner<FlashStore>;

/// Build the engine. TODO: load (or mint via TRNG + persist) the real
/// device identity; `[0; 32]` secrets here are compile-time placeholders and
/// the USB task must not enumerate until real keys exist.
pub fn provisioner() -> DeviceProvisioner {
    let identity = Identity {
        device_id: [0; 12],
        sign: ephemerkey_envelope::SigningKey::from_bytes(&[0; 32]),
        kx_priv: [0; 32],
        fw: concat!("ephemerkey-rs-", env!("CARGO_PKG_VERSION")),
    };
    Provisioner::new(identity, FlashStore::new())
}
