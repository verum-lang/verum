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

### §B — Cross-module record-layout loss in match destructure — **CLOSED 2026-05-15**
**Symptom**: `match parse_X(&"...") { Ok(v) => v.field }` panics with
"field access out of bounds: field index 4 (offset 32+8=40) exceeds
object data size 16" for X ∈ {bigint, bigdecimal, rational}, fine
for decimal.  Earlier audit (and earlier `[FunctionNotFound]`
manifestation) was the same root cause.

**Root cause**: `archive_ctx_loader::primitive_typeid_name`
hardcoded only the 16 numeric/scalar primitive TypeIds — `Result`
(TypeId 516) / `Maybe` (515) / `List` (512) and other well-known
GENERIC carriers were NOT recognised.  When a function's return
type was `Result<X, E>`, `type_ref_simple_name` looked up
`Result`'s TypeId in `module.types` (which only carries types
DEFINED in THIS module) and got None.  The call's `return_type_name`
was therefore `None`, even though `return_type_inner` correctly
held `["X", "E"]`.  Downstream `extract_expr_type_name` couldn't
form `"Result<X, E>"`, `compile_match` lost the scrutinee type, and
`compile_pattern_bind`'s `Ok(v)` arm fell through to the global
field-intern fallback for `v.field`, surfacing as
`field index 4 (offset 32+8=40) exceeds object data size 16`.

**Fix** (two-layer):
1. `archive_ctx_loader::primitive_typeid_name`: added the 15
   well-known generic-carrier TypeIds (Maybe, Result, List, Map,
   Set, Deque, Channel, Range, Array, Heap, Shared, Tuple, Pi,
   Sigma, Witness) so cross-module return types resolve their
   base names regardless of which module's perspective the
   archive is read from.
2. `extract_expr_type_name`'s Call arm in `compile_unary`:
   compose the canonical generic form
   `format!("{}<{}>", ret_type, return_type_inner.join(", "))`
   when inner args are present and ret_type lacks `<` — preserving
   the full instantiation through type inference.

**Architectural rule**: every TypeId reserved by the runtime
that may appear as a function return type, field type, or
parameter type MUST be recognised by `primitive_typeid_name`.
Cross-module nominal identity must NOT depend on the module-local
descriptor table being populated.

**Validated**: 4-of-4 `test_parse_simple_int` instances (decimal,
bigint, bigdecimal, rational) all green; `test_add` 13-of-19
green (residual: Duration arithmetic — separate Time-module class).

### §C — Arithmetic semantic failures (downstream of §A/§B) — **PARTIAL**
**Status**: §A and §B closure unblocks the construction surface;
some arithmetic tests pass.  Residual failures in `test_add` /
`test_mul` chains concentrate on Duration / Time module (separate
defect class — task #21).

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
