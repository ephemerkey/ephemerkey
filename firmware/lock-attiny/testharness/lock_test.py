#!/usr/bin/env python3
"""Host-side test harness for the ephemerkey lock over the RedBoard I2C bridge.

Runs the authenticated protocol (STATUS / NONCE / COMMAND / CONFIG) against the
ATtiny1616 lock (TWI target @ 0x60). Requires pyserial:

    uv run --with pyserial firmware/lock-attiny/testharness/lock_test.py status
    uv run --with pyserial firmware/lock-attiny/testharness/lock_test.py unlock
    uv run --with pyserial firmware/lock-attiny/testharness/lock_test.py lock

Actuation is a programmable STEP SEQUENCE (see --unlock / --lock below): an
ordered list of phases, each firing any of {servo1, servo2, solenoid} together
with per-step servo targets, a run time, and an optional hall early-off.

    STEP  = field[,field...]      (no spaces inside a step)
    SEQ   = "STEP STEP ..."       (space-separated, up to 6 steps)
    fields:
      s1=<us>       drive servo1 to <us> (500..2500)
      s2=<us>       drive servo2 to <us>
      sol           fire the solenoid (sol+servo in one step => 6 V, full DC)
      dur=<ms>      run time for the step (servo drive / solenoid hold)
      eoff=door-    early-off: advance when the DOOR sensor's magnet is absent
      eoff=door+ / eoff=bolt+ / eoff=bolt-   (+ = magnet present, - = absent)

    e.g.  --unlock "s1=2000,dur=600 sol,dur=5000,eoff=door- s2=2000,dur=600"

The lock NACKs its first transaction after waking, so reads/writes are retried.
"""
import sys
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
CMD_UNLOCK, CMD_LOCK, CMD_ABORT = 0x01, 0x02, 0x03

# config.h layout
CFG_MAGIC = 0xE4
SEQ_STEPS = 6
STEP_BYTES = 5
CONFIG_HDR = 5
CONFIG_LEN = CONFIG_HDR + STEP_BYTES * SEQ_STEPS * 2   # 65

STEP_SERVO1, STEP_SERVO2, STEP_SOLENOID = 0x01, 0x02, 0x04
EOFF_DOOR, EOFF_BOLT, EOFF_EDGE_ABSENT = 1, 2, 0x04
SENSOR_SRC = {"j6": 0, "j7": 1, "off": 2}

