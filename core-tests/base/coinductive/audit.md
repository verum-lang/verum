# `core/base/coinductive` — Audit

> Module: `core/base/coinductive.vr` — productivity & bisimulation
> primitives for greatest-fixed-point types. The user-facing surface
> for `verum_types.coinductive_analysis`.

## §1 — Public API surface (current)

### 1.1 Types

| Type | Shape | Public? |
|---|---|---|
| `CorecursiveCall` | record `{ callee: Text, guard_depth: Int{>= 0} }` | yes |
| `ProductivityResult` | sum `Productive \| NonProductive { unguarded: List<Text> }` | yes |
| `Observation` | record `{ label: Text, payload: Text }` | yes |
| `ObservationTrace` | record `{ steps: List<Observation> }` | yes |
| `BisimulationResult` | sum `Bisimilar { observed_depth: Int } \| Distinct { divergence_at: Int }` | yes |

### 1.2 Free functions

| Free fn | Signature |
|---|---|
| `corec_call` | `(Text, Int{>= 0}) -> CorecursiveCall` |
| `is_guarded` | `(CorecursiveCall) -> Bool` |
| `check_productivity` | `(List<CorecursiveCall>) -> ProductivityResult` |
| `observation` | `(Text, Text) -> Observation` |
| `trace` | `(List<Observation>) -> ObservationTrace` |
| `trace_prefix` | `(ObservationTrace, Int{>= 0}) -> ObservationTrace` |
| `observations_equal` | `(Observation, Observation) -> Bool` |
| `bisimilar_up_to` | `(ObservationTrace, ObservationTrace, Int{>= 0}) -> BisimulationResult` |

### 1.3 Method-form additions (this branch — pending precompiled stdlib refresh)

| Type | Method | Equivalence |
|---|---|---|
| `CorecursiveCall` | `.is_guarded(&self) -> Bool` | mirrors free-fn `is_guarded(c)` |
| `CorecursiveCall` | `.new(callee, guard_depth)` | mirrors `corec_call(callee, depth)` |
| `ProductivityResult` | `.is_productive(&self) -> Bool` | NEW |
| `ProductivityResult` | `.is_non_productive(&self) -> Bool` | NEW |
| `ProductivityResult` | `.unguarded_names(&self) -> List<Text>` | accessor over the `NonProductive` payload |
| `Observation` | `.new(label, payload)` | mirrors `observation(...)` |
| `ObservationTrace` | `.new(steps)` / `.from_list(steps)` | mirrors `trace(steps)` |
| `ObservationTrace` | `.len(&self) -> Int` | accessor over `steps.len()` |
| `ObservationTrace` | `.is_empty(&self) -> Bool` | derived from `.len() == 0` |
| `ObservationTrace` | `.get(&self, i) -> Maybe<Observation>` | bounds-checked subscript |
| `ObservationTrace` | `.prefix(&self, n) -> ObservationTrace` | mirrors `trace_prefix(self, n)` |

These method-forms are additive: every existing call site continues to
work. They are exercised once the precompiled binary regenerates
`runtime.vbca` against this branch's source.

## §2 — Findings landed in this branch

### 2.1 integration_test.vr referenced a hallucinated API surface

Pre-fix `integration_test.vr` called methods and ctors that don't exist:

| Pre-fix call | Status |
|---|---|
| `CorecursiveCall.new("name", depth, [nested_calls])` | 3-arg form — does not exist. The type has 2 fields. |
| `outer.recursive_calls.get(0)` | Field `recursive_calls` does not exist. |
| `deep.call_depth` | Field is `guard_depth`, not `call_depth`. |
| `outer.is_guarded()` | Method form did not exist (now added in this branch). |
| `Observation.new(1, "step-1")` | Implied `Observation<T>` is generic with `value: T, label: Text` — actual is non-generic `{ label: Text, payload: Text }`. |
| `obs.value` | Field `value` does not exist; the payload field is `payload`. |
| `ObservationTrace.from_list([...])` | Method form did not exist (now added). |
| `trace_prefix(&trace, n)` | `trace_prefix` takes `ObservationTrace` by value, not by ref. |
| `prefix.len()` | Method form did not exist on `ObservationTrace` (now added). |
| `prefix.get(0).map(|o| o.value)` | `.get` method form did not exist (now added). |
| `ProductivityVerdict` | Type is `ProductivityResult`. |
| `check_productivity(&single_call)` | Takes `List<CorecursiveCall>`, not a single call. |
| `verdict.is_productive()` | Method form did not exist (now added). |

