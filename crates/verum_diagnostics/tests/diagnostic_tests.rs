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
// Unit tests for diagnostic.rs
//
// Migrated from src/diagnostic.rs to comply with CLAUDE.md test organization.

use verum_diagnostics::diagnostic::*;

#[test]
fn test_span_creation() {
    let span = Span::new("test.vr", 10, 5, 12);
    assert_eq!(span.file, "test.vr");
    assert_eq!(span.line, 10);
    assert_eq!(span.column, 5);
    assert_eq!(span.end_column, 12);
    assert_eq!(span.length(), 7);
    assert!(!span.is_multiline());
}

#[test]
fn test_multiline_span() {
    let span = Span::new_multiline("test.vr", 5, 10, 8, 15);
    assert!(span.is_multiline());
}

#[test]
fn test_diagnostic_builder() {
    let diag = DiagnosticBuilder::error()
        .code("E0308")
        .message("verification failed")
        .span(Span::new("main.vr", 10, 5, 12))
        .label("cannot prove postcondition")
        .add_note("SMT solver found counterexample")
        .help("add precondition")
        .build();

    assert_eq!(diag.severity(), Severity::Error);
    assert_eq!(diag.code(), Some("E0308"));
    assert_eq!(diag.message(), "verification failed");
    assert_eq!(diag.primary_labels().len(), 1);
    assert_eq!(diag.notes().len(), 1);
    assert_eq!(diag.helps().len(), 1);
}

#[test]
fn test_severity_ordering() {
    // Severity is ordered Error > Warning > Note > Help
    // but the enum order is Help < Note < Warning < Error
    assert!(Severity::Error >= Severity::Warning);
    assert!(Severity::Warning >= Severity::Note);
    assert!(Severity::Note >= Severity::Help);
}
