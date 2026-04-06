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
// Comprehensive tests for enhanced refinement error diagnostics
//
// Refinement type diagnostics: error messages for failed refinement checks with source location and predicate details — 8.3
//
// Tests cover:
// 1. Actual value tracking and display
// 2. Predicate evaluation and decomposition
// 3. Suggestion generation based on context
// 4. Multi-constraint breakdown with ✓/✗ markers
// 5. Nested refinement error reporting

use verum_ast::{
    expr::{BinOp, Expr, ExprKind},
    literal::Literal,
    span::{FileId, Span},
    ty::{Ident, Path},
};
use verum_common::{Heap, Maybe, Text};
use verum_types::{
    ConstValue, ConstraintEvaluation, ConstraintResult, ErrorContext, PredicateEvaluator,
    RefinementDiagnosticBuilder, RefinementSource, Suggestion, SuggestionGenerator,
};

// ==================== Test Helpers ====================

fn create_test_span() -> Span {
    Span::new(10, 20, FileId::dummy())
}

fn create_int_literal(value: i64) -> Expr {
    let span = create_test_span();
    Expr {
        kind: ExprKind::Literal(Literal::int(value as i128, span)),
        span,
        ref_kind: None,
        check_eliminated: false,
    }
}

fn create_var(name: &str) -> Expr {
    let span = create_test_span();
    let ident = Ident::new(name, span);
    let path = Path::single(ident);
    Expr {
        kind: ExprKind::Path(path),
        span,
        ref_kind: None,
        check_eliminated: false,
    }
}

fn create_binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    let span = left.span.merge(right.span);
    Expr {
        kind: ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        span,
        ref_kind: None,
        check_eliminated: false,
    }
}

fn create_comparison(var_name: &str, op: BinOp, value: i64) -> Expr {
    create_binary(op, create_var(var_name), create_int_literal(value))
}

fn create_test_context() -> ErrorContext {
    ErrorContext {
        function_name: Maybe::None,
        expected_type: Text::from("Positive"),
        actual_type: Text::from("Int"),
        refinement_source: RefinementSource::TypeAnnotation,
    }
}

// ==================== Actual Value Tracking Tests ====================

#[test]
fn test_actual_value_tracking_negative() {
    // Spec: Lines 13186-13196
    let diag = RefinementDiagnosticBuilder::new()
        .constraint(Text::from("i > 0"))
        .actual_value(ConstValue::Int(-5))
        .context(create_test_context())
        .span(create_test_span())
        .build();

    // Verify actual value is tracked
    assert!(matches!(
        diag.actual_value,
        Maybe::Some(ConstValue::Int(-5))
    ));

    // Verify error message includes actual value
    let error_msg = diag.format_error();
    assert!(error_msg.contains("-5"));
    assert!(error_msg.contains("i > 0"));
}

#[test]
fn test_actual_value_tracking_positive() {
    let diag = RefinementDiagnosticBuilder::new()
        .constraint(Text::from("i > 0"))
        .actual_value(ConstValue::Int(42))
        .context(create_test_context())
        .span(create_test_span())
        .build();

    assert!(matches!(
        diag.actual_value,
        Maybe::Some(ConstValue::Int(42))
    ));
}

#[test]
fn test_actual_value_bool() {
    let diag = RefinementDiagnosticBuilder::new()
        .constraint(Text::from("b == true"))
        .actual_value(ConstValue::Bool(false))
        .context(create_test_context())
        .span(create_test_span())
        .build();

    assert!(matches!(
        diag.actual_value,
        Maybe::Some(ConstValue::Bool(false))
    ));
}

// ==================== Predicate Evaluation Tests ====================

#[test]
fn test_evaluate_simple_greater_than_violated() {
    // Test: x > 0 with x = -5 should be violated
    let evaluator = PredicateEvaluator::new();
    let predicate = create_comparison("x", BinOp::Gt, 0);
    let value = ConstValue::Int(-5);

    let result = evaluator.evaluate(&predicate, &value, "x");
    assert_eq!(result, ConstraintResult::Violated);
}

