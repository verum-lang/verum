//! T1692: field reads and writes must use the same declaring record owner.
//! Source imports and descriptor consumers are independent paths here; the
//! compiler bootstrap companion exercises actual archive production separately.
use std::sync::Arc;
use verum_ast::{FileId, ItemKind};
use verum_common::{List, ResourceDiscipline, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::core_metadata::{
    CoreMetadata, FieldDescriptor, TypeDescriptor, TypeDescriptorKind,
};
use verum_types::{Type, TypeChecker};

const ALPHA: &str = r#"
    public type Packet is { public body: Int, public alpha_only: Int };
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
                    declared_visibility: None,
                        name: name.into(),
                        ty: ty.into(),
                        is_public: true,
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

fn maps(checker: &TypeChecker) -> Text {
    [
        "__struct_fields_Packet",
        "__struct_fields_core.field_alpha.Packet",
        "core.field_alpha.__struct_fields_Packet",
        "__struct_fields_core.field_beta.Packet",
        "core.field_beta.__struct_fields_Packet",
    ]
    .into_iter()
    .map(|key| Text::from(format!("{key}={:?}", checker.lookup_type_for_testing(key))))
    .collect::<List<_>>()
    .join("; ")
    .into()
}

fn assert_owner(checker: &mut TypeChecker, function: &str, expected: &str) {
    let scheme = checker
        .context_mut()
        .env
        .lookup(function)
        .expect("registered function");
    let Type::Function { return_type, .. } = &scheme.ty else {
        panic!("not a function: {scheme:?}")
    };
    let Type::Named { path, .. } = return_type.as_ref() else {
        panic!("not nominal: {scheme:?}")
    };
    let owner = path
        .segments
        .iter()
        .map(|segment| match segment {
            verum_ast::ty::PathSegment::Name(name) => name.name.as_str(),
            _ => panic!("unexpected owner segment"),
        })
        .collect::<List<_>>()
        .join(".");
    assert_eq!(owner, expected, "{scheme:?}");
}

fn source_body(input: Input, body: &str, reverse: bool) -> (TypeChecker, List<Text>) {
    let mounts = if reverse {
        "mount core.field_beta.{Packet as B}; mount core.field_alpha.{Packet as A};"
    } else {
        "mount core.field_alpha.{Packet as A}; mount core.field_beta.{Packet as B};"
    };
    check(&format!("{mounts} {body}"), input, reverse, false)
}

fn accepts(input: Input, body: &str) {
    for reverse in [false, true] {
        let (checker, errors) = source_body(input, body, reverse);
        assert!(
            errors.is_empty(),
            "{input:?}, reverse={reverse}: {errors:?}; {}",
            maps(&checker)
        );
    }
}

fn refuses_wrong_type(input: Input) {
    for reverse in [false, true] {
        for body in [
            "fn wrong(value: &mut A) { value.body = true; }",
            "fn wrong(value: &mut B) { value.body = 7; }",
        ] {
            let (checker, errors) = source_body(input, body, reverse);
            assert!(
                errors.iter().any(|e| e.contains("Mismatch")),
                "{input:?}, reverse={reverse}: {errors:?}; {}",
                maps(&checker)
            );
            assert!(
                !errors
                    .iter()
                    .any(|e| e.contains("not found") || e.contains("UnknownField")),
                "wrong-type refusal must resolve the declared field: {errors:?}"
            );
        }
    }
}

fn refuses_sibling_field(input: Input) {
    for reverse in [false, true] {
        for body in [
            "fn wrong(value: &A) -> Int { value.beta_only }",
            "fn wrong(value: &mut A) { value.beta_only = 7; }",
            "fn wrong(value: &B) -> Int { value.alpha_only }",
            "fn wrong(value: &mut B) { value.alpha_only = 7; }",
        ] {
            let (checker, errors) = source_body(input, body, reverse);
            assert!(
                errors
                    .iter()
                    .any(|e| e.contains("UnknownField") || e.contains("not found")),
                "{input:?}, reverse={reverse}: foreign field was accepted: {errors:?}; {}",
                maps(&checker)
            );
        }
    }
}

#[test]
fn local_source_read_and_write_remain_valid() {
    let source = format!(
        "{ALPHA} fn probe() -> Int {{ let mut value = Packet.new(1); value.body = 7; value.body }}"
    );
    let (checker, errors) = check(&source, Input::Source, false, true);
    assert!(errors.is_empty(), "{errors:?}; {}", maps(&checker));
}

#[test]
fn source_mounts_preserve_both_nominal_owners() {
    for reverse in [false, true] {
        let (mut checker, errors) = source_body(
            Input::Source,
            "fn first(value: A) -> A { value } fn second(value: B) -> B { value }",
            reverse,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_owner(&mut checker, "first", "core.field_alpha.Packet");
        assert_owner(&mut checker, "second", "core.field_beta.Packet");
    }
}

#[test]
fn source_mounted_reads_use_the_declared_owner() {
    accepts(
        Input::Source,
        "fn first(value: &A) -> Int { value.body } fn second(value: &B) -> Bool { value.body }",
    );
}

#[test]
fn source_mounted_writes_use_the_declared_owner() {
    accepts(
        Input::Source,
        "fn first(value: &mut A) { value.body = 7; } fn second(value: &mut B) { value.body = true; }",
    );
}

#[test]
fn source_constructor_result_allows_field_write_and_read() {
    accepts(
        Input::Source,
        "fn first() -> Int { let mut value = A.new(1); value.body = 7; value.body } fn second() -> Bool { let mut value = B.new(false); value.body = true; value.body }",
    );
}

#[test]
fn source_writes_refuse_the_sibling_field_type() {
    refuses_wrong_type(Input::Source);
}

#[test]
fn source_read_and_write_refuse_a_sibling_only_field() {
    refuses_sibling_field(Input::Source);
}

#[test]
fn metadata_mounted_reads_use_the_declared_owner() {
    for eager in [false, true] {
        accepts(
            Input::Metadata { eager },
            "fn first(value: &A) -> Int { value.body } fn second(value: &B) -> Bool { value.body }",
        );
    }
}

#[test]
fn metadata_mounted_writes_use_the_declared_owner() {
    for eager in [false, true] {
        accepts(
            Input::Metadata { eager },
            "fn first(value: &mut A) { value.body = 7; } fn second(value: &mut B) { value.body = true; }",
        );
    }
}

#[test]
fn metadata_writes_refuse_the_sibling_field_type() {
    for eager in [false, true] {
        refuses_wrong_type(Input::Metadata { eager });
    }
}

#[test]
fn metadata_read_and_write_refuse_a_sibling_only_field() {
    for eager in [false, true] {
        refuses_sibling_field(Input::Metadata { eager });
    }
}

#[test]
fn missing_export_cannot_take_fields_from_a_same_leaf_metadata_record() {
    for eager in [false, true] {
        for reverse in [false, true] {
            let source = "mount core.field_missing.{Packet}; fn wrong(value: &mut Packet) { value.body = 7; }";
            let (_, errors) = check(source, Input::Metadata { eager }, reverse, false);
            assert!(
                errors
                    .iter()
                    .any(|e| e.contains("Import") || e.contains("ModuleNotFound")),
                "eager={eager}, reverse={reverse}: {errors:?}"
            );
        }
    }
}
