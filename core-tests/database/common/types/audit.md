# `database/common/types` audit

Module: `core/database/common/types.vr` (~912 LOC) — Spindle wire-format
types: TypeOid, WireFormat, ColumnSchema, RowSchema, WireValue, WireRow,
WireBuf (mutable buffer), WireReader, WireReadError, ArenaSlice +
ArenaWireBuf / ArenaWireValue / ArenaWireRow.

Tests: 39 unit tests over the data-only subset: 21 OID constants +
WireFormat 2-variant + WireValue 2-variant + WireReadError 2-variant +
Eq matrix.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.database.postgres.adapter` | encodes parameter values into a `WireBuf` per Postgres v3 binary protocol; decodes server messages via `WireReader` |
| `core.database.mysql.adapter` | binary protocol uses `WireValue.WvPresent { format: FmtBinary }` |
| `core.database.sqlite.adapter` | maps SQLite serial-type encoding → `WireValue { format: FmtBinary }` |
| `core.database.common.protocol` | `RowSchema.columns` describes the shape returned by every adapter |
| `verum_runtime::db::oid_codec` | mirrors the 21 OID constants for runtime decode dispatch |

## 2. Crate-side hardcodes

* `verum_runtime::db::oid_table` mirrors the 21 OID values
  (NULL=0 .. UNKNOWN=20). This file's tests are the canonical
  drift pin — any new OID added MUST be added in 3 places.
* `verum_runtime::db::wire_protocol` mirrors WireFormat / WireValue
  variant set. `FmtBinary` and `FmtText` map to Postgres
  `RowDescription.format_code` fields directly.

## 3. Language-implementation gaps

### §3.1 TypeOid / ColumnSchema / RowSchema record-ctor tests deferred

These are record types with cross-module factories:
* `TypeOid.new(value: Int) -> TypeOid`
* `ColumnSchema.new(name, type_oid, nullable, format) -> ColumnSchema`
* `RowSchema.empty() -> RowSchema` / `.from_columns(cols) -> RowSchema`
* All `.with_*` builder methods on these records

Subsequent field access (`.value`, `.columns`, `.name`, `.type_oid`,
`.column_count`, `.index_of`) hits the cross-module record-return
defect class (see `meta/span` audit §3.1).

Direct record-literal construction at the test site IS feasible — e.g.
`TypeOid { value: 7 }` — but the surrounding tests of `.column()`
accessor methods + `.index_of()` linear scan would still hit the
defect on the final field access.

### §3.2 WireRow / WireBuf / WireReader mutable-state tests deferred

These types carry mutable state (position, bytes List). The
`WireReader.read_u8/u16/u32_be/i32_be/i64_be/bytes(n)` chain needs
the `?` operator + `WireReader.position` accessor — both cross-module
returns. Deferred to integration suite.

### §3.3 Arena variants (ArenaSlice / ArenaWireBuf / ArenaWireValue /
ArenaWireRow) require live CBGR arena allocation

These types reference per-query CBGR arena memory and are only
meaningful within a connection-scoped arena context. Tested at the
verum_runtime layer.

## Action items landed in this branch

* `core-tests/database/common/types/unit_test.vr` — 39 unit tests:
  - 21 individual OID constant value pins (OID_NULL=0 ... OID_UNKNOWN=20)
  - OID family-pairwise-distinctness checks (numeric / float / text /
    temporal / composite + OID_UNKNOWN sentinel)
  - WireFormat 2-variant + disjointness
  - WireValue 2-variant + disjointness (WvPresent ctor with binary/text)
  - WireReadError 2-variant + disjointness + Eq matrix (reflexivity for
    both variants + payload-sensitivity (wanted differs, available differs) +
    cross-variant inequality)
* `core-tests/database/common/types/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| TypeOid / ColumnSchema / RowSchema record-ctor tests (§3.1) | this folder | 1-2 h after cross-module fix |
| WireRow / WireBuf / WireReader integration tests (§3.2) | this folder | 2-3 h |
| Arena variants tests (§3.3) | this folder + integration | 4 h |
| Property test: read_uN_be / put_uN_be round-trip preserves values exactly | this folder | 1 h |
| Drift-pinning Rust unit test for the 21 OID constants | crates/verum_runtime/src/db/oid_table.rs | 30 min |
