# `sync/mod` audit

Module: `core/sync/mod.vr` (163 LOC) — top-level re-export
surface for the synchronization primitives + marker-protocol
(`Send`, `Sync`) implementations for primitive / collection /
sync types + prelude exports.

The mod file is structurally:

```
public module atomic;       public module mutex;
public module rwlock;       public module once;
public module semaphore;    public module condvar;
public module barrier;      public module waitgroup;

public mount .atomic.{AtomicInt, AtomicU8, ..., AtomicBool, AtomicPtr,
                     MemoryOrdering, AtomicOrdering, Ordering, fence};
public mount .mutex.{Mutex, MutexGuard};
public mount .rwlock.{RwLock, RwLockReadGuard, RwLockWriteGuard};
public mount .once.{Once, OnceState};
public mount .semaphore.{Semaphore, SemaphoreGuard};
public mount .condvar.{Condvar, CondvarNotifyGuard, producer_consumer_pair};
public mount .barrier.{Barrier, BarrierWaitResult, Phaser, CountDownLatch};
public mount .waitgroup.{WaitGroup};

mount core.base.protocols.{Send, Sync};
implement Send for Int { } / Sync for Int { } / ... (Float, Bool, Byte,
    Char, Text, List<T>, Map<K,V>, Set<T>, Heap<T>, Shared<T>,
    Channel<T>, Mutex<T>, Maybe<T>, Result<T,E>, AtomicInt, AtomicBool)

public module prelude { public mount super.{Mutex, MutexGuard, RwLock,
    AtomicInt, AtomicBool, Send, Sync}; }
```

Tests focus on:
* Re-export resolution — every name listed in mod.vr can be
  constructed via the `core.sync.*` import path (mirror of the
  direct-submodule import path), confirming the `mount` graph
  in mod.vr is wired correctly.
* Two-source-of-truth equivalence — `Mutex` from
  `core.sync.mutex.Mutex` and from `core.sync.Mutex` resolve to
  the same type with identical method dispatch.

Send / Sync marker-protocol resolution is structural at this
audit's scope — we cannot exercise the actual cross-thread send
semantics from a single-threaded interpreter, but we CAN exercise
the trait bound resolution by constructing wrappers (e.g.,
`Heap<Mutex<Int>>`) that require `Mutex<Int>: Send`.

## 1. Cross-stdlib usage

`core.sync.prelude` and the top-level re-exports are used as the
canonical import path everywhere downstream:

| consumer | typical import |
|---|---|
| `core.runtime.*` | `mount core.sync.{Mutex, AtomicInt};` |
| `core.cache.*`   | `mount core.sync.{RwLock, Shared};` (Shared from elsewhere) |
| `core.collections.channel` | `mount core.sync.{Mutex, Condvar};` |
| `core.io.*` | `mount core.sync.{AtomicBool};` |

## 2. Crate-side hardcodes

None at this scope — the file is purely a `mount` graph + marker
protocol impls.  The marker-protocol bounds (`Send`, `Sync`) are
consulted by the unifier when resolving trait bounds; drift would
surface at compile-time.

## 3. Language-implementation gaps

### §3.1 Top-level re-export and submodule-direct must agree

`core.sync.Mutex` and `core.sync.mutex.Mutex` must resolve to the
same type.  Pinned by `unit_test.vr` §1.

### §3.2 Marker protocols Send / Sync auto-derivable

`mod.vr:96-148` declares `implement Send/Sync for Int / Float / Bool
/ Byte / Char / Text / List<T> / Map<K,V> / Set<T> / Heap<T> /
Shared<T> / Channel<T> / Mutex<T> / Maybe<T> / Result<T,E> /
AtomicInt / AtomicBool`.  We can't exercise the cross-thread send
semantics at single-thread level, but we can pin that the
constructors don't trip a `T: Send` bound failure when the inner
type satisfies it.

### §3.3 `prelude` re-exports a curated subset

`mod.vr:154-162` — Mutex, MutexGuard, RwLock, AtomicInt, AtomicBool,
Send, Sync.  Drift in this set is a public-API change — pin via
construction through the prelude path.

## Action items landed in this branch

* `core-tests/sync/mod/unit_test.vr` — re-export resolution for
  every top-level name listed in mod.vr; prelude resolution.
* `core-tests/sync/mod/regression_test.vr` — LOCK-IN for §3.1
  (top-level vs submodule-direct equivalence) + §3.3 (prelude
  curated set).
* `core-tests/sync/mod/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Live cross-thread Send / Sync verification | `vcs/specs/L2-standard/sync/markers/` | 1 day |
| Curated prelude expansion (e.g., add Semaphore, Condvar, Barrier) | `core/sync/mod.vr` + doc + tests | 30 min |
