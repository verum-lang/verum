//! Standard attribute registration for Verum.
//!
//! This module registers all built-in Verum attributes with the registry.
//! Attributes are organized by category for maintainability.

use verum_ast::attr::{
    // Typed attributes for association
    AlignAttr,
    ArgSpec,
    ArgType,
    AttributeCategory,
    AttributeMetadata,
    AttributeTarget,
    BitfieldAttr,
    BitOffsetAttr,
    BitsAttr,
    ColdAttr,
    EndianAttr,
    FeatureAttr,
    HotAttr,
    InlineAttr,
    LockLevelAttr,
    NamedArgSpec,
    OptimizeAttr,
    ProfileAttr,
    ReprAttr,
    SpecializeAttr,
    Stability,
    StdAttr,
    UnrollAttr,
    VectorizeAttr,
    VerifyAttr,
};

use super::registry::AttributeRegistry;

/// Register all standard Verum attributes with the registry.
pub fn register_standard_attributes(registry: &mut AttributeRegistry) {
    register_optimization_attributes(registry);
    register_serialization_attributes(registry);
    register_validation_attributes(registry);
    register_documentation_attributes(registry);
    register_safety_attributes(registry);
    register_layout_attributes(registry);
    register_bitfield_attributes(registry);
    register_module_attributes(registry);
    register_language_core_attributes(registry);
    register_concurrency_attributes(registry);
    register_meta_system_attributes(registry);
    register_testing_attributes(registry);
    register_ffi_attributes(registry);
}

// =============================================================================
// OPTIMIZATION ATTRIBUTES
// Optimization hints: @inline, @cold, @likely annotations for guiding compiler optimizations
// =============================================================================

