# `core.mem.thin_ref` — audit findings

> Module under test: `core/mem/thin_ref.vr` (644 LOC; 1 record type
> `ThinRef<T>` with 3 fields, `@repr(C, size(16), align(8))`, plus
> construction/deref helpers and the `deref_thin` / `deref_thin_mut`
> free functions).
>
> Test surfaces (this branch):
> `unit_test.vr` (~150 LOC), `property_test.vr` (~160 LOC),
> `integration_test.vr` (~135 LOC), `regression_test.vr` (~100 LOC).
>
> These tests pin the static-shape contract (layout, packed-field
> semantics, generation/epoch sentinels) WITHOUT touching a live
> allocation. Live-allocation tests are in `core-tests/base/memory/cbgr_test.vr`
> and the in-progress `core-tests/mem/allocator/` suite.

## 1. Cross-stdlib usage

`ThinRef<T>` is the foundational sized-reference type. Every `Heap<T>`
/ `Shared<T>` / `Cow<T>` / `Pin<T>` carries a `ThinRef<T>` internally;
every `&T` for sized `T` lowers to a ThinRef value at the VBC layer.

| Consumer | Use |
|---|---|
| `core/base/memory.vr` | `Heap.new` / `Shared.new` produce ThinRef-shaped values. |
| `core/mem/fat_ref.vr` | `FatRef<T>` carries a ThinRef-equivalent 16-byte head + 16 bytes of metadata. |
| `core/mem/header.vr` | `validate_reference(ptr, gen, epoch)` consumes the same (generation, epoch, caps) triple a ThinRef carries. |
| Compiler codegen | `&T` field arguments lower to ThinRef instances in the VBC register file. |

## 2. Crate-side hardcodes

Drift surfaces:

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `@repr(C, size(16), align(8))` | 16-byte total size | EVERY VBC opcode that reads/writes a ThinRef hardcodes 16 bytes. Drift = wrong-offset reads silently. |
| Field order: ptr @ 0, generation @ 8, epoch_and_caps @ 12 | Bit-layout of ThinRef in registers and memory | LLVM lowering reads at these offsets; Rust-side `verum_cbgr::ThinRef` mirror struct must match. |
| `epoch_and_caps` packing — caps in upper 16, epoch in lower 16 | Inherited from `capability.vr::pack_epoch_caps` | Cross-module drift between thin_ref / fat_ref / capability — any of the three drifting silently corrupts validity checks. |

## 3. Language-implementation gaps

### 3.1 `unsafe fn new` — caller obligation pinned only by comment

`ThinRef.new(ptr, gen, epoch, caps)` is `unsafe` with three documented
preconditions:

1. `ptr` is non-null and aligned for T
2. `ptr` points to memory with a valid `AllocationHeader` at `ptr - 32`
3. `gen` and `epoch` match the header values

There's no compiler-enforced check; the obligation rests on the caller.
Static analysis covers (1) and (3) via the type system; (2) is the
purview of `cbgr_alloc` / `Heap.new`.

### 3.2 `ThinRef.null()` is `unsafe` for type-level reasons only

The function constructs `ThinRef { ptr: 0 as &unsafe T, generation:
GEN_UNALLOCATED, ... }`. It IS safe to construct (the null sentinel
is a recognised state), but cast from `0` to `&unsafe T` requires
unsafe scope. A future API revision could provide a safe `null` constructor.

### 3.3 Live-dereference tests gated on allocator coverage

`deref_thin` / `deref_thin_mut` require:
1. A valid AllocationHeader (provided by `cbgr_alloc`)
2. The hazard pointer system (`acquire_hazard`)
3. The epoch manager (`GLOBAL_EPOCH`)

These live in `core/mem/{hazard, epoch}.vr` — the test coverage for
those modules is partial (🟡 in `docs/stdlib/mem.md`). Dedicated
deref-tests deferred to `core-tests/mem/allocator/` once the
allocator suite lands.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Missing test coverage for `core/mem/thin_ref.vr` | `core-tests/mem/thin_ref/{unit,property,integration,regression}_test.vr` | New 4-file suite; ~545 LOC total. Static-shape only — live-deref deferred. |
| 2 | Missing `audit.md` for `core-tests/mem/thin_ref/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Live-deref tests — require `core-tests/mem/allocator/` to land first. | Blocked on §B | open |
| §B | `core-tests/mem/allocator/` integration suite — must cover `cbgr_alloc` + `deref_thin` + a Drop cycle. | ~2 hours | open |
| §C | Cross-tier divergence sweep: all four files under `--aot` + `--interp` for exit-code parity. | 1 hour wall-clock | open |
| §D | `ThinRef.new` should have a refinement-typed safer wrapper that takes a `&AllocationHeader` directly, removing precondition (2) from the caller's obligation. | ~2 hours | open |

## 6. Investigation 2026-05-27 — UseAfterFreeError.new (+2 field-index shift)

Earlier audits attributed the 3 `@ignore`'d tests in
`unit_test.vr` §6 (`test_use_after_free_error_new_constructor_round_trip_pinned_by_collision`,
`test_use_after_free_error_null_pointer_uses_gen_unallocated_sentinel_pinned`,
and an analogous `.message()` pin) to a **dispatch-side name
collision** (task #17/#39 manifested as bare-suffix `.new` competing
with sibling `Heap.new` / `Shared.new` / `HazardStats.new`). A precise
diagnostic sweep on this branch (probe matrix in
`core-tests/mem/thin_ref/probe_test.vr` — removed after isolation)
shows the defect is NOT in the dispatch:

```text
DIRECT  UseAfterFreeError.new(100,99,7,6,"ProbeT")
        →  expected_gen=7  actual_gen=6  expected_epoch="ProbeT"
           actual_epoch=0.0  type_name=0.0

