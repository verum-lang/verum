# `sync/semaphore` audit

Module: `core/sync/semaphore.vr` (264 LOC) — `Semaphore` (counting
semaphore with futex-based blocking acquire), `SemaphoreGuard`
(RAII guard releasing permits on drop).

The semaphore is a 3-AtomicInt record:
* `permits: AtomicInt` — currently-available permit count.
* `max_permits: AtomicInt` — capacity (mutable via `add_permits` /
  `forget_permit`).
* `waiters: AtomicInt` — blocked-acquirer count, used to gate the
  `futex_wake` syscall.

The release-ordering rationale on `waiters.fetch_add` (avoid missed
wakeup when notifier reads `waiters == 0` before waiter's
registration becomes visible) is pinned in the doc-comment at
`semaphore.vr:78-87`.  Any drift to `Relaxed` on that fetch_add
silently re-opens the missed-wakeup race.

Tests focus on the static + single-threaded uncontended surface:
constructor, binary helper, try_acquire / release on a sequential
permit pool, available_permits / max_permits accessors, capacity
growth (`add_permits`) and shrink (`forget_permit`).  Live blocking
acquire + futex wake-up exercised at the L2-spec level.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.runtime.rate_limiter` | Bounded-parallelism gate at task-spawn sites |
| `core.io.fs.bounded_writer` | Limits concurrent open-FD count for parallel writers |
| `core.net.connection_pool`  | Pool-size cap for outbound HTTP / TCP connections |
| `core.cache.cooperative`    | Permit-token-bucket for cache-line refresh coalescing |
| `core.database.pool`        | Per-pool max-concurrent-statement gate |

## 2. Crate-side hardcodes

* `sys.{linux,darwin,windows}.thread.{futex_wait, futex_wake}` —
  same per-platform OS-blocking primitives as `mutex` / `rwlock` /
  `condvar`.  Drift in the AtomicInt layout (currently
  `{ value: Int }`) breaks the `&self.permits.value` pointer passed
  to `futex_wait`.
* `assert(permits >= 0, ...)` in `Semaphore.new` — guards against
  negative-permit init.  Hard-coded message; pin in
  `regression_test.vr` indirectly via the construction surface.

## 3. Language-implementation gaps

### §3.1 Live acquire/release on contention requires multi-threaded harness

Cannot be tested at the data-shape level.  See
`vcs/specs/L2-standard/sync/semaphore/`.

### §3.2 `acquire_guard` / `acquire_many_guard` return SemaphoreGuard

These guards release permits on drop.  We cannot exercise the drop-
release without a multi-step ownership transition that the current
interpreter test harness doesn't sequence reliably for `&Semaphore`-
borrowing guards.  Construction-only pinned in `unit_test.vr` §3.

### §3.3 `add_permits` increases capacity AND availability simultaneously

`semaphore.vr:194-205` — `add_permits(n)` increments BOTH
`max_permits` AND `permits` by n, then wakes up to n waiters.
The dual-bump invariant (capacity follows availability when growing)
is documented as the contract; pin in `property_test.vr` §C.

### §3.4 `forget_permit` decreases capacity permanently

`semaphore.vr:208-211` — only decrements `max_permits`, NOT
`permits`.  This breaks the invariant `permits <= max_permits`
temporarily until the forgotten permit is returned.  Pinned as a
"caller-must-have-acquired-first" contract — the operation is
intentionally asymmetric.  LOCK-IN in `regression_test.vr` §3.4.

### §3.5 `binary()` helper is `new(1)`

Semantic alias for mutex-like usage.  Pinned in `unit_test.vr` §1.

### §3.6 `available_permits` clamps to non-negative

`semaphore.vr:184-186` — `permits.load(Relaxed).max(0)`.  The clamp
exists because `try_acquire_many` can transiently underflow
`permits` between the load and CAS in the contended path.  The
clamp at the observation site hides that transient.  Pin in
`property_test.vr` §B.

## Action items landed in this branch

* `core-tests/sync/semaphore/unit_test.vr` — `@test`s for
  constructor surface, binary helper, available_permits /
  max_permits accessors, try_acquire / release single-threaded,
  add_permits capacity growth.
* `core-tests/sync/semaphore/property_test.vr` — algebraic laws:
  fresh-permits == requested, available_permits clamps non-negative,
  add_permits is additive, binary() ≡ new(1).
* `core-tests/sync/semaphore/regression_test.vr` — LOCK-IN pins for
  §3.4 (forget_permit asymmetry) + §3.3 (add_permits dual-bump
  invariant).
* `core-tests/sync/semaphore/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Live contention + missed-wakeup race verification | `vcs/specs/L2-standard/sync/semaphore/` | 1 day |
| SemaphoreGuard drop-release end-to-end | language-level harness | 30 min |
| `assert(permits >= 0)` ctor-message pin | trivial regression | 15 min |
