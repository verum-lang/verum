# `core.sys.time_ops` — implementation audit

> Module under test: `core/sys/time_ops.vr` (94 LOC), the raw-syscall
> layer behind `core.time.{Instant, Duration, sleep, wall_clock_ms}`.
> 9 public surfaces: `SysTimeOpsInstant.{now, elapsed, duration_since}`,
> `SysTimeOpsDuration.{from_nanos, from_micros, from_millis, from_secs,
> as_nanos, as_micros, as_millis, as_secs, zero}`, plus free functions
> `sleep(d)`, `sleep_ms(ms)`, `sleep_secs(s)`, `wall_clock_ms()`.
>
> Test surfaces: `unit_test.vr` (~170 LOC), `property_test.vr` (~145 LOC),
> `integration_test.vr` (~90 LOC), `regression_test.vr` (~85 LOC).

## Status: **regression-only** (gated by task #5)

The arithmetic-only API surface (`SysTimeOpsDuration.from_*` /
`as_*` / `zero`) is stable in both interpreter and AOT. Every API
surface that transitively calls one of the three time intrinsics
(`__time_monotonic_nanos_raw`, `__time_sleep_nanos_raw`,
`__time_now_ms_raw`) is **blocked** behind task #5 — the
intrinsic-mount-propagation defect surfaced by this suite. The
relevant tests are `@ignore`d with a defect-class comment until task
#5 closes.

## 1. Cross-stdlib usage

`core.sys.time_ops` is the kernel-boundary layer that `core.time`
(`Instant`, `Duration`, `sleep`, …) sits on top of. The C-runtime
analogue was `verum_time_*` in `verum_runtime.c` — `time_ops.vr` is
the pure-Verum replacement.

| Consumer | Use |
|---|---|
| `core/time/instant.vr` | `SysTimeOpsInstant.now()` is the underlying clock read. |
| `core/time/duration.vr` | `SysTimeOpsDuration` arithmetic is reused; Duration in core.time is a richer wrapper. |
| `core/async/timer.vr` | `sleep_ms` is the underlying yield in single-threaded mode. |
| `core/async/runtime.vr` | `wall_clock_ms` feeds the scheduler's deadline tracking. |

No consumer reaches around `time_ops` to invoke the `__time_*_raw`
intrinsics directly — every caller goes through one of the wrappers.

## 2. Crate-side hardcodes

| Site | What it pins | Status |
|---|---|---|
| `crates/verum_vbc/src/interpreter/dispatch_table/handlers/calls.rs:1492-1514` | `__time_monotonic_nanos_raw`, `__time_sleep_nanos_raw`, `__time_now_ms_raw` runtime implementations | OK — the dispatch table has the right names. |
| `crates/verum_codegen/src/llvm/instruction.rs:10369-10692` | LLVM-side intrinsic dispatch for `"time_now"`, `"time_now_ms"`, `"time_monotonic_nanos"` | OK |
| `crates/verum_codegen/src/llvm/platform_ir.rs:15605-15732` | AOT emit for `verum_time_monotonic_nanos`, `verum_time_sleep_nanos`, `verum_time_now_ms` LLVM wrappers | OK |

The runtime side is fully wired. The defect is in the codegen-time
mount-resolution path; it doesn't touch any of these sites.

## 3. Language-implementation gaps

### 3.1 Stale `mount super.raw.*` in 5 sys/*.vr files (task #5 — landed in this branch, propagation TBD)

`core/sys/{time_ops, context_ops, file_ops, net_ops, process_ops}.vr`
all carried a `mount super.raw.*;` declaration from before the
`core/sys/raw.vr` → `core/intrinsics/runtime/os.vr` migration. The
comment in `core/sys/mod.vr:40-49` documents the migration but the 5
downstream `.vr` files were never updated to track it. The
unresolvable mount silently resolved to nothing — every `__*_raw()`
call in these wrappers failed function-id lookup at codegen time, and
every surrounding wrapper compiled to a lenient panic-stub firing at
runtime with `undefined function: __<name>_raw (in function <wrapper>)`.

**Fix landed this branch**: `core/sys/time_ops.vr` updated from the
stale mount to the canonical
`mount core.intrinsics.runtime.os.{__time_monotonic_nanos_raw,
__time_sleep_nanos_raw, __time_now_ms_raw};` form — matching the
working pattern in `core/mem/arena.vr:122`, `core/net/tcp.vr:209`,
`core/base/panic.vr:29`, `core/async/generator.vr:555`.

