#!/usr/bin/env python3
"""Regenerate the companion lock-board schematic from this manifest.

    python3 scripts/lock.schgen.py   (or: make gen-lock)

This is the SECOND PCB in the ephemerkey repo (alongside hardware/ephemerkey/);
it shares this repo's engine (scripts/kschgen.py), Makefile, and lib tables, the
same way reefvolt-sensorbuddy carries both sensorbuddy and plugcontrol.

The lock is the *consumer* half of the ephemerkey system: it receives an emitted
TOTP/unlock request over an AUTHENTICATED I2C link (ephemerkey is the master,
this lock is the target), verifies it with a firmware HMAC challenge-response
(shared secret in MCU flash -- no secure element), and drives a 12 V solenoid
with a firmware peak-and-hold (economizer) profile from a single 1S Li-ion cell.

Power architecture (chosen for "off most of the time, draw as little as
possible"):
  - The ATtiny1616 controller runs DIRECTLY off the cell (1.8-5.5 V) -- NO LDO.
    It sleeps at ~0.1 uA in power-down and wakes on I2C bus activity (first START).
  - A MT3608 boost makes +12 V (VSOL) but is GATED OFF (SOL_BOOST_EN) except
    during an actuation, so there is no standing 12 V draw or switching noise.
  - A low-side AO3400A + SS34 flyback switches the coil; firmware does
    peak (~full duty, ~20-50 ms) then hold (reduced PWM duty) -- the economizer.

Authentication is firmware HMAC-SHA1 challenge-response over I2C (reuse
smalltotp's HMAC-SHA1 on both boards); the secret lives in ATtiny flash (protect
it with UPDI lockbits in production).

Places every part onto a child sheet (MCU / PWR / DRV), each resolving to a real
KiCad bundled symbol + footprint + JLCPCB LCSC, with a per-sheet wiring/pin note.
Components are PLACED, not wired -- wire them in eeschema using the notes as the
spec (regenerate BEFORE wiring; regen reassigns UUIDs).

0402 passives throughout; bulk/boost caps 0805/1206; reservoir is an electrolytic.
"""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import kschgen as K

HW = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))   # hardware/
PROJ_DIR = os.path.join(HW, "lock")
ROOT_UUID = "e1000000-0000-4000-8000-000000000002"   # keep stable across regens

# ---- symbol libraries (all KiCad bundled) -----------------------------------
K.register_stdlib("Device", "R", "C", "C_Polarized", "L", "LED", "D_Schottky")
K.register_stdlib("Regulator_Switching", "MT3608")
K.register_stdlib("Transistor_FET", "Q_NMOS_GSD")
K.register_stdlib("MCU_Microchip_ATtiny", "ATtiny1616-M")
K.register_stdlib("Connector_Generic", "Conn_01x02", "Conn_01x03", "Conn_01x04")

# ---- footprint shorthands ---------------------------------------------------
R0402 = "Resistor_SMD:R_0402_1005Metric"
C0402 = "Capacitor_SMD:C_0402_1005Metric"
C0805 = "Capacitor_SMD:C_0805_2012Metric"
C1206 = "Capacitor_SMD:C_1206_3216Metric"
LED0402 = "LED_SMD:LED_0402_1005Metric"
SOT23 = "Package_TO_SOT_SMD:SOT-23"
SOT236 = "Package_TO_SOT_SMD:SOT-23-6"
SMA = "Diode_SMD:D_SMA"
QFN20 = "Package_DFN_QFN:QFN-20-1EP_3x3mm_P0.4mm_EP1.65x1.65mm"
FNR6045 = "Inductor_SMD:L_Changjiang_FNR6045S"
ELEC637 = "Capacitor_SMD:CP_Elec_6.3x7.7"
JSTPH = "Connector_JST:JST_PH_S2B-PH-K_1x02_P2.00mm_Horizontal"
HDR3 = "Connector_PinHeader_2.54mm:PinHeader_1x03_P2.54mm_Vertical"
HDR4 = "Connector_PinHeader_2.54mm:PinHeader_1x04_P2.54mm_Vertical"

# JLCPCB LCSC for the common 0402 Basic passives
RLCSC = {"4.7k": "C25900", "10k": "C25744", "100k": "C25741",
         "200k": "C25764", "1k": "C11702", "100R": "C106232"}
CLCSC = {"100nF": "C1525", "1uF": "C29266", "10uF": "C15850", "22uF": "C12891"}


def R(ref, val, **kw):
    return dict(ref=ref, lib_id="Device:R", value=val, fp=R0402,
               lcsc=RLCSC.get(val, ""), **kw)


def C(ref, val, fp=C0402, **kw):
    return dict(ref=ref, lib_id="Device:C", value=val, fp=fp,
               lcsc=CLCSC.get(val, ""), **kw)


