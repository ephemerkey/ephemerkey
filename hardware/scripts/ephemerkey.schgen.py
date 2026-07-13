#!/usr/bin/env python3
"""Regenerate the ephemerkey hierarchical schematic from this manifest.

    python3 scripts/ephemerkey.schgen.py   (or: make gen-ephemerkey)

Places every part (DESIGN.md "KiCad Library Map" + power/charger subsystem) onto
a child sheet (MCU / PSU / GNSS / Sensors / WiFi), each resolving to a real KiCad
bundled symbol + footprint + JLCPCB LCSC, with a per-sheet wiring/pin note.
(WiFi's ESP32-C3-MINI-1 is the one vendored symbol/footprint — lib/ "ephemerkey".)
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
                  "Antenna_Chip", "D", "Buzzer")
K.register_stdlib("MCU_ST_STM32U0", "STM32U083KCUx")
K.register_stdlib("RF_GPS", "MAX-M10S")
K.register_stdlib("Regulator_Switching", "TPS63900")
K.register_stdlib("Sensor_Motion", "LIS3DH")
K.register_stdlib("Battery_Management", "MCP73831-2-OT")
K.register_stdlib("Regulator_Linear", "AP2112K-3.3")
K.register_stdlib("Memory_EEPROM", "CAT24M01W")
K.register_stdlib("Power_Protection", "USBLC6-2SC6")
K.register_stdlib("Transistor_FET", "Q_PMOS_GSD", "Q_NMOS_GSD")
K.register_stdlib("Switch", "SW_Push")
K.register_stdlib("Connector", "USB_C_Receptacle_USB2.0_16P",
                  "Conn_ARM_SWD_TagConnect_TC2030-NL")
K.register_stdlib("Connector_Generic", "Conn_01x02", "Conn_01x03", "Conn_01x04")
K.register_stdlib("Display_Graphic", "ER_OLEDM0.91_1x-I2C")  # 0.91" = 128x32 I2C OLED
# vendored (espressif/kicad-libraries, CC-BY-SA 4.0 w/ lib exception) -> lib/symbols/
K.register_lib("ephemerkey", os.path.join(HW, "lib", "symbols", "ephemerkey.kicad_sym"),
               "ESP32-C3-MINI-1", "M24M02E-F")

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
# MLT-8530 has no bundled land pattern; CUI CMT-8504 (4-corner-pad 8.5mm SMD buzzer)
# is a VERIFIED-compatible placeholder (checked vs MLT-8530 datasheet 5.2):
#   pad1/pad2 (top-left/bottom-left) = MLT +Lead/-Lead exactly; pad3/pad4 land on the
#   MLT mechanical dummy pads (left unconnected -- optionally tie to GND on the PCB).
#   Pads 2.5mm vs datasheet 2.3mm and centers +/-3.5 vs +/-3.55mm = generous, harmless.
# TODO(optional): swap for an exact ephemerkey:MLT-8530 footprint for a clean release.
BUZZER = "Buzzer_Beeper:MagneticBuzzer_CUI_CMT-8504-100-SMT"
# project copy of the KiCad footprint, model repointed to vendored lib/3dmodels/ STEP
USBC_VERT = "ephemerkey:USB_C_Receptacle_G-Switch_GT-USB-7051x"  # vertical SMT, 16-pin USB2.0

# JLCPCB LCSC for the common 0402 Basic passives
RLCSC = {"5.1k": "C25905", "4.7k": "C25900", "10k": "C25744",
         "100k": "C25741", "1k": "C11702", "0R": "C17168", "100R": "C106232"}
CLCSC = {"100nF": "C1525", "1uF": "C29266", "12pF": "C1547", "10uF": "C15850",
         "1.8pF": "C1549"}


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
             fp="ephemerkey:UFQFPN-32-1EP_5x5mm_P0.5mm_EP3.5x3.5mm",
             lcsc="C22459164", mpn="STM32U083KCU6", mfr="STMicroelectronics"),
        dict(ref="J1", lib_id="Connector:Conn_ARM_SWD_TagConnect_TC2030-NL",
             value="SWD TC2030-NL",
             fp="Connector:Tag-Connect_TC2030-IDC-NL_2x03_P1.27mm_Vertical"),
        dict(ref="J2", lib_id="Connector_Generic:Conn_01x04", value="LOCK IF (I2C JST-PH)",
             fp="Connector_JST:JST_PH_S4B-PH-K_1x04_P2.00mm_Horizontal",
             lcsc="C157926", mpn="S4B-PH-K-S", mfr="JST"),
        dict(ref="DS1", lib_id="Display_Graphic:ER_OLEDM0.91_1x-I2C",
             value="OLED 128x32 I2C", fp="Connector_PinSocket_2.54mm:PinSocket_1x04_P2.54mm_Vertical"),
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
        R("R11", "4.7k"), R("R12", "4.7k"),   # lock I2C bus pull-ups -> +3V3 (master side)
        # buzzer (LS1) + low-side driver, PB4/TIM3_CH1 PWM @ ~2.7kHz
        dict(ref="LS1", lib_id="Device:Buzzer", value="MLT-8530", fp=BUZZER,
             lcsc="C94599", mpn="MLT-8530", mfr="Jiangsu Huaneng"),
        dict(ref="Q2", lib_id="Transistor_FET:Q_NMOS_GSD", value="AO3400A",
             fp=SOT23, lcsc="C20917", mpn="AO3400A", mfr="AOS"),
        dict(ref="D5", lib_id="Device:D", value="1N4148W", fp=SOD123,
             lcsc="C81598", mpn="1N4148W", mfr="Changjiang"),
        R("R13", "100R"),                                # buzzer gate series
        R("R14", "100k"),                                # buzzer gate pulldown (off-safe)
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
        dict(ref="J5", lib_id="Connector:USB_C_Receptacle_USB2.0_16P",
             value="USB-C (vert)", fp=USBC_VERT,
             lcsc="C2843970", mpn="GT-USB-7051A", mfr="G-Switch"),   # 2nd port, same bus (upright)
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
        R("R15", "5.1k"), R("R16", "5.1k"),   # J5 (2nd USB-C) CC1/CC2 pulldowns (sink)
        dict(ref="U8", lib_id="ephemerkey:MAX17048", value="MAX17048",
             fp="Package_DFN_QFN:TDFN-8-1EP_2x2mm_P0.5mm_EP0.8x1.2mm",
             lcsc="C2682616", mpn="MAX17048G+T10", mfr="Analog Devices"),  # fuel gauge, I2C1 @0x36
        C("C31", "100nF"),                               # U8 VDD bypass
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
        # W3011A pi-match: series 0R; antenna-side shunt Cm2 = 1.8pF (datasheet
        # reference value), RF_IN-side shunt Cm3 = DNP. Trim both with a VNA.
        R("Rm1", "0R"),
        C("Cm2", "1.8pF"),
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

# ============================ WiFi sheet (optional) ==========================
WIFI = dict(name="WiFi", file="wifi.kicad_sch",
    title="WiFi (optional, ESP32-C3-MINI-1 + LDO)", page="6",
    big=[
        dict(ref="MD2", lib_id="ephemerkey:ESP32-C3-MINI-1", value="ESP32-C3-MINI-1-N4",
             fp="ephemerkey:ESP32-C3-MINI-1", lcsc="C2838502",
             mpn="ESP32-C3-MINI-1-N4", mfr="Espressif"),
    ],
    small=[
        dict(ref="U6", lib_id="Regulator_Linear:AP2112K-3.3", value="AP2112K-3.3",
             fp=SOT235, lcsc="C51118", mpn="AP2112K-3.3TRG1",
             mfr="Diodes Incorporated"),
        R("R26", "100k"),                  # U6 EN pulldown — WiFi off by default
                                           # (R17 taken: hand-added 4.7k on Sensors)
        R("R18", "10k"), C("C25", "1uF"),  # MD2 EN RC power-on-reset (no MCU pin)
        R("R19", "10k"),                   # IO8 strap pull-up (1 for download)
        R("R20", "10k"),                   # IO2 strap pull-up (1 always)
        R("R21", "10k"),                   # WIFI_TXD -> IO9 boot-strap drive
        R("R22", "1k"), R("R23", "1k"),    # UART series (back-power limit)
        C("C26", "10uF", C0805), C("C27", "10uF", C0805),  # U6 VIN / VOUT
        C("C29", "10uF", C0805), C("C28", "100nF"),        # MD2 3V3 bulk + HF
        dict(ref="R24", lib_id="Device:R", value="0R", fp=R0402, dnp=True,
             lcsc="C17168"),               # IO4 -> MCU BOOT0 (recovery, DNP)
        dict(ref="R25", lib_id="Device:R", value="0R", fp=R0402, dnp=True,
             lcsc="C17168"),               # IO5 -> MCU NRST  (recovery, DNP)
    ])

# ============================ Storage sheet ==================================
STORAGE = dict(name="Storage", file="storage.kicad_sch",
    title="Audit-log memory (I2C EEPROM)", page="7", big=[],
    uuid="fbc20a33-d179-4972-ae6a-708e23148e6a",   # keep stable (root refs it)
    small=[
        # UFDFPN8 2x3mm DFN: ~1/4 the SOIC-8 land area.  Symbol + footprint
        # vendored (easyeda2kicad C29549719; verified vs DS14157/DS6638
        # UFDFPN8 Table 22: rows on the 2mm ends, 3mm apart, EP 1.2-1.6mm).
        dict(ref="U7", lib_id="ephemerkey:M24M02E-F", value="M24M02E",
             fp="ephemerkey:ST_UFDFPN8-8-1EP_2x3mm_P0.5mm_EP1.4x1.4mm",
             lcsc="C29549719", mpn="M24M02E-FMC6TG", mfr="STMicroelectronics"),
        C("C30", "100nF"),
    ])

# ============================ wiring notes (pinout guides) ====================
MCU["note"] = (12, 158, """MCU / RTC / Programming — pinout (U1 STM32U083KCU6, UFQFPN-32).  PLACED, not wired.
 pin  name            net / function           pin  name            net / function
  1   VDD             +3V3  (C3 100nF)           17  VDDUSB          +3V3  (C5 100nF)
  2   PC14/OSC32_IN   Y1 LSE 32.768kHz           18  PA8             ACC_INT2 (EXTI tamper)
  3   PC15/OSC32_OUT  Y1 LSE 32.768kHz           19  PA9             USART1_TX -> GNSS RXD
  4   PF2/NRST        NRST (C9 100nF, J1)        20  PA10            USART1_RX <- GNSS TXD
  5   VDDA/VREF+      +3V3 (C7 1uF, C8 100nF)    21  PA11            USB_DM   (<- U3 ESD)
  6   PA0             GNSS_PPS (TIM2_CH1 in)     22  PA12            USB_DP   (<- U3 ESD)
  7   PA1             GNSS_EXTINT (out)          23  PA13            SWDIO (J1)
  8   PA2             WIFI_TXD (LPUART1_TX)      24  PA14            SWCLK (J1)
  9   PA3             WIFI_RXD (LPUART1_RX)      25  PA15            BTN2 SW2 (pull-up)
 10   PA4             GNSS_RESET_N (OD out)      26  PB3             ACC_INT1 (EXTI wake)
 11   PA5             BTN1 SW1 (pull-up->GND)    27  PB4             BUZZER_PWM (TIM3_CH1) -> R13 -> Q2
 12   PA6             LOCK_SDA <> J2.3 (I2C3 R11)  28  PB5             WIFI_PWR -> U6 EN (WiFi)
 13   PA7             LOCK_SCL -> J2.4 (I2C3 R12)  29  PB6             I2C1_SCL -> U5,U7,U8,DS1
 14   PB0             LED_GRN  D1 + R2 1k         30  PB7             I2C1_SDA -> U5,U7,U8,DS1
 15   PB1             LED_RED  D2 + R3 1k         31  PF3/BOOT0       BTN3 SW3 + DFU
 16   VSS             GND                        32  VSS  / EP       GND
