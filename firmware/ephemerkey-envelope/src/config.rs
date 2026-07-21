//! Geofence types + the sealed config's shared low-level pieces.
//!
//! This module owns the geofence [`Zone`] (with the FPU-free membership test),
//! the [`Role`] enum, the top-level constants, and [`parse_zone`] — the one CBOR
//! zone decoder. The FULL config parser (role, staleness, zones, and the whole
//! policy schema → engine) lives in `ephemerkey-config`, which reuses these; a
//! device never parses the config here.
//!
//! Coordinates are degrees × 10⁷ (`lat_e7`/`lon_e7`, signed); `radius_m` is the
//! fence radius in metres.

use crate::cbor::Dec;
use crate::Error;

/// Most geofence zones a single config may carry (fixed RAM footprint).
pub const MAX_ZONES: usize = 8;

/// Default emission-freshness window when the config omits its staleness key:
/// 3× the 30 s TOTP period. On this device the GNSS is powered on-demand
/// (button press), so freshness is measured from the *last acquired fix*.
pub const DEFAULT_STALENESS_S: u32 = 90;

/// Largest fence radius accepted (1000 km). Bounds the squared-distance
/// arithmetic in [`Zone::contains`] well inside `i64`; real fences are metres
/// to a few km. Oversized radii are clamped, not rejected.
pub const MAX_RADIUS_M: u32 = 1_000_000;

/// The device personality selected by the sealed config.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Role {
    /// GNSS-geofenced TOTP code generator.
    Generator,
    /// TOTP receiver driving the lock board.
    LockController,
}

/// A single circular geofence: a centre (degrees × 10⁷) and a radius in metres.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Zone {
    pub lat_e7: i32,
    pub lon_e7: i32,
    pub radius_m: u32,
}

impl Zone {
    /// Is the point (`lat_e7`, `lon_e7`) inside this fence?
    ///
    /// Uses an equirectangular projection: at geofence scale (radii up to tens
    /// of km) its error versus the great-circle haversine is well under a
    /// metre, and — unlike haversine — it needs no `sqrt`/`atan` on the
    /// FPU-less Cortex-M0+. We compare *squared* distances, so there is no
    /// `sqrt` at all; the only transcendental is `cos(latitude)` for the
    /// east-west metres-per-degree, evaluated in fixed point ([`cos_lat_q15`]).
    pub fn contains(&self, lat_e7: i32, lon_e7: i32) -> bool {
        let dlat_e7 = lat_e7 as i64 - self.lat_e7 as i64;
        let dlon_e7 = lon_e7 as i64 - self.lon_e7 as i64;

        // Metres-per-degree ≈ 111_320. Work in millimetres (×1000):
        //   mm = e7 · 111_320 / 10_000
        let dlat_mm = dlat_e7 * 111_320 / 10_000;
        // East-west distance shrinks by cos(latitude).
        let cos_q15 = cos_lat_q15(self.lat_e7) as i64; // 0..=32768
        let dlon_mm = (dlon_e7 * 111_320 / 10_000) * cos_q15 / 32_768;

        let r_mm = self.radius_m as i64 * 1000;
        // Bounding-box reject first: keeps the squares from overflowing i64 for
        // a far-away fix (dlon_mm can reach ~10^10 before this guard).
        if dlat_mm.abs() > r_mm || dlon_mm.abs() > r_mm {
            return false;
        }
        dlat_mm * dlat_mm + dlon_mm * dlon_mm <= r_mm * r_mm
    }
}

/// Decode one CBOR zone entry — an array `[lat_e7, lon_e7, radius_m]`, extra
/// trailing fields ignored — into a [`Zone`], clamping the radius to
/// [`MAX_RADIUS_M`]. The single zone decoder, shared by `ephemerkey-config`'s
/// full-config parser.
pub fn parse_zone(d: &mut Dec) -> Result<Zone, Error> {
    let fields = d.array()?;
    if fields < 3 {
        return Err(Error::Malformed);
    }
    let lat_e7 = clamp_i32(d.int()?);
    let lon_e7 = clamp_i32(d.int()?);
    let radius_m = (d.uint()? as u32).min(MAX_RADIUS_M);
    for _ in 3..fields {
        d.skip()?; // a newer server may add optional per-zone fields
    }
    Ok(Zone { lat_e7, lon_e7, radius_m })
}

