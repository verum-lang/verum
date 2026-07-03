# `intrinsics/runtime/async_ops` audit

Module: `core/intrinsics/runtime/async_ops.vr` (~263 LOC) — async runtime
intrinsics (spawn/await/executor/supervision) + opaque handle types.

## Coverage decision: covered by `core-tests/async/intrinsics/` (no duplicate folder)

The executable surface (future_poll_sync round-trips, yield_now lifecycle,
executor_current coherence, spawn/block_on) is ALREADY conformance-tested
at `core-tests/async/intrinsics/` (unit+property+integration — discovered
in the same `verum test` walk; see INVENTORY).  A second folder keyed to
the *declaration* module would duplicate the suite without adding surface:
the intrinsics require the async runtime, and that runtime's home suite is
the async tree.  This audit file exists so the strict mirror rule has an
explicit, deliberate exception recorded (same shape as the syscall
decision).

## Findings

* The opaque-handle aliases (`JoinHandleOpaque` = `RawJoinHandleOpaque`,
  `AllocHandle` = `RawAllocHandle`) were added for conformance-test
  ergonomics — the canonical short names re-export through
  `core.intrinsics`.
* Supervision/recovery intrinsics (spawn_supervised, exec_with_recovery,
  circuit breaker) have no value-level surface without a multi-task
  runtime; they are exercised indirectly by `core-tests/async/supervisor`
  paths where present.

## Action items

* When the fuel-scheduler work lands (scripting P2 roadmap), revisit for
  deterministic single-thread executor conformance here.
