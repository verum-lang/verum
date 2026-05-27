# `core.sys.no_runtime` — implementation audit

## Status: **complete** (under `--interp`; user-space-reachable surface)

* This module ships the async-to-sync stubs that replace the full
  Verum runtime on `@cfg(runtime = "none")` or `@cfg(runtime = "embedded")`
  targets. The types themselves are NOT @cfg-gated (the gate sits at
  the umbrella module declaration in `core/sys/mod.vr`), so the
  conformance suite reaches them through direct file-level mount
  (`mount core.sys.no_runtime.X`).
* Public API:
  - `block_on<T>(future: T) -> T` — identity transformation.
  - `spawn_sync<T>(task: fn() -> T) -> T` — inline execution.
  - `SyncChannel<T>` — `Empty | Full(T)` two-state buffer.
    - `.new()`, `.send(value)`, `.recv()`, `.is_empty()`.
  - `NoOpMutex<T>` — single-threaded mutex wrapper.
    - `.new(value)`, `.lock()`, `.unlock()`.

## 1. Cross-stdlib usage

`core.sys.no_runtime` is reached only under embedded / no-runtime
builds, where it replaces the async executor + channel/mutex layers
of the standard runtime. Under those modes:

| Caller | Substitution |
|---|---|
| async-block syntax (`async { … }`) | strips `async` modifier; bodies compile as sync. |
| `.await` expressions | identity (the future is already a sync value). |
| `spawn(task)` | rewrites to `task()` via `spawn_sync`. |
| `Channel<T>` | substituted with `SyncChannel<T>`. |
| `Mutex<T>` | substituted with `NoOpMutex<T>`. |

## 2. Action items landed in this branch

1. `unit_test.vr` — 18 `@test`s pinning the complete user-space surface:
   `block_on` identity over Int / Text / Bool / negative-Int; `spawn_sync`
   inline execution over 3 function shapes; SyncChannel 7-test state-
   machine sweep (Empty after new; send→Full; send-on-Full fails; recv-
   on-Empty is None; round-trip Int + Text; cycle preserves invariants);
   NoOpMutex 4-test construction + lock + unlock cycles.
2. `property_test.vr` — 13 algebraic laws sweeping: block_on identity
   over Int.max/min boundaries; spawn_sync return-value preservation;
   SyncChannel state-machine laws (new is Empty; send-on-Empty succeeds
   over Int sample set; send-on-Full fails; recv-on-Empty is None;
   round-trip identity for arbitrary Int payload; recv clears state;
   N-cycle alternation returns to Empty); NoOpMutex payload-identity
   sweep over Int domain + lock/unlock-preserves-value pin.
3. `integration_test.vr` — 8 cross-stdlib scenarios composing:
   SyncChannel producer/consumer pipeline draining a List<Int> into an
   accumulator + collecting received values back into List<Int>;
   block_on funneled through Maybe<Int> + Result<Int, Text>;
   spawn_sync fleet pattern summing 4 closures; NoOpMutex<IntPair>
   custom-record payload + lock/unlock value-preserved round-trip;
   8-cycle retransmit pattern with send/recv count invariants.
4. `regression_test.vr` — 4 `@test`s pinning known defect classes:
   send-on-Full preserves existing payload (no clobber); recv clears
   state to Empty; block_on returns argument (not Unit); NoOpMutex
   captures payload byte-identical.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `@cfg(runtime = "none")` build-mode exercise | The umbrella `core.sys.no_runtime` mount in `core/sys/mod.vr` is gated by `@cfg(runtime = "none")` or `@cfg(runtime = "embedded")`. Tests above reach the types via direct file mount (which bypasses the gate). Verifying the gated mount path requires a `@cfg(runtime = "none")` test build — tracked in `vcs/specs/L0-critical/`. |
| 2 | Compile-time `select!` substitution rejection | Per module header: `select { ... } => first-ready poll (compile-time error if > 1)`. Needs a `@expected-error` vcs spec. |
| 3 | Compile-time `channels => error` rejection on multi-threaded shapes | Same — needs `@expected-error` vcs spec. |