RTC:  Y1 32.768kHz across PC14/PC15; C1,C2 12pF load caps (match to Y1 CL; trim via RTC SMOOTHCALIB).
BUZZER:  LS1 MLT-8530 (3.6V magnetic transducer, ~2.7kHz, 95mA): pin1 = +3V3, pin2 = BUZZ_DRV (Q2 drain).
      Q2 AO3400A low-side: G <- R13 100R <- BUZZER_PWM (PB4/TIM3_CH1); S -> GND; D -> BUZZ_DRV.  R14 100k gate->GND
      (off-safe). D5 1N4148W flyback across LS1: A = BUZZ_DRV, K = +3V3.  FW drives PB4 PWM ~2.7kHz for tones.
PWR:  +3V3 from PSU sheet, C6 10uF bulk.   J1 = SWD TC2030-NL: SWDIO, SWCLK, NRST, +3V3, GND.
BTN:  3 user buttons. SW1->PA5, SW2->PA15 active-low (MCU pull-ups, to GND).
      SW3->PF3/BOOT0 active-HIGH to +3V3 (R1 10k pulldown = default boot-from-flash).
      Hold SW3 at reset (NRST via J1 / power cycle) -> ROM bootloader -> USB DFU over USB-C
      (STM32U0 supports USB DFU, AN2606; crystal-less USB via HSI48+CRS).
