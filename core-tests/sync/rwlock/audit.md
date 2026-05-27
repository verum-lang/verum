# `sync/rwlock` audit

Module: `core/sync/rwlock.vr` (341 LOC) — `RwLock<T>` (read-write
lock with writer-preferred fairness), `RwLockReadGuard<T>` /
`RwLockWriteGuard<T>` (RAII guards), re-exports `LockResult` /
`TryLockResult` / `PoisonError` / `TryLockError` from
`core.sync.mutex`.

State encoding:
* `state: AtomicInt` — 0 = unlocked, positive = reader count, -1
  (`WRITE_LOCKED`) = write-locked.
* `writers_waiting: AtomicInt` — writer-preference gate.  When
  `writers_waiting > 0`, incoming readers step aside and sleep so
  queued writers can drain the reader pile and acquire.
* `data: T` — protected data.
* `poisoned: AtomicBool` — advisory poison flag (same protocol as
  `Mutex<T>`; see `core-tests/sync/mutex/audit.md` §3.2).

Tests focus on the static + single-threaded uncontended surface:
constructor, poison protocol, advisory state.  Live `read()` /
`write()` blocking + reader/writer fairness require a multi-
threaded harness — tested at the language level in
`vcs/specs/L2-standard/sync/rwlock/`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.cache.lru`             | LRU consistency guard around eviction policy state |
| `core.context.scope`         | Singleton-scope ContextScope holds RwLock<HashMap<TypeId, Box<dyn Any>>> |
| `core.runtime.observability` | Metrics-counter snapshot uses many-reader / single-writer pattern |
| `core.collections.frozen_map` | Reader-heavy concurrent map exposes `RwLock<Map<K,V>>` |
| `core.io.fs.watcher`         | Inotify path-set protected by RwLock for many readers + occasional updates |

## 2. Crate-side hardcodes

* `WRITE_LOCKED: Int = -1` — the sentinel value for the write-locked
  state.  Codegen consumes the same constant when emitting the
  inline `compare_exchange(0, WRITE_LOCKED, ...)` in `try_write()`.
  Drift here breaks every `try_write` call site.
* `sys.{linux,darwin,windows}.thread.{futex_wait, futex_wake}` —
  per-platform OS-blocking primitives. The `&self.state.value`
  pointer-into-AtomicInt is passed verbatim to the futex syscall;
  changing `AtomicInt`'s record-layout (which is a single
  `value: Int`) silently breaks every RwLock blocking path.
* `RwLockReadGuard` / `RwLockWriteGuard` `Drop` impls call
  `self.rwlock.release_read()` / `release_write()` — these are
  package-private accessors (not `public`) intentionally; the only
  release path is via the RAII guard's drop.

## 3. Language-implementation gaps

### §3.1 Live read/write require multi-threaded harness

Cannot be tested at the data-shape level.  See `vcs/specs/L2-standard/sync/rwlock/`.

### §3.2 Writer-preference fairness gate

`rwlock.vr:57-67, 106-128, 160-178` documents the fairness model:
writer-preferred via the `writers_waiting` gate.  The architectural
rationale (readers-preferred starves writers) is pinned in the
doc-comment; runtime verification is a multi-threaded job.

### §3.3 Re-exported error types from `sync.mutex`

`rwlock.vr:338-341` re-exports `LockResult` / `TryLockResult` /
`PoisonError` / `TryLockError`.  Drift between mutex.vr's and
rwlock.vr's view of these is structurally invisible until a
caller-side type confusion blows up at compile-time.  Pinned in
`unit_test.vr` §3 by importing the same names from rwlock and
checking construction works for both paths.

### §3.4 `Default<T>` for `RwLock<T>` substitutes literal `0`

`rwlock.vr:257-261` — same task #17 workaround as Mutex.  LOCK-IN
pinned in `regression_test.vr`.

### §3.5 Read/Write guards do not share an ancestor protocol

`RwLockReadGuard` and `RwLockWriteGuard` both implement `Deref<Target=T>`
but only `RwLockWriteGuard` implements `DerefMut`.  This is the
correct algebraic distinction; the alternative (a single guard
type with a Bool discriminator) would conflate read-vs-write
safety.

## Action items landed in this branch

* `core-tests/sync/rwlock/unit_test.vr` — `@test`s covering
  `RwLock.new` + poison protocol + WRITE_LOCKED-sentinel-equivalent
  state observation + re-exported error types.
* `core-tests/sync/rwlock/property_test.vr` — poison-flag idempotence
  + state-machine laws + multi-RwLock independence.
* `core-tests/sync/rwlock/regression_test.vr` — LOCK-IN pin for §3.4
  (Default literal-0 workaround) + writer-preference gate documented
  in audit.
* `core-tests/sync/rwlock/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Live concurrent read+write tests | `vcs/specs/L2-standard/sync/rwlock/` | 1 day |
| Writer-preference fairness verification | multi-threaded harness | 1 day |
| Per-tier `--interp` vs `--aot` parity gate | full cross-tier run | 30 min once binary + runtime.vbca co-built |
