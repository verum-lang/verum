# `core.mem.size_class` — audit findings

> Module under test: `core/mem/size_class.vr` (575 LOC; 9 constants, 1
> `[Int; 73]` table `SIZE_CLASSES`, 1 private `[UInt8; 129]` lookup
> `SMALL_BIN_LOOKUP`, 12 free functions, 1 sum type `SizeClassKind`
> (alias `PageKind`), 1 statistics record `SizeClassStats`).
>
> Test surfaces (this branch):
> `unit_test.vr` (~350 LOC), `property_test.vr` (~210 LOC),
> `integration_test.vr` (~155 LOC), `regression_test.vr` (~130 LOC).
> All pass `verum test --interp`; AOT verification is the next step.

## 1. Cross-stdlib usage

Size-class lookup is on the **hot path** of every allocation — every
`cbgr_alloc(size, align)` call passes through `size_to_bin(size)` first.

| Consumer | Use |
|---|---|
| `core/mem/allocator.vr` | `cbgr_alloc` calls `size_to_bin(size)` to dispatch to the correct page queue. |
| `core/mem/heap.vr` | `LocalHeap` direct-lookup table maps small sizes to bin via `SMALL_BIN_LOOKUP`. |
| `core/mem/segment.vr` | Slice-allocation logic reads `slices_for_bin(bin)` to choose page layout. |
| `core/mem/arena.vr` | `ArenaConfig.initial_capacity` defaults align to `MAX_ALIGN_SIZE` (16). |
| `core/base/memory.vr` | `Heap.new` / `Shared.new` request sizes are dispatched here implicitly via `cbgr_alloc`. |
| `core/collections/list.vr` | List growth requests pass through `cbgr_realloc` → `size_to_bin` for both old and new sizes. |

## 2. Crate-side hardcodes

Drift surfaces — Rust-side code that hardcodes the same values:

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `BIN_COUNT = 73` / `QUEUE_COUNT = 75` | Allocator-table dimensions | Every Rust-side allocator emit path (`crates/verum_runtime/src/mimalloc_alloc.rs` if present) MUST agree on the bin-count for fixed-size arrays. |
| `SMALL_SIZE_MAX = 8192` / `MEDIUM_SIZE_MAX = 65536` / `LARGE_SIZE_MAX = 16777216` / `HUGE_SIZE_THRESHOLD = 5242880` | Page-tier boundaries | A drift between Verum-side and Rust-side boundaries means the same size could classify differently across tiers — Tier-0 interpreter vs. Tier-2 AOT divergence. |
| `WORD_SIZE = 8` | 64-bit assumption | If a 32-bit platform is added, this becomes a per-target constant. Currently hardcoded everywhere. |
| `MAX_ALIGN_SIZE = 16` | Default alignment cap | Drift here breaks `aligned_size` invariants. |
| `SIZE_CLASSES[72] = 5_242_880` | Top regular-bin equals huge-threshold | Pinned by `unit_test.test_size_classes_top_is_5_MiB` and `regression_test §A`. |
| Slice size (64 KiB) | `slices_for_bin` and `page_size_for_bin` assume 64 KiB slices | If mimalloc-style slice size changes, every page-size calculation breaks. |
| Page header (64 bytes) in `blocks_per_page` | mimalloc page-header overhead | Cross-checked against `core/mem/heap.vr::PAGE_HEADER_SIZE = 128` — `blocks_per_page` uses 64 but `heap.vr` defines 128. **Drift surface!** See §3.4. |

## 3. Language-implementation gaps

### 3.1 `size_to_bin_large` off-by-8 (CLOSED — #100)

Pre-#100, the canonical formula for the post-lookup-table range was
written as:

```verum
let b   = 63 - clz_u64(wsize as UInt64) as Int;
let sub = (wsize >> (b - 2)) & 0x03;
let bin = ((b - 3) << 2) + sub;
```

This is off by one entire bin-family (4 bins = ~50% size error) versus
the mimalloc-canonical:

```verum
let w   = wsize - 1;             // ← the missing alignment
let b   = 63 - clz_u64(w as UInt64) as Int;
let sub = (w >> (b - 2)) & 0x03;
let bin = (b << 2) + sub - 4;
```