J2 LOCK IF (RA JST-PH 4-pin, S4B-PH-K; AUTHENTICATED I2C; ephemerkey = MASTER, lock = TARGET; = hardware/lock):
      1 = GND   2 = VSYS (battery/system rail -> POWERS the lock; the lock has NO own cell)   3 = LOCK_SDA   4 = LOCK_SCL.
      The lock draws its logic + boost/actuator current from VSYS over this cable.  CAUTION: a 12V solenoid pull-in is
      ~3-4A from VSYS -> exceeds JST-PH (~2A/contact) and ephemerkey's load-share path; keep actuation to the 6V servo /
      low duty w/ the lock's reservoir caps, or run a heavier dedicated power feed to the lock.
      Wake-on-I2C (no discrete line): lock wakes on SCL START; master sends a dummy/wake xfer then retries.
      PINS: PA6/PA7 = hardware I2C3 (AF4).  (Swapped with the LEDs, which moved to PB0/PB1: PB0/PB1 carry NO
      I2C alternate function on the U083 -- found by the fw pin-AF compile check.)
      I2C pull-ups R11/R12 4.7k -> +3V3 (KEEP at 3V3 -- do NOT pull to VSYS: 3V3 idle meets the lock's VIH across
      the discharge curve, and verify PA6/PA7 V_tol in the DS before reconsidering).  AUTH = HMAC-SHA1 (reuse smalltotp).
