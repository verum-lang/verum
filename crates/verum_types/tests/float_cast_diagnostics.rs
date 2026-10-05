//! Floating-point narrowing has a different contract from integer truncation.
use verum_ast::ItemKind;
use verum_common::{List, Text};
use verum_diagnostics::Severity;
use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

fn warnings(source: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().expect("source parses");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    for item in &module.items {
        if let ItemKind::Type(declaration) = &item.kind {
            checker
                .register_type_declaration(declaration)
                .expect("type declaration");
        }
    }
    for item in &module.items {
        if let ItemKind::Function(function) = &item.kind {
            checker
                .register_function_signature(function)
                .expect("signature");
        }
    }
    for item in &module.items {
        checker.check_item(item).expect("source typechecks");
    }
    let diagnostics = checker.diagnostics();
    let errors: List<_> = diagnostics
        .iter()
        .filter(|d| d.severity() == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    diagnostics
        .iter()
        .filter(|d| d.severity() == Severity::Warning)
        .map(|d| Text::from(d.message()))
        .collect()
}

#[test]
fn float_narrowing_warns_about_precision_and_range_without_integer_rounding_help() {
    for (from, to) in [("Float", "Float32"), ("Float64", "Float32"), ("F64", "F32")] {
        let source = format!("fn narrow(x: {from}) -> {to} {{ x as {to} }}");
        let warnings = warnings(&source);
        assert_eq!(warnings.len(), 1, "{source}: {warnings:?}");
        assert!(
            warnings[0].contains("precision") && warnings[0].contains("range"),
            "{warnings:?}"
        );
        for wrong in [
            "truncates toward zero",
            ".trunc()",
            ".floor()",
            ".ceil()",
            ".round()",
        ] {
            assert!(!warnings[0].contains(wrong), "{warnings:?}");
        }
    }
}

#[test]
fn lossless_float_widening_and_same_width_aliases_do_not_warn() {
    for (from, to) in [
        ("Float32", "Float"),
        ("Float32", "Float64"),
        ("Float", "Float64"),
        ("Float64", "Float"),
    ] {
        let source = format!("fn widen(x: {from}) -> {to} {{ x as {to} }}");
        assert!(warnings(&source).is_empty(), "{source}");
    }
}

#[test]
fn float_to_integer_keeps_its_distinct_truncation_diagnostic() {
    for to in ["Int", "Int32"] {
        let source = format!("fn integer(x: Float) -> {to} {{ x as {to} }}");
        let warnings = warnings(&source);
        assert_eq!(warnings.len(), 1, "{source}: {warnings:?}");
        assert!(
            warnings[0].contains("truncates toward zero"),
            "{warnings:?}"
        );
        assert!(warnings[0].contains(".trunc()"), "{warnings:?}");
    }
}

#[test]
fn declared_float_alias_uses_the_resolved_narrowing_contract() {
    let warnings = warnings("type Small is Float32; fn narrow(x: Float) -> Small { x as Small }");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("precision") && warnings[0].contains("range"),
        "{warnings:?}"
    );
    assert!(
        !warnings[0].contains("truncates toward zero"),
        "{warnings:?}"
    );
}
