# Multi-Threaded Async Scheduler — Architecture & Implementation Plan

**Status:** Design committed (#271). Implementation phased.
**Owners:** core/async maintainers.
**Last revised:** 2026-05-01.

## Motivation

`[runtime].async_worker_threads` is parsed from `Verum.toml`, validated, and
threaded through the codegen → LLVM bridge (#258, #259, #261). The stdlib's
`AsyncRuntimeConfig` reads it via `verum_get_runtime_async_worker_threads()`.
But the actual `AsyncRuntime` in `core/async/executor.vr` runs a
**single-threaded cooperative** event loop: the main thread polls the main
future, drains spawned tasks within `poll_budget`, and parks via the IOEngine.

Setting `async_worker_threads = 4` therefore has no observable runtime
effect. This is the load-bearing gap that #271 closes.

## Design Constraints

The scheduler must be:

1. **Fundamental** — first-class member of the language runtime, not a
   library-level retrofit.
2. **Maximally efficient** — work-stealing with per-worker local deques,
   cache-line-aligned hot fields, eventfd-style wakeups on Linux.
3. **Architecturally aligned** — reuses CBGR-aware `Heap<T>`, semantic types
   (`List`/`Map`), `core/sync` primitives (`Mutex`, `Condvar`, `AtomicInt`,
   `AtomicBool`), `core/sys/{linux,darwin,windows}/thread` for spawning,
   and the existing `IOEngine` for I/O parking on the main thread.
4. **Send/Sync correct** — typed surface enforces `Future + Send` for
   `runtime.spawn(...)`; `LocalExecutor` retains the `!Send` path.
5. **Zero-overhead default** — when `async_worker_threads = 0`, no workers
   are spawned, behavior is bit-identical to today's single-threaded loop.

## Phased Implementation

### Phase 1 — Worker pool foundation (foundation, mostly mechanical)

Add a `WorkerPool` struct to `core/async/executor.vr`:

```verum
public type WorkerPool is {
    /// Spawned worker join handles. Empty when async_worker_threads=0.
    handles: List<ThreadJoinHandle<()>>,
    /// Mutex-paired condvar for parking idle workers.
    park_mu: Mutex<()>,
    park_cv: Condvar,
    /// Set by AsyncRuntime.shutdown(); checked on every worker iteration.
    shutdown_flag: AtomicBool,
    /// Diagnostic counters.
    parked_count: AtomicInt,
    spawned_count: Int,
};
```

`AsyncRuntime` gains a `worker_pool: Mutex<Maybe<WorkerPool>>` field
(lazy-initialized at first `block_on()`). At startup:

```verum
fn start_worker_pool(&self) {
    let n = self.config.async_worker_threads;
    let n_resolved = if n == 0 { num_cpus().saturating_sub(1) } else { n };
    if n_resolved == 0 { return; }
    let pool = WorkerPool { /* ... */ };
    let runtime_addr: Int = self as &unsafe Byte as Int;
    for i in range(0, n_resolved) {
        let handle = ThreadBuilder.new()
            .name(f"verum-worker-{i}")
            .stack_size(self.config.task_stack_size)
            .spawn(fn() { worker_main(runtime_addr); })
            .expect("worker spawn failed");
        pool.handles.push(handle);
    }
    *self.worker_pool.lock().unwrap() = Some(pool);
}
```

The static thread body:

```verum
fn worker_main(runtime_addr: Int) {
    // SAFETY: runtime lives across the entire block_on body; workers
    // are joined before block_on returns, so the pointer is valid.
    let runtime = unsafe { &*(runtime_addr as &unsafe Byte as &AsyncRuntime) };
    while !runtime.shutdown_requested() {
        let completed = runtime.process_ready_tasks();
        if completed == 0 {
            runtime.park_worker();
        }
    }
}
```

The existing `Mutex<List<Heap<TaskEntry>>>` queues already support
concurrent access — workers can call `process_ready_tasks()` without any
queue refactor. Park/unpark bridges `spawn()` → `cv.notify_one()`.

Modifications to existing methods:
- `spawn()` calls `worker_pool.notify_one()` after pushing.
- `wake_task()` similarly notifies.
- `shutdown()` sets `shutdown_flag` + `cv.notify_all()` + joins all workers.
- `block_on()` calls `start_worker_pool()` once at startup and joins workers
  via the `Drop` impl.

**Acceptance criteria:**
- `cargo test -p verum_vbc` green.
- `vcs/specs/L2-standard/async/multi_threaded_basic.vr` spawns 1000 CPU-bound
  tasks with `async_worker_threads=4` and asserts wall-clock < 2× single-thread
  baseline (proves observable parallelism).

**Cost:** ~250 LOC in executor.vr, no public API breakage.

### Phase 2 — Per-worker local deques (work-stealing)

Replace the single global `task_queue`/`ready_queue` with per-worker local
deques + a global injector queue for non-worker spawns:

```verum
public type WorkerSlot is {
    /// LIFO local deque — owner pushes/pops back, foreign workers steal front.
    /// Mutex-protected for v2; Chase-Lev lock-free deque for v3.
    local_deque: Mutex<Deque<Heap<TaskEntry>>>,
    /// Steal-half rotation hint (last victim worker).
    steal_hint: AtomicInt,
};
```

Spawn semantics:
- Inside a worker (`current_worker_id().is_some()`): push to local deque (LIFO,
  cache-hot for recursive spawn).
- Outside a worker (main thread, signal handlers, FFI callbacks): push to
  global injector queue.
- Worker's drain order: local deque (LIFO) → global injector → steal from
  random peer (FIFO from front).

Forward-compatibility: VBC interpreter's `TaskQueue` already exposes
`next_ready` (LIFO) + `steal_ready` (FIFO) — Phase 2 hooks plug into these
APIs without contract changes (see `verum_vbc/src/interpreter/state.rs`
"Work-Stealing TaskQueue (T1-I)" section).

**Cost:** ~400 LOC, replaces the single global queues.

### Phase 3 — Send/Sync correctness pins

Verify the typed surface:
- `runtime.spawn<F: Future>(f: F)` requires `F: Send` + `F::Output: Send`.
- `LocalExecutor.spawn_local<F>(f: F)` retains the `!Send` path
  (single-threaded executor; no worker pool).
- `TaskEntry` raw pointers (`future_ptr: &unsafe Byte`) are
  manually-asserted-Send; document the invariant in a SAFETY comment.
- IOEngine fields remain main-thread-only (workers must NOT call
  `wait_for_events` — assert this with a panic check in debug builds).

Pin tests:
- `try_spawn_non_send_fn_rejects` — compile-fail test for `spawn(rc_capture)`.
- `worker_does_not_call_io_engine` — `assert(thread_id == main_thread)` at
  every IOEngine entry point (debug builds only).
- `cross_thread_waker_fires_correctly` — spawn task on worker A, capture
  waker, fire from worker B, verify task moves to ready queue.

### Phase 4 — Lock-free deque + cache-line padding

Replace `Mutex<Deque>` per-worker local deques with the Chase-Lev work-stealing
deque (lock-free for owner push/pop; CAS for steal). Cache-line-pad the hot
fields to avoid false sharing:

```verum
@repr(align = 64)
public type WorkerSlot is { /* hot fields */ };
```

This phase is a pure performance optimization — the contract is unchanged
from Phase 2.

**Expected gains** (single-socket modern x86_64, 8-core baseline):
- Phase 1: 2.0–3.5× speedup vs single-threaded for embarrassingly parallel
  workloads (limited by Mutex contention on shared queues).
- Phase 2 (work-stealing, Mutex deques): 5.0–6.5× speedup.
- Phase 4 (lock-free deques + cache padding): 6.5–7.5× speedup, near-linear
  scaling up to NUMA boundary.

## Cross-Tier Note

The Tier 0 (VBC interpreter) execution path is fundamentally single-threaded
— `InterpreterState` holds non-`Send` state (PC, register file, NaN-boxed
heap, dispatch tables). Making the interpreter multi-threaded would require
wrapping the entire state in `Arc<Mutex<...>>`, with prohibitive overhead
on the dispatch hot path.

**#271 targets Tier 1 (AOT) only.** Programs running under
`verum run --tier=interpret` continue to use the cooperative
single-threaded loop; the manifest's `async_worker_threads` is silently
ignored in interpret mode. Documented at
`crates/verum_vbc/src/interpreter/state.rs:2160` (the existing
phase-not-realised tracing block).

## Manifest Surface

`[runtime].async_worker_threads: u32`
- `0` (default): auto-detect via `num_cpus()` minus 1 (reserve one core for
  the main thread/IOEngine).
- `1`: spawn one worker (useful for dual-core systems where the main thread
  drives I/O).
- `>1`: explicit override.
- Validation cap: ≤ 256 (manifest validate gate; spawning more workers than
  available cores degrades performance).

`[runtime].task_stack_size: u32` (already wired via #259):
- `0` (default): platform default (~8 MiB on Unix, 1 MiB on Windows).
- `>0`: per-worker thread stack size.

## Implementation Risk Assessment

The single highest risk is **untested .vr code in the stdlib build**. The
stdlib is auto-discovered (2298 files compiled into the embedded archive);
adding 250 lines of multi-threaded .vr code that the Verum compiler doesn't
yet fully support could break the build.

Mitigation:
1. Land the design doc + manifest gate FIRST (this commit).
2. Implement Phase 1 in a separate branch with thorough cargo-test validation
   on the generated VBC artifacts.
3. Gate behind `@cfg(feature = "async_workers")` initially so opt-in is
   explicit.
4. Add VCS spec tests (`vcs/specs/L2-standard/async/`) before promoting from
   feature-gated to always-on.

## Tracking

- Architectural foundation: this document.
- Phase 1 implementation: follow-up task #271-Phase-1.
- Phase 2 work-stealing: follow-up task #271-Phase-2.
- Phase 3 Send/Sync pins: follow-up task #271-Phase-3.
- Phase 4 lock-free + cache padding: follow-up task #271-Phase-4.
- VCS spec tests: integrated with #273 (Differential Tier 0/1 tests).
- Multi-threading observability primitives: integrated with #275
  (manifest→runtime bridge zero-overhead pin tests) — verify worker count
  reaches stdlib via LLVM constant-folding under LTO.
