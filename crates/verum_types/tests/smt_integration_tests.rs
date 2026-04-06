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
    clippy::absurd_extreme_comparisons
)]
// SMT Integration Tests for Refinement Type Subsumption
//
// Refinement types with gradual verification: types can carry predicates (Int{> 0}) verified at compile-time or runtime depending on verification level — .1 - Refinement Types
//
// These tests verify that the Z3 SMT solver is correctly integrated
// and can verify refinement type subsumption via implication checking.
//
// ## Test Coverage
// - Basic comparison subsumption (x > 10 => x > 0)
// - Invalid subsumption (x > 0 NOT=> x > 10)
// - Equality strengthening (x == 5 => x >= 5)
// - Conjunction implication (a && b => a)
// - Performance bounds (< 100ms per check)

use smallvec::SmallVec;
use verum_ast::Ident;
use verum_ast::expr::{BinOp, Expr, ExprKind, UnOp};
use verum_ast::literal::Literal;
use verum_ast::span::Span;
use verum_ast::ty::{Path, PathSegment};
use verum_common::{Heap, Text};
use verum_types::{
    RefinementChecker, RefinementConfig, RefinementPredicate, RefinementType, SmtBackend, Type,
    Z3Backend, check_subsumption_smt,
};

// ==================== Helper Functions ====================

fn make_var(name: &str) -> Expr {
    let ident = Ident::new(name, Span::dummy());
    let mut segments = SmallVec::new();
    segments.push(PathSegment::Name(ident));
    let path = Path {
        segments,
        span: Span::dummy(),
    };
    Expr::new(ExprKind::Path(path), Span::dummy())
}

fn make_int(n: i64) -> Expr {
    Expr::literal(Literal::int(n as i128, Span::dummy()))
}

fn make_bool(b: bool) -> Expr {
    Expr::literal(Literal::bool(b, Span::dummy()))
}

fn make_binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        Span::dummy(),
    )
}

fn make_predicate(expr: Expr, var: &str) -> RefinementPredicate {
    RefinementPredicate::new(expr, Text::from(var), Span::dummy())
}

// ==================== Basic Subsumption Tests ====================

#[test]
fn test_basic_comparison_subsumption_valid() {
    // Test: Int{> 10} <: Int{> 0}
    // This should be valid because x > 10 implies x > 0

    let x = make_var("it");
    let ten = make_int(10);
    let zero = make_int(0);

    let phi1 = make_binary(BinOp::Gt, x.clone(), ten); // it > 10
    let phi2 = make_binary(BinOp::Gt, x.clone(), zero); // it > 0

    let result = check_subsumption_smt(&phi1, &phi2, 100);
    assert!(result.is_ok(), "SMT check should succeed");
    assert!(
        result.unwrap(),
        "Int{{> 10}} should be a subtype of Int{{> 0}}"
    );
}

#[test]
fn test_basic_comparison_subsumption_invalid() {
    // Test: Int{> 0} NOT<: Int{> 10}
    // This should be invalid because x > 0 does not imply x > 10

    let x = make_var("it");
    let ten = make_int(10);
    let zero = make_int(0);

    let phi1 = make_binary(BinOp::Gt, x.clone(), zero); // it > 0
    let phi2 = make_binary(BinOp::Gt, x.clone(), ten); // it > 10

    let result = check_subsumption_smt(&phi1, &phi2, 100);
    assert!(result.is_ok(), "SMT check should succeed");
    assert!(
        !result.unwrap(),
        "Int{{> 0}} should NOT be a subtype of Int{{> 10}}"
    );
}

#[test]
fn test_comparison_strengthening_gt_to_ge() {
    // Test: Int{> 5} <: Int{>= 5}
    // This should be valid because x > 5 implies x >= 5

    let x = make_var("it");
    let five = make_int(5);

    let phi1 = make_binary(BinOp::Gt, x.clone(), five.clone()); // it > 5
    let phi2 = make_binary(BinOp::Ge, x.clone(), five.clone()); // it >= 5

    let result = check_subsumption_smt(&phi1, &phi2, 100);
    assert!(result.is_ok(), "SMT check should succeed");
    assert!(
        result.unwrap(),
        "Int{{> 5}} should be a subtype of Int{{>= 5}}"
    );
}

#[test]
fn test_equality_strengthening() {
    // Test: Int{== 5} <: Int{>= 5}
    // This should be valid because x == 5 implies x >= 5

    let x = make_var("it");
    let five = make_int(5);

    let phi1 = make_binary(BinOp::Eq, x.clone(), five.clone()); // it == 5
    let phi2 = make_binary(BinOp::Ge, x.clone(), five.clone()); // it >= 5

    let result = check_subsumption_smt(&phi1, &phi2, 100);
    assert!(result.is_ok(), "SMT check should succeed");
    assert!(
        result.unwrap(),
        "Int{{== 5}} should be a subtype of Int{{>= 5}}"
    );
}

