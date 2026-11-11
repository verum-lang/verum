# core/async Test Suite

Comprehensive test coverage for Verum's asynchronous programming module.

## Architecture Overview

The async module provides asynchronous programming support:
- **Poll<T>**: Result of polling a future (Ready/Pending)
- **Future**: Protocol for asynchronous computations
- **Waker/Context**: Task wake-up mechanism for async runtime
- **Task**: Spawned asynchronous computation management
- **Channel**: Async MPSC message passing
- **Stream**: Async iterator protocol with combinators
- **Nursery**: Structured concurrency with task tracking
- **Executor**: Runtime configuration and builder

## Test Organization

| File | Module | Coverage | Tests |
|------|--------|----------|-------|
| `stream_test.vr` | `async/stream` | Stream protocol, constructors, combinators, polling | ~184 |
| `channel_test.vr` | `async/channel` | Sender, Receiver, bounded/unbounded, oneshot, errors | ~102 |
| `oneshot_fix_test.vr` | `async/channel` | Oneshot shared state fix, send/recv polling, multiple types | 12 |
| `executor_test.vr` | `async/executor` | RuntimeConfig, RuntimeBuilder, Sleep, Timeout, block_on | ~83 |
| `future_map_test.vr` | `async/future` | MapFuture, AndThenFuture, Lazy, Join2/3, Select2, block | ~61 |
| `future_test.vr` | `async/future` | ReadyFuture, PendingFuture, SelectResult, polling | ~57 |
| `nursery_test.vr` | `async/nursery` | NurseryOptions builder, NurseryError, error behaviors | ~47 |
| `select_test.vr` | `async/select` | Either, SelectAllResult, timeout, ready, yield_now | ~44 |
| `poll_test.vr` | `async/poll` | Poll variants, is_ready, is_pending, map, unwrap | ~27 |
| `task_test.vr` | `async/task` | TaskId, JoinError, YieldNow, JoinSet | ~27 |
| `waker_test.vr` | `async/waker` | Waker, Context, noop_waker, will_wake | ~25 |
| `future_minimal_test.vr` | `async/future` | Minimal future isolation tests | ~3 |
| `channel_protocols_test.vr` | `async/channel` | SendError/TrySendError/TryRecvError/RecvError Display, Sender/Receiver/Oneshot Debug | 36 |
| `channel_clone_test.vr` | `async/channel` | SendError/TrySendError/TryRecvError/RecvError Clone | 14 |
| `async_extended_test.vr` | `async/select`, `async/task`, `async/channel`, `async/future`, `async/nursery` | select functions (select_either, race, select_all, join, join3, join4, join_all, timeout, try_first, yield_now), spawn_blocking, JoinSet operations, TaskId uniqueness, JoinError variants, NurseryError/NurseryOptions, Either patterns, SelectAllResult, TimeoutError, ReadyFuture/NeverFuture, Channel error types, Poll operations | 64 |

## Key Types Tested

### Poll<T>
- `Ready(T)` - Future completed with value
- `Pending` - Future not yet complete
- Methods: `is_ready()`, `is_pending()`, `map()`, `unwrap()`, `unwrap_or()`, `ready()`

### Future Protocol & Combinators
- `ReadyFuture<T>` - Immediately complete future
- `PendingFuture<T>` - Never completes
- `MapFuture<Fut, F>` - Transform output with map
- `AndThenFuture<Fut1, F, Fut2>` - Chain futures
- `Lazy<F, T>` - Deferred computation
- `Join2<Fut1, Fut2>` - Wait for two futures
- `Join3<Fut1, Fut2, Fut3>` - Wait for three futures
- `Select2<Fut1, Fut2>` - Race two futures
- `SelectResult<A, B>` - Left/Right result of select
- `IntoFuture` - Protocol for future conversion
- `FutureExt` - Extension methods (map, and_then, block)

### Stream Protocol & Combinators
- Constructors: `iter`, `stream_once`, `stream_empty`, `stream_repeat`, `stream_repeat_n`, `stream_from_fn`, `interval`
- Transformations: `map`, `filter`, `filter_map`, `take`, `skip`, `enumerate`, `scan`, `inspect`
- Composition: `chain`, `zip`, `flatten`, `flat_map`
- Consumers: `collect`, `fold`, `for_each`, `any`, `all`, `find`, `count`, `first`, `last`, `min`, `max`, `nth`
- Flow control: `buffered`, `chunks`, `fuse`, `peekable`

### Task Management
- `TaskId` - Unique task identifier
- `JoinError` - Cancelled or Panicked
- `YieldNow` - Cooperative yield future
- `JoinSet<T>` - Collection of spawned tasks

### Channels
- `Sender<T>` - MPSC sender (cloneable)
- `Receiver<T>` - MPSC receiver
- `channel()` - Unbounded channel
- `bounded(n)` - Bounded channel with capacity
- `oneshot()` - One-shot channel pair
- Error types: `SendError`, `TrySendError`, `TryRecvError`

### Nursery (Structured Concurrency)
- `NurseryOptions` - Configuration with builder (timeout, max_tasks, error_behavior)
- `NurseryErrorBehavior` - CancelAll, WaitAll, FailFast
- `NurseryError` - Cancelled, Timeout, TaskLimitExceeded, Panic, Single, Multiple

### Executor
- `RuntimeConfig` - Presets: default, cpu_bound, io_bound
- `RuntimeBuilder` - Fluent builder for Runtime
- `Sleep` - Timer future
- `TimeoutError` - Timeout result type
- `ExecutionEnv` - Execution environment with parent tracking

### Waker System
- `Waker` - Task wake-up handle
- `Context` - Polling context containing waker
- `noop_waker()` - No-op waker for testing

## Key Invariants Tested

1. **Poll state**: Ready and Pending are mutually exclusive
2. **Future polling**: ReadyFuture returns Ready on first poll, PendingFuture always returns Pending
3. **MapFuture**: Transforms output, supports chaining (map().map())
4. **AndThenFuture**: State machine First→Second, chains futures sequentially
5. **Join2/Join3**: Polls all futures, returns Ready only when all complete
6. **Select2**: Returns Left for first ready future, Right for second; both Pending = Pending
7. **block()**: Synchronously runs future to completion using noop_waker
8. **TaskId uniqueness**: Each TaskId.new() returns unique ID
9. **YieldNow behavior**: First poll returns Pending, second poll returns Ready
10. **Channel FIFO**: Messages received in order sent
11. **Waker identity**: Cloned wakers wake same task
12. **NurseryOptions builder**: Chaining preserves all fields
13. **Stream size_hint**: Decreases as items are consumed

## Test Count: ~774 tests total (15 passing files)

## Known Limitations

- `join(mapped_future, mapped_future)` output tuple indexing fails — type checker can't resolve Output<Join2<MapFuture<...>>>
- Tests for `spawn()`, `join()`, `select()` async functions require runtime executor
- Lazy futures and future combinators with complex types may cause stack overflow
- Tests focus on types that can be tested without runtime execution
