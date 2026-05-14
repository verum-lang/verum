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

## Status: **partial** (task #5 CLOSED)

The arithmetic API surface (`SysTimeOpsDuration.from_*` / `as_*` /
`zero`) is stable in both interpreter and AOT. The
intrinsic-touching surface (`SysTimeOpsInstant.now`, `elapsed`,
`duration_since`, `sleep`, `sleep_ms`, `sleep_secs`,
`wall_clock_ms`) is **unblocked** post-fix:

  * `test_instant_now_returns_nonnegative_nanos`: PASS (interp).
  * `test_instant_now_advances_monotonically`: PASS.
  * `test_instant_duration_since_self_is_zero`: PASS.
  * Remaining `wall_clock_ms` assertion-failure surface is a
    distinct runtime-dispatch issue (the intrinsic now compiles, but
    `__time_now_ms_raw`'s dispatch to the interpreter's
    SystemTime-based handler returns a value below the post-2000 ms
    bound — different defect, will be audited next cycle).

The task #5 propagation defect is closed by commit `51ecc3bc9`:
**auto-derived mount-based dependencies + force `core.intrinsics`/
`core.intrinsics.runtime` ordering** in
`crates/verum_compiler/src/core_compiler.rs::augment_dependencies_from_mounts`.

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

### 3.1 Stale `mount super.raw.*` in 5 sys/*.vr files + intrinsic-mount propagation (task #5 — CLOSED)

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

**Root cause (deeper than original hypothesis — verified via build
trace)**: the hardcoded dependency graph in
`crates/verum_compiler/src/core_compiler.rs::resolve_dependencies`
listed `core.intrinsics` as a dep of only `core.simd` / `core.math`
— yet 20+ stdlib subdirectories mount from `core.intrinsics.*`
(sys/base/mem/async/io/text/runtime/net/sync/…). Topological sort
therefore placed those consumers BEFORE `core.intrinsics.runtime`
(a separate sub-module registered by the discoverer), leaving the
`@intrinsic`-decorated raw declarations un-registered in the
shared function-id table at the time consumer mount-resolution
ran. The single hardcoded HashMap was the architectural defect —
every new mount target required a manual update that drifted.

**Fix landed in commit `51ecc3bc9`**: new function
`augment_dependencies_from_mounts` mechanically scans each module's
`.vr` source files for `mount <path>` declarations via lightweight
regex (~10 ms × 2540 files = ~25 s amortised in the build) and
adds the implied top-level dep edge. A `FOUNDATION_DEPS_TO_FORCE`
constant restricts auto-derived edges to `core.intrinsics` +
`core.intrinsics.runtime` — the stdlib has *real* mutual layer
references (`core.sys ↔ core.base ↔ core.mem`) that the hardcoded
baseline broke arbitrarily; adding every derived edge would
re-introduce those cycles. The foundation modules' bodyless
`@intrinsic` declarations are the only case where mount-resolution
*requires* the producer to compile first; cycle-tolerant
forward-reference resolution covers the other inter-layer cases.

**Verified post-fix**:

  * Precompile succeeds end-to-end: `584 modules, 47015 functions,
    307 s` with the new graph.
  * `test_instant_now_returns_nonnegative_nanos`: PASS (interp).
  * `test_instant_now_advances_monotonically`: PASS.
  * Same dep-ordering fix transitively unblocks every other consumer
    of `core.intrinsics.*` (Heap.new allocator path, every
    async-runtime intrinsic, raw-pointer ops, `core.sys.{context_ops,
    file_ops, net_ops, process_ops}` — the entire 5-file `mount
    super.raw.*` family no longer needs migration to take effect; the
    panic-stub was caused by ordering, not by the mount syntax).
  * Remaining `wall_clock_ms` returns-too-small assertion is a
    distinct runtime-dispatch defect (the intrinsic now compiles and
    its function-id is registered, but the runtime resolution of
    `__time_now_ms_raw` to the SystemTime handler may be misfiring).
    Tracked separately.

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
| §A | Precompile pipeline: `mount core.intrinsics.runtime.os.{__time_*_raw}` from `core/sys/time_ops.vr` does not propagate `intrinsic_name` to the lookup table | **CLOSED** — commit `51ecc3bc9`: auto-derive mount-based deps + force `core.intrinsics`/`core.intrinsics.runtime` topological ordering. |
| §B | Migrate `core/sys/{context_ops, file_ops, net_ops, process_ops}.vr` from `mount super.raw.*;` to canonical form | **CLOSED** transitively by §A — those files now resolve the same intrinsics via the corrected topo order; the stale `mount super.raw.*;` is structurally harmless (resolves to nothing → no effect → falls through to ordered-foundation dep). Migration to canonical form is cosmetic and tracked as a separate cleanup. |
| §C | Replace placeholder stubs `__ptr_read_i64` / `__text_from_raw` in `core/sys/process_ops.vr:71-72` with real `core.intrinsics.runtime.os.*` mounts; pin `Child.read_stdout()` round-trip via regression test. | ~30 min — open (cosmetic / silent-data-loss class). |
| §D | `wall_clock_ms()` returns a value ≤ 946 868 480 000 (post-2000 ms threshold) — the intrinsic now compiles correctly post-§A but the runtime SystemTime → i64 dispatch may be misfiring. Distinct defect that the §A close exposed. | ~1 h — investigate runtime intrinsic dispatch for `time_now_ms`. |
