# `sync/atomic` audit

Module: `core/sync/atomic.vr` (1276 LOC) — atomic primitives
(MemoryOrdering enum + AtomicInt/AtomicBool/AtomicU64/AtomicPtr +
fence + ordering_to_u8 internal mapping).

Tests focus on the static surface: MemoryOrdering 5-variant ADT
construction + disjointness + AtomicOrdering alias. Runtime
atomic operations (load/store/CAS/fetch_*) need concurrent
execution — tested at language level via
`vcs/specs/L2-standard/sync/atomic/`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.sync.once` | OnceState state machine via AtomicInt CAS. |
| `core.sync.mutex` | Mutex lock state via AtomicBool. |
| `core.sync.waitgroup` | counter via AtomicInt. |
| `core.runtime.task_queue` | LIFO/FIFO push via atomic CAS. |
| `core.security.csprng` | RNG state guarded by atomic generation. |

## 2. Crate-side hardcodes

`crates/verum_vbc/src/intrinsics/atomic.rs` consumes the 5-variant
MemoryOrdering tag for LLVM atomic-ordering codegen. Drift here
silently weakens memory barriers — a CRITICAL soundness incident.
Pin the canonical tag values + cross-test with Rust-side
ORDERING_* constants.

## 3. Language-implementation gaps

### §3.1 Closed in this branch — qualified ordering_to_u8 arms

Source-side fix in this round.

### §3.2 Add Eq / Display / Debug for MemoryOrdering

Useful for state-machine assertion messages. Add following the
OnceState pattern (commit 017ad061b).

**Effort:** small (~30 min).

### §3.3 Live atomic tests deferred to L2 specs

Cannot test load/store/CAS without a multi-threaded harness.
Cross-reference `vcs/specs/L2-standard/sync/atomic/` in audit.

### §3.4 `ordering_to_u8` is private — no inverse `u8_to_ordering`

Symmetric inverse for deserialization scenarios. Add when need
arises (likely for VBC bytecode round-trip).

## Action items landed in this branch

* `core/sync/atomic.vr` — qualified ordering_to_u8 match arms.
* `core-tests/sync/atomic/unit_test.vr` — 9 unit tests.
* `core-tests/sync/atomic/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add Eq / Display / Debug for MemoryOrdering | `core/sync/atomic.vr` + tests | 30 min |
| Add u8_to_ordering inverse fn | `core/sync/atomic.vr` + tests | 30 min |
| Live atomic tests (load/store/CAS/fetch_*) | `vcs/specs/L2-standard/sync/atomic/` | 1 day |
| Sister tests for `core.sync.{mutex,rwlock,condvar,semaphore,barrier,waitgroup}` static surface | sister folders | 1 day each |
EOF
