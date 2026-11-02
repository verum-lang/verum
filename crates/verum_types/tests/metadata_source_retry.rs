//! Failed source elaboration must not certify a nominal placeholder on retry.
use std::sync::Arc;
use verum_ast::{FileId, ItemKind, TypeDeclBody, decl::VariantData};
use verum_common::{List, OrderedMap, Text};
use verum_fast_parser::FastParser;
use verum_types::{TypeChecker, core_metadata::*};

fn metadata(source: &str) -> CoreMetadata {
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(1))
        .unwrap();
    let mut metadata = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..Default::default()
    };
    let mut bindings = OrderedMap::new();
    bindings.insert("Missing".into(), None);
    let field = |field: &verum_ast::decl::RecordField| FieldDescriptor {
        name: field.name.name.clone(),
        ty: "__opaque_src".into(),
        is_public: true,
        declared_visibility: Some(verum_ast::Visibility::Public),
    };
    for item in &ast.items {
        let ItemKind::Type(declaration) = &item.kind else {
            panic!("type")
        };
        let name = declaration.name.name.clone();
        let key: Text = format!("owner.{name}").into();
        bindings.insert(name.clone(), Some(key.clone()));
        let kind = match &declaration.body {
            TypeDeclBody::Record(fields) => TypeDescriptorKind::Record {
                fields: fields.iter().map(field).collect(),
            },
            TypeDeclBody::Variant(variants) => TypeDescriptorKind::Variant {
                cases: variants
                    .iter()
                    .map(|variant| VariantCase {
                        name: variant.name.name.clone(),
                        payload: variant.data.as_ref().map(|payload| match payload {
                            VariantData::Record(fields) => {
                                VariantPayload::Record(fields.iter().map(field).collect())
                            }
                            VariantData::Tuple(types) => VariantPayload::Tuple(
                                types.iter().map(|_| "__opaque_src".into()).collect(),
                            ),
                        }),
                    })
                    .collect(),
            },
            TypeDeclBody::Protocol(_) => TypeDescriptorKind::Protocol {
                super_protocols: List::new(),
                associated_types: List::new(),
                required_methods: List::new(),
                default_methods: List::new(),
            },
            _ => panic!("fixture shape"),
        };
        metadata.types.insert(
            key.clone(),
            TypeDescriptor {
                source_declaration: verum_ast::source_signature::source_type_declaration(
                    declaration,
                )
                .unwrap(),
                name,
                module_path: "owner".into(),
                origin_module_path: None,
                generic_params: List::new(),
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
        metadata.type_declaration_order.push(key);
    }
    metadata
        .declaration_type_bindings
        .insert("owner".into(), bindings);
    metadata
}

fn checker(metadata: Arc<CoreMetadata>, eager: bool) -> TypeChecker {
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker
}

fn read_field(checker: &mut TypeChecker, name: &str, field: &str, attempt: usize) -> Vec<String> {
    check_source(checker, &format!("fn read{attempt}(x: owner.{name}) -> Type(1) {{ x.{field} }}"))
}

fn check_source(checker: &mut TypeChecker, source: &str) -> Vec<String> {
    let before = checker.diagnostics().len();
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(2))
        .unwrap();
    checker.register_stdlib_types_for_module(&ast);
    let mut errors: Vec<String> = checker
        .prepare_module_declarations(&ast)
        .iter()
        .map(ToString::to_string)
        .collect();
    for item in &ast.items {
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
            .skip(before)
            .filter(|diagnostic| diagnostic.is_error())
            .map(|diagnostic| format!("{diagnostic:?}")),
    );
    errors
}

