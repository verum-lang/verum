# `money/currency` audit

Module: `core/money/currency.vr` (97 LOC) — ISO 4217 currency
descriptors. Defines `Currency` (code: Text + minor_units: Int)
record + `Currency.new` constructor + `Currency.eq` code-only
comparison + 25 predefined currency factory functions spanning
0/2/3-minor-unit classes.

Tests: `unit_test.vr` (~32 unit tests — construction + eq matrix +
all 25 factories), `property_test.vr` (~10 properties — eq laws +
pairwise distinct codes + minor-unit class consistency).

## 1. Cross-stdlib usage

`Currency` is consumed by:

| crate / module | what it does |
|---|---|
| `core.money.money` | `Money` record pairs an amount (BigDecimal) with a Currency to enforce currency-correct arithmetic. |
| Application code (financial) | `usd()`, `eur()`, `jpy()` factories at API boundaries; `Currency.new("XYZ", 2)` for custom currencies. |

## 2. Crate-side hardcodes

None today — `Currency` is pure Verum data with no Rust-side intercepts.
Future serialisation surface (TOML/JSON for config files) would need
the `Currency` field layout pinned.

## 3. Language-implementation gaps

### §3.1 No `Currency.from_iso_code(text)` runtime lookup

V0 ships 25 predefined currency factories. V1 should add a runtime
lookup keyed by ISO code that returns `Maybe<Currency>` for the full
~180-currency ISO 4217 list. The current design comment in
`currency.vr:14` documents this as planned.

**Effort:** medium (~1 day) — requires the full ISO table generation
(possibly via a build-time codegen pass).

### §3.2 No `Display` / `Debug` impls

`Currency` has no Display/Debug. `f"{usd()}"` would fail compile.
Either add the impls (returning the code string) or document the
lack as intentional (force callers to access `.code` explicitly).

**Effort:** trivial (~10 min) + 2 tests.

### §3.3 `Currency.eq` is code-only — minor-unit drift goes undetected

Doc-stated contract: code-only eq. But if a Currency carries a
mismatching minor-unit count for its ISO code (e.g. `USD` with 0
minor units instead of 2), the bad value compares equal to a
canonical USD. This is by-design (ISO contract gives canonical
minor units; drift is a bug in the caller) but the test
`test_currency_eq_ignores_minor_units` pins the contract so it
doesn't drift to "structural eq" silently.

### §3.4 No `Currency.is_iso_compliant(&self) -> Bool` validator

Given a `Currency`, can we check that the (code, minor_units) pair
is ISO-canonical? Today no — the validator would need the same
generated ISO table as §3.1. Add once that lands.

## Action items landed in this branch

* `core-tests/money/currency/unit_test.vr` — first conformance suite.
* `core-tests/money/currency/property_test.vr` — algebraic laws.
* `core-tests/money/currency/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `Currency.from_iso_code(text) -> Maybe<Currency>` runtime lookup | `core/money/currency.vr` + table + tests | 1 day |
| Add `Display` / `Debug` impls for Currency | `core/money/currency.vr` + tests | 10 min |
| Add `Currency.is_iso_compliant` validator | gated on §3.1 | 30 min after §3.1 |
| Add `core-tests/money/money/` suite for the `Money` type | sister folder | 2-3h |
