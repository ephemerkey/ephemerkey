//! `ekemu serial` — emulate an ephemerkey in provisioning mode on a TCP
//! socket, speaking the framed protocol (ephemerkey-frame) with the real
//! envelope crypto (ephemerkey-envelope). This is the stand-in device for
//! ephemerkey-control's courier flow until firmware `provision.rs` lands:
//! same frames, same TOFU rules, same acks.
//!
//!   ekemu serial <state.json> [listen_addr]     (default 127.0.0.1:8422)
//!
//! State (device identity, owner binding, config, events) persists in the
//! JSON file; delete it for a factory-fresh device.

use ephemerkey_envelope as env;
use ephemerkey_envelope::cbor::Enc;
use ephemerkey_envelope::schema;
use ephemerkey_frame::{encode, err_code, frame_type as ft, Parser};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;

#[derive(Serialize, Deserialize, Default)]
struct State {
    device_id: String,
    sign_priv: String,
    kx_priv: String,
    fw: String,
    owner_pub: Option<String>, // TOFU: bound by the first accepted config
    seq: u64,
    config_b64: Option<String>,
    event_seq: u64,
    events: Vec<(u64, u64, u64)>, // (seq, ts, kind)
    wifi_ssid: Option<String>,
    wifi_psk: Option<String>,
}

fn rand32() -> [u8; 32] {
    let mut b = [0u8; 32];
    getrandom(&mut b);
    b
}

fn getrandom(buf: &mut [u8]) {
    // std-only emulator: /dev/urandom keeps us dependency-free
    use std::io::Read as _;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(buf))
        .expect("urandom");
}

fn load_or_init(path: &str) -> State {
    if let Ok(text) = std::fs::read_to_string(path) {
        return serde_json::from_str(&text).expect("state file parse");
    }
    let mut id = [0u8; 12];
    getrandom(&mut id);
    let st = State {
        device_id: hex(&id),
        sign_priv: hex(&rand32()),
        kx_priv: hex(&rand32()),
        fw: "ekemu-0.1".into(),
        ..Default::default()
    };
    st
}

