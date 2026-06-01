# `context/standard` audit

Module: `core/context/standard.vr` (~700 LOC) — the canonical set of
DI context types: 10 `context` protocols (Logger, Database, Auth,
Config, Cache, Metrics, Tracer, Clock, Random, FileSystem) plus the
supporting data types (ContextLogLevel, ContextLogRecord, Row,
QueryResult, AuthUser).

Tests: `unit_test.vr` covers all data types (ContextLogLevel ADT
with severity/name/is_enabled + Eq/Ord/Clone/Copy/Debug/Display,
AuthUser.has_role, Row.get_index, QueryResult Display).

## 1. Cross-stdlib usage

`ContextLogLevel`:
* `core/base/log.vr` defines a SEPARATE `LogLevel` ADT (`Trace |
  Debug | Info | Warn | Error`). These two types co-exist — the
  `LogLevel` is the lower-level base/log integration; the
  `ContextLogLevel` is the context-system DI surface (includes
  `Fatal` as a 6th variant).

`Row`, `QueryResult`:
* Mirror the row/result types in `core/database/common/*` (with
  potentially different field layouts). The context-system `Row`
  is the test-time seam; production code uses the database-module
  types directly.

`AuthUser`:
* Used as the return type of `Auth.current_user() -> Maybe<AuthUser>`.
  No other consumer in `core/`.

The 10 context protocols themselves are user-facing: applications
provide concrete implementations at startup.

## 2. Crate-side hardcodes

`crates/verum_compiler/src/phases/context_check.rs` recognises
these context type names as the "well-known" set and assigns them
the compile-time slot IDs (0..N range) used by `env_ctx_get` /
`env_ctx_set` for the O(1) slot fast-path. Drift here is caught
by integration tests of the runtime ctx_bridge.

The `Tracer` context's `Span` type is `core.tracing.span.Span`
(NOT a parallel definition in standard.vr); this is documented at
the top of the Tracer section in `standard.vr` and verified at
compile time by cross-module name resolution.

## 3. Language-implementation gaps

### §3.1 Two `LogLevel` types (`base/log.LogLevel` vs `context/standard.ContextLogLevel`)

The duplication is intentional (different audiences, different
variant sets — Fatal is in Context but not Base) but easily
confusing. `unit_test.vr` consistently uses `ContextLogLevel` to
dodge the bare-name collision. Document the distinction in
`standard.vr` doc-comment and in the website docs.

**Effort:** trivial (~10 min doc edit).

### §3.2 `Row.get(name: &Text) -> Maybe<&Text>` linear scan

```verum
public fn get(&self, column: &Text) -> Maybe<&Text> {
    for i in 0..self.columns.len() {
        if &self.columns[i] == column {
            return self.values[i].as_ref();
        }
    }
    None
}
```

Linear scan is fine for tens of columns but quadratic for joins
with hundreds of result columns. Acceptable for the test-seam
contract, but the production database `Row` types should use a
hash-indexed column lookup. Document the trade-off; add a
`Row.get_indexed(idx: Int) -> Maybe<&Text>` accessor (already
exists as `get_index`) for callers that pre-resolved the index.

**Effort:** documentation only.

### §3.3 Context protocols cannot be unit-tested in pure Verum

The `context Logger {}` / `context Database {}` etc. declarations
require compiler support for `provide` / `using` to instantiate.
Without that, `unit_test.vr` here is restricted to the DATA TYPES
that back the protocols. The protocols themselves are tested at
the language level (in `vcs/specs/L2-standard/contexts/`), not
in this folder. Document this clearly in `core-tests/README.md`.

**Effort:** ~30 min doc + cross-reference.

### §3.4 No `ContextLogLevel.from_severity(Int) -> Maybe<ContextLogLevel>`

The forward map `severity()` is one-way today. Adding
`from_severity(n)` makes `ContextLogLevel` round-trippable via
its ordinal — useful for serialization (e.g. Prometheus log-level
labels). Add it and pin the round-trip law in property_test.vr.

**Effort:** ~30 min impl + test.

### §3.5 `Row.get_index` / `Row.get` SIGSEGV — archive-method `Maybe<&T>` return (NEW, HIGH)

Calling `Row.get_index(n)` (or `Row.get(&name)`) from user code and
consuming the result **SIGSEGVs the compiler** during execution-compile
(signal 11, hard corruption — not a clean panic). Both methods return
`Maybe<&Text>` borrowed from `self.values[i].as_ref()`.

