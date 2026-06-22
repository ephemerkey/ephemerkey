# ephemerkey Design Document

## Overview

ephemerkey is a battery-powered **GPS-geofenced TOTP generator**. It emits
RFC 6238 time-based one-time passwords, but only when the device holds a valid
GNSS fix *inside an authorized geofence*. The two inputs that gate a code are
therefore **time** and **place**, both sourced from the same u-blox GNSS module:

- **Time** — UTC from the GNSS, with the 1 PPS TIMEPULSE disciplining the
  STM32 RTC. The RTC is the TOTP time base (so codes are valid even between
  GNSS wake-ups).
- **Place** — a position fix that must lie within a configured lat/lon radius,
  with sufficient fix quality (min satellites, max HDOP).

A companion analog **TOTP lock** board (separate project) consumes the emitted
codes and drives an actuator. ephemerkey is the *generator*; the lock is the
*consumer*. See § Code Output Interface.

This board was ported from an Altium part-selection skeleton
(`elec/pr/totp/totp_gps_gen`). The five anchor parts (MCU, GNSS, antenna,
power, accelerometer) come from that skeleton; the supporting circuitry and
firmware are designed here. Repository layout follows `reefvolt-sensorbuddy/`.

## Architecture Decisions

### MCU: STM32U083KCU6 (UFQFPN-32)

- **Ultra-low-power** Cortex-M0+ (STM32U0 family) — the right fit for a
  coin/Li-ion-cell device that spends most of its life in Stop mode waiting on
  motion or a duty-cycle timer.
- 256KB flash / 40KB RAM — ample for the HAL, NMEA parser, TOTP engine, and
  geofence tables, with room for readout protection (RDP) of the secret.
- **RTC with LSE** (32.768kHz crystal on PC14/PC15) is the TOTP time base.
  Disciplined by GNSS 1 PPS so it stays accurate while the GNSS is powered down.
- **USB FS device** (PA11/PA12) for provisioning/console (load the TOTP secret
  and geofence over USB; crystal-less USB clock recovery available on U0).
- AES hardware + TRNG on U0 — useful for at-rest secret protection / future
  challenge-response.
- Same family/footprint discipline as the reefvolt STM32 boards (UFQFPN custom
  footprint), so PCB practices carry over.

### GNSS: MAX-M10S-00B

- u-blox M10 module: integrated TCXO, LNA, and SAW filter — designed to work
  with a **passive** chip antenna, which is exactly the W3011A here.
- Concurrent GNSS, very low tracking power (~25mW), good for a duty-cycled,
  battery device.
- **UART (NMEA)** is the primary interface (TXD/RXD → STM32 USART1). The module
  also exposes DDC (I2C) and SPI; we use UART and leave I2C for the accelerometer.
- **TIMEPULSE** (1 PPS) → STM32 timer input capture for RTC discipline.
- **EXTINT** driven by the MCU for software wake / time-mark.
- **V_BCKP** kept alive from the always-on 3V3 tap so hot-starts are fast
  (RTC + last ephemeris retained between GNSS power cycles).
- Default UART baud 9600, UBX + NMEA. Configured at boot via UBX-CFG-VALSET
  (set message rates, power mode, dynamic model).

### GPS Antenna: W3011A

- SMD chip antenna, 1.559–1.606 GHz (covers GPS L1 / Galileo E1 / GLONASS G1).
- Needs a **π-match** (series + two shunt placeholders) on a 50Ω controlled-
  impedance trace into MAX-M10S RF_IN, plus the manufacturer's keep-out/ground
  clearance on that PCB corner. Tune match with a VNA at bring-up.
- Passive antenna — the MAX-M10S internal LNA provides the gain; no external
  LNA/bias-tee required. (LNA_EN available if an active antenna is ever fitted.)

### Power: TPS63900DSKR (buck-boost)

- Single-inductor buck-boost with **nanoamp-class quiescent current** and a
  selectable dual-output-voltage feature — ideal for a primary-cell or Li-ion
  device whose input spans both above and below 3.3V over its discharge curve.
- Input range covers 1×Li-ion (3.0–4.2V), 2×alkaline/NiMH (1.8–3.2V), or a
  LiSOCl₂ cell. Output set to **3.3V**.
