//! T1717: exact metadata record shapes must not depend on the flat-slot winner.
//! The parsed-source and raw/JSON CoreMetadata paths are separate controls;
//! this does not exercise the archive producer or an ordinary product build.

use std::sync::Arc;
use verum_ast::{FileId, ItemKind};
use verum_common::{List, ResourceDiscipline, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    extract_exports_from_module, ModuleId, ModuleInfo, ModulePath, ModuleRegistry,
};
use verum_types::core_metadata::{
    CoreMetadata, FieldDescriptor, GenericParam, TypeDescriptor, TypeDescriptorKind,
};
use verum_types::{Type, TypeChecker};

const OWNERS: [&str; 2] = ["core.shape_alpha", "core.shape_beta"];
const MOUNTS: &str = "mount core.shape_alpha.{Packet as A}; mount core.shape_beta.{Packet as B};";

#[derive(Clone, Copy, Debug)]
enum Input {
    Source,
    Metadata { eager: bool, decoded: bool },
}

fn inputs() -> [Input; 5] {
    [
        Input::Source,
        Input::Metadata {
            eager: false,
            decoded: false,
        },
        Input::Metadata {
            eager: true,
            decoded: false,
        },
        Input::Metadata {
            eager: false,
            decoded: true,
        },
        Input::Metadata {
            eager: true,
            decoded: true,
        },
    ]
}

fn metadata(reverse: bool, generic: bool, bundled: bool) -> CoreMetadata {
    let mut metadata = CoreMetadata::default();
    let mut entries = [
        (OWNERS[0], "Int", "alpha_only"),
        (OWNERS[1], "Bool", "beta_only"),
    ];
    if reverse {
        entries.reverse();
    }
    for (owner, value_type, unique) in entries {
        let descriptor = TypeDescriptor {
            name: "Packet".into(),
            module_path: if bundled {
                "core.bundle".into()
            } else {
                owner.into()
            },
            origin_module_path: if bundled { Some(owner.into()) } else { None },
            generic_params: if generic {
                [GenericParam {
                    name: "T".into(),
                    bounds: List::new(),
                    default: None,
                    type_bounds: List::new(),
                    pid: Some(0),
                }]
                .into_iter()
                .collect()
            } else {
                List::new()
            },
            kind: TypeDescriptorKind::Record {
                fields: [
                    FieldDescriptor {
                        name: "body".into(),
                        ty: if generic {
                            "T".into()
                        } else {
                            value_type.into()
                        },
                        is_public: true,
                        declared_visibility: Some(verum_ast::Visibility::Public),
                    },
                    FieldDescriptor {
                        name: unique.into(),
                        ty: "Int".into(),
                        is_public: true,
                        declared_visibility: Some(verum_ast::Visibility::Public),
                    },
                ]
                .into_iter()
                .collect(),
            },
            size: Some(16),
            alignment: Some(8),
            methods: List::new(),
            implements: List::new(),
            decl_span: None,
            is_public: true,
            is_transparent_wrapper: false,
            resource_discipline: ResourceDiscipline::Unrestricted,
        };
        let key: Text = format!("{owner}.Packet").into();
        metadata.types.insert(key.clone(), descriptor.clone());
        metadata.type_declaration_order.push(key);
        metadata.types.insert("Packet".into(), descriptor);
    }
    metadata
}

