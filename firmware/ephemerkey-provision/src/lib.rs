//! Transport-agnostic provisioning engine: give it raw link bytes (USB-CDC,
//! the ESP32 UART bridge, the emulator's TCP socket), it hands back framed
//! responses. Implements the contract in ephemerkey-control
//! `docs/serial-protocol.md` with the real envelope crypto:
//!
//! - IDENTITY: self-signed enrollment doc `{1:device_id, 2:sign_pub,
//!   3:kx_pub, 4:fw}`
//! - CHALLENGE: `Ed25519(sign, "ek-identify-v1" ‖ nonce)`
//! - CONFIG_*: chunked sealed blob → `envelope::open` → inner Sign1 whose
//!   `kid` carries `owner_pub` — **TOFU**: a factory-fresh device adopts the
//!   first owner, a bound device rejects others (`wrong-set`); `seq` must
//!   strictly increase (`seq-rollback`); success commits via [`Store`] and
//!   answers with a device-signed CONFIG_ACK `{1:seq, 2:sha256(blob)}`
//! - EVENTS_REQ: device-signed batch of the in-engine event ring
//! - WIFI_SET / WIFI_STATUS_REQ: `{1:ssid, 2:psk}` pass-through to [`Store`]
//!
//! no_std, no-alloc: all buffers live inside [`Provisioner`] (~14 KiB) — put
//! it in a `static` cell on the MCU, not on a task stack. `ekemu serial` is
//! the behavioral twin; ephemerkey-control's e2e suites exercise both ends.

#![cfg_attr(not(test), no_std)]

use ed25519_dalek::Signer;
use ephemerkey_envelope as env;
use ephemerkey_envelope::cbor::{Dec, Enc};
use ephemerkey_envelope::schema;
use ephemerkey_frame::{encode, err_code, frame_type as ft, Parser, MAX_PAYLOAD};
use sha2::{Digest, Sha256};

pub const CONFIG_MAX: usize = 4096;
const EVENT_RING: usize = 8;

/// Persistence the engine needs from the platform (flash journal on the
/// U083, a JSON file in tests/emulator). `commit` must be atomic: a torn
/// write may lose the new config but must never lose the owner binding.
pub trait Store {
    fn owner_pub(&self) -> Option<[u8; 32]>;
    fn seq(&self) -> u64;
    fn commit(&mut self, owner_pub: &[u8; 32], seq: u64, config: &[u8]) -> Result<(), ()>;
    fn wifi_set(&mut self, ssid: &str, psk: &str) -> Result<(), ()>;
    fn wifi_clear(&mut self) -> Result<(), ()>;
    fn wifi_ssid(&self) -> Option<&str>;
    /// Unix seconds (RTC); event timestamps only.
    fn now(&self) -> u64;
}

pub struct Identity {
    pub device_id: [u8; 12],
    pub sign: env::SigningKey,
    pub kx_priv: [u8; 32],
    pub fw: &'static str,
}

struct Xfer {
    len: usize,
    filled: usize,
    seq_hint: u32,
    crc: u32,
}

pub struct Provisioner<S: Store> {
    pub store: S,
    id: Identity,
    parser: Parser,
    xfer: Option<Xfer>,
    cfg_buf: [u8; CONFIG_MAX],
    pt_buf: [u8; CONFIG_MAX],
    scratch: [u8; CONFIG_MAX + 256],
    events: [(u64, u64, u64); EVENT_RING], // (seq, ts, kind)
    event_seq: u64,
}

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

impl<S: Store> Provisioner<S> {
    pub fn new(id: Identity, store: S) -> Self {
        Provisioner {
            store,
            id,
            parser: Parser::new(),
            xfer: None,
            cfg_buf: [0; CONFIG_MAX],
            pt_buf: [0; CONFIG_MAX],
            scratch: [0; CONFIG_MAX + 256],
            events: [(0, 0, 0); EVENT_RING],
            event_seq: 0,
        }
    }