# ============================ MCU sheet ======================================
# ATtiny1616 (runs direct off BAT, no LDO) + firmware HMAC auth over an
# authenticated I2C target bus (ephemerkey = master) + UPDI programming header.
MCU = dict(name="MCU", file="mcu.kicad_sch",
    title="Controller (ATtiny1616) / firmware-HMAC auth / authenticated I2C (target)",
    page="2",
    big=[
        dict(ref="U1", lib_id="MCU_Microchip_ATtiny:ATtiny1616-M",
             value="ATtiny1616", fp=QFN20, lcsc="C507118",
             mpn="ATTINY1616-MNR", mfr="Microchip"),
        dict(ref="J2", lib_id="Connector_Generic:Conn_01x03",
             value="I2C IF (auth)", fp=HDR3),
        dict(ref="J4", lib_id="Connector_Generic:Conn_01x03",
             value="UPDI PROG", fp=HDR3),
    ],
    small=[
        C("C1", "100nF"), C("C2", "1uF"),                 # U1 VCC decouple/bulk
        dict(ref="D1", lib_id="Device:LED", value="STAT", fp=LED0402,
             lcsc="C160479", mpn="LTST-C281KGKT", mfr="Lite-On"),
        R("R1", "1k"),                                    # status LED series
    ])

# ============================ PWR sheet ======================================
# 1S Li-ion -> MT3608 boost -> +12V (VSOL), gated by SOL_BOOST_EN.
PWR = dict(name="PWR", file="psu.kicad_sch",
    title="Battery 1S Li-ion / 12V boost (gated)", page="3",
    big=[
        dict(ref="J1", lib_id="Connector_Generic:Conn_01x02",
             value="BAT 1S Li-ion", fp=JSTPH,
             lcsc="C173752", mpn="S2B-PH-K-S", mfr="JST"),
        dict(ref="U2", lib_id="Regulator_Switching:MT3608", value="MT3608",
             fp=SOT236, lcsc="C84817", mpn="MT3608", mfr="Aerosemi"),
    ],
    small=[
        dict(ref="L1", lib_id="Device:L", value="10uH", fp=FNR6045,
             lcsc="C168076", mpn="FNR6045S100MT", mfr="Changjiang"),
        dict(ref="D2", lib_id="Device:D_Schottky", value="SS34", fp=SMA,
             lcsc="C8678", mpn="SS34", mfr="MDD"),               # boost rectifier
        R("R3", "200k"), R("R4", "10k"),                  # FB divider -> ~12.6V
        R("R2", "100k"),                                  # EN pulldown (off at boot)
        C("C3", "10uF", C0805),                           # boost VIN
        C("C4", "22uF", C1206),                           # boost VOUT (VSOL)
    ])

# ============================ DRV sheet ======================================
# 12V solenoid peak-and-hold low-side driver + reservoir + flyback.
DRV = dict(name="DRV", file="drv.kicad_sch",
    title="12V solenoid driver (peak-and-hold)", page="4",
    big=[
        dict(ref="Q1", lib_id="Transistor_FET:Q_NMOS_GSD", value="AO3400A",
             fp=SOT23, lcsc="C20917", mpn="AO3400A", mfr="AOS"),
        dict(ref="J3", lib_id="Connector_Generic:Conn_01x02",
             value="SOLENOID 12V", fp=JSTPH,
             lcsc="C173752", mpn="S2B-PH-K-S", mfr="JST"),
    ],
    small=[
        dict(ref="C5", lib_id="Device:C_Polarized", value="220uF 25V", fp=ELEC637,
             lcsc="C2918361", mpn="RVT1E221M0607", mfr="Rubycon-alt"),  # reservoir
        C("C6", "22uF", C1206),                           # VSOL HF bypass
        dict(ref="D3", lib_id="Device:D_Schottky", value="SS34", fp=SMA,
             lcsc="C8678", mpn="SS34", mfr="MDD"),               # coil flyback
        R("R5", "100R"),                                  # gate series
        R("R6", "100k"),                                  # gate pulldown (off-safe)
        # optional drain snubber -- fit at bring-up only if ringing is high
        R("R9", "10R", dnp=True),
        dict(ref="C7", lib_id="Device:C", value="1nF", fp=C0402, dnp=True),
    ])

