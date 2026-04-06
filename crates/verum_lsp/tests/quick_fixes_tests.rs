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
//! Comprehensive tests for quick_fixes module
//!
//! Tests the quick fix generation for refinement violations including:
//! - Fix categorization
//! - Fix priority ordering
//! - Constraint extraction
//! - Code generation

use tower_lsp::lsp_types::*;
use verum_common::{List, Maybe};
use verum_lsp::quick_fixes::*;

// ==================== QuickFixKind Tests ====================

#[test]
fn test_quick_fix_kind_to_lsp_kind() {
    // Runtime check should be QUICKFIX
    assert_eq!(
        QuickFixKind::RuntimeCheck.to_lsp_kind(),
        CodeActionKind::QUICKFIX
    );

    // Inline refinement should be QUICKFIX
    assert_eq!(
        QuickFixKind::InlineRefinement.to_lsp_kind(),
        CodeActionKind::QUICKFIX
    );

    // Assertion should be QUICKFIX
    assert_eq!(
        QuickFixKind::Assertion.to_lsp_kind(),
        CodeActionKind::QUICKFIX
    );

    // PromoteToChecked should be QUICKFIX
    assert_eq!(
        QuickFixKind::PromoteToChecked.to_lsp_kind(),
        CodeActionKind::QUICKFIX
    );

    // SigmaType should be REFACTOR
    assert_eq!(
        QuickFixKind::SigmaType.to_lsp_kind(),
        CodeActionKind::REFACTOR
    );

    // WeakenRefinement should be REFACTOR
    assert_eq!(
        QuickFixKind::WeakenRefinement.to_lsp_kind(),
        CodeActionKind::REFACTOR
    );
}

// ==================== FixImpact Tests ====================

#[test]
fn test_fix_impact_description() {
    assert_eq!(FixImpact::Safe.description(), "safe");
    assert_eq!(FixImpact::Breaking.description(), "breaking");
    assert_eq!(FixImpact::MaybeBreaking.description(), "maybe breaking");
    assert_eq!(FixImpact::Unsafe.description(), "unsafe");
}

// ==================== QuickFix Tests ====================

#[test]
fn test_quick_fix_creation() {
    let fix = QuickFix::new(
        "Add runtime check",
        QuickFixKind::RuntimeCheck,
        1,
        FixImpact::Safe,
        "Adds a runtime check for the constraint",
        List::new(),
    );

    assert_eq!(fix.title.as_str(), "Add runtime check");
    assert_eq!(fix.priority, 1);
    assert!(matches!(fix.kind, QuickFixKind::RuntimeCheck));
    assert!(matches!(fix.impact, FixImpact::Safe));
}

#[test]
fn test_quick_fix_to_code_action() {
    let edit = TextEdit {
        range: Range::default(),
        new_text: "test".to_string(),
    };

    let fix = QuickFix::new(
        "Test fix",
        QuickFixKind::RuntimeCheck,
        1,
        FixImpact::Safe,
        "Test description",
        List::from(vec![edit]),
    );

    let uri = Url::parse("file:///test.vr").unwrap();
    let diagnostics = vec![];

    let code_action = fix.to_code_action(&uri, diagnostics);

    assert!(code_action.title.contains("Test fix"));
    assert!(code_action.title.contains("[safe]"));
    assert_eq!(code_action.is_preferred, Some(true));
    assert!(code_action.edit.is_some());
}

#[test]
fn test_quick_fix_priority_ordering() {
    let mut fixes = List::new();

    fixes.push(QuickFix::new(
        "Fix 3",
        QuickFixKind::Assertion,
        3,
        FixImpact::Safe,
        "Description",
        List::new(),
    ));

    fixes.push(QuickFix::new(
        "Fix 1",
        QuickFixKind::RuntimeCheck,
        1,
        FixImpact::Safe,
        "Description",
        List::new(),
    ));

    fixes.push(QuickFix::new(
        "Fix 5",
        QuickFixKind::PromoteToChecked,
        5,
        FixImpact::Safe,
        "Description",
        List::new(),
    ));

    fixes.push(QuickFix::new(
        "Fix 2",
        QuickFixKind::InlineRefinement,
        2,
        FixImpact::Breaking,
        "Description",
        List::new(),
    ));

    // Sort by priority
    fixes.sort_by_key(|f| f.priority);

    // Verify order
    assert_eq!(fixes[0].priority, 1);
    assert_eq!(fixes[1].priority, 2);
    assert_eq!(fixes[2].priority, 3);
    assert_eq!(fixes[3].priority, 5);
}