- Powers everything: STM32 (VDD/VDDA/VDDUSB), MAX-M10S (VCC/VCC_RF/V_IO),
  LIS3DH, and the V_BCKP tap.
- 0.4A capability comfortably covers the GNSS acquisition peak (~30mA) and USB.
- CFG1/2/3 set the output-voltage presets and operating mode per datasheet;
  SEL chooses between the two presets at runtime (e.g. a lower sleep rail);
  EN gated by the MCU or tied on.

### Accelerometer: LIS3DHTR (LGA-16)

- 3-axis, low-power (µA-class), I2C — shares the I2C1 bus with nothing else
  (GNSS is on UART), addr 0x18/0x19.
- **Wake-on-motion** (INT1) lets the MCU sleep in Stop mode and only run the
  GNSS/TOTP pipeline when the device is handled.
- **Tamper detection** (INT2 / free-fall / orientation) — optional policy to
  zeroize the TOTP secret in flash if the enclosure is disturbed while armed.

## Geofence + TOTP Logic

```
   wake (motion INT1 or duty timer)
        │
        ▼
   power GNSS ──► acquire fix ──► parse NMEA (RMC/GGA/GSA)
        │                              │
        │                       fix valid? sats ≥ N, HDOP ≤ H?
        │                              │ yes
        ▼                              ▼
   discipline RTC from PPS      inside geofence? (haversine ≤ radius)
        │                              │ yes
        ▼                              ▼
   RTC = TOTP time base    ─────►  totp_generate_current()
                                       │
                                       ▼
                                emit code (UART + strobe), blink green
        out-of-fence / no fix / stale clock ──► no code, blink red
```

- **Geofence test:** haversine distance from fix to each authorized center;
  inside if ≤ radius. Centers/radii stored in flash (provisioned over USB).
- **Fix gating:** require GNSS valid flag, ≥ N satellites, HDOP ≤ H, and an
  RTC that has been disciplined within a max-staleness window. Reject codes if
  the clock has not seen PPS/UTC recently enough (prevents replay with a frozen
  clock).
- **TOTP:** `smalltotp` (`totp_generate_current`) with the RTC-derived Unix
  time; 6 digits / 30 s by default. Secret stored base32, decoded at boot.

## Power Tree

Resolved: **1S Li-ion (rechargeable)**, **USB-C powers + charges + provisions**,
GNSS kept hot via **V_BCKP** (see decisions below).

```
 USB-C ─VBUS─┬─ USBLC6-2SC6 (ESD: VBUS, D+, D-)
 (GCT        │      D+/D- ─────────────► STM32 USB (PA11/PA12)
  USB4105)   │      CC1/CC2 ─ 5.1k ─ GND (UFP/sink)
             │
             ├─ MCP73831T-2ACI/OT ──BAT── 1S Li-ion ──┐  (charge; Rprog sets Ichg)
             │   (VBUS in, STAT→LED)                  │
             │                                        │
             └─ Schottky ─┐                  load-share P-FET (AO3401A):
                          ▼                  src=BAT, drn=SYS, gate=VBUS
                         SYS ◄────────────────── ON when VBUS absent (run from BAT)
                          │                       OFF when VBUS present (run from VBUS,
                          │                                              charge cleanly)
                          ▼
                  TPS63900DSKR ──── 3V3 rail ──┬── STM32U083  VDD/VDDA-VREF+/VDDUSB (~5–25mA)
                  (buck-boost,                 ├── MAX-M10S   VCC / VCC_RF / V_IO   (~25–30mA acq)
                   ~75nA Iq, L=2.2µH,          ├── LIS3DH     VDD / VDD_IO          (~2µA–2mA)
                   Cin/Cout per DS)            ├── V_BCKP tap (always-on, GNSS RTC/ephemeris backup)
                                               └── pull-ups, LEDs
```

SYS (TPS63900 VIN) is VBUS-via-Schottky (~4.7V) when USB present, else BAT
(3.0–4.2V) — both inside the TPS63900 1.8–5.5V input range, so the buck-boost
outputs a steady 3.3V either way.

### Buck-boost: TPS63900DSKR

- L: 2.2µH shielded (per datasheet typical), DCR-low for efficiency.
- Cin: 10µF X7R; Cout: 2×10µF X7R (low ESR for ripple at the GNSS RF supply).
- Output: 3.3V via CFG pins; SEL for second preset (sleep rail) if used.
- EN: MCU-controllable or tied to VIN through a pull-up (always on).
- Thermal pad to ground pour.