# ============================ wiring notes (pinout guides) ====================
MCU["note"] = (12, 140, """Controller — ATtiny1616 (QFN-20) + firmware HMAC auth + I2C target (ephemerkey = master; wake-on-I2C).  PLACED, not wired.
U1 ATtiny1616 (runs DIRECT off BAT 1.8-5.5V -- NO LDO; ~0.1uA power-down):
  pin fn               net                       pin fn        net
   4  VCC              BAT+  (C1 100nF, C2 1uF)    3  GND       GND   (+ EP pin 21 -> GND)
  19  PA0/RESET        UPDI -> J4 (program)        5  PA4       spare
  14  PB0 TWI0 SCL     <- J2.2 (I2C clk + WAKE)    6  PA5 TCB0 WO  SOL_PWM -> DRV Q1 gate
  13  PB1 TWI0 SDA     <> J2.3 (I2C data)          7  PA6       SOL_BOOST_EN -> PWR U2.EN
   2  PA3              STATUS LED D1 + R1 1k      ..  spare     PA1,PA2,PA4,PA7,PB2,PB3,PB4,PB5,PC0..PC3
I2C: lock = TARGET (addr 0x60); ephemerkey = MASTER ("ephemerkey drives"). Bus pull-ups are on the MASTER side
  at +3V3 (NOT on the lock) -- the lock runs at VBAT and its TWI pins are open-drain / sink-only, so master-side
  3V3 pull-ups avoid the 3V3/VBAT cross-domain (target reads a 3V3 high fine: VIH ~0.7*VBAT). Short cable, ~100kHz.
WAKE-ON-I2C (NO discrete wake / "button" line): in power-down the lock arms a pin-change interrupt on SCL (PB0);
  the master's first START wakes it -> firmware disables the pin-int and enables TWI0 as target. The just-woken
  target NACKs the very first address, so the master sends a dummy/wake xfer then retries (or clock-stretches).
AUTHENTICATION (firmware HMAC on U1 -- NO secure element): a shared secret in ATtiny flash backs an
  HMAC-SHA1 challenge-response over I2C (J2): master READS a fresh random nonce from the lock, then WRITES
  HMAC(secret, nonce[||code]); lock recomputes and compares constant-time. Anti-replay via the nonce.
  HMAC-SHA1 reuses smalltotp on BOTH boards and fits easily in 16KB flash / 2KB SRAM (HMAC-SHA1 stays
  sound -- it does not rely on SHA1 collision resistance). Protect the secret: disable UPDI / set lockbits
  in production so flash cannot be read back.
J2 I2C INTERFACE (authenticated, to ephemerkey master) 1x3: 1 GND  2 SCL (+ wake)  3 SDA.   (No discrete wake line.)
J4 UPDI PROGRAM 1x3: 1 UPDI (PA0)  2 VCC (BAT)  3 GND.  1-wire UPDI: pymcuprog / megaTinyCore / Atmel-ICE / Serial-UPDI.
SLEEP: U1 power-down ~0.1uA; SCL START (pin-int on PB0) wakes it -> TWI target on -> authenticate -> boost on -> drive -> sleep.""")

PWR["note"] = (12, 120, """Power — BAT 1S Li-ion -> MT3608 boost -> +12V (VSOL).  Boost GATED OFF except during actuation.  PLACED, not wired.
J1 BAT 1S (JST-PH): 1 = BAT+   2 = GND   (protected 1S Li-ion pack, 3.0-4.2V).
   BAT+ ALSO powers U1 directly (MCU sheet) -- there is no LDO; the ATtiny runs on the raw cell.
U2 MT3608 (SOT-23-6 boost):
   1 SW  -> L1 / D2 anode        4 EN  <- SOL_BOOST_EN  (R2 100k pulldown = OFF at boot / MCU asleep)
   2 GND -> GND                  5 IN  <- BAT+  (C3 10uF)
   3 FB  <- R3/R4 node           6 NC
   BOOST: BAT+ -> L1 10uH -> SW(1) ; SW -> D2 SS34 -> VSOL(+12V) ; C4 22uF on VSOL.
   FB divider: VSOL -> R3 200k -> FB -> R4 10k -> GND.  Vout = 0.6*(1 + R3/R4) = 12.6V (trim R3 toward 12V).
SIZING: MT3608 ~2A switch ~= ~0.5A @12V continuous -> sized for HOLD current + reservoir recharge.
   A sustained 1A @12V pull-in exceeds this -> use a bigger boost (e.g. TPS61088) + larger reservoir; see DESIGN.md.""")

DRV["note"] = (12, 95, """Solenoid driver — 12V peak-and-hold, low-side N-FET.  PLACED, not wired.
RAIL: VSOL(+12V) from PWR sheet.  C5 220uF (electrolytic) + C6 22uF reservoir on VSOL -> supplies the pull-in surge.
SOLENOID J3 (JST-PH): 1 = VSOL(+12V)   2 = SOL_DRV (Q1 drain).
D3 SS34 FLYBACK across the coil: anode = SOL_DRV, cathode = VSOL  (clamps the coil's collapse to VSOL + ~0.5V).
Q1 AO3400A low-side N-FET (SOT-23, logic-level): 1 G <- R5 100R <- SOL_PWM ;  2 S -> GND ;  3 D -> SOL_DRV.
   R6 100k gate->GND: holds Q1 OFF through reset / power-up / while the MCU sleeps (the lock cannot self-fire).
PEAK-AND-HOLD (firmware on U1): SOL_BOOST_EN=1 -> wait VSOL settle -> SOL_PWM 100% pull-in (~20-50 ms)
   -> drop PWM duty for HOLD (~1/3, tune to the coil) -> on release SOL_PWM=0 then SOL_BOOST_EN=0.
SNUBBER (optional, DNP): R9 10R + C7 1nF from SOL_DRV -> GND; fit at bring-up only if drain ringing is high.""")


# ============================ generate =======================================
K.build(
    project="lock", proj_dir=PROJ_DIR, root_uuid=ROOT_UUID,
    title=dict(title="lock", date="2026-06-28", rev="0.1",
               company="EphemerKey Authors",
               comments=["Companion TOTP lock — authenticated 12V solenoid driver",
                         "ATtiny1616 (firmware HMAC); 1S Li-ion; peak-and-hold economizer"]),
    sheets=[MCU, PWR, DRV],
)
