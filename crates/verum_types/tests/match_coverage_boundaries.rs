//! Conditional arms must not disable source match coverage checking.
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(declarations: &str, parameter: &str, expression: &str, checking: bool) -> Vec<String> {
    let function = if checking {
        format!("fn probe({parameter}) -> Int {{{expression}}}")
    } else {
        format!("fn probe({parameter}) {{let result = {expression};}}")
    };
    let source = format!("{declarations} {function}");
    let ast = Parser::new(&source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let declarations = checker.prepare_module_declarations(&ast);
    assert!(declarations.is_empty(), "{declarations:?}");
    let mut errors = Vec::new();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}"));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| format!("{error:?}")),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .map(|diagnostic| format!("{:?} {}", diagnostic.code(), diagnostic.message())),
    );
    errors
}

#[test]
fn guards_do_not_disable_boolean_or_variant_coverage() {
    for checking in [false, true] {
        for (declarations, parameter, expression) in [
            (
                "",
                "value: Bool",
                "match value {true if false => 1, false => 0}",
            ),
            (
                "type State is | Idle | Busy;",
                "value: State",
                "match value {State.Idle if false => 1, State.Busy => 0}",
            ),
        ] {
            let errors = errors(declarations, parameter, expression, checking);
            assert!(
                errors.iter().any(|error| error.contains("E0601")),
                "checking={checking}: {errors:?}"
            );
        }
    }
}

#[test]
fn integer_patterns_require_coverage_of_remaining_values() {
    for checking in [false, true] {
        for expression in [
            "match value {0 => 5, 1 => 99}",
            "match value {0 => 5, _ if false => 99}",
        ] {
            let errors = errors("", "value: Int", expression, checking);
            assert!(
                errors.iter().any(|error| error.contains("E0601")),
                "checking={checking}: {errors:?}"
            );
        }
    }
}

#[test]
fn unproved_guards_are_not_reported_as_concrete_counterexamples() {
    for checking in [false, true] {
        let errors = errors(
            "",
            "value: Int",
            "match value {n if n < 0 => 1, n if n >= 0 => 2}",
            checking,
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("coverage") && error.contains("not established")),
            "{errors:?}"
        );
        assert!(
            errors.iter().all(|error| !error.contains("not covered")),
            "{errors:?}"
        );
    }
}

#[test]
fn unconditional_coverage_remains_valid_with_conditional_arms() {
    for checking in [false, true] {
        for (declarations, parameter, expression) in [
            (
                "",
                "value: Bool",
                "match value {true if value => 1, true => 2, false => 0}",
            ),
            ("", "value: Int", "match value {0 if false => 1, _ => 2}"),
            ("", "value: Int", "match value {0 => 1, _ => 2}"),
            (
                "type State is | Idle | Busy;",
                "value: State",
                "match value {State.Idle if false => 1, State.Idle => 2, State.Busy => 0}",
            ),
        ] {
            let errors = errors(declarations, parameter, expression, checking);
            assert!(
                errors.is_empty(),
                "checking={checking}: {expression}: {errors:?}"
            );
        }
    }
}
