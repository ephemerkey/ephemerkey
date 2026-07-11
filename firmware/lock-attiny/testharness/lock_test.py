#!/usr/bin/env python3
"""Host-side test harness for the ephemerkey lock over the RedBoard I2C bridge.

Runs the authenticated protocol (STATUS / NONCE / COMMAND) against the ATtiny1616
lock (TWI target @ 0x60). Requires pyserial:

    uv run --with pyserial firmware/lock-attiny/testharness/lock_test.py status
    uv run --with pyserial firmware/lock-attiny/testharness/lock_test.py unlock
    uv run --with pyserial firmware/lock-attiny/testharness/lock_test.py lock

The lock NACKs its first transaction after waking from power-down, so every
read/write is retried a few times.
"""
import sys
import time
import hmac
import hashlib
import argparse

import serial  # pyserial

ADDR = 0x60
# DEV fallback secret (matches src/secret.c k_fallback when USERROW is blank).
SECRET = b"ephemerkey-dev-secrt"

REG_STATUS, REG_NONCE, REG_COMMAND = 0x00, 0x01, 0x10
CMD_UNLOCK, CMD_LOCK = 0x01, 0x02

STATUS_BITS = [
    (0x01, "DOOR_CLOSED"), (0x02, "BOLT_LOCKED"), (0x04, "ACTUATOR=servo"),
    (0x08, "RAIL_12V"), (0x10, "BUSY"), (0x20, "LAST_CMD_OK"),
]


class Bridge:
    def __init__(self, port, baud=57600):
        self.ser = serial.Serial(port, baud, timeout=1)
        time.sleep(2.0)              # RedBoard resets on open; wait for boot
        self.ser.reset_input_buffer()

    def _cmd(self, s):
        self.ser.write((s + "\n").encode())
        return self.ser.readline().decode(errors="replace").strip()

    def write(self, data, retries=6):        # data = [reg, ...payload]
        s = "W %02X " % ADDR + " ".join("%02X" % b for b in data)
        for _ in range(retries):
            if self._cmd(s) == "OK":
                return True
            time.sleep(0.05)                 # just-woken target NACKs the first
        return False

    def read(self, n):
        r = self._cmd("R %02X %02X" % (ADDR, n))
        if r.startswith("D"):
            return bytes(int(x, 16) for x in r.split()[1:])
        return None

    def read_reg(self, reg, n, retries=6):
        for _ in range(retries):
            self.write([reg])        # set sticky register pointer
            d = self.read(n)
            if d and len(d) == n:
                return d
            time.sleep(0.05)         # wake latency / first-NACK
        return None


def show_status(b):
    d = b.read_reg(REG_STATUS, 1)
    if d is None:
        print("STATUS: no response"); return None
    s = d[0]
    flags = [name for bit, name in STATUS_BITS if s & bit]
    print("STATUS: 0x%02X  [%s]" % (s, ", ".join(flags) or "-"))
    return s


def send_command(b, cmd_byte):
    nonce = b.read_reg(REG_NONCE, 16)
    if nonce is None:
        print("NONCE: no response"); return False
    print("NONCE:  " + nonce.hex())
    mac = hmac.new(SECRET, nonce + bytes([cmd_byte]), hashlib.sha1).digest()
    print("HMAC:   " + mac.hex())
    ok = b.write([REG_COMMAND, cmd_byte] + list(mac))
    print("COMMAND write:", "OK" if ok else "FAILED (retry — first wake NACKs)")
    return ok


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("action", choices=["status", "unlock", "lock", "nonce"])
    ap.add_argument("--port", default="/dev/ttyUSB1")
    ap.add_argument("--baud", type=int, default=57600)
    args = ap.parse_args()

    b = Bridge(args.port, args.baud)

    if args.action == "status":
        show_status(b)
    elif args.action == "nonce":
        d = b.read_reg(REG_NONCE, 16)
        print("NONCE:", d.hex() if d else "no response")
    else:
        cmd = CMD_UNLOCK if args.action == "unlock" else CMD_LOCK
        show_status(b)
        send_command(b, cmd)
        time.sleep(1.5)              # let actuation run
        for _ in range(10):          # poll until not BUSY
            s = show_status(b)
            if s is not None and not (s & 0x10):
                break
            time.sleep(0.5)


if __name__ == "__main__":
    sys.exit(main())
