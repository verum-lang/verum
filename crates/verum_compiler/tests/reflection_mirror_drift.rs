//! META-REFLECTION-DRIFT-1 — drift-pin tests for the THREE unsynchronized
//! mirrors of the reflection data model.
//!
//! The reflection data model exists in three places that are NOT generated
//! from each other:
//!
//!   1. `core/meta/reflection.vr`                      — stdlib SOURCE OF TRUTH
//!   2. `crates/verum_compiler/src/meta/reflection/`   — meta-runtime Rust mirror
//!   3. `crates/verum_compiler/src/derives/common.rs`  — derive-side Rust mirror
//!      (consumed by the 13 @derive codegens: Clone, Debug, Default, Display,
//!      Hash, PartialEq, Serialize, Deserialize, Error, Builder, CliCommand,
//!      ShellRender, TypedRow/MysqlTypedRow)
//!
//! @derive codegen consumes the RUST mirrors, not the stdlib — so silent
//! drift between the three produces wrong derives without any test failing.
//! This file pins all three surfaces:
//!
//!   * Test group 1 parses `core/meta/reflection.vr` TEXTUALLY and asserts
//!     the stdlib surface equals the const inventory below (authored
//!     2026-07-06). Any edit to reflection.vr shows up here first.
//!   * Test group 2 pins the meta-runtime mirror. Every mirrored enum is
//!     pinned by an exhaustive `match` with NO wildcard arm (adding or
//!     removing a variant breaks the build), plus a full-variant array
//!     (removing a variant breaks the build) and runtime name/discriminant
//!     asserts. Every mirrored struct is pinned by a struct literal that
//!     names EVERY field (a new field -> "missing field" compile error;
//!     a removed/renamed field -> "no field named" compile error) plus a
//!     `..`-less destructuring pattern (an added field also breaks it).
//!   * Test group 3 pins the derive-side mirror the same way.
//!
//! KNOWN DRIFT as of authoring (2026-07-06) — documented, NOT fixed here
//! (stdlib must not be modified by this test task). Strict-equality tests
//! for these are `#[ignore = "KNOWN-DRIFT: ..."]`; active tests pin the
//! drift EXACTLY so any further movement still fails:
//!
//!   * meta TypeKind has an 18th variant `Alias = 17` absent from stdlib.
//!   * meta record mirrors systematically DROP the stdlib `span` field
//!     (FieldInfo, VariantInfo, GenericParam, LifetimeParam, ParamInfo,
//!     FunctionInfo, ProtocolInfo, AssociatedTypeInfo, TraitBound).
//!   * meta FieldInfo has an extra internal `field_type: verum_ast::ty::Type`.
//!   * meta Visibility::keyword(In(path)) returns None; stdlib returns
//!     Some("public(in {path})") — behavioral drift.
//!   * derive-side FieldInfo/VariantInfo are a differently-shaped AST-level
//!     view (see DERIVE_* consts below).
//!   * stdlib `TypeInfo` is a CONTEXT (core/meta/contexts.vr), not a record
//!     in reflection.vr; both Rust `TypeInfo` structs are Rust-only shapes
//!     and are pinned here without a stdlib comparison.

use verum_ast::ty::Type;
use verum_ast::Span;
use verum_common::{List, Maybe, Text};
use verum_compiler::derives as derive_side;
use verum_compiler::meta::reflection as meta_refl;

// =========================================================================
// Section 0 — canonical stdlib inventory
// (extracted by hand from core/meta/reflection.vr at test-authoring time;
//  test group 1 re-derives these from the file and asserts equality)
// =========================================================================

const STDLIB_TYPE_KIND_VARIANTS: &[&str] = &[
    "Struct",
    "Enum",
    "Newtype",
    "Unit",
    "Protocol",
    "Tuple",
    "Array",
    "Slice",
    "Reference",
    "Pointer",
    "Function",
    "TypeParam",
    "Associated",
    "Primitive",
    "Never",
    "Infer",
    "Unknown",
];

const STDLIB_PRIMITIVE_TYPE_VARIANTS: &[&str] = &[
    "Bool", "Char", "Int8", "Int16", "Int32", "Int64", "Int", "UInt8", "UInt16", "UInt32",
    "UInt64", "ISize", "USize", "Float32", "Float64", "Text", "Unit",
];

const STDLIB_VISIBILITY_VARIANTS: &[&str] = &["Public", "Private", "Crate", "Super", "In"];

const STDLIB_SELF_KIND_VARIANTS: &[&str] = &["Value", "Ref", "RefMut"];

const STDLIB_METHOD_SOURCE_VARIANTS: &[&str] = &["Inherent", "Protocol", "Derived", "Generated"];

const STDLIB_VARIANT_KIND_VARIANTS: &[&str] = &["Unit", "Tuple", "Struct"];

const STDLIB_GENERIC_PARAM_KIND_VARIANTS: &[&str] = &["Type", "Lifetime", "Const"];

const STDLIB_FIELD_INFO_FIELDS: &[&str] = &[
    "name",
    "index",
    "type_name",
    "type_kind",
    "visibility",
    "is_mutable",
    "attributes",
    "doc",
    "span",
];

const STDLIB_VARIANT_INFO_FIELDS: &[&str] =
    &["name", "index", "kind", "fields", "attributes", "doc", "span"];

const STDLIB_GENERIC_PARAM_FIELDS: &[&str] =
    &["name", "index", "kind", "bounds", "default", "span"];

const STDLIB_PROTOCOL_INFO_FIELDS: &[&str] = &[
    "name",
    "path",
    "generics",
    "super_protocols",
    "associated_types",
    "required_methods",
    "provided_methods",
    "attributes",
    "doc",
    "span",
];

const STDLIB_ASSOCIATED_TYPE_INFO_FIELDS: &[&str] = &["name", "bounds", "default", "doc", "span"];

const STDLIB_FUNCTION_INFO_FIELDS: &[&str] = &[
    "name",
    "path",
    "generics",
    "params",
    "return_type",
    "return_kind",
    "is_async",
    "is_const",
    "is_unsafe",
    "is_pure",
    "is_meta",
    "contexts",
    "attributes",
    "doc",
    "visibility",
    "span",
];

const STDLIB_PARAM_INFO_FIELDS: &[&str] = &[
    "name",
    "index",
    "type_name",
    "type_kind",
    "is_mut",
    "is_self_param",
    "self_kind",
    "default",
    "attributes",
    "span",
];

const STDLIB_TRAIT_BOUND_FIELDS: &[&str] = &[
    "protocol_name",
    "protocol_path",
    "type_args",
    "associated_types",
    "is_negative",
    "is_maybe",
    "span",
];

const STDLIB_LIFETIME_PARAM_FIELDS: &[&str] =
    &["name", "index", "bounds", "is_static", "is_anonymous", "span"];

const STDLIB_OWNERSHIP_INFO_FIELDS: &[&str] = &[
    "is_copy",
    "is_clone",
    "is_send",
    "is_sync",
    "has_drop",
    "needs_drop",
    "is_unpin",
    "has_interior_mutability",
    "blocking_fields",
];

