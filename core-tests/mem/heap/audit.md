# `core.mem.heap` — audit findings

> Module under test: `core/mem/heap.vr` (1507 LOC; 5 constants
> (DIRECT_LOOKUP_SIZE, PAGE_HEADER_SIZE, 3 PAGE_FLAG_* bits),
> records HeapPageHeader/PageHeader/PageQueue/HeapStats/LocalHeap,
> HeapError sum type, free functions get_heap / init_thread_heap /
> shutdown_thread_heap / heap_alloc(_zeroed) / heap_free(_validated) /
> get_heap_stats).
>
> Test surfaces (this branch):
> `unit_test.vr` (~90 LOC), `property_test.vr` (~55 LOC),
> `integration_test.vr` (~45 LOC), `regression_test.vr` (~40 LOC).
>
> Static-shape only — live heap_alloc / free round-trip covered in
> `core-tests/base/memory/cbgr_test.vr`.

## 1. Cross-stdlib usage

LocalHeap is the thread-local fast path for cbgr_alloc.

| Consumer | Use |
|---|---|
| `core/mem/allocator.vr` | `cbgr_alloc` routes to `LocalHeap::alloc` for sizes ≤ LARGE_THRESHOLD. |
| Every allocating stdlib type | Through cbgr_alloc → heap_alloc fast path. |

## 2. Crate-side hardcodes

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `DIRECT_LOOKUP_SIZE = 129` | Direct lookup table size for wsize ≤ 128 | Drift causes misclassification at the 1024-byte boundary. |
| `PAGE_HEADER_SIZE = 128` | Per-page metadata | **Drift surface with `size_class.vr::blocks_per_page` which uses 64** — see `core-tests/mem/size_class/audit.md §3.4`. |
| `PAGE_FLAG_*` bit positions | Page-state flags | Drift breaks free-list traversal. |

## 3. Language-implementation gaps

### 3.1 PAGE_HEADER_SIZE mismatch with size_class.vr

`heap.vr` declares 128 bytes; `size_class.vr::blocks_per_page` uses
64 bytes. The discrepancy causes `blocks_per_page` to overestimate
available blocks by ~64 bytes/page → may overflow into adjacent
slots. Tracked as deferred follow-up in
`core-tests/mem/size_class/audit.md §3.4`.

### 3.2 Live heap_alloc tests

Pre-existing coverage in `core-tests/base/memory/cbgr_test.vr` covers
the Heap.new / Shared.new paths which funnel through heap_alloc.
Direct heap_alloc unit tests deferred.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Missing test coverage for `core/mem/heap.vr` | `core-tests/mem/heap/{unit,property,integration,regression}_test.vr` | New 4-file suite; ~230 LOC total (static-shape only). |
| 2 | Missing `audit.md` for `core-tests/mem/heap/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Unify PAGE_HEADER_SIZE between heap.vr (128) and size_class.vr (64) — see size_class §3.4. | ~30 min | open |
| §B | Direct `heap_alloc` / `heap_free` round-trip tests (currently only via Heap.new in cbgr_test). | ~1 hour | open |
| §C | Test `init_thread_heap` / `shutdown_thread_heap` lifecycle. | ~1 hour | open |
| §D | Test HeapError variants (OutOfMemory, InvalidPointer, etc.). | ~30 min | open |
| §E | Cross-tier divergence sweep on `--aot` + `--interp`. | 1 hour wall-clock | open |
