# `protobuf/wire` audit

Module: `core/protobuf/wire.vr` (~394 LOC) — Protocol Buffers wire
format. proto3 v3 canonical encoding: tags + varints + length-delim
records + ProtobufCursor reader state machine.

Tests: 42 unit tests over WireType 4-variant + .to_u8 canonical
proto3 spec table + tag_value bit-packing + MAX_LENGTH_DELIM DoS-guard
constant + varint write/read round-trip + fixed32/64 endianness +
float32/64 wire bytes + WireType.from_u8 Result chain + ProtobufCursor
tag reads.

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

### THE THREE DEFERRALS BELOW WERE RE-MEASURED 2026-09-12 AND ARE CLOSED

§3.1, §3.2 and §3.3 all rested on one premise — "cross-module
record-return defect" — and all three named it as the reason the tests
could not be written. The premise was tested directly at Tier 0, from a
consumer module, against the shipped `core.protobuf.wire`:

    WireType.from_u8(0)         Result.Ok(Varint)          §3.1 route
    WireType.from_u8(3)         Result.Err(UnsupportedGroup)
    write_varint(&mut b, 300)   [172, 2]                   §3.2 route
    read_varint(&b, 0)          Result.Ok((300, 2))        tuple fields read
    ProtobufCursor.new(&b)
        .read_tag()             Result.Ok((1, Varint))     §3.3 route

A `Result<T, E>` crosses the module boundary, a `List<Byte>` buffer is
mutated through `&mut` across it, and tuple-field access works on the
returned pair. The tests are now in `unit_test.vr` sections 5-10.

### What the deferral cost

While the buffer surface went untested, `write_float32` and
`write_float64` called `@intrinsic("verum.float.f32_to_bits")` and its
f64 twin — keys the intrinsic registry has never carried. An
unregistered `@intrinsic` lowers to a Panic, so encoding ANY float or
double aborted the process, and every test in this file passed. The
registered names are the BARE `f32_to_bits` / `f64_to_bits`, which
`core.metrics` and `core.database.mysql` were already calling.

Measured at Tier 0 with `write_fixed32` as the control on the same
route: control gave `[0, 0, 128, 63]`, subject gave the Panic. After the
reroute both give `[0, 0, 128, 63]` for 1.0, and `[0, 0, 0, 0]` for 0.0.

The general lesson, which is not about protobuf: a deferral whose stated
blocker is a DEFECT ELSEWHERE outlives the defect unless someone re-runs
it. Three sections here named the same blocker and none was re-tested
after it was fixed.

### §3.1 from_u8 + Result chain — CLOSED (section 8)

Four live wire codes accepted (0/1/2/5), the two proto2 group codes
refused with their OWN error variant, and 6/7/255 refused.

### §3.2 Varint round-trip — CLOSED (sections 5 and 9)

Round-trip identity, the 127/128 one-to-two-byte boundary, and zero
encoding as one byte rather than as nothing.

Still open, and genuinely so: the 10-byte `UInt64.MAX` length and the
non-canonical-10th-byte rejection. Those need a decoder-refusal path
this module does not yet expose to a caller.

### §3.3 ProtobufCursor state machine — CLOSED (sections 10 and 11)

Reads back a tag this module wrote, at field 1 and at field 16 (the
first two-byte tag), and refuses on an empty buffer. Section 11 then
walks a whole message — three fields of three wire types — and asks
`at_end` before the first read and after the last, which is the
condition the `while !cursor.at_end()` form in the reference depends on.
`.skip_field` remains untested.

That walk also took `protobuf.md` to zero unexercised documented
methods: `at_end`, `read_varint`, `read_string` and `read_fixed64` were
all on the repository's exercised-methods roster and are not any more.

## Action items landed in this branch

* `core-tests/protobuf/wire/unit_test.vr` — 42 unit tests:
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
| Varint 10-byte `UInt64.MAX` length + non-canonical 10th-byte rejection | this folder | 1 h — needs a caller-visible refusal path |
| `ProtobufCursor.skip_field` | this folder | 30 min — the multi-field walk landed with section 11 |
| Property test: tag_value invertible — extract field_number / wire_type from packed tag | this folder | 30 min |
| Drift-pinning Rust unit test for WireType wire-tag codes | crates/verum_runtime/src/proto/wire_type.rs | 30 min |
