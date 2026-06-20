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

### §4.3 `self.iter.next()?` in generic adapters yields `None` — CLOSED (commit `3858edf52`)

With §4.2 fixed, adapter `.next()` dispatch resolves correctly
(`m.next()` → `EnumerateIter.next`, `recv_type='EnumerateIter'` via
`VERUM_TRACE_DISPATCH`). But every adapter body is `let item =
self.iter.next()?; …`, and the `?` on a **generic-type-param-field** receiver
(`self.iter: I`) propagated `None` even for a `Some`. Reproduced in pure
single-file user code: `implement<I> Wrap<I> { fn step(&mut self) -> Maybe<Int>
{ let x = self.inner.next()?; Some(x) } }` returned `None` on the first call for
`Counter{n:0,max:3}`; the same body with explicit `match` works.

**Root cause** (found via `VERUM_TRACE_MATCHTAG` + a per-instruction
`VERUM_TRACE_PC` trace — the "byte-identical bytecode, different result" was a
false lead caused by the **stale script-cache**, see the NOTE below): the `?`
on a generic-`next()` reaches `compile_try`, whose `extract_expr_type_name`
returns no Maybe-classifiable base, so `success_tag` defaults to **0** — which
for `Maybe` is the *None* tag. `IS_VAR(Some, tag=0)` → false → the `?` takes the
failure path and propagates `None`. (A probe capturing the value confirmed
`is_some=true` while `?` still returned `None`; the matchtag trace showed
`expected_tag=0` despite the disassembler printing `tag=1`.)

**Fix**: in `compile_try`, force `Maybe` classification when `?` is applied
directly to a `next`/`next_back` MethodCall. Every `fn next`/`fn next_back` in
`core/` that can appear under `?` returns a top-level `Maybe` (the only
non-Maybe `next`s are RNG `-> UInt64`, never `?`-applied), so the override is
sound and overrides a mis-resolved Result-shaped base. (`AsVar` extracts the
success payload positionally — field 0 — so it is correct for both Maybe-Some
and Result-Ok; no payload-extraction change needed.)

Validated (cache cleared): stdlib `xs.iter().enumerate()` manual `.next()` loop
yields `0:10/1:20/2:30`; user generic `Wrap<I>` adapters yield correctly.
Regression-safe + improvement: base/maybe/property 21/9→22/8, result 26/5,
ordering 23/3.

### §4.4 for-loop over non-intercepted adapters → native `IterNew` SIGSEGV — CLOSED (commit `ae4b3d22a`)

`for p in xs.iter().enumerate()` (and `.take`/`.skip`/`.zip`/`.chain`/…)
crashed: `is_custom_iterator_type` uses `infer_expr_type_name` (no MethodCall
arm) → None → the loop falls to native `IterNew`, which maps every non-builtin
`type_id` to `ITER_TYPE_LIST` and reads the adapter record's fields as a `List`
`[count,cap,entries_ptr]` header → SIGSEGV. (`map`/`filter`/`fold` are
runtime-intercepted onto the native blob — eager-collect — so they worked.)

**Fix**: `is_custom_iterator_type` recognizes the non-intercepted adapter
methods (`enumerate`/`take`/`skip`/`take_while`/`skip_while`/`zip`/`chain`/
`flat_map`/`flatten`/`scan`/`step_by`/`peekable`/`rev`/`fuse`/`cycle`/`dedup`/
`windows`/`chunks`/`intersperse`/`map_while`/`inspect`/`copied`/`cloned`) in the
for-loop iterator position and routes them to `compile_for_custom_iterator`
(`loop { match it.next() {...} }`), which calls the record's `.next()` —
correct after §4.3. `map`/`filter`/`fold` stay on the fast native blob path.

Iterator suite (`--interp`): property **SIGSEGV→13/9**, protocol_agnostic
**SIGSEGV→20/2** (≈33 tests recovered from whole-file crashes); enumerate/take/
skip for-loops yield correctly. Remaining: integration 4/9 (`.collect()`
pipelines on adapters), a residual `unit_test` crasher, and the property §M
range-count residuals — tracked separately.

### NOTE — the script-cache trap (lost ~hours of this session)

`verum run`/`verum test` cache compiled modules in `~/.verum/script-cache`,
keyed by **blake3 over `.vr` content** — NOT compiler version. So codegen fixes
do **not** take effect on an unchanged `.vr` source until the cache is cleared
(`rm -rf ~/.verum/script-cache/*`). Every codegen-fix validation on a fixed repro
must clear it first; otherwise stale bytecode is served and the fix appears
inert. This is the same blake3-content-cache pattern as the embedded archive
(`build.rs:173`).

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

