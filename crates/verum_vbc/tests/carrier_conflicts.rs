//! Declaration conflicts must be rejected before archive imports enrich ABI state.
#![cfg(feature = "codegen")]

use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, CodegenError, CodegenErrorKind, SkipClass, VbcCodegen},
    module::VbcModule,
    types::TypeDescriptor,
};

fn compiled(owner: &str, declaration: &str) -> VbcModule {
    let source = format!("module {owner}; {declaration}");
    let ast = Parser::new(&source).parse_module().expect("valid grammar");
    VbcCodegen::with_config(CodegenConfig::new(owner))
        .compile_module(&ast)
        .expect("valid source descriptor")
}

fn carrier(module: &VbcModule) -> &TypeDescriptor {
    module
        .types
        .iter()
        .find(|descriptor| module.get_string(descriptor.name) == Some("Carrier"))
        .expect("exact Carrier declaration")
}

fn assert_conflict(error: CodegenError) {
    assert_eq!(error.skip_class(), SkipClass::Fatal);
    assert!(matches!(
        &error.kind,
        CodegenErrorKind::InvalidSourceSignature(_)
    ));
    let message = error.to_string();
    assert!(message.contains("owner.Carrier"), "{message}");
    assert!(message.contains("conflicting semantic source"), "{message}");
}

#[test]
fn conflicting_imports_fail_in_both_orders_without_layout_enrichment() {
    let first = compiled("owner", "public type Carrier is { value: Type(1) };");
    let second = compiled(
        "owner",
        "public type Carrier is { value: Type(2), extra: Int };",
    );
    for (accepted, conflicting) in [(&first, &second), (&second, &first)] {
        let mut consumer = VbcCodegen::new();
        consumer.import_archive_module_types(accepted);
        let original_layouts = consumer.export_type_layouts();
        consumer.import_archive_module_types(conflicting);
        assert_eq!(consumer.export_type_layouts(), original_layouts);
        assert_conflict(consumer.finalize_module_from_state().unwrap_err());
        consumer.import_archive_module_types(accepted);
        assert_conflict(consumer.finalize_module().unwrap_err());
    }
}

#[test]
fn duplicate_conflicts_inside_one_archive_fail_before_registration() {
    let mut source = compiled("owner", "public type Carrier is { value: Type(1) };");
    let other = compiled("owner", "public type Carrier is { value: Type(2) };");
    let mut duplicate = carrier(&source).clone();
    duplicate.id.0 += 100;
    duplicate.source_declaration = carrier(&other).source_declaration.clone();
    source.types.push(duplicate);
    let mut consumer = VbcCodegen::new();
    consumer.import_archive_module_types(&source);
    assert_eq!(consumer.type_layout_count(), 0);
    assert_conflict(consumer.finalize_module_from_state().unwrap_err());
}

#[test]
fn single_descriptor_import_also_rejects_a_conflicting_owner() {
    let first = compiled("owner", "public type Carrier is { value: Type(1) };");
    let second = compiled("owner", "public type Carrier is { value: Type(2) };");
    let mut consumer = VbcCodegen::new();
    for source in [&first, &second] {
        consumer.register_archive_type_qualified(
            carrier(source).clone(),
            "Carrier".into(),
            Some("owner"),
            Some(&source.strings),
        );
    }
    assert_conflict(consumer.finalize_module_from_state().unwrap_err());
}

#[test]
fn identical_clones_are_idempotent_and_different_owners_stay_independent() {
    let first = compiled("left", "public type Carrier is { value: Type(1) };");
    let second = compiled("right", "public type Carrier is { value: Type(2) };");
    let mut consumer = VbcCodegen::new();
    consumer.import_archive_module_types(&first);
    consumer.import_archive_module_types(&first.clone());
    consumer.import_archive_module_types(&second);
    let result = consumer.finalize_module_from_state().unwrap();
    let declarations: List<_> = result
        .types
        .iter()
        .filter_map(|descriptor| descriptor.source_declaration.as_ref())
        .collect();
    assert_eq!(declarations.len(), 2);
    assert!(declarations.contains(&carrier(&first).source_declaration.as_ref().unwrap()));
    assert!(declarations.contains(&carrier(&second).source_declaration.as_ref().unwrap()));
}

#[test]
fn protocol_imports_preserve_full_carriers_and_enrich_runtime_stubs() {
    for declaration in [
        "public type Carrier<A> is protocol { fn value(&self, input: A) -> Type(1); };",
        "public type Carrier<A = Type(1)> is protocol {};",
    ] {
        let source = compiled("owner", declaration);
        let mut runtime_stub = source.clone();
        for descriptor in &mut runtime_stub.types {
            descriptor.source_declaration = None;
            descriptor.type_params.clear();
            descriptor.variants.clear();
        }
        let mut consumer = VbcCodegen::new();
        consumer.import_archive_module_types(&runtime_stub);
        consumer.import_archive_module_types(&source);
        consumer.import_archive_module_types(&source);
        let result = consumer.finalize_module_from_state().unwrap();
        let actual = carrier(&result);
        let original = carrier(&source);
        assert_eq!(actual.source_declaration, original.source_declaration);
        assert_eq!(actual.type_params.len(), original.type_params.len());
        assert_eq!(actual.variants.len(), original.variants.len());
        for (actual, expected) in actual.type_params.iter().zip(&original.type_params) {
            assert_eq!(
                result.get_string(actual.name),
                source.get_string(expected.name)
            );
            assert_eq!(actual.id, expected.id);
        }
        for (actual, expected) in actual.variants.iter().zip(&original.variants) {
            assert_eq!(
                result.get_string(actual.name),
                source.get_string(expected.name)
            );
        }
    }
}

#[test]
fn conflicting_protocol_methods_are_not_hidden_by_runtime_stubs() {
    let first = compiled(
        "owner",
        "public type Carrier<A> is protocol { fn value(&self, input: A) -> Type(1); };",
    );
    let second = compiled(
        "owner",
        "public type Carrier<A> is protocol { fn value(&self, input: A) -> Type(2); };",
    );
    let mut consumer = VbcCodegen::new();
    consumer.import_archive_module_types(&first);
    consumer.import_archive_module_types(&second);
    assert_conflict(consumer.finalize_module_from_state().unwrap_err());
}

#[test]
fn runtime_only_copy_cannot_rewrite_declaration_owned_field_types() {
    use verum_vbc::types::{TypeId, TypeRef};

    let source = compiled("owner", "public type Carrier is { value: Type(1) };");
    let mut runtime_only = source.clone();
    for descriptor in &mut runtime_only.types {
        descriptor.source_declaration = None;
        if let Some(field) = descriptor.fields.first_mut() {
            field.type_ref = TypeRef::Concrete(TypeId::INT);
            field.declaration_type = Some(TypeRef::Concrete(TypeId::INT));
        }
    }
    let mut consumer = VbcCodegen::new();
    consumer.import_archive_module_types(&source);
    consumer.import_archive_module_types(&runtime_only);
    assert_conflict(consumer.finalize_module_from_state().unwrap_err());
}
