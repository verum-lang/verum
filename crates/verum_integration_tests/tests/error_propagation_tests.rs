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
#![cfg(test)]

// Error Handling Integration Tests
//
// Tests error propagation through the compilation pipeline,
// diagnostic quality, error recovery, and error continuation.

// NOTE: These tests require full compiler pipeline infrastructure.
// They are kept as smoke tests to verify basic imports work.

use verum_diagnostics::DiagnosticBuilder;

// ============================================================================
// Diagnostic Quality Tests
// ============================================================================

#[test]
fn test_diagnostic_builder_basic() {
    let diagnostic = DiagnosticBuilder::error()
        .message("Type mismatch")
        .label("expected Int, found Bool")
        .build();

    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.message(), "Type mismatch");
}

#[test]
fn test_diagnostic_builder_with_help() {
    let diagnostic = DiagnosticBuilder::warning()
        .message("Unused variable")
        .label("variable 'x' is never read")
        .help("remove this variable or prefix with underscore: `_x`")
        .build();

    assert!(diagnostic.is_warning());
}

// All other tests are omitted as they require full compiler infrastructure.
// Tests can be added as the pipeline is implemented.
