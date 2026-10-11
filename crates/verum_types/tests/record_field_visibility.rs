//! T1713: imported nominal records retain their declared field visibility.
//! Source imports and descriptor consumers are independent paths here; the
//! compiler bootstrap companion exercises actual archive production separately.
use std::sync::Arc;
use verum_ast::{FileId, ItemKind, Visibility};
use verum_common::{List, ResourceDiscipline, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;
use verum_types::core_metadata::{
    CoreMetadata, FieldDescriptor, TypeDescriptor, TypeDescriptorKind,
};

const ALPHA: &str = r#"
    public type Packet is { body: Int, public alpha_only: Int };
    implement Packet {
        public fn new(body: Int) -> Packet { Packet { body, alpha_only: 3 } }
    }
"#;
const BETA: &str = r#"
    public type Packet is { public body: Bool, public beta_only: Int };
    implement Packet {
        public fn new(body: Bool) -> Packet { Packet { body, beta_only: 5 } }
    }
"#;
const OWNERS: [&str; 2] = ["core.field_alpha", "core.field_beta"];

#[derive(Clone, Copy, Debug)]
enum Input {
    Source,
    Metadata { eager: bool },
}

fn metadata(reverse: bool) -> CoreMetadata {
    let mut metadata = CoreMetadata::default();
    let mut declarations = [
        (OWNERS[0], "Int", "alpha_only"),
        (OWNERS[1], "Bool", "beta_only"),
    ];
    if reverse {
        declarations.reverse();
    }
    for (owner, body, own_field) in declarations {
        let descriptor = TypeDescriptor {
            name: "Packet".into(),
            module_path: owner.into(),
            origin_module_path: None,
            generic_params: List::new(),
            kind: TypeDescriptorKind::Record {
                fields: [("body", body), (own_field, "Int")]
                    .into_iter()
                    .map(|(name, ty)| FieldDescriptor {
                        name: name.into(),
                        ty: ty.into(),
                        is_public: owner != OWNERS[0] || name != "body",
                        declared_visibility: Some(if owner == OWNERS[0] && name == "body" {
                            Visibility::Private
                        } else {
                            Visibility::Public
                        }),
                    })
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
        let exact: Text = format!("{owner}.Packet").into();
        metadata.types.insert(exact.clone(), descriptor.clone());
        metadata.type_declaration_order.push(exact);
        // Deliberately vary the flat-slot winner while retaining both owners.
        metadata.types.insert("Packet".into(), descriptor);
    }
    metadata
}

fn check(consumer: &str, input: Input, reverse: bool, local: bool) -> (TypeChecker, List<Text>) {
    let mut checker = match input {
        Input::Source => TypeChecker::new(),
        Input::Metadata { eager } => {
            let metadata = Arc::new(metadata(reverse));
            if eager {
                TypeChecker::new_with_core_eager(metadata)
            } else {
                TypeChecker::new_with_core(metadata)
            }
        }
    };
    checker.register_primitives();
    if consumer.contains("Heap<") {
        // Register the existing bare smart-pointer carrier form. A record
        // declared in consumer would instead be nominal consumer.Heap.
        let ast = FastParser::new()
            .parse_module_str("type Heap<T> is { payload: T };", FileId::new(90))
            .expect("pointer grammar");
        for item in &ast.items {
            if let ItemKind::Type(declaration) = &item.kind {
                checker
                    .register_type_declaration(declaration)
                    .expect("pointer declaration");
            }
        }
    }
    checker.set_current_module_path("consumer");
    let mut registry = ModuleRegistry::new();
    let mut declarations = [(OWNERS[0], ALPHA), (OWNERS[1], BETA)];
    if reverse {
        declarations.reverse();
    }
    for (index, (owner, source)) in declarations.into_iter().enumerate() {
        let source = if matches!(input, Input::Source) && !local {
            source
        } else {
            ""
        };
        let id = ModuleId::new(index as u32);
        let file = FileId::new(index as u32);
        let ast = FastParser::new()
            .parse_module_str(source, file)
            .expect("source grammar");
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), file, source.into());
        info.exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        registry.register(info);
    }
    let ast = FastParser::new()
        .parse_module_str(consumer, FileId::new(2))
        .expect("consumer grammar");
    let id = ModuleId::new(2);
    let path = ModulePath::from_str("consumer");
    let mut info = ModuleInfo::new(
        id,
        path.clone(),
        ast.clone(),
        FileId::new(2),
        consumer.into(),
    );
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
            ItemKind::Type(decl) => checker.register_type_declaration(decl),
            ItemKind::Impl(decl) => checker.register_impl_block(decl),
            ItemKind::Function(decl) => checker.register_function_signature(decl),
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

fn assert_private(input: Input, body: &str) {
    for reverse in [false, true] {
        let source = format!(
            "mount core.field_alpha.{{Packet as A}}; mount core.field_beta.{{Packet as B}}; {body}"
        );
        let (_, errors) = check(&source, input, reverse, false);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("VisibilityError") && e.contains("body")),
            "{input:?}, reverse={reverse}: private field must be specifically refused: {errors:?}"
        );
    }
}

