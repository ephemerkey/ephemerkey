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

Everything (MCU + boost + driver) runs from a **single 1S Li-ion cell**.

## Architecture

```
 BAT 1S Li-ion (3.0–4.2V) ─┬─────────────────────────► U1 ATtiny1616 VCC   (direct, NO LDO; ~0.1µA sleep)
   (JST-PH, J1)            │                            │  PA5 SOL_PWM ─┐   PB0/PB1 I2C ── J2 (target; key=master)
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
| Q1 | AO3400A | N-FET 30 V/5.7 A logic-level | C20917 | **Basic** | SOT-23 |
| D2,D3 | SS34 | Schottky 40 V/3 A | C8678 | **Basic** | SMA |
| L1 | FNR6045S100MT | 10 µH power inductor | C168076 | ext | 6×6 mm |
| C5 | 220 µF 25 V | reservoir (electrolytic) | C2918361 | ext | Ø6.3×7.7 |
| C4,C6 | 22 µF 25 V | boost out / VSOL bypass | C12891 | **Basic** | 1206 |
| C3 | 10 µF 25 V | boost in | C15850 | **Basic** | 0805 |
| C1,C2,C7 | 100 nF / 1 µF / 1 nF | decouple / snubber(DNP) | C1525/C29266 | **Basic** | 0402 |
| R3 | 200 k | FB top | C25764 | **Basic** | 0402 |
| R2,R6 | 100 k | EN pulldown / gate pulldown | C25741 | **Basic** | 0402 |
| R4 | 10 k | FB bottom | C25744 | **Basic** | 0402 |
| R5 | 100 Ω | gate series | C106232 | ext* | 0402 |
| R1 | 1 k | status LED | C11702 | **Basic** | 0402 |
| D1 | LED green | status | C160479 | ext | 0402 |
| J1,J3 | JST-PH 2-pin | battery / solenoid | C173752 | ext | PH 2.0 |
| J2,J4 | pin header | auth I2C / UPDI | — | — | 1×3 / 1×3 |

\* jlcsearch under-reports Basic flags; verify in JLCPCB's BOM tool at order time.

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

## Authenticated digital interface + Firmware plan

The ephemerkey↔lock link is a **3-wire authenticated I2C** bus — **ephemerkey is
the master**, this lock is the target (J2 here ↔ ephemerkey J2, straight-through
cable):

| Pin | lock (this board, target) | ephemerkey (key, master) |
|-----|---------------------------|--------------------------|
| 1 | GND | GND |
| 2 | `SCL` → PB0 (clock + wake) | `LOCK_SCL` (PB1) |
| 3 | `SDA` ↔ PB1 | `LOCK_SDA` (PB0) |

Bus pull-ups live on **ephemerkey** (master) to its 3.3 V — not on this board (the
lock runs at VBAT; master-side pull-ups avoid the cross-domain). The lock is the
**target** at addr 0x60 with **no separate wake line** — it wakes from power-down on
the first I2C START (a pin-change interrupt on SCL), so we don't mix a discrete
"button"-style input with the bus.

Authentication is **firmware HMAC** — no secure element. A pairing secret lives
in flash on **both** boards (separate from ephemerkey's TOTP secret).

**Protocol (challenge-response, anti-replay):**

1. Master starts an I2C transaction — the first START wakes the lock — and
   **reads** a fresh random **nonce** from it.
2. Master **writes** `HMAC-SHA1(secret, nonce [‖ code])` to the lock.
3. Lock recomputes locally and **constant-time** compares. Match → actuate;
   mismatch/timeout → stay locked, rate-limit, re-sleep.

The nonce defeats replay. Optionally fold in a monotonic counter (EEPROM) so a
weak RNG can't be exploited. `code` may be the ephemerkey TOTP digits, binding
the unlock to a fresh in-fence code.

**Lock firmware (ATtiny1616 — megaTinyCore or bare AVR):**

- State machine: `SLEEP(power-down)` → `AUTH` → `ACTUATE` → `SLEEP`.
- In `SLEEP`: TWI/boost off; `R6` holds Q1 off; `R2` holds boost off; a pin-change
  interrupt on **SCL** (PB0) is armed so the first I2C START wakes it (wake-on-I2C,
  no separate trigger line). On wake, enable TWI0 as target.
- `ACTUATE` = the **peak-and-hold economizer**: `SOL_BOOST_EN=1` → wait ~2–5 ms
  for VSOL → `SOL_PWM` 100 % for the pull-in window (~20–50 ms) → reduce PWM duty
  (~⅓, tune to coil) for the hold/unlock window → `SOL_PWM=0` → `SOL_BOOST_EN=0`.
  PWM via TCB0 (PA5).
- HMAC-SHA1 (reuse `smalltotp`'s implementation — `smalltotp` is portable C and
  compiles for AVR) fits easily in 16 KB flash / 2 KB SRAM. Store the secret in
  USERROW/a flash page; **disable UPDI / set lockbits** in production so flash
  can't be read back.

**Key firmware (ephemerkey STM32U083 — add to its superloop):**

- On an unlock request (button + in-fence + fresh, valid TOTP), drive I2C as
  master (PB0/PB1) — the first START wakes the lock — read the nonce, compute the
  same `HMAC-SHA1(secret, nonce[‖ code])`, write it back, return to Stop. (Send a
  dummy/wake byte first and retry, since the just-woken target NACKs the first.)
- Use the **same** HMAC-SHA1 code as the lock for interop — `smalltotp` already
  ships HMAC-SHA1/SHA1/Base32, so both boards reuse it directly (no extra crypto
  to write or audit).
- The pairing secret is provisioned over USB during a pairing step.

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
