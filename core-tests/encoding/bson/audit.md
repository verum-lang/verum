# `core/encoding/bson` — conformance audit

| | |
|---|---|
| Module | `core.encoding.bson` |
| Spec | MongoDB BSON wire format |
| Tier | regression-only (pure-data ADT + constant surface) |
| Status | **partial** — BSON_* type-byte constants + BsonError 10-variant ADT covered; BsonValue / records / encode/decode deferred |

## What's covered (`unit_test.vr` — 27 @test GREEN under `--interp`)

### §1 — BSON type-byte constants (10)
MongoDB BSON spec type-byte numbering:
| Constant | Value | Use |
|---|---:|---|
| BSON_DOUBLE | 0x01 | 64-bit IEEE 754 float |
| BSON_STRING | 0x02 | UTF-8 string |
| BSON_DOCUMENT | 0x03 | embedded document |
| BSON_ARRAY | 0x04 | embedded array (index keys) |
| BSON_BINARY | 0x05 | binary subtype |
| BSON_UNDEFINED | 0x06 | deprecated; decoded as Null |
| BSON_OBJECT_ID | 0x07 | 12-byte ObjectID |
| BSON_BOOLEAN | 0x08 | true/false |
| BSON_DATETIME | 0x09 | int64 ms since Unix epoch |
| BSON_NULL | 0x0A | null sentinel |

### §2 — BsonError 10-variant ADT (10)
- BsonE_Truncated(Int)            — offset where read stopped
- BsonE_BadType(Byte)             — unrecognised type byte
- BsonE_BadLength(Int)            — length-prefix inconsistency
- BsonE_BadUtf8                   — invalid UTF-8 in string / cstring
- BsonE_NestingTooDeep
- BsonE_TooManyFields(Int)
- BsonE_DocumentTooLarge(Int)
- BsonE_BadDecimal128
- BsonE_TrailingBytes(Int)
- BsonE_MissingDocumentTerminator

### §3 — BsonError pairwise disjointness — scalar variants (4)

### §4 — BsonError scalar payload preservation (3)

## Deferred

### §A — Record types
- BsonObjectId { bytes: List<Byte> }      — 12-byte payload
- BsonBinary { subtype: Byte, bytes: List<Byte> }
- BsonRegex { pattern: Text, options: Text }
- BsonTimestamp { increment: UInt32, time: UInt32 }
- BsonDocument { ... }
All gated on the record-destructure through match-binding defect
class (sister of [[btree_pattern_match_ref_generic_class]]).

### §B — BsonValue 18-variant ADT
The full BSON value sum type covering Double / String / Document /
Array / Binary / Undefined / ObjectId / Boolean / DateTime / Null /
Regex / DBPointer / JavaScript / Symbol / JavaScriptScope / Int32 /
Timestamp / Int64 / Decimal128 / MinKey / MaxKey. Compound-payload
extraction gated on §A.

### §C — Wire encode/decode round-trip
- encode_document(&BsonDocument) -> List<Byte>
- decode_document(&[Byte]) -> Result<BsonDocument, BsonError>
Gated on byte-array compile-time class (sister of CBOR / Base58
round-trip) + Text-builder lenient-stub cascade for UTF-8 string
field handling.

### §D — BsonDocumentBuilder fluent API

### §E — MongoDB canonical vectors
Deferred until §C unblocks:
- Empty document: 0x05 0x00 0x00 0x00 0x00 (5-byte length + terminator)
- Single-field {"a": 1} (Int32 BSON_INT32=0x10)
- ObjectId round-trip (12-byte binary)
- Embedded nested document
- Array element index-key "0" / "1" / "2" string serialisation

### §F — Display / Debug / Eq impls
Implementations exist; gated on stub-cascade family.

## Tier-1 (AOT) gate

Same as other encoding/ modules — pre-existing AOT stdlib build
blocker (task #7).
