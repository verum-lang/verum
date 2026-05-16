# `core.mem.fat_ref` — audit findings

> Module under test: `core/mem/fat_ref.vr` (785 LOC; 1 record type
> `FatRef<T>` with 6 fields, `@repr(C, size(32), align(8))`, plus
> construction/slice/dyn helpers and the `empty_slice` free function).
>
> Test surfaces (this branch):
> `unit_test.vr` (~145 LOC), `property_test.vr` (~135 LOC),
> `integration_test.vr` (~125 LOC), `regression_test.vr` (~115 LOC).
>
> Static-shape tests only — live-allocation tests are deferred to
> `core-tests/mem/allocator/`.

## 1. Cross-stdlib usage

`FatRef<T>` is the reference for unsized types — slices (`[T]`),
trait objects (`dyn P`), and subslice views.

| Consumer | Use |
|---|---|
| Compiler codegen | `&[T]` and `&dyn P` field arguments lower to FatRef instances. |
| `core/collections/list.vr` | `List<T>.as_slice()` produces a FatRef-shaped value. |
| `core/text/text.vr` | `&Text.as_bytes()` produces a `FatRef<[Byte]>`. |
| `core/mem/thin_ref.vr` | Shares the first 16 bytes of layout (ptr + generation + epoch_and_caps). |
| `core/mem/header.vr` | Validation reads (gen, epoch) the same way as for ThinRef. |

## 2. Crate-side hardcodes

Drift surfaces:

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `@repr(C, size(32), align(8))` | 32-byte total size | Cache-line-half alignment. Every VBC opcode hardcodes 32 bytes. |
| Field order ptr/gen/epoch_caps/metadata/offset/reserved | Bit-layout of FatRef in registers and memory | LLVM lowering reads at hardcoded offsets; Rust-side `verum_cbgr::FatRef` mirror must match. |
| `metadata: Int` (signed 64-bit) | Slice length OR vtable pointer OR -1 sentinel | If switched to UInt, the -1 sentinel collides with `2^63 - 1` slice length silently. |
| `offset_from_base: UInt32` | 4 GiB max subslice offset | Allocations larger than 4 GiB cannot use FatRef subslices. Mostly fine — pin the constraint. |

## 3. Language-implementation gaps

### 3.1 Subslice offset arithmetic must be uniform with header recovery

The CBGR validation reads the header at `ptr - offset_from_base - HEADER_SIZE`.
Every site that creates a sub-FatRef must update offset_from_base
correctly. Pre-fix some early drafts omitted the offset subtraction
when recovering the header, reading garbage for subslices.

Pinned by `regression_test §C`.

### 3.2 Metadata field is `Int` (signed) for sentinel support

`metadata: Int` supports:
- Slice length (positive, 0 to UInt63.MAX)
- Vtable pointer (positive memory address)
- -1 sentinel (unknown length / invalid vtable)

A switch to UInt would silently collide `2^63 - 1` slice length with
the -1 sentinel. Pinned by `unit_test.test_fat_ref_metadata_field_*` and
`regression_test §D`.

### 3.3 Live-deref tests gated on allocator coverage

Same as thin_ref §3.3 — deferred to `core-tests/mem/allocator/`.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Missing test coverage for `core/mem/fat_ref.vr` | `core-tests/mem/fat_ref/{unit,property,integration,regression}_test.vr` | New 4-file suite; ~520 LOC total. |
| 2 | Missing `audit.md` for `core-tests/mem/fat_ref/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Live-FatRef tests with real slices / dyn objects — must follow `core-tests/mem/allocator/`. | Blocked on `mem/allocator/` landing | open |
| §B | Cross-tier divergence sweep: all four files under `--aot` + `--interp`. | 1 hour wall-clock | open |
| §C | Test the `slice_view(start, end)` subslice constructor with bounds violations (currently returns Maybe<FatRef<[T]>>). Requires live allocation. | ~30 min | open |
| §D | Test `dyn Protocol` vtable layout — requires live trait-object construction. | ~1 hour | open |
