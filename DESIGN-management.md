# ephemerkey Management Plane — Sets, Keys, Encoding, Web UI

Companion to `DESIGN-policies.md` (what policies exist) — this covers how
policies get **into** devices and how signed telemetry gets **out**: the key
model, the wire encoding, and the web UI that authors it all. Applies to both
personalities (Generator / LockController).

## The "set" — key-bound fleet model

An **ephemerkey set** is an owner keypair plus the devices enrolled under it.

| Key | Where | Algorithm | Purpose |
|-----|-------|-----------|---------|
| **Owner key** | web UI / operator (exportable, or WebCrypto non-extractable) | Ed25519 | signs every config document; the root of the set |
| **Device signing key** | generated ON-device (U0 TRNG), never leaves | Ed25519 | signs telemetry, receipts, enrollment proof |
| **Device agreement key** | generated on-device, pubkey exported at enrollment | X25519 | lets the UI encrypt secrets *to* a device |
| Zone/TOTP/pairing secrets | symmetric, delivered inside config | (existing HMAC-SHA1 machinery) | the data plane stays symmetric and cheap |

- `set_id = SHA-256(owner_pub)[0..8]` — devices are *bound* to the set by
  storing `owner_pub` at enrollment; every config they ever accept must verify
  under it. A config for set A is meaningless to a device in set B.
- **Enrollment (TOFU, physically gated):** device in provisioning mode
  (button + USB) emits an enrollment doc `{device_id, sign_pub, kx_pub, fw}`
  signed by its device key; the UI adds it to the roster and writes back
  `owner_pub`. First writer wins; re-keying requires physical provisioning
  mode again. Remote enrollment is deliberately impossible.
- **Owner rotation:** a rotation doc signed by the *old* owner key, or
  physical re-enrollment. (Losing the owner key = re-enroll everything —
  document this loudly in the UI.)

ECC choice: **Ed25519/X25519**, not P-256. No PKA hardware on the U083 either
way, `salty` is written for Cortex-M (verify ≈ tens of ms at 48 MHz, fine for
config-rate events), no ECDSA nonce-reuse footgun, and WebCrypto ships Ed25519
in current browsers (libraries like `@noble/curves` cover stragglers).

## Encoding: CBOR + COSE

Config documents and telemetry are **CBOR** (RFC 8949), wrapped in **COSE**
(RFC 9052) envelopes:

- **Config update** = `COSE_Encrypt0( COSE_Sign1(config, owner_key), device_kx )`
  — sign-then-encrypt. Signature = authority; encryption (ephemeral X25519 →
  HKDF → AES — the U083 has an AES engine) because configs *carry symmetric
  secrets* and the remote path traverses the ESP and the internet.
- **Telemetry** = `COSE_Sign1(events, device_key)` — no secrets inside
  (audit records are already encrypted at rest before they hit the EEPROM),
  so integrity/attribution only.

Why CBOR over the alternatives:

| | verdict |
|---|---|
| custom TLV (what the 65-byte lock blob uses) | right for a fixed tiny struct, wrong for a growing policy tree — every field addition is a migration |
| protobuf | schema compiler in the loop, no canonical bytes without care, signing needs wrapper conventions |
| JSON | not compact, not canonical, floats/strings ambiguity — hostile to signatures |
| **CBOR/COSE** | deterministic encoding rules for signing, integer-keyed maps stay tiny, decodes with `minicbor` (no_std, no alloc) on the M0+, native in JS (`cbor-x`), and COSE is *the* standard for exactly this shape of problem |

Skeleton (integer keys, unknown keys ignored → forward compatible):

```
config = {
  1: seq,          ; uint, strictly increasing per device (anti-rollback)
  2: set_id,       ; bstr[8]
  3: target,       ; device_id or 0 = whole set
  4: role,         ; 1 = generator, 2 = lock-controller
  5: keys,         ; [ {id, kind(totp/zone/pairing/confirm/decoy), secret,
                   ;    digits(4-10), display(plain/scatter/short/once), decoy_ref} ]
  6: zones,        ; [ {id, shape(circle/poly), coords, hdop, minsats} ]
  7: slots,        ; [ {key, policy, gates, action, progress, reset_on_invalid} ]
  8: policy-blobs, ; per-slot params, tagged by policy type
  9: device-opts   ; staleness window, tamper policy, buzzer, display
}

event = {
  1: seq, 2: device_id, 3: rtc_ts, 4: type,   ; unlock/lock/duress/tamper/
  5: detail, 6: chain_tag                     ; fence-enter/exit/power/config-ack
}
```

