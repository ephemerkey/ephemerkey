#!/usr/bin/env python3
"""Regenerate the ephemerkey hierarchical schematic from this manifest.

    python3 scripts/ephemerkey.schgen.py   (or: make gen-ephemerkey)

Places every part (DESIGN.md "KiCad Library Map" + power/charger subsystem) onto
a child sheet (MCU / PSU / GNSS / Sensors), each resolving to a real KiCad
bundled symbol + footprint + JLCPCB LCSC, with a per-sheet wiring/pin note.
Components are PLACED, not wired — wire them in eeschema using the notes as the
spec (regenerate BEFORE wiring; regen reassigns UUIDs).

This is DATA; the engine is scripts/kschgen.py. 0402 passives throughout; bulk
caps 0805. JLCPCB Basic parts preferred where available.
"""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import kschgen as K

HW = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))   # hardware/
PROJ_DIR = os.path.join(HW, "ephemerkey")
ROOT_UUID = "e1000000-0000-4000-8000-000000000001"   # keep stable across regens

# ---- symbol libraries (all KiCad bundled) -----------------------------------
K.register_stdlib("Device", "R", "C", "L", "LED", "Crystal", "D_Schottky",
                  "Antenna_Chip")
K.register_stdlib("MCU_ST_STM32U0", "STM32U083KCUx")
K.register_stdlib("RF_GPS", "MAX-M10S")
K.register_stdlib("Regulator_Switching", "TPS63900")
K.register_stdlib("Sensor_Motion", "LIS3DH")
K.register_stdlib("Battery_Management", "MCP73831-2-OT")
K.register_stdlib("Power_Protection", "USBLC6-2SC6")
K.register_stdlib("Transistor_FET", "Q_PMOS_GSD")
K.register_stdlib("Switch", "SW_Push")
K.register_stdlib("Connector", "USB_C_Receptacle_USB2.0_16P",
                  "Conn_ARM_SWD_TagConnect_TC2030-NL")
K.register_stdlib("Connector_Generic", "Conn_01x02", "Conn_01x04")

# ---- footprint shorthands ---------------------------------------------------
R0402 = "Resistor_SMD:R_0402_1005Metric"
C0402 = "Capacitor_SMD:C_0402_1005Metric"
C0805 = "Capacitor_SMD:C_0805_2012Metric"
L0402 = "Inductor_SMD:L_0402_1005Metric"
LED0402 = "LED_SMD:LED_0402_1005Metric"
SOT235 = "Package_TO_SOT_SMD:SOT-23-5"
SOT236 = "Package_TO_SOT_SMD:SOT-23-6"
SOT23 = "Package_TO_SOT_SMD:SOT-23"
SOD123 = "Diode_SMD:D_SOD-123"
BTN = "ephemerkey:SW_Push_1P1T_XKB_TS-1187A"

# JLCPCB LCSC for the common 0402 Basic passives
RLCSC = {"5.1k": "C25905", "4.7k": "C25900", "10k": "C25744",
         "100k": "C25741", "1k": "C11702", "0R": "C17168"}
CLCSC = {"100nF": "C1525", "1uF": "C29266", "12pF": "C1547", "10uF": "C15850"}


def R(ref, val):
    return dict(ref=ref, lib_id="Device:R", value=val, fp=R0402,
               lcsc=RLCSC.get(val, ""))


def C(ref, val, fp=C0402):
    return dict(ref=ref, lib_id="Device:C", value=val, fp=fp,
               lcsc=CLCSC.get(val, ""))


