# `core.sys.process_ops` — implementation audit

## Status: **partial** (raw-intrinsic mount migration landed; process spawn surface deferred)

* Every public ADT (`ProcessExitStatus`, `Child`) is covered by
  `unit_test.vr` at the construction + accessor layer.
* `success()` / `code()` predicates pinned with the full {-1, 0, 1, 2,
  137, 255} spread.
* `args()` / `arg_count()` coherence pinned end-to-end.
* The **process-spawn surface** (`spawn` / `run` of a real child) is
  **deferred** — it would require either (a) a fixture binary in
  `core-tests/sys/process_ops/fixtures/` that the test runner can
  `exec`, or (b) tests against `/bin/true` / `/usr/bin/false` which
  may not exist on every CI runner. Tracked as a follow-up.

## 1. Cross-stdlib usage

`core.sys.process_ops` is the canonical process-spawn shim. Consumers:

| Consumer | Touches | Notes |
|---|---|---|
| `core/base/env.vr` | `arg_count` / `arg_unchecked` | safe `arg(i) -> Maybe<Text>` wrapper. |
| `core/cli/*` | `args` | argv-driven CLI parser. |

## 2. Crate-side hardcodes

| Site | What it pins | Status |
|---|---|---|
| `crates/verum_vbc/src/interpreter/dispatch_table/handlers/calls.rs` | `__args_count_raw` / `__arg_raw` argv read intrinsics | OK |

## 3. Language-implementation gaps surfaced by this suite

### 3.1 Stale `super.raw.*` mount (CLOSED in this branch)

* Same architectural defect as the one closed for `time_ops.vr`,
  `context_ops.vr`, `file_ops.vr`, and `net_ops.vr`.
* **Status**: **CLOSED** in this branch. Pinned by `regression_test.vr` §A.

## 4. Action items landed in this branch

1. **Fundamental fix**: replaced `mount super.raw.*` in
   `core/sys/process_ops.vr` with the canonical
   `mount core.intrinsics.runtime.os.{__process_*_raw, __fd_*_raw,
   __args_count_raw, __arg_raw}`.
2. `unit_test.vr` — 8 `@test`s.
3. `property_test.vr` — 5 algebraic-law `@test`s pinning the
   success ↔ code==0 partitioning and the args/arg_count coherence.
4. `integration_test.vr` — 3 `@test`s.
5. `regression_test.vr` — 5 `@test`s.

## 5. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `spawn` / `run` of a real child | Needs CI-portable fixture. |
| 2 | `Child.read_stdout` data-loss class | Tracked at `core-tests/sys/time_ops/audit.md` §C — the wrapper's `__fd_read_all_raw` path silently returns "" when the child has unflushed buffered output. Same defect surfaces here. |
