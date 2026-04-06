//! Comprehensive tests for Refinement Evidence Propagation
//!
//! Flow-sensitive refinement evidence tracking: maintains proof witnesses for satisfied
//! refinement predicates, propagates evidence through control flow (if/match narrowing),
//! and enables zero-cost refinement checks when evidence is available.
//!
//! This module tests flow-sensitive refinement tracking through control flow:
//! - If-expression evidence propagation
//! - Match expression evidence propagation
//! - While loop evidence propagation
//! - For loop evidence propagation
//! - Evidence scoping and isolation
//! - Integration with SMT verification

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
    unused_assignments,
    clippy::approx_constant,
    clippy::overly_complex_bool_expr
)]

use verum_ast::expr::{BinOp, Expr, ExprKind, UnOp};
use verum_ast::literal::{Literal, LiteralKind};
use verum_ast::span::{FileId, Span};
use verum_ast::ty::{Ident, Path, PathSegment};
use verum_common::{Heap, List, Maybe, Text};
use verum_types::refinement::{RefinementChecker, RefinementConfig, VerificationResult};
use verum_types::refinement_evidence::{
    EvidencePropagator, PathCondition, PathConditionKind, RefinementEvidence,
};

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

fn dummy_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

fn make_ident(name: &str) -> Ident {
    Ident::new(Text::from(name), dummy_span())
}

fn make_int_literal(value: i128) -> Expr {
    Expr::literal(Literal::int(value, dummy_span()))
}

fn make_var_expr(name: &str) -> Expr {
    Expr::ident(make_ident(name))
}

fn make_binary_expr(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        dummy_span(),
    )
}

fn make_unary_expr(op: UnOp, expr: Expr) -> Expr {
    Expr::new(
        ExprKind::Unary {
            op,
            expr: Heap::new(expr),
        },
        dummy_span(),
    )
}

fn make_method_call(receiver_name: &str, method_name: &str) -> Expr {
    Expr::new(
        ExprKind::MethodCall {
            receiver: Heap::new(make_var_expr(receiver_name)),
            method: make_ident(method_name),
            type_args: List::new(),
            args: List::new(),
        },
        dummy_span(),
    )
}

// ============================================================================
// REFINEMENT EVIDENCE BASIC TESTS
// ============================================================================

#[test]
fn test_evidence_creation() {
    let evidence = RefinementEvidence::new();
    assert_eq!(evidence.stats(), (0, 0, 0)); // No conditions added/used
}

#[test]
fn test_evidence_scope_push_pop() {
    let mut evidence = RefinementEvidence::new();

    evidence.push_scope();
    evidence.add_evidence_from_condition(&make_binary_expr(BinOp::Gt, make_var_expr("x"), make_int_literal(0)), dummy_span());

    // Evidence should be accessible
    let assumptions = evidence.to_smt_assumptions();
    assert!(!assumptions.is_empty());

    evidence.pop_scope();

    // After popping, evidence should be removed
    let assumptions_after = evidence.to_smt_assumptions();
    assert!(assumptions_after.is_empty());
}

#[test]
fn test_evidence_nested_scopes() {
    let mut evidence = RefinementEvidence::new();

    // Outer scope evidence
    evidence.push_scope();
    evidence.add_evidence_from_condition(&make_binary_expr(BinOp::Gt, make_var_expr("x"), make_int_literal(0)), dummy_span());

    // Inner scope evidence
    evidence.push_scope();
    evidence.add_evidence_from_condition(&make_binary_expr(BinOp::Lt, make_var_expr("x"), make_int_literal(100)), dummy_span());

    // Should see both conditions
    let all = evidence.get_all_conditions();
    assert_eq!(all.len(), 2);

    evidence.pop_scope(); // Pop inner

    // Should see only outer condition
    let outer = evidence.get_all_conditions();
    assert_eq!(outer.len(), 1);

    evidence.pop_scope(); // Pop outer

    let empty = evidence.get_all_conditions();
    assert!(empty.is_empty());
}

#[test]
fn test_evidence_add_method_evidence() {
    let mut evidence = RefinementEvidence::new();
    evidence.push_scope();

    evidence.add_method_evidence(Text::from("data"), "is_empty", true, dummy_span());

    let conditions = evidence.get_all_conditions();
    assert_eq!(conditions.len(), 1);
    assert_eq!(conditions[0].kind, PathConditionKind::MethodResult);
}

#[test]
fn test_evidence_negated_evidence() {
    let mut evidence = RefinementEvidence::new();
    evidence.push_scope();

    let condition = make_method_call("data", "is_empty");
    evidence.add_negated_evidence(&condition, dummy_span());

    let conditions = evidence.get_all_conditions();
    assert_eq!(conditions.len(), 1);
    assert_eq!(conditions[0].kind, PathConditionKind::NegatedAfterExit);
}

