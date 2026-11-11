# core/sync Test Suite

Test coverage for Verum's synchronization primitives module.

## Test Organization

| File | Module | Coverage | Tests |
|------|--------|----------|-------|
| `mutex_test.vr` | `sync/mutex` | Mutex, MutexGuard, PoisonError, TryLockError, type variations | 61 |
| `rwlock_test.vr` | `sync/rwlock` | RwLock, RwLockReadGuard, RwLockWriteGuard, upgrade/downgrade | 80 |
| `atomic_test.vr` | `sync/atomic` | AtomicInt, AtomicU8/U16/U32/U64, AtomicBool, swap, CAS, bitwise, fence | 87 |
| `once_test.vr` | `sync/once` | Once, OnceState, call_once | 42 |
| `barrier_test.vr` | `sync/barrier` | Barrier, BarrierWaitResult, Phaser, CountDownLatch | 63 |
| `condvar_test.vr` | `sync/condvar` | Condvar, wait, wait_timeout, wait_while, notify | 79 |
| `semaphore_test.vr` | `sync/semaphore` | Semaphore, SemaphoreGuard, permit operations | 50 |
| `sync_improvements_test.vr` | `sync/*` | Mutex.is_locked, RwLock Default/Debug, AtomicInt fetch_and/or/max/min, OnceLock.get_or_try_init | 30 |
| `sync_protocols_test.vr` | `sync/mutex`, `sync/rwlock` | PoisonError Display/Debug, TryLockError Display/Debug, RwLock guard Debug, type existence | 24 |

## Key Types Tested

### Mutex<T>
Mutual exclusion primitive for protecting shared data.

**Construction:**
- `Mutex.new(value)` - Create with initial value
- `Mutex.default()` - Create with default value

**Lock Operations:**
- `lock()` - Acquire exclusive lock (blocking)
- `try_lock()` - Attempt non-blocking lock
- `get_mut()` - Get mutable reference (exclusive ownership)
- `into_inner()` - Consume mutex and return inner value

**Poison State:**
- `is_poisoned()` - Check if mutex was poisoned
- `clear_poison()` - Clear poison state

### MutexGuard<T>
RAII guard returned by `Mutex.lock()`.

**Methods:**
- `data()` - Get immutable reference to protected data
- `data_mut()` - Get mutable reference to protected data

### RwLock<T>
Reader-writer lock allowing multiple readers or single writer.

**Construction:**
- `RwLock.new(value)` - Create with initial value

**Lock Operations:**
- `read()` - Acquire read lock (multiple readers allowed)
- `write()` - Acquire write lock (exclusive)
- `try_read()` - Attempt non-blocking read lock
- `try_write()` - Attempt non-blocking write lock
- `get_mut()` - Get mutable reference (exclusive ownership)
- `into_inner()` - Consume and return inner value

### AtomicInt
Atomic integer for lock-free operations.

**Construction:**
- `AtomicInt.new(value)` - Create with initial value

**Operations:**
- `load(ordering)` - Load current value
- `store(value, ordering)` - Store new value
- `fetch_add(value, ordering)` - Add and return old value
- `fetch_sub(value, ordering)` - Subtract and return old value

### AtomicBool
Atomic boolean for lock-free flag operations.

**Construction:**
- `AtomicBool.new(value)` - Create with initial value

**Operations:**
- `load(ordering)` - Load current value
- `store(value, ordering)` - Store new value

### Memory Ordering
Ordering semantics for atomic operations.

**Variants:**
- `Relaxed` - No synchronization, only atomicity
- `Acquire` - Acquire ordering (load synchronization)
- `Release` - Release ordering (store synchronization)
- `AcqRel` - Both acquire and release
- `SeqCst` - Sequential consistency (strongest)

### Once
Initialization primitive that runs exactly once.

**Operations:**
- `new()` - Create new Once
- `call_once(f)` - Execute f exactly once
- `is_completed()` - Check if already executed
- `state()` - Get current OnceState

### OnceState
State of a Once initialization.

**Variants:**
- `New` - Not yet initialized
- `InProgress` - Currently initializing
- `Complete` - Initialization complete
- `Poisoned` - Initialization panicked

### Barrier
Synchronization point for multiple threads.

**Construction:**
- `Barrier.new(count)` - Create barrier for N threads

**Operations:**
- `wait()` - Wait at barrier (returns BarrierWaitResult)

### BarrierWaitResult
Result returned from `Barrier.wait()`.

**Methods:**
- `is_leader()` - Check if this thread is the leader

### Condvar
Condition variable for signaling between threads.

**Construction:**
- `Condvar.new()` - Create new condition variable

**Operations:**
- `wait(guard)` - Wait for notification
- `wait_timeout(guard, duration)` - Wait with timeout
- `notify_one()` - Wake one waiting thread
- `notify_all()` - Wake all waiting threads

