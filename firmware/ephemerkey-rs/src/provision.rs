//! Provisioning platform glue: the flash-journal-backed [`Store`], the
//! persisted device identity, and the engine constructor. The protocol/crypto
//! itself lives in the shared `ephemerkey-provision` crate (the emulator's
//! `ekemu serial` is its behavioral twin); this module owns only the pieces
//! that touch real silicon:
//!
//!   - **[`FlashStore`]** — owner binding, config `seq`, verified config, and
//!     WiFi creds persisted in the internal-flash journal via `ephemerkey-store`
//!     (identity page + 2-slot ping-pong record; a torn write can lose the new
//!     config but never the owner binding).
//!   - **Identity** — device_id + Ed25519/X25519 secrets, minted from the TRNG
//!     on first boot and persisted; loaded verbatim thereafter.
//!
//! The transport (USB FS CDC, button-gated) is `crate::usbprov`.

use embassy_stm32::flash::{Blocking, Flash};
use ephemerkey_envelope::SigningKey;
use ephemerkey_provision::{Identity, Provisioner, Store};
use ephemerkey_store::{
    Error as StoreError, Flash as StoreFlash, Journal, Layout, StoredIdentity, PAGE,
};

/// Adapts embassy's blocking `Flash` to the store's [`StoreFlash`] trait.
/// Offsets are bank-relative, exactly what `blocking_*` expect.
pub struct EmbassyFlash(pub Flash<'static, Blocking>);

impl StoreFlash for EmbassyFlash {
    fn read(&mut self, off: u32, buf: &mut [u8]) -> Result<(), StoreError> {
        self.0.blocking_read(off, buf).map_err(|_| StoreError::Flash)
    }
    fn erase_page(&mut self, off: u32) -> Result<(), StoreError> {
        self.0
            .blocking_erase(off, off + PAGE as u32)
            .map_err(|_| StoreError::Flash)
    }
    fn write(&mut self, off: u32, data: &[u8]) -> Result<(), StoreError> {
        self.0.blocking_write(off, data).map_err(|_| StoreError::Flash)
    }
}

pub type DeviceJournal = Journal<EmbassyFlash>;

/// Mount the flash journal and resolve the device identity. On a factory-fresh
/// device (no identity page) this mints one from `fill` — which must write 76
/// bytes of TRNG output (12 device_id ‖ 32 Ed25519 seed ‖ 32 X25519 secret) —
/// and persists it before returning. Panics only on a flash fault, which on
/// this device means the part is unusable for provisioning anyway.
pub fn mount_and_identity(
    flash: Flash<'static, Blocking>,
    fill: impl FnOnce(&mut [u8]),
) -> (DeviceJournal, StoredIdentity) {
    let mut journal = Journal::mount(EmbassyFlash(flash), Layout::DEFAULT).unwrap();
    let id = match journal.identity() {
        Some(id) => id,
        None => {
            let mut seed = [0u8; 76];
            fill(&mut seed);
            let mut id = StoredIdentity {
                device_id: [0; 12],
                sign_seed: [0; 32],
                kx_priv: [0; 32],
            };
            id.device_id.copy_from_slice(&seed[0..12]);
            id.sign_seed.copy_from_slice(&seed[12..44]);
            id.kx_priv.copy_from_slice(&seed[44..76]);
            journal.set_identity(&id).unwrap();
            id
        }
    };
    (journal, id)
}

/// The provisioning engine's `Store`, backed by the flash journal.
pub struct FlashStore {
    journal: DeviceJournal,
}

impl FlashStore {
    pub fn new(journal: DeviceJournal) -> Self {
        FlashStore { journal }
    }
}

impl Store for FlashStore {
    fn owner_pub(&self) -> Option<[u8; 32]> {
        self.journal.owner_pub()
    }
    fn seq(&self) -> u64 {
        self.journal.seq()
    }
    fn commit(&mut self, owner_pub: &[u8; 32], seq: u64, config: &[u8]) -> Result<(), ()> {
        self.journal.commit_config(owner_pub, seq, config).map_err(|_| ())
    }
    fn wifi_set(&mut self, ssid: &str, psk: &str) -> Result<(), ()> {
        self.journal.wifi_set(ssid, psk).map_err(|_| ())
    }
    fn wifi_clear(&mut self) -> Result<(), ()> {
        self.journal.wifi_clear().map_err(|_| ())
    }
    fn wifi_ssid(&self) -> Option<&str> {
        self.journal.wifi_ssid()
    }
    fn now(&self) -> u64 {
        // RTC UTC once disciplined by the GNSS; 0 on a cold clock (the server
        // records receive-time regardless).
        crate::clock::now_unix().unwrap_or(0)
    }
}

/// Build the provisioning engine from a persisted identity and flash store.
///
/// Uses the software AES-GCM backend. The engine is generic over the backend
/// (`Provisioner<S, A>`) so the STM32U0 AES engine could be dropped in here via
/// `Provisioner::new_with_aes` — but embassy-stm32 0.6 has no driver for the
/// U0's AES v2 peripheral (it only implements aes_v3b), so that awaits either
/// newer embassy support or a hardware-verified raw-PAC backend.
pub fn provisioner(id: StoredIdentity, journal: DeviceJournal) -> Provisioner<FlashStore> {
    let identity = Identity {
        device_id: id.device_id,
        sign: SigningKey::from_bytes(&id.sign_seed),
        kx_priv: id.kx_priv,
        fw: concat!("ephemerkey-rs-", env!("CARGO_PKG_VERSION")),
    };
    Provisioner::new(identity, FlashStore::new(journal))
}
