# ephemerkey Unlock Policies — Ideas & Design

Companion to `DESIGN.md`. This covers the *behavioral* layer: what combinations
of codes, places, and timing a lock demands before it acts. Firmware sketch:
`firmware/ephemerkey-rs/src/policy.rs`.

## Personalities

One firmware image, one hardware design, two roles selected by provisioned
config (`firmware/ephemerkey-rs/src/config.rs`):

- **Generator** — the classic ephemerkey: emits TOTP codes only with a valid
  GNSS fix inside (or outside — see zone keys) its authorized geofence.
- **LockController** — receives/validates codes and drives the companion
  ATtiny lock board over the bit-banged LOCK I2C bus. It has the *same* GNSS
  hardware, so it can impose position requirements on **itself** (portable
  locks).

Both are programmed with a signed config **file** delivered over USB
(explicit provisioning mode: button + enumeration) or the WiFi/ESP32-C3 link
(`provision.rs`). The file carries the role, secrets, geofences, and the
**code slot table** below.

## Who can enforce what (the honest part)

A TOTP code is six digits; it carries no location. So:

- A lock can enforce **its own** position (it has GNSS), timing between codes,
  code counts, and code validity.
- A lock **cannot observe the generator's position**. "The generator must be
  ≥5 km away" is enforced one of two ways:
  1. **Generator-side policy** — the generator simply refuses to emit unless
     its own policy (fix + fence + clock freshness) is satisfied. Fine when
     both devices are ours (they are — that's the product).
  2. **Zone-keyed secrets** — the generator holds several TOTP secrets and
     selects one by which of *its* configured zones it currently occupies
     (e.g. `K_home`, `K_far`, `K_transit`). A code proves, cryptographically,
     "a trusted generator, in zone X, at time T." The lock's policy then says
     "I need a `K_far` code," and distance claims stop being honor-system.

Zone-keyed secrets are the primitive that makes most of the pedantic
sequences below real instead of theatrical.

## Code slots

A lock holds up to `N_SLOTS` **parallel, independent code slots**. Each slot
is its own secret (or zone-key set), its own policy state machine, and its own
action. Examples of a slot table:

| Slot | Secret | Policy | Action |
|------|--------|--------|--------|
| 0 | `K_master` | AlwaysValid | unlock |
| 1 | `K_far` | Sequence{n=3, window=10min, gap=90s..240s} | unlock |
| 2 | `K_home` | AlwaysValid | lock |
| 3 | `K_duress` | AlwaysValid | unlock + silent flag |

Semantics:

- Every received code is tried against **every slot** (constant-time compare
  per slot; a code is consumed by at most one slot — first match in priority
  order).
- A code that matches **no** slot is an *invalid code*: it resets the
  sequence state of every slot marked `reset_on_invalid` (default: all).
- Each TOTP counter value is counted **once per slot** — replaying the same
  code twice in one period advances nothing (dedupe by period counter, same
  discipline as the lock board's armed nonce).

## The canonical pedantic sequence (the motivating example)

Lock fixed at the vault; generator must be taken far away. Unlock requires
**N codes within Y minutes**, with **min and max spacing between codes**:

```
policy Sequence {
    key        = K_far          # zone-keyed: generator must be in the "far" zone
    n          = 3              # codes required
    window     = 10 min         # all N must land inside this (arrival)
    gap_min    = 90 s           # too fast -> reset (no code hoarding)
    gap_max    = 4 min          # too slow -> reset (continuous presence)
    delay      = 30..35 min     # walk time: codes valid this long AFTER minting
                                # (0..~30s = instant; see "Time-shifted validity")
    reset_on_invalid = true
    progress   = visible | hidden
}
```

State machine: `Idle --code--> Counting(1) --code(in gap)--> Counting(2) ...
--> Fire`. Any violation (early, late, invalid, window expiry) → `Idle`,
silently or visibly per `progress`. `gap_min` prevents pre-generating a batch
of codes and replaying them in one burst; `gap_max` proves the generator
*stayed* in the far zone for the whole ritual.

**Progress display** is a per-slot flag: show a count on the OLED/LEDs, or
show nothing (an observer can't tell a 2-of-3 state from idle — decoy option:
always display the same idle screen).

## Time-shifted validity (walk time)

The person opening the lock and the holder of the generator are usually the
**same person** — the far zone and the lock may be a 30-minute walk apart. So
a slot can accept codes with a configured **delay window**: a code generated
at T=0 is valid at the lock from `T+delay_min` to `T+delay_max` (e.g.
+30…+35 min) — and, importantly, *not* valid immediately.

Mechanics (all from standard TOTP, no new crypto):

- The lock knows which TOTP counter a code matched, i.e. its **generation
  time** to one period. Delay check: `now - gen_time ∈ [delay_min, delay_max]`.
  The verifier simply searches the counter range
  `[(now-delay_max)/P … (now-delay_min)/P]` instead of "current ± 1".
- With a delay window, sequence **gap timing splits in two**, and both are
  checked:
  - **generation cadence** — spacing between the *counters* of accepted
    codes (`Δcounter × P ∈ [gap_min, gap_max]`): proves the codes were
    minted at the far place at the required rhythm. Immune to arrival
    jitter, and immune to hoarding-then-bursting by construction.
  - **arrival window** — all M codes must still be entered within the
    slot's overall window (and optionally with their own arrival gaps).
- A delayed slot proves "*you were in the far zone at T-30min, M times, at
  the right cadence*" — the person writes the codes down (or remembers them)
  and walks. This is the intended, primary usage of the sequence policy.
- Composition rule: **split-epoch freshness** (catalog #11) is the delay
  window's opposite (`delay = [0, X]`); a slot has exactly one arrival
  window, `[delay_min, delay_max]`, with instant slots being `[0, ~P]`.
- Replay unchanged: each accepted counter burns once per slot; the wider
  counter search only widens brute-force surface, which is countered by code
  length (below) and lockout (#12).

## Code length & display modes (generator side)

**Code length is per-key configurable, 4–10 digits.** RFC 6238 dynamic
truncation yields 31 bits, so entropy saturates just above 9 digits — 10 is
allowed but adds nothing (and its leading digit skews low); the UI says so.
Short codes (4–5 digits) are for low-stakes slots and pair naturally with
strict lockout; delayed/high-stakes slots should use 8+ (a wider delay window
multiplies the guessing surface).

The generator's *display* is itself policy-configurable per key:

- **Scatter reveal (secret mode)** — the code is shown **one digit at a
  time, in its correct position, in random order** (`__3___`, `4_____`, …).
  A shoulder-surfer must capture every frame *and* its position; the owner
  just fills in a mental grid. Config: per-digit dwell time, auto-advance or
  button-step.
- **Short reveal** — correct codes display for a bounded time (e.g. 5 s),
  then blank. Config: reveal seconds.
- **Show-once + refuse** — after one reveal, the generator refuses to show
  another code until the *next legitimate cadence window* (it knows the
  slot's `gap_min`/`gap_max` and counts down to it). Someone who grabs the
  device after you've read your code gets nothing.
- **Show-once + decoy (poison mode)** — stronger: instead of refusing,
  subsequent reveals show a plausible code drawn from a distinct decoy
  secret (`K_decoy`). Entering a decoy at the lock is a **negative match**:
  it's not "invalid noise", it's a definite signal someone squeezed the
  generator for extra codes. Per-slot response: reset, hard lockout, or
  silent duress telemetry. Under coercion, "show me another code" hands the
  attacker a tripwire.

Generator-side display/reveal state is RAM-only and per-key, mirroring the
lock's slot config so the countdown UX ("next code in 90 s") matches what
the lock will actually accept.

## More pedantic sequences (catalog)

1. **Rhythm lock** — codes must arrive at a fixed cadence ± tolerance
   (e.g. every 120 s ± 15 s). A stricter `gap_min == gap_max` variant of the
   canonical sequence; humans with two devices find this maddening, which is
   the point.
2. **Walk-the-path** — ordered zone-keyed codes: one from `K_zoneA`, then
   `K_zoneB`, then `K_zoneC`, in order, each within a per-leg time window.
   Proves a route was physically traveled (courier / patrol verification).
3. **Walk-away unlock** — codes from progressively farther zone rings
   (`K_1km`, then `K_5km`); the vault opens only as its keyholder gets
   farther away. The paranoid-executor special.
4. **Two-person rule / quorum** — codes from M of N distinct generators
   (distinct secrets), interleaved within a window; optionally alternating
   (A, B, A, B) so one person holding both devices still has to juggle.
5. **Dead-man re-arm** — once unlocked, the lock re-locks unless it keeps
   receiving a valid code every ≤ T. Miss one beat and it snaps shut and
   resets to full ritual.
6. **Calendar gate** — slot only accepts codes inside RTC windows
   (business hours, specific weekdays). Composable with any other policy.
7. **Stillness gate** — the lock's LIS3DH must read quiet for ≥ S seconds
   before/while codes count (nobody unlocks it while it's being carried);
   inverse variant: *must* be in motion (dead-drop handoff in transit).
8. **Portable-lock positioning** — the lock uses its own GNSS: unlock codes
   only count while the lock sits in fence P_unlock; locking (or the lock
   confirm code, below) only issues in fence P_lock. A container that can
   only be opened at the destination and only sealed at the origin.
9. **Duress slot** — a code that unlocks normally to the eye but flags the
   audit log and emits a *distinct* confirm-TOTP; a remote validator sees
   the duress confirm, the person at the lock doesn't.
10. **Receipt chaining** — the confirm-TOTP from session k is an extra input
    to session k+1's first code (validator-mediated): you cannot start the
    next ritual without proof the last one was seen. Makes unlocks an
    append-only, externally witnessed chain.
11. **Split-epoch freshness** — a code only counts in the first X seconds of
    its TOTP period. Forces on-the-spot generation with a tight clock, kills
    read-me-a-code-over-the-phone-slowly relays.
12. **Backoff / lockout** — per-slot failure counter with exponential
    cool-down (composes with `progress = hidden`: an attacker can't tell
    lockout from idle).
13. **Master slot** — `AlwaysValid`: one code, immediate action, no
    ceremony. Because someday you will be standing in the rain.

## Confirm-TOTP (the lock talks back)

On completing an action (unlock, lock, duress, tamper) the lock generates its
**own** TOTP from a per-lock confirm secret, bound to the event:

```
confirm = TOTP(K_confirm, t)  displayed alongside an event tag
        (or: HOTP-style over event counter ‖ action, shown as code+seq)
```

- Displayed on the OLED / read over USB, for the user to relay to a
  **validator** (a CLI/web tool holding `K_confirm` — or another ephemerkey
  provisioned as validator).
- Or reported autonomously over WiFi when the ESP rail is up.

This closes the loop for remote parties: "the vault really did lock at
14:02" is a 6-digit code someone can text you, and receipt chaining (#10)
turns it into a required input for the next cycle.

## Reset semantics (summary table)

| Event | Effect |
|-------|--------|
| Valid code, right slot, timing OK | advance that slot's state |
| Valid code, timing violated (early/late/window) | reset that slot |
| Code matching no slot | reset all `reset_on_invalid` slots |
| **Decoy code (negative match, `K_decoy`)** | per-slot: reset, hard lockout, or silent duress telemetry — always logged |
| Replayed code (same TOTP counter) | ignored (no advance, no reset) |
| Gate unsatisfied (position/stillness/calendar) | code ignored or slot reset (per-slot option) |
| Power loss | sequence state is RAM-only → full reset (deliberate) |
| Tamper (accel) while armed | per-policy: reset, lockout, or zeroize |

## Open questions

- Where codes physically enter a LockController: USB console and WiFi are
  free; a keypad is new hardware; the lock-board I2C link is master-out
  today. Nearest-term: the paired generator relays user-entered codes? To
  decide when the controller personality gets its first real deployment.
- Slot count / config-file size budget (flash journal page = 2 KB, so ~a
  dozen slots with zone tables fits comfortably).
- Whether confirm-TOTP should be HOTP (event counter) rather than TOTP —
  leaning HOTP: it's an event receipt, not a time proof, and it never
  collides on two events in one period.

**Resolved since first draft:** the provisioning encoding, key model (owner
Ed25519 / device Ed25519+X25519, "set"-bound), transports, and the web UI
that configures these policies are specified in `DESIGN-management.md`
(CBOR + COSE_Sign1/Encrypt0). The "signed config file" sketched in
`provision.rs` is that envelope.
