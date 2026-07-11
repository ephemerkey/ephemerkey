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
| `0x00 STATUS`  | read  | bit0 DOOR_CLOSED · bit1 BOLT_LOCKED · bit2 ACTUATOR (1=servo) · bit3 RAIL_12V · bit4 BUSY · bit5 LAST_CMD_OK |
| `0x01 NONCE`   | read  | fresh 16-byte nonce; **reading arms it** (single-use) |
| `0x10 COMMAND` | write | `cmd(1) ‖ HMAC-SHA1(pairing_secret, nonce ‖ cmd)(20)`; `0x01`=UNLOCK, `0x02`=LOCK |
| `0x20 CONFIG`  | write | `blob(9) ‖ HMAC-SHA1(config_secret, nonce ‖ blob)(20)` · read = current blob |

Flow: read `NONCE` → write `COMMAND`/`CONFIG` → lock recomputes the HMAC,
**constant-time** compares, **burns the nonce** (replay-proof), then actuates or
saves config. Master re-reads `STATUS`. HMAC covers `nonce ‖ payload` only (no
TOTP code — the lock has no clock). UNLOCK/LOCK are **idempotent** (re-lock /
re-unlock is fine).

- **Two secrets, split across USERROW** (`SECRET_LEN`=16 each): pairing
  `[0:16]` (UNLOCK/LOCK) and config `[16:32]` (admin CONFIG writes). DEV
  fallbacks when USERROW is blank; provision over UPDI + set lockbits.
- **Anti-replay:** the armed nonce is single-use *and* derived from a monotonic
  EEPROM counter (survives resets/power loss; no TRNG).

### Programmable config (bit-packed, 10 bytes, persisted in EEPROM)

| Byte | Field | Encoding |
|---|---|---|
| 0 | magic (0xE2) | validity guard |
| 1 | flags | b0 servo1_en · b1 servo2_en · b2 solenoid_en · b3 servo_boost |
| 2–5 | servo1/2 lock & unlock pos | 8-bit each → 500–2500 µs |
| 6 | servo drive time | ×10 ms (full-power servo drive) |
| 7 | solenoid strike time | ×10 ms (full pull-in) |
| 8 | solenoid hold time | ×100 ms (0 = none) |
| 9 | solenoid hold duty | 0–255 → 0–100 % (TCD0 economizer) |

Actuator selection + timing are runtime config, not compile-time. Servos get
**full power** for the drive time then release; the **duty cycle applies to the
solenoid** hold only. Servo drive and solenoid strike times are independent.

> **`servo_boost` (flag b3) — do NOT set on the current board.** It drives the
> servo phase from the boost rail (BOOST_VSEL + SOL_BOOST_EN, with a ramp before
> and a drain after) for higher-voltage servos. Today's hardware can't: BOOST_VSEL
> high engages the Q5 interlock that disables servo power, so this needs a
> boost/interlock hardware rev. Off by default; the exact VSEL level (6 V vs 12 V)
> is finalized with that hardware.

### Non-blocking actuation

Actuation runs as a **TCB0-tick-driven state machine** (`actuate.c`), never
blocking the main loop: the machine keeps the right rails powered then turns
them off while I²C stays fully responsive, and a new lock/unlock **aborts** the
in-flight cycle. Boost↔servo mutual-exclusion + VSOL drain are preserved. The
LED mirrors BUSY.

> **Verification status:** protocol verified **live on hardware** via the
> RedBoard I²C bridge (`testharness/`) — unlock/lock accept with a valid HMAC,
> reject a bad one, nonces are fresh per command, hall status reads. Sleep is
> **IDLE** for bringup (`LOCK_SLEEP_MODE`): a TWI target in continuous
> POWER-DOWN both NACK-wedges the bus and makes UPDI unreachable, so power-down
> is deferred until the wake path is hardened. Every reset opens an ~8 s awake
> window (LED flutter) for reliable reprogramming. STM32 master I²C+HMAC is
> still unimplemented (it talks one-way over UART today — a known spec gap).

## Roadmap

- [x] Blink + sleep (LED + flash + wake path)
- [x] Dual servo actuation on battery voltage (TCA0 PWM, both connectors)
- [x] Gated 12 V solenoid, peak-and-hold, boost↔servo mutual exclusion + drain
- [x] I²C target + HMAC-SHA1 challenge-response (unlock/lock) + hall status
- [x] Power-down sleep, wake on I²C
- [ ] Live protocol test against the STM32 master (needs STM32 I²C+HMAC side)
- [ ] Confirm hall-sensor output polarity against the real part
- [ ] Provision USERROW secret + set UPDI lockbits
- [ ] Fuse config: enable BOD level for actuator brown-out margin
