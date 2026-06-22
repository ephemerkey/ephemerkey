# ephemerkey 3D models

Most parts get their 3D model automatically from KiCad's bundled libraries
(0402/0805 R/C/L/LED, 32.768kHz crystal, JST-PH, GCT USB4105 USB-C, SOT-23/-5/-6,
WSON-10, LGA-16, SOD-123, 1x4 header).

**Provided here** (vendored project models + a project footprint copy that points
its `(model …)` at `${KIPRJMOD}/../lib/3dmodels/`):

| Part | File | Footprint | Source |
|------|------|-----------|--------|
| MAX-M10S-00B (GNSS) | `ublox_MAX-M10S.step` | `ephemerkey:ublox_MAX` | u-blox MAX form-factor, fcmadwar/3D-Step-Models-Library (AP214) |
| SW1–SW3 tactile (XKB TS-1187A) | `SW_Push_1P1T_XKB_TS-1187A.step` | `ephemerkey:SW_Push_1P1T_XKB_TS-1187A` | reused from the tsumikoro project |

> Verify the MAX model's offset/rotation in the 3D viewer at layout — third-party
> STEP origins don't always match the KiCad footprint origin.

**Still to download** (no model in KiCad; every source needs a free login):

| Part | Drop file here as | Where to download |
|------|-------------------|-------------------|
| STM32U083KCU6 (UFQFPN-32) | `STM32U083KCU6.step` | ST CAD Resources: https://www.st.com/en/microcontrollers-microprocessors/stm32u083kc.html · or Ultra Librarian / SnapMagic / TraceParts |
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
