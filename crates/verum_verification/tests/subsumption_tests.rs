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
// Comprehensive tests for refinement type subsumption checking
//
// Refinement Type Subsumption: T{phi1} <: T{phi2} iff phi1 logically implies phi2.
//
// Formal rule: Gamma |- phi1 => phi2 (in SMT logic) / Gamma |- T{phi1} <: T{phi2}
//
// Three-tier checking algorithm:
// - Mode 1 (Syntactic, <1ms, ~70% coverage): Pattern-based implication for simple
//   predicates. E.g., > 0 implies >= 0; >= 10 implies >= 0; > 5 implies != 0.
//   Conservative: rejects complex predicates even if they hold.
// - Mode 2 (SMT, 10-500ms, ~95%): Precise checking via Z3. Constructs query
//   assert(not(=> phi1 phi2)), checks sat. UNSAT = valid. SAT = counterexample.
//   Timeout (default 100ms) treated as conservative rejection.
// - Mode 3 (User proof, future): Explicit proof terms cached at 0ms, 100% accuracy.
//
// Subsumption variance: contravariant in function parameters, covariant in returns.
//
// Tests cover all three modes, counterexample extraction, and performance.

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::{IntLit, Literal, LiteralKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path, PathSegment};
use verum_common::{Heap, Map, Text};
use verum_types::Type;
use verum_types::refinement::{RefinementPredicate, RefinementType};
use verum_verification::subsumption::{
    CompareOp, Counterexample, Predicate, SubsumptionChecker, SubsumptionConfig, SubsumptionResult,
    Value, check_subsumption, smt_check, try_syntactic_check,
};

// ==================== Test Helpers ====================

/// Create a dummy span for testing
fn dummy_span() -> Span {
    Span::dummy()
}

/// Create an integer literal expression
fn int_lit(value: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit::new(value as i128)),
            span: dummy_span(),
        }),
        dummy_span(),
    )
}

/// Create a boolean literal expression
fn bool_lit(value: bool) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Bool(value),
            span: dummy_span(),
        }),
        dummy_span(),
    )
}

/// Create a variable reference expression
fn var(name: &str) -> Expr {
    Expr::new(
        ExprKind::Path(Path {
            segments: vec![PathSegment::Name(Ident::new(name, dummy_span()))].into(),
            span: dummy_span(),
        }),
        dummy_span(),
    )
}

/// Create a binary expression
fn binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        dummy_span(),
    )
}

/// Create x > n expression
fn gt(var_name: &str, value: i64) -> Expr {
    binary(BinOp::Gt, var(var_name), int_lit(value))
}

/// Create x >= n expression
fn ge(var_name: &str, value: i64) -> Expr {
    binary(BinOp::Ge, var(var_name), int_lit(value))
}

/// Create x < n expression
fn lt(var_name: &str, value: i64) -> Expr {
    binary(BinOp::Lt, var(var_name), int_lit(value))
}

/// Create x <= n expression
fn le(var_name: &str, value: i64) -> Expr {
    binary(BinOp::Le, var(var_name), int_lit(value))
}

/// Create x == n expression
fn eq(var_name: &str, value: i64) -> Expr {
    binary(BinOp::Eq, var(var_name), int_lit(value))
}

/// Create x != n expression
fn ne(var_name: &str, value: i64) -> Expr {
    binary(BinOp::Ne, var(var_name), int_lit(value))
}

/// Create P && Q expression
fn and(left: Expr, right: Expr) -> Expr {
    binary(BinOp::And, left, right)
}

/// Create P || Q expression
fn or(left: Expr, right: Expr) -> Expr {
    binary(BinOp::Or, left, right)
}

/// Create Int base type
fn int_type() -> Type {
    Type::int()
}

/// Create refinement type Int{predicate}
fn refined_int(predicate: Expr) -> RefinementType {
    RefinementType::refined(
        int_type(),
        RefinementPredicate::inline(predicate, dummy_span()),
        dummy_span(),
    )
}

/// Create unrefined Int type
fn unrefined_int() -> RefinementType {
    RefinementType::unrefined(int_type(), dummy_span())
}

// ==================== Mode 1: Syntactic Tests ====================

mod syntactic_tests {
    use super::*;

