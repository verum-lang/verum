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
// Comprehensive Refinement Type Integration Tests
//
// Tests refinement type verification including:
// - Basic refinement type checking
// - Subsumption (subtyping) with refinements
// - Function precondition/postcondition verification
// - SMT integration
// - Counterexample generation
// - Performance characteristics

use verum_ast::{
    expr::*,
    literal::{IntLit, Literal, LiteralKind},
    span::Span,
    ty::{Ident, Path, PathSegment},
};
use verum_common::Text;
use verum_types::context::TypeContext;
use verum_types::refinement::*;
use verum_types::ty::Type;

// ============================================================================
// Helper Functions
// ============================================================================

fn var_expr(name: &str, span: Span) -> Expr {
    Expr::ident(Ident::new(name, span))
}

fn int_literal(value: i128, span: Span) -> Expr {
    Expr::literal(Literal {
        kind: LiteralKind::Int(IntLit {
            value,
            suffix: None,
        }),
        span,
    })
}

fn binary_expr(op: BinOp, left: Expr, right: Expr, span: Span) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        span,
    )
}

// ============================================================================
// Basic Refinement Type Tests
// ============================================================================

#[test]
fn test_positive_int_verification() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    // type Positive = Int{> 0}
    let predicate_expr = binary_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);
    let predicate = RefinementPredicate::inline(predicate_expr, span);
    let positive_type = RefinementType::refined(Type::int(), predicate, span);

    // Test: 42 satisfies Int{> 0}
    let value = int_literal(42, span);
    let result = checker.check(&value, &positive_type, &ctx);

    assert!(result.is_ok(), "Verification should succeed");
    let verification_result = result.unwrap();
    assert!(
        verification_result.is_valid(),
        "42 > 0 should be valid: {:?}",
        verification_result
    );
}

#[test]
fn test_positive_int_failure() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    // type Positive = Int{> 0}
    let predicate_expr = binary_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);
    let predicate = RefinementPredicate::inline(predicate_expr, span);
    let positive_type = RefinementType::refined(Type::int(), predicate, span);

    // Test: -5 does NOT satisfy Int{> 0}
    let value = int_literal(-5, span);
    let result = checker.check(&value, &positive_type, &ctx);

    assert!(result.is_ok(), "Verification should run");
    let verification_result = result.unwrap();
    assert!(
        !verification_result.is_valid(),
        "-5 > 0 should be invalid: {:?}",
        verification_result
    );
}

#[test]
fn test_non_zero_int_verification() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    // type NonZero = Int{!= 0}
    let predicate_expr = binary_expr(BinOp::Ne, var_expr("it", span), int_literal(0, span), span);
    let predicate = RefinementPredicate::inline(predicate_expr, span);
    let non_zero_type = RefinementType::refined(Type::int(), predicate, span);

    // Test: 5 != 0
    let value = int_literal(5, span);
    let result = checker.check(&value, &non_zero_type, &ctx);
    assert!(result.is_ok());
    assert!(result.unwrap().is_valid());

    // Test: -5 != 0
    let value = int_literal(-5, span);
    let result = checker.check(&value, &non_zero_type, &ctx);
    assert!(result.is_ok());
    assert!(result.unwrap().is_valid());

    // Test: 0 != 0 (should fail)
    let value = int_literal(0, span);
    let result = checker.check(&value, &non_zero_type, &ctx);
    assert!(result.is_ok());
    assert!(!result.unwrap().is_valid());
}

