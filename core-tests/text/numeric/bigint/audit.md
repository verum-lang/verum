# `core.text.numeric.bigint` — audit

> Status: **partial** (conformance suite landed 2026-05-15; arithmetic
> surface gated by task #24).  Arbitrary-precision signed integers in
> base 10^9.  Construction surface (`zero`, `one`, `from_int`,
> `parse_bigint`) and predicates (`is_zero`, `is_negative`,
> `is_positive`, `is_even`, `is_odd`) work.  Arithmetic surface
> (`add`, `sub`, `mul`, `div_rem`, `compare`, `abs`, `neg`) is blocked
> by task #24 — every method-return loses the inner `digits` List
> field.
>
> Suite: `unit_test.vr` (~338 lines, original — many tests fail due to
> §A) + `property_test.vr` (new, 12 algebraic laws — all @ignored
> until §A) + `integration_test.vr` (new, 7 cross-stdlib scenarios —
> all @ignored) + `regression_test.vr` (new, 5 PASS-GUARDs + 5 §A pins).

---

## 1. Cross-stdlib usage

| Module | Usage |
|---|---|
| `core/text/numeric/bigdecimal.vr` | `coefficient: BigInt` |
| `core/text/numeric/rational.vr` | `numerator: BigInt`, `denominator: BigInt` |
| `core/text/numeric/modular.vr` | every function takes `&BigInt` args |
| `core/text/numeric/decimal.vr` | optional fallback to BigInt for overflow |
| `core/security/crypto/*` (future) | RSA / Curve25519 use BigInt mod_pow |
| `core/proof/*` | universe ordinal arithmetic on heap-tagged BigInt |

## 2. Crate-side hardcodes

| Path | What |
|---|---|
| `crates/verum_vbc/src/codegen/expressions.rs` | record-construction lowering for `BigInt { sign, digits }` — drift surface for task #24 |
| `crates/verum_vbc/src/interpreter/dispatch_table/handlers/method_dispatch.rs` | (none — BigInt is pure Verum, no Tier-0 intercepts) |

## 3. Language-implementation gaps surfaced by this folder

### §A — Method-return loses `digits` List field — **TASK #24**

**Symptom**: every method that returns a freshly-constructed `BigInt`
(via `BigInt { sign: ..., digits: ... }` record literal) returns a
value whose `digits` field behaves as an empty List downstream.

```verum
let a = BigInt.from_int(7);  // a.digits.len() == 1, a.digits[0] == 7 — VERIFIED
let b = a.abs();
b.digits.len()                // panic: IndexOutOfBounds { index: 0, length: 0 }
```

Verified `abs`'s body is correct (calls `clone_digits` which is a
verified-correct free function building a List<Int> via
`List.with_capacity(n)` + `push`).  The defect is in the
record-construction + return path.  Affects every BigInt method
that returns a new BigInt: `abs`, `neg`, `add`, `sub`, `mul`,
`div_rem` (Ok variant), `compare` (not affected — returns Ordering).

Investigation path documented in task #24.

---

## 4. Algebraic laws pinned (property_test.vr, @ignore until §A)

| Law | Property |
|---|---|
| L1 | `a.add(&b) == b.add(&a)` (Add commutativity) |
| L2 | `(a + b) + c == a + (b + c)` (Add associativity) |
| L3 | `a + 0 == a` (additive identity) |
| L4 | `a + (-a) == 0` (additive inverse) |
| L5 | `a * b == b * a` (Mul commutativity) |
| L6 | `(a * b) * c == a * (b * c)` (Mul associativity) |
| L7 | `a * 1 == a` (multiplicative identity) |
| L8 | `a * 0 == 0` (multiplicative zero) |
| L9 | `a * (b + c) == a*b + a*c` (distributivity) |
| L10 | `cmp(a, b) == reverse(cmp(b, a))` (Ord antisymmetry) |
| L11 | `a - a == 0` (sub identity) |
| L12 | `div_rem(a, b) = (q, r)` ⇒ `q*b + r == a` (div_rem reconstruction) |

## 5. Cross-stdlib integration pinned (integration_test.vr, @ignore until §A)

- `parse_bigint("12345").to_text() == "12345"` (positive round-trip)
- `parse_bigint("-42").to_text() == "-42"` (negative round-trip)
- `parse_bigint("not-a-number")` → Err
- `BigInt(10).div_rem(BigInt(0))` → Err (divide by zero)
- `BigInt.from_unsigned_bytes_be(b.to_unsigned_bytes_be())` round-trip
- `BigInt.from_unsigned_bytes_le(b.to_unsigned_bytes_le())` round-trip
- `match a.compare(&b) { Less => …, Equal => …, Greater => … }` routes correctly

## 6. Action items

### Landed in this branch
- 4-file conformance suite (property + integration + regression + this audit).
- Task #24 filed with bisection evidence.

### Deferred
- Task #24 close — unblocks all 12 property laws + 7 integration tests
  + 5 §A pins.