#[test]
fn semantic_variant_constructors_use_checked_payloads() {
    let data = Arc::new(metadata("public type Choice is Ready { value: Type(1) } | Empty;"));
    for eager in [false, true] {
        let mut checker = checker(data.clone(), eager);
        let errors = check_source(&mut checker,
            "fn wrap(value: Type(1)) -> owner.Choice { owner.Choice.Ready { value: value } }");
        assert!(errors.is_empty(), "eager={eager}: {errors:?}, parent={:?}", checker.resolved_type("owner.Choice"));
        let errors = check_source(&mut checker,
            "fn wrong(value: Type(2)) -> owner.Choice { owner.Choice.Ready { value: value } }");
        assert!(!errors.is_empty(), "semantic variant accepted a lowered universe payload");
    }
}

#[test]
fn missing_protocol_method_types_are_not_dropped_or_replaced_with_unit() {
    for signature in ["fn bad(&self, value: Missing);", "fn bad(&self) -> Missing;"] {
        let data = Arc::new(metadata(&format!("public type Broken is protocol {{ fn level(&self) -> Type(1); {signature} }};")));
        for eager in [false, true] {
            let mut checker = checker(data.clone(), eager);
            checker.take_deferred_errors();
            for attempt in 0..2 {
                let errors = check_source(&mut checker, &format!("fn use{attempt}(x: &owner.Broken) -> Type(1) {{ x.level() }}"));
                assert!(errors.iter().any(|error| error.contains("Missing")), "{signature}, eager={eager}: {errors:?}");
                assert!(checker.resolved_type("owner.Broken").is_none());
            }
        }
    }
}

#[test]
fn failed_record_load_still_fails_after_diagnostics_are_drained() {
    let data = Arc::new(metadata(
        "public type Broken is { public value: Type(1), public bad: Missing };",
    ));
    for eager in [false, true] {
        let mut checker = checker(data.clone(), eager);
        let initial = checker.take_deferred_errors();
        if eager {
            assert!(!initial.is_empty());
        }
        for attempt in 0..3 {
            let errors = read_field(&mut checker, "Broken", "value", attempt);
            assert!(
                errors.iter().any(|error| error.contains("Missing")),
                "eager={eager}, attempt={attempt}: {errors:?}"
            );
            assert!(checker.resolved_type("owner.Broken").is_none());
            assert!(
                checker
                    .resolved_type("__struct_fields_owner.Broken")
                    .is_none()
            );
        }
    }
}

#[test]
fn nested_record_templates_do_not_survive_a_failed_recursive_group() {
    let data = Arc::new(metadata(
        "public type Outer is { public child: Child, public bad: Missing, public value: Type(1) }; public type Child is { public parent: &Outer, public value: Type(1) };",
    ));
    for eager in [false, true] {
        let mut checker = checker(data.clone(), eager);
        checker.take_deferred_errors();
        for (attempt, name) in ["Outer", "Child", "Outer", "Child"].into_iter().enumerate() {
            let errors = read_field(&mut checker, name, "value", attempt);
            assert!(
                errors.iter().any(|error| error.contains("Missing")),
                "eager={eager}, {name}: {errors:?}"
            );
            for key in [
                "owner.Outer",
                "owner.Child",
                "__struct_fields_owner.Outer",
                "__struct_fields_owner.Child",
                "__struct_fields_Child",
                "__type_params_owner.Child",
                "__type_var_order_owner.Child",
            ] {
                assert!(checker.resolved_type(key).is_none(), "stale {key}");
            }
        }
    }
}

