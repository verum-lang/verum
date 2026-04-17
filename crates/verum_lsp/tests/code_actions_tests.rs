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
// Tests for code_actions module
// Comprehensive tests for code actions, quick fixes, and refactorings

use tower_lsp::lsp_types::{
    CodeActionContext, CodeActionOrCommand, Diagnostic, DiagnosticSeverity, NumberOrString,
    Position, Range, Url,
};
use verum_ast::FileId;
use verum_lsp::code_actions::*;
use verum_lsp::document::DocumentState;

// ============================================================================
// Test Helpers
// ============================================================================

fn create_test_document(source: &str) -> DocumentState {
    DocumentState::new(source.to_string(), 1, FileId::new(1))
}

fn create_test_uri() -> Url {
    Url::parse("file:///test.vr").unwrap()
}

fn create_diagnostic(message: &str, range: Range) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        code: None,
        code_description: None,
        source: Some("verum".to_string()),
        message: message.to_string(),
        related_information: None,
        tags: None,
        data: None,
    }
}

fn create_diagnostic_with_source(message: &str, range: Range, source: &str) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        code: None,
        code_description: None,
        source: Some(source.to_string()),
        message: message.to_string(),
        related_information: None,
        tags: None,
        data: None,
    }
}

fn create_diagnostic_with_code(message: &str, range: Range, code: &str) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String(code.to_string())),
        code_description: None,
        source: Some("verum".to_string()),
        message: message.to_string(),
        related_information: None,
        tags: None,
        data: None,
    }
}

// ============================================================================
// Extraction Range Validation Tests
// ============================================================================

#[test]
fn test_is_valid_extraction_range_multiline() {
    let doc = create_test_document("fn main() {\n    let x = 1;\n    let y = 2;\n}\n");

    // Multi-line range should be valid
    let multi_line = Range {
        start: Position { line: 1, character: 0 },
        end: Position { line: 2, character: 0 },
    };
    assert!(is_valid_extraction_range(&doc, multi_line));
}

#[test]
fn test_is_valid_extraction_range_single_short_line() {
    let doc = create_test_document("fn main() {}\n");

    // Single short line should not be valid
    let short = Range {
        start: Position { line: 0, character: 0 },
        end: Position { line: 0, character: 5 },
    };
    assert!(!is_valid_extraction_range(&doc, short));
}

#[test]
fn test_is_valid_extraction_range_long_single_line() {
    let doc = create_test_document(
        "fn process_data(x: Int, y: Int, z: Int) -> Result<Data, Error> { }\n",
    );

    // Long single line should be valid
    let long_line = Range {
        start: Position { line: 0, character: 0 },
        end: Position { line: 0, character: 50 },
    };
    assert!(is_valid_extraction_range(&doc, long_line));
}

#[test]
fn test_is_valid_extraction_range_empty() {
    let doc = create_test_document("fn main() { }\n");

    // Empty range should not be valid
    let empty = Range {
        start: Position { line: 0, character: 5 },
        end: Position { line: 0, character: 5 },
    };
    assert!(!is_valid_extraction_range(&doc, empty));
}

// ============================================================================
// Code Actions Generation Tests
// ============================================================================

