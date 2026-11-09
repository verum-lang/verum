use std::sync::Arc;
use verum_ast::{ItemKind, TypeDeclBody, Visibility};
use verum_common::{List, OrderedMap};
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::*};

fn metadata(callable: bool, with_default: bool) -> CoreMetadata {
    let (preamble, parameter, bound, default) = if callable {
        ("", "F", "fn(Int)->Int", "fn(Int)->Int")
    } else {
        ("public type P is protocol {};", "T", "P", "Int")
    };
    let suffix = if with_default {
        format!(" = {default}")
    } else {
        String::new()
    };
    let source = format!(
        "{preamble} public type Required<{parameter}:{bound}{suffix}> is {{value:{parameter}}};"
    );
    let module = Parser::new(&source).parse_module().unwrap();
    let mut metadata = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..Default::default()
    };
    let mut bindings = OrderedMap::new();
    bindings.insert("P".into(), Some("provider.P".into()));
    bindings.insert("Required".into(), Some("provider.Required".into()));
    metadata
        .declaration_type_bindings
        .insert("provider".into(), bindings);
    for item in &module.items {
        let ItemKind::Type(declaration) = &item.kind else {
            panic!("type")
        };
        let (kind, generic_params) = match declaration.body {
            TypeDeclBody::Protocol(_) => (
                TypeDescriptorKind::Protocol {
                    super_protocols: List::new(),
                    associated_types: List::new(),
                    required_methods: List::new(),
                    default_methods: List::new(),
                },
                List::new(),
            ),
            TypeDeclBody::Record(_) => (
                TypeDescriptorKind::Record {
                    fields: List::from_iter([FieldDescriptor {
                        name: "value".into(),
                        ty: if callable { "F" } else { "T" }.into(),
                        is_public: true,
                        declared_visibility: Some(Visibility::Public),
                    }]),
                },
                List::from_iter([GenericParam {
                    name: if callable { "F" } else { "T" }.into(),
                    bounds: if callable {
                        List::new()
                    } else {
                        List::from_iter(["provider.P".into()])
                    },
                    type_bounds: if callable {
                        List::from_iter(["fn(Int) -> Int".into()])
                    } else {
                        List::new()
                    },
                    default: with_default.then(|| default.into()),
                    pid: Some(0),
                }]),
            ),
            _ => panic!("shape"),
        };
        let key = format!("provider.{}", declaration.name.name);
        metadata.type_declaration_order.push(key.clone().into());
        let source_declaration =
            verum_ast::source_signature::source_type_declaration(declaration).unwrap();
        assert!(
            source_declaration.is_some(),
            "real source producer must retain the fixture"
        );
        metadata.types.insert(
            key.into(),
            TypeDescriptor {
                source_declaration,
                name: declaration.name.name.clone(),
                module_path: "provider".into(),
                origin_module_path: None,
                generic_params,
                kind,
                size: None,
                alignment: None,
                methods: List::new(),
                implements: List::new(),
                decl_span: None,
                is_public: true,
                is_transparent_wrapper: false,
                resource_discipline: verum_common::ResourceDiscipline::Unrestricted,
            },
        );
    }
    metadata
}

fn check(metadata: &CoreMetadata, source: &str, eager: bool, wire: bool) -> Vec<String> {
    let metadata = if wire {
        decode_metadata(&encode_metadata(metadata).unwrap()).unwrap()
    } else {
        metadata.clone()
    };
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(Arc::new(metadata))
    } else {
        TypeChecker::new_with_core(Arc::new(metadata))
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    let module = Parser::new(source).parse_module().unwrap();
    checker.register_stdlib_types_for_module(&module);
    let mut errors: Vec<_> = checker
        .prepare_module_declarations(&module)
        .iter()
        .map(ToString::to_string)
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error.to_string());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(ToString::to_string),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|error| error.is_error())
            .map(|error| format!("{error:?}")),
    );
    errors
}

#[test]
fn retained_nominal_protocol_bounds_are_proved_in_their_declaring_scope() {
    for with_default in [false, true] {
        let metadata = metadata(false, with_default);
        for eager in [false, true] {
            for wire in [false, true] {
                let good = check(
                    &metadata,
                    "type Good<T:provider.P> is {value:provider.Required<T>};",
                    eager,
                    wire,
                );
                assert!(good.is_empty(), "eager={eager} wire={wire}: {good:#?}");
                let bad = check(
                    &metadata,
                    "type Bad is {value:provider.Required<Bool>};",
                    eager,
                    wire,
                );
                assert!(
                    bad.iter().any(|error| error.contains("no implementation")),
                    "eager={eager} wire={wire}: {bad:#?}"
                );
            }
        }
    }
}

#[test]
fn retained_nominal_callable_bounds_keep_actual_parameter_types() {
    let metadata = metadata(true, true);
    for eager in [false, true] {
        for wire in [false, true] {
            let good = check(
                &metadata,
                "type Good is {value:provider.Required<fn(Int)->Int>};",
                eager,
                wire,
            );
            assert!(good.is_empty(), "eager={eager} wire={wire}: {good:#?}");
            let bad = check(
                &metadata,
                "type Bad is {value:provider.Required<fn(Bool)->Int>};",
                eager,
                wire,
            );
            assert!(
                bad.iter().any(|error| error.contains("Type mismatch")),
                "eager={eager} wire={wire}: {bad:#?}"
            );
        }
    }
}

#[test]
fn bounded_metadata_cannot_drop_its_source_telescope() {
    for callable in [false, true] {
        let mut metadata = metadata(callable, true);
        metadata
            .types
            .get_mut(&"provider.Required".into())
            .unwrap()
            .source_declaration = None;
        for eager in [false, true] {
            let errors = check(
                &metadata,
                "type Bad is {value:provider.Required<Bool>};",
                eager,
                false,
            );
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("bounded metadata type")
                        && error.contains("source telescope")),
                "callable={callable} eager={eager}: {errors:#?}"
            );
        }
    }
}

#[test]
fn unbounded_legacy_nominal_metadata_keeps_its_existing_application_path() {
    let mut metadata = metadata(false, true);
    let declaration = metadata.types.get_mut(&"provider.Required".into()).unwrap();
    declaration.source_declaration = None;
    declaration.generic_params[0].bounds.clear();
    declaration.generic_params[0].default = None;
    for eager in [false, true] {
        for wire in [false, true] {
            let errors = check(
                &metadata,
                "type Good is {value:provider.Required<Bool>};",
                eager,
                wire,
            );
            assert!(errors.is_empty(), "eager={eager} wire={wire}: {errors:#?}");
        }
    }
}

#[test]
fn missing_nominal_bound_carrier_refuses_encoding_and_forged_wire() {
    for callable in [false, true] {
        let mut metadata = metadata(callable, true);
        metadata
            .types
            .get_mut(&"provider.Required".into())
            .unwrap()
            .source_declaration = None;
        let encode_error = encode_metadata(&metadata).unwrap_err();
        assert!(
            encode_error.contains("bounded metadata type")
                && encode_error.contains("source telescope"),
            "{encode_error}"
        );
        let mut forged = CORE_METADATA_MAGIC.to_vec();
        forged.extend(CORE_METADATA_VERSION.to_le_bytes());
        forged.extend(bincode::serialize(&metadata).unwrap());
        let decode_error = decode_metadata(&forged).unwrap_err();
        assert!(
            decode_error.contains("bounded metadata type")
                && decode_error.contains("source telescope"),
            "{decode_error}"
        );
    }
}
