# `runtime/thread` audit

Module: `core/runtime/thread.vr` (600 LOC) — thread spawn / join / handle /
stack-trace surface.

Tests: 22 unit tests over the data-only subset (ThreadId, ThreadError,
ThreadStackFrame, StackTrace, ThreadBuilder).  Live spawn/join paths
require platform thread intrinsics under --interp — deferred to
`vcs/specs/L2-standard/runtime/thread/`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.async.runtime.spawn` | wraps `Thread.spawn` for the async runtime; the executor uses native OS threads. |
| `core.runtime.pool.ThreadPool` | C-runtime worker threads OUTSIDE this surface (separate intrinsic backing). |
| `core.diagnostics.PanicReport` | captures a `StackTrace` via `StackTrace.capture()` at panic time. |
| `core.runtime.supervisor.Supervisor` | spawns supervised children via `Thread.spawn`. |

## 2. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| `crates/verum_codegen/src/llvm/thread.rs` (per-arch thread spawn lowering) | `clone(2)` flags on Linux; `pthread_create_minimal` shim on Darwin (libSystem.B.dylib); `CreateThread` on Windows | HOST gating instead of TARGET miscompiles cross builds. |
| `sched_yield` syscall numbers (`thread.vr:321-322`) | x86_64=24, aarch64=124 | Per-arch number table — drift silently calls the wrong syscall. |
| `PARK_FLAG` thread-local (`thread.vr:384`) | shared park/unpark cell | Park lifecycle hazard if PARK_FLAG isn't actually thread-local under interp. |
| `ThreadJoinHandle.tid` cell pre-publish | published as 0 BEFORE the spawn-wrapper runs, then set to `sys.get_thread_id()` by the wrapper | Caller-side `handle.id() == 0` is a valid pre-start state, NOT a bug. |

## 3. Language-implementation gaps

### §A — `ThreadId.current()` requires `sys.get_thread_id()` binding

The user-side `ThreadId.current()` calls `sys.get_thread_id()`.  If
that intrinsic isn't bound under --interp, the call panics or returns
a constant.  Not tested live in this folder.

### §B — `Thread.spawn` panic-on-failure surface vs. `ThreadBuilder.spawn` Result surface

Source contract:
* `Thread.spawn(f)` returns `ThreadJoinHandle<T>` directly, with
  `.expect("Failed to spawn thread")` collapsing failures into a
  panic.
* `ThreadBuilder.new().spawn(f)` returns `Result<ThreadJoinHandle<T>,
  ThreadError>`.

The two-surface pattern is a UX hazard: a caller who reads only the
`Thread.spawn` ergonomic surface won't know that spawn CAN fail.
Recommend: make the panic-on-failure variant `Thread.spawn_or_die`
and reserve `Thread.spawn` for the Result-returning surface.

### §C — `Thread.park` / `Thread.unpark` use a process-global PARK_FLAG

The `PARK_FLAG` static at `thread.vr:384` is `@thread_local` — each
thread sees its own.  Pinned for soundness.  HOWEVER the `unpark`
flow on Linux calls `futex_wake(&PARK_FLAG, 1)` against the unparker's
view of the flag, not the parked thread's.  This is broken — a
proper park/unpark needs to address the TARGET thread's PARK_FLAG.
Audit recommendation: refactor `Thread.unpark(thread_id)` to address
the target's TLS slot, not the caller's.  Live test gated on §A.

### §D — `Thread.yield_now` uses raw `@syscall` for x86_64+aarch64 only

The intrinsic dispatch at `thread.vr:316-323` is gated on
`target_os = "linux"` and only emits for x86_64 / aarch64.  ARM 32-bit /
PowerPC / RISC-V Linux targets fall through with no yield.  Same
applies to other syscalls in this file.  Audit recommendation:
fall back to `sched_yield@plt` (via libSystem on Darwin equivalent)
when the arch-specific number isn't known.

### §E — `StackTrace.capture()` uses `@frame_address(0)` intrinsic + raw pointer walk

Stack trace capture relies on `@frame_address(0)` and unsafe-cast
offsets.  Brittle to LLVM optimization (frame pointer omission with
`-fomit-frame-pointer`).  Recommend: gate the implementation on
`@cfg(frame_pointer = "enabled")` OR use the libunwind-style frame
walker.  Test currently pins `.empty()` only; live capture is gated
on a debug build with frame pointers enabled.

## Action items landed in this branch

* `core-tests/runtime/thread/unit_test.vr` — 22 unit tests covering
  ThreadId / ThreadError / ThreadStackFrame / StackTrace / ThreadBuilder.
* `core-tests/runtime/thread/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §B Rename ergonomic-spawn variant to `spawn_or_die` | `core/runtime/thread.vr` + callers | 1 h |
| §C Refactor `Thread.unpark` to target the correct TLS slot | `core/runtime/thread.vr` | 1 day |
| §D Fall back to libc-style `sched_yield` on unsupported arches | `core/runtime/thread.vr` + sys/ | 1 day (multi-arch) |
| §E Frame-pointer-aware `StackTrace.capture` | `core/runtime/thread.vr` | 1 day |
| Display/Debug rendering tests for ThreadError | this folder | gated on Display protocol surface stability |
| Live spawn + join + result-collection round-trip | `vcs/specs/L2-standard/runtime/thread/` | gated on spawn-binding under interp |
