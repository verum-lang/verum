# `core.text.numeric.rational` — audit

> Status: **partial** (conformance suite landed 2026-05-15; arithmetic
> blocked by task #24).  Arbitrary-precision rational numbers — pair of
> `BigInt` numerator/denominator + canonical reduction.  All arithmetic
> delegates to BigInt and inherits §A from bigint/audit.md.
>
> Suite: `unit_test.vr` (~78 lines) + `property_test.vr` (new, 8 laws —
> all @ignored) + `integration_test.vr` (new, 3 scenarios — all
> @ignored) + `regression_test.vr` (new, 4 PASS-GUARDs).

## 1. Cross-stdlib usage

| Module | Usage |
|---|---|
| `core/text/numeric/bigdecimal.vr` | `Rational.from_bigdecimal(&bd)` |
| (Future) `core/math/exact` | exact arithmetic primitive |

## 2. Crate-side hardcodes

None.  Pure Verum delegation to BigInt.

## 3. Defect classes (transitively inherited from BigInt)

§A — see `core-tests/text/numeric/bigint/audit.md::§A` (task #24).
Every Rational arithmetic delegates to BigInt's `add` / `sub` /
`mul` / `div`, so the same record-return defect propagates here.

## 4. Algebraic laws pinned (property_test.vr, @ignore until task #24)

- Add commutativity: `a + b == b + a`
- Add zero identity: `a + 0 == a`
- Mul commutativity: `a * b == b * a`
- Mul one identity: `a * 1 == a`
- Neg involution: `--a == a`
- Reciprocal round-trip: `reciprocal(reciprocal(a)) == a` (a != 0)
- Reciprocal of zero: returns Err
- Div by zero: returns Err

## 5. Action items

### Landed in this branch
- 4-file conformance suite + this audit.

### Deferred
- Task #24 close — unblocks all property + integration tests.
