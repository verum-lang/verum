# `sync/condvar` audit

Module: `core/sync/condvar.vr` (353 LOC) — `Condvar` (condition
variable backed by a futex-monitored sequence counter),
`WaitTimeoutResult` (data-only result wrapping a `timed_out: Bool`),
`CondvarNotifyGuard` (RAII guard that notifies on drop),
`producer_consumer_pair<T>` (factory returning a linked
`(Mutex<T>, Condvar)`).

Tests focus on the data-shape and static surface:
* `Condvar.new()` constructor + `Default` impl.
* `Condvar.waiter_count()` accessor on a quiescent condvar (== 0).
* `Condvar.notify_one()` / `notify_all()` on a no-waiter condvar
  (always increments `seq` + issues `futex_wake`; documented to NOT
  gate on `waiters > 0` per the missed-wakeup fix at `condvar.vr:217-230`).
* `WaitTimeoutResult` data-record construction + `timed_out()` accessor.
* `CondvarNotifyGuard.notify_one_guard` / `notify_all_guard` constructors.
* `producer_consumer_pair(initial)` factory returns a 2-tuple of
  `Mutex<T>` + `Condvar`.

Live `wait(MutexGuard<T>)` / `wait_timeout` / `wait_while` semantics
require a multi-threaded harness — tested at
`vcs/specs/L2-standard/sync/condvar/`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.sync.barrier`            | `Barrier.condvar: Condvar` + `Phaser.condvar` + `CountDownLatch.condvar` |
| `core.collections.channel`     | Bounded MPMC channel uses `Condvar` for not-empty / not-full signalling |
| `core.runtime.task_pool`       | Worker-thread sleep-wake on incoming task |
| `core.io.async_runtime`        | Reactor wake on submitted-task availability |

## 2. Crate-side hardcodes

* `sys.{linux,darwin,windows}.thread.{futex_wait, futex_wake}` —
  per-platform futex primitives.  `&self.seq.value` (AtomicInt's
  single `value: Int` field) is passed verbatim to the syscall.
  Drift in AtomicInt layout breaks every condvar wait/wake.
* The `seq.fetch_add(1, Release)` in `notify_one` / `notify_all` is
  unconditional (post-fix at `condvar.vr:217-230`) — it MUST stay
  unconditional or the missed-wakeup race surfaces in the
  notify-without-lock pattern.  Documented in the source-side
  doc-comment.

## 3. Language-implementation gaps

### §3.1 Live wait / wait_timeout / wait_while require multi-threaded harness

Cannot be tested at the data-shape level.  See
`vcs/specs/L2-standard/sync/condvar/`.

### §3.2 Notify-without-lock missed-wakeup fix is doc-only at single-thread level

The fix at `condvar.vr:201-241` removes the `if waiters > 0` gate
from `notify_one` / `notify_all`.  The single-threaded interpreter
cannot exercise the race that the fix closes; we pin the
architectural rule via:
* `unit_test.vr` §3 — notify_one / notify_all on a fresh condvar
  succeed (do NOT panic) even though there are no waiters.
* `regression_test.vr` — a future re-gating of `notify_*` on
  `waiters > 0` would silently break the live test (but not this
  single-threaded surface).  Pinned as a doc-comment-tied
  invariant.

### §3.3 `producer_consumer_pair` returns `(Mutex<T>, Condvar)`

Factory function exposes the canonical producer/consumer wiring.
Pin in `unit_test.vr` §5.

### §3.4 `WaitTimeoutResult` carries `timed_out: Bool`

Data-record with single accessor `timed_out() -> Bool`.  Construction
+ accessor pinned in `unit_test.vr` §4.

### §3.5 `CondvarNotifyGuard` is RAII over notification

Constructors `notify_one_guard(&Condvar)` / `notify_all_guard(&Condvar)`
return a guard whose `Drop` impl issues the matching notification.
Cannot be observed at the data-shape level — guard's `Drop` runs
on scope-exit and the notification is silent in absence of waiters.
Pin construction only.

## Action items landed in this branch

* `core-tests/sync/condvar/unit_test.vr` — Condvar.new + Default +
  waiter_count + notify_{one,all} on quiescent condvar +
  WaitTimeoutResult data-record + CondvarNotifyGuard ctor +
  producer_consumer_pair factory.
* `core-tests/sync/condvar/property_test.vr` — fresh-condvar
  invariant + notify_* idempotence-of-quiescence + WaitTimeoutResult
  round-trip.
* `core-tests/sync/condvar/regression_test.vr` — LOCK-IN for §3.2
  (notify_one / notify_all unconditional) + §3.4 (WaitTimeoutResult
  single-field shape).
* `core-tests/sync/condvar/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Live wait + notify race tests | `vcs/specs/L2-standard/sync/condvar/` | 1 day |
| CondvarNotifyGuard drop-notification observability | language-level harness | 30 min |