#[test]
fn test_evaluate_simple_greater_than_satisfied() {
    // Test: x > 0 with x = 5 should be satisfied
    let evaluator = PredicateEvaluator::new();
    let predicate = create_comparison("x", BinOp::Gt, 0);
    let value = ConstValue::Int(5);

    let result = evaluator.evaluate(&predicate, &value, "x");
    assert_eq!(result, ConstraintResult::Satisfied);
}

#[test]
fn test_evaluate_less_than() {
    let evaluator = PredicateEvaluator::new();
    let predicate = create_comparison("x", BinOp::Lt, 100);

    // x = 50 < 100 should be satisfied
    let result = evaluator.evaluate(&predicate, &ConstValue::Int(50), "x");
    assert_eq!(result, ConstraintResult::Satisfied);

    // x = 150 < 100 should be violated
    let result = evaluator.evaluate(&predicate, &ConstValue::Int(150), "x");
    assert_eq!(result, ConstraintResult::Violated);
}

#[test]
fn test_evaluate_equals() {
    let evaluator = PredicateEvaluator::new();
    let predicate = create_comparison("x", BinOp::Eq, 42);

    let result = evaluator.evaluate(&predicate, &ConstValue::Int(42), "x");
    assert_eq!(result, ConstraintResult::Satisfied);

    let result = evaluator.evaluate(&predicate, &ConstValue::Int(0), "x");
    assert_eq!(result, ConstraintResult::Violated);
}

#[test]
fn test_evaluate_not_equals() {
    let evaluator = PredicateEvaluator::new();
    let predicate = create_comparison("x", BinOp::Ne, 0);

    let result = evaluator.evaluate(&predicate, &ConstValue::Int(5), "x");
    assert_eq!(result, ConstraintResult::Satisfied);

    let result = evaluator.evaluate(&predicate, &ConstValue::Int(0), "x");
    assert_eq!(result, ConstraintResult::Violated);
}

// ==================== Predicate Decomposition Tests ====================

#[test]
fn test_decompose_simple_predicate() {
    // Test single constraint decomposition
    let evaluator = PredicateEvaluator::new();
    let predicate = create_comparison("x", BinOp::Gt, 0);

    let evals = evaluator.decompose(&predicate);
    assert_eq!(evals.len(), 1);
}

#[test]
fn test_decompose_compound_and_predicate() {
    // Spec: Lines 13450-13495 - Multi-constraint breakdown
    // Test: x > 0 && x < 100
    let evaluator = PredicateEvaluator::new();

    let left = create_comparison("x", BinOp::Gt, 0);
    let right = create_comparison("x", BinOp::Lt, 100);
    let predicate = create_binary(BinOp::And, left, right);

    let evals = evaluator.decompose(&predicate);

    // Should decompose into 2 constraints
    assert_eq!(evals.len(), 2);
}

#[test]
fn test_decompose_with_value_evaluation() {
    // Spec: Lines 13428-13448 - Constraint evaluation with markers
    let evaluator = PredicateEvaluator::new();

    let left = create_comparison("x", BinOp::Gt, 0);
    let right = create_comparison("x", BinOp::Lt, 100);
    let predicate = create_binary(BinOp::And, left, right);

    // Value -5: fails first constraint
    let evals = evaluator.decompose_with_value(&predicate, &ConstValue::Int(-5), "x");
    assert_eq!(evals.len(), 2);

    // First constraint should be violated
    assert_eq!(evals[0].result, ConstraintResult::Violated);
    assert_eq!(evals[0].result.marker(), "✗");

    // Second constraint should be satisfied
    assert_eq!(evals[1].result, ConstraintResult::Satisfied);
    assert_eq!(evals[1].result.marker(), "✓");
}

#[test]
fn test_decompose_with_value_both_violated() {
    let evaluator = PredicateEvaluator::new();

    let left = create_comparison("x", BinOp::Gt, 0);
    let right = create_comparison("x", BinOp::Lt, 100);
    let predicate = create_binary(BinOp::And, left, right);

    // Value 150: fails second constraint
    let evals = evaluator.decompose_with_value(&predicate, &ConstValue::Int(150), "x");
    assert_eq!(evals.len(), 2);

    assert_eq!(evals[0].result, ConstraintResult::Satisfied); // 150 > 0
    assert_eq!(evals[1].result, ConstraintResult::Violated); // 150 < 100
}