#[test]
fn test_bounded_int_verification() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    // type Percentage = Int{0 <= it && it <= 100}
    let lower_bound = binary_expr(BinOp::Ge, var_expr("it", span), int_literal(0, span), span);
    let upper_bound = binary_expr(
        BinOp::Le,
        var_expr("it", span),
        int_literal(100, span),
        span,
    );
    let predicate_expr = binary_expr(BinOp::And, lower_bound, upper_bound, span);
    let predicate = RefinementPredicate::inline(predicate_expr, span);
    let percentage_type = RefinementType::refined(Type::int(), predicate, span);

    // Test: 50 is in range [0, 100]
    let value = int_literal(50, span);
    let result = checker.check(&value, &percentage_type, &ctx);
    assert!(result.is_ok());
    assert!(result.unwrap().is_valid());

    // Test: 0 is in range [0, 100]
    let value = int_literal(0, span);
    let result = checker.check(&value, &percentage_type, &ctx);
    assert!(result.is_ok());
    assert!(result.unwrap().is_valid());

    // Test: 100 is in range [0, 100]
    let value = int_literal(100, span);
    let result = checker.check(&value, &percentage_type, &ctx);
    assert!(result.is_ok());
    assert!(result.unwrap().is_valid());

    // Test: 150 is NOT in range [0, 100]
    let value = int_literal(150, span);
    let result = checker.check(&value, &percentage_type, &ctx);
    assert!(result.is_ok());
    assert!(!result.unwrap().is_valid());

    // Test: -10 is NOT in range [0, 100]
    let value = int_literal(-10, span);
    let result = checker.check(&value, &percentage_type, &ctx);
    assert!(result.is_ok());
    assert!(!result.unwrap().is_valid());
}

// ============================================================================
// Subsumption (Subtyping) Tests
// ============================================================================

#[test]
fn test_refinement_subsumption_simple() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());

    // type StrictPositive = Int{> 10}
    let strict_pred = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(10, span), span);
    let strict_type = RefinementType::refined(
        Type::int(),
        RefinementPredicate::lambda(strict_pred, "x".into(), span),
        span,
    );

    // type Positive = Int{> 0}
    let weak_pred = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);
    let weak_type = RefinementType::refined(
        Type::int(),
        RefinementPredicate::lambda(weak_pred, "x".into(), span),
        span,
    );

    // Test: Int{> 10} <: Int{> 0}
    // (x > 10) implies (x > 0)
    let result = checker.check_subsumption(&strict_type, &weak_type);
    assert!(result.is_ok(), "Subsumption check should succeed");
    assert!(
        result.unwrap(),
        "Int{{> 10}} should be a subtype of Int{{> 0}}"
    );
}

#[test]
fn test_refinement_subsumption_transitive() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());

    // type VeryStrict = Int{> 100}
    let very_strict_pred =
        binary_expr(BinOp::Gt, var_expr("x", span), int_literal(100, span), span);
    let very_strict_type = RefinementType::refined(
        Type::int(),
        RefinementPredicate::lambda(very_strict_pred, "x".into(), span),
        span,
    );

    // type Strict = Int{> 10}
    let strict_pred = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(10, span), span);
    let strict_type = RefinementType::refined(
        Type::int(),
        RefinementPredicate::lambda(strict_pred, "x".into(), span),
        span,
    );

    // type Weak = Int{> 0}
    let weak_pred = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);
    let weak_type = RefinementType::refined(
        Type::int(),
        RefinementPredicate::lambda(weak_pred, "x".into(), span),
        span,
    );

    // Test: Int{> 100} <: Int{> 10}
    let result = checker.check_subsumption(&very_strict_type, &strict_type);
    assert!(result.is_ok());
    assert!(result.unwrap(), "Int{{> 100}} <: Int{{> 10}}");

    // Test: Int{> 10} <: Int{> 0}
    let result = checker.check_subsumption(&strict_type, &weak_type);
    assert!(result.is_ok());
    assert!(result.unwrap(), "Int{{> 10}} <: Int{{> 0}}");

    // Test: Int{> 100} <: Int{> 0} (transitivity)
    let result = checker.check_subsumption(&very_strict_type, &weak_type);
    assert!(result.is_ok());
    assert!(result.unwrap(), "Int{{> 100}} <: Int{{> 0}}");
}

#[test]
fn test_refinement_subsumption_negative() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());

    // type Positive = Int{> 0}
    let positive_pred = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);
    let positive_type = RefinementType::refined(
        Type::int(),
        RefinementPredicate::lambda(positive_pred, "x".into(), span),
        span,
    );

    // type StrictPositive = Int{> 10}
    let strict_pred = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(10, span), span);
    let strict_type = RefinementType::refined(
        Type::int(),
        RefinementPredicate::lambda(strict_pred, "x".into(), span),
        span,
    );

    // Test: Int{> 0} <: Int{> 10} should FAIL
    // (x > 0) does NOT imply (x > 10)
    let result = checker.check_subsumption(&positive_type, &strict_type);
    assert!(result.is_ok());
    assert!(
        !result.unwrap(),
        "Int{{> 0}} should NOT be a subtype of Int{{> 10}}"
    );
}