    #[test]
    fn test_reflexivity() {
        // P => P should always hold
        let pred = gt("x", 0);
        let result = try_syntactic_check(&pred, &pred);
        assert!(result.is_some());
        assert!(result.unwrap().holds());
    }

    #[test]
    fn test_tautology_true() {
        // anything => true
        let pred = gt("x", 0);
        let true_pred = bool_lit(true);
        let result = try_syntactic_check(&pred, &true_pred);
        assert!(result.is_some());
        assert!(result.unwrap().holds());
    }

    #[test]
    fn test_contradiction_false() {
        // false => anything
        let false_pred = bool_lit(false);
        let pred = gt("x", 0);
        let result = try_syntactic_check(&false_pred, &pred);
        assert!(result.is_some());
        assert!(result.unwrap().holds());
    }

    #[test]
    fn test_true_not_implies_false() {
        // true => false should fail
        let true_pred = bool_lit(true);
        let false_pred = bool_lit(false);
        let result = try_syntactic_check(&true_pred, &false_pred);
        assert!(result.is_some());
        assert!(result.unwrap().fails());
    }

    #[test]
    fn test_gt_strengthening() {
        // x > 10 => x > 5 (stronger bound implies weaker)
        let strong = gt("x", 10);
        let weak = gt("x", 5);
        let result = try_syntactic_check(&strong, &weak);
        assert!(result.is_some());
        assert!(result.unwrap().holds());
    }

    #[test]
    fn test_gt_weakening_fails() {
        // x > 5 => x > 10 should fail
        let weak = gt("x", 5);
        let strong = gt("x", 10);
        let result = try_syntactic_check(&weak, &strong);
        assert!(result.is_some());
        assert!(result.unwrap().fails());
    }

    #[test]
    fn test_gt_to_ge() {
        // x > 10 => x >= 10 (strict implies non-strict)
        let strict = gt("x", 10);
        let non_strict = ge("x", 10);
        let result = try_syntactic_check(&strict, &non_strict);
        assert!(result.is_some());
        assert!(result.unwrap().holds());
    }

    #[test]
    fn test_ge_to_gt_requires_stronger() {
        // x >= 10 => x > 9 (need strictly larger value)
        let ge_pred = ge("x", 10);
        let gt_pred = gt("x", 9);
        let result = try_syntactic_check(&ge_pred, &gt_pred);
        assert!(result.is_some());
        assert!(result.unwrap().holds());
    }

    #[test]
    fn test_ge_to_gt_same_fails() {
        // x >= 10 => x > 10 should fail (x could be exactly 10)
        let ge_pred = ge("x", 10);
        let gt_pred = gt("x", 10);
        let result = try_syntactic_check(&ge_pred, &gt_pred);
        assert!(result.is_some());
        assert!(result.unwrap().fails());
    }

    #[test]
    fn test_lt_strengthening() {
        // x < 5 => x < 10 (stronger upper bound implies weaker)
        let strong = lt("x", 5);
        let weak = lt("x", 10);
        let result = try_syntactic_check(&strong, &weak);
        assert!(result.is_some());
        assert!(result.unwrap().holds());
    }

    #[test]
    fn test_lt_weakening_fails() {
        // x < 10 => x < 5 should fail
        let weak = lt("x", 10);
        let strong = lt("x", 5);
        let result = try_syntactic_check(&weak, &strong);
        assert!(result.is_some());
        assert!(result.unwrap().fails());
    }

    #[test]
    fn test_eq_implies_ge() {
        // x == 5 => x >= 5
        let eq_pred = eq("x", 5);
        let ge_pred = ge("x", 5);
        let result = try_syntactic_check(&eq_pred, &ge_pred);
        assert!(result.is_some());
        assert!(result.unwrap().holds());
    }

    #[test]
    fn test_eq_implies_le() {
        // x == 5 => x <= 5
        let eq_pred = eq("x", 5);
        let le_pred = le("x", 5);
        let result = try_syntactic_check(&eq_pred, &le_pred);
        assert!(result.is_some());
        assert!(result.unwrap().holds());
    }

    #[test]
    fn test_eq_implies_gt_when_larger() {
        // x == 10 => x > 5
        let eq_pred = eq("x", 10);
        let gt_pred = gt("x", 5);
        let result = try_syntactic_check(&eq_pred, &gt_pred);
        assert!(result.is_some());
        assert!(result.unwrap().holds());
    }

