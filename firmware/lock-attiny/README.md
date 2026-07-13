# ephemerkey lock board — ATtiny1616 firmware

Firmware for the companion "TOTP lock" board. The ATtiny1616 runs directly
off the cell (1.8–5.5 V, no LDO), authenticates codes over I²C via HMAC-SHA1
(shared with `smalltotp`), and drives the actuator with a peak-and-hold
economizer.

This tree implements the **authenticated I²C lock**: an HMAC-SHA1 challenge-
response that gates unlock/lock actuation, plus unauthenticated hall-sensor
status — running from power-down sleep, woken by the I²C bus.

## Hardware map (from `hardware/lock/`)

| Signal        | Pin  | Notes                                        |
|---------------|------|----------------------------------------------|
| Status LED    | PC3  | active-high: `PC3 → D1 → R1(1k) → GND`       |
| UPDI / RESET  | PA0  | `SYSCFG0=0xF6` — RESET pin is UPDI            |
| I²C SCL       | PB0  | TWI0 (target @ 0x60); START wakes from power-down |
| I²C SDA       | PB1  | TWI0                                          |
| HALL_PWR      | PA4  | powers both hall sensors during a read only  |
| HALL_DOOR     | PA7  | door sensor in (J6.3)                         |
| HALL_BOLT     | PB3  | bolt sensor in (J7.3)                         |
| Servo1 signal | PB2  | `TCA0/WO2 → R15(1k) → J5.1`, 50 Hz / 0.6–2.4 ms |
| Servo2 signal | PB4  | software pulse (TCA0 OVF+CMP1) → R24 → J8.1   |
| Servo power   | PA2  | `SERVO_PWR_EN → Q3` high-side (VSERVO)        |
| Solenoid drive| PA5  | `SOL_PWM → R5 → Q1` low-side; hold PWM = TCD0/WOB ~31 kHz |
| Boost enable  | PA6  | `SOL_BOOST_EN → MT3608 EN` (12 V for solenoid) |
| Boost 6/12 V  | PA1  | `BOOST_VSEL` select + servo interlock (Q5)    |

Fuses read factory-default; device is unlocked. See the bringup notes in the
repo for the full fuse dump.

### Servo power — servo runs on VSOL (default strap R13)

The servo supply (`VSERVO_SRC`) is strap-selected on the DRV sheet:
**`R13`(0Ω, fitted) → VSOL** (the boost rail, the default) or **`R14`(DNP) →
BAT+** (direct battery, alt). So the servo is on VSOL: at ~Vbat when the boost
is off (VSOL's passive L1+D2 path), or **6 V when boosted** (`servo_boost`
config flag → `SOL_BOOST_EN` on, `BOOST_VSEL` low = Q5 interlock clear).
`BOOST_VSEL` high (12 V) always interlocks servo power off.