fn clamp_i32(v: i64) -> i32 {
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// `cos(latitude)` in Q15 (0..=32768), via Bhaskara I's sine approximation in
/// degrees (max error ≈ 0.0016 — sub-metre at fence scale). `cos θ = sin(90°−θ)`
/// and cosine is even, so only `|θ|` matters.
fn cos_lat_q15(lat_e7: i32) -> i32 {
    // |lat| in millidegrees, clamped to the valid [0°, 90°].
    let lat_md = ((lat_e7 as i64).abs() / 10_000).min(90_000);
    // Argument to sin, in millidegrees: x = 90° − |lat|, in [0, 90_000].
    let x = 90_000 - lat_md;
    // sin(x°) ≈ 4·x(180−x) / (40500 − x(180−x)). In millidegrees (X = 1000·x):
    //   P = X(180_000 − X);  sin = 4P / (40_500·10^6 − P).
    let p = x * (180_000 - x); // ≤ 8.1e9
    let num = 4 * p * 32_768; // ≤ ~1.06e15
    let den = 40_500_000_000 - p; // ≥ 3.24e10 > 0
    (num / den) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cbor::Enc;

    #[test]
    fn geofence_inside_and_outside() {
        // Fence centred on ~Zürich HB, 500 m radius.
        let z = Zone { lat_e7: 473_766_000, lon_e7: 85_417_000, radius_m: 500 };

        // Dead centre: inside.
        assert!(z.contains(z.lat_e7, z.lon_e7));
        // ~200 m north (1° lat ≈ 111_320 m ⇒ 200 m ≈ 17_966 e7): inside.
        assert!(z.contains(z.lat_e7 + 17_966, z.lon_e7));
        // ~2 km north (≈ 179_660 e7): outside.
        assert!(!z.contains(z.lat_e7 + 179_660, z.lon_e7));
        // ~2 km east: outside, and this exercises the cos(lat) scaling — at
        // 47.4°, 1° lon ≈ 75 km, so 2 km ≈ 265_500 e7. Without the cos term
        // this same offset would read as only ~1.35 km and wrongly pass.
        assert!(!z.contains(z.lat_e7, z.lon_e7 + 265_500));
        // ~300 m east (≈ 39_800 e7): inside the 500 m fence.
        assert!(z.contains(z.lat_e7, z.lon_e7 + 39_800));
    }

    #[test]
    fn cos_latitude_is_reasonable() {
        // Q15: cos(0°)=1, cos(60°)=0.5, cos(90°)=0.
        assert!((cos_lat_q15(0) - 32_768).abs() <= 60);
        assert!((cos_lat_q15(600_000_000) - 16_384).abs() <= 60);
        assert!(cos_lat_q15(900_000_000).abs() <= 60);
        // Even function.
        assert_eq!(cos_lat_q15(473_766_000), cos_lat_q15(-473_766_000));
    }

    #[test]
    fn parse_zone_clamps_and_skips_extras() {
        let mut buf = [0u8; 64];
        let mut e = Enc::new(&mut buf);
        // A 4-field zone (extra trailing field) with an oversized radius.
        e.array(4).unwrap();
        e.int(473_766_000).unwrap();
        e.int(85_417_000).unwrap();
        e.uint(9_000_000).unwrap(); // > MAX_RADIUS_M
        e.tstr("name").unwrap(); // extra field — must be skipped
        let n = e.len();

        let mut d = Dec::new(&buf[..n]);
        let z = parse_zone(&mut d).unwrap();
        assert_eq!(z.lat_e7, 473_766_000);
        assert_eq!(z.lon_e7, 85_417_000);
        assert_eq!(z.radius_m, MAX_RADIUS_M);

        // Too few fields is malformed.
        let mut buf = [0u8; 16];
        let mut e = Enc::new(&mut buf);
        e.array(2).unwrap();
        e.int(1).unwrap();
        e.int(2).unwrap();
        let n = e.len();
        let mut d = Dec::new(&buf[..n]);
        assert_eq!(parse_zone(&mut d), Err(Error::Malformed));
    }
}
