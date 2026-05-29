# `net/tls13/named_group` audit

Module: `core/net/tls13/named_group.vr` — RFC 8446 §4.2.7
supported_groups: `NamedGroup` 8-variant + to_u16/from_u16 (IANA
codepoints).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.tls13.handshake` | key_share + supported_groups extensions. |
| `core.net.tls13.keyschedule` | ECDHE/FFDHE group selection. |

## 2. Crate-side hardcodes

The 8 IANA codepoints (X25519=0x001D, secp256r1=0x0017, secp384r1=0x0018,
secp521r1=0x0019, x448=0x001E, ffdhe2048=0x0100, ffdhe3072=0x0101,
ffdhe4096=0x0102) are RFC 8446 §4.2.7 verbatim. Drift breaks key_share
negotiation silently.

## 3. Language-implementation findings

None. 8-variant unit enum + to_u16/from_u16 `match`. Compiles cleanly
under `--interp`.

## 4. Action items landed in this branch

* `unit_test.vr` — 8 codepoint pins + from_u16 (2 cases).
* `property_test.vr` — from_u16 ∘ to_u16 == id over all 8 groups.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| is_ecdhe / is_ffdhe classification (if present) | this folder | 30min |
| key_share keypair generation per group | tls13/handshake | gated on crypto |
