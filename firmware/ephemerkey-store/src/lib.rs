//! Persistent store for the ephemerkey firmware: the device identity and the
//! owner-bound config live in the STM32U0's internal flash, reached through a
//! tiny [`Flash`] trait so the whole journal is host-testable against a RAM
//! fake (see the `tests` module) and the on-target adapter is a thin shim over
//! `embassy_stm32::flash`.
//!
//! Two independent regions in the top pages of the bank (DESIGN.md §Storage,
//! "last 2×2KB pages, ping-pong journal + CRC"):
//!
//! - **Identity page** — device_id + Ed25519 seed + X25519 secret, minted from
//!   the TRNG on first boot and then immutable. One page, written once.
//! - **Config journal** — two 2 KiB slots holding `{owner_pub, seq, config,
//!   wifi}`. A commit writes a *fresh* record (generation + 1) to the **stale**
//!   slot and only then is it live; the currently-valid slot is never touched
//!   until the new one is durable and CRC-valid. So a torn write can lose the
//!   new config but **never the owner binding** — exactly the atomicity the
//!   provisioning engine's `Store::commit` contract demands. On mount the
//!   valid record with the highest generation wins; a torn slot fails CRC and
//!   the older one stands.
//!
//! Flash geometry is fixed to the STM32U0x3: 2048-byte erase pages, 8-byte
//! program words, 0xFF erased. Everything is bank-relative byte offsets, so the
//! journal is oblivious to where the linker places code as long as the three
//! pages it owns are carved out of the app's FLASH region (see the firmware's
//! `memory.x`).

#![cfg_attr(not(test), no_std)]

/// Erase granularity of the STM32U0x3 main flash.
pub const PAGE: usize = 2048;
/// Program granularity: the U0 programs one 64-bit double-word at a time and
/// (ECC) forbids re-programming a word before erase.
pub const WORD: usize = 8;
/// Value of erased flash.
pub const ERASED: u8 = 0xFF;

/// Max WiFi SSID / PSK we persist (bytes).
pub const SSID_MAX: usize = 32;
pub const PSK_MAX: usize = 64;

const REC_MAGIC: u32 = 0x314A_4B45; // "EKJ1" little-endian
const ID_MAGIC: u32 = 0x3149_4B45; // "EKI1" little-endian
const REC_HDR: usize = 64; // fixed config-record header, then variable tail

/// Largest config blob that fits one journal slot alongside the header, the
/// WiFi credentials, and the trailing CRC. Real configs are well under 1 KiB;
/// [`Journal::commit_config`] rejects anything larger with [`Error::TooBig`].
pub const CONFIG_CAP: usize = PAGE - REC_HDR - SSID_MAX - PSK_MAX - 4 - WORD;

/// Flash operations the journal needs. Offsets are bank-relative bytes; the
/// on-target impl forwards to `embassy_stm32::flash::Flash::blocking_*`.
pub trait Flash {
    /// Read `buf.len()` bytes starting at `off`.
    fn read(&mut self, off: u32, buf: &mut [u8]) -> Result<(), Error>;
    /// Erase the 2 KiB page starting at `off` (must be [`PAGE`]-aligned).
    fn erase_page(&mut self, off: u32) -> Result<(), Error>;
    /// Program `data` at `off`. Both `off` and `data.len()` are multiples of
    /// [`WORD`]; the target must have been erased since it was last written.
    fn write(&mut self, off: u32, data: &[u8]) -> Result<(), Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The underlying flash refused a read/erase/write.
    Flash,
    /// A config blob (or WiFi field) is too large for a journal slot.
    TooBig,
}

/// Where the three journal pages live in the bank.
#[derive(Clone, Copy)]
pub struct Layout {
    pub identity: u32,
    pub slot_a: u32,
    pub slot_b: u32,
}

impl Layout {
    /// Top three pages of a 256 KiB bank (identity, then the two config slots).
    /// Must match the FLASH length reserved in the firmware's `memory.x`.
    pub const DEFAULT: Layout = Layout {
        identity: 0x3E800, // 250 KiB
        slot_a: 0x3F000,   // 252 KiB
        slot_b: 0x3F800,   // 254 KiB
    };
}

/// The immutable per-device secrets, minted once from the TRNG.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct StoredIdentity {
    pub device_id: [u8; 12],
    pub sign_seed: [u8; 32],
    pub kx_priv: [u8; 32],
}

