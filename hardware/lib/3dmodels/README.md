# ephemerkey 3D models

Most parts get their 3D model automatically from KiCad's bundled libraries
(0402/0805 R/C/L/LED, 32.768kHz crystal, JST-PH, GCT USB4105 USB-C, SOT-23/-5/-6,
WSON-10, LGA-16, SOD-123, 1x4 header). **Four placed parts have no model in
KiCad** (verified against KiCad's upstream 3D repo) and should be downloaded and
dropped here:

| Part | Drop file here as | Where to download (free, account required) |
|------|-------------------|--------------------------------------------|
| STM32U083KCU6 (UFQFPN-32) | `STM32U083KCU6.step` | ST product page → CAD Resources: https://www.st.com/en/microcontrollers-microprocessors/stm32u083kc.html · or Ultra Librarian / SnapMagic / TraceParts |
| MAX-M10S-00B (GNSS) | `ublox_MAX-M10S.step` | SnapMagic: https://www.snapeda.com/parts/MAX-M10S-00B/u-blox/ · Component Search Engine: https://componentsearchengine.com/part-view/MAX-M10S-00B/u-blox · DigiKey models: https://www.digikey.com/en/models/15712906 |
| W3011A (antenna) | `Pulse_W3011.step` | SnapMagic: https://www.snapeda.com/parts/W3011/PulseLarsen%20Antennas/ · Yageo (acquired Pulse) part page |
| SW1 tactile button (XKB TS-1187A) | `XKB_TS-1187A.step` | *Cosmetic only* (render/enclosure clearance). SnapMagic / LCSC C318884 EDA model · or substitute any KiCad-modelled SMD tactile (e.g. Panasonic EVQ) |

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
