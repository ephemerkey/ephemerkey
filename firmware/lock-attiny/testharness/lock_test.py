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
import os
import time
import hmac
import hashlib
import argparse
import secrets as _secrets
import subprocess

import serial  # pyserial

# EEPROM/USERROW layout must match the firmware (config.c / secret.c).
EE_CONFIG_OFFSET = 16    # config.c EE_CONFIG_ADDR
USERROW_PAIRING_OFFSET = 0
USERROW_CONFIG_OFFSET = 16

ADDR = 0x60
# DEV fallback secrets — match src/secret.c when USERROW is blank (16 bytes each).
SECRET = b"ephemerkey-dev01"        # pairing (unlock/lock)
CONFIG_SECRET = b"ephemerkey-cfg01"  # config (admin)

REG_STATUS, REG_NONCE, REG_COMMAND, REG_CONFIG = 0x00, 0x01, 0x10, 0x20
CMD_UNLOCK, CMD_LOCK = 0x01, 0x02
CONFIG_LEN = 10
CFG_MAGIC = 0xE2

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


def us_to_pos(us):
    return max(0, min(255, round((us - 500) * 255 / 2000)))


def build_config(servo1=True, servo2=False, solenoid=False,
                 s1_lock_us=1000, s1_unlock_us=2000,
                 s2_lock_us=1000, s2_unlock_us=2000,
                 servo_ms=600, strike_ms=50, hold_ms=200, hold_duty=128):
    flags = (0x01 if servo1 else 0) | (0x02 if servo2 else 0) | (0x04 if solenoid else 0)
    return bytes([
        CFG_MAGIC, flags,
        us_to_pos(s1_lock_us), us_to_pos(s1_unlock_us),
        us_to_pos(s2_lock_us), us_to_pos(s2_unlock_us),
        min(255, servo_ms // 10), min(255, strike_ms // 10),
        min(255, hold_ms // 100), hold_duty,
    ])


def show_config(b):
    d = b.read_reg(REG_CONFIG, CONFIG_LEN)
    if d is None:
        print("CONFIG: no response"); return None
    print("CONFIG: " + d.hex()
          + "  (magic=0x%02X flags=0x%02X servo=%dms strike=%dms hold=%dms duty=%d)"
          % (d[0], d[1], d[6] * 10, d[7] * 10, d[8] * 100, d[9]))
    return d


def write_config(b, blob):
    nonce = b.read_reg(REG_NONCE, 16)
    if nonce is None:
        print("NONCE: no response"); return False
    mac = hmac.new(CONFIG_SECRET, nonce + blob, hashlib.sha1).digest()
    ok = b.write([REG_CONFIG] + list(blob) + list(mac))
    print("CONFIG write:", "OK" if ok else "FAILED")
    return ok


# --- UPDI provisioning (via pymcuprog, bypasses the authenticated I2C path) ---

def parse_secret(s):
    """32 hex chars -> 16 bytes; otherwise ASCII, padded/truncated to 16."""
    t = s.strip()
    if len(t) == 32:
        try:
            return bytes.fromhex(t)
        except ValueError:
            pass
    b = t.encode()[:16]
    return b + b"\x00" * (16 - len(b))


def updi_write(pymcuprog, port, memtype, offset, data):
    cmd = (pymcuprog.split()
           + ["write", "-d", "attiny1616", "-t", "uart", "-u", port,
              "-m", memtype, "-o", str(offset), "-l"]
           + ["0x%02X" % b for b in data])
    print("  $ " + " ".join(cmd))
    r = subprocess.run(cmd, capture_output=True, text=True)
    out = (r.stdout + r.stderr).strip().splitlines()
    if r.returncode != 0:
        print("    " + (out[-1] if out else "failed"))
    return r.returncode == 0


def provision_keys(args):
    if args.random:
        pair, conf = _secrets.token_bytes(16), _secrets.token_bytes(16)
    else:
        pair, conf = parse_secret(args.pairing_secret), parse_secret(args.config_secret)
    print("pairing secret: " + pair.hex())
    print("config  secret: " + conf.hex())
    # USERROW is one 32-byte page: pairing[0:16] ‖ config[16:32], written together.
    ok = updi_write(args.pymcuprog, args.updi_port, "user_row",
                    USERROW_PAIRING_OFFSET, pair + conf)
    print("provision keys:", "OK" if ok else "FAILED")
    print("** record these — the ephemerkey master must use the SAME secrets **")


def provision_config(args):
    blob = build_config(bool(args.servo1), bool(args.servo2), bool(args.solenoid),
                        args.s1_lock, args.s1_unlock, s2_lock_us=1000, s2_unlock_us=2000,
                        servo_ms=args.servo_ms, strike_ms=args.strike_ms,
                        hold_ms=args.hold_ms, hold_duty=args.hold_duty)
    print("config blob: " + blob.hex())
    ok = updi_write(args.pymcuprog, args.updi_port, "eeprom", EE_CONFIG_OFFSET, blob)
    print("provision config:", "OK" if ok else "FAILED")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("action",
                    choices=["status", "unlock", "lock", "nonce", "getconfig", "setconfig",
                             "provision-keys", "provision-config", "provision-all"])
    ap.add_argument("--port", default="/dev/ttyUSB1")
    ap.add_argument("--baud", type=int, default=57600)
    # setconfig knobs
    ap.add_argument("--servo1", type=int, default=1)
    ap.add_argument("--servo2", type=int, default=0)
    ap.add_argument("--solenoid", type=int, default=0)
    ap.add_argument("--s1-lock", type=int, default=1000)
    ap.add_argument("--s1-unlock", type=int, default=2000)
    ap.add_argument("--s2-lock", type=int, default=1000)
    ap.add_argument("--s2-unlock", type=int, default=2000)
    ap.add_argument("--servo-ms", type=int, default=600)
    ap.add_argument("--strike-ms", type=int, default=50)
    ap.add_argument("--hold-ms", type=int, default=200)
    ap.add_argument("--hold-duty", type=int, default=128)
    # provisioning / secrets (also used by the I2C actions so they match USERROW)
    ap.add_argument("--updi-port", default="/dev/ttyUSB0")
    ap.add_argument("--pymcuprog", default="pymcuprog",
                    help="pymcuprog invocation, e.g. 'uvx pymcuprog'")
    ap.add_argument("--pairing-secret", default="ephemerkey-dev01",
                    help="16-byte ASCII or 32 hex chars")
    ap.add_argument("--config-secret", default="ephemerkey-cfg01")
    ap.add_argument("--random", action="store_true", help="provision-keys: generate random")
    args = ap.parse_args()

    # Make the I2C actions use the same secrets we'd provision (default = DEV).
    global SECRET, CONFIG_SECRET
    SECRET = parse_secret(args.pairing_secret)
    CONFIG_SECRET = parse_secret(args.config_secret)

    if args.action.startswith("provision"):
        if args.action in ("provision-keys", "provision-all"):
            provision_keys(args)
        if args.action in ("provision-config", "provision-all"):
            provision_config(args)
        return

    b = Bridge(args.port, args.baud)

    if args.action == "status":
        show_status(b)
    elif args.action == "nonce":
        d = b.read_reg(REG_NONCE, 16)
        print("NONCE:", d.hex() if d else "no response")
    elif args.action == "getconfig":
        show_config(b)
    elif args.action == "setconfig":
        show_config(b)
        blob = build_config(bool(args.servo1), bool(args.servo2), bool(args.solenoid),
                            args.s1_lock, args.s1_unlock,
                            s2_lock_us=args.s2_lock, s2_unlock_us=args.s2_unlock,
                            servo_ms=args.servo_ms, strike_ms=args.strike_ms,
                            hold_ms=args.hold_ms, hold_duty=args.hold_duty)
        write_config(b, blob)
        time.sleep(0.3)
        show_config(b)
    else:
        cmd = CMD_UNLOCK if args.action == "unlock" else CMD_LOCK
        show_status(b)
        send_command(b, cmd)
        time.sleep(0.5)
        for _ in range(40):          # poll until not BUSY (covers a long hold)
            s = show_status(b)
            if s is not None and not (s & 0x10):
                break
            time.sleep(0.5)


if __name__ == "__main__":
    sys.exit(main())
