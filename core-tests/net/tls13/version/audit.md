# `net/tls13/version` audit

Module: `core/net/tls13/version.vr` — RFC 8446 §4.1.2/§4.2.1 protocol
versions: `ProtocolVersion { major, minor }`, `TLS_1_3` = {3,4},
`LEGACY_VERSION_TLS_1_2` = {3,3}, to_u16/from_u16/eq, plus the
RFC 8446 §4.1.3 downgrade-protection sentinels.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.tls13.record` | legacy_record_version field. |
| `core.net.tls13.handshake` | supported_versions extension; downgrade sentinels. |

## 2. Crate-side hardcodes

`TLS_1_3 = {0x03, 0x04}` (wire 0x0304) and
`LEGACY_VERSION_TLS_1_2 = {0x03, 0x03}` (wire 0x0303) are RFC 8446 §4.1.2
verbatim. Pinned by explicit field + to_u16 + round-trip tests.

## 3. Language-implementation findings

None. ProtocolVersion is a 2-`UInt8`-field record + to_u16/from_u16/eq.
Compiles cleanly under `--interp`.

The `DOWNGRADE_SENTINEL_TLS12 / _TLS11` `[Byte; 8]` constants are not
directly pinned here — fixed-array const access at a user-test boundary is
deferred pending array-literal codegen verification (sister of the QuicFrame
fixed-array variant deferral).

## 4. Action items landed in this branch

* `unit_test.vr` — version constants (major/minor); to_u16 wire form;
  from_u16 + round-trip + eq reflexivity + TLS1.3≠legacy.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| DOWNGRADE_SENTINEL_TLS12/11 byte-array pins | this folder | gated on array-literal codegen |
| supported_versions extension round-trip | tls13/handshake | gated on codec |