fn check(
    body: &str,
    input: Input,
    reverse: bool,
    generic: bool,
    bundled: bool,
) -> (TypeChecker, List<Text>) {
    let mut checker = match input {
        Input::Source => TypeChecker::new(),
        Input::Metadata { eager, decoded } => {
            let metadata = metadata(reverse, generic, bundled);
            let metadata = if decoded {
                serde_json::from_slice(&serde_json::to_vec(&metadata).expect("metadata encode"))
                    .expect("metadata decode")
            } else {
                metadata
            };
            if eager {
                TypeChecker::new_with_core_eager(Arc::new(metadata))
            } else {
                TypeChecker::new_with_core(Arc::new(metadata))
            }
        }
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    let mut registry = ModuleRegistry::new();
    let mut owners = [
        (OWNERS[0], "Int", "alpha_only"),
        (OWNERS[1], "Bool", "beta_only"),
    ];
    if reverse {
        owners.reverse();
    }
    for (index, (owner, value_type, unique)) in owners.into_iter().enumerate() {
        let source: Text = if matches!(input, Input::Source) {
            if generic {
                format!("public type Packet<T> is {{ public body: T, public {unique}: Int }};")
                    .into()
            } else {
                format!(
                    "public type Packet is {{ public body: {value_type}, public {unique}: Int }};"
                )
                .into()
            }
        } else {
            "".into()
        };
        let file = FileId::new(index as u32);
        let id = ModuleId::new(index as u32);
        let ast = FastParser::new()
            .parse_module_str(source.as_str(), file)
            .expect("declaration grammar");
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), file, source);
        info.exports = extract_exports_from_module(&ast, id, &path).expect("exports");
        registry.register(info);
    }
    let source: Text = format!("{MOUNTS} {body}").into();
    let file = FileId::new(2);
    let ast = FastParser::new()
        .parse_module_str(source.as_str(), file)
        .expect("consumer grammar");
    let id = ModuleId::new(2);
    let path = ModulePath::from_str("consumer");
    let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), file, source);
    info.exports = extract_exports_from_module(&ast, id, &path).expect("consumer exports");
    registry.register(info);
    checker.set_module_registry_direct(registry.clone());
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "consumer", &registry) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Type(declaration) => checker.register_type_declaration(declaration),
            ItemKind::Function(declaration) => checker.register_function_signature(declaration),
            _ => continue,
        };
        if let Err(error) = result {
            errors.push(format!("{error:?}").into());
        }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    (checker, errors)
}