# ============================ MCU sheet ======================================
MCU = dict(name="MCU", file="mcu.kicad_sch", title="MCU / RTC / Programming",
    page="2",
    big=[
        dict(ref="U1", lib_id="MCU_ST_STM32U0:STM32U083KCUx", value="STM32U083KCU6",
             fp="Package_DFN_QFN:UFQFPN-32-1EP_5x5mm_P0.5mm_EP3.5x3.5mm",
             lcsc="C22459164", mpn="STM32U083KCU6", mfr="STMicroelectronics"),
        dict(ref="J1", lib_id="Connector:Conn_ARM_SWD_TagConnect_TC2030-NL",
             value="SWD TC2030-NL",
             fp="Connector:Tag-Connect_TC2030-IDC-NL_2x03_P1.27mm_Vertical"),
        dict(ref="J2", lib_id="Connector_Generic:Conn_01x04", value="LOCK OUT",
             fp="Connector_PinHeader_2.54mm:PinHeader_1x04_P2.54mm_Vertical"),
    ],
    small=[
        dict(ref="Y1", lib_id="Device:Crystal", value="32.768kHz",
             fp="Crystal:Crystal_SMD_3215-2Pin_3.2x1.5mm",
             lcsc="C32346", mpn="Q13FC13500004", mfr="Epson"),
        C("C1", "12pF"), C("C2", "12pF"),               # LSE load caps
        C("C3", "100nF"), C("C4", "100nF"), C("C5", "100nF"),  # VDD/VDDA/VDDUSB
        C("C6", "10uF", C0805),                          # 3V3 bulk
        C("C7", "1uF"), C("C8", "100nF"),                # VDDA/VREF+ filter
        C("C9", "100nF"),                                # NRST cap
        R("R1", "10k"),                                  # BOOT0 pulldown
        dict(ref="SW1", lib_id="Switch:SW_Push", value="USER1", fp=BTN,
             lcsc="C318884", mpn="TS-1187A-B-A-B"),
        dict(ref="SW2", lib_id="Switch:SW_Push", value="USER2", fp=BTN,
             lcsc="C318884", mpn="TS-1187A-B-A-B"),
        dict(ref="SW3", lib_id="Switch:SW_Push", value="USER3/DFU", fp=BTN,
             lcsc="C318884", mpn="TS-1187A-B-A-B"),
        dict(ref="D1", lib_id="Device:LED", value="GRN", fp=LED0402,
             lcsc="C160479", mpn="LTST-C281KGKT", mfr="Lite-On"),
        R("R2", "1k"),
        dict(ref="D2", lib_id="Device:LED", value="RED", fp=LED0402,
             lcsc="C130719", mpn="NCD0402R1"),
        R("R3", "1k"),
    ])

# ============================ PSU sheet ======================================
PSU = dict(name="PSU", file="psu.kicad_sch",
    title="USB-C / Li-ion charge / load-share / buck-boost", page="3",
    big=[
        dict(ref="J3", lib_id="Connector:USB_C_Receptacle_USB2.0_16P", value="USB-C",
             fp="Connector_USB:USB_C_Receptacle_GCT_USB4105-xx-A_16P_TopMnt_Horizontal",
             lcsc="C2927039", mpn="USB-TYPE-C-019", mfr="GCT"),
        dict(ref="J4", lib_id="Connector_Generic:Conn_01x02", value="BAT 1S Li-ion",
             fp="Connector_JST:JST_PH_S2B-PH-K_1x02_P2.00mm_Horizontal",
             lcsc="C173752", mpn="S2B-PH-K-S", mfr="JST"),
        dict(ref="U2", lib_id="Regulator_Switching:TPS63900", value="TPS63900DSKR",
             fp="Package_SON:WSON-10-1EP_2.5x2.5mm_P0.5mm_EP1.2x2mm",
             lcsc="C1518762", mpn="TPS63900DSKR", mfr="Texas Instruments"),
    ],
    small=[
        dict(ref="U3", lib_id="Power_Protection:USBLC6-2SC6", value="USBLC6-2SC6",
             fp=SOT236, lcsc="C2687116", mpn="USBLC6-2SC6", mfr="STMicroelectronics"),
        R("R4", "5.1k"), R("R5", "5.1k"),                # CC1/CC2 (sink)
        dict(ref="U4", lib_id="Battery_Management:MCP73831-2-OT",
             value="MCP73831-2-OT", fp=SOT235, lcsc="C424093",
             mpn="MCP73831T-2ACI/OT", mfr="Microchip"),
        R("R6", "4.7k"),                                 # PROG: ~213mA (0.5C of 500mAh)
        C("C10", "10uF", C0805), C("C11", "10uF", C0805),  # charger in/out
        dict(ref="Q1", lib_id="Transistor_FET:Q_PMOS_GSD", value="AO3401A",
             fp=SOT23, lcsc="C15127", mpn="AO3401A", mfr="AOS"),
        dict(ref="D3", lib_id="Device:D_Schottky", value="B5819W", fp=SOD123,
             lcsc="C8598", mpn="B5819W", mfr="Slkor"),
        R("R7", "100k"),                                 # load-share gate pulldown
        dict(ref="L1", lib_id="Device:L", value="2.2uH",
             fp="Inductor_SMD:L_Changjiang_FNR3015S",
             lcsc="C167747", mpn="FNR3015S2R2MT", mfr="Changjiang"),
        C("C12", "10uF", C0805), C("C13", "10uF", C0805), C("C14", "10uF", C0805),  # buck-boost in/out
        C("C15", "10uF", C0805), C("C16", "10uF", C0805),  # VBUS / VBAT bulk
        dict(ref="D4", lib_id="Device:LED", value="CHG", fp=LED0402,
             lcsc="C130719", mpn="NCD0402R1"),
        R("R8", "1k"),
    ])

