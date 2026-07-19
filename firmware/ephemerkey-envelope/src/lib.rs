//! ephemerkey envelope — the pinned wire format for configs, acks, and
//! telemetry (DESIGN-management.md §Encoding), shared by firmware, the
//! emulator, and ekctl-server. Browser code mirrors it independently; the
//! control repo's smoke tests are the cross-implementation check.
//!
//! Profile "ekenv-v1":
//! - `COSE_Sign1` = `[protected: bstr{1: -8/EdDSA}, unprotected: {4: kid?},
//!   payload: bstr, sig: bstr64]`, Ed25519 over the standard
//!   `Sig_structure ["Signature1", protected, b"", payload]`.
//! - `COSE_Encrypt0` = `[protected: bstr{1: 1/A128GCM, 4: kid=target
//!   device_id, -65537: seq}, unprotected: {5: iv12, -65537-1: eph_pub32},
//!   ciphertext ‖ tag16]`. Key = HKDF-SHA256(X25519(eph, device_kx),
//!   salt=eph_pub, info="ekenv-v1", 16). AAD = `Enc_structure ["Encrypt0",
//!   protected, b""]`. seq/target ride in the *protected* header so relays
//!   can order and route blobs without decrypting — and cannot forge either
//!   without failing the tag.
//! - A sealed config is `Encrypt0(Sign1(config_cbor, owner_key), device_kx)`.
//!
//! no_std, no-alloc: callers pass output + scratch buffers, and supply
//! randomness (ephemeral key, IV) explicitly — firmware uses its TRNG,
//! hosts use OS randomness, tests use fixed bytes.

#![cfg_attr(not(test), no_std)]

pub mod cbor;
pub mod schema;

use aes_gcm::aead::AeadInPlace;
use aes_gcm::{Aes128Gcm, KeyInit, Nonce, Tag};
use cbor::{Dec, Enc};
pub use ed25519_dalek::{SigningKey, VerifyingKey};
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey as KxPublic, StaticSecret as KxSecret};

pub const ALG_EDDSA: i64 = -8;
pub const ALG_A128GCM: i64 = 1;
pub const HDR_ALG: i64 = 1;
pub const HDR_KID: i64 = 4;
pub const HDR_IV: i64 = 5;
pub const HDR_SEQ: i64 = -65537; // private-use label: config sequence
pub const HDR_EPH: i64 = -65538; // private-use label: ephemeral X25519 pub

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    BufferTooSmall,
    Truncated,
    Malformed,
    WrongAlgorithm,
    BadSignature,
    DecryptFailed,
}

// --- COSE_Sign1 -----------------------------------------------------------

/// Protected header for Sign1 is constant: {1: -8}.
const SIGN1_PROTECTED: &[u8] = &[0xa1, 0x01, 0x27]; // map(1) 1 => -8

fn sig_structure(scratch: &mut [u8], protected: &[u8], payload: &[u8]) -> Result<usize, Error> {
    let mut e = Enc::new(scratch);
    e.array(4)?;
    e.tstr("Signature1")?;
    e.bstr(protected)?;
    e.bstr(&[])?; // external_aad
    e.bstr(payload)?;
    Ok(e.len())
}

/// Encode + sign a COSE_Sign1 into `out`; `scratch` must hold
/// `payload.len() + 32` bytes. Returns the encoded length.
pub fn sign1_write(
    out: &mut [u8],
    scratch: &mut [u8],
    payload: &[u8],
    kid: Option<&[u8]>,
    key: &SigningKey,
) -> Result<usize, Error> {
    use ed25519_dalek::Signer;
    let sig_len = sig_structure(scratch, SIGN1_PROTECTED, payload)?;
    let sig = key.sign(&scratch[..sig_len]);

    let mut e = Enc::new(out);
    e.array(4)?;
    e.bstr(SIGN1_PROTECTED)?;
    match kid {
        Some(k) => {
            e.map(1)?;
            e.int(HDR_KID)?;
            e.bstr(k)?;
        }
        None => e.map(0)?,
    }
    e.bstr(payload)?;
    e.bstr(&sig.to_bytes())?;
    Ok(e.len())
}