    /// Feed raw link bytes; complete frames are handled and each response
    /// frame is passed to `tx` ready for the wire.
    pub fn feed(&mut self, bytes: &[u8], mut tx: impl FnMut(&[u8])) {
        // Collect parsed frames first (the parser borrows must end before
        // handling, which needs &mut self).
        let mut queue: [(u8, [u8; MAX_PAYLOAD], usize); 2] = [(0, [0; MAX_PAYLOAD], 0); 2];
        let mut n = 0;
        self.parser.feed(bytes, |t, p| {
            if n < queue.len() {
                queue[n].0 = t;
                queue[n].1[..p.len()].copy_from_slice(p);
                queue[n].2 = p.len();
                n += 1;
            }
        });
        for item in queue.iter().take(n) {
            let (rt, out, olen) = {
                let mut resp = [0u8; MAX_PAYLOAD];
                let (rt, rlen) = self.handle(item.0, &item.1[..item.2], &mut resp);
                (rt, resp, rlen)
            };
            let mut framed = [0u8; MAX_PAYLOAD + 8];
            if let Some(flen) = encode(&mut framed, rt, &out[..olen]) {
                tx(&framed[..flen]);
            }
        }
    }

    fn record_event(&mut self, kind: u64) {
        self.event_seq += 1;
        let slot = ((self.event_seq - 1) as usize) % EVENT_RING;
        self.events[slot] = (self.event_seq, self.store.now(), kind);
    }

    /// Returns (frame_type, payload_len) with the payload written to `resp`.
    fn handle(&mut self, ftype: u8, payload: &[u8], resp: &mut [u8]) -> (u8, usize) {
        match ftype {
            ft::IDENTITY_REQ => self.identity(resp),
            ft::CHALLENGE => {
                let mut msg = [0u8; 14 + 64];
                let nlen = payload.len().min(64);
                msg[..14].copy_from_slice(b"ek-identify-v1");
                msg[14..14 + nlen].copy_from_slice(&payload[..nlen]);
                let sig = self.id.sign.sign(&msg[..14 + nlen]);
                resp[..64].copy_from_slice(&sig.to_bytes());
                (ft::CHALLENGE_SIG, 64)
            }
            ft::CONFIG_BEGIN => {
                if payload.len() != 10 {
                    return err(resp, err_code::BAD_STATE);
                }
                let len = u16::from_le_bytes([payload[0], payload[1]]) as usize;
                if len == 0 || len > CONFIG_MAX {
                    return err(resp, err_code::BAD_STATE);
                }
                self.xfer = Some(Xfer {
                    len,
                    filled: 0,
                    seq_hint: u32::from_le_bytes(payload[2..6].try_into().unwrap()),
                    crc: u32::from_le_bytes(payload[6..10].try_into().unwrap()),
                });
                (ft::OK, 0)
            }
            ft::CONFIG_CHUNK => {
                let Some(x) = self.xfer.as_mut() else {
                    return err(resp, err_code::BAD_STATE);
                };
                if payload.len() < 2 {
                    return err(resp, err_code::BAD_STATE);
                }
                let off = u16::from_le_bytes([payload[0], payload[1]]) as usize;
                let data = &payload[2..];
                if off + data.len() > x.len {
                    return err(resp, err_code::BAD_STATE);
                }
                self.cfg_buf[off..off + data.len()].copy_from_slice(data);
                x.filled += data.len();
                (ft::OK, 0)
            }
            ft::CONFIG_COMMIT => self.commit(resp),
            ft::EVENTS_REQ => self.events_resp(payload, resp),
            ft::WIFI_SET => self.wifi_set(payload, resp),
            ft::WIFI_STATUS_REQ => {
                let ssid = self.store.wifi_ssid().unwrap_or("");
                let mut e = Enc::new(resp);
                let ok = (|| {
                    e.map(2)?;
                    e.uint(1)?;
                    e.uint(0)?; // connected: the provisioning link can't know
                    e.uint(2)?;
                    e.tstr(ssid)
                })();
                match ok {
                    Ok(()) => (ft::WIFI_STATUS, e.len()),
                    Err(_) => err(resp, err_code::BAD_STATE),
                }
            }
            _ => err(resp, err_code::BAD_STATE),
        }
    }

