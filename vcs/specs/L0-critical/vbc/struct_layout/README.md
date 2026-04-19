# Struct layout regressions

This directory holds **deterministic regression witnesses** for VBC
compile + runtime bugs affecting struct field layout. Each test has
`@expected-exit: 0` and `@expected-stdout: ok` — when the compiler is
fixed they must turn green and stay green.

The tests are written to the smallest-possible repro principle: the
minimum set of language features + stdlib types that exposes each bug.

## Current witnesses

| File | What it proves | Status today |
|---|---|---|
| `builder_chain_with_spread.vr` | `..self` spread across many chained builders preserves layout for a plain `{Int × N}` struct | ✅ passes |
| `flex_item_builder.vr`         | `FlexItem.new().min(5)` from `core.term.layout` — a single builder call on a stdlib struct with `FlexBasis` + `Maybe<Int>` fields | 🔴 **fails** — `field access out of bounds: field index 3 (offset 24+8 = 32) exceeds object data size 16` |
| `ref_to_list_element.vr`       | `&list[i]` yields a usable reference through which struct fields read correctly | 🔴 **fails** — `field access out of bounds: field index 1 (offset 8+8 = 16) exceeds object data size 0` |

## Root-cause notes

Diagnostic instrumentation in `crates/verum_vbc/src/codegen/expressions.rs`
(temporary, reverted after investigation) gave us this picture:

### `flex_item_builder.vr`

Traced record allocations across a full run of
`FlexItem.new().min(5).grow(1.0)`:

```
[record] type=FlexItem declared=Some(6) literal_fields=6 alloc_slots=6   in_fn=FlexItem.new
[record-spread] type=FlexItem literal_fields=1                            in_fn=FlexItem.min
[record-spread] type=FlexItem literal_fields=1                            in_fn=FlexItem.grow
```

— so at **codegen time** the `..self` spread is correctly detected and
the `Clone` instruction is emitted. At **runtime**, however, no `Clone`
event fires for the FlexItem object (verified by instrumenting
`handle_clone`); instead the result of `.min(5)` comes out with only
2 data slots, and a subsequent field access at idx 3 panics with
"object data size 16".

Working hypothesis: `compile_method_call` has a fast-path (method
inlining or specialization) that, for stdlib builder-style methods, skips
the `Clone` and allocates a **new** object sized only from the explicit
fields in the spread literal. For methods whose body uses `..self` this
is incorrect — the spread must materialise a full-size copy of the
receiver.

Suspect region: `crates/verum_vbc/src/codegen/expressions.rs` around
`fn compile_method_call` (line ~4216) plus `fn compile_record` (spread
branch at ~10295).

### `ref_to_list_element.vr`

`&list[i]` (taking a reference to an indexed list element) returns a
value whose field-access reads back zeroes. Works if the element is
first copied into a local (`let v = xs[0]; let r = &v`). Suggests the
`Ref` + `Index` combination in expression position emits a `BorrowIndex`
sequence that doesn't preserve the element's heap identity — it ends up
referencing a 0-byte temporary.

## How to run

```
cd vcs
../target/release/vtest run specs/L0-critical/vbc/struct_layout/
```

## Fix protocol

1. Land the green reproducer (`builder_chain_with_spread.vr`) as the
   baseline invariant — **must stay green**.
2. Investigate the two red witnesses with `VERUM_TRACE_RECORD=1` and
   `VERUM_TRACE_CLONE=1` after temporarily reinstating the diagnostics
   (see the commit that originally added them under
   `crates/verum_vbc/src/codegen/expressions.rs:compile_record`).
3. Write the fix + add a `builder_chain_sum_type_field.vr` counterpart
   once the mechanism is understood.
4. Do **not** close this directory until all three witnesses are green
   and `examples/term_*` can round-trip through `verum run --aot`.
