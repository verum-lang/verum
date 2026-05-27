# `sync/mutex` audit

Module: `core/sync/mutex.vr` (296 LOC) —
`Mutex<T>` (futex-backed mutual-exclusion lock with explicit
`poison()` / `clear_poison()` advisory protocol; **not** auto-poisoned
on panic — see §3.2), `MutexGuard<T>` (RAII guard with
`Deref`/`DerefMut`), `LockResult<T>` / `TryLockResult<T>` aliases,
`PoisonError<T>` (returned when a previously-poisoned mutex is
acquired), and `TryLockError<T>` 2-variant sum
(`WouldBlock` / `Poisoned(PoisonError<T>)`).

Tests focus on the data-shape and single-threaded-uncontended
surface that the interpreter can exercise without a concurrent
harness:

* `Mutex.new(T) -> Mutex<T>` — constructor lays down the underlying
  futex + boxed data + zeroed AtomicBool poison flag without
  contention.
* `Mutex.is_poisoned()` — Acquire-load on the flag; default `false`
  after `new(...)`.
* `Mutex.poison()` / `Mutex.clear_poison()` — Release-stores that
  flip the flag back and forth without touching the lock itself.
* `Mutex.is_locked()` — futex-side `is_locked()` predicate; `false`
  immediately after `new(...)` with no acquirers.
* `TryLockError<T>` variant construction + disjointness for both
  `WouldBlock` and `Poisoned(PoisonError<T>)`.
* `PoisonError<T>` constructor + `get_ref` / `into_inner` accessors
  on the type-parameterised inner-value field.

Live `lock()` / `try_lock()` blocking semantics + contention
behaviour + `MutexGuard` `Deref`/`DerefMut` over the protected data
+ `Drop` releasing the futex require an actually-running concurrent
harness — tested at the language level in
`vcs/specs/L2-standard/sync/mutex/`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.sync.barrier`        | `Barrier.state: Mutex<BarrierState>` + `CountDownLatch.latch_lock: Mutex<LatchState>` |
| `core.sync.condvar`        | `Condvar.wait(MutexGuard<T>)` borrows the wrapped mutex pointer through the guard |
| `core.runtime.global`      | Global runtime context bootstrap holds a `Mutex<RuntimeState>` |
| `core.collections.channel` | Bounded MPMC channels back the slot-array with a `Mutex` |
| `core.cache.lru`           | LRU's hot-list invariant is guarded by `Mutex<LruState>` |

## 2. Crate-side hardcodes

* `FutexLock` referenced by `Mutex.futex` is a stdlib-side wrapper
  that compiles down to the per-platform futex intrinsic
  (`__futex_wait_raw` / `__futex_wake_raw`) — see
  `core/sys/{linux,darwin,windows}/thread.vr`. The wrapper's
  record-layout MUST stay 1-field (`state: AtomicInt`) so the
  `Mutex { futex, data, poisoned }` record stays at a stable
  3-field layout. Codegen consumes this layout for `&self.data`
  field-access at the `data_mut` site (`mutex.vr:201`).
* `MutexGuard.drop` calls `self.mutex.futex.unlock()` — the
  receiver-type-narrowing at codegen MUST resolve through
  the field-borrow chain `&self.mutex -> &Mutex<T> -> &FutexLock`,
  not the global-suffix-search fallback. Drift here was the cause
  of the `core.io` round-7 method-shadow class — pinned in audit
  as the architectural-rule on namespace-qualified suffix-scan.

## 3. Language-implementation gaps

### §3.1 Live lock / try_lock require multi-threaded harness

Cannot be tested at the data-shape level. The futex wait/wake
machinery is exercised at `vcs/specs/L2-standard/sync/mutex/`.

### §3.2 Poisoning is *advisory*, not panic-driven

Documented in the doc-comment at `mutex.vr:91-104`: Verum does not
yet expose a "currently unwinding a panic" predicate, so
`MutexGuard.drop` cannot auto-poison. The poison flag is only set
by explicit `poison()` calls before `panic(...)` sites. The
testable surface here is the `poison()` / `clear_poison()` /
`is_poisoned()` round-trip, which IS exercised in `unit_test.vr`
§4 and `property_test.vr` §B (idempotence + round-trip laws).

### §3.3 `Default<T>` for `Mutex<T>` substitutes literal `0`

`mutex.vr:163-168` — the `Default<T>` impl falls back to
`Mutex.new(0)` per task #17 workaround instead of `Mutex.new(T.default())`.
Closes when task #17 (cross-module generic-instantiation default
propagation) lands. Pinned in `regression_test.vr` as a LOCK on
the current behaviour so the fix is a deliberate flip, not a
silent regression.

### §3.4 `MutexGuard.data` / `data_mut` are package-private

`mutex.vr:196-203` — the accessors are not `public`. Library
consumers go through `Deref`/`DerefMut`. The `unsafe` cast in
`data_mut` (`&*const T as *mut T`) is sound only because the
calling site holds the lock, which is enforced by `MutexGuard`
being the only way to construct a `&mut` to the data. Pinned
implicitly by the `Deref<Target=T>` impl + the fact that
`Mutex.data` is private.

### §3.5 `TryLockError` is two-variant; `WouldBlock` carries no payload

The `WouldBlock` variant is **payload-free** while `Poisoned` carries
a `PoisonError<T>`. Codegen for the variant tag must distinguish
the two — pinned in `unit_test.vr` §6 + `property_test.vr` §C.

## Action items landed in this branch

* `core-tests/sync/mutex/unit_test.vr` — `@test`s covering
  Mutex.new + poison/clear_poison/is_poisoned + is_locked +
  TryLockError variants + PoisonError construction + accessors +
  LockResult / TryLockResult result-type round-trips.
* `core-tests/sync/mutex/property_test.vr` — algebraic laws:
  poison-clear idempotence, double-poison absorbs, fresh-mutex
  pristine.
* `core-tests/sync/mutex/regression_test.vr` — LOCK-IN pins for
  §3.3 (Default literal-0 substitution) + §3.5 (variant
  disjointness invariant).
* `core-tests/sync/mutex/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Live lock/unlock + contention tests | `vcs/specs/L2-standard/sync/mutex/` | 1 day |
| Auto-poisoning on panic (Rust-style) | language-level: thread-panicking predicate + codegen guard insertion | multi-day |
| `Default<T>` flip from `Mutex.new(0)` → `Mutex.new(T.default())` | gated on task #17 close | 30 min once unblocked |
| `MutexGuard.Deref`/`DerefMut` runtime verification | `vcs/specs/L2-standard/sync/mutex/` | language-level |
