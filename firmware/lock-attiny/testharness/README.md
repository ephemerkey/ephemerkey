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
uv run --with pyserial lock_test.py abort                  # emergency stop (everything off)
uv run --with pyserial lock_test.py getconfig
uv run --with pyserial lock_test.py setconfig \            # authenticated (config secret)
    --servo-boost 1 \
    --unlock "s1=2500,s2=2500,dur=550 sol,dur=10000,eoff=door-" \
    --lock   "s1=500,s2=500,dur=550"
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
L                        -> L SDA=<0|1> SCL=<0|1>   (raw bus line levels)
```
`ERR FF` = bus timeout (nothing responding / no pull-ups). `ERR 48`/`20` =
address NACK (expected on the lock's first post-wake transaction). `ERR 00` =
TWI bus error — an illegal START/STOP seen on the wire (electrical glitch, see
below). The bridge **fully re-inits its TWI on every error**, so one glitch
cannot latch it.

## ⚠️ Bus transients during actuation (bench finding, July 2026)

Soak-testing (`soak.py`) showed **bursts of bus errors on every actuation**:
3–5 failed transactions (`ERR 00` bus error / `ERR 20` NACK / timeouts) in the
~0.4 s window where the MT3608 boost soft-starts into two stalled servos at
6 V. The solenoid phases produce none — it is the boost-under-stall switching
transient coupling into SDA/SCL, **not** a firmware fault (the lock's engine
and TWI target stay healthy throughout).

The original "lock goes deaf" wedge was the **ATmega328 TWI master latching**
after one such bus error and failing every subsequent START — which looks
identical, from the host, to the target dying. Recovery attempts confounded
this for days: every new `lock_test.py` invocation DTR-resets the RedBoard,
silently un-latching the bridge while we credited the target reset.

Caveats and requirements that follow:

- **This bench exaggerates transients**: flying jumpers, one thin shared
  ground lead carrying servo/boost return current, and dev-board wiring. A
  real installation (short traces, solid ground, local decoupling) should be
  much quieter — but do not count on zero glitches.
- **Any I²C master talking to the lock during actuation MUST be robust**: on
  bus error / arbitration-lost / NACK, re-init the peripheral (or run a
  bus-clear: 9 SCL toggles), back off, retry. This applies to the real
  **STM32 master** — its I²C driver needs explicit bus-error recovery, not
  just a retry loop. Polling STATUS during a cycle is safe *only* with that
  in place.
- `soak.py` is the regression test: repeated lock/unlock pairs, aggressive
  polling, one serial session (no hidden DTR resets), per-error `L` line
  states, and a wedge-attribution ladder (in-session retry → bridge-only
  reset → target). Baseline: 40 pairs, ~250 transient errors, zero wedges.

```sh
uv run --with pyserial python3 soak.py /dev/ttyUSB1 40
```
