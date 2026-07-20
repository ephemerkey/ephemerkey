//! Minimal NMEA 0183 parsing for the ephemerkey GNSS pipeline.
//!
//! Only what the firmware needs to discipline its RTC: the **RMC** sentence,
//! which carries UTC date + time and a fix-valid status. Checksum-verified, any
//! talker (GP/GN/GL/GA/…), no_std / no-alloc, and host-tested — the STM32 GNSS
//! task ([`crate::gnss`] in the firmware) feeds it line by line.
//!
//! RMC also carries position (lat/lon), which the firmware feeds to the
//! geofence gate. Fix-quality (satellites / HDOP, from GGA/GSA) is a later
//! addition; RMC alone gets the clock and the fence going.

#![cfg_attr(not(test), no_std)]

/// A parsed RMC fix: UTC wall-clock, position, and whether the receiver reports
/// a valid position fix (`status == 'A'`). Only trust the time and position for
/// disciplining the clock / testing the geofence when `valid` is true.
///
/// `lat_e7`/`lon_e7` are degrees × 10⁷ (north/east positive), matching
/// `ephemerkey_envelope::config::Zone`. They are `0` when the sentence carries
/// no position (a void fix), so gate them on `valid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RmcFix {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub min: u8,
    pub sec: u8,
    pub valid: bool,
    pub lat_e7: i32,
    pub lon_e7: i32,
}

/// Parse one NMEA RMC sentence (leading `$`, trailing `*HH` checksum, optional
/// CR/LF). Returns `None` unless it is a well-formed, checksum-valid RMC line
/// with parseable date and time. Talker-agnostic: matches `??RMC`.
pub fn parse_rmc(line: &[u8]) -> Option<RmcFix> {
    let line = trim(line);
    if line.first()? != &b'$' {
        return None;
    }
    let star = line.iter().position(|&b| b == b'*')?;
    let body = &line[..star]; // '$'..(exclusive of '*')
    let csum = &line[star + 1..];

    // Checksum: XOR of every byte between '$' and '*'.
    let want = hex2(csum)?;
    let got = body[1..].iter().fold(0u8, |a, &b| a ^ b);
    if got != want {
        return None;
    }

    let mut f = body.split(|&b| b == b',');
    let tag = f.next()?;
    if tag.len() < 6 || &tag[tag.len() - 3..] != b"RMC" {
        return None;
    }
    let time = f.next()?; // field 1: hhmmss(.sss)
    let status = f.next()?; // field 2: A/V
    let lat = f.next()?; // field 3: ddmm.mmmm
    let ns = f.next()?; // field 4: N/S
    let lon = f.next()?; // field 5: dddmm.mmmm
    let ew = f.next()?; // field 6: E/W
    let _spd = f.next()?;
    let _trk = f.next()?;
    let date = f.next()?; // field 9: ddmmyy

    if time.len() < 6 || date.len() < 6 {
        return None; // no fix yet: empty time/date
    }
    let hour = d2(&time[0..2])?;
    let min = d2(&time[2..4])?;
    let sec = d2(&time[4..6])?;
    if hour > 23 || min > 59 || sec > 60 {
        return None; // 60 allowed for a leap second
    }
    let day = d2(&date[0..2])?;
    let month = d2(&date[2..4])?;
    let yy = d2(&date[4..6])?;
    if !(1..=31).contains(&day) || !(1..=12).contains(&month) {
        return None;
    }

    // Position is optional (empty on a void fix): parse leniently, defaulting
    // to (0, 0). Callers only trust it when `valid`.
    let (lat_e7, lon_e7) = match (coord_e7(lat, ns), coord_e7(lon, ew)) {
        (Some(a), Some(o)) => (a, o),
        _ => (0, 0),
    };

    Some(RmcFix {
        year: 2000 + yy as u16,
        month,
        day,
        hour,
        min,
        sec,
        valid: status == b"A",
        lat_e7,
        lon_e7,
    })
}

/// Convert an NMEA coordinate (`ddmm.mmmm` / `dddmm.mmmm`) plus its hemisphere
/// (`N`/`S`/`E`/`W`) to degrees × 10⁷, sign by hemisphere. The last two integer
/// digits are whole minutes; everything before them is degrees. Returns `None`
/// for an empty/garbled field. All integer math (no FPU on the M0+).
fn coord_e7(field: &[u8], hemi: &[u8]) -> Option<i32> {
    if field.is_empty() {
        return None;
    }
    let dot = field.iter().position(|&b| b == b'.').unwrap_or(field.len());
    let int_part = &field[..dot];
    if int_part.len() < 3 {
        return None; // need at least d + mm
    }
    let (deg_s, min_s) = int_part.split_at(int_part.len() - 2);
    let deg = uint(deg_s)? as i64;
    let min_int = uint(min_s)? as i64;

    // Fractional minutes, accumulated to a fixed 10⁵ scale (minutes × 1e5).
    let mut min_frac = 0i64;
    if dot < field.len() {
        let mut scale = 100_000i64;
        for &c in &field[dot + 1..] {
            let d = digit(c)? as i64;
            scale /= 10;
            if scale == 0 {
                break; // ignore precision past 5 decimals
            }
            min_frac += d * scale;
        }
    }
    let minutes_scaled = min_int * 100_000 + min_frac; // minutes × 1e5

    // degrees×1e7 = deg×1e7 + minutes/60×1e7 = deg×1e7 + minutes_scaled×10/6.
    let e7 = deg * 10_000_000 + minutes_scaled * 10 / 6;
    let signed = match hemi {
        b"S" | b"W" => -e7,
        _ => e7,
    };
    Some(signed as i32)
}

