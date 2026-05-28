# `core.sys.windows.time` — implementation audit

## Status: **partial** (under `--interp`; constant + WindowsDuration surface, QPC / sleep / FILETIME path deferred)

* Provides:
  * Time-unit constants (`NANOS_PER_SEC`, `NANOS_PER_MILLI`,
    `NANOS_PER_MICRO`, `MILLIS_PER_SEC`, `MICROS_PER_SEC`,
    `FILETIME_UNITS_PER_SEC`, `WINDOWS_EPOCH_OFFSET`).
  * `WindowsDuration` newtype over `UInt64` nanoseconds, with the
    full `from_*` / `as_*` constructor + accessor surface plus
    `add` / `sub` (saturating) / `mul` / `div` arithmetic.
  * `WindowsInstant` newtype over QueryPerformanceCounter ticks
    with `now` / `elapsed` / `duration_since` / `add` / `sub`.
  * `WindowsStopwatch` / `WindowsPerfCounter` / `WindowsDeadlineTimer`
    higher-level types.
  * Free functions `monotonic_nanos` / `monotonic_micros` /
    `monotonic_millis`, `realtime_nanos`, `unix_timestamp`,
    `system_time`, `local_time`, `sleep` / `sleep_ms` /
    `sleep_us` / `sleep_until`, `time_it`.

## 1. Pinned invariants

| Invariant | Source-level form | Why pinned |
|---|---|---|
| Scale ladder | `NANOS_PER_SEC == NANOS_PER_MILLI * MILLIS_PER_SEC` | Drift would silently break every cross-unit conversion. |
| Ladder x2 | `NANOS_PER_MILLI == NANOS_PER_MICRO * 1000` | Same. |
| FILETIME | `FILETIME_UNITS_PER_SEC == 10_000_000` | Microsoft commits this in `<minwinbase.h>`; drift would break every FILETIME ↔ Duration conversion. |
| Epoch offset | `WINDOWS_EPOCH_OFFSET == 116444736000000000` | Documented count of 100-ns ticks between 1601-01-01 and 1970-01-01.  Used for FILETIME ↔ Unix-time round-trips. |

## 2. Action items landed in this branch

1. `unit_test.vr` — 27 `@test`s pinning the seven constants, the
   `from_nanos` / `from_micros` / `from_millis` / `from_secs` /
   `zero` constructors with round-trip via `as_nanos` / `as_micros` /
   `as_millis` / `as_secs`, the subsecond accessors
   (`subsec_nanos` / `subsec_millis` for both whole-second and
   fractional inputs), `is_zero` for both arms, the four arithmetic
   operators with the documented saturate-at-zero behaviour on
   `sub`, and three cross-scale equivalence laws.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `WindowsInstant.now()` / `elapsed()` round-trip | Requires QueryPerformanceCounter — Windows host only. |
| 2 | `sleep` / `sleep_ms` / `sleep_us` / `sleep_until` | Requires kernel32 Sleep / SleepEx. |
| 3 | `system_time` / `local_time` / `realtime_nanos` / `unix_timestamp` | Requires GetSystemTime / GetSystemTimeAsFileTime / GetSystemTimePreciseAsFileTime. |
| 4 | `WindowsStopwatch` start/stop/elapsed | Wraps WindowsInstant; gated on §1. |
| 5 | property_test.vr — Duration arithmetic associativity / commutativity / identity laws | Deferred until the broader stdlib base/protocols suite ships a generic property runner. |