// Redacts the secrets: the seeds must never reach a log line (or an assert
// dump). Only the non-secret device_id is shown.
impl core::fmt::Debug for StoredIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StoredIdentity")
            .field("device_id", &self.device_id)
            .field("sign_seed", &"<redacted>")
            .field("kx_priv", &"<redacted>")
            .finish()
    }
}

/// Parsed config-record header (fields copied out; the variable tail stays in
/// the caller's page buffer).
#[derive(Clone, Copy)]
struct Header {
    generation: u32,
    owner: Option<[u8; 32]>,
    seq: u64,
    config_len: usize,
    ssid_len: usize,
    psk_len: usize,
}

pub struct Journal<F: Flash> {
    flash: F,
    layout: Layout,

    // Live config-record mirror (RAM), so the `Store` accessors are cheap.
    generation: u32,
    cur_slot: Option<u32>,
    owner: Option<[u8; 32]>,
    seq: u64,
    config: [u8; CONFIG_CAP],
    config_len: usize,
    ssid: [u8; SSID_MAX],
    ssid_len: usize,
    psk: [u8; PSK_MAX],
    psk_len: usize,

    identity: Option<StoredIdentity>,
}

impl<F: Flash> Journal<F> {
    /// Read both regions and load the newest valid config record. A blank
    /// device mounts cleanly with no identity, no owner, seq 0.
    pub fn mount(flash: F, layout: Layout) -> Result<Self, Error> {
        let mut j = Journal {
            flash,
            layout,
            generation: 0,
            cur_slot: None,
            owner: None,
            seq: 0,
            config: [0; CONFIG_CAP],
            config_len: 0,
            ssid: [0; SSID_MAX],
            ssid_len: 0,
            psk: [0; PSK_MAX],
            psk_len: 0,
            identity: None,
        };
        j.load_identity()?;
        j.load_config()?;
        Ok(j)
    }

    // ---- identity -------------------------------------------------------

    pub fn identity(&self) -> Option<StoredIdentity> {
        self.identity
    }

    /// Write the device identity (erases the identity page first). Called once
    /// on first boot after minting; idempotent to re-call with the same bytes.
    pub fn set_identity(&mut self, id: &StoredIdentity) -> Result<(), Error> {
        let mut buf = [0u8; 96];
        put_u32(&mut buf[0..4], ID_MAGIC);
        buf[8..20].copy_from_slice(&id.device_id);
        buf[24..56].copy_from_slice(&id.sign_seed);
        buf[56..88].copy_from_slice(&id.kx_priv);
        let crc = crc32(&buf[..88]);
        put_u32(&mut buf[88..92], crc);
        // 92 -> next WORD multiple = 96.
        self.flash.erase_page(self.layout.identity)?;
        self.flash.write(self.layout.identity, &buf[..96])?;
        self.identity = Some(*id);
        Ok(())
    }

    fn load_identity(&mut self) -> Result<(), Error> {
        let mut buf = [0u8; 96];
        self.flash.read(self.layout.identity, &mut buf)?;
        if get_u32(&buf[0..4]) != ID_MAGIC {
            return Ok(());
        }
        if crc32(&buf[..88]) != get_u32(&buf[88..92]) {
            return Ok(());
        }
        let mut id = StoredIdentity { device_id: [0; 12], sign_seed: [0; 32], kx_priv: [0; 32] };
        id.device_id.copy_from_slice(&buf[8..20]);
        id.sign_seed.copy_from_slice(&buf[24..56]);
        id.kx_priv.copy_from_slice(&buf[56..88]);
        self.identity = Some(id);
        Ok(())
    }

    // ---- config-record accessors (back the provisioning `Store`) --------

    pub fn owner_pub(&self) -> Option<[u8; 32]> {
        self.owner
    }
    pub fn seq(&self) -> u64 {
        self.seq
    }
    pub fn config(&self) -> &[u8] {
        &self.config[..self.config_len]
    }
    pub fn wifi_ssid(&self) -> Option<&str> {
        if self.ssid_len == 0 {
            None
        } else {
            core::str::from_utf8(&self.ssid[..self.ssid_len]).ok()
        }
    }
    pub fn wifi_psk(&self) -> Option<&str> {
        if self.psk_len == 0 {
            None
        } else {
            core::str::from_utf8(&self.psk[..self.psk_len]).ok()
        }
    }

    /// Atomically commit a verified config: owner binding + anti-rollback seq +
    /// the config bytes. WiFi credentials are carried over unchanged.
    pub fn commit_config(&mut self, owner: &[u8; 32], seq: u64, config: &[u8]) -> Result<(), Error> {
        if config.len() > CONFIG_CAP {
            return Err(Error::TooBig);
        }
        self.owner = Some(*owner);
        self.seq = seq;
        self.config[..config.len()].copy_from_slice(config);
        self.config_len = config.len();
        self.persist()
    }

