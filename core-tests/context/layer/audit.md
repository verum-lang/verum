# `context/layer` audit

Module: `core/context/layer.vr` (82 LOC) — **documentation-only**. The
file contains the design narrative for the `layer { provide ... }`
composition construct and the `layer A = B + C;` merge syntax, but ships
**no Verum types, no functions, and no runtime hooks**. The actual layer
composition is a compiler construct (`layer_def` / `layer_expr` in
`grammar/verum.ebnf`), lowered compiler-side.

## 1. Cross-stdlib usage

There is no Verum-level surface to consume. Layers are compile-time
constructs: `layer DatabaseLayer { provide ConnectionPool = ...; }`
expands its `provide` statements in declaration order; `layer App = A + B;`
concatenates them left-to-right. The compiler resolves inter-layer
dependencies and emits the initialisation order.

## 2. Crate-side hardcodes

The `layer` keyword and `+` composition are parsed by
`crates/verum_fast_parser` (`layer_def` / `layer_expr`) and lowered by
the context-system phase in `crates/verum_compiler` / `crates/verum_types`
(same pass family as `context_check.rs`). The doc in `layer.vr` is the
spec; the code lives in the compiler.

## 3. Why there is no pure-Verum conformance suite here

A `core-tests/context/layer/` unit/property/integration suite would need
to *evaluate* `layer { provide ... }` blocks, which requires the full
`provide` / `using` runtime (task-local context slots) plus the compiler's
layer-lowering pass. Those are exercised at the **language level** in
`vcs/specs/L2-standard/contexts/`, not in `core-tests/` (which tests
pure-Verum stdlib types). This split — *doc in `core/`, code in
`crates/`, behaviour test in `vcs/specs/`* — is the documented
architectural choice (mirrored in `context/mod/audit.md §3.2`).

This folder therefore carries **only this `audit.md`** (no `*_test.vr`)
— the honest state. Adding placeholder tests that don't exercise real
layer behaviour would be decoration, which the suite charter forbids.

## 4. Language-implementation gap / enhancement

### §4.1 No runtime `Layer` builder type (the website doc advertises one that does not exist)

`website:docs/stdlib/context.md` shows a fluent runtime API —
`Layer.new().with_singleton<T>(..).with_request<T>(..).merge(..).run(..)`
— that has **no implementation** anywhere (`layer.vr` is doc-only; no
`type Layer is { ... }` exists). This is doc drift: the page promises an
API the stdlib does not provide.

Two ways to close it, both fundamental:
* **(A) Implement a real `Layer<>` builder** in `layer.vr` — a value-level
  type collecting `(slot_id, scope, provider-thunk)` entries with
  `with_singleton`/`with_request`/`with_transient`/`merge`/`run`,
  backed by `ScopedProvider` + the `ctx_bridge`. Gives the website API a
  real referent and a conformance suite. Requires a compiler rebuild
  (embedded VBC) + careful interaction with the compiler-side `layer`
  keyword lowering (the two must not collide on the name `Layer`).
* **(B) Correct the website doc** to describe only the compile-time
  `layer { provide ... }` construct and delete the fictional fluent API.

The website-doc reconciliation in this branch takes path **(B)** for
honesty now; path **(A)** is tracked as the deferred enhancement.

**Effort:** (A) ~1 day + rebuild + tests; (B) ~20 min doc edit (done in
this branch).

## Action items landed in this branch

* `core-tests/context/layer/audit.md` — this file (first registration of
  the doc-only module in the conformance inventory).
* Website doc reconciled to remove the fictional `Layer.new()...` API
  (path B).

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Implement a real value-level `Layer` builder type | `core/context/layer.vr` + rebuild + new conformance suite | ~1 day |
| Compile-time pin that `layer`-keyword lowering stays aligned with the doc | `crates/verum_compiler` + `vcs/specs/L2-standard/contexts/` | ~1 day |
