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
- All 5 anchor parts use **KiCad bundled libraries** (symbols + footprints) —
  see DESIGN.md "KiCad Library Map". No custom symbols/footprints needed.
- hardware/lib/{symbols,footprints.pretty} + the lib-tables are kept empty,
  reserved for any future part not in KiCad's standard libs.
- LCSC/MPN/Manufacturer are set as symbol fields when parts are placed
  (values in DESIGN.md); KiCad field-name template recommended.
- Generate JLCPCB outputs: `cd hardware && make jlc`
- Generate docs/images: `cd hardware && make docs`

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