fn register_optimization_attributes(registry: &mut AttributeRegistry) {
    // @inline, @inline(always), @inline(never), @inline(release)
    registry
        .register(
            AttributeMetadata::new("inline")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::Optional(ArgType::Ident))
                .category(AttributeCategory::Optimization)
                .doc("Control function inlining behavior")
                .doc_extended(
                    r#"
## Usage
- `@inline` - Suggest inlining (compiler decides)
- `@inline(always)` - Always inline
- `@inline(never)` - Never inline (cold paths)
- `@inline(release)` - Inline only in release builds

## Example
```verum
@inline(always)
fn hot_path(x: Int) -> Int { x * 2 }
```
"#,
                )
                .conflicts_with(["cold"])
                .typed_as::<InlineAttr>()
                .builtin()
                .build(),
        )
        .expect("inline registration");

    // @cold
    registry
        .register(
            AttributeMetadata::new("cold")
                .targets(AttributeTarget::Function | AttributeTarget::MatchArm)
                .args(ArgSpec::None)
                .category(AttributeCategory::Optimization)
                .doc("Mark code path as rarely executed")
                .conflicts_with(["hot", "inline"])
                .typed_as::<ColdAttr>()
                .builtin()
                .build(),
        )
        .expect("cold registration");

    // @hot
    registry
        .register(
            AttributeMetadata::new("hot")
                .targets(AttributeTarget::Function | AttributeTarget::MatchArm)
                .args(ArgSpec::None)
                .category(AttributeCategory::Optimization)
                .doc("Mark code path as frequently executed")
                .conflicts_with(["cold"])
                .typed_as::<HotAttr>()
                .builtin()
                .build(),
        )
        .expect("hot registration");

    // @optimize
    registry
        .register(
            AttributeMetadata::new("optimize")
                .targets(AttributeTarget::Function | AttributeTarget::Module)
                .args(ArgSpec::Required(ArgType::Ident))
                .category(AttributeCategory::Optimization)
                .doc("Override optimization level: @optimize(size|speed|none|balanced)")
                .typed_as::<OptimizeAttr>()
                .builtin()
                .build(),
        )
        .expect("optimize registration");

    // @vectorize
    registry
        .register(
            AttributeMetadata::new("vectorize")
                .targets(AttributeTarget::Loop | AttributeTarget::Function)
                .args(ArgSpec::Optional(ArgType::Ident))
                .category(AttributeCategory::Optimization)
                .doc("Control loop vectorization")
                .typed_as::<VectorizeAttr>()
                .builtin()
                .build(),
        )
        .expect("vectorize registration");

    // @simd (alias for vectorize)
    registry
        .register(
            AttributeMetadata::new("simd")
                .targets(AttributeTarget::Loop | AttributeTarget::Function)
                .args(ArgSpec::Optional(ArgType::Ident))
                .category(AttributeCategory::Optimization)
                .doc("Enable SIMD vectorization")
                .builtin()
                .build(),
        )
        .expect("simd registration");

    // @no_vectorize
    registry
        .register(
            AttributeMetadata::new("no_vectorize")
                .targets(AttributeTarget::Loop | AttributeTarget::Function)
                .args(ArgSpec::None)
                .category(AttributeCategory::Optimization)
                .doc("Disable vectorization")
                .builtin()
                .build(),
        )
        .expect("no_vectorize registration");

    // @unroll
    registry
        .register(
            AttributeMetadata::new("unroll")
                .targets(AttributeTarget::Loop)
                .args(ArgSpec::Required(ArgType::UInt))
                .category(AttributeCategory::Optimization)
                .doc("Unroll loop N times: @unroll(4)")
                .typed_as::<UnrollAttr>()
                .builtin()
                .build(),
        )
        .expect("unroll registration");

    // @no_unroll
    registry
        .register(
            AttributeMetadata::new("no_unroll")
                .targets(AttributeTarget::Loop)
                .args(ArgSpec::None)
                .category(AttributeCategory::Optimization)
                .doc("Prevent loop unrolling")
                .builtin()
                .build(),
        )
        .expect("no_unroll registration");

    // @parallel
    registry
        .register(
            AttributeMetadata::new("parallel")
                .targets(AttributeTarget::Loop)
                .args(ArgSpec::None)
                .category(AttributeCategory::Optimization)
                .doc("Enable parallel loop execution")
                .builtin()
                .build(),
        )
        .expect("parallel registration");

    // @likely
    registry
        .register(
            AttributeMetadata::new("likely")
                .targets(AttributeTarget::Expr | AttributeTarget::MatchArm)
                .args(ArgSpec::None)
                .category(AttributeCategory::Optimization)
                .doc("Mark branch as likely to be taken")
                .conflicts_with(["unlikely"])
                .builtin()
                .build(),
        )
        .expect("likely registration");

    // @unlikely
    registry
        .register(
            AttributeMetadata::new("unlikely")
                .targets(AttributeTarget::Expr | AttributeTarget::MatchArm)
                .args(ArgSpec::None)
                .category(AttributeCategory::Optimization)
                .doc("Mark branch as unlikely to be taken")
                .conflicts_with(["likely"])
                .builtin()
                .build(),
        )
        .expect("unlikely registration");

    // @prefetch
    registry
        .register(
            AttributeMetadata::new("prefetch")
                .targets(AttributeTarget::Stmt | AttributeTarget::Expr)
                .args(ArgSpec::Named(vec![
                    NamedArgSpec::optional("access", ArgType::Ident), // read|write
                    NamedArgSpec::optional("locality", ArgType::UInt), // 0-3
                ].into()))
                .category(AttributeCategory::Optimization)
                .doc("Prefetch data into cache")
                .builtin()
                .build(),
        )
        .expect("prefetch registration");

    // @assume
    registry
        .register(
            AttributeMetadata::new("assume")
                .targets(AttributeTarget::Stmt | AttributeTarget::Function)
                .args(ArgSpec::Required(ArgType::Expr))
                .category(AttributeCategory::Optimization)
                .doc("Provide optimization hint: @assume(data.len() % 8 == 0)")
                .builtin()
                .build(),
        )
        .expect("assume registration");

    // @no_alias
    registry
        .register(
            AttributeMetadata::new("no_alias")
                .targets(AttributeTarget::Loop | AttributeTarget::Function | AttributeTarget::Param)
                .args(ArgSpec::None)
                .category(AttributeCategory::Optimization)
                .doc("Assert no pointer aliasing")
                .builtin()
                .build(),
        )
        .expect("no_alias registration");

    // @ivdep
    registry
        .register(
            AttributeMetadata::new("ivdep")
                .targets(AttributeTarget::Loop)
                .args(ArgSpec::None)
                .category(AttributeCategory::Optimization)
                .doc("Ignore vector dependencies in loop")
                .builtin()
                .build(),
        )
        .expect("ivdep registration");

    // @black_box
    registry
        .register(
            AttributeMetadata::new("black_box")
                .targets(AttributeTarget::Expr)
                .args(ArgSpec::None)
                .category(AttributeCategory::Optimization)
                .doc("Prevent optimization of value (for benchmarks)")
                .builtin()
                .build(),
        )
        .expect("black_box registration");

    // @optimize_barrier
    registry
        .register(
            AttributeMetadata::new("optimize_barrier")
                .targets(AttributeTarget::Stmt)
                .args(ArgSpec::None)
                .category(AttributeCategory::Optimization)
                .doc("Prevent optimization across this point")
                .builtin()
                .build(),
        )
        .expect("optimize_barrier registration");

    // @target_cpu
    registry
        .register(
            AttributeMetadata::new("target_cpu")
                .targets(AttributeTarget::Function | AttributeTarget::Module)
                .args(ArgSpec::Required(ArgType::String))
                .category(AttributeCategory::Optimization)
                .doc("Target specific CPU: @target_cpu(\"native\")")
                .builtin()
                .build(),
        )
        .expect("target_cpu registration");

    // @target_feature
    registry
        .register(
            AttributeMetadata::new("target_feature")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::Required(ArgType::String))
                .category(AttributeCategory::Optimization)
                .doc("Enable CPU features: @target_feature(\"+avx2,+fma\")")
                .builtin()
                .build(),
        )
        .expect("target_feature registration");
}

// =============================================================================
// SERIALIZATION ATTRIBUTES
// Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Section 13.7.1
// =============================================================================

