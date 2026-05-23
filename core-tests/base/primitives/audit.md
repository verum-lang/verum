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

### §C `()` (Unit) method dispatch escapes Unit-typed receivers — CLOSED in this branch

**Defect class** — pre-fix `().cmp(&())` panicked with `method 'Once.next'
not found on receiver of runtime kind ()` because the dispatcher's
bare-method suffix-scan picked the `Iterator.cmp` default body
(monomorphised onto `Once<T>` during stdlib precompile) over the inherent
`implement Ord for () { fn cmp(...) }` impl.  Iterator.cmp's body
executes `self.next()` → `CallM { method_id: "Once.next" }` (because the
body was monomorphised with `self_type = Once`), and Unit has no `.next`
method, so the dispatcher panicked with the misleading error.

**Blast radius** — every protocol method on `()`: Eq.eq / Ord.cmp /
Hash.hash_value / Clone.clone / Default.default / Display.fmt were all
reachable but routed unpredictably depending on HashMap-iteration-order
of the function table.  Affected `bool_byte_char_test::test_unit_ord_protocol`
+ `test_unit_default` in this conformance suite.

**Fix** — Unit-receiver intercept in
`dispatch_primitive_method` that returns the mathematical identity for
each protocol method.  `()` has exactly one value, so every method
reduces to a constant:

  * `().cmp(&()) == Equal`, `() == ()`, `() <= ()`, `() >= ()`,
    `!(() < ())`, `!(() > ())`.
  * `().hash_value() == fxhash_bytes(0, &[])` (seed-0 unchanged).
  * `().clone() == ()`, `Unit.default() == ()`.
  * `f"{()}" == "()"`, `().to_text() == "()"`, etc.

**Architectural rule pinned**: every primitive's protocol-default surface
MUST be reachable through `dispatch_primitive_method` so the
function-table-side suffix-scan never wins over the canonical primitive
intercept.  Pinned by `regression_test.vr::regression_unit_receiver_dispatch_pinned`.

## 3. Action items

### Landed in this branch

  * §A — Three-site Cast arm in `crates/verum_vbc/src/codegen/expressions.rs`
    (extract_expr_type_name / infer_expr_type_name /
    compile_method_call::effective_method_name Case 6c) + runtime
    width-prefix normalisation in `dispatch_primitive_method`.  Wraps and
    checked arithmetic on cast-receiver Byte now correct.
  * §B — `test_char_from_digit` updated to expect canonical lowercase.
  * §C — Unit-receiver intercept in `dispatch_primitive_method` returns
    canonical values for every protocol method on `()`.  Closes the
    "Iterator.cmp default body monomorphised onto Once<T> bleeds into
    Unit receiver" defect class.
  * New `regression_test.vr` with 4 regression pins (Char.from_digit
    lowercase + Cast-receiver wrap u8 + Cast-receiver checked u8 + Unit
    receiver dispatch).
  * 2 `@ignore`'d pins for residual saturating_add intrinsic-width
    propagation + let-bound cast `VarTypeKind::Byte` folding.
  * Test contract fix — `test_byte_is_ascii_whitespace` now correctly
    asserts that 0x0C (form feed) IS canonical ASCII whitespace per
    Rust / `core/text/char.vr` / `core/base/primitives.vr` and 0x0B
    (vertical tab) is NOT (POSIX-aligned).
  * `is_builtin_prefix` allowlist extended in `dispatch_primitive_method`
    to cover every sized-int alias (Byte/UInt8/U8/u8/Int8/I8/i8/UInt16/
    U16/u16/Int16/I16/i16/Int32/I32/i32/UInt64/U64/u64/Int64/I64/i64/
    UInt128/U128/u128/Int128/I128/i128/USize/UIntSize/Usize/usize/
    ISize/IntSize/Isize/isize).  Unblocks the width-prefix normaliser
    for typechecker-resolved alias-canonical-name CallM forms.  New
    regression pin `regression_byte_alias_dispatch_through_normaliser_pinned`.

### §D `<TypeName>.<Variant>.<method>()` codegen — CLOSED 2026-05-23

  * Pre-fix every chained `<EnumType>.<Variant>.<method>()` shape
    panicked with `undefined variable: <EnumType>`.  Affected
    `LogLevel.Trace.name()`, `Ordering.Less.reverse()`,
    `Maybe.None.is_none()`, `f"{Severity.Warn.name()}"` interpolation,
    and the entire ord_default_comparison / log / severity test class.
  * Root cause: the `field_writeback_target` block at
    `crates/verum_vbc/src/codegen/expressions.rs:~8142` called
    `compile_expr(base)` on the bare `Path(TypeName)` (e.g.
    `Path(LogLevel)`).  `compile_simple_path` then rejected the
    type name as undefined because user-defined sum types are NOT in
    the `is_type_name` allowlist (which only covers stdlib well-known
    types).  Pre-fix the only way to invoke a variant's method was via
    let-binding (`let x = LogLevel.Trace; x.name()`) which bypassed
    this code path.
  * Fix: two surgical fast paths.
      * `compile_method_call` Phase 0a — detect
        `Field { Path(TypeName), Variant }` receiver with
        `<TypeName>.<Variant>` resolving to a 0-arg variant ctor;
        materialise via `MakeVariant` + emit `CallM` directly.
      * `compile_field_access` type-name fast path — same shape but
        without the `.method()` suffix.  Probes the function table
        for `<TypeName>.<field>` before the existing flatten path
        runs.
  * Acceptance gate: leading uppercase + not a local register +
    qualified lookup returns a variant ctor with `param_count == 0`.
    Dodges the `is_type_name` allowlist limitation that gated
    user-defined type names out of the fast path.
  * New regression pin `regression_chained_type_variant_method_pinned`
    covers `Ordering.<Variant>.reverse()` ordering identity.

### Deferred

  * §A residual — `saturating_add` intrinsic width propagation.  The
    codegen emits `ArithExtendedOpcode(SaturatingAdd)` directly (not
    `CallM`), and the opcode reads a `width` byte from the bytecode
    stream that defaults to 64 (i64 semantics) for the bare
    `saturating_add` intrinsic.  Fix needs width propagation through
    the intrinsic-emission path keyed on the cast target's spelling.
  * §A residual — let-bound cast `VarTypeKind` folding.  Unannotated
    `let a = N as Byte` lands as `VarTypeKind::Int` because the
    expr-type inference path at `compile_let:341` folds primitive
    integer widths to `TypeKind::Int → VarTypeKind::Int`.  Needs a
    cast-target-aware step that recovers the width spelling for
    primitive-byte-shape RHS expressions.
  * Cross-tier AOT validation — verify all green interpreter tests also
    pass under `verum test --aot`.  Pending separate AOT validation
    cycle for cast-receiver dispatch.
  * ~~`test_byte_is_ascii_whitespace`~~ — CLOSED via test contract
    update: 12 (form feed) IS canonical ASCII whitespace per Rust /
    `core/text/char.vr` / `core/base/primitives.vr`.
  * ~~`test_byte_case_conversion_roundtrip`~~ — CLOSED via
    `is_builtin_prefix` allowlist extension in
    `dispatch_primitive_method`.  Pre-fix the allowlist missed `UInt8`
    (and U8/u8/I32/etc.); the typechecker resolves cast receivers via
    the alias canonical name (`UInt8` is Byte's canonical name in
    `NUMERIC_ALIAS_MATRIX`), so qualified-prefix CallM dispatch
    short-circuited at the early-return gate before reaching the
    width-prefix normaliser.  Allowlist now covers every sized-int
    alias spelling (canonical + uppercase-short + Rust-style lowercase).
