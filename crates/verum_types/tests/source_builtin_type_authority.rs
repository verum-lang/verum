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

#[test]
fn literal_carrier_uses_the_declared_standard_text_representation_through_aliases() {
    let library = "@no_implicit_prelude module core.text.text; @opaque public type Text is { ptr: &unsafe Byte, len: Int, cap: Int };";
    for body in [
        "fn make()->Words { \"verum\" }",
        "fn receive(words:Words)->Text {words}",
        "fn storage_size()->Int {Words.size}",
        "fn take(text:Text)->Int {0} fn call(words:Words)->Int {take(words)}",
    ] {
        let consumer = format!(
            "@no_implicit_prelude module consumer; mount core.text.text.Text as Words; {body}"
        );
        let errors = check(
            &mut checker(),
            "consumer",
            &[("core.text.text", library), ("consumer", &consumer)],
        );
        assert!(errors.is_empty(), "{body}: {errors:?}");
    }
}

#[test]
fn foreign_text_records_remain_distinct_from_the_literal_carrier() {
    let library = "@no_implicit_prelude module foreign; @opaque public type Text is { ptr: &unsafe Byte, len: Int, cap: Int };";
    for body in [
        "fn make()->Words { \"verum\" }",
        "fn receive(words:Words)->Text {words}",
    ] {
        let consumer =
            format!("@no_implicit_prelude module consumer; mount foreign.Text as Words; {body}");
        let errors = check(
            &mut checker(),
            "consumer",
            &[("foreign", library), ("consumer", &consumer)],
        );
        assert!(
            errors.iter().any(|e| e.contains("Mismatch")),
            "{body}: {errors:?}"
        );
    }
}

#[test]
fn source_representation_methods_share_the_literal_type_and_keep_fields_private() {
    let library = r#"@no_implicit_prelude module core.text.text;
        @opaque public type Text is { ptr: &unsafe Byte, len: Int, cap: Int };
        implement Text {
            public fn byte_count(&self) -> Int { self.len }
            public fn from_owned(value: Text) -> Text { value }
        }"#;
    for consumer in [
        "@no_implicit_prelude module consumer; mount core.text.text.Text as Words; fn make()->Text {Words.from_owned(\"abc\")}",
        "@no_implicit_prelude module consumer; mount core.text.text.Text as Words; fn read(words:Words)->Int { words.byte_count() }",
    ] {
        let errors = check(
            &mut checker(),
            "consumer",
            &[("core.text.text", library), ("consumer", consumer)],
        );
        assert!(errors.is_empty(), "{consumer}: {errors:?}");
    }
    let consumer = "@no_implicit_prelude module consumer; mount core.text.text.Text as Words; fn read(words:Words)->Int {words.cap}";
    let errors = check(
        &mut checker(),
        "consumer",
        &[("core.text.text", library), ("consumer", consumer)],
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("VisibilityError") && error.contains("cap")),
        "{errors:?}"
    );
    let decoy =
        "@no_implicit_prelude module foreign; public type Text is { public foreign_field: Bool };";
    let consumer = "@no_implicit_prelude module consumer; mount core.text.text.Text as Words; mount foreign.Text as Foreign; fn read(words:Words)->Bool {words.foreign_field}";
    let errors = check(
        &mut checker(),
        "consumer",
        &[
            ("core.text.text", library),
            ("foreign", decoy),
            ("consumer", consumer),
        ],
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("E103") && error.contains("foreign_field")),
        "{errors:?}"
    );
}

#[test]
fn archive_representation_keeps_literal_identity_only_for_the_defining_module() {
    for eager in [false, true] {
        for (owner, accepts) in [("core.text.text", true), ("foreign", false)] {
            let mut meta = verum_types::core_metadata::CoreMetadata::default();
            add_archive_record(&mut meta, owner, "Text", "size_field", "Int", true);
            let consumer = format!(
                "@no_implicit_prelude module consumer; mount {owner}.Text as Words; fn make()->Words {{ \"verum\" }}"
            );
            let errors = check(
                &mut archive_checker(meta, eager),
                "consumer",
                &[("consumer", &consumer)],
            );
            if accepts {
                assert!(errors.is_empty(), "eager={eager} {owner}: {errors:?}");
            } else {
                assert!(
                    errors.iter().any(|e| e.contains("Mismatch")),
                    "eager={eager} {owner}: {errors:?}"
                );
            }
        }
    }
}

#[test]
fn descriptor_only_archives_use_the_representation_field_owner_and_type() {
    use verum_types::core_metadata::{CoreMetadata, TypeDescriptorKind};
    for (visibility, is_public) in [
        (verum_ast::Visibility::Public, true),
        (verum_ast::Visibility::Private, false),
    ] {
        let mut metadata = CoreMetadata::default();
        add_archive_record(&mut metadata, "core.text.text", "Text", "data", "Int", true);
        let descriptor = metadata
            .types
            .get_mut(&Text::from("core.text.text.Text"))
            .unwrap();
        descriptor.source_declaration = None;
        let TypeDescriptorKind::Record { fields } = &mut descriptor.kind else {
            panic!("expected a record descriptor")
        };
        fields[0].declared_visibility = Some(visibility);
        fields[0].is_public = is_public;
        let source = "@no_implicit_prelude module consumer; mount core.text.text.Text as Words; fn read(words:Words)->Int { words.data }";
        for eager in [false, true] {
            let errors = check(
                &mut archive_checker(metadata.clone(), eager),
                "consumer",
                &[("consumer", source)],
            );
            if is_public {
                assert!(errors.is_empty(), "eager={eager}: {errors:?}");
            } else {
                assert!(
                    errors
                        .iter()
                        .any(|e| e.contains("VisibilityError") && e.contains("data")),
                    "eager={eager}: {errors:?}"
                );
            }
        }
    }
}
