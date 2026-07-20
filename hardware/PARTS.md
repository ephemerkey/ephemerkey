# ephemerkey — Parts (BOM source of truth)

Every line resolves to a real KiCad **bundled** symbol + footprint and a JLCPCB
**LCSC** number. 0402 passives throughout; bulk caps 0805. Charge current set for
a ~500 mAh 1S Li-ion (Rprog 4.7 kΩ ≈ 213 mA, ~0.5 C). The schematic is generated
from `scripts/ephemerkey.schgen.py` — **edit that, not the `.kicad_sch` files**.

> JLCPCB Basic vs Extended: the jlcsearch API under-reports the Basic flag, so
> verify in JLCPCB's BOM tool at order time. The common 0402 R/C (Uniohm/Samsung)
> and SOT-23 jellybeans here are Basic; the ICs, crystal, inductor, connectors,
> and LEDs are Extended.

## Actives / modules

| Ref | Value | MPN | LCSC | Footprint |
|-----|-------|-----|------|-----------|
| U1 | STM32U083KCU6 | STM32U083KCU6 | C22459164 | ephemerkey:UFQFPN-32-1EP_5x5mm_P0.5mm_EP3.5x3.5mm (project copy → generic QFN-32 3D model) |
| MD1 | MAX-M10S-00B | MAX-M10S-00B | C4153167 | ephemerkey:ublox_MAX (project copy + vendored 3D model) |
| AE1 | W3011A | W3011A | C5830926 | RF_Antenna:Pulse_W3011 |
| U2 | TPS63900DSKR | TPS63900DSKR | C1518762 | Package_SON:WSON-10-1EP_2.5x2.5mm_P0.5mm_EP1.2x2mm |
| U3 | USBLC6-2SC6 | USBLC6-2SC6 | C2687116 | Package_TO_SOT_SMD:SOT-23-6 |
| U4 | MCP73831-2-OT | MCP73831T-2ACI/OT | C424093 | Package_TO_SOT_SMD:SOT-23-5 |
| U5 | LIS3DHTR | LIS3DHTR | C15134 | Package_LGA:LGA-16_3x3mm_P0.5mm |
| Q1, Q3 | AO3401A (P-FET) | AO3401A | C15127 | Package_TO_SOT_SMD:SOT-23 |
| Q2, Q4 | AO3400A (N-FET) | AO3400A | C20917 | Package_TO_SOT_SMD:SOT-23 |
| D3 | B5819W (Schottky) | B5819W | C8598 | Diode_SMD:D_SOD-123 |
| MD2 | ESP32-C3-MINI-1 (WiFi, optional) | ESP32-C3-MINI-1-N4 | C2838502 | ephemerkey:ESP32-C3-MINI-1 (vendored from espressif/kicad-libraries, w/ 3D model) |
| U6 | AP2112K-3.3 (WiFi LDO, optional) | AP2112K-3.3TRG1 | C51118 | Package_TO_SOT_SMD:SOT-23-5 |
| U7 | M24M02E (2Mbit audit-log EEPROM) | M24M02E-FMC6TG | C29549719 | ephemerkey:ST_UFDFPN8-8-1EP_2x3mm_P0.5mm_EP1.4x1.4mm (vendored; verified vs ST UFDFPN8 outline) |

## Frequency / power magnetics

| Ref | Value | MPN | LCSC | Footprint | Note |
|-----|-------|-----|------|-----------|------|
| Y1 | 32.768 kHz | Q13FC13500004 | C32346 | Crystal:Crystal_SMD_3215-2Pin_3.2x1.5mm | RTC LSE; match load caps to CL |
| L1 | 2.2 µH | FNR3015S2R2MT | C167747 | Inductor_SMD:L_Changjiang_FNR3015S | TPS63900 (3×3 mm, small) |
| L2 | FB 600Ω@100MHz | BLM15AG601SN1D | C76884 | Inductor_SMD:L_0402_1005Metric | VCC_RF isolation |

## Connectors / switch / LEDs