    #[test]
    fn test_eq_implies_ne_different() {
        // x == 5 => x != 10
        let eq_pred = eq("x", 5);
        let ne_pred = ne("x", 10);
        let result = try_syntactic_check(&eq_pred, &ne_pred);
        assert!(result.is_some());
        assert!(result.unwrap().holds());
    }

    #[test]
    fn test_gt_implies_ne() {
        // x > 5 => x != 5 (any x > 5 is != 5)
        let gt_pred = gt("x", 5);
        let ne_pred = ne("x", 5);
        let result = try_syntactic_check(&gt_pred, &ne_pred);
        assert!(result.is_some());
        assert!(result.unwrap().holds());
    }

    #[test]
    fn test_conjunction_elimination() {
        // (x > 0 && x < 10) => x > 0
        let conj = and(gt("x", 0), lt("x", 10));
        let target = gt("x", 0);
        let result = try_syntactic_check(&conj, &target);
        assert!(result.is_some());
        assert!(result.unwrap().holds());
    }

    #[test]
    fn test_conjunction_elimination_right() {
        // (x > 0 && x < 10) => x < 10
        let conj = and(gt("x", 0), lt("x", 10));
        let target = lt("x", 10);
        let result = try_syntactic_check(&conj, &target);
        assert!(result.is_some());
        assert!(result.unwrap().holds());
    }

    #[test]
    fn test_disjunction_introduction() {
        // x > 0 => (x > 0 || x < 0)
        let source = gt("x", 0);
        let disj = or(gt("x", 0), lt("x", 0));
        let result = try_syntactic_check(&source, &disj);
        assert!(result.is_some());
        assert!(result.unwrap().holds());
    }

    #[test]
    fn test_different_variables_no_syntactic() {
        // x > 0 => y > 0 cannot be determined syntactically
        let pred1 = gt("x", 0);
        let pred2 = gt("y", 0);
        let result = try_syntactic_check(&pred1, &pred2);
        // Should return None (cannot determine syntactically)
        assert!(result.is_none() || result.unwrap().is_unknown());
    }
}

// ==================== Mode 2: SMT Tests ====================

mod smt_tests {
    use super::*;

    #[test]
    fn test_smt_simple_gt() {
        // x > 10 => x > 5 via SMT
        let pred1 = gt("x", 10);
        let pred2 = gt("x", 5);
        let result = smt_check(&pred1, &pred2);
        assert!(result.holds());
    }

    #[test]
    fn test_smt_fails_with_counterexample() {
        // x > 0 => x > 5 should fail
        let pred1 = gt("x", 0);
        let pred2 = gt("x", 5);
        let result = smt_check(&pred1, &pred2);
        assert!(result.fails());
    }

    #[test]
    fn test_smt_complex_conjunction() {
        // (x > 0 && x < 10) => (x >= 1 && x <= 9)
        let pred1 = and(gt("x", 0), lt("x", 10));
        let pred2 = and(ge("x", 1), le("x", 9));
        let result = smt_check(&pred1, &pred2);
        assert!(result.holds());
    }

    #[test]
    fn test_smt_range_containment() {
        // 10 <= x <= 20 => 0 <= x <= 100
        let pred1 = and(ge("x", 10), le("x", 20));
        let pred2 = and(ge("x", 0), le("x", 100));
        let result = smt_check(&pred1, &pred2);
        assert!(result.holds());
    }

    #[test]
    fn test_smt_non_containment_fails() {
        // 0 <= x <= 100 does not imply 10 <= x <= 20
        let pred1 = and(ge("x", 0), le("x", 100));
        let pred2 = and(ge("x", 10), le("x", 20));
        let result = smt_check(&pred1, &pred2);
        assert!(result.fails());
    }
}

// ==================== Refinement Type Tests ====================

mod refinement_type_tests {
    use super::*;

    #[test]
    fn test_unrefined_subtype_of_unrefined() {
        // Int <: Int (unrefined types)
        let sub = unrefined_int();
        let sup = unrefined_int();
        let result = check_subsumption(&sub, &sup);
        assert!(result.holds());
    }