---

## Session 2026-06-19 — escaping-stack-ref UAF FIXED (`&*p ≡ p` fold) + 2 residual defects root-caused

### §4.5 `&*p ≡ p` raw-pointer fold — CLOSED (commit on main)

**Root cause** (traced via `VERUM_TRACE_CBGRGEN`): `ListIter::next`'s
`let item = &*self.ptr; … Maybe.Some(item)` compiled the inner `*self.ptr`
to a register holding a COPY of the loaded pointee, then wrapped that
ephemeral stack temp in a CBGR register-ref encoding `next`'s frame
`abs_index`.  When the frame's `pop_frame` bumped slot generations the ref
dangled → "CBGR use-after-free detected: expected generation 2, found 4"
on the 2nd `next()` (the gen-2 ref into the recycled slot).  The fail slot
(abs_index 22) lived in `next`'s OWN frame (base 19, range 19..29) and was
bumped twice by frame push/pop, NOT by DropRef — the DropRef-over-bump
hypothesis was wrong.

**Fix**: `compile_unary` now folds `&*p` / `&mut *p` to `p` when `p` is a
raw pointer (`&unsafe T` / `*const T` / `*mut T`), returning the heap-anchored
pointer directly.  Gated on side-effect-free Path/Field operands so the
non-pointer fall-through (`&*heap_box`, cbgr-ref reborrow) recompiles
idempotently.  **Net:** property 13→19 pass, regression 8→9, integration
4→6; basic 13/13 + protocol_agnostic 20/22 unchanged; zero regressions.
Manual `while let Some(x)=it.next()` loops now correct (sum=15).

### §4.6 RESIDUAL-A: `collect()` return-type inference → `FFIAbi.from_iter` mis-dispatch (OPEN, deep)

`collect<C: FromIterator<Self.Item>>(self) -> C { C.from_iter(self) }`.  With
`let combined: List<Int> = xs.iter()…​.collect();` the annotation type must
bind `C = List<Int>` so `C.from_iter` → `List.from_iter`.  Instead the
unresolved `C` (type-param-as-namespace) resolves to `FFIAbi` (a fabricated
fallback — FFIAbi has NO FromIterator impl in core/) → runtime panic
"method 'FFIAbi.from_iter' not found … 8 candidate(s): Text/List/Map/Set/…".
Blocks ~12 property+integration tests, BUT NOT all `.collect()` sites — the
generic body's `C` resolves CORRECTLY for some shapes (e.g.
`arr.iter().map(|x| *x*2).collect()` in `protocol_agnostic::test_collect_to_list`
passes today) and mis-resolves to `FFIAbi` for others (chain / zip / range).
The C-resolution is therefore CONTEXT-DEPENDENT, not uniformly broken.

**Call-site-rewrite approach TRIED and REVERTED this session.**  Added a
`current_collect_target` ctx field (the un-unwrapped annotation base, since
`current_return_type_name` unwraps `List<Int>`→`Int`) threaded around the
`compile_let` RHS, and a `collect()` intercept in `compile_method_call`
rewriting `iter.collect()` → `<Base>.from_iter(iter)` (modelled on the
`into()`→`From::from` arm).  Result: property 19→21, integration 6→12, but
**protocol_agnostic 20→19 — regressed `test_collect_to_list`**: the rewrite
emits `Instruction::Call{func_id: List.from_iter}` against the GENERIC
(un-monomorphised) `from_iter`, so its inner `for item in iter` over the
generic param `I` fails to dispatch `MapIter<ListIter>::next` and silently
yields a 0-length list.  Curiously `MapIter<Chain<…>>` / `MapIter<Rev<…>>`
DO iterate via that path — only `MapIter<ListIter>` collapses to 0.  A
silent-empty on the common `list.iter().map().collect()` pattern is WORSE
than the loud FFIAbi panic on rarer shapes, so the intercept was reverted.

