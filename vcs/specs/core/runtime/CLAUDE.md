# core/runtime Test Suite

Test coverage for Verum's runtime module including task spawning, supervision, and error recovery.

## Test Organization

| File | Module | Coverage | Tests |
|------|--------|----------|-------|
| `mod_test.vr` | `runtime/mod` | Context slot constants, function existence, PriorityLevel, CircuitState, SupervisionStrategy, BackoffStrategy, type existence | 41 |
| `config_test.vr` | `runtime/config` | InitError, RuntimeIoError, IoOp, IoHandle, NoopExecutor | 34 |
| `recovery_test.vr` | `runtime/recovery` | BackoffStrategy, CircuitState, JitterConfig, RecoveryStrategy | 57 |
| `spawn_test.vr` | `runtime/spawn` | PriorityLevel, SpawnConfig, SpawnConfigBuilder, InlineContextStorage, RestartPolicy, RecoveryStrategy, spawn function existence | 73 |
| `supervisor_test.vr` | `runtime/supervisor` | Supervision trees, restart strategies, child specs, escalation | 73 |
| `env_test.vr` | `runtime/env` | ExecutionTier, IsolationLevel, CpuAffinity, TaskId, RestartPolicy | 64 |
| `task_queue_test.vr` | `runtime/task_queue` | StealResult, WorkDeque, Stealer, WorkStealingPool, BoundedQueue | 57 |
| `stack_alloc_test.vr` | `runtime/stack_alloc` | StackAllocator, ArenaAllocator, PoolAllocator, savepoints, type aliases | 44 |
| `recovery_protocols_test.vr` | `runtime/recovery` | CircuitState Eq, BackoffStrategy/JitterConfig/RetryPredicate/RetryConfig/RecoveryStrategy Clone, Debug | 30 |
| `config_protocols_test.vr` | `runtime/config` | InitError Debug/Display, RuntimeIoError Debug/Display | 21 |
| `runtime_behavior_test.vr` | `runtime/recovery`, `runtime/supervisor`, `runtime/spawn`, `runtime/task_queue`, `runtime/stack_alloc`, `runtime/env` | CircuitBreakerConfig/Stats/Error, BackoffStrategy edge cases, CircuitState, JitterConfig, RetryConfig, RecoveryStrategy, SupervisionStrategy, RestartStrategy behavior, FailureReason classification, ChildSpec builder, RestartIntensity, ShutdownStrategy, EscalationPolicy/Reason, SupervisorConfig, SpawnConfig advanced, TaskQueue types, Stack allocators, Execution environment types, ErrorPredicate/RetryPredicate | 98 |

## Key Types Tested

### PriorityLevel
Task scheduling priority hint.

**Variants:**
- `Background` (32) - Batch processing, cleanup
- `Low` (64) - Non-urgent work
- `Normal` (128) - Default priority
- `High` (192) - User-facing operations
- `Critical` (255) - System operations, health checks

**Methods:**
- `to_u8()` - Convert to numeric value
- `from_u8(val)` - Create from numeric value
- `default()` - Returns Normal

### SpawnConfig
Zero-cost builder for task spawn configuration.

**Fields:**
- `name` - Task name (optional)
- `recovery` - Recovery strategy (optional)
- `restart` - Restart policy (optional)
- `isolation` - Isolation level (optional)
- `priority` - Priority level (default: Normal)
- `supervisor` - Supervisor handle (optional)
- `deadline_ns` - Absolute deadline (optional)
- `stack_size` - Stack size hint (optional)

**Builder Methods:**
- `new()`, `default()` - Construct with defaults
- `with_name(name)` - Set task name
- `with_priority(level)` - Set priority
- `with_isolation(level)` - Set isolation
- `with_timeout_ms(ms)` - Set relative timeout
- `with_deadline_ns(ns)` - Set absolute deadline
- `with_stack_size(bytes)` - Set stack size
- `with_simple_retry(count, delay)` - Fixed retry
- `with_exponential_retry(count, base, max)` - Exponential backoff
- `with_restart_limits(count, window)` - Restart limits
- `clone()` - Clone configuration