    #[test]
    fn test_refined_subtype_of_unrefined() {
        // Int{> 0} <: Int (refined is subtype of unrefined)
        let sub = refined_int(gt("it", 0));
        let sup = unrefined_int();
        let result = check_subsumption(&sub, &sup);
        assert!(result.holds());
    }

    #[test]
    fn test_unrefined_not_subtype_of_refined() {
        // Int is NOT <: Int{> 0}
        let sub = unrefined_int();
        let sup = refined_int(gt("it", 0));
        let result = check_subsumption(&sub, &sup);
        assert!(result.fails());
    }

    #[test]
    fn test_stronger_refinement_is_subtype() {
        // Int{> 10} <: Int{> 0}
        let sub = refined_int(gt("it", 10));
        let sup = refined_int(gt("it", 0));
        let result = check_subsumption(&sub, &sup);
        assert!(result.holds());
    }

    #[test]
    fn test_weaker_refinement_not_subtype() {
        // Int{> 0} is NOT <: Int{> 10}
        let sub = refined_int(gt("it", 0));
        let sup = refined_int(gt("it", 10));
        let result = check_subsumption(&sub, &sup);
        assert!(result.fails());
    }

    #[test]
    fn test_positive_implies_nonzero() {
        // Int{> 0} <: Int{!= 0}
        let sub = refined_int(gt("it", 0));
        let sup = refined_int(ne("it", 0));
        let result = check_subsumption(&sub, &sup);
        assert!(result.holds());
    }
}

// ==================== Counterexample Tests ====================

mod counterexample_tests {
    use super::*;

    #[test]
    fn test_counterexample_format() {
        let mut values = Map::new();
        values.insert(Text::from("x"), Value::Int(5));

        let ce = Counterexample::new(values, Text::from("x != 5"));

        let formatted = ce.format_error(&Text::from("x >= 0"), &Text::from("x != 5"));

        assert!(formatted.as_str().contains("x = 5"));
        assert!(formatted.as_str().contains("Counterexample"));
    }

    #[test]
    fn test_counterexample_display() {
        let mut values = Map::new();
        values.insert(Text::from("x"), Value::Int(0));

        let ce = Counterexample::new(values, Text::from("x > 0"));

        let display = format!("{}", ce);
        assert!(display.contains("x = 0"));
        assert!(display.contains("x > 0"));
    }

    #[test]
    fn test_value_display() {
        assert_eq!(format!("{}", Value::Int(42)), "42");
        assert_eq!(format!("{}", Value::Bool(true)), "true");
        assert_eq!(format!("{}", Value::Text(Text::from("hello"))), "\"hello\"");
    }
}

// ==================== Predicate Parsing Tests ====================

mod predicate_tests {
    use super::*;

    #[test]
    fn test_parse_gt() {
        let expr = gt("x", 5);
        let pred = Predicate::from_expr(&expr);

        match pred {
            Predicate::Compare { var, op, value } => {
                assert_eq!(var.as_str(), "x");
                assert_eq!(op, CompareOp::Gt);
                assert_eq!(value, 5);
            }
            _ => panic!("Expected Compare predicate"),
        }
    }

    #[test]
    fn test_parse_and() {
        let expr = and(gt("x", 0), lt("x", 10));
        let pred = Predicate::from_expr(&expr);

        match pred {
            Predicate::And(_, _) => {}
            _ => panic!("Expected And predicate"),
        }
    }

    #[test]
    fn test_parse_or() {
        let expr = or(gt("x", 0), lt("x", 0));
        let pred = Predicate::from_expr(&expr);

        match pred {
            Predicate::Or(_, _) => {}
            _ => panic!("Expected Or predicate"),
        }
    }

    #[test]
    fn test_parse_literal() {
        let expr = bool_lit(true);
        let pred = Predicate::from_expr(&expr);

        assert_eq!(pred, Predicate::Literal(true));
    }

    #[test]
    fn test_simple_predicate_check() {
        let simple = gt("x", 5);
        let pred = Predicate::from_expr(&simple);
        assert!(pred.is_simple());
    }

    #[test]
    fn test_complex_and_is_simple() {
        let complex = and(gt("x", 0), lt("x", 10));
        let pred = Predicate::from_expr(&complex);
        assert!(pred.is_simple()); // Conjunction of simple is simple
    }
}

