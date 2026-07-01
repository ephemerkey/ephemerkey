# lock — companion TOTP lock board

The **consumer** half of the ephemerkey system, and the **second PCB in this
repo** (alongside `hardware/ephemerkey/`, sharing the same `scripts/kschgen.py`
engine, `Makefile`, and lib tables — the multi-project layout used by
`reefvolt-sensorbuddy`). ephemerkey *generates* an authenticated unlock; this
board *verifies* it and drives the actuator.

```
make gen-lock      # regenerate the schematic from scripts/lock.schgen.py (placed, not wired)
make check-lock    # component/footprint/dup-ref/ERC tally
make render-lock   # per-sheet PNGs
```

The schematic is **generated from a manifest** (`scripts/lock.schgen.py`), like
ephemerkey. Components are **placed, not wired** — each sheet carries an
on-canvas note that is the wiring spec for the eeschema phase. Regenerate
*before* wiring (regen reassigns UUIDs).

## What it does

1. Sleeps at ~0.1 µA (ATtiny1616 power-down). Coil de-energized, 12 V rail off.
2. ephemerkey starts an I2C transaction; the first START wakes the MCU (wake-on-I2C).
3. **Authenticated** challenge-response over I2C proves the request is
   genuine (HMAC-SHA1, shared secret in flash — see *Firmware plan*).
4. On success: enable the boost, run the **peak-and-hold** solenoid profile,
   then drop the rail and go back to sleep.

