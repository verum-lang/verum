# `core/async/panic_fence.vr` — audit

`PanicFence<F: Future>` wraps any Future so that panics during the
inner future's `poll` are caught (via `catch_unwind`) and surfaced
as `Err(IntrinsicPanicInfo)` instead of unwinding the executor's
task.  See module-level doc for the production-server threat model.

## Public API surface

| Name | Shape | Status under interpreter |
|---|---|---|
| `PanicFence<F: Future>` (record `{ inner: Maybe<F> }`) | wrapper Future | green (4 unit, 2 property) |
| `panic_safe<F: Future>(future: F) -> PanicFence<F>` | factory | green (1 unit, 2 property) |
| `Future::poll → Poll<Result<F.Output, IntrinsicPanicInfo>>` | poll surface (Ready/Pending arms) | green for Ready-Ok (6 tests); Ready-Err arm via panic-in-poll deferred to integration once catch_unwind is exercised directly |

## Cross-stdlib usage

* `core.intrinsics.control.catch_unwind` is the bedrock primitive
  this module wraps.  Drift surface: if catch_unwind's signature
  changes from `fn() -> T` to a different closure shape, panic_fence
  needs a parallel update.
* `core.async.future` provides the Future protocol and Context that
  panic_fence dispatches on.

## Crate-side hardcodes

None observed in crates/.  The fence is pure user-code at the
language level.

## Language-implementation gaps

1. **Task #11 — `Maybe.take()` mutation through `&mut self` does not
   flow back to a generic record field.** Repro pinned in
   `regression_test.vr §A` (`@ignore`).  The fence's documented
   lifecycle invariant ("inner = None after Ready") relies on this
   mutation; observably the field stays `Some(f)` and a second poll
   would re-enter a completed future. The `panic("PanicFence polled
   after completion")` guard at the top of poll therefore does
   *not* fire as intended.  Defect class likely lives in
   `crates/verum_vbc/src/codegen/` — the CBGR-ref writeback path
   for `*self = None` inside a generic-typed Maybe field.
2. **AOT global crash — SIGABRT in compiler.phase.generate_native.**
   Same root cause as `async/parallel`; tracked under task #10.
3. **Panic-arm test deferred.** Exercising the `Err(panic_info)`
   path requires a `Future::poll` body that panics (e.g. via
   `divide_by_zero` or explicit `panic`), wrapped in `catch_unwind`
   inside the fence.  Today the fence body uses `catch_unwind`
   directly; testing the Err arm needs a Future that panics on its
   first poll.  Land alongside catch_unwind's per-test bed.

## Action items landed in this branch

* Created `core-tests/async/panic_fence/{unit,property,integration,regression}_test.vr,audit.md`.
* 12 tests under interpreter — all green; 1 @ignore pin for task #11.
* Pinned: panic_safe factory shape, record-literal construction (Some/None
  inner), Ready(Ok) payload round-trip across Int + Text + bounded
  payload sets, fence-outcome → tag classification, fenced-Future list
  consumption summing 15.

## Action items deferred

* Task #11 (Maybe.take generic-field mutation defect) — the strict
  lifecycle invariant gates the "double-poll = panic" guard.
* AOT validation pending task #10.
* Panic-arm coverage pending a Future-that-panics test bed.
