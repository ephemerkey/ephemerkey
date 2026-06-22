# Claude Code Assistant Guidelines

## General Rules
- Do not run cat or stty commands
- Use makefile targets when possible
- This is a KiCad 10 project — use kicad-cli for schematic/PCB operations
- Use `uv` for all Python operations (install, run, etc.) — never use bare pip

## Project Context
- ephemerkey: a GPS-geofenced TOTP (RFC 6238) code generator. Codes are only
  emitted when the device has a valid GNSS fix inside an authorized geofence.
- Companion: an analog "TOTP lock" board (separate project) consumes the codes
  and drives an actuator.
- Ported from an Altium part-selection skeleton (elec/pr/totp/totp_gps_gen).
- Structure modeled after reefvolt-sensorbuddy/ (KiCad 10, STM32 + sub-sheets).
- Firmware reuses the smalltotp library (sibling dir github/smalltotp).

## Hardware
- KiCad project files live in hardware/ephemerkey/
- **The schematic is GENERATED from a data manifest, not hand-edited.** Edit
  `hardware/scripts/ephemerkey.schgen.py` then `cd hardware && make gen-ephemerkey`.
  Components are PLACED, not wired — wiring is an eeschema phase guided by the
  per-sheet on-canvas notes. Regenerate BEFORE wiring (regen reassigns UUIDs).
- All parts use **KiCad bundled libraries** (symbols + footprints); see DESIGN.md
  "KiCad Library Map" and `hardware/PARTS.md` (BOM → real JLCPCB LCSC). The
  project's own lib/symbols + lib/footprints.pretty are kept empty for future
  non-standard parts.
- `make check-ephemerkey` — components/footprints/dup-refs/ERC tally.
- `make render-ephemerkey` — per-sheet PNGs for review.
- `make docs` (images + BOM) · `make jlc` (JLCPCB fab+assembly zip).
- Close KiCad before regenerating; `extends`-symbols show a benign
  lib_symbol_mismatch in ERC until first saved in eeschema.

## Firmware
- STM32U083 app: firmware/ephemerkey-stm32/ (STM32CubeU0 HAL)
- TOTP engine: vendored/linked from github/smalltotp (RFC 6238, SHA1/HMAC/Base32)
- Set CUBE_U0 to your STM32CubeU0 checkout before building (see firmware README)

## Key Parts (from the Altium skeleton)
- STM32U083KCU6 (ARM Cortex-M0+ ULP MCU, 256KB flash, UFQFPN-32)
- MAX-M10S-00B (u-blox M10 GNSS module)
- W3011A (GPS chip antenna, 1.559-1.606 GHz)
- TPS63900DSKR (buck-boost, ultra-low Iq, battery rail)
- LIS3DHTR (3-axis accelerometer — motion/tamper, low-power wake)
