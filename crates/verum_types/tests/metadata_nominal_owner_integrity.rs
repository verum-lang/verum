//! One declaration owner cannot acquire different source meanings through aliases.
use std::sync::Arc;
use verum_ast::{FileId, ItemKind};
use verum_common::{List, OrderedMap, Text};
use verum_fast_parser::FastParser;
use verum_types::{
    AnalysisControl, CoreMetadataSnapshot, Type, TypeChecker, TypeError, core_metadata::*,
};

fn descriptor(owner: &str, level: u32) -> TypeDescriptor {
    let source = format!("public type Carrier is {{ public value: Type({level}) }};");
    let ast = FastParser::new()
        .parse_module_str(&source, FileId::new(1))
        .unwrap();
    let ItemKind::Type(declaration) = &ast.items[0].kind else {
        panic!("type")
    };
    TypeDescriptor {
        source_declaration: verum_ast::source_signature::source_type_declaration(declaration)
            .unwrap(),
        name: "Carrier".into(),
        module_path: "bundle".into(),
        origin_module_path: Some(owner.into()),
        generic_params: List::new(),
        kind: TypeDescriptorKind::Record {
            fields: List::from_single(FieldDescriptor {
                name: "value".into(),
                ty: "__opaque_src".into(),
                is_public: true,
                declared_visibility: Some(verum_ast::Visibility::Public),
            }),
        },
        size: None,
        alignment: None,
        methods: List::new(),
        implements: List::new(),
        decl_span: None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: verum_common::ResourceDiscipline::Unrestricted,
    }
}

fn metadata(level: u32) -> CoreMetadata {
    let mut data = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..Default::default()
    };
    data.types
        .insert("owner.Carrier".into(), descriptor("owner", level));
    data.type_declaration_order.push("owner.Carrier".into());
    data.declaration_type_bindings
        .insert("owner".into(), OrderedMap::new());
    data
}

fn conflicting(reverse: bool, abi_only: bool) -> CoreMetadata {
    let mut data = metadata(if reverse { 2 } else { 1 });
    let mut other = descriptor("owner", if reverse { 1 } else { 2 });
    if abi_only {
        if reverse {
            data.types
                .get_mut(&Text::from("owner.Carrier"))
                .unwrap()
                .source_declaration = None;
        } else {
            other.source_declaration = None;
        }
    }
    // A lookup alias changes neither the origin nor the declaration identity.
    other.module_path = "relay".into();
    data.types.insert("Carrier".into(), other);
    data.type_declaration_order.insert(0, "Carrier".into());
    data
}

fn framed_without_global_validation(data: &CoreMetadata) -> Vec<u8> {
    let mut bytes = CORE_METADATA_MAGIC.to_vec();
    bytes.extend(CORE_METADATA_VERSION.to_le_bytes());
    // Both AST payloads remain individually well formed; only their shared
    // declaration authority conflicts. Exercise the reader independently.
    bytes.extend(bincode::serialize(data).unwrap());
    bytes
}

#[test]
fn nested_descriptor_names_bind_the_replayed_scope() {
    let mut data = metadata(1);
    let descriptor = data.types.get_mut(&Text::from("owner.Carrier")).unwrap();
    descriptor.name = "nested.Carrier".into();
    data.declaration_type_bindings.insert("owner.nested".into(), OrderedMap::new());
    let mut checker = TypeChecker::new_with_core(Arc::new(data.clone()));
    assert!(checker.take_deferred_errors().is_empty());
    data.declaration_type_bindings.get_mut(&Text::from("owner.nested")).unwrap()
        .insert("Private".into(), Some("changed.Private".into()));
    checker.set_core_metadata(Arc::new(data));
    assert!(checker.take_deferred_errors().iter().any(|error|
        error.to_string().contains("conflicting semantic type binding scopes for owner.nested.Carrier")));
}

fn assert_owner_error(errors: &[TypeError], fragment: &str) {
    assert!(
        errors.iter().any(|error| {
            error.is_soundness_critical()
                && matches!(error, TypeError::UniverseBinding { message, .. }
                if message.contains("owner.Carrier") && message.contains(fragment))
        }),
        "{errors:?}"
    );
}

fn field_level(checker: &TypeChecker, owner: &str) -> Type {
    let Type::Record(fields) = checker
        .resolved_type(&format!("__struct_fields_{owner}.Carrier"))
        .unwrap()
    else {
        panic!("record template")
    };
    fields[&Text::from("value")].clone()
}

#[test]
fn wire_rejects_conflicting_semantic_aliases_and_mixed_carriers() {
    for abi_only in [false, true] {
        for reverse in [false, true] {
            let data = conflicting(reverse, abi_only);
            let fragment = if abi_only {
                "semantic and ABI-only"
            } else {
                "conflicting semantic type declarations"
            };
            let write_error = encode_metadata(&data).unwrap_err();
            assert!(
                write_error.contains(fragment) && write_error.contains("owner.Carrier"),
                "{write_error}"
            );
            let read_error = decode_metadata(&framed_without_global_validation(&data)).unwrap_err();
            assert!(
                read_error.contains(fragment) && read_error.contains("owner.Carrier"),
                "{read_error}"
            );
        }
    }
}

