# Audit — `core/base/ops.vr`

> Carrier-protocol audit: `Try`, `FromResidual`, `ControlFlow<B, C>`,
> `Never`, `Residual`, `Drop`. These power Verum's `?`-operator and
> RAII. Findings range from a critical architectural drift in how `?`
> is lowered (the protocol path is bypassed entirely) down to small
> arity-mismatch typos in the codegen builtin registry.

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/ops.vr` (305 lines) |
| Tests | `core-tests/base/ops/` — `unit_test.vr` (1207, migrated), `property_test.vr` (NEW, ~250 LOC, algebraic laws + @property + @test_case demos), `integration_test.vr` (NEW, ~210 LOC, ?-chains, cross-type residual, error promotion, ControlFlow pipelines) |
| Hardcodes in `crates/` | 7 critical sites; one CRITICAL-class architectural defect |

## §1  CRITICAL — `?`-operator does NOT use Try/FromResidual

The protocol architecture in `ops.vr` defines `Try.branch()`,
`Try.from_output()`, `FromResidual.from_residual()` as the lawful path
for the `?` operator. **Codegen ignores them entirely.**

`crates/verum_vbc/src/codegen/expressions.rs:13134-13219` —
`compile_try()` is a hardcoded fast-path:

```text
expressions.rs:13151-13172   detects type by name (is_maybe);
                             hardcodes variant names "Some"/"None"
                             or "Ok"/"Err".
expressions.rs:13176-13206   emits IsVar + conditional jump directly.
                             No call to Try.branch() or
                             FromResidual.from_residual().
