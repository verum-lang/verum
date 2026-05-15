# `core.text.numeric.bigint` — audit

> Status: **partial** (tasks #14 / #15 / #16 ALL CLOSED 2026-05-15
> by single fundamental fix in `try_compile_builtin` — primitive
> poly-arith bare-call direct intrinsic emission, commit
> `f89967176`).  `abs` / `neg` / `add` (positive operands) work
> after task #24.  Now ALSO working: `add` (mixed signs), `sub`
> (via direct `a.sub(&b)` with explicit stack-local `b`), `mul`
> (was StackOverflow at depth 16384), `from_int(-N)` (was
> function-id collision on `abs_int#1`).  **All three were the
> SAME defect** — `implement Sub for Int { fn sub(self, rhs) -> Int { sub(self, rhs) } }`
> wrappers in `core/base/primitives.vr` were resolving the bare
> body call `sub(self, rhs)` to the enclosing `Int.sub` method
> itself (registered in the function table under the bare name
> `sub`), emitting a self-recursive `Call(Int.sub_func_id)` that
> stack-overflowed at depth 16384.  The fix routes bare-name
> calls matching the poly-arith intrinsic set (`add` / `sub` /
> `mul` / `div` / `rem` / `neg` / `bitand` / `bitor` / `bitxor` /
> `shl` / `shr` / `clamp` / `signum` / `abs_signed`) with
> primitive-numeric args directly to the `ArithExtendedOpcode`
> opcode — mirrors task #20 §A's `compile_unary::Neg` direct-
> opcode rule for unary.  **Blast radius**: every stdlib `implement <Op> for <Numeric>`
> wrapper closed.  BigInt sub/mul/from_int-of-negative all green
> now; bigdecimal/rational/modular sub/mul should likewise close
> on rebuild (they depend on bigint sub/mul).
>
> **Residual (task #17)**: `BigInt.from_int(10).sub(&BigInt.from_int(3))`
> (transient receiver) and `let a = BigInt.from_int(10); a.sub(&b)`
> (BigInt.sub's body delegates `self.add(&other.neg())` — a chained
> method call where `&other.neg()` is an `&` into a fresh BigInt
> return value) NullPointer at GetF (op=0x62) pc=61 inside
> `clone_digits`.  Same defect class as task #24 (interior-field-
> ref auto-deref) but with one more indirection layer through
> chained method receivers.  Direct stack-local equivalent
> `let nb = b.neg(); a.add(&nb)` works fine.
>
> Suite: `unit_test.vr` (~338 lines, original) + `property_test.vr`
> (12 algebraic laws — un-@ignore on task #17 close) +
> `integration_test.vr` (7 cross-stdlib scenarios — un-@ignore
> on task #17 close) + `regression_test.vr` (5 PASS-GUARDs + 5
> §A pins now passing on the explicit-binding shape).

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
