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
K.register_stdlib("Transistor_FET", "Q_NMOS_GSD", "Q_PMOS_GSD")
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
JSTPH4 = "Connector_JST:JST_PH_S4B-PH-K_1x04_P2.00mm_Horizontal"  # I2C, right-angle
JSTPH3 = "Connector_JST:JST_PH_S3B-PH-K_1x03_P2.00mm_Horizontal"  # hall sensors, right-angle
HDR3 = "Connector_PinHeader_2.54mm:PinHeader_1x03_P2.54mm_Vertical"
HDR4 = "Connector_PinHeader_2.54mm:PinHeader_1x04_P2.54mm_Vertical"

# JLCPCB LCSC for the common 0402 Basic passives
RLCSC = {"4.7k": "C25900", "10k": "C25744", "100k": "C25741",
         "200k": "C25764", "1k": "C11702", "100R": "C106232", "0R": "C17168",
         "22k": "C25768", "20k": "C25765"}
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
        dict(ref="J2", lib_id="Connector_Generic:Conn_01x04",
             value="I2C JST-PH RA", fp=JSTPH4,
             lcsc="C157926", mpn="S4B-PH-K-S", mfr="JST"),
        dict(ref="J4", lib_id="Connector_Generic:Conn_01x03",
             value="UPDI PROG", fp=HDR3),
        dict(ref="J6", lib_id="Connector_Generic:Conn_01x03",
             value="HALL DOOR", fp=JSTPH3,
             lcsc="C157929", mpn="S3B-PH-K-S", mfr="JST"),
        dict(ref="J7", lib_id="Connector_Generic:Conn_01x03",
             value="HALL BOLT", fp=JSTPH3,
             lcsc="C157929", mpn="S3B-PH-K-S", mfr="JST"),
    ],
    small=[
        C("C1", "100nF"), C("C2", "1uF"),                 # U1 VCC decouple/bulk
        dict(ref="D1", lib_id="Device:LED", value="STAT", fp=LED0402,
             lcsc="C160479", mpn="LTST-C281KGKT", mfr="Lite-On"),
        R("R1", "1k"),                                    # status LED series
        # door/bolt hall sensors (powered from HALL_PWR GPIO -> ~0uA when asleep)
        R("R22", "10k"), R("R23", "10k"),                 # hall OUT pull-ups -> HALL_PWR
        C("C9", "100nF"), C("C10", "100nF"),              # hall debounce / ESD
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
        R("R3", "200k"), R("R4", "22k"),                  # FB base -> 6V default
        R("R2", "100k"),                                  # EN pulldown (off at boot)
        C("C3", "10uF", C0805),                           # boost VIN
        C("C4", "22uF", C1206),                           # boost VOUT (VSOL)
        # firmware boost-select: Q2 switches R17 in parallel with R4 -> ~12V
        dict(ref="Q2", lib_id="Transistor_FET:Q_NMOS_GSD", value="AO3400A",
             fp=SOT23, lcsc="C20917", mpn="AO3400A", mfr="AOS"),
        R("R17", "20k"),                                  # FB parallel (12V when Q2 on)
        R("R18", "100k"),                                 # BOOST_VSEL pulldown (default 6V)
    ])

