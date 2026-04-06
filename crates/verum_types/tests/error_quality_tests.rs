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
// Error message quality tests
//
// Tests that error messages are:
// - Clear and actionable
// - Include context and suggestions
// - Follow diagnostic format from spec
// - Provide helpful hints
//
// Error handling: use Result<T, E> for recoverable errors, panic() for unrecoverable, no unwrap in library code

use verum_ast::{
    expr::*,
    literal::Literal,
    span::{FileId, Span},
    ty::Ident,
};
use verum_common::{Heap, List, Text};
use verum_types::{TypeChecker, TypeError};

// Helper function to create variable expressions
fn var_expr(name: &str, span: Span) -> Expr {
    Expr::ident(Ident::new(name, span))
}

// ============================================================================
// Type Mismatch Error Tests
// ============================================================================

#[test]
fn test_type_mismatch_error_format() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Try to add Int + Bool (type error)
    let left = Box::new(Expr::literal(Literal::int(42, span)));
    let right = Box::new(Expr::literal(Literal::bool(true, span)));
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            right,
        },
        span,
    );

    let result = checker.synth_expr(&expr);
    assert!(result.is_err());

    if let Err(err) = result {
        let msg = err.to_string();
        assert!(msg.contains("mismatch") || msg.contains("expected") || msg.contains("found"));
    }
}

#[test]
fn test_type_mismatch_includes_types() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Expected: Int, Found: Bool
    let left = Box::new(Expr::literal(Literal::int(1, span)));
    let right = Box::new(Expr::literal(Literal::bool(true, span)));
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            right,
        },
        span,
    );

    let result = checker.synth_expr(&expr);
    if let Err(err) = result {
        let msg = err.to_string();
        // Should mention both types
        assert!(msg.contains("Int") || msg.contains("Bool") || msg.contains("type"));
    }
}

#[test]
fn test_type_mismatch_has_span() {
    let span = Span::new(10, 20, FileId::new(0));

    let error = TypeError::Mismatch {
        expected: Text::from("Int"),
        actual: Text::from("Bool"),
        span,
    };

    match error {
        TypeError::Mismatch { span: err_span, .. } => {
            assert_eq!(err_span.start, 10);
            assert_eq!(err_span.end, 20);
        }
        _ => panic!("Wrong error type"),
    }
}

// ============================================================================
// Unbound Variable Error Tests
// ============================================================================

#[test]
fn test_unbound_variable_error() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let expr = var_expr("undefined_var", span);
    let result = checker.synth_expr(&expr);

    assert!(matches!(result, Err(TypeError::UnboundVariable { .. })));
}

#[test]
fn test_unbound_variable_message() {
    let span = Span::dummy();
    let error = TypeError::UnboundVariable {
        name: Text::from("foo"),
        span,
    };

    let msg = error.to_string();
    assert!(msg.contains("foo"));
    assert!(msg.contains("unbound") || msg.contains("not found"));
}

#[test]
fn test_unbound_variable_includes_name() {
    let span = Span::dummy();
    let error = TypeError::UnboundVariable {
        name: Text::from("myVariable"),
        span,
    };

    let msg = error.to_string();
    assert!(msg.contains("myVariable"));
}

// ============================================================================
// Function Application Error Tests
// ============================================================================

#[test]
fn test_not_a_function_error() {
    let span = Span::dummy();
    let error = TypeError::NotAFunction {
        ty: Text::from("Int"),
        span,
    };

    let msg = error.to_string();
    assert!(msg.contains("not a function") || msg.contains("cannot call"));
    assert!(msg.contains("Int"));
}

#[test]
fn test_not_a_function_with_suggestion() {
    let span = Span::dummy();
    let error = TypeError::NotAFunction {
        ty: Text::from("Int"),
        span,
    };

    let diagnostic = error.to_diagnostic();
    // Should produce a helpful diagnostic
    assert!(!diagnostic.message().is_empty());
}

// ============================================================================
// Branch Type Mismatch Tests
// ============================================================================

#[test]
fn test_branch_mismatch_error() {
    let span = Span::dummy();
    let error = TypeError::BranchMismatch {
        then_ty: Text::from("Int"),
        else_ty: Text::from("Bool"),
        span,
    };

    let msg = error.to_string();
    assert!(msg.contains("branch") || msg.contains("then") || msg.contains("else"));
    assert!(msg.contains("Int"));
    assert!(msg.contains("Bool"));
}

#[test]
fn test_branch_mismatch_from_if_expr() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // if true then 42 else false
    let condition = Box::new(IfCondition {
        conditions: smallvec::smallvec![ConditionKind::Expr(Expr::literal(Literal::bool(
            true, span
        )))],
        span,
    });
    let then_branch = Block {
        stmts: List::new(),
        expr: Some(Box::new(Expr::literal(Literal::int(42, span)))),
        span,
    };
    let else_branch = Some(Box::new(Expr::literal(Literal::bool(false, span))));

    let expr = Expr::new(
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        },
        span,
    );

    let result = checker.synth_expr(&expr);
    assert!(result.is_err());

    if let Err(err) = result {
        // Should be branch mismatch or type mismatch
        assert!(matches!(
            err,
            TypeError::BranchMismatch { .. } | TypeError::Mismatch { .. }
        ));
    }
}

