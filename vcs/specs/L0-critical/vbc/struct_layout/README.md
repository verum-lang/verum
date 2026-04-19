# Struct layout regressions — **all witnesses green** ✅

This directory holds the regression suite that guards against Tier-0 VBC
bugs affecting struct-field layout. Every test has
`@expected-exit: 0` / `@expected-stdout: ok` — the file landed once
they all turned green and must stay green from here on.

## Suite

| File | Scope | Status |
|---|---|---|
| `builder_chain_with_spread.vr` | `..self` spread across many chained builders on a plain `{Int × N}` struct | ✅ passes |
| `flex_item_builder.vr` | Single-call builder on the stdlib `FlexItem` (sum-type first field + `Maybe<T>` fields) | ✅ passes |
| `ref_to_list_element.vr` | `&list[i]` followed by `.field` — interior reference + field access | ✅ passes |

## Root-cause summary

Both originally-red witnesses boiled down to VBC dispatch paths that
misrouted on specific object shapes. Fixes landed in
`crates/verum_vbc/src/interpreter/dispatch_table/handlers/`:

### `flex_item_builder.vr` — array-dispatch guard range

`dispatch_array_method` had a stale guard that skipped user record
types in **`FIRST_USER..256`** (`16..256`) only. Once a module defines
enough record types for the type-id counter to cross 256 (the stdlib
easily does — `FlexItem` landed at id 275), user types **in the gap
256..511** slipped through the guard and were treated as builtin
arrays: `flex_item.min(5)` picked the array-`min` builtin instead of
the compiled `FlexItem.min` function, returned a truncated object and
a later field access crashed with `field access out of bounds: field
index 3 … exceeds object data size 16`.

Fix: widen the skip to `FIRST_USER..TypeId::LIST.0` (`16..512`),
i.e. every id below the first built-in collection. Comment in
`method_dispatch.rs` pins the invariant so new built-ins between
FIRST_USER and LIST must update this range in lockstep.

### `ref_to_list_element.vr` — missing auto-deref for interior refs

`&list[i]` (`RefListElement`) produces a raw pointer into the List's
backing storage. Each slot holds a `Value` — typically an object
pointer for struct elements. The `Deref` handler already auto-derefs
this slot (pointer → value → element), but `handle_get_field` did
**not** — so `ref.field` read from the slot pointer as if it were an
`ObjectHeader`, producing `field access out of bounds: field index 1 …
data size 0`. Fix: in `handle_get_field`, when the pointer is in
`state.cbgr_mutable_ptrs`, load the Value from the slot and follow its
pointer before the normal header + field access path.

### Defence-in-depth: receiver-type-aware CallM

While investigating we also hardened `handle_call_method`: when the
receiver is a heap object and the method is unqualified, the dispatch
now *first* searches for `<ReceiverType>.<method>` using the receiver's
actual type-id, only falling back to the historic "first suffix match"
scan when the qualified form is missing. This prevents a related
failure mode where two stdlib types share a method name (e.g. many
`.min` registrations) and the first one registered wins — picked the
wrong body, returned a wrong-shape object.

## How to run

```
cd vcs
../target/release/vtest run specs/L0-critical/vbc/struct_layout/
```
