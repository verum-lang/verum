//! Prepared T1713 compatibility controls; not executed by the preparing agent.
//! Parsed declarations supply field/variant/protocol authority. The two late
//! projection controls explicitly seed the existing deferred Type carrier so
//! eager annotation resolution cannot hide the field-access ordering boundary.

use verum_ast::{FileId, ItemKind, Module};
use verum_common::{Heap, List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::{Type, TypeChecker, context::TypeScheme};

const OWNER: &str = "core.field_compatibility";
const DECLARATIONS: &str = r#"
    public type Packet is { public visible: Int, hidden: Int, body: Int };
    public type Choice is | Value { public number: Int } | Empty;
"#;

fn parse(source: &str, file: u32) -> Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(file))
        .expect("fixture follows the production grammar")
}

fn collect_diagnostics(checker: &mut TypeChecker, errors: &mut List<Text>) {
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
}

fn register_items(checker: &mut TypeChecker, ast: &Module, errors: &mut List<Text>) {
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Type(declaration) => checker.register_type_declaration(declaration),
            ItemKind::Impl(declaration) => checker.register_impl_block(declaration),
            ItemKind::Function(declaration) => checker.register_function_signature(declaration),
            _ => continue,
        };
        if let Err(error) = result {
            errors.push(format!("{error:?}").into());
        }
    }
}

fn check_items(checker: &mut TypeChecker, ast: &Module, errors: &mut List<Text>) {
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    collect_diagnostics(checker, errors);
}

fn local_errors(source: &str, two_pass_only: bool) -> List<Text> {
    let ast = parse(source, 0);
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("local");
    let mut errors = List::new();
    if two_pass_only {
        // Intentionally no ModuleRegistry and no single-shot registration.
        // Checking the Type items afterward would obscure this entry point.
        checker.register_all_type_names(&ast.items);
        for result in checker.resolve_all_type_definitions(&ast.items) {
            if let Err(error) = result {
                errors.push(format!("{error:?}").into());
            }
        }
        for item in &ast.items {
            if let ItemKind::Function(declaration) = &item.kind {
                if let Err(error) = checker.register_function_signature(declaration) {
                    errors.push(format!("{error:?}").into());
                }
            }
        }
        for item in &ast.items {
            if matches!(&item.kind, ItemKind::Function(_)) {
                if let Err(error) = checker.check_item(item) {
                    errors.push(format!("{error:?}").into());
                }
            }
        }
        collect_diagnostics(&mut checker, &mut errors);
    } else {
        register_items(&mut checker, &ast, &mut errors);
        check_items(&mut checker, &ast, &mut errors);
    }
    errors
}

fn source_checker(consumer: &str) -> (TypeChecker, List<Text>) {
    let mut registry = ModuleRegistry::new();
    for (index, (owner, source)) in [(OWNER, DECLARATIONS), ("consumer", consumer)]
        .into_iter()
        .enumerate()
    {
        let file = FileId::new(index as u32);
        let id = ModuleId::new(index as u32);
        let ast = parse(source, index as u32);
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), file, source.into());
        info.exports = extract_exports_from_module(&ast, id, &path).expect("fixture exports");
        registry.register(info);
    }
    let ast = parse(consumer, 1);
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("consumer");
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
    register_items(&mut checker, &ast, &mut errors);
    check_items(&mut checker, &ast, &mut errors);
    (checker, errors)
}

fn assert_private_hidden(errors: &List<Text>) {
    assert!(
        errors.iter().any(|error| {
            error.contains("VisibilityError")
                && error.contains("name: Text { inner: \"hidden\" }")
                && error.contains("module_path: Text { inner: \"core.field_compatibility\" }")
        }),
        "the exact declared private field must be refused, not an unknown projection: {errors:?}"
    );
}

#[test]
fn two_pass_local_private_record_construction_remains_valid() {
    let errors = local_errors(
        "type Packet is { hidden: Int }; fn make() -> Packet { Packet { hidden: 7 } }",
        true,
    );
    assert!(errors.is_empty(), "same-owner construction: {errors:?}");
}

#[test]
fn two_pass_local_private_record_read_remains_valid() {
    let errors = local_errors(
        "type Packet is { hidden: Int }; fn read(value: &Packet) -> Int { value.hidden }",
        true,
    );
    assert!(errors.is_empty(), "same-owner read: {errors:?}");
}

#[test]
fn ordinary_record_variant_construction_remains_valid() {
    let errors = local_errors(
        "type Choice is | Value { public number: Int } | Empty; fn make() -> Choice { Choice.Value { number: 7 } }",
        false,
    );
    assert!(errors.is_empty(), "ordinary record variant: {errors:?}");
}