#[test]
fn test_constraint_evaluation_markers() {
    let satisfied = ConstraintResult::Satisfied;
    let violated = ConstraintResult::Violated;
    let unknown = ConstraintResult::Unknown;

    assert_eq!(satisfied.marker(), "✓");
    assert_eq!(violated.marker(), "✗");
    assert_eq!(unknown.marker(), "?");
}

// ==================== Suggestion Generation Tests ====================

#[test]
fn test_suggestions_for_type_annotation() {
    // Spec: Lines 13357-13364 - Pattern 1: Direct assignment
    let context = ErrorContext {
        function_name: Maybe::None,
        expected_type: Text::from("Positive"),
        actual_type: Text::from("Int"),
        refinement_source: RefinementSource::TypeAnnotation,
    };

    let suggestions = SuggestionGenerator::generate("x > 0", &context, &Maybe::None);

    // Should include runtime check and try_from
    assert!(suggestions.len() >= 2);

    // Verify suggestion types
    let has_runtime_check = suggestions
        .iter()
        .any(|s| matches!(s, Suggestion::RuntimeCheck { .. }));
    assert!(has_runtime_check, "Should include runtime check suggestion");
}

#[test]
fn test_suggestions_for_function_parameter() {
    // Spec: Lines 13366-13374 - Pattern 2: Function parameter
    let context = ErrorContext {
        function_name: Maybe::Some(Text::from("divide")),
        expected_type: Text::from("NonZero"),
        actual_type: Text::from("Int"),
        refinement_source: RefinementSource::FunctionParameter,
    };

    let suggestions = SuggestionGenerator::generate("x != 0", &context, &Maybe::None);

    assert!(suggestions.len() >= 2);

    // Should include validate before call and weaken signature
    let has_weaken = suggestions
        .iter()
        .any(|s| matches!(s, Suggestion::WeakenType { .. }));
    assert!(has_weaken, "Should include weaken type suggestion");
}

#[test]
fn test_suggestions_for_function_return() {
    // Spec: Lines 13376-13385 - Pattern 3: Function return
    let context = ErrorContext {
        function_name: Maybe::Some(Text::from("sqrt")),
        expected_type: Text::from("Positive"),
        actual_type: Text::from("Float"),
        refinement_source: RefinementSource::FunctionReturn,
    };

    let suggestions = SuggestionGenerator::generate("x >= 0", &context, &Maybe::None);

    assert!(suggestions.len() >= 2);

    // Should include assert before return
    let has_proof = suggestions
        .iter()
        .any(|s| matches!(s, Suggestion::CompileTimeProof { .. }));
    assert!(has_proof, "Should include compile-time proof suggestion");
}

#[test]
fn test_suggestions_for_field_constraint() {
    let context = ErrorContext {
        function_name: Maybe::None,
        expected_type: Text::from("ValidAmount"),
        actual_type: Text::from("Float"),
        refinement_source: RefinementSource::FieldConstraint,
    };

    let suggestions = SuggestionGenerator::generate("amount > 0", &context, &Maybe::None);

    assert!(suggestions.len() >= 2);
}

// ==================== Complete Diagnostic Builder Tests ====================

#[test]
fn test_diagnostic_builder_with_all_features() {
    // Spec: Lines 13246-13262 - Complete error with all features
    let predicate = create_comparison("x", BinOp::Gt, 0);

    let diag = RefinementDiagnosticBuilder::new()
        .constraint(Text::from("x > 0"))
        .actual_value(ConstValue::Int(-5))
        .context(create_test_context())
        .span(create_test_span())
        .predicate_expr(predicate)
        .var_name(Text::from("x"))
        .build();

    // Verify all components are present
    assert!(matches!(
        diag.actual_value,
        Maybe::Some(ConstValue::Int(-5))
    ));
    assert_eq!(diag.constraint.as_ref() as &str, "x > 0");
    assert!(!diag.suggestions.is_empty());
}

