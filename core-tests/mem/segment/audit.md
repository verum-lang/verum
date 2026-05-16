# `core.mem.segment` — audit findings

> Module under test: `core/mem/segment.vr` (1045 LOC; ~13 public
> constants, 1 record `SliceInfo`, 1 record `MemSegment` (alias
> `Segment`), 1 sum type `SegmentError`, 1 record `SegmentStats`,
> 5 free functions `segment_alloc` / `segment_free` / `segment_abandon`
> / `ptr_to_segment` / `get_segment_stats`).
>
> Test surfaces (this branch):
> `unit_test.vr` (~110 LOC), `property_test.vr` (~90 LOC),
> `integration_test.vr` (~80 LOC), `regression_test.vr` (~75 LOC).
>
> Tests pin the constants + the read-only `get_segment_stats` surface.
> Live segment_alloc → segment_free round-trip requires OS mmap
> integration; deferred to `core-tests/mem/allocator/`.

## 1. Cross-stdlib usage

| Consumer | Use |
|---|---|
| `core/mem/heap.vr` | `LocalHeap` requests pages from segments; segment_alloc backs the multi-MiB allocations. |
| `core/mem/allocator.vr` | `cbgr_alloc` for large sizes (> LARGE_PAGE_THRESHOLD) directly allocates segments. |
| `core/mem/size_class.vr` | Imports SLICE_SIZE for page-size calculations (slices_for_bin / page_size_for_bin). |

## 2. Crate-side hardcodes

Drift surfaces:

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `SEGMENT_SIZE = 32 MiB` | mmap allocation granularity | OS mmap behaviour changes if this drifts from page-boundary alignment. |
| `SLICE_SIZE = 64 KiB` | Within-segment subdivision unit | Cross-module: every page-tier calculation in size_class assumes 64 KiB slices. |
| `SLICES_PER_SEGMENT = 512` | Derived from SEGMENT_SIZE / SLICE_SIZE | Drift here breaks the slice-allocation algorithm. |
| `SMALL_PAGE_SIZE = SLICE_SIZE` (1 slice) | Small allocations occupy 1 slice | Pinned by `regression_test §D`. |
| `MEDIUM_PAGE_SIZE = 8 × SLICE_SIZE` (8 slices) | Medium allocations occupy 8 slices | Pinned by `regression_test §E`. |

## 3. Language-implementation gaps

### 3.1 Live segment_alloc requires OS mmap path

`segment_alloc(thread_id) -> Result<&mut MemSegment, SegmentError>`
allocates a 32 MiB virtual region via mmap (on Linux/macOS) or
VirtualAlloc (on Windows). Testing this requires deliberate
allocator-level integration; deferred.

### 3.2 `ptr_to_segment` recovers the segment from any user pointer

The bit trick is `ptr & ~(SEGMENT_SIZE - 1)` — works because every
segment is SEGMENT_ALIGN-aligned. This invariant MUST hold for the
ptr-to-segment recovery to work correctly. Live tests in
`core-tests/mem/allocator/`.

### 3.3 SliceInfo uses UInt8 status enum (SLICE_FREE/USED/SPAN_START/SPAN_CONTINUE)

Tag values 0/1/2/3 are encoded into a single UInt8. Pre-fix some
drafts used a sum type which broke direct array indexing in the
segment's slice-status bitmap.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Missing test coverage for `core/mem/segment.vr` | `core-tests/mem/segment/{unit,property,integration,regression}_test.vr` | New 4-file suite; ~355 LOC total. |
| 2 | Missing `audit.md` for `core-tests/mem/segment/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Live segment_alloc + free round-trip — requires OS mmap. | Blocked on `core-tests/mem/allocator/` | open |
| §B | Test ptr_to_segment bit-trick on a real allocation. | Blocked on §A | open |
| §C | Test segment_abandon (thread-detach + reclaim). | Blocked on §A | open |
| §D | Cross-tier divergence sweep on `--aot` + `--interp`. | 1 hour wall-clock | open |
