# `runtime/mod` audit

Module: `core/runtime/mod.vr` (643 LOC) — runtime module root.
Re-exports the public surface from 9 submodules under a single
`core.runtime.*` namespace.

Tests: 11 integration tests verifying umbrella re-exports work and
resolve to the same types as the qualified-form submodule names.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| Every `core/*` module that touches the runtime | `mount core.runtime.*` for the canonical names. |
| User-facing apps | `runtime.init()`, `runtime.current_env()`, `runtime.shutdown()` for the lifecycle surface. |
| `@bench`-annotated functions | `Bencher.new()` + `b.iter(f)` for performance measurement. |
| VCS performance tests | `benchmark(name, f)` + `BenchmarkResult` for the standard timing surface. |
| Diagnostics / panic-report paths | `current_env()` provides the `ExecutionEnv` for panic reports. |

## 2. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| `EXEC_ENV_SLOT = 0` (`mod.vr:255`) | TLS slot reserved for ExecutionEnv | Drift breaks every `current_env()` call. |
| `SUPERVISOR_SLOT = 1` (`mod.vr:258`) | TLS slot reserved for current supervisor | Same. |
| `RECOVERY_SLOT = 2` (`mod.vr:261`) | TLS slot reserved for recovery context | Same. |
| `static mut ENV_ID_COUNTER: UInt64 = 0` (`mod.vr:407`) | global runtime instance counter | Process-wide; first runtime gets id 1. |
| `detect_cbgr_tier()` per-cfg gates | maps build config → ExecutionTier | Same drift class as task-queue's @cfg gating: ambiguity if multiple cfgs apply. |

## 3. Language-implementation gaps

### §A — `current_env() / current_env_mut()` panic-on-uninitialised

Source contract:
```
ctx_get(EXEC_ENV_SLOT)
    .map(|ptr| unsafe { ptr as &ExecutionEnv })
    .expect("ExecutionEnv not initialized - call runtime.init() first")
```

Returns a fresh-from-the-allocator panic message — every consumer
that forgets to call `runtime.init()` first crashes here.
Recommend: surface `try_current_env() -> Maybe<&ExecutionEnv>` for
the non-panic surface so callers can lazy-init.

### §B — `init_with<R: RuntimeConfig>()` cannot be called multiple times

The dispatcher at `init()` (`mod.vr:312-346`) doesn't check
`is_initialized()` first.  A second `init()` call would silently
overwrite the EXEC_ENV_SLOT, leaking the prior runtime.  Recommend:
return `Err(InitError.AlreadyInitialized)` if `is_initialized()`.

### §C — `shutdown()` early-returns without warning if not initialised

```
public fn shutdown() {
    if !is_initialized() {
        return;
    }
    ...
}
```

Silent no-op on `shutdown()` before `init()` — defensive but hides
caller-side ordering bugs.  Recommend: surface a `try_shutdown()
-> Result<(), ShutdownError>` for callers that want to detect the
"never initialised" state.

### §D — `next_env_id()` `unsafe { ENV_ID_COUNTER = ENV_ID_COUNTER + 1; }`

The increment is not atomic.  Two concurrent threads calling
`init()` could collide on the same ID.  Recommend: use `AtomicU64`
with `fetch_add(1, SeqCst)`.

### §E — `Runtime.current_epoch() -> UInt32` returns 0 unconditionally

Source (`mod.vr:512-522`): the body explicitly returns 0 even when
initialised, with a comment "// surface a summary value".  This is
a stub.  The CBGR epoch lives at `state.cbgr_epoch` (interpreter)
or in per-allocation headers (AOT); a proper `current_epoch()`
implementation needs to read those.  Same defect class as
[[runtime/cbgr §A]].

### §F — `Runtime.memory_usage() -> Int` returns 0 unconditionally

Stub.  Should call into the allocator's introspection surface
(`Alloc.total_allocated`).

### §G — `Bencher.iter<F>` doesn't warm up + doesn't drop outliers

Standard benchmarking practice runs a warm-up phase and discards
outliers (3-sigma).  Bencher does neither — pure mean across
N iterations.  For micro-benchmarks (sub-100ns) this surfaces
~30% jitter.  Recommend: add `with_warmup(N)` builder method.

## Action items landed in this branch

* `core-tests/runtime/mod/integration_test.vr` — 11 integration
  tests covering umbrella re-exports + qualified-form identity.
* `core-tests/runtime/mod/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §A `try_current_env()` non-panic surface | `core/runtime/mod.vr` | 30 min |
| §B Double-init guard in `init()` | `core/runtime/mod.vr` | 30 min |
| §C `try_shutdown()` Result surface | `core/runtime/mod.vr` | 30 min |
| §D Atomic ENV_ID_COUNTER | `core/runtime/mod.vr` | 30 min |
| §E `Runtime.current_epoch()` proper implementation | `core/runtime/mod.vr` + interpreter binding | 1 day (shared root with cbgr §A) |
| §F `Runtime.memory_usage()` proper implementation | `core/runtime/mod.vr` + Alloc protocol | 1 day |
| §G `Bencher` warmup + outlier elimination | `core/runtime/mod.vr` | 2 h |
| Live `init() → current_env() → shutdown()` lifecycle test | `vcs/specs/L2-standard/runtime/mod/` | gated on runtime intrinsics |
| Cross-cfg test (verify only ONE RuntimeConfig matches per build) | `vcs/specs/L2-standard/runtime/mod/` | 1 h |