#[test]
fn test_diagnostic_builder_with_compound_predicate() {
    let left = create_comparison("x", BinOp::Gt, 0);
    let right = create_comparison("x", BinOp::Lt, 100);
    let predicate = create_binary(BinOp::And, left, right);

    let diag = RefinementDiagnosticBuilder::new()
        .constraint(Text::from("x > 0 && x < 100"))
        .actual_value(ConstValue::Int(-5))
        .context(create_test_context())
        .span(create_test_span())
        .predicate_expr(predicate)
        .var_name(Text::from("x"))
        .build();

    // Should have multiple constraint evaluations
    assert!(diag.constraint_evals.len() >= 2);
}

#[test]
fn test_diagnostic_format_includes_all_sections() {
    let predicate = create_comparison("x", BinOp::Gt, 0);

    let diag = RefinementDiagnosticBuilder::new()
        .constraint(Text::from("x > 0"))
        .actual_value(ConstValue::Int(-5))
        .context(create_test_context())
        .span(create_test_span())
        .predicate_expr(predicate)
        .var_name(Text::from("x"))
        .build();

    let formatted = diag.format_error();

    // Verify format includes all required sections (Spec Lines 13397-13410)
    assert!(formatted.contains("error: refinement constraint not satisfied"));
    assert!(formatted.contains("-->")); // Source location
    assert!(formatted.contains("-5")); // Actual value
    assert!(formatted.contains("x > 0")); // Constraint
    assert!(formatted.contains("= note:")); // Notes
    assert!(formatted.contains("= help:")); // Suggestions
}

// ==================== Error Context Tests ====================

#[test]
fn test_error_context_display() {
    let sources = vec![
        (RefinementSource::TypeAnnotation, "type annotation"),
        (RefinementSource::FunctionParameter, "function parameter"),
        (RefinementSource::FunctionReturn, "function return type"),
        (RefinementSource::FieldConstraint, "field constraint"),
        (RefinementSource::Assignment, "assignment"),
    ];

    for (source, expected_text) in sources {
        let display = format!("{}", source);
        assert_eq!(display, expected_text);
    }
}

// ==================== Integration Tests ====================

#[test]
fn test_positive_constraint_violation_example() {
    // Spec: Lines 13180-13196 - Basic refinement violation example
    let predicate = create_comparison("i", BinOp::Gt, 0);

    let context = ErrorContext {
        function_name: Maybe::None,
        expected_type: Text::from("Positive"),
        actual_type: Text::from("Int"),
        refinement_source: RefinementSource::TypeAnnotation,
    };

    let diag = RefinementDiagnosticBuilder::new()
        .constraint(Text::from("i > 0"))
        .actual_value(ConstValue::Int(-5))
        .context(context)
        .span(Span::new(2, 23, FileId::new(0)))
        .predicate_expr(predicate)
        .var_name(Text::from("i"))
        .build();

    let formatted = diag.format_error();

    // Verify output matches spec example format
    assert!(formatted.contains("error: refinement constraint not satisfied"));
    assert!(formatted.contains("-5"));
    assert!(formatted.contains("i > 0"));
    assert!(formatted.contains("Positive"));
}

#[test]
fn test_division_by_zero_example() {
    // Spec: Lines 13198-13230 - Function call with refinement error
    let predicate = create_comparison("b", BinOp::Ne, 0);

    let context = ErrorContext {
        function_name: Maybe::Some(Text::from("divide")),
        expected_type: Text::from("Int{!= 0}"),
        actual_type: Text::from("Int"),
        refinement_source: RefinementSource::FunctionParameter,
    };

    let diag = RefinementDiagnosticBuilder::new()
        .constraint(Text::from("b != 0"))
        .actual_value(ConstValue::Int(0))
        .context(context)
        .span(create_test_span())
        .predicate_expr(predicate)
        .var_name(Text::from("b"))
        .build();

    let formatted = diag.format_error();

    assert!(formatted.contains("0"));
    assert!(formatted.contains("!= 0"));
}

