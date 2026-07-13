# ephemerkey project symbols

Symbols for parts that are **not** in KiCad's bundled libraries. Registered as
lib name `ephemerkey` via `../../sym-lib-table` / `../../ephemerkey/sym-lib-table`.

| Symbol | Source |
|--------|--------|
| `ESP32-C3-MINI-1` | [espressif/kicad-libraries](https://github.com/espressif/kicad-libraries) `Espressif.kicad_sym` (CC-BY-SA 4.0 with library exception), footprint-property repointed to `ephemerkey:ESP32-C3-MINI-1` |
| `M24M02E-F` | EasyEDA/LCSC C29549719 via `easyeda2kicad`, cleaned up: pin types set (NC/power/bidir), EP pin 9 passive, footprint-property → `ephemerkey:ST_UFDFPN8-8-1EP_2x3mm_P0.5mm_EP1.4x1.4mm`, pinout verified vs ST DS14157 |
| `MAX17048` | Authored from the ADI MAX17048/49 datasheet pin table (TDFN-8: CTG/CELL/VDD/GND/ALRT/QSTRT/SCL/SDA + EP=pin 9). Cross-checked pin-for-pin against the independently authored `notchdeck:MAX17048` (sibling project, same part + stdlib `Package_DFN_QFN:TDFN-8-1EP_2x2mm_P0.5mm_EP0.8x1.2mm` footprint) — identical. Shared caveat: verify the stdlib EP land vs the ADI recommended pattern (21-0168) before fab. |