```

**Failure mode (real, not hypothetical):** any user type implementing
`Try` for a custom error monad is silently bypassed. The `?` operator
falls through to the type-name detector, fails to recognise the user
type, and emits dead bytecode or a wrong-shape branch. There is no
diagnostic.

**Architectural fix (deferred — cross-cutting):**
1. Lower `expr?` to `Try.branch(expr)` followed by a `match` on the
   resulting `ControlFlow`. The `Continue(v)` arm yields `v`; the
   `Break(r)` arm calls `FromResidual.from_residual(r)` and early-
   returns it.
2. Maintain a fast-path optimisation pass for the
   {Maybe, Result}-receiver case: lower to direct `IsVar` when the
   receiver is statically a Maybe/Result. But this fast-path must be
   *under* the protocol path, not in lieu of it.
3. Add a CI check that walks `core/` for `implement Try for ...` and
   exercises the ?-operator on each — any test that the bytecode emits
   no `CallM "branch"` instruction is a regression.

**Why deferred from this branch:** changing `?` lowering touches
codegen, the runtime, and every test that uses ?. Single-task scope is
out of band; needs its own multi-PR effort.

## §2  Hardcoded variant tags & arity (drift surfaces)

| Site | Problem | Severity |
|---|---|---|
| `crates/verum_vbc/src/codegen/mod.rs:1810-1811` | Hardcoded `Continue=0, Break=1` in a separate map | MEDIUM |
| `crates/verum_vbc/src/codegen/mod.rs:1985-1993` | Variant tag map: `None=0,Some=1,Ok=0,Err=1,Continue=0,Break=1` | MEDIUM |
| `crates/verum_vbc/src/codegen/mod.rs:4942` | **Arity inconsistency**: `("ControlFlow", "Continue", 1, …)` registers arity = 1 but Continue is registered as arity 1 carrying payload C | LOW (type-correct) |
| `crates/verum_vbc/src/codegen/expressions.rs:13151-13294` | Type names + variant names embedded in compile_try fast-path | HIGH (this is §1's surface) |

The `ControlFlow` arity registration is *correct* by chance: both
`Continue(C)` and `Break(B)` carry exactly one payload. But the
tag-order hardcode (`Continue=0, Break=1`) is a drift surface analogous
to Ordering's. Apply the same pattern as `ORDERING_VARIANT_LAYOUT` in
`verum_common/src/well_known_types.rs`.

**Recommendation landed:** none yet; tracked here. The
ControlFlow layout pinning is small (~50 LOC) but waits for the
architectural fix in §1 — landing it now would tie the new constant
to the broken fast-path, making §1 harder to fix later.

## §3  Residual is documentation-only

`Residual` (ops.vr:271-274) declares an associated `TryType`. The
typechecker (`verum_types/src/protocol.rs:7900-7906`) resolves it for
type inference. **Codegen never consumes it** — see §1; the fast-path
skips the protocol layer entirely.

**Action item:** once §1 is closed, write a real consumer for
`Residual` in the FromResidual lookup pipeline; until then,
`Residual.TryType` is a phantom type.

## §4  Stdlib consumer inventory

Today only Maybe and Result implement `Try`:

```text
core/base/maybe.vr:574       implement<T> Try for Maybe<T>
core/base/result.vr:507      implement<T, E> Try for Result<T, E>
```

Plus FromResidual cross-direction:

```text
maybe.vr:591   FromResidual<Maybe<Never>>            for Maybe<T>
maybe.vr:607   FromResidual<Result<Never, E>>        for Maybe<T>
result.vr:525  FromResidual<Result<Never, E>>        for Result<T, E>
result.vr:540  FromResidual<Result<Never, F>>        for Result<T, E>  (where E: From<F>)
result.vr:551  FromResidual<Maybe<Never>>            for Result<T, E>
```

No iterator `try_fold`, no custom error monad, no async-Try. This is
a *consequence* of §1: there's no visible value-add to writing custom
Try impls when the codegen ignores them.

**Action item:** once §1 is closed, port `Iterator.try_fold` from the
specification doc to actual stdlib code.

## §5  Drop semantics

`Drop` (ops.vr:301-305) is a single-method protocol consumed directly
by the CBGR runtime via a function pointer in TypeDescriptor — see
`crates/verum_vbc/src/types.rs:1735-1736, 2380-2381`. **No drift
surface here**: drop dispatch goes through a stable C-style fn ptr,
not a string-name lookup.

This is the right architecture: `Drop` is RAII-load-bearing and must
not depend on metadata-driven dispatch.

## §6  Action items landed in this branch

- [x]  Migrate `vcs/specs/core/core/ops_test.vr` →
       `core-tests/base/ops/unit_test.vr` (vtest frontmatter stripped)
- [x]  Add `property_test.vr` covering ControlFlow predicate disjointness,
       map_continue/map_break functor laws, Continue/Break round-trip,
       Try-coherence on Maybe and Result, FromResidual round-trip,
       Never uninhabitedness, plus @property and @test_case demos
- [x]  Add `integration_test.vr` covering happy-path ?, failure-path
       short-circuit, cross-type ? (Maybe in Result, Result in Maybe),
       error promotion via From, ControlFlow-driven pipeline patterns
- [x]  Add this audit document

## §7  Action items deferred (not landed)

1. **§1 — Replace hardcoded ?-fast-path with real protocol dispatch.**
   This is the architectural unblock; once landed, the rest of the
   ops protocol surface becomes load-bearing rather than ornamental.
   *Scope:* multi-file (codegen + runtime + tests) — its own PR series.
2. **ControlFlow drift-pin.** After §1, mirror the `ORDERING_VARIANT_LAYOUT`
   pattern: add `CONTROLFLOW_VARIANT_LAYOUT` in `verum_common::well_known_types`,
   consult from the (now-correct) protocol-dispatch path. Add the matrix-pinning
   unit test in the same module.
3. **Iterator.try_fold.** Port from specification once §1 is closed —
   this turns ?-in-iterator-bodies into a normal stdlib idiom.
4. **Custom error-monad cookbook.** A `core/base/error_monads.md` that
   shows how to define a custom `Try`-implementing type (e.g. for
   accumulating warnings) — meaningful only after §1 lands.
