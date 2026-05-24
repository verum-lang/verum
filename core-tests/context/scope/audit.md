# `context/scope` audit

Module: `core/context/scope.vr` (200 LOC) — defines the 3-valued DI
`Scope` ADT (`Singleton | Request | Transient`), the `Scope.can_depend_on`
hierarchy predicate, `name`/`rank` accessors, and the runtime
`ContextScope` depth-tracking record.

Tests: `unit_test.vr` (~30 `@test`s — variant construction + 3×3
`can_depend_on` matrix + `ContextScope.{root,enter,current_depth,parent}`
+ nested-stress), `property_test.vr` (~20 `@test`s — algebraic laws
exhausted over the 3-element domain + monotonic depth chain),
`integration_test.vr` (~10 `@test`s — DI-graph well-scopedness
checks, ContextScope models nested `provide ... in {}` hierarchy,
Scope.name composes with f-strings, ScopeViolation message
construction), `regression_test.vr` (~5 `@test`s — every defect
surfaced while building this suite, pinned forever).

## 1. Cross-stdlib usage

`Scope` is consumed by:

| crate / module | what it does |
|---|---|
| `core/context/error.vr` | `ContextError.ScopeViolation` stores `dependent_scope: Text` and `dependency_scope: Text`. The Text values are conventionally produced by `Scope.name()` at the violation site (verified by `integration_scope_violation_uses_scope_name_text`). |
| `core/context/standard.vr` | `@injectable(Scope.Singleton)` / `@injectable(Scope.Request)` decorators — compile-time scope tagging. |
| `core/context/provider.vr` | re-exports `Scope` for downstream consumers. |

`ContextScope` is consumed by:

| crate / module | what it does |
|---|---|
| (none today — this is a forward-looking surface) | `ContextScope` is the runtime depth-tracking record for `provide ... in {}` lexical scoping. Today the actual stack is held in `core.runtime.ctx_bridge` slot operations; `ContextScope` is the typed Verum surface intended for compiler-emitted scope-management code. Compiler integration is task #28. |

## 2. Crate-side hardcodes

`crates/verum_compiler/src/...` references the `Scope` variant names
in `@injectable` attribute parsing (compile-time scope check); the
canonical mapping is **Singleton=0, Request=1, Transient=2** (matches
`Scope.rank()`). Any reordering of the variants in `scope.vr` is a
compile-time break — pinned by `test_scope_rank_*` and the property
`property_rank_monotone_by_lifetime`.

E806 (scope violation) is emitted by the type checker's scope-rule
enforcement pass; its error message must be aligned with
`ContextError.ScopeViolation.message()` output. Drift between the
two paths is caught at the integration level by
`integration_scope_violation_uses_scope_name_text`.

## 3. Language-implementation gaps

### §3.1 Cross-module name collision — bare `Transient` (CRITICAL)

`Transient` is a unit variant of `Scope` BUT ALSO of 4 other types:
* `core/async/spawn_config.vr:89` — task lifecycle
* `core/runtime/env.vr:711`        — restart strategy
* `core/runtime/supervisor.vr:177` — RestartStrategy
* `core/database/common/pool_supervisor.vr` — SupervisorRestartStrategy

A test that says `mount core.context.scope.{Transient}` and references
bare `Transient` fails to compile with `UndefinedVariable("Transient")`
because the first-wins bare-name resolution rejects the requested
mount in favour of a sibling-module candidate. The same hazard exists
for `Request` (which also collides with `core.net.http.Request`).

**Workaround in this suite:** every reference uses the qualified form
`Scope.Transient` / `Scope.Request`. Pinned in
`regression_bare_transient_collision_workaround_via_qualification`.

**Upstream fix:** task #17/#39 — mount-scope-aware function/variant
lookup. Multi-day refactor of every `lookup_function*` call site to
prefer the mount-imported scope's variant before falling back to the
bare global lookup.

### §3.2 Bare `Singleton` value-of-Scope mis-dispatches `.name()` and `.can_depend_on()` (CRITICAL)