# ============================ GNSS sheet =====================================
GNSS = dict(name="GNSS", file="gnss.kicad_sch", title="GNSS (MAX-M10S + antenna)",
    page="4",
    big=[
        dict(ref="MD1", lib_id="RF_GPS:MAX-M10S", value="MAX-M10S-00B",
             fp="ephemerkey:ublox_MAX", lcsc="C4153167", mpn="MAX-M10S-00B", mfr="u-blox"),
        dict(ref="AE1", lib_id="Device:Antenna_Chip", value="W3011A",
             fp="RF_Antenna:Pulse_W3011", lcsc="C5830926", mpn="W3011A",
             mfr="Pulse Electronics"),
    ],
    small=[
        dict(ref="L2", lib_id="Device:L", value="FB 600R@100MHz", fp=L0402,
             lcsc="C76884", mpn="BLM15AG601SN1D", mfr="Murata"),   # VCC_RF ferrite
        C("C17", "100nF"), dict(ref="C18", lib_id="Device:C", value="10pF", fp=C0402),
        C("C19", "10uF", C0805), C("C20", "100nF"),       # MAX-M10S VCC
        C("C21", "100nF"),                                # V_BCKP
        C("C22", "100nF"),                                # V_IO
        # W3011A pi-match: series populated 0R, two shunts DNP (tune at bring-up)
        R("Rm1", "0R"),
        dict(ref="Cm2", lib_id="Device:C", value="DNP", fp=C0402, dnp=True),
        dict(ref="Cm3", lib_id="Device:C", value="DNP", fp=C0402, dnp=True),
    ])

# ============================ Sensors sheet ==================================
SENSORS = dict(name="Sensors", file="sensors.kicad_sch",
    title="Accelerometer (LIS3DH)", page="5", big=[],
    small=[
        dict(ref="U5", lib_id="Sensor_Motion:LIS3DH", value="LIS3DHTR",
             fp="Package_LGA:LGA-16_3x3mm_P0.5mm", lcsc="C15134",
             mpn="LIS3DHTR", mfr="STMicroelectronics"),
        C("C23", "100nF"), C("C24", "100nF"),             # VDD / VDD_IO
        R("R9", "4.7k"), R("R10", "4.7k"),                # I2C1 pull-ups
    ])

# ============================ wiring notes (pinout guides) ====================
MCU["note"] = (12, 158, """MCU / RTC / Programming — pinout (U1 STM32U083KCU6, UFQFPN-32).  PLACED, not wired.
 pin  name            net / function           pin  name            net / function
  1   VDD             +3V3  (C3 100nF)           17  VDDUSB          +3V3  (C5 100nF)
  2   PC14/OSC32_IN   Y1 LSE 32.768kHz           18  PA8             GNSS_EN  -> GNSS
  3   PC15/OSC32_OUT  Y1 LSE 32.768kHz           19  PA9             USART1_TX -> GNSS RXD
  4   PF2/NRST        NRST (C9 100nF, J1)        20  PA10            USART1_RX <- GNSS TXD
  5   VDDA/VREF+      +3V3 (C7 1uF, C8 100nF)    21  PA11            USB_DM   (<- U3 ESD)
  6   PA0             GNSS_PPS (TIM2_CH1 in)     22  PA12            USB_DP   (<- U3 ESD)
  7   PA1             GNSS_EXTINT (out)          23  PA13            SWDIO (J1)
  8   PA2             ACC_INT1 (EXTI wake)       24  PA14            SWCLK (J1)
  9   PA3             ACC_INT2 (EXTI tamper)     25  PA15            BTN2 SW2 (pull-up)
 10   PA4             GNSS_RESET_N (OD out)      26  PB3             spare
 11   PA5             BTN1 SW1 (pull-up->GND)    27  PB4             spare
 12   PA6             LED_GRN  D1 + R2 1k        28  PB5             spare
 13   PA7             LED_RED  D2 + R3 1k        29  PB6             I2C1_SCL -> U5
 14   PB0             LOCK_TX  -> J2.2           30  PB7             I2C1_SDA -> U5
 15   PB1             CODE_VALID (OD) -> J2.3    31  PF3/BOOT0       BTN3 SW3 + DFU
 16   VSS             GND                        32  VSS  / EP       GND
RTC:  Y1 32.768kHz across PC14/PC15; C1,C2 12pF load caps (match to Y1 CL; trim via RTC SMOOTHCALIB).
PWR:  +3V3 from PSU sheet, C6 10uF bulk.   J1 = SWD TC2030-NL: SWDIO, SWCLK, NRST, +3V3, GND.
BTN:  3 user buttons. SW1->PA5, SW2->PA15 active-low (MCU pull-ups, to GND).
      SW3->PF3/BOOT0 active-HIGH to +3V3 (R1 10k pulldown = default boot-from-flash).
      Hold SW3 at reset (NRST via J1 / power cycle) -> ROM bootloader -> USB DFU over USB-C
      (STM32U0 supports USB DFU, AN2606; crystal-less USB via HSI48+CRS).
J2 LOCK OUT (1x4):  1 = +3V3   2 = LOCK_TX   3 = CODE_VALID   4 = GND   -> companion lock board.""")