    pub fn wifi_set(&mut self, ssid: &str, psk: &str) -> Result<(), Error> {
        if ssid.len() > SSID_MAX || psk.len() > PSK_MAX {
            return Err(Error::TooBig);
        }
        self.ssid[..ssid.len()].copy_from_slice(ssid.as_bytes());
        self.ssid_len = ssid.len();
        self.psk[..psk.len()].copy_from_slice(psk.as_bytes());
        self.psk_len = psk.len();
        self.persist()
    }

    pub fn wifi_clear(&mut self) -> Result<(), Error> {
        self.ssid_len = 0;
        self.psk_len = 0;
        self.persist()
    }

    // ---- ping-pong write ------------------------------------------------

    /// Serialize the current mirror into the stale slot with the next
    /// generation, then flip to it. The live slot is untouched until this
    /// returns Ok, so a failure/power-loss here leaves the prior record valid.
    fn persist(&mut self) -> Result<(), Error> {
        let target = match self.cur_slot {
            Some(s) if s == self.layout.slot_a => self.layout.slot_b,
            _ => self.layout.slot_a,
        };
        let gen = self.generation.wrapping_add(1);

        let mut buf = [0u8; PAGE];
        put_u32(&mut buf[0..4], REC_MAGIC);
        put_u32(&mut buf[4..8], gen);
        put_u64(&mut buf[8..16], self.seq);
        let mut flags = 0u32;
        if self.owner.is_some() {
            flags |= 1;
        }
        if self.config_len > 0 {
            flags |= 2;
        }
        put_u32(&mut buf[16..20], flags);
        put_u32(&mut buf[20..24], self.config_len as u32);
        put_u32(&mut buf[24..28], self.ssid_len as u32);
        put_u32(&mut buf[28..32], self.psk_len as u32);
        if let Some(o) = self.owner {
            buf[32..64].copy_from_slice(&o);
        }
        let mut off = REC_HDR;
        buf[off..off + self.config_len].copy_from_slice(&self.config[..self.config_len]);
        off += self.config_len;
        buf[off..off + self.ssid_len].copy_from_slice(&self.ssid[..self.ssid_len]);
        off += self.ssid_len;
        buf[off..off + self.psk_len].copy_from_slice(&self.psk[..self.psk_len]);
        off += self.psk_len;
        let crc = crc32(&buf[..off]);
        put_u32(&mut buf[off..off + 4], crc);
        off += 4;
        let end = round_up(off, WORD);

        self.flash.erase_page(target)?;
        self.flash.write(target, &buf[..end])?;

        self.cur_slot = Some(target);
        self.generation = gen;
        Ok(())
    }

    fn load_config(&mut self) -> Result<(), Error> {
        let mut buf = [0u8; PAGE];
        for &slot in &[self.layout.slot_a, self.layout.slot_b] {
            if self.flash.read(slot, &mut buf).is_err() {
                continue;
            }
            if let Some(h) = parse_header(&buf) {
                if self.cur_slot.is_none() || h.generation > self.generation {
                    self.load_from(&buf, slot, &h);
                }
            }
        }
        Ok(())
    }

    fn load_from(&mut self, buf: &[u8; PAGE], slot: u32, h: &Header) {
        self.generation = h.generation;
        self.cur_slot = Some(slot);
        self.owner = h.owner;
        self.seq = h.seq;
        let mut off = REC_HDR;
        self.config[..h.config_len].copy_from_slice(&buf[off..off + h.config_len]);
        self.config_len = h.config_len;
        off += h.config_len;
        self.ssid[..h.ssid_len].copy_from_slice(&buf[off..off + h.ssid_len]);
        self.ssid_len = h.ssid_len;
        off += h.ssid_len;
        self.psk[..h.psk_len].copy_from_slice(&buf[off..off + h.psk_len]);
        self.psk_len = h.psk_len;
    }
}