### SpawnConfigBuilder
Validated builder pattern for SpawnConfig.

**Methods:**
- `new()`, `from_config(config)` - Construct
- `name(name)` - Set name
- `permanent()`, `transient()`, `temporary()` - Restart policies
- `background()`, `high_priority()`, `critical()` - Priority shortcuts
- `isolated()`, `send_only()` - Isolation shortcuts
- `with_retry_default()`, `with_circuit_breaker_default()` - Recovery defaults
- `build()`, `build_unchecked()` - Build config

### IsolationLevel
Memory isolation modes for spawned tasks.

**Variants:**
- `Shared` - Share parent's memory context
- `SendOnly` - Only Send types cross boundary
- `Full` - Complete isolation

### Supervision Tree Types

**SupervisorId / ChildId:**
- `new()` - Generate unique ID
- `root()` - Root supervisor ID (0)
- `raw()` - Get raw value

**SupervisionStrategy:**
- `OneForOne` - Restart only failed child
- `OneForAll` - Restart all children
- `RestForOne` - Restart failed + subsequent children
- `description()` - Human-readable description

**RestartStrategy:**
- `Permanent` - Always restart
- `Transient` - Restart only on abnormal exit
- `Temporary` - Never restart
- `should_restart(reason)` - Check if should restart

**FailureReason:**
- `NormalExit` - Clean shutdown
- `Shutdown` - Requested shutdown
- `Crash { message }` - Error/panic
- `Timeout` - Operation timeout
- `Killed` - Forcefully killed
- `is_abnormal()` - Check if abnormal
- `description()` - Get description

**ChildStatus:**
- `Running`, `Starting`, `Restarting` - Active states
- `Terminated`, `Failed`, `Stopped` - Inactive states
- `from_u8(val)` - Create from UInt8
- `is_active()` - Check if active state

**ChildSpec:**
- `new()`, `permanent()`, `temporary()`, `default()` - Constructors
- `with_restart(strategy)` - Set restart strategy
- `with_shutdown_timeout_ms(ms)` - Set shutdown timeout
- `with_restart_limits(count, window)` - Set restart limits
- `with_priority(level)` - Set priority

**RestartIntensity:**
- `new(max, window_ms)` - Create tracker
- `record_restart()` - Record and check if allowed
- `would_allow_restart()` - Check without recording
- `current_count()` - Get current count
- `reset()` - Reset tracking

**ShutdownStrategy:**
- `graceful()` - Default graceful (5s timeout)
- `brutal()` - Immediate shutdown
- `Graceful { timeout_ms }` - Custom timeout
- `timeout_ms()` - Get timeout

**SupervisorConfig:**
- `one_for_one()`, `one_for_all()`, `rest_for_one()` - Strategy constructors
- `default()` - OneForOne with defaults
- `with_restart_limits(count, window)` - Set limits
- `with_escalation(policy)` - Set escalation policy

**EscalationPolicy:**
- `Escalate` - Escalate to parent supervisor
- `Restart` - Restart this supervisor
- `Shutdown` - Shutdown supervisor tree
- `Ignore` - Ignore and continue

**EscalationReason:**
- `MaxRestartsExceeded` - Hit restart limit
- `ChildEscalated` - Child escalated error
- `ShutdownFailed` - Shutdown didn't complete
- `CircuitBreakerOpen` - Circuit breaker triggered
- `description()` - Get description

### BackoffStrategy
Retry delay calculation strategies.

**Variants:**
- `Fixed { delay_ms }` - Constant delay
- `Linear { base_ms, increment_ms, max_ms }` - Linear increase
- `Exponential { base_ms, max_ms, multiplier }` - Exponential backoff
- `None` - No delay

**Methods:**
- `calculate_delay_ms(attempt)` - Get delay for attempt
- `exponential(base, max, mult)` - Convenience constructor

