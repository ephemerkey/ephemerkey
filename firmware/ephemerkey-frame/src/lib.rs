//! Serial framing for the ephemerkey provisioning link (USB-CDC / emulator
//! TCP): `'E' 'K' ver type len:u16le payload crc16:u16le`, CRC-16/CCITT-FALSE
//! over `ver..payload`. Contract: ephemerkey-control `docs/serial-protocol.md`;
//! host mirror `web/src/lib/serial.ts`. The framing is plumbing — devices
//! trust only the COSE envelope carried inside CONFIG frames.

#![cfg_attr(not(test), no_std)]

pub const VERSION: u8 = 0x01;
pub const MAX_PAYLOAD: usize = 1024;
const MAGIC: [u8; 2] = *b"EK";

pub mod frame_type {
    pub const IDENTITY_REQ: u8 = 0x01;
    pub const IDENTITY: u8 = 0x02;
    pub const CHALLENGE: u8 = 0x03;
    pub const CHALLENGE_SIG: u8 = 0x04;
    pub const CONFIG_BEGIN: u8 = 0x10;
    pub const CONFIG_CHUNK: u8 = 0x11;
    pub const CONFIG_COMMIT: u8 = 0x12;
    pub const CONFIG_ACK: u8 = 0x13;
    pub const EVENTS_REQ: u8 = 0x30;
    pub const EVENTS: u8 = 0x31;
    pub const WIFI_SET: u8 = 0x40;
    pub const WIFI_STATUS_REQ: u8 = 0x41;
    pub const WIFI_STATUS: u8 = 0x42;
    pub const OK: u8 = 0x7e;
    pub const ERROR: u8 = 0x7f;
}

pub mod err_code {
    pub const BAD_STATE: u8 = 1;
    pub const BAD_SIG: u8 = 2;
    pub const SEQ_ROLLBACK: u8 = 3;
    pub const WRONG_SET: u8 = 4;
    pub const STORAGE: u8 = 5;
    pub const CRC: u8 = 6;
}

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xffff;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

/// Encode one frame into `out`; returns the encoded length.
pub fn encode(out: &mut [u8], ftype: u8, payload: &[u8]) -> Option<usize> {
    if payload.len() > MAX_PAYLOAD {
        return None;
    }
    let total = 2 + 4 + payload.len() + 2;
    if out.len() < total {
        return None;
    }
    out[..2].copy_from_slice(&MAGIC);
    out[2] = VERSION;
    out[3] = ftype;
    out[4..6].copy_from_slice(&(payload.len() as u16).to_le_bytes());
    out[6..6 + payload.len()].copy_from_slice(payload);
    let crc = crc16(&out[2..6 + payload.len()]);
    out[6 + payload.len()..total].copy_from_slice(&crc.to_le_bytes());
    Some(total)
}

/// Incremental scanner: feed raw bytes, get complete valid frames via the
/// callback. Skips non-frame noise (boot logs) and drops bad-CRC frames.
pub struct Parser {
    buf: [u8; 2 + 4 + MAX_PAYLOAD + 2],
    len: usize,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub const fn new() -> Self {
        Parser { buf: [0; 2 + 4 + MAX_PAYLOAD + 2], len: 0 }
    }

    pub fn feed(&mut self, chunk: &[u8], mut on_frame: impl FnMut(u8, &[u8])) {
        for &b in chunk {
            if self.len == self.buf.len() {
                // full buffer without a frame: drop the front byte
                self.buf.copy_within(1.., 0);
                self.len -= 1;
            }
            self.buf[self.len] = b;
            self.len += 1;
            self.scan(&mut on_frame);
        }
    }

    fn scan(&mut self, on_frame: &mut impl FnMut(u8, &[u8])) {
        loop {
            // hunt for magic
            let mut start = 0;
            while start + 1 < self.len
                && !(self.buf[start] == MAGIC[0] && self.buf[start + 1] == MAGIC[1])
            {
                start += 1;
            }
            if start > 0 {
                self.buf.copy_within(start..self.len, 0);
                self.len -= start;
            }
            if self.len < 8 {
                return;
            }
            let plen = u16::from_le_bytes([self.buf[4], self.buf[5]]) as usize;
            if plen > MAX_PAYLOAD || self.buf[2] != VERSION {
                // false sync: shift past this magic and re-hunt
                self.buf.copy_within(2..self.len, 0);
                self.len -= 2;
                continue;
            }
            let total = 2 + 4 + plen + 2;
            if self.len < total {
                return;
            }
            let want = u16::from_le_bytes([self.buf[total - 2], self.buf[total - 1]]);
            if crc16(&self.buf[2..6 + plen]) == want {
                on_frame(self.buf[3], &self.buf[6..6 + plen]);
                self.buf.copy_within(total..self.len, 0);
                self.len -= total;
            } else {
                self.buf.copy_within(2..self.len, 0);
                self.len -= 2;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_noise() {
        let mut wire = Vec::new();
        wire.extend_from_slice(b"boot log noise\n");
        let mut f = [0u8; 64];
        let n = encode(&mut f, frame_type::CHALLENGE, &[7; 32]).unwrap();
        wire.extend_from_slice(&f[..n]);
        wire.extend_from_slice(b"EK"); // stray magic
        let n2 = encode(&mut f, frame_type::OK, &[]).unwrap();
        wire.extend_from_slice(&f[..n2]);

        let mut got = Vec::new();
        let mut p = Parser::new();
        // feed byte-at-a-time to exercise the incremental path
        for b in wire {
            p.feed(&[b], |t, pl| got.push((t, pl.to_vec())));
        }
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], (frame_type::CHALLENGE, vec![7; 32]));
        assert_eq!(got[1], (frame_type::OK, vec![]));
    }

    #[test]
    fn bad_crc_dropped() {
        let mut f = [0u8; 64];
        let n = encode(&mut f, frame_type::OK, b"hi").unwrap();
        f[7] ^= 1; // corrupt payload
        let mut count = 0;
        let mut p = Parser::new();
        p.feed(&f[..n], |_, _| count += 1);
        assert_eq!(count, 0);
    }

    #[test]
    fn crc16_known_vector() {
        // CRC-16/CCITT-FALSE("123456789") = 0x29B1
        assert_eq!(crc16(b"123456789"), 0x29b1);
    }
}
