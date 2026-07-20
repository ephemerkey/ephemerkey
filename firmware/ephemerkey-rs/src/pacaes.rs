//! Raw-register AES-128-GCM on the STM32U0 AES engine (peripheral **aes_v2**),
//! implementing the envelope's [`AesGcm128`] backend so the device opens sealed
//! configs on the accelerator instead of the software `aes-gcm` crate.
//!
//! embassy-stm32 0.6 ships an AES driver only for `aes_v3b` (U5/WBA), so this
//! drives the metapac register block directly per RM0503, cross-checked against
//! embassy's `aes_v3b` sequence (the register maps are near-identical; the v2
//! differences handled here are: **no ICR register** — CCF is cleared via the
//! `CCFC` bit in CR — and the data/key/IV registers are plain `u32` newtypes).
//!
//! ⚠️ **UNVERIFIED ON SILICON.** This code has never executed on a real U083.
//! GCM fails closed (a wrong tag simply fails authentication, so a bug breaks
//! provisioning rather than weakening it), but it must not be trusted until
//! bench-verified — which is why it lives behind the `hw-aes` feature (off by
//! default) while software AES stays the shipping default. To verify: build
//! `--features hw-aes`, run the console `/push` flow against the device, and
//! confirm the sealed config opens and the signed ack verifies. GCM is GCM, so
//! success means byte-compatibility with the soft path.

use embassy_stm32::pac;
use embassy_stm32::pac::aes::regs::{Dinr, Ivr, Keyr};
use embassy_stm32::pac::aes::vals::{Datatype, Gcmph, Mode};
use embassy_stm32::peripherals::AES;
use embassy_stm32::Peri;
use ephemerkey_envelope::AesGcm128;
use ephemerkey_provision::CONFIG_MAX;

pub struct PacAesGcm {
    _aes: Peri<'static, AES>,
    // The engine reads DINR and writes DOUTR, so payload needs a staging buffer
    // separate from the in-place envelope buffer. Sized to the largest blob.
    scratch: [u8; CONFIG_MAX],
}

fn regs() -> pac::aes::Aes {
    pac::AES
}

fn wait_ccf() {
    while !regs().sr().read().ccf() {}
}

/// Clear the computation-complete flag (v2: via CR.CCFC, there is no ICR).
fn clear_ccf() {
    regs().cr().modify(|w| w.set_ccfc(true));
}

/// Write a 16-byte block to DINR as four big-endian words (NO_SWAP datatype).
fn write_block(block: &[u8; 16]) {
    let p = regs();
    for i in 0..4 {
        let w = u32::from_be_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
        p.dinr().write_value(Dinr(w));
    }
}

