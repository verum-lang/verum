# `core/async/intrinsics.vr` — audit

Bedrock async primitives that bridge Verum's `async fn` surface to
the underlying runtime.

## Public API surface

| Name | Shape | Status under interpreter |
|---|---|---|
| `Executor` (record `{ handle: Int }`) + `Executor.current()`, `Executor.in_async_context()` | opaque runtime handle | green (4 unit, 1 property) |
| `future_poll_sync<F: Future>(future: &mut F) -> Maybe<F.Output>` | synchronous one-shot driver | green (3 unit, 2 property, 1 integration) |
| `IntrinsicsYieldNow` + `yield_now()` | cooperative yield-once | green (4 unit, 2 property, 1 integration) |
| `async_sleep_ms` / `async_sleep_ns` | re-exported from `core/intrinsics/runtime/async_ops.vr` | covered by the canonical module's tests |
| `spawn_with_env<F>(future) -> JoinHandle<F.Output>` | `@intrinsic("async_spawn")` | deferred — requires live executor |
| `executor_spawn<F>(&Executor, future) -> JoinHandle<F.Output>` | `@intrinsic("async_executor_spawn")` | deferred — requires live executor |
| `executor_block_on<F>(future) -> F.Output` | `@intrinsic("async_block_on")` | deferred — requires live executor |

## Cross-stdlib usage

* `async/timer.vr` re-exports `async_sleep_ms`/`ns` through this
  module, so it sits in the dependency graph of every timer test.
* `async/future.vr` Future combinators are driven by `future_poll_sync`
  in user-level code that wants synchronous-style polling (tests,
  REPLs, manual event loops).
* `async/executor.vr` calls `Executor.current()` via the public
  `current_runtime()` accessor.

## Crate-side hardcodes

The three `@intrinsic("async_spawn|async_executor_spawn|async_block_on")`
declarations bind to the same-named names in the VBC intrinsic
registry.  Drift surface: if the registry renames any of those keys
without updating this file, the binding silently breaks at call
time.  No test pins those keys yet — file a Rust-side pinning macro
when the registry's three keys move into a single source of truth.

## Language-implementation gaps

1. **AOT global crash — SIGABRT in compiler.phase.generate_native.**
   Same root cause as `async/parallel`; tracked under task #10.
2. **Spawn family runtime tests deferred** — they require a live
   executor to drive the `JoinHandle` lifecycle.  Will land alongside
   the executor test suite.

## Action items landed in this branch

* Created `core-tests/async/intrinsics/{unit_test.vr,property_test.vr,integration_test.vr,audit.md}`.
* 19 tests under interpreter — all green.
* Pinned the Executor.current/in_async_context coherence invariant,
  the synchronous Future-poll round-trip across Int + Text payload
  shapes, and the cooperative yield-now two-state lifecycle (Pending →
  Ready(())) including the "exactly one Pending before Ready" tightness.

## Action items deferred

* AOT validation pending task #10.
* Spawn-family @intrinsic tests pending executor test-bed.
* Sleep-family tests live with the canonical `core/intrinsics/runtime/async_ops.vr`
  module's tests; this module's re-exports are exercised transitively
  through async/timer.