/// Verify a COSE_Sign1; returns `(payload, kid)` borrowed from `blob`.
/// `scratch` must hold `payload.len() + 32` bytes.
pub fn sign1_verify<'a>(
    blob: &'a [u8],
    scratch: &mut [u8],
    key: &VerifyingKey,
) -> Result<(&'a [u8], Option<&'a [u8]>), Error> {
    use ed25519_dalek::Verifier;
    let (protected, kid, payload, sig) = sign1_parse(blob)?;
    let mut alg_dec = Dec::new(protected);
    let n = alg_dec.map()?;
    let mut alg_ok = false;
    for _ in 0..n {
        if alg_dec.int()? == HDR_ALG {
            alg_ok = alg_dec.int()? == ALG_EDDSA;
        } else {
            alg_dec.skip()?;
        }
    }
    if !alg_ok {
        return Err(Error::WrongAlgorithm);
    }
    let sig_len = sig_structure(scratch, protected, payload)?;
    let sig = ed25519_dalek::Signature::from_slice(sig).map_err(|_| Error::Malformed)?;
    key.verify(&scratch[..sig_len], &sig)
        .map_err(|_| Error::BadSignature)?;
    Ok((payload, kid))
}

/// Structural parse without verification: `(protected, kid, payload, sig)`.
pub fn sign1_parse(blob: &[u8]) -> Result<(&[u8], Option<&[u8]>, &[u8], &[u8]), Error> {
    let mut d = Dec::new(blob);
    if d.array()? != 4 {
        return Err(Error::Malformed);
    }
    let protected = d.bstr()?;
    let mut kid = None;
    let n = d.map()?;
    for _ in 0..n {
        if d.int()? == HDR_KID {
            kid = Some(d.bstr()?);
        } else {
            d.skip()?;
        }
    }
    let payload = d.bstr()?;
    let sig = d.bstr()?;
    Ok((protected, kid, payload, sig))
}

// --- COSE_Encrypt0 (seal / peek / open) -----------------------------------

fn derive_key(shared: &[u8; 32], eph_pub: &[u8; 32]) -> [u8; 16] {
    let hk = Hkdf::<Sha256>::new(Some(eph_pub), shared);
    let mut key = [0u8; 16];
    hk.expand(b"ekenv-v1", &mut key).expect("hkdf len");
    key
}

fn enc_structure(scratch: &mut [u8], protected: &[u8]) -> Result<usize, Error> {
    let mut e = Enc::new(scratch);
    e.array(3)?;
    e.tstr("Encrypt0")?;
    e.bstr(protected)?;
    e.bstr(&[])?;
    Ok(e.len())
}

fn encrypt0_protected(buf: &mut [u8], seq: u64, target: &[u8]) -> Result<usize, Error> {
    let mut e = Enc::new(buf);
    e.map(3)?;
    e.int(HDR_ALG)?;
    e.int(ALG_A128GCM)?;
    e.int(HDR_KID)?;
    e.bstr(target)?;
    e.int(HDR_SEQ)?;
    e.uint(seq)?;
    Ok(e.len())
}

