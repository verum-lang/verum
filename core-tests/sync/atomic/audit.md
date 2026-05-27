# `sync/atomic` audit

Module: `core/sync/atomic.vr` (1295 LOC) — atomic primitives:

* `MemoryOrdering` — 5-variant enum (Relaxed / Acquire / Release /
  AcqRel / SeqCst) + canonical `.name()` + Display + Debug + Eq
  (added this branch — see §3.2 below).
* `AtomicOrdering` — type alias for `MemoryOrdering`.
* `AtomicInt` / `AtomicU8` / `AtomicU16` / `AtomicU32` / `AtomicU64`
  / `AtomicBool` / `AtomicPtr<T>` — atomic-cell wrappers around the
  typed atomic intrinsics (`atomic_load_uN`, `atomic_store_uN`,
  `atomic_cas_uN`, `atomic_fetch_*_uN`).
* `fence(MemoryOrdering)` — explicit memory fence with per-arch
  asm dispatch (`mfence`/`lfence`/`sfence` on x86_64, `dmb` on
  aarch64).
* `SpinLock` / `FutexLock` — basic locks used by higher-level
  primitives (Mutex, RwLock, Condvar, ...).
* `ordering_to_u8` (private) — maps each MemoryOrdering variant to
  the LLVM-canonical UInt8 constant consumed by the typed
  atomic intrinsics.

Tests focus on the static surface that is testable without a
concurrent harness:
* MemoryOrdering 5-variant construction + pairwise disjointness +
  `name()` canonical-token round-trip + Eq reflexivity / inequality
  matrix + Display / Debug agreement with `name()`.
* AtomicOrdering alias preserves variant identity.
* `AtomicInt.new` + load/store/fetch_add/fetch_sub single-threaded
  round-trip (regression suite).
* `AtomicBool.new` + load/store single-threaded round-trip.

Live atomic operations under contention (CAS race, fetch_max/min
race, memory-ordering observability) need concurrent execution —
tested at language level via `vcs/specs/L2-standard/sync/atomic/`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.sync.once`     | OnceState state machine via AtomicInt CAS. |
| `core.sync.mutex`    | Mutex lock state via AtomicBool (`poisoned` flag) + FutexLock state. |
| `core.sync.rwlock`   | RwLock reader-count via AtomicInt, writer-queue gate via AtomicInt. |
| `core.sync.waitgroup` | (At Tier 1) counter via AtomicInt; (at Tier 0) host-side handle table. |
| `core.runtime.task_queue` | LIFO/FIFO push via atomic CAS. |
| `core.security.csprng` | RNG state guarded by atomic generation. |

## 2. Crate-side hardcodes

`crates/verum_vbc/src/intrinsics/atomic.rs` consumes the 5-variant
MemoryOrdering tag for LLVM atomic-ordering codegen.  Drift here
silently weakens memory barriers — a CRITICAL soundness incident.
Pin the canonical tag values + `name()` token strings + cross-test
with Rust-side ORDERING_* constants.

The `ordering_to_u8` mapping at `atomic.vr:114-122` is the
single source of truth for the variant-to-UInt8 mapping:
Relaxed=0, Acquire=1, Release=2, AcqRel=3, SeqCst=4.

## 3. Language-implementation gaps

### §3.1 Closed in this branch — qualified ordering_to_u8 arms

Source-side fix in earlier round (round-7 task #17 workaround).

### §3.2 Closed in this branch — Eq / Display / Debug / name for MemoryOrdering

Added `implement MemoryOrdering { fn name() }` + `Display` + `Debug`
+ `Eq` to `core/sync/atomic.vr` (this branch).  Pinned by
`property_test.vr` §B/§C + `regression_test.vr`.  Closes a previously-
deferred ergonomic gap: state-machine assertions like
`assert_eq(o, MemoryOrdering.Acquire)` and
`assert_eq(o.name(), "Acquire")` now work directly.

### §3.3 Live atomic tests deferred to L2 specs

Cannot test CAS-race, fetch_max-race, memory-ordering observability
without a multi-threaded harness.  Cross-reference
`vcs/specs/L2-standard/sync/atomic/`.

The single-threaded round-trip (load/store/fetch_add) IS exercised
in `regression_test.vr` — under no contention these degrade to
plain ops + the LLVM atomic fences, sufficient for a sanity
contract on the intrinsic-to-runtime wiring.

### §3.4 `ordering_to_u8` is private — no inverse `u8_to_ordering`

Symmetric inverse for deserialization scenarios.  Add when need
arises (likely for VBC bytecode round-trip).

## Action items landed in this branch

* `core/sync/atomic.vr` — MemoryOrdering `name()` / `Display` /
  `Debug` / `Eq` impls (closes audit §3.2).
* `core-tests/sync/atomic/unit_test.vr` — 9 unit tests (pre-existing).
* `core-tests/sync/atomic/property_test.vr` — pairwise disjointness
  matrix + name() injectivity + Eq laws + AtomicOrdering alias
  identity.
* `core-tests/sync/atomic/regression_test.vr` — LOCK-IN for §3.2
  + AtomicInt/AtomicBool single-threaded load/store/fetch_add/
  fetch_sub round-trip.
* `core-tests/sync/atomic/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `u8_to_ordering` inverse fn | `core/sync/atomic.vr` + tests | 30 min |
| Live atomic tests (CAS race, fetch_max/min race, memory-ordering observability) | `vcs/specs/L2-standard/sync/atomic/` | 1 day |
| AtomicPtr<T> single-threaded round-trip + AtomicU8/U16/U32 sister coverage | `core-tests/sync/atomic/integration_test.vr` | 1 day |
