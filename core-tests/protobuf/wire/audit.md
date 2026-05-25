# `protobuf/wire` audit

Module: `core/protobuf/wire.vr` (~394 LOC) — Protocol Buffers wire
format. proto3 v3 canonical encoding: tags + varints + length-delim
records + ProtobufCursor reader state machine.

Tests: 24 unit tests over WireType 4-variant + .to_u8 canonical
proto3 spec table + tag_value bit-packing + MAX_LENGTH_DELIM DoS-guard
constant.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.protobuf.codec` | encode/decode loop calls write_varint / read_varint / write_tag |
| `core.mesh.xds.client` | xDS DiscoveryRequest / DiscoveryResponse serialisation |
| `core.database.postgres.adapter` | gRPC pgwire via JSON-over-protobuf endpoints |
| `verum_runtime::proto::dispatch` | every proto3 message decode path reads tags via this module |

## 2. Crate-side hardcodes

| site | hardcode |
|---|---|
| `verum_runtime::proto::wire_type` mirrors WireType 4-variant. Drift breaks wire compat with every protobuf consumer (gRPC, Envoy, Istio). |
| `verum_runtime::proto::varint_codec` mirrors the 10-byte UInt64 varint encoding rule + canonical-rejection of non-canonical 10th-byte encodings. Drift here lets adversaries inject overflowing varints. |
| `verum_runtime::proto::dos_guard` mirrors MAX_LENGTH_DELIM=32 MiB. This is THE single DoS-guard for protobuf decoders — any consumer that bypasses it is a security bug. |

## 3. Language-implementation gaps

### §3.1 from_u8 + Result chain not tested

`WireType.from_u8(b)` returns `Result<WireType, ProtobufError>` —
the Result type carries Ok/Err variants. Testing requires importing
ProtobufError and pattern-matching. Cross-module record-return defect
makes this awkward.

Tests deferred:
* `WireType.from_u8(0) is Ok(Varint)` (and the 4 valid byte codes 0/1/2/5)
* `WireType.from_u8(3)` returns `Err(UnsupportedGroup)`
* `WireType.from_u8(4)` returns `Err(UnsupportedGroup)`
* `WireType.from_u8(6) ... .from_u8(255)` returns `Err(InvalidWireType(b))`

### §3.2 Varint round-trip tests deferred

`write_varint(buf, v)` + `read_varint(buf)` round-trip exercises the
List<Byte> + cursor mutation chain. Both hit cross-module record-return
defect for the Result<T, E> return.

Property tests deferred:
* Round-trip identity: read_varint(write_varint([], v).as_slice()) == Ok(v) for canonical v
* Length boundary: write_varint([], 127).len() == 1
* Length boundary: write_varint([], 128).len() == 2
* Length boundary: write_varint([], u64::MAX).len() == 10
* Non-canonical encoding rejection at 10th byte

### §3.3 ProtobufCursor state machine tests deferred

The cursor record + .read_tag / .read_varint / .skip_field methods
operate on owned List<Byte>. Same defect class.

## Action items landed in this branch

* `core-tests/protobuf/wire/unit_test.vr` — 24 unit tests:
  - WireType 4-variant + 4-way disjointness
  - .to_u8 canonical proto3 wire-tag table (Varint=0, Fixed64=1,
    LengthDelim=2, Fixed32=5; explicit absence of deprecated SGROUP=3
    and EGROUP=4)
  - tag_value (field_number << 3 | wire_type) bit-packing:
    field=1 + all 4 wire-types + field=2 / field=15 (1-byte boundary) /
    field=16 (2-byte boundary) / field=2047 (max 2-byte) /
    field=536870911 (max proto3 field number)
  - MAX_LENGTH_DELIM=32MiB DoS-guard pinned to exact value (33554432)
    + positive + under Int32 max
* `core-tests/protobuf/wire/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| WireType.from_u8 + Result chain (§3.1) | this folder | 30 min after cross-module fix |
| Varint write/read round-trip (§3.2) | this folder | 1-2 h after cross-module fix |
| ProtobufCursor state machine (§3.3) | this folder | 2 h |
| Property test: varint encoded length matches the boundary table | this folder | 30 min |
| Property test: tag_value invertible — extract field_number / wire_type from packed tag | this folder | 30 min |
| Drift-pinning Rust unit test for WireType wire-tag codes | crates/verum_runtime/src/proto/wire_type.rs | 30 min |
