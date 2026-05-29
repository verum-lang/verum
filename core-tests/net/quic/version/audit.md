# `net/quic/version` audit

Module: `core/net/quic/version.vr` (~52 LOC) — QUIC version
constants and predicates (RFC 9000 §15, RFC 9369, RFC 8701).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.packet` | long-header version field. |
| `core.net.quic.connection_sm` | version negotiation. |

## 2. Crate-side hardcodes

`VERSION_1=0x00000001` (RFC 9000) and `VERSION_2=0x6B3343CF`
(RFC 9369) are IANA-registered wire values; `VERSION_NEGOTIATION
=0x00000000` is the reserved VN sentinel. All three pinned by
explicit equality tests. The greasing pattern `0x?A?A?A?A`
(RFC 8701) is verified by construction over all high-nibble
choices, and falsified by single-nibble perturbation.

## 3. Language-implementation findings

None. Pure `UInt32` bit arithmetic — compiles and runs cleanly
under `--interp`.

## 4. Action items landed in this branch

* `unit_test.vr` — 12 tests: 3 constants, is_supported (v1 only),
  is_greased (positive + negative vectors).
* `property_test.vr` — 3 laws: single-version support; constructed
  greased versions; single-nibble-breaks-greasing.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Version-negotiation packet parse/build round-trip | quic/packet folder | gated on packet codec |
