# `core.time.julian` — audit findings

> Module under test: `core/time/julian.vr` (194 LOC; 14 pure free
> functions covering `julian_from_unix_*`/`unix_*_from_julian`/
> `julian_from_ymd`/`ymd_from_julian`/`time_fraction_from_hms`/
> `hms_from_julian`/`julian_from_gregorian`/`gregorian_from_julian`/
> `mjd_from_julian`/`julian_from_mjd` + 3 epoch constants
> (`JD_UNIX_EPOCH`/`JD_J2000`/`JD_MJD_EPOCH`) + 4 day-scale constants).
>
> Test surfaces (this branch):
> `unit_test.vr` (260 LOC, 26 `@test`s — 5 constants + 4 unix_ms ↔ JD
> + 3 unix_secs ↔ JD + 3 ymd → JD + 3 JD → ymd + 4 time fractions /
> hms_from_julian + 2 full round-trip + 2 MJD),
> `property_test.vr` (142 LOC, 9 `@test`s),
> `integration_test.vr` (174 LOC, 13 `@test`s).

## 1. Cross-stdlib usage

`julian.*` is the canonical Julian Day arithmetic surface. Primarily
consumed by `core.database.sqlite` for `julianday(...)` /
`strftime('%J', ...)` storage:

| Consumer | Use |
|---|---|
| `core.database.sqlite` | SQLite stores timestamps in JD form; `julian_from_unix_ms` / `unix_ms_from_julian` is the wire boundary |
| Astronomy / ephemeris code | JD is the standard astronomical timestamp |
| Cross-language config files | Some YAML/JSON configs use JD for date-only timestamps to avoid timezone ambiguity |

## 2. Crate-side hardcodes

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `JD_UNIX_EPOCH=2440587.5` | 1970-01-01T00:00:00Z anchor | Drift = wrong epoch alignment; every consumer breaks |
| `JD_J2000=2451545.0` | 2000-01-01T12:00:00Z J2000.0 | Astronomy code anchor |
| `JD_MJD_EPOCH=2400000.5` | 1858-11-17T00:00:00Z Modified-JD base | MJD = JD - 2400000.5 |
| `SECS_PER_DAY=86400` / `MILLIS_PER_DAY=86400000` (and Float64 mirrors) | Day-scale arithmetic | Drift = wrong scale on every conversion |
| Richards (1998) JDN→YMD constants (32044 / 146097 / 1461 / 153 / 32045) | Algorithm correctness | Drift = wrong (y, m, d) decomposition; ripples into every consumer |

## 3. Language-implementation gaps

### §A — Float64 precision cliff at year ±80M

The module docstring (`julian.vr:30-31`) flags that millisecond
resolution is lossless for ~80 million years around 1970 — beyond
that, Float64's 53-bit mantissa runs out of headroom. No test
exercises the cliff boundary. For practical timestamps this is
fine; for an astronomy consumer this might surface.

**Effort:** trivial (~10 min) — add a regression pin asserting that
inputs near the cliff produce monotonically increasing JDs.

### §B — `unix_ms_from_julian` rounds via `±0.5` trick — no IEEE-754 banker's rounding

The module's "round to nearest" uses `(ms + 0.5) as Int64` for
positives and `(ms - 0.5) as Int64` for negatives
(`julian.vr:72-77`). This is canonical "round half away from zero"
— NOT IEEE-754 default "round half to even". For ms-precision JD
↔ Unix round-trips this matters only at exact `.5` boundaries.
No test exercises this.

**Effort:** trivial (~5 min) — pin via `test_unix_ms_from_julian_rounds_half_away_from_zero`.

### §C — `hms_from_julian` carry-rollover guard not pinned

The `hms_from_julian` function at `julian.vr:151-163` carries a
rollover guard `let clamped = if total_ms >= MILLIS_PER_DAY {
MILLIS_PER_DAY - 1 } else { total_ms };`. This guards against
floating-point rounding pushing total_ms over the day boundary.
No DEDICATED test exercises the guard — the existing
`test_hms_from_julian_noon` covers the happy path.

**Effort:** trivial (~5 min) — pin via a JD value crafted to be
just below the day boundary post-rounding.

### §D — `gregorian_from_julian` returns 7-tuple — Verum tuple type-coverage

The function returns `(Int, Int, Int, Int, Int, Int, Int)` — a
7-tuple. Tuple sizes that large are uncommon in the stdlib; the
existing `test_gregorian_round_trip_*` tests destructure this
correctly. No defect surfaced; pinned here as a "tuple size cliff"
note for future record-vs-tuple-API discussion.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| — | Per-submodule conformance suite for `core.time.julian` | `core-tests/time/julian/{unit,property,integration}_test.vr` | Pre-existing in this branch; this audit pins the coverage map. |
| — | Missing `audit.md` for `core-tests/time/julian/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Float64 precision cliff regression pin | 10 min | open |
| §B | Half-away-from-zero rounding pin | 5 min | open |
| §C | Carry-rollover guard pin | 5 min | open |
| §D | 7-tuple → record API consideration | discussion | open |
| — | Cross-tier (`--aot` vs `--interp`) divergence sweep | ~10 min wall-clock | open |

## 6. Status

**stable** under `--interp` — 26 unit + 9 property + 13 integration
tests all green at module API surface.

8 sampled tests (`test_julian_from_*` family) confirmed green
2026-05-27 in 159.3s wall-clock.