    fn identity(&mut self, resp: &mut [u8]) -> (u8, usize) {
        let kx_pub = x25519_pub(&self.id.kx_priv);
        let mut payload = [0u8; 128];
        let mut e = Enc::new(&mut payload);
        let built = (|| {
            e.map(4)?;
            e.uint(1)?;
            e.bstr(&self.id.device_id)?;
            e.uint(2)?;
            e.bstr(&self.id.sign.verifying_key().to_bytes())?;
            e.uint(3)?;
            e.bstr(&kx_pub)?;
            e.uint(4)?;
            e.tstr(self.id.fw)
        })();
        if built.is_err() {
            return err(resp, err_code::BAD_STATE);
        }
        let plen = e.len();
        match env::sign1_write(resp, &mut self.scratch, &payload[..plen], Some(&self.id.device_id), &self.id.sign) {
            Ok(n) => (ft::IDENTITY, n),
            Err(_) => err(resp, err_code::BAD_STATE),
        }
    }

    fn commit(&mut self, resp: &mut [u8]) -> (u8, usize) {
        let Some(x) = self.xfer.take() else {
            return err(resp, err_code::BAD_STATE);
        };
        if x.filled != x.len || crc32(&self.cfg_buf[..x.len]) != x.crc {
            return err(resp, err_code::CRC);
        }
        let Ok((ilen, seq)) = env::open(&self.cfg_buf[..x.len], &mut self.pt_buf, &self.id.kx_priv) else {
            return err(resp, err_code::BAD_SIG);
        };
        if seq != x.seq_hint as u64 {
            return err(resp, err_code::BAD_STATE);
        }
        if self.store.owner_pub().is_some() && seq <= self.store.seq() {
            return err(resp, err_code::SEQ_ROLLBACK);
        }
        let Ok((_, kid, _, _)) = env::sign1_parse(&self.pt_buf[..ilen]) else {
            return err(resp, err_code::BAD_SIG);
        };
        let Some(kid) = kid else {
            return err(resp, err_code::WRONG_SET);
        };
        let Ok(owner_bytes) = <[u8; 32]>::try_from(kid) else {
            return err(resp, err_code::WRONG_SET);
        };
        if let Some(bound) = self.store.owner_pub() {
            if bound != owner_bytes {
                return err(resp, err_code::WRONG_SET);
            }
        }
        let Ok(owner_key) = env::VerifyingKey::from_bytes(&owner_bytes) else {
            return err(resp, err_code::WRONG_SET);
        };
        let config_len = {
            let Ok((config, _)) = env::sign1_verify(&self.pt_buf[..ilen], &mut self.scratch, &owner_key) else {
                return err(resp, err_code::BAD_SIG);
            };
            let len = config.len();
            let start = config.as_ptr() as usize - self.pt_buf.as_ptr() as usize;
            self.pt_buf.copy_within(start..start + len, 0);
            len
        };
        let blob_hash: [u8; 32] = Sha256::digest(&self.cfg_buf[..x.len]).into();
        {
            let (pt, _) = self.pt_buf.split_at(config_len);
            if self.store.commit(&owner_bytes, seq, pt).is_err() {
                return err(resp, err_code::STORAGE);
            }
        }
        self.record_event(schema::EVT_CONFIG_ACK);

        let mut ack = [0u8; 64];
        let Ok(alen) = schema::ack_encode(&mut ack, seq, &blob_hash) else {
            return err(resp, err_code::BAD_STATE);
        };
        match env::sign1_write(resp, &mut self.scratch, &ack[..alen], Some(&self.id.device_id), &self.id.sign) {
            Ok(n) => (ft::CONFIG_ACK, n),
            Err(_) => err(resp, err_code::BAD_STATE),
        }
    }