| Ref | Value | MPN | LCSC | Footprint |
|-----|-------|-----|------|-----------|
| J3 | USB-C (GCT USB4105) | USB-TYPE-C-019 | C2927039 | Connector_USB:USB_C_Receptacle_GCT_USB4105-xx-A_16P_TopMnt_Horizontal |
| J4 | BAT 1S (JST PH RA) | S2B-PH-K-S | C173752 | Connector_JST:JST_PH_S2B-PH-K_1x02_P2.00mm_Horizontal |
| J1 | SWD | Conn_ARM_SWD_TagConnect_TC2030-NL | — | Connector:Tag-Connect_TC2030-IDC-NL_2x03_P1.27mm_Vertical |
| J2 | LOCK OUT (1x4) | header 2.54 | — | Connector_PinHeader_2.54mm:PinHeader_1x04_P2.54mm_Vertical |
| DS1 | OLED 128x32 I2C (1x4, 0.1") | `Display_Graphic:ER_OLEDM0.91_1x-I2C` | — | 1x4 female socket P2.54 (PinSocket_1x04): GND/VCC/SCL/SDA → I2C1 |
| SW1–SW3 | USER1/2/3 (SW3=BOOT0/DFU) | TS-1187A-B-A-B | C318884 | ephemerkey:SW_Push_1P1T_XKB_TS-1187A (vendored from tsumikoro, has 3D model) |
| D1 | LED green (status) | LTST-C281KGKT | C160479 | LED_SMD:LED_0402_1005Metric |
| D2 / D4 | LED red (fault / charge) | NCD0402R1 | C130719 | LED_SMD:LED_0402_1005Metric |

## 0402 passives (Basic)

| Value | LCSC | Used for |
|-------|------|----------|
| R 5.1k | C25905 | USB-C CC1/CC2 (R4,R5) |
| R 4.7k | C25900 | Rprog (R6, 213 mA), I²C pull-ups (R9,R10) |
| R 100k | C25741 | load-share gate pulldown (R7), WiFi LDO EN pulldown (R26), +3V3_SW switch (R30 Q3 gate PU, R31 PERI_EN PD) |
| R 10k | C25744 | BOOT0 pulldown (R1), WiFi EN RC + straps (R18–R21) |
| R 1k | C11702 | LED series (R2,R3,R8), WiFi UART series (R22,R23) |
| R 0Ω | C17168 | antenna π-match series (Rm1), TPS63900 CFG2 = I_lim Unlimited (R28), WiFi→MCU recovery links (R24,R25 — DNP) |
| R 36.5k 1% | C25887 | TPS63900 CFG1 = V_O(2) 3.3V (R27) |
| R 16.2k 1% | C27176 | TPS63900 CFG3 = V_O(1) 3.3V, the active preset (R29) |
| C 100nF | C1525 | decoupling (C3,C4,C5,C8,C9,C17,C20,C21,C22,C23,C24,C28,C30) |
| C 1µF | C29266 | VDDA/VREF+ filter (C7), WiFi EN RC (C25), +3V3_SW reservoir (C31) |
| C 12pF | C1547 | LSE load caps (C1,C2) — tune to crystal CL |
| C 10pF | (TBD) | VCC_RF HF bypass (C18) |
| C 10µF 0805 | C15850 | bulk: 3V3, charger, buck-boost, VBUS/VBAT, GNSS VCC (C6,C10–C16,C19), WiFi LDO/module (C26,C27,C29) |
| C DNP | — | antenna π-match shunts (Cm2,Cm3 — fit at bring-up) |

## Notes
- **Rprog → charge current:** I_chg ≈ 1000 V / Rprog. 4.7 kΩ = 213 mA (~0.5 C of
  500 mAh). Use 3.9 kΩ for ~256 mA if faster charging is wanted.
- **LSE load caps:** 12 pF shown; set to ≈2·(CL − C_stray) for the chosen Y1 CL,
  trim residual with the STM32 RTC SMOOTHCALIB.
- **TPS63900 CFG straps (R27–R29):** read once at EN rising by the R2D
  interface — 1% / ≤200 ppm required (TI: total RMS error < 3%), keep CFG
  traces short (< 10 pF), SEL strapped to GND. Both presets 3.3 V, I_lim
  Unlimited (L1 Isat 2 A meets the ≥ 2 A requirement).
- **3 parts need a downloaded 3D model** (STM32U083KCU6, MAX-M10S, W3011A) — see
  `lib/3dmodels/README.md`.
- **WiFi is optional:** MD2 + U6 + R18–R26 + C25–C29 live on `wifi.kicad_sch`
  and can be depopulated as a group. Own LDO rail from VSYS (350mA TX bursts
  never load the TPS63900); off by default (PB5 low, <1µA leak).
- **U7 audit log** (`storage.kicad_sch`): I2C1 @ 0x50–0x53, encrypted +
  HMAC-chained 32B records, ~8000-record ring (2Mbit). Secrets stay in internal
  flash; STM32 OTA images stage in the ESP32's 4MB flash — see DESIGN.md
  "Storage, Logging & OTA".