Everything (MCU + boost + driver) is **powered from ephemerkey over the I2C
connector** (J2.2 = VSYS, ephemerkey's ~3.0–4.7 V battery/system rail). The lock
carries **no local battery** (J1 removed). See the *current caveat* under the
interface section — a 12 V solenoid pull-in draws several amps through that cable.

## Architecture

```
 VSYS from ephemerkey ──┬─────────────────────────────► U1 ATtiny1616 VCC   (direct, NO LDO; ~0.1µA sleep)
   (J2.2, I2C connector) │                            │  PA5 SOL_PWM ─┐   PB0/PB1 I2C ── J2 (target; key=master)
                           │                            │  PA6 SOL_BOOST_EN  wake-on-I2C: SCL START (no wake line)
                           │   SOL_BOOST_EN ──► EN       │  PA0 UPDI ─────── J4 (program)
                           └─► U2 MT3608 boost ──► +12V (VSOL) ─┬─ C5 220µF + C6 22µF  (reservoir)
                               L1 10µH · D2 SS34               │
                               FB: R3 200k / R4 10k ≈ 12.6V    ▼
                                                          ┌─ SOLENOID ─┐  (J3 JST-PH)
                                                  D3 SS34 │            │
                                                  flyback └─────┬──────┘
                                                                │ SOL_DRV (drain)
                                                  SOL_PWM ─[R5 100R]─┤ Q1 AO3400A (low-side)
                                                              [R6 100k]│
                                                                    GND┘
```

Sheets: **MCU** (`mcu.kicad_sch`) · **PWR** (`psu.kicad_sch`) · **DRV** (`drv.kicad_sch`).

## BOM (all JLCPCB / LCSC)

| Ref | Part | Value | LCSC | JLC | Footprint |
|-----|------|-------|------|-----|-----------|
| U1 | ATtiny1616 | tinyAVR-1, runs 1.8–5.5 V | C507118 | ext | QFN-20 3×3 0.4 mm |
| U2 | MT3608 | boost, ~2 A switch | C84817 | ext | SOT-23-6 |
| Q1,Q2,Q4,Q5 | AO3400A | N-FET (solenoid LS, FB switch, servo logic, interlock) | C20917 | **Basic** | SOT-23 |
| Q3 | AO3401A | P-FET (servo high-side load switch) | C15127 | **Basic** | SOT-23 |
| D2,D3 | SS34 | Schottky 40 V/3 A | C8678 | **Basic** | SMA |
| L1 | FNR6045S100MT | 10 µH power inductor | C168076 | ext | 6×6 mm |
| C5,C8 | 220 µF 25 V | reservoir / VSERVO bulk (electrolytic) | C4747974 | ext | Ø6.3×7.7 |
| C4,C6 | 22 µF 25 V | boost out / VSOL bypass | C12891 | **Basic** | 1206 |
| C3 | 10 µF 25 V | boost in | C15850 | **Basic** | 0805 |
| C1,C2,C7,C9,C10 | 100 nF / 1 µF / 1 nF | decouple / hall debounce / snubber(DNP) | C1525/C29266 | **Basic** | 0402 |
| R3 | 200 k | FB top | C25764 | **Basic** | 0402 |
| R4 | 22 k | FB base → 6 V default | C25768 | **Basic** | 0402 |
| R17 | 20 k | FB switch → 12 V (Q2) | C25765 | **Basic** | 0402 |
| R2,R6,R18,R19,R20 | 100 k | pulldowns / P-FET gate pull-up | C25741 | **Basic** | 0402 |
| R16,R21,R22,R23,R25 | 10 k | servo pulldowns/series, hall pull-ups | C25744 | **Basic** | 0402 |
| R5 | 100 Ω | gate series | C106232 | ext* | 0402 |
| R1,R15,R24 | 1 k | status LED / servo signal series | C11702 | **Basic** | 0402 |
| D1 | LED green | status (driven by **PC3**) | C160479 | ext | 0402 |
| J3 | JST-PH 2-pin | solenoid | C173752 | ext | PH 2.0 |
| J2 | JST-PH 4-pin RA (S4B-PH-K) | I2C link (right-angle) | C157926 | ext | PH 2.0 RA |
| J4 | pin header 1×3 | UPDI program | — | — | 1×3 |
| J5,J8 | pin header 1×3 | servo outputs (S/V+/GND), DNP unless servo build | — | — | 1×3 |
| R13,R14 | 0 Ω | VSERVO source select (fit one) | C17168 | **Basic** | 0402 |
| J6,J7 | JST-PH 3-pin RA (S3B-PH-K) | door / bolt hall sensors | C157929 | ext | PH 2.0 RA |

\* jlcsearch under-reports Basic flags; verify in JLCPCB's BOM tool at order time.

Every footprint is a KiCad-bundled package, so **all parts carry 3D models** — the
board is clearance-checkable in KiCad's 3D viewer once it's laid out.

## Boost + reservoir sizing (read before ordering)

12 V × up to 1 A ≈ **12 W ≈ ~3.8 A from the cell** — heavy. The design splits it:

- The **MT3608** is sized for the **hold** current (~0.3 A @ 12 V ≈ within its
  ~0.5 A @ 12 V capability) plus recharging the reservoir between actuations.
- The **reservoir** (C5) supplies the brief **pull-in** surge. Sizing:
  `ΔV ≈ I_pull · t_pull / C`. A 1 A, 30 ms pull-in pulls 30 mC; with 220 µF that
  sags the rail a lot, so **220 µF is the baseline for ≤ ~500 mA / short
  pull-ins**. For a true sustained **1 A @ 12 V pull-in**, either grow C5 into
  the multi-thousand-µF range **or** swap the MT3608 for a **TPS61088-class**
  boost that can source the full 12 W directly. Pin this down against the actual
  solenoid's pull-in current × time and re-fire rate.

Set Vout exactly with the FB divider: `Vout = 0.6·(1 + R3/R4)`. 200 k/10 k ≈
12.6 V; trim R3 toward 12.0 V if desired.

## Servo variant + firmware actuator control

The board drives a **12 V solenoid** or a **6 V RC servo**. Because the boost
voltage and the servo power are both firmware-controlled, **one fully-populated board
can carry both** and switch between them (one actuator active at a time — a single
boost = a single voltage). Firmware drives three lines:

- **`BOOST_VSEL`** (PA1) — picks the rail: low/default **6 V** (servo-safe), high
  **12 V** (solenoid). NMOS Q2 switches R17 across the FB divider.
- **`SERVO_PWR_EN`** (PA2) — high-side P-FET (Q3) load switch on VSERVO. Servo power
  comes on only when firmware asserts it **and** the boost is at 6 V: a **hardware
  interlock** (Q5, gated by `BOOST_VSEL`) forces servo power OFF at 12 V regardless of
  firmware, so a 12 V solenoid pulse can never reach a 6 V servo.
- **`SERVO_SIG`** (PB2 / TCA0) — 50 Hz position pulse, via R15 (1 k) + R16 (10 k idle
  pulldown) so the servo can't twitch at boot.

| Mode | BOOST_VSEL | SERVO_PWR_EN | Actuator |
|------|-----------|--------------|----------|
| Solenoid | 12 V | off (interlock-forced) | Q1/D3/J3 peak-and-hold |
| Servo, 6 V | 6 V | on | J5 servo, VSERVO ← VSOL (`R13`, populated) |
| Servo, direct | boost off | on | J5 servo, VSERVO ← VSYS (`R14`, DNP alt) |

- **J5 / J8** — two 3-pin RC-servo outputs (`1=SIG, 2=V+ (VSERVO), 3=GND`), e.g. a
  dual-latch lock. `SERVO_SIG` (PB2/TCA0) and `SERVO_SIG2` (PB4, software-timed);
  both share the `VSERVO` rail and the load-switch/interlock. MT3608 ~1 A @ 6 V —
  drive them sequentially, or upsize C5/C8 for simultaneous travel.
- **VSERVO source** — fit **exactly one** 0 Ω: `R13 = VSOL` (6 V from the boost —
  the populated default) or `R14 = VSYS` (servo direct off the connector rail,
  ~3–4.7 V). **Never both.** C8 (220 µF) buffers servo inrush.
- `SERVO_SIG` is VSYS-level logic (~3–4.7 V), accepted by typical servos, GND-referenced.

## Authenticated digital interface + Firmware plan

The ephemerkey↔lock link is an authenticated I2C bus — **ephemerkey is the
master**, this lock is the target — on a **right-angle 4-pin JST-PH** (`S4B-PH-K`,
a standard 4-pin I2C cable, straight-through):

| Pin | lock (this board, target) | ephemerkey (key, master) |
|-----|---------------------------|--------------------------|
| 1 | GND (+ actuation return) | GND |
| 2 | **VCC ← powers the lock** (VSYS in) | **VSYS** (battery/system rail out) |
| 3 | `SDA` ↔ PB1 | `LOCK_SDA` (PB0) |
| 4 | `SCL` → PB0 (clock + wake) | `LOCK_SCL` (PB1) |

The lock has **no local battery** — its VCC (and the boost/actuator draw) comes in
on J2.2 from ephemerkey's VSYS rail. **Current caveat:** a 12 V solenoid pull-in is
~3–4 A from VSYS, beyond a JST-PH contact (~2 A) and ephemerkey's load-share path —
favor the 6 V servo / low-duty buffered by C5/C8, or run a heavier dedicated feed.
The I2C pull-ups live on ephemerkey at +3V3 (its PB0/PB1 aren't >3.6 V tolerant, so
the bus must not be pulled to VSYS). The lock (running at VSYS) reads the 3.3 V idle
level fine across the discharge curve. The lock is the **target** at addr 0x60 with
**no separate wake line** — it wakes from power-down on the first I2C START (a
pin-change interrupt on SCL), so we don't mix a discrete "button"-style input with
the bus.

Authentication is **firmware HMAC** — no secure element. A pairing secret lives
in flash on **both** boards (separate from ephemerkey's TOTP secret).

**I2C register/command map** (lock = target @ 0x60). The master reads status/nonce
and writes *authenticated* commands:

| Reg | Access | Contents |
|-----|--------|----------|
| `0x00 STATUS` | read | bit0 `DOOR_CLOSED` · bit1 `BOLT_LOCKED` · bit2 `ACTUATOR` (0=solenoid,1=servo) · bit3 `RAIL_12V` · bit4 `BUSY` · bit5 `LAST_CMD_OK` |
| `0x01 NONCE` | read | a fresh 16-byte random nonce; reading **arms** it (consumed by the next COMMAND) |
| `0x10 COMMAND` | write | `[cmd] ‖ HMAC-SHA1(secret, nonce ‖ cmd ‖ code)` — cmd `0x01`=UNLOCK, `0x02`=LOCK |

**Probe lid/door state (unauthenticated read — status only):** the I2C START wakes
the lock; the master reads `STATUS`. `DOOR_CLOSED` / `BOLT_LOCKED` report the J6/J7
hall sensors; `ACTUATOR` tells the key whether a servo is fitted.

**Lock / unlock (authenticated challenge-response, anti-replay):**

1. Master reads `NONCE` (0x01).
2. Master writes `COMMAND` = `cmd ‖ HMAC-SHA1(secret, nonce ‖ cmd ‖ code)`.
3. Lock recomputes, **constant-time** compares against the armed nonce (then clears
   it → replay-proof), and only then drives the actuator:
   - **UNLOCK** (0x01): solenoid → peak-and-hold release; servo → unlock angle.
   - **LOCK** (0x02): **servo → lock angle** (a servo holds position). For a
     momentary solenoid, LOCK is a no-op — it re-latches mechanically (fail-secure).
4. Master may re-read `STATUS` to confirm `BOLT_LOCKED` flipped.

`code` may be the ephemerkey TOTP digits, binding the action to a fresh in-fence
code. Optionally fold a monotonic counter (EEPROM) into the HMAC so a weak RNG
can't be exploited.

**Lock firmware (ATtiny1616 — megaTinyCore or bare AVR):**

- State machine: `SLEEP(power-down)` → (I2C START wakes it) → service TWI as target
  → `SLEEP`; a verified `COMMAND` branches to `ACTUATE`. In `SLEEP`: TWI/boost off,
  `R6` holds Q1 off, `R2` holds boost off; SCL (PB0) pin-change wakes it (wake-on-I2C).
- **STATUS read** → pulse `HALL_PWR` (PA4) high, settle ~1 ms, sample `HALL_DOOR`
  (PA7) + `HALL_BOLT` (PB3), drop HALL_PWR, return the bits (~0 µA between reads).
- **ACTUATE**:
  - *Solenoid* — the **peak-and-hold economizer**: `SOL_BOOST_EN=1` → wait VSOL →
    `SOL_PWM` 100 % pull-in (~20–50 ms) → reduce duty for hold → release. PWM = TCB0 (PA5).
  - *Servo* — `BOOST_VSEL`=6 V, `SERVO_PWR_EN`=on, drive `SERVO_SIG` (PB2/TCA0) to
    the lock/unlock angle, wait for travel, then cut servo power. The hardware
    interlock keeps servo power off whenever `BOOST_VSEL`=12 V.
- HMAC-SHA1 (reuse `smalltotp`, portable C → compiles for AVR) fits in 16 KB/2 KB.
  Secret in USERROW/flash; **disable UPDI / set lockbits** in production.

**Key firmware (ephemerkey STM32U083 — add to its superloop):**

- **Probe the lid:** wake the lock (I2C START) and read `STATUS` — surface door
  open/closed + bolt locked/unlocked (e.g. on the OLED); no auth needed.
- **Lock / unlock:** on a request (button + in-fence + fresh, valid TOTP), read
  `NONCE`, compute `HMAC-SHA1(secret, nonce ‖ cmd ‖ code)`, write `COMMAND` with cmd
  = UNLOCK or LOCK. (Send a dummy/wake byte first and retry — the just-woken target
  NACKs the first.) Re-read `STATUS` to confirm.
- Use the **same** HMAC-SHA1 as the lock — `smalltotp` ships it, so both reuse it.
- The pairing secret is provisioned over USB during pairing.

## Programming

`J4` UPDI header: `1 = UPDI (PA0)`, `2 = VCC (BAT)`, `3 = GND`. One-wire UPDI via
`pymcuprog`, a serial-UPDI adapter, megaTinyCore, or Atmel-ICE/PICkit.

## Bring-up checklist

1. Power from a current-limited 1S cell; confirm the MCU enumerates over UPDI and
   sleeps at ~µA with the I2C bus idle.
2. Pulse `SOL_BOOST_EN`; scope VSOL → should reach ~12.6 V; trim R3 if needed.
3. With a solenoid fitted, tune the pull-in time and hold duty; add the R9/C7
   snubber (DNP) only if drain ringing is high.
4. Bring up the I2C challenge-response against ephemerkey (confirm wake-on-I2C +
   the dummy-then-retry handshake); verify a wrong/late response never actuates.