const STDLIB_FIELD_OFFSET_FIELDS: &[&str] = &["name", "offset", "size", "align", "padding_before"];

const STDLIB_METHOD_RESOLUTION_FIELDS: &[&str] =
    &["function", "source", "providing_protocol", "is_default_impl"];

// =========================================================================
// Current surface of the meta-runtime mirror
// (crates/verum_compiler/src/meta/reflection/). These consts MUST be kept
// literally in sync with the struct-literal compile pins in Section 2 —
// the literal is what the compiler enforces; the const is what the runtime
// asserts compare. If a compile pin below breaks, update BOTH.
// =========================================================================

const META_FIELD_INFO_FIELDS: &[&str] = &[
    "name",
    "index",
    "type_name",
    "type_kind",
    "field_type", // Rust-mirror-only internal field (KNOWN-DRIFT)
    "visibility",
    "is_mutable",
    "attributes",
    "doc",
    // NO span (KNOWN-DRIFT)
];

const META_VARIANT_INFO_FIELDS: &[&str] = &["name", "index", "kind", "fields", "attributes", "doc"];

const META_GENERIC_PARAM_FIELDS: &[&str] = &["name", "index", "kind", "bounds", "default"];

const META_PROTOCOL_INFO_FIELDS: &[&str] = &[
    "name",
    "path",
    "generics",
    "super_protocols",
    "associated_types",
    "required_methods",
    "provided_methods",
    "attributes",
    "doc",
];

const META_ASSOCIATED_TYPE_INFO_FIELDS: &[&str] = &["name", "bounds", "default", "doc"];

const META_FUNCTION_INFO_FIELDS: &[&str] = &[
    "name",
    "path",
    "generics",
    "params",
    "return_type",
    "return_kind",
    "is_async",
    "is_const",
    "is_unsafe",
    "is_pure",
    "is_meta",
    "contexts",
    "attributes",
    "doc",
    "visibility",
];

const META_PARAM_INFO_FIELDS: &[&str] = &[
    "name",
    "index",
    "type_name",
    "type_kind",
    "is_mut",
    "is_self_param",
    "self_kind",
    "default",
    "attributes",
];

const META_TRAIT_BOUND_FIELDS: &[&str] = &[
    "protocol_name",
    "protocol_path",
    "type_args",
    "associated_types",
    "is_negative",
    "is_maybe",
];

const META_LIFETIME_PARAM_FIELDS: &[&str] =
    &["name", "index", "bounds", "is_static", "is_anonymous"];

const META_OWNERSHIP_INFO_FIELDS: &[&str] = STDLIB_OWNERSHIP_INFO_FIELDS; // exact match today

const META_FIELD_OFFSET_FIELDS: &[&str] = STDLIB_FIELD_OFFSET_FIELDS; // exact match today

const META_METHOD_RESOLUTION_FIELDS: &[&str] = STDLIB_METHOD_RESOLUTION_FIELDS; // exact match today

// =========================================================================
// Current surface of the derive-side mirror
// (crates/verum_compiler/src/derives/common.rs — re-exported from
//  verum_compiler::derives). Same rule: keep in sync with the compile pins
// in Section 3.
// =========================================================================

const DERIVE_FIELD_INFO_FIELDS: &[&str] = &[
    "name",
    "ty",
    "index",
    "is_public",
    "has_default",
    "default_value",
    "attributes",
    "span",
];

const DERIVE_VARIANT_INFO_FIELDS: &[&str] =
    &["name", "fields", "is_unit", "is_tuple", "index", "span"];

const DERIVE_TYPE_INFO_FIELDS: &[&str] = &[
    "name",
    "generics",
    "fields",
    "variants",
    "is_enum",
    "is_newtype",
    "is_refinement",
    "span",
];

// Rust-only shape (no stdlib record counterpart; stdlib TypeInfo is a
// context in core/meta/contexts.vr).
const META_TYPE_INFO_FIELDS: &[&str] = &[
    "name",
    "kind",
    "generics",
    "doc",
    "attributes",
    "implements",
    "fields",
    "variants",
    "methods",
];

// =========================================================================
// Shared helpers
// =========================================================================

const SYNC_NOTE: &str = "The three reflection mirrors MUST be updated IN SYNC: \
core/meta/reflection.vr (stdlib source of truth), \
crates/verum_compiler/src/meta/reflection/ (meta-runtime Rust mirror), and \
crates/verum_compiler/src/derives/common.rs (derive-side Rust mirror, \
re-exported as verum_compiler::derives::{FieldInfo, VariantInfo, TypeInfo}). \
@derive codegen (the 13 derives) consumes the RUST mirrors, not the stdlib, \
so silent drift produces wrong derives without any other test failing. \
Also update the pinned consts + compile pins in \
crates/verum_compiler/tests/reflection_mirror_drift.rs and the exhaustive-match \
pin in core-tests/meta/reflection/drift_pin_test.vr.";

fn drift_msg(mirror: &str, item: &str, detail: &str) -> String {
    format!(
        "DRIFT DETECTED: {mirror} `{item}` drifted — {detail}. {SYNC_NOTE}"
    )
}

fn to_owned_vec(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// (missing-from-mirror, extra-in-mirror) relative to `reference`.
fn diff(reference: &[&str], mirror: &[&str]) -> (Vec<String>, Vec<String>) {
    let missing = reference
        .iter()
        .filter(|r| !mirror.contains(r))
        .map(|s| s.to_string())
        .collect();
    let extra = mirror
        .iter()
        .filter(|m| !reference.contains(m))
        .map(|s| s.to_string())
        .collect();
    (missing, extra)
}

fn strip<'a>(list: &[&'a str], remove: &[&str]) -> Vec<&'a str> {
    list.iter().copied().filter(|x| !remove.contains(x)).collect()
}

/// Assert `mirror` differs from `stdlib` by EXACTLY the given known drift
/// (no more, no less), and that the shared fields keep stdlib order.
fn assert_known_drift_exactly(
    mirror_label: &str,
    item: &str,
    stdlib: &[&str],
    mirror: &[&str],
    expected_missing: &[&str],
    expected_extra: &[&str],
) {
    assert_known_drift_exactly_impl(
        mirror_label,
        item,
        stdlib,
        mirror,
        expected_missing,
        expected_extra,
        true,
    );
}

#[allow(clippy::too_many_arguments)]
fn assert_known_drift_exactly_impl(
    mirror_label: &str,
    item: &str,
    stdlib: &[&str],
    mirror: &[&str],
    expected_missing: &[&str],
    expected_extra: &[&str],
    check_shared_order: bool,
) {
    let (missing, extra) = diff(stdlib, mirror);
    assert_eq!(
        missing,
        to_owned_vec(expected_missing),
        "{}",
        drift_msg(
            mirror_label,
            item,
            &format!(
                "fields/variants missing relative to core/meta/reflection.vr changed; \
                 pinned known-missing set is {expected_missing:?} but the mirror is now missing {missing:?}"
            ),
        )
    );
    assert_eq!(
        extra,
        to_owned_vec(expected_extra),
        "{}",
        drift_msg(
            mirror_label,
            item,
            &format!(
                "extra fields/variants relative to core/meta/reflection.vr changed; \
                 pinned known-extra set is {expected_extra:?} but the mirror now adds {extra:?}"
            ),
        )
    );
    if check_shared_order {
        assert_eq!(
            strip(mirror, expected_extra),
            strip(stdlib, expected_missing),
            "{}",
            drift_msg(
                mirror_label,
                item,
                "shared fields/variants no longer appear in core/meta/reflection.vr declaration order",
            )
        );
    }
}

