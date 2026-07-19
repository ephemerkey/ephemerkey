//! Payload schemas carried inside Sign1 envelopes (DESIGN-management.md):
//! the config-ack a device mints after applying a config, and telemetry
//! event batches. Shared by firmware (encode) and server/console (decode).

use crate::cbor::{Dec, Enc};
use crate::Error;

pub const EVT_UNLOCK: u64 = 1;
pub const EVT_LOCK: u64 = 2;
pub const EVT_DURESS: u64 = 3;
pub const EVT_TAMPER: u64 = 4;
pub const EVT_FENCE_ENTER: u64 = 5;
pub const EVT_FENCE_EXIT: u64 = 6;
pub const EVT_POWER: u64 = 7;
pub const EVT_CONFIG_ACK: u64 = 8;

/// config-ack payload: `{1: seq, 2: sha256(sealed blob as delivered)}`.
pub fn ack_encode(buf: &mut [u8], seq: u64, blob_hash: &[u8; 32]) -> Result<usize, Error> {
    let mut e = Enc::new(buf);
    e.map(2)?;
    e.uint(1)?;
    e.uint(seq)?;
    e.uint(2)?;
    e.bstr(blob_hash)?;
    Ok(e.len())
}

pub fn ack_decode(payload: &[u8]) -> Result<(u64, [u8; 32]), Error> {
    let mut d = Dec::new(payload);
    let n = d.map()?;
    let (mut seq, mut hash) = (None, None);
    for _ in 0..n {
        match d.uint()? {
            1 => seq = Some(d.uint()?),
            2 => hash = Some(d.bstr()?),
            _ => d.skip()?,
        }
    }
    let hash: [u8; 32] = hash
        .ok_or(Error::Malformed)?
        .try_into()
        .map_err(|_| Error::Malformed)?;
    Ok((seq.ok_or(Error::Malformed)?, hash))
}

/// Enrollment doc payload (the serial IDENTITY frame's Sign1 body):
/// `{1: device_id, 2: sign_pub, 3: kx_pub, 4: fw}`.
pub struct Enrollment<'a> {
    pub device_id: &'a [u8],
    pub sign_pub: &'a [u8],
    pub kx_pub: &'a [u8],
    pub fw: &'a str,
}

pub fn enrollment_decode(payload: &[u8]) -> Result<Enrollment<'_>, Error> {
    let mut d = Dec::new(payload);
    let n = d.map()?;
    let (mut device_id, mut sign_pub, mut kx_pub, mut fw) = (None, None, None, "");
    for _ in 0..n {
        match d.uint()? {
            1 => device_id = Some(d.bstr()?),
            2 => sign_pub = Some(d.bstr()?),
            3 => kx_pub = Some(d.bstr()?),
            4 => fw = d.tstr()?,
            _ => d.skip()?,
        }
    }
    match (device_id, sign_pub, kx_pub) {
        (Some(device_id), Some(sign_pub), Some(kx_pub)) => {
            Ok(Enrollment { device_id, sign_pub, kx_pub, fw })
        }
        _ => Err(Error::Malformed),
    }
}

/// One telemetry event: `{1: seq, 3: rtc_ts, 4: type, 5: detail?, 6: chain_tag?}`.
#[derive(Debug, Clone, Copy)]
pub struct Event<'a> {
    pub seq: u64,
    pub rtc_ts: u64,
    pub kind: u64,
    pub detail: Option<&'a [u8]>,
    pub chain_tag: Option<&'a [u8]>,
}

pub fn event_encode(e: &mut Enc, ev: &Event) -> Result<(), Error> {
    let extra = ev.detail.is_some() as u64 + ev.chain_tag.is_some() as u64;
    e.map(3 + extra)?;
    e.uint(1)?;
    e.uint(ev.seq)?;
    e.uint(3)?;
    e.uint(ev.rtc_ts)?;
    e.uint(4)?;
    e.uint(ev.kind)?;
    if let Some(d) = ev.detail {
        e.uint(5)?;
        e.bstr(d)?;
    }
    if let Some(t) = ev.chain_tag {
        e.uint(6)?;
        e.bstr(t)?;
    }
    Ok(())
}

/// Iterate a batch payload (CBOR array of event maps).
pub struct EventIter<'a> {
    dec: Dec<'a>,
    remaining: u64,
}

impl<'a> EventIter<'a> {
    pub fn new(payload: &'a [u8]) -> Result<Self, Error> {
        let mut dec = Dec::new(payload);
        let remaining = dec.array()?;
        Ok(EventIter { dec, remaining })
    }
}

impl<'a> Iterator for EventIter<'a> {
    type Item = Result<Event<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        Some(self.read_one())
    }
}

impl<'a> EventIter<'a> {
    fn read_one(&mut self) -> Result<Event<'a>, Error> {
        let n = self.dec.map()?;
        let mut ev = Event { seq: 0, rtc_ts: 0, kind: 0, detail: None, chain_tag: None };
        let (mut got_seq, mut got_kind) = (false, false);
        for _ in 0..n {
            match self.dec.uint()? {
                1 => {
                    ev.seq = self.dec.uint()?;
                    got_seq = true;
                }
                3 => ev.rtc_ts = self.dec.uint()?,
                4 => {
                    ev.kind = self.dec.uint()?;
                    got_kind = true;
                }
                5 => ev.detail = Some(self.dec.bstr()?),
                6 => ev.chain_tag = Some(self.dec.bstr()?),
                _ => self.dec.skip()?,
            }
        }
        if got_seq && got_kind {
            Ok(ev)
        } else {
            Err(Error::Malformed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_roundtrip() {
        let mut buf = [0u8; 64];
        let n = ack_encode(&mut buf, 9, &[0xab; 32]).unwrap();
        assert_eq!(ack_decode(&buf[..n]).unwrap(), (9, [0xab; 32]));
    }

    #[test]
    fn event_batch_roundtrip() {
        let mut buf = [0u8; 256];
        let mut e = Enc::new(&mut buf);
        e.array(2).unwrap();
        event_encode(&mut e, &Event { seq: 1, rtc_ts: 1000, kind: EVT_UNLOCK, detail: None, chain_tag: Some(&[1; 20]) }).unwrap();
        event_encode(&mut e, &Event { seq: 2, rtc_ts: 1060, kind: EVT_LOCK, detail: Some(b"x"), chain_tag: None }).unwrap();
        let len = e.len();

        let evs: Vec<_> = EventIter::new(&buf[..len]).unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(evs.len(), 2);
        assert_eq!((evs[0].seq, evs[0].kind), (1, EVT_UNLOCK));
        assert_eq!(evs[1].detail, Some(&b"x"[..]));
    }
}
