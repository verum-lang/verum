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

### §H — Display dispatch falls through for NULLARY enum variants (cross-module)

**Surface:** `f"{x}"` (→ `format_display(&x)`) does **not** dispatch the
user `Display` impl for *nullary* enum variants — it falls through to the
Debug/variant-name path. It works correctly for variants *with payloads*.
Empirically, in this module:

| expression | expected | result |
|---|---|---|
| `f"{RuntimeIoError.Other(42)}"` (payload) | `I/O error (code 42)` | ✅ correct (`cfg_law_io_error_other_display_includes_code` GREEN) |
| `f"{RuntimeIoError.WouldBlock}"` (nullary) | `operation would block` | ❌ wrong (`cfg_law_io_error_display_strings` @ignore) |
| `f"{RuntimeInitError.TlsInitFailed}"` (nullary) | `Failed to…` | ❌ wrong (`cfg_law_init_error_display_equals_message` @ignore) |

`f"{x:?}"` Debug works for nullary variants (`cfg_law_io_error_debug_strings`
GREEN) and `Eq` / `is` work too — so the gap is specific to `format_display`
on the bare-tag (no heap object, no `type_id`) representation of a nullary
variant losing the `Display` impl lookup. Same family as the recovery
module's §H (`RecoveryCircuitState`).

**DOWNSTREAM FUNCTIONAL BUG (not just a formatting nit):**
`runtime.recovery.execute_with_retry` does `let error_msg = f"{error}";`
then `is_transient_error(&error_msg)`. For a nullary `RuntimeIoError`
(`WouldBlock`, `TimedOut`, `Interrupted`, `ConnectionReset`, …) the broken
Display yields the Debug form — e.g. `"WouldBlock"` lowercases to
`"wouldblock"`, which does **not** contain the substring `"would block"`,
so `is_transient_error` returns **false** and the retry layer silently
abandons a retriable I/O failure. Pinned by the `@ignore`d
`cfg_it_transient_io_errors_recognised_by_recovery`; the cross-module
contract is still verified Display-independently via
`cfg_it_transient_contract_strings` (GREEN).

**Fix surface (compiler, needs rebuild):** `format_display` must dispatch
the `Display` impl for nullary variants the way `format_debug` already
does — give nullary variants a type-tagged representation at the
interpolation lowering / VBC dispatch site. Tracked as task #6.

## Action items landed in this branch

* `core-tests/runtime/config/unit_test.vr` — 30 unit tests covering
  RuntimeInitError 6-variant + RuntimeIoError 9-variant + IoHandle/
  IoCompletion records + NoopExecutor/NoopDriver sentinels.
* `core-tests/runtime/config/property_test.vr` — 11 behavioural-law tests
  (message() exact strings + interpolation, Display(payload) string, Debug
  strings, Eq reflexive/payload-sensitive/cross-variant). 9 GREEN; 2
  `@ignore` on §H (nullary Display).
* `core-tests/runtime/config/integration_test.vr` — 8 cross-type/cross-module
  tests (IoCompletion Result wrapping, collections, transient-classifier
  contract agreement with recovery). 7 GREEN; 1 `@ignore` on §H.
* `core-tests/runtime/config/audit.md` — this file (§H added).

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §A `@cfg_must_be_exclusive` annotation | language feature + `core/runtime/mod.vr` | 1 day |
| §B Replace `-1` magic with `NoopDriverInert` variant | `core/runtime/config.vr` + callers | 30 min |
| §C Atomic init-flag in `runtime.init()` | `core/runtime/mod.vr` | 1 day |
| Display/Debug rendering tests for the 2 error ADTs | this folder | 30 min |
| Live FullRuntime.init() / shutdown() cycle | `vcs/specs/L2-standard/runtime/config/` | gated on platform thread+I/O intrinsics |
| IoOp 5-variant per-payload tests | this folder | gated on safe-FD harness |
