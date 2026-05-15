# `core.text.numeric.bigint` — audit

> Status: **partial** (task #24 closed 2026-05-15 — interior-field-ref
> auto-deref landed; `abs` / `neg` / `add` (positive operands) green;
> `sub` / `mul` gated on separate defect classes).
> Arbitrary-precision signed integers in base 10^9.  Construction
> surface (`zero`, `one`, `from_int` for non-negative operands,
> `parse_bigint`) and predicates (`is_zero`, `is_negative`,
> `is_positive`, `is_even`, `is_odd`) work.  `abs`, `neg`, `add`
> (positive) work after task #24 close.  Remaining gaps: `sub`
> (NullPointer through `add(&other.neg())` chain), `mul` (StackOverflow
> at recursion depth 16384), `from_int(-N)` (function-id collision on
> `abs_int#1` — separate task #20 class).
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

### §A — Method-return loses `digits` List field — **TASK #24 CLOSED 2026-05-15**

**Symptom**: every method that returns a freshly-constructed `BigInt`
(via `BigInt { sign: ..., digits: ... }` record literal) returns a
value whose `digits` field behaves as an empty List downstream.

**Root cause**: NOT in record construction.  When `&self.digits` is
passed across a function boundary (`clone_digits(&self.digits)`),
the `Value::from_ptr` interior pointer was not auto-derefed by
`dispatch_method_call`, `handle_get_index`, or `handle_set_index`.
Callee dispatchers then treated the parent-record header as the
receiver — every `src.len()` / `src[i]` inside `clone_digits`
mis-routed to the parent's ObjectHeader, surfacing as
`IndexOutOfBounds(0, 0)` and `length: 0` downstream.

**Fix** (`crates/verum_vbc/src/interpreter/dispatch_table/handlers/method_dispatch.rs`
+ `memory_collections.rs`): added the parallel third deref branch
alongside the existing `is_cbgr_ref` and `is_thin_ref` arms — when
the receiver/array is `Value::from_ptr` whose address is in
`state.cbgr_mutable_ptrs`, deref to the actual underlying Value
before dispatching.

**Architectural rule**: every dispatch path that matches
`is_cbgr_ref` or `is_thin_ref` MUST also handle the
heap-interior-pointer case — same shape, third branch.

**Validated** by 21 regression tests across
`core-tests/text/numeric/bigint/regression_test.vr` (5 §A pins),
`core-tests/text/text/regression_test.vr` (2 §V pins), and the
unit_test surface (`test_abs_positive_identity`,
`test_abs_zero_is_zero` previously @ignored — now PASS).

**Remaining gaps** (separate tasks):
- `sub` — `add(&other.neg())` chain NullPointers at pc=54
- `mul` — StackOverflow at depth 16384 in `mul_magnitudes` recursion
- `from_int(-N)` — `abs_int#1` function-id collision (task #20 class)

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