fn register_serialization_attributes(registry: &mut AttributeRegistry) {
    // @serialize
    registry
        .register(
            AttributeMetadata::new("serialize")
                .targets(AttributeTarget::Type | AttributeTarget::Field | AttributeTarget::Variant)
                .args(ArgSpec::Named(vec![
                    NamedArgSpec::optional("rename", ArgType::String),
                    NamedArgSpec::optional("as", ArgType::Expr),
                    NamedArgSpec::optional("with", ArgType::Path),
                ].into()))
                .category(AttributeCategory::Serialization)
                .doc("Enable or customize serialization")
                .builtin()
                .build(),
        )
        .expect("serialize registration");

    // @deserialize
    registry
        .register(
            AttributeMetadata::new("deserialize")
                .targets(AttributeTarget::Type | AttributeTarget::Field)
                .args(ArgSpec::Named(vec![
                    NamedArgSpec::optional("rename", ArgType::String),
                    NamedArgSpec::optional("default", ArgType::Expr),
                    NamedArgSpec::optional("with", ArgType::Path),
                ].into()))
                .category(AttributeCategory::Serialization)
                .doc("Enable or customize deserialization")
                .builtin()
                .build(),
        )
        .expect("deserialize registration");

    // @skip_serialize
    registry
        .register(
            AttributeMetadata::new("skip_serialize")
                .targets(AttributeTarget::Field)
                .args(ArgSpec::None)
                .category(AttributeCategory::Serialization)
                .doc("Exclude field from serialization")
                .builtin()
                .build(),
        )
        .expect("skip_serialize registration");

    // @skip_deserialize
    registry
        .register(
            AttributeMetadata::new("skip_deserialize")
                .targets(AttributeTarget::Field)
                .args(ArgSpec::None)
                .category(AttributeCategory::Serialization)
                .doc("Exclude field from deserialization")
                .builtin()
                .build(),
        )
        .expect("skip_deserialize registration");

    // @rename
    registry
        .register(
            AttributeMetadata::new("rename")
                .targets(AttributeTarget::Field | AttributeTarget::Variant)
                .args(ArgSpec::Named(vec![NamedArgSpec::required(
                    "name",
                    ArgType::String,
                )].into()))
                .category(AttributeCategory::Serialization)
                .doc("Use alternative name in serialization")
                .builtin()
                .build(),
        )
        .expect("rename registration");

    // @flatten
    registry
        .register(
            AttributeMetadata::new("flatten")
                .targets(AttributeTarget::Field)
                .args(ArgSpec::None)
                .category(AttributeCategory::Serialization)
                .doc("Inline nested fields during serialization")
                .builtin()
                .build(),
        )
        .expect("flatten registration");

    // @default (for deserialization)
    registry
        .register(
            AttributeMetadata::new("default")
                .targets(AttributeTarget::Field | AttributeTarget::Variant)
                .args(ArgSpec::Optional(ArgType::Expr))
                .category(AttributeCategory::Serialization)
                .doc("Provide default value for missing fields")
                .builtin()
                .build(),
        )
        .expect("default registration");
}

// =============================================================================
// VALIDATION ATTRIBUTES
// Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Section 13.7.1
// =============================================================================

fn register_validation_attributes(registry: &mut AttributeRegistry) {
    // @validate
    registry
        .register(
            AttributeMetadata::new("validate")
                .targets(AttributeTarget::Type | AttributeTarget::Field)
                .args(ArgSpec::Named(vec![
                    NamedArgSpec::optional("min_length", ArgType::UInt),
                    NamedArgSpec::optional("max_length", ArgType::UInt),
                    NamedArgSpec::optional("min", ArgType::Expr),
                    NamedArgSpec::optional("max", ArgType::Expr),
                ].into()))
                .category(AttributeCategory::Validation)
                .doc("Enable validation for type/field")
                .builtin()
                .build(),
        )
        .expect("validate registration");

    // @range
    registry
        .register(
            AttributeMetadata::new("range")
                .targets(AttributeTarget::Field)
                .args(ArgSpec::Named(vec![
                    NamedArgSpec::optional("min", ArgType::Expr),
                    NamedArgSpec::optional("max", ArgType::Expr),
                ].into()))
                .category(AttributeCategory::Validation)
                .doc("Numeric range constraint")
                .builtin()
                .build(),
        )
        .expect("range registration");

    // @length
    registry
        .register(
            AttributeMetadata::new("length")
                .targets(AttributeTarget::Field)
                .args(ArgSpec::Named(vec![
                    NamedArgSpec::optional("min", ArgType::UInt),
                    NamedArgSpec::optional("max", ArgType::UInt),
                    NamedArgSpec::optional("exact", ArgType::UInt),
                ].into()))
                .category(AttributeCategory::Validation)
                .doc("Length constraint for text/collections")
                .builtin()
                .build(),
        )
        .expect("length registration");

    // @pattern
    registry
        .register(
            AttributeMetadata::new("pattern")
                .targets(AttributeTarget::Field)
                .args(ArgSpec::Required(ArgType::String))
                .category(AttributeCategory::Validation)
                .doc("Regex pattern constraint: @pattern(\"^[a-z]+$\")")
                .builtin()
                .build(),
        )
        .expect("pattern registration");

    // @custom (validation)
    registry
        .register(
            AttributeMetadata::new("custom")
                .targets(AttributeTarget::Field | AttributeTarget::Type)
                .args(ArgSpec::Required(ArgType::Path))
                .category(AttributeCategory::Validation)
                .doc("Custom validator function: @custom(my_validator)")
                .builtin()
                .build(),
        )
        .expect("custom validation registration");
}