### USB-C input + Li-ion charging

Built from parts already used in sibling projects (BOM consolidation):

- **USB-C receptacle:** GCT **USB4105-xx-A** 16-pin USB-2.0 receptacle
  (MPN USB-TYPE-C-019, LCSC C2927039) — the house-standard connector
  (footprint `Connector_USB:USB_C_Receptacle_GCT_USB4105-xx-A_16P_TopMnt_Horizontal`,
  used in pulsarfab + others). Alt: HRO `TYPE-C-31-M-12` (C165948, notchdeck).
  - **CC1/CC2:** 5.1kΩ pull-downs to GND (device/sink/UFP role; advertises
    default USB current). Two resistors, one per CC line.
  - **D+/D-:** to STM32 USB (PA11/PA12), through the ESD device.
  - **VBUS:** 10µF bulk + feeds charger and the SYS power path.
  - **SHIELD:** to chassis/GND via a 1MΩ ∥ 4.7nF (or direct, TBD).
- **ESD:** **USBLC6-2SC6** (SOT-23-6, LCSC C2687116) on VBUS/D+/D- — same part
  the reefvolt/notchdeck boards use.
- **Charger:** **MCP73831T-2ACI/OT** (SOT-23-5, LCSC C424093) — the house Li-ion
  charger (used in notchdeck). Single-cell, 4.2V (the "-2" variant).
  - VBUS → VDD (input); VBAT → battery (+) node; STAT → indicator LED + 1kΩ.
  - **Rprog** sets charge current: I_chg ≈ 1000V / R_prog. Pick for the cell,
    e.g. 4.7kΩ → ~210mA (≈0.5C of a 400–500mAh cell). 4.7µF on VDD and VBAT.
  - **TODO:** finalize cell capacity → Rprog; add NTC/temperature qualification
    only if the cell pack lacks its own protection.
- **Power path (load sharing):** **AO3401A** P-FET (SOT-23, LCSC C15127) — the
  house P-MOS. Standard Microchip/Adafruit load-share:
  - Q: source=BAT, drain=SYS, gate→VBUS (100kΩ gate-to-GND pull-down).
  - VBUS present → gate high → P-FET OFF → battery isolated from the system load
    (charger sees only the battery → clean charge termination); SYS is fed from
    VBUS through a small **Schottky** (e.g. house SS-series) → run + provision
    from USB without cycling the battery.
  - VBUS absent → gate pulled low → P-FET ON → battery powers SYS.
- **Battery protection:** use a protected 1S Li-ion pack (integrated
  over/under-voltage + over-current), or add a 1S protection IC (e.g. DW01 +
  dual-FET) if using a bare cell. JST-PH 2-pin battery connector (house style).

This keeps provisioning-over-USB clean (system runs from VBUS, battery charges)
and adds no quiescent penalty when unplugged (P-FET on, ~75nA TPS63900 Iq path).

### Supply decoupling

- STM32: 100nF per VDD/VDDA/VDDUSB + 4.7µF bulk; VREF+ filtered (1µF + 10nF).
- MAX-M10S: VCC 100nF + 10µF; **VCC_RF** clean (ferrite bead + 100nF/10pF) —
  RF supply isolation matters for sensitivity; V_BCKP 100nF.
- LIS3DH: 100nF VDD + 100nF VDD_IO.

## Pin Budget: STM32U083KCU6 (UFQFPN-32)

32 perimeter pins + exposed pad. Assignments below are preliminary and must be
verified against the STM32U083 datasheet AF table for the UFQFPN-32 package.