#[test]
fn test_unrefined_subsumption() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());

    // Unrefined Int
    let unrefined = RefinementType::unrefined(Type::int(), span);

    // type Positive = Int{> 0}
    let refined_pred = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);
    let refined = RefinementType::refined(
        Type::int(),
        RefinementPredicate::lambda(refined_pred, "x".into(), span),
        span,
    );

    // Test: Int{> 0} <: Int (refined <: unrefined)
    let result = checker.check_subsumption(&refined, &unrefined);
    assert!(result.is_ok());
    assert!(
        result.unwrap(),
        "Refined type should be subtype of unrefined"
    );

    // Test: Int <: Int{> 0} should FAIL (unrefined <: refined)
    let result = checker.check_subsumption(&unrefined, &refined);
    assert!(result.is_ok());
    assert!(
        !result.unwrap(),
        "Unrefined should NOT be subtype of refined"
    );
}

// ============================================================================
// Five Binding Rules Tests
// ============================================================================

#[test]
fn test_inline_binding() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    // type Positive = Int{> 0} (inline, implicit 'it')
    let predicate_expr = binary_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);
    let predicate = RefinementPredicate::inline(predicate_expr, span);

    assert_eq!(predicate.bound_variable(), Text::from("it"));
    assert!(matches!(predicate.binding, RefinementBinding::Inline));

    let refined_type = RefinementType::refined(Type::int(), predicate, span);
    let value = int_literal(5, span);
    let result = checker.check(&value, &refined_type, &ctx);

    assert!(result.is_ok());
    assert!(result.unwrap().is_valid());
}

#[test]
fn test_lambda_binding() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    // type Positive = Int where |x| x > 0 (lambda-style)
    let predicate_expr = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);
    let predicate = RefinementPredicate::lambda(predicate_expr, "x".into(), span);

    assert_eq!(predicate.bound_variable(), Text::from("x"));
    assert!(matches!(predicate.binding, RefinementBinding::Lambda(_)));

    let refined_type = RefinementType::refined(Type::int(), predicate, span);
    let value = int_literal(5, span);
    let result = checker.check(&value, &refined_type, &ctx);

    assert!(result.is_ok());
    assert!(result.unwrap().is_valid());
}

#[test]
fn test_sigma_binding() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    // type Positive = x: Int where x > 0 (sigma-type, dependent)
    let predicate_expr = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);
    let predicate = RefinementPredicate::sigma(predicate_expr, "x".into(), span);

    assert_eq!(predicate.bound_variable(), Text::from("x"));
    assert!(matches!(predicate.binding, RefinementBinding::Sigma(_)));

    let refined_type = RefinementType::refined(Type::int(), predicate, span);
    let value = int_literal(5, span);
    let result = checker.check(&value, &refined_type, &ctx);

    assert!(result.is_ok());
    assert!(result.unwrap().is_valid());
}

#[test]
fn test_named_predicate_binding() {
    let span = Span::dummy();

    // type Positive = Int where is_positive (named predicate)
    let predicate_path = Path {
        segments: vec![PathSegment::Name(Ident::new("is_positive", span))].into(),
        span,
    };
    let predicate = RefinementPredicate::named(predicate_path.clone(), span);

    assert_eq!(predicate.bound_variable(), Text::from("it"));
    assert!(matches!(predicate.binding, RefinementBinding::Named(_)));

    // Verify that a call expression was created
    if let ExprKind::Call { func, args, .. } = &predicate.predicate.kind {
        assert_eq!(args.len(), 1);
        // Function is the path to the predicate
        if let ExprKind::Path(path) = &func.kind {
            assert_eq!(path.segments.len(), 1);
        }
    } else {
        panic!("Expected Call expression for named predicate");
    }
}

