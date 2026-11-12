//! Source signatures retain actual language declarations without borrowing caller aliases.
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::{Type, TypeChecker};

fn check(checker: &mut TypeChecker, owner: &str, sources: &[(&str, &str)]) -> List<Text> {
    let mut registry = ModuleRegistry::new();
    for (index, (name, source)) in sources.iter().enumerate() {
        let ast = Parser::new(source).parse_module().expect("source grammar");
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(name);
        let exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        let mut module =
            ModuleInfo::new(id, path, ast, FileId::new(index as u32), (*source).into());
        module.exports = exports;
        registry.register(module);
    }
    let ast = registry
        .get_by_path(owner)
        .expect("selected owner")
        .ast
        .clone();
    checker.set_module_registry_direct(registry);
    checker.set_current_module_path(owner);
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&ast)
        .into_iter()
        .map(|error| format!("{error:?}").into())
        .collect();
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
    errors
}

fn checker() -> TypeChecker {
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    checker
}

fn declare_foreign_alias(checker: &mut TypeChecker, source: &str) {
    let ast = Parser::new(source).parse_module().expect("alias grammar");
    let ItemKind::Type(declaration) = &ast.items[0].kind else {
        panic!("type declaration")
    };
    checker.set_current_module_path("foreign");
    checker
        .register_type_declaration(declaration)
        .expect("valid foreign declaration");
}

#[test]
fn registered_language_aliases_survive_direct_and_imported_source_scopes() {
    for name in ["RawPtr", "Epoch", "u32"] {
        let provider = format!("public type Slot is {{ public pointer: {name} }};");
        let sources = [
            ("realm.storage", provider.as_str()),
            (
                "realm.client",
                "mount realm.storage.Slot; fn inspect(slot: &Slot) {}",
            ),
        ];
        for owner in ["realm.storage", "realm.client"] {
            let errors = check(&mut checker(), owner, &sources);
            assert!(errors.is_empty(), "{name} in {owner}: {errors:?}");
        }
    }
}

#[test]
fn a_scalar_shaped_foreign_alias_is_not_a_language_declaration() {
    for target in ["Int", "Bool", "RawPtr"] {
        let mut checker = checker();
        declare_foreign_alias(&mut checker, &format!("type ForeignValue is {target};"));
        checker.register_builtins();
        let errors = check(
            &mut checker,
            "realm.storage",
            &[(
                "realm.storage",
                "public type Slot is { pointer: ForeignValue };",
            )],
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("TypeNotFound") && error.contains("ForeignValue")),
            "unmounted alias of {target} acquired builtin authority: {errors:?}"
        );
    }
}