// =============================================================================
// DOCUMENTATION ATTRIBUTES
// =============================================================================

fn register_documentation_attributes(registry: &mut AttributeRegistry) {
    // @doc
    registry
        .register(
            AttributeMetadata::new("doc")
                .targets(AttributeTarget::All)
                .args(ArgSpec::Required(ArgType::String))
                .category(AttributeCategory::Documentation)
                .doc("Documentation string")
                .repeatable()
                .builtin()
                .build(),
        )
        .expect("doc registration");

    // @deprecated
    registry
        .register(
            AttributeMetadata::new("deprecated")
                .targets(AttributeTarget::All)
                .args(ArgSpec::Named(vec![
                    NamedArgSpec::optional("since", ArgType::String),
                    NamedArgSpec::optional("use", ArgType::String),
                    NamedArgSpec::optional("reason", ArgType::String),
                ].into()))
                .category(AttributeCategory::Documentation)
                .doc("Mark item as deprecated")
                .builtin()
                .build(),
        )
        .expect("deprecated registration");

    // @experimental
    registry
        .register(
            AttributeMetadata::new("experimental")
                .targets(AttributeTarget::All)
                .args(ArgSpec::None)
                .category(AttributeCategory::Documentation)
                .doc("Mark API as experimental/unstable")
                .builtin()
                .build(),
        )
        .expect("experimental registration");

    // @todo
    registry
        .register(
            AttributeMetadata::new("todo")
                .targets(AttributeTarget::All)
                .args(ArgSpec::Optional(ArgType::String))
                .category(AttributeCategory::Documentation)
                .doc("Mark incomplete implementation")
                .repeatable()
                .builtin()
                .build(),
        )
        .expect("todo registration");

    // @unused
    registry
        .register(
            AttributeMetadata::new("unused")
                .targets(AttributeTarget::Param | AttributeTarget::Field | AttributeTarget::Item)
                .args(ArgSpec::None)
                .category(AttributeCategory::Documentation)
                .doc("Suppress unused warnings")
                .builtin()
                .build(),
        )
        .expect("unused registration");

    // @allow
    registry
        .register(
            AttributeMetadata::new("allow")
                .targets(AttributeTarget::All)
                .args(ArgSpec::Variadic(ArgType::Ident))
                .category(AttributeCategory::Documentation)
                .doc("Suppress specific warnings: @allow(dead_code)")
                .repeatable()
                .builtin()
                .build(),
        )
        .expect("allow registration");

    // @warn
    registry
        .register(
            AttributeMetadata::new("warn")
                .targets(AttributeTarget::All)
                .args(ArgSpec::Variadic(ArgType::Ident))
                .category(AttributeCategory::Documentation)
                .doc("Enable specific warnings")
                .repeatable()
                .builtin()
                .build(),
        )
        .expect("warn registration");

    // @deny
    registry
        .register(
            AttributeMetadata::new("deny")
                .targets(AttributeTarget::All)
                .args(ArgSpec::Variadic(ArgType::Ident))
                .category(AttributeCategory::Documentation)
                .doc("Turn warnings into errors")
                .repeatable()
                .builtin()
                .build(),
        )
        .expect("deny registration");
}

// =============================================================================
// SAFETY ATTRIBUTES
// =============================================================================

fn register_safety_attributes(registry: &mut AttributeRegistry) {
    // @verify
    registry
        .register(
            AttributeMetadata::new("verify")
                .targets(AttributeTarget::Function | AttributeTarget::Type)
                .args(ArgSpec::Required(ArgType::Ident))
                .category(AttributeCategory::Safety)
                .doc("Verification level: @verify(proof|static|runtime)")
                .typed_as::<VerifyAttr>()
                .builtin()
                .build(),
        )
        .expect("verify registration");

    // @trusted
    registry
        .register(
            AttributeMetadata::new("trusted")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::None)
                .category(AttributeCategory::Safety)
                .doc("Mark function as verified safe despite unsafe ops")
                .builtin()
                .build(),
        )
        .expect("trusted registration");

    // @unsafe_fn
    registry
        .register(
            AttributeMetadata::new("unsafe_fn")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::None)
                .category(AttributeCategory::Safety)
                .doc("Mark function as requiring unsafe context")
                .builtin()
                .build(),
        )
        .expect("unsafe_fn registration");

    // @must_use
    registry
        .register(
            AttributeMetadata::new("must_use")
                .targets(AttributeTarget::Function | AttributeTarget::Type | AttributeTarget::Param)
                .args(ArgSpec::Optional(ArgType::String))
                .category(AttributeCategory::Safety)
                .doc("Warn if return value is unused")
                .builtin()
                .build(),
        )
        .expect("must_use registration");

    // @unreachable
    registry
        .register(
            AttributeMetadata::new("unreachable")
                .targets(AttributeTarget::MatchArm | AttributeTarget::Expr)
                .args(ArgSpec::None)
                .category(AttributeCategory::Safety)
                .doc("Document that code path is unreachable")
                .builtin()
                .build(),
        )
        .expect("unreachable registration");

    // @pure
    registry
        .register(
            AttributeMetadata::new("pure")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::None)
                .category(AttributeCategory::Safety)
                .doc("Assert function has no side effects")
                .builtin()
                .build(),
        )
        .expect("pure registration");
}