# ============================ DRV sheet ======================================
# 12V solenoid peak-and-hold low-side driver + reservoir + flyback.
DRV = dict(name="DRV", file="drv.kicad_sch",
    title="Actuator: solenoid (12V peak-hold) OR servo (Vbat / 6V) -- build one", page="4",
    big=[
        dict(ref="Q1", lib_id="Transistor_FET:Q_NMOS_GSD", value="AO3400A",
             fp=SOT23, lcsc="C20917", mpn="AO3400A", mfr="AOS"),
        dict(ref="J3", lib_id="Connector_Generic:Conn_01x02",
             value="SOLENOID 12V", fp=JSTPH,
             lcsc="C173752", mpn="S2B-PH-K-S", mfr="JST"),
        dict(ref="J5", lib_id="Connector_Generic:Conn_01x03",
             value="SERVO S/V+/GND", fp=HDR3),
        dict(ref="J8", lib_id="Connector_Generic:Conn_01x03",
             value="SERVO2 S/V+/GND", fp=HDR3),
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
        # --- servo option (parallel actuator; build as solenoid OR servo) ---
        R("R13", "0R"),                                   # VSERVO_SRC <- VBAT (1S servo)
        R("R14", "0R", dnp=True),                         # VSERVO_SRC <- VSOL (6V servo)
        R("R15", "1k"),                                   # servo signal series
        R("R16", "10k"),                                  # servo signal idle pulldown
        dict(ref="C8", lib_id="Device:C_Polarized", value="220uF 25V", fp=ELEC637,
             lcsc="C2918361", mpn="RVT1E221M0607", mfr="Rubycon-alt"),  # VSERVO bulk
        # VSERVO high-side load switch (P-FET) + interlock to BOOST_VSEL
        dict(ref="Q3", lib_id="Transistor_FET:Q_PMOS_GSD", value="AO3401A",
             fp=SOT23, lcsc="C15127", mpn="AO3401A", mfr="AOS"),   # servo high-side P-FET
        dict(ref="Q4", lib_id="Transistor_FET:Q_NMOS_GSD", value="AO3400A",
             fp=SOT23, lcsc="C20917", mpn="AO3400A", mfr="AOS"),   # P-FET gate driver
        dict(ref="Q5", lib_id="Transistor_FET:Q_NMOS_GSD", value="AO3400A",
             fp=SOT23, lcsc="C20917", mpn="AO3400A", mfr="AOS"),   # interlock (VSEL 12V -> off)
        R("R19", "100k"),                                 # Q3 gate pull-up (default off)
        R("R20", "100k"),                                 # SERVO_PWR_EN node pulldown
        R("R21", "10k"),                                  # SERVO_PWR_EN series (Q5 override)
        R("R24", "1k"),                                   # servo2 signal series
        R("R25", "10k"),                                  # servo2 signal idle pulldown
    ])

# ============================ wiring notes (pinout guides) ====================
MCU["note"] = (12, 140, """MCU — ATtiny1616 (QFN-20) controller.  Pinout / nets (PLACED, not wired).  Runs off BAT, no LDO.
 pin name        net                              pin name   net
  4 VCC          BAT+  (C1, C2 to GND)             3 GND      GND  (+ EP pin 21)
 19 PA0/RESET    UPDI = J4.1                        5 PA4      HALL_PWR -> J6.1, J7.1, R22, R23
 14 PB0 SCL      = J2.4   (I2C clk + wake)          6 PA5      SOL_PWM -> DRV R5
 13 PB1 SDA      = J2.3   (I2C data)                7 PA6      SOL_BOOST_EN -> PWR U2.EN
  2 PA3          LED: PA3 -> D1 -> R1 -> GND        8 PB2      SERVO_SIG  -> DRV R15
 20 PA1          BOOST_VSEL -> PWR Q2 + DRV Q5      1 PA2      SERVO_PWR_EN -> DRV R21
  8 PA7          HALL_DOOR  <- J6.3                11 PB3      HALL_BOLT  <- J7.3
                 PB4  SERVO_SIG2 -> DRV R24            spare:  PB5, PC0, PC1, PC2, PC3
PASSIVES:  C1 100nF VCC--GND    C2 1uF VCC--GND    D1 LED + R1 1k:  PA3 -- D1 -- R1 -- GND
J2 I2C   (S4B-PH-K 4-pin):  1 = GND   2 = VCC (No-Connect)   3 = SDA (PB1)   4 = SCL (PB0)
J4 UPDI  (1x3 header):      1 = UPDI (PA0)   2 = VCC (BAT)   3 = GND
J6 HALL DOOR (S3B 3-pin):   1 = HALL_PWR   2 = GND   3 = OUT -> PA7 ;  R22 10k OUT--HALL_PWR ;  C9  100nF OUT--GND
J7 HALL BOLT (S3B 3-pin):   1 = HALL_PWR   2 = GND   3 = OUT -> PB3 ;  R23 10k OUT--HALL_PWR ;  C10 100nF OUT--GND""")