// =========================================================================
// Section 1 — pin the stdlib surface by parsing core/meta/reflection.vr
// =========================================================================

fn read_reflection_vr() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../core/meta/reflection.vr");
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read stdlib reflection source {}: {e}. {SYNC_NOTE}",
            path.display()
        )
    })
}

fn strip_line_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Extract the variant names of a stdlib sum type:
/// `public type <name> is | A | B(T) | C;` (newlines/doc-comments allowed).
fn extract_vr_variants(src: &str, type_name: &str) -> Vec<String> {
    let marker = format!("public type {type_name} is");
    let start = src
        .find(&marker)
        .unwrap_or_else(|| {
            panic!(
                "core/meta/reflection.vr no longer declares `{marker}` — \
                 the stdlib reflection surface was renamed or removed. {SYNC_NOTE}"
            )
        })
        + marker.len();
    let mut body = String::new();
    for line in src[start..].lines() {
        let code = strip_line_comment(line);
        if let Some(semi) = code.find(';') {
            body.push_str(&code[..semi]);
            break;
        }
        body.push_str(code);
        body.push(' ');
    }
    body.split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let end = s
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(s.len());
            s[..end].to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Extract the field names of a stdlib record type:
/// `public type <name> is { a: T, b: U, ... };`
fn extract_vr_fields(src: &str, type_name: &str) -> Vec<String> {
    let marker = format!("public type {type_name} is {{");
    let start = src
        .find(&marker)
        .unwrap_or_else(|| {
            panic!(
                "core/meta/reflection.vr no longer declares record `{type_name}` \
                 (marker `{marker}` not found). {SYNC_NOTE}"
            )
        })
        + marker.len();
    let mut depth: i32 = 1;
    let mut fields = Vec::new();
    for line in src[start..].lines() {
        let code = strip_line_comment(line).trim();
        if depth == 1 {
            if let Some(colon) = code.find(':') {
                let name = code[..colon].trim();
                if !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_')
                    && !name.starts_with(|c: char| c.is_ascii_digit())
                {
                    fields.push(name.to_string());
                }
            }
        }
        for ch in code.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                return fields;
            }
        }
    }
    panic!("unterminated record body for `{type_name}` in core/meta/reflection.vr. {SYNC_NOTE}")
}

fn assert_stdlib_surface(kind: &str, name: &str, parsed: Vec<String>, pinned: &[&str]) {
    assert_eq!(
        parsed,
        to_owned_vec(pinned),
        "{}",
        drift_msg(
            "stdlib core/meta/reflection.vr",
            name,
            &format!(
                "its {kind} list no longer equals the inventory pinned in this test \
                 (pinned from reflection.vr on 2026-07-06). If the stdlib change is \
                 intentional, update BOTH Rust mirrors and then this inventory"
            ),
        )
    );
}

/// Test 1a: the stdlib enum surfaces are exactly the pinned inventory.
#[test]
fn stdlib_reflection_vr_enum_variants_pinned() {
    let src = read_reflection_vr();
    assert_stdlib_surface(
        "variant",
        "TypeKind",
        extract_vr_variants(&src, "TypeKind"),
        STDLIB_TYPE_KIND_VARIANTS,
    );
    assert_stdlib_surface(
        "variant",
        "PrimitiveType",
        extract_vr_variants(&src, "PrimitiveType"),
        STDLIB_PRIMITIVE_TYPE_VARIANTS,
    );
    assert_stdlib_surface(
        "variant",
        "Visibility",
        extract_vr_variants(&src, "Visibility"),
        STDLIB_VISIBILITY_VARIANTS,
    );
    assert_stdlib_surface(
        "variant",
        "SelfKind",
        extract_vr_variants(&src, "SelfKind"),
        STDLIB_SELF_KIND_VARIANTS,
    );
    assert_stdlib_surface(
        "variant",
        "MethodSource",
        extract_vr_variants(&src, "MethodSource"),
        STDLIB_METHOD_SOURCE_VARIANTS,
    );
    assert_stdlib_surface(
        "variant",
        "VariantKind",
        extract_vr_variants(&src, "VariantKind"),
        STDLIB_VARIANT_KIND_VARIANTS,
    );
    assert_stdlib_surface(
        "variant",
        "GenericParamKind",
        extract_vr_variants(&src, "GenericParamKind"),
        STDLIB_GENERIC_PARAM_KIND_VARIANTS,
    );
}

/// Test 1b: the stdlib record surfaces are exactly the pinned inventory.
#[test]
fn stdlib_reflection_vr_record_fields_pinned() {
    let src = read_reflection_vr();
    assert_stdlib_surface(
        "field",
        "FieldInfo",
        extract_vr_fields(&src, "FieldInfo"),
        STDLIB_FIELD_INFO_FIELDS,
    );
    assert_stdlib_surface(
        "field",
        "VariantInfo",
        extract_vr_fields(&src, "VariantInfo"),
        STDLIB_VARIANT_INFO_FIELDS,
    );
    assert_stdlib_surface(
        "field",
        "GenericParam",
        extract_vr_fields(&src, "GenericParam"),
        STDLIB_GENERIC_PARAM_FIELDS,
    );
    assert_stdlib_surface(
        "field",
        "ProtocolInfo",
        extract_vr_fields(&src, "ProtocolInfo"),
        STDLIB_PROTOCOL_INFO_FIELDS,
    );
    assert_stdlib_surface(
        "field",
        "AssociatedTypeInfo",
        extract_vr_fields(&src, "AssociatedTypeInfo"),
        STDLIB_ASSOCIATED_TYPE_INFO_FIELDS,
    );
    assert_stdlib_surface(
        "field",
        "FunctionInfo",
        extract_vr_fields(&src, "FunctionInfo"),
        STDLIB_FUNCTION_INFO_FIELDS,
    );
    assert_stdlib_surface(
        "field",
        "ParamInfo",
        extract_vr_fields(&src, "ParamInfo"),
        STDLIB_PARAM_INFO_FIELDS,
    );
    assert_stdlib_surface(
        "field",
        "TraitBound",
        extract_vr_fields(&src, "TraitBound"),
        STDLIB_TRAIT_BOUND_FIELDS,
    );
    assert_stdlib_surface(
        "field",
        "LifetimeParam",
        extract_vr_fields(&src, "LifetimeParam"),
        STDLIB_LIFETIME_PARAM_FIELDS,
    );
    assert_stdlib_surface(
        "field",
        "OwnershipInfo",
        extract_vr_fields(&src, "OwnershipInfo"),
        STDLIB_OWNERSHIP_INFO_FIELDS,
    );
    assert_stdlib_surface(
        "field",
        "FieldOffset",
        extract_vr_fields(&src, "FieldOffset"),
        STDLIB_FIELD_OFFSET_FIELDS,
    );
    assert_stdlib_surface(
        "field",
        "MethodResolution",
        extract_vr_fields(&src, "MethodResolution"),
        STDLIB_METHOD_RESOLUTION_FIELDS,
    );
}

