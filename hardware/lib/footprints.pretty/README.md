# ephemerkey project footprints

Custom footprints referenced by `hardware/lib/symbols/ephemerkey.kicad_sym`
that are **not** in the KiCad standard libraries. Create these before PCB
layout (and drop matching STEP files in `../3dmodels/`).

| Footprint (`ephemerkey:<name>`) | Symbol | Source |
|---------------------------------|--------|--------|
| `ublox_MAX-M10S_LCC`            | MAX-M10S-00B | u-blox MAX-M10S integration manual (LCC, 9.7×10mm, 18 pad) |
| `W3011A`                        | W3011A | Pulse W3011A antenna datasheet — pads + mfr ground keep-out |
| `Texas_DSK0010A_VSON-10`        | TPS63900DSKR | TI DSK (VSON-10, 2.5×2.5mm, EP) — TI MPDS package drawing |

Symbols whose footprints come from KiCad standard libs (no entry needed here):

| Part | Footprint |
|------|-----------|
| STM32U083KCU6 | `Package_DFN_QFN:UFQFPN-32-1EP_5x5mm_P0.5mm_EP3.45x3.45mm` |
| LIS3DHTR      | `Package_LGA:LGA-16_3x3mm_P0.5mm` |

> Tip: u-blox and TI publish recommended-land-pattern dimensions in their
> datasheets; use the KiCad Footprint Editor's QFN/LCC wizards as a starting
> point, then adjust to the recommended pattern. The W3011A also needs the
> antenna keep-out drawn on the courtyard/fab layer.
