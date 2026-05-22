// =============================================================================
# `base/primitives` audit
// =============================================================================

Module: `core/base/primitives.vr` — inherent methods on `Int`, `Float`, `Bool`,
`Char`, `Byte`, `Unit`, and the sized integer families (`Int8/16/32/64/128`,
`UInt8/16/32/64/128`, `ISize`, `USize`).

This is **the most fundamental module** in `core/`: every other module either
uses primitive arithmetic / comparison / cast operations or imports a type
defined here.  A defect here cascades across the entire stdlib + every user
program.

## 1. Test surface

| File | LOC | @test | what it covers |
|------|----:|------:|----------------|
| `int_test.vr`              | 875 | ~100 | Int: arithmetic / wrapping / saturating / checked / bit ops / parsing / formatting |
| `float_test.vr`            | 997 | ~95  | Float: IEEE 754 algebra, NaN/Inf, transcendentals, rounding, parsing |
| `uint_isize_test.vr`       | 177 | ~25  | UInt / ISize / USize family — width-specific arithmetic + cast lattice |
| `bool_byte_char_test.vr`   | 839 | 60+  | Bool / Byte / Char / Unit protocols (Eq / Ord / Hash / Clone / Default / Display / Debug) and inherent methods |
| `comparison_test.vr`       |1059 |  60+ | Cross-type cmp/eq laws (Int↔Int, Int↔Float, Byte↔Byte, …) |
| `bitmanip_test.vr`         | 146 |   15 | Bit-manipulation intrinsics (count_ones, leading_zeros, rotate_left/right, swap_bytes) |
| `arithmetic_edge_test.vr`  | 161 |   16 | Edge cases (overflow, underflow, MIN/MAX boundaries, signed-vs-unsigned interactions) |
| `property_test.vr`         | 196 |  15  | Cross-cutting algebraic laws (associativity, commutativity, distributivity, identity, double-negation) |
| `regression_test.vr`       |  ~95 |   3  | One @test per past defect — see §2 |

Total ~4540 LOC.  The split-by-type layout (rather than the canonical
unit/property/integration triad) reflects that primitives are tested
*per-type-axis* — Int's API surface is mostly disjoint from Float's, and
mixing them would obscure failure attribution.

## 2. Language-implementation gaps

### §A `as Byte` / `as <SizedInt>` cast loses target type for method dispatch — CLOSED in this branch

**Defect class** — pre-fix the VBC codegen's `extract_expr_type_name` and
`infer_expr_type_name` (in
`crates/verum_vbc/src/codegen/expressions.rs`) had NO `ExprKind::Cast` arm.
Both fell through to `_ => None`, so method dispatch on a cast-receiver
used the *inner* expression's type (`Int` for integer literals), not the
cast's target type.

Reproduction:

```verum
// Form A: cast-as-receiver
(255 as Byte).saturating_add(1 as Byte)  // → 256, EXPECTED 255

// Form B: let-bound cast
let a = 255 as Byte;
let b = 1 as Byte;
a.saturating_add(b)                       // → 256, EXPECTED 255

// Form C: annotated let (worked)
let c: Byte = 255;
let d: Byte = 1;
c.saturating_add(d)                       // → 255, CORRECT
```

Method dispatch on Forms A+B routed through `Int.saturating_add` (which has
no 255 ceiling) instead of `Byte.saturating_add`, returning the
unsaturated sum.

**Blast radius** — every primitive-sized-int cast where the dispatcher
relies on `extract_expr_type_name` / `infer_expr_type_name` to route to
the correct primitive impl.  Affected types: Byte, UInt8/16/32/64,
Int8/16/32, Char-as-Int conversions, Float-as-X.  Affected methods
include every per-type wrapping/saturating/checked arithmetic, every
per-type `Eq`/`Ord`/`Hash` dispatch, every per-type formatting and
parsing method.  The defect was masked in Form C (type-annotated `let`)
because the type-annotation directly populated `variable_type_names`,
bypassing the missing Cast arm.

**Fix** — extended both `extract_expr_type_name` and
`infer_expr_type_name` with an `ExprKind::Cast { ty, .. }` arm that
returns the target-type name from `ty.kind` (mirrors `infer_expr_type_kind`'s
existing Cast arm and the normalisation in `compile_cast`, so all three
sites agree on the recognised primitive name set).

**Architectural rule pinned**: `as TargetType` MUST propagate `TargetType`
to every codegen-side type-extraction function so all downstream
method-dispatch sites see the cast type, not the source type.  Pinned by
`regression_test.vr::regression_as_byte_cast_propagates_type_to_dispatch_pinned`
and `regression_let_bound_as_cast_propagates_type_pinned`.

### §B `Char.from_digit` returns lowercase for radix > 10 — TEST FIX in this branch

**Defect class** — pre-fix two `Char.from_digit` definitions existed in
the stdlib.  `core/base/primitives.vr` returned UPPERCASE letters
(`A`..=`Z`) for hex digits 10..=35.  `core/text/char.vr` returned
LOWERCASE (`a`..=`z`), matching the standard `from_digit` convention
shared with Rust / Swift / Python.  `archive_ctx_loader` registered bare
`Char.from_digit` first-wins, picking whichever loaded first.

Closed in task #22 §C (commit `24c4e0155`) by aligning primitives.vr to
lowercase.  The test `test_char_from_digit` in `bool_byte_char_test.vr`
was still asserting the pre-fix UPPERCASE expectation — updated in this
branch to match the canonical lowercase contract.

**Architectural rule pinned**: there must be EXACTLY ONE
`Char.from_digit` definition in the stdlib.  Canonical home:
`core/text/char.vr`.  Callers that need uppercase output should chain
`.to_ascii_uppercase()` on the result.  Pinned by
`regression_test.vr::regression_char_from_digit_returns_lowercase_pinned`.

### §C `()` (Unit) method dispatch escapes Unit-typed receivers — OPEN

Surface: `bool_byte_char_test::test_unit_ord_protocol`,
`test_unit_default`, and several Once/Iterator chains report
`method 'Once.next' not found on receiver of runtime kind ()` — the
dispatcher is somehow mapping `().cmp(...)` to `Once.next(...)` candidate
class.  Root cause not yet investigated.  Suspected: shared codegen
classifier for Unit receivers escaping to the IntoIterator fallback path.

**Action**: investigate the codegen path that emits `CallM` for receiver
of kind Unit, and the `IntoIterator`/`Once<T>` candidate scan in the
dispatch panic builder, then write a regression pin.

## 3. Action items

### Landed in this branch

  * §A — Cast arm added to `extract_expr_type_name` AND `infer_expr_type_name`
    in `crates/verum_vbc/src/codegen/expressions.rs`.
  * §B — `test_char_from_digit` updated to expect canonical lowercase.
  * New `regression_test.vr` with three regression pins (Char.from_digit
    lowercase, Cast type-propagation receiver-form, Cast type-propagation
    let-binding-form).

### Deferred

  * §C — Unit-receiver method-dispatch defect.  Requires tracing the
    codegen path that emits the wrong `CallM` opcode for `().cmp(...)`.
    Scope estimate: 1 day.
  * Cross-tier AOT validation — verify all green interpreter tests also
    pass under `verum test --aot`.  Pending stdlib precompile / binary
    rebuild cycle to complete.
