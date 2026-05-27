# `sync/barrier` audit

Module: `core/sync/barrier.vr` (601 LOC) — three primitives:

* `Barrier` — fixed-N rendezvous synchronization point; can be
  reused after all N threads have arrived (`generation` counter
  increments).  Backed by `Mutex<BarrierState> + Condvar`.
* `BarrierWaitResult` — data record `{ is_leader: Bool }` returned
  from `Barrier.wait()`.  Exactly one thread per synchronization is
  designated the leader.
* `Phaser` — multi-phase barrier with dynamic party registration
  (atomic packed state: 16 bits parties + 16 bits arrived + 30 bits
  phase + 1 terminated bit at bit 62).  Documents the
  arithmetic-right-shift hazard fix that moved `TERMINATED_BIT`
  from the sign bit to bit 62.
* `CountDownLatch` — single-use count-down barrier; `wait_for_zero`
  + `wait_for_zero_timeout` block until the counter reaches zero
  via `count_down()` calls.

Tests focus on the data-shape and single-threaded surface:
* `Barrier.new(n)` constructor + `num_threads()` accessor +
  `waiting_count()` returning 0 on a fresh barrier +
  `generation()` returning 0 on a fresh barrier.
* `Barrier.new(1)` — single-thread barrier; `wait()` returns
  immediately with `is_leader == true`.
* `BarrierWaitResult` data-record construction + `is_leader()` +
  `Default` impl (defaults to `is_leader: false`).
* `Phaser.new(parties)` constructor + `get_phase()` /
  `get_registered_parties()` / `get_arrived_parties()` /
  `is_terminated()` accessors.
* `Phaser.register()` / `terminate()` mutations + observable state
  changes.
* `CountDownLatch.new(n)` + `get_count()` accessor +
  `count_down()` decrement + `wait_for_zero()` on a zero latch
  returns immediately.

Live multi-thread `Barrier.wait` rendezvous, `Phaser.arrive_and_await`
phase transitions, and `CountDownLatch.wait_for_zero_timeout`
condvar interactions are exercised at the L2-spec level.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.runtime.task_pool`   | Worker-thread fan-out / fan-in via Barrier |
| `core.concurrency.tasks`   | `TaskGroup.join()` semantics layer over Barrier |
| `core.async.executor`      | Reactor startup synchronization via CountDownLatch |
| `core.io.stream_pipeline`  | Multi-stage pipeline phase advance via Phaser |

## 2. Crate-side hardcodes

* `Phaser` packed-state encoding (bit ranges + `TERMINATED_BIT = 1 << 62`):
  bits 0-15 = parties, bits 16-31 = arrived, bits 32-61 = phase
  (30 bits = ~1.07 billion phases), bit 62 = terminated, bit 63
  UNUSED (sign bit hazard — see `barrier.vr:262-288`).  Drift in
  the bit layout silently corrupts every Phaser op.
* `Phaser.arrive_and_await` and `Phaser.arrive_and_deregister`
  phase-advance paths use CAS loops that MUST preserve `TERMINATED_BIT`
  and concurrent party-count changes (post task #32 fix).  See
  `barrier.vr:332-388` + `:396-442`.
* `CountDownLatch.wait_for_zero_timeout` deadline arithmetic uses
  `monotonic_nanos()` intrinsic — pinned to per-iteration recomputation
  against an absolute monotonic deadline (post-fix at `barrier.vr:548-587`).

## 3. Language-implementation gaps

### §3.1 Live Barrier.wait / Phaser.arrive_and_await / CountDownLatch.wait require multi-threaded harness AND Tier-0 futex-FFI stub

Two-layer gating:

1. **Multi-threaded harness** — `Barrier.wait()` / `Phaser.arrive_and_await()` /
   `CountDownLatch.wait_for_zero_timeout(>0)` block on a condvar that
   only releases via cross-thread notification.  Cannot be tested at
   the data-shape level.  See `vcs/specs/L2-standard/sync/barrier/`.

2. **Tier-0 futex-FFI gap** — even single-threaded paths that touch
   the futex (e.g. `Phaser.register()` acquires Phaser.mutex,
   `Phaser.terminate()` issues `condvar.notify_all()`,
   `CountDownLatch.count_down()` triggers `notify_all` when counter
   reaches zero) trip a "FFI symbol not found: FfiSymbolId(61)" at the
   Tier-0 interpreter — the underlying `futex_wake`/`futex_wait`
   syscall isn't bound in the FFI symbol table.  Tests `@ignore`d
   here pin the static-state contracts; promote to live `@test` when
   the Tier-0 futex FFI lands at
   `crates/verum_vbc/src/interpreter/dispatch_table/handlers/ffi_extended.rs`.

### §3.2 `Phaser.terminate` + termination-flag preservation

The `TERMINATED_BIT` invariant under concurrent phase advance is
pinned in the doc-comment at `barrier.vr:262-288`.  Single-threaded:
verify `terminate()` flips `is_terminated()` from false → true.

### §3.3 `Barrier.new(0)` panics by assert

`barrier.vr:113-114` — `assert(n > 0, "Barrier requires at least 1 thread")`.
Cannot easily verify the panic surface without an `expect = panic`
test mechanism; we instead pin the boundary case
`Barrier.new(1)` constructs cleanly.

### §3.4 `CountDownLatch.new(0)` is degenerate but valid

`barrier.vr:513-514` — `assert(count >= 0, ...)` permits zero.
A zero-counter latch is immediately drained; `wait_for_zero()`
returns immediately + `wait_for_zero_timeout(any)` returns true.

### §3.5 `BarrierWaitResult.Default` is `is_leader: false`

`barrier.vr:201-204` — pinned in `unit_test.vr` §2.

### §3.6 `CountDownLatch.wait_for_zero_timeout` loop-on-predicate
+ absolute deadline

The deadline arithmetic — recompute remaining_ns each iteration
against `monotonic_nanos()` — is load-bearing for correctness under
spurious wakes (audit pre-fix shape: single-shot loop returning
false on first spurious wake).  Cannot exercise the spurious-wake
path at single-thread level; pinned by doc-comment + L2-spec.

## Action items landed in this branch

* `core-tests/sync/barrier/unit_test.vr` — Barrier.new construction
  + num_threads / waiting_count / generation accessors +
  BarrierWaitResult data-record + Default + Phaser.new constructor
  + accessors + register / terminate mutations + CountDownLatch.new
  + count_down + wait_for_zero on drained latch.
* `core-tests/sync/barrier/property_test.vr` — fresh-barrier
  invariant + Phaser bit-layout pins + CountDownLatch drain law.
* `core-tests/sync/barrier/regression_test.vr` — LOCK-IN for §3.2
  (TERMINATED_BIT preservation) + §3.6 (CountDownLatch zero-counter
  drained-immediately).
* `core-tests/sync/barrier/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Live Barrier rendezvous + leader-election | `vcs/specs/L2-standard/sync/barrier/` | 1 day |
| Phaser concurrent register / arrive_and_await | multi-thread | 1 day |
| CountDownLatch.wait_for_zero_timeout spurious-wake | multi-thread | 1 day |
