# ephemerkey 3D models

Most parts get their 3D model automatically from KiCad's bundled libraries
(0402/0805 R/C/L/LED, 32.768kHz crystal, JST-PH, GCT USB4105 USB-C, SOT-23/-5/-6,
WSON-10, LGA-16, SOD-123, 1x4 header).

**Provided here** (vendored project models + a project footprint copy that points
its `(model …)` at `${KIPRJMOD}/../lib/3dmodels/`):

| Part | File / model | Footprint | Source |
|------|--------------|-----------|--------|
| MAX-M10S-00B (GNSS) | `ublox_MAX-M10S.step` | `ephemerkey:ublox_MAX` | u-blox MAX form-factor, fcmadwar/3D-Step-Models-Library (AP214) |
| SW1–SW3 tactile (XKB TS-1187A) | `SW_Push_1P1T_XKB_TS-1187A.step` | `ephemerkey:SW_Push_1P1T_XKB_TS-1187A` | reused from the tsumikoro project |
| STM32U083KCU6 (UFQFPN-32) | KiCad `QFN-32-1EP_5x5mm_P0.5mm_EP3.45x3.45mm.step` | `ephemerkey:UFQFPN-32-1EP_5x5mm_P0.5mm_EP3.5x3.5mm` | KiCad generic QFN-32 model (UFQFPN = ultra-thin same body; only height differs). No file vendored — model lives in KiCad's 3D dir. |
| USB-C vertical (J5, GT-USB-7051A) | `USB_C_GT-USB-7051A.step` | `ephemerkey:USB_C_Receptacle_G-Switch_GT-USB-7051x` | EasyEDA/LCSC C2843970 via `easyeda2kicad`. KiCad ships the footprint but not the model in this install. |
| ESP32-C3-MINI-1 (WiFi, optional) | `ESP32-C3-MINI-1.step` | `ephemerkey:ESP32-C3-MINI-1` | [espressif/kicad-libraries](https://github.com/espressif/kicad-libraries) (CC-BY-SA 4.0 w/ library exception); footprint + symbol vendored from the same source. |

> Verify the MAX **and USB-C-vertical** model offset/rotation in the 3D viewer at
> layout — third-party STEP origins don't always match the KiCad footprint origin
> (the GT-USB-7051A STEP came from EasyEDA, aligned to its own origin).

**Still to download** (no model in KiCad; source needs a free login):

| Part | Drop file here as | Where to download |
|------|-------------------|-------------------|
| W3011A (antenna) | `Pulse_W3011.step` | SnapMagic: https://www.snapeda.com/parts/W3011/PulseLarsen%20Antennas/ · Yageo (Pulse) part page |

`J1` (SWD Tag-Connect TC2030-NL) intentionally has **no** 3D model and needs none
— it is a bare pogo-pin pad land, no component is mounted.

Notes:
- These sources need a **free login**, so the files can't be auto-fetched in CI.
  Download once and commit them here (STEP only; ~tens of KB each).
- KiCad ships `RF_Antenna:Pulse_W3000` (a *different, larger* antenna) — do **not**
  use it as a stand-in for the W3011A (3.2×1.6×1.1 mm); dimensions differ.

## Attaching a downloaded model to its (bundled) footprint

The bundled footprints reference `${KICAD10_3DMODEL_DIR}/...` paths that don't
contain these models. After dropping a `.step` here, attach it per-board in the
PCB editor:

1. Place the part, open **Footprint Properties → 3D Models**.
2. Add `${KIPRJMOD}/../lib/3dmodels/<file>.step`.
3. Set offset/rotation if the vendor model isn't origin-centered.

(`${KIPRJMOD}` is `hardware/ephemerkey/`, so `../lib/3dmodels/` resolves here.)
This keeps the schematic on bundled symbols/footprints while the project still
carries its own 3D models for rendering and mechanical/STEP export.