PWR["note"] = (12, 120, """PWR — battery 1S + MT3608 boost.  Pinout / nets (PLACED, not wired).
J1 BAT 1S   1 = BAT+    2 = GND        (JST-PH; BAT+ also powers U1/MCU, and feeds DRV)
U2 MT3608   1 SW    2 GND    3 FB    4 EN    5 IN    6 NC
L1 10uH     BAT+ -- SW (U2.1)                       D2 SS34:  A = SW    K = VSOL(+12V)
C3 10uF     1 = BAT+ (= U2 IN)   2 = GND            C4 22uF:  1 = VSOL   2 = GND
R3 200k     VSOL -- FB                              (FB top)
R4 22k      FB -- GND                               (FB base -> 6V)
R17 20k     FB -- Q2.D                              (switched in -> 12V)
Q2 AO3400   1 G = BOOST_VSEL   2 S = GND   3 D = R17    (FB switch: 6V <-> 12V)
R2 100k     U2.EN -- GND                            R18 100k:  BOOST_VSEL -- GND   (default OFF / 6V)
NETS IN:  U2.EN <- SOL_BOOST_EN (PA6) ;  BOOST_VSEL <- PA1.    OUT:  BAT+, VSOL -> DRV.
Vout = 0.6*(1 + R3/Rbot):  Rbot = R4 -> 6V ;  R4 || R17 (Q2 on) -> 12V.""")

DRV["note"] = (12, 95, """DRV — solenoid + dual-servo driver.  Pinout / nets (PLACED, not wired).  Tie pins that share a net name.
NETS IN:  VSOL(+12V), BAT+ <- PWR ;  SOL_PWM(PA5) SERVO_SIG(PB2) SERVO_SIG2(PB4) SERVO_PWR_EN(PA2) BOOST_VSEL(PA1) <- MCU
Q1  AO3400A   1 G = Q1G        2 S = GND          3 D = SOL_DRV       (solenoid low-side)
J3  SOLENOID  1 = VSOL         2 = SOL_DRV
D3  SS34      A = SOL_DRV      K = VSOL                               (flyback across the coil)
C5  220uF     + = VSOL    - = GND        C6 22uF:  1 = VSOL    2 = GND        (reservoir)
R5  100R      SOL_PWM -- Q1G             R6 100k:  Q1G -- GND     (gate drive + off-safe pulldown)
R9* 10R       SOL_DRV -- C7.1            C7* 1nF:  C7.2 -- GND    (*DNP drain snubber)
R13 0R        BAT+ -- VSERVO_SRC   |   R14* 0R:  VSOL -- VSERVO_SRC   (fit ONE: BAT+ = 1S servo, VSOL = 6V servo)
Q3  AO3401A   1 G = Q3G        2 S = VSERVO_SRC   3 D = VSERVO        (servo high-side switch)
R19 100k      Q3G -- VSERVO_SRC                                      (Q3 gate pull-up = default OFF)
Q4  AO3400A   1 G = ENNODE     2 S = GND          3 D = Q3G          (pulls Q3 ON when ENNODE high)
R20 100k      ENNODE -- GND               R21 10k:  SERVO_PWR_EN -- ENNODE
Q5  AO3400A   1 G = BOOST_VSEL  2 S = GND   3 D = ENNODE             (INTERLOCK: 12V -> servo power OFF)
C8  220uF     + = VSERVO   - = GND                                   (servo bulk)
J5  SERVO     1 = SIG1   2 = VSERVO   3 = GND    R15 1k:  SERVO_SIG  -- SIG1   R16 10k:  SIG1 -- GND
J8  SERVO2    1 = SIG2   2 = VSERVO   3 = GND    R24 1k:  SERVO_SIG2 -- SIG2   R25 10k:  SIG2 -- GND""")


# ============================ generate =======================================
K.build(
    project="lock", proj_dir=PROJ_DIR, root_uuid=ROOT_UUID,
    title=dict(title="lock", date="2026-06-28", rev="0.1",
               company="EphemerKey Authors",
               comments=["Companion TOTP lock — authenticated 12V solenoid driver",
                         "ATtiny1616 (firmware HMAC); 1S Li-ion; peak-and-hold economizer"]),
    sheets=[MCU, PWR, DRV],
)