// =============================================================================
// LAYOUT ATTRIBUTES
// =============================================================================

fn register_layout_attributes(registry: &mut AttributeRegistry) {
    // @repr
    registry
        .register(
            AttributeMetadata::new("repr")
                .targets(AttributeTarget::Type)
                .args(ArgSpec::Required(ArgType::Ident))
                .category(AttributeCategory::Layout)
                .doc("Control memory layout: @repr(C|packed|transparent)")
                .typed_as::<ReprAttr>()
                .builtin()
                .build(),
        )
        .expect("repr registration");

    // @align
    registry
        .register(
            AttributeMetadata::new("align")
                .targets(AttributeTarget::Type | AttributeTarget::Field)
                .args(ArgSpec::Required(ArgType::UInt))
                .category(AttributeCategory::Layout)
                .doc("Specify memory alignment: @align(16)")
                .typed_as::<AlignAttr>()
                .builtin()
                .build(),
        )
        .expect("align registration");

    // @packed
    registry
        .register(
            AttributeMetadata::new("packed")
                .targets(AttributeTarget::Type)
                .args(ArgSpec::None)
                .category(AttributeCategory::Layout)
                .doc("Remove padding between fields")
                .builtin()
                .build(),
        )
        .expect("packed registration");

    // @used
    registry
        .register(
            AttributeMetadata::new("used")
                .targets(AttributeTarget::Static | AttributeTarget::Const)
                .args(ArgSpec::None)
                .category(AttributeCategory::Layout)
                .doc("Prevent dead code elimination")
                .builtin()
                .build(),
        )
        .expect("used registration");
}

// =============================================================================
// BITFIELD ATTRIBUTES
// First-class bitfield support for hardware/network protocol programming.
// Bitfield system: packed integer fields with compile-time layout computation and accessor generation
// =============================================================================

fn register_bitfield_attributes(registry: &mut AttributeRegistry) {
    // @bitfield - marks a type as a bitfield container
    registry
        .register(
            AttributeMetadata::new("bitfield")
                .targets(AttributeTarget::Type)
                .args(ArgSpec::None)
                .category(AttributeCategory::Layout)
                .doc("Mark type as a bitfield container")
                .doc_extended(
                    r#"
## Usage
Marks a record type as a bitfield, enabling bit-level field packing.
Fields must use `@bits(N)` to specify their bit width.

## Example
```verum
@bitfield
type Flags is {
    @bits(1) carry: Bool,
    @bits(1) zero: Bool,
    @bits(6) reserved: UInt,
};
```

## Constraints
- All fields must have `@bits(N)` attribute
- Total bits are packed sequentially (or use `@offset(N)`)
- Cannot be combined with `@repr(C)` or `@packed`
"#,
                )
                .conflicts_with(["repr", "packed"])
                .typed_as::<BitfieldAttr>()
                .builtin()
                .build(),
        )
        .expect("bitfield registration");

    // @bits - specifies bit width for a field in a bitfield type
    registry
        .register(
            AttributeMetadata::new("bits")
                .targets(AttributeTarget::Field)
                .args(ArgSpec::Required(ArgType::UInt))
                .category(AttributeCategory::Layout)
                .doc("Specify bit width for bitfield member: @bits(N)")
                .doc_extended(
                    r#"
## Usage
Specifies the number of bits this field occupies in a `@bitfield` type.

## Example
```verum
@bitfield
type StatusRegister is {
    @bits(1) enabled: Bool,    // 1 bit
    @bits(4) mode: UInt,       // 4 bits
    @bits(3) priority: UInt,   // 3 bits
};
```

## Constraints
- N must be > 0
- N must not exceed storage type width (e.g., Bool max 1, U8 max 8)
- Only valid on fields within `@bitfield` types
"#,
                )
                .typed_as::<BitsAttr>()
                .builtin()
                .build(),
        )
        .expect("bits registration");

    // @offset - explicit bit offset for sparse bitfield layouts
    registry
        .register(
            AttributeMetadata::new("offset")
                .targets(AttributeTarget::Field)
                .args(ArgSpec::Required(ArgType::UInt))
                .category(AttributeCategory::Layout)
                .doc("Specify explicit bit offset in bitfield: @offset(N)")
                .doc_extended(
                    r#"
## Usage
Specifies an explicit bit offset for a field, allowing sparse layouts
or matching hardware register specifications.

## Example
```verum
@bitfield
type SparseRegister is {
    @bits(1) @offset(0) enable: Bool,
    @bits(3) @offset(4) mode: UInt,     // Gap at bits 1-3
    @bits(8) @offset(24) status: UInt,  // Gap at bits 7-23
};
```

## Constraints
- Offset N must not cause field to overlap with other fields
- Fields with explicit offsets are placed at that position
- Auto-positioned fields fill remaining gaps
"#,
                )
                .typed_as::<BitOffsetAttr>()
                .builtin()
                .build(),
        )
        .expect("offset registration");

    // @endian - byte order for multi-byte bitfields
    registry
        .register(
            AttributeMetadata::new("endian")
                .targets(AttributeTarget::Type | AttributeTarget::Field)
                .args(ArgSpec::Required(ArgType::Ident))
                .category(AttributeCategory::Layout)
                .doc("Specify byte order: @endian(big|little|native)")
                .doc_extended(
                    r#"
## Usage
Specifies byte order for multi-byte bitfield types or fields.

## Modes
- `big` - Big-endian (network byte order, RFC 791)
- `little` - Little-endian (x86, ARM default)
- `native` - Platform-dependent order

## Example
```verum
@bitfield
@endian(big)  // Network byte order
type Ipv4Header is {
    @bits(4) version: UInt,
    @bits(4) ihl: UInt,
    @bits(8) dscp_ecn: UInt,
    @bits(16) total_length: UInt,
};
```

## Default
Little-endian if not specified.
"#,
                )
                .typed_as::<EndianAttr>()
                .builtin()
                .build(),
        )
        .expect("endian registration");
}

