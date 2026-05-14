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

### §3.3 Format-literal Display dispatch — CLOSED 2026-05-14

**Defect class** — `f"{x}"` interpolation and `print(x)` both lowered
unconditionally to the runtime `ToString` opcode (`expressions.rs::
compile_interpolated_string` at the legacy `else` branch).
`ToString` calls `format_value_for_print` (a Debug-style runtime
formatter) which prints variant NAMES (`Less` / `Equal` / `Greater`)
instead of dispatching to the user-defined
`implement Display for <T> { fn fmt(&self, f: &mut Formatter) … }`
body.  Every Display impl across `core/` (Ordering, Maybe, Result,
domain errors, every glyph or operator-style render) was silently
bypassed by f-strings.

**Live repro** — `f"{Less}"` produced `Less` instead of `<`.
Workaround was manual Formatter wiring at every call site:
```verum
let mut buf: Text = "";
let mut f = Formatter.new(&mut buf);
let _ = value.fmt(&mut f);
print(buf);
```

**Fundamental fix** — new codegen helper
`try_emit_display_dispatch` in
`crates/verum_vbc/src/codegen/expressions.rs` (~150 LOC) emits the
canonical Formatter wiring inline at every f-string placeholder:

```text
buf       := ""
formatter := Formatter.new(&mut buf)
_         := value.fmt(&mut formatter)    # CallM
str_reg   := buf
```

**Detection signals** — the dispatch fires only when the receiver
type has a Display impl, gated by EITHER:
* `lookup_function(<TypeName>.fmt)` finds the impl in the local
  function table (user-defined or eagerly mounted), OR
* The codegen's type-descriptor table for `<TypeName>` lists a
  `Display`-family protocol impl (`type_desc.protocols`) —
  surfaces stdlib Display impls that the user-side module never
  references directly.

The actual call is emitted as `CallM` (dynamic dispatch by method
name + receiver), not static `Call`, so the runtime resolves the
fmt body even when the local function table doesn't contain
`<TypeName>.fmt` — necessary because stdlib Display impls are
lazily linked into user modules.

**Primitives bypass** — Int / Float / Bool / Byte / Char / Int32 /
UInt64 / Float32 / Float64 / Int{8,16}/ UInt{8,16,32} / USize /
ISize keep the inline `ToString` fast path.  Their runtime
formatter matches the stdlib `implement Display for <Primitive>`
bit-for-bit, and routing through the protocol-dispatch chain
would inflate each interpolation from one opcode to ~9 opcodes
for zero semantic gain.

**Architectural rule** — every f-string placeholder + every
`print(value)` site that wants protocol-aware formatting MUST
funnel through `try_emit_display_dispatch`.  Emitting `ToString`
directly is a regression on user Display impls.

### §3.3.1 Cross-module Display dispatch — CLOSED 2026-05-14 (task #10)

The initial §3.3 fix worked only when `<Type>.fmt` was already
materialised in the user-side function table — user-defined
impls in the same module + stdlib impls explicitly referenced
via `.fmt()`.  Stdlib Display impls (Ordering / Maybe / Result /
…) land in the function table under FULLY-QUALIFIED keys like
`core.base.Ordering.fmt`, so the bare `lookup_function("Ordering.fmt")`
missed them and `f"{var}"` for `var: Ordering` fell back to
`ToString`.

Closed by two architectural channels added to
`try_emit_display_dispatch` (commit `a840262f9`):

1. **Function-table parent-scan** — walks `ctx.functions` for
   entries whose `parent_type_name == base` AND key ends with
   `.fmt` (or key matches `.<Base>.fmt`).  Captures the
   FunctionId into `display_func_id` for static `Call`
   emission.  Surfaces every stdlib Display impl loaded under
   its canonical module-qualified key.

2. **TypeDescriptor → ProtocolImpl probe** — `ProtocolId` is a
   TypeId reference (protocols are types).  When `self.types`
   contains the base type's descriptor with a `ProtocolImpl`
   pointing to the Display protocol type, `proto_impl.methods[0]`
   is the canonical Display `fmt` FunctionId.  This channel
   surfaces Display impls even when the function table doesn't
   have them — the type descriptor is loaded eagerly by
   `register_archive_type` for every archived type.

Same multi-channel applied to `Formatter.new`: bare key first,
then function-table scan pinned by `parent_type_name == "Formatter"`
AND key ends with `.new`.

The emit path branches on `display_func_id`:
* `Some(fid)`: static `Call { func_id: fid, … }` — fastest
  dispatch shape, resolves at compile time, optimal for both
  Tier-0 interpreter and Tier-1 LLVM AOT.
* `None`: `CallM` fallback (dynamic dispatch by method name).

### §3.4 Other defects unrelated to ordering

`test_ordering_across_units` (lives in
`core-tests/time/duration/integration_test.vr` and exercises
Duration's `Ordering`-returning `cmp`) — closed transitively by
§3.2's primitive-intercept fix.

## Action items landed in this branch

* Added canonical `resolve_arg_value` helper to
  `cbgr_helpers.rs` (+62 LOC including doc).
* Replaced 24 buggy `is_cbgr_ref / else` sites in
  `method_dispatch.rs` with one-line `resolve_arg_value` calls
  (–~140 LOC, +24 LOC).
* Added `try_emit_display_dispatch` codegen helper in
  `expressions.rs` (+150 LOC) — routes f-strings through user
  Display impls when one exists.
* Pinned 8 fresh regression tests in
  `core-tests/base/ordering/regression_test.vr` (4 for §3.2 +
  4 for §3.3).
* Updated INVENTORY row with `base/ordering` status.
* Documentation updates in `internal/website/docs/stdlib/base.md`.

## Action items deferred

* §3.1 iterator-item method resolution (type-inference ordering) —
  pre-existing defect, separate task.
