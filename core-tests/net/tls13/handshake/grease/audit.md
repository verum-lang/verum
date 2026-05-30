# `net/tls13/handshake/grease` audit

Module: `core/net/tls13/handshake/grease.vr` — RFC 8701 GREASE:
`is_grease(code)` predicate + `pick_grease(entropy)` selector +
convenience extension/cipher/group emitters.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.tls13.handshake` | ClientHello GREASE cipher/group/extension injection. |

## 2. Crate-side hardcodes

GREASE codepoints are the 16 reserved `0x?A?A` values (low byte == high
byte, low nibble of each byte == 0xA), per RFC 8701. `is_grease` matches
exactly that pattern; `pick_grease` always returns one of the 16.

## 3. Language-implementation findings

None. `is_grease` is pure `UInt16` bit arithmetic; `pick_grease` is a
modulo index into the 16-value table. Compile cleanly under `--interp`.

## 4. Action items landed in this branch

* `unit_test.vr` — is_grease positive (4) + negative (3, incl. real cipher
  suite + byte-mismatch + wrong-nibble) + pick_grease-is-always-grease sweep.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| grease_extension / grease_cipher / grease_group emitter coverage | this folder | 1h |
| pick_grease uniform-distribution property | this folder | 1h |