// =============================================================================
// MODULE CONTROL ATTRIBUTES
// =============================================================================

fn register_module_attributes(registry: &mut AttributeRegistry) {
    // @profile
    registry
        .register(
            AttributeMetadata::new("profile")
                .targets(AttributeTarget::Module)
                .args(ArgSpec::Variadic(ArgType::Ident))
                .category(AttributeCategory::ModuleControl)
                .doc("Declare module profile: @profile(application|systems|research)")
                .typed_as::<ProfileAttr>()
                .builtin()
                .build(),
        )
        .expect("profile registration");

    // @feature
    registry
        .register(
            AttributeMetadata::new("feature")
                .targets(AttributeTarget::Module)
                .args(ArgSpec::Named(vec![NamedArgSpec::required(
                    "enable",
                    ArgType::Expr,
                )].into()))
                .category(AttributeCategory::ModuleControl)
                .doc("Enable language features: @feature(enable: [\"unsafe\"])")
                .typed_as::<FeatureAttr>()
                .builtin()
                .build(),
        )
        .expect("feature registration");

    // @no_implicit_prelude
    registry
        .register(
            AttributeMetadata::new("no_implicit_prelude")
                .targets(AttributeTarget::Module)
                .args(ArgSpec::None)
                .category(AttributeCategory::ModuleControl)
                .doc("Disable implicit prelude imports")
                .builtin()
                .build(),
        )
        .expect("no_implicit_prelude registration");
}

// =============================================================================
// LANGUAGE CORE ATTRIBUTES
// =============================================================================