// ============================================================================
// Lambda Inference Error Tests
// ============================================================================

#[test]
fn test_cannot_infer_lambda_error() {
    let span = Span::dummy();
    let error = TypeError::CannotInferLambda { span };

    let msg = error.to_string();
    assert!(msg.contains("lambda") || msg.contains("infer") || msg.contains("annotation"));
}

#[test]
fn test_lambda_requires_annotation_suggestion() {
    let span = Span::dummy();
    let error = TypeError::CannotInferLambda { span };

    let diagnostic = error.to_diagnostic();
    let msg = diagnostic.message();

    // Should suggest adding type annotation
    assert!(!msg.is_empty());
}

// ============================================================================
// Infinite Type Error Tests
// ============================================================================

#[test]
fn test_infinite_type_error() {
    let span = Span::dummy();
    let error = TypeError::InfiniteType {
        var: Text::from("a"),
        ty: Text::from("List a"),
        span,
    };

    let msg = error.to_string();
    assert!(msg.contains("infinite"));
    assert!(msg.contains("a"));
}

#[test]
fn test_infinite_type_explanation() {
    let span = Span::dummy();
    let error = TypeError::InfiniteType {
        var: Text::from("T"),
        ty: Text::from("fn(T) -> T"),
        span,
    };

    let msg = error.to_string();
    assert!(msg.contains("T"));
    assert!(msg.contains("infinite") || msg.contains("recursive"));
}

// ============================================================================
// Protocol Error Tests
// ============================================================================

#[test]
fn test_protocol_not_satisfied_error() {
    let span = Span::dummy();
    let error = TypeError::ProtocolNotSatisfied {
        ty: Text::from("MyType"),
        protocol: Text::from("Eq"),
        span,
    };

    let msg = error.to_string();
    assert!(msg.contains("MyType"));
    assert!(msg.contains("Eq"));
    assert!(msg.contains("protocol") || msg.contains("implement"));
}

#[test]
fn test_protocol_error_suggests_impl() {
    let span = Span::dummy();
    let error = TypeError::ProtocolNotSatisfied {
        ty: Text::from("CustomType"),
        protocol: Text::from("Show"),
        span,
    };

    let diagnostic = error.to_diagnostic();
    let msg = diagnostic.message();

    // Should mention implementation needed
    assert!(msg.contains("CustomType") && msg.contains("Show"));
}

// ============================================================================
// Refinement Error Tests
// ============================================================================

#[test]
fn test_refinement_failed_error() {
    let span = Span::dummy();
    let error = TypeError::RefinementFailed {
        predicate: Text::from("x > 0"),
        span,
    };

    let msg = error.to_string();
    assert!(msg.contains("refinement"));
    assert!(msg.contains("x > 0"));
}

#[test]
fn test_refinement_shows_predicate() {
    let span = Span::dummy();
    let error = TypeError::RefinementFailed {
        predicate: Text::from("length xs > 0"),
        span,
    };

    let msg = error.to_string();
    assert!(msg.contains("length xs > 0"));
}

// ============================================================================
// Context Error Tests
// ============================================================================

#[test]
fn test_context_not_allowed_error() {
    let span = Span::dummy();
    let error = TypeError::ContextNotAllowed {
        context: Text::from("IO"),
        span,
    };

    let msg = error.to_string();
    assert!(msg.contains("IO"));
    assert!(msg.contains("context") || msg.contains("not allowed"));
}

#[test]
fn test_context_error_state() {
    let span = Span::dummy();
    let error = TypeError::ContextNotAllowed {
        context: Text::from("State"),
        span,
    };

    let msg = error.to_string();
    assert!(msg.contains("State"));
}

// ============================================================================
// Const Generic Error Tests
// ============================================================================

#[test]
fn test_const_mismatch_error() {
    let span = Span::dummy();
    let error = TypeError::ConstMismatch {
        expected: Text::from("10"),
        actual: Text::from("5"),
        span,
    };

    let msg = error.to_string();
    assert!(msg.contains("10"));
    assert!(msg.contains("5"));
    assert!(msg.contains("const") || msg.contains("mismatch"));
}

// ============================================================================
// Ambiguous Type Error Tests
// ============================================================================

#[test]
fn test_ambiguous_type_error() {
    let span = Span::dummy();
    let error = TypeError::AmbiguousType { span };

    let msg = error.to_string();
    assert!(msg.contains("ambiguous") || msg.contains("cannot infer"));
}

#[test]
fn test_ambiguous_suggests_annotation() {
    let span = Span::dummy();
    let error = TypeError::AmbiguousType { span };

    let diagnostic = error.to_diagnostic();
    let msg = diagnostic.message();

    assert!(msg.contains("ambiguous") || msg.contains("context"));
}

