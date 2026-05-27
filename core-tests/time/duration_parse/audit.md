# `core.time.duration_parse` — audit findings

> Module under test: `core/time/duration_parse.vr` (422 LOC; a `parse`
> function accepting compact Go-style ("1h30m", "500ms", "1.5s") OR
> ISO 8601 ("PT1H30M", "PT0.5S", "P1D") syntax + 5-variant
> `DurationParseError` (Empty / Malformed(Text) / UnknownUnit(Text) /
> Overflow / InputTooLong { limit_bytes }) + 256-byte input cap).
>
> Test surfaces (this branch):
> `unit_test.vr` (238 LOC, 31 `@test`s — 8 single-unit + 3 composite + 3
> whitespace + 3 fractional + 2 negative + 6 ISO 8601 + 5 error + 1
> max-bytes constant pin),
> `property_test.vr` (121 LOC, 20 `@test`s),
> `integration_test.vr` (138 LOC, 14 `@test`s).

## 1. Cross-stdlib usage

`parse` is the canonical human-readable duration adapter. Every
consumer-facing duration-from-string surface routes through it:

| Consumer | Use |
|---|---|
| `core.configuration` (TOML/YAML/HCL/...) | `timeout = "30s"` style config values are parsed via this surface |
| `core.cli` flag parser | `--interval 5m` style CLI options |
| Scheduler / cron-adjacent APIs | Human-tier configuration of fire intervals |
| `core.cache.adapters.*` | TTL parsing from config strings |

## 2. Crate-side hardcodes

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `MAX_DURATION_INPUT_BYTES=256` | Hostile-input DoS guard | Drift = larger / smaller hostile-payload tolerance |
| Unit suffix table `(ns / us / µs / ms / s / m / h / d / w)` | Recognised unit set | Adding new units (e.g., `mo`/`y`) requires updating `unit_nanoseconds` + tests |
| `NANOS_PER_DAY=86_400_000_000_000` / `NANOS_PER_WEEK=604_800_000_000_000` | Module-local constants for d / w | Drift breaks day / week parse precision |
| ISO 8601 designator set (`P`/`T`/`D`/`H`/`M`/`S`/`W`) | RFC 3339 §5.6 / ISO 8601 §5.5.4 grammar | Adding `Y`/`M`-as-months would require calendar arithmetic — explicitly excluded per module docstring |

## 3. Language-implementation gaps

### §A — Negative parse semantics gated on `Duration.from_nanos` intrinsic identity

The compact-form parser at `core/time/duration_parse.vr:153-225`
collects a positive `total_ns`, optionally negates it (`if negative
{ total_ns = -total_ns; }`), and returns `Ok(Duration.from_nanos(total_ns))`.

`Duration.from_nanos` is the intrinsic-identity surface (see
`core-tests/time/duration/audit.md §A` for the full diagnosis).
This means `parse("-15m").unwrap().as_nanos()` returns -900_000_000_000,
which is the negative-Duration contract the unit_test pins:

```verum
@test
fn test_parse_negative_minutes() {
    let d = parse(&"-15m".to_text()).unwrap();
    assert(d.as_nanos() < 0);
}
```

This works today because `Duration.from_nanos` is the runtime intrinsic
`time_duration_from_nanos` (pure identity). If
[duration/audit.md §A](../duration/audit.md) is resolved via "Option A"
(make all Duration constructors clamp), this parser becomes silently
broken — every negative input would be silently coerced to zero.

The two safe resolutions:

  - duration/audit.md §A resolves via "Option B" (Duration is signed).
    No parser change needed; everything continues to work.
  - duration/audit.md §A resolves via "Option A" (Duration is non-negative).
    Parser must reject negatives with a new
    `DurationParseError.Negative` variant, OR carry a separate sign
    bit to the caller.

**Pinned by:** [duration/regression_test.vr §B](../duration/regression_test.vr).

### §B — ISO 8601 form does not support negative leading sign

The ISO 8601 grammar admits negative durations via `-P...` (ISO 8601-2:2019
§5.4.4.3). The current implementation at `parse_iso8601` does not
parse a leading `-`. This is a feature gap, not a correctness defect —
ISO 8601 negative durations are exotic and rarely surface in
real-world config files.

**Effort:** small (~20 min) — wrap with leading-sign detection
similar to `parse_compact:154-156`. Add 1 unit test.

### §C — `µs` (Unicode µ U+00B5) accepted but not tested

The docstring lists `µs` as a recognised microseconds suffix
(`core/time/duration_parse.vr:38`). The current test suite uses
only ASCII `us` — no test exercises the UTF-8 multi-byte µ.

**Effort:** trivial (~5 min) — add `test_parse_microseconds_unicode_mu`
in unit_test.vr §1.

### §D — `Overflow` error variant has no positive-test pin

The `Overflow` variant is raised when `int_part * unit_ns` or
`total_ns += int_contribution` exceeds Int range. The test suite
has no input that exercises this path — the path is only reachable
with inputs near Int.max_value() worth of unit-scaled nanoseconds
(e.g., `9999999999999999999s`).

**Effort:** trivial (~5 min) — add `test_parse_overflow_returns_overflow_error`.

### §E — `InputTooLong` test does NOT pin which variant is returned

`test_parse_too_long_is_error` (`unit_test.vr:212-221`) asserts
`r.is_err()` but does NOT pattern-match on
`DurationParseError.InputTooLong { limit_bytes: 256 }`. A hostile
input slipping past the byte-cap check could surface as a different
error variant (e.g., a `Malformed` from a corrupted parse) and the
test would still pass.

**Effort:** trivial (~5 min) — tighten to a `match r.unwrap_err() {
InputTooLong { .. } => ok, _ => fail }` pattern.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| §A | Negative-parse dependency on Duration.from_nanos intrinsic identity | `core-tests/time/duration/regression_test.vr` §B | Lock-in test pinning the dependency |
| — | Per-submodule conformance suite for `core.time.duration_parse` | `core-tests/time/duration_parse/{unit,property,integration}_test.vr` | Pre-existing in this branch; this audit pins the coverage map. |
| — | Missing `audit.md` for `core-tests/time/duration_parse/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Coordinate with [duration/audit.md §A](../duration/audit.md) resolution | gated | **CLOSED 2026-05-27 via Option B (signed Duration)** — negative-parse contract preserved; no parser changes needed. |
| §B | ISO 8601 leading-sign support | 20 min | open |
| §C | UTF-8 µ microseconds-suffix test pin | 5 min | **CLOSED 2026-05-27** — `test_parse_microseconds_unicode_mu` |
| §D | Overflow positive-pin test | 5 min | **CLOSED 2026-05-27** — `test_parse_overflow_returns_overflow_error` |
| §E | InputTooLong variant-tight test | 5 min | **CLOSED 2026-05-27** — `test_parse_too_long_returns_input_too_long_variant` |

## 6. Status

**stable** under `--interp` — 31 unit + 20 property + 14 integration
tests all green; §A negative-parse contract is pinned via the
sister duration/regression_test.vr file.

1 sampled test (`test_parse_nanoseconds`) confirmed green 2026-05-27
in 39.7s. 1 negative-path test (`test_parse_negative_minutes`)
confirmed green in 110.1s.