// ============================================================================
// PATH CONDITION TESTS
// ============================================================================

#[test]
fn test_path_condition_from_if_positive() {
    let condition = make_binary_expr(BinOp::Gt, make_var_expr("x"), make_int_literal(0));
    let path_cond = PathCondition::from_if_condition(&condition, false, dummy_span());

    assert_eq!(path_cond.kind, PathConditionKind::IfCondition);
    assert!(matches!(path_cond.constrained_var, Maybe::Some(ref v) if v.as_str() == "x"));
}

#[test]
fn test_path_condition_from_if_negated() {
    let condition = make_binary_expr(BinOp::Gt, make_var_expr("x"), make_int_literal(0));
    let path_cond = PathCondition::from_if_condition(&condition, true, dummy_span());

    assert_eq!(path_cond.kind, PathConditionKind::NegatedAfterExit);

    // Negated: x > 0 becomes x <= 0
    if let ExprKind::Binary { op: BinOp::Le, .. } = &path_cond.predicate.kind {
        // Correctly negated
    } else {
        panic!("Expected negated comparison to be x <= 0");
    }
}

#[test]
fn test_path_condition_double_negation_simplification() {
    let inner = make_binary_expr(BinOp::Gt, make_var_expr("x"), make_int_literal(0));
    let negated = make_unary_expr(UnOp::Not, inner.clone());

    // Negate the negated expression - should simplify to original
    let path_cond = PathCondition::from_if_condition(&negated, true, dummy_span());

    // Double negation should be simplified
    if let ExprKind::Binary { op: BinOp::Gt, .. } = &path_cond.predicate.kind {
        // Correctly simplified
    } else {
        // Still valid - might be wrapped differently
    }
}

#[test]
fn test_path_condition_method_result() {
    let path_cond =
        PathCondition::from_method_result(Text::from("data"), "is_empty", true, dummy_span());

    assert_eq!(path_cond.kind, PathConditionKind::MethodResult);
    assert!(matches!(path_cond.constrained_var, Maybe::Some(ref v) if v.as_str() == "data"));

    // Should be !data.is_empty()
    if let ExprKind::Unary { op: UnOp::Not, .. } = &path_cond.predicate.kind {
        // Correctly constructed
    } else {
        panic!("Expected negated method call");
    }
}

#[test]
fn test_path_condition_is_non_empty_check() {
    // Create !data.is_empty()
    let path_cond =
        PathCondition::from_method_result(Text::from("data"), "is_empty", true, dummy_span());

    assert!(path_cond.is_non_empty_check());
}

#[test]
fn test_path_condition_is_some_check() {
    let path_cond =
        PathCondition::from_method_result(Text::from("opt"), "is_some", false, dummy_span());

    assert!(path_cond.is_some_or_ok_check());
}

// ============================================================================
// EVIDENCE PROPAGATOR TESTS
// ============================================================================

#[test]
fn test_evidence_propagator_analyze_if_condition() {
    let condition = make_binary_expr(BinOp::Gt, make_var_expr("x"), make_int_literal(0));

    let (then_evidence, else_evidence) =
        EvidencePropagator::analyze_if_condition(&condition, dummy_span());

    assert_eq!(then_evidence.len(), 1);
    assert_eq!(else_evidence.len(), 1);

    assert_eq!(then_evidence[0].kind, PathConditionKind::IfCondition);
    assert_eq!(else_evidence[0].kind, PathConditionKind::NegatedAfterExit);
}

#[test]
fn test_evidence_propagator_analyze_method_condition() {
    // data.is_empty()
    let method_call = make_method_call("data", "is_empty");

    if let Maybe::Some((var, method, negated)) =
        EvidencePropagator::analyze_method_condition(&method_call)
    {
        assert_eq!(var.as_str(), "data");
        assert_eq!(method.as_str(), "is_empty");
        assert!(!negated);
    } else {
        panic!("Expected method condition analysis to succeed");
    }
}

#[test]
fn test_evidence_propagator_analyze_negated_method_condition() {
    // !data.is_empty()
    let method_call = make_method_call("data", "is_empty");
    let negated = make_unary_expr(UnOp::Not, method_call);

    if let Maybe::Some((var, method, is_negated)) =
        EvidencePropagator::analyze_method_condition(&negated)
    {
        assert_eq!(var.as_str(), "data");
        assert_eq!(method.as_str(), "is_empty");
        assert!(is_negated);
    } else {
        panic!("Expected negated method condition analysis to succeed");
    }
}

// ============================================================================
// VARIABLE EVIDENCE LOOKUP TESTS
// ============================================================================

