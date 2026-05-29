# `net/tls13/alert` audit

Module: `core/net/tls13/alert.vr` — RFC 8446 §6 alert protocol:
`AlertLevel` (Warning=1, Fatal=2) + to_u8/from_u8, and
`AlertDescription` (RFC 8446 §6 wire codes) + to_u8.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.tls13.record` | alerts ride the alert content-type record. |
| `core.net.tls13.handshake` | handshake failures emit a fatal alert. |
| `core.net.quic.error` | CRYPTO_ERROR = 0x0100 + alert code. |

## 2. Crate-side hardcodes

AlertLevel Warning=1 / Fatal=2 and the AlertDescription wire codes
(CloseNotify=0, UnexpectedMessage=10, BadRecordMac=20, HandshakeFailure=40,
DecodeError=50, DecryptError=51, ProtocolVersion=70, InternalError=80, …)
are RFC 8446 §6 verbatim. from_u8 uses a conservative Fatal default for
unrecognised levels (pinned).

## 3. Language-implementation findings

None. AlertLevel is a 2-variant unit enum with round-trip; AlertDescription
is a ~25-variant unit enum with to_u8. Tag-only `match` dispatch, no slices,
compiles cleanly under `--interp`.

## 4. Action items landed in this branch

* `unit_test.vr` — AlertLevel to_u8/from_u8 + round-trip + Fatal-default;
  12 AlertDescription wire-code pins + disjointness sample.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| AlertDescription.from_u8 (if added) round-trip | this folder | 1h |
| Full AlertDescription wire-code table (all ~25) | this folder | 1h |
| Alert record encode/decode round-trip | tls13/record | gated on EXTSLICE-1 |