(NB: an on-canvas note in `drv.kicad_sch` labels R13/R14 the other way round —
it's stale; R13→VSOL per the `lock.schgen.py` manifest and the built board.)

## Toolchain

- **Compiler:** `avr-gcc` / `avr-libc` (Fedora: `sudo dnf install avr-gcc
  avr-libc avr-binutils`). Needs avr-libc ≥ 2.0 for tinyAVR-1 headers.
- **Programmer:** [`pymcuprog`](https://pypi.org/project/pymcuprog/) SerialUPDI
  over an Adafruit UPDI Friend. Invoked via `uvx` — no system install needed.
- Adapter enumerates as a CH340 at `/dev/ttyUSB0` (override with `PORT=`).

The HMAC-SHA1 core is the **`smalltotp`** sibling repo (`src/sha1.c`,
`src/hmac_sha1.c`), linked — not vendored — exactly as the STM32 side does.
Point `SMALLTOTP` at your checkout:

## Build & flash

```sh
make SMALLTOTP=~/path/to/smalltotp        # compile + link + size
make flash SMALLTOTP=~/path/to/smalltotp  # erase, write, verify
make ping                                 # UPDI sig check (1E9421)
make fuses                                # dump fuses
make clean
```

Default is `SMALLTOTP ?= ../../../smalltotp` (a sibling of the repo). Select the
actuator with `-DLOCK_ACTUATOR=ACTUATOR_SERVO` (default) or `ACTUATOR_SOLENOID`.

## I²C protocol (target @ 0x60 — see `hardware/lock/README.md`)

| Reg | Access | Contents |
|-----|--------|----------|
| `0x00 STATUS`  | read  | bit0 DOOR_CLOSED · bit1 BOLT_LOCKED · bit2 SERVO_ON (live) · bit3 RAIL_12V · bit4 BUSY · bit5 LAST_CMD_OK · bit6 SOL_ON (live: coil energized, incl. drain) |
| `0x01 NONCE`   | read  | fresh 16-byte nonce; **reading arms it** (single-use) |
| `0x10 COMMAND` | write | `cmd(1) ‖ HMAC-SHA1(pairing_secret, nonce ‖ cmd)(20)`; `0x01`=UNLOCK, `0x02`=LOCK, `0x03`=ABORT (emergency stop: everything off) |
| `0x20 CONFIG`  | write | `blob(65) ‖ HMAC-SHA1(config_secret, nonce ‖ blob)(20)` · read = current blob |

Flow: read `NONCE` → write `COMMAND`/`CONFIG` → lock recomputes the HMAC,
**constant-time** compares, **burns the nonce** (replay-proof), then actuates or
saves config. Master re-reads `STATUS`. HMAC covers `nonce ‖ payload` only (no
TOTP code — the lock has no clock). UNLOCK/LOCK are **idempotent** (re-lock /
re-unlock is fine).

- **Two secrets, split across USERROW** (`SECRET_LEN`=16 each): pairing
  `[0:16]` (UNLOCK/LOCK) and config `[16:32]` (admin CONFIG writes). DEV
  fallbacks when USERROW is blank; provision over UPDI + set lockbits.
- **Anti-replay:** the armed nonce is single-use *and* derived from a monotonic
  EEPROM counter (survives resets/power loss; no TRNG):
  `nonce = HMAC-SHA1(pairing_secret, counter)[0:16]`. The counter provides
  freshness; keying with the pairing secret makes nonces opaque (an unkeyed
  hash would let one captured nonce brute-force the 2³² counter offline and
  predict all future nonces — no forgery, but a usage-count leak).
  Regenerating takes ~tens of ms (software HMAC at 3.3 MHz), so back-to-back
  NONCE reads can briefly serve the same value. Harmless: one armed slot,
  burned on first use, and regeneration is ordered before command servicing —
  a MAC verifies at most once (replay of the exact bytes is rejected; verified
  live).

### Programmable config (bit-packed, 65 bytes, persisted in EEPROM)

Actuation is a **programmable step sequence**: an ordered list of up to 6 steps
run on UNLOCK, and an independent list run on LOCK. Header + two sequences:

| Byte | Field | Encoding |
|---|---|---|
| 0 | magic (0xE4) | validity guard |
| 1 | flags | b0 servo_boost (servo-only steps at 6 V) |
| 2 | solenoid strike time | ×10 ms (full pull-in, 12 V economizer) |
| 3 | solenoid hold duty | 0–255 → 0–100 % (TCD0 economizer) |
| 4 | sensor_map | b0-1 DOOR_CLOSED src · b2-3 BOLT_LOCKED src (0=J6, 1=J7, 2=off) |
| 5–34  | `seq_unlock[6]` | 6 × step (5 bytes) |
| 35–64 | `seq_lock[6]`   | 6 × step (5 bytes) |

Each **step** is 5 bytes — `act`, `s1_pos`, `s2_pos`, `dur_ds`, `eoff`:

| Field | Encoding |
|---|---|
| `act` | b0 servo1 · b1 servo2 · b2 solenoid · **0 = end of sequence** |
| `s1_pos` / `s2_pos` | 8-bit target → 500–2500 µs (per step, either direction) |
| `dur_ds` | run time ×100 ms (servo drive / solenoid hold), 0–25.5 s |
| `eoff` | b0-1 sensor (0 none · 1 DOOR · 2 BOLT) · b2 edge (0 present / 1 absent) |

A step fires any combination of its actuators **together**, for `dur_ds`, then
the sequence advances. If `eoff` names a (logical) hall sensor, the step can
end early — heavily deglitched, because actuation disturbs the sensors (on the
bench the hall reads a *sustained* false-absent whenever actuators are loaded):

1. **Arm** — the step must first see the *opposite* state for 5 consecutive
   10 ms samples (e.g. `eoff=door-` requires the door seen **closed** for
   50 ms within the step). A sensor that's disturbed from the start, broken,
   or disconnected never arms — the step simply runs its full time.
2. **Fire** — an integrating counter over 10 ms samples: wanted state +1,
   opposite −5 (floor 0); fires at 50 (~500 ms clean dwell). A few transients
   only delay it; sustained opposite readings drain it.

Both layers fail toward "keep driving". Then the sequence advances. Per-step
**rail selection** (the boost makes VSOL, which also feeds the servo):

| Step drives | Rail | Solenoid |
|---|---|---|
| solenoid **+** servo | **6 V** (`BOOST_VSEL` low, interlock clear) | **full DC**, no PWM |
| solenoid only | **12 V** (`BOOST_VSEL` high) | strike → economizer PWM hold; servo interlocked out |
| servo + `servo_boost` | **6 V** | — |
| servo only | Vbat (boost off) | — |

> **Combined servo+solenoid** is the only way to move a servo while the solenoid
> fires: forced to **6 V** with the solenoid at **full DC** (no economizer PWM),
> so the shared rail can power both. Normally the two are electrically exclusive
> (12 V solenoid interlocks servo power off).

> **`servo_boost` (flag b0) — 6 V boosted servo.** Servo-only steps run off the
> boost rail at **6 V** (`SOL_BOOST_EN` on, `BOOST_VSEL` **low** = Q5 interlock
> clear), with a boost ramp before and a VSOL drain after. Requires the servo
> strapped to VSOL (R13, the default strap) and a 6 V servo. **Off by default; do
> not set it unless the board is wired for a boosted servo.**

### Non-blocking actuation

Actuation runs as a **TCB0-tick-driven step engine** (`actuate.c`), never
blocking the main loop: it walks the configured UNLOCK/LOCK step list, ramping
each step's rail, driving its actuators for the step time (or until early-off),
then draining — all while I²C stays fully responsive, and a new lock/unlock
**aborts** the in-flight cycle. Per-step rail selection + VSOL drain are
preserved. The LED mirrors BUSY.

> **Verification status:** protocol verified **live on hardware** via the
> RedBoard I²C bridge (`testharness/`) — unlock/lock accept with a valid HMAC,
> reject a bad one, nonces are fresh per command, hall status reads; soaked 40
> lock/unlock pairs with aggressive mid-cycle polling. Sleep is
> **IDLE** for bringup (`LOCK_SLEEP_MODE`): a TWI target in continuous
> POWER-DOWN both NACK-wedges the bus and makes UPDI unreachable, so power-down
> is deferred until the wake path is hardened. Every reset opens an ~8 s awake
> window (LED flutter) for reliable reprogramming. STM32 master I²C+HMAC is
> still unimplemented (it talks one-way over UART today — a known spec gap).

> **⚠️ I²C during actuation:** the boost soft-starting into stalled servos
> glitches SDA/SCL for ~0.4 s at actuation start (bus-error bursts on every
> cycle on the bench; dev-board wiring exaggerates it, but assume nonzero on
> any build). The lock target rides it out fine — but **the master must
> implement bus-error recovery** (peripheral re-init / 9-clock bus-clear +
> retry) or it can latch and go permanently deaf, as the RedBoard's ATmega TWI
> did. Full write-up + soak/regression tool: `testharness/README.md`.

## Roadmap

- [x] Blink + sleep (LED + flash + wake path)
- [x] Dual servo actuation on battery voltage (TCA0 PWM, both connectors)
- [x] Gated 12 V solenoid, peak-and-hold, boost↔servo mutual exclusion + drain
- [x] I²C target + HMAC-SHA1 challenge-response (unlock/lock) + hall status
- [x] Power-down sleep, wake on I²C
- [ ] Live protocol test against the STM32 master (needs STM32 I²C+HMAC side)
- [ ] STM32 master I²C driver: bus-error recovery (re-init / 9-clock bus-clear
      + retry) — actuation glitches the bus every cycle; see testharness/README
- [ ] Confirm hall-sensor output polarity against the real part
- [ ] Provision USERROW secret + set UPDI lockbits
- [ ] Fuse config: enable BOD level for actuator brown-out margin
