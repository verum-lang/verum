//! Explicit throws and residual propagation share one lexical error contract.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, TypeError};

#[derive(Debug)]
enum Diagnostic {
    Mismatch { expected: Text, actual: Text },
    Other(Text),
}

fn errors(source: &str) -> List<Diagnostic> {
    let module = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("throw_recovery");
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| match error {
            TypeError::Mismatch {
                expected, actual, ..
            } => Diagnostic::Mismatch {
                expected: expected.clone(),
                actual: actual.clone(),
            },
            other => Diagnostic::Other(format!("{other:?}").into()),
        })
        .collect()
}

fn assert_mismatch(errors: &[Diagnostic], expected: &str, actual: &str) {
    assert!(
        !errors.is_empty(),
        "the incompatible payload must be rejected"
    );
    for error in errors {
        match error {
            Diagnostic::Mismatch {
                expected: found_expected,
                actual: found_actual,
            } => {
                assert_eq!(found_expected.as_str(), expected);
                assert_eq!(found_actual.as_str(), actual);
            }
            Diagnostic::Other(detail) => panic!("unexpected diagnostic: {detail}"),
        }
    }
}

#[test]
fn recovery_receives_the_actual_thrown_type() {
    let found = errors("fn probe()->Int {try {throw 73} recover |error| {error + 0}}");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn recovery_cannot_reinterpret_a_thrown_payload() {
    let found =
        errors("fn probe()->Bool {try {throw 73} recover |error| {let wrong:Bool=error; wrong}}");
    assert_mismatch(&found, "Bool", "Int");
}

#[test]
fn nested_handlers_constrain_only_their_own_payloads() {
    let found = errors(
        r#"
        fn probe()->Int {
            try {
                let flag:Bool = try {throw true} recover |inner| {inner};
                if flag {throw 73}
                0
            } recover |outer| {outer + 0}
        }
    "#,
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_rethrow_carries_the_inner_payload_into_the_outer_contract() {
    for (result, body, accepts) in [
        ("Int", "outer + 0", true),
        ("Bool", "let wrong:Bool=outer; wrong", false),
    ] {
        let found = errors(&format!(
            "fn probe()->{result} {{try {{try {{throw 73}} recover |inner| {{throw inner}}}} recover |outer| {{{body}}}}}"
        ));
        if accepts {
            assert!(found.is_empty(), "{found:?}");
        } else {
            assert_mismatch(&found, "Bool", "Int");
        }
    }
}

#[test]
fn unrelated_throw_types_do_not_manufacture_an_implicit_sum() {
    let found = errors(
        "fn probe(flag:Bool)->Int {try {if flag {throw 73} else {throw true}} recover |_| {0}}",
    );
    assert_mismatch(&found, "Int", "Bool");
}

#[test]
fn a_declared_error_sum_keeps_its_distinct_alternatives() {
    let found = errors(
        r#"
        type Failure is Bad(Int) | Broken(Bool);
        fn probe(flag:Bool)->Int {
            try { if flag {throw Failure.Bad(73)} else {throw Failure.Broken(true)} }
            recover { Failure.Bad(value) => value, Failure.Broken(_) => 0 }
        }
    "#,
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_generic_error_payload_keeps_the_lexical_parameter_identity() {
    let found = errors("fn probe<T>(value:T)->T {try {throw value} recover |error| {error}}");
    assert!(found.is_empty(), "{found:?}");
}
