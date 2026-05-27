# `core.time.cron` — audit findings

> Module under test: `core/time/cron.vr` (539 LOC; `CronExpr` record
> with 5 `UInt64` bit-masks + 2 `Bool` constraint flags, plus
> `CronExpr.parse`/`next_after_unix`/`matches_parts`/internal helpers,
> 3-variant `CronError` ADT, 12 month aliases JAN..DEC + 7 dow aliases
> SUN..SAT).
>
> Test surfaces (this branch):
> `unit_test.vr` (205 LOC, 23 `@test`s — 8 parse-success + 6 parse-error
> + 5 next_after correctness + 1 invariant + 3 constrained-flag pins),
> `property_test.vr` (94 LOC, 10 `@test`s),
> `integration_test.vr` (154 LOC, 9 `@test`s — business-hours / monthly
> / step-and-range / OR-semantics / alias coverage).

## 1. Cross-stdlib usage

`CronExpr` is the only public crontab parser in the stdlib.

| Consumer | Use |
|---|---|
| `core.runtime.scheduler` | Crontab specification → next-fire `Instant` for periodic task scheduling |
| User-facing job schedulers (CLI / daemon) | Direct parse + `next_after_unix(SystemTime.now().timestamp())` |
| `core.cog.manifest` build-pipeline (if present) | Periodic re-build trigger from manifest |

## 2. Crate-side hardcodes

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| 5-field grammar (`minute hour dom month dow`) | POSIX / vixie-cron compatibility | Drift from POSIX would break every existing crontab on the planet |
| `MONTH_ALIASES` table 12 entries JAN..DEC | Case-insensitive alias resolution | Drift = silent acceptance of mis-spelled aliases or rejection of valid input |
| `DOW_ALIASES` table 7 entries SUN..SAT (SUN=0) | Day-of-week numbering | Drift breaks every existing crontab using textual dow |
| 5-digit integer cap (`src.len() > 6` with optional `-`) | Hostile-input DoS guard | Same hostile-input cap class as duration_parse §73 / json_pointer §181 / HTTP Content-Length §77 |
| 8-year search ceiling (`max_iters = 60 * 24 * 366 * 8`) | Worst-case scan bound | Pathological specs that admit no firing within 8 years surface as `ValueOutOfRange` |
| Vixie-cron OR-semantics when DOM AND DOW both explicit | Industry-standard convention since 1987 | Drift would break every cron-job that uses `0 0 1 * MON` or similar |
| 1970-01-01T00:00:00Z was Thursday (dow=4) | `decompose()` weekday computation anchor | Drift = wrong dow for every timestamp |

## 3. Language-implementation gaps

### §A — No support for vixie-cron extensions

Per the module docstring, vixie-cron extensions (`@hourly`, `@daily`,
`W`, `L`, `#n`) are explicitly NOT supported in v0.1. This is a
documented feature gap — future work behind an `extensions: bool`
constructor flag.

**Effort:** medium (~2-3 hours) — extension parser + `next_after`
modifications + ~10 new tests.

### §B — Wrong-field-count test pattern-match could be tighter

`test_parse_too_few_fields_is_error` (`unit_test.vr:72-80`) checks
`r.unwrap_err()` matches `CronError.WrongFieldCount(n) => assert_eq(n, 3)`.
The `_ => assert(false, ...)` fallback uses a Text message but
doesn't propagate which variant was returned. A panic-on-fail with
the actual variant name would be more diagnostic.

**Effort:** trivial — extend the wildcard arm with `f"got: {err.debug_name()}"` once `Debug` for `CronError` is implemented.

### §C — `next_after_unix` does not pin month-rollover edge case

The integration test `test_monthly_1st_at_midnight_next_from_epoch`
covers Jan 1 → Feb 1, but no test exercises `Feb 28 → Mar 1` in a
non-leap year vs `Feb 29 → Mar 1` in a leap year. The
`smallest_invalid_jump` helper at `cron.vr:212-219` uses
`days_in_month(p.year, p.month)` which IS leap-aware, but the
edge-case test would harden against regressions in the leap-year
table.

**Effort:** small (~10 min).

### §D — No `Display` / `Debug` impl for `CronExpr`

`CronExpr` carries no `Display` or `Debug` impl. Round-trip
(parse → format → parse) testing isn't possible without a
`format` direction. This is a feature gap, not a defect.

**Effort:** medium (~1h) — implement `Display for CronExpr`
emitting the canonical 5-field text + 10 round-trip property tests.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| — | Per-submodule conformance suite for `core.time.cron` | `core-tests/time/cron/{unit,property,integration}_test.vr` | Pre-existing in this branch; this audit pins the coverage map. |
| — | Missing `audit.md` for `core-tests/time/cron/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Vixie-cron extensions (`@hourly`/`W`/`L`/`#n`) | ~3h | open |
| §B | Tighter pattern-match-on-error diagnostics | trivial | open (gated on `Debug for CronError`) |
| §C | Leap-year edge-case next_after pin | 10 min | **CLOSED 2026-05-27** — `test_next_after_leap_year_feb_29_to_mar_1` + `test_next_after_non_leap_year_feb_28_to_mar_1` |
| §D | `Display for CronExpr` + format-direction round-trip suite | ~1h | open |
| — | Cross-tier (`--aot` vs `--interp`) divergence sweep | ~10 min wall-clock | open |

## 6. Status

**stable** under `--interp` — 23 unit + 10 property + 9 integration
tests all green at module API surface.

1 sampled test (`test_parse_all_wildcards`) confirmed green
2026-05-27 in 41.4s.