/// Seal `plaintext` (normally a COSE_Sign1) to `device_kx_pub`. The caller
/// supplies the ephemeral secret and IV (randomness is external by design).
pub fn seal_write(
    out: &mut [u8],
    plaintext: &[u8],
    device_kx_pub: &[u8; 32],
    seq: u64,
    target: &[u8],
    eph_secret: &[u8; 32],
    iv: &[u8; 12],
) -> Result<usize, Error> {
    let eph = KxSecret::from(*eph_secret);
    let eph_pub = KxPublic::from(&eph);
    let shared = eph.diffie_hellman(&KxPublic::from(*device_kx_pub));
    let key = derive_key(shared.as_bytes(), eph_pub.as_bytes());

    let mut protected = [0u8; 80];
    let plen = encrypt0_protected(&mut protected, seq, target)?;
    let mut aad = [0u8; 112];
    let alen = enc_structure(&mut aad, &protected[..plen])?;

    let mut e = Enc::new(out);
    e.array(3)?;
    e.bstr(&protected[..plen])?;
    e.map(2)?;
    e.int(HDR_IV)?;
    e.bstr(iv)?;
    e.int(HDR_EPH)?;
    e.bstr(eph_pub.as_bytes())?;
    let ct_range = e.bstr_reserve(plaintext.len() + 16)?;
    let total = e.len();

    let (ct, tag_out) = out[ct_range].split_at_mut(plaintext.len());
    ct.copy_from_slice(plaintext);
    let cipher = Aes128Gcm::new((&key).into());
    let tag = cipher
        .encrypt_in_place_detached(Nonce::from_slice(iv), &aad[..alen], ct)
        .map_err(|_| Error::DecryptFailed)?;
    tag_out.copy_from_slice(&tag);
    Ok(total)
}

/// Parse routing metadata without decrypting: `(seq, target)`. Relays use
/// this; note the values are only *trustworthy* after a successful `open`.
pub fn peek(blob: &[u8]) -> Result<(u64, &[u8]), Error> {
    let mut d = Dec::new(blob);
    if d.array()? != 3 {
        return Err(Error::Malformed);
    }
    let protected = d.bstr()?;
    let mut p = Dec::new(protected);
    let n = p.map()?;
    let (mut seq, mut target, mut alg_ok) = (None, None, false);
    for _ in 0..n {
        match p.int()? {
            HDR_ALG => alg_ok = p.int()? == ALG_A128GCM,
            HDR_KID => target = Some(p.bstr()?),
            HDR_SEQ => seq = Some(p.uint()?),
            _ => p.skip()?,
        }
    }
    if !alg_ok {
        return Err(Error::WrongAlgorithm);
    }
    match (seq, target) {
        (Some(s), Some(t)) => Ok((s, t)),
        _ => Err(Error::Malformed),
    }
}