// =========================================================================
// Section 2 — meta-runtime mirror pins
// (crates/verum_compiler/src/meta/reflection/)
// =========================================================================

// ---- enum compile pins: exhaustive match, NO wildcard arm. Adding a
// ---- variant to the Rust enum makes the match non-exhaustive (build
// ---- breaks); removing one makes the arm unresolvable (build breaks).

fn meta_type_kind_name(k: meta_refl::TypeKind) -> &'static str {
    use meta_refl::TypeKind as K;
    match k {
        K::Struct => "Struct",
        K::Enum => "Enum",
        K::Newtype => "Newtype",
        K::Unit => "Unit",
        K::Protocol => "Protocol",
        K::Tuple => "Tuple",
        K::Array => "Array",
        K::Slice => "Slice",
        K::Reference => "Reference",
        K::Pointer => "Pointer",
        K::Function => "Function",
        K::TypeParam => "TypeParam",
        K::Associated => "Associated",
        K::Primitive => "Primitive",
        K::Never => "Never",
        K::Infer => "Infer",
        K::Unknown => "Unknown",
        K::Alias => "Alias", // KNOWN-DRIFT: not in core/meta/reflection.vr
    }
}

const META_TYPE_KIND_ALL: [meta_refl::TypeKind; 18] = [
    meta_refl::TypeKind::Struct,
    meta_refl::TypeKind::Enum,
    meta_refl::TypeKind::Newtype,
    meta_refl::TypeKind::Unit,
    meta_refl::TypeKind::Protocol,
    meta_refl::TypeKind::Tuple,
    meta_refl::TypeKind::Array,
    meta_refl::TypeKind::Slice,
    meta_refl::TypeKind::Reference,
    meta_refl::TypeKind::Pointer,
    meta_refl::TypeKind::Function,
    meta_refl::TypeKind::TypeParam,
    meta_refl::TypeKind::Associated,
    meta_refl::TypeKind::Primitive,
    meta_refl::TypeKind::Never,
    meta_refl::TypeKind::Infer,
    meta_refl::TypeKind::Unknown,
    meta_refl::TypeKind::Alias,
];

fn meta_primitive_type_name(p: meta_refl::PrimitiveType) -> &'static str {
    use meta_refl::PrimitiveType as P;
    match p {
        P::Bool => "Bool",
        P::Char => "Char",
        P::Int8 => "Int8",
        P::Int16 => "Int16",
        P::Int32 => "Int32",
        P::Int64 => "Int64",
        P::Int => "Int",
        P::UInt8 => "UInt8",
        P::UInt16 => "UInt16",
        P::UInt32 => "UInt32",
        P::UInt64 => "UInt64",
        P::ISize => "ISize",
        P::USize => "USize",
        P::Float32 => "Float32",
        P::Float64 => "Float64",
        P::Text => "Text",
        P::Unit => "Unit",
    }
}

const META_PRIMITIVE_TYPE_ALL: [meta_refl::PrimitiveType; 17] = [
    meta_refl::PrimitiveType::Bool,
    meta_refl::PrimitiveType::Char,
    meta_refl::PrimitiveType::Int8,
    meta_refl::PrimitiveType::Int16,
    meta_refl::PrimitiveType::Int32,
    meta_refl::PrimitiveType::Int64,
    meta_refl::PrimitiveType::Int,
    meta_refl::PrimitiveType::UInt8,
    meta_refl::PrimitiveType::UInt16,
    meta_refl::PrimitiveType::UInt32,
    meta_refl::PrimitiveType::UInt64,
    meta_refl::PrimitiveType::ISize,
    meta_refl::PrimitiveType::USize,
    meta_refl::PrimitiveType::Float32,
    meta_refl::PrimitiveType::Float64,
    meta_refl::PrimitiveType::Text,
    meta_refl::PrimitiveType::Unit,
];

fn meta_visibility_name(v: &meta_refl::Visibility) -> &'static str {
    use meta_refl::Visibility as V;
    match v {
        V::Public => "Public",
        V::Private => "Private",
        V::Crate => "Crate",
        V::Super => "Super",
        V::In(_) => "In",
    }
}

fn meta_self_kind_name(s: meta_refl::SelfKind) -> &'static str {
    use meta_refl::SelfKind as S;
    match s {
        S::Value => "Value",
        S::Ref => "Ref",
        S::RefMut => "RefMut",
    }
}

const META_SELF_KIND_ALL: [meta_refl::SelfKind; 3] = [
    meta_refl::SelfKind::Value,
    meta_refl::SelfKind::Ref,
    meta_refl::SelfKind::RefMut,
];

fn meta_method_source_name(m: meta_refl::MethodSource) -> &'static str {
    use meta_refl::MethodSource as M;
    match m {
        M::Inherent => "Inherent",
        M::Protocol => "Protocol",
        M::Derived => "Derived",
        M::Generated => "Generated",
    }
}

const META_METHOD_SOURCE_ALL: [meta_refl::MethodSource; 4] = [
    meta_refl::MethodSource::Inherent,
    meta_refl::MethodSource::Protocol,
    meta_refl::MethodSource::Derived,
    meta_refl::MethodSource::Generated,
];

fn meta_variant_kind_name(v: meta_refl::VariantKind) -> &'static str {
    use meta_refl::VariantKind as V;
    match v {
        V::Unit => "Unit",
        V::Tuple => "Tuple",
        V::Struct => "Struct",
    }
}

const META_VARIANT_KIND_ALL: [meta_refl::VariantKind; 3] = [
    meta_refl::VariantKind::Unit,
    meta_refl::VariantKind::Tuple,
    meta_refl::VariantKind::Struct,
];

fn meta_generic_param_kind_name(g: meta_refl::GenericParamKind) -> &'static str {
    use meta_refl::GenericParamKind as G;
    match g {
        G::Type => "Type",
        G::Lifetime => "Lifetime",
        G::Const => "Const",
    }
}

const META_GENERIC_PARAM_KIND_ALL: [meta_refl::GenericParamKind; 3] = [
    meta_refl::GenericParamKind::Type,
    meta_refl::GenericParamKind::Lifetime,
    meta_refl::GenericParamKind::Const,
];