Triangulation (each isolated, `--interp --test-threads 1`):

| construct | result |
|---|---|
| `Maybe.Some(x).as_ref() is Maybe.Some` | OK |
| `(xs: List<Maybe<Text>>)[0].as_ref() is Maybe.Some` | OK |
| LOCAL record `Bag { items }` w/ `at(i){ self.items[i].as_ref() }`, consumed | OK |
| archive-loaded `Row.get_index(0) is Maybe.Some` | **SIGSEGV** |
| same via `match` | **SIGSEGV** |

So the trigger is the **cross-module / archive-loaded method-return of a
reference-bearing ADT** — NOT `Maybe<&T>` per se, NOT List-of-Maybe
indexing, NOT the `is`-vs-`match` consumer. Same family as
[[btree_pattern_match_ref_generic_class]] / CLASS-9 / D2 (recent commits
64607bb8e, 1e75b40ad). `Display for Row` is also affected (it does the
same `self.values[i].as_ref()`).

Pinned in `regression_test.vr` (`regression_row_get_index_bounds_guarded`,
`@ignore`'d) and the three `unit_test.vr` `get_index` tests are `@ignore`'d.
**Verified an `@ignore`'d test never trips the crash** (it is not
execution-compiled). Fundamental fix is VBC codegen of archive-method
ref-ADT returns + a compiler rebuild — deferred (the codegen crate is
actively edited by a concurrent session; rebuild is hazardous this cycle).

### §3.6 `f"{Type.Variant}"` does not dispatch `Display` (NEW, MEDIUM)

A DIRECT variant-constructor in an interpolation placeholder
(`f"{ContextLogLevel.Info}"`) renders the variant name (`"Info"`) instead
of the `Display` output (`"INFO"`). Binding first
(`let l = ContextLogLevel.Info; f"{l}"`) dispatches `Display` correctly.
`Debug` (`:?`) works in both forms. General (not context-specific) — the
same enum-`Display`-under-`--interp` regression another session pinned for
`runtime/*` enums. Pinned `@ignore`'d in `regression_test.vr`
(`regression_display_direct_ctor_renders_uppercase_name`); the live
`regression_display_via_bound_var_dispatches` + the rewritten
`property_display_equals_name` / `test_log_level_display_returns_name` keep
the `Display` contract covered via the working bound-var idiom.
Tracked as [[fstring_direct_variant_ctor_display_dispatch]].

### §3.7 Row field-shift on cross-module direct field read (NEW, part of CLASS-9)

Reading `Row`'s own fields (`r.columns` / `r.values`) from USER code (e.g.
through `mount core.context.*`) panics `field access out of bounds: field
index 4 ... type='List'` — the archive-loaded record's field index is
mis-resolved. `AuthUser` fields (`u.id`/`u.name`/`u.roles`) and
`QueryResult.rows` read correctly, so the shift is type/layout-specific to
`Row`. `mod/unit_test.vr`'s Row test was reworked to read only
`qr.rows.len()` (the working path). Same CLASS-9 family as §3.5.

## Conformance status (2026-06-01, interpreter / `--test-threads 1`)

`ContextLogLevel` (severity / name / is_enabled / Eq / Ord / Clone / Debug
/ Display-via-bound-var), `AuthUser` (has_role / Display), `QueryResult`
(construction / Display / `.rows`) are GREEN. Blocked + pinned: `Row.get`
/ `Row.get_index` / `Row` direct field read / `f"{Type.Variant}"` Display.
Status: **partial** (was `regression-only`).

## Action items landed in this branch

* `core-tests/context/standard/unit_test.vr` — surface coverage for
  ContextLogLevel + data types; `get_index` tests `@ignore`'d (§3.5),
  Display test rewritten to bound-var idiom (§3.6).
* `core-tests/context/standard/regression_test.vr` (NEW) — pins §3.5,
  §3.6 with minimal repros + live working-idiom companions.
* `core-tests/context/standard/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Document LogLevel vs ContextLogLevel distinction | `core/context/standard.vr` doc + website | 10 min |
| Add `Row` linear-scan note + `get_index` recommendation | `standard.vr` doc | 10 min |
| Add `ContextLogLevel.from_severity(Int) -> Maybe<...>` + round-trip property | `standard.vr` + tests | 30 min |
| Write property/integration/regression tests for ContextLogLevel | this folder | 1h |
| Cross-tier validate context protocols (requires `provide`/`using`-aware test harness) | language-level vcs specs | tracked elsewhere |
