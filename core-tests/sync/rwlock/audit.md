# `sync/rwlock` audit

Module: `core/sync/rwlock.vr` — `RwLock<T>` (read-write
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
* `reader_sequence` and `writer_sequence` are private `AtomicU32` words.
  Each waiter class registers on its own address.
* `data: T` — protected data.
* `poisoned: AtomicBool` — advisory poison flag (same protocol as
  `Mutex<T>`; see `core-tests/sync/mutex/audit.md` §3.2).

The local suites check construction, advisory poisoning and uncontended
acquisitions on separate locks. The public fixtures below exercise successive
guards on one lock and a contended handoff.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.cache.lru`             | LRU consistency guard around eviction policy state |
| `core.context.scope`         | Singleton-scope ContextScope holds RwLock<HashMap<TypeId, Box<dyn Any>>> |
| `core.runtime.observability` | Metrics-counter snapshot uses many-reader / single-writer pattern |
| `core.collections.frozen_map` | Reader-heavy concurrent map exposes `RwLock<Map<K,V>>` |
| `core.io.fs.watcher`         | Inotify path-set protected by RwLock for many readers + occasional updates |

## 2. Acquisition and release

The full-width signed state holds the reader count or the exclusive
sentinel. A successful acquire CAS synchronizes with release by the prior
holder. A waiting writer increments `writers_waiting` once and removes its
registration after acquiring exclusive ownership. A reader checks that gate
before trying to increment the shared state.

Each contender samples its class's sequence before inspecting the lock state.
A release advances the relevant 32-bit sequence before waking, so a contender
between inspection and parking rechecks its acquisition predicate. The last
reader signals one writer. A departing writer signals one writer and
broadcasts to readers, which recheck the writer gate. The atomic owner forms
the wait address from the same field used for increment and comparison.

## 3. Execution coverage

### §3.1 Public guard contract

The [guard fixture](../../../vcs/specs/L2-standard/async/synchronization/rwlock_guards.vr)
acquires two read guards, checks writer exclusion after one drops, writes
through an exclusive guard and checks reader exclusion. Both read and write
poison errors must return the held guard. The fixture repairs the value and
checks the final contents. Ordinary `verum check` admits the full fixture.
The development CLI stops before execution on both tiers because it lacks
the embedded standard-library archive. The existing local regression records
an earlier interpreter in which an exclusive guard remained held after scope
exit; its two-lock control cannot exercise reacquisition of the same lock.

### §3.2 Last-reader handoff and writer preference

With a single mixed wait queue, a final reader could wake another reader.
That reader would observe a queued writer and park again while the lock was
free. The final reader now signals only the writer address. Writer preference
places no bound on OS scheduling or on a specific writer's wait time.

The [VBC release test](../../../crates/verum_vbc/tests/stdlib_rwlock_release.rs)
compiles the real owner methods, typed atomic operations and registered
futex wake. The probe sets up two readers. Releasing the first leaves both
sequences unchanged; releasing the last advances just the writer sequence.
An exclusive release resets the state and advances both sequences. The
probe calls the release methods directly and does not exercise guard drop.

The [threaded fixture](../../../vcs/specs/L2-standard/async/synchronization/rwlock_writer_handoff.vr)
holds a read guard until a second `try_read` observes the queued writer.
A later reader must observe the writer's published value. Ordinary source
checking rejects both `ThreadBuilder.spawn` callbacks with E400 because
their synchronous callable properties are unknown. The
[VBC futex tests](../../../crates/verum_vbc/tests/futex_waiters.rs) execute
typed wait, wake and address isolation beneath this public handoff.

### §3.3 Re-exported error types from `sync.mutex`

The owner re-exports `LockResult` / `TryLockResult` /
`PoisonError` / `TryLockError`.  Drift between mutex.vr's and
rwlock.vr's view of these is structurally invisible until a
caller-side type confusion blows up at compile-time.  Pinned in
`unit_test.vr` §3 by importing the same names from rwlock and
checking construction works for both paths.

### §3.4 Default construction

`RwLock<T>.default()` delegates to `T.default()`. The local construction
regression uses `RwLock.new(0)`; that test exercises the explicit constructor.

### §3.5 Read/Write guards do not share an ancestor protocol

`RwLockReadGuard` and `RwLockWriteGuard` both implement `Deref<Target=T>`
but only `RwLockWriteGuard` implements `DerefMut`.  This is the
correct algebraic distinction; the alternative (a single guard
type with a Bool discriminator) would conflate read-vs-write
safety.

## Local suites

* [Unit tests](unit_test.vr) cover construction, poison state and error types.
* [Property tests](property_test.vr) cover poison idempotence and independent locks.
* [Regression tests](regression_test.vr) also acquire read and write guards on separate uncontended locks.
