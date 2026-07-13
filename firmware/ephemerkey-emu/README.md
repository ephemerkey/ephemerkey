# ekemu — generator + lock emulator

Runs the **exact `ephemerkey-core` engines the firmware ships** — policy
state machines, TOTP validation with walk-time delay windows, the reveal
scheduler with scatter/show-once/decoy display modes — under a virtual
clock, with the display and key-entry surfaces mimicked in text. A
30-minute walk takes one `time +30m`.

```sh
make demo          # self-checking walk-vault ritual (CI-able)
make run           # interactive REPL
make test          # core unit tests (RFC 4226/6238 vectors, etc.)
```

## Commands

```
time +<n>[s|m|h]     advance the virtual clock (ticks slot expiry)
time set <unix>      jump the clock
gen reveal [key]     request a reveal — prints the display frames the user
                     (and any shoulder-surfer) would see; scatter mode shows
                     one digit per frame, correct position, random order
gen next [key]       countdown to the next REAL reveal (show-once modes)
lock enter <code|@N> type a code; @N = Nth revealed code (the "notebook"
                     the user wrote codes into before walking)
lock status          slot states (respects show_progress hiding)
expect <substring>   assert on the last event line; any miss => exit 1
echo / # / quit
```

The `DECOY` tag on reveal events is ground truth from the core's
`introspect` feature (emulator-only — firmware builds cannot see it; the
display pipeline is identical for real and poison codes by construction).

## Scenario files

JSON (`scenarios/*.json`): a key table (secret, digits 4-10, optional decoy
twin, display spec) and a slot table (policy + parameters). Policies:
`always`, `sequence` (N codes, generation cadence, walk delay), `path`
(ordered zone-keyed legs), `deadman` (beat or RELOCK), `quorum` (M-of-N
distinct keys, optional alternating).

| Scenario | Script demonstrates |
|----------|---------------------|
| `walk-vault` | the canonical ritual: 3 codes on 90-240 s minting cadence, valid 30-35 min later; scatter display; poison-mode decoy → lockout |
| `courier` | walk-the-path: zone A→B→C legs, order + pace proven by counters, entered from the notebook at the destination |
| `deadman` | unlock, keep-alive beats, missed-beat RELOCK, and the forced-reset-still-relocks invariant |
| `quorum` | two-person rule: alternating violation, completion, window expiry |

## What the demo script proves

`scripts/walk-vault.txt` walks the full story: cadence minting → squeezed
4th reveal is a decoy → early entry rejected → 31-min walk → 3 codes
accepted on generation cadence → replays ignored without reset → unlock
fires → post-fire replay burned → the decoy arrives later and trips a
lockout. Every step is `expect`-checked.
