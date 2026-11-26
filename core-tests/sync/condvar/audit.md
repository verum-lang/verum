# `sync/condvar` audit

Module: `core/sync/condvar.vr` — `Condvar` (condition
variable backed by a futex-monitored sequence counter),
`WaitTimeoutResult` (data-only result wrapping a `timed_out: Bool`),
`CondvarNotifyGuard` (RAII guard that notifies on drop),
`producer_consumer_pair<T>` (factory returning a linked
`(Mutex<T>, Condvar)`).

Tests focus on the data-shape and static surface:
* `Condvar.new()` constructor + `Default` impl.
* `Condvar.waiter_count()` accessor on a quiescent condvar (== 0).
* `Condvar.notify_one()` / `notify_all()` on a no-waiter condvar
  (increments the sequence and issues a wake independently of the waiter count).
* `WaitTimeoutResult` data-record construction + `timed_out()` accessor.
* `CondvarNotifyGuard.notify_one_guard` / `notify_all_guard` constructors.
* `producer_consumer_pair(initial)` factory returns a 2-tuple of
  `Mutex<T>` + `Condvar`.

The [public deadline fixture](../../../vcs/specs/L2-standard/async/synchronization/condvar_deadline.vr)
uses the actual mutex and condition variable. It covers the ready predicate,
two timed waits separated by a broadcast, and reacquisition of a poisoned
guard after an immediate timeout. Its source passes ordinary `verum check`.
The development CLI requires an embedded standard-library archive for execution.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.sync.barrier`            | `Barrier.condvar: Condvar` + `Phaser.condvar` + `CountDownLatch.condvar` |
| `core.collections.channel`     | Bounded MPMC channel uses `Condvar` for not-empty / not-full signalling |
| `core.runtime.task_pool`       | Worker-thread sleep-wake on incoming task |
| `core.io.async_runtime`        | Reactor wake on submitted-task availability |

## 2. Wait protocol

`Condvar` snapshots an `AtomicU32` sequence while holding the predicate mutex.
The consuming `MutexGuard.unlock_for_wait` retains the original mutex reference
and releases the guard through its destructor. Conditional parking and waking
use the same 32-bit field as atomic load and fetch-add. Notifications increment
the sequence regardless of the diagnostic waiter count. `wait_while` rechecks
the predicate after acquiring the mutex again.

## 3. Execution coverage

### §3.1 Thread handoff

The public thread handoff is blocked at callback admission. With
`mount core.runtime.thread.{ThreadBuilder};` and a static
`PHASE: Mutex<Int>` and `CHANGED: Condvar`, ordinary `verum check` rejects
`ThreadBuilder.new().spawn(publish)` for this worker:

```verum
fn publish() -> Int {
    let mut guard = PHASE.lock().unwrap();
    *guard = 1;
    drop(guard);
    CHANGED.notify_one();
    17
}
```

The diagnostic is E400. The required callable excludes `Async`, while the
worker's observed properties are unknown. Assigning a function that returns
a constant to `fn() -> Int` succeeds. Introducing only `PHASE.lock()` or only
`CHANGED.notify_one()` into that function reproduces the rejection.

### §3.2 Interpreter wake registration

The interpreter routes the futex operations through
`crates/verum_vbc/src/interpreter/dispatch_table/handlers/sync_extended.rs`.
The underlying `futex_park` in `ffi_extended.rs` checks for a wake token before
parking, while the notification side updates tokens and signals without
holding the parking mutex. A wake can land between that check and the park.

An independent execution of the actual parking module on an unchanged
`AtomicI32` initialized to 7 produced the following results. Both waits used
expected value 7 and a timeout of 2,000,000 ns.

| Operation | Result | Expected |
|---|---:|---:|
| First wait | -110 | -110 |
| Wake with count 2147483647 after the only waiter timed out | 2147483647 | 0 |
| Second wait | 0 | -110 |

This exercises the Rust parking implementation. The public handoff fixture
requires the compiler, archive, VBC dispatch and guard cleanup in addition.

### §3.3 `producer_consumer_pair` returns `(Mutex<T>, Condvar)`

Factory function exposes the canonical producer/consumer wiring.
Pin in `unit_test.vr` §5.

### §3.4 `WaitTimeoutResult` carries `timed_out: Bool`

Data-record with single accessor `timed_out() -> Bool`.  Construction
+ accessor pinned in `unit_test.vr` §4.

### §3.6 Native operations

The native `emit_futex_ir` implementation in
`crates/verum_codegen/src/llvm/platform_ir.rs` emits Darwin ulock calls
without consulting the module's target triple. Native guard destruction
and target-specific comparison, wake and timeout behavior require
independent execution.

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
| Live wait + notify race tests | Thread callback admission, a matching archive and the backend boundaries above | Unmeasured |
| CondvarNotifyGuard drop-notification observability | language-level harness | 30 min |
