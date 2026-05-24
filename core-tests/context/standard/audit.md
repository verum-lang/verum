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

## Action items landed in this branch

* `core-tests/context/standard/unit_test.vr` — first surface
  coverage for ContextLogLevel + data types.
* `core-tests/context/standard/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Document LogLevel vs ContextLogLevel distinction | `core/context/standard.vr` doc + website | 10 min |
| Add `Row` linear-scan note + `get_index` recommendation | `standard.vr` doc | 10 min |
| Add `ContextLogLevel.from_severity(Int) -> Maybe<...>` + round-trip property | `standard.vr` + tests | 30 min |
| Write property/integration/regression tests for ContextLogLevel | this folder | 1h |
| Cross-tier validate context protocols (requires `provide`/`using`-aware test harness) | language-level vcs specs | tracked elsewhere |