Device rules: reject bad signature, wrong `set_id`, `seq <=` stored seq (the
config journal already stores a monotonic sequence — same slot). A `config-ack`
telemetry event (echoing `seq` + config hash) closes the loop in the UI.

## Transports (same bytes everywhere)

The envelope is transport-independent — the device trusts only the
COSE layers, never the channel:

- **Local / USB:** provisioning mode (button + USB CDC). Web UI talks
  **WebSerial** directly from the browser (Chromium; elsewhere: download the
  `.ekcfg` blob and use a tiny CLI, or file-drop once USB-MSC-style ingest
  exists). Telemetry pulls the audit ring the same way.
- **Remote / WiFi:** the UI publishes the sealed blob (any dumb HTTPS
  endpoint or relay — it needs no trust); the ESP32-C3 fetches it, streams it
  over LPUART1, the STM32 verifies exactly as if it came over USB. Telemetry
  pushes back through the ESP as signed event batches.
- **Sneakernet:** the sealed blob is just bytes — QR-code chunks or a file on
  a phone work for air-gapped sites.

## Web UI — configuring the policy catalog

One page per set: roster (enrollment status, last-seen telemetry, config seq),
a **slot table** per device, and a policy editor per slot. Per-policy forms:

| Policy | UI controls |
|--------|-------------|
| Master (AlwaysValid) | key slot picker; big red "no ceremony" warning |
| Time-gated sequence (N/Y/min-max) | steppers for N; duration fields for window + min/max gap; live validity check (`n·gap_min ≤ window`); progress visible/hidden toggle |
| Walk-time delay | dual duration slider "codes valid from +X to +Y after minting" (0 = instant); warning that a wide window multiplies guess surface → suggests longer digits |
| Code length / display | digits stepper 4–10 (entropy note at 10: 31-bit truncation cap); display mode dropdown per key: plain / scatter-reveal (dwell ms, step-by-button) / short-reveal (secs) / show-once → refuse-or-decoy; decoy severity picker (reset / lockout / silent duress) |
| Rhythm lock | cadence + tolerance; renders a metronome preview |
| Zone-keyed distance ("generator must be far") | map widget: draw the far zone(s) on the *generator's* config, UI auto-creates the zone key and references it from the lock's slot — the cross-device wiring is done by the UI, invisibly |
| Walk-the-path | ordered waypoint list on the map, per-leg deadline fields; drag to reorder |
| Walk-away unlock | concentric ring editor (radii sliders) |
| Two-person / quorum | multi-select from the set roster, M-of-N stepper, "alternating" toggle |
| Dead-man re-arm | beat interval; scary copy about what happens when you miss one |
| Calendar gate | week grid (paint hours), attachable to any slot |
| Stillness / motion gate | quiet-seconds slider; invert checkbox |
| Portable-lock positioning | two map fences: P_unlock, P_lock |
| Duress slot | clone-of-slot picker + silent-flag notice; UI shows its distinct confirm-code stream |
| Receipt chaining | toggle per slot; picks which validator key verifies receipts |
| Backoff/lockout | failure threshold + base delay; graph preview of the curve |

Plus a **ritual simulator**: given a slot's config, the UI renders a timeline
("code at t=0, next accepted 90 s–4 min, all three inside 10 min…") and lets
you scrub through a simulated attempt — the pedantic policies are exactly the
ones people will misconfigure, so dry-run before signing.

Signing UX: "Review & sign" shows the canonical CBOR diff vs the device's
acked config, then signs with the owner key (WebCrypto) and queues per-device
sealed blobs for whichever transport is available.

## Firmware crates (all no_std, verified available)

| Crate | Use |
|-------|-----|
| `minicbor` | CBOR encode/decode, zero-alloc |
| `salty` | Ed25519 sign/verify (Cortex-M optimized) |
| `x25519-dalek` | ECDH for COSE_Encrypt0 unseal |
| U083 AES engine | payload cipher (via embassy/PAC), HKDF via existing SHA1-HMAC or add sha2 |

COSE envelopes are small enough to hand-roll with `minicbor` (COSE_Sign1 /
Encrypt0 are ~4-field arrays) — no heavy COSE crate needed.

## Open questions

- Telemetry batching/ack window when the ESP is only powered occasionally
  (how much audit ring to replay; the chain tags make gaps detectable).
- Whether the web UI is a static SPA only (fits the "dumb relay" model) or
  also a small server for fleets — leaning static-first.
- SHA-256 shows up in set_id/HKDF — pull in `sha2` (fine) or reuse SHA-1
  everywhere (avoid; keep SHA-1 confined to the legacy lock link + TOTP).
- Config size budget: a maxed-out slot table with polygon zones should stay
  ≤ 2 KB (one journal page) — enforce in the UI, paginate zones if ever needed.