// ==================== Configuration Tests ====================

mod config_tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = SubsumptionConfig::default();
        assert_eq!(config.timeout_ms, 100);
        assert!(config.try_syntactic_first);
        assert!(config.enable_cache);
        assert_eq!(config.cache_size, 10000);
    }

    #[test]
    fn test_custom_config() {
        let config = SubsumptionConfig {
            timeout_ms: 500,
            try_syntactic_first: false,
            max_smt_complexity: 50,
            enable_cache: false,
            cache_size: 1000,
        };

        let checker = SubsumptionChecker::with_config(config.clone());
        assert_eq!(checker.config().timeout_ms, 500);
        assert!(!checker.config().try_syntactic_first);
    }
}

// ==================== Statistics Tests ====================

mod stats_tests {
    use super::*;

    #[test]
    fn test_stats_tracking() {
        let checker = SubsumptionChecker::new();

        // Perform a syntactic check
        let sub = refined_int(gt("it", 10));
        let sup = refined_int(gt("it", 0));
        let _ = checker.check_subsumption(&sub, &sup);

        let stats = checker.stats();
        assert!(stats.syntactic_hits > 0 || stats.smt_checks > 0);
    }

    #[test]
    fn test_stats_report() {
        let checker = SubsumptionChecker::new();

        // Perform some checks
        let sub1 = refined_int(gt("it", 10));
        let sup1 = refined_int(gt("it", 0));
        let _ = checker.check_subsumption(&sub1, &sup1);

        let sub2 = unrefined_int();
        let sup2 = unrefined_int();
        let _ = checker.check_subsumption(&sub2, &sup2);

        let report = checker.stats().report();
        assert!(report.as_str().contains("Subsumption Checking Statistics"));
        assert!(report.as_str().contains("Total checks"));
    }

    #[test]
    fn test_stats_reset() {
        let checker = SubsumptionChecker::new();

        // Perform a check
        let sub = unrefined_int();
        let sup = unrefined_int();
        let _ = checker.check_subsumption(&sub, &sup);

        // Reset stats
        checker.reset_stats();

        let stats = checker.stats();
        assert_eq!(stats.syntactic_hits, 0);
        assert_eq!(stats.smt_checks, 0);
    }
}

// ==================== Cache Tests ====================

mod cache_tests {
    use super::*;

    #[test]
    fn test_cache_hit() {
        let checker = SubsumptionChecker::new();

        let sub = refined_int(gt("it", 10));
        let sup = refined_int(gt("it", 0));

        // First check - cache miss
        let _ = checker.check_subsumption(&sub, &sup);

        // Second check - should be cache hit
        let _ = checker.check_subsumption(&sub, &sup);

        let stats = checker.stats();
        assert!(stats.cache_hits > 0 || stats.syntactic_hits >= 2);
    }

    #[test]
    fn test_cache_clear() {
        let checker = SubsumptionChecker::new();

        let sub = refined_int(gt("it", 10));
        let sup = refined_int(gt("it", 0));

        // Populate cache
        let _ = checker.check_subsumption(&sub, &sup);

        // Clear cache
        checker.clear_cache();

        // Next check should be cache miss
        let _ = checker.check_subsumption(&sub, &sup);

        // Stats should show cache miss
        let stats = checker.stats();
        assert!(stats.cache_misses > 0 || stats.syntactic_hits > 0);
    }
}

// ==================== Spec Compliance Tests ====================

mod spec_tests {
    use super::*;

    /// Syntactic subsumption: Int{> 0} <: Int{>= 0} (> 0 obviously implies >= 0)
    #[test]
    fn test_spec_example_positive_implies_nonnegative() {
        let sub = refined_int(gt("it", 0));
        let sup = refined_int(ge("it", 0));
        let result = check_subsumption(&sub, &sup);
        assert!(result.holds());
    }

    /// Syntactic subsumption: Int{>= 10} <: Int{>= 0} (>= 10 obviously implies >= 0)
    #[test]
    fn test_spec_example_ge_10_implies_ge_0() {
        let sub = refined_int(ge("it", 10));
        let sup = refined_int(ge("it", 0));
        let result = check_subsumption(&sub, &sup);
        assert!(result.holds());
    }

