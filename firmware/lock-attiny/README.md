# ephemerkey lock board — ATtiny1616 firmware

Firmware for the companion "TOTP lock" board. The ATtiny1616 runs directly
off the cell (1.8–5.5 V, no LDO), authenticates codes over I²C via HMAC-SHA1
(shared with `smalltotp`), and drives the actuator with a peak-and-hold
economizer.

This tree currently holds **bringup stage 1**: a low-power heartbeat blink.

## Hardware map (from `hardware/lock/`)

| Signal        | Pin  | Notes                                        |
|---------------|------|----------------------------------------------|
| Status LED    | PC3  | active-high: `PC3 → D1 → R1(1k) → GND`       |
| UPDI / RESET  | PA0  | `SYSCFG0=0xF6` — RESET pin is UPDI            |
| Servo1 signal | PB2  | `TCA0/WO2 → R15(1k) → J5.1`, 50 Hz / 0.6–2.4 ms |
| Servo2 signal | PB4  | software pulse (TCA0 OVF+CMP1) → R24 → J8.1   |
| Servo power   | PA2  | `SERVO_PWR_EN → Q3` high-side (VSERVO)        |
| Solenoid drive| PA5  | `SOL_PWM → R5 → Q1` low-side; hold PWM = TCD0/WOB ~31 kHz |
| Boost enable  | PA6  | `SOL_BOOST_EN → MT3608 EN` (12 V for solenoid) |
| Boost 6/12 V  | PA1  | `BOOST_VSEL` select + servo interlock (Q5)    |

Fuses read factory-default; device is unlocked. See the bringup notes in the
repo for the full fuse dump.

### Servo power — runs on battery voltage, not the boost

The servo supply is strap-selected: `R13`(0Ω, fitted) → VSOL, or `R14`(DNP)
→ VCC/VSYS. With the MT3608 **disabled**, its L1+D2 Schottky path passes Vin
through, so VSOL settles at ~Vbat−0.3 V — either strap yields ~Vbat at the
servo as long as the boost stays off. Firmware **never** asserts
`SOL_BOOST_EN`, and holds `BOOST_VSEL` low (also required — Q5 interlocks servo
power off whenever `BOOST_VSEL` is high / 12 V mode).

## Toolchain

- **Compiler:** `avr-gcc` / `avr-libc` (Fedora: `sudo dnf install avr-gcc
  avr-libc avr-binutils`). Needs avr-libc ≥ 2.0 for tinyAVR-1 headers.
- **Programmer:** [`pymcuprog`](https://pypi.org/project/pymcuprog/) SerialUPDI
  over an Adafruit UPDI Friend. Invoked via `uvx` — no system install needed.
- Adapter enumerates as a CH340 at `/dev/ttyUSB0` (override with `PORT=`).

## Build & flash

```sh
make            # compile + link + size
make ping       # confirm the chip answers over UPDI (sig 1E9421)
make fuses      # dump fuses
make flash      # erase, write, verify build/lock-attiny.hex
make clean
```

## Behavior (bringup demo cycle)

The LED (PC3) blinks at 1 Hz throughout via the RTC PIT as an "alive" beacon.
The main loop then runs one actuation cycle, forever:

1. **Servo** — both servos sweep on battery voltage (boost off, VSOL≈Vbat).
2. Servo power off.
3. **Boost** — MT3608 enabled to +12 V; 500 ms to ramp and charge C5.
4. **Solenoid** — coil driven ~10 s: full-power strike (50 ms) then 50 % hold.
5. **Drain** — boost disabled *with the solenoid still conducting*, so the 12 V
   on VSOL bleeds through the coil back to ~Vbat; only then is the coil
   released.

**The boost and the servo supply are never energised together** — enforced by
this ordering *and* the hardware Q5 interlock. The drain step is mandatory:
VSOL is the servo's supply, so it must be back at Vbat before the servo runs
again (never feed 12 V into a Vbat servo).

## Roadmap

- [x] Blink + sleep (LED + flash + wake path)
- [x] Dual servo actuation on battery voltage (TCA0 PWM, both connectors)
- [x] Gated 12 V solenoid, peak-and-hold, boost↔servo mutual exclusion + drain
- [ ] Fuse config: enable BOD level for actuator brown-out margin
- [ ] I²C target + HMAC-SHA1 auth (link `smalltotp`)
- [ ] Trigger actuation from an authenticated I²C unlock (replace the demo loop)
- [ ] Return to STANDBY sleep between actuations for battery life
