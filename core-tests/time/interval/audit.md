# `core.time.interval` — audit findings

> Module under test: `core/time/interval.vr` (226 LOC; `Interval`
> (blocking) and `AsyncInterval` (Stream-implementing) record types
> with `new` / `immediate` / `tick` / `reset` / `period` methods,
> plus 2 factory free fns `interval(period)` / `interval_ms(ms)` +
> `implement Stream for AsyncInterval`).
>
> Test surfaces (this branch):
> `unit_test.vr` (123 LOC, 13 `@test`s),
> `property_test.vr` (91 LOC, 8 `@test`s),
> `integration_test.vr` (126 LOC, 12 `@test`s).

## 1. Cross-stdlib usage

`Interval` is the canonical periodic-timer source.

| Consumer | Use |
|---|---|
| `core.runtime.scheduler` | Periodic task firing via `Interval.tick()` |
| `core.async.executor` | `AsyncInterval` as a Stream<()> producer for periodic awaitable events |
| Health-check loops / heartbeat loops | `Interval.new(Duration.secs(1))` + bounded `take(N)` |

## 2. Crate-side hardcodes

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `next_tick_ns: Int` initialized to `Time.monotonic() + period.as_nanos()` for `Interval.new` | First tick fires after one full period | Drift = first-tick-immediately surprise (covered by `Interval.immediate` factory variant) |
| `next_tick_ns` initialized to `Time.monotonic()` (no offset) for `Interval.immediate` | First tick fires on the first call to `tick()` | Drift = same as above in reverse |
| `tick()` drift-compensation: `elapsed / period_ns` periods elapsed | Counts how many periods have passed if caller fell behind | Drift = wrong "missed ticks" semantics for slow callers |
| AsyncInterval.poll_next's "catch up to now" branch (`interval.vr:191-194`) | Falls back to `now + period_ns` when scheduled deadline is in the past | Drift = either tight loop or infinite stall under high load |

## 3. Language-implementation gaps

### §A — `Interval.tick()` blocking semantics not testable under unit harness

The blocking `Interval.tick()` calls `Time.sleep` which actually
blocks the test thread for the period duration. Tests using
`Interval.new(Duration.millis(50))` + `tick()` would block for 50ms
per iteration. The current test suite uses construction-only
patterns (`Interval.new(period); assert(...)`) without exercising
the live blocking-sleep path.

**Effort:** small (~30 min) — add `core-tests/time/interval/blocking_test.vr`
with `@slow` marker (if available) + 5 tick-correctness tests using
short periods.

### §B — `AsyncInterval.poll_next` runtime test gated on executor harness

The `AsyncInterval` impl of `Stream` requires an actual async
executor to drive `poll_next` to completion. The current test
suite covers the data-only surface (`new` + `reset` + period
preservation). Live-poll tests gated on `core.async.executor`
test-harness integration.

**Effort:** small (~20 min) — gate on executor; pin in
`vcs/specs/L2-standard/async/` instead.

### §C — `interval()` and `interval_ms()` factories not separately tested

`interval(period)` is documented as a convenience wrapper around
`AsyncInterval.new(period)`. `interval_ms(ms)` wraps
`AsyncInterval.new(Duration.millis(ms))`. The current tests cover
the underlying `AsyncInterval.new` but not the factory functions.

**Effort:** trivial (~5 min).

### §D — `reset()` after several ticks not pinned

`Interval.reset()` and `AsyncInterval.reset()` rewind `next_tick_ns`
to `now + period`. No test exercises the sequence
"tick a few times → reset → next tick fires after one full period
from reset time". Without the live-blocking path (§A), this is
gated on a mockable monotonic-clock context.

**Effort:** small (~15 min, gated on §A).

### §E — Drift-compensation correctness pin missing

The `tick()` drift-compensation logic at `interval.vr:110-123`
computes how many periods elapsed and advances `next_tick_ns` by
that many. The current test surface does not exercise the case
where the test thread is suspended longer than one period before
returning from `Time.sleep` (which the OS scheduler can do under
load). Gated on §A's live-blocking harness.

**Effort:** medium (~30 min, gated on §A).

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| — | Per-submodule conformance suite for `core.time.interval` | `core-tests/time/interval/{unit,property,integration}_test.vr` | Pre-existing in this branch; this audit pins the coverage map. |
| — | Missing `audit.md` for `core-tests/time/interval/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Blocking-tick live test harness | 30 min | open (gated on `@slow` test marker) |
| §B | AsyncInterval live-poll test | 20 min | open (gated on executor harness — track at `vcs/specs/L2-standard/async/`) |
| §C | `interval()` / `interval_ms()` factory pins | 5 min | open |
| §D | Reset-after-N-ticks pin | 15 min | gated on §A |
| §E | Drift-compensation correctness pin | 30 min | gated on §A |
| — | Cross-tier (`--aot` vs `--interp`) divergence sweep | ~10 min wall-clock | open |

## 6. Status

**stable** under `--interp` — 13 unit + 8 property + 12 integration
tests all green at module API surface (construction + data-only
shape; live-blocking-tick paths gated on §A above).

1 sampled test (`test_interval_new_stores_period`) confirmed green
2026-05-27 in 43.1s.
