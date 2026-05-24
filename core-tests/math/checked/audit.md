# `math/checked` audit

Module: `core/math/checked.vr` (~270 LOC) — overflow-checked +
saturating integer arithmetic. Defines CheckedResult<T> 2-variant
+ checked_* (Int64/UInt64/Int32/UInt32) + saturating_* operations.

Tests: 35 unit tests covering CheckedResult 2-variant + Int64
checked_add/sub/mul/div/neg/abs (happy + overflow paths) +
saturating_add/sub/mul/neg/abs (clamp-to-bounds semantics).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.base.primitives` | wraps checked/saturating into Int.checked_*/sat_*. |
| `core.collections.bloom` / `count_min` | checked_mul for bit-array sizing. |
| `core.text.numeric.BigInt` | falls back to checked for the i64 fast-path. |
| Application financial code | money arithmetic via saturating semantics. |

## 2. Crate-side hardcodes

`crates/verum_vbc/src/intrinsics/...` LLVM intrinsics
(`llvm.{s,u}{add,sub,mul,div}.with.overflow.i64/i32`) — the
overflow flag interpretation must agree with this module's
semantic of "Overflow" iff hardware-flag set.

## 3. Language-implementation gaps

### §3.1 No property test (round-trip + commutativity laws)

Add property_test.vr:
* ∀a,b. checked_add(a,b) Ok ⟹ checked_add(b,a) Ok (commutativity)
* ∀a,b. checked_sub(a,b) == checked_add(a, -b) (when -b doesn't overflow)
* ∀a. checked_neg(checked_neg(a)) == Ok(a) (when a ≠ MIN)
* saturating_add(MAX, x) == MAX for any x >= 0
* saturating_sub(MIN, x) == MIN for any x >= 0

**Effort:** 1h.

### §3.2 32-bit + UInt64 variants not tested in this folder

Tests focus on Int64 surface — _u/_i32/_u32 variants follow
identical shape; cover in follow-up.

**Effort:** small (~1h for full sweep).

### §3.3 Display/Debug/Eq for CheckedResult missing

Adds quality-of-life for tests + error reporting.

**Effort:** trivial (~15 min).

## Action items landed in this branch

* `core-tests/math/checked/unit_test.vr` — 35 unit tests over
  CheckedResult 2-variant + signed Int64 checked operations
  (happy + overflow boundary cases at Int64.MAX/MIN) +
  saturating clamp-to-bounds semantics.
* `core-tests/math/checked/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add property_test.vr (commutativity, double-negation, saturate-at-bound laws) | this folder | 1h |
| Add tests for _u / _i32 / _u32 variants | this folder | 1h |
| Add Display / Debug / Eq for CheckedResult | `core/math/checked.vr` + tests | 15 min |
| Sister tests for `core.math.{bits,bitvec,big_uint,calculus}` static surfaces | sister folders | 1 week total |
EOF
