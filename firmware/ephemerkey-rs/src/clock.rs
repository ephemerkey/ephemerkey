//! Wall-clock / TOTP time base on the STM32 RTC.
//!
//! The RTC (LSE on the product board, LSI on the Nucleo — chosen by the RCC
//! `ls` config in `main`) is the TOTP time base: it keeps running while the
//! GNSS is powered down and, later, across Stop mode. It is **disciplined** by
//! a trusted UTC source — the GNSS, via [`discipline_from_unix`] (the 1 PPS
//! sub-second capture is a later refinement) — and untrusted until then.
//!
//! Two things the rest of the firmware needs from here:
//!   - [`now_unix`] — current UTC as unix seconds, or `None` until disciplined
//!     (so event timestamps and TOTP never run off a cold, wrong clock).
//!   - [`is_fresh`] — has the clock been disciplined recently enough? This is
//!     the anti-replay staleness gate (DESIGN.md §Security): a frozen or
//!     rolled-back clock must not yield valid codes.
//!
//! The `RtcTimeProvider` returned by `Rtc::new` is the only public reader, so
//! it (and the `Rtc` itself, for `set_datetime`) live in a critical-section
//! static that `now_unix`/`discipline_from_unix` reach from any context.

use core::cell::RefCell;

use embassy_stm32::peripherals::RTC;
use embassy_stm32::rtc::{DateTime, DayOfWeek, Rtc, RtcConfig, RtcTimeProvider};
use embassy_stm32::Peri;
use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex};
use embassy_time::{Duration, Instant};

struct Clock {
    rtc: Rtc,
    reader: RtcTimeProvider,
    /// Monotonic instant of the last successful discipline; `None` = never.
    last_sync: Option<Instant>,
}

static CLOCK: Mutex<CriticalSectionRawMutex, RefCell<Option<Clock>>> =
    Mutex::new(RefCell::new(None));

/// Bring up the RTC. Call once, early in `main`, after `embassy_stm32::init`
/// (which applies the `ls` clock-source selection). Idempotent-safe: a second
/// call replaces the reader but does not reset the counter.
pub fn init(rtc: Peri<'static, RTC>) {
    let (rtc, reader) = Rtc::new(rtc, RtcConfig::default());
    CLOCK.lock(|c| {
        *c.borrow_mut() = Some(Clock {
            rtc,
            reader,
            last_sync: None,
        });
    });
}

/// Set the wall clock from a trusted UTC source (GNSS) and mark it fresh.
/// Out-of-range inputs are ignored.
pub fn discipline_from_unix(secs: u64) {
    let Some(dt) = unix_to_datetime(secs) else {
        return;
    };
    CLOCK.lock(|c| {
        if let Some(clk) = c.borrow_mut().as_mut() {
            if clk.rtc.set_datetime(dt).is_ok() {
                clk.last_sync = Some(Instant::now());
            }
        }
    });
}

/// Discipline the clock from a UTC calendar time (e.g. parsed from a GNSS RMC
/// sentence). Ignores obviously-out-of-range fields.
pub fn discipline_utc(year: u16, month: u8, day: u8, hour: u8, min: u8, sec: u8) {
    if !(2000..=2099).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return;
    }
    let days = days_from_civil(year as i64, month as i64, day as i64);
    let secs = days * 86_400 + hour as i64 * 3_600 + min as i64 * 60 + sec as i64;
    if secs >= 0 {
        discipline_from_unix(secs as u64);
    }
}

/// Current UTC as unix seconds, or `None` if the RTC has never been
/// disciplined (a cold clock has no trustworthy time).
pub fn now_unix() -> Option<u64> {
    CLOCK.lock(|c| {
        let b = c.borrow();
        let clk = b.as_ref()?;
        clk.last_sync?; // undisciplined -> no time
        let dt = clk.reader.now().ok()?;
        Some(datetime_to_unix(&dt))
    })
}

/// Whether the clock was disciplined within `max_age`. The anti-replay gate:
/// reject TOTP generation when this is false (stale/rolled clock).
pub fn is_fresh(max_age: Duration) -> bool {
    CLOCK.lock(|c| {
        c.borrow()
            .as_ref()
            .and_then(|clk| clk.last_sync)
            .map(|t| t.elapsed() <= max_age)
            .unwrap_or(false)
    })
}

// ---- calendar <-> unix seconds (Howard Hinnant's civil algorithms) ---------

fn datetime_to_unix(dt: &DateTime) -> u64 {
    let days = days_from_civil(dt.year() as i64, dt.month() as i64, dt.day() as i64);
    let secs = days * 86_400
        + dt.hour() as i64 * 3_600
        + dt.minute() as i64 * 60
        + dt.second() as i64;
    secs.max(0) as u64
}

fn unix_to_datetime(secs: u64) -> Option<DateTime> {
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    // The STM32 RTC stores a two-digit year offset from 2000.
    if !(2000..=2099).contains(&y) {
        return None;
    }
    let (hour, minute, second) = (
        (rem / 3_600) as u8,
        ((rem % 3_600) / 60) as u8,
        (rem % 60) as u8,
    );
    DateTime::from(
        y as u16,
        m as u8,
        d as u8,
        weekday(days),
        hour,
        minute,
        second,
        0,
    )
    .ok()
}

/// Days since 1970-01-01 for a proleptic-Gregorian y/m/d.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`]: (year, month, day).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Day of week for a day-count since the epoch (1970-01-01 was a Thursday).
fn weekday(days: i64) -> DayOfWeek {
    match ((days % 7) + 4).rem_euclid(7) {
        0 => DayOfWeek::Sunday,
        1 => DayOfWeek::Monday,
        2 => DayOfWeek::Tuesday,
        3 => DayOfWeek::Wednesday,
        4 => DayOfWeek::Thursday,
        5 => DayOfWeek::Friday,
        _ => DayOfWeek::Saturday,
    }
}