/// Decrypt a sealed blob with the device kx secret. Writes the plaintext
/// (the inner COSE_Sign1) into `out`, returning `(plaintext_len, seq)`.
/// The caller must then verify the inner Sign1 against the owner key and
/// enforce seq monotonicity — decryption alone proves nothing.
pub fn open(
    blob: &[u8],
    out: &mut [u8],
    kx_secret: &[u8; 32],
) -> Result<(usize, u64), Error> {
    let mut d = Dec::new(blob);
    if d.array()? != 3 {
        return Err(Error::Malformed);
    }
    let protected = d.bstr()?;
    let n = d.map()?;
    let (mut iv, mut eph_pub) = (None, None);
    for _ in 0..n {
        match d.int()? {
            HDR_IV => iv = Some(d.bstr()?),
            HDR_EPH => eph_pub = Some(d.bstr()?),
            _ => d.skip()?,
        }
    }
    let ct = d.bstr()?;
    let (seq, _target) = peek(blob)?;
    let iv: &[u8; 12] = iv
        .ok_or(Error::Malformed)?
        .try_into()
        .map_err(|_| Error::Malformed)?;
    let eph_pub: &[u8; 32] = eph_pub
        .ok_or(Error::Malformed)?
        .try_into()
        .map_err(|_| Error::Malformed)?;
    if ct.len() < 16 {
        return Err(Error::Malformed);
    }

    let secret = KxSecret::from(*kx_secret);
    let shared = secret.diffie_hellman(&KxPublic::from(*eph_pub));
    let key = derive_key(shared.as_bytes(), eph_pub);

    let mut aad = [0u8; 112];
    let alen = enc_structure(&mut aad, protected)?;

    let (ct_body, tag) = ct.split_at(ct.len() - 16);
    let pt = out.get_mut(..ct_body.len()).ok_or(Error::BufferTooSmall)?;
    pt.copy_from_slice(ct_body);
    let cipher = Aes128Gcm::new((&key).into());
    cipher
        .decrypt_in_place_detached(Nonce::from_slice(iv), &aad[..alen], pt, Tag::from_slice(tag))
        .map_err(|_| Error::DecryptFailed)?;
    Ok((ct_body.len(), seq))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> (SigningKey, [u8; 32], [u8; 32]) {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let kx_secret = [9u8; 32];
        let kx_pub = *KxPublic::from(&KxSecret::from(kx_secret)).as_bytes();
        (sk, kx_secret, kx_pub)
    }

    #[test]
    fn sign1_roundtrip() {
        let (sk, _, _) = keys();
        let payload = b"hello envelope";
        let (mut out, mut scratch) = ([0u8; 256], [0u8; 256]);
        let n = sign1_write(&mut out, &mut scratch, payload, Some(b"dev-1"), &sk).unwrap();
        let (got, kid) = sign1_verify(&out[..n], &mut scratch, &sk.verifying_key()).unwrap();
        assert_eq!(got, payload);
        assert_eq!(kid, Some(&b"dev-1"[..]));
    }

    #[test]
    fn sign1_tamper_detected() {
        let (sk, _, _) = keys();
        let (mut out, mut scratch) = ([0u8; 256], [0u8; 256]);
        let n = sign1_write(&mut out, &mut scratch, b"payload", None, &sk).unwrap();
        // flip a payload byte
        out[n - 70] ^= 1;
        assert!(sign1_verify(&out[..n], &mut scratch, &sk.verifying_key()).is_err());
    }

    #[test]
    fn seal_peek_open_roundtrip() {
        let (sk, kx_secret, kx_pub) = keys();
        let config = b"{'role':2,'slots':[]} pretend-cbor";
        let (mut inner, mut scratch) = ([0u8; 512], [0u8; 512]);
        let ilen = sign1_write(&mut inner, &mut scratch, config, None, &sk).unwrap();

        let mut sealed = [0u8; 1024];
        let slen = seal_write(
            &mut sealed, &inner[..ilen], &kx_pub, 42, b"device-7", &[3u8; 32], &[5u8; 12],
        )
        .unwrap();

        let (seq, target) = peek(&sealed[..slen]).unwrap();
        assert_eq!((seq, target), (42, &b"device-7"[..]));

        let mut pt = [0u8; 1024];
        let (plen, seq2) = open(&sealed[..slen], &mut pt, &kx_secret).unwrap();
        assert_eq!(seq2, 42);
        assert_eq!(&pt[..plen], &inner[..ilen]);
        let (payload, _) = sign1_verify(&pt[..plen], &mut scratch, &sk.verifying_key()).unwrap();
        assert_eq!(payload, config);
    }

    #[test]
    fn seal_tamper_detected() {
        let (_, kx_secret, kx_pub) = keys();
        let mut sealed = [0u8; 256];
        let n =
            seal_write(&mut sealed, b"inner", &kx_pub, 1, b"d", &[3u8; 32], &[5u8; 12]).unwrap();
        // tampering with the protected seq must break the AEAD tag
        let mut evil = sealed;
        evil[6] ^= 1;
        let mut pt = [0u8; 256];
        assert!(open(&evil[..n], &mut pt, &kx_secret).is_err());
    }

    #[test]
    fn wrong_recipient_cannot_open() {
        let (_, _, kx_pub) = keys();
        let mut sealed = [0u8; 256];
        let n =
            seal_write(&mut sealed, b"inner", &kx_pub, 1, b"d", &[3u8; 32], &[5u8; 12]).unwrap();
        let mut pt = [0u8; 256];
        assert_eq!(
            open(&sealed[..n], &mut pt, &[8u8; 32]),
            Err(Error::DecryptFailed)
        );
    }
}