fn accepted(body: &str, generic: bool, bundled: bool) {
    let mut failures: List<Text> = List::new();
    for input in inputs() {
        for reverse in [false, true] {
            let (_, errors) = check(body, input, reverse, generic, bundled);
            if !errors.is_empty() {
                failures.push(format!("{input:?},reverse={reverse}: {errors:?}").into());
            }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
fn direct_public_reads_keep_both_owners() {
    accepted(
        "fn a(value: &A) -> Int { value.body } fn b(value: &B) -> Bool { value.body }",
        false,
        false,
    );
}
#[test]
fn public_construction_keeps_both_owners() {
    accepted(
        "fn a() -> A { A { body: 7, alpha_only: 1 } } fn b() -> B { B { body: true, beta_only: 2 } }",
        false,
        false,
    );
}
#[test]
fn public_patterns_keep_both_owners() {
    accepted(
        "fn a(value: A) -> Int { let A { body, .. } = value; body } fn b(value: B) -> Bool { let B { body, .. } = value; body }",
        false,
        false,
    );
}
#[test]
fn constructed_public_value_can_be_destructured() {
    accepted(
        "fn a() -> Int { let value: A = A { body: 7, alpha_only: 1 }; let A { body, .. } = value; body } fn b() -> Bool { let value: B = B { body: true, beta_only: 2 }; let B { body, .. } = value; body }",
        false,
        false,
    );
}
#[test]
fn optional_public_projection_keeps_both_owners() {
    accepted(
        "fn a(value: A) { let selected = value?.body; } fn b(value: B) { let selected = value?.body; }",
        false,
        false,
    );
}
#[test]
fn canonical_field_maps_exist_after_mount_even_when_flat_owner_matches() {
    for input in inputs()
        .into_iter()
        .filter(|input| matches!(input, Input::Metadata { .. }))
    {
        for reverse in [false, true] {
            let (checker, errors) = check("fn probe() {}", input, reverse, false, false);
            assert!(errors.is_empty(), "{errors:?}");
            for (owner, expected) in [(OWNERS[0], Type::int()), (OWNERS[1], Type::bool())] {
                let key: Text = format!("__struct_fields_{owner}.Packet").into();
                let shape = checker.lookup_type_for_testing(key.as_str());
                assert!(
                    matches!(&shape, Some(Type::Record(fields)) if fields.get("body") == Some(&expected)),
                    "{input:?},reverse={reverse},key={key}: {shape:?}"
                );
            }
        }
    }
}
#[test]
fn generic_public_reads_substitute_exact_parameters() {
    accepted(
        "fn a(value: &A<Int>) -> Int { value.body } fn b(value: &B<Bool>) -> Bool { value.body }",
        true,
        false,
    );
}
#[test]
fn generic_public_patterns_substitute_exact_parameters() {
    accepted(
        "fn a(value: A<Int>) -> Int { let A { body, .. } = value; body } fn b(value: B<Bool>) -> Bool { let B { body, .. } = value; body }",
        true,
        false,
    );
}
#[test]
fn generic_optional_public_projection_resolves_fields() {
    accepted(
        "type Maybe<T> is None | Some(T); fn a(value: A<Int>) { let selected: Maybe<Int> = value?.body; } fn b(value: B<Bool>) { let selected: Maybe<Bool> = value?.body; }",
        true,
        false,
    );
}
#[test]
fn generic_public_construction_and_pattern_agree() {
    accepted(
        "fn a() -> Int { let value: A<Int> = A { body: 7, alpha_only: 1 }; let A { body, .. } = value; body } fn b() -> Bool { let value: B<Bool> = B { body: true, beta_only: 2 }; let B { body, .. } = value; body }",
        true,
        false,
    );
}
#[test]
fn shared_archive_bundle_retains_distinct_origin_shapes() {
    accepted(
        "fn a(value: A) -> Int { let A { body, .. } = value; body } fn b(value: B) -> Bool { let B { body, .. } = value; body }",
        false,
        true,
    );
}
#[test]
fn sibling_only_field_is_never_borrowed() {
    for input in inputs() {
        for reverse in [false, true] {
            let (_, errors) = check(
                "fn probe(value: A) -> Int { value.beta_only }",
                input,
                reverse,
                false,
                false,
            );
            assert!(
                errors.iter().any(|error| error.contains("beta_only")),
                "{input:?},reverse={reverse}: {errors:?}"
            );
        }
    }
}
#[test]
fn absent_qualified_owner_cannot_borrow_a_public_shape() {
    for input in inputs() {
        for reverse in [false, true] {
            let (_, errors) = check(
                "fn probe(value: core.shape_missing.Packet) -> Int { let core.shape_missing.Packet { body, .. } = value; body }",
                input,
                reverse,
                false,
                false,
            );
            assert!(
                !errors.is_empty(),
                "{input:?},reverse={reverse}: missing owner accepted"
            );
        }
    }
}

#[test]
fn generic_field_and_constructor_types_remain_checked() {
    for input in inputs() {
        for reverse in [false, true] {
            for body in [
                "fn probe(value: A<Int>) -> Bool { value.body }",
                "fn probe(value: A<Int>) -> Bool { let A { body, .. } = value; body }",
                "fn probe() { let value: A<Int> = A { body: true, alpha_only: 1 }; }",
            ] {
                let (_, errors) = check(body, input, reverse, true, false);
                assert!(
                    errors.iter().any(|error| error.contains("Mismatch")),
                    "{input:?},reverse={reverse},body={body}: {errors:?}"
                );
            }
        }
    }
}

#[test]
fn generic_record_never_borrows_a_sibling_only_field() {
    for input in inputs() {
        for reverse in [false, true] {
            let (_, errors) = check(
                "fn probe(value: A<Int>) -> Int { value.beta_only }",
                input,
                reverse,
                true,
                false,
            );
            assert!(
                errors.iter().any(|error| error.contains("beta_only")),
                "{input:?},reverse={reverse}: {errors:?}"
            );
        }
    }
}

#[test]
fn nested_identity_aliases_keep_record_arguments() {
    accepted(
        "type Identity<T> is T; fn probe(value: Identity<Identity<A<Int>>>) -> Int { let A { body, .. } = value; body }",
        true, false,
    );
}
