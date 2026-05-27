# `runtime/time` audit

Module: `core/runtime/time.vr` (30 LOC) — runtime-layer clock + sleep
intrinsics: 6 free fns (`monotonic_nanos`, `realtime_secs`,
`realtime_nanos`, `num_cpus`, `sleep_ms`, `sleep_ns`).

Tests: 13 unit tests over monotonicity invariants, callability, sleep
zero-arg no-op behaviour, and cross-surface coherence.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.time.instant.Instant` | `Instant.now()` calls `monotonic_nanos()`. |
| `core.time.system_time.SystemTime` | `now()` calls `realtime_secs() * 1_000_000_000 + realtime_nanos()`. |
| `core.async.timer.{sleep,sleep_until,timeout}` | the async sleep paths trampoline into `sleep_ns` under the single-threaded executor. |
| `core.runtime.pool.ThreadPool` | uses `num_cpus()` to pick a default worker count. |
| `core.runtime.config.FullRuntime` | reads `num_cpus()` for parallelism config. |

`grep -r "monotonic_nanos\|realtime_secs\|realtime_nanos\|num_cpus"
core/` returns ~30 sites.

## 2. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| `crates/verum_codegen/src/llvm/syscalls.rs` | `clock_gettime(CLOCK_MONOTONIC)` on Linux/FreeBSD; `mach_absolute_time` on Darwin (via libSystem); `QueryPerformanceCounter` on Windows (via kernel32) | Drift on any of these paths breaks `Instant.now()`. |
| Per-arch sleep encoding | `nanosleep(timespec*)` on Unix; `Sleep` (ms granularity) on Windows | Resolution drift (`sleep_ns(1)` ≥ 15ms on coarse-grained sleep) is a known platform quirk; tests above use 0-arg to dodge it. |

## 3. Language-implementation gaps

### §A — intrinsic dispatch binding (shared root with cbgr / sync / syscall)

Same root as [[runtime/cbgr §A]].  The 6 ident strings
`verum.runtime.{monotonic_nanos,realtime_secs,realtime_nanos,num_cpus,sleep_ms,sleep_ns}`
have either no handler in `crates/verum_vbc` (→ default-zero return)
OR are bound under a different ident.

Empirically (post `verum test --interp --filter` probes 2026-05-27):

| intrinsic | --interp behaviour | bound? |
|---|---|---|
| `monotonic_nanos` | returns non-negative Int | likely YES |
| `realtime_secs`   | returns non-negative Int | likely YES |
| `realtime_nanos`  | returns non-negative Int | likely YES |
| `num_cpus`        | returns **0** (fail `>= 1`) | **NO** — pinned `@ignore test_num_cpus_callable` |
| `sleep_ms(0)`     | no-op | likely YES |
| `sleep_ns(0)`     | no-op | likely YES |

`num_cpus` returning 0 means downstream consumers (work-stealing pool
worker count, FullRuntime parallelism config) silently fall back to
hardcoded defaults — the contract is violated invisibly.  Pinned at
`test_num_cpus_callable` with `@ignore`; flips when the binding lands.

Confirm the binding lives at `crates/verum_vbc/src/interpreter/dispatch_table/handlers/{time,clock}_extended.rs`
and isn't being mis-routed through the higher-level `core.time` API.
Documented contract for the user-side surface should explicitly say
which idents are runtime-bound vs which fall through to the default
intrinsic stub.

### §B — wall-clock NTP-correction back-jump pin

`realtime_secs` / `realtime_nanos` CAN jump backwards under NTP
correction (or manual `date` adjustment).  The
`test_realtime_consistent_with_monotonic_ordering` test is
deliberately weaker than `test_monotonic_nanos_monotone_non_decreasing`
to honour this.  Audit recommendation: document the contract
explicitly at `core/runtime/time.vr` — comment header for
`realtime_*` should note "NOT monotone".

### §C — `sleep_ms` / `sleep_ns` minimum granularity

Most Unix kernels round `sleep_ns(N)` up to the timer-tick granularity
(~1ms on a CONFIG_HZ=1000 kernel; ~10ms on Windows non-multimedia).
Tests here use 0-arg to avoid coupling to the granularity; any
non-zero `sleep_ns(N)` test belongs in `vcs/specs/L4-performance/`
with explicit granularity bounds.

## Action items landed in this branch

* `core-tests/runtime/time/unit_test.vr` — 13 unit tests (monotonicity,
  callability, zero-sleep, cross-surface coherence).
* `core-tests/runtime/time/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §A dispatch-binding audit | `crates/verum_vbc` | 1 h |
| §B `realtime_*` non-monotone documentation pin | `core/runtime/time.vr` | 15 min |
| Sleep-resolution per-arch property test | `vcs/specs/L4-performance/runtime/time/` | 1 h |
| Live `Instant.elapsed() > Duration.from_millis(N)` after `sleep_ms(N)` test | this folder or sister | 30 min once §A confirmed |