fn register_language_core_attributes(registry: &mut AttributeRegistry) {
    // @derive
    registry
        .register(
            AttributeMetadata::new("derive")
                .targets(AttributeTarget::Type)
                .args(ArgSpec::Variadic(ArgType::Ident))
                .category(AttributeCategory::LanguageCore)
                .doc("Derive protocol implementations: @derive(Clone, Serialize)")
                .builtin()
                .build(),
        )
        .expect("derive registration");

    // @std
    registry
        .register(
            AttributeMetadata::new("std")
                .targets(AttributeTarget::Function | AttributeTarget::Type)
                .args(ArgSpec::Optional(ArgType::Ident))
                .category(AttributeCategory::LanguageCore)
                .doc("Automatic context provisioning")
                .typed_as::<StdAttr>()
                .builtin()
                .build(),
        )
        .expect("std registration");

    // @specialize
    registry
        .register(
            AttributeMetadata::new("specialize")
                .targets(AttributeTarget::Impl)
                .args(ArgSpec::Named(vec![
                    NamedArgSpec::optional("negative", ArgType::Bool),
                    NamedArgSpec::optional("rank", ArgType::Int),
                ].into()))
                .category(AttributeCategory::LanguageCore)
                .doc("Protocol implementation specialization")
                .typed_as::<SpecializeAttr>()
                .builtin()
                .build(),
        )
        .expect("specialize registration");

    // @marker
    registry
        .register(
            AttributeMetadata::new("marker")
                .targets(AttributeTarget::Protocol)
                .args(ArgSpec::None)
                .category(AttributeCategory::LanguageCore)
                .doc("Mark protocol as marker (no methods)")
                .builtin()
                .build(),
        )
        .expect("marker registration");

    // @auto
    registry
        .register(
            AttributeMetadata::new("auto")
                .targets(AttributeTarget::Protocol)
                .args(ArgSpec::None)
                .category(AttributeCategory::LanguageCore)
                .doc("Auto-implement protocol for qualifying types")
                .builtin()
                .build(),
        )
        .expect("auto registration");

    // @sealed
    registry
        .register(
            AttributeMetadata::new("sealed")
                .targets(AttributeTarget::Protocol | AttributeTarget::Type)
                .args(ArgSpec::None)
                .category(AttributeCategory::LanguageCore)
                .doc("Prevent external implementations/extensions")
                .builtin()
                .build(),
        )
        .expect("sealed registration");

    // @prototype - Relaxed type inference for rapid prototyping
    // @prototype mode: relaxed type checking for rapid prototyping, deferred refinement verification — @prototype Mode
    registry
        .register(
            AttributeMetadata::new("prototype")
                .targets(AttributeTarget::Function | AttributeTarget::Module)
                .args(ArgSpec::None)
                .category(AttributeCategory::LanguageCore)
                .doc("Enable relaxed type inference mode for rapid prototyping")
                .doc_extended(
                    r#"
## Usage
`@prototype` enables relaxed type inference that turns certain type errors into warnings,
allowing faster iteration during development. This is part of Verum's Safe Dynamic Typing system.

## Behavior Changes
- Unknown field access → WARNING + type inference (instead of error)
- Missing type annotations → WARNING + type inference (instead of error)
- Ambiguous types → WARNING + pick default (instead of error)
- Explicit type mismatches → ERROR (unchanged for safety)

## Example
```verum
@prototype
fn experiment() {
    let data = fetch("/api/users");  // Inferred as Data
    let name = data.user.name;       // WARNING: unchecked field access
    let count = data.users.len();    // WARNING: assuming Array type
    let x: Int = "hello";            // ERROR: explicit mismatch (unchanged)
}

@prototype
module experiments {
    // All functions in relaxed mode
}
```

## When to Use
- Rapid prototyping and experimentation
- Early development stages
- Exploring API responses
- Migrating code incrementally

## Production Recommendation
Remove @prototype before production deployment to get full type safety.
"#,
                )
                .builtin()
                .stability(Stability::Stable)
                .build(),
        )
        .expect("prototype registration");
}

// =============================================================================
// CONCURRENCY ATTRIBUTES
// Concurrency model: structured concurrency with nurseries, async/await, channels, Send/Sync protocol bounds
// =============================================================================

fn register_concurrency_attributes(registry: &mut AttributeRegistry) {
    // @lock_level
    registry
        .register(
            AttributeMetadata::new("lock_level")
                .targets(AttributeTarget::Type)
                .args(ArgSpec::Named(vec![NamedArgSpec::required(
                    "level",
                    ArgType::UInt,
                )].into()))
                .category(AttributeCategory::Concurrency)
                .doc("Declare lock ordering level for deadlock prevention")
                .typed_as::<LockLevelAttr>()
                .builtin()
                .build(),
        )
        .expect("lock_level registration");

    // @deadlock_detection
    registry
        .register(
            AttributeMetadata::new("deadlock_detection")
                .targets(AttributeTarget::Function | AttributeTarget::Module)
                .args(ArgSpec::Named(vec![
                    NamedArgSpec::optional("enabled", ArgType::Bool),
                    NamedArgSpec::optional("timeout", ArgType::Duration),
                ].into()))
                .category(AttributeCategory::Concurrency)
                .doc("Enable runtime deadlock detection")
                .builtin()
                .build(),
        )
        .expect("deadlock_detection registration");

    // @thread_local
    registry
        .register(
            AttributeMetadata::new("thread_local")
                .targets(AttributeTarget::Static)
                .args(ArgSpec::None)
                .category(AttributeCategory::Concurrency)
                .doc("Declare thread-local storage")
                .builtin()
                .build(),
        )
        .expect("thread_local registration");
}

// =============================================================================
// META-SYSTEM ATTRIBUTES
// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept
// =============================================================================

fn register_meta_system_attributes(registry: &mut AttributeRegistry) {
    // @tagged_literal
    registry
        .register(
            AttributeMetadata::new("tagged_literal")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::Required(ArgType::String))
                .category(AttributeCategory::MetaSystem)
                .doc("Register handler for tagged literals: @tagged_literal(\"json\")")
                .builtin()
                .build(),
        )
        .expect("tagged_literal registration");

    // @const_eval
    registry
        .register(
            AttributeMetadata::new("const_eval")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::None)
                .category(AttributeCategory::MetaSystem)
                .doc("Force compile-time evaluation")
                .builtin()
                .build(),
        )
        .expect("const_eval registration");

    // @differentiable
    registry
        .register(
            AttributeMetadata::new("differentiable")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::Named(vec![
                    NamedArgSpec::required("wrt", ArgType::String),
                    NamedArgSpec::optional("mode", ArgType::Ident),
                ].into()))
                .category(AttributeCategory::MetaSystem)
                .doc("Enable automatic differentiation")
                .stability(Stability::Experimental)
                .builtin()
                .build(),
        )
        .expect("differentiable registration");
}

