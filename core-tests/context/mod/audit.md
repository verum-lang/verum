# `context/mod` audit

Module: `core/context/mod.vr` (108 LOC) — umbrella module that
re-exports the public surface of `scope`, `error`, `provider`,
`layer`, and `standard` submodules under `core.context.*`.

Tests: `integration_test.vr` — verifies each re-exported type is
reachable via the bare `mount core.context.*` mount.

## 1. Cross-stdlib usage

Every consumer of the context system reaches it through
`mount core.context.*` or `mount core.context.{Provider, ...}`.
Direct submodule mounts (`mount core.context.scope.Scope`) are
also valid and are used in this test suite to dodge name
collisions (see scope/audit.md §3.1).

## 2. Crate-side hardcodes

`crates/verum_compiler/src/phases/context_check.rs` and adjacent
compiler-side code reference `core.context.*` paths when emitting
DI infrastructure. The umbrella's structure (Provider re-exported,
Scope re-exported, etc.) is part of the language ABI contract.

## 3. Language-implementation gaps

### §3.1 `public mount self.standard.*` is doc-stated but not parser-verified

`mod.vr:103` `public mount self.standard.*;` re-exports every
public item from `standard.vr`. There is no compile-time
verification that the umbrella's promises (Provider, Scope,
ContextError reachable via `core.context.*`) hold — they're
load-bearing for user code but the only test today is
`integration_test.vr`.

Adding compile-time pin in `crates/verum_modules/src/...` would
catch a removed re-export at module-load time instead of at
user-code compile time.

**Effort:** small (~2h).

### §3.2 Layer composition (`layer.vr`) is doc-only today

`core/context/layer.vr` (82 LOC) is ENTIRELY documentation
comments — it documents the `layer { provide ... }` syntax that
the compiler is supposed to lower, but ships no Verum types or
runtime hooks. The compiler-side layer composition is in
`crates/verum_compiler/src/phases/layer_compose.rs` (or similar).
This split between "doc lives in core, code lives in compiler"
is a documented architectural choice; pin it in the audit so
future agents don't try to add Verum types to `layer.vr` that
would conflict with the compiler-side.

**Effort:** ~30 min doc update across `layer.vr` + website docs.

### §3.3 Row direct field read via umbrella hits the CLASS-9 field-shift (NEW)

`test_umbrella_row_and_query_result` originally read `r.columns.len()` /
`r.values.len()` on an archive-loaded `Row` reached via `core.context.*`;
that panics `field access out of bounds: field index 4 ... type='List'`
(the cross-module field-index shift — see `standard/audit.md §3.7`). The
test was reworked to exercise umbrella reachability via `QueryResult`
construction + `qr.rows.len()` (the working path). `Row.get_index` is
likewise blocked (`standard/audit.md §3.5`). No mod-specific defect — these
are CLASS-9 manifestations surfaced through the umbrella.

## Conformance status (2026-06-01, interpreter / `--test-threads 1`)

`mod` is **partial**: `unit_test.vr` (umbrella reachability of Scope /
ContextScope / ContextError / Provider / ScopedProvider / LazyProvider /
get_context / has_context / ContextLogLevel / AuthUser / QueryResult),
`property_test.vr` (re-exported types retain Scope-hierarchy /
ContextError-Eq / ContextLogLevel-severity / Provider-idempotence laws via
the umbrella), `integration_test.vr`, and `regression_test.vr` (the
`standard.*` re-export gap + qualified-variant routing) are GREEN. The only
constrained surface is archive-loaded `Row` field/`get_index` access
(CLASS-9, pinned in `standard/`).

## Action items landed in this branch

* `core-tests/context/mod/integration_test.vr` — verifies the
  umbrella re-exports.
* `core-tests/context/mod/{unit,property,regression}_test.vr` (NEW) —
  umbrella reachability + re-exported-type laws + re-export regressions.
* `core-tests/context/mod/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Compile-time pin for `mod.vr` re-export contract | `crates/verum_modules` | 2h |
| Layer composition: clarify doc-only vs compiler-side surface | `core/context/layer.vr` doc | 30 min |
| Add property/regression tests once submodule audits close | this folder | as defects close |
