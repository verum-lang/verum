# `runtime/pool` audit

Module: `core/runtime/pool.vr` (153 LOC) — fixed-size thread pool with
RAII Drop + 2 record types + 5 raw intrinsics (`pool_create`,
`pool_submit`, `pool_await`, `pool_destroy`, `pool_global_submit`).

Tests: 6 unit tests covering data-only surface (record construction +
field layout).  Live thread-pool path deferred until §A.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| HTTP server per-request orchestration | `ThreadPool.new(N)` per request scope, `submit` per task, RAII destroy on scope exit. |
| `core.runtime.config.FullRuntime` | uses the global pool via `ThreadPool.global_submit`. |
| Batch-job orchestrators | submit + await pattern across N tasks before continuing. |

## 2. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| C runtime `pool_create` / `pool_submit` / `pool_await` / `pool_destroy` symbols | ABI: `Int` handle + `Int(Int)` worker fn + `Int` arg, all i64 | ABI drift would silently corrupt the pool handle table. |
| Capacity constants (4096 pending tasks, 64 worker threads) | hardcoded in C runtime | Verum-side caller has no compile-time check that `num_workers <= 64`. |
| `awaited: Bool` field in PoolTaskHandle | tracked by user code, read by `Drop for PoolTaskHandle` | If a future refactor stops setting `awaited = true` in `await()`, the Drop impl double-frees. |

## 3. Language-implementation gaps

### §A — `pool_*` intrinsics not bound under --interp

Same defect class as the broader runtime-intrinsic stub family.  The
ident strings `pool_create` / `pool_submit` / `pool_await` /
`pool_destroy` / `pool_global_submit` are forward-declared via
`@intrinsic` but their dispatch handlers in
`crates/verum_vbc/src/interpreter/dispatch_table/handlers/` are not
registered.  Live tests gated on this fix.

### §B — `num_workers <= 0` defaults to 4 silently

Source contract (`pool.vr:84`): `if num_workers <= 0, defaults to 4`.
Silent default is a UX hazard — a caller passing `num_workers = 0` to
mean "no parallelism" gets 4 workers spinning instead.  Recommend:
either panic on `num_workers <= 0` OR document the silent default
prominently.

### §C — `submit` takes `fn(Int) -> Int` — only Int-shaped tasks

The pool can only accept tasks of shape `fn(Int) -> Int`.  Higher-
shaped tasks (closures over locals, generic-typed args) must be
manually marshalled through Int.  This is a deliberate runtime
constraint (the C-side worker dispatches a raw fn pointer with an
Int arg) but the docstring should call it out.

### §D — Drop-on-unawaited-handle drains the result silently

Source contract (`pool.vr:144-152`): if `Drop for PoolTaskHandle`
fires on an unawaited handle, the result is fetched and discarded.
This is correct for resource hygiene but **silent failure-handling
hazard**: a task that panicked (returns an error via the Int
encoding) gets its error swallowed.  Recommend: surface a
`logger.warn` (gated on `@cfg(debug_assertions)`) when an unawaited
handle drops with a non-zero result.

## Action items landed in this branch

* `core-tests/runtime/pool/unit_test.vr` — 6 unit tests on record
  surface.
* `core-tests/runtime/pool/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §A wire `pool_*` intrinsics in VBC | `crates/verum_vbc/src/interpreter/dispatch_table/handlers/` | 1 day |
| §B refinement type on `num_workers` (or explicit panic) | `core/runtime/pool.vr` | 30 min |
| §C document Int-only task shape | `core/runtime/pool.vr` | 15 min |
| §D debug-only Drop-on-unawaited diagnostic | `core/runtime/pool.vr` | 30 min |
| Live submit + await round-trip test | `vcs/specs/L2-standard/runtime/pool/` | gated on §A |
| Drop-on-leak test (create pool, drop without destroy, verify no leak) | this folder | gated on §A + leak detector |
