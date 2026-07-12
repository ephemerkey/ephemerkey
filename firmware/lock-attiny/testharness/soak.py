#!/usr/bin/env python3
"""Soak-test the lock: repeated lock/unlock with aggressive STATUS polling in
ONE Bridge session (serial stays open — no DTR resets hiding the evidence).

Every I2C error is logged with raw bus line states ('L'). On a sustained comms
wedge, attribute the failing layer:
  1. in-session retry     — recovers => bridge TWI latch (auto-reinit fixed it)
  2. serial close/reopen  — DTR-resets ONLY the bridge; recovers => bridge-hard
  3. still dead           => target-side; left unreset for UPDI forensics

    uv run --with pyserial python3 soak.py [port] [pairs]
"""
import sys
import time
import hmac
import hashlib

import lock_test as L


def raw_status(b):
    """Single-attempt STATUS read (no retry masking). -> (status|None, err)"""
    r = b._cmd("W %02X 00" % L.ADDR)
    if r != "OK":
        return None, "wr:" + r
    r = b._cmd("R %02X 01" % L.ADDR)
    if r.startswith("D"):
        try:
            return int(r.split()[1], 16), ""
        except (IndexError, ValueError):
            return None, "parse:" + r
    return None, "rd:" + r


def lines(b):
    return b._cmd("L")              # raw SDA/SCL levels from the bridge


def send(b, c):
    n = b.read_reg(L.REG_NONCE, 16)
    if n is None:
        return False
    m = hmac.new(L.SECRET, n + bytes([c]), hashlib.sha1).digest()
    return b.write([L.REG_COMMAND, c] + list(m))


def wait_idle(b, limit):
    """Aggressively poll until idle. -> (ok, comm_err_count)"""
    t0 = time.time()
    errs = consec = 0
    seen_busy = False
    while time.time() - t0 < limit:
        s, e = raw_status(b)
        if s is None:
            errs += 1
            consec += 1
            print("    [%5.2fs] comm ERR %s (consec %d)  %s"
                  % (time.time() - t0, e, consec, lines(b)))
            if consec >= 20:
                return False, errs
            time.sleep(0.05)
            continue
        consec = 0
        if s & 0x10:
            seen_busy = True
        elif seen_busy or time.time() - t0 > 2.0:
            return True, errs
        time.sleep(0.08)            # aggressive: the traffic pattern that wedged
    return False, errs


def attribute(b, port):
    print("=== ATTRIBUTION ===")
    print("bus lines:", lines(b))
    s, e = raw_status(b)
    if s is not None:
        print("in-session retry: RECOVERED (0x%02X) -> bridge TWI latch "
              "(auto-reinit healed it)" % s)
        return
    print("in-session retry: still dead (%s)" % e)
    print("reopening serial (DTR-resets ONLY the bridge; target untouched)...")
    b.ser.close()
    time.sleep(0.5)
    b2 = L.Bridge(port)
    print("bus lines:", b2._cmd("L"))
    s, e = raw_status(b2)
    if s is not None:
        print("after bridge-only reset: RECOVERED (0x%02X) -> BRIDGE-side hard "
              "wedge; target was fine all along" % s)
    else:
        print("after bridge-only reset: STILL DEAD (%s) -> TARGET-side wedge; "
              "left unreset — inspect over UPDI before resetting" % e)


def main():
    sys.stdout.reconfigure(line_buffering=True)
    port = sys.argv[1] if len(sys.argv) > 1 else "/dev/ttyUSB1"
    pairs = int(sys.argv[2]) if len(sys.argv) > 2 else 40
    b = L.Bridge(port)
    total = 0
    for i in range(pairs):
        for name, c, limit in (("unlock", L.CMD_UNLOCK, 10),
                               ("lock", L.CMD_LOCK, 10)):
            if not send(b, c):
                print("[pair %d] %s: COMMAND SEND FAILED  %s" % (i, name, lines(b)))
                attribute(b, port)
                return 1
            ok, errs = wait_idle(b, limit)
            total += errs
            print("[pair %2d] %-6s %s  (comm errs so far: %d)"
                  % (i, name, "ok" if ok else "WEDGE", total))
            if not ok:
                attribute(b, port)
                return 1
    print("SOAK PASS: %d lock/unlock pairs, %d transient comm errors "
          "(all self-recovered)" % (pairs, total))
    return 0


if __name__ == "__main__":
    sys.exit(main())
