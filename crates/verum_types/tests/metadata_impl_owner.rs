//! Imported impl targets retain the declaring owner across alias spellings.
use std::sync::Arc;
use verum_ast::ItemKind;
use verum_common::{List, Maybe, OrderedMap, ResourceDiscipline, Text};
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::*, infer::parse_descriptor_type_string};

fn wrapper(owner: &str) -> TypeDescriptor {
    TypeDescriptor {
        name: "Wrapper".into(),
        module_path: "bundle".into(),
        origin_module_path: Maybe::Some(owner.into()),
        generic_params: List::from_iter([GenericParam {
            name: "T".into(),
            bounds: List::new(),
            default: Maybe::None,
            type_bounds: List::new(),
            pid: Maybe::Some(0),
        }]),
        kind: TypeDescriptorKind::Record {
            fields: List::new(),
        },
        size: Maybe::None,
        alignment: Maybe::None,
        methods: List::new(),
        implements: List::new(),
        decl_span: Maybe::None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
    }
}
fn metadata(qualified: bool, reverse: bool) -> CoreMetadata {
    let mut metadata = CoreMetadata::default();
    let alpha = wrapper("alpha");
    let beta = wrapper("beta");
    metadata.types.insert("Wrapper".into(), alpha.clone());
    metadata
        .types
        .insert("export.Renamed".into(), alpha.clone());
    for (key, descriptor) in if reverse {
        [("beta.Wrapper", beta), ("alpha.Wrapper", alpha)]
    } else {
        [("alpha.Wrapper", alpha), ("beta.Wrapper", beta)]
    } {
        metadata.types.insert(key.into(), descriptor);
        metadata.type_declaration_order.push(key.into());
    }
    metadata.protocols.insert(
        "Deref".into(),
        ProtocolDescriptor {
            name: "Deref".into(),
            module_path: "traits".into(),
            origin_module_path: Maybe::None,
            generic_params: List::new(),
            super_protocols: List::new(),
            associated_types: List::new(),
            required_methods: List::new(),
            default_methods: List::new(),
            decl_span: Maybe::None,
        },
    );
    metadata.implementations.push(ImplementationDescriptor {
        protocol: "Deref".into(),
        target_type: if qualified {
            "alpha.Wrapper"
        } else {
            "Wrapper"
        }
        .into(),
        generic_params: List::new(),
        where_clause: List::new(),
        associated_types: OrderedMap::from_iter([("Target".into(), "__generic_0".into())]),
        methods: List::new(),
        protocol_args: List::new(),
    });
    serde_json::from_slice(&serde_json::to_vec(&metadata).unwrap()).unwrap()
}
fn checker(metadata: CoreMetadata, eager: bool) -> TypeChecker {
    if eager {
        TypeChecker::new_with_core_eager(Arc::new(metadata))
    } else {
        TypeChecker::new_with_core(Arc::new(metadata))
    }
}
fn target(checker: &TypeChecker, owner: &str) -> Option<verum_types::Type> {
    checker.protocol_checker.read().try_find_associated_type(
        &parse_descriptor_type_string(&format!("{owner}<Int>")),
        &Text::from("Target"),
    )
}
#[test]
fn legacy_target_uses_its_descriptor_owner_in_eager_and_lazy_loaders() {
    for eager in [false, true] {
        for lookup in ["Wrapper", "alpha.Wrapper", "export.Renamed"] {
            let mut checker = checker(metadata(false, false), eager);
            checker.ensure_stdlib_type_loaded(&lookup.into(), &mut Vec::new());
            assert_eq!(
                target(&checker, "alpha.Wrapper"),
                Some(verum_types::Type::Int),
                "{eager} {lookup}"
            );
            assert_eq!(
                target(&checker, "beta.Wrapper"),
                None,
                "foreign owner must not acquire the impl"
            );
        }
    }
}
#[test]
fn exact_impl_and_qualified_alias_lookup_preserve_foreign_negative() {
    for eager in [false, true] {
        for reverse in [false, true] {
            let mut checker = checker(metadata(true, reverse), eager);
            for lookup in if reverse {
                ["beta.Wrapper", "export.Renamed"]
            } else {
                ["export.Renamed", "beta.Wrapper"]
            } {
                checker.ensure_stdlib_type_loaded(&lookup.into(), &mut Vec::new());
            }
            assert_eq!(
                target(&checker, "alpha.Wrapper"),
                Some(verum_types::Type::Int),
                "eager={eager} reverse={reverse}"
            );
            assert_eq!(target(&checker, "beta.Wrapper"), None);
        }
    }
}
fn errors(owner: &str) -> List<Text> {
    let mut checker = checker(metadata(false, false), false);
    checker.set_current_module_path("consumer");
    let source = format!(
        "type Payload is {{ value: Int }}; implement Payload {{ fn inspect(&self)->Int {{self.value}} }} fn probe(value: {owner}<Payload>)->Int {{value.inspect()}}"
    );
    let module = Parser::new(&source).parse_module().unwrap();
    checker.register_stdlib_types_for_module(&module);
    for item in &module.items {
        match &item.kind {
            ItemKind::Type(decl) => checker.register_type_declaration(decl).unwrap(),
            ItemKind::Impl(decl) => checker.register_impl_block(decl).unwrap(),
            ItemKind::Function(decl) => checker.register_function_signature(decl).unwrap(),
            _ => (),
        }
    }
    let mut errors: List<Text> = module
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|e| Text::from(format!("{e:?}")))
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors
}
#[test]
fn source_method_dispatch_uses_the_selected_imported_impl() {
    assert!(
        errors("alpha.Wrapper").is_empty(),
        "{:?}",
        errors("alpha.Wrapper")
    );
    assert!(
        !errors("beta.Wrapper").is_empty(),
        "foreign wrapper has no Deref impl"
    );
}