/// Meta-runtime enums that match stdlib exactly today. Order-sensitive:
/// the repr(u8) discriminants feed to_meta_value(), so order IS surface.
#[test]
fn meta_mirror_enums_match_stdlib() {
    let prim: Vec<&str> = META_PRIMITIVE_TYPE_ALL
        .iter()
        .map(|p| meta_primitive_type_name(*p))
        .collect();
    assert_eq!(
        prim,
        STDLIB_PRIMITIVE_TYPE_VARIANTS,
        "{}",
        drift_msg(
            "meta-runtime mirror (meta/reflection/primitive_type.rs)",
            "PrimitiveType",
            "variant list no longer equals core/meta/reflection.vr PrimitiveType",
        )
    );
    for (i, p) in META_PRIMITIVE_TYPE_ALL.iter().enumerate() {
        assert_eq!(
            *p as u8, i as u8,
            "{}",
            drift_msg(
                "meta-runtime mirror (meta/reflection/primitive_type.rs)",
                "PrimitiveType",
                "discriminant order no longer matches core/meta/reflection.vr declaration order",
            )
        );
    }

    let vis_all = [
        meta_refl::Visibility::Public,
        meta_refl::Visibility::Private,
        meta_refl::Visibility::Crate,
        meta_refl::Visibility::Super,
        meta_refl::Visibility::In(Text::from("path")),
    ];
    let vis: Vec<&str> = vis_all.iter().map(meta_visibility_name).collect();
    assert_eq!(
        vis,
        STDLIB_VISIBILITY_VARIANTS,
        "{}",
        drift_msg(
            "meta-runtime mirror (meta/reflection/type_kind.rs)",
            "Visibility",
            "variant list no longer equals core/meta/reflection.vr Visibility",
        )
    );

    let selfk: Vec<&str> = META_SELF_KIND_ALL
        .iter()
        .map(|s| meta_self_kind_name(*s))
        .collect();
    assert_eq!(
        selfk,
        STDLIB_SELF_KIND_VARIANTS,
        "{}",
        drift_msg(
            "meta-runtime mirror (meta/reflection/param_info.rs)",
            "SelfKind",
            "variant list no longer equals core/meta/reflection.vr SelfKind",
        )
    );

    let src: Vec<&str> = META_METHOD_SOURCE_ALL
        .iter()
        .map(|m| meta_method_source_name(*m))
        .collect();
    assert_eq!(
        src,
        STDLIB_METHOD_SOURCE_VARIANTS,
        "{}",
        drift_msg(
            "meta-runtime mirror (meta/reflection/method_resolution.rs)",
            "MethodSource",
            "variant list no longer equals core/meta/reflection.vr MethodSource",
        )
    );

    let vk: Vec<&str> = META_VARIANT_KIND_ALL
        .iter()
        .map(|v| meta_variant_kind_name(*v))
        .collect();
    assert_eq!(
        vk,
        STDLIB_VARIANT_KIND_VARIANTS,
        "{}",
        drift_msg(
            "meta-runtime mirror (meta/reflection/type_kind.rs)",
            "VariantKind",
            "variant list no longer equals core/meta/reflection.vr VariantKind",
        )
    );

    let gpk: Vec<&str> = META_GENERIC_PARAM_KIND_ALL
        .iter()
        .map(|g| meta_generic_param_kind_name(*g))
        .collect();
    assert_eq!(
        gpk,
        STDLIB_GENERIC_PARAM_KIND_VARIANTS,
        "{}",
        drift_msg(
            "meta-runtime mirror (meta/reflection/generic_param.rs)",
            "GenericParamKind",
            "variant list no longer equals core/meta/reflection.vr GenericParamKind",
        )
    );
}

/// ACTIVE pin of the KNOWN TypeKind drift: the meta-runtime mirror is
/// exactly stdlib's 17 variants PLUS a trailing `Alias`. Any further
/// movement (new variant, reorder, removal — including fixing the drift)
/// fails here, forcing this inventory and the ignored strict test below
/// to be revisited together.
#[test]
fn meta_mirror_type_kind_known_drift_pinned() {
    let names: Vec<&str> = META_TYPE_KIND_ALL
        .iter()
        .map(|k| meta_type_kind_name(*k))
        .collect();
    let mut expected: Vec<&str> = STDLIB_TYPE_KIND_VARIANTS.to_vec();
    expected.push("Alias"); // KNOWN-DRIFT(2026-07-06)
    assert_eq!(
        names,
        expected,
        "{}",
        drift_msg(
            "meta-runtime mirror (meta/reflection/type_kind.rs)",
            "TypeKind",
            "variant list moved beyond the pinned known drift \
             (= stdlib 17 variants + trailing `Alias`)",
        )
    );
    for (i, k) in META_TYPE_KIND_ALL.iter().enumerate() {
        assert_eq!(
            *k as u8, i as u8,
            "{}",
            drift_msg(
                "meta-runtime mirror (meta/reflection/type_kind.rs)",
                "TypeKind",
                "discriminant order changed",
            )
        );
    }
}

#[test]
#[ignore = "KNOWN-DRIFT(2026-07-06): meta-runtime TypeKind (crates/verum_compiler/src/meta/reflection/type_kind.rs) has an 18th variant `Alias = 17` that does NOT exist in core/meta/reflection.vr TypeKind (17 variants). Resolve by adding Alias to reflection.vr or removing it from the Rust mirror — and update derives/common.rs consumers IN SYNC."]
fn meta_mirror_type_kind_matches_stdlib_strict() {
    let names: Vec<&str> = META_TYPE_KIND_ALL
        .iter()
        .map(|k| meta_type_kind_name(*k))
        .collect();
    assert_eq!(
        names,
        STDLIB_TYPE_KIND_VARIANTS,
        "{}",
        drift_msg(
            "meta-runtime mirror (meta/reflection/type_kind.rs)",
            "TypeKind",
            "variant list does not equal core/meta/reflection.vr TypeKind",
        )
    );
}

// ---- struct compile pins. Each builder names EVERY field in a struct
// ---- literal: a field ADDED to the mirror -> E0063 "missing field";
// ---- a field REMOVED/RENAMED -> E0560 "no field named". The paired
// ---- `..`-less destructuring in the test doubles the pin.

fn build_meta_field_info() -> meta_refl::FieldInfo {
    let span = Span::default();
    meta_refl::FieldInfo {
        name: Text::from("x"),
        index: 0,
        type_name: Text::from("Int"),
        type_kind: meta_refl::TypeKind::Primitive,
        field_type: Type::int(span),
        visibility: meta_refl::Visibility::Public,
        is_mutable: false,
        attributes: List::new(),
        doc: Maybe::None,
    }
}

fn build_meta_variant_info() -> meta_refl::VariantInfo {
    meta_refl::VariantInfo {
        name: Text::from("Some"),
        index: 0,
        kind: meta_refl::VariantKind::Tuple,
        fields: List::new(),
        attributes: List::new(),
        doc: Maybe::None,
    }
}

fn build_meta_generic_param() -> meta_refl::GenericParam {
    meta_refl::GenericParam {
        name: Text::from("T"),
        index: 0,
        kind: meta_refl::GenericParamKind::Type,
        bounds: List::new(),
        default: Maybe::None,
    }
}

fn build_meta_lifetime_param() -> meta_refl::LifetimeParam {
    meta_refl::LifetimeParam {
        name: Text::from("a"),
        index: 0,
        bounds: List::new(),
        is_static: false,
        is_anonymous: false,
    }
}

fn build_meta_param_info() -> meta_refl::ParamInfo {
    meta_refl::ParamInfo {
        name: Text::from("p"),
        index: 0,
        type_name: Text::from("Int"),
        type_kind: meta_refl::TypeKind::Primitive,
        is_mut: false,
        is_self_param: false,
        self_kind: Maybe::None,
        default: Maybe::None,
        attributes: List::new(),
    }
}

