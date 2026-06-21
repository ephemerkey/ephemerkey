# ephemerkey-stm32

STM32U083 application firmware for the ephemerkey GPS-geofenced TOTP generator.
Built on the **STM32CubeU0 HAL** and the **smalltotp** RFC 6238 engine.

## What it does

Wake → power the GNSS → acquire a fix → parse NMEA → discipline the RTC from
GNSS UTC → gate on **fix quality** (sats, HDOP), **clock freshness**
(anti-replay), and **geofence** (haversine) → generate a TOTP code → emit it to
the companion lock (UART line + `CODE_VALID` strobe) → sleep until motion
(LIS3DH) or a duty timer. See `../../DESIGN.md` for the full design.

## Layout

```
ephemerkey-stm32/
├── Makefile                  # arm-none-eabi build (CUBE_U0 + smalltotp)
├── STM32U083KCUx_FLASH.ld    # linker script (256K flash / 40K RAM)
├── include/
│   ├── board.h               # pin map (mirrors DESIGN.md pin budget)
│   ├── ephemerkey_config.h   # secret, geofence, gating thresholds
│   ├── gnss.h                # NMEA RMC/GGA parser
│   ├── geofence.h            # haversine geofence test
│   ├── totp_app.h            # RTC <-> Unix bridge + emit
│   └── stm32u0xx_hal_conf.h  # HAL module config
└── src/
    ├── main.c                # superloop + peripheral init
    ├── gnss.c                # NMEA parser (host-testable)
    ├── geofence.c            # haversine (host-testable)
    ├── totp_app.c            # TOTP glue (RTC, smalltotp, lock output)
    ├── stm32u0xx_hal_msp.c   # AF pin / clock setup
    └── stm32u0xx_it.c        # ISRs (SysTick, EXTI2_3)
```

## Prerequisites

```bash
# ARM toolchain (already on this machine)
arm-none-eabi-gcc --version

# STM32CubeU0 (HAL + CMSIS + startup + system file)
git clone https://github.com/STMicroelectronics/STM32CubeU0 \
    ../vendor/STM32CubeU0        # -> firmware/vendor/STM32CubeU0 (gitignored)

# smalltotp is the sibling repo github/smalltotp (default path used by Makefile)
```

## Build

```bash
make                                   # uses CUBE_U0=../../vendor/STM32CubeU0
# or point at an existing checkout:
make CUBE_U0=/path/to/STM32CubeU0 SMALLTOTP=/path/to/smalltotp
make flash                             # st-flash to 0x08000000
make size
```

## TODOs before first silicon

- `SystemClock_Config()` — generate the LSE/MSI(/HSI48+CRS for USB) clock tree
  with CubeMX for STM32U083 and paste it into `main.c`.
- Verify AF numbers in `board.h` / `stm32u0xx_hal_msp.c` against the datasheet
  (USART2 TX on PB0, I2C1 on PB6/PB7, USART1 on PA9/PA10).
- I2C `Timing` value in `MX_I2C1_Init` — recompute for the chosen I2C clock.
- `enter_stop_until_event()` — implement Stop 2 entry + LIS3DH INT1 EXTI wake +
  RTC wakeup timer (currently a placeholder delay).
- `lis3dh_init()` — configure motion/tamper interrupts.
- USB CDC provisioning path (load secret + geofence into protected flash).

## Host-testable units

`gnss.c` and `geofence.c` depend only on libc/libm (no HAL) and can be compiled
and unit-tested natively:

```bash
cc -Iinclude -c src/gnss.c src/geofence.c   # sanity compile
```

## License

Apache 2.0 — see `../LICENSE`. smalltotp is also Apache 2.0.
