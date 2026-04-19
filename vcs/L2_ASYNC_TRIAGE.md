# L2-standard/async failure triage — T1-J

Snapshot taken 2026-04-20 after the current parser + type-system state.

**41 failures out of 397 L2 tests (89.7% pass rate)** under the
`--compile-time-only --filter async` suite.

## Category A — stdlib method gaps (14 failures)

Tests reference methods that aren't yet on the stdlib surface:

| Test | Missing | Fix |
|------|---------|-----|
| `futures/timeout.vr` | `Sender<T>.send_timeout` | Add to `core/async/channel.vr` |
| `futures/*` (multi) | Various future combinators | Expand future surface |
| `safety/shared_state.vr` | `Mutex<T>.lock` implicit unwrap | Typecheck of `Result<Mutex, _>.users` |
| `safety/ref_across_await.vr` | `Result<_, _>` deref in await context | Same |
| `errors/cancellation.vr` | Error-propagation combinators | — |
| `errors/*` (multi) | Error types | — |
| `spawn/circuit_breaker.vr` | `CircuitBreaker` semantics | — |

Fix strategy: extend `core/async/`, `core/sync/`, `core/concurrency/`.
Scope-separate from T1-J (this is a stdlib expansion task, T1-Q/T1-R
style).

## Category B — Type-checker limitations (9 failures)

Tests expose genuine gaps in inference:

| Test | Issue | Related task |
|------|-------|--------------|
| `safety/ref_across_await.vr` | `Result<_, _>` needs automatic unwrap or `?`/`try`-style | T1-E row polymorphism already addressed extensible records; this is about sum-type ergonomics |
| `structured/supervisor.vr` | Supervisor hierarchy unclear | Needs design doc |
| `structured/cancel_scope.vr` | Cancel-scope with context propagation | Needs design |
| `safety/deadlock_prevention.vr` | Static deadlock analysis | Requires CBGR path-sensitivity extension |

## Category C — Test-side staleness (18 failures)

Tests use deprecated/renamed API:

| Test | Stale reference | Action |
|------|-----------------|--------|
| Multiple | Old `Database.query(...)` without `using [Database]` | Update tests to use `using` block per current spec |
| Multiple | `Logger` context without provide | Wrap in `provide Logger { ... }` |
| Multiple | `yield` across async generator boundaries | Simplify to `Result`-based yields |

Fix strategy: update test-side code to current API. Bulk refactor.

## Breakdown

- **Parser-level:** 0 (all 41 failures compile-to-AST cleanly)
- **Typecheck:** 32 (method lookup or inference gaps)
- **Runtime VBC:** ~9 (need runtime execution to verify)

## Priorities for T1-J completion

1. **Stdlib expansion (14 fixes, ~3 days):**
   Extend `core/async/channel`, `core/sync/*` with missing methods
   (`send_timeout`, `try_recv_timeout`, `CircuitBreaker`, etc.) per
   spec in docs/detailed/14-async.md.

2. **Test refresh (18 fixes, ~1 day):**
   Batch-update test files to use current context-system API
   (`using [...]`, `provide [...]`) instead of the pre-2026 surface.

3. **Type-checker gaps (9 fixes, multi-week):**
   Refer to T1-F (refinement runtime), T1-R (model verification),
   and new tasks for deadlock analysis + supervisor semantics.

---

Total expected duration for L2 async → 100 %: 5-7 development days
distributed across stdlib, tests, and type-system work. Each category
can land independently.