#[test]
fn source_declarations_can_lexically_shadow_language_aliases() {
    let mut checker = checker();
    let errors = check(
        &mut checker,
        "realm.storage",
        &[(
            "realm.storage",
            "public type RawPtr is {public value:Bool}; public type Slot is {public pointer:RawPtr}; fn inspect(slot:Slot)->Bool {slot.pointer.value}",
        )],
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn an_explicit_source_mount_selects_its_declared_alias_owner() {
    let mut checker = checker();
    let errors = check(
        &mut checker,
        "realm.storage",
        &[
            ("foreign", "public type RawPtr is {public value:Bool};"),
            (
                "realm.storage",
                "mount foreign.RawPtr; public type Slot is {public pointer:RawPtr}; fn inspect(slot:Slot)->Bool {slot.pointer.value}",
            ),
        ],
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn a_foreign_homonym_cannot_replace_a_registered_language_alias() {
    let mut checker = checker();
    declare_foreign_alias(&mut checker, "type RawPtr is Bool;");
    let errors = check(
        &mut checker,
        "realm.storage",
        &[(
            "realm.storage",
            "public type Slot is {public pointer:RawPtr};",
        )],
    );
    assert!(errors.is_empty(), "{errors:?}");
    let Some(Type::Record(fields)) = checker.resolved_type("__struct_fields_realm.storage.Slot")
    else {
        panic!("declared fields")
    };
    assert!(
        matches!(fields.get(&Text::from("pointer")), Some(Type::Pointer {mutable:true, inner}) if **inner == Type::Int),
        "{fields:?}"
    );
}

#[test]
fn registration_is_required_before_a_language_alias_is_available() {
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    let errors = check(
        &mut checker,
        "realm.storage",
        &[("realm.storage", "public type Slot is {pointer:RawPtr};")],
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("TypeNotFound") && error.contains("RawPtr")),
        "{errors:?}"
    );
}

// Synthetic consumer metadata retains parsed declaration authority. Archive
// production itself is covered by the compiler's source/archive execution gates.
fn add_archive_record(
    metadata: &mut verum_types::core_metadata::CoreMetadata,
    owner: &str,
    name: &str,
    field: &str,
    field_type: &str,
    is_public: bool,
) {
    use verum_types::core_metadata::{FieldDescriptor, TypeDescriptor, TypeDescriptorKind};
    let source = format!("public type {name} is {{ public {field}: {field_type} }};");
    let ast = Parser::new(&source)
        .parse_module()
        .expect("archive declaration grammar");
    let ItemKind::Type(declaration) = &ast.items[0].kind else {
        panic!("record declaration")
    };
    verum_ast::source_signature::validate_source_type_declaration(declaration)
        .expect("valid retained declaration");
    let key: Text = format!("{owner}.{name}").into();
    metadata.types.insert(
        key.clone(),
        TypeDescriptor {
            source_declaration: Some(declaration.clone()),
            name: name.into(),
            module_path: owner.into(),
            origin_module_path: None,
            generic_params: List::new(),
            kind: TypeDescriptorKind::Record {
                fields: List::from_iter([FieldDescriptor {
                    name: field.into(),
                    ty: field_type.into(),
                    is_public: true,
                    declared_visibility: Some(verum_ast::Visibility::Public),
                }]),
            },
            size: None,
            alignment: None,
            methods: List::new(),
            implements: List::new(),
            decl_span: None,
            is_public,
            is_transparent_wrapper: false,
            resource_discipline: verum_common::ResourceDiscipline::Unrestricted,
        },
    );
    metadata.type_declaration_order.push(key);
}

fn archive_checker(metadata: verum_types::core_metadata::CoreMetadata, eager: bool) -> TypeChecker {
    let metadata = std::sync::Arc::new(metadata);
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.register_builtins();
    checker
}

#[test]
fn an_explicit_archive_mount_precedes_an_ambient_language_alias() {
    for (name, mount) in [
        ("RawPtr", "mount archive.defining.RawPtr;"),
        ("RawPtr", "mount archive.defining.{RawPtr};"),
        ("Handle", "mount archive.defining.Handle as RawPtr;"),
        ("Handle", "mount archive.defining.{Handle as RawPtr};"),
        ("RawPtr", "mount archive.defining.*;"),
    ] {
        for eager in [false, true] {
            for owner in ["realm.storage", "realm.client"] {
                let mut metadata = verum_types::core_metadata::CoreMetadata::default();
                add_archive_record(
                    &mut metadata,
                    "archive.defining",
                    name,
                    "value",
                    "Bool",
                    true,
                );
                let provider = format!(
                    "{mount} public type Slot is {{public pointer:RawPtr}}; fn inspect(slot:Slot)->Bool {{slot.pointer.value}}"
                );
                let errors = check(
                    &mut archive_checker(metadata, eager),
                    owner,
                    &[
                        ("realm.storage", provider.as_str()),
                        (
                            "realm.client",
                            "mount realm.storage.Slot; fn inspect(slot:Slot)->Bool {slot.pointer.value}",
                        ),
                    ],
                );
                assert!(
                    errors.is_empty(),
                    "mount={mount}, eager={eager}, owner={owner}: {errors:?}"
                );
            }
        }
    }
}

#[test]
fn an_invalid_explicit_archive_mount_cannot_fall_back_to_a_builtin() {
    for (mount, is_public, expected) in [
        ("mount archive.defining.RawPtr;", false, "VisibilityError"),
        ("mount archive.defining.{RawPtr};", false, "VisibilityError"),
        ("mount archive.defining.Missing as RawPtr;", true, "Missing"),
        (
            "mount archive.defining.{Missing as RawPtr};",
            true,
            "Missing",
        ),
    ] {
        for eager in [false, true] {
            let mut metadata = verum_types::core_metadata::CoreMetadata::default();
            add_archive_record(
                &mut metadata,
                "archive.defining",
                "RawPtr",
                "value",
                "Bool",
                is_public,
            );
            let provider = format!("{mount} public type Slot is {{public pointer:RawPtr}};");
            let errors = check(
                &mut archive_checker(metadata, eager),
                "realm.client",
                &[
                    ("realm.storage", provider.as_str()),
                    (
                        "realm.client",
                        "mount realm.storage.Slot; fn inspect(slot:&Slot) {}",
                    ),
                ],
            );
            assert!(
                errors.iter().any(|error| error.contains(expected)),
                "mount={mount}, eager={eager}: {errors:?}"
            );
        }
    }
}

#[test]
fn a_retained_declaration_binding_precedes_a_builtin() {
    for eager in [false, true] {
        let mut metadata = verum_types::core_metadata::CoreMetadata::default();
        add_archive_record(
            &mut metadata,
            "archive.defining",
            "Handle",
            "value",
            "Bool",
            true,
        );
        add_archive_record(
            &mut metadata,
            "archive.surface",
            "Slot",
            "pointer",
            "RawPtr",
            true,
        );
        let mut bindings = verum_common::OrderedMap::new();
        bindings.insert("RawPtr".into(), Some("archive.defining.Handle".into()));
        metadata
            .declaration_type_bindings
            .insert("archive.surface".into(), bindings);
        let errors = check(
            &mut archive_checker(metadata, eager),
            "realm.client",
            &[(
                "realm.client",
                "mount archive.surface.Slot; fn inspect(slot:Slot)->Bool {slot.pointer.value}",
            )],
        );
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
    }
}

#[test]
fn unresolved_retained_bindings_cannot_be_replaced_by_builtins() {
    for binding in [None, Some(Text::from("archive.missing.Handle"))] {
        for eager in [false, true] {
            let mut metadata = verum_types::core_metadata::CoreMetadata::default();
            add_archive_record(
                &mut metadata,
                "archive.surface",
                "Slot",
                "pointer",
                "RawPtr",
                true,
            );
            let mut bindings = verum_common::OrderedMap::new();
            bindings.insert("RawPtr".into(), binding.clone());
            metadata
                .declaration_type_bindings
                .insert("archive.surface".into(), bindings);
            let errors = check(
                &mut archive_checker(metadata, eager),
                "realm.client",
                &[(
                    "realm.client",
                    "mount archive.surface.Slot; fn inspect(slot:&Slot) {}",
                )],
            );
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("TypeNotFound") && error.contains("RawPtr")),
                "binding={binding:?}, eager={eager}: {errors:?}"
            );
        }
    }
}
