# ephemerkey project footprints

Footprints for parts that are **not** in KiCad's bundled libraries (or that need
a project-modified copy, e.g. a repointed 3D model path) — see the "KiCad
Library Map" in `../../../DESIGN.md`. Registered via `../../fp-lib-table` /
`../../ephemerkey/fp-lib-table` (lib name `ephemerkey`).

| Footprint | Source |
|-----------|--------|
| `ublox_MAX` | u-blox MAX form-factor land pattern |
| `SW_Push_1P1T_XKB_TS-1187A` | reused from the tsumikoro project |
| `UFQFPN-32-1EP_5x5mm_P0.5mm_EP3.5x3.5mm` | KiCad bundled copy, project-adjusted |
| `USB_C_Receptacle_G-Switch_GT-USB-7051x` | KiCad bundled copy, model repointed to `../3dmodels/` |
| `ESP32-C3-MINI-1` | [espressif/kicad-libraries](https://github.com/espressif/kicad-libraries) (CC-BY-SA 4.0 w/ library exception), model repointed to `../3dmodels/` |
| `ST_UFDFPN8-8-1EP_2x3mm_P0.5mm_EP1.4x1.4mm` | EasyEDA/LCSC C29549719 via `easyeda2kicad` (M24M02E-F). Verified vs ST UFDFPN8 outline (DS6638 Fig. 20/Table 22): pin rows on the 2 mm ends, 3 mm apart; EP within the 1.2–1.6 mm die-pad spec. No 3D model (0.55 mm slab). |