### Semaphore
Counting semaphore for limiting concurrent access.

**Construction:**
- `Semaphore.new(permits)` - Create with initial permit count
- `Semaphore.binary()` - Create binary semaphore (1 permit)
- `Semaphore.default()` - Create binary semaphore

**Acquire Operations:**
- `acquire()` - Acquire permit (blocking)
- `acquire_many(n)` - Acquire multiple permits (blocking)
- `try_acquire()` - Try to acquire without blocking
- `try_acquire_many(n)` - Try to acquire multiple without blocking

**Release Operations:**
- `release()` - Release one permit
- `release_many(n)` - Release multiple permits
- `add_permits(n)` - Add permits (increase capacity)
- `forget_permit()` - Forget a permit (decrease capacity)

**Query:**
- `available_permits()` - Get current available permits

**RAII Guard:**
- `acquire_guard()` - Acquire and return RAII guard
- `acquire_many_guard(n)` - Acquire multiple and return RAII guard

### SemaphoreGuard
RAII guard that releases permits on drop.

**Methods:**
- `forget()` - Forget the guard, not releasing permits

## Tests by Category

### Mutex Tests (~24 tests)
- Construction (new, default, various types)
- Lock operations (lock, try_lock, nested)
- MutexGuard data access (data, data_mut)
- Poison state (is_poisoned, clear_poison)
- PoisonError methods (get_ref, get_mut, into_inner)
- TryLockError variants (WouldBlock, Poisoned)
- Multiple lock/unlock cycles
- Type-specific tests (List, Maybe)
- Edge cases (zero-sized types, immediate drop)

### RwLock Tests (~18 tests)
- Construction (new with various types)
- Read lock operations
- Write lock operations
- Multiple readers
- Reader-writer exclusion
- Upgrade/downgrade patterns
- Poison handling

### Atomic Tests (~27 tests)
- AtomicInt construction and load/store
- Fetch operations (fetch_add, fetch_sub)
- AtomicBool operations
- Memory ordering (Relaxed, Acquire, Release, SeqCst)
- Edge cases (max/min values)
- Counter simulation

### Once Tests (~40 tests)
- Basic call_once semantics
- Multiple call attempts
- State transitions
- Nested Once usage
- Panic handling
- Complex initialization patterns

### Barrier Tests (~27 tests)
- Basic barrier synchronization
- Leader election
- Multiple barriers
- Various thread counts

### Condvar Tests (~19 tests)
- Basic wait/notify
- notify_one vs notify_all
- Wait timeout
- Spurious wakeup handling

### Semaphore Tests (~42 tests)
- Construction (new, binary, default, various counts)
- try_acquire operations (single, multiple, exhausted, zero permits)
- try_acquire_many operations (success, exact, insufficient)
- Release operations (single, multiple, incremental)
- Blocking acquire operations (when available, multiple)
- available_permits and add_permits operations
- SemaphoreGuard RAII (creation, many, nested, forget)
- Binary semaphore behavior (mutex-like)
- Multiple acquire/release cycles
- Edge cases (single permit cycles, mixed operations)
- Capacity tracking and boundary tests

## Error Types

### LockResult<T>
Result type for lock operations.
- `Ok(guard)` - Lock acquired successfully
- `Err(PoisonError)` - Lock poisoned by panic

### TryLockResult<T>
Result type for non-blocking lock operations.
- `Ok(guard)` - Lock acquired successfully
- `Err(TryLockError)` - Lock not acquired

### TryLockError
Error from failed try_lock.
- `WouldBlock` - Lock already held
- `Poisoned(PoisonError)` - Lock poisoned

### PoisonError<T>
Error when a lock holder panicked.
- `get_ref()` - Get reference to guard
- `get_mut()` - Get mutable reference
- `into_inner()` - Extract guard

## Known Limitations

- Actual concurrent execution not tested (single-threaded simulation)
- Deadlock detection not tested
- Fair locking semantics not verified
- Platform-specific behavior not tested
- Thread local storage integration not tested

## Test Count: 516 tests total (9 test files)

## Architecture Notes

### Memory Model
Verum uses a C++11-style memory model:
- Acquire-release semantics for synchronization
- SeqCst for total ordering when needed
- Relaxed for simple atomic flags

### Lock-Free Patterns
- Atomic operations for counters and flags
- Compare-and-swap for lock-free data structures
- Memory barriers for visibility

### Poisoning
When a thread panics while holding a lock:
- Mutex/RwLock becomes "poisoned"
- Subsequent lock attempts return `Err(PoisonError)`
- `clear_poison()` allows recovery

### Performance Targets
- `AtomicInt.fetch_add()`: Single atomic instruction
- `Mutex.lock()` (uncontended): ~50ns
- `RwLock.read()` (uncontended): ~30ns