For `wsize = 129` (1032 bytes), pre-fix returned bin 16 (320-byte slot)
when the correct answer is bin 24 (1280-byte slot). Every allocation
above 1024 bytes was being placed in a far-too-small slot, and a write
of the full requested size would have overrun the slot boundary into
the next block's header — a write-anywhere primitive.

**Closed** by the canonical rewrite in `core/mem/size_class.vr` lines
237-250. Inline tests at line 514-534 (`test_size_to_bin_large`) plus
this branch's `regression_test §A` pin the boundary points.

### 3.2 No test coverage for the `optimal_bin` 25%-fragmentation heuristic

`optimal_bin(size)` returns either `size_to_bin(size)` or
`size_to_bin(size) - 1` based on whether the fragmentation in the
chosen bin exceeds 25%. The branch that returns the smaller bin is
NEVER exercised by an inline test or by this branch's unit tests —
the heuristic body has 4 branches and only one is tested.

**Action item deferred**: construct a size for which the fragmentation
in `size_to_bin(size)` > 25% AND `size <= SIZE_CLASSES[bin - 1]`.
This appears to be a contradiction (if size ≤ the previous bin,
`size_to_bin` would have chosen the previous bin to begin with) — the
branch may be dead code. Audit the source for the actual code path.

### 3.3 `SMALL_BIN_LOOKUP` is private; bypass-only test surface

`SMALL_BIN_LOOKUP[wsize]` is the inline-emitted fast path inside
`size_to_bin`. The table is `const SMALL_BIN_LOOKUP: [UInt8; 129] = [...]`
without `public` — by design, but it means the test must exercise the
lookup table via `size_to_bin` (which routes through it for
`wsize ≤ 128`). This is the right architectural choice (the lookup is
a compiled-in detail of the fast path), but it means the test cannot
directly compare `SMALL_BIN_LOOKUP[i]` against the slow-path result.
The implicit-correspondence check happens via
`law_round_trip_full_table_exhaustive` + the per-size sweep.

### 3.4 `blocks_per_page` uses 64-byte page header; `heap.vr` defines 128

In `size_class.vr::blocks_per_page` (line 319) the page-header overhead
is hardcoded to 64 bytes. But `core/mem/heap.vr` declares
`PAGE_HEADER_SIZE: Int = 128`. The two-line mismatch is a real drift
surface: `blocks_per_page` will overestimate available block slots by
≈64 bytes per page, which can drive small-block allocators to overflow
into the page-header tail of the previous page.

**Action item LANDED (this branch)**: the `regression_test §D` pins
the constant ordering, and the audit records the discrepancy. The
canonical fix is to hoist the page-header size into a single import
from `core.mem.heap.PAGE_HEADER_SIZE`. Deferred to a follow-up
because it requires updating `size_class.vr` to depend on `heap.vr`
which may introduce a cycle.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Missing test coverage for `core/mem/size_class.vr` | `core-tests/mem/size_class/{unit,property,integration,regression}_test.vr` | New 4-file suite; ~845 LOC total. |
| 2 | Missing `audit.md` for `core-tests/mem/size_class/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Hoist `64` (page-header in `blocks_per_page`) into a single canonical const shared with `core/mem/heap.vr::PAGE_HEADER_SIZE` (currently 128). The two values disagree — clarify which is correct, then unify. | ~30 min | open — actual size depends on page-header layout consensus |
| §B | Audit the `optimal_bin` 25%-fragmentation heuristic for a dead-code branch (§3.2). | ~20 min | open |
| §C | Pin `BIN_COUNT` / `QUEUE_COUNT` / `MAX_ALIGN_SIZE` with `verum_common::well_known_types` constants so Rust-side allocator emits cannot drift. | ~45 min | open |
| §D | Cross-tier divergence sweep: run all four test files under `--aot` and confirm exit-code parity with `--interp`. | 1 hour wall-clock | open |
| §E | Remove inline `@test` functions from `core/mem/size_class.vr` (lines 486-575) — coverage has migrated to `core-tests/mem/size_class/` and the inline tests are now duplicates. | ~10 min | open |