DS1 OLED (1x4, 0.1in header, 128x32 I2C, 3V3):  1 = GND  2 = +3V3  3 = SCL (PB6)  4 = SDA (PB7).
      Shares I2C1 with U5 0x18, U7 0x50-53, U8 0x36 (fuel gauge, PSU sheet); pull-ups R9/R10 on Sensors sheet serve all.""")

PSU["note"] = (12, 158, """POWER  --  USB-C -> charge -> load-share -> buck-boost.   Components PLACED, not wired.

REF  PART            PIN  SIGNAL  CONNECTION                    REF  PART           PIN    SIGNAL    CONNECTION
---  --------------  ---  ------  --------------------------    ---  -------------  -----  --------  -----------------------------
J3   USB-C 16P       --   VBUS    VBUS_5V                       U4   MCP73831-2-OT  1      STAT      D4 CHG LED + R8 1k
     (horizontal)    --   GND     GND                                (SOT-23-5)     2      VSS       GND
                     --   CC1     R4 5.1k -> GND                                    3      VBAT      BAT+  (C11 10uF)
                     --   CC2     R5 5.1k -> GND                                    4      VDD       VBUS_5V  (C10 10uF)
                     --   D+      U3 -> USB_DP / PA12                               5      PROG      R6 4.7k -> GND  (Ichg ~213mA)
                     --   D-      U3 -> USB_DM / PA11
                     --   SBU1/2  NC                            Q1   AO3401A PMOS   1      G         VBUS_5V ; R7 100k -> GND
                     --   SHIELD  GND                                load-share     2      S         VSYS   <-- NOT BAT+
                                                                     (SOT-23)       3      D         BAT+   <-- NOT VSYS
