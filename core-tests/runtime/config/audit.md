# `runtime/config` audit

Module: `core/runtime/config.vr` (1208 LOC) — RuntimeConfig protocol +
4 build-target implementations (FullRuntime / SingleThreadRuntime /
SyncRuntime / EmbeddedRuntime) + CustomRuntime + executor implementations
(NoopExecutor / WorkStealingExecutor / SingleThreadExecutor) + I/O
driver protocol (RuntimeIoDriver + NoopDriver) + IoOp/IoHandle/
IoCompletion + RuntimeInitError/RuntimeIoError ADTs.

Tests: 30 unit tests over the data-only subset.  Live RuntimeConfig
implementations (.init() / .shutdown() / executor handles / I/O driver
methods) require platform thread+I/O intrinsics under --interp —
deferred to `vcs/specs/L2-standard/runtime/config/`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.runtime.mod.init()` | dispatches to `FullRuntime.init()` / `SingleThreadRuntime.init()` / `SyncRuntime.init()` / `EmbeddedRuntime.init()` per `@cfg(runtime = ...)`. |
| `core.async.spawn` | uses `RuntimeConfig.executor_handle()` to dispatch the spawn. |
| `core.io.engine` | uses `RuntimeConfig.io_driver_ref()` to dispatch I/O ops. |
| `core.mem.alloc` | uses `RuntimeConfig.allocator_ref()` (`TieredAllocator` / `BumpAllocator` etc.). |

## 2. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| `@cfg(runtime = "...")` gating | 4 runtime profiles (full / single_thread / no_async / no_heap / embedded) | The build target determines which RuntimeConfig is callable; cross-target test runs must respect this. |
| `@repr(UInt8)` on RuntimeInitError + RuntimeIoError | wire-format ordinal | ADT field-order drift would silently re-tag every error variant; load-bearing for any audit log that ingests these tags. |
| `@repr(C)` on IoOp | per-variant payload alignment | The kernel-facing surface (io_uring sqe / kqueue kevent / IOCP packet) reads these at fixed offsets. |
| NoopDriver returns `Err(Other(-1))` for submit/wait | sentinel-error contract on embedded | Calling code must handle the `-1` sentinel; not a typed constant — drift surface. |

## 3. Language-implementation gaps

### §A — RuntimeConfig protocol has 4 implementations but no compile-time uniqueness check

`@cfg(runtime = "full")` selects FullRuntime, `@cfg(runtime = "single_thread")`
selects SingleThreadRuntime, etc.  Each is unique under its cfg gate.
HOWEVER: if a future custom @cfg accidentally enables two profiles
simultaneously, `init()` would have ambiguous resolution at the
`core.runtime.mod.init()` dispatch.  Recommend: pin a build-time
assertion via `@cfg_must_be_exclusive(runtime)` annotation.

### §B — Magic-number sentinel `-1` in NoopDriver `Err(Other(-1))`

`NoopDriver.submit/wait` returns `Err(RuntimeIoError.Other(-1))`.
The `-1` is undocumented — a caller reading `Other(code)` has no way
to recognise the NoopDriver sentinel without knowing this magic
number.  Recommend: replace with `RuntimeIoError.PlatformNotSupported`
OR a new `RuntimeIoError.NoopDriverInert` variant.

### §C — RuntimeInitError.AlreadyInitialized races

The `RuntimeInitError.AlreadyInitialized` variant is raised when `init()`
is called on a runtime that's already initialised.  The check is not
atomic — two concurrent `init()` calls could both observe "not
initialised" and proceed.  Recommend: atomic CAS on the init-flag
in the `core.runtime.mod` dispatcher.

### §D — Eq impl on RuntimeIoError treats `Other(N)` cross-instance carefully

Source (`config.vr:368-376`) deliberately excludes `Other` from the
tag-equality fast path: `_ => runtime_io_error_tag(...) != 8`.  This
prevents two different `Other(N)` payloads from being treated as
equal when only the tag matches.  Pinned by
`test_runtime_io_error_eq_other_different_code`.

## Action items landed in this branch

* `core-tests/runtime/config/unit_test.vr` — 30 unit tests covering
  RuntimeInitError 6-variant + RuntimeIoError 9-variant + IoHandle/
  IoCompletion records + NoopExecutor/NoopDriver sentinels.
* `core-tests/runtime/config/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §A `@cfg_must_be_exclusive` annotation | language feature + `core/runtime/mod.vr` | 1 day |
| §B Replace `-1` magic with `NoopDriverInert` variant | `core/runtime/config.vr` + callers | 30 min |
| §C Atomic init-flag in `runtime.init()` | `core/runtime/mod.vr` | 1 day |
| Display/Debug rendering tests for the 2 error ADTs | this folder | 30 min |
| Live FullRuntime.init() / shutdown() cycle | `vcs/specs/L2-standard/runtime/config/` | gated on platform thread+I/O intrinsics |
| IoOp 5-variant per-payload tests | this folder | gated on safe-FD harness |