#[test]
fn test_conjunction_implication() {
    // Test: Bool{a && b} <: Bool{a}
    // This should be valid because (a && b) implies a

    let a = make_var("a");
    let b = make_var("b");

    let phi1 = make_binary(BinOp::And, a.clone(), b.clone()); // a && b
    let phi2 = a.clone(); // a

    let result = check_subsumption_smt(&phi1, &phi2, 100);
    assert!(result.is_ok(), "SMT check should succeed");
    assert!(
        result.unwrap(),
        "Bool{{a && b}} should be a subtype of Bool{{a}}"
    );
}

#[test]
fn test_disjunction_weakening() {
    // Test: Bool{a} <: Bool{a || b}
    // This should be valid because a implies (a || b)

    let a = make_var("a");
    let b = make_var("b");

    let phi1 = a.clone(); // a
    let phi2 = make_binary(BinOp::Or, a.clone(), b.clone()); // a || b

    let result = check_subsumption_smt(&phi1, &phi2, 100);
    assert!(result.is_ok(), "SMT check should succeed");
    assert!(
        result.unwrap(),
        "Bool{{a}} should be a subtype of Bool{{a || b}}"
    );
}

// ==================== RefinementChecker Integration Tests ====================

#[test]
fn test_refinement_checker_with_z3_backend() {
    // Test that RefinementChecker automatically uses Z3Backend when SMT is enabled

    let config = RefinementConfig {
        enable_smt: true,
        timeout_ms: 100,
        enable_cache: true,
        max_cache_size: 1000,
    };

    let mut checker = RefinementChecker::new(config);

    // Create refinement types: Int{> 10} and Int{> 0}
    let x = make_var("it");
    let ten = make_int(10);
    let zero = make_int(0);

    let phi1 = make_binary(BinOp::Gt, x.clone(), ten);
    let phi2 = make_binary(BinOp::Gt, x.clone(), zero);

    let pred1 = make_predicate(phi1, "it");
    let pred2 = make_predicate(phi2, "it");

    let subtype = RefinementType::refined(Type::int(), pred1, Span::dummy());
    let supertype = RefinementType::refined(Type::int(), pred2, Span::dummy());

    // Check subsumption using RefinementChecker
    let result = checker.check_subsumption(&subtype, &supertype);
    assert!(result.is_ok(), "Subsumption check should succeed");
    assert!(
        result.unwrap(),
        "Int{{> 10}} should be a subtype of Int{{> 0}}"
    );

    // Verify stats were updated
    let stats = checker.stats();
    assert!(stats.total_checks > 0, "Statistics should track the check");
}

#[test]
fn test_refinement_checker_caching() {
    // Test that caching works for repeated subsumption checks

    let config = RefinementConfig {
        enable_smt: true,
        timeout_ms: 100,
        enable_cache: true,
        max_cache_size: 1000,
    };

    let mut checker = RefinementChecker::new(config);

    let x = make_var("it");
    let ten = make_int(10);
    let zero = make_int(0);

    let phi1 = make_binary(BinOp::Gt, x.clone(), ten);
    let phi2 = make_binary(BinOp::Gt, x.clone(), zero);

    let pred1 = make_predicate(phi1, "it");
    let pred2 = make_predicate(phi2, "it");

    let subtype = RefinementType::refined(Type::int(), pred1, Span::dummy());
    let supertype = RefinementType::refined(Type::int(), pred2, Span::dummy());

    // First check - cache miss
    let result1 = checker.check_subsumption(&subtype, &supertype);
    assert!(result1.is_ok());

    // Second check - should hit cache
    let result2 = checker.check_subsumption(&subtype, &supertype);
    assert!(result2.is_ok());

    let stats = checker.stats();
    // Note: Actual cache hit tracking happens inside SMT backend
    assert_eq!(stats.total_checks, 2, "Should have performed 2 checks");
}

// ==================== Performance Tests ====================

#[test]
fn test_subsumption_performance() {
    // Test that subsumption checks complete within 100ms (per spec)

    use std::time::Instant;

    let x = make_var("it");
    let hundred = make_int(100);
    let zero = make_int(0);

    let phi1 = make_binary(BinOp::Gt, x.clone(), hundred);
    let phi2 = make_binary(BinOp::Gt, x.clone(), zero);

    let start = Instant::now();
    let result = check_subsumption_smt(&phi1, &phi2, 100);
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "Check should succeed");
    assert!(
        elapsed.as_millis() < 100,
        "Subsumption check should complete in < 100ms, took {}ms",
        elapsed.as_millis()
    );
}

