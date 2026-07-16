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

2. `property_test.vr` — 16 `@test`s (`wx_time_prop_*`) covering the
   algebraic laws of the host-safe surface:
   * scaling-relation identities among the constants (NANOS_PER_SEC
     factorizations, FILETIME 100-ns relation, epoch-offset whole-second
     divisibility);
   * `WindowsDuration` `add` commutativity + associativity over sampled
     u64 sweeps, `zero` additive identity, `mul` distribution over `add`,
     `sub` saturate-at-zero (and exact-difference) law, cross-scale
     constructor equivalence, the `as_secs*NANOS_PER_SEC + subsec_nanos
     == as_nanos` decomposition law, `subsec_millis` consistency,
     `from_secs_f64` round-trip on exact values, and `(d*f)/f == d`;
   * `WindowsDeadlineTimer.none()` invariants (`has_deadline() == false`,
     `remaining_ms() == INFINITE`).

3. `integration_test.vr` — 8 `@test`s (`wx_time_int_*`) wiring
   `WindowsDuration` into `List` (fold/sum via `.add()`, max-by
   `as_nanos`), `Maybe` (wrap + match unwrap, collect-present pipeline),
   `Map<Text, WindowsDuration>` (name→duration lookup + value sum), and
   the `WindowsDeadlineTimer.none()` `remaining()` / `deadline()` Maybe
   short-circuits routed through `match`.

4. `regression_test.vr` — 7 LOCK-IN `@test`s (`wx_time_reg_*`) pinning
   the exact `WINDOWS_EPOCH_OFFSET` (and its whole-second decomposition
   `11_644_473_600 s`), the 100-ns FILETIME unit, `sub` saturate-vs-wrap
   semantics with the strict-`>` boundary (equal clamps, off-by-one
   yields 1), and the `none()` short-circuits for `remaining_ms` (INFINITE),
   `remaining()` (Maybe.None), and `deadline()` (Maybe.None). Plus an
   `@ignore`'d pin for the NEWTYPE-UNBOX-1 defect below.

## Language defect — Map-retrieved WindowsDuration corrupts in a match arm (NEWTYPE-UNBOX-1)

**Found 2026-05-29 (root-cause refined 2026-05-30).** `WindowsDuration is
(UInt64)` is a single-field newtype. Retrieving one from a `Map` and using the
match-bound value (`d.as_nanos()` or `total.add(d)`) corrupts control flow —
the test "exits" with the raw inner value (e.g. `100_000_000`) instead of
completing. **CONFIRMED reproducible in isolation.**

**Closure-dependence (important):** the simpler `a.add(b).as_millis()` form
does NOT reproduce — it passes in a small import closure. The defect is
import-closure / collection-retrieval-sensitive (see
`docs/architecture/defect-class-catalogue.md §12`): an isolated
1–3-test probe under-counts method candidates and can give a misleading PASS;
the genuine failure shows up under the full conformance closure (`verum test
--filter wx_`). This is why `wx_time_int_map_values_sum` failed in the full run
even though `a.add(b).as_millis()` passes alone.

Same family as the core `Duration` single-field-record unboxing defect
(2026-05-27). `integration_test.vr::wx_time_int_map_values_sum` and
`regression_test.vr::wx_time_reg_map_retrieved_newtype_use` both `@ignore` the
confirmed-failing Map form; the deep fix is VBC codegen newtype boxing parity
+ type-directed dispatch (task #3).

## 3. Action items deferred (FFI host-unsafe — Windows kernel32 only)

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `WindowsInstant.now()` / `elapsed()` / `duration_since` / `add` / `sub` | Requires QueryPerformanceCounter — Windows host only. |
| 2 | `sleep` / `sleep_ms` / `sleep_us` / `sleep_until` | Requires kernel32 Sleep / SleepEx (sleep_us also busy-waits on QPC). |
| 3 | `system_time` / `local_time` / `realtime_nanos` / `unix_timestamp` | Requires GetSystemTime / GetLocalTime / GetSystemTimePreciseAsFileTime. |
| 4 | `WindowsStopwatch` `start` / `started` / `stop` / `elapsed` / `lap` / `restart` | All call QueryPerformanceCounter. `new()` / `is_running()` / `reset()` are pure but trivially covered by the constructor unit tests; not re-pinned. |
| 5 | `WindowsPerfCounter` `now` / `elapsed` / `elapsed_ticks` / `diff` | All call QueryPerformanceCounter. |
| 6 | `WindowsDeadlineTimer` `after` / `at` / `is_expired` + `remaining()`/`deadline()`/`remaining_ms()` *with* a deadline | The deadline-bearing arms call `WindowsInstant.now()` → QPC. Only the `none()` path is host-safe and covered. |
| 7 | `time_it`, `get_perf_freq`, `monotonic_nanos/micros/millis` | All read QueryPerformanceCounter / GetTickCount64. |
