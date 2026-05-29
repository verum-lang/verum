# `net/tls13/record/content_type` audit

Module: `core/net/tls13/record/content_type.vr` — RFC 8446 §5.1 record
content types: `ContentType` 5-variant + to_u8/from_u8.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.tls13.record` | TLSPlaintext/TLSCiphertext record framing. |

## 2. Crate-side hardcodes

Invalid=0, ChangeCipherSpec=20, Alert=21, Handshake=22, ApplicationData=23
are RFC 8446 §5.1 / RFC 5246 §6.2.1 verbatim.

## 3. Language-implementation findings

None. 5-variant unit enum + to_u8/from_u8 `match`. Compiles cleanly under
`--interp`.

## 4. Action items landed in this branch

* `unit_test.vr` — 5 wire-code pins + from_u8 + from_u8∘to_u8==id round-trip.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Record framing round-trip per content type | tls13/record | gated on AEAD + EXTSLICE-1 |
