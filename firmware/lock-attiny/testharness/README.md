# Lock I²C test harness (RedBoard bridge)

Drive the ATtiny1616 lock's authenticated I²C protocol from the host, without
the STM32 master. A SparkFun RedBoard (ATmega328P) runs a serial↔I²C-master
bridge; `lock_test.py` speaks the NONCE/COMMAND/STATUS protocol over it.

## Build & flash the bridge

```sh
make flash                      # /dev/ttyUSB1, Optiboot @ 115200
# older bootloader? make flash PROG_BAUD=57600
```

## Wiring  ⚠️ level safety

The RedBoard is 5 V, the lock runs at ~Vbat (≈3.3 V). I²C is open-drain and the
bridge **only pulls low** (internal pull-ups disabled), so this is safe **iff**:

| RedBoard | Lock (J2) | notes |
|----------|-----------|-------|
| A4 (SDA) | SDA / PB1 (J2.3) | |
| A5 (SCL) | SCL / PB0 (J2.4) | |
| GND      | GND (J2.1)       | **common ground required** (also with the UPDI adapter) |

- Bus **pull-ups (≈4.7 kΩ) go to the LOCK's Vdd, NOT the RedBoard 5 V.** If the
  lock board already has I²C pull-ups to its Vdd, add none.
- **Do not** wire RedBoard 5 V to the lock.
- The lock must be powered (its own cell or the UPDI adapter's Vtarget).

## Run the protocol (authenticated, over the I2C bridge)

```sh
uv run --with pyserial lock_test.py status                 # door/bolt/status bits
uv run --with pyserial lock_test.py nonce                  # arm + print a nonce
uv run --with pyserial lock_test.py unlock                 # NONCE -> HMAC -> COMMAND -> poll
uv run --with pyserial lock_test.py lock
uv run --with pyserial lock_test.py getconfig
uv run --with pyserial lock_test.py setconfig \            # authenticated (config secret)
    --servo1 1 --servo2 1 --solenoid 1 \
    --servo-ms 600 --strike-ms 50 --hold-ms 10000 --hold-duty 128
```

If you provision non-default secrets (below), pass them to the I2C actions too:
`--pairing-secret <16B ASCII|32 hex>` / `--config-secret ...`.

## Provision config + keys over UPDI (the debug probe, no I2C/auth)

Writes straight to the chip via `pymcuprog` on the UPDI adapter (`/dev/ttyUSB0`)
— for factory/first provisioning before any secret exists. Config → EEPROM,
keys → USERROW (pairing `[0:16]`, config `[16:32]`). Needs `pymcuprog`
(`uv tool install pymcuprog`, or pass `--pymcuprog "uvx pymcuprog"`).

```sh
# keys: default DEV secrets, explicit, or random (prints them — record for the master)
uv run --with pyserial lock_test.py provision-keys
uv run --with pyserial lock_test.py provision-keys \
    --pairing-secret 00112233445566778899aabbccddeeff --config-secret <32hex>
uv run --with pyserial lock_test.py provision-keys --random

# config straight to EEPROM (same knobs as setconfig)
uv run --with pyserial lock_test.py provision-config --servo1 1 --solenoid 1 --hold-ms 10000

uv run --with pyserial lock_test.py provision-all --random   # keys + config in one go
```

USERROW keys survive a chip erase; the EEPROM config does not — re-provision
config after any `--erase` reflash. Once USERROW holds a real secret, set UPDI
lockbits in production.

The lock NACKs its first transaction after waking from power-down, so every op
is retried. The DEV secret in `lock_test.py` matches `src/secret.c` while
USERROW is blank.

## Bridge serial protocol (all hex, one response line each)

```
W <addr> <b0> <b1> ...   -> OK | ERR <tw_status>
R <addr> <n>             -> D <b0> <b1> ... | ERR <tw_status>
```
`ERR FF` = bus timeout (nothing responding / no pull-ups). `ERR 48`/`20` =
address NACK (expected on the lock's first post-wake transaction).