#[test]
fn test_bare_where_binding() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    // type Positive = Int where it > 0 (bare where, deprecated)
    let predicate_expr = binary_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);
    let predicate = RefinementPredicate::bare(predicate_expr, span);

    assert_eq!(predicate.bound_variable(), Text::from("it"));
    assert!(matches!(predicate.binding, RefinementBinding::Bare));

    let refined_type = RefinementType::refined(Type::int(), predicate, span);
    let value = int_literal(5, span);
    let result = checker.check(&value, &refined_type, &ctx);

    assert!(result.is_ok());
    assert!(result.unwrap().is_valid());
}

// ============================================================================
// Performance and Statistics Tests
// ============================================================================

#[test]
fn test_verification_statistics() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    let predicate_expr = binary_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);
    let predicate = RefinementPredicate::inline(predicate_expr, span);
    let positive_type = RefinementType::refined(Type::int(), predicate, span);

    // Run multiple checks
    for i in 1..=10 {
        let value = int_literal(i, span);
        let _ = checker.check(&value, &positive_type, &ctx);
    }

    let stats = checker.stats();
    assert_eq!(stats.total_checks, 10);
    assert!(stats.successful > 0);
    assert_eq!(stats.failed, 0); // All should succeed
}

#[test]
fn test_cache_behavior() {
    let span = Span::dummy();
    let config = RefinementConfig {
        enable_cache: true,
        enable_smt: true,
        timeout_ms: 100,
        max_cache_size: 100,
    };
    let mut checker = RefinementChecker::new(config);
    let ctx = TypeContext::new();

    let predicate_expr = binary_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);
    let predicate = RefinementPredicate::inline(predicate_expr, span);
    let positive_type = RefinementType::refined(Type::int(), predicate, span);

    // First check - cache miss
    let value = int_literal(42, span);
    let _ = checker.check(&value, &positive_type, &ctx);

    // Second check - same value, should hit cache (if SMT was used)
    let _ = checker.check(&value, &positive_type, &ctx);

    let stats = checker.stats();
    // Note: Cache is only used for SMT results, not syntactic checks
    // So cache_hits might be 0 if syntactic check succeeded
    assert!(stats.total_checks >= 2);
}

#[test]
fn test_trivial_refinement() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    // Trivial refinement (always true)
    let trivial = RefinementPredicate::trivial(span);
    assert!(trivial.is_trivial());

    let trivial_type = RefinementType::refined(Type::int(), trivial, span);
    assert!(trivial_type.is_unrefined());

    // Any value should satisfy trivial refinement
    let value = int_literal(42, span);
    let result = checker.check(&value, &trivial_type, &ctx);

    assert!(result.is_ok());
    assert!(result.unwrap().is_valid());
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_refinement_error_generation() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    // type Positive = Int{> 0}
    let predicate_expr = binary_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);
    let predicate = RefinementPredicate::inline(predicate_expr, span);
    let positive_type = RefinementType::refined(Type::int(), predicate, span);

    // Test with invalid value: -5 > 0 is false
    let value = int_literal(-5, span);
    let result = checker.check(&value, &positive_type, &ctx);

    assert!(result.is_ok());
    let verification_result = result.unwrap();
    assert!(!verification_result.is_valid());

    match verification_result {
        VerificationResult::Invalid { counterexample: _ } => {
            // Counterexample might be None for syntactic checks
            // but the result should be invalid
        }
        _ => panic!("Expected Invalid result"),
    }
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_equality_refinement() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    // type Five = Int{== 5}
    let predicate_expr = binary_expr(BinOp::Eq, var_expr("it", span), int_literal(5, span), span);
    let predicate = RefinementPredicate::inline(predicate_expr, span);
    let five_type = RefinementType::refined(Type::int(), predicate, span);

    // Test: 5 == 5
    let value = int_literal(5, span);
    let result = checker.check(&value, &five_type, &ctx);
    assert!(result.is_ok());
    assert!(result.unwrap().is_valid());

    // Test: 6 == 5 (should fail)
    let value = int_literal(6, span);
    let result = checker.check(&value, &five_type, &ctx);
    assert!(result.is_ok());
    assert!(!result.unwrap().is_valid());
}

