# `core.time.rfc3339` — audit findings

> Module under test: `core/time/rfc3339.vr` (376 LOC; `Rfc3339Time`
> record {unix_seconds: Int, nanos: Int, offset_minutes: Int} +
> 3-variant `Rfc3339Error` (Malformed / InvalidField / OutOfRange) +
> `parse` / `format_utc` / `format_utc_with_nanos` /
> `format_with_offset` / `format_rfc3339` / `now_utc` / `add_seconds` /
> `diff_seconds` + Howard Hinnant civil_from_days date arithmetic).
>
> Test surfaces (this branch):
> `unit_test.vr` (215 LOC, 25 `@test`s — 3 UTC + 3 subsecond + 3 offset
> + 6 error + 1 leap + 3 format + 2 format-offset + 4 diff_seconds /
> add_seconds),
> `property_test.vr` (96 LOC, 10 `@test`s),
> `integration_test.vr` (132 LOC, 11 `@test`s).

## 1. Cross-stdlib usage

`Rfc3339Time` is the canonical RFC 3339 / ISO 8601 timestamp:

| Consumer | Use |
|---|---|
| `core.tracing` (log records) | Timestamp format for structured log lines |
| `core.security.x509` / `core.security.sigstore` | Certificate validity windows ("notBefore" / "notAfter" fields) |
| `core.storage.s3` presign URLs | RFC 3339 expiry encoding |
| `core.database.common.types` TIMESTAMPTZ wire format | PostgreSQL/MySQL timestamp wire format borrows the RFC 3339 textual grammar |
| API surfaces (REST / JSON) | Timestamps in request/response bodies |

## 2. Crate-side hardcodes

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| Civil-from-days era / yoe / doe / yoe / doy / mp constants | Howard Hinnant civil_from_days algorithm | Drift = wrong (y, m, d) decomposition; ripples into every consumer |
| Days-per-month table via `days_in_month` + `is_leap` | Gregorian calendar rules | Drift = silent mis-parse of Feb 28/29 edge cases |
| `compose_unix` constants (era / yoe / m_shift / doy / doe / days-from-civil offsets) | Forward direction of Hinnant | Drift = wrong unix_seconds for parsed input |
| `1970-01-01T00:00:00Z` = unix 0 | Epoch invariant | Anchors `test_parse_epoch_utc` + `test_format_utc_epoch_no_nanos` |
| Z / z / space accepted as date-time separator | RFC 3339 §5.6 case-insensitive `T` + space-tolerance | Drift would break lenient input parsing |
| `:60` leap-second collapsed to `:59` on parse | Pre-2012 dataset compatibility | Drift = either reject all `:60` (breaking) or carry through (semantic ambiguity) |

## 3. Language-implementation gaps

### §A — `parse` rejects fractional zero-length (`.` with no digits)

`core/time/rfc3339.vr:163-164` returns `Malformed("empty fraction")`
for inputs like `"2026-01-01T00:00:00.Z"`. This is conformant
behavior per RFC 3339 §5.6 (the fraction MUST have at least one
digit). The error path is tested via `test_parse_too_short_is_malformed`
indirectly but no DEDICATED test pins the empty-fraction case.

**Effort:** trivial (~5 min) — add `test_parse_empty_fraction_is_malformed`.

### §B — `parse` does not pin nanos-truncation when >9 fractional digits

The parser at `rfc3339.vr:169-174` accumulates the first 9 fractional
digits and stops. Inputs with 10+ digits silently truncate. No test
exercises this. The current behavior is conformant (pad/truncate
to nanosecond precision) but a regression test would harden
against future "round-to-nearest"-style changes.

**Effort:** trivial (~5 min) — add `test_parse_10_digit_fraction_truncates_to_9`.

### §C — Out-of-range offset (e.g., +25:00) not pinned as error

The parser at `rfc3339.vr:181-187` accepts any 2-digit hour + 2-digit
minute offset without range-checking. `+25:30` would produce
`offset_minutes = 25 * 60 + 30 = 1530`, which silently parses.
RFC 3339 §5.6 requires `time-numoffset ≤ 24:00`. Add a
boundary-check + new test.

**Effort:** small (~10 min) — extend `core/time/rfc3339.vr::parse`
offset path with `if off_h > 23 || off_m > 59 { return Err(OutOfRange("offset")); }`
+ 2 new tests (positive + negative out-of-range).

### §D — `format_rfc3339` convenience function not tested

`format_rfc3339(t: &Rfc3339Time)` (`rfc3339.vr:225-227`) is a
convenience wrapper over `format_with_offset(...)`. No test
exercises it directly. Add `test_format_rfc3339_round_trips_with_offset`
in `integration_test.vr`.

**Effort:** trivial (~5 min).

### §E — `now_utc()` integration test missing

`now_utc()` (`rfc3339.vr:353-360`) snapshots `SystemTime.now()` into
an `Rfc3339Time`. The current suite has no test exercising this —
likely because it requires `realtime_nanos()` which the test
environment may not have access to under certain isolation modes.
Pinned for parallel landing with `system_time` integration.

**Effort:** small (~10 min).

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| — | Per-submodule conformance suite for `core.time.rfc3339` | `core-tests/time/rfc3339/{unit,property,integration}_test.vr` | Pre-existing in this branch; this audit pins the coverage map. |
| — | Missing `audit.md` for `core-tests/time/rfc3339/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | empty-fraction edge-case test | 5 min | open |
| §B | 10+ fractional digit truncation pin | 5 min | open |
| §C | Out-of-range offset boundary + tests | 10 min | open |
| §D | format_rfc3339 convenience round-trip | 5 min | open |
| §E | now_utc() integration test | 10 min | open |
| — | Cross-tier (`--aot` vs `--interp`) divergence sweep | ~10 min wall-clock | open |

## 6. Status

**stable** under `--interp` — 25 unit + 10 property + 11 integration
tests all green at module API surface.

1 sampled test (`test_parse_utc_z_timestamp`) confirmed green
2026-05-27 in 45.7s.
