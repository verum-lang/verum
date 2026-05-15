# `core.text.numeric.bigdecimal` — audit

> Status: **partial** (conformance suite landed 2026-05-15; arithmetic
> blocked by task #24).  Arbitrary-precision decimal numbers — a thin
> wrapper around `BigInt` coefficient + `Int` scale.  All arithmetic
> delegates to BigInt and inherits §A from bigint/audit.md.
>
> Suite: `unit_test.vr` (~83 lines) + `property_test.vr` (new, 6 laws —
> all @ignored) + `integration_test.vr` (new, 4 scenarios — all
> @ignored) + `regression_test.vr` (new, 4 PASS-GUARDs + 2 §A pins).

## 1. Cross-stdlib usage

| Module | Usage |
|---|---|
| `core/text/numeric/rational.vr` | `Rational.from_bigdecimal(&bd)` constructor |
| `core/text/numeric/decimal.vr` | `BigDecimal.from_decimal(&d)` widening conversion |

## 2. Crate-side hardcodes

None.  Pure Verum delegation to BigInt.

## 3. Defect classes (transitively inherited from BigInt)

§A — see `core-tests/text/numeric/bigint/audit.md::§A` (task #24).
Every BigDecimal arithmetic delegates to BigInt's `add` / `sub` /
`mul` / `div_rem`, so the same record-return defect propagates here.

## 4. Algebraic laws pinned (property_test.vr, @ignore until task #24)

- Add commutativity: `a + b == b + a`
- Add zero identity: `a + 0 == a`
- Mul commutativity: `a * b == b * a`
- Mul one identity: `a * 1 == a`
- Neg involution: `--a == a`
- Abs non-negativity: `|a| >= 0`

## 5. Action items

### Landed in this branch
- 4-file conformance suite + this audit.

### Deferred
- Task #24 close — unblocks all property + integration + §A pins.
