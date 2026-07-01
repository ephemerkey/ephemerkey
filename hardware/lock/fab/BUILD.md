# lock — fab package snapshot

A captured copy of the JLCPCB manufacturing outputs so we have a record of
*exactly* what was generated. The live build dir (`hardware/lock/jlcpcb/`) is
gitignored/regenerable; this `fab/` folder is the committed snapshot.

## Provenance
- **Source commit:** `92903b9` (schematic `mcu.kicad_sch` with U1 LCSC restored +
  repoured `lock.kicad_pcb` with clearances cleared).
- **Built with:** `make jlc-lock` → `scripts/jlcpcb-package.sh` (KiCad 10 `kicad-cli`).
- **Date:** 2026-06-30.
- **Board:** 2-layer, 1.6 mm, ~64 × 23 mm. Design rules = JLCPCB 2-layer.

## Contents
| File | What |
|------|------|
| `gerbers/*.gtl/.gbl/.gts/.gbs/.gto/.gbo/.gtp/.gbp` | copper / mask / silk / paste (F+B) |
| `gerbers/lock-Edge_Cuts.gm1` | board outline |
| `gerbers/lock.drl`, `lock-drl_map.gbr` | merged Excellon drill + map |
| `gerbers/lock-job.gbrjob` | Gerber job file |
| `lock-BOM.csv` | assembly BOM (Designator, Value, Footprint, Qty, LCSC, MPN, Mfr) |
| `lock-CPL.csv` | component placement (pick-and-place) |
| `lock-jlcpcb.zip` | the upload bundle (gerbers + drill + BOM + CPL) |

Upload the zip to <https://cart.jlcpcb.com/quote>, enable PCB Assembly, attach
BOM + CPL.

## BOM/CPL format (JLCPCB match)
- BOM lists every designator **individually** (`--ref-range-delimiter ''`) — JLCPCB
  can't expand ranges like `R18-R20`, which would silently drop those SMD parts.
- CPL is **DNP-excluded** (`--exclude-dnp`) and drops mounting holes (H*) + the
  hand-solder list; the BOM applies the same hand-solder exclusion. **BOM and CPL
  now match exactly (42 = 42 parts, no diffs)** — no JLCPCB part-mismatch warnings.

## Assembly plan (JLCPCB **Standard** tier)
- **JLC-assembled (SMD + THT):** all SMD, plus the **JST connectors** J2 (I2C),
  J3 (solenoid), J6/J7 (hall) — they carry LCSC and are placed via **THT assembly**.
- **Hand-soldered (excluded from BOM+CPL):** the 1×3 pin headers **J4** (UPDI),
  **J5 / J8** (servo) — no LCSC. Listed in `hardware/lock/lock-handsolder.txt`.
- **Use the *Standard* assembly tier, not Economic:**
  - **C5 / C8 — 220 µF 25 V (C2918361, RVT1E221M0607) is NOT available for
    Economic assembly** → Economic can't place it. Standard tier stocks it.
  - THT assembly (the JST connectors) also requires Standard tier.

## State at capture — READ BEFORE ORDERING
This is a **working snapshot, not a signed-off release.** Known-open items:

- **DRC:** 0 unconnected, 0 schematic-parity. **3 starved-thermal** on the
  through-hole GND pins of **J6/J7** (hall connectors) — mild (pads still connect
  to GND); fix by setting those pads to solid zone connection or widening the
  thermal spoke.
- **THT rotation:** verify the JST connector rotations in the CPL against JLCPCB's
  convention at review (JST/THT parts sometimes need a rotation offset).
- **ERC (schematic hygiene, does not affect these outputs):** no-connect flags
  still needed on PA3/PB5/PC0–PC2; PWR_FLAGs on VCC + GND.

## DNP (not assembled) / hand-solder
- **DNP** (excluded from BOM+CPL): `R14` (VSERVO source alt), `R9` + `C7` (drain snubber).
- **Hand-solder** (excluded from BOM+CPL, fitted by hand): `J4`, `J5`, `J8`.

## Regenerate
`cd hardware && make jlc-lock` → rebuilds `hardware/lock/jlcpcb/` + the zip, then
copy over this `fab/` folder to re-capture.
