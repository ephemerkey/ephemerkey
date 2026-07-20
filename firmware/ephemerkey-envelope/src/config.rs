//! Device configuration: the integer-keyed CBOR schema the sealed config
//! carries, plus the geofence membership test it drives.
//!
//! The provisioning path delivers a `COSE_Encrypt0(COSE_Sign1(config))` blob;
//! once [`crate::open`] + [`crate::sign1_verify`] have peeled it, the innermost
//! `config` bytes are this map. It is deliberately small and forward-compatible
//! (unknown keys are skipped), no_std / no-alloc — the whole thing decodes into
//! a fixed-size [`DeviceConfig`] with no borrows of the input.
//!
//! ## Wire schema (CBOR map, unsigned integer keys)
//!
//! | key | field         | type                                   | default |
//! |-----|---------------|----------------------------------------|---------|
//! | 1   | `role`        | uint: 1 = Generator, 2 = LockController | (required) |
//! | 2   | `staleness_s` | uint: max age of the last GNSS fix before codes are refused | 90 |
//! | 3   | `zones`       | array of `[lat_e7, lon_e7, radius_m]`  | (empty) |
//!
//! Coordinates are degrees × 10⁷ (`lat_e7`/`lon_e7`, signed); `radius_m` is the
//! fence radius in metres. Extra trailing fields in a zone entry are ignored,
//! and zones past [`MAX_ZONES`] are dropped, so a newer server can add optional
//! per-zone data without breaking an older device.
//!
//! Anything a config *depends on* for a security promise still travels in the
//! COSE `crit`-style feature list checked by `Store::validate_config` on the
//! device — this module is the layout, not the policy.

use crate::cbor::Dec;
use crate::Error;

/// Most geofence zones a single config may carry (fixed RAM footprint).
pub const MAX_ZONES: usize = 8;

/// Default emission-freshness window when the config omits key 2: 3× the 30 s
/// TOTP period. On this device the GNSS is powered on-demand (button press),
/// so freshness is measured from the *last acquired fix*, not a continuous PPS.
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

/// A fully-parsed device configuration. Fixed size, no borrows of the source
/// bytes — safe to copy into a `static` or hold across the config buffer being
/// reused.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceConfig {
    pub role: Role,
    /// Max age (seconds) of the last GNSS fix before emission is refused.
    pub staleness_s: u32,
    zones: [Zone; MAX_ZONES],
    zone_count: usize,
}

impl DeviceConfig {
    /// The "shut" default: a Generator with no fences, so [`in_any_fence`] is
    /// always false until a real config is provisioned. Used on a
    /// factory-fresh device and as the fail-safe when parsing fails.
    ///
    /// [`in_any_fence`]: DeviceConfig::in_any_fence
    pub const fn shut_default() -> Self {
        DeviceConfig {
            role: Role::Generator,
            staleness_s: DEFAULT_STALENESS_S,
            zones: [Zone { lat_e7: 0, lon_e7: 0, radius_m: 0 }; MAX_ZONES],
            zone_count: 0,
        }
    }

    /// The configured geofence zones.
    pub fn zones(&self) -> &[Zone] {
        &self.zones[..self.zone_count]
    }

    /// Whether a point lies inside *any* configured fence. With no zones this
    /// is always false — an unprovisioned generator can never emit (DESIGN:
    /// "inside an authorized geofence").
    pub fn in_any_fence(&self, lat_e7: i32, lon_e7: i32) -> bool {
        self.zones().iter().any(|z| z.contains(lat_e7, lon_e7))
    }
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self::shut_default()
    }
}

