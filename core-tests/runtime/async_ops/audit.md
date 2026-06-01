# `runtime/async_ops` audit

Module: `core/runtime/async_ops.vr` (136 LOC) — runtime-layer async
intrinsic surface + 13 opaque-handle newtypes + AsyncRecoveryError ADT.

Tests: 23 unit tests covering opaque-handle construction, cross-type
distinctness, AsyncRecoveryError record + Eq protocol.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.async.spawn` | `spawn_with_env(env, f)` returns JoinHandleOpaque. |
| `core.async.executor.Executor` | wraps `ExecutorHandle` from `default_executor()`. |
| `core.async.future.Future` | wraps `FutureHandle`; `future_poll_sync` is the executor-side fallback. |
| `core.runtime.supervisor.Supervisor` | wraps `SupervisorHandleOpaque`; `spawn_supervised` returns JoinHandleOpaque. |
| `core.runtime.recovery.{RecoveryRetryPolicy, RecoveryCircuitBreaker}` | use AsyncRecoveryError to surface failures past the `exec_with_recovery` boundary. |
| `core.mem.alloc.{Heap, Arena}` | wraps `AllocHandle` from `global_allocator()`. |
| `core.io.engine.{IoEngine}` | wraps `IODriverHandle` from `default_io_driver()`. |

`grep -r "JoinHandleOpaque\|ExecutorHandle\|FutureHandle" core/` returns
~10 sites in `core/async/*` and `core/runtime/supervisor.vr`.

## 2. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| `crates/verum_vbc/src/interpreter/dispatch_table/handlers/spawn_extended.rs` | `spawn_with_env` / `executor_spawn` dispatch | The spawn surface is gated on stdlib AOT build (task #7). |
| `crates/verum_codegen/src/llvm/spawn.rs` | AOT spawn lowering (thread creation + parent-context fork) | Same task #7 gate. |
| AsyncRecoveryError ABI | 1-Text-field record; `Display`/`Debug`/`Eq` impls | Drift here changes the error-surface wire format. |

## 3. Language-implementation gaps

### §A — opaque-handle newtypes don't enforce type distinctness at the FFI boundary

The 13 opaque newtypes (`JoinHandleOpaque`, `ExecutorHandle`, ...) all
wrap a bare `Int`.  At the type-checker level they ARE distinct (the
`test_opaque_handles_are_distinct_types` test pins this).  At the
ABI / wire-format level they are interchangeable.  A caller bug that
passes an `AllocHandle` where `ExecutionEnvOpaque` is expected would
be caught by the typechecker; a caller bug that passes a raw `Int`
cast via `as ExecutionEnvOpaque` would NOT be caught.

Recommend: add `@no_int_cast` annotation (when the language supports
it) to gate the implicit Int → handle conversion behind explicit
`unsafe`.

### §B — AsyncRecoveryError lacks an error-kind discriminator

Single-field `message: Text` ADT.  Downstream consumers can't
programmatically distinguish "timeout" vs "circuit-open" vs
"poisoned" without parsing the message string.  Recommend:
add a `kind: AsyncRecoveryErrorKind` field with 5..8 variants
(Timeout / CircuitOpen / Cancelled / Panicked / Poisoned / Other(Text))
to align with the structured-error discipline established at
`core.cache.types.CacheError` / `core.storage.types.StorageError`.

### §C — Live spawn / future-poll path requires task #7 (AOT stdlib build)

The intrinsic call paths (`spawn_with_env`, `executor_spawn`,
`future_poll_sync`, `executor_block_on`, etc.) cannot be exercised
under --interp today.  Live tests live at
`vcs/specs/L2-standard/async/` and need the AOT stdlib build to
land first.  In-scope for this folder: data-only surface coverage.

### §D — Transparent opaque-newtype inner extraction does not round-trip

`type JoinHandleOpaque is (Int)` (and the 12 sibling opaque handles) are
runtime-TRANSPARENT (see §A: distinctness is typechecker-only). Extracting
the wrapped Int via `match h { JoinHandleOpaque(v) => v }` does **not**
round-trip under `--interp`: `JoinHandleOpaque(42)` then matched yields a
value `!= 42`. This is the same `__newtype_inner_*` transparent-wrapper
access gap recorded for `sys/common` `FileDesc` (archive-loaded
transparent-wrapper const-access). By design these handles are opaque
tokens handed to the runtime intrinsics, never inspected by user code, so
the gap has no functional impact today — but the inner-extraction surface
is unsound and pinned `@ignore`:
`ao_law_{join,executor,future}_handle_inner_value`.

**Fix surface (compiler, needs rebuild):** newtype/tuple-struct inner
binding in pattern-match codegen must read the wrapped scalar, not the
header, for transparent single-field wrappers. Shares root with the
`__newtype_inner` typechecker gap.

## Action items landed in this branch

* `core-tests/runtime/async_ops/unit_test.vr` — 23 unit tests covering
  opaque-handle construction + cross-type distinctness + AsyncRecoveryError.
* `core-tests/runtime/async_ops/property_test.vr` — 10 law tests; 7 GREEN
  (AsyncRecoveryError Eq reflexive/symmetric/payload-sensitive + message
  round-trip + **record Display==message + Debug** — confirms the §H Display
  gap is nullary-enum-specific, records dispatch Display fine); 3 `@ignore`
  on §D (opaque-newtype inner extraction). 2026-06-01.
* `core-tests/runtime/async_ops/audit.md` — this file (§D added).

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §A `@no_int_cast` annotation on opaque handles | language feature + `core/runtime/async_ops.vr` | gated on language feature |
| §B AsyncRecoveryErrorKind discriminator | `core/runtime/async_ops.vr` + all callers | 1 day (cross-cutting) |
| §C Live spawn / await round-trip tests | `vcs/specs/L2-standard/async/` | gated on task #7 |
| Display/Debug rendering tests | this folder | 30 min — gated on Display protocol surface stability |