#[test]
fn test_comparison_operators() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    let ops_and_values = vec![
        (BinOp::Gt, 10, 11, true),  // 11 > 10
        (BinOp::Gt, 10, 10, false), // 10 > 10
        (BinOp::Ge, 10, 10, true),  // 10 >= 10
        (BinOp::Ge, 10, 11, true),  // 11 >= 10
        (BinOp::Lt, 10, 9, true),   // 9 < 10
        (BinOp::Lt, 10, 10, false), // 10 < 10
        (BinOp::Le, 10, 10, true),  // 10 <= 10
        (BinOp::Le, 10, 9, true),   // 9 <= 10
        (BinOp::Eq, 10, 10, true),  // 10 == 10
        (BinOp::Eq, 10, 11, false), // 11 == 10
        (BinOp::Ne, 10, 11, true),  // 11 != 10
        (BinOp::Ne, 10, 10, false), // 10 != 10
    ];

    for (op, bound, value, expected_valid) in ops_and_values {
        let predicate_expr = binary_expr(op, var_expr("it", span), int_literal(bound, span), span);
        let predicate = RefinementPredicate::inline(predicate_expr, span);
        let refined_type = RefinementType::refined(Type::int(), predicate, span);

        let test_value = int_literal(value, span);
        let result = checker.check(&test_value, &refined_type, &ctx);

        assert!(result.is_ok(), "Check for {:?} failed", op);
        assert_eq!(
            result.unwrap().is_valid(),
            expected_valid,
            "Expected {} {:?} {} to be {}",
            value,
            op,
            bound,
            expected_valid
        );
    }
}

// ============================================================================
// Evidence-Aware Verification Tests
// ============================================================================

#[test]
fn test_check_with_evidence_basic() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    // type Positive = Int{> 0}
    let predicate_expr = binary_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);
    let predicate = RefinementPredicate::inline(predicate_expr, span);
    let positive_type = RefinementType::refined(Type::int(), predicate, span);

    // With evidence: we know x > 0
    let evidence = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);
    let value = var_expr("x", span);
    let result = checker.check_with_evidence(&value, &positive_type, &[evidence], &ctx);
    assert!(result.is_ok());
    assert!(
        result.unwrap().is_valid(),
        "With evidence x > 0, variable x should satisfy Int{{> 0}}"
    );
}

#[test]
fn test_check_with_evidence_stronger() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    // type NonNegative = Int{>= 0}
    let predicate_expr = binary_expr(BinOp::Ge, var_expr("it", span), int_literal(0, span), span);
    let predicate = RefinementPredicate::inline(predicate_expr, span);
    let non_negative = RefinementType::refined(Type::int(), predicate, span);

    // Evidence: x > 5 (stronger than >= 0)
    let evidence = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(5, span), span);
    let value = var_expr("x", span);
    let result = checker.check_with_evidence(&value, &non_negative, &[evidence], &ctx);
    assert!(result.is_ok());
    assert!(
        result.unwrap().is_valid(),
        "With evidence x > 5, variable x should satisfy Int{{>= 0}}"
    );
}

// ============================================================================
// SMT-Based Subsumption Tests
// ============================================================================

#[test]
fn test_smt_subsumption_equality_implies_ge() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());

    // Int{== 5}
    let eq5_pred = binary_expr(BinOp::Eq, var_expr("x", span), int_literal(5, span), span);
    let eq5_type = RefinementType::refined(
        Type::int(),
        RefinementPredicate::lambda(eq5_pred, "x".into(), span),
        span,
    );

    // Int{>= 0}
    let ge0_pred = binary_expr(BinOp::Ge, var_expr("x", span), int_literal(0, span), span);
    let ge0_type = RefinementType::refined(
        Type::int(),
        RefinementPredicate::lambda(ge0_pred, "x".into(), span),
        span,
    );

    let result = checker.check_subsumption(&eq5_type, &ge0_type);
    assert!(result.is_ok());
    assert!(result.unwrap(), "Int{{== 5}} should be subtype of Int{{>= 0}}");
}

