//! RFC 4226 HOTP / RFC 6238 TOTP with configurable code length (4-10).
//!
//! Dynamic truncation yields 31 bits, so entropy saturates just above nine
//! digits — ten is representable but adds nothing (DESIGN-policies.md).

use hmac::{Hmac, Mac};
use sha1::Sha1;

/// TOTP period (RFC 6238 default). Counter = unix / PERIOD_S.
pub const PERIOD_S: u32 = 30;

pub const MIN_DIGITS: u8 = 4;
pub const MAX_DIGITS: u8 = 10;

/// A code as entered/displayed: value plus its (leading-zero-padded) length.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Code {
    pub value: u32,
    pub digits: u8,
}

impl Code {
    /// Parse an entered code. Length must equal `digits` exactly (leading
    /// zeros are significant).
    pub fn parse(s: &str, digits: u8) -> Option<Code> {
        if s.len() != digits as usize || !(MIN_DIGITS..=MAX_DIGITS).contains(&digits) {
            return None;
        }
        let mut v: u64 = 0;
        for b in s.bytes() {
            if !b.is_ascii_digit() {
                return None;
            }
            v = v * 10 + (b - b'0') as u64;
        }
        Some(Code {
            value: v as u32, // 10 digits of truncated HOTP always fit u32
            digits,
        })
    }

    /// Render into a fixed buffer, zero-padded; returns the used slice.
    pub fn render<'a>(&self, buf: &'a mut [u8; 10]) -> &'a str {
        let mut v = self.value;
        let n = self.digits as usize;
        for i in (0..n).rev() {
            buf[i] = b'0' + (v % 10) as u8;
            v /= 10;
        }
        core::str::from_utf8(&buf[..n]).unwrap()
    }
}

/// RFC 4226 HOTP: HMAC-SHA1, dynamic truncation, mod 10^digits.
pub fn hotp(secret: &[u8], counter: u64, digits: u8) -> Code {
    let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(secret).expect("hmac any key len");
    mac.update(&counter.to_be_bytes());
    let d = mac.finalize().into_bytes();
    let off = (d[19] & 0x0f) as usize;
    let dt = (u32::from(d[off] & 0x7f) << 24)
        | (u32::from(d[off + 1]) << 16)
        | (u32::from(d[off + 2]) << 8)
        | u32::from(d[off + 3]);
    Code {
        value: (u64::from(dt) % 10u64.pow(u32::from(digits))) as u32,
        digits,
    }
}

/// TOTP counter for a unix time.
pub fn counter_at(unix: u64) -> u32 {
    (unix / u64::from(PERIOD_S)) as u32
}

/// TOTP code minted at `unix`.
pub fn totp_at(secret: &[u8], unix: u64, digits: u8) -> Code {
    hotp(secret, u64::from(counter_at(unix)), digits)
}

#[cfg(test)]
mod tests {
    use super::*;
    const SECRET: &[u8] = b"12345678901234567890";

    #[test]
    fn rfc4226_hotp_vectors() {
        let want = [
            755224, 287082, 359152, 969429, 338314, 254676, 287922, 162583, 399871, 520489,
        ];
        for (c, w) in want.iter().enumerate() {
            assert_eq!(hotp(SECRET, c as u64, 6).value, *w, "counter {c}");
        }
    }

    #[test]
    fn rfc6238_totp_sha1_vectors() {
        // (time, 8-digit code) from RFC 6238 Appendix B, SHA1 rows
        let want = [
            (59u64, 94287082u32),
            (1111111109, 7081804),
            (1111111111, 14050471),
            (1234567890, 89005924),
            (2000000000, 69279037),
            (20000000000, 65353130),
        ];
        for (t, w) in want {
            assert_eq!(totp_at(SECRET, t, 8).value, w, "t={t}");
        }
    }

    #[test]
    fn parse_leading_zeros_and_length() {
        assert_eq!(
            Code::parse("07081804", 8),
            Some(Code {
                value: 7081804,
                digits: 8
            })
        );
        assert_eq!(Code::parse("7081804", 8), None); // wrong length
        assert_eq!(
            Code::parse("0708", 4),
            Some(Code {
                value: 708,
                digits: 4
            })
        );
        assert_eq!(Code::parse("12a4", 4), None);
    }

    #[test]
    fn render_roundtrip() {
        let mut buf = [0u8; 10];
        assert_eq!(
            Code {
                value: 708,
                digits: 4
            }
            .render(&mut buf),
            "0708"
        );
    }
}
