//! T1713 field-guard ordering controls for type and value name shadowing.
//! A locally bound value named i32 must not take a type-property shortcut.
//! Parsed module and declaration setup reuses the production-parser harness.

use verum_ast::{FileId, ItemKind, Module};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;

const OWNER: &str = "core.field_shadow";
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

fn check(owner_source: &str, probe: &str) -> List<Text> {
    let source = format!("mount {OWNER}.{{Packet}}; {probe}");
    let mut registry = ModuleRegistry::new();
    for (index, (owner, text)) in [(OWNER, owner_source), ("consumer", source.as_str())]
        .into_iter()
        .enumerate()
    {
        let file = FileId::new(index as u32);
        let id = ModuleId::new(index as u32);
        let ast = parse(text, index as u32);
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), file, text.into());
        info.exports = extract_exports_from_module(&ast, id, &path).expect("fixture exports");
        registry.register(info);
    }
    let ast = parse(&source, 1);
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry.clone());
    checker.register_stdlib_types_for_module(&ast);
    let mut setup_errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "consumer", &registry) {
                setup_errors.push(Text::from(format!("{error:?}")));
            }
        }
    }
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Type(declaration) => checker.register_type_declaration(declaration),
            ItemKind::Impl(declaration) => checker.register_impl_block(declaration),
            ItemKind::Function(declaration) => checker.register_function_signature(declaration),
            _ => continue,
        };
        if let Err(error) = result {
            setup_errors.push(Text::from(format!("{error:?}")));
        }
    }
    for item in &ast.items {
        if matches!(&item.kind, ItemKind::Function(declaration) if declaration.name.name.as_str() == "probe")
        {
            continue;
        }
        if let Err(error) = checker.check_item(item) {
            setup_errors.push(Text::from(format!("{error:?}")));
        }
    }
    collect_diagnostics(&mut checker, &mut setup_errors);
    assert!(
        setup_errors.is_empty(),
        "the declared protocol/impl/owner setup must pass independently: {setup_errors:?}"
    );
    let probe = ast
        .items
        .iter()
        .find(|item| matches!(&item.kind, ItemKind::Function(declaration) if declaration.name.name.as_str() == "probe"))
        .expect("exactly named probe body");
    let mut errors = List::new();
    if let Err(error) = checker.check_item(probe) {
        errors.push(Text::from(format!("{error:?}")));
    }
    collect_diagnostics(&mut checker, &mut errors);
    errors
}

fn assert_private(probe: &str) {
    let errors = check("public type Packet is { min: Int };", probe);
    assert_private_errors(&errors);
}

fn assert_private_errors(errors: &List<Text>) {
    assert!(
        !errors.is_empty()
            && errors.iter().all(|error| {
                error.contains("VisibilityError")
                    && error.contains("name: Text { inner: \"min\" }")
                    && error.contains("visibility: Text { inner: \"Private\" }")
                    && error.contains("module_path: Text { inner: \"core.field_shadow\" }")
            }),
        "the actual value's private field must be refused: {errors:?}"
    );
}

#[test]
fn ordinary_record_private_min_field_is_refused() {
    assert_private("fn probe(value: &Packet) -> Int { value.min }");
}

#[test]
fn primitive_spelling_parameter_keeps_private_field_policy() {
    assert_private("fn probe(i32: &Packet) -> Int { i32.min }");
}

#[test]
fn ordinary_record_public_min_field_keeps_its_bool_type() {
    let errors = check(
        "public type Packet is { public min: Bool };",
        "fn probe(value: &Packet) -> Bool { value.min }",
    );
    assert!(errors.is_empty(), "ordinary public Bool field: {errors:?}");
}

#[test]
fn primitive_spelling_parameter_keeps_public_field_type() {
    let errors = check(
        "public type Packet is { public min: Bool };",
        "fn probe(i32: &Packet) -> Bool { i32.min }",
    );
    assert!(
        errors.is_empty(),
        "the value's Bool field outranks a type property: {errors:?}"
    );
}

#[test]
fn static_type_spelling_parameter_keeps_private_field_policy() {
    assert_private(
        "type Metrics is (); implement Metrics { const min: Int = 23; } fn probe(Metrics: &Packet) -> Int { Metrics.min }",
    );
}

#[test]
fn static_type_spelling_parameter_keeps_public_field_type() {
    let errors = check(
        "public type Packet is { public min: Bool };",
        "type Metrics is (); implement Metrics { const min: Int = 23; } fn probe(Metrics: &Packet) -> Bool { Metrics.min }",
    );
    assert!(
        errors.is_empty(),
        "a parameter's field outranks a same-named type: {errors:?}"
    );
}

#[test]
fn unshadowed_primitive_properties_remain_available() {
    for expression in ["i32.min", "u64.max", "f32.bits"] {
        let errors = check(
            "public type Packet is { min: Int };",
            &format!("fn probe() -> Int {{ {expression} }}"),
        );
        assert!(errors.is_empty(), "{expression}: {errors:?}");
    }
}

#[test]
fn unshadowed_static_property_remains_available() {
    let errors = check(
        "public type Packet is { min: Int };",
        "type Metrics is (); implement Metrics { const LIMIT: Int = 23; } fn probe() -> Int { Metrics.LIMIT }",
    );
    assert!(
        errors.is_empty(),
        "unshadowed associated constant: {errors:?}"
    );
}

#[test]
fn unshadowed_variant_constructor_remains_available() {
    let errors = check(
        "public type Packet is { min: Int };",
        "type Switch is On | Off; fn probe() -> Switch { Switch.On }",
    );
    assert!(
        errors.is_empty(),
        "unshadowed variant constructor: {errors:?}"
    );
}

#[test]
fn ordinary_imported_constant_keeps_private_field_policy() {
    let errors = check(
        "public type Packet is { min: Int }; public const PACKET: Packet = Packet { min: 23 };",
        "mount core.field_shadow.PACKET; fn probe() -> Int { PACKET.min }",
    );
    assert_private_errors(&errors);
}

#[test]
fn primitive_spelling_imported_constant_keeps_private_field_policy() {
    let errors = check(
        "public type Packet is { min: Int }; public const i32: Packet = Packet { min: 23 };",
        "mount core.field_shadow.i32; fn probe() -> Int { i32.min }",
    );
    assert_private_errors(&errors);
}

#[test]
fn ordinary_imported_constant_keeps_public_field_type() {
    let errors = check(
        "public type Packet is { public min: Bool }; public const PACKET: Packet = Packet { min: true };",
        "mount core.field_shadow.PACKET; fn probe() -> Bool { PACKET.min }",
    );
    assert!(
        errors.is_empty(),
        "the imported constant's public Bool field: {errors:?}"
    );
}

#[test]
fn primitive_spelling_imported_constant_keeps_public_field_type() {
    let errors = check(
        "public type Packet is { public min: Bool }; public const i32: Packet = Packet { min: true };",
        "mount core.field_shadow.i32; fn probe() -> Bool { i32.min }",
    );
    assert!(
        errors.is_empty(),
        "the imported constant outranks a primitive type property: {errors:?}"
    );
}