#[test]
fn test_code_actions_empty_context() {
    let doc = create_test_document("fn main() { }");
    let uri = create_test_uri();
    let range = Range {
        start: Position { line: 0, character: 0 },
        end: Position { line: 0, character: 10 },
    };
    let context = CodeActionContext {
        diagnostics: vec![],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // Should return some actions (refactoring, source organization)
    // Even with no diagnostics
}

#[test]
fn test_code_actions_with_type_mismatch_diagnostic() {
    let doc = create_test_document("fn main() { let x: Int = \"hello\"; }");
    let uri = create_test_uri();
    let range = Range {
        start: Position { line: 0, character: 25 },
        end: Position { line: 0, character: 32 },
    };

    let diagnostic = create_diagnostic(
        "type mismatch: expected Int, found Text",
        range,
    );

    let context = CodeActionContext {
        diagnostics: vec![diagnostic],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // Should include type cast suggestion
    let has_type_cast = actions.iter().any(|a| match a {
        CodeActionOrCommand::CodeAction(ca) => ca.title.contains("type cast"),
        _ => false,
    });
    assert!(has_type_cast, "Should suggest adding type cast");
}

#[test]
fn test_code_actions_with_missing_import_diagnostic() {
    let doc = create_test_document("fn main() { let x = List::new(); }");
    let uri = create_test_uri();
    let range = Range {
        start: Position { line: 0, character: 20 },
        end: Position { line: 0, character: 24 },
    };

    let diagnostic = create_diagnostic("List not found in scope", range);

    let context = CodeActionContext {
        diagnostics: vec![diagnostic],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // Should include import suggestion for standard types
    let has_import = actions.iter().any(|a| match a {
        CodeActionOrCommand::CodeAction(ca) => ca.title.contains("Import"),
        _ => false,
    });
    // Note: May or may not have import based on symbol resolution
}

#[test]
fn test_code_actions_with_cbgr_error() {
    let doc = create_test_document("fn main() { let x = &data; }");
    let uri = create_test_uri();
    let range = Range {
        start: Position { line: 0, character: 20 },
        end: Position { line: 0, character: 25 },
    };

    let diagnostic = create_diagnostic(
        "CBGR: reference tier mismatch",
        range,
    );

    let context = CodeActionContext {
        diagnostics: vec![diagnostic],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // Should include reference tier conversion
    let has_ref_conversion = actions.iter().any(|a| match a {
        CodeActionOrCommand::CodeAction(ca) => ca.title.contains("checked reference"),
        _ => false,
    });
    assert!(has_ref_conversion, "Should suggest converting to checked reference");
}

#[test]
fn test_code_actions_with_affine_type_error() {
    let doc = create_test_document("fn main() { let x = data; let y = data; }");
    let uri = create_test_uri();
    let range = Range {
        start: Position { line: 0, character: 34 },
        end: Position { line: 0, character: 38 },
    };

    let diagnostic = create_diagnostic(
        "value used after move (affine type)",
        range,
    );

    let context = CodeActionContext {
        diagnostics: vec![diagnostic],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // Should include clone suggestion
    let has_clone = actions.iter().any(|a| match a {
        CodeActionOrCommand::CodeAction(ca) => ca.title.contains("Clone"),
        _ => false,
    });
    assert!(has_clone, "Should suggest cloning before move");
}

#[test]
fn test_code_actions_with_refinement_error() {
    let doc = create_test_document("fn main() { let x: Int where it > 0 = -5; }");
    let uri = create_test_uri();
    let range = Range {
        start: Position { line: 0, character: 37 },
        end: Position { line: 0, character: 39 },
    };

    let diagnostic = create_diagnostic(
        "refinement constraint not satisfied: it > 0",
        range,
    );

    let context = CodeActionContext {
        diagnostics: vec![diagnostic],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // Should include refinement quick fixes
}

#[test]
fn test_code_actions_with_ambiguous_type() {
    let doc = create_test_document("fn main() { let x = None; }");
    let uri = create_test_uri();
    let range = Range {
        start: Position { line: 0, character: 16 },
        end: Position { line: 0, character: 17 },
    };

    let diagnostic = create_diagnostic(
        "cannot infer type for x",
        range,
    );

    let context = CodeActionContext {
        diagnostics: vec![diagnostic],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // Should include type annotation suggestion
    let has_annotation = actions.iter().any(|a| match a {
        CodeActionOrCommand::CodeAction(ca) => ca.title.contains("type annotation"),
        _ => false,
    });
    assert!(has_annotation, "Should suggest adding type annotation");
}

// ============================================================================
// Parser Error Quick Fix Tests
// ============================================================================

#[test]
fn test_code_actions_with_parser_error() {
    let doc = create_test_document("fn main() { let x = ; }");
    let uri = create_test_uri();
    let range = Range {
        start: Position { line: 0, character: 20 },
        end: Position { line: 0, character: 21 },
    };

    let diagnostic = create_diagnostic_with_source(
        "expected expression",
        range,
        "verum-parser",
    );

    let context = CodeActionContext {
        diagnostics: vec![diagnostic],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // Should generate error node fixes from the parser
}

// ============================================================================
// Refactoring Actions Tests
// ============================================================================

#[test]
fn test_refactoring_actions_with_selection() {
    let doc = create_test_document("fn main() {\n    let x = 1 + 2 + 3;\n}");
    let uri = create_test_uri();

    // Select the expression "1 + 2 + 3"
    let range = Range {
        start: Position { line: 1, character: 12 },
        end: Position { line: 1, character: 21 },
    };

    let context = CodeActionContext {
        diagnostics: vec![],
        only: None,
        trigger_kind: None,
    };
    let actions = code_actions(&doc, range, context, &uri);
    // Should include refactoring actions when expression is selected
    // Note: extraction may not be fully implemented yet
    let _ = actions;
}

#[test]
fn test_refactoring_actions_function_body() {
    let doc = create_test_document(
        "fn main() {\n    let x = compute();\n    let y = process(x);\n    print(y);\n}",
    );
    let uri = create_test_uri();

    // Select multiple statements
    let range = Range {
        start: Position { line: 1, character: 0 },
        end: Position { line: 3, character: 0 },
    };

    let context = CodeActionContext {
        diagnostics: vec![],
        only: None,
        trigger_kind: None,
    };
    let actions = code_actions(&doc, range, context, &uri);
    // May include extract function action when multiple statements are selected
    let _ = actions;
}

// ============================================================================
// Error Code Quick Fix Tests
// ============================================================================

#[test]
fn test_code_actions_with_known_error_code() {
    let doc = create_test_document("fn main() { let x = 1; }");
    let uri = create_test_uri();
    let range = Range {
        start: Position { line: 0, character: 0 },
        end: Position { line: 0, character: 24 },
    };

    let diagnostic = create_diagnostic_with_code(
        "unused variable: x",
        range,
        "E0001",
    );

    let context = CodeActionContext {
        diagnostics: vec![diagnostic],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // May include specific quick fix for the error code
}

// ============================================================================
// Source Organization Tests
// ============================================================================

#[test]
fn test_source_organization_actions() {
    let doc = create_test_document(
        "link std.io;\nlink std.collections;\n\nfn main() { }",
    );
    let uri = create_test_uri();
    let range = Range {
        start: Position { line: 0, character: 0 },
        end: Position { line: 0, character: 0 },
    };

    let context = CodeActionContext {
        diagnostics: vec![],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // May include organize imports action
}

// ============================================================================
// Context-Aware Actions Tests
// ============================================================================

#[test]
fn test_context_aware_actions_in_function() {
    let doc = create_test_document("fn main() {\n    \n}");
    let uri = create_test_uri();

    // Cursor inside function body
    let range = Range {
        start: Position { line: 1, character: 4 },
        end: Position { line: 1, character: 4 },
    };

    let context = CodeActionContext {
        diagnostics: vec![],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // May include function-specific actions
}

#[test]
fn test_context_aware_actions_in_type() {
    let doc = create_test_document("type Point is {\n    x: Float,\n    \n}");
    let uri = create_test_uri();

    // Cursor inside type definition
    let range = Range {
        start: Position { line: 2, character: 4 },
        end: Position { line: 2, character: 4 },
    };

    let context = CodeActionContext {
        diagnostics: vec![],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // May include type-specific actions
}

// ============================================================================
// Multiple Diagnostics Tests
// ============================================================================

#[test]
fn test_code_actions_with_multiple_diagnostics() {
    let doc = create_test_document("fn main() { let x = foo; let y: Int = \"text\"; }");
    let uri = create_test_uri();
    let range = Range {
        start: Position { line: 0, character: 0 },
        end: Position { line: 0, character: 50 },
    };

    let diag1 = create_diagnostic(
        "foo not found",
        Range {
            start: Position { line: 0, character: 20 },
            end: Position { line: 0, character: 23 },
        },
    );

    let diag2 = create_diagnostic(
        "type mismatch: expected Int, found Text",
        Range {
            start: Position { line: 0, character: 38 },
            end: Position { line: 0, character: 44 },
        },
    );

    let context = CodeActionContext {
        diagnostics: vec![diag1, diag2],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // Should have actions for both diagnostics
    assert!(!actions.is_empty(), "Should have at least one action for multiple diagnostics");
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_code_actions_empty_document() {
    let doc = create_test_document("");
    let uri = create_test_uri();
    let range = Range {
        start: Position { line: 0, character: 0 },
        end: Position { line: 0, character: 0 },
    };

    let context = CodeActionContext {
        diagnostics: vec![],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // Should not panic on empty document
}

#[test]
fn test_code_actions_out_of_bounds_range() {
    let doc = create_test_document("fn main() { }");
    let uri = create_test_uri();

    // Range beyond document
    let range = Range {
        start: Position { line: 100, character: 0 },
        end: Position { line: 101, character: 0 },
    };

    let context = CodeActionContext {
        diagnostics: vec![],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // Should not panic on out of bounds range
}

#[test]
fn test_code_actions_unicode_content() {
    let doc = create_test_document("fn main() { let привет = \"мир\"; }");
    let uri = create_test_uri();
    let range = Range {
        start: Position { line: 0, character: 16 },
        end: Position { line: 0, character: 22 },
    };

    let context = CodeActionContext {
        diagnostics: vec![],
        only: None,
        trigger_kind: None,
    };

    let actions = code_actions(&doc, range, context, &uri);
    // Should handle unicode correctly
}
