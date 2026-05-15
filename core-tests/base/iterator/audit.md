# `base/iterator` audit

Module: `core/base/iterator.vr` (4430 LOC) — the largest base/ module,
defines the `Iterator` protocol with ~80 default methods (map / filter /
fold / scan / chain / zip / take / drop / window / chunk / step_by / cycle
/ … ) plus 6 source builders (range / count_from / once / repeat / empty
/ from_list).  Also defines `Transducer<A, B>` rank-2 polymorphic
combinator type (map / filter / filter_map / take / take_while / drop /
drop_while / dedupe / enumerate / inspect / identity / compose / compose2).

Tests: `basic_test.vr` (262 LOC), `unit_test.vr` (2157 LOC, ~120
`@test`s), `property_test.vr` (292 LOC), `integration_test.vr` (175 LOC),
`protocol_agnostic_test.vr` (378 LOC), `regression_test.vr` (this file
~150 LOC — task #5 fundamental closures).

## 1. Cross-stdlib usage

`Iterator` and its adapters are consumed by:

| crate | what it does |
|---|---|
| `core/collections/list.vr` | `List<T>` implements `Iterator` via `.iter()` / `.into_iter()`; List.filter_map / map / fold / collect funnel through Iterator's default methods. |
| `core/collections/map.vr` | `Map<K, V>` implements Iterator over `(K, V)` entries. |
| `core/collections/set.vr` | `Set<T>` implements Iterator over distinct elements. |
| `core/collections/deque.vr` | `Deque<T>` Iterator + DoubleEndedIterator combo. |
| every `for` loop | the language-level `for x in collection { ... }` desugars to `Iterator.next()` calls. |

## 2. Crate-side hardcodes

`crates/verum_vbc/src/codegen/expressions.rs::is_custom_iterator_type`
(line ~10277) recognises slice-shape types (`[T]` / `[T; N]` / `&[T]`)
as Iterator-compatible.  Mirrors the runtime IterNew/IterNext opcode
support pinned by §3.4 in `text/text/audit.md`.

## 3. Language-implementation gaps

### §3.1 Bidirectional variant-constructor resolution — CLOSED 2026-05-15 (task #5)

**Defect class 1 (`Ok(())` single-arg Unit-payload)** —
`crates/verum_types/src/infer/expr.rs::check_expr_inner`'s bidirectional
Call arm for variant constructors gated single-payload handling on
`!matches!(payload_ty, Type::Unit)`.  When the call was `Ok(())` (one
argument, the unit literal `()`) and the expected payload type was
`Type::Unit` (e.g. `Result<(), Int>.Ok` payload), the gate failed and
the code fell through to `synth_and_check` → arity-only first-wins.

Arity-1 matched multiple registered `Ok(_)` parents in
`variant_constructor_parents`: the canonical collision was
`Result.Ok(T)` vs `core.security.zk.halo2.prover.ProveResult.Ok(Proof)`,
plus several other stdlib types with single-arg `Ok(_)` variants.
First-registered-wins picked the wrong parent and `check_expr` then
type-checked the `()` argument against the wrong payload type
(`Proof`), surfacing as "expected 'Proof', found 'Unit'".

**Live repro** — every `assert_eq(iter.advance_by(N), Ok(()))` site in
`core-tests/base/iterator/unit_test.vr`, plus every direct `let r:
Result<(), E> = Ok(());` binding in stdlib bodies (the `advance_by`
no-op success path itself, the early-return idiom in
`Iterator.try_for_each`, every void-returning fallible operation).

**Fundamental fix** — drop the `!Type::Unit` gate so single-argument
calls always take the bidirectional check path, regardless of whether
the payload type is Unit:

```rust
if call_args.len() == 1 {
    self.check_expr(&call_args[0], payload_ty)?;
    Ok(InferResult::new(expected.clone()))
} else if call_args.is_empty() && matches!(payload_ty, Type::Unit) {
    Ok(InferResult::new(expected.clone()))
} else if call_args.len() > 1 {
    // tuple payload — unchanged
    ...
}
```

**Defect class 2 (bare-path variant constructor)** —
`check_expr_inner` had a bidirectional arm for `Call { func: Path, ... }`
but NONE for bare `Path` expressions.  Writing `None`, `Less`,
`Greater`, or any 0-arg variant constructor WITHOUT trailing
parentheses fell through to `synth_and_check`, which uses arity-blind
`try_resolve_variant_constructor` and picks first-registered parent.

Canonical collisions: `Maybe.None` vs
`core.graphics.gpu.Backend.None` (Backend = `CUDA | ROCm | Metal |
Vulkan | SYCL | None`), `Ordering.Less` vs any user-defined sum type
with a `Less` variant, and so on.  Every bare-`None` site in stdlib
bodies that targeted `Maybe<T>` silently dispatched to `Backend.None`
when Backend was loaded into the user's module graph (which is most
non-trivial test files via `mount core.prelude.*`).

**Live repro** — every `else { None }` branch in iterator unit tests
(`test_filter_map_basic`, `test_scan_*`, every `Maybe<_>`-returning
closure body); every bare `None` return in stdlib bodies under the
same architectural defect class.

**Fundamental fix** — new bidirectional Path arm in `check_expr_inner`
that fires when (a) the path is a single-segment name AND (b) the
simple name is registered in `variant_constructor_parents`.  Expands
the expected type to its variant form via `expand_generic_to_variant`
and binds the bare path to the matching Unit-payload variant of THAT
expected type.  Mirrors the bidirectional Call arm.

