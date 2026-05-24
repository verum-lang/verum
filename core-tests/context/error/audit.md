# `context/error` audit

Module: `core/context/error.vr` (104 LOC) — defines `ContextError`, a
5-variant sum type covering every failure mode of the context system,
plus `Display`, `Debug`, and `Eq` implementations.

Tests: `unit_test.vr` (~30 `@test`s — variant construction +
message() format + Eq matrix + Display/Debug formatting),
`property_test.vr` (~15 `@test`s — Eq reflexivity/symmetry/disjointness +
message determinism + Display≡message + discriminating data),
`integration_test.vr` (~10 `@test`s — Result/Maybe/List wrapping +
ScopeViolation ⇔ Scope.name() round-trip), `regression_test.vr`
(~5 `@test`s — qualified-construction discipline + message format).

## 1. Cross-stdlib usage

`ContextError` is consumed by:

| crate / module | what it does |
|---|---|
| `core/context/provider.vr` | `get_context<T>` returns `Maybe<T>` (today) but the design intent is `Result<T, ContextError>` once the slot-based fast path is mature. Today the error is implicit (`None` = `NotFound`). |
| compiler-emitted code | The compiler emits `ContextError.ScopeViolation` for E806 at runtime if static analysis was disabled or the runtime DI graph diverged from the static graph. |
| user code via `?` | Application code receives `ContextError` from `provide` failures and propagates via `?` through `Result<T, ContextError>`. |

## 2. Crate-side hardcodes

`crates/verum_diagnostics/src/codes.rs` defines error codes
**E3050**, **E3051**, **E3052** (positive/negative/conflicting
constraint violations), **E805** (circular dependency), **E806**
(scope violation), **E807** (missing `@inject` constructor),
**E808** (constructor parameter mismatch). The `ContextError`
variants map to these codes 1:1; drift is caught when the
diagnostics code table is regenerated.

`crates/verum_types/src/passes/context_check.rs` is the static
scope-violation checker; it emits **E806** with the same template
text as `ContextError.ScopeViolation.message()`. The two must stay
aligned — `integration_scope_violation_uses_scope_name_text`
catches drift at the text-rendering level.

## 3. Language-implementation gaps

### §3.1 No standalone error-code accessor

`ContextError` carries no `code(&self) -> Text` method that returns
the canonical `"E3050"` / `"E805"` / `"E806"` string. Consumers
that need the code today must hard-code the variant→code mapping at
their call site. Add a `code(&self) -> Text` method to the
`implement ContextError` block; pin its output in a new unit test.

**Effort:** small (~30 min). Tracked here for the next agent.

### §3.2 No `NotFound` for E807/E808

The current 5 variants cover NotFound / NotProvided / TypeMismatch /
CircularDependency / ScopeViolation. The diagnostics codes table
defines **E807** (missing `@inject` constructor) and **E808**
(constructor parameter mismatch) — these have no `ContextError`
variant today. Either:
* Add `MissingInjectCtor { type_name: Text }` and `InjectCtorArity { type_name: Text, expected: Int, found: Int }` variants, OR
* Document that E807/E808 are compile-time-only diagnostics with no runtime equivalent.

**Effort:** small (~1h) for the variant addition + audit pin.

### §3.3 Debug format does not surface CircularDependency chain content

`Debug for CircularDependency` renders `f"CircularDependency {{ chain: <N entries> }}"`
— the chain content itself is dropped. For debugging this is
strictly worse than Display, which `.join(" -> ")`s the chain. Either
match Display's content (and pay the rendering cost in `f"{x:?}"`)
or accept the trade-off; the current state is half-applied.

**Effort:** trivial (~10 min) once decided.

## Action items landed in this branch

* `core-tests/context/error/{unit,property,integration,regression}_test.vr`
  — first conformance suite for the module.
* `core-tests/context/error/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `code(&self) -> Text` accessor | `core/context/error.vr` + unit test | 30 min |
| Add E807/E808 variants OR document compile-time-only | `core/context/error.vr` + variant tests | 1h |
| Fix `Debug for CircularDependency` to render chain content | `core/context/error.vr` | 10 min |
| Cross-test E806 message alignment between static checker and `ScopeViolation.message()` | `crates/verum_types/src/passes/context_check.rs` integration test | 2h |
