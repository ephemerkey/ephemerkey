# ephemerkey-rs — STM32U083 firmware (Rust / Embassy)

Async-Rust replacement for the C `ephemerkey-stm32/` skeleton, built on
[Embassy](https://embassy.dev). Chip: **STM32U083KCU6** (Cortex-M0+,
UFQFPN-32, `thumbv6m-none-eabi`), officially supported by `embassy-stm32`
(feature `stm32u083kc`). No STM32Cube dependency.

## Personalities

One image, two roles, selected by provisioned config (`src/config.rs`):

| Role | Pipeline |
|------|----------|
| **Generator** | GNSS (USART1) → geofence → RTC-disciplined TOTP emission |
| **LockController** | code slots / policy engine (`src/policy.rs`) → authenticated I2C link to the lock board |

Both are programmed with signed config **files** over USB (button-gated
provisioning mode) or the WiFi/ESP32-C3 link — see `src/provision.rs` and
`../../DESIGN-policies.md` for the slot/policy model.

## Layout

| File | Subsystem |
|------|-----------|
| `src/main.rs` | boot, role dispatch, LEDs (PA6/PA7), buttons (PA5/PA15), PPS placeholder (PA0) |
| `src/gnss.rs` | MAX-M10S on USART1 (PA9/PA10, DMA RX) + RESET_N (PA4, OD) + EXTINT (PA1) |
| `src/lock.rs` | **bit-banged** I2C master to the lock board (PB0/PB1) — see below |
| `src/sensors.rs` | I2C1 (PB6/PB7): LIS3DH @0x18, OLED, M24M02E log EEPROM; INT1/INT2 EXTI |
| `src/wifi.rs` | ESP32-C3 on LPUART1 (PA2/PA3) + PB5 power gate (off by default) |
| `src/buzzer.rs` | TIM3_CH1 PWM on PB4 (boot chirp) |
| `src/config.rs` | role + persistent config (flash journal TODO) |
| `src/provision.rs` | signed config file ingestion (USB/WiFi transports TODO) |
| `src/policy.rs` | code-slot / pedantic-unlock state machines (sketch) |

Every pin binding is type-checked against the U083 AF table at compile time —
the crate builds only if the DESIGN.md pin budget is electrically coherent.

### Hardware finding: PB0/PB1 have no I2C silicon

The pin budget assigns the lock link to PB0/PB1, but on the U083 those pins
carry **no I2C alternate function** (only LCD/LPTIM3/SPI1-CS/UART-flow). The
link is therefore a software open-drain master (`src/lock.rs`). This is
acceptable-to-preferable: the lock glitches the bus during actuation
(~0.4 s bursts — see `../lock-attiny/README.md`), and a bit-banged master
does `bus_clear()` + retry with no peripheral error-state to unwedge. If
hardware I2C is ever required, the schematic must re-pin (e.g. swap the LEDs
PA6/PA7 ↔ PB0/PB1 to free I2C3).

### Timer allocation

| Timer | Use |
|-------|-----|
| TIM15 | embassy time driver (do not touch) |
| TIM2 | reserved: GNSS PPS input capture (PA0, CH1) |
| TIM3 | buzzer PWM (CH1, PB4) |
| LPTIM1 | future: time driver in Stop mode (LSE), with the `low-power` work |

## Build / flash

```sh
make build          # debug
make release
make run            # probe-rs flash + defmt console (SWD on PA13/PA14)
make flash          # release flash
```

Requires `rustup` (pulls the pinned toolchain + target via
`rust-toolchain.toml`) and `probe-rs` (`--chip STM32U083KC`).

## Roadmap

- [ ] Clock tree: LSE→RTC, HSI48+CRS→USB; then Stop-mode + `low-power` executor
- [ ] TOTP core: RustCrypto `hmac`/`sha1` port of smalltotp (share test vectors)
- [ ] NMEA parse + geofence (port `geofence.c` point-in-polygon)
- [ ] TIM2_CH1 PPS capture → RTC discipline + staleness window
- [ ] Flash config journal (last 2×2KB pages, CRC ping-pong) + RDP/HDP
- [ ] USB FS provisioning console (`embassy-usb`, button-gated)
- [ ] Lock link: nonce/HMAC command flow over the bit-bang master (+ soak
      against the real lock through actuation transients)
- [ ] Policy engine: gates, Path/DeadMan machines, confirm-TOTP (HOTP receipt)
- [ ] ESP32-C3 protocol: OTA staging, config file transport