| Pin | MCU | Function | Notes |
|-----|-----|----------|-------|
| 1 | VDD | 3V3 | 100nF + 4.7µF |
| 2 | PC14-OSC32_IN | LSE in | 32.768kHz crystal (RTC) |
| 3 | PC15-OSC32_OUT | LSE out | 32.768kHz crystal (RTC) |
| 4 | PF2-NRST | Reset | 100nF to GND |
| 5 | VDDA/VREF+ | 3V3 analog | 1µF + 10nF |
| 6 | PA0-CK_IN | GNSS PPS in | TIM2_CH1 input capture (RTC discipline) |
| 7 | PA1 | GNSS EXTINT | MCU → GNSS wake / time-mark |
| 8 | PA2 | LIS3DH INT1 | EXTI wake-on-motion |
| 9 | PA3 | LIS3DH INT2 | EXTI tamper / free-fall |
| 10 | PA4 | GNSS RESET_N | open-drain output to GNSS |
| 11 | PA5 | Button | provision / show-code (internal pull-up) |
| 12 | PA6 | LED green | in-fence / code-valid |
| 13 | PA7 | LED red | out-of-fence / fault |
| 14 | PB0 | LOCK_TX | USART → companion lock board |
| 15 | PB1 | LOCK strobe | CODE_VALID open-drain (or LOCK_RX) |
| 16 | VSS_1 | GND | |
| 17 | VDDUSB | 3V3 USB | 100nF (USB transceiver supply) |
| 18 | PA8 | Spare / GNSS_EN | optional GNSS power-gate FET control |
| 19 | PA9 | USART1_TX | → GNSS RXD (NMEA/UBX) |
| 20 | PA10 | USART1_RX | ← GNSS TXD (NMEA/UBX) |
| 21 | PA11 | USB_DM | USB FS (provisioning/console) |
| 22 | PA12 | USB_DP | USB FS (provisioning/console) |
| 23 | PA13 | SWDIO | debug |
| 24 | PA14 | SWCLK | debug |
| 25 | PA15 | Spare | |
| 26 | PB3 | Spare | |
| 27 | PB4 | Spare | |
| 28 | PB5 | Spare | |
| 29 | PB6 | I2C1_SCL | LIS3DH (and optional GNSS DDC) |
| 30 | PB7 | I2C1_SDA | LIS3DH (and optional GNSS DDC) |
| 31 | PF3-BOOT0 | BOOT0 | 10k pull-down (boot from flash) |
| 32 | VSS_2 | GND | |
| EP (33) | GND | thermal/exposed pad | via stitching |

**Peripheral summary:** USART1 (GNSS), USART/bit-bang (lock out), I2C1 (accel),
TIM2 capture (PPS), RTC+LSE (TOTP time), USB FS (provisioning), 2×EXTI (accel),
SWD (debug). ~6 spare GPIO (PA8, PA15, PB3–PB5).

**Notes**
- USB DM/DP must land on PA11/PA12. On the UFQFPN-32, PA9/PA10 and PA11/PA12
  share pads with a SYSCFG remap — here PA9/PA10 (pins 19/20) carry USART1 and
  PA11/PA12 (pins 21/22) carry USB. Verify the remap configuration in firmware
  (`SYSCFG` PA11/PA12 remap) matches this intent.
- GNSS UART could move to **LPUART1** if wake-on-RX in Stop mode is desired;
  the duty-cycle design keeps the MCU awake while GNSS is on, so USART1 is the
  baseline.

## Code Output Interface (to the companion lock)

The generator presents the code to the analog lock board over a simple,
opto-isolatable interface:

- **LOCK_TX (PB0):** ASCII line UART at 9600 8N1, e.g. `CODE 482913\n`, emitted
  only when (in-fence) ∧ (valid fix) ∧ (fresh clock) ∧ (button or armed window).
- **CODE_VALID strobe (PB1):** open-drain, asserted while a fresh valid code is
  available — lets a purely analog lock latch/sample without parsing UART.
- Optional: the same code shown on a small display for manual entry.

The lock board's own design (actuator drive, fail-secure logic) lives in the
companion project; this interface is the contract between them.

## Security Considerations

- **Secret at rest:** TOTP shared secret stored in MCU flash; enable RDP
  (level 1 minimum) in production. Optionally wrap with the U0 AES engine using
  a key derived from a device-unique value.
- **Tamper:** LIS3DH INT2 (free-fall/motion-while-armed) can trigger secret
  zeroization. Policy is firmware-configurable.
- **Anti-replay on clock:** reject code generation if the RTC has not been
  disciplined by GNSS within a configurable staleness window — a frozen or
  rolled-back clock must not yield valid codes.
- **Geofence integrity:** geofence table also in protected flash; provisioning
  over USB requires the device be in an explicit provisioning mode (button +
  USB), not silently writable.

## Resolved Decisions