    fn events_resp(&mut self, payload: &[u8], resp: &mut [u8]) -> (u8, usize) {
        let after = if payload.len() >= 4 {
            u32::from_le_bytes(payload[..4].try_into().unwrap()) as u64
        } else {
            0
        };
        let mut batch = [0u8; 512];
        let mut e = Enc::new(&mut batch);
        let count = self.events.iter().filter(|(s, _, _)| *s > after).count() as u64;
        let built = (|| {
            e.array(count)?;
            for (seq, ts, kind) in self.events.iter().filter(|(s, _, _)| *s > after) {
                schema::event_encode(
                    &mut e,
                    &schema::Event { seq: *seq, rtc_ts: *ts, kind: *kind, detail: None, chain_tag: None },
                )?;
            }
            Ok::<(), env::Error>(())
        })();
        if built.is_err() {
            return err(resp, err_code::BAD_STATE);
        }
        let blen = e.len();
        match env::sign1_write(resp, &mut self.scratch, &batch[..blen], Some(&self.id.device_id), &self.id.sign) {
            Ok(n) => (ft::EVENTS, n),
            Err(_) => err(resp, err_code::BAD_STATE),
        }
    }

    fn wifi_set(&mut self, payload: &[u8], resp: &mut [u8]) -> (u8, usize) {
        let mut d = Dec::new(payload);
        let Ok(n) = d.map() else {
            return err(resp, err_code::BAD_STATE);
        };
        let (mut ssid, mut psk) = (None, None);
        for _ in 0..n {
            match d.uint() {
                Ok(1) => ssid = d.tstr().ok(),
                Ok(2) => psk = d.tstr().ok(),
                _ => {
                    if d.skip().is_err() {
                        return err(resp, err_code::BAD_STATE);
                    }
                }
            }
        }
        let result = match ssid {
            Some("") => self.store.wifi_clear(),
            Some(s) => self.store.wifi_set(s, psk.unwrap_or("")),
            None => return err(resp, err_code::BAD_STATE),
        };
        match result {
            Ok(()) => (ft::OK, 0),
            Err(()) => err(resp, err_code::STORAGE),
        }
    }
}

fn err(resp: &mut [u8], code: u8) -> (u8, usize) {
    resp[0] = code;
    (ft::ERROR, 1)
}

