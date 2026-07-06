# `core.mem.allocator` — audit findings

> Module under test: `core/mem/allocator.vr` (1933 LOC; the largest
> file in `core/mem/`).  Surface includes ~9 constants (SIZE_CLASSES
> table, PAGE_SIZE, CHUNK_SIZE, LARGE_THRESHOLD, MAX_SLOTS_PER_PAGE,
> GUARD_PAGE_SIZE), Layout record, AllocError sum type, Alloc protocol,
> size_class_index/size_class_to_size, AllocPageHeader/PageHeader,
> LargeAllocHeader, AllocatorLocalHeap/LocalHeap, cbgr_alloc + family
> (cbgr_alloc_zeroed / cbgr_dealloc / cbgr_realloc /
> get_header_from_ptr), context-scoped allocator (ctx_alloc family),
> Allocator/GlobalAllocator protocols, TieredAllocator/SimpleAllocator/
> MemStackAllocator, AllocStats + get_alloc_stats /
> get_global_alloc_stats, align_up / align_down.
>
> Test surfaces (this branch):
> `unit_test.vr` (~145 LOC), `property_test.vr` (~85 LOC),
> `integration_test.vr` (~65 LOC), `regression_test.vr` (~50 LOC).
>
> Tests cover the STATIC-SHAPE surface only — constants, alignment
> arithmetic, size_class round-trip, Layout record.  Live
> cbgr_alloc / cbgr_dealloc / cbgr_realloc round-trips are exercised
> in `core-tests/base/memory/cbgr_test.vr` because they require the
> complete CBGR header lifecycle.

## 1. Cross-stdlib usage

`cbgr_alloc` is the **bottom of the allocation stack** — every `Heap.new`,
`Shared.new`, `List`/`Map`/`Set` backing-buffer allocation, `Text` heap
spill, and `core/mem/segment` extension all funnel through here.

| Consumer | Use |
|---|---|
| `core/base/memory.vr` | `Heap.new` / `Shared.new` → `cbgr_alloc(layout)` |
| `core/collections/list.vr` | List growth → `cbgr_realloc` |
| `core/mem/heap.vr` | `LocalHeap` thread-local fast path layers on top |
| `core/mem/arena.vr` | Arena buffer extension calls `cbgr_alloc` |
| `core/mem/segment.vr` | Segment backing-mmap may call cbgr_alloc for metadata |

## 2. Crate-side hardcodes

Drift surfaces:

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `SIZE_CLASSES` 11-entry basic mimalloc table | Allocation-bin selection | Drift would silently route allocations to wrong-sized bins → write overruns. |
| `PAGE_SIZE = 64 KiB` | Within-segment unit | Cross-module: segment.SLICE_SIZE must agree. |
| `CHUNK_SIZE = 2 MiB` | Cross-thread chunk granularity | Affects thread-local heap reclamation. |
| `LARGE_THRESHOLD = 2048` | Above this → direct mmap path | Pinned to top of SIZE_CLASSES. |
| `MAX_SLOTS_PER_PAGE = 512` | Page's free-list capacity | Drift breaks free-list traversal. |
| `GUARD_PAGE_SIZE = 4 KiB` | Between-allocation overflow detection | Drift breaks overflow trap. |

## 3. Language-implementation gaps

### 3.1 Live allocator tests gated on CBGR header lifecycle

The full surface (`cbgr_alloc` → `cbgr_dealloc` → `cbgr_realloc`) is
exercised in `core-tests/base/memory/cbgr_test.vr`. Tests here pin
the static-shape contract only because exercising the live surface
requires the CBGR header to be initialised correctly, which is
itself tested elsewhere.

### 3.2 Context-scoped allocator API

`set_context_allocator` / `clear_context_allocator` / `ctx_alloc`
family are designed for per-task allocator injection (e.g., switching
to an arena for a request scope). Test coverage requires task
spawning; deferred.

### 3.3 Specialised allocator implementations

`TieredAllocator`, `SimpleAllocator`, `MemStackAllocator` are
implementations of the `Allocator` protocol. Coverage for each
involves constructing the allocator, observing behaviour, and
verifying conformance. Deferred to follow-up.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Missing test coverage for `core/mem/allocator.vr` | `core-tests/mem/allocator/{unit,property,integration,regression}_test.vr` | 4-file suite; ~345 LOC total (static-shape only). |
| 2 | Missing `audit.md` for `core-tests/mem/allocator/` | This file. |
| 3 | §A live cbgr_alloc / cbgr_dealloc lift | `integration_test.vr` §2 — `integration_heap_new_returns_valid_allocation`, `integration_heap_drop_at_scope_end`, `integration_heap_alloc_dealloc_burst` (32-cycle churn), `integration_heap_small_size_class`, `integration_heap_medium_size_class` (128-byte struct), `integration_heap_into_raw_from_raw_round_trip`, `integration_shared_clone_drops_back_to_one`. Routes through public `Heap<T>` / `Shared<T>` API which internally calls `cbgr_alloc` / `cbgr_dealloc`. Closes §A. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| ~~§A~~ | ~~Live cbgr_alloc / cbgr_dealloc round-trip — covered partially in `core-tests/base/memory/cbgr_test.vr`; refactor and consolidate here.~~ | **CLOSED via §4.3 above** — lifted through public `Heap<T>` / `Shared<T>` API. |
| §B | Test cbgr_realloc with growth across size-class boundaries.  Lift route: `let mut v: List<Int> = []; for ... { v.push(i) }` — every grow crosses class boundaries through `cbgr_realloc`. | ~1 hour | open |
| §C | Test context-scoped allocator (`set_context_allocator` + `ctx_alloc`). | ~1 hour | open |
| §D | Test `Alloc` / `Allocator` protocol implementations (TieredAllocator, SimpleAllocator, MemStackAllocator). | ~3 hours | open |
| §E | Test AllocStats accuracy across alloc + dealloc cycles. | ~30 min | open |
| §F | Cross-tier divergence sweep on `--aot` + `--interp`. | 1 hour wall-clock | open |

## Session 2026-07-05 — wrapper-method surface closed; 63/63 GREEN

The 2 residual failures closed in `799cff9b2` (HEAP-INTORAW-1 /
SHARED-STRONGCOUNT-1): the Tier-0 runtime representations of the
allocating wrappers (`Heap<T>` = CBGR data pointer; `Shared<T>` =
`[ObjectHeader][refcount][value]`) do not match the source-level
records in `core/base/memory.vr`, so the COMPILED
`into_raw`/`from_raw`/`strong_count` bodies misread memory
(`self.ptr` on a Heap surfaced the stored payload as a bogus
type_id).  Shape-guarded Rust intercepts now cover BOTH dispatch
paths (`wrapper_runtime.rs` for statically-resolved `Call`,
`method_dispatch.rs` arms for `CallM`): `into_raw`/`from_raw` are
identity on the CBGR data pointer (forget-semantics), `strong_count`
/`weak_count`/`is_unique` read the refcount slot, `clone` bumps it on
all three clone dispatch arms, binding-drop (`DropRef`) decrements
it, and `*shared` derefs to the inner value.  §4 round-trip and §5
clone-count tests both green; suite **63/63** (was 61/2).
