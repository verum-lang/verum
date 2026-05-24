# `context/provider` audit

Module: `core/context/provider.vr` (~320 LOC) — defines `Provider<T>`
(lazy factory + cache), `LazyProvider<T>` (compatibility alias),
`ScopedProvider<T>` (RAII block-scoped context provision), and the
free fns `get_context<T>` and `has_context`.

Tests: `unit_test.vr` covers each public API surface (factory caching,
reset, is_resolved, get_ref, ScopedProvider construction, has_context
in unprovided slots).

## 1. Cross-stdlib usage

`Provider<T>`:
* Compiler-emitted code uses `Provider.new(factory)` to lazily
  initialise context bindings under `provide X = ...;` (function-
  scoped form, distinct from `in { block }`).

`ScopedProvider<T>`:
* Compiler emits `ScopedProvider.new(slot_id, value).run(body)` for
  the `provide X = value in { body }` block form.

`get_context<T>` / `has_context`:
* Compiler emits these for `using [X]` accesses inside a body.
* The bridge into runtime TLS is `core.runtime.ctx_bridge` (see
  `env_ctx_get`, `env_ctx_set`, `env_ctx_end`).

## 2. Crate-side hardcodes

`crates/verum_compiler/src/phases/context_check.rs` and adjacent
files in the context-system validation pass treat `Provider<T>` as
the canonical lazy-DI wrapper; any rename of `Provider.{new,of,get,
get_ref,is_resolved,reset}` must be paired with a compiler-side
update.

`crates/verum_vbc/src/interpreter/dispatch_table/handlers/calls.rs`
intercepts `verum_get_runtime_async_worker_threads` and
`verum_get_runtime_task_stack_size` (see the codegen intercept block
~line 4965-5000) — these are runtime-bridge accessors needed by
provider-managed async pools. The intercept emits LoadI{0} so that
Tier-0 doesn't fall through to FFI dispatch that would otherwise hit
`SymbolNotFound(FfiSymbolId(1))`. This intercept is integral to
making providers functional under interpreter.

## 3. Language-implementation gaps

### §3.1 No `Provider.map` / `Provider.flat_map`

The current `Provider<T>` exposes only construction and access.
Common patterns like `provider.map(|v| transform(v))` or composing
providers (`p.flat_map(|v| Provider.new(|| derived(v)))`) require
manual wrapping today. Add functor + monad operations:

```verum
implement<T: Clone, U: Clone> Provider<T> {
    public fn map(self, f: fn(T) -> U) -> Provider<U> { ... }
    public fn flat_map(self, f: fn(T) -> Provider<U>) -> Provider<U> { ... }
}
```

**Effort:** ~1h + tests.

### §3.2 No `ScopedProvider.try_run` for fallible bodies

`ScopedProvider.run<R>` panics if the body panics. Library authors
often want `try_run<R, E>(self, body: fn() -> Result<R, E>) -> Result<R, E>`
that propagates the error AFTER popping the TLS slot. Add it.

**Effort:** ~30 min + a test.

### §3.3 `get_context` returns `Maybe<T>` instead of `Result<T, ContextError>`

The compile-time `using [X]` clause guarantees `X` is provided —
the runtime accessor returning `Maybe` swallows the "not provided"
case as `None`, losing the discrimination. Caller side has to
double-check via `has_context`. Switch to `Result<T, ContextError>`
or document why `Maybe` is fine.

**Effort:** medium (~1 day) — touches code-generated call sites.
Coordinate with `context_check.rs`.

### §3.4 `@bitcast` in `ScopedProvider.run` is unsafe and untyped

```verum
let raw_value: Int = @bitcast(self.value);
env_ctx_set(self.slot_id, raw_value);
```

`@bitcast` from arbitrary `T` to `Int` is sound only when `T`
fits in a NaN-boxed `i64`. For larger payloads (multi-field records),
this corrupts the TLS slot silently. The contract needs to be:
* If `T` is NaN-boxed-friendly (Int, Bool, small Text, pointer-like),
  inline-store.
* Else, heap-allocate and store the pointer.

**Effort:** medium-large (~2 days). Touches the `ctx_bridge` API
and every compiler-emitted `provide` call site.

## Action items landed in this branch

* `core-tests/context/provider/unit_test.vr` — first conformance
  surface for the module's public types.
* `core-tests/context/provider/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `Provider.map`/`flat_map` | `core/context/provider.vr` + tests | 1h |
| Add `ScopedProvider.try_run` | same | 30 min |
| Add `get_context` `Result` overload + caller migration | cross-cutting | 1 day |
| Fix `@bitcast` payload-size hazard | provider + ctx_bridge + compiler | 2 days |
| Write property/integration/regression tests once defects above close | this folder | 1 day |