J5   USB-C 16P VERT  --   VBUS    VBUS_5V  (|| J3, same bus)
     GT-USB-7051A    --   GND     GND                           D3   B5819W         A      --        VBUS_5V
                     --   CC1     R15 5.1k -> GND                    (SOD-123)      K      --        VSYS
                     --   CC2     R16 5.1k -> GND
                     --   D+/D-   same net as J3 (via U3)       U2   TPS63900       1      EN        VIN  (tied on)
                                  USE ONE PORT AT A TIME             (WSON-10)      2      SEL       GND  (preset 1)
                                                                                    3/4/5  CFG1/2/3  strap = 3.3V out
U3   USBLC6-2SC6     1    I/O1    D+ (connector)                                    6      VOUT      +3V3  (C13, C14 10uF)
     (SOT-23-6)      2    GND     GND                                               7      LX2       L1 2.2uH
                     3    I/O2    D- (connector)                                    9      LX1       L1 2.2uH
                     4    I/O2    D- (MCU, PA11)                                    8      GND       GND
                     5    VBUS    VBUS_5V                                           10     VIN       VSYS  (C12 10uF)
                     6    I/O1    D+ (MCU, PA12)                                    11     EP        GND

                                                                J4   JST-PH (RA)    1      BAT+      BAT+  (1S Li-ion, protected)
                                                                                    2      GND       GND

LOAD-SHARE:  VSYS = VBUS_5V - D3  (USB in: Q1 OFF, U4 charges the cell)   |   BAT+  (USB out: Q1 ON via R7).
  A PMOS body diode conducts D->S.  With D=BAT+ / S=VSYS it points BAT+ -> VSYS: the battery always reaches the
  load, and USB can never back-feed the cell.  Swapping S/D forward-biases the body diode when VSYS ~4.7V > BAT+,
  dumping uncontrolled charge current into the cell and bypassing U4.
  The lock's Q3 is a HIGH-SIDE switch and uses the OPPOSITE S/D assignment on purpose -- do not copy it here.

BULK:  C15 = VBUS_5V.   C16 = BAT+.

U8 MAX17048 FUEL GAUGE (TDFN-8 2x2 + EP; ModelGauge 1S SoC; 3uA hibernate; I2C1 target @0x36 fixed):
  1  CTG    GND                                5  /ALRT  NC (no spare MCU pin -- FW polls SoC; 0x36 has an alert reg)
  2  CELL   BAT+  (sense AT the cell terminal, 6  QSTRT  GND (unused; do NOT float)
            J4.1 side -- NOT VSYS)             7  SCL    <- PB6 (I2C1 -- shared w/ U5 0x18, DS1 0x3C, U7 0x50-53;
  3  VDD    BAT+  (C31 100nF close)                       pull-ups R9/R10 on Sensors sheet)
  4  GND    GND                                8  SDA    <> PB7 (I2C1)
  9  EP     GND
  SDA/SCL are tolerant above VDD (datasheet: logic independent of VDD) -> 3V3 bus vs. fading BAT+ is fine.
  VERIFY vs DS before fab: EP-to-GND requirement and the TDFN-8 2x2 land (bundled EP0.8x1.2 footprint vs 21-0168).""")

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
ANTENNA:  AE1 W3011A feed -> Rm1 (0R series) -> 50 ohm CPWG -> MD1 RF_IN.  Cm2 = 1.8pF shunt at the antenna feed
          (W3011A datasheet reference value; -A rated w/ shunt 1.8pF), Cm3 = DNP (RF_IN-side).  VNA-trim at bring-up
          (S11 < -10dB, 1559-1606 MHz).  MAX-M10S RF_IN is 50 ohm w/ internal DC block -> no external DC-block cap.
          Honor the 4.0 x 6.25 mm ground keep-out under/around AE1; reserve a board corner.""")

