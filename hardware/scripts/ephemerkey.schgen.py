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
BTN = "Button_Switch_SMD:SW_Push_1P1T_XKB_TS-1187A"

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
        dict(ref="SW1", lib_id="Switch:SW_Push", value="PROV/SHOW", fp=BTN,
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
             fp="RF_GPS:ublox_MAX", lcsc="C4153167", mpn="MAX-M10S-00B", mfr="u-blox"),
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

# ============================ wiring notes ===================================
MCU["note"] = (15, 150, """ephemerkey — MCU / RTC / Programming (U1 STM32U083KCU6, UFQFPN-32). PLACED, not wired. Pin#=package pin (DESIGN.md pin budget).
POWER: VDD(1)+VDDUSB(17)->+3V3 (C3,C5 100nF each); VDDA/VREF+(5)->+3V3 via C7 1uF+C8 100nF; EP(33)+VSS(16,32)->GND; C6 10uF bulk on +3V3.
RTC LSE: Y1 32.768kHz -> PC14/OSC32_IN(2), PC15/OSC32_OUT(3); C1,C2 load caps (12pF shown — match to Y1 CL via 2*(CL-Cstray), trim w/ RTC SMOOTHCALIB).
RESET/BOOT: NRST(4)+C9 100nF to GND; BOOT0/PF3(31) -> R1 10k to GND.
USB: PA11(21)=USB_DM, PA12(22)=USB_DP  <- from PSU sheet via U3 ESD.   SWD: PA13/SWDIO(23), PA14/SWCLK(24) -> J1 (+3V3,GND,NRST too).
UI: SW1 -> PA5(11) (internal pull-up); D1 GRN+R2 1k -> PA6(12); D2 RED+R3 1k -> PA7(13).
GNSS (to GNSS sheet): USART1 PA9(19)->GNSS RXD, PA10(20)<-GNSS TXD; PA0(6)=TIMEPULSE/1PPS (TIM2_CH1); PA1(7)=EXTINT; PA4(10)=GNSS RESET_N (OD); PA8(18)=GNSS_EN.
I2C1 (to Sensors): PB6(29)=SCL, PB7(30)=SDA. ACC INT: PA2(8)=INT1, PA3(9)=INT2.
LOCK OUT J2: 1=+3V3, 2=LOCK_TX/PB0(14), 3=CODE_VALID/PB1(15) (open-drain), 4=GND.  Spare: PA15(25),PB3(26),PB4(27),PB5(28).""")

PSU["note"] = (15, 165, """ephemerkey — Power (USB-C -> charge -> load-share -> TPS63900 3V3). PLACED, not wired. See DESIGN.md "USB-C input + Li-ion charging".
USB-C J3 (16P): VBUS->USB_VBUS; CC1->R4, CC2->R5 (5.1k each to GND, sink/UFP); D+/D- -> U3 ESD -> STM32 PA12/PA11; SBU=NC; SHIELD->GND (or 1M||cap).
ESD U3 USBLC6-2SC6: at the connector, on VBUS + D+ + D-.
CHARGER U4 MCP73831-2-OT: VDD<-USB_VBUS; VBAT->BAT+; PROG via R6 4.7k = ~213mA (~0.5C of a 500mAh cell); STAT->D4 CHG LED + R8 1k. C10/C11 10uF in/out.
LOAD-SHARE: Q1 AO3401A P-FET src=BAT+, drn=VSYS, gate->USB_VBUS via R7 100k (pulldown). D3 B5819W: USB_VBUS->VSYS. Battery feeds VSYS only when USB absent.
BUCK-BOOST U2 TPS63900: VIN<-VSYS (1.8-5.5V ok: VBUS~4.7V or BAT 3.0-4.2V); L1 2.2uH on LX1/LX2; Cin C12, Cout C13/C14 (10uF). CFG1/2/3 strap=3.3V; EN->VIN. VOUT=+3V3. EP->GND.
BATTERY J4 JST-PH (RA): pin1=BAT+, pin2=GND (1S Li-ion, protected pack). C15 VBUS bulk, C16 VBAT bulk (10uF).""")

GNSS["note"] = (15, 120, """ephemerkey — GNSS (MD1 MAX-M10S-00B + AE1 W3011A). PLACED, not wired. See DESIGN.md GNSS / GPS Antenna.
ANTENNA: AE1 W3011A feed -> pi-match -> 50ohm CPWG trace -> MD1 RF_IN(11). Rm1 series (0R populated); Cm2/Cm3 shunt pads (DNP, tune w/ VNA at bring-up). Honor 4.0x6.25mm ground keep-out under/around AE1; reserve a board corner.
SUPPLY: VCC(8)+V_IO(7)->+3V3 (C20/C22 100nF); VCC_RF(14) via L2 ferrite + C17 100nF/C18 10pF (RF supply isolation); C19 10uF bulk on VCC. V_BCKP(6)->+3V3 always-on tap + C21 100nF (hot-start retention). GND=1,10,12.
INTERFACE (to MCU): TXD(2)->PA10, RXD(3)<-PA9 (USART1 NMEA/UBX 9600); TIMEPULSE(4)->PA0 (1PPS); EXTINT(5)<-PA1; RESET_N(9)<-PA4 (OD). VIO_SEL(15)=open (3.3V). SDA(16)/SCL(17)=NC (UART used). SAFEBOOT_N(18)=pull per DS. LNA_EN(13)=open (passive antenna).""")

SENSORS["note"] = (15, 95, """ephemerkey — Accelerometer (U5 LIS3DHTR, LGA-16). PLACED, not wired. See DESIGN.md Sensors / Security.
SUPPLY: VDD(14)+VDD_IO(1)->+3V3, C23/C24 100nF each. GND(5,12)->GND. RES(10)->GND. ADC1-3(13,15,16)=NC. CS(8)->+3V3 (force I2C).
I2C1: SCL(4)->PB6, SDA(6)->PB7; R9/R10 4.7k pull-ups to +3V3. SDO/SA0(7) strap sets addr (0x18 low / 0x19 high).
INT: INT1(11)->PA2 (wake-on-motion), INT2(9)->PA3 (tamper/free-fall). Tamper policy may zeroize the TOTP secret (DESIGN.md Security).""")

# ============================ generate =======================================
K.build(
    project="ephemerkey", proj_dir=PROJ_DIR, root_uuid=ROOT_UUID,
    title=dict(title="ephemerkey", date="2026-06-21", rev="0.1",
               company="EphemerKey Authors",
               comments=["GPS-geofenced TOTP (RFC 6238) generator",
                         "Top-level — hierarchical sheets per subsystem"]),
    sheets=[MCU, PSU, GNSS, SENSORS],
)