#[test]
fn sibling_impls_keep_distinct_targets_in_both_load_orders() {
    for eager in [false, true] {
        for reverse in [false, true] {
            let mut metadata = metadata(true, reverse);
            let mut other = metadata.implementations[0].clone();
            other.target_type = "beta.Wrapper".into();
            other
                .associated_types
                .insert("Target".into(), "Bool".into());
            metadata.implementations.push(other);
            let mut checker = checker(metadata, eager);
            for key in if reverse {
                ["beta.Wrapper", "alpha.Wrapper"]
            } else {
                ["alpha.Wrapper", "beta.Wrapper"]
            } {
                checker.ensure_stdlib_type_loaded(&key.into(), &mut Vec::new());
            }
            assert_eq!(
                target(&checker, "alpha.Wrapper"),
                Some(verum_types::Type::Int)
            );
            assert_eq!(
                target(&checker, "beta.Wrapper"),
                Some(verum_types::Type::Bool)
            );
        }
    }
}

#[test]
fn existing_primitive_target_spelling_is_retained() {
    for eager in [false, true] {
        let mut metadata = metadata(false, false);
        let mut primitive = wrapper("core.primitives");
        primitive.name = "Int".into();
        primitive.generic_params = List::new();
        primitive.kind = TypeDescriptorKind::Opaque;
        metadata.types.insert("Int".into(), primitive);
        metadata.implementations[0].target_type = "Int".into();
        metadata.implementations[0]
            .associated_types
            .insert("Target".into(), "Bool".into());
        let mut checker = checker(metadata, eager);
        checker.ensure_stdlib_type_loaded(&"Int".into(), &mut Vec::new());
        assert_eq!(
            checker
                .protocol_checker
                .read()
                .try_find_associated_type(&verum_types::Type::Int, &"Target".into()),
            Some(verum_types::Type::Bool)
        );
        assert_eq!(target(&checker, "beta.Wrapper"), None);
    }
}
