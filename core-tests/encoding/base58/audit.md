# `core/encoding/base58` — conformance audit

| | |
|---|---|
| Module | `core.encoding.base58` |
| Spec | Bitcoin-style Base58 (no canonical RFC) + Base58Check |
| Tier | regression-only (pure-data ADT surface) |
| Status | **partial** — Base58Error 4-variant ADT covered; encode/decode round-trip gated on byte-array compile-time SIGSEGV |

## What's covered (`unit_test.vr` — 13 @test GREEN + 3 @ignored under `--interp`)

### §1 — Base58Error 4-variant construction (4)
- InvalidCharacter(Byte)
- ChecksumMismatch
- TruncatedChecksum
- InputTooLong { len: Int, limit: Int }

### §2 — Variant pairwise disjointness (4)

### §3 — Match exhaustiveness over `describe(e) -> Text` (4)

### §4 — Payload preservation (1 GREEN + 1 @ignored)
- InvalidCharacter(Byte) — GREEN
- InputTooLong record-destructure — @ignored (sister of
  [[btree_pattern_match_ref_generic_class]] — record-field index
  resolver loses field-name mapping through match-binding)

## Deferred

### §A — encode/decode round-trip
Tests pinning the round-trip surface trip a compile-time SIGSEGV
during VBC lowering. Repro: the test body invokes
`encode(&[0_u8])` against a `[Byte; N]` fixed-array. Likely related
to byte-array element-addr lowering (sister of
[[typed_array_ref_addr_closed_2026-05-25]] residual or a different
byte-array-on-stack defect). 2 placeholder tests pinned @ignored.

**Source-side preventive fix landed 2026-05-29** (commit `41882e63b`):
3 EXTSLICE-1 sites inside `core/encoding/base58.vr` (encode buf-copy,
encode_check payload+checksum concat, decode_check payload trim)
replaced with byte-by-byte push walks per the
[EXTSLICE-1 discipline](docs/architecture/defect-class-catalogue.md#1-extend_from_slice-intrinsic-chain-sigsegv).
This eliminates one potential SIGSEGV trigger surface but does not
address the byte-array element-addr lowering defect that gates §A.

### §B — Bitcoin-style canonical vectors
Deferred until §A unblocks:
- "111ZiQRDqUSnLNm" → known-byte-string vector
- leading-zero preservation: 0x00 → '1', 0x00 0x00 → '11', etc.
- alphabet exclusions: '0' / 'O' / 'I' / 'l' all reject as
  InvalidCharacter
- Base58Check 4-byte SHA256-SHA256 trailing checksum verify

### §C — Display / Debug / Eq impls
Implementations exist; gated on same stub-cascade family.

## Tier-1 (AOT) gate

Same as other encoding/ modules — pre-existing AOT stdlib build
blocker (task #7).
