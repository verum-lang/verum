# `core.text.numeric.decimal` — audit

> Status: **partial**. Sweep on 2026-05-13: 27 / 45 unit tests pass
> (60%). The Decimal source is algebraically correct (the field/variant
> shapes, predicates, constants, and abs are all green). The 18
> failures concentrate in two upstream defect classes:
>
> - 4 tests panic with `Int.neg not found on receiver of runtime kind 'Int'`
>   — same family as Char/§A and text/text/§B.
> - 6 tests panic with `FunctionNotFound(FunctionId(N))` — same
>   function-id collision class as text/text/§D and the
>   `stdlib_bootstrap.initialize()` REVERTED entry in MEMORY.
> - The remaining 8 failures cascade from those (arithmetic that needs
>   neg or function-id resolution returns wrong values).

---

## 1. Cross-stdlib usage

| Module | Usage |
|---|---|
| `core/database/postgres/numeric.rs` (Rust side) | binds `Decimal` as the Verum-side `NUMERIC` representation |
| `core/database/sqlite/...` REAL/NUMERIC | Decimal is the canonical financial value type |
| `core/money.vr` | uses Decimal for monetary amounts (no binary-float drift) |
| User code: any monetary or fixed-precision arithmetic | the only safe-arithmetic decimal type in the stdlib |

## 2. Crate-side hardcodes

| Path | What | Pin |
|---|---|---|
| `crates/verum_compiler/src/precompile.rs` | `Decimal` precompiled into the runtime archive | function-id space shared with the rest of stdlib (subject to §B collision) |
| `crates/verum_database_postgres/src/numeric.rs` | `NUMERIC ↔ Decimal` codec | hardcodes the `{coefficient, scale}` field layout |
| `crates/verum_money/src/money.rs` | `Money(Decimal, Currency)` | hardcodes the Decimal field set |

## 3. Language-implementation gaps surfaced by this folder

### §A — `Int.neg` not found on receiver of runtime kind `Int` — **CLOSED 2026-05-15**
**Symptom**: `Decimal.neg()` panics. Body computes `-self.coefficient`
which lowers to `Int.neg(self.coefficient)`. The dispatcher cannot
find the `neg` method for `Int`.
**Root cause**: `compile_unary` for `UnOp::Neg` checked
`has_neg_method` FIRST and routed through `CallM("neg")` whenever
`<T>.neg` was registered — including `Int.neg` from `implement Neg
for Int` in primitives.vr.  The CallM had no runtime intercept for
primitive `Int.neg` and panicked.
**Fix** (two-layer, defense-in-depth):
1. Codegen (`compile_unary` Neg arm): for primitive integer / float
   operands, ALWAYS emit `UnaryI{Neg}` / `UnaryF{Neg}` directly.
   Canonical predicate `type_names::is_integer_type` /
   `is_float_type` covers every spelling.  User types with `Neg`
   protocol impls still route through `CallM("neg")`.
2. Runtime intercept (`dispatch_primitive_method` Int + Float arms):
   added `neg` / `wrapping_neg` / `checked_neg` / `saturating_neg`
   / `not` for Int, `neg` for Float — catches LEGACY precompiled
   bodies still emitting `CallM("neg")`.
**Result**: `test_neg_positive`, `test_neg_negative`, `test_neg_zero`
all green; `regression_a_decimal_neg_panic_pinned` un-`@ignore`d.
Same fix unblocks `BigInt.neg(&self)` / `Rational.neg` and every
user record method shaped like `Self { field: -self.field, ... }`.

### §B — FunctionNotFound on parse_decimal / arithmetic
**Symptom**: `parse_decimal(&"42")` panics with
`FunctionNotFound(FunctionId(2374))`. Same shape for add / mul / div
under specific arithmetic patterns.
**Root cause**: function-id collision under archive remap (cross-
module function-id namespaces). Same defect class as text/text §D.
**Action**: closes when text/text §D closes.

### §C — Arithmetic semantic failures (downstream of §A/§B)
**Symptom**: tests like `mul 3 * 4 = 12` see the right shape but
wrong coefficient. Likely consequence of Method Call dispatch
reaching the wrong impl body when the function-id remap drifts.
**Action**: closes when §B closes.

---

## 4. Action items

### Landed in this branch
- 45 unit tests + 17 property tests + 6 regression pins + 6 PASS-GUARDs.

### Deferred
| # | Item | Effort | Tests unblocked |
|---|------|------:|------:|
| 1 | §A — Int.neg dispatch | shared with Char/§A | 4 |
| 2 | §B — function-id collision | shared with text/text §D | 6 (and ~8 cascade) |

### Drift-pin recommendations
1. Pin `MAX_SCALE = 18` and the 4-variant RoundingMode layout in
   `crates/verum_common/src/well_known_types.rs::DECIMAL_PIN`.
2. Pin the 5-variant DecimalError set so a future renaming surfaces in
   `crates/verum_database_postgres/src/numeric.rs` codec immediately.
