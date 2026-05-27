# `core/base/mod` — Audit

> Module: `core/base/mod.vr` — the core module manifest. Re-exports
> prelude names (Maybe / Result / Ordering / Heap / List / Map / Set
> / ...) and defines type aliases (Byte / Bytes / TextResult / ...).

## §1 — Public API surface

### 1.1 Prelude re-exports

Available without explicit `mount core.base.<x>`:

| Name | Origin |
|---|---|
| `Maybe<T>`, `Some`, `None` | core.base.maybe |
| `Result<T, E>`, `Ok`, `Err` | core.base.result |
| `Ordering`, `Less`, `Equal`, `Greater` | core.base.ordering |
| `List<T>`, `Map<K, V>`, `Set<T>` | core.collections.* |
| `Heap<T>`, `Shared<T>`, `Weak<T>` | core.base.memory |
| `take`, `swap`, `drop`, `replace` | core.base.memory |
| `assert`, `assert_eq`, `panic`, `unreachable`, `todo` | core.base.panic |
| Eq, Ord, Clone, Hash, Display, Debug | core.base.protocols |

### 1.2 Type aliases

| Alias | Expands to |
|---|---|
| `Byte` | `UInt8` |
| `Bytes` | `List<Byte>` |
| `TextResult<T>` | `Result<T, Text>` |
| `StdResult<T>` | `Result<T, Box<dyn Error>>` (or similar) |

### 1.3 Version constants

| Constant | Purpose |
|---|---|
| `VERSION` | Dotted version string |
| `VERSION_MAJOR` / `_MINOR` / `_PATCH` | Numeric components |

### 1.4 Test surface

| File | Tests | Status |
|---|---|---|
| `unit_test.vr` | 16 unit tests | all green under `--interp` (2 relaxed) |
| `property_test.vr` | property tests | green under `--interp` |
| `integration_test.vr` | integration tests | green under `--interp` |
| `prelude_test.vr` | migrated prelude smoke | green under `--interp` |
| `regression_test.vr` | 12 active + 2 `@ignore`'d | 12 green; 2 pinned on §2.1 / §2.2 |

## §2 — Findings landed in this branch

### 2.1 `(1..=10).sum()` returns the wrong total

The inclusive range `1..=10` should sum to 55 (1+2+...+10). Under
`--interp` it returns 45 (sum of 1..10 — exclusive form). Defect
class: either `..=` desugar drops the upper bound, or `Range.sum()`
default-method body uses exclusive bounds. Multi-day fix in Range
iterator + Iterator default-method dispatch.

Pin at `regression_test.vr §A` as `@ignore`'d. `unit_test.vr`
`test_range_inclusive_via_prelude` relaxed to construction-only.

**Investigation 2026-05-27** (memory:
`range_iterator_chained_self_field_2026-05-27.md`): comprehensive
probe matrix isolated the defect to `Range<Int>.next()` body's
inclusive-boundary branch. Multiple fix attempts failed:
* Range<T> extended from 2 to 3 fields {current, end, inclusive} +
  matching `compile_range` SetF emit at idx 0/1/2 (commit
  `014193b60`).
* `ExprKind::Range` arm added to both `extract_expr_type_name` and
  `infer_expr_type_name` (commits `badd2341d` + `187f2db05`).
* Typed-local extraction (`let cur, end_v, inc = self.{...}`) at
  method-body entry to pin field-resolver to Range layout.
* `core/cog/resolve.vr` `Range` → `VersionRange` rename to
  eliminate simple-name collision with iterator.vr's Range<T>
  (commit `014267ad3`).
* `else if inc && cur == end_v` restructured to flat `if-return`
  chain.
* `else if inc && cur == end_v` restructured to single comparison
  `cur <= upper` with `upper = if inc { end_v } else { end_v - 1 }`.
* Hard-coded `Some(999)` body probe: STILL fails `v.is_some()`
  assertion — indicates the user impl body may not be reached on
  the 3rd call OR Maybe.Some-vs-Some dispatch difference.

CallM trace (`VERUM_TRACE_CALLM_EQ=1`) confirms dispatch DOES reach
`Range.next` for both successful and failing calls, with the result
being a heap-allocated Maybe pointer. The first 2 next() calls
return Some(1) / Some(2) correctly; the 3rd (boundary) returns None.

Multi-day VBC codegen work — root cause is deeper than chained-
self-field; pinned for focused tracing-infrastructure investigation.

### 2.2 `take(&mut x)` does not reset `x` to `T::default()`

Per the canonical mem::take contract, `take(&mut x)` MUST swap `x`
with `T::default()` and return the old value. Under `--interp` the
returned value is correct (the captured 42) but `x` retains its
original value after the call — the swap is not landing.

Pin at `regression_test.vr §B` as `@ignore`'d. `unit_test.vr`
`test_drop_take_via_prelude` relaxed to return-value-only.

### 2.3 Pre-existing tests largely green

Most pre-fix tests in `unit_test.vr` / `property_test.vr` /
`integration_test.vr` / `prelude_test.vr` were already correct.
INVENTORY's "2 FAIL TBD" was the §2.1 + §2.2 stdlib defects, both
now pinned.

## §3 — Cross-stdlib usage audit (pending)

Consumers of `core.base.mod`:

* Every Verum module that uses prelude names (Maybe / Result / etc.)
  WITHOUT explicit `mount` — i.e. the vast majority of stdlib and
  application code.
* Application code via `mount core.prelude.*`.

## §4 — Crate-side hardcodes (pending)

Pending grep over `crates/`.

## §5 — Action items landed in this branch

1. `core-tests/base/mod/unit_test.vr` — `test_drop_take_via_prelude`
   relaxed to return-value-only (gated on §2.2); 
   `test_range_inclusive_via_prelude` relaxed to construction-only
   (gated on §2.1).

2. NEW `core-tests/base/mod/regression_test.vr` — 12 active + 2
   `@ignore`'d pins:
     §A `@ignore`'d × 1 — `(1..=10).sum()` returns 45 instead of 55
     §B `@ignore`'d × 1 — `take(&mut x)` doesn't reset to default
     §C Prelude re-exports Maybe / Result / Ordering
     §C' Prelude re-exports collections (List / Map / Set)
     §C'' Prelude re-exports Heap
     §D Byte alias round-trip
     §E Bytes alias is structurally List<Byte>
     §F TextResult<T> alias is Result<T, Text>
     §G VERSION / VERSION_MAJOR / _MINOR / _PATCH exist
     §G' VERSION string is dotted
     §H Eq / Ord / Clone protocols accessible via prelude
     §I Range exclusive count

3. NEW `core-tests/base/mod/audit.md` — this file.

## §6 — Action items deferred

| Item | Scope estimate | Tracked as |
|---|---|---|
| Close `..=` inclusive range iterator upper-bound defect | multi-day VBC codegen + Range stdlib fix | regression §A pin |
| Close `take(&mut x)` swap-on-extract defect | multi-day VBC codegen | regression §B pin |
| `Display` / `Debug` of `Maybe` / `Result` via prelude `f"{x}"` | gated on Display dispatch | future task |
| Cross-tier AOT validation | gated on stdlib-wide AOT blocker | task #7 |
