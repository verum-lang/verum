# `core.mem.header` — audit findings

> Module under test: `core/mem/header.vr` (863 LOC; 10 constants, 9
> single-bit FLAG_* constants, 1 `(UInt32)` newtype `AllocationFlags`
> with 6 predicate methods, 1 sum type `MemValidationError` with 2
> variants and constructor helpers, 1 alias `ValidationError`, 1 record
> `AllocationHeader` with 8 UInt32 fields, 2 free functions
> `validate_reference` and `validate_reference_fast`).
>
> Test surfaces (this branch):
> `unit_test.vr` (~230 LOC), `property_test.vr` (~175 LOC),
> `integration_test.vr` (~135 LOC), `regression_test.vr` (~120 LOC).
>
> Tests pin the value-level layout WITHOUT touching live allocations —
> see `core-tests/mem/allocator/` for the live-allocation tests.

## 1. Cross-stdlib usage

`core.mem.header.AllocationHeader` is the **physical layout** of every
CBGR allocation's 32-byte prefix.  Every reference type
(`ThinRef<T>`, `FatRef<T>`) and every dereference site
(`Heap.deref`, `Shared.deref`, `MaybeUninit.read`) reads the header.

| Consumer | Use |
|---|---|
| `core/mem/thin_ref.vr` | `ThinRef.{generation, epoch_and_caps}` stored copies of the header's first two UInt32 fields. |
| `core/mem/fat_ref.vr` | `FatRef.{generation, epoch_and_caps}` ditto. |
| `core/mem/allocator.vr` | `cbgr_alloc` writes a fresh `AllocationHeader` at the start of every allocation. |
| `core/mem/diagnostics.vr` | `MemHeaderView` is the read-only observer surface around `AllocationHeader`. |
| `core/base/memory.vr` | `Heap<T>.{is_valid, is_freed, current_epoch, capabilities}` each read the header via `ptr - 32`. |
| `core/mem/cap_audit_ring.vr` | Header-change writers (`record_gen_bump`, `record_attenuate`, etc.) post events when CBGR state transitions fire. |

## 2. Crate-side hardcodes

Drift surfaces — Rust-side code that hardcodes the same values:

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `HEADER_SIZE = 32` | 32-byte CBGR header — referenced as literal `32` at 4-6 sites in `core/base/memory.vr` | Pin via `regression_test §A`.  See also `core-tests/base/memory/audit.md §3.2`. |
| `GEN_INITIAL = 1` / `GEN_UNALLOCATED = 0` / `GEN_MAX = 0xFFFFFFFE` | Generation lifecycle constants | Rust-side `verum_cbgr::header::GENERATION_INITIAL` etc. MUST agree.  If a Rust-side allocator writes generation = 0 to a fresh allocation (instead of 1), `is_valid` lies; if it bumps past 0xFFFFFFFE to 0xFFFFFFFF, wraparound corrupts. |
| `FLAG_PINNED..FLAG_BORROWED` bit positions | Allocation state flag layout | Verum and Rust must agree on each bit position; drift produces silent misclassification (e.g., LEAKED reported as SHARED). |
| 8-field × 4-byte header layout (generation @ 0, epoch_and_caps @ 4, …) | The byte-for-byte field offsets | Every `atomic_load_u32(header + offset)` site in `core/mem/header.vr` reads at a hardcoded offset.  Pin the layout via a separate `verum_common::well_known_types` constant. |

## 3. Language-implementation gaps

### 3.1 `MemValidationError` short-alias `ValidationError`

`ValidationError` was added as an alias for the canonical short name.
Pre-fix, every constructor / match site had to spell the full
`MemValidationError`.  The alias works because Verum's type system
substitutes the alias name at use sites.  Pinned by
`unit_test.test_validation_error_alias_compatible` and
`integration_test.integration_validation_error_alias_round_trip`.

### 3.2 32-byte header offset literal duplicated across 4-6 sites

Currently expressed as `32` in `core/base/memory.vr` at lines
263/372/383/406/415/443 (audit pending exact line numbers).  Hoisting
into `HEADER_SIZE` import would eliminate the drift surface.  Tracked
as deferred — see `core-tests/base/memory/audit.md §3.2`.

### 3.3 Variant-construction shape coverage

The branch tests both qualified-path construction
(`MemValidationError.GenerationMismatch { expected, actual }`) AND
constructor-helper construction
(`MemValidationError.generation_mismatch(expected, actual)`).  Pinned
by `regression_test §E`.  Pre-task-#5-§3.1 the bare-name variant
resolution path could collide across stdlib sum types — closed via
commit `6cac007b1`.

### 3.4 Generation `GEN_INITIAL = 1` not `0`

A common bug pattern in CBGR allocators is to initialise the
generation counter to 0 (matching the memset-zeroed memory pattern).
This breaks the "freshly allocated" check because GEN_UNALLOCATED is
also 0.  Pin GEN_INITIAL = 1 via `unit_test.test_gen_initial_is_one`
and `unit_test.test_gen_constants_strictly_ordered`.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Missing test coverage for `core/mem/header.vr` | `core-tests/mem/header/{unit,property,integration,regression}_test.vr` | New 4-file suite; ~660 LOC total. |
| 2 | Missing `audit.md` for `core-tests/mem/header/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Hoist `32` literal CBGR header offset in `core/base/memory.vr` into a single `import core.mem.header.HEADER_SIZE`. | ~15 min | open (parent deferral in `core-tests/base/memory/audit.md §E`) |
| §B | Pin `HEADER_SIZE` / `GEN_INITIAL` / `FLAG_*` with `verum_common::well_known_types` constants so the Rust-side `verum_cbgr` crate cannot silently drift. | ~45 min | open |
| §C | Cross-tier divergence sweep: run the four test files under `--aot` and confirm exit-code parity with `--interp`. | 1 hour wall-clock | open |
| §D | Test coverage for `validate_reference` and `validate_reference_fast` — these require a live header pointer.  Move to `core-tests/mem/allocator/` integration suite. | ~30 min | open |