fn build_meta_function_info() -> meta_refl::FunctionInfo {
    meta_refl::FunctionInfo {
        name: Text::from("f"),
        path: Text::from("m.f"),
        generics: List::new(),
        params: List::new(),
        return_type: Text::from("Int"),
        return_kind: meta_refl::TypeKind::Primitive,
        is_async: false,
        is_const: false,
        is_unsafe: false,
        is_pure: false,
        is_meta: false,
        contexts: List::new(),
        attributes: List::new(),
        doc: Maybe::None,
        visibility: meta_refl::Visibility::Public,
    }
}

fn build_meta_protocol_info() -> meta_refl::ProtocolInfo {
    meta_refl::ProtocolInfo {
        name: Text::from("P"),
        path: Text::from("m.P"),
        generics: List::new(),
        super_protocols: List::new(),
        associated_types: List::new(),
        required_methods: List::new(),
        provided_methods: List::new(),
        attributes: List::new(),
        doc: Maybe::None,
    }
}

fn build_meta_associated_type_info() -> meta_refl::AssociatedTypeInfo {
    meta_refl::AssociatedTypeInfo {
        name: Text::from("Item"),
        bounds: List::new(),
        default: Maybe::None,
        doc: Maybe::None,
    }
}

fn build_meta_trait_bound() -> meta_refl::TraitBound {
    meta_refl::TraitBound {
        protocol_name: Text::from("Clone"),
        protocol_path: Text::from("core.Clone"),
        type_args: List::new(),
        associated_types: List::new(),
        is_negative: false,
        is_maybe: false,
    }
}

fn build_meta_ownership_info() -> meta_refl::OwnershipInfo {
    meta_refl::OwnershipInfo {
        is_copy: false,
        is_clone: true,
        is_send: true,
        is_sync: true,
        has_drop: false,
        needs_drop: false,
        is_unpin: true,
        has_interior_mutability: false,
        blocking_fields: List::new(),
    }
}

fn build_meta_field_offset() -> meta_refl::FieldOffset {
    meta_refl::FieldOffset {
        name: Text::from("x"),
        offset: 0,
        size: 8,
        align: 8,
        padding_before: 0,
    }
}

fn build_meta_method_resolution() -> meta_refl::MethodResolution {
    meta_refl::MethodResolution {
        function: build_meta_function_info(),
        source: meta_refl::MethodSource::Inherent,
        providing_protocol: Maybe::None,
        is_default_impl: false,
    }
}

fn build_meta_type_info() -> meta_refl::TypeInfo {
    meta_refl::TypeInfo {
        name: Text::from("T"),
        kind: meta_refl::TypeKind::Struct,
        generics: List::new(),
        doc: Maybe::None,
        attributes: List::new(),
        implements: List::new(),
        fields: List::new(),
        variants: List::new(),
        methods: List::new(),
    }
}

