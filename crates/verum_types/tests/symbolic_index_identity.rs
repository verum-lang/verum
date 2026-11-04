//! Value-index equality is stronger than equality of the scalar value sort.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(tail: &str) -> List<Text> {
    let source = format!("module value_indices; type Cell<N: meta Int> is {{ value: () }}; {tail}");
    let module = Parser::new(&source).parse_module().expect("source parses");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("value_indices");
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&module)
        .iter()
        .map(|error| Text::from(format!("{error:?}")))
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|e| e.is_error())
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors
}

#[test]
fn adding_one_does_not_preserve_the_value_index() {
    let errors = errors("fn wrong<N: meta Int>(cell: Cell<N>) -> Cell<N + 1> { cell }");
    assert!(
        errors.iter().any(
            |e| e.contains("generic value expression has no represented compile-time identity")
        ),
        "{errors:?}"
    );
}

#[test]
fn subtracting_one_does_not_preserve_the_value_index() {
    let errors = errors("fn wrong<N: meta Int>(cell: Cell<N>) -> Cell<N - 1> { cell }");
    assert!(
        errors.iter().any(
            |e| e.contains("generic value expression has no represented compile-time identity")
        ),
        "{errors:?}"
    );
}

#[test]
fn same_symbolic_index_is_preserved() {
    let errors = errors("fn same<N: meta Int>(cell: Cell<N>) -> Cell<N> { cell }");
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn unrelated_symbolic_binders_cannot_be_identified() {
    assert!(
        !errors("fn wrong<N: meta Int, M: meta Int>(cell: Cell<N>) -> Cell<M> { cell }").is_empty()
    );
}

#[test]
fn concrete_value_indices_are_distinct() {
    assert!(!errors("fn wrong(cell: Cell<0>) -> Cell<1> { cell }").is_empty());
}

#[test]
fn evaluated_constant_arithmetic_preserves_the_actual_value() {
    let errors = errors("fn same(cell: Cell<1 + 1>) -> Cell<2> { cell }");
    assert!(errors.is_empty(), "{errors:?}");
}