- **MCU:** STM32U083KCU6 (UFQFPN-32, LCSC C22459164) — ULP Cortex-M0+, RTC,
  USB, 256KB flash. Kept over the JLCPCB-stocked STM32U073KCU6 to retain AES.
- **GNSS:** MAX-M10S-00B over UART (USART1), PPS to TIM2 capture.
- **Antenna:** W3011A passive chip antenna + π-match into RF_IN.
- **Power:** TPS63900DSKR buck-boost, 3.3V out.
- **Battery:** 1S Li-ion (rechargeable), JST-PH; protected pack or add 1S
  protection IC. Buck-boost spans the 3.0–4.2V discharge curve.
- **USB-C:** powers + charges + provisions. GCT USB4105 (USB-TYPE-C-019,
  C2927039), USBLC6-2SC6 ESD, MCP73831T-2ACI/OT charger, AO3401A load-share
  P-FET — all house parts (see § USB-C input + Li-ion charging).
- **GNSS power:** keep V_BCKP alive (~15µA) for hot starts (~1–2s vs ~25s cold);
  MCU may still gate VCC via PA8.
- **Accel:** LIS3DHTR on I2C1, INT1 wake / INT2 tamper.
- **Time base:** STM32 RTC w/ LSE crystal, GNSS-disciplined.
- **Code output:** UART line + open-drain CODE_VALID strobe.

## Open Questions

1. ~~**Battery chemistry / holder**~~ **RESOLVED:** 1S Li-ion (rechargeable),
   JST-PH connector; USB-C charges via MCP73831 + load-share. Use a protected
   pack or add a 1S protection IC. Remaining: pick cell capacity → set Rprog.
2. ~~**GNSS power gating**~~ **RESOLVED:** keep V_BCKP alive for hot starts
   (energy math strongly favors it); optionally also gate VCC via PA8/GNSS_EN.
3. **W3011A placement/keep-out:** confirm ground clearance and match topology
   against the antenna datasheet; reserve a board corner.
4. **TPS63900 CFG/SEL strapping:** finalize the resistor/strap values for 3.3V
   primary + (optional) lower sleep rail, and whether SEL is MCU-driven.
5. ~~**USB-C role**~~ **RESOLVED:** USB-C powers + charges + provisions
   (GCT USB4105, MCP73831 charger, AO3401A load-share). Remaining: USB-C SHIELD
   tie (direct vs 1MΩ∥cap).
6. **Lock interface electrical level:** 3.3V logic direct, or opto-isolated /
   open-drain only? Depends on the companion lock board's input stage.
6. **Provisioning UX:** USB CDC console only, or also a button-driven on-device
   secret-entry mode?
7. **Enclosure / display:** is a display fitted (manual code entry) or is the
   UART-to-lock path the only output?
8. **LSE vs internal:** is a 32.768kHz crystal populated, or run RTC from LSI +
   GNSS discipline only (cheaper, less accurate between fixes)?

## Parts List

### Active Components

| Part | MPN | Package | Qty | Purpose | LCSC | Stock | JLC |
|------|-----|---------|-----|---------|------|-------|-----|
| MCU | STM32U083KCU6 | UFQFPN-32 | 1 | ULP Cortex-M0+, RTC, USB, TOTP | C22459164 | ~10 | extended |
| GNSS module | MAX-M10S-00B | LCC-18 9.7×10mm | 1 | u-blox M10 GNSS (time + place) | C4153167 | ~183 | extended |
| GPS antenna | W3011A | 1206 SMD | 1 | 1.559–1.606 GHz passive antenna | C5830926 | ~101 | extended |
| Buck-boost | TPS63900DSKR | WSON-10-EP | 1 | Battery → 3.3V, nanoamp Iq | C1518762 | ~4187 | extended |
| Accelerometer | LIS3DHTR | LGA-16 3×3 | 1 | Motion wake + tamper | C15134 | ~89984 | extended |
| USB-C conn | USB-TYPE-C-019 (GCT USB4105) | 16P SMD | 1 | USB-C power/charge/data | C2927039 | ~34k | extended |
| Li-ion charger | MCP73831T-2ACI/OT | SOT-23-5 | 1 | 1S Li-ion charger (4.2V) | C424093 | ~2.7k | extended |
| USB ESD | USBLC6-2SC6 | SOT-23-6 | 1 | USB VBUS/D± ESD | C2687116 | ~231k | extended |
| Load-share FET | AO3401A | SOT-23 | 1 | USB/battery power path (P-FET) | C15127 | ~1.2M | extended |

