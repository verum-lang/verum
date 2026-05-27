# `core/base/data` — Audit

> Module: `core/base/data.vr` — `Data` (dynamic JSON-like value) and
> `DataError`. Surface for ad-hoc untyped configuration, JSON round-
> trip, and runtime-shape data.

## §1 — Public API surface

### 1.1 Types

| Type | Shape | Public? |
|---|---|---|
| `Data` | 7-variant sum `Null \| Bool(Bool) \| Int(Int) \| Float(Float) \| Text(Text) \| Array(List<Data>) \| Object(Map<Text, Data>)` | yes |
| `DataError` | sum (parse + type-coercion errors) | yes |
| `DataBuilder` | record — fluent builder for Object / Array shapes | yes |

### 1.2 Free functions

| Item | Signature |
|---|---|
| `parse_json` | `(Text) -> Result<Data, DataError>` |

### 1.3 DataBuilder API

| Item | Signature |
|---|---|
| `DataBuilder.new` | `() -> DataBuilder` (default = empty Object) |
| `.new_array` | `() -> DataBuilder` (default = empty Array) |
| `.set` | `(Self, Text, Data) -> DataBuilder` |
| `.push` | `(Self, Data) -> DataBuilder` |
| `.build` | `(Self) -> Data` |

### 1.4 Data accessors

| Item | Signature |
|---|---|
| `Data.len` | `(&self) -> Int` (variant-aware: Array/Object/Text len, else 0) |
| `Data.path` | `(&self, &Text) -> Maybe<Data>` (`a.b.c` dotted access) |
| `Data.to_json` | `(&self) -> Text` (canonical JSON serialization) |
| `Data.is_null` / `.is_bool` / `.is_int` / etc. | predicates |

### 1.5 Test surface

| File | Tests | Status |
|---|---|---|
| `unit_test.vr` | 60+ unit tests | green (1 `@ignore`'d for §2.1) |
| `property_test.vr` | property tests | green |
| `integration_test.vr` | integration scenarios | green |
| `regression_test.vr` | 9 active + 1 `@ignore`'d | 9 green; 1 pinned on §2.1 |

## §2 — Findings landed in this branch

### 2.1 Chained DataBuilder return-value defect

`let data = DataBuilder.new().set(k1, v1).set(k2, v2).set(k3, v3).build();`
returns a Data record with the correct shape, but downstream
`data.len()` mis-routes — same root family as `base/log §A` and
`base/mod §A`: chained-builder type tracking through `.build()`
return loses the receiver type, and the `.len()` method dispatch
picks the wrong variant body.

**Fix in this branch**: pinned `test_builder_chaining` as
`@ignore`'d in unit_test + `regression_test.vr §A`. Multi-day VBC
codegen fix.

### 2.2 Pre-existing tests largely green

After §2.1 pin, the remaining 60+ unit + property + integration
tests pass cleanly under `--interp`. Coverage is deep: 7-variant
Data ADT, DataError variants, type predicates, accessor coverage,
parse_json over canonical JSON, to_json round-trip on every variant
shape, DataBuilder.new / .set / .push / .build (single-call), path
traversal `a.b.c`.

## §3 — Cross-stdlib usage audit

Consumers of `core.base.data`:

* `core.configuration.*` — ad-hoc untyped config layer (toml / json /
  yaml all converge to Data).
* `core.cli.*` — argument JSON parsing.
* `core.encoding.*` — JSON / msgpack / cbor backends round-trip via Data.

## §4 — Crate-side hardcodes (pending)

Pending grep over `crates/`.

## §5 — Action items landed in this branch

1. `core-tests/base/data/unit_test.vr` — `test_builder_chaining`
   `@ignore`'d (§2.1 chained-builder defect).

2. NEW `core-tests/base/data/regression_test.vr` — 9 active + 1
   `@ignore`'d pins:
     §A `@ignore`'d × 1 — DataBuilder.build() chain returns wrong-len
     §B Data 7-variant ADT construction
     §C Data.Int and Data.Float are distinct variants
     §D Empty Array / Object construct cleanly
     §E Data.Null and Data.Bool(false) are distinct variants
     §F parse_json("null") returns Data.Null
     §F' parse_json("true") returns Data.Bool(true)
     §F'' parse_json("false") returns Data.Bool(false)
     §F''' parse_json("42") returns Data.Int or Data.Float
     §G parse_json rejects garbage / empty string

3. NEW `core-tests/base/data/audit.md` — this file.

## §6 — Action items deferred

| Item | Scope estimate | Tracked as |
|---|---|---|
| Close chained-builder return-value type tracking | multi-day VBC codegen work | regression §A pin (shared with base/log §A) |
| `parse_json` full RFC 8259 conformance suite (escapes / nested / unicode) | 2-3h | future task |
| `to_json` Data → Text round-trip property test exhaustive | 1h | future task |
| Cross-tier AOT validation | gated on stdlib-wide AOT blocker | task #7 |
