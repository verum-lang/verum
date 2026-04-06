#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Unit tests for lib.rs
//
// Migrated from src/lib.rs to comply with CLAUDE.md test organization.

use verum_diagnostics::{DiagnosticBuilder, RefinementErrorBuilder, Severity, Span, codes};

#[test]
fn test_basic_diagnostic() {
    let diagnostic = DiagnosticBuilder::error()
        .code(codes::E0312)
        .message("refinement constraint not satisfied")
        .span(Span::new("main.vr", 3, 12, 13))
        .label("value `-5` fails constraint `i > 0`")
        .build();

    assert_eq!(diagnostic.severity(), Severity::Error);
    assert_eq!(diagnostic.code(), Some(codes::E0312));
}

#[test]
fn test_refinement_error() {
    let error = RefinementErrorBuilder::new()
        .constraint("i > 0")
        .actual_value("-5")
        .span(Span::new("main.vr", 3, 12, 13))
        .suggestion("wrap in runtime check: `PositiveInt::try_from(x)?`")
        .suggestion("or use compile-time proof: `@verify x > 0`")
        .build();

    assert!(error.constraint().expression.contains(">"));
}