> LCSC numbers verified 2026-06-21 (jlcsearch API; STM32U083KCU6 confirmed on
> the LCSC storefront under C22459164, which jlcsearch does not index). Stock
> figures are a snapshot — recheck before ordering. All parts are JLCPCB
> **extended** (per-part setup fee); none are basic.
>
> **STM32U083KCU6 (C22459164)** is in the LCSC/JLCPCB library, so the MCU is
> JLCPCB-assemblable — but storefront stock is thin (~10), so verify availability
> before an assembly run. If stock is short, the pin-compatible
> **STM32U073KCU6** (UFQFPN-32, C22445363) is a drop-in alternative that drops
> only the unused AES accelerator.

### KiCad Library Map

All five anchor parts are in **KiCad's bundled libraries** — no custom symbols
or footprints are needed. When placing each part in the schematic, use the
symbol below and assign the listed footprint, then set the `LCSC`, `MPN`, and
`Manufacturer` fields (a project Field-Name Template pre-defining these three
fields is recommended). The project's own symbol/footprint libs are kept empty,
reserved for future non-standard parts.

| Part | Symbol (lib:name) | Footprint (lib:name) | LCSC | MPN |
|------|-------------------|----------------------|------|-----|
| MCU | `MCU_ST_STM32U0:STM32U083KCUx` | `Package_DFN_QFN:UFQFPN-32-1EP_5x5mm_P0.5mm_EP3.5x3.5mm` | C22459164 | STM32U083KCU6 |
| GNSS | `RF_GPS:MAX-M10S` | `RF_GPS:ublox_MAX` (symbol default) | C4153167 | MAX-M10S-00B |
| Antenna | `Device:Antenna_Chip` (2-pin: feed+GND) | `RF_Antenna:Pulse_W3011` (pads 1,2,2) | C5830926 | W3011A |
| Buck-boost | `Regulator_Switching:TPS63900` | `Package_SON:WSON-10-1EP_2.5x2.5mm_P0.5mm_EP1.2x2mm` | C1518762 | TPS63900DSKR |
| Accel | `Sensor_Motion:LIS3DH` | `Package_LGA:LGA-16_3x3mm_P0.5mm` | C15134 | LIS3DHTR |
| USB-C | `Connector:USB_C_Receptacle_USB2.0_16P` | `Connector_USB:USB_C_Receptacle_GCT_USB4105-xx-A_16P_TopMnt_Horizontal` | C2927039 | USB-TYPE-C-019 |
| Charger | `Battery_Management:MCP73831-2-OT` | `Package_TO_SOT_SMD:SOT-23-5` | C424093 | MCP73831T-2ACI/OT |
| USB ESD | `Power_Protection:USBLC6-2SC6` | `Package_TO_SOT_SMD:SOT-23-6` | C2687116 | USBLC6-2SC6 |
| Load-share FET | `Device:Q_PMOS_GSD` | `Package_TO_SOT_SMD:SOT-23` | C15127 | AO3401A |

Manufacturers: STM32U083KCU6 / LIS3DHTR = STMicroelectronics; MAX-M10S-00B =
u-blox; W3011A = Pulse Electronics; TPS63900DSKR = Texas Instruments.

Notes:
- `RF_GPS:MAX-M10S` symbol already defaults to the `RF_GPS:ublox_MAX` footprint
  (18-pad LCC, the shared MAX-series land pattern) and carries the MAX-M10S
  datasheet link.
- `RF_Antenna:Pulse_W3011` has 3 pads numbered 1,2,2 (feed + two GND) — it pairs
  with the 2-pin `Device:Antenna_Chip` symbol. Honor the antenna ground keep-out
  (4.0 × 6.25 mm for the W3011A variant) and 50 Ω feed; see § GPS Antenna.
- STM32 symbols ship with an empty Footprint field — assign the UFQFPN-32 one
  above explicitly (use the `_ThermalVias` variant for the EP if preferred).
