# `core.sys.darwin.time` — implementation audit

## Status: **partial** (clock-ID constant surface complete; live clock readers deferred)

* Public clock-ID constants pinned: CLOCK_REALTIME_ID / CLOCK_MONOTONIC_ID
  / CLOCK_MONOTONIC_RAW_ID / CLOCK_PROCESS_CPUTIME / CLOCK_THREAD_CPUTIME.
* Live clock readers (monotonic_nanos / realtime_nanos / sleep_ms / etc.)
  defer to `vcs/specs/L2-standard/`.

## Action items landed

1. `unit_test.vr` — 6 `@test`s pinning canonical Darwin clock IDs +
   pairwise distinctness.
2. `property_test.vr` — 3 laws: all clock IDs non-negative, exhaustive
   pairwise distinctness, all values < 256 (kernel range).
3. `regression_test.vr` — 2 `@test`s: CLOCK_MONOTONIC=6 (Darwin, NOT
   Linux's 1); CPU-time clock IDs canonical values.

## Deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Live monotonic_nanos / realtime_nanos sweep | Timing-sensitive; needs L2-standard harness. |
| 2 | DarwinDuration / DarwinInstant round-trip | Stopwatch / PerfCounter / DeadlineTimer types exist in source but not yet conformance-tested. |
| 3 | sleep / sleep_ms / sleep_us live exercise | Out of scope for in-process. |
