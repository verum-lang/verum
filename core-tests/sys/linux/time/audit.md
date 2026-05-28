# `core.sys.linux.time` — implementation audit

## Status: **complete** (under `--interp`; constant surface; live clock APIs deferred)

* Linux clock IDs: 11 canonical kernel values pinned (CLOCK_REALTIME=0,
  CLOCK_MONOTONIC=1, CLOCK_PROCESS_CPUTIME_ID=2, CLOCK_THREAD_CPUTIME_ID=3,
  CLOCK_MONOTONIC_RAW=4, CLOCK_REALTIME_COARSE=5, CLOCK_MONOTONIC_COARSE=6,
  CLOCK_BOOTTIME=7, CLOCK_REALTIME_ALARM=8, CLOCK_BOOTTIME_ALARM=9,
  CLOCK_TAI=11). TIMER_ABSTIME = 1.
* Live clock readers (clock_gettime / clock_nanosleep) deferred to
  `vcs/specs/L2-standard/`.

## Action items landed

1. `unit_test.vr` — 13 `@test`s pinning Linux clock IDs + TIMER_ABSTIME
   + pairwise distinctness sweep.
2. `property_test.vr` — 3 laws: clock IDs in 0..=11; count = 11;
   CLOCK_MONOTONIC=1 (Linux-divergent from Darwin's 6).
3. `regression_test.vr` — 4 `@test`s: CLOCK_REALTIME=0 universal;
   CLOCK_MONOTONIC=1 NOT 6 (Linux-divergent defect class);
   CLOCK_BOOTTIME=7 Linux-specific; TIMER_ABSTIME=1.

## Deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Live clock APIs | needs target_os=linux build. |
| 2 | LinuxDuration / LinuxInstant types | Type-shape exists in source; not yet conformance-tested. |