SENSORS["note"] = (12, 70, """Sensors — pinout (U5 LIS3DHTR, LGA-16).  PLACED, not wired.
U5 LIS3DH:
  1  VDD_IO   +3V3 (C24 100nF)        9  INT2     -> PA8 (tamper / free-fall)
  2  NC                              10  RES      -> GND
  3  NC                              11  INT1     -> PB3 (wake-on-motion)
  4  SCL      -> PB6 (I2C1)          12  GND      GND
  5  GND      GND                    13  ADC3     NC
  6  SDA      -> PB7 (I2C1)          14  VDD      +3V3 (C23 100nF)
  7  SDO/SA0  addr 0x18(GND)/0x19    15  ADC2     NC
  8  CS       +3V3 (force I2C)       16  ADC1     NC
I2C1:  R9/R10 4.7k pull-ups to +3V3 — serve U5 + DS1 (OLED, MCU sheet) + U7 (EEPROM, Storage sheet) + U8 (fuel gauge 0x36, PSU sheet).
Tamper (INT2) may zeroize the TOTP secret (DESIGN.md Security).""")

WIFI["note"] = (12, 130, """WiFi (OPTIONAL) — MD2 ESP32-C3-MINI-1 + U6 AP2112K-3.3.  PLACED, not wired.  Depopulate this sheet to omit WiFi.
POWER:  WIFI_3V3 comes from VSYS via U6 — NOT from +3V3.  TX bursts (350mA @ +21dBm, 802.11b) must never load the
  TPS63900 (400mA rating) or ripple the GNSS VCC_RF rail.
  U6 AP2112K-3.3 (SOT-23-5):  1 VIN = VSYS (C26 10uF)   2 GND   3 EN = WIFI_PWR <- PB5 (R26 100k -> GND: off at MCU reset)
                              4 NC   5 VOUT = WIFI_3V3 (C27 10uF).  Off-leak 0.01uA typ / 1uA max; EN low = internal 60R
                              VOUT discharge (rail bleeds in ~ms -> clean re-strap).  Iq on = 55uA.
  NO ESP EN/RESET GPIO — power-cycling WIFI_PWR IS the reset.
MD2 ESP32-C3-MINI-1-N4 (used pins; all others NC):
   3  3V3    WIFI_3V3 (C29 10uF + C28 100nF close)    22  IO8   R19 10k -> WIFI_3V3 (strap: must be 1 for download)
   5  IO2    R20 10k -> WIFI_3V3 (strap: must be 1)   23  IO9   R21 10k <- WIFI_TXD (boot strap — see DOWNLOAD)
   8  EN     R18 10k -> WIFI_3V3, C25 1uF -> GND      30  IO20  U0RXD <- R22 1k <- WIFI_TXD (PA2 = LPUART1_TX)
  18  IO4    R24 0R DNP -> BOOT0 (MCU recovery)       31  IO21  U0TXD -> R23 1k -> WIFI_RXD (PA3 = LPUART1_RX)
  19  IO5    R25 0R DNP -> NRST  (MCU recovery)       GND = 1,2,11,14,36-53 + pour.  PCB antenna end at board edge,
                                                      ground keep-out under the antenna zone per Espressif DS 8.2.
MCU SIDE — 3 GPIOs total.  Pin moves on the MCU sheet: ACC_INT1 PA2 -> PB3, ACC_INT2 PA3 -> PA8 (GNSS_EN earmark retired):
  PA2 = WIFI_TXD (LPUART1_TX / USART2_TX)   PA3 = WIFI_RXD (LPUART1_RX: wake-from-Stop on RX)   PB5 = WIFI_PWR (U6 EN)
DOWNLOAD (flash MD2 through the MCU; FW exposes a USB-CDC bridge emulating DTR/RTS -> stock esptool.py just works):
  1. PB5 low >= 50ms (rail off + 60R discharge)      2. PA2 = GPIO, drive low (holds IO9 low via R21)
  3. PB5 high; EN RC releases; IO9 samples LOW -> UART ROM loader on IO20/21    4. PA2 back to LPUART1_TX; bridge bytes.
  NORMAL BOOT: PA2 Hi-Z or idle-high during power-up -> IO8/IO9 pull-ups win -> run from 4MB flash.
  WIFI OFF: PB5 low; PA2/PA3 -> analog Hi-Z (R22/R23 1k limit back-powering the dead rail through MD2 ESD diodes).
FW POLICY: prefer WiFi when USB present (VSYS ~4.6V -> full 3.3V out).  On battery < ~3.5V the LDO drops out toward
  the C3's 3.0V floor — gate WiFi on power state.  Keep WiFi TX and GNSS acquisition time-separated (RF hygiene).
LATER (STM32-from-WiFi): app jumps to a UART bootloader on LPUART1.  PA2/PA3 = USART2 = a STM32U0 ROM-bootloader
  UART (AN2606 — VERIFY for U083) -> with R24/R25 fitted the ESP can drive BOOT0/NRST and reflash even a blank MCU.""")