#[test]
fn aliased_record_variant_construction_remains_valid() {
    let errors = local_errors(
        "type Choice is | Value { public number: Int } | Empty; type Alias is Choice; fn make() -> Choice { Alias.Value { number: 7 } }",
        false,
    );
    assert!(errors.is_empty(), "aliased record variant: {errors:?}");
}

#[test]
fn mounted_record_variant_construction_remains_valid() {
    let (_, errors) = source_checker(
        "mount core.field_compatibility.{Choice}; fn make() -> Choice { Choice.Value { number: 7 } }",
    );
    assert!(errors.is_empty(), "mounted record variant: {errors:?}");
}

#[test]
fn mount_alias_record_variant_construction_remains_valid() {
    let (_, errors) = source_checker(
        "mount core.field_compatibility.{Choice as Imported}; fn make() -> Imported { Imported.Value { number: 7 } }",
    );
    assert!(
        errors.is_empty(),
        "mounted alias record variant: {errors:?}"
    );
}

#[test]
fn record_variant_unknown_field_is_still_named_and_refused() {
    let errors = local_errors(
        "type Choice is | Value { public number: Int } | Empty; fn make() -> Choice { Choice.Value { absent_payload: 7 } }",
        false,
    );
    assert!(
        errors.iter().any(|error| error.contains("absent_payload")),
        "a constructor exception cannot accept an undeclared payload field: {errors:?}"
    );
}

const PROJECTED_SOURCE: &str = r#"
    mount core.field_compatibility.{Packet};
    type Supply is protocol { type Item; };
    type Producer is ();
    implement Supply for Producer { type Item = Packet; }
"#;

#[test]
fn parsed_associated_public_field_remains_valid() {
    let source =
        format!("{PROJECTED_SOURCE} fn probe(value: Producer.Item) -> Int {{ value.visible }}");
    let (_, errors) = source_checker(&source);
    assert!(errors.is_empty(), "parsed associated field: {errors:?}");
}

#[test]
fn parsed_associated_private_field_keeps_declaring_owner() {
    let source =
        format!("{PROJECTED_SOURCE} fn probe(value: Producer.Item) -> Int {{ value.hidden }}");
    let (_, errors) = source_checker(&source);
    assert_private_hidden(&errors);
}

fn late_projection_errors(field: &str) -> List<Text> {
    let (mut checker, mut errors) = source_checker(PROJECTED_SOURCE);
    assert!(
        errors.is_empty(),
        "source declaration/protocol setup: {errors:?}"
    );
    let producer = checker
        .context_mut()
        .lookup_type("consumer.Producer")
        .cloned()
        .expect("parsed Producer retains its declaring nominal key");
    // This is an explicit carrier-boundary control, not a claim that this
    // fixture's eager function signature retains a deferred projection.
    let projection = Type::Generic {
        name: "::Item".into(),
        args: [producer].into_iter().collect(),
    };
    checker.context_mut().env.insert(
        "value",
        TypeScheme::mono(Type::Reference {
            mutable: false,
            inner: Heap::new(projection),
        }),
    );
    let expression = FastParser::new()
        .parse_expr_str(&format!("value.{field}"), FileId::new(2))
        .expect("field expression grammar");
    match checker.synth_expr(&expression) {
        Ok(result) => assert_eq!(result.ty, Type::Int, "selected field type"),
        Err(error) => errors.push(format!("{error:?}").into()),
    }
    collect_diagnostics(&mut checker, &mut errors);
    errors
}

#[test]
fn late_associated_public_field_remains_valid() {
    let errors = late_projection_errors("visible");
    assert!(errors.is_empty(), "deferred associated field: {errors:?}");
}

#[test]
fn late_associated_private_field_keeps_declaring_owner() {
    assert_private_hidden(&late_projection_errors("hidden"));
}

#[test]
fn unconstrained_generic_cannot_read_a_private_record_field() {
    let (_, errors) = source_checker(
        r#"
            mount core.field_compatibility.{Packet};
            fn leak<T>(value: &T) -> Int { value.body }
            fn invoke(value: &Packet) -> Int { leak(value) }
        "#,
    );
    assert!(
        errors.iter().any(|error| {
            error.contains("name: Text { inner: \"body\" }")
                || error.contains("field: \"body\"")
                || error.contains("field 'body'")
        }),
        "the undeclared generic field access must be named and refused: {errors:?}"
    );
}

#[test]
fn declared_protocol_method_on_a_generic_receiver_remains_valid() {
    let errors = local_errors(
        r#"
            public type HasBody is protocol { fn body(&self) -> Int; };
            fn allowed<T: HasBody>(value: &T) -> Int { value.body() }
        "#,
        false,
    );
    assert!(errors.is_empty(), "declared generic method: {errors:?}");
}
