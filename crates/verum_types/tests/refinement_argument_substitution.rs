//! Caller names cannot change a dependent contract's verification obligation.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn diagnostics(source: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let mut found: List<Text> = checker
        .prepare_module_declarations(&ast)
        .into_iter()
        .map(|error| format!("{error:?}").into())
        .collect();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            found.push(format!("{error:?}").into());
        }
    }
    found.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| format!("{error:?}").into()),
    );
    found.extend(
        checker
            .diagnostics()
            .iter()
            .map(|diagnostic| format!("{diagnostic:?}").into()),
    );
    found
}

#[test]
fn caller_renaming_cannot_discharge_a_dependent_obligation() {
    for name in ["outside", "b", "a"] {
        let source = format!(
            r#"
fn need(a: Int, b: Int, c: Int {{self == a}}) {{}}
fn caller({name}: Int) {{ need({name}, 0, 0); }}
"#
        );
        let found = diagnostics(&source);
        assert!(
            found
                .iter()
                .any(|message| message.contains("W0500") || message.contains("E500")),
            "a caller with unconstrained {name} cannot establish 0 == {name}: {found:?}"
        );
    }
}

#[test]
fn established_constant_arguments_remain_valid() {
    let found =
        diagnostics("fn need(a: Int, b: Int, c: Int {self == a}) {} fn caller() {need(7, 0, 7);}");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_transport_failure_is_not_discarded_by_argument_checking() {
    let found = diagnostics(
        r#"
fn need(value: Int { {fn nested() -> Int {1} nested() == it} }) {}
fn caller() {need(1);}
"#,
    );
    assert!(
        found
            .iter()
            .any(|message| message.contains("local declaration") && message.contains("E500")),
        "{found:?}"
    );
}