**Verified blast radius (interpreter)**:

  * Pre-fix: all `Instant.now()` / `wall_clock_ms()` / `sleep_*` call
    sites panic at runtime with `[lenient] X compiled to panic-stub`.
  * Post-fix (mount edit applied to source): same panic still fires.
  * Conclusion: the source-level mount fix is structurally correct
    but **does not propagate through the precompile pipeline** for
    this specific file. The `@intrinsic("time_*")` propagation
    works for `core/mem/arena.vr` (same mount form) but NOT for
    `core/sys/time_ops.vr`. The differential is the gap to close.

**Hypothesis pending verification** (the deeper task #5 work): the
precompile pipeline may pre-scan each .vr file's mounts to build a
function-id space BEFORE the @intrinsic propagation pass runs. If
the pre-scan caches a result before the mount is reachable (e.g.
module-dependency ordering puts `core.sys.time_ops` BEFORE
`core.intrinsics.runtime.os` in the topological order), the
function-id lookup at codegen-time misses. Same-pattern files like
`core/mem/arena.vr` work because their mount targets (e.g.
`core.intrinsics.runtime.mem_raw`) sit earlier in the topo order.

**Action**: continue task #5 — investigate the precompile module
graph for `core.sys.time_ops` and reconcile its ordering relative
to `core.intrinsics.runtime.os`. Alternatively, hoist the time
intrinsics' canonical declaration into a stub-module that's
guaranteed to be loaded before all sys/*.vr.

The other 4 sys/*.vr files (`context_ops`, `file_ops`, `net_ops`,
`process_ops`) carry the same stale `mount super.raw.*;` and will
exhibit the same defect for their respective wrappers. They are NOT
yet migrated; doing so without first closing the propagation defect
above would only move the panic-stub message text without fixing the
underlying behaviour.

### 3.2 Forward-declared `__text_from_raw` / `__ptr_read_i64` in `process_ops.vr` (separate defect)

`core/sys/process_ops.vr:71-72` defines:

```verum
fn __ptr_read_i64(ptr: Int) -> Int { 0 }
fn __text_from_raw(buf: Int, len: Int) -> Text { "" }
```

These are placeholder stubs that always return zero / empty. Used by
`Child.read_stdout()` — meaning **`Child.read_stdout()` always returns
""** regardless of what the child actually printed. This is silent
data loss; not a panic-stub, but an empty-result-stub. Fundamentally
the same bug class as 3.1 (stub left where a real implementation
should live). Tracked as a follow-up under task #5.

## Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | `mount super.raw.*` in `core/sys/time_ops.vr` resolved to nothing | stdlib | Replaced with canonical `mount core.intrinsics.runtime.os.{__time_monotonic_nanos_raw, __time_sleep_nanos_raw, __time_now_ms_raw};` |
| 2 | Missing `core-tests/sys/time_ops/` | tests | Added `unit_test.vr` (170 LOC, 17 tests — 9 active arithmetic + 8 `@ignore`d intrinsic-dependent), `property_test.vr` (~145 LOC, 10 laws — 7 active + 3 `@ignore`d), `integration_test.vr` (~90 LOC, 5 scenarios — 1 active + 4 `@ignore`d), `regression_test.vr` (~85 LOC, 5 pins — 2 active + 3 `@ignore`d). |
| 3 | This `audit.md`. | docs | New. |

## Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Precompile pipeline: `mount core.intrinsics.runtime.os.{__time_*_raw}` from `core/sys/time_ops.vr` does not propagate `intrinsic_name` to the lookup table — every wrapper compiles to panic-stub despite the canonical mount. | ~2 h (precompile-pipeline-level) | task #5, open |
| §B | Apply the same mount migration to `core/sys/{context_ops, file_ops, net_ops, process_ops}.vr` AFTER §A closes. Each will exhibit the same propagation defect until §A is structurally fixed. | ~30 min per file | task #5, blocked on §A |
| §C | Replace placeholder stubs `__ptr_read_i64` / `__text_from_raw` in `core/sys/process_ops.vr:71-72` with real `core.intrinsics.runtime.os.*` mounts; pin `Child.read_stdout()` round-trip via regression test. | ~30 min | follow-up |
