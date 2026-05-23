# `core.async.executor` — audit

The executor is the **production async runtime**: spawn, block_on,
work-stealing scheduler, timer driver, I/O driver bridge over
io_uring/kqueue/IOCP via `sys.io_engine`, context propagation via
`sys.common.ctx_*`, manifest-driven worker-thread / stack-size knobs
through `verum_get_runtime_*` extern globals.

## 1. Cross-stdlib usage

| consumer | role |
|---|---|
| `core.async.task` | `Task` + `JoinHandle` types — every spawned task surfaces through executor |
| `core.async.future.Future` | the protocol the executor drives |
| `core.async.poll.Poll` | the wakeup-aware return shape every poll cycle observes |
| `core.async.waker.Waker` / `Context` | wake-by-ref source the executor injects per polling cycle |
| `core.async.semaphore.AsyncSemaphore` | acquire path hooks into the executor's blocked-task list |
| `core.async.timer` | timer driver lives inside the runtime; `sleep_ms`/`interval` route through it |
| `core.async.nursery` / `core.async.broadcast` | structured-spawn + multi-consumer fan-out built on `spawn`/`spawn_with_env` intrinsics |
| `core.net.tcp` / `core.net.unix` | every TCP/UDP/Unix socket request defers to the executor's I/O driver |
| `core.sync.*` | atomic + mutex primitives used by executor internals (worker queue, wait counter) |

## 2. Crate-side hardcodes / drift surfaces

| site | drift surface | risk |
|---|---|---|
| `verum_get_runtime_async_worker_threads` extern | manifest bridge global (#261) | LOW — emitted by `platform_ir.rs::emit_runtime_globals` from Verum.toml `[runtime].async_worker_threads`; LTO-folds to constant |
| `verum_get_runtime_task_stack_size` extern | same manifest-bridge pattern | LOW |
| `DEFAULT_SQ_ENTRIES` / `WAKE_TOKEN` from `sys.io_engine` | re-exported constants | LOW — sourced from `sys` layer |
| `sys.common.MAX_CONTEXT_SLOTS` | ContextSlots array capacity | LOW — single source of truth |
| `SLOT_RUNTIME = 0` / `SLOT_CURRENT_TASK = 1` | TLS slot assignments | MEDIUM — drift between executor.vr and consumers in `intrinsics.vr::Executor.current` (slot 0 read) MUST stay aligned |
| `verum_thread_spawn_multi` FFI signature | task #19 (1-arg shape) | CLOSED |

## 3. Language-implementation gaps surfaced by this suite

### §A — Live executor invocation gated by upstream tasks (DEFERRED)

The executor's `block_on`/`spawn`/`run` paths require:
* Task #10 — LLVM `generate_native` SmallVector SIGBUS at IntervalMap
  insert.  Affects every AOT build of executor-touching code.
  Pinned by `vcs/aot_skip_waivers.toml`.
* Task #12 — `AsyncSemaphore.new` null-derefs through `AtomicInt.swap`
  in the Mutex/AtomicBool init chain.  Blocks 9 of 11 semaphore
  lifecycle tests; transitively blocks any executor scenario that
  acquires the worker-pool semaphore.

This suite therefore pins the **non-runtime configuration surface**:
builder construction, fluent chaining, config presets, type aliases.
Live runtime tests stay in `tests/` (project-level) where they can
be exercised conditionally once the upstream defects close.

### §B — `RuntimeBuilder.<setter>` returns `mut self` requires consume semantics

Each `.max_tasks(n)`, `.stack_size(s)`, `.enable_*()` setter takes
`mut self` and returns `RuntimeBuilder`.  This is the fluent-builder
idiom — chain composition stays clean.  The interpreter dispatches
each setter call as a normal method call; the `mut self` receiver
is moved into the call frame and the modified builder is returned.

Pinned in `unit_test.vr::test_executor_runtime_builder_full_chain` —
verifies the 5-setter chain composes without panic.  Property test
section A pins commutativity over independent knobs.

### §C — Manifest-bridge `extern fn verum_get_runtime_*` shape

The two extern functions `verum_get_runtime_async_worker_threads` /
`verum_get_runtime_task_stack_size` are emitted with internal linkage
in LLVM-AOT and folded by LTO; under the Tier-0 interpreter they
return 0 (the "auto-detect" path), so the stdlib fall-back to the
hardcoded defaults (1 MiB stack, work-stealing on, etc.) kicks in.

Both interpreter and AOT paths produce a valid `AsyncRuntimeConfig`
— this suite verifies that branch survives.  Manifest-override
behaviour itself is tested at the build-system level (Verum.toml
parsing → emit_runtime_globals → LLVM IR), not here.

## Action items landed in this branch

- `unit_test.vr` expanded from 2 → 16 tests:
  - `RuntimeBuilder.new()` independent constructions
  - `AsyncRuntimeConfig.default()` / `cpu_bound()` / `io_bound()`
    presets
  - `RuntimeConfig` short-alias surface
  - Every public builder setter pinned individually + composed
  - Every `AsyncRuntimeConfig` fluent setter pinned
- New `property_test.vr` with 6 algebraic laws (commutativity over
  independent knobs / construction independence / N-fold associative
  composition / preset non-interference / idempotent default).
- New `integration_test.vr` with 3 cross-stdlib scenarios
  (`List<RuntimeBuilder>` / `List<AsyncRuntimeConfig>` /
  `RuntimeConfig` alias at function boundary).
- New `audit.md` (this file).

## Action items deferred

| § | scope | tracking | est. |
|---|---|---|---|
| §A.1 | live `block_on(future)` round-trip | task #10 | multi-day (LLVM SmallVector) |
| §A.2 | `AsyncSemaphore.new` chain (executor worker-pool semaphore) | task #12 | 1-2d |
| §B | `mut self` consume-and-return idiom audit across stdlib | upstream | gated on grammar profile review |
| §C | manifest-bridge end-to-end test from Verum.toml | upstream | build-system layer |