/// Parse the integer-keyed CBOR config map. Unknown keys are skipped; the
/// `role` key (1) is required. Returns [`Error::Malformed`] on a bad shape or
/// an unrecognised role.
pub fn parse(bytes: &[u8]) -> Result<DeviceConfig, Error> {
    let mut d = Dec::new(bytes);
    let n = d.map()?;
    let mut cfg = DeviceConfig::shut_default();
    let mut have_role = false;

    for _ in 0..n {
        match d.uint()? {
            1 => {
                cfg.role = match d.uint()? {
                    1 => Role::Generator,
                    2 => Role::LockController,
                    _ => return Err(Error::Malformed),
                };
                have_role = true;
            }
            2 => cfg.staleness_s = d.uint()? as u32,
            3 => {
                let zn = d.array()?;
                cfg.zone_count = 0;
                for _ in 0..zn {
                    let fields = d.array()?;
                    if fields < 3 {
                        return Err(Error::Malformed);
                    }
                    let lat_e7 = clamp_i32(d.int()?);
                    let lon_e7 = clamp_i32(d.int()?);
                    let radius_m = (d.uint()? as u32).min(MAX_RADIUS_M);
                    // Ignore any extra per-zone fields a newer server added.
                    for _ in 3..fields {
                        d.skip()?;
                    }
                    if cfg.zone_count < MAX_ZONES {
                        cfg.zones[cfg.zone_count] = Zone { lat_e7, lon_e7, radius_m };
                        cfg.zone_count += 1;
                    }
                }
            }
            _ => d.skip()?,
        }
    }

    if !have_role {
        return Err(Error::Malformed);
    }
    Ok(cfg)
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

    /// Encode a config map with role (key 1), optional staleness (key 2), and
    /// zones (key 3) as `[lat_e7, lon_e7, radius_m]` triples.
    fn cfg_bytes(role: u64, staleness: Option<u64>, zones: &[(i64, i64, u64)]) -> Vec<u8> {
        let mut buf = [0u8; 512];
        let mut e = Enc::new(&mut buf);
        let entries = 1 + staleness.is_some() as u64 + !zones.is_empty() as u64;
        e.map(entries).unwrap();
        e.uint(1).unwrap();
        e.uint(role).unwrap();
        if let Some(s) = staleness {
            e.uint(2).unwrap();
            e.uint(s).unwrap();
        }
        if !zones.is_empty() {
            e.uint(3).unwrap();
            e.array(zones.len() as u64).unwrap();
            for &(lat, lon, r) in zones {
                e.array(3).unwrap();
                e.int(lat).unwrap();
                e.int(lon).unwrap();
                e.uint(r).unwrap();
            }
        }
        let n = e.len();
        buf[..n].to_vec()
    }

    #[test]
    fn minimal_generator_defaults() {
        let cfg = parse(&cfg_bytes(1, None, &[])).unwrap();
        assert_eq!(cfg.role, Role::Generator);
        assert_eq!(cfg.staleness_s, DEFAULT_STALENESS_S);
        assert!(cfg.zones().is_empty());
        // No fences => never inside one.
        assert!(!cfg.in_any_fence(481_173_000, 115_166_666));
    }

    #[test]
    fn lock_controller_role_and_staleness() {
        let cfg = parse(&cfg_bytes(2, Some(300), &[])).unwrap();
        assert_eq!(cfg.role, Role::LockController);
        assert_eq!(cfg.staleness_s, 300);
    }

    #[test]
    fn unknown_role_and_missing_role_rejected() {
        assert_eq!(parse(&cfg_bytes(9, None, &[])), Err(Error::Malformed));
        // A map with only key 2 (no role).
        let mut buf = [0u8; 16];
        let mut e = Enc::new(&mut buf);
        e.map(1).unwrap();
        e.uint(2).unwrap();
        e.uint(90).unwrap();
        let n = e.len();
        assert_eq!(parse(&buf[..n]), Err(Error::Malformed));
    }

    #[test]
    fn geofence_inside_and_outside() {
        // Fence centred on ~Zürich HB, 500 m radius.
        let (lat, lon, r) = (473_766_000_i64, 85_417_000_i64, 500);
        let cfg = parse(&cfg_bytes(1, None, &[(lat, lon, r)])).unwrap();

        // Dead centre: inside.
        assert!(cfg.in_any_fence(lat as i32, lon as i32));

        // ~200 m north. 1° lat ≈ 111_320 m ⇒ 200 m ≈ 17_966 e7: inside.
        assert!(cfg.in_any_fence((lat + 17_966) as i32, lon as i32));

        // ~2 km north (≈ 179_660 e7): outside.
        assert!(!cfg.in_any_fence((lat + 179_660) as i32, lon as i32));

        // ~2 km east: outside, and this exercises the cos(lat) scaling — at
        // 47.4°, 1° lon ≈ 75 km, so 2 km ≈ 265_500 e7. Without the cos term
        // this same offset would read as only ~1.35 km and wrongly pass.
        assert!(!cfg.in_any_fence(lat as i32, (lon + 265_500) as i32));
        // ~300 m east (≈ 39_800 e7): inside the 500 m fence.
        assert!(cfg.in_any_fence(lat as i32, (lon + 39_800) as i32));
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
    fn radius_clamped_and_zone_overflow_dropped() {
        // Oversized radius is clamped, not rejected.
        let cfg = parse(&cfg_bytes(1, None, &[(0, 0, 9_000_000)])).unwrap();
        assert_eq!(cfg.zones()[0].radius_m, MAX_RADIUS_M);

        // More than MAX_ZONES zones: extras are dropped, parse still succeeds.
        let many: Vec<_> = (0..(MAX_ZONES as i64 + 4)).map(|i| (i * 1_000_000, 0, 100)).collect();
        let cfg = parse(&cfg_bytes(1, None, &many)).unwrap();
        assert_eq!(cfg.zones().len(), MAX_ZONES);
    }

    #[test]
    fn unknown_keys_skipped() {
        // Hand-roll a map with an unknown key 7 between the known ones.
        let mut buf = [0u8; 64];
        let mut e = Enc::new(&mut buf);
        e.map(3).unwrap();
        e.uint(1).unwrap();
        e.uint(1).unwrap(); // role
        e.uint(7).unwrap();
        e.tstr("future").unwrap(); // unknown -> skipped
        e.uint(2).unwrap();
        e.uint(120).unwrap(); // staleness
        let n = e.len();
        let cfg = parse(&buf[..n]).unwrap();
        assert_eq!(cfg.role, Role::Generator);
        assert_eq!(cfg.staleness_s, 120);
    }
}
