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
- CPL is **SMD-only, DNP-excluded** (`--smd-only --exclude-dnp`) so it matches the
  BOM's assembled set. SMD BOM↔CPL parts reconcile exactly.
- **Through-hole connectors J2–J8 are intentionally NOT in the CPL** (they're THT,
  not SMT-placed). JLCPCB will warn "J2–J8 won't be assembled" — expected:
  **hand-solder the connectors**, or enable THT assembly (the JST parts have LCSC;
  the J4/J5/J8 pin headers don't).

## State at capture — READ BEFORE ORDERING
This is a **working snapshot, not a signed-off release.** Known-open items:

- **DRC:** 0 unconnected, 0 schematic-parity. **3 starved-thermal** on the
  through-hole GND pins of **J6/J7** (hall connectors) — mild (pads still connect
  to GND); fix by setting those pads to solid zone connection or widening the
  thermal spoke.
- **BOM:** all actives + passives carry an LCSC. **J4 / J5 / J8** (UPDI + servo
  1×3 headers, through-hole) have **no LCSC** — mark *Do-not-place* for assembly
  or hand-solder.
- **CPL** lists the 3 DNP parts (**R14, R9, C7**); the BOM correctly omits them.
  JLC skips the unmatched CPL rows (optional: `--exclude-dnp` on the pos export).
- **ERC (schematic hygiene, does not affect these outputs):** no-connect flags
  still needed on PA3/PB5/PC0–PC2; PWR_FLAGs on VCC + GND.

## DNP (not assembled)
`R14` (VSERVO source alt), `R9` + `C7` (drain snubber).

## Regenerate
`cd hardware && make jlc-lock` → rebuilds `hardware/lock/jlcpcb/` + the zip, then
copy over this `fab/` folder to re-capture.