STORAGE["note"] = (12, 70, """Storage — audit-log EEPROM (U7 M24M02E-F, 2Mbit I2C, UFDFPN8/DFN 2x3mm).  PLACED, not wired.
U7 M24M02E-F (UFDFPN8):
  1  NC                                    5  SDA   <> PB7 (I2C1 — shared w/ U5 LIS3DH 0x18 + DS1 OLED 0x3C;
  2  NC                                              R9/R10 4.7k pull-ups live on the Sensors sheet)
  3  NC                                    6  SCL   <- PB6 (I2C1)
  4  VSS  GND                              7  /WC   GND (writes enabled; FW uses the SWP register instead)
  9  EP   GND (datasheet: VSS or float)    8  VCC   +3V3 (C30 100nF close)
I2C ADDR:  1010 C2 A17 A16 -> 0x50-0x53 (C2=0 default; CDA register can move/lock it).  ID page via 1011 -> 0x58+.
  No conflicts w/ LIS3DH 0x18/0x19 + OLED 0x3C + MAX17048 0x36.  Bus is good to 1MHz (runs at 400kHz for the other targets).
CONTENT (DESIGN.md "Storage, Logging & OTA"):  append-only audit ring — 32B records (seq, RTC ts, event type,
  fix meta, code hash, chained HMAC-SHA1 tag), ENCRYPTED with a key held in INTERNAL flash (RDP/HDP).
  The external chip is desolderable: it carries no plaintext and no secrets; an excised/edited record breaks
  every later chain tag (tamper-evident).  ~8000 records (~13mo @ 20 ev/day) rolling; 4M-cycle endurance;
  350nA standby.  The lockable 256B ID page can hold board serial / provisioning fingerprint.
NOT for secrets (TOTP / pairing / device keys -> internal flash, RDP+HDP) and NOT for OTA staging
  (STM32 images stage in the ESP32-C3's 4MB flash -> stream over LPUART1 to the WRP-protected bootloader).""")


# ============================ generate =======================================
K.build(
    project="ephemerkey", proj_dir=PROJ_DIR, root_uuid=ROOT_UUID,
    title=dict(title="ephemerkey", date="2026-06-21", rev="0.1",
               company="EphemerKey Authors",
               comments=["GPS-geofenced TOTP (RFC 6238) generator",
                         "Top-level — hierarchical sheets per subsystem"]),
    sheets=[MCU, PSU, GNSS, SENSORS, WIFI, STORAGE],
)
