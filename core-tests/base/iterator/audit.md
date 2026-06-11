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

## 4. Crasher root causes — 2026-06-11

The `--interp` crashers (`unit_test`/`property_test`/`protocol_agnostic_test`
SIGSEGV or timeout) decompose into TWO independent root defects.

### §4.1 Archive lazy-apply bare-leaf fanout explosion — CLOSED (commit `946f3d787`)

ANY code calling a method named like a common stdlib method (`next`, `map`,
`get`, …) took **~84 seconds to COMPILE** (not execute). The archive-driven
`ArchiveCtxCache::apply_lazy_with_types` → `SymbolGraph::reachable` BFS seeds
its transitive closure with bare method names harvested from user code. A bare
leaf `next` resolves via `leaf_to_qualified` to EVERY type's same-named impl —
**172** distinct `*.next` bodies in the archive — each of which calls
`self.iter.next()` (another bare `next` `CallM` edge) and re-fans transitively.
The closure pulled in ~most of the 585 archive modules; decoding them was the
84s. Iterator tests (which call `.next()`/`.map()`/etc. densely) timed out.

**Fix**: cap the per-callee bare-leaf fanout in `reachable`
(`crates/verum_compiler/src/archive_ctx_loader.rs`, `MAX_BARE_LEAF_FANOUT=24`,
overridable `VERUM_LEAF_FANOUT_CAP`). A high-fanout bare name is a polymorphic
protocol method resolved at runtime by the receiver's concrete type — whose
defining module loads independently — so blanket leaf-fanning is redundant for
correctness and catastrophic for cost. Measured 84730ms→3979ms (21×).
Regression-safe (maybe/property identical pass/fail cap on vs off).

### §4.2 Cross-module record construction bakes `NEW ()` (untyped) — CLOSED (commit `8d8214d83`)

`xs.iter().enumerate()` then `.next()`/`for` → runtime **stack overflow /
SIGSEGV**. Root: the `Iterator` protocol's default combinators
(`map`/`filter`/`take`/`zip`/`chain`/`enumerate`/…) construct a DIFFERENT
generic adapter record (`EnumerateIter<Self>` etc.). When monomorphised onto a
concrete iterator in another module (e.g. `TextMatches.enumerate` in `core.text`
constructing `EnumerateIter` from `core.base.iterator`), `compile_record`'s
`type_name_to_id.get("EnumerateIter")` MISSES → `type_id=0` → `NEW ()
(fields=2)`. The heap object carries no concrete type, so every later `.next()`
dispatch fails to recover the receiver type and routes to the lowest-id
same-named method (`SignalStream.next`) → infinite recursion.

Confirmed via `VERUM_TRACE_RECNEW`: `in_name_to_id=false in_field_layouts=true`
— the bootstrap shares the type's FIELD LAYOUT cross-module (`import_type_layouts`,
`crates/verum_vbc/src/codegen/mod.rs:3056`) but is **deliberately TypeId-free**
(ids are per-module-local; CLASS-9/D2b). 2814 such sites archive-wide
(EnumerateIter/MapIter/AdapterSpecific/OSError/DerError/Request/…).

**Fix** (`8d8214d83`): a consumer-side recovery in `compile_record` — when a
plain-record literal names a type whose LAYOUT is known (`type_field_layouts`)
but whose id is not (`type_name_to_id` miss), allocate a fresh module-local
`TypeId` and push a `Record` descriptor under the SAME simple name. This works
because the archive body-merge builds its type-id remap **BY NAME**
(`merge_archive_function_bodies`, `codegen/mod.rs:16697-16706`: archive `ty.name`
→ user-codegen `type_name_to_id[name]`), so the local id is remapped to the
canonical descriptor at load — no `external_type_names` machinery needed
(the linker's id→id map was a red herring; the archive-load remap is by-name).

Validated: `xs.iter().map(|x| *x*2)` for-loop now yields correct `2/4/6` (was
timeout/SIGSEGV); `.next()` on a typed adapter no longer mis-recurses to
`SignalStream.next`. Regression-safe (base/{maybe,result,ordering}/property
identical vs pre-fix). Re-bake the embedded archive to ship: it is blake3-cached
over `core/**/*.vr` content (`build.rs:173`), so `rm
target/precompiled-stdlib/runtime.vbca.checksum && touch
crates/verum_compiler/build.rs && cargo build` (~12-16 min).

### §4.3 `self.iter.next()?` in generic adapters yields `None` — OPEN (deeper)

With §4.2 fixed, adapter `.next()` dispatch resolves correctly
(`m.next()` → `EnumerateIter.next`, `recv_type='EnumerateIter'` via
`VERUM_TRACE_DISPATCH`), and `map`-adapter for-loops yield correct values. But
`enumerate`/`map` **manual `.next()` chains still yield empty**, because every
adapter body is `let item = self.iter.next()?; …` and the `?` on a
**generic-type-param-field** receiver (`self.iter: I`) is broken. Reproduces in
pure single-file user code: `implement<I> Wrap<I> { fn step(&mut self) ->
Maybe<Int> { let x = self.inner.next()?; Some(x) } }` returns `None` on the first
call even for `Counter{n:0,max:3}` — whereas the SAME body with an explicit
`match self.inner.next() { Some(x) => …, None => … }` works (`Some(0)`).

Two distinct codegen defects found via `--emit-vbc` (compare `?` vs `match`
lowering of the identical `self.inner.next()` call — bytecode 0000-0002 is
byte-identical):

  1. **success_tag mis-classification** — `compile_try` (`expressions.rs:17938`)
     classifies the inner type via `extract_expr_type_name`, which returns `None`
     for `self.inner.next()` (the receiver type is the generic param `I`, and
     `I.next` isn't a registered fn). Unknown → `success_tag` falls back to **0**,
     which for `Maybe` is the *None* tag, so `?` tests "is None?" not "is Some?".
     A naive fix (force `is_maybe` when the inner method is `next`/`next_back`)
     **regresses** `Result`-returning `.next()?` (result/property 26→25) — the
     classification must distinguish Maybe-vs-Result for the generic case, not
     blanket-assume Maybe.
  2. **success-path payload extraction** — even with the tag corrected, the `?`
     success path emits `AS_VAR r, val, tag=0` (extracts the *None* variant's
     payload) where the `match` lowering correctly emits `GET_VDATA val.0`. So a
     `Some(x)` would yield a garbage payload.

Beyond (1)+(2), the observed first-call result is `None` (failure path taken),
which is **paradoxical** given the `next()` call bytecode is identical to the
working `match` version — pointing to a third, interpreter-level divergence in
how the `?` instruction sequence executes vs `match` (needs instruction-level
tracing / lldb; not VBC-inspectable). Tracked as ADAPTER-TRY-NEXT-1.

Until §4.3 is closed, iterator-adapter `.next()` chains (and `for`-loops routed
to the custom `.next()` path) yield nothing; `.collect()`/`.fold()` and native
`for x in xs.iter()` (builtin `IterNew`) remain the working paths. A related
for-loop misclassification — `is_custom_iterator_type` uses `infer_expr_type_name`
(no MethodCall arm) so `for p in xs.iter().enumerate()` falls to native `IterNew`
on the adapter record → SIGSEGV — is the remaining for-loop crasher.

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