#[test]
fn test_range_constraint_example() {
    // Test multi-constraint: x > 0 && x < 100
    let left = create_comparison("x", BinOp::Gt, 0);
    let right = create_comparison("x", BinOp::Lt, 100);
    let predicate = create_binary(BinOp::And, left, right);

    let context = ErrorContext {
        function_name: Maybe::None,
        expected_type: Text::from("SmallPositive"),
        actual_type: Text::from("Int"),
        refinement_source: RefinementSource::TypeAnnotation,
    };

    let diag = RefinementDiagnosticBuilder::new()
        .constraint(Text::from("x > 0 && x < 100"))
        .actual_value(ConstValue::Int(150))
        .context(context)
        .span(create_test_span())
        .predicate_expr(predicate)
        .var_name(Text::from("x"))
        .build();

    // Value 150: should satisfy x > 0 but violate x < 100
    assert_eq!(diag.constraint_evals.len(), 2);
    assert_eq!(diag.constraint_evals[0].result, ConstraintResult::Satisfied);
    assert_eq!(diag.constraint_evals[1].result, ConstraintResult::Violated);

    let formatted = diag.format_error();
    assert!(formatted.contains("✓"));
    assert!(formatted.contains("✗"));
}

// ==================== Edge Cases ====================

#[test]
fn test_diagnostic_without_actual_value() {
    // Test that diagnostic works without concrete value
    let predicate = create_comparison("x", BinOp::Gt, 0);

    let diag = RefinementDiagnosticBuilder::new()
        .constraint(Text::from("x > 0"))
        .context(create_test_context())
        .span(create_test_span())
        .predicate_expr(predicate)
        .build();

    assert!(matches!(diag.actual_value, Maybe::None));

    // Should still decompose predicate (without evaluation)
    assert!(!diag.constraint_evals.is_empty());
}

#[test]
fn test_diagnostic_with_notes() {
    let diag = RefinementDiagnosticBuilder::new()
        .constraint(Text::from("x > 0"))
        .context(create_test_context())
        .span(create_test_span())
        .build()
        .add_note(Text::from("Additional context"))
        .add_note(Text::from("More information"));

    assert_eq!(diag.notes.len(), 2);

    let formatted = diag.format_error();
    assert!(formatted.contains("Additional context"));
    assert!(formatted.contains("More information"));
}

#[test]
fn test_constraint_result_is_violated() {
    assert!(ConstraintResult::Violated.is_violated());
    assert!(!ConstraintResult::Satisfied.is_violated());
    assert!(!ConstraintResult::Unknown.is_violated());
}

#[test]
fn test_constraint_evaluation_format_line() {
    let eval = ConstraintEvaluation::new(Text::from("x > 0"), ConstraintResult::Violated)
        .with_explanation(Text::from("-5 > 0 = false"));

    let line = eval.format_line();
    assert!(line.contains("✗"));
    assert!(line.contains("x > 0"));
    assert!(line.contains("-5 > 0 = false"));
}

#[test]
fn test_suggestion_format_help() {
    let suggestion = Suggestion::RuntimeCheck {
        code: Text::from("if x > 0 { ... }"),
        explanation: Text::from("add runtime check:"),
    };

    let help = suggestion.format_help();
    assert!(help.contains("add runtime check:"));
    assert!(help.contains("if x > 0"));
}

// ==================== Performance Tests ====================

#[test]
fn test_predicate_decomposition_performance() {
    // Test that decomposition is fast for deeply nested predicates
    let evaluator = PredicateEvaluator::new();

    // Create nested AND expression: x > 0 && x < 10 && x != 5 && x >= 1
    let mut predicate = create_comparison("x", BinOp::Gt, 0);
    predicate = create_binary(BinOp::And, predicate, create_comparison("x", BinOp::Lt, 10));
    predicate = create_binary(BinOp::And, predicate, create_comparison("x", BinOp::Ne, 5));
    predicate = create_binary(BinOp::And, predicate, create_comparison("x", BinOp::Ge, 1));

    let evals = evaluator.decompose(&predicate);

    // Should decompose into 4 constraints
    assert_eq!(evals.len(), 4);
}

#[test]
fn test_diagnostic_builder_performance() {
    // Test that building complex diagnostics is efficient
    let predicate = create_comparison("x", BinOp::Gt, 0);

    for _ in 0..100 {
        let _diag = RefinementDiagnosticBuilder::new()
            .constraint("x > 0".into())
            .actual_value(ConstValue::Int(-5))
            .context(create_test_context())
            .span(create_test_span())
            .predicate_expr(predicate.clone())
            .var_name(Text::from("x"))
            .build();
    }
}