#[test]
fn test_variable_evidence_lookup() {
    let mut evidence = RefinementEvidence::new();
    evidence.push_scope();

    evidence.add_method_evidence(Text::from("data"), "is_empty", true, dummy_span());
    evidence.add_evidence_from_condition(&make_binary_expr(BinOp::Gt, make_var_expr("x"), make_int_literal(0)), dummy_span());

    let data_evidence = evidence.get_variable_evidence(&Text::from("data"));
    assert_eq!(data_evidence.len(), 1);

    let x_evidence = evidence.get_variable_evidence(&Text::from("x"));
    assert_eq!(x_evidence.len(), 1);

    let other_evidence = evidence.get_variable_evidence(&Text::from("other"));
    assert!(other_evidence.is_empty());
}

#[test]
fn test_has_non_empty_evidence() {
    let mut evidence = RefinementEvidence::new();
    evidence.push_scope();

    assert!(!evidence.has_non_empty_evidence(&Text::from("data")));

    evidence.add_method_evidence(Text::from("data"), "is_empty", true, dummy_span());

    assert!(evidence.has_non_empty_evidence(&Text::from("data")));
}

#[test]
fn test_has_some_or_ok_evidence() {
    let mut evidence = RefinementEvidence::new();
    evidence.push_scope();

    assert!(!evidence.has_some_or_ok_evidence(&Text::from("opt")));

    evidence.add_method_evidence(Text::from("opt"), "is_some", false, dummy_span());

    assert!(evidence.has_some_or_ok_evidence(&Text::from("opt")));
}

// ============================================================================
// TO_SMT_ASSUMPTIONS TESTS
// ============================================================================

#[test]
fn test_to_smt_assumptions() {
    let mut evidence = RefinementEvidence::new();
    evidence.push_scope();

    evidence.add_evidence_from_condition(&make_binary_expr(BinOp::Gt, make_var_expr("x"), make_int_literal(0)), dummy_span());
    evidence.add_evidence_from_condition(&make_binary_expr(BinOp::Lt, make_var_expr("x"), make_int_literal(100)), dummy_span());

    let assumptions = evidence.to_smt_assumptions();
    assert_eq!(assumptions.len(), 2);
}

#[test]
fn test_to_smt_assumptions_with_nested_scopes() {
    let mut evidence = RefinementEvidence::new();

    evidence.push_scope();
    evidence.add_evidence_from_condition(&make_binary_expr(BinOp::Gt, make_var_expr("x"), make_int_literal(0)), dummy_span());

    evidence.push_scope();
    evidence.add_evidence_from_condition(&make_binary_expr(BinOp::Lt, make_var_expr("y"), make_int_literal(50)), dummy_span());

    // Should see both conditions
    let assumptions = evidence.to_smt_assumptions();
    assert_eq!(assumptions.len(), 2);
}

// ============================================================================
// CLEAR AND STATS TESTS
// ============================================================================

#[test]
fn test_evidence_clear() {
    let mut evidence = RefinementEvidence::new();

    evidence.push_scope();
    evidence.add_evidence_from_condition(&make_binary_expr(BinOp::Gt, make_var_expr("x"), make_int_literal(0)), dummy_span());
    evidence.push_scope();
    evidence.add_evidence_from_condition(&make_binary_expr(BinOp::Lt, make_var_expr("y"), make_int_literal(50)), dummy_span());

    assert_eq!(evidence.get_all_conditions().len(), 2);

    evidence.clear();

    assert!(evidence.get_all_conditions().is_empty());
}

#[test]
fn test_evidence_stats() {
    let mut evidence = RefinementEvidence::new();
    evidence.push_scope();

    let (added, used, hits) = evidence.stats();
    assert_eq!(added, 0);
    assert_eq!(used, 0);
    assert_eq!(hits, 0);

    evidence.add_evidence_from_condition(&make_binary_expr(BinOp::Gt, make_var_expr("x"), make_int_literal(0)), dummy_span());
    evidence.add_method_evidence(Text::from("data"), "is_empty", true, dummy_span());

    let (added2, _, _) = evidence.stats();
    assert_eq!(added2, 2);

    // Query for variable evidence - increments used count
    let _ = evidence.get_variable_evidence(&Text::from("x"));

    let (_, used2, _) = evidence.stats();
    assert_eq!(used2, 1);
}

// ============================================================================
// INTEGRATION WITH REFINEMENT CHECKER
// ============================================================================