/// ACTIVE pin of the meta-runtime record shapes, including the exact
/// KNOWN drift vs stdlib (span systematically dropped; FieldInfo carries
/// the extra internal `field_type`). The struct literals + `..`-less
/// destructurings are the compile-time half of the pin.
#[test]
fn meta_mirror_records_known_drift_pinned() {
    // FieldInfo — compile pin (destructure with no `..`)
    let meta_refl::FieldInfo {
        name,
        index,
        type_name,
        type_kind,
        field_type,
        visibility,
        is_mutable,
        attributes,
        doc,
    } = build_meta_field_info();
    let _ = (
        name, index, type_name, type_kind, field_type, visibility, is_mutable, attributes, doc,
    );
    assert_known_drift_exactly(
        "meta-runtime mirror (meta/reflection/field_info.rs)",
        "FieldInfo",
        STDLIB_FIELD_INFO_FIELDS,
        META_FIELD_INFO_FIELDS,
        &["span"],
        &["field_type"],
    );

    // VariantInfo
    let meta_refl::VariantInfo {
        name,
        index,
        kind,
        fields,
        attributes,
        doc,
    } = build_meta_variant_info();
    let _ = (name, index, kind, fields, attributes, doc);
    assert_known_drift_exactly(
        "meta-runtime mirror (meta/reflection/variant_info.rs)",
        "VariantInfo",
        STDLIB_VARIANT_INFO_FIELDS,
        META_VARIANT_INFO_FIELDS,
        &["span"],
        &[],
    );

    // GenericParam
    let meta_refl::GenericParam {
        name,
        index,
        kind,
        bounds,
        default,
    } = build_meta_generic_param();
    let _ = (name, index, kind, bounds, default);
    assert_known_drift_exactly(
        "meta-runtime mirror (meta/reflection/generic_param.rs)",
        "GenericParam",
        STDLIB_GENERIC_PARAM_FIELDS,
        META_GENERIC_PARAM_FIELDS,
        &["span"],
        &[],
    );

    // LifetimeParam
    let meta_refl::LifetimeParam {
        name,
        index,
        bounds,
        is_static,
        is_anonymous,
    } = build_meta_lifetime_param();
    let _ = (name, index, bounds, is_static, is_anonymous);
    assert_known_drift_exactly(
        "meta-runtime mirror (meta/reflection/generic_param.rs)",
        "LifetimeParam",
        STDLIB_LIFETIME_PARAM_FIELDS,
        META_LIFETIME_PARAM_FIELDS,
        &["span"],
        &[],
    );

    // ParamInfo
    let meta_refl::ParamInfo {
        name,
        index,
        type_name,
        type_kind,
        is_mut,
        is_self_param,
        self_kind,
        default,
        attributes,
    } = build_meta_param_info();
    let _ = (
        name, index, type_name, type_kind, is_mut, is_self_param, self_kind, default, attributes,
    );
    assert_known_drift_exactly(
        "meta-runtime mirror (meta/reflection/param_info.rs)",
        "ParamInfo",
        STDLIB_PARAM_INFO_FIELDS,
        META_PARAM_INFO_FIELDS,
        &["span"],
        &[],
    );

    // FunctionInfo
    let meta_refl::FunctionInfo {
        name,
        path,
        generics,
        params,
        return_type,
        return_kind,
        is_async,
        is_const,
        is_unsafe,
        is_pure,
        is_meta,
        contexts,
        attributes,
        doc,
        visibility,
    } = build_meta_function_info();
    let _ = (
        name,
        path,
        generics,
        params,
        return_type,
        return_kind,
        is_async,
        is_const,
        is_unsafe,
        is_pure,
        is_meta,
        contexts,
        attributes,
        doc,
        visibility,
    );
    assert_known_drift_exactly(
        "meta-runtime mirror (meta/reflection/function_info.rs)",
        "FunctionInfo",
        STDLIB_FUNCTION_INFO_FIELDS,
        META_FUNCTION_INFO_FIELDS,
        &["span"],
        &[],
    );

    // ProtocolInfo
    let meta_refl::ProtocolInfo {
        name,
        path,
        generics,
        super_protocols,
        associated_types,
        required_methods,
        provided_methods,
        attributes,
        doc,
    } = build_meta_protocol_info();
    let _ = (
        name,
        path,
        generics,
        super_protocols,
        associated_types,
        required_methods,
        provided_methods,
        attributes,
        doc,
    );
    assert_known_drift_exactly(
        "meta-runtime mirror (meta/reflection/protocol_info.rs)",
        "ProtocolInfo",
        STDLIB_PROTOCOL_INFO_FIELDS,
        META_PROTOCOL_INFO_FIELDS,
        &["span"],
        &[],
    );

    // AssociatedTypeInfo
    let meta_refl::AssociatedTypeInfo {
        name,
        bounds,
        default,
        doc,
    } = build_meta_associated_type_info();
    let _ = (name, bounds, default, doc);
    assert_known_drift_exactly(
        "meta-runtime mirror (meta/reflection/protocol_info.rs)",
        "AssociatedTypeInfo",
        STDLIB_ASSOCIATED_TYPE_INFO_FIELDS,
        META_ASSOCIATED_TYPE_INFO_FIELDS,
        &["span"],
        &[],
    );

    // TraitBound
    let meta_refl::TraitBound {
        protocol_name,
        protocol_path,
        type_args,
        associated_types,
        is_negative,
        is_maybe,
    } = build_meta_trait_bound();
    let _ = (
        protocol_name,
        protocol_path,
        type_args,
        associated_types,
        is_negative,
        is_maybe,
    );
    assert_known_drift_exactly(
        "meta-runtime mirror (meta/reflection/trait_bound.rs)",
        "TraitBound",
        STDLIB_TRAIT_BOUND_FIELDS,
        META_TRAIT_BOUND_FIELDS,
        &["span"],
        &[],
    );

    // OwnershipInfo — exact match with stdlib today
    let meta_refl::OwnershipInfo {
        is_copy,
        is_clone,
        is_send,
        is_sync,
        has_drop,
        needs_drop,
        is_unpin,
        has_interior_mutability,
        blocking_fields,
    } = build_meta_ownership_info();
    let _ = (
        is_copy,
        is_clone,
        is_send,
        is_sync,
        has_drop,
        needs_drop,
        is_unpin,
        has_interior_mutability,
        blocking_fields,
    );
    assert_known_drift_exactly(
        "meta-runtime mirror (meta/reflection/ownership_info.rs)",
        "OwnershipInfo",
        STDLIB_OWNERSHIP_INFO_FIELDS,
        META_OWNERSHIP_INFO_FIELDS,
        &[],
        &[],
    );

    // FieldOffset — exact match with stdlib today
    let meta_refl::FieldOffset {
        name,
        offset,
        size,
        align,
        padding_before,
    } = build_meta_field_offset();
    let _ = (name, offset, size, align, padding_before);
    assert_known_drift_exactly(
        "meta-runtime mirror (meta/reflection/field_info.rs)",
        "FieldOffset",
        STDLIB_FIELD_OFFSET_FIELDS,
        META_FIELD_OFFSET_FIELDS,
        &[],
        &[],
    );

    // MethodResolution — exact match with stdlib today
    let meta_refl::MethodResolution {
        function,
        source,
        providing_protocol,
        is_default_impl,
    } = build_meta_method_resolution();
    let _ = (function, source, providing_protocol, is_default_impl);
    assert_known_drift_exactly(
        "meta-runtime mirror (meta/reflection/method_resolution.rs)",
        "MethodResolution",
        STDLIB_METHOD_RESOLUTION_FIELDS,
        META_METHOD_RESOLUTION_FIELDS,
        &[],
        &[],
    );

    // TypeInfo — Rust-only shape (stdlib TypeInfo is a context, not a
    // record in reflection.vr); pin the shape itself.
    let meta_refl::TypeInfo {
        name,
        kind,
        generics,
        doc,
        attributes,
        implements,
        fields,
        variants,
        methods,
    } = build_meta_type_info();
    let _ = (
        name, kind, generics, doc, attributes, implements, fields, variants, methods,
    );
    assert_eq!(
        META_TYPE_INFO_FIELDS.len(),
        9,
        "{}",
        drift_msg(
            "meta-runtime mirror (meta/reflection/type_info.rs)",
            "TypeInfo",
            "pinned field inventory changed — update the destructuring pin and this const together",
        )
    );
}

#[test]
#[ignore = "KNOWN-DRIFT(2026-07-06): meta-runtime record mirrors (crates/verum_compiler/src/meta/reflection/) systematically DROP the stdlib `span` field of FieldInfo, VariantInfo, GenericParam, LifetimeParam, ParamInfo, FunctionInfo, ProtocolInfo, AssociatedTypeInfo, TraitBound; meta FieldInfo additionally carries a Rust-internal `field_type: verum_ast::ty::Type`; meta attributes are List<Text> vs stdlib List<Attribute>. Align the mirrors with core/meta/reflection.vr (or amend the stdlib) and update derives/common.rs consumers IN SYNC."]
fn meta_mirror_records_match_stdlib_strict() {
    let cases: &[(&str, &[&str], &[&str])] = &[
        ("FieldInfo", STDLIB_FIELD_INFO_FIELDS, META_FIELD_INFO_FIELDS),
        (
            "VariantInfo",
            STDLIB_VARIANT_INFO_FIELDS,
            META_VARIANT_INFO_FIELDS,
        ),
        (
            "GenericParam",
            STDLIB_GENERIC_PARAM_FIELDS,
            META_GENERIC_PARAM_FIELDS,
        ),
        (
            "LifetimeParam",
            STDLIB_LIFETIME_PARAM_FIELDS,
            META_LIFETIME_PARAM_FIELDS,
        ),
        ("ParamInfo", STDLIB_PARAM_INFO_FIELDS, META_PARAM_INFO_FIELDS),
        (
            "FunctionInfo",
            STDLIB_FUNCTION_INFO_FIELDS,
            META_FUNCTION_INFO_FIELDS,
        ),
        (
            "ProtocolInfo",
            STDLIB_PROTOCOL_INFO_FIELDS,
            META_PROTOCOL_INFO_FIELDS,
        ),
        (
            "AssociatedTypeInfo",
            STDLIB_ASSOCIATED_TYPE_INFO_FIELDS,
            META_ASSOCIATED_TYPE_INFO_FIELDS,
        ),
        (
            "TraitBound",
            STDLIB_TRAIT_BOUND_FIELDS,
            META_TRAIT_BOUND_FIELDS,
        ),
    ];
    for (name, stdlib, mirror) in cases {
        assert_eq!(
            mirror,
            stdlib,
            "{}",
            drift_msg(
                "meta-runtime mirror (crates/verum_compiler/src/meta/reflection/)",
                name,
                "field list does not equal core/meta/reflection.vr",
            )
        );
    }
}