#[test]
fn source_external_private_read_is_refused() {
    assert_private(Input::Source, "fn probe(value: &A) -> Int { value.body }");
}

#[test]
fn source_external_private_write_is_refused() {
    assert_private(Input::Source, "fn probe(value: &mut A) { value.body = 7; }");
}

#[test]
fn source_inferred_private_construction_is_refused() {
    assert_private(
        Input::Source,
        "fn probe() { let value = A { body: 7, alpha_only: 3 }; }",
    );
}

#[test]
fn source_annotated_private_construction_is_refused() {
    assert_private(
        Input::Source,
        "fn probe() { let value: A = A { body: 7, alpha_only: 3 }; }",
    );
}

#[test]
fn source_private_record_pattern_is_refused() {
    assert_private(
        Input::Source,
        "fn probe(value: A) -> Int { let A { body, .. } = value; body }",
    );
}

#[test]
fn source_private_spread_construction_is_refused() {
    assert_private(
        Input::Source,
        "fn probe(value: A) -> A { A { alpha_only: 7, ..value } }",
    );
}

#[test]
fn source_private_optional_field_is_refused() {
    assert_private(
        Input::Source,
        "fn probe(value: A) { let selected = value?.body; }",
    );
}

#[test]
fn source_alias_keeps_private_field_owner() {
    assert_private(
        Input::Source,
        "type Alias is A; fn probe(value: &Alias) -> Int { value.body }",
    );
}