fn x25519_pub(priv_key: &[u8; 32]) -> [u8; 32] {
    let secret = x25519_dalek::StaticSecret::from(*priv_key);
    *x25519_dalek::PublicKey::from(&secret).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ephemerkey_frame::frame_type as ftt;

    struct MemStore {
        owner: Option<[u8; 32]>,
        seq: u64,
        config: Vec<u8>,
        ssid: Option<String>,
    }

    impl Store for MemStore {
        fn owner_pub(&self) -> Option<[u8; 32]> {
            self.owner
        }
        fn seq(&self) -> u64 {
            self.seq
        }
        fn commit(&mut self, owner_pub: &[u8; 32], seq: u64, config: &[u8]) -> Result<(), ()> {
            self.owner = Some(*owner_pub);
            self.seq = seq;
            self.config = config.to_vec();
            Ok(())
        }
        fn wifi_set(&mut self, ssid: &str, _psk: &str) -> Result<(), ()> {
            self.ssid = Some(ssid.into());
            Ok(())
        }
        fn wifi_clear(&mut self) -> Result<(), ()> {
            self.ssid = None;
            Ok(())
        }
        fn wifi_ssid(&self) -> Option<&str> {
            self.ssid.as_deref()
        }
        fn now(&self) -> u64 {
            1_750_000_000
        }
    }

    fn device() -> Provisioner<MemStore> {
        let id = Identity {
            device_id: [0xd; 12],
            sign: env::SigningKey::from_bytes(&[11; 32]),
            kx_priv: [13; 32],
            fw: "test-0.1",
        };
        Provisioner::new(id, MemStore { owner: None, seq: 0, config: vec![], ssid: None })
    }

    fn roundtrip(p: &mut Provisioner<MemStore>, ftype: u8, payload: &[u8]) -> (u8, Vec<u8>) {
        let mut wire = [0u8; 1100];
        let n = encode(&mut wire, ftype, payload).unwrap();
        let mut frames = Vec::new();
        p.feed(&wire[..n], |resp| frames.push(resp.to_vec()));
        assert_eq!(frames.len(), 1, "expected one response frame");
        let mut out = None;
        Parser::new().feed(&frames[0], |t, pl| out = Some((t, pl.to_vec())));
        out.expect("response parses")
    }

    fn seal_config(owner: &env::SigningKey, kx_pub: &[u8; 32], seq: u64, cfg: &[u8]) -> Vec<u8> {
        let (mut inner, mut scratch) = ([0u8; 1024], [0u8; 1024]);
        let kid = owner.verifying_key().to_bytes();
        let ilen = env::sign1_write(&mut inner, &mut scratch, cfg, Some(&kid), owner).unwrap();
        let mut sealed = [0u8; 2048];
        let slen = env::seal_write(&mut sealed, &inner[..ilen], kx_pub, seq, &[0xd; 12], &[3; 32], &[5; 12]).unwrap();
        sealed[..slen].to_vec()
    }

    fn push(p: &mut Provisioner<MemStore>, blob: &[u8], seq: u32) -> (u8, Vec<u8>) {
        let mut begin = [0u8; 10];
        begin[..2].copy_from_slice(&(blob.len() as u16).to_le_bytes());
        begin[2..6].copy_from_slice(&seq.to_le_bytes());
        begin[6..10].copy_from_slice(&crc32(blob).to_le_bytes());
        let (t, _) = roundtrip(p, ftt::CONFIG_BEGIN, &begin);
        assert_eq!(t, ftt::OK);
        for (i, chunk) in blob.chunks(256).enumerate() {
            let mut frame = vec![0u8; 2 + chunk.len()];
            frame[..2].copy_from_slice(&((i * 256) as u16).to_le_bytes());
            frame[2..].copy_from_slice(chunk);
            let (t, _) = roundtrip(p, ftt::CONFIG_CHUNK, &frame);
            assert_eq!(t, ftt::OK);
        }
        roundtrip(p, ftt::CONFIG_COMMIT, &[])
    }

    #[test]
    fn identity_and_challenge() {
        let mut p = device();
        let (t, payload) = roundtrip(&mut p, ftt::IDENTITY_REQ, &[]);
        assert_eq!(t, ftt::IDENTITY);
        let mut scratch = vec![0u8; payload.len() + 64];
        let (doc, kid) =
            env::sign1_verify(&payload, &mut scratch, &p.id.sign.verifying_key()).unwrap();
        assert_eq!(kid, Some(&[0xd; 12][..]));
        assert!(!doc.is_empty());

        let (t, sig) = roundtrip(&mut p, ftt::CHALLENGE, &[9; 32]);
        assert_eq!((t, sig.len()), (ftt::CHALLENGE_SIG, 64));
        use ed25519_dalek::Verifier;
        let mut msg = b"ek-identify-v1".to_vec();
        msg.extend_from_slice(&[9; 32]);
        p.id.sign
            .verifying_key()
            .verify(&msg, &ed25519_dalek::Signature::from_slice(&sig).unwrap())
            .unwrap();
    }

    #[test]
    fn tofu_config_rollback_and_wrong_owner() {
        let mut p = device();
        let kx_pub = test_kx_pub(&[13; 32]);
        let owner = env::SigningKey::from_bytes(&[21; 32]);
        let cfg = br#"{"role":2,"keys":[],"slots":[]}"#;

        // fresh device adopts the first owner and acks
        let blob = seal_config(&owner, &kx_pub, 1, cfg);
        let (t, ack) = push(&mut p, &blob, 1);
        assert_eq!(t, ftt::CONFIG_ACK, "err code {:?}", ack.first());
        assert_eq!(p.store.owner, Some(owner.verifying_key().to_bytes()));
        assert_eq!(p.store.config, cfg);
        let mut scratch = vec![0u8; ack.len() + 64];
        let (ackp, _) = env::sign1_verify(&ack, &mut scratch, &p.id.sign.verifying_key()).unwrap();
        let (seq, hash) = schema::ack_decode(ackp).unwrap();
        assert_eq!(seq, 1);
        assert_eq!(hash, <[u8; 32]>::from(Sha256::digest(&blob)));

        // replay -> rollback
        let (t, e) = push(&mut p, &blob, 1);
        assert_eq!((t, e[0]), (ftt::ERROR, err_code::SEQ_ROLLBACK));

        // different owner -> wrong-set
        let evil = env::SigningKey::from_bytes(&[22; 32]);
        let blob2 = seal_config(&evil, &kx_pub, 2, cfg);
        let (t, e) = push(&mut p, &blob2, 2);
        assert_eq!((t, e[0]), (ftt::ERROR, err_code::WRONG_SET));

        // same owner, higher seq -> accepted
        let blob3 = seal_config(&owner, &kx_pub, 3, cfg);
        let (t, _) = push(&mut p, &blob3, 3);
        assert_eq!(t, ftt::CONFIG_ACK);
        assert_eq!(p.store.seq, 3);
    }

    #[test]
    fn events_and_wifi() {
        let mut p = device();
        let kx_pub = test_kx_pub(&[13; 32]);
        let owner = env::SigningKey::from_bytes(&[21; 32]);
        let blob = seal_config(&owner, &kx_pub, 1, b"{}");
        assert_eq!(push(&mut p, &blob, 1).0, ftt::CONFIG_ACK);

        let (t, batch) = roundtrip(&mut p, ftt::EVENTS_REQ, &[0, 0, 0, 0]);
        assert_eq!(t, ftt::EVENTS);
        let mut scratch = vec![0u8; batch.len() + 64];
        let (payload, _) = env::sign1_verify(&batch, &mut scratch, &p.id.sign.verifying_key()).unwrap();
        let evs: Vec<_> = schema::EventIter::new(payload).unwrap().map(|e| e.unwrap()).collect();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].kind, schema::EVT_CONFIG_ACK);

        let mut wifi = [0u8; 64];
        let mut e = Enc::new(&mut wifi);
        e.map(2).unwrap();
        e.uint(1).unwrap();
        e.tstr("prov-net").unwrap();
        e.uint(2).unwrap();
        e.tstr("secret").unwrap();
        let wlen = e.len();
        let (t, _) = roundtrip(&mut p, ftt::WIFI_SET, &wifi[..wlen]);
        assert_eq!(t, ftt::OK);
        assert_eq!(p.store.ssid.as_deref(), Some("prov-net"));

        let (t, status) = roundtrip(&mut p, ftt::WIFI_STATUS_REQ, &[]);
        assert_eq!(t, ftt::WIFI_STATUS);
        let mut d = Dec::new(&status);
        let n = d.map().unwrap();
        let mut ssid = "";
        for _ in 0..n {
            if d.uint().unwrap() == 2 {
                ssid = d.tstr().unwrap();
            } else {
                d.skip().unwrap();
            }
        }
        assert_eq!(ssid, "prov-net");
    }

    fn test_kx_pub(priv_key: &[u8; 32]) -> [u8; 32] {
        let secret = x25519_dalek::StaticSecret::from(*priv_key);
        *x25519_dalek::PublicKey::from(&secret).as_bytes()
    }
}