**The correct fix is the generic body, not the call site**: the call-site
`Call{func_id}` bypasses monomorphisation, which is exactly what makes the
generic `collect` body work where it does.  Two real fix surfaces: (a) make
the generic `collect<C>(self)->C{C.from_iter(self)}` resolve `C` from the
return-type-directed expected type at monomorphisation time (so `C.from_iter`
binds `List.from_iter` for ALL shapes, not just the lucky ones); and/or
(b) close the `for item in iter` over a generic `I` param defect where
`I=MapIter<ListIter>` yields 0 (a monomorphisation-keying collision distinct
from the adapter-for-loop routing fixed in §33/bug#4).  Both are deeper than
a call-site rewrite.

### §4.7 RESIDUAL-B: range arithmetic assertions (OPEN)

`integration_range_sum`, `integration_range_product_for_factorial`,
`law_range_inclusive_count_includes_endpoint`, `law_take_plus_skip_recovers_original`
fail with `AssertionFailed: left != right`.  Likely tied to the documented
RangeInclusive `.next()` field-layout intercept defect
(range_inclusive_codegen_intercept) — `NewRange{inclusive:true}` heap object
layout `[current,end,inclusive]` mismatches the stdlib RangeInclusive
`{current,end,done}` declared layout.  Separate from §4.6.

### NOTE: `Counter` name-collision red herring

Early repros using `type Counter is {n:Int}` produced garbage field reads —
NOT a codegen defect: `Counter` shadows stdlib `core/metrics/instrument.vr`
`public type Counter` (different layout).  Always use a unique type name in
scratch repros; a `<Stdlib>{…}` literal silently binds the stdlib layout.

### §4.8 RESIDUAL-C ROOT CAUSE: combinator `&T`-element deref returns identity (find/fold/position hang+wrong) (OPEN, deep)

**This is the root cause behind the find/fold/position combinator failures AND
the `MapIter<ListIter>` collect-0 behaviour** — confirmed by bytecode + PC trace
2026-06-19/20.

Mechanism, end to end:
1. `it.next()` on a **direct local** receiver is **natively intercepted**
   (no `ListIter.next` bytecode runs — confirmed: `VERUM_TRACE_PC=ListIter.next`
   fires ZERO times for `let mut it=…; it.next()`), returning the element
   **value** (e.g. `10`). This is why manual loops and `for x in xs` work.
2. Reached via a combinator (`find`/`fold`/`position` call `self.next()`) or a
   `&mut ListIter` parameter, the real stdlib `ListIter.next` **bytecode** runs.
   `List.iter()` correctly sets `ptr = self.data` (interior data pointer), and
   `next` returns `Maybe.Some(&*self.ptr)` = the interior `&T` pointer (after the
   §4.5 `&*p` fold, `&*self.ptr` IS just `self.ptr`, a raw pointer — correct).
3. The consumer's `*x` (x bound from `Some(x)`, `x: &Int`) compiles to a GENERIC
   `DEREF` opcode, NOT the typed scalar deref (`FfiExtended DerefRaw size=8`),
   because the match-arm binding `x` is not resolved to the primitive-pointee
   type `&Int` at the deref site (it comes from `GET_VDATA` of a generic
   `Maybe<&Self.Item>`). `handle_deref` (cbgr.rs ~247) then takes the `else`
   identity branch — the interior data pointer is not a registered
   `cbgr_allocation` at `ptr-32`, so it is returned UNCHANGED instead of reading
   the 8-byte scalar. `Display`/`to_text` of that pointer renders the CONTAINING
   List (`*x` → `[10,20,30]` instead of `10`); after `offset(1)` it renders the
   raw pointer-as-int. Wrong values → assertion failures; in `find`'s
   `while let Some(x)=self.next()` the misread drives the loop state and it
   HANGS.

**Why it only bites combinators:** `handle_deref`'s identity-for-heap-object
`else` branch is CORRECT for `&Variant` (so `match *self` preserves the variant),
but WRONG for a `&primitive` interior element pointer, which must read the scalar.
The runtime cannot distinguish the two from the `Value::from_ptr` alone; the
codegen knows (`x: &Int`) but doesn't thread that type to the `*x` deref site.

**Real fix surfaces (both non-trivial):**
- (A, surgical) Type-directed deref: resolve the `Some(x)` binding's type to
  `&Int` from `it.next()`'s `Maybe<&Self.Item>` return type so `*x` emits the
  `typed_primitive_pointee_deref` FfiExtended DerefRaw (size = `T.size`) for
  primitive pointees. Only touches the primitive-pointee path; `&Record`/`&Variant`
  keep the sound identity deref. Requires match-arm-binding type propagation from
  a generic protocol-method return type.
- (B, architectural) Make element `&T` self-describing at runtime (ThinRef /
  CBGR-tracked element refs) so `handle_deref` reads the scalar without a static
  type hint — touches the core iterator reference representation.

This is the long-standing "&T into collection" hard problem (see MEMORY). The
native-intercept layer masks it for the 80% direct-loop case; combinators and
`collect` over adapter chains are the exposed 20%.