**Fix in this branch**: integration_test.vr rewritten end-to-end against
the actual surface (16 tests, all `--interp` green). The stdlib was
ALSO extended with idiomatic method-form constructors and accessors
(see §1.3), so the next iteration of integration_test.vr can promote
to a richer surface once `runtime.vbca` regenerates.

### 2.2 Field-access defect through `List<Observation>[i]` subscript

`t.steps[i].payload` and `t.steps[i].label` lose the `Observation`
type at the field-access site and route to a wider stdlib record's
field offset.

* Panic shape: `"field access out of bounds: field index 5 (offset 40+8 = 48) exceeds object data size 16 type_id=454 type='Observation'"`
* Defect class: same root as [[btree_pattern_match_ref_generic_class]]
  and [[enactment_field_access_oob_2026-05-24]].
* Cause: VBC codegen `extract_expr_type_name` and the field-index
  resolver lose the receiver's monomorphised type when the receiver
  is a `List<T>` subscript on a stdlib record. The resolver falls back
  to a global field-name scan and picks the first record with a
  matching field name (Database SqliteApiValue / TracePayload /
  ResultPayload / etc. all have a `payload` field).
* Workaround applied in `integration_test.vr`: bind the subscript
  result to an explicitly-typed local first
  (`let o: Observation = t.steps[i]; assert_eq(o.payload, ...)`),
  which pins the type for the field-resolver.
* Underlying defect pinned at `regression_test.vr §E` as `@ignore`'d
  test — flips green when VBC codegen propagates the subscript's
  element type through `compile_field_access`.

## §3 — Cross-stdlib usage audit (pending)

Consumers of `core.base.coinductive` that need surface validation:

* `verum_types.coinductive_analysis` (Rust-side; outside the
  `--interp` test surface — covered at the verum_types layer).
* No other `core/` modules reference this layer at present.

## §4 — Crate-side hardcodes (pending)

Pending grep over `crates/` for hardcoded names / tags / signatures
of `CorecursiveCall` / `Observation` / `ObservationTrace`.

## §5 — Action items landed in this branch

1. `core/base/coinductive.vr` — three impl-blocks added with
   idiomatic method-form constructors and accessors (CorecursiveCall,
   ProductivityResult, Observation, ObservationTrace).
2. `core-tests/base/coinductive/integration_test.vr` — rewritten
   end-to-end against the actual API surface; 6 sections, 16 tests.
3. `core-tests/base/coinductive/regression_test.vr` — NEW: 6 pinned
   regressions covering canonical record layout, productivity-list
   semantics, threshold at `guard_depth == 1`, `trace_prefix`
   saturation, and one `@ignore`'d entry for the
   `[[btree_pattern_match_ref_generic_class]]` field-access defect
   surfaced in this branch's testing.
4. `core-tests/base/coinductive/audit.md` — NEW (this file).

## §6 — Action items deferred

| Item | Scope estimate | Tracked as |
|---|---|---|
| Method-form integration tests via `Observation.new` / `ObservationTrace.from_list` / `.get` / `.prefix` / `.is_productive` | 1h — add a `method_form_test.vr` once precompiled stdlib refreshes | follow-up after binary rebuild |
| Property-test sweep over `bisimilar_up_to` symmetric / antisymmetric / reflexive / transitive laws | 2h | future task |
| `ObservationTrace` size-bounded `Default` impl | 30min | future task |
| `Display` / `Debug` impls for `CorecursiveCall` / `ProductivityResult` / `Observation` / `ObservationTrace` / `BisimulationResult` | 1h | future task |
| Field-access through `List<T>[i]` subscript defect close-out | multi-day VBC codegen work | shared with [[btree_pattern_match_ref_generic_class]] |
