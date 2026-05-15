# `core.text.numeric.modular` — audit

> Status: **complete** (conformance suite landed 2026-05-15).  9 number-
> theoretic functions over BigInt: `gcd`, `lcm`, `ext_gcd`, `mod_pow`,
> `mod_inverse`, `mod_sqrt`, `is_probable_prime`, `crt`, `crt2`.
>
> Suite: `unit_test.vr` (~136 lines, original) + `property_test.vr` (new,
> ~280 lines, 9 algebraic laws) + `integration_test.vr` (new, ~110 lines,
> cross-stdlib scenarios: gcd-over-List reduction, CRT 2/3-system solve,
> ext_gcd → mod_inverse round-trip, Fermat's little theorem) +
> `regression_test.vr` (new, 6 PASS-GUARDs).
>
> All defects in this module historically inherit from `core/text/numeric/
> bigint` (gcd/lcm dispatch goes through BigInt.add/sub/mul/compare).
> When BigInt operations are correct, modular operations are correct.

---

## 1. Cross-stdlib usage

| Module | Usage |
|---|---|
| `core/text/numeric/bigint.vr` | Every modular function takes `&BigInt` args and returns BigInt; all arithmetic dispatches through BigInt's add/sub/mul/div_rem |
| `core/text/numeric/rational.vr` | Rational reduction uses `gcd` on numerator/denominator |
| `core/text/numeric/bigdecimal.vr` | (Future) common-divisor reduction for fraction simplification |
| `core/security/crypto/*` (future) | `mod_pow` + `mod_inverse` + `is_probable_prime` are the canonical RSA / DH primitives |

## 2. Crate-side hardcodes

None.  `modular.vr` is pure Verum — every function is a composition of
`BigInt.*` method calls.  If BigInt drifts, modular tracks the drift.

## 3. Language-implementation gaps surfaced by this folder

### §A (transitive) — every defect class in `core/text/numeric/bigint` propagates here
The modular layer has no defects of its own once BigInt's arithmetic
is correct — gcd / lcm / ext_gcd / mod_pow / mod_inverse / crt all
delegate to `BigInt.add` / `sub` / `mul` / `div_rem` / `compare`.  See
`core-tests/text/numeric/bigint/audit.md` for the inherited defect
taxonomy.

---

## 4. Algebraic laws verified (property_test.vr)

| Law | Property |
|---|---|
| L1 | `gcd(a, b) == gcd(b, a)` (commutativity) |
| L2 | `gcd(a, 0) == \|a\|` (zero identity) |
| L3 | `gcd(a, b) * lcm(a, b) == \|a * b\|` (gcd-lcm product) |
| L4 | `ext_gcd(a, b) = (g, x, y)` → `a*x + b*y == g` (Bezout identity) |
| L5 | `mod_pow(a, 0, m) == 1` (zero exponent identity) |
| L6 | `mod_pow(a, e+1, m) ≡ (mod_pow(a, e, m) * a) mod m` (exponent recurrence) |
| L7 | `(a * mod_inverse(a, m)) mod m == 1` when `gcd(a, m) == 1` (inverse round-trip) |
| L8 | `is_probable_prime(p, witnesses) == true` for known primes |
| L9 | `is_probable_prime(n, witnesses) == false` for known composites |

## 5. Cross-stdlib integration verified (integration_test.vr)

- `gcd`/`lcm` reduce over `List<BigInt>` correctly
- `crt2(2, 3, 3, 5) → 8 (mod 15)` (two-system CRT)
- `crt([2,3,2], [3,5,7]) → 23 (mod 105)` (three-system CRT)
- `ext_gcd` Bezout coefficient mod m equals `mod_inverse` for coprime args
- Fermat's little theorem `a^(p-1) ≡ 1 (mod p)` verified for p ∈ {7, 11}

## 6. Action items

### Landed in this branch
- Conformance suite (4 files + this audit).  Module promoted from
  **partial** → **complete**.

### Deferred
None at the modular layer.  Every remaining gap surfaces in `bigint`.