STATUS_BITS = [
    (0x01, "DOOR_CLOSED"), (0x02, "BOLT_LOCKED"), (0x04, "SERVO_ON"),
    (0x08, "RAIL_12V"), (0x10, "BUSY"), (0x20, "LAST_CMD_OK"),
    (0x40, "SOL_ON"),
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


def pos_to_us(pos):
    return round(500 + pos * 2000 / 255)


# --- step-sequence encoding ------------------------------------------------

def parse_step(tok):
    """One step spec -> 5 bytes [act, s1_pos, s2_pos, dur_ds, eoff]."""
    act = s1 = s2 = dur_ds = eoff = 0
    for f in tok.split(","):
        f = f.strip()
        if not f:
            continue
        if f == "sol":
            act |= STEP_SOLENOID
        elif f.startswith("s1="):
            act |= STEP_SERVO1; s1 = us_to_pos(int(f[3:]))
        elif f.startswith("s2="):
            act |= STEP_SERVO2; s2 = us_to_pos(int(f[3:]))
        elif f.startswith("dur="):
            dur_ds = min(255, round(int(f[4:]) / 100))   # ms -> x100 ms
        elif f.startswith("eoff="):
            v = f[5:]
            sensor = EOFF_DOOR if v.startswith("door") else \
                     EOFF_BOLT if v.startswith("bolt") else 0
            if not sensor:
                raise ValueError("eoff sensor must be door/bolt: " + f)
            eoff = sensor | (EOFF_EDGE_ABSENT if v.endswith("-") else 0)
        else:
            raise ValueError("unknown step field: " + f)
    if act == 0:
        raise ValueError("step drives no actuator: " + tok)
    return bytes([act, s1, s2, dur_ds, eoff])


def parse_seq(spec):
    """Space-separated steps -> SEQ_STEPS*5 bytes (unused steps = act 0)."""
    steps = [parse_step(t) for t in spec.split() if t.strip()]
    if len(steps) > SEQ_STEPS:
        raise ValueError("too many steps (max %d): %r" % (SEQ_STEPS, spec))
    steps += [bytes(STEP_BYTES)] * (SEQ_STEPS - len(steps))
    return b"".join(steps)


def build_config(unlock_seq, lock_seq, servo_boost=False,
                 strike_ms=50, hold_duty=128, door_src="j6", bolt_src="j7"):
    flags = 0x01 if servo_boost else 0
    sensor_map = SENSOR_SRC[door_src] | (SENSOR_SRC[bolt_src] << 2)
    hdr = bytes([CFG_MAGIC, flags, min(255, strike_ms // 10), hold_duty, sensor_map])
    blob = hdr + parse_seq(unlock_seq) + parse_seq(lock_seq)
    assert len(blob) == CONFIG_LEN, len(blob)
    return blob


def decode_step(b):
    act, s1, s2, dur, eoff = b
    if act == 0:
        return None
    parts = []
    if act & STEP_SERVO1: parts.append("s1=%dus" % pos_to_us(s1))
    if act & STEP_SERVO2: parts.append("s2=%dus" % pos_to_us(s2))
    if act & STEP_SOLENOID: parts.append("sol")
    txt = "+".join(parts) + " %dms" % (dur * 100)
    sel = eoff & 0x03
    if sel:
        txt += " eoff=%s%s" % ("door" if sel == EOFF_DOOR else "bolt",
                               "-" if eoff & EOFF_EDGE_ABSENT else "+")
    return txt


def decode_seq(blob):
    out = []
    for i in range(SEQ_STEPS):
        s = decode_step(blob[i * STEP_BYTES:(i + 1) * STEP_BYTES])
        if s is None:
            break
        out.append("[%s]" % s)
    return " ".join(out) or "(empty)"


def show_config(b):
    d = b.read_reg(REG_CONFIG, CONFIG_LEN)
    if d is None:
        print("CONFIG: no response"); return None
    src = {0: "J6", 1: "J7", 2: "off"}
    print("CONFIG: " + d.hex())
    print("  magic=0x%02X flags=0x%02X strike=%dms hold_duty=%d door<-%s bolt<-%s"
          % (d[0], d[1], d[2] * 10, d[3],
             src.get(d[4] & 3, "?"), src.get((d[4] >> 2) & 3, "?")))
    print("  UNLOCK: " + decode_seq(d[CONFIG_HDR:CONFIG_HDR + SEQ_STEPS * STEP_BYTES]))
    print("  LOCK:   " + decode_seq(d[CONFIG_HDR + SEQ_STEPS * STEP_BYTES:]))
    return d


def show_hall(b):
    d = b.read_reg(REG_STATUS, 1)
    if d is None:
        print("HALL: no response"); return None
    s = d[0]
    print("HALL: door=%s  bolt=%s  (status 0x%02X)"
          % ("CLOSED" if s & 0x01 else "OPEN",
             "LOCKED" if s & 0x02 else "UNLOCKED", s))
    return s


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


def blob_from_args(args):
    unlock_seq = args.unlock or ("s1=%d,dur=%d" % (args.s1_unlock, args.servo_ms))
    lock_seq = args.lock or ("s1=%d,dur=%d" % (args.s1_lock, args.servo_ms))
    return build_config(
        unlock_seq, lock_seq,
        servo_boost=bool(args.servo_boost),
        strike_ms=args.strike_ms, hold_duty=args.hold_duty,
        door_src=args.door_src, bolt_src=args.bolt_src)


def provision_config(args):
    blob = blob_from_args(args)
    print("config blob: " + blob.hex())
    ok = updi_write(args.pymcuprog, args.updi_port, "eeprom", EE_CONFIG_OFFSET, blob)
    print("provision config:", "OK" if ok else "FAILED")


def main():
    # Line-buffer stdout even when piped/backgrounded, so timelines are live and
    # a killed run still shows how far it got.
    sys.stdout.reconfigure(line_buffering=True)
    ap = argparse.ArgumentParser(
        formatter_class=argparse.RawDescriptionHelpFormatter, description=__doc__)
    ap.add_argument("action",
                    choices=["status", "hall", "unlock", "lock", "abort", "nonce",
                             "getconfig", "setconfig",
                             "provision-keys", "provision-config", "provision-all"])
    ap.add_argument("--port", default="/dev/ttyUSB1")
    ap.add_argument("--baud", type=int, default=57600)
    # --- step sequences (full control) ---
    ap.add_argument("--unlock", default=None,
                    help='UNLOCK sequence, e.g. "s1=2000,dur=600 sol,dur=5000,eoff=door-"')
    ap.add_argument("--lock", default=None,
                    help='LOCK sequence, e.g. "s1=1000,dur=600"')
    # --- convenience defaults (used to build a single-servo1 seq if --unlock/--lock omitted) ---
    ap.add_argument("--s1-lock", type=int, default=1000)
    ap.add_argument("--s1-unlock", type=int, default=2000)
    ap.add_argument("--servo-ms", type=int, default=600)
    # --- global config knobs ---
    ap.add_argument("--servo-boost", type=int, default=0,
                    help="servo-only steps run at 6 V via boost — only if wired for it")
    ap.add_argument("--strike-ms", type=int, default=50,
                    help="solenoid strike (full pull-in) time, 12 V economizer mode")
    ap.add_argument("--hold-duty", type=int, default=128,
                    help="solenoid economizer hold PWM duty, 0..255")
    ap.add_argument("--door-src", choices=["j6", "j7", "off"], default="j6",
                    help="which sensor drives DOOR_CLOSED / eoff=door")
    ap.add_argument("--bolt-src", choices=["j6", "j7", "off"], default="j7",
                    help="which sensor drives BOLT_LOCKED / eoff=bolt")
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
    elif args.action == "hall":
        show_hall(b)
    elif args.action == "setconfig":
        show_config(b)
        write_config(b, blob_from_args(args))
        time.sleep(0.3)
        show_config(b)
    else:
        cmd = {"unlock": CMD_UNLOCK, "lock": CMD_LOCK, "abort": CMD_ABORT}[args.action]
        show_status(b)
        send_command(b, cmd)
        # Poll until not BUSY, with timestamps; distinguish "no response" (comms
        # layer dead) from BUSY (cycle running) so a wedge is attributable. Wait
        # for BUSY to ASSERT first — the first poll can race ahead of the lock
        # servicing the command — but don't wait forever (abort ends ~instantly).
        t0 = time.time()
        noresp = 0
        seen_busy = False
        while time.time() - t0 < 40:
            d = b.read_reg(REG_STATUS, 1)
            el = time.time() - t0
            if d is None:
                noresp += 1
                print("[%6.2fs] STATUS: NO RESPONSE (%d consecutive)" % (el, noresp))
                if noresp >= 5:
                    print("-> I2C/bridge unresponsive; giving up (comms wedge?)")
                    break
                continue
            noresp = 0
            s = d[0]
            flags = [name for bit, name in STATUS_BITS if s & bit]
            print("[%6.2fs] STATUS: 0x%02X  [%s]" % (el, s, ", ".join(flags) or "-"))
            if s & 0x10:
                seen_busy = True
            elif seen_busy or el > 2.0:
                break                # done (or the cycle never started/ended fast)
            time.sleep(0.35)
        else:
            print("-> still BUSY after 40s: actuator wedge? try 'abort'")


if __name__ == "__main__":
    sys.exit(main())