- **3D models:** the WSON-10, LGA-16, SOT-23/-5/-6, and GCT USB4105 footprints
  carry bundled 3D models. **STM32U083KCU6 (UFQFPN-32), MAX-M10S, and W3011A have
  no model in KiCad** (confirmed vs the upstream 3D repo) — download STEP files
  into `hardware/lib/3dmodels/` and attach per `hardware/lib/3dmodels/README.md`.

### Power Passives

| Part | Value | Package | Qty | Purpose |
|------|-------|---------|-----|---------|
| Inductor | 2.2µH shielded | 2520/2016 | 1 | TPS63900 buck-boost |
| Input cap | 10µF 16V X7R | 0805 | 1 | TPS63900 VIN |
| Output cap | 10µF 16V X7R | 0805 | 2 | TPS63900 VOUT |
| EN/CFG straps | 10k–1M | 0402 | ~4 | TPS63900 CFG1/2/3, SEL, EN |
| Bulk | 4.7µF 16V X7R | 0805 | 1 | 3V3 rail bulk |
| Decoupling | 100nF 16V X7R | 0402 | ~10 | per-IC supply decoupling |

### GNSS / RF Passives

| Part | Value | Package | Qty | Purpose |
|------|-------|---------|-----|---------|
| RF match series | DNP/0Ω (tune) | 0402 | 1 | W3011A π-match series element |
| RF match shunt | DNP (tune) | 0402 | 2 | W3011A π-match shunt elements |
| VCC_RF ferrite | ferrite bead | 0402 | 1 | RF supply isolation |
| VCC_RF cap | 100nF + 10pF | 0402 | 2 | RF supply decoupling |
| GNSS bulk | 10µF 16V X7R | 0805 | 1 | MAX-M10S VCC bulk |
| V_BCKP cap | 100nF | 0402 | 1 | GNSS backup-supply decoupling |

### MCU / Misc Passives

| Part | Value | Package | Qty | Purpose |
|------|-------|---------|-----|---------|
| LSE crystal | 32.768kHz | 3215 | 1 | RTC time base |
| LSE load caps | ~6–12pF | 0402 | 2 | crystal load (per CL) |
| VREF filter | 1µF + 10nF | 0402 | 2 | VDDA/VREF+ |
| BOOT0 pull-down | 10k | 0402 | 1 | boot from flash |
| NRST cap | 100nF | 0402 | 1 | reset filter |
| I2C pull-ups | 4.7k | 0402 | 2 | I2C1 SCL/SDA |
| Button | tactile SMD | — | 1 | provision / show-code |
| LEDs | red / green | 0402 | 2 | status |
| LED resistors | 1k | 0402 | 2 | LED current limit |
| USB connector | USB-C / micro | — | 1 | provisioning + (opt) power |
| USB CC resistors | 5.1k | 0402 | 2 | USB-C CC (if USB-C) |
| Battery holder | per chemistry | — | 1 | see Open Questions |

## Schematic Sheet Plan

| Sheet | Contents |
|-------|----------|
| ephemerkey.kicad_sch | Top-level: STM32U083, SWD, USB, button, LEDs, lock interface, inter-sheet buses |
| psu.kicad_sch | Battery input + TPS63900 buck-boost + 3V3 distribution + V_BCKP tap |
| gnss.kicad_sch | MAX-M10S + W3011A antenna + π-match + RF supply + UART/PPS/EXTINT |
| sensors.kicad_sch | LIS3DH accelerometer + I2C + interrupt lines |

## Firmware Dependencies (STM32U083)

| Library | License | Purpose | Source |
|---------|---------|---------|--------|
| smalltotp | Apache-2.0 | TOTP / HMAC-SHA1 / Base32 / RTC time helpers | github/smalltotp (sibling) |
| STM32CubeU0 HAL/LL | BSD-3 | RTC, USART, I2C, USB, GPIO, low-power | ST (set CUBE_U0) |
| minmea (or hand-rolled) | MIT | NMEA sentence parser (RMC/GGA/GSA) | github.com/kosma/minmea |

Architecture: low-power superloop. Sleep in Stop mode; wake on LIS3DH motion
(INT1) or a duty-cycle RTC alarm; power/duty-cycle the GNSS; parse NMEA;
discipline the RTC from PPS; run the geofence test; generate and emit the TOTP
code; return to Stop. USB provisioning entered via button + USB enumeration.
