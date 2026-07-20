//! Lock-controller I/O loop (bench stub) over USB CDC-ACM.
//!
//! Holds the [`LockEngine`] built from the sealed config and drives it live: a
//! line typed at the CDC console is either a code (fed to
//! [`LockEngine::enter_code`], the [`Outcome`] logged over RTT and echoed back)
//! or a `T<unix>` command that disciplines the clock so bench codes validate
//! against a known time. On the product board the codes arrive over the LOCK
//! I2C bus and a fire drives the actuator; here the USB console stands in for
//! that edge while the validation engine is the shipping one.
//!
//! Enumerates only in lock-controller mode (never in provisioning mode — the
//! two are chosen at boot and share the USB peripheral).

use embassy_futures::join::join;
use embassy_stm32::usb::Driver;
use embassy_stm32::{peripherals, Peri};
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use ephemerkey_config::Calendars;
use ephemerkey_core::engine::{LockEngine, Outcome};
use ephemerkey_core::policy::Sensors;

use crate::clock;
use crate::usbcdc::{cdc, CdcBuffers, PACKET};
use crate::Irqs;

/// The gate environment the lock evaluates codes against. The calendar gate is
/// RTC-driven from the sealed config's windows; the lock's own geofence and the
/// stillness gate need sensors that aren't wired to this bench build yet, so
/// they read "open" (documented non-enforcement, unchanged from before).
struct LockEnv {
    calendars: Calendars,
    now: u64,
}

impl Sensors for LockEnv {
    fn inside_fence(&self, _fence: u8) -> bool {
        true // lock's own GNSS fence not wired on the bench
    }
    fn still_for_s(&self) -> u32 {
        u32::MAX // accelerometer not wired
    }
    fn calendar_open(&self, window: u8) -> bool {
        self.calendars.open(window, self.now)
    }
}

const LINE_MAX: usize = 24;

#[embassy_executor::task]
pub async fn task(
    usb: Peri<'static, peripherals::USB>,
    dp: Peri<'static, peripherals::PA12>,
    dm: Peri<'static, peripherals::PA11>,
    lock: LockEngine,
    calendars: Calendars,
) {
    let driver = Driver::new(usb, Irqs, dp, dm);
    let mut buffers = CdcBuffers::new();
    let mut state = State::new();
    let (mut device, mut class) = cdc(driver, "ephemerkey lock console", &mut buffers, &mut state);

    let mut lock = lock;
    let usb_fut = device.run();
    let console_fut = async {
        loop {
            class.wait_connection().await;
            defmt::info!("lock: console connected");
            let _ = console(&mut class, &mut lock, &calendars).await;
            defmt::info!("lock: console disconnected");
        }
    };
    join(usb_fut, console_fut).await;
}

/// Read CDC bytes into lines; dispatch each complete line.
async fn console<'d>(
    class: &mut CdcAcmClass<'d, Driver<'d, peripherals::USB>>,
    lock: &mut LockEngine,
    calendars: &Calendars,
) -> Result<(), EndpointError> {
    let mut line: heapless::Vec<u8, LINE_MAX> = heapless::Vec::new();
    let mut rx = [0u8; PACKET];
    loop {
        let n = class.read_packet(&mut rx).await?;
        for &b in &rx[..n] {
            if b == b'\r' || b == b'\n' {
                if !line.is_empty() {
                    let reply = handle_line(lock, calendars, &line);
                    class.write_packet(reply).await?;
                    line.clear();
                }
            } else if line.push(b).is_err() {
                line.clear(); // oversized — drop it
            }
        }
    }
}

/// A code line → `enter_code_with` (so the calendar gate is evaluated against
/// the RTC); a `T<unix>` line → discipline the clock.
fn handle_line(lock: &mut LockEngine, calendars: &Calendars, line: &[u8]) -> &'static [u8] {
    if let [b'T' | b't', rest @ ..] = line {
        return match parse_u64(rest) {
            Some(secs) => {
                clock::discipline_from_unix(secs);
                defmt::info!("lock: clock set to {}", secs);
                b"TIME OK\r\n"
            }
            None => b"TIME?\r\n",
        };
    }
    let Ok(code) = core::str::from_utf8(line) else {
        return b"UTF8?\r\n";
    };
    let now = clock::now_unix().unwrap_or(0);
    let env = LockEnv { calendars: *calendars, now };
    let out = lock.enter_code_with(code, now, &env);
    defmt::info!("lock: {} -> {}", code, out);
    label(&out)
}

fn label(o: &Outcome) -> &'static [u8] {
    match o {
        Outcome::Fired(..) => b"FIRE\r\n",
        Outcome::Progress(..) => b"PROG\r\n",
        Outcome::Reset(..) => b"RSET\r\n",
        Outcome::Beat(..) => b"BEAT\r\n",
        Outcome::Negative(..) => b"NEG!\r\n",
        Outcome::Gated(..) => b"GATE\r\n",
        Outcome::Armed(..) => b"ARMD\r\n",
        Outcome::Vetoed(..) => b"VETO\r\n",
        Outcome::Exhausted(..) => b"SPNT\r\n",
        Outcome::LockedOut(..) => b"LOUT\r\n",
        Outcome::Replay(..) => b"RPLY\r\n",
        Outcome::Invalid => b"----\r\n",
    }
}

fn parse_u64(b: &[u8]) -> Option<u64> {
    let b = b.strip_prefix(b" ").unwrap_or(b);
    if b.is_empty() {
        return None;
    }
    let mut v = 0u64;
    for &c in b {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add((c - b'0') as u64)?;
    }
    Some(v)
}
