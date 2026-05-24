# `encoding/varint` audit

Module: `core/encoding/varint.vr` (~400 LOC) — SQLite-style + QUIC
varint encoding (1..9-byte big-endian variable-length integers per
SQLite file format spec + 1/2/4/8-byte length-prefixed QUIC varint
per RFC 9000 §16).

Tests: `unit_test.vr` (~14 unit tests pinning the
`sqlite_encoded_len` boundary-value table for lengths 1..5 plus
neg-1 / neg-2 / Int64.MIN all → 9-byte form, plus VarintErrorKind
disjointness).

Full encode/decode round-trip + canonical-vector tests live in
`vcs/specs/L2-standard/database/sqlite/l2_record/varint_roundtrip.vr`
— this file covers the static API surface.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.database.sqlite.native` | record-format integers + B-tree key encoding. |
| `core.protobuf.wire` | sibling LEB128 (separate impl). |
| `core.net.http3.frame` | sibling QUIC varint (separate impl) — colocated here. |

## 2. Crate-side hardcodes

None today. Future SIMD intercepts for batch varint decoding (a
common SQLite hot-path) must produce bit-identical output verified
by round-trip property tests.

## 3. Language-implementation gaps

### §3.1 No property test for length-class round-trip

Property law: ∀v. let len = encoded_len(v); decode(encode(v))
should yield v + consume exactly len bytes. Test in
property_test.vr would cover this exhaustively over boundary
values from each length class.

**Effort:** small (~1h).

### §3.2 Negative values: doc-stated 9-byte form not unit-tested for
       boundary case

The doc comment at varint.vr:42-44 states "negative values use
the full 9-byte form (their top bit is always set), unlike
LEB128 which zig-zags first". Pinned for -1 / -2 / Int64.MIN.
Should also pin a positive value > 2^56 to verify the
9-byte boundary on the positive side.

### §3.3 QUIC varint surface tested separately in `quic_encoded_len`

The QUIC variant uses 1/2/4/8-byte length prefix (much sparser
than SQLite). Add boundary tests for its 4-class split (0..63,
64..16383, 16384..(2^30-1), 2^30..(2^62-1)).

**Effort:** small (~30 min).

### §3.4 `VarintError` Eq impl missing

ErrorKind has @derive (implicit), but VarintError doesn't have a
manual Eq impl. Test pattern shared with HexError / Base64Error.

**Effort:** small (~30 min).

## Action items landed in this branch

* `core-tests/encoding/varint/unit_test.vr` — 14 unit tests
  covering VarintErrorKind 2-variant + sqlite_encoded_len boundary
  values for lengths 1..5 + negative values 9-byte form.
* `core-tests/encoding/varint/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add property_test.vr round-trip law (cross-referenced with vcs/specs) | this folder | 1h |
| Add positive-value 9-byte boundary tests | this folder | 15 min |
| Add QUIC varint boundary tests | this folder | 30 min |
| Add Eq impl for VarintError | `core/encoding/varint.vr` + 2 tests | 30 min |
| Migrate vcs/specs/.../varint_roundtrip.vr into this folder | git mv + frontmatter strip | 15 min |
