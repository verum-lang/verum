# `core/encoding/msgpack` — conformance audit

| | |
|---|---|
| Module | `core.encoding.msgpack` |
| Spec | MessagePack format specification |
| Tier | regression-only (pure-data ADT surface) |
| Status | **partial** — MsgPackError scalar 5/10 variants covered; record-payload variants (ArrayTooLarge / MapTooLarge / StringTooLong / BinTooLong / ExtTooLong) deferred behind field-destructure defect |

## What's covered (`unit_test.vr` — 17 @test GREEN under `--interp`)

### §1 — MsgPackError scalar variants (5)
- Truncated(Int)
- InvalidPrefix(Byte)
- InvalidUtf8
- NestingTooDeep
- TrailingBytes(Int)

### §2 — Variant pairwise disjointness — scalars (4)
Each scalar variant tested against its sibling scalars.

### §3 — Payload preservation (3)
Truncated / InvalidPrefix / TrailingBytes scalar payloads
round-trip through construction + match.

### §4 — Match exhaustiveness — scalar arms (5)
`describe_scalar(e) -> Text` covers the 5 scalar variants with a
catch-all `_` for the 5 record-payload variants.

## Deferred

### §A — Record-payload variants (5 of 10)
- ArrayTooLarge { declared: Int, limit: Int }
- MapTooLarge { declared: Int, limit: Int }
- StringTooLong { declared: Int, limit: Int }
- BinTooLong { declared: Int, limit: Int }
- ExtTooLong { declared: Int, limit: Int }

All gated on the same record-destructure-through-match-binding defect
that gates `Base58Error.InputTooLong { len, limit }` extraction —
sister of [[btree_pattern_match_ref_generic_class]] field-index
resolver. The construction itself works, but
`MsgPackError.X { declared, limit } => ...` arm mis-binds.

### §B — MsgPackValue 11-variant ADT
- Nil / Bool(Bool) / PositiveInt(UInt64) / NegativeInt(Int) /
  Float32(Float) / Float64(Float) / Str(Text) / Bin(List<Byte>) /
  Array(List<MsgPackValue>) / Map(List<(MsgPackValue, MsgPackValue)>) /
  Ext { type_id, data }
Recursive structure tests deferred — likely hits the same field-index
resolver class as MsgPackError record variants.

### §C — Encode/decode round-trip
Gated on the same byte-array compile-time SIGSEGV class that gates
Base58 + CBOR round-trip. This is the
[EXTSLICE-1 / BSTRLIT-1](../../../internal/website/docs/stdlib/defect-class-catalogue.md)
family applied to byte-array element-addr lowering. The
EXTSLICE-1 byte-push discipline has already been applied at the
stdlib side to `core/encoding/msgpack.vr` (commit `ab9ec931b`);
residual gate is on the byte-array-on-stack defect.

### §D — MessagePack canonical vectors
Pinned vector tests deferred until §C unblocks:
- positive fixint (0..0x7F) round-trip
- negative fixint (-32..-1) round-trip
- fixstr / str8 / str16 / str32 boundaries
- fixarray / array16 / array32 boundaries
- ext type-id round-trip (timestamp ext-1)

## Tier-1 (AOT) gate

Same as other encoding/ modules — pre-existing AOT stdlib build
blocker (task #7).