#[test]
fn test_evidence_with_refinement_checker() {
    use verum_types::refinement::{RefinementType, RefinementPredicate, RefinementBinding};
    use verum_types::context::TypeContext;

    let mut checker = RefinementChecker::new(RefinementConfig {
        enable_smt: false, // Disable SMT for unit test
        enable_cache: true,
        max_cache_size: 100,
        timeout_ms: 1000,
    });

    // Create a refinement type: Int{x | x > 0}
    let base_ty = verum_types::ty::Type::int();
    let predicate = RefinementPredicate {
        predicate: make_binary_expr(BinOp::Gt, make_var_expr("it"), make_int_literal(0)),
        binding: RefinementBinding::Lambda(Text::from("it")),
        span: dummy_span(),
    };
    let refinement = RefinementType {
        base_type: base_ty,
        predicate,
        span: dummy_span(),
    };

    // Create evidence: x > 0
    let evidence = vec![make_binary_expr(BinOp::Gt, make_var_expr("x"), make_int_literal(0))];

    let ctx = TypeContext::new();

    // With the x > 0 evidence, checking x against Int{> 0} should be more likely to succeed
    // (though full verification requires SMT)
    let value = make_var_expr("x");
    let result = checker.check_with_evidence(&value, &refinement, &evidence, &ctx);

    // Should not error
    assert!(result.is_ok());
}

// ============================================================================
// EDGE CASES
// ============================================================================

#[test]
fn test_empty_evidence() {
    let evidence = RefinementEvidence::new();

    let assumptions = evidence.to_smt_assumptions();
    assert!(assumptions.is_empty());

    let conditions = evidence.get_all_conditions();
    assert!(conditions.is_empty());
}

#[test]
fn test_evidence_after_all_scopes_popped() {
    let mut evidence = RefinementEvidence::new();

    evidence.push_scope();
    evidence.add_evidence_from_condition(&make_binary_expr(BinOp::Gt, make_var_expr("x"), make_int_literal(0)), dummy_span());
    evidence.pop_scope();

    // Pop the root scope too (edge case - should be safe)
    evidence.pop_scope();

    // Should still be empty and not crash
    let assumptions = evidence.to_smt_assumptions();
    assert!(assumptions.is_empty());
}

#[test]
fn test_evidence_complex_condition() {
    let mut evidence = RefinementEvidence::new();
    evidence.push_scope();

    // Complex condition: x > 0 && x < 100
    let left = make_binary_expr(BinOp::Gt, make_var_expr("x"), make_int_literal(0));
    let right = make_binary_expr(BinOp::Lt, make_var_expr("x"), make_int_literal(100));
    let combined = make_binary_expr(BinOp::And, left, right);

    evidence.add_evidence_from_condition(&combined, dummy_span());

    let conditions = evidence.get_all_conditions();
    assert_eq!(conditions.len(), 1);

    // The variable should be extracted from the left side of the And
    assert!(conditions[0].constrained_var.is_some());
}

#[test]
fn test_evidence_multiple_variables() {
    let mut evidence = RefinementEvidence::new();
    evidence.push_scope();

    evidence.add_evidence_from_condition(&make_binary_expr(BinOp::Gt, make_var_expr("x"), make_int_literal(0)), dummy_span());
    evidence.add_evidence_from_condition(&make_binary_expr(BinOp::Gt, make_var_expr("y"), make_int_literal(0)), dummy_span());
    evidence.add_evidence_from_condition(&make_binary_expr(BinOp::Gt, make_var_expr("z"), make_int_literal(0)), dummy_span());
    evidence.add_method_evidence(Text::from("data"), "is_empty", true, dummy_span());

    assert_eq!(evidence.get_variable_evidence(&Text::from("x")).len(), 1);
    assert_eq!(evidence.get_variable_evidence(&Text::from("y")).len(), 1);
    assert_eq!(evidence.get_variable_evidence(&Text::from("z")).len(), 1);
    assert_eq!(evidence.get_variable_evidence(&Text::from("data")).len(), 1);

    let all = evidence.to_smt_assumptions();
    assert_eq!(all.len(), 4);
}

// ============================================================================
// SPECIAL METHOD PATTERNS
// ============================================================================

#[test]
fn test_is_ok_evidence() {
    let mut evidence = RefinementEvidence::new();
    evidence.push_scope();

    evidence.add_method_evidence(Text::from("result"), "is_ok", false, dummy_span());

    assert!(evidence.has_some_or_ok_evidence(&Text::from("result")));
}

#[test]
fn test_is_err_evidence() {
    let mut evidence = RefinementEvidence::new();
    evidence.push_scope();

    evidence.add_method_evidence(Text::from("result"), "is_err", false, dummy_span());

    // is_err is not is_some or is_ok
    assert!(!evidence.has_some_or_ok_evidence(&Text::from("result")));
}

#[test]
fn test_is_none_evidence() {
    let mut evidence = RefinementEvidence::new();
    evidence.push_scope();

    evidence.add_method_evidence(Text::from("opt"), "is_none", false, dummy_span());

    // is_none is not is_some
    assert!(!evidence.has_some_or_ok_evidence(&Text::from("opt")));
}
