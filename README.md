# ephemerkey

A battery-powered, **GPS-geofenced TOTP code generator**. It produces RFC 6238
time-based one-time passwords (TOTP) — but only when the device holds a valid
GNSS fix inside an authorized geofence. Outside the fence, no codes. A companion
analog "TOTP lock" board consumes the codes and drives an actuator.

> Time comes from GNSS (disciplined RTC), place comes from GNSS — so a code is
> proof of *being in the right place at the right time*.

## Features

- **GNSS time + place**: u-blox MAX-M10S provides UTC time (TIMEPULSE-disciplined
  RTC) and a position fix. Codes are gated on both an accurate clock and a fix
  inside the configured geofence.
- **RFC 6238 TOTP**: SHA1/HMAC/Base32 engine from the `smalltotp` library,
  6-digit / 30-second codes (configurable).
- **Ultra-low-power**: STM32U083 (Cortex-M0+ ULP) + TPS63900 buck-boost with
  nanoamp-class quiescent current for long battery life; GNSS duty-cycled.
- **Tamper / motion**: LIS3DH accelerometer for wake-on-motion and tamper
  detection (optionally zeroizes the secret on tamper).
- **Single coin/Li-ion cell** input via buck-boost (works above and below 3.3 V).

## Architecture

```
                    ┌───────────────── RF ─────────────────┐
   W3011A chip ANT ─┤  (50Ω + π-match)                      │
                    └──────────────┬───────────────────────┘
                                   │ RF_IN
                          ┌────────┴─────────┐
                          │  MAX-M10S-00B    │  u-blox M10 GNSS
                          │  (TCXO, LNA, SAW)│
                          └─┬───┬────┬───┬───┘
                   UART(TXD/RXD)│    │TIMEPULSE (1 PPS → RTC discipline)
                            │   │EXTINT (wake)
                            │   │
   batt ── TPS63900 ──3V3───┼───┴──────────────┐
   (buck-boost,             │                  │
    ultra-low Iq)           │ USART1           │ V_BCKP (RTC/GNSS backup)
                     ┌───────┴────────┐         │
                     │  STM32U083KCU6 │         │
                     │  Cortex-M0+    │── RTC ──┘ (TOTP time base)
                     │  256KB flash   │
                     └─┬────┬────┬────┘
                  I2C  │    │    │ SWD (PA13/PA14)
                  ┌────┴──┐ │    │
                  │LIS3DH │ │    └── USB (PA11/PA12) — provisioning/console
                  │ accel │ │
                  └───────┘ └── code out → companion "TOTP lock" board
                  (motion/tamper)   (UART / open-drain / display)
```

## Repository Structure

```
ephemerkey/
├── hardware/                    # PCB design (KiCad 10, CERN-OHL-P v2)
│   ├── lib/                     # empty — anchor parts use KiCad bundled libs
│   │   ├── symbols/             #   (reserved for future custom symbols)
│   │   ├── footprints.pretty/   #   (reserved for future custom footprints)
│   │   └── 3dmodels/            #   (reserved for future 3D STEP models)
│   ├── ephemerkey/              # The generator board
│   │   ├── ephemerkey.kicad_pro
│   │   ├── ephemerkey.kicad_sch # Top-level: MCU, USB, SWD, button, LED
│   │   ├── psu.kicad_sch        # Battery → TPS63900 buck-boost sub-sheet
│   │   ├── gnss.kicad_sch       # MAX-M10S + W3011A antenna sub-sheet
│   │   └── sensors.kicad_sch    # LIS3DH accelerometer sub-sheet
│   ├── scripts/
│   ├── Makefile
│   ├── sym-lib-table
│   └── fp-lib-table
├── firmware/
│   ├── ephemerkey-stm32/        # STM32U083 application (STM32CubeU0 HAL)
│   ├── common/                  # Shared HAL/utilities
│   ├── shared/                  # Shared protocol definitions
│   └── LICENSE                  # Apache 2.0
├── reference/                   # Datasheets, source-skeleton pointers
├── DESIGN.md                    # Full hardware/firmware design document
└── LICENSE
```

## Key Components

| Component | Part | Package | Purpose |
|-----------|------|---------|---------|
| MCU | STM32U083KCU6 | UFQFPN-32 | ULP Cortex-M0+, 256KB flash, RTC, USB, TOTP engine |
| GNSS | MAX-M10S-00B | LGA (9.7×10mm) | u-blox M10: UTC time + position fix |
| GPS antenna | W3011A | SMD chip | 1.559–1.606 GHz GNSS antenna |
| Power | TPS63900DSKR | VSON-10 | Buck-boost, nanoamp Iq, single-cell → 3V3 |
| Accelerometer | LIS3DHTR | LGA-16 | Motion wake + tamper detection |

## Companion Project

The **TOTP lock** (separate analog board) reads the code stream from this
generator and drives an actuator (solenoid/relay). ephemerkey is the *generator*
half; the lock is the *consumer* half. The interface is documented in
`DESIGN.md` (§ Code Output Interface).

## License

- **Hardware**: CERN-OHL-P v2 (permissive open hardware)
- **Firmware**: Apache 2.0 (TOTP engine via smalltotp, also Apache 2.0)

Copyright (c) 2025-2026 EphemerKey Authors