/// Parse an all-ASCII-digit field as a `u32` (any length). `None` if empty or
/// non-digit.
fn uint(b: &[u8]) -> Option<u32> {
    if b.is_empty() {
        return None;
    }
    let mut v = 0u32;
    for &c in b {
        v = v.checked_mul(10)?.checked_add(digit(c)? as u32)?;
    }
    Some(v)
}

fn trim(mut s: &[u8]) -> &[u8] {
    while let Some((&c, rest)) = s.split_last() {
        if c == b'\r' || c == b'\n' || c == b' ' {
            s = rest;
        } else {
            break;
        }
    }
    s
}

fn d2(b: &[u8]) -> Option<u8> {
    if b.len() != 2 {
        return None;
    }
    let hi = digit(b[0])?;
    let lo = digit(b[1])?;
    Some(hi * 10 + lo)
}

fn digit(c: u8) -> Option<u8> {
    if c.is_ascii_digit() {
        Some(c - b'0')
    } else {
        None
    }
}

fn hex2(b: &[u8]) -> Option<u8> {
    if b.len() < 2 {
        return None;
    }
    Some(hexdigit(b[0])? << 4 | hexdigit(b[1])?)
}

fn hexdigit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_valid_fix() {
        // The canonical NMEA RMC example (checksum 6A).
        // 4807.038 N = 48° 07.038′ = 48.1173° → 481_173_000 e7.
        // 01131.000 E = 11° 31.000′ = 11.516666…° → 115_166_666 e7.
        let s = b"$GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W*6A";
        let f = parse_rmc(s).unwrap();
        assert_eq!(
            f,
            RmcFix {
                year: 2094,
                month: 3,
                day: 23,
                hour: 12,
                min: 35,
                sec: 19,
                valid: true,
                lat_e7: 481_173_000,
                lon_e7: 115_166_666,
            }
        );
    }

    #[test]
    fn gnss_talker_and_crlf_and_frac_seconds() {
        // $GNRMC with fractional seconds and a trailing CRLF (checksum computed
        // over the body between '$' and '*'). Southern & eastern hemisphere.
        let body = b"GNRMC,081836.00,A,3751.65,S,14507.36,E,000.0,360.0,130226,011.3,E";
        let cs = body.iter().fold(0u8, |a, &b| a ^ b);
        let mut line = alloc_line(body, cs);
        line.extend_from_slice(b"\r\n");
        let f = parse_rmc(&line).unwrap();
        assert_eq!(f.year, 2026);
        assert_eq!((f.month, f.day), (2, 13));
        assert_eq!((f.hour, f.min, f.sec), (8, 18, 36));
        assert!(f.valid);
        // 3751.65 S = 37° 51.65′ = 37.860833…° → −378_608_333 e7.
        // 14507.36 E = 145° 07.36′ = 145.122666…° → 1_451_226_666 e7.
        assert_eq!(f.lat_e7, -378_608_333);
        assert_eq!(f.lon_e7, 1_451_226_666);
    }

    #[test]
    fn void_status_parses_but_marks_invalid() {
        let body = b"GPRMC,235959,V,,,,,,,010100,,";
        let cs = body.iter().fold(0u8, |a, &b| a ^ b);
        let line = alloc_line(body, cs);
        let f = parse_rmc(&line).unwrap();
        assert!(!f.valid);
        assert_eq!((f.year, f.month, f.day), (2000, 1, 1));
        // No position in a void fix → (0, 0).
        assert_eq!((f.lat_e7, f.lon_e7), (0, 0));
    }

    #[test]
    fn bad_checksum_rejected() {
        let s = b"$GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W*00";
        assert_eq!(parse_rmc(s), None);
    }

    #[test]
    fn non_rmc_rejected() {
        let body = b"GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,";
        let cs = body.iter().fold(0u8, |a, &b| a ^ b);
        let line = alloc_line(body, cs);
        assert_eq!(parse_rmc(&line), None);
    }

    #[test]
    fn garbage_and_empty_rejected() {
        assert_eq!(parse_rmc(b""), None);
        assert_eq!(parse_rmc(b"not a sentence"), None);
        assert_eq!(parse_rmc(b"$GPRMC,,,,,,,,,,,*"), None);
    }

    // test helper: "$" + body + "*" + two-hex-uppercase checksum
    fn alloc_line(body: &[u8], cs: u8) -> Vec<u8> {
        let mut v = vec![b'$'];
        v.extend_from_slice(body);
        v.push(b'*');
        v.extend_from_slice(format!("{cs:02X}").as_bytes());
        v
    }
}
