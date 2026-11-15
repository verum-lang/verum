//! Standard-library placement cannot change the validity of a source program.
use std::path::Path;
use verum_common::{List, Text};
use verum_compiler::{CompilationPipeline, CompilerOptions, Session, VerifyMode};
use verum_diagnostics::Severity;

fn check(path: &Path, source: &str) -> List<(Text, Text)> {
    std::fs::write(path, source).unwrap();
    let mut session = Session::new(CompilerOptions {
        input: path.to_owned(),
        verify_mode: VerifyMode::Runtime,
        ..Default::default()
    });
    let outcome = CompilationPipeline::new_check(&mut session).run_check_only();
    let mut errors: List<_> = session
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.severity() == Severity::Error)
        .map(|diagnostic| {
            (
                Text::from(diagnostic.code().unwrap_or("")),
                Text::from(diagnostic.message()),
            )
        })
        .collect();
    assert_eq!(
        outcome.is_ok(),
        errors.is_empty(),
        "{outcome:?}: {errors:?}"
    );
    errors.sort();
    errors
}

fn compare_paths(source: &str, expected_error: Option<&str>) {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    std::fs::create_dir(&core).unwrap();
    // Exercise the old anchor, including its marker. A directory called core
    // without mod.vr would never have selected the unsound checking mode.
    std::fs::write(core.join("mod.vr"), "module core;").unwrap();
    let source = format!("module example.contract;\n{source}");
    let plain = check(&directory.path().join("subject.vr"), &source);
    let anchored = check(&core.join("subject.vr"), &source);
    assert_eq!(plain, anchored, "source location changed the verdict");
    if let Some(expected) = expected_error {
        assert!(
            plain.iter().any(|(_, message)| message.contains(expected)),
            "missing {expected:?}: {plain:?}"
        );
    } else {
        assert!(plain.is_empty(), "valid source rejected: {plain:?}");
    }
}

#[test]
fn return_type_mismatch_is_not_discarded() {
    compare_paths("fn probe() -> Int { true }", Some("Type mismatch"));
}

#[test]
fn missing_method_is_not_an_unknown_result() {
    compare_paths(
        "type Token is { value: Int }; fn probe(token: Token) -> Int { token.absent() }",
        Some("absent"),
    );
}

#[test]
fn unsupported_unary_operators_are_not_unknown_results() {
    for operator in ["!", "-"] {
        compare_paths(
            &format!(
                "type Token is {{ value: Int }}; fn probe(token: Token) -> Int {{ {operator}token }}"
            ),
            Some(if operator == "!" { "NOT" } else { "negate" }),
        );
    }
}

#[test]
fn variant_fields_must_match_the_declaration() {
    for (initializer, expected) in [("{ other: 1 }", "other"), ("{}", "value")] {
        compare_paths(
            &format!(
                "type Packet is Empty | Payload {{ value: Int }}; fn probe() -> Packet {{ Packet.Payload {initializer} }}"
            ),
            Some(expected),
        );
    }
}

#[test]
fn invalid_call_is_not_an_unknown_result() {
    compare_paths("fn probe() -> Int { 42() }", Some("function"));
}

#[test]
fn known_function_arity_is_enforced() {
    compare_paths(
        "fn identity<T>(value: T) -> T { value } fn probe() -> Int { identity<Int>() }",
        Some("argument"),
    );
}

#[test]
fn missing_import_is_rejected_at_the_mount() {
    compare_paths(
        "module library { public type Token is { value: Int }; } mount library.Missing; fn probe() -> Int { 1 }",
        Some("Missing"),
    );
}

#[test]
fn valid_generic_calls_and_variant_fields_remain_valid() {
    compare_paths(
        "type Packet is Empty | Payload { value: Int }; fn identity<T>(value: T) -> T { value } fn probe() -> Packet { Packet.Payload { value: identity<Int>(7) } }",
        None,
    );
}