HELPER  build_uaf(...) wraps the same call inside a user-side helper
        →  IDENTICAL output to DIRECT

LITERAL UseAfterFreeError { expected_gen: 100, ... }
        →  CORRECT (100, 99, 7, 6, "LitT")

USERUAF UserUaf.new(...) — a user-space type with the IDENTICAL
        signature + body declared inside the test file
        →  CORRECT (100, 99, 7, 6, "UserT")

CALLFRAME CallFrame.new(...) — a stdlib type with 5-arg arity but
        all "normal" field types (Text, Text, Int, Int, UInt64)
        →  CORRECT
```

The constructor body of `UseAfterFreeError.new` writes fields **at the
correct offsets** (verified by the LITERAL probe reading slots
0..4 correctly off a directly-constructed record). The defect is at
**field READ time** on the test side, where reads of `e.<field>`
shift by **+2 indices** for every field — the read at index 0 returns
slot 2's value, index 1 returns slot 3, index 2 returns slot 4, and
indices 3/4 return uninitialised slots (rendered as NaN-boxed `0.0`).

The 1-arg `UseAfterFreeError.null_pointer("NullT")` probe pins this
sharply:

```text
constructor body writes:  f0=0, f1=0, f2=0, f3=0, f4="NullT"
read e.expected_gen (f0)  →  expected 0,   got 0        ✓ (slot 2 = 0)
read e.actual_gen   (f1)  →  expected 0,   got 0        ✓ (slot 3 = 0)
read e.expected_epoch(f2) →  expected 0,   got "NullT"  ❌ (slot 4)
read e.actual_epoch (f3)  →  expected 0,   got 0.0      ❌ (slot 5 uninit)
read e.type_name    (f4)  →  expected "NullT", got 0.0  ❌ (slot 6 uninit)
```

The match is exact: reads shift by +2 because the value-of-`e`
returned from `UseAfterFreeError.new` carries no type information
through the cross-module fn-return path. `compile_field_access` at
the test site does NOT resolve `e`'s type to `UseAfterFreeError`,
falls through `resolve_field_index`'s type-aware lookups, and lands
on the global `intern_field_name(field_name)` fallback at
`crates/verum_vbc/src/codegen/mod.rs:13687` — which returns the
sequential global StringId allocated when the field name was first
seen across the entire stdlib. For `expected_gen` that global ID
happens to be 2 in the current intern order.

**This is the same defect class as `[[btree_pattern_match_ref_generic_class]]`
and `[[enactment_field_access_oob_2026-05-24]]`** — the
"cross-module record-return field-access OOB" surface. The dispatch
itself works correctly; the `func_info.return_type_name` for
`UseAfterFreeError.new` IS recorded at `register_impl_function`
(mod.rs:8225 via `extract_type_name` + `substitute_self_in_type_name`),
but is not propagated through the static-call result register to
`variable_type_names[<dst>]` at the call site, so the test's let-
binding annotation `let e: UseAfterFreeError = ...` doesn't help
when the typechecker downstream re-derives the type for
field-access.

**Workaround pinned in `unit_test.vr` §5**: construct via direct
record literal at the test site. `test_use_after_free_error_record_construction`
+ `_eq_reflexive_via_record_literal` + `_eq_distinct_field_diff`
exercise the full surface and all pass GREEN — the `@ignore`'d
trio that pin the `.new(...)` / `.null_pointer(...)` constructor
calls remains gated on the fundamental fix.

**Fundamental fix surface** (multi-day VBC codegen work, not closed
in this branch):

1. At `compile_static_method_call` (expressions.rs:10798) — when the
   resolved `func_info.return_type_name` is `Some(name)`, propagate
   it to `self.ctx.variable_type_names[__temp_r{dst}]` so the
   downstream let-binding's `variable_type_names[var]` annotation
   isn't the only source.

2. At `compile_let` (statements.rs:285) — when the binding has an
   explicit type annotation AND the RHS is a cross-module fn return,
   the annotation is already inserted into `variable_type_names`,
   but the downstream `compile_field_access` apparently isn't
   consulting it for `resolve_field_index(Some(type_name), ...)` —
   the lookup chain at `resolve_field_index` (mod.rs:13433) needs
   verification that the archive-loaded `UseAfterFreeError` is in
   `type_field_layouts` keyed by the simple name "UseAfterFreeError".
   If `populate_types_from_archive` (mod.rs:4320) registers it
   under a different key (e.g., `core.mem.UseAfterFreeError`),
   the user-side simple-name lookup misses.

3. At `populate_types_from_archive` — verify that the simple name
   stored is exactly the bare type name without module prefix
   (the simple-name extraction at mod.rs:4338 uses
   `module.strings.get(ty.name)` which SHOULD be the bare name,
   but the archive's TypeDescriptor.name might already be a
   qualified-name StringId for stdlib types — needs verification).

This investigation supersedes the earlier dispatch-collision
hypothesis pinned in the `@ignore` comments at `unit_test.vr:204-212`.
The comments are kept for historical reference but the actual
fundamental-fix target is the cross-module type-layout propagation,
not the dispatch resolution. Pin updated in `audit.md` (this
section). See memory entry
`use_after_free_error_field_shift_2026-05-27.md` for the full
diagnostic trace.