#[test]
fn source_local_private_fields_remain_usable() {
    let source = format!(
        "{ALPHA} fn probe() -> Int {{ let mut value = Packet {{ body: 1, alpha_only: 3 }}; value.body = 7; value.body }}"
    );
    let (_, errors) = check(&source, Input::Source, false, true);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn source_public_fields_remain_usable() {
    let source = "mount core.field_alpha.{Packet as A}; mount core.field_beta.{Packet as B}; fn first(value: &A) -> Int { value.alpha_only } fn second(value: &mut B) -> Bool { value.body = true; value.body }";
    let (_, errors) = check(source, Input::Source, false, false);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn metadata_external_private_read_is_refused() {
    for eager in [false, true] {
        assert_private(
            Input::Metadata { eager },
            "fn probe(value: &A) -> Int { value.body }",
        );
    }
}

#[test]
fn metadata_external_private_write_is_refused() {
    for eager in [false, true] {
        assert_private(
            Input::Metadata { eager },
            "fn probe(value: &mut A) { value.body = 7; }",
        );
    }
}

#[test]
fn metadata_external_private_construction_is_refused() {
    for eager in [false, true] {
        assert_private(
            Input::Metadata { eager },
            "fn probe() { let value: A = A { body: 7, alpha_only: 3 }; }",
        );
    }
}

#[test]
fn metadata_public_fields_remain_usable() {
    for eager in [false, true] {
        let source = "mount core.field_alpha.{Packet as A}; mount core.field_beta.{Packet as B}; fn first(value: &A) -> Int { value.alpha_only } fn second(value: &mut B) -> Bool { value.body = true; value.body }";
        let (_, errors) = check(source, Input::Metadata { eager }, false, false);
        assert!(errors.is_empty(), "{errors:?}");
    }
}

#[test]
fn source_compound_private_write_is_refused() {
    assert_private(
        Input::Source,
        "fn probe(value: &mut A) { value.body += 7; }",
    );
}

#[test]
fn source_private_scalar_field_call_remains_refused() {
    for reverse in [false, true] {
        let source = "mount core.field_alpha.{Packet as A}; mount core.field_beta.{Packet as B}; fn probe(value: A) -> Int { value.body() }";
        let (_, errors) = check(source, Input::Source, reverse, false);
        // This nominal receiver already refuses scalar-field call syntax.
        // Preserve that boundary without claiming it was an accepted bypass.
        assert!(
            errors.iter().any(
                |e| (e.contains("VisibilityError") || e.contains("MethodNotFound"))
                    && e.contains("body")
            ),
            "{errors:?}"
        );
    }
}

#[test]
fn source_inferred_factory_value_keeps_private_fields() {
    assert_private(
        Input::Source,
        "fn probe() -> Int { let value = A.new(7); value.body }",
    );
}

#[test]
fn source_generic_alias_keeps_private_owner() {
    for alias in ["Identity<A>", "Identity<Identity<A>>"] {
        assert_private(
            Input::Source,
            &format!("type Identity<T> is T; fn probe(value: &{alias}) -> Int {{ value.body }}"),
        );
    }
}

#[test]
fn metadata_generic_alias_keeps_private_owner() {
    for eager in [false, true] {
        for alias in ["Identity<A>", "Identity<Identity<A>>"] {
            assert_private(
                Input::Metadata { eager },
                &format!(
                    "type Identity<T> is T; fn probe(value: &{alias}) -> Int {{ value.body }}"
                ),
            );
        }
    }
}

#[test]
fn auto_dereference_keeps_private_owner() {
    for input in [
        Input::Source,
        Input::Metadata { eager: false },
        Input::Metadata { eager: true },
    ] {
        assert_private(input, "fn probe(value: &Heap<A>) -> Int { value.body }");
    }
}

#[test]
fn generic_aliases_and_auto_dereference_keep_public_fields() {
    for input in [
        Input::Source,
        Input::Metadata { eager: false },
        Input::Metadata { eager: true },
    ] {
        for reverse in [false, true] {
            for alias in ["Identity<B>", "Identity<Identity<B>>", "Heap<B>"] {
                let source = format!(
                    "mount core.field_alpha.{{Packet as A}}; mount core.field_beta.{{Packet as B}}; type Identity<T> is T; fn probe(value: &{alias}) -> Bool {{ value.body }}"
                );
                let (_, errors) = check(&source, input, reverse, false);
                assert!(
                    errors.is_empty(),
                    "{input:?}, reverse={reverse}, {alias}: {errors:?}"
                );
            }
        }
    }
}

#[test]
fn metadata_private_record_pattern_is_refused() {
    for eager in [false, true] {
        assert_private(
            Input::Metadata { eager },
            "fn probe(value: A) -> Int { let A { body, .. } = value; body }",
        );
    }
}

#[test]
fn metadata_private_optional_field_is_refused() {
    for eager in [false, true] {
        assert_private(
            Input::Metadata { eager },
            "fn probe(value: A) { let selected = value?.body; }",
        );
    }
}

#[test]
fn public_record_construction_and_pattern_remain_usable() {
    for input in [
        Input::Source,
        Input::Metadata { eager: false },
        Input::Metadata { eager: true },
    ] {
        for reverse in [false, true] {
            let source = "mount core.field_beta.{Packet as B}; fn probe() -> Bool { let value: B = B { body: true, beta_only: 5 }; let B { body, .. } = value; body }";
            let (_, errors) = check(source, input, reverse, false);
            assert!(
                errors.is_empty(),
                "{input:?}, reverse={reverse}: {errors:?}"
            );
        }
    }
}

#[test]
fn a_structural_annotation_cannot_expose_private_record_fields() {
    for input in [
        Input::Source,
        Input::Metadata { eager: false },
        Input::Metadata { eager: true },
    ] {
        for probe in [
            "fn probe(value: A) -> Int { let fields: { body: Int, alpha_only: Int } = value; fields.body }",
            "fn unpack(value: { body: Int, alpha_only: Int }) -> Int { value.body } fn probe(value: A) -> Int { unpack(value) }",
        ] {
            let source = format!("mount core.field_alpha.{{Packet as A}}; {probe}");
            let (_, errors) = check(&source, input, false, false);
            assert!(
                !errors.is_empty(),
                "{input:?}: structural coercion cannot erase private authority: {probe}"
            );
        }
    }
}

#[test]
fn a_structural_annotation_preserves_public_record_fields() {
    for input in [
        Input::Source,
        Input::Metadata { eager: false },
        Input::Metadata { eager: true },
    ] {
        for probe in [
            "fn probe(value: B) -> Bool { let fields: { body: Bool, beta_only: Int } = value; fields.body }",
            "fn unpack(value: { body: Bool, beta_only: Int }) -> Bool { value.body } fn probe(value: B) -> Bool { unpack(value) }",
        ] {
            let source = format!("mount core.field_beta.{{Packet as B}}; {probe}");
            let (_, errors) = check(&source, input, false, false);
            assert!(
                errors.is_empty(),
                "{input:?}: public structural view: {errors:?}"
            );
        }
    }
}
