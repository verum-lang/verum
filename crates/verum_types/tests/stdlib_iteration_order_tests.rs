//! Locks the architectural invariant that `load_stdlib_from_metadata` walks
//! types in source declaration order (recorded via `type_declaration_order`)
//! and uses first-registered-wins for variant signature ownership.
//!
//! Pre-fix the loader sorted types alphabetically with a hardcoded priority
//! list (`["Result", "Maybe", "Ordering", "Bool"]`) to force well-known stdlib
//! types to register first. That violates the no-stdlib-knowledge-in-compiler
//! rule (see `crates/verum_types/src/CLAUDE.md`).
//!
//! These tests construct two competing variant types with the same `Ok|Err`
//! signature and verify that whichever appears first in
//! `type_declaration_order` owns the signature — independent of alphabetical
//! ordering, independent of any name-based priority.

use verum_common::{List, Maybe, Text};
use verum_types::core_metadata::{
    CoreMetadata, GenericParam, TypeDescriptor, TypeDescriptorKind, VariantCase, VariantPayload,
};
use verum_types::TypeChecker;

fn variant_type_with_ok_err(name: &str) -> TypeDescriptor {
    TypeDescriptor {
        name: name.into(),
        module_path: "test.module".into(),
        generic_params: List::from_iter([
            GenericParam { name: "T".into(), bounds: List::new(), default: Maybe::None },
            GenericParam { name: "E".into(), bounds: List::new(), default: Maybe::None },
        ]),
        kind: TypeDescriptorKind::Variant {
            cases: List::from_iter([
                VariantCase {
                    name: "Ok".into(),
                    payload: Maybe::Some(VariantPayload::Tuple(List::from_iter(["T".into()]))),
                },
                VariantCase {
                    name: "Err".into(),
                    payload: Maybe::Some(VariantPayload::Tuple(List::from_iter(["E".into()]))),
                },
            ]),
        },
        size: Maybe::None,
        alignment: Maybe::None,
        methods: List::new(),
        implements: List::new(),
    }
}

fn make_metadata(declaration_order: &[&str]) -> CoreMetadata {
    let mut metadata = CoreMetadata::default();
    for name in declaration_order {
        let text: Text = (*name).into();
        metadata.types.insert(text.clone(), variant_type_with_ok_err(name));
        metadata.type_declaration_order.push(text);
    }
    metadata
}

/// Variant signature for `Variant { Ok(_), Err(_) }` matches the format
/// produced by `variant_type_signature_relaxed`: payload-types ignored,
/// names sorted alphabetically.
fn ok_err_relaxed_signature() -> Text {
    "Variant(Err|Ok)".into()
}

#[test]
fn first_registered_in_declaration_order_owns_variant_signature() {
    // CanonicalResult registered first, AltResult second.
    // Pre-fix this would have been broken by alphabetical sort: AltResult (A)
    // would come before CanonicalResult (C) and steal the Ok|Err signature.
    let metadata = make_metadata(&["CanonicalResult", "AltResult"]);
    let checker = TypeChecker::new_with_core(metadata);

    let owner = checker
        .protocol_checker
        .read()
        .get_variant_type_name(&ok_err_relaxed_signature())
        .cloned();
    assert_eq!(
        owner,
        Some(Text::from("CanonicalResult")),
        "first-declared type must own the variant signature; got {:?}",
        owner
    );
}

#[test]
fn reversing_declaration_order_reverses_signature_ownership() {
    // Same two types, opposite declaration order. AltResult wins because it's
    // declared first. This proves there is NO hardcoded type-name priority —
    // ordering comes purely from `type_declaration_order`, not from a list of
    // "well-known" stdlib names baked into the compiler.
    let metadata = make_metadata(&["AltResult", "CanonicalResult"]);
    let checker = TypeChecker::new_with_core(metadata);

    let owner = checker
        .protocol_checker
        .read()
        .get_variant_type_name(&ok_err_relaxed_signature())
        .cloned();
    assert_eq!(
        owner,
        Some(Text::from("AltResult")),
        "first-declared type must own the variant signature regardless of name; got {:?}",
        owner
    );
}

#[test]
fn types_missing_from_declaration_order_still_register_alphabetically() {
    // Defensive: an entry that exists in `metadata.types` but was never pushed
    // to `type_declaration_order` (a bug in some hypothetical producer) must
    // still be registered, falling back to alphabetical iteration. This keeps
    // the loader resilient — types are never silently dropped.
    let mut metadata = CoreMetadata::default();
    metadata
        .types
        .insert(Text::from("Orphan"), variant_type_with_ok_err("Orphan"));
    // Note: NOT pushed to type_declaration_order.

    let checker = TypeChecker::new_with_core(metadata);

    let owner = checker
        .protocol_checker
        .read()
        .get_variant_type_name(&ok_err_relaxed_signature())
        .cloned();
    assert_eq!(
        owner,
        Some(Text::from("Orphan")),
        "orphan type must still be registered via alphabetical fallback; got {:?}",
        owner
    );
}
