# `meta/reflection` audit

Module: `core/meta/reflection.vr` (~1059 LOC) — compile-time type
introspection data shapes consumed by the `TypeInfo` context.

Tests: 58 unit tests over the pure-data subset (variant enums +
FieldOffset/OwnershipInfo records with no Span dependencies).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.meta.contexts.TypeInfo` | every `TypeInfo.foo_of<T>()` accessor returns one of these data types |
| `core.meta.diakrisis_attrs` | not used directly |
| `verum_compiler::derives::*` | every `@derive(Debug/Clone/Eq/...)` consumes FieldInfo / VariantInfo / GenericParam |
| `verum_compiler::reflection::lower` | translates Rust-side reflection state into these stdlib types |
| `verum_lsp::completion` | reads ProtocolInfo / FunctionInfo to compute auto-complete suggestions |

## 2. Crate-side hardcodes

* `verum_compiler::reflection::TypeKind` mirrors the 17-variant
  TypeKind. Adding a kind requires changes in 3 places (this
  file, the compiler enum, the lowering).
* `verum_compiler::reflection::Visibility` mirrors the 5-variant
  Visibility (Public / Private / Crate / Super / In(Path)).
  Note the `In(Path)` variant payload: parser-side this is a
  `Path` AST node, stdlib-side it's a flat `Text` — the lowering
  in `verum_compiler::lower::vis` flattens the path.
* `verum_compiler::reflection::PrimitiveType` mirrors the 18-variant
  primitive type enum + the `.size()` table that hardcodes byte
  sizes (Bool=1, Char=4, Int8/UInt8=1, Int16/UInt16=2, …, Text=24).
  Drift here would break `@derive(Repr)` calculations.

## 3. Language-implementation gaps

### §3.1 `Visibility.keyword` for `In(path)` materialises a Text by interpolation

```verum
In(path) => Some(f"public(in {path})"),
```

`f"…"` format strings call `Text.format` which is a cross-module
fn return — same defect class as [meta/span audit §3.1]. Not
exercised in this folder; covered at the proof-level for the
audit walker.

### §3.2 Records depending on Span / Attribute are not unit-testable here

FieldInfo / VariantInfo / GenericParam / ProtocolInfo / FunctionInfo /
TraitBound / LifetimeParam / MethodResolution / ParamInfo /
AssociatedTypeInfo all carry a `Span` field and/or `List<Attribute>`.
Constructing them directly requires constructing a Span (works
via direct-field ctor) and Attribute (also works via direct ctor),
which is feasible but tedious. The first integration test in this
folder should exercise FieldInfo's `.has_attribute` / `.get_attribute`
once the Attribute ctor cross-module path is solid.

### §3.3 `PrimitiveType` size table is stdlib-source-canonical

The `.size()` table in `PrimitiveType` is the single source of
truth for primitive byte-sizes across the entire stdlib. The
codegen MUST agree with it for `T.size` (canonical
type-property form) under `--interp`. Pinned by
`test_primitive_is_signed_int8` / `test_primitive_is_float_f32` /
... — every test that hits a size or signedness predicate.

Drift-pinning Rust unit test suggested:

```rust
#[test]
fn primitive_size_matches_stdlib_declared() {
    use verum_compiler::reflection::PrimitiveType as P;
    assert_eq!(P::Int8.size(),    1);
    assert_eq!(P::Int16.size(),   2);
    // ... full table
}
```

## Action items landed in this branch

* `core-tests/meta/reflection/unit_test.vr` — 58 unit tests over:
  - TypeKind 17-variant + .is_compound / .is_reference /
    .is_primitive predicates
  - Visibility 5-variant
  - GenericParamKind 3-variant
  - VariantKind 3-variant
  - SelfKind 3-variant
  - MethodSource 4-variant
  - PrimitiveType 18-variant (sample) + .is_signed / .is_unsigned /
    .is_float / .is_numeric
  - FieldOffset record + .end / .has_padding
  - OwnershipInfo record + .is_thread_safe / .is_trivially_copyable /
    .requires_cleanup
* `core-tests/meta/reflection/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Property test: `TypeKind.is_compound | is_reference | is_primitive` are pairwise disjoint and exhaustive | this folder | 30 min |
| Integration test: FieldInfo `.has_attribute` / `.get_attribute` round-trip | this folder | 1 h |
| Drift-pinning Rust unit test for `PrimitiveType.size` table (§3.3) | crates/verum_compiler/src/reflection/tests.rs | 30 min |
| Property test: `Visibility.is_public` ⇔ variant is Public | this folder | 15 min |
| Tests for `FieldInfo.accessor` (`.0` vs `.name`) once cross-module field-access defect closes | this folder | 30 min |
| Tests for `VariantInfo.pattern` / `.wildcard_pattern` once Span construction is fixed | this folder | 1 h |