Even though `Singleton` is uniquely defined in `Scope` (verified by
grep), `Singleton.name()` returned a string OTHER than `"Singleton"`
and `Singleton.can_depend_on(Scope.Request)` returned `true` (must be
`false`). Root cause: the bare-variant constructor `Singleton`
produces a `Scope`-typed value, but at the method-dispatch site the
receiver's enclosing-type association is dropped — the dispatch
picks a `.name()` / `.can_depend_on()` from a different type that
happens to be in scope first.

This is **silent dispatch corruption**: the compiler accepts the
program but the wrong code runs. Worse than §3.1's compile-time
rejection because it doesn't trigger user-visible failures unless
the result is later asserted against.

**Workaround:** always qualify the receiver as `Scope.Singleton.name()`.
Pinned in `regression_qualified_singleton_name_returns_singleton` and
`regression_qualified_singleton_can_depend_on_only_singleton`.

**Upstream fix:** the same task #17/#39 mount-scope-aware lookup,
plus the broader `T.method()` method-resolution fix
(memory/callg_emission_fix_blueprint_2026-05-19.md). When the bare
variant value reaches method dispatch, the type-context required for
correct method resolution must be preserved through the value
construction. Today it is dropped.

### §3.3 `global_ctors: FunctionNotFound(FunctionId(0xFEFFXXXX))` cascade (FUNDAMENTAL)

Surfaced repeatedly during this session even on the baseline
`test_ordering_less_construction`. The 0xFEFF range matches the
stage-3 cross-module-Call stub IDs emitted by task #47 (commit
962f44ed0 `feat(vbc): task #47 — cross-module Call name-encoding
via stage-3 stubs + descriptor-emit`).

The interpreter's `global_ctors` initialisation phase walks the
archive looking up function IDs to invoke as ctors; under certain
sequences of test-file edits the lookup fails for a stage-3 stub ID
that was emitted but never resolved to a concrete function.

Symptom: tests that were passing minutes earlier suddenly fail with
`global_ctors: FunctionNotFound(FunctionId(N))` for some `N` in the
0xFEFFXXXX range, regardless of the test's own code.

Workaround during this session: stash the parallel-agent
WIP changes touching `crates/verum_vbc/` and rebuild the binary —
not stable across rebuilds.

**Upstream fix:** task #47 close-out. Stage-3 stubs must be
either (a) ALWAYS resolved before `global_ctors` runs, or (b)
filtered from the ctor walk. The current implementation appears
to emit stubs whose resolution races with the ctor invocation.
Track as a P0 stdlib soundness incident.

### §3.4 AOT path completely blocked

Every `--aot` test (incl. baselines like `test_ordering_less_construction`)
fails with a stdlib compile error:

* `undefined function: sync_connect_binlog (in function connect_binlog_async)`
  — `sync_connect_binlog` is referenced from the async mysql connection
  module but the symbol is not registered in the runtime function table.
* `wrong number of arguments for translate: expected 1, found 3 (in function handle_translate)`
  — `translate` is being called with 3 args where the function expects 1.
* `core/math/tactics.vr:839` parse error at module level
  (`let` keyword used unexpectedly inside a module body).

These are pre-existing AOT-pipeline blockers unrelated to `context/`
but they prevent cross-tier validation of the context tests
(`--interp` only). Tracked as task #7 in this session's task list.

## Action items landed in this branch

* `core-tests/context/scope/{unit,property,integration,regression}_test.vr`
  — first conformance suite for the module.
* `core-tests/context/scope/audit.md` — this file.
* Pinned regressions §1, §2, §3 above with concrete repros.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Close task #17/#39 mount-scope-aware lookup | All bare-name dispatch sites in `crates/verum_vbc/src/codegen/` | 2-3 days |
| Close §3.2 method-receiver type-tracking | Cross-cutting in `verum_types` + `verum_vbc/src/codegen/expressions.rs` | 2-3 days |
| Close §3.3 task #47 stage-3 stub global_ctors race | `crates/verum_vbc/src/precompile/` + `interpreter/global_ctors.rs` | 1-2 days |
| Close §3.4 AOT-stdlib build cascade | mysql/sync_connect_binlog symbol + translate arity + math/tactics parse | 1 day per defect |
| Wire `ContextScope` into compiler emission of `provide ... in {}` | `crates/verum_compiler/src/...` | 1 week (task #28) |