/// Behavioral pin: stdlib Visibility.keyword(In(path)) returns
/// Some("public(in {path})"); the meta-runtime mirror returns None.
/// Active test pins the CURRENT (drifted) behavior.
#[test]
fn meta_mirror_visibility_keyword_in_behavior_pinned() {
    let v = meta_refl::Visibility::In(Text::from("crate.mod"));
    assert_eq!(
        v.keyword(),
        None,
        "{}",
        drift_msg(
            "meta-runtime mirror (meta/reflection/type_kind.rs)",
            "Visibility::keyword(In)",
            "behavior moved beyond the pinned known drift (mirror returns None; \
             core/meta/reflection.vr returns Some(\"public(in {path})\"))",
        )
    );
}

#[test]
#[ignore = "KNOWN-DRIFT(2026-07-06): meta-runtime Visibility::keyword(In(path)) returns None; core/meta/reflection.vr Visibility.keyword returns Some(\"public(in {path})\") for the In variant. Align the mirror with reflection.vr and update derives/common.rs consumers IN SYNC."]
fn meta_mirror_visibility_keyword_in_matches_stdlib_strict() {
    let v = meta_refl::Visibility::In(Text::from("crate.mod"));
    assert_eq!(
        v.keyword(),
        Some("public(in crate.mod)"),
        "{}",
        drift_msg(
            "meta-runtime mirror (meta/reflection/type_kind.rs)",
            "Visibility::keyword(In)",
            "does not implement the stdlib Some(\"public(in {path})\") behavior",
        )
    );
}

// =========================================================================
// Section 3 — derive-side mirror pins
// (crates/verum_compiler/src/derives/common.rs, re-exported from
//  verum_compiler::derives — the shapes the 13 @derive codegens consume)
// =========================================================================

fn build_derive_field_info() -> derive_side::FieldInfo {
    let span = Span::default();
    derive_side::FieldInfo {
        name: Text::from("x"),
        ty: Type::int(span),
        index: 0,
        is_public: true,
        has_default: false,
        default_value: Maybe::None,
        attributes: List::new(),
        span,
    }
}

fn build_derive_variant_info() -> derive_side::VariantInfo {
    let span = Span::default();
    derive_side::VariantInfo {
        name: Text::from("Some"),
        fields: List::new(),
        is_unit: true,
        is_tuple: false,
        index: 0,
        span,
    }
}

fn build_derive_type_info() -> derive_side::TypeInfo {
    let span = Span::default();
    derive_side::TypeInfo {
        name: Text::from("T"),
        generics: List::new(),
        fields: List::new(),
        variants: List::new(),
        is_enum: false,
        is_newtype: false,
        is_refinement: false,
        span,
    }
}

/// ACTIVE pin of the derive-side shapes (compile pins + exact known drift
/// vs the stdlib reflection records). The derive mirror is an AST-level
/// view: FieldInfo keeps the full `ty: verum_ast::ty::Type` instead of
/// (type_name, type_kind), collapses visibility to `is_public`, adds
/// @builder default tracking, and drops doc/is_mutable; VariantInfo
/// collapses `kind: VariantKind` into is_unit/is_tuple bools and drops
/// attributes/doc.
#[test]
fn derive_mirror_shapes_known_drift_pinned() {
    // FieldInfo — compile pin
    let derive_side::FieldInfo {
        name,
        ty,
        index,
        is_public,
        has_default,
        default_value,
        attributes,
        span,
    } = build_derive_field_info();
    let _ = (
        name,
        ty,
        index,
        is_public,
        has_default,
        default_value,
        attributes,
        span,
    );
    assert_known_drift_exactly(
        "derive-side mirror (derives/common.rs)",
        "FieldInfo",
        STDLIB_FIELD_INFO_FIELDS,
        DERIVE_FIELD_INFO_FIELDS,
        // missing vs stdlib:
        &["type_name", "type_kind", "visibility", "is_mutable", "doc"],
        // extra vs stdlib:
        &["ty", "is_public", "has_default", "default_value"],
    );

    // VariantInfo — compile pin
    let derive_side::VariantInfo {
        name,
        fields,
        is_unit,
        is_tuple,
        index,
        span,
    } = build_derive_variant_info();
    let _ = (name, fields, is_unit, is_tuple, index, span);
    // Shared-order check disabled: the derive mirror declares `fields`
    // before `index` (stdlib declares index first) — part of the pinned
    // KNOWN-DRIFT for this AST-level shape.
    assert_known_drift_exactly_impl(
        "derive-side mirror (derives/common.rs)",
        "VariantInfo",
        STDLIB_VARIANT_INFO_FIELDS,
        DERIVE_VARIANT_INFO_FIELDS,
        // missing vs stdlib:
        &["kind", "attributes", "doc"],
        // extra vs stdlib:
        &["is_unit", "is_tuple"],
        false,
    );

    // TypeInfo — Rust-only shape; pin the shape itself.
    let derive_side::TypeInfo {
        name,
        generics,
        fields,
        variants,
        is_enum,
        is_newtype,
        is_refinement,
        span,
    } = build_derive_type_info();
    let _ = (
        name,
        generics,
        fields,
        variants,
        is_enum,
        is_newtype,
        is_refinement,
        span,
    );
    assert_eq!(
        DERIVE_TYPE_INFO_FIELDS.len(),
        8,
        "{}",
        drift_msg(
            "derive-side mirror (derives/common.rs)",
            "TypeInfo",
            "pinned field inventory changed — update the destructuring pin and this const together",
        )
    );
}

#[test]
#[ignore = "KNOWN-DRIFT(2026-07-06): derive-side FieldInfo (crates/verum_compiler/src/derives/common.rs) is an AST-level shape {name, ty, index, is_public, has_default, default_value, attributes, span} — it lacks stdlib reflection.vr FieldInfo's {type_name, type_kind, visibility, is_mutable, doc} and adds {ty, is_public, has_default, default_value}. If reflection.vr FieldInfo changes, this mirror (consumed by all 13 @derive codegens) must be re-reviewed IN SYNC with meta/reflection/."]
fn derive_mirror_field_info_matches_stdlib_strict() {
    assert_eq!(
        DERIVE_FIELD_INFO_FIELDS,
        STDLIB_FIELD_INFO_FIELDS,
        "{}",
        drift_msg(
            "derive-side mirror (derives/common.rs)",
            "FieldInfo",
            "field list does not equal core/meta/reflection.vr FieldInfo",
        )
    );
}

#[test]
#[ignore = "KNOWN-DRIFT(2026-07-06): derive-side VariantInfo (crates/verum_compiler/src/derives/common.rs) is an AST-level shape {name, fields, is_unit, is_tuple, index, span} — it collapses stdlib reflection.vr VariantInfo's `kind: VariantKind` into is_unit/is_tuple bools and lacks {attributes, doc}; field order also differs. If reflection.vr VariantInfo changes, this mirror (consumed by all 13 @derive codegens) must be re-reviewed IN SYNC with meta/reflection/."]
fn derive_mirror_variant_info_matches_stdlib_strict() {
    assert_eq!(
        DERIVE_VARIANT_INFO_FIELDS,
        STDLIB_VARIANT_INFO_FIELDS,
        "{}",
        drift_msg(
            "derive-side mirror (derives/common.rs)",
            "VariantInfo",
            "field list does not equal core/meta/reflection.vr VariantInfo",
        )
    );
}