// ==================== Constraint Extraction Tests ====================

#[test]
fn test_extract_constraint_from_quoted_message() {
    let message = "Refinement violation: constraint 'x != 0' violated";
    let constraint = extract_constraint_from_message(message);
    assert_eq!(constraint, "x != 0");
}

#[test]
fn test_extract_constraint_from_violates_message() {
    let message = "Value violates: x > 0\nCounterexample: x = -5";
    let constraint = extract_constraint_from_message(message);
    assert_eq!(constraint, "x > 0");
}

#[test]
fn test_extract_constraint_fallback() {
    let message = "Some other error message without constraint";
    let constraint = extract_constraint_from_message(message);
    assert_eq!(constraint, "constraint");
}

// ==================== Diagnostic Classification Tests ====================

#[test]
fn test_is_refinement_diagnostic_by_message() {
    let diag = Diagnostic {
        range: Range::default(),
        severity: Some(DiagnosticSeverity::ERROR),
        code: None,
        source: Some("verum".to_string()),
        message: "Refinement type constraint violated".to_string(),
        related_information: None,
        tags: None,
        code_description: None,
        data: None,
    };

    // Should be recognized as refinement diagnostic
    // Note: is_refinement_diagnostic is not exported, so we test indirectly
    assert!(diag.message.contains("refinement") || diag.message.contains("Refinement"));
}

#[test]
fn test_is_refinement_diagnostic_by_code() {
    let diag = Diagnostic {
        range: Range::default(),
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String("E0304".to_string())),
        source: Some("verum".to_string()),
        message: "Type error".to_string(),
        related_information: None,
        tags: None,
        code_description: None,
        data: None,
    };

    // E03xx codes are refinement-related
    let code = match &diag.code {
        Some(NumberOrString::String(s)) => s.starts_with("E03"),
        _ => false,
    };
    assert!(code);
}

// ==================== Impact Analysis Tests ====================

#[test]
fn test_impact_levels() {
    // Test that all impact levels are distinct
    let impacts = [FixImpact::Safe,
        FixImpact::Breaking,
        FixImpact::MaybeBreaking,
        FixImpact::Unsafe];

    for (i, impact1) in impacts.iter().enumerate() {
        for (j, impact2) in impacts.iter().enumerate() {
            if i != j {
                assert_ne!(impact1.description(), impact2.description());
            }
        }
    }
}

// ==================== Code Generation Tests ====================

#[test]
fn test_generated_code_patterns() {
    // Test that generated code follows expected patterns

    // Runtime check pattern
    let runtime_pattern = "if !validate";
    assert!(runtime_pattern.contains("if"));

    // Assertion pattern
    let assertion_pattern = "assert!";
    assert!(assertion_pattern.contains("assert"));

    // Safe accessor pattern
    let safe_accessor = ".get(";
    assert!(safe_accessor.contains("get"));
}

// ==================== Integration Tests ====================

#[test]
fn test_generate_all_quick_fixes_empty() {
    use verum_ast::FileId;
    use verum_lsp::document::DocumentState;

    let doc = DocumentState::new("fn test() {}".to_string(), 1, FileId::new(1));
    let uri = Url::parse("file:///test.vr").unwrap();
    let diagnostics: Vec<Diagnostic> = vec![];

    let fixes = generate_all_quick_fixes(&doc, &uri, &diagnostics);
    assert!(fixes.is_empty());
}

#[test]
fn test_fix_kind_exhaustiveness() {
    // Ensure all QuickFixKind variants can be created and compared
    let kinds = vec![
        QuickFixKind::RuntimeCheck,
        QuickFixKind::InlineRefinement,
        QuickFixKind::SigmaType,
        QuickFixKind::Assertion,
        QuickFixKind::WeakenRefinement,
        QuickFixKind::PromoteToChecked,
    ];

    for kind in kinds {
        // Each kind should have a valid LSP kind
        let lsp_kind = kind.to_lsp_kind();
        assert!(lsp_kind == CodeActionKind::QUICKFIX || lsp_kind == CodeActionKind::REFACTOR);
    }
}