#[test]
fn test_smt_subsumption_le_directions() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());

    let lt5_pred = binary_expr(BinOp::Lt, var_expr("x", span), int_literal(5, span), span);
    let lt5_type = RefinementType::refined(
        Type::int(),
        RefinementPredicate::lambda(lt5_pred, "x".into(), span),
        span,
    );

    let lt10_pred = binary_expr(BinOp::Lt, var_expr("x", span), int_literal(10, span), span);
    let lt10_type = RefinementType::refined(
        Type::int(),
        RefinementPredicate::lambda(lt10_pred, "x".into(), span),
        span,
    );

    let result = checker.check_subsumption(&lt5_type, &lt10_type);
    assert!(result.is_ok());
    assert!(result.unwrap(), "Int{{< 5}} should be subtype of Int{{< 10}}");

    let result = checker.check_subsumption(&lt10_type, &lt5_type);
    assert!(result.is_ok());
    assert!(!result.unwrap(), "Int{{< 10}} should NOT be subtype of Int{{< 5}}");
}

// ============================================================================
// Evidence Propagation Tests
// ============================================================================

#[test]
fn test_evidence_propagation_scope() {
    use verum_types::refinement_evidence::*;

    let mut evidence = RefinementEvidence::new();
    let span = Span::dummy();

    let outer = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);
    evidence.add_evidence_from_condition(&outer, span);

    evidence.push_scope();
    let inner = binary_expr(BinOp::Lt, var_expr("x", span), int_literal(100, span), span);
    evidence.add_evidence_from_condition(&inner, span);

    assert_eq!(evidence.get_all_conditions().len(), 2);

    evidence.pop_scope();
    assert_eq!(evidence.get_all_conditions().len(), 1);
}

#[test]
fn test_evidence_negation() {
    use verum_types::refinement_evidence::*;

    let mut evidence = RefinementEvidence::new();
    let span = Span::dummy();

    let condition = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);
    evidence.add_negated_evidence(&condition, span);

    let conditions = evidence.get_all_conditions();
    assert_eq!(conditions.len(), 1);
    if let ExprKind::Binary { op, .. } = &conditions[0].predicate.kind {
        assert_eq!(*op, BinOp::Le, "Negation of > should be <=");
    }
}

#[test]
fn test_refinement_type_base_mismatch_subsumption() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());

    let int_pred = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);
    let int_refined = RefinementType::refined(
        Type::int(),
        RefinementPredicate::lambda(int_pred, "x".into(), span),
        span,
    );

    let float_pred = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);
    let float_refined = RefinementType::refined(
        Type::float(),
        RefinementPredicate::lambda(float_pred, "x".into(), span),
        span,
    );

    let result = checker.check_subsumption(&int_refined, &float_refined);
    assert!(result.is_ok());
    assert!(!result.unwrap(), "Different base types should not be subtypes");
}

#[test]
fn test_verification_stats_tracking() {
    let span = Span::dummy();
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();

    let predicate_expr = binary_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);
    let predicate = RefinementPredicate::inline(predicate_expr, span);
    let positive_type = RefinementType::refined(Type::int(), predicate, span);

    let _ = checker.check(&int_literal(42, span), &positive_type, &ctx);
    let _ = checker.check(&int_literal(-1, span), &positive_type, &ctx);
    let unrefined = RefinementType::unrefined(Type::int(), span);
    let _ = checker.check(&int_literal(0, span), &unrefined, &ctx);

    let stats = checker.stats();
    assert!(stats.total_checks >= 3);
    assert!(stats.successful >= 2);
}

#[test]
fn test_path_condition_from_if() {
    use verum_types::refinement_evidence::*;

    let span = Span::dummy();
    let condition = binary_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);

    let pos = PathCondition::from_if_condition(&condition, false, span);
    assert_eq!(pos.kind, PathConditionKind::IfCondition);
    assert!(pos.constrains_variable(&"x".into()));

    let neg = PathCondition::from_if_condition(&condition, true, span);
    assert_eq!(neg.kind, PathConditionKind::NegatedAfterExit);
}
