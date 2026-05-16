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
