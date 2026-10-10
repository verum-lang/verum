//! T1693: an existing binding does not become builtin because builtins are registered later.
use std::sync::Arc;
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Maybe, ResourceDiscipline, Text};
use verum_fast_parser::FastParser;
use verum_modules::ModuleRegistry;
use verum_types::{Type, TypeChecker, context::TypeScheme, core_metadata::*};

fn metadata(reverse: bool) -> CoreMetadata {
    let mut metadata = CoreMetadata::default();
    let mut owners = ["core.first", "core.second"];
    if reverse {
        owners.reverse();
    }
    for owner in owners {
        let descriptor = TypeDescriptor {
            name: "Parcel".into(),
            module_path: owner.into(),
            origin_module_path: None,
            generic_params: List::new(),
            kind: TypeDescriptorKind::Record {
                fields: List::from_iter([FieldDescriptor {
                    declared_visibility: None,
                    name: "value".into(),
                    ty: "Int".into(),
                    is_public: true,
                }]),
            },
            size: Some(8),
            alignment: Some(8),
            methods: List::new(),
            implements: List::new(),
            decl_span: None,
            is_public: true,
            is_transparent_wrapper: false,
            resource_discipline: ResourceDiscipline::Unrestricted,
        };
        let exact: Text = format!("{owner}.Parcel").into();
        metadata.types.insert(exact.clone(), descriptor.clone());
        metadata.type_declaration_order.push(exact);
        metadata.types.insert("Parcel".into(), descriptor);
        let function = FunctionDescriptor {
            name: "published_fn".into(),
            module_path: owner.into(),
            origin_module_path: None,
            generic_params: List::new(),
            params: List::new(),
            return_type: "Int".into(),
            contexts: List::new(),
            is_async: false,
            is_unsafe: false,
            intrinsic_id: None,
            parent_type: None,
            impl_generic_names: List::new(),
            is_const: false,
            decl_span: None,
            is_public: true,
            explicit_type_param_ids: None,
        };
        metadata
            .functions
            .insert(format!("{owner}.published_fn").into(), function.clone());
        metadata.functions.insert("published_fn".into(), function);
    }
    metadata
}

fn checker(eager: bool, reverse: bool) -> TypeChecker {
    let metadata = Arc::new(metadata(reverse));
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.set_current_module_path("consumer");
    checker
}

fn register(checker: &mut TypeChecker, full: bool) {
    if full {
        checker.register_builtins();
    } else {
        checker.register_primitives();
    }
}

fn missing_import(checker: &mut TypeChecker, name: &str) -> Maybe<Text> {
    let source = format!("mount core.not_declared.{{{name}}};");
    let ast = FastParser::new()
        .parse_module_str(&source, FileId::new(40))
        .expect("mount grammar");
    let ItemKind::Mount(import) = &ast.items[0].kind else {
        panic!("mount item")
    };
    checker
        .process_import(import, "consumer", &ModuleRegistry::new())
        .err()
        .map(|error| format!("{error:?}").into())
}

fn assert_missing_refused(
    checker: &mut TypeChecker,
    name: &str,
    label: &str,
    failures: &mut List<Text>,
) {
    let error = missing_import(checker, name);
    if !error.as_ref().is_some_and(|error| {
        error.contains("ImportModuleNotFound") || error.contains("ImportItemNotFound")
    }) {
        failures.push(format!("{label}: missing {name} import gave {error:?}").into());
    }
}

#[test]
fn unrelated_metadata_names_never_gain_builtin_import_provenance() {
    let mut failures = List::new();
    for eager in [false, true] {
        for reverse in [false, true] {
            for full in [false, true] {
                for name in ["Parcel", "published_fn"] {
                    let mut checker = checker(eager, reverse);
                    register(&mut checker, full);
                    assert_missing_refused(
                        &mut checker,
                        name,
                        &format!("eager={eager}, reverse={reverse}, full={full}"),
                        &mut failures,
                    );
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn repeated_builtin_registration_never_promotes_loaded_metadata_names() {
    let mut failures = List::new();
    for eager in [false, true] {
        for reverse in [false, true] {
            for full_first in [false, true] {
                for full_second in [false, true] {
                    for name in ["Parcel", "published_fn"] {
                        let mut checker = checker(eager, reverse);
                        register(&mut checker, full_first);
                        // Exercise names already installed before a second registration.
                        let usage = FastParser::new()
                            .parse_module_str("fn named(value: Parcel) {}", FileId::new(7))
                            .unwrap();
                        checker.register_stdlib_types_for_module(&usage);
                        register(&mut checker, full_second);
                        assert_missing_refused(
                            &mut checker,
                            name,
                            &format!(
                                "eager={eager}, reverse={reverse}, first={full_first}, second={full_second}"
                            ),
                            &mut failures,
                        );
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn source_declarations_before_a_builtin_pass_never_gain_builtin_provenance() {
    let mut failures = List::new();
    let ast = FastParser::new()
        .parse_module_str(
            "public type Parcel is { value: Int }; public fn published_fn() -> Int { 3 }",
            FileId::new(1),
        )
        .expect("source grammar");
    for full in [false, true] {
        for name in ["Parcel", "published_fn"] {
            let mut checker = TypeChecker::with_minimal_context();
            checker.set_current_module_path("consumer");
            checker.register_primitives();
            for item in &ast.items {
                match &item.kind {
                    ItemKind::Type(declaration) => {
                        checker.register_type_declaration(declaration).unwrap()
                    }
                    ItemKind::Function(function) => {
                        checker.register_function_signature(function).unwrap()
                    }
                    _ => unreachable!(),
                }
            }
            register(&mut checker, full);
            assert_missing_refused(
                &mut checker,
                name,
                &format!("source full={full}"),
                &mut failures,
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn actual_primitive_builtin_mount_compatibility_survives_repeated_registration() {
    for eager in [false, true] {
        for reverse in [false, true] {
            for full in [false, true] {
                let mut checker = checker(eager, reverse);
                register(&mut checker, full);
                register(&mut checker, full);
                for name in ["Bool", "Int64"] {
                    assert!(
                        missing_import(&mut checker, name).is_none(),
                        "eager={eager}, reverse={reverse}, full={full}: {name}"
                    );
                }
            }
        }
    }
}

#[test]
fn intrinsic_and_meta_builtin_provenance_survives_a_later_primitive_pass() {
    for eager in [false, true] {
        for reverse in [false, true] {
            let mut checker = checker(eager, reverse);
            checker.register_builtins();
            checker.register_primitives();
            for name in ["print", "type_name", "TokenStream"] {
                assert!(
                    missing_import(&mut checker, name).is_none(),
                    "eager={eager}, reverse={reverse}: {name}"
                );
            }
        }
    }
}

#[test]
fn actual_builtin_registration_preserves_existing_overwrite_behavior() {
    let mut checker = TypeChecker::with_minimal_context();
    checker.context_mut().define_type("Bool", Type::Int);
    checker
        .context_mut()
        .env
        .insert("print", TypeScheme::mono(Type::Int));
    checker.register_builtins();
    assert!(matches!(
        checker.lookup_type_for_testing("Bool"),
        Some(Type::Bool)
    ));
    let scheme = checker
        .context_mut()
        .env
        .lookup("print")
        .expect("builtin print");
    assert!(
        matches!(&scheme.ty, Type::Function { return_type, .. } if matches!(return_type.as_ref(), Type::Unit)),
        "{scheme:?}"
    );
    assert!(missing_import(&mut checker, "Bool").is_none());
    assert!(missing_import(&mut checker, "print").is_none());
}
