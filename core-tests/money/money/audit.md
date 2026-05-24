# `money/money` audit

Module: `core/money/money.vr` (353 LOC) — currency-aware monetary
value. Pairs a `BigDecimal` amount with a `Currency` to enforce
currency-correct arithmetic. The amount's scale must match the
currency's `minor_units` invariant.

Tests focus on `MoneyError` 5-variant ADT (constructible without
needing BigDecimal). Broader Money round-trip tests are deferred
until the underlying `core.text.numeric.BigDecimal` surface
stabilises.

## 1. Cross-stdlib usage

`Money` is consumed by:
| crate / module | what it does |
|---|---|
| Application financial code | `from_amount(BigDecimal, Currency) -> Result<Money, MoneyError>` constructor, `add` / `sub` / `mul_scalar` / `div_scalar` / `split(n)` arithmetic, `compare` / `eq` / `lt` / `gt` ordering. |

`MoneyError` is consumed by:
| Every Money API entry | returns `Result<..., MoneyError>` for fallible operations. |

## 2. Crate-side hardcodes

None today. `Money` is pure Verum data layered on `BigDecimal` +
`Currency`. The BigDecimal Rust intercepts (if any) live under
`core.text.numeric` and don't reach back to Money's API.

## 3. Language-implementation gaps

### §3.1 Money tests blocked on BigDecimal surface

The full `Money` test surface requires constructing `BigDecimal`
values to feed `from_amount`, `add`, `sub`, etc. The current
`BigDecimal` API is complex (BigInt coefficient + scale + sign)
and has its own conformance gaps. Once `core-tests/text/numeric/`
covers BigDecimal, sister Money tests should construct sample
values via `BigDecimal.from_text("12.34", 2)` and exercise the
full Money round-trip + arithmetic + split + render surface.

**Effort:** medium (~1 day) once BigDecimal tests are stable.

### §3.2 `MoneyError.Underlying` variant requires BigDecimalError

`MoneyError.Underlying { source: BigDecimalError }` wraps a
BigDecimalError from underlying numeric operations. Cannot be
constructed in tests without a BigDecimal failure surface. Deferred
to the integration tests once BigDecimal is testable.

### §3.3 No `Display` / `Debug` impl for `Money` or `MoneyError`

Missing observability — `f"{money}"` / `f"{err}"` won't compile.
Add `Display for Money` returning the canonical "amount CODE"
format documented at `money.vr:42`. Add Display + Debug for
MoneyError with per-variant message rendering similar to
ContextError.

**Effort:** ~1h + tests.

### §3.4 No `MoneyError.Eq` impl

Cannot compare two MoneyError values for equality. Standard pattern
for error types — add following ContextError's discipline (qualified
variants in match arms).

**Effort:** ~30 min + tests.

## Action items landed in this branch

* `core-tests/money/money/unit_test.vr` — MoneyError 5-variant
  surface (~15 unit tests covering construction, disjointness,
  field preservation).
* `core-tests/money/money/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `Display` / `Debug` for `Money` + `MoneyError` | `core/money/money.vr` + tests | 1h |
| Add `Eq` for `MoneyError` | `core/money/money.vr` + tests | 30 min |
| Full Money round-trip tests | gated on `core-tests/text/numeric/` BigDecimal | 1 day |
| MoneyError.Underlying integration tests | gated on BigDecimalError test surface | 30 min after BigDecimal |