PSU["note"] = (12, 158, """Power — pinout (USB-C -> charge -> load-share -> buck-boost).  PLACED, not wired.
J3  USB-C 16P:   VBUS -> VBUS_5V;  GND -> GND;  CC1 -> R4 5.1k -> GND;  CC2 -> R5 5.1k -> GND
                 D+ -> U3 -> USB_DP (PA12);  D- -> U3 -> USB_DM (PA11);  SBU1/2 = NC;  SHIELD -> GND
U3  USBLC6-2SC6:  1 I/O1(D+ conn)   2 GND   3 I/O2(D- conn)   4 I/O2(D- MCU)   5 VBUS   6 I/O1(D+ MCU)
U4  MCP73831-2-OT (SOT-23-5):  1 STAT -> D4 + R8 1k    2 VSS -> GND    3 VBAT -> BAT+ (C11 10uF)
                               4 VDD <- VBUS_5V (C10 10uF)   5 PROG -> R6 4.7k -> GND  (Ichg ~213mA = 0.5C/500mAh)
Q1  AO3401A load-share:  G -> VBUS_5V & R7 100k -> GND;  S -> BAT+;  D -> VSYS.   D3 B5819W: VBUS_5V -> VSYS.
                         => VSYS = VBUS (USB in: Q1 off, battery charges) | BAT (USB out: Q1 on)
U2  TPS63900 (WSON-10):  1 EN -> VIN    2 SEL -> GND    3/4/5 CFG1/2/3 strap = 3.3V    6 VOUT -> +3V3
                         7 LX2   9 LX1  (L1 2.2uH across LX1-LX2)    8 GND    10 VIN <- VSYS    11 EP -> GND
                         C12 on VIN; C13, C14 on +3V3 (10uF).   C15 VBUS bulk; C16 VBAT bulk.
J4  JST-PH (RA):  1 = BAT+    2 = GND    (1S Li-ion, protected pack).""")

GNSS["note"] = (12, 150, """GNSS — pinout (MD1 MAX-M10S-00B, AE1 W3011A).  PLACED, not wired.
MD1 MAX-M10S (18-pin LCC):
  1  GND        GND                  10  GND        GND
  2  TXD        -> PA10 (NMEA)       11  RF_IN      <- match <- AE1
  3  RXD        <- PA9  (NMEA)       12  GND        GND
  4  TIMEPULSE  -> PA0 (1PPS)        13  LNA_EN     NC (passive antenna)
  5  EXTINT     <- PA1               14  VCC_RF     L2 ferrite + C17 100nF / C18 10pF
  6  V_BCKP     +3V3 always-on, C21  15  VIO_SEL    open = 3.3V
  7  V_IO       +3V3, C22 100nF      16  SDA        NC (UART used)
  8  VCC        +3V3, C19 10uF/C20   17  SCL        NC
  9  RESET_N    <- PA4 (OD)          18  SAFEBOOT_N pull per datasheet
ANTENNA:  AE1 W3011A feed -> Rm1 (0R series) -> 50 ohm CPWG -> MD1 RF_IN.   Cm2/Cm3 shunt = DNP (tune w/ VNA).
          Honor the 4.0 x 6.25 mm ground keep-out under/around AE1; reserve a board corner.""")

SENSORS["note"] = (12, 70, """Sensors — pinout (U5 LIS3DHTR, LGA-16).  PLACED, not wired.
U5 LIS3DH:
  1  VDD_IO   +3V3 (C24 100nF)        9  INT2     -> PA3 (tamper / free-fall)
  2  NC                              10  RES      -> GND
  3  NC                              11  INT1     -> PA2 (wake-on-motion)
  4  SCL      -> PB6 (I2C1)          12  GND      GND
  5  GND      GND                    13  ADC3     NC
  6  SDA      -> PB7 (I2C1)          14  VDD      +3V3 (C23 100nF)
  7  SDO/SA0  addr 0x18(GND)/0x19    15  ADC2     NC
  8  CS       +3V3 (force I2C)       16  ADC1     NC
I2C1:  R9/R10 4.7k pull-ups to +3V3.   Tamper (INT2) may zeroize the TOTP secret (DESIGN.md Security).""")


# ============================ generate =======================================
K.build(
    project="ephemerkey", proj_dir=PROJ_DIR, root_uuid=ROOT_UUID,
    title=dict(title="ephemerkey", date="2026-06-21", rev="0.1",
               company="EphemerKey Authors",
               comments=["GPS-geofenced TOTP (RFC 6238) generator",
                         "Top-level — hierarchical sheets per subsystem"]),
    sheets=[MCU, PSU, GNSS, SENSORS],
)
