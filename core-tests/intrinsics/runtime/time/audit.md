# `intrinsics/runtime/time` audit

Module: `core/intrinsics/runtime/time.vr` (~100 LOC) — kernel-boundary
time intrinsics: monotonic/realtime clocks, CPU count, sleep.

Tests: unit (10) + property (5) + integration (4) + regression (1).
Timing laws use lower bounds + generous ceilings only — a tight upper
bound on a timing test is a flake, not a law.

## 1. Findings (2026-07-03 first pass)

* TIME-UNWIRED-1 — FIXED (d31878ee8): `sleep_ms` / `sleep_ns` /
  `realtime_nanos` had NO registry entries and no handlers — sleeps
  returned instantly and the wall-clock reads failed every sanity window.
  Name-dispatch arms added (thread::sleep / SystemTime).  6 failures → 0.
* TIME-MONO-CONTEXT-1 (task #17, OPEN): `monotonic_nanos` is
  context-dependent — a probe file printed t=42 (placeholder-looking, yet
  comparisons passed) while `test_monotonic_nanos_is_positive` fails
  deterministically under `--exact`.  Suspect the dual-emitter drift class
  (expressions.rs vs intrinsics/codegen.rs — the same split behind the
  ghost-symbol fix) plus boxed-int rendering printing the box handle.
  1 red pinned until resolved.
* `monotonic_nanos`/`realtime_nanos` return `UInt64` whose values exceed
  the 48-bit NaN-box small-int range → they exercise the boxed-int path
  (the `01a2406dc` large-int class) on every call.  The regression guard
  pins delta arithmetic staying small and non-negative.

## 2. Contract notes

* `monotonic_nanos` — never decreases (pinned over 1000 samples + across
  sleep).  Basis differs per platform (mach_absolute_time vs
  CLOCK_MONOTONIC): only deltas are meaningful; absolute values are
  pinned positive, nothing more.
* `realtime_secs`/`realtime_nanos` — sane epoch window (2020..2100) and
  mutual coherence within 2s.
* `sleep_ms`/`sleep_ns` — lower-bound guarantee (8ms floor for a 10ms
  request; scheduler-tick slack documented inline).
* `num_cpus` — ≥1, ≤4096, deterministic.

## 3. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.time` | Duration/Instant built on monotonic_nanos. |
| `core.async` | timers, deadlines. |
| `core.metrics` | timestamps. |

## 4. Crate-side hardcodes / drift surfaces

* `SystemSubOpcode::TimeMonotonicNanos/TimeRealtimeNanos/TimeSleepNanos`
  (0x70-0x75) — `@vbc_direct_lowering` route.
* AOT: VDSO clock_gettime (Linux) / mach_absolute_time (macOS) /
  QueryPerformanceCounter (Windows) — per-triple, never host-cfg.

## 5. Action items

**Landed this branch**
* Full conformance suite with flake-resistant law design.

**Deferred (tracked)**
* TIME-MONO-CONTEXT-1 (#17) — monotonic dual-emitter + boxed-int render.
