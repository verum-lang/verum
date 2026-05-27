# `runtime/task_queue` audit

Module: `core/runtime/task_queue.vr` (1051 LOC) — Chase-Lev work-
stealing deque + WorkStealingPool + BoundedQueue + StealResult<T> ADT.

**`@cfg(any(runtime = "full", runtime = "single_thread"))`** — the
module is mounted only under these two profiles (per `core/runtime/
mod.vr:116`).  Under the default conformance suite (`runtime = "full"`)
the module IS in scope.

Tests: 13 unit tests covering StealResult<T> 3-variant + .is_success /
.is_empty / .into_option helpers.  Live work-stealing deque tests
(push / pop / steal / grow / fairness) deferred to
`vcs/specs/L2-standard/runtime/task_queue/`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.runtime.config.WorkStealingExecutor` | uses `WorkDeque<T>` + `Stealer<T>` per worker thread. |
| `core.async.spawn` | inserts tasks into the per-worker `WorkDeque` via `push`. |
| Async runtime scheduler | round-robins `Stealer.steal()` from idle workers. |

## 2. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| `DEFAULT_CAPACITY = 256` (`task_queue.vr:91`) | initial ring-buffer slots per deque | Workload over 256-deep blocks on resize amortisation; not a hard hazard but pins the per-worker memory baseline at 256 × `sizeof(T)`. |
| `MIN_CAPACITY = 16` (`task_queue.vr:94`) | smallest allowed ring-buffer | A new() with `< 16` silently rounds up. |
| `MAX_CAPACITY = 1 << 24` (~16M tasks) (`task_queue.vr:97`) | upper bound | A workload that hits this stalls / OOMs the runtime. |
| `next_power_of_two` enforcement | mask-based modulo (`& (cap - 1)`) | Non-power-of-2 capacity would silently corrupt the index mapping. |
| Atomic ordering on Bottom (Relaxed) + Top (SeqCst) | Chase-Lev correctness | Drift from the published memory-ordering contract (`docs/detailed/10-concurrency-model.md`) reintroduces the famous Chase-Lev memory-ordering bug. |
| `os_alloc(layout.size(), layout.align())` | direct page allocator (not the higher-level TieredAllocator) | Bypasses CBGR for the ring buffer; the buffer itself is `&unsafe mut T`. |

## 3. Language-implementation gaps

### §A — `StealResult.into_option` discards the `Retry` signal

Source contract: `Empty` and `Retry` both map to `Maybe.None`.  The
caller can no longer distinguish "queue is permanently empty" from
"transient race; retry me".  Recommend: introduce `into_result()
-> Result<T, StealResult>` that preserves the variant.

### §B — `RingBuffer.new` panics on os_alloc null

`task_queue.vr:167-169` panics on null pointer from `os_alloc`.
This is correct for catastrophic OOM during pool init but the
panic message includes layout details that may not survive
unstripped builds.  Sound; pinned as informational.

### §C — `grow` allocates new buffer without freeing old

`RingBuffer.grow` returns a new buffer of doubled capacity but
the caller is responsible for freeing the old via `os_free`.
Lifetime ownership is not enforced in the type — easy to leak
the old buffer.  Recommend: `grow(&mut self)` with explicit
old-buffer free.

### §D — `BoundedQueue<T>` lacks an Eq/Display surface

Cross-tier comparison via `==` is not implemented.  For
queue-level test invariants ("two queues hold the same items in
the same order") an Eq impl would be useful.  Audit-only.

### §E — No CBGR tracking on the ring-buffer pointer

`ptr: &unsafe mut T` bypasses CBGR.  This is correct for the
zero-allocation hot path (CBGR check on every read/write would
cost ~15ns Tier0 — that's > 50% of the per-op budget) but the
docstring should call out that the deque opts out.

## Action items landed in this branch

* `core-tests/runtime/task_queue/unit_test.vr` — 13 unit tests
  covering StealResult<T> 3-variant + 3 helper predicates.
* `core-tests/runtime/task_queue/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §A `into_result` overload preserving Retry signal | `core/runtime/task_queue.vr` | 30 min |
| §B Defence-in-depth os_alloc null vs panic disambiguation | `core/runtime/task_queue.vr` | 1 h |
| §C `grow(&mut self)` with explicit old-buffer free | `core/runtime/task_queue.vr` | 1 h |
| §D Eq impl on BoundedQueue<T> | `core/runtime/task_queue.vr` | 1 h |
| §E CBGR-opt-out docstring | `core/runtime/task_queue.vr` | 15 min |
| Live push/pop/steal round-trip + monotonic-bottom invariant | `vcs/specs/L2-standard/runtime/task_queue/` | gated on atomic intrinsics |
| Chase-Lev memory-ordering correctness regression (multi-thread) | sister | gated on real multi-thread harness |
| Fairness property test (round-robin steal distribution) | sister | gated on the same |