    /// Syntactic subsumption: Int{> 5} <: Int{!= 0} (> 5 obviously implies != 0)
    #[test]
    fn test_spec_example_gt_5_implies_ne_0() {
        let sub = refined_int(gt("it", 5));
        let sup = refined_int(ne("it", 0));
        let result = check_subsumption(&sub, &sup);
        assert!(result.holds());
    }

    /// Correct rejection: Int{>= 0} NOT <: Int{> 0} (>= 0 does NOT imply > 0, counterexample: x=0)
    #[test]
    fn test_spec_example_nonnegative_not_implies_positive() {
        let sub = refined_int(ge("it", 0));
        let sup = refined_int(gt("it", 0));
        let result = check_subsumption(&sub, &sup);
        assert!(result.fails());
    }

    /// Test SMT counterexample extraction per spec lines 3870-3896
    #[test]
    fn test_spec_counterexample_format() {
        let sub = refined_int(ge("it", 0));
        let sup = refined_int(ne("it", 0));
        let result = check_subsumption(&sub, &sup);

        if let SubsumptionResult::Fails { counterexample } = result {
            // Counterexample should show x = 0
            let formatted =
                counterexample.format_error(&Text::from("it >= 0"), &Text::from("it != 0"));
            assert!(formatted.as_str().contains("Counterexample"));
        } else {
            panic!("Expected Fails result");
        }
    }
}

// ==================== Edge Case Tests ====================

mod edge_case_tests {
    use super::*;

    #[test]
    fn test_zero_literal() {
        let sub = refined_int(eq("it", 0));
        let sup = refined_int(ge("it", 0));
        let result = check_subsumption(&sub, &sup);
        assert!(result.holds());
    }

    #[test]
    fn test_negative_values() {
        // Int{> -10} <: Int{> -20}
        let sub = refined_int(gt("it", -10));
        let sup = refined_int(gt("it", -20));
        let result = check_subsumption(&sub, &sup);
        assert!(result.holds());
    }

    #[test]
    fn test_large_values() {
        // Int{> 1000000} <: Int{> 0}
        let sub = refined_int(gt("it", 1_000_000));
        let sup = refined_int(gt("it", 0));
        let result = check_subsumption(&sub, &sup);
        assert!(result.holds());
    }

    #[test]
    fn test_nested_conjunction() {
        // ((x > 0 && x < 10) && x != 5) => x > 0
        let inner = and(gt("it", 0), lt("it", 10));
        let pred1 = and(inner, ne("it", 5));
        let pred2 = gt("it", 0);

        let sub = refined_int(pred1);
        let sup = refined_int(pred2);
        let result = check_subsumption(&sub, &sup);
        assert!(result.holds());
    }

    #[test]
    fn test_identical_predicates() {
        // Same predicate should always subsume
        let pred = and(gt("it", 0), lt("it", 100));
        let sub = refined_int(pred.clone());
        let sup = refined_int(pred);
        let result = check_subsumption(&sub, &sup);
        assert!(result.holds());
    }
}

// ==================== Performance Tests ====================

mod performance_tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_syntactic_check_is_fast() {
        let checker = SubsumptionChecker::new();

        let sub = refined_int(gt("it", 10));
        let sup = refined_int(gt("it", 0));

        let start = Instant::now();
        for _ in 0..1000 {
            let _ = checker.check_subsumption(&sub, &sup);
        }
        let elapsed = start.elapsed();

        // 1000 checks should complete in well under 1 second for syntactic
        assert!(
            elapsed.as_millis() < 1000,
            "Syntactic checks too slow: {:?}",
            elapsed
        );
    }

    #[test]
    fn test_stats_accuracy() {
        let checker = SubsumptionChecker::new();

        // Run 10 checks
        for i in 0..10 {
            let sub = refined_int(gt("it", i * 10));
            let sup = refined_int(gt("it", 0));
            let _ = checker.check_subsumption(&sub, &sup);
        }

        let stats = checker.stats();
        let total = stats.syntactic_hits + stats.smt_checks + stats.fallbacks;

        // Should have recorded all checks (some might be trivial cases)
        assert!(total >= 10, "Not all checks recorded: {}", total);
    }
}
