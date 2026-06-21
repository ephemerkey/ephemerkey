# Reference Material

Pointers to source material this project was built from.

## Source skeleton
- **Altium part-selection skeleton:** `elec/pr/totp/totp_gps_gen/` — the
  original schematic with the five anchor parts. Pin names/numbers for the
  KiCad symbols were extracted from `totp_gps_gen.SchDoc`.
- **Repo skeleton modeled after:** `github/reefvolt-sensorbuddy/` (KiCad 10
  layout, Makefile, lib-tables, JLCPCB script, DESIGN.md style).

## Firmware
- **smalltotp:** `github/smalltotp/` — RFC 6238 TOTP engine (SHA1, HMAC-SHA1,
  Base32) with STM32 RTC time helpers. Linked/vendored by the firmware.

## Datasheets (to download into this folder via the datasheets skill)
| Part | MPN | Notes |
|------|-----|-------|
| MCU | STM32U083KCU6 | AF map for UFQFPN-32, RTC/USB/LPUART, RDP |
| GNSS | MAX-M10S-00B | u-blox M10 integration manual (UART, PPS, V_BCKP, RF) |
| Antenna | W3011A | π-match topology, ground keep-out |
| Power | TPS63900DSKR | CFG/SEL strapping, inductor + cap selection |
| Accel | LIS3DHTR | I2C addressing, INT1/INT2 config |

> Use the `digikey`/`datasheets` skills to populate this folder, then the
> `kicad`/`emc`/`spice` analyzers can consume verified specs.