fn save(path: &str, st: &State) {
    std::fs::write(path, serde_json::to_string_pretty(st).unwrap()).expect("state write");
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

struct Device {
    st: State,
    path: String,
    sign_key: env::SigningKey,
    // in-flight config transfer
    xfer: Option<Xfer>,
}

struct Xfer {
    buf: Vec<u8>,
    filled: usize,
    seq_hint: u32,
    crc32: u32,
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

impl Device {
    fn new(path: String) -> Self {
        let st = load_or_init(&path);
        let sign_key = env::SigningKey::from_bytes(
            unhex(&st.sign_priv).as_slice().try_into().expect("sign_priv"),
        );
        save(&path, &st);
        Device { st, path, sign_key, xfer: None }
    }

    fn record_event(&mut self, kind: u64) {
        self.st.event_seq += 1;
        self.st.events.push((self.st.event_seq, now(), kind));
    }

    /// Handle one frame; returns (frame_type, payload) to send back.
    fn handle(&mut self, ftype: u8, payload: &[u8]) -> (u8, Vec<u8>) {
        match ftype {
            ft::IDENTITY_REQ => self.identity(),
            ft::CHALLENGE => {
                use ed25519_dalek::Signer;
                let mut msg = b"ek-identify-v1".to_vec();
                msg.extend_from_slice(payload);
                (ft::CHALLENGE_SIG, self.sign_key.sign(&msg).to_bytes().to_vec())
            }
            ft::CONFIG_BEGIN => {
                if payload.len() != 10 {
                    return err(err_code::BAD_STATE);
                }
                let total = u16::from_le_bytes([payload[0], payload[1]]) as usize;
                let seq_hint = u32::from_le_bytes(payload[2..6].try_into().unwrap());
                let crc = u32::from_le_bytes(payload[6..10].try_into().unwrap());
                if total == 0 || total > 4096 {
                    return err(err_code::BAD_STATE);
                }
                self.xfer = Some(Xfer { buf: vec![0; total], filled: 0, seq_hint, crc32: crc });
                (ft::OK, vec![])
            }
            ft::CONFIG_CHUNK => {
                let Some(x) = self.xfer.as_mut() else { return err(err_code::BAD_STATE) };
                if payload.len() < 2 {
                    return err(err_code::BAD_STATE);
                }
                let off = u16::from_le_bytes([payload[0], payload[1]]) as usize;
                let data = &payload[2..];
                if off + data.len() > x.buf.len() {
                    return err(err_code::BAD_STATE);
                }
                x.buf[off..off + data.len()].copy_from_slice(data);
                x.filled += data.len();
                (ft::OK, vec![])
            }
            ft::CONFIG_COMMIT => self.commit(),
            ft::EVENTS_REQ => self.events(payload),
            ft::WIFI_SET => self.wifi_set(payload),
            ft::WIFI_STATUS_REQ => {
                let mut buf = [0u8; 128];
                let mut e = Enc::new(&mut buf);
                let ssid = self.st.wifi_ssid.clone().unwrap_or_default();
                e.map(2).unwrap();
                e.uint(1).unwrap(); // connected: 0/1 (emulator never connects)
                e.uint(0).unwrap();
                e.uint(2).unwrap();
                e.tstr(&ssid).unwrap();
                let n = e.len();
                (ft::WIFI_STATUS, buf[..n].to_vec())
            }
            _ => err(err_code::BAD_STATE),
        }
    }

    fn identity(&mut self) -> (u8, Vec<u8>) {
        let device_id = unhex(&self.st.device_id);
        let sign_pub = self.sign_key.verifying_key().to_bytes();
        let kx_priv: [u8; 32] = unhex(&self.st.kx_priv).try_into().unwrap();
        let kx_pub = x25519_pub(&kx_priv);

        let mut payload = [0u8; 256];
        let mut e = Enc::new(&mut payload);
        e.map(4).unwrap();
        e.uint(1).unwrap();
        e.bstr(&device_id).unwrap();
        e.uint(2).unwrap();
        e.bstr(&sign_pub).unwrap();
        e.uint(3).unwrap();
        e.bstr(&kx_pub).unwrap();
        e.uint(4).unwrap();
        e.tstr(&self.st.fw).unwrap();
        let plen = e.len();

        let (mut out, mut scratch) = ([0u8; 512], [0u8; 512]);
        let n = env::sign1_write(&mut out, &mut scratch, &payload[..plen], Some(&device_id), &self.sign_key)
            .expect("identity sign");
        (ft::IDENTITY, out[..n].to_vec())
    }

    fn commit(&mut self) -> (u8, Vec<u8>) {
        let Some(x) = self.xfer.take() else { return err(err_code::BAD_STATE) };
        if x.filled != x.buf.len() || crc32(&x.buf) != x.crc32 {
            return err(err_code::CRC);
        }
        let kx_priv: [u8; 32] = unhex(&self.st.kx_priv).try_into().unwrap();

        let mut inner = vec![0u8; x.buf.len()];
        let (ilen, seq) = match env::open(&x.buf, &mut inner, &kx_priv) {
            Ok(v) => v,
            Err(_) => return err(err_code::BAD_SIG),
        };
        if seq != x.seq_hint as u64 {
            return err(err_code::BAD_STATE);
        }
        if seq <= self.st.seq && self.st.owner_pub.is_some() {
            return err(err_code::SEQ_ROLLBACK);
        }

        // Owner binding: the inner Sign1 carries owner_pub as its kid.
        // TOFU — a factory-fresh device adopts the first owner it hears;
        // a bound device rejects any other (physical re-provisioning =
        // delete the state file).
        let (_, kid, _, _) = match env::sign1_parse(&inner[..ilen]) {
            Ok(v) => v,
            Err(_) => return err(err_code::BAD_SIG),
        };
        let Some(kid) = kid else { return err(err_code::WRONG_SET) };
        let owner_hex = hex(kid);
        match &self.st.owner_pub {
            Some(bound) if *bound != owner_hex => return err(err_code::WRONG_SET),
            _ => {}
        }
        let key_bytes: [u8; 32] = match kid.try_into() {
            Ok(k) => k,
            Err(_) => return err(err_code::WRONG_SET),
        };
        let Ok(owner_key) = env::VerifyingKey::from_bytes(&key_bytes) else {
            return err(err_code::WRONG_SET);
        };
        let mut scratch = vec![0u8; ilen + 64];
        let config = match env::sign1_verify(&inner[..ilen], &mut scratch, &owner_key) {
            Ok((payload, _)) => payload.to_vec(),
            Err(_) => return err(err_code::BAD_SIG),
        };

        self.st.owner_pub = Some(owner_hex);
        self.st.seq = seq;
        self.st.config_b64 = Some(b64(&config));
        self.record_event(schema::EVT_CONFIG_ACK);
        save(&self.path, &self.st);

        let hash: [u8; 32] = Sha256::digest(&x.buf).into();
        let mut payload = [0u8; 64];
        let n = schema::ack_encode(&mut payload, seq, &hash).unwrap();
        let (mut out, mut scratch) = ([0u8; 256], [0u8; 256]);
        let device_id = unhex(&self.st.device_id);
        let alen = env::sign1_write(&mut out, &mut scratch, &payload[..n], Some(&device_id), &self.sign_key)
            .expect("ack sign");
        (ft::CONFIG_ACK, out[..alen].to_vec())
    }

    fn events(&mut self, payload: &[u8]) -> (u8, Vec<u8>) {
        let after = if payload.len() >= 4 {
            u32::from_le_bytes(payload[..4].try_into().unwrap()) as u64
        } else {
            0
        };
        let evs: Vec<_> = self.st.events.iter().filter(|(s, _, _)| *s > after).collect();
        let mut buf = vec![0u8; 64 + evs.len() * 32];
        let mut e = Enc::new(&mut buf);
        e.array(evs.len() as u64).unwrap();
        for (seq, ts, kind) in &evs {
            schema::event_encode(
                &mut e,
                &schema::Event { seq: *seq, rtc_ts: *ts, kind: *kind, detail: None, chain_tag: None },
            )
            .unwrap();
        }
        let blen = e.len();
        let device_id = unhex(&self.st.device_id);
        let mut out = vec![0u8; blen + 192];
        let mut scratch = vec![0u8; blen + 64];
        let n = env::sign1_write(&mut out, &mut scratch, &buf[..blen], Some(&device_id), &self.sign_key)
            .expect("events sign");
        (ft::EVENTS, out[..n].to_vec())
    }

    fn wifi_set(&mut self, payload: &[u8]) -> (u8, Vec<u8>) {
        // CBOR {1: ssid tstr, 2: psk tstr}; empty ssid clears.
        let mut d = env::cbor::Dec::new(payload);
        let Ok(n) = d.map() else { return err(err_code::BAD_STATE) };
        let (mut ssid, mut psk) = (None, None);
        for _ in 0..n {
            match d.uint() {
                Ok(1) => ssid = d.tstr().ok().map(String::from),
                Ok(2) => psk = d.tstr().ok().map(String::from),
                _ => {
                    if d.skip().is_err() {
                        return err(err_code::BAD_STATE);
                    }
                }
            }
        }
        match ssid {
            Some(s) if s.is_empty() => {
                self.st.wifi_ssid = None;
                self.st.wifi_psk = None;
            }
            Some(s) => {
                self.st.wifi_ssid = Some(s);
                self.st.wifi_psk = psk;
            }
            None => return err(err_code::BAD_STATE),
        }
        save(&self.path, &self.st);
        (ft::OK, vec![])
    }
}

fn err(code: u8) -> (u8, Vec<u8>) {
    (ft::ERROR, vec![code])
}

fn b64(data: &[u8]) -> String {
    // tiny local base64 (std has none); emulator-only convenience
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let v = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((v >> (18 - 6 * i)) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn x25519_pub(priv_key: &[u8; 32]) -> [u8; 32] {
    let secret = x25519_dalek::StaticSecret::from(*priv_key);
    *x25519_dalek::PublicKey::from(&secret).as_bytes()
}

pub fn run(state_path: &str, listen: &str) {
    let mut dev = Device::new(state_path.to_string());
    eprintln!(
        "ekemu serial: device {} listening on {} (state: {})",
        dev.st.device_id, listen, state_path
    );
    let listener = TcpListener::bind(listen).expect("bind");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        eprintln!("ekemu serial: host connected");
        let mut parser = Parser::new();
        let mut buf = [0u8; 512];
        loop {
            let n = match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let mut responses: Vec<(u8, Vec<u8>)> = Vec::new();
            parser.feed(&buf[..n], |t, p| responses.push((t, p.to_vec())));
            for (t, p) in responses {
                let (rt, rp) = dev.handle(t, &p);
                let mut out = vec![0u8; rp.len() + 16];
                let len = encode(&mut out, rt, &rp).expect("encode");
                if stream.write_all(&out[..len]).is_err() {
                    break;
                }
            }
        }
        eprintln!("ekemu serial: host disconnected");
    }
}