/// Read a 16-byte block from DOUTR (four big-endian words).
fn read_block(out: &mut [u8; 16]) {
    let p = regs();
    for i in 0..4 {
        let w = p.doutr().read().0;
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
}

impl PacAesGcm {
    pub fn new(aes: Peri<'static, AES>) -> Self {
        // Enable and reset the AES peripheral clock.
        pac::RCC.ahbenr().modify(|w| w.set_aesen(true));
        pac::RCC.ahbrstr().modify(|w| w.set_aesrst(true));
        pac::RCC.ahbrstr().modify(|w| w.set_aesrst(false));
        PacAesGcm { _aes: aes, scratch: [0; CONFIG_MAX] }
    }

    /// One-shot AES-128-GCM over `buf` in place; returns the 16-byte tag.
    /// `dir` is the CR MODE field: encrypt (0) or decrypt (2).
    fn gcm(
        &mut self,
        dir: Mode,
        key: &[u8; 16],
        iv12: &[u8; 12],
        aad: &[u8],
        buf: &mut [u8],
    ) -> Option<[u8; 16]> {
        if buf.len() > self.scratch.len() {
            return None;
        }
        let p = regs();

        // ---- setup + INIT phase (RM: load key/IV, compute hash key H) ----
        p.cr().modify(|w| w.set_en(false));
        p.cr().modify(|w| {
            w.set_datatype(Datatype::from_bits(0)); // NO_SWAP: big-endian words
            w.set_keysize(false); // AES-128
            w.set_chmod10(0b11); // CHMOD[1:0]
            w.set_chmod2(false); // CHMOD[2] -> CHMOD = 0b011 = GCM
            w.set_mode(dir);
            w.set_gcmph(Gcmph::from_bits(0)); // INIT
        });
        // Key: 4 words, reverse register order, big-endian.
        for i in 0..4 {
            let w = u32::from_be_bytes([key[i * 4], key[i * 4 + 1], key[i * 4 + 2], key[i * 4 + 3]]);
            p.keyr(3 - i).write_value(Keyr(w));
        }
        // IV = iv12 ‖ 0x00000002 (initial counter), reverse register order.
        let mut iv = [0u8; 16];
        iv[..12].copy_from_slice(iv12);
        iv[15] = 2;
        for i in 0..4 {
            let w = u32::from_be_bytes([iv[i * 4], iv[i * 4 + 1], iv[i * 4 + 2], iv[i * 4 + 3]]);
            p.ivr(3 - i).write_value(Ivr(w));
        }
        p.cr().modify(|w| w.set_en(true));
        wait_ccf();
        clear_ccf();

        // ---- header (AAD) phase ----
        p.cr().modify(|w| w.set_gcmph(Gcmph::from_bits(1)));
        p.cr().modify(|w| w.set_en(true));
        let mut ai = 0;
        while ai + 16 <= aad.len() {
            let mut b = [0u8; 16];
            b.copy_from_slice(&aad[ai..ai + 16]);
            write_block(&b);
            wait_ccf();
            clear_ccf();
            ai += 16;
        }
        if ai < aad.len() {
            let mut b = [0u8; 16]; // zero-padded partial AAD block
            b[..aad.len() - ai].copy_from_slice(&aad[ai..]);
            write_block(&b);
            wait_ccf();
            clear_ccf();
        }

        // ---- payload phase ----
        let n = buf.len();
        self.scratch[..n].copy_from_slice(buf);
        p.cr().modify(|w| {
            w.set_gcmph(Gcmph::from_bits(2));
            w.set_npblb(0);
        });
        let mut off = 0;
        while off + 16 <= n {
            let mut b = [0u8; 16];
            b.copy_from_slice(&self.scratch[off..off + 16]);
            write_block(&b);
            wait_ccf();
            let mut o = [0u8; 16];
            read_block(&mut o);
            clear_ccf();
            buf[off..off + 16].copy_from_slice(&o);
            off += 16;
        }
        if off < n {
            let rem = n - off;
            p.cr().modify(|w| w.set_npblb((16 - rem) as u8));
            let mut b = [0u8; 16];
            b[..rem].copy_from_slice(&self.scratch[off..n]);
            write_block(&b);
            wait_ccf();
            let mut o = [0u8; 16];
            read_block(&mut o);
            clear_ccf();
            buf[off..n].copy_from_slice(&o[..rem]);
        }

        // ---- final phase: write the length block, read the tag ----
        while regs().sr().read().busy() {}
        p.cr().modify(|w| w.set_gcmph(Gcmph::from_bits(3)));
        let aad_bits = (aad.len() as u64) * 8;
        let pay_bits = (n as u64) * 8;
        // 128-bit length block: [aad_len_bits:64 ‖ payload_len_bits:64], BE.
        // Our sizes fit 32 bits, so the high words are zero.
        p.dinr().write_value(Dinr((aad_bits >> 32) as u32));
        p.dinr().write_value(Dinr(aad_bits as u32));
        p.dinr().write_value(Dinr((pay_bits >> 32) as u32));
        p.dinr().write_value(Dinr(pay_bits as u32));
        wait_ccf();
        let mut tag = [0u8; 16];
        read_block(&mut tag);
        clear_ccf();
        p.cr().modify(|w| w.set_en(false));
        Some(tag)
    }
}

impl AesGcm128 for PacAesGcm {
    fn seal(&mut self, key: &[u8; 16], iv: &[u8; 12], aad: &[u8], buf: &mut [u8]) -> [u8; 16] {
        self.gcm(Mode::from_bits(0), key, iv, aad, buf).unwrap_or([0; 16])
    }
    fn open(
        &mut self,
        key: &[u8; 16],
        iv: &[u8; 12],
        aad: &[u8],
        buf: &mut [u8],
        tag: &[u8; 16],
    ) -> Result<(), ()> {
        let computed = self.gcm(Mode::from_bits(2), key, iv, aad, buf).ok_or(())?;
        if ct_eq(&computed, tag) {
            Ok(())
        } else {
            Err(())
        }
    }
}

/// Constant-time 16-byte comparison.
fn ct_eq(a: &[u8; 16], b: &[u8; 16]) -> bool {
    let mut diff = 0u8;
    for i in 0..16 {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}