**Architectural rule** — every site that resolves a variant
constructor by simple name MUST funnel through the user's
expected-type context, not through an arity-blind first-registered-wins
scan.  Pin: grep `try_resolve_variant_constructor` in
`crates/verum_types/src/infer/` — every call site outside the
expected-type-aware bidirectional arms is a candidate for similar
scoping.  Mirrors the discipline pinned by tasks #11 / #22 / #24
/ #25 / #26.

Pinned by `core-tests/base/iterator/regression_test.vr` (9 new tests
spanning both defect classes).

## Action items deferred

### §A `unfold` / `successors` builders + ReduceResult ctors — CLOSED 2026-05-15

Closed by adding `unfold`, `successors`, `Continue`, `Reduced` to
`core/base/mod.vr`'s `public mount .iterator.{...}` re-export clause
so the `core/prelude.vr`'s `super.base.*` glob mount surfaces them as
bare names.  Pre-fix tests using `mount core.prelude.*` couldn't
resolve them and fell through to `E100 unbound variable`.

The companion `ReduceResult<R>` variant-constructor side
(`let r: ReduceResult<Int> = Reduced(99);` failed with `expected 'R',
found 'Int'`) requires the metadata-side type-param substitution map
for variant constructors to fire — this works for the source-driven
path but the precompiled-metadata loader path still has the gap.
Tracked as task #5 §F below.

### §F Higher-order-function closure-shape bound metadata serialisation

**Partial source-side fix landed in this branch** (`crates/verum_types/
src/infer/decls.rs::register_function_signature` + `crates/verum_types
/src/infer/expr.rs::infer_expr_call`):

  * `register_function_signature` now extracts function-type bounds
    (`F: fn(A) -> B`) from each generic param's `bounds` list via
    `extract_type_bounds_from_ast` and attaches them to the
    `TypeScheme` via `.with_type_bounds(...)`.  Mirrors the existing
    `with_protocol_bounds` discipline.

  * `infer_expr_call`'s default lookup path now instantiates BOTH
    protocol and type bounds for the fresh TypeVars and registers the
    function-type bounds on the global env via
    `register_type_var_type_bound`, so
    `check_closure_expr::get_function_type_bound(fresh_F)` recovers
    the closure shape (`fn() -> Maybe<T>` for `from_fn`, etc.) and
    propagates `Maybe<T>` as the closure body's expected return type.

  * Validation: user-defined `fn run_with<T, F: fn() -> Maybe<T>>(f:
    F)` correctly type-checks a closure body whose else branch is
    bare `None` — the bare-Path bidirectional arm in §3.1 resolves
    `None` against `Maybe<T>` instead of arity-blind first-wins.

**Remaining gap**: stdlib functions like `from_fn`, `unfold`,
`successors`, and every other HOF in `core/base/iterator.vr` are
loaded via the precompiled CoreMetadata path (`load_stdlib_from_
metadata` / `register_stdlib_constructors_from_metadata` /
`resolve_metadata_reexport_function`), which doesn't yet serialise
function-type bounds for generic parameters in
`metadata.functions[name].generic_params[i].type_bounds`.  The
metadata format needs a new `type_bounds: List<TypeString>` field on
each generic param descriptor, with corresponding emit at precompile
(`crates/verum_compiler/build.rs`'s metadata-emission walker) and
parse at user-side load (`archive_metadata` /
`load_stdlib_from_metadata`).  Tracked as a follow-up task.

### §B `Transducer.<method>` chain not yet implemented

Tests `test_transducer_*` (~15 tests) fail with "no method named X
found for type Transducer".  The `Transducer` type's static method
suite (`Transducer.map`, `Transducer.filter`, `Transducer.compose`,
…) is declared in `core/base/iterator.vr` but the `implement
Transducer<A, B> { ... }` block is incomplete — most methods are
declared but bodies are missing or under construction.  Separate
task — requires significant stdlib work on the rank-2 polymorphic
transducer combinator suite.

### §C `Range<Int>.reduce_with` not yet implemented

Tests `test_reduce_with_*` fail with "no method named `reduce_with`
found for type Range<Int>".  The `reduce_with` default method is
declared on the `Iterator` protocol but the user-side dispatch path
through `Range<Int>`'s Iterator impl doesn't route it.  Separate
task — likely a default-method monomorphisation gap on Range
specifically.

### §D `Iterator.try_fold` R generic-parameter resolution drift

Tests `test_try_fold_success` / `test_try_fold_empty` fail with
"expected 'R', found 'Int'".  `try_fold<R, F: fn(R, Self.Item) ->
Result<R, E>>` declares `R` as a method-local generic that must
unify with the closure's accumulator type AND the result type.  The
typechecker currently leaks `R` as a rigid named-parameter into the
closure-body inference instead of substituting it with a fresh
TypeVar.  Mirrors the protocol-method-local-generic discipline that
#131 Layer E established for protocol decls; needs parallel
extension for method-local generics inside default-method bodies.

### §E filter_map / scan closure with bare `None` through Iterator path

Tests `test_filter_map_basic` / `test_scan_state_change` fail with
"expected 'CUDA(Unit) | ... | None(Unit)' (Backend), found
'Maybe<Int>'" at the if-expression that returns Some/None.  The
List-side path (List.filter_map) works after §3.1's fix; the
Iterator-side path through `Iterator.filter_map<B, F: fn(Self.Item)
-> Maybe<B>>` doesn't because `Self.Item` association + `B`
substitution doesn't surface `Maybe<TypeVar>` as the closure return
expected by the time the bare `None` in the else branch is checked.
Tracked as a follow-up: the bidirectional expected-type plumbing
needs to flow through the method-chain iterative typing
(`infer_method_chain_iterative` in modules.rs).  Separate from §3.1
but mechanically similar.