#[test]
fn nested_variant_registrations_do_not_survive_a_failed_recursive_group() {
    let data = Arc::new(metadata(
        "public type Outer is { public child: Child, public bad: Missing, public value: Type(1) }; public type Child is Link(&Outer) | Marker(Type(1)) | Empty; public type Stable is StableValue(Type(1)) | StableEmpty;",
    ));
    assert!(data.types.get(&Text::from("owner.Child")).unwrap().source_declaration.is_some());
    for eager in [false, true] {
        let mut checker = checker(data.clone(), eager);
        checker.take_deferred_errors();
        let errors = check_source(&mut checker, "fn stable(value: &owner.Stable) {}");
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        let stable = checker.constructor_payloads_for_testing("Stable");
        assert!(!stable.is_empty());
        for attempt in 0..2 {
            let errors = read_field(&mut checker, "Outer", "value", attempt);
            assert!(errors.iter().any(|error| error.contains("Missing")), "eager={eager}: {errors:?}");
            for name in ["Child", "owner.Child"] {
                assert!(checker.constructor_payloads_for_testing(name).is_empty(), "stale {name}");
            }
            assert_eq!(checker.constructor_payloads_for_testing("Stable"), stable);
            let registry = checker.protocol_checker.read();
            for signature in ["Variant(Empty|Link|Marker)", "VariantRelaxed(Empty|Link|Marker)"] {
                assert!(registry.get_variant_type_name(&signature.into()).is_none(), "stale {signature}");
                assert!(registry.variant_type_name_candidates_for(&signature.into()).is_none(), "stale claimant {signature}");
            }
            drop(registry);
            let errors = check_source(&mut checker,
                &format!("fn stale{attempt}() {{ let value = Empty; }}"));
            assert!(!errors.is_empty(), "failed child's bare constructor remained callable");
        }
    }
}

#[test]
fn earlier_record_variant_payloads_are_rolled_back_when_a_later_case_fails() {
    let data = Arc::new(metadata(
        "public type Broken is Ready { value: Type(1) } | Invalid { bad: Missing };",
    ));
    for eager in [false, true] {
        let mut checker = checker(data.clone(), eager);
        checker.take_deferred_errors();
        for attempt in 0..2 {
            let errors = read_field(&mut checker, "Broken", "value", attempt);
            assert!(
                errors.iter().any(|error| error.contains("Missing")),
                "{errors:?}"
            );
            for key in [
                "owner.Broken",
                "__struct_fields_Ready",
                "__struct_fields_owner.Ready",
            ] {
                assert!(checker.resolved_type(key).is_none(), "stale {key}");
            }
        }
    }
}

#[test]
fn successful_recursive_record_loads_remain_reusable() {
    let data = Arc::new(metadata(
        "public type Node is { public next: &Node, public value: Type(1) };",
    ));
    for eager in [false, true] {
        let mut checker = checker(data.clone(), eager);
        assert!(checker.take_deferred_errors().is_empty());
        for attempt in 0..3 {
            let errors = read_field(&mut checker, "Node", "value", attempt);
            assert!(
                errors.is_empty(),
                "eager={eager}, attempt={attempt}: {errors:?}"
            );
            assert!(
                checker
                    .resolved_type("__struct_fields_owner.Node")
                    .is_some()
            );
        }
    }
}

#[test]
fn failed_recursive_group_removes_its_protocol_without_erasing_other_shared_definitions() {
    let data = Arc::new(metadata(
        "public type Outer is { public observer: Observer, public bad: Missing, public value: Type(1) }; public type Observer is protocol { fn parent(&self) -> &Outer; fn value(&self) -> Type(1); };",
    ));
    for eager in [false, true] {
        let mut checker = checker(data.clone(), eager);
        checker.take_deferred_errors();
        // The protocol registry really is shared across independent checkers.
        // Rollback must touch only the failed group's declared identities.
        let mut other = TypeChecker::new();
        other.protocol_checker = checker.protocol_checker.clone();
        let ast = FastParser::new()
            .parse_module_str(
                "type Untouched is protocol { fn unrelated(&self) -> Int; };",
                FileId::new(3),
            )
            .unwrap();
        other.check_item(&ast.items[0]).unwrap();
        for attempt in 0..2 {
            let errors = read_field(&mut checker, "Outer", "value", attempt);
            assert!(
                errors.iter().any(|error| error.contains("Missing")),
                "{errors:?}"
            );
            assert!(
                checker
                    .protocol_checker
                    .read()
                    .get_protocol(&Text::from("owner.Observer"))
                    .is_none()
            );
            assert!(
                other
                    .protocol_checker
                    .read()
                    .get_protocol(&Text::from("Untouched"))
                    .is_some()
            );
            assert!(checker.resolved_type("owner.Observer").is_none());
        }
    }
}
