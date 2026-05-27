# `sync/waitgroup` audit

Module: `core/sync/waitgroup.vr` (110 LOC) — `WaitGroup` (Go-style
counter-based task-completion barrier).  Wraps a runtime-side
handle via 6 raw intrinsics defined in
`core/intrinsics/runtime/sync.vr`:

* `__waitgroup_new_raw()` → handle
* `__waitgroup_add_raw(handle, delta)`
* `__waitgroup_done_raw(handle)`
* `__waitgroup_wait_raw(handle)` — blocks until counter == 0
* `__waitgroup_try_wait_raw(handle)` → 0/1 (1 = counter already 0)
* `__waitgroup_destroy_raw(handle)` — RAII Drop calls this

Tests focus on the static + single-threaded "counter-at-zero"
surface: construction, `try_wait()` on a fresh / drained WaitGroup,
`add(0)` no-op, `add(N) + done()`×N drain, RAII Drop invocation
through scope-exit.

Live `wait()` blocking + multi-task `done()` propagation require a
multi-threaded harness — tested at the language level in
`vcs/specs/L2-standard/sync/waitgroup/`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.runtime.spawn`        | Spawn-group fan-in barrier |
| `core.io.async_runtime`     | Task-set completion gate before shutdown |
| `core.concurrency.tasks`    | TaskGroup primitive layers on WaitGroup |
| `core.async.executor`       | Drain-pending-tasks-at-shutdown contract |

## 2. Crate-side hardcodes

* `core/intrinsics/runtime/sync.vr` declares the 6 `__waitgroup_*_raw`
  intrinsics with `@intrinsic("waitgroup_*")` identity.  Codegen
  routes through `FunctionInfo.intrinsic_name`.  Drift between
  intrinsic name and codegen-side table silently breaks every
  WaitGroup operation.
* Runtime-side handle is opaque (`Int`); the runtime must keep the
  handle table consistent across `new_raw` / `destroy_raw` calls.

## 3. Language-implementation gaps

### §3.1 Live wait() requires multi-threaded harness

Cannot be tested at the single-threaded level — `wait()` on a
non-zero counter would deadlock.  Live tests are at
`vcs/specs/L2-standard/sync/waitgroup/`.

### §3.2 RAII Drop calls __waitgroup_destroy_raw

`waitgroup.vr:106-110` — `implement Drop for WaitGroup` calls
`__waitgroup_destroy_raw(self.handle)`.  Without this, every
short-lived task-orchestration WaitGroup accumulates a runtime-
side resource leak.  Pinned in `unit_test.vr` §3 by constructing
a WaitGroup in an inner scope and confirming subsequent
construction does not error (any handle-table leak would
eventually surface as exhaustion).

### §3.3 try_wait returns Bool by 0/1 mapping

`waitgroup.vr:82-85` — `__waitgroup_try_wait_raw` returns Int
`0` or `1`; the public method maps `!= 0` → `true`.  Pinned in
`regression_test.vr` so a future flip (e.g., `1 = "would block"`)
fails this test loudly.

### §3.4 add(0) is a no-op (zero-delta increment)

The `__waitgroup_add_raw(handle, 0)` call MUST not touch the
counter.  Pin via `try_wait` returning true after `new() + add(0)`.

## Action items landed in this branch

* `core-tests/sync/waitgroup/unit_test.vr` — construction +
  try_wait on fresh / drained / add(0) / add+done round-trip.
* `core-tests/sync/waitgroup/property_test.vr` — invariants:
  fresh-wait-group is "done", add(N) + done×N drains.
* `core-tests/sync/waitgroup/regression_test.vr` — LOCK-IN for
  §3.3 try_wait Bool-encoding + §3.4 add(0) no-op.
* `core-tests/sync/waitgroup/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Live wait() blocking + multi-task done() | `vcs/specs/L2-standard/sync/waitgroup/` | 1 day |
| Drop-leak detection via handle-table exhaustion probe | language-level | 30 min |
