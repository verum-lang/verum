# `core/async/semaphore.vr` — audit

Async-task counting semaphore — the cooperative-yield "at most N
concurrent operations" primitive.  Unlike `core.sync.semaphore`, this
one parks waiters via `Waker` clones in a `Deque` rather than blocking
the OS thread.

## Public API surface

| Name | Shape | Status under interpreter |
|---|---|---|
| `SemaphoreError = Closed` | error variant | green (3 unit, 2 property, 2 integration) outside the `is`-operator (task #13) |
| `AsyncSemaphore` (record `{ inner: Shared<Mutex<SemaphoreInner>> }`) | counting semaphore | **blocked by task #12** — construction null-derefs in AtomicInt.swap during Mutex init |
| `AsyncSemaphore.new(permits: Int) -> AsyncSemaphore` | factory; clamps negative to 0 | blocked by #12 |
| `try_acquire() -> Maybe<SemaphorePermit>` | non-blocking acquire | blocked by #12 |
| `acquire().await -> Result<SemaphorePermit, SemaphoreError>` | async acquire (waker-parked) | blocked by #12 + needs executor |
| `add_permits(n: Int)` | runtime capacity widening | blocked by #12 |
| `close()` | flip closed flag, wake all waiters | blocked by #12 |
| `available_permits() -> Int` | metric-only count | blocked by #12 |
| `SemaphorePermit` (Drop releases permit) | RAII guard | blocked by #12 |

## Cross-stdlib usage

* `core.sync.{Shared, Mutex}` — wrapped at construction.  The
  AtomicInt.swap defect lives in the Mutex/AtomicBool init path.
* `core.async.waker.Waker` — waker clones are parked on
  SemaphoreInner.waiters.
* `core.collections.deque.Deque` — FIFO waiter queue.

## Crate-side hardcodes

None observed.  The semaphore is pure stdlib code at the language
level.

## Language-implementation gaps

1. **Task #12 — AsyncSemaphore.new null-derefs through AtomicInt.swap.**
   13/14 lifecycle tests pinned in `regression_test.vr §A`.
   Defect class: VBC interpreter atomic-primitive dispatch on a
   freshly-allocated atomic cell inside Mutex.
2. **Task #13 — `is`-operator returns false on a single-variant sum.**
   `let e = SemaphoreError.Closed; e is SemaphoreError.Closed` ⇒ false,
   yet match-arm + Eq.eq route correctly.  1 test pinned in
   `regression_test.vr §B`.  Likely shares root with the task #22
   variant-tag stability cluster for the degenerate single-variant
   shape.
3. **Task #10 — AOT generate_native SIGABRT.** Inherits the global
   AOT blocker.

## Action items landed in this branch

* Created `core-tests/async/semaphore/{unit,property,integration,regression}_test.vr,audit.md`.
* 7 tests pass under interpreter; 10 `@ignore` regression pins for
  tasks #12 + #13.
* Pinned the SemaphoreError single-variant algebra (match-arm
  routing + Eq.eq reflexivity + Result+Maybe wrapping + variant
  partition by manual match classifier).

## Action items deferred

* Tasks #12 + #13 + #10.
* The full SemaphoreFuture / Future-driven acquire-with-waker path
  needs the executor test-bed once #12 unblocks construction.
