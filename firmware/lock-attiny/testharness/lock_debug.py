#!/usr/bin/env python3
"""Deep debug harness for the ephemerkey lock protocol, over the RedBoard bridge.

Requires the lock firmware built with the debug register enabled:

    (in firmware/lock-attiny)  make LOCK_DEBUG=1 flash
    (here)                     uv run --with pyserial lock_debug.py

REG_DEBUG (0x11) returns 32 bytes: the nonce the ISR *armed* (16) ‖ the nonce
service_command *verified* against (16). This is the tool that caught the
lost-wakeup bug where the lock verified one transaction late (armed==host but
verified lagged). It runs N valid trials plus a deliberate bad-HMAC trial and
checks arm/verify consistency and accept/reject.

All values hex. DEV secret matches src/secret.c while USERROW is blank.
"""
import sys
import time
import hmac
import hashlib
import argparse

import serial  # pyserial

ADDR = 0x60
SECRET = b"ephemerkey-dev-secrt"
REG_STATUS, REG_NONCE, REG_COMMAND, REG_DEBUG = 0x00, 0x01, 0x10, 0x11
CMD_UNLOCK = 0x01


class Bridge:
    def __init__(self, port, baud=57600):
        self.ser = serial.Serial(port, baud, timeout=1.0)
        time.sleep(2.0)
        self.ser.reset_input_buffer()

    def _cmd(self, s):
        self.ser.write((s + "\n").encode())
        return self.ser.readline().decode(errors="replace").strip()

    def read_reg(self, reg, n, tries=8):
        for _ in range(tries):
            self._cmd("W %02X %02X" % (ADDR, reg))
            r = self._cmd("R %02X %02X" % (ADDR, n))
            if r.startswith("D"):
                b = bytes(int(x, 16) for x in r.split()[1:])
                if len(b) == n:
                    return b
            time.sleep(0.05)
        return None

    def write(self, data, tries=8):
        line = "W %02X " % ADDR + " ".join("%02X" % b for b in data)
        for _ in range(tries):
            if self._cmd(line) == "OK":
                return True
            time.sleep(0.05)
        return False


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", default="/dev/ttyUSB1")
    ap.add_argument("--baud", type=int, default=57600)
    ap.add_argument("--trials", type=int, default=3)
    args = ap.parse_args()
    b = Bridge(args.port, args.baud)

    ok_all = True
    for t in range(args.trials):
        nonce = b.read_reg(REG_NONCE, 16)
        mac = hmac.new(SECRET, nonce + bytes([CMD_UNLOCK]), hashlib.sha1).digest()
        b.write([REG_COMMAND, CMD_UNLOCK] + list(mac))
        time.sleep(1.0)
        dbg = b.read_reg(REG_DEBUG, 32)
        st = b.read_reg(REG_STATUS, 1)[0]
        arm_ok = nonce == dbg[:16]
        ver_ok = nonce == dbg[16:32]
        acc = bool(st & 0x20)  # LAST_CMD_OK
        ok_all &= arm_ok and ver_ok and acc
        print("valid t%d: armed_matches=%s verified_matches=%s accepted=%s status=0x%02X"
              % (t, arm_ok, ver_ok, acc, st))

    # deliberate bad HMAC -> must be rejected
    nonce = b.read_reg(REG_NONCE, 16)
    b.write([REG_COMMAND, CMD_UNLOCK] + [0] * 20)
    time.sleep(0.5)
    st = b.read_reg(REG_STATUS, 1)[0]
    rejected = not (st & 0x20)
    ok_all &= rejected
    print("bad HMAC: rejected=%s status=0x%02X" % (rejected, st))

    print("RESULT:", "PASS" if ok_all else "FAIL")
    return 0 if ok_all else 1


if __name__ == "__main__":
    sys.exit(main())
