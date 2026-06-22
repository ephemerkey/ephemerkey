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
| U1 | STM32U083KCU6 | STM32U083KCU6 | C22459164 | Package_DFN_QFN:UFQFPN-32-1EP_5x5mm_P0.5mm_EP3.5x3.5mm |
| MD1 | MAX-M10S-00B | MAX-M10S-00B | C4153167 | ephemerkey:ublox_MAX (project copy + vendored 3D model) |
| AE1 | W3011A | W3011A | C5830926 | RF_Antenna:Pulse_W3011 |
| U2 | TPS63900DSKR | TPS63900DSKR | C1518762 | Package_SON:WSON-10-1EP_2.5x2.5mm_P0.5mm_EP1.2x2mm |
| U3 | USBLC6-2SC6 | USBLC6-2SC6 | C2687116 | Package_TO_SOT_SMD:SOT-23-6 |
| U4 | MCP73831-2-OT | MCP73831T-2ACI/OT | C424093 | Package_TO_SOT_SMD:SOT-23-5 |
| U5 | LIS3DHTR | LIS3DHTR | C15134 | Package_LGA:LGA-16_3x3mm_P0.5mm |
| Q1 | AO3401A (P-FET) | AO3401A | C15127 | Package_TO_SOT_SMD:SOT-23 |
| D3 | B5819W (Schottky) | B5819W | C8598 | Diode_SMD:D_SOD-123 |

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
| SW1–SW3 | USER1/2/3 (SW3=BOOT0/DFU) | TS-1187A-B-A-B | C318884 | ephemerkey:SW_Push_1P1T_XKB_TS-1187A (vendored from tsumikoro, has 3D model) |
| D1 | LED green (status) | LTST-C281KGKT | C160479 | LED_SMD:LED_0402_1005Metric |
| D2 / D4 | LED red (fault / charge) | NCD0402R1 | C130719 | LED_SMD:LED_0402_1005Metric |

## 0402 passives (Basic)

| Value | LCSC | Used for |
|-------|------|----------|
| R 5.1k | C25905 | USB-C CC1/CC2 (R4,R5) |
| R 4.7k | C25900 | Rprog (R6, 213 mA), I²C pull-ups (R9,R10) |
| R 100k | C25741 | load-share gate pulldown (R7) |
| R 10k | C25744 | BOOT0 pulldown (R1) |
| R 1k | C11702 | LED series (R2,R3,R8) |
| R 0Ω | C17168 | antenna π-match series (Rm1) |
| C 100nF | C1525 | decoupling (C3,C4,C5,C8,C9,C17,C20,C21,C22,C23,C24) |
| C 1µF | C29266 | VDDA/VREF+ filter (C7) |
| C 12pF | C1547 | LSE load caps (C1,C2) — tune to crystal CL |
| C 10pF | (TBD) | VCC_RF HF bypass (C18) |
| C 10µF 0805 | C15850 | bulk: 3V3, charger, buck-boost, VBUS/VBAT, GNSS VCC (C6,C10–C16,C19) |
| C DNP | — | antenna π-match shunts (Cm2,Cm3 — fit at bring-up) |

## Notes
- **Rprog → charge current:** I_chg ≈ 1000 V / Rprog. 4.7 kΩ = 213 mA (~0.5 C of
  500 mAh). Use 3.9 kΩ for ~256 mA if faster charging is wanted.
- **LSE load caps:** 12 pF shown; set to ≈2·(CL − C_stray) for the chosen Y1 CL,
  trim residual with the STM32 RTC SMOOTHCALIB.
- **3 parts need a downloaded 3D model** (STM32U083KCU6, MAX-M10S, W3011A) — see
  `lib/3dmodels/README.md`.
