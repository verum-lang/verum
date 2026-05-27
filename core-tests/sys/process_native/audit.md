# `core.sys.process_native` — implementation audit

## Status: **partial** (data-shape complete; live syscall surface needs fixture)

* This module implements the fork-safe `pipe + fork + dup2 + execve`
  flow with no C-runtime dependency. The full public surface is:
  `SpawnResult`, `native_spawn`, `native_kill`, `native_fd_write_all`,
  `native_fd_read_chunk`.
* What IS conformance-tested here from the in-process harness:
  - `SpawnResult` record construction + every-field projection identity
    across the 8 standard capture configurations (2³ Bool triples).
  - Sentinel disjointness: -1 != n for every valid fd n >= 0.
  - The fork(2) pid trichotomy (< 0 = error, = 0 = child, > 0 = parent).
  - SpawnResult × List<SpawnResult> fleet iteration patterns.
  - SpawnResult × Result<SpawnResult, Text> error funnel (mirror of
    native_spawn's actual return shape).
  - Capture-mode dispatch table via a custom 4-variant ADT.
* What is OUT OF SCOPE for `--interp` conformance:
  - Live `native_spawn` — needs a CI-portable fixture binary (e.g.
    a tiny pre-built `/usr/bin/true`-equivalent compiled by the build
    pipeline). Tracked in `core-tests/integration/process_native/`.
  - `native_kill` — needs a live child PID to signal.
  - `native_fd_write_all` / `native_fd_read_chunk` — need live pipe
    fds, which only `native_spawn` can produce.

## 1. Cross-stdlib usage

`core.sys.process_native` is the lowest layer of the spawn stack.
Consumers:

| Caller | Path |
|---|---|
| `core.io.process` | High-level `Process` API + async-friendly streams. |
| `core.shell.exec` | Pipeline construction (`a \| b \| c`). |
| `core.sys.process_ops` | `spawn(program, args)` adapter — convenience surface. |

## 2. Action items landed in this branch

1. `unit_test.vr` — 8 `@test`s pinning SpawnResult record construction
   + field-access round-trip across the 8 standard capture configs.
2. `property_test.vr` — 6 `@test`s sweeping field-projection identity
   per-field; exhaustive 8-case capture configuration sweep; sentinel
   disjointness; fork(2) pid trichotomy.
3. `integration_test.vr` — 6 cross-stdlib scenarios composing SpawnResult
   with List<SpawnResult> fleet iteration, Result<SpawnResult, Text>
   error funnel, a CaptureMode dispatch table, and Maybe<SpawnResult>
   lift.
4. `regression_test.vr` — 2 `@test`s pinning (a) the `-1` sentinel
   on `stdin_fd / stdout_fd / stderr_fd` and (b) the negative-pid error
   path on the `pid` field. Both require the Int (signed) typing.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Live `native_spawn` round-trip | Needs CI-portable fixture binary. Belongs in `core-tests/integration/process_native/`. |
| 2 | `native_kill` signal-delivery semantics | Needs live child PID + signal-trap observation. |
| 3 | `native_fd_write_all` retry-on-EINTR contract | Needs a synthetic pipe-fd with deterministic short-write injection. |
| 4 | `native_fd_read_chunk` bounded-read | Needs synthetic pipe-fd. |
| 5 | Windows `native_spawn` implementation | Currently `Err("not implemented")`. Tracked in `core/sys/process_native.vr` cfg-gated block. |
