# `base/ordering` audit

Module: `core/base/ordering.vr` (196 LOC) — defines the three-valued
`Ordering` ADT (`Less | Equal | Greater`) plus its `Eq` / `Ord` /
`Clone` / `Default` / `Debug` / `Display` instances and the canonical
`reverse` / `is_less` / `is_eq` / `is_greater` / `then` /
`then_with` helpers consumed by every `Ord` implementation across the
stdlib.

Tests: `unit_test.vr` (1015 LOC, ~75 `@test`s), `property_test.vr`
(429 LOC — exhaustive Cartesian product over the 3-element domain),
`integration_test.vr` (328 LOC — `Maybe.cmp` / bubble-sort / lex chain /
sys.memory.Ordering interop), `regression_test.vr` (~150 LOC — pre-fix
bug shapes pinned forever).

## 1. Cross-stdlib usage

`Ordering` is consumed by:

| crate | what it does |
|---|---|
| `core/base/primitives.vr` | `implement Ord for Int / Float / Bool / Char / Byte / Int32 / UInt64` — every primitive `cmp` returns `Ordering`. |
| `core/base/maybe.vr` | `implement Ord for Maybe<T> where T: Ord` — `None < Some(_)`; inner compare delegates. |
| `core/base/result.vr` | `implement Ord for Result<T, E>` — `Err(_) < Ok(_)`. |
| `core/collections/list.vr` | `List.sort`, `binary_search`, `sort_by` consume the comparator-returned `Ordering`. |
| `core/sys/memory.vr` | Names a *different* type (`Ordering` for memory-barrier strength). Conflicts on bare `Ordering` are handled by canonical-name resolution; verified by the §6 integration tests in `integration_test.vr`. |

Every consumer reaches `Ordering` through `core.base.ordering.{Less,
Equal, Greater}` — no consumer reimports under a different name.

## 2. Crate-side hardcodes

`crates/verum_common/src/well_known_types.rs::ORDERING_VARIANT_LAYOUT`
pins the variant tag → name mapping (Less=0, Equal=1, Greater=2). Any
codegen or runtime intercept that emits `Ordering` MUST agree with this
table — drift is caught at module load by
`type_id_drift::tests::ordering_variant_layout_pinned`.

`crates/verum_vbc/src/interpreter/dispatch_table/handlers/method_dispatch.rs::make_ordering`
is the single point of truth for converting `std::cmp::Ordering` into a
Verum `Ordering` value. Every Rust-side primitive `cmp` intercept funnels
through it.

## 3. Language-implementation gaps

### §3.1 Iterator-item method resolution (pre-existing)

`for ord in xs.iter() { ord.reverse() }` — the iter-item type is
inferred too late for method lookup, so the bare `.reverse()` call
fails to resolve. Work-around: `(*ord).reverse()`. Pinned by
`regression_test.vr::test_iterator_deref_reverse`.

### §3.2 Primitive-method intercepts missed heap-interior refs — CLOSED 2026-05-14

**Defect class** — every primitive comparison / equality method
intercept (`Int.cmp` / `eq` / `ne` / `lt` / `le` / `gt` / `ge`,
plus the Bool / Float / Byte / Char / Int32 / UInt64 mirrors) only
handled CBGR REGISTER refs (`is_cbgr_ref`). HEAP-INTERIOR refs
produced by `RefListElement` (`&xs[i]`) / `RefField` (`&record.f`)
fell through the `else` branch and `arg.as_i64()` returned the
pointer ADDRESS as a value.

**Live repro** — `xs[0].cmp(&xs[1])` for `xs = [5, 2]` returned
`Less` instead of `Greater`. Every bubble-sort over indexed elements
silently produced the input unchanged. `is_greater()` on the
returned Ordering was correct given its corrupted input.

**Fundamental fix** — new canonical helper in
`crates/verum_vbc/src/interpreter/dispatch_table/handlers/cbgr_helpers.rs`:

```rust
pub(super) fn resolve_arg_value(state: &InterpreterState, val: Value) -> Value;
```

Unifies all three Verum ref shapes (CBGR register-ref, heap-interior
ptr marked in `cbgr_mutable_ptrs`, ThinRef) into a single Value
that callers `.as_i64()` / `.as_bool()` / `.as_f64()` against.

Replaces 24+ duplicated `if is_cbgr_ref(&other_val) { ... } else
{ other_val.as_<T>() }` sites in `method_dispatch.rs` with one-line
calls. Pinned forever by `regression_test.vr §regression_cmp_*`
+ `regression_bubble_sort_via_indexed_cmp_chain` +
`regression_eq_on_two_indexed_int_arguments` +
`regression_lt_le_gt_ge_on_indexed_arguments`.

**Architectural rule** — every primitive-method intercept that
consumes `other: &T` MUST funnel the raw register read through
`resolve_arg_value`. The buggy alternative (only check
`is_cbgr_ref`, else call `.as_T()`) silently regresses on every
`&xs[i]` / `&record.f` borrow. Pin: grep
`is_cbgr_ref\(&(other_val|arg_val|receiver)\) \{` in
`method_dispatch.rs` — every match outside `resolve_arg_value`
itself is a bug.

### §3.3 Other defects unrelated to ordering

None remaining for the `Ordering` API surface itself. Three failing
integration tests landed pre-§3.2:
* `integration_sort_via_cmp_ordering` — closed by §3.2.
* `integration_display_ordering_shows_operator_glyph` — gated by
  Ordering Display impl returning a garbage Text on `f"{ord}"`;
  the `Ordering` Display body itself looks correct in
  `core/base/ordering.vr` — needs a separate audit pass to confirm
  whether Display goes through the same intercept defect or whether
  this is a fresh Display-dispatch shape. Tracked in INVENTORY.
* `test_ordering_across_units` — lives in
  `core-tests/time/duration/integration_test.vr` and exercises
  Duration's `Ordering`-returning `cmp`; bisected dependency on
  §3.2 — re-evaluate post-fix.

## Action items landed in this branch

* Added canonical `resolve_arg_value` helper to
  `cbgr_helpers.rs` (+62 LOC including doc).
* Replaced 24 buggy `is_cbgr_ref / else` sites in
  `method_dispatch.rs` with one-line `resolve_arg_value` calls
  (–~140 LOC, +24 LOC).
* Pinned 4 fresh regression tests in
  `core-tests/base/ordering/regression_test.vr`.
* Updated INVENTORY row with `base/ordering` status.

## Action items deferred

* §3.1 iterator-item method resolution (type-inference ordering) —
  pre-existing defect, separate task.
* Display(Ordering) → garbage Text path — needs Display-dispatch
  audit similar to §3.2.