// =============================================================================
// TESTING ATTRIBUTES
// =============================================================================

fn register_testing_attributes(registry: &mut AttributeRegistry) {
    // @test
    registry
        .register(
            AttributeMetadata::new("test")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::None)
                .category(AttributeCategory::Testing)
                .doc("Mark function as a test")
                .builtin()
                .build(),
        )
        .expect("test registration");

    // @bench
    registry
        .register(
            AttributeMetadata::new("bench")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::None)
                .category(AttributeCategory::Testing)
                .doc("Mark function as a benchmark")
                .builtin()
                .build(),
        )
        .expect("bench registration");

    // @ignore
    registry
        .register(
            AttributeMetadata::new("ignore")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::Optional(ArgType::String))
                .category(AttributeCategory::Testing)
                .doc("Skip test: @ignore or @ignore(\"reason\")")
                .builtin()
                .build(),
        )
        .expect("ignore registration");

    // @should_panic
    registry
        .register(
            AttributeMetadata::new("should_panic")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::Named(vec![NamedArgSpec::optional(
                    "expected",
                    ArgType::String,
                )].into()))
                .category(AttributeCategory::Testing)
                .doc("Test expects a panic")
                .builtin()
                .build(),
        )
        .expect("should_panic registration");
}

// =============================================================================
// FFI ATTRIBUTES
// =============================================================================

fn register_ffi_attributes(registry: &mut AttributeRegistry) {
    // @export
    registry
        .register(
            AttributeMetadata::new("export")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::Optional(ArgType::String))
                .category(AttributeCategory::FFI)
                .doc("Export function for FFI: @export(\"C\")")
                .builtin()
                .build(),
        )
        .expect("export registration");

    // @import
    registry
        .register(
            AttributeMetadata::new("import")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::Named(vec![
                    NamedArgSpec::required("from", ArgType::String),
                    NamedArgSpec::optional("name", ArgType::String),
                ].into()))
                .category(AttributeCategory::FFI)
                .doc("Import external function")
                .builtin()
                .build(),
        )
        .expect("import registration");

    // @link
    registry
        .register(
            AttributeMetadata::new("link")
                .targets(AttributeTarget::Module)
                .args(ArgSpec::Named(vec![
                    NamedArgSpec::required("name", ArgType::String),
                    NamedArgSpec::optional("kind", ArgType::Ident),
                ].into()))
                .category(AttributeCategory::FFI)
                .doc("Link external library")
                .builtin()
                .build(),
        )
        .expect("link registration");

    // @calling_convention
    registry
        .register(
            AttributeMetadata::new("calling_convention")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::Required(ArgType::Ident))
                .category(AttributeCategory::FFI)
                .doc("Specify calling convention: @calling_convention(C)")
                .builtin()
                .build(),
        )
        .expect("calling_convention registration");

    // @no_mangle
    registry
        .register(
            AttributeMetadata::new("no_mangle")
                .targets(AttributeTarget::Function | AttributeTarget::Static)
                .args(ArgSpec::None)
                .category(AttributeCategory::FFI)
                .doc("Preserve symbol name for linking")
                .builtin()
                .build(),
        )
        .expect("no_mangle registration");
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_attributes_registered() {
        let mut registry = AttributeRegistry::new();
        register_standard_attributes(&mut registry);

        // Check some key attributes exist
        assert!(registry.exists("inline"));
        assert!(registry.exists("cold"));
        assert!(registry.exists("hot"));
        assert!(registry.exists("derive"));
        assert!(registry.exists("serialize"));
        assert!(registry.exists("validate"));
        assert!(registry.exists("deprecated"));
        assert!(registry.exists("test"));
        assert!(registry.exists("export"));

        // Should have many attributes
        assert!(
            registry.len() >= 60,
            "Expected 60+ attributes, got {}",
            registry.len()
        );
    }

    #[test]
    fn test_categories() {
        let mut registry = AttributeRegistry::new();
        register_standard_attributes(&mut registry);

        // Check each category has attributes
        for category in [
            AttributeCategory::Optimization,
            AttributeCategory::Serialization,
            AttributeCategory::Validation,
            AttributeCategory::Documentation,
            AttributeCategory::Safety,
            AttributeCategory::Layout,
            AttributeCategory::ModuleControl,
            AttributeCategory::LanguageCore,
            AttributeCategory::Concurrency,
            AttributeCategory::MetaSystem,
            AttributeCategory::Testing,
            AttributeCategory::FFI,
        ] {
            let attrs = registry.by_category(category);
            assert!(
                !attrs.is_empty(),
                "Category {:?} has no attributes",
                category
            );
        }
    }

    #[test]
    fn test_conflicts() {
        let mut registry = AttributeRegistry::new();
        register_standard_attributes(&mut registry);

        let cold = registry.get("cold").unwrap();
        assert!(cold.conflicts_with.contains(&verum_common::Text::from("hot")));
    }
}
