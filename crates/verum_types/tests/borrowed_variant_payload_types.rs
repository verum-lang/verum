//! T1746: qualified borrowed patterns retain their instantiated payload types.
//! This checker reduction does not establish the cause of the runtime allocation panic.
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

const DECLARATIONS: &str = r#"
type Cell is { padding: Int, value: Int };
type Flag is { enabled: Bool };
type Choice<Payload> is Absent | Value(Payload);
"#;

fn check(body: &str) -> List<Text> {
    let source = format!("{DECLARATIONS}\n{body}");
    let module = FastParser::new()
        .parse_module_str(&source, FileId::new(0))
        .expect("production parser accepts fixture grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("probe");
    for item in &module.items {
        match &item.kind {
            ItemKind::Type(declaration) => checker
                .register_type_declaration(declaration)
                .expect("local type setup"),
            ItemKind::Function(declaration) => checker
                .register_function_signature(declaration)
                .expect("function signature setup"),
            _ => {}
        }
    }
    let mut errors = List::new();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(Text::from(format!("{error:?}")));
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

fn accepted(body: &str) {
    let errors = check(body);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn owned_qualified_payload_preserves_its_record_type() {
    accepted(
        r#"
fn inspect(value: Choice<Cell>) -> Int {
    match value { Choice.Value(cell) => cell.padding, _ => -1 }
}
"#,
    );
}

#[test]
fn shared_unqualified_payload_preserves_its_record_type() {
    accepted(
        r#"
fn inspect(value: &Choice<Cell>) -> Int {
    match value { Value(cell) => cell.padding, _ => -1 }
}
"#,
    );
}

#[test]
fn shared_qualified_payload_preserves_its_record_type() {
    accepted(
        r#"
fn inspect(value: &Choice<Cell>) -> Int {
    match value { Choice.Value(cell) => cell.padding, _ => -1 }
}
"#,
    );
}

#[test]
fn mutable_qualified_payload_preserves_its_record_type() {
    accepted(
        r#"
fn inspect(value: &mut Choice<Cell>) -> Int {
    match value { Choice.Value(cell) => cell.padding, _ => -1 }
}
"#,
    );
}

#[test]
fn checked_qualified_payload_preserves_its_record_type() {
    accepted(
        r#"
fn inspect(value: &checked Choice<Cell>) -> Int {
    match value { Choice.Value(cell) => cell.padding, _ => -1 }
}
"#,
    );
}

#[test]
fn unsafe_qualified_payload_preserves_its_record_type() {
    accepted(
        r#"
fn inspect(value: &unsafe Choice<Cell>) -> Int {
    match value { Choice.Value(cell) => cell.padding, _ => -1 }
}
"#,
    );
}

#[test]
fn nested_variant_then_borrowed_optional_payload_preserves_its_record_type() {
    accepted(
        r#"
type Options is { version: Int, features: Choice<Cell> };
type Dependency is Requirement(Int) | Detailed(Options);
fn inspect(value: Choice<Dependency>) -> Int {
    match value {
        Choice.Value(Dependency.Detailed(options)) => {
            match &options.features {
                Choice.Value(cell) => options.version * 10000 + cell.padding * 100 + cell.value,
                _ => -2,
            }
        },
        _ => -1,
    }
}
"#,
    );
}

#[test]
fn borrowed_instances_keep_different_payload_types_in_one_callable() {
    accepted(
        r#"
fn inspect(number: &Choice<Cell>, flag: &Choice<Flag>) -> Int {
    let count = match number { Choice.Value(cell) => cell.padding, _ => -1 };
    let enabled = match flag { Choice.Value(item) => item.enabled, _ => false };
    if enabled { count } else { 0 }
}
"#,
    );
}

#[test]
fn borrowed_payload_retains_its_declared_parameter_position() {
    accepted(
        r#"
type Reordered<Phantom, Payload> is Absent | Value(Payload);
fn inspect(value: &Reordered<Text, Cell>) -> Int {
    match value { Reordered.Value(cell) => cell.padding, _ => -1 }
}
"#,
    );
}

#[test]
fn borrowed_record_field_does_not_acquire_the_callers_expected_type() {
    let errors = check(
        r#"
fn inspect(value: &Choice<Cell>) -> Bool {
    match value { Choice.Value(cell) => cell.padding, _ => false }
}
"#,
    );
    assert!(
        errors.iter().any(|error| error.contains("Mismatch")
            && error.contains("Int")
            && error.contains("Bool")),
        "{errors:?}"
    );
    assert!(
        !errors.iter().any(|error| error.contains("unresolved")),
        "{errors:?}"
    );
}