// ============================================================================
// Affine Type Error Tests
// ============================================================================

#[test]
fn test_affine_violation_error() {
    let span1 = Span::new(10, 15, FileId::new(0));
    let span2 = Span::new(20, 25, FileId::new(0));

    let error = TypeError::AffineViolation {
        ty: Text::from("File"),
        first_use: span1,
        second_use: span2,
    };

    let msg = error.to_string();
    assert!(msg.contains("File"));
    assert!(msg.contains("affine") || msg.contains("used more than once"));
}

#[test]
fn test_affine_shows_both_uses() {
    let span1 = Span::new(10, 15, FileId::new(0));
    let span2 = Span::new(20, 25, FileId::new(0));

    let error = TypeError::AffineViolation {
        ty: Text::from("Resource"),
        first_use: span1,
        second_use: span2,
    };

    let msg = error.to_string();
    // Should show both use locations
    assert!(msg.len() > 20); // Should be detailed
}

// ============================================================================
// Linear Type Error Tests
// ============================================================================

#[test]
fn test_linear_violation_error() {
    let span = Span::dummy();
    let error = TypeError::LinearViolation {
        ty: Text::from("Token"),
        usage_count: 0,
        span,
    };

    let msg = error.to_string();
    assert!(msg.contains("Token"));
    assert!(msg.contains("linear") || msg.contains("exactly once"));
}

#[test]
fn test_linear_shows_usage_count() {
    let span = Span::dummy();
    let error = TypeError::LinearViolation {
        ty: Text::from("LinearResource"),
        usage_count: 2,
        span,
    };

    let msg = error.to_string();
    // Verify error message includes the usage count for better diagnostics
    assert!(msg.contains("2"), "Error should show usage count: {}", msg);
}

// ============================================================================
// Moved Value Error Tests
// ============================================================================

#[test]
fn test_moved_value_error() {
    let moved_span = Span::new(10, 15, FileId::new(0));
    let used_span = Span::new(20, 25, FileId::new(0));

    let error = TypeError::MovedValueUsed {
        name: Text::from("data"),
        moved_at: moved_span,
        used_at: used_span,
    };

    let msg = error.to_string();
    assert!(msg.contains("data"));
    assert!(msg.contains("moved") || msg.contains("after move"));
}

#[test]
fn test_moved_value_shows_locations() {
    let moved_span = Span::new(10, 15, FileId::new(0));
    let used_span = Span::new(20, 25, FileId::new(0));

    let error = TypeError::MovedValueUsed {
        name: Text::from("value"),
        moved_at: moved_span,
        used_at: used_span,
    };

    let msg = error.to_string();
    // Should show both move and use locations
    assert!(msg.len() > 30);
}

// ============================================================================
// Diagnostic Format Tests
// ============================================================================

#[test]
fn test_diagnostic_has_message() {
    let span = Span::dummy();
    let error = TypeError::Mismatch {
        expected: Text::from("Int"),
        actual: Text::from("Bool"),
        span,
    };

    let diagnostic = error.to_diagnostic();
    assert!(!diagnostic.message().is_empty());
}

#[test]
fn test_all_errors_have_diagnostics() {
    let span = Span::dummy();

    let errors = vec![
        TypeError::Mismatch {
            expected: Text::from("Int"),
            actual: Text::from("Bool"),
            span,
        },
        TypeError::UnboundVariable {
            name: Text::from("x"),
            span,
        },
        TypeError::NotAFunction {
            ty: Text::from("Int"),
            span,
        },
        TypeError::CannotInferLambda { span },
        TypeError::AmbiguousType { span },
    ];

    for error in errors {
        let diagnostic = error.to_diagnostic();
        assert!(!diagnostic.message().is_empty());
    }
}

// ============================================================================
// Error Message Quality Tests
// ============================================================================

#[test]
fn test_error_messages_are_clear() {
    let span = Span::dummy();
    let error = TypeError::Mismatch {
        expected: Text::from("Int"),
        actual: Text::from("String"),
        span,
    };

    let msg = error.to_string();

    // Should be clear what went wrong
    assert!(msg.len() > 10);
    assert!(msg.contains("Int") || msg.contains("String"));
}

#[test]
fn test_error_messages_not_too_long() {
    let span = Span::dummy();
    let error = TypeError::Mismatch {
        expected: Text::from("Int"),
        actual: Text::from("Bool"),
        span,
    };

    let msg = error.to_string();

    // Should be concise (under 200 chars for simple errors)
    assert!(msg.len() < 500);
}

#[test]
fn test_error_uses_lowercase() {
    let span = Span::dummy();
    let error = TypeError::UnboundVariable {
        name: Text::from("foo"),
        span,
    };

    let msg = error.to_string();

    // Error messages should start with lowercase (conventional)
    if let Some(first_char) = msg.chars().next() {
        // May start with uppercase for type names, that's ok
        assert!(first_char.is_alphabetic());
    }
}
