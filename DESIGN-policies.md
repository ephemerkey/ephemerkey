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
    window     = 10 min         # all N must land inside this
    gap_min    = 90 s           # too fast -> reset (no code hoarding)
    gap_max    = 4 min          # too slow -> reset (continuous presence)
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
