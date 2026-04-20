# L2-standard/async triage

Strict audit on 2026-04-20 of `vcs/specs/L2-standard/async/` against
`target/release/vtest` built from `main`. Result: **16/57 PASS, 41 FAIL**.
The 41 failures cluster into four root-cause categories listed below.
Fixing each category is tracked as an individual follow-up task
because the remedies live in unrelated parts of the codebase.

## Category A — stdlib API mismatch `Result<T, _>` vs `T` (16 files)

The tests were written against an earlier `core.sync.*` API where
constructors like `Mutex.new(value)` returned `Mutex` directly. The
current stdlib returns `Result<Mutex, MutexError>`, so a test that
writes `let m = Mutex.new(value); m.lock()` type-checks against
`Result<…>.lock()` and fails with `E103 Cannot access field … on
non-record type`.

Representative files:
  - `synchronization/mutex.vr`        (E103 balance/counter)
  - `synchronization/rwlock.vr`
  - `synchronization/semaphore.vr`
  - `synchronization/condition_variable.vr`
  - `synchronization/oneshot.vr`
  - `synchronization/channel.vr`       (+ missing `recv_batch`)
  - `synchronization/barrier.vr`       (+ closure capture)
  - `safety/ref_across_await.vr`
  - `safety/shared_state.vr`
  - `spawn/spawn_with_config.vr`
  - `spawn/circuit_breaker.vr`
  - `spawn/restart_strategies.vr`
  - `spawn/supervision_tree.vr`
  - `streams/stream_buffering.vr`
  - `streams/stream_combinators.vr`
  - `streams/stream_error_handling.vr`

Remedy: rewrite construction sites to `let m = Mutex.new(value).unwrap();`
(or the pattern-matching equivalent) and propagate the new types
through the test's assertions. Each file is self-contained — edits
are mechanical but must be proof-read to keep the test's semantic
intent.

## Category B — codegen bug: spawn-block closure capture (6 files)

`spawn { ... }` bodies nested inside `.map(|i| ...)` closures fail
with `VBC codegen error: undefined variable: <i|owned|...>`. The
spawn-block captures the enclosing closure's parameters but the
synthetic function the VBC codegen emits for the spawn body does
not list the captured variables in its environment.

Representative files:
  - `synchronization/barrier.vr`   (`undefined variable: i`)
  - `structured/cancel_scope.vr`
  - `structured/nursery.vr`
  - `structured/task_group.vr`
  - `spawn/basic_spawn.vr`         (`undefined variable: owned`)
  - `spawn/supervisor.vr`

Remedy: teach `VbcCodegen::compile_spawn_block` to walk the
synthetic function's free variables and emit `NewClosure` with the
correct captures list. The Heap / Shared pass-through path in
`compile_method_call` is a close analogue — it already captures
from the enclosing scope. The work lives in
`crates/verum_vbc/src/codegen/expressions.rs` around the spawn
expression lowering.

## Category C — runtime output mismatch (16 files)

Tests run to completion and produce output, but stdout differs from
`@expected-stdout`. Causes break down further:

  - **Context propagation across spawn** — child task does not see
    parent's `provide Context = value` even after that was the whole
    point of the test (`spawn/context_propagation.vr`).
  - **Task scheduling ordering** — the test asserts a particular
    interleaving of messages from multiple concurrent tasks; the
    single-threaded interpreter schedules them in a different order
    than the test's expected stdout.
  - **Stream back-pressure / combinator count** — the test expects N
    elements to reach the consumer but the current channel
    implementation drops or reorders under load.

Representative files:
  - `spawn/context_propagation.vr`
  - `structured/nursery.vr`
  - `structured/cancel_scope.vr`
  - `streams/async_stream_basic.vr`
  - most other `structured/*` and `streams/*`

Remedy: either fix the runtime (context-on-spawn, scheduler order,
channel semantics) or relax the tests to check invariants rather
than exact stdout. The correct path is test-by-test — some tests
are measuring a real guarantee, others were written against an
implementation detail that is no longer guaranteed.

## Category D — missing stdlib method (3 files)

Tests call a method that does not exist on the stdlib type. The
identified instances:

  - `channel.vr`: `Receiver<T>.recv_batch(n)` not implemented.
  - (plus ad-hoc `.drain_to_vec()`-style calls in a couple of
    stream tests — same category, different types).

Remedy: implement the missing method in `core.async.*` or retire
the test's use of it. `recv_batch` is a documented extension so it
should be added, not removed.

## Pattern to apply category-by-category

1. Category A is mechanical and the highest-leverage — one pass
   through the 16 files lifts the overall pass rate from 28% to
   roughly 50%.
2. Category B is a single codegen fix that unblocks 6 files.
3. Category C needs per-file judgement but has clear categories.
4. Category D is scoped and small.

Fixing everything in one commit is not advisable — the categories
span different parts of the stack (test files / VBC codegen /
async runtime / stdlib) and should land in separate reviewable
commits.