/// Validate a slot's magic, field bounds, and CRC; return the header fields.
fn parse_header(buf: &[u8; PAGE]) -> Option<Header> {
    if get_u32(&buf[0..4]) != REC_MAGIC {
        return None;
    }
    let generation = get_u32(&buf[4..8]);
    let seq = get_u64(&buf[8..16]);
    let flags = get_u32(&buf[16..20]);
    let config_len = get_u32(&buf[20..24]) as usize;
    let ssid_len = get_u32(&buf[24..28]) as usize;
    let psk_len = get_u32(&buf[28..32]) as usize;
    if config_len > CONFIG_CAP || ssid_len > SSID_MAX || psk_len > PSK_MAX {
        return None;
    }
    let body_end = REC_HDR + config_len + ssid_len + psk_len;
    if body_end + 4 > PAGE {
        return None;
    }
    if crc32(&buf[..body_end]) != get_u32(&buf[body_end..body_end + 4]) {
        return None;
    }
    let owner = if flags & 1 != 0 {
        let mut o = [0u8; 32];
        o.copy_from_slice(&buf[32..64]);
        Some(o)
    } else {
        None
    };
    Some(Header { generation, owner, seq, config_len, ssid_len, psk_len })
}

fn round_up(n: usize, to: usize) -> usize {
    (n + to - 1) / to * to
}

fn get_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
fn get_u64(b: &[u8]) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[..8]);
    u64::from_le_bytes(a)
}
fn put_u32(b: &mut [u8], v: u32) {
    b[..4].copy_from_slice(&v.to_le_bytes());
}
fn put_u64(b: &mut [u8], v: u64) {
    b[..8].copy_from_slice(&v.to_le_bytes());
}