### CircuitState
Circuit breaker state machine.

**Variants:**
- `Closed` - Normal operation
- `Open` - Blocking requests
- `HalfOpen` - Testing recovery

**Methods:**
- `to_u8()`, `from_u8(val)` - Conversion

### JitterConfig
Jitter application for retry delays.

**Variants:**
- `None` - No jitter
- `Proportional(percent)` - ±X% jitter
- `Fixed(ms)` - ±X ms jitter
- `Full` - 0 to 100% jitter

**Methods:**
- `apply(delay_ms)` - Apply jitter to delay

## Tests by Category

### Config Tests (~38 tests)
- InitError construction and message
- RuntimeIoError variants
- IoOp and IoHandle construction
- IoCompletion result handling
- NoopExecutor and NoopDriver protocols

### Recovery Tests (~30 tests)
- BackoffStrategy variant construction
- `calculate_delay_ms()` for all strategies
- Delay capping for Linear and Exponential
- CircuitState conversion methods
- JitterConfig variant construction and apply

### Spawn Tests (~35 tests)
- PriorityLevel variants and conversion
- Priority ordering and default
- SpawnConfig construction and defaults
- Builder methods (name, priority, isolation, timeout, deadline, stack_size)
- Builder chaining and clone
- SpawnConfigBuilder validation helpers
- Recovery configuration shortcuts
- Edge cases (empty name, zero timeout, overrides)

### Supervisor Tests (~50 tests)
- SupervisorId and ChildId generation
- SupervisionStrategy variants and description
- RestartStrategy behavior (permanent, transient, temporary)
- FailureReason classification and is_abnormal
- ChildStatus transitions and is_active
- ChildSpec construction and builder pattern
- RestartIntensity tracking and limits
- ShutdownStrategy configuration
- SupervisorConfig construction and configuration
- EscalationPolicy and EscalationReason variants

### Stack Allocator Tests (~40 tests)
- StackSavepoint construction and field access
- ArenaSavepoint construction and field access
- StackAllocator construction, capacity, used, remaining, watermark, alloc_count
- StackAllocator save/restore and reset/reset_all
- ArenaAllocator construction, capacity, used, remaining_in_chunk
- ArenaAllocator save/restore and reset
- PoolAllocator construction, block_size, block_count, allocated, available
- Type aliases: TinyStack, SmallStack, MediumStack, LargeStack, SmallArena, MediumArena, ConnectionPool, RequestContextPool
- Allocator invariants (used + remaining == capacity, etc.)
- Multiple save/restore cycles

## Known Limitations

- ExecutionEnv (θ+) not tested (requires runtime initialization)
- Actual task spawning not tested (requires async executor)
- Runtime initialization/shutdown not tested
- RetryPolicy and CircuitBreaker full lifecycle not tested (trait object issues)
- Stack allocator alloc/dealloc not tested (requires Layout + pointer operations)
- ScopedStack/ScopedArena not tested (requires mutable reference passing)
- Integration tests require actual runtime

## Test Count: 598 tests total (12 test files)

## Architecture Notes

### Zero-Allocation Design
- SpawnConfig uses inline storage (~10ns construction)
- InlineContextStorage holds up to 4 contexts inline
- InlineCircuitBreaker: 64 bytes (cache-line aligned)
- InlineRetryPolicy: 32 bytes
- Overflow to heap only in rare cases

### Performance Targets
- `SpawnConfig::new()`: ~10ns
- Builder methods: ~0-5ns each
- `apply_to_env()`: ~50-100ns
- Context merge: ~5-30ns per context
- Total spawn overhead: ~100-170ns

### Supervision Hierarchy
```
Root Supervisor (id=0)
├── Child A (Permanent)
├── Child B (Transient)
└── Nested Supervisor
    ├── Child C
    └── Child D
```

When restart limits exceeded, escalation flows up the tree according to EscalationPolicy.
