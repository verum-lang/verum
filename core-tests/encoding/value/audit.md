# `core/encoding/value` — conformance audit

| | |
|---|---|
| Module | `core.encoding.value` |
| Spec | Universal Value type for cross-codec conversion (CBOR/BSON/MessagePack interop) |
| Tier | regression-only (pure-data ADT surface) |
| Status | **partial** — Value scalar 8/12 variants + ConvertError scalar 5/6 variants covered |

## What's covered (`unit_test.vr` — 27 @test GREEN under `--interp`)

### §1 — Value scalar variants (8)
- VNull
- VBool(Bool)
- VInt(Int)
- VFloat(Float)
- VText(Text)
- VTimestamp(Int)  — ms since Unix epoch
- VUuid(List<Byte>)  — exactly 16 bytes
- VDecimal(List<Byte>)  — opaque 16-byte decimal128

### §2 — Value pairwise disjointness — scalars (4)
Each scalar variant tested against its peers.

### §3 — Value payload preservation — scalars (3)
VBool / VInt (Int.MIN+1 boundary) / VTimestamp round-trip.

### §4 — ConvertError scalar variants (5)
- CeUnsupportedVariant(Text)
- CeBytesNotUuid(Int)
- CeBytesNotDecimal(Int)
- CeMapKeyNotText
- CeExtensionUnsupported(Int)

### §5 — ConvertError pairwise disjointness — scalars (4)

### §6 — ConvertError scalar payload preservation (3)

## Deferred

### §A — Value compound variants (4 of 12)
- VBytes(List<Byte>)
- VArray(List<Value>)              — recursive ADT
- VMap(List<(Text, Value)>)        — recursive ADT
- VExtension { tag: Int, payload: List<Byte> }
All gated on (a) the record-destructure field-resolver defect class
or (b) recursive-ADT field-tracking through generic List<Self>.

### §B — ConvertError record-payload variant (1 of 6)
- CeIntegerOutOfRange { value: Int, target: Text }
Same field-destructure defect as Base58Error.InputTooLong and
MsgPackError.ArrayTooLarge.

### §C — Cross-codec conversion entry points
Conversion between Value ↔ CborValue / BsonValue / MsgPackValue is
gated on the encode/decode round-trip of those codecs, which all
trip the byte-array compile-time SIGSEGV or Text-builder
lenient-stub cascade. The SIGSEGV class is the
[EXTSLICE-1 / BSTRLIT-1](docs/architecture/defect-class-catalogue.md)
family applied to byte-array element-addr lowering — stdlib-side
byte-push discipline already applied to cbor/jcs/msgpack/base58
(commits `ab9ec931b` + `41882e63b`); the residual gate is the
byte-array-on-stack defect (multi-day VBC codegen work).

### §D — Display / Debug / Eq impls
Implementations exist; gated on stub-cascade family.

## Tier-1 (AOT) gate

Same as other encoding/ modules — pre-existing AOT stdlib build
blocker (task #7).