/// CRC-32 (IEEE, reflected 0xEDB88320) — same polynomial the provisioning
/// engine uses for its config-transfer checksum.
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RAM stand-in for the three journal pages. Enforces the U0's rules: erase
    /// sets 0xFF, and a program may only touch currently-erased bytes (ECC
    /// forbids re-writing a word before erase) — so any accidental double-write
    /// in the journal would blow up here.
    struct FakeFlash {
        mem: Vec<u8>,
    }
    impl FakeFlash {
        fn new() -> Self {
            FakeFlash { mem: vec![ERASED; 3 * PAGE] }
        }
    }
    impl Flash for FakeFlash {
        fn read(&mut self, off: u32, buf: &mut [u8]) -> Result<(), Error> {
            let off = off as usize;
            buf.copy_from_slice(&self.mem[off..off + buf.len()]);
            Ok(())
        }
        fn erase_page(&mut self, off: u32) -> Result<(), Error> {
            assert_eq!(off as usize % PAGE, 0, "erase must be page-aligned");
            let off = off as usize;
            for b in &mut self.mem[off..off + PAGE] {
                *b = ERASED;
            }
            Ok(())
        }
        fn write(&mut self, off: u32, data: &[u8]) -> Result<(), Error> {
            assert_eq!(off as usize % WORD, 0, "write offset must be word-aligned");
            assert_eq!(data.len() % WORD, 0, "write length must be word-aligned");
            let off = off as usize;
            for (i, &b) in data.iter().enumerate() {
                assert_eq!(self.mem[off + i], ERASED, "programming a non-erased byte");
                self.mem[off + i] = b;
            }
            Ok(())
        }
    }

    // Compact layout for tests: the FakeFlash is exactly the three pages.
    const TL: Layout = Layout { identity: 0, slot_a: PAGE as u32, slot_b: 2 * PAGE as u32 };

    fn owner(n: u8) -> [u8; 32] {
        [n; 32]
    }

    #[test]
    fn blank_device_mounts_empty() {
        let j = Journal::mount(FakeFlash::new(), TL).unwrap();
        assert_eq!(j.identity(), None);
        assert_eq!(j.owner_pub(), None);
        assert_eq!(j.seq(), 0);
        assert!(j.config().is_empty());
        assert_eq!(j.wifi_ssid(), None);
    }

    #[test]
    fn identity_persists_across_remount() {
        let id = StoredIdentity { device_id: [7; 12], sign_seed: [9; 32], kx_priv: [11; 32] };
        let mut j = Journal::mount(FakeFlash::new(), TL).unwrap();
        j.set_identity(&id).unwrap();
        assert_eq!(j.identity(), Some(id));
        // Remount the same flash: identity survives.
        let j2 = Journal::mount(j.flash, TL).unwrap();
        assert_eq!(j2.identity(), Some(id));
    }

    #[test]
    fn config_commit_persists_and_bumps_generation() {
        let mut j = Journal::mount(FakeFlash::new(), TL).unwrap();
        let cfg1 = br#"{"role":2,"slots":[]}"#;
        j.commit_config(&owner(1), 1, cfg1).unwrap();
        assert_eq!(j.owner_pub(), Some(owner(1)));
        assert_eq!(j.seq(), 1);
        assert_eq!(j.config(), cfg1);
        // first commit lands in slot_a
        assert_eq!(j.cur_slot, Some(TL.slot_a));

        let cfg2 = br#"{"role":2,"slots":[{"idx":0}]}"#;
        j.commit_config(&owner(1), 2, cfg2).unwrap();
        assert_eq!(j.seq(), 2);
        assert_eq!(j.config(), cfg2);
        // second commit ping-pongs to slot_b
        assert_eq!(j.cur_slot, Some(TL.slot_b));

        let j2 = Journal::mount(j.flash, TL).unwrap();
        assert_eq!(j2.owner_pub(), Some(owner(1)));
        assert_eq!(j2.seq(), 2);
        assert_eq!(j2.config(), cfg2);
    }

    #[test]
    fn wifi_round_trips_and_survives_config_commit() {
        let mut j = Journal::mount(FakeFlash::new(), TL).unwrap();
        j.wifi_set("prov-net", "hunter2").unwrap();
        j.commit_config(&owner(5), 1, b"{}").unwrap();
        // wifi carried over through the config commit
        assert_eq!(j.wifi_ssid(), Some("prov-net"));
        assert_eq!(j.wifi_psk(), Some("hunter2"));
        let j2 = Journal::mount(j.flash, TL).unwrap();
        assert_eq!(j2.wifi_ssid(), Some("prov-net"));
        assert_eq!(j2.wifi_psk(), Some("hunter2"));
        // clear
        let mut j2 = j2;
        j2.wifi_clear().unwrap();
        assert_eq!(j2.wifi_ssid(), None);
        assert_eq!(Journal::mount(j2.flash, TL).unwrap().wifi_ssid(), None);
    }

    #[test]
    fn torn_commit_preserves_prior_owner_binding() {
        // Two good commits: slot_a(gen1), slot_b(gen2 live).
        let mut j = Journal::mount(FakeFlash::new(), TL).unwrap();
        j.commit_config(&owner(1), 1, b"one").unwrap();
        j.commit_config(&owner(1), 2, b"two").unwrap();
        assert_eq!(j.cur_slot, Some(TL.slot_b));
        let mut flash = j.flash;

        // Simulate a torn third commit: it targets the stale slot_a — power is
        // lost after the erase but mid-write, so slot_a is erased + partially
        // programmed with a wrong CRC.
        flash.erase_page(TL.slot_a).unwrap();
        let mut garbage = [0u8; WORD];
        put_u32(&mut garbage[0..4], REC_MAGIC); // looks like a record...
        put_u32(&mut garbage[4..8], 99); // ...huge generation...
        flash.write(TL.slot_a, &garbage).unwrap(); // ...but truncated: CRC fails

        // Remount: slot_a is rejected, slot_b(gen2) stands — binding intact.
        let j = Journal::mount(flash, TL).unwrap();
        assert_eq!(j.owner_pub(), Some(owner(1)));
        assert_eq!(j.seq(), 2);
        assert_eq!(j.config(), b"two");
        assert_eq!(j.cur_slot, Some(TL.slot_b));
    }

    #[test]
    fn oversized_config_is_rejected() {
        let mut j = Journal::mount(FakeFlash::new(), TL).unwrap();
        let big = vec![b'x'; CONFIG_CAP + 1];
        assert_eq!(j.commit_config(&owner(1), 1, &big), Err(Error::TooBig));
        // a config that exactly fills capacity is fine
        let max = vec![b'y'; CONFIG_CAP];
        assert!(j.commit_config(&owner(1), 1, &max).is_ok());
        assert_eq!(Journal::mount(j.flash, TL).unwrap().config().len(), CONFIG_CAP);
    }

    #[test]
    fn higher_generation_wins_regardless_of_slot_order() {
        // Force the live record into slot_a with a higher generation than a
        // stale slot_b, and confirm mount picks slot_a.
        let mut j = Journal::mount(FakeFlash::new(), TL).unwrap();
        j.commit_config(&owner(1), 1, b"a").unwrap(); // slot_a gen1
        j.commit_config(&owner(1), 2, b"b").unwrap(); // slot_b gen2
        j.commit_config(&owner(1), 3, b"c").unwrap(); // slot_a gen3 (live)
        assert_eq!(j.cur_slot, Some(TL.slot_a));
        let j = Journal::mount(j.flash, TL).unwrap();
        assert_eq!(j.seq(), 3);
        assert_eq!(j.config(), b"c");
        assert_eq!(j.cur_slot, Some(TL.slot_a));
    }
}