#[test]
fn every_in_memory_loading_entry_rejects_conflicts_before_registering_types() {
    for abi_only in [false, true] {
        for reverse in [false, true] {
            for mode in 0..5 {
                let data = Arc::new(conflicting(reverse, abi_only));
                let mut checker = match mode {
                    0 => TypeChecker::new_with_core(data),
                    1 => TypeChecker::new_with_core_eager(data),
                    2 => {
                        let mut checker = TypeChecker::with_minimal_context();
                        checker.set_core_metadata(data);
                        checker
                    }
                    3 => {
                        let mut checker = TypeChecker::with_minimal_context();
                        checker.load_stdlib_from_metadata(&data);
                        checker
                    }
                    _ => {
                        let snapshot = Arc::new(
                            CoreMetadataSnapshot::prepare(data, &AnalysisControl::default())
                                .unwrap(),
                        );
                        TypeChecker::new_with_core_snapshot(snapshot)
                    }
                };
                assert!(checker.core_metadata().is_none(), "mode={mode}");
                assert!(
                    checker.resolved_type("owner.Carrier").is_none(),
                    "mode={mode}"
                );
                assert_owner_error(
                    &checker.take_deferred_errors(),
                    if abi_only {
                        "semantic and ABI-only"
                    } else {
                        "conflicting semantic type declarations"
                    },
                );
            }
        }
    }
}

#[test]
fn identical_aliases_and_foreign_homonyms_remain_valid_in_both_loading_modes() {
    let mut data = metadata(1);
    let alias = data.types.get(&Text::from("owner.Carrier")).unwrap().clone();
    data.types.insert("Carrier".into(), alias);
    data.types
        .insert("foreign.Carrier".into(), descriptor("foreign", 2));
    data.type_declaration_order.push("foreign.Carrier".into());
    data.declaration_type_bindings
        .insert("foreign".into(), OrderedMap::new());
    let bytes = encode_metadata(&data).unwrap();
    for eager in [false, true] {
        let data = Arc::new(decode_metadata(&bytes).unwrap());
        let mut checker = if eager {
            TypeChecker::new_with_core_eager(data)
        } else {
            TypeChecker::new_with_core(data)
        };
        checker.register_primitives();
        checker.set_current_module_path("consumer");
        let ast = FastParser::new().parse_module_str(
            "fn a(x: owner.Carrier) -> Type(1) { x.value } fn b(x: foreign.Carrier) -> Type(2) { x.value }",
            FileId::new(2),
        ).unwrap();
        assert!(checker.prepare_module_declarations(&ast).is_empty());
        for item in &ast.items {
            checker.check_item(item).unwrap();
        }
        assert!(checker.take_deferred_errors().is_empty());
        assert!(
            !checker
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.is_error())
        );
        assert_ne!(
            field_level(&checker, "owner"),
            field_level(&checker, "foreign")
        );
    }
}

#[test]
fn cached_declarations_retain_authority_across_intermediate_snapshots() {
    for use_eager_loader in [false, true] {
        let accepted = Arc::new(metadata(1));
        let mut checker = TypeChecker::new_with_core_eager(accepted.clone());
        assert!(checker.take_deferred_errors().is_empty());
        let original = field_level(&checker, "owner");
        let empty = CoreMetadata {
            version: CORE_METADATA_VERSION,
            ..Default::default()
        };
        checker.set_core_metadata(Arc::new(empty));
        assert!(checker.take_deferred_errors().is_empty());
        if use_eager_loader {
            checker.load_stdlib_from_metadata(&metadata(2));
        } else {
            checker.set_core_metadata(Arc::new(metadata(2)));
        }
        assert_owner_error(
            &checker.take_deferred_errors(),
            "conflicting semantic type declarations",
        );
        assert_eq!(field_level(&checker, "owner"), original);
        checker.set_core_metadata(accepted);
        assert!(
            checker.take_deferred_errors().is_empty(),
            "failed admission must not poison receipts"
        );
    }
}

#[test]
fn equal_ast_cannot_change_its_declaring_import_scope_after_admission() {
    let accepted = metadata(1);
    let mut checker = TypeChecker::new_with_core(Arc::new(accepted.clone()));
    assert!(checker.take_deferred_errors().is_empty());
    let mut changed = accepted.clone();
    changed
        .declaration_type_bindings
        .get_mut(&Text::from("owner"))
        .unwrap()
        .insert("Token".into(), Some("foreign.Token".into()));
    checker.set_core_metadata(Arc::new(changed));
    assert_owner_error(&checker.take_deferred_errors(), "binding scopes");
    let mut absent = accepted.clone();
    absent
        .declaration_type_bindings
        .remove(&Text::from("owner"));
    checker.set_core_metadata(Arc::new(absent));
    assert_owner_error(&checker.take_deferred_errors(), "binding scopes");
    checker.set_core_metadata(Arc::new(accepted));
    assert!(checker.take_deferred_errors().is_empty());
}
