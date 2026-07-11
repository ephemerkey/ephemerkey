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

## Run the protocol

```sh
uv run --with pyserial lock_test.py status     # read door/bolt/status bits
uv run --with pyserial lock_test.py nonce      # arm + print a nonce
uv run --with pyserial lock_test.py unlock     # NONCE -> HMAC -> COMMAND -> poll
uv run --with pyserial lock_test.py lock
```

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