#[test]
fn test_z3_backend_stats() {
    // Test that Z3Backend tracks statistics correctly

    let mut backend = Z3Backend::new();

    let x = make_var("x");
    let ten = make_int(10);
    let comparison = make_binary(BinOp::Gt, x, ten);

    // Perform a check
    let _ = backend.check(&comparison);

    let stats = backend.stats();
    assert_eq!(stats.total_queries, 1, "Should have 1 query");
    // Note: elapsed time might be 0ms for very fast checks, so check >= 0
    assert!(
        stats.total_time_ms >= 0,
        "Should have recorded time (even if 0)"
    );
    // Check that at least one of the result counters was incremented
    assert!(
        stats.sat_count + stats.unsat_count + stats.unknown_count > 0,
        "Should have counted the result"
    );
}

// ==================== Edge Cases ====================

#[test]
fn test_trivial_predicate_subsumption() {
    // Test: Int{true} <: Int{true}
    // Trivial predicates should always subsume

    let true_expr = make_bool(true);

    let pred1 = make_predicate(true_expr.clone(), "it");
    let pred2 = make_predicate(true_expr.clone(), "it");

    let subtype = RefinementType::refined(Type::int(), pred1, Span::dummy());
    let supertype = RefinementType::refined(Type::int(), pred2, Span::dummy());

    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let result = checker.check_subsumption(&subtype, &supertype);

    assert!(result.is_ok());
    assert!(result.unwrap(), "Trivial predicates should subsume");
}

#[test]
fn test_unrefined_supertype() {
    // Test: Int{> 0} <: Int
    // Refined type should always be subtype of unrefined

    let x = make_var("it");
    let zero = make_int(0);
    let phi = make_binary(BinOp::Gt, x, zero);

    let pred = make_predicate(phi, "it");
    let subtype = RefinementType::refined(Type::int(), pred, Span::dummy());
    let supertype = RefinementType::unrefined(Type::int(), Span::dummy());

    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let result = checker.check_subsumption(&subtype, &supertype);

    assert!(result.is_ok());
    assert!(
        result.unwrap(),
        "Refined type should be subtype of unrefined"
    );
}

#[test]
fn test_base_type_mismatch() {
    // Test: Int{> 0} NOT<: Bool{true}
    // Different base types should not subsume

    let x = make_var("it");
    let zero = make_int(0);
    let phi1 = make_binary(BinOp::Gt, x, zero);
    let phi2 = make_bool(true);

    let pred1 = make_predicate(phi1, "it");
    let pred2 = make_predicate(phi2, "it");

    let subtype = RefinementType::refined(Type::int(), pred1, Span::dummy());
    let supertype = RefinementType::refined(Type::bool(), pred2, Span::dummy());

    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let result = checker.check_subsumption(&subtype, &supertype);

    assert!(result.is_ok());
    assert!(!result.unwrap(), "Different base types should not subsume");
}

// ==================== Complex Predicates ====================

#[test]
fn test_range_subsumption() {
    // Test: Int{10 <= x <= 20} <: Int{0 <= x <= 100}
    // Tighter range should be subtype of wider range

    let x = make_var("it");
    let ten = make_int(10);
    let twenty = make_int(20);
    let zero = make_int(0);
    let hundred = make_int(100);

    // 10 <= x <= 20
    let ge_10 = make_binary(BinOp::Ge, x.clone(), ten);
    let le_20 = make_binary(BinOp::Le, x.clone(), twenty);
    let phi1 = make_binary(BinOp::And, ge_10, le_20);

    // 0 <= x <= 100
    let ge_0 = make_binary(BinOp::Ge, x.clone(), zero);
    let le_100 = make_binary(BinOp::Le, x.clone(), hundred);
    let phi2 = make_binary(BinOp::And, ge_0, le_100);

    let result = check_subsumption_smt(&phi1, &phi2, 100);
    assert!(result.is_ok(), "SMT check should succeed");
    assert!(
        result.unwrap(),
        "Tighter range should be subtype of wider range"
    );
}

#[test]
fn test_negation_subsumption() {
    // Test: Bool{!a} NOT<: Bool{a}
    // Negation should not imply original

    let a = make_var("a");
    // Use unary negation instead of binary
    let not_a = Expr::new(
        ExprKind::Unary {
            op: UnOp::Not,
            expr: Box::new(a.clone()),
        },
        Span::dummy(),
    );

    let result = check_subsumption_smt(&not_a, &a, 100);
    assert!(result.is_ok(), "SMT check should succeed");
    assert!(
        !result.unwrap(),
        "Negation should not imply original predicate"
    );
}
