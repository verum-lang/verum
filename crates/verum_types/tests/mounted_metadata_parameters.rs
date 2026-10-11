//! T1733: mounted metadata templates own their arity, defaults and parameter slots.
//!
//! The metadata rows below mirror the parsed declarations explicitly. Decoding
//! their JSON exercises the consumer data model; it is not an archive producer
//! test. Empty module ASTs with parsed export tables select the real metadata
//! import path instead of registering the declaration source as a shortcut.

use verum_ast::{FileId, ItemKind};
use verum_common::{List, ResourceDiscipline, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::{TypeChecker, core_metadata::*};

const ALPHA: &str = "core.parameter_alpha";
const BETA: &str = "core.parameter_beta";
const ALPHA_SOURCE: &str = r#"
public type Choice<Value, Failure = Text> is | Item(Value) | Other(Failure);
public type Handler<Return, Input = Text> is fn(Input) -> Return;
"#;
const BETA_SOURCE: &str = r#"
public type Choice<Noise, Value, Failure> is | Item(Value) | Other(Failure);
public type Handler<Noise, Return, Input> is fn(Input) -> Return;
"#;

#[derive(Clone, Copy, Debug)]
enum Surface {
    Source,
    Metadata { eager: bool, decoded: bool },
}

fn descriptor(owner: &str, alias: bool) -> TypeDescriptor {
    let names: &[&str] = match (owner == ALPHA, alias) {
        (true, false) => &["Value", "Failure"],
        (false, false) => &["Noise", "Value", "Failure"],
        (true, true) => &["Return", "Input"],
        (false, true) => &["Noise", "Return", "Input"],
    };
    TypeDescriptor {
        name: if alias { "Handler" } else { "Choice" }.into(),
        module_path: "core.parameter_bundle".into(),
        origin_module_path: Some(owner.into()),
        generic_params: names
            .iter()
            .enumerate()
            .map(|(index, name)| GenericParam {
                name: (*name).into(),
                bounds: List::new(),
                default: (owner == ALPHA && index == 1).then(|| "Text".into()),
                type_bounds: List::new(),
                pid: Some(index as u16),
            })
            .collect(),
        kind: if alias {
            TypeDescriptorKind::Alias {
                target: "fn(Input) -> Return".into(),
            }
        } else {
            TypeDescriptorKind::Variant {
                cases: [("Item", "Value"), ("Other", "Failure")]
                    .into_iter()
                    .map(|(name, parameter)| VariantCase {
                        name: name.into(),
                        payload: Some(VariantPayload::Tuple(List::from_iter([parameter.into()]))),
                    })
                    .collect(),
            }
        },
        size: None,
        alignment: None,
        methods: List::new(),
        implements: List::new(),
        decl_span: None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
    }
}

fn check(source: &str, surface: Surface, reverse: bool) -> List<Text> {
    check_with_ambient(source, surface, reverse, BETA)
}

fn check_with_ambient(source: &str, surface: Surface, reverse: bool, ambient: &str) -> List<Text> {
    let owners = if reverse {
        [BETA, ALPHA]
    } else {
        [ALPHA, BETA]
    };
    let mut metadata = CoreMetadata::default();
    for owner in owners {
        for alias in [false, true] {
            let descriptor = descriptor(owner, alias);
            let key: Text = format!("{owner}.{}", descriptor.name).into();
            metadata.types.insert(key.clone(), descriptor);
            metadata.type_declaration_order.push(key);
        }
    }
    // Mounted controls choose the conflicting three-parameter owner; ambient
    // controls choose the two-parameter owner with its own declared default.
    for alias in [false, true] {
        let descriptor = descriptor(ambient, alias);
        metadata.types.insert(descriptor.name.clone(), descriptor);
    }
    let mut checker = match surface {
        Surface::Source => TypeChecker::new(),
        Surface::Metadata { eager, decoded } => {
            let metadata = if decoded {
                serde_json::from_slice(&serde_json::to_vec(&metadata).expect("metadata encode"))
                    .expect("metadata decode")
            } else {
                metadata
            };
            if eager {
                TypeChecker::new_with_core_eager(std::sync::Arc::new(metadata))
            } else {
                TypeChecker::new_with_core(std::sync::Arc::new(metadata))
            }
        }
    };
    checker.register_primitives();
    checker.set_current_cog("demo");
    checker.set_current_module_path("demo.main");
    let mut registry = ModuleRegistry::new();
    for (index, owner) in owners.iter().enumerate() {
        let file = FileId::new(index as u32);
        let declaration = if *owner == ALPHA {
            ALPHA_SOURCE
        } else {
            BETA_SOURCE
        };
        let parsed = FastParser::new()
            .parse_module_str(declaration, file)
            .expect("declaration grammar");
        let ast = if matches!(surface, Surface::Source) {
            parsed.clone()
        } else {
            FastParser::new()
                .parse_module_str("", file)
                .expect("empty metadata AST")
        };
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(id, path.clone(), ast, file, declaration.into());
        info.exports = extract_exports_from_module(&parsed, id, &path).expect("parsed exports");
        registry.register(info);
    }
    let file = FileId::new(2);
    let ast = FastParser::new()
        .parse_module_str(source, file)
        .expect("consumer grammar");
    let id = ModuleId::new(2);
    let path = ModulePath::from_str("demo.main");
    let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), file, source.into());
    info.exports = extract_exports_from_module(&ast, id, &path).expect("consumer exports");
    registry.register(info);
    checker.set_module_registry_direct(registry.clone());
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "demo.main", &registry) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    for item in &ast.items {
        if let ItemKind::Function(function) = &item.kind {
            if let Err(error) = checker.register_function_signature(function) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    for item in &ast.items {
        if !matches!(item.kind, ItemKind::Mount(_)) {
            if let Err(error) = checker.check_item(item) {
                errors.push(format!("{error:?}").into());
            }
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

fn surfaces() -> [Surface; 5] {
    [
        Surface::Source,
        Surface::Metadata {
            eager: false,
            decoded: false,
        },
        Surface::Metadata {
            eager: false,
            decoded: true,
        },
        Surface::Metadata {
            eager: true,
            decoded: false,
        },
        Surface::Metadata {
            eager: true,
            decoded: true,
        },
    ]
}

fn assert_accepts(source: &str) {
    for surface in surfaces() {
        for reverse in [false, true] {
            let errors = check(source, surface, reverse);
            assert!(
                errors.is_empty(),
                "{surface:?}, reverse={reverse}: {errors:?}"
            );
        }
    }
}

fn assert_arity_refusal(source: &str, name: &str) {
    for surface in surfaces() {
        for reverse in [false, true] {
            let errors = check(source, surface, reverse);
            assert!(
                errors.iter().any(|error| error.contains("E407")
                    && error.contains(name)
                    && error.contains("takes 2 type argument(s)")),
                "{surface:?}, reverse={reverse}: {errors:?}"
            );
            assert!(
                !errors
                    .iter()
                    .any(|error| error.contains("ImportItemNotFound")
                        || error.contains("TypeNotFound")),
                "setup must resolve: {errors:?}"
            );
        }
    }
}

#[test]
fn mounted_sum_uses_its_declared_default_payload() {
    assert_accepts(
        r#"
mount core.parameter_alpha.Choice;
fn text(value: Text) {}
fn probe(value: Choice<Int>) -> Int {
    match value { Choice.Item(item) => item, Choice.Other(error) => { text(error); 0 } }
}
"#,
    );
}

#[test]
fn mounted_sum_keeps_both_explicit_parameter_slots() {
    assert_accepts(
        r#"
mount core.parameter_alpha.Choice;
fn boolean(value: Bool) {}
fn probe(value: Choice<Int, Bool>) -> Int {
    match value { Choice.Item(item) => item, Choice.Other(error) => { boolean(error); 0 } }
}
"#,
    );
}

#[test]
fn mounted_sum_rejects_the_foreign_declarations_arity() {
    assert_arity_refusal(
        "mount core.parameter_alpha.Choice; fn probe(value: Choice<Int, Text, Bool>) {}",
        "Choice",
    );
}

#[test]
fn mounted_alias_uses_its_declared_default_argument() {
    assert_accepts(
        r#"
mount core.parameter_alpha.Handler;
fn probe(callback: Handler<Int>) -> Int { callback("input") }
"#,
    );
}

#[test]
fn mounted_alias_keeps_explicit_argument_order() {
    assert_accepts(
        r#"
mount core.parameter_alpha.Handler;
fn probe(callback: Handler<Int, Bool>) -> Int { callback(true) }
"#,
    );
}

#[test]
fn mounted_alias_rejects_the_foreign_declarations_arity() {
    assert_arity_refusal(
        "mount core.parameter_alpha.Handler; fn probe(callback: Handler<Int, Text, Bool>) {}",
        "Handler",
    );
}

#[test]
fn ambient_sum_uses_its_declared_default_payload() {
    let source = r#"
fn text(value: Text) {}
fn make() -> Choice<Int> { Choice.Item(7) }
fn make_error() -> Choice<Int> { Choice.Other("expected") }
fn probe(value: Choice<Int>) -> Int {
    match value { Choice.Item(item) => item, Choice.Other(error) => { text(error); 0 } }
}
fn check_constructed() -> Int { probe(make()) + probe(make_error()) }
"#;
    for surface in surfaces()
        .into_iter()
        .filter(|surface| matches!(surface, Surface::Metadata { .. }))
    {
        for reverse in [false, true] {
            let errors = check_with_ambient(source, surface, reverse, ALPHA);
            assert!(
                errors.is_empty(),
                "{surface:?}, reverse={reverse}: {errors:?}"
            );
            let errors = check_with_ambient(
                "fn wrong_error() -> Choice<Int> { Choice.Other(3) }",
                surface,
                reverse,
                ALPHA,
            );
            assert!(
                errors.iter().any(|error| error.contains("Mismatch")
                    && error.contains("Text")
                    && error.contains("Int")),
                "a constructor must use the defaulted error slot: {surface:?}, reverse={reverse}: {errors:?}"
            );
            assert!(
                !errors
                    .iter()
                    .any(|error| error.contains("TypeNotFound")
                        || error.contains("ImportItemNotFound")),
                "setup must resolve: {errors:?}"
            );
        }
    }
}

#[test]
fn ambient_alias_uses_its_declared_default_argument() {
    let source = "fn probe(callback: Handler<Int>) -> Int { callback(\"input\") }";
    for surface in surfaces()
        .into_iter()
        .filter(|surface| matches!(surface, Surface::Metadata { .. }))
    {
        for reverse in [false, true] {
            let errors = check_with_ambient(source, surface, reverse, ALPHA);
            assert!(
                errors.is_empty(),
                "{surface:?}, reverse={reverse}: {errors:?}"
            );
        }
    }
}

#[test]
fn ambient_templates_reject_the_foreign_declarations_arity() {
    for (name, source) in [
        ("Choice", "fn probe(value: Choice<Int, Text, Bool>) {}"),
        ("Handler", "fn probe(callback: Handler<Int, Text, Bool>) {}"),
    ] {
        for surface in surfaces()
            .into_iter()
            .filter(|surface| matches!(surface, Surface::Metadata { .. }))
        {
            for reverse in [false, true] {
                let errors = check_with_ambient(source, surface, reverse, ALPHA);
                assert!(
                    errors.iter().any(|error| error.contains("E407")
                        && error.contains(name)
                        && error.contains("takes 2 type argument(s)")),
                    "{name}, {surface:?}, reverse={reverse}: {errors:?}"
                );
                assert!(
                    !errors.iter().any(|error| error.contains("TypeNotFound")
                        || error.contains("ImportItemNotFound")),
                    "setup must resolve: {errors:?}"
                );
            }
        }
    }
}
