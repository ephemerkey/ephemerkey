# ephemerkey project footprints

All five anchor parts use **KiCad bundled libraries** (symbols + footprints) —
see the "KiCad Library Map" in `../../../DESIGN.md`. No custom footprints are
currently required.

This directory is reserved for any future part that is **not** in KiCad's
standard libraries. Add `*.kicad_mod` files here and register them via
`../../fp-lib-table` / `../../ephemerkey/fp-lib-table` (lib name `ephemerkey`).
