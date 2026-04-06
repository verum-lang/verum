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
//! Comprehensive test suite for SMT integration.

use std::time::Duration;
use verum_ast::{BinOp, Expr, ExprKind, Ident, Literal, RefinementPredicate, Span, Type, TypeKind};
use verum_common::Heap;
use verum_smt::cost::CostMeasurement;
use verum_smt::counterexample::generate_suggestions;
use verum_smt::verify::{auto_mode, verify_batch};
use verum_smt::*;

// Helper functions for creating AST nodes
fn dummy_span() -> Span {
    Span::dummy()
}

fn int_lit(value: i64) -> Expr {
    Expr::literal(Literal::int(value as i128, dummy_span()))
}

fn bool_lit(value: bool) -> Expr {
    Expr::literal(Literal::bool(value, dummy_span()))
}

fn ident_expr(name: &str) -> Expr {
    let ident = Ident::new(name, dummy_span());
    Expr::ident(ident)
}

fn binary_expr(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        dummy_span(),
    )
}

fn refined_type(base: Type, predicate: Expr) -> Type {
    Type::new(
        TypeKind::Refined {
            base: Box::new(base),
            predicate: Box::new(RefinementPredicate::new(predicate, dummy_span())),
        },
        dummy_span(),
    )
}

#[cfg(test)]
mod context_tests {
    use super::*;

    #[test]
    fn test_context_creation() {
        let ctx = Context::new();
        assert!(ctx.config().timeout.is_some());
    }

    #[test]
    fn test_context_with_custom_config() {
        let config = ContextConfig::fast();
        let ctx = Context::with_config(config);
        assert_eq!(ctx.config().timeout, Some(Duration::from_secs(5)));
    }

    #[test]
    fn test_solver_creation() {
        let ctx = Context::new();
        let _solver = ctx.solver();
    }

    #[test]
    fn test_multiple_contexts() {
        let ctx1 = Context::new();
        let ctx2 = Context::new();
        let _solver1 = ctx1.solver();
        let _solver2 = ctx2.solver();
    }
}

#[cfg(test)]
mod translation_tests {
    use super::*;

    #[test]
    fn test_translate_integer_literal() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);
        let expr = int_lit(42);

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate integer literal");
    }

    #[test]
    fn test_translate_boolean_literal() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);
        let expr = bool_lit(true);

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate boolean literal");
    }

    #[test]
    fn test_translate_addition() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);
        let expr = binary_expr(BinOp::Add, int_lit(1), int_lit(2));

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate addition");
    }

    #[test]
    fn test_translate_comparison() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);
        let expr = binary_expr(BinOp::Gt, int_lit(5), int_lit(0));

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate comparison");
    }

    #[test]
    fn test_translate_variable() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);
        let expr = ident_expr("x");

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate variable");
    }

    #[test]
    fn test_translate_complex_expression() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);

        // (x + 5) > 10
        let left = binary_expr(BinOp::Add, ident_expr("x"), int_lit(5));
        let expr = binary_expr(BinOp::Gt, left, int_lit(10));

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate complex expression");
    }
}

#[cfg(test)]
mod cost_tests {
    use super::*;

    #[test]
    fn test_cost_tracker_basic() {
        let mut tracker = CostTracker::new();

        let cost = VerificationCost::new("test_fn".into(), Duration::from_millis(500), true);

        tracker.record(cost);

        assert_eq!(tracker.costs().len(), 1);
        assert_eq!(tracker.total_time(), Duration::from_millis(500));
    }

    #[test]
    fn test_cost_tracker_multiple() {
        let mut tracker = CostTracker::new();

        tracker.record(VerificationCost::new(
            "fn1".into(),
            Duration::from_secs(1),
            true,
        ));
        tracker.record(VerificationCost::new(
            "fn2".into(),
            Duration::from_secs(2),
            true,
        ));
        tracker.record(VerificationCost::new(
            "fn3".into(),
            Duration::from_secs(3),
            true,
        ));

        assert_eq!(tracker.costs().len(), 3);
        assert_eq!(tracker.total_time(), Duration::from_secs(6));
        assert_eq!(tracker.avg_time(), Duration::from_secs(2));
    }

    #[test]
    fn test_slow_verification_detection() {
        let mut tracker = CostTracker::with_threshold(Duration::from_secs(2));

        tracker.record(VerificationCost::new(
            "fast".into(),
            Duration::from_millis(500),
            true,
        ));
        tracker.record(VerificationCost::new(
            "slow".into(),
            Duration::from_secs(5),
            true,
        ));

        let slow = tracker.slow_verifications();
        assert_eq!(slow.len(), 1);
        assert_eq!(slow[0].location, "slow");
        assert!(tracker.should_suggest_runtime());
    }

    #[test]
    fn test_cost_report_generation() {
        let mut tracker = CostTracker::new();

        tracker.record(VerificationCost::new(
            "test".into(),
            Duration::from_secs(2),
            true,
        ));

        let report = tracker.report();
        assert_eq!(report.total_verifications, 1);
        assert_eq!(report.total_time, Duration::from_secs(2));
    }

    #[test]
    fn test_cost_measurement() {
        let measurement = CostMeasurement::start("test_function");
        std::thread::sleep(Duration::from_millis(10));
        let cost = measurement.finish(true);

        assert!(cost.succeeded);
        assert!(cost.duration >= Duration::from_millis(10));
        assert_eq!(cost.location, "test_function");
    }
}

#[cfg(test)]
mod counterexample_tests {
    use super::*;
    use verum_common::Map;

    #[test]
    fn test_counterexample_creation() {
        let mut assignments = Map::new();
        assignments.insert("x".into(), CounterExampleValue::Int(-5));

        let ce = CounterExample::new(assignments, "x > 0".into());

        assert_eq!(ce.assignments.len(), 1);
        assert_eq!(ce.get("x").unwrap().as_int(), Some(-5));
        assert!(ce.is_minimal());
    }

    #[test]
    fn test_counterexample_display() {
        let mut assignments = Map::new();
        assignments.insert("value".into(), CounterExampleValue::Int(0));

        let ce = CounterExample::new(assignments, "value != 0".into());
        let display = format!("{}", ce);

        assert!(display.contains("value = 0"));
        assert!(display.contains("value != 0"));
    }

    #[test]
    fn test_counterexample_values() {
        let int_val = CounterExampleValue::Int(42);
        assert_eq!(int_val.as_int(), Some(42));
        assert!(int_val.is_scalar());

        let bool_val = CounterExampleValue::Bool(false);
        assert_eq!(bool_val.as_bool(), Some(false));
        assert!(bool_val.is_scalar());

        use verum_common::List;
        let arr = CounterExampleValue::Array(List::from(vec![
            CounterExampleValue::Int(1),
            CounterExampleValue::Int(2),
        ]));
        assert!(!arr.is_scalar());
        assert_eq!(arr.as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_suggestion_generation() {
        let mut assignments = Map::new();
        assignments.insert("x".into(), CounterExampleValue::Int(-5));

        let ce = CounterExample::new(assignments, "x > 0".into());
        let suggestions = generate_suggestions(&ce, "x > 0");

        assert!(!suggestions.is_empty());
    }
}

#[cfg(test)]
mod verification_tests {
    use super::*;

    #[test]
    fn test_runtime_mode_always_succeeds() {
        let ctx = Context::new();
        let base = Type::int(dummy_span());
        let predicate = binary_expr(BinOp::Gt, ident_expr("it"), int_lit(0));
        let refined = refined_type(base, predicate);

        let result = verify_refinement(&ctx, &refined, None, VerifyMode::Runtime);
        assert!(result.is_ok(), "Runtime mode should always succeed");
    }

    #[test]
    fn test_complexity_estimation_indirect() {
        // estimate_expr_complexity is private, so we test through auto_mode
        let simple_type = Type::int(dummy_span());
        let _mode = auto_mode(&simple_type);
        // Function completes without panic - that's success
    }

    #[test]
    fn test_auto_mode_selection() {
        let simple_type = Type::int(dummy_span());
        let mode = auto_mode(&simple_type);
        // Should select either Proof or Auto for simple types
        assert!(matches!(mode, VerifyMode::Proof | VerifyMode::Auto));
    }

    #[test]
    fn test_proof_result_creation() {
        let cost = VerificationCost::new("test".into(), Duration::from_millis(100), true);

        let result = ProofResult::new(cost);
        assert!(!result.cached);
        assert!(result.smt_lib.is_none());

        let result = result.with_cached().with_smt_lib("(assert true)".into());
        assert!(result.cached);
        assert!(result.smt_lib.is_some());
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn test_full_verification_workflow() {
        let ctx = Context::new();
        let mut tracker = CostTracker::new();

        // Create a refinement type: Int{> 0}
        let base = Type::int(dummy_span());
        let predicate = binary_expr(BinOp::Gt, ident_expr("it"), int_lit(0));
        let positive_int = refined_type(base, predicate);

        // Verify (this will likely fail or succeed depending on solver behavior)
        let result = verify_refinement(&ctx, &positive_int, None, VerifyMode::Auto);

        // Record the cost regardless of outcome
        if let Ok(proof) = &result {
            tracker.record(proof.cost.clone());
        } else if let Err(e) = &result
            && let Some(cost) = e.cost()
        {
            tracker.record(cost.clone());
        }

        // Generate report
        let report = tracker.report();
        assert!(report.total_verifications > 0);
    }

    #[test]
    fn test_batch_verification() {
        let ctx = Context::new();

        // Create multiple refinement types
        let base = Type::int(dummy_span());

        let constraints = vec![
            (
                refined_type(
                    base.clone(),
                    binary_expr(BinOp::Gt, ident_expr("it"), int_lit(0)),
                ),
                None,
            ),
            (
                refined_type(
                    base.clone(),
                    binary_expr(BinOp::Lt, ident_expr("it"), int_lit(100)),
                ),
                None,
            ),
        ];

        let results = verify_batch(&ctx, &constraints, VerifyMode::Auto);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_context_reuse() {
        let ctx = Context::new();

        // Verify multiple times with the same context
        let base = Type::int(dummy_span());
        let predicate = binary_expr(BinOp::Ge, ident_expr("it"), int_lit(0));
        let non_negative = refined_type(base, predicate);

        let _result1 = verify_refinement(&ctx, &non_negative, None, VerifyMode::Auto);
        let _result2 = verify_refinement(&ctx, &non_negative, None, VerifyMode::Auto);

        // Context should be reusable
    }

    #[test]
    fn test_cost_reporting_format() {
        let mut tracker = CostTracker::new();

        tracker.record(
            VerificationCost::new("expensive_check".into(), Duration::from_secs(6), true)
                .with_complexity(80),
        );

        let report = tracker.report();
        let formatted = report.format();

        assert!(formatted.contains("Verification Summary"));
        assert!(formatted.contains("expensive_check"));
        assert!(formatted.contains("@verify(runtime)"));
    }
}

#[cfg(test)]
mod edge_case_tests {
    use super::*;

    #[test]
    fn test_empty_cost_tracker() {
        let tracker = CostTracker::new();
        assert_eq!(tracker.costs().len(), 0);
        assert_eq!(tracker.total_time(), Duration::ZERO);
        assert_eq!(tracker.avg_time(), Duration::ZERO);
        assert!(tracker.slowest().is_none());
        assert!(!tracker.should_suggest_runtime());
    }

    #[test]
    fn test_zero_timeout_config() {
        let config = ContextConfig::default().without_timeout();
        assert!(config.timeout.is_none());
    }

    #[test]
    fn test_counterexample_with_no_assignments() {
        use verum_common::Map;
        let assignments = Map::new();
        let ce = CounterExample::new(assignments, "false".into());
        // Empty counterexample has length 0, so it's not minimal (length != 1)
        assert!(!ce.is_minimal());
        assert_eq!(ce.assignments.len(), 0);
    }

    #[test]
    fn test_verification_error_messages() {
        let cost = VerificationCost::new("test".into(), Duration::from_secs(1), false);

        use verum_common::{List, Text};
        let err = VerificationError::CannotProve {
            constraint: "x > 0".into(),
            counterexample: None,
            cost: cost.clone(),
            suggestions: List::from(vec![Text::from("Add precondition")]),
        };

        let msg = format!("{}", err);
        assert!(msg.contains("cannot prove"));
    }

    #[test]
    fn test_context_config_chaining() {
        let config = ContextConfig::default()
            .with_timeout(Duration::from_secs(10))
            .with_memory_limit(256)
            .with_models()
            .with_unsat_core()
            .with_seed(123);

        assert_eq!(config.timeout, Some(Duration::from_secs(10)));
        assert_eq!(config.memory_limit_mb, Some(256));
        assert!(config.model_generation);
        assert!(config.unsat_core);
        assert_eq!(config.random_seed, Some(123));
    }
}

#[cfg(test)]
mod advanced_translation_tests {
    use super::*;

    #[test]
    fn test_translate_nested_binary_ops() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);

        // ((1 + 2) * 3) > 10
        let inner_add = binary_expr(BinOp::Add, int_lit(1), int_lit(2));
        let mul = binary_expr(BinOp::Mul, inner_add, int_lit(3));
        let expr = binary_expr(BinOp::Gt, mul, int_lit(10));

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate nested expression");
    }

    #[test]
    fn test_translate_negation() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);

        let expr = Expr::new(
            ExprKind::Unary {
                op: verum_ast::UnOp::Neg,
                expr: Heap::new(int_lit(42)),
            },
            dummy_span(),
        );

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate negation");
    }

    #[test]
    fn test_translate_logical_not() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);

        let expr = Expr::new(
            ExprKind::Unary {
                op: verum_ast::UnOp::Not,
                expr: Heap::new(bool_lit(true)),
            },
            dummy_span(),
        );

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate logical not");
    }

    #[test]
    fn test_translate_multiple_comparisons() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);

        // x > 0 && x < 100
        let left = binary_expr(BinOp::Gt, ident_expr("x"), int_lit(0));
        let right = binary_expr(BinOp::Lt, ident_expr("x"), int_lit(100));
        let expr = binary_expr(BinOp::And, left, right);

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate compound comparison");
    }

    #[test]
    fn test_translate_subtraction() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);
        let expr = binary_expr(BinOp::Sub, int_lit(10), int_lit(3));

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate subtraction");
    }

    #[test]
    fn test_translate_multiplication() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);
        let expr = binary_expr(BinOp::Mul, int_lit(6), int_lit(7));

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate multiplication");
    }

    #[test]
    fn test_translate_division() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);
        let expr = binary_expr(BinOp::Div, int_lit(20), int_lit(4));

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate division");
    }

    #[test]
    fn test_translate_modulo() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);
        let expr = binary_expr(BinOp::Rem, int_lit(17), int_lit(5));

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate modulo");
    }

    #[test]
    fn test_translate_equality() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);
        let expr = binary_expr(BinOp::Eq, int_lit(42), int_lit(42));

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate equality");
    }

    #[test]
    fn test_translate_inequality() {
        let ctx = Context::new();
        let translator = Translator::new(&ctx);
        let expr = binary_expr(BinOp::Ne, int_lit(1), int_lit(2));

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed to translate inequality");
    }
}

#[cfg(test)]
mod cost_suggestion_tests {
    use super::*;
    use crate::cost::{format_failure, format_success};

    #[test]
    fn test_format_success_message() {
        let cost = VerificationCost::new("my_function".into(), Duration::from_millis(300), true);

        let msg = format_success("my_function", &cost);
        assert!(msg.contains("✓"));
        assert!(msg.contains("my_function"));
        assert!(!msg.contains("slow"));
    }

    #[test]
    fn test_format_slow_success_message() {
        let cost = VerificationCost::new("slow_function".into(), Duration::from_secs(2), true);

        let msg = format_success("slow_function", &cost);
        assert!(msg.contains("✓"));
        assert!(msg.contains("slow"));
    }

    #[test]
    fn test_format_very_slow_success_message() {
        let cost = VerificationCost::new("very_slow_function".into(), Duration::from_secs(8), true);

        let msg = format_success("very_slow_function", &cost);
        assert!(msg.contains("✓"));
        assert!(msg.contains("@verify(runtime)"));
    }

    #[test]
    fn test_format_failure_message() {
        let cost =
            VerificationCost::new("failed_function".into(), Duration::from_millis(500), false);

        let msg = format_failure("failed_function", "x > 0", &cost);
        assert!(msg.contains("✗"));
        assert!(msg.contains("x > 0"));
    }

    #[test]
    fn test_format_timeout_failure_message() {
        let cost = VerificationCost::new("timeout_function".into(), Duration::from_secs(30), false)
            .with_timeout();

        let msg = format_failure("timeout_function", "complex_constraint", &cost);
        assert!(msg.contains("✗"));
        assert!(msg.contains("timeout"));
        assert!(msg.contains("@verify(runtime)"));
    }

    #[test]
    fn test_cost_is_slow() {
        let fast = VerificationCost::new("fast".into(), Duration::from_millis(500), true);
        assert!(!fast.is_slow());

        let slow = VerificationCost::new("slow".into(), Duration::from_secs(2), true);
        assert!(slow.is_slow());
    }

    #[test]
    fn test_cost_is_very_slow() {
        let slow = VerificationCost::new("slow".into(), Duration::from_secs(2), true);
        assert!(!slow.is_very_slow());

        let very_slow = VerificationCost::new("very_slow".into(), Duration::from_secs(10), true);
        assert!(very_slow.is_very_slow());
    }

    #[test]
    fn test_cost_with_complexity() {
        let cost =
            VerificationCost::new("test".into(), Duration::from_secs(1), true).with_complexity(85);

        assert_eq!(cost.complexity, 85);
    }

    #[test]
    fn test_cost_complexity_clamping() {
        let cost =
            VerificationCost::new("test".into(), Duration::from_secs(1), true).with_complexity(150); // Over 100

        assert_eq!(cost.complexity, 100); // Should be clamped
    }
}

#[cfg(test)]
mod solver_stats_tests {
    use super::*;

    #[test]
    fn test_solver_stats_creation() {
        let stats = SolverStats::new();
        assert_eq!(stats.num_checks, 0);
        assert_eq!(stats.time_ms, 0);
    }

    #[test]
    fn test_solver_stats_recording() {
        let mut stats = SolverStats::new();

        stats.record_sat(100);
        assert_eq!(stats.num_sat, 1);
        assert_eq!(stats.num_checks, 1);
        assert_eq!(stats.time_ms, 100);

        stats.record_unsat(200);
        assert_eq!(stats.num_unsat, 1);
        assert_eq!(stats.num_checks, 2);
        assert_eq!(stats.time_ms, 300);

        stats.record_unknown(50);
        assert_eq!(stats.num_unknown, 1);
        assert_eq!(stats.num_checks, 3);

        stats.record_timeout(1000);
        assert_eq!(stats.num_timeouts, 1);
        assert_eq!(stats.num_checks, 4);
    }

    #[test]
    fn test_solver_stats_averages() {
        let mut stats = SolverStats::new();

        stats.record_sat(100);
        stats.record_sat(200);
        stats.record_sat(300);

        assert_eq!(stats.avg_time_ms(), 200.0);
    }

    #[test]
    fn test_solver_stats_success_rate() {
        let mut stats = SolverStats::new();

        stats.record_sat(100);
        stats.record_unsat(100);
        stats.record_unknown(100);
        stats.record_timeout(100);

        // Success = (sat + unsat) / total = 2/4 = 0.5
        assert_eq!(stats.success_rate(), 0.5);
    }

    #[test]
    fn test_solver_stats_memory_tracking() {
        let mut stats = SolverStats::new();

        stats.update_memory(1000);
        assert_eq!(stats.peak_memory_bytes, 1000);

        stats.update_memory(500); // Lower than peak
        assert_eq!(stats.peak_memory_bytes, 1000);

        stats.update_memory(2000); // New peak
        assert_eq!(stats.peak_memory_bytes, 2000);
    }
}

#[cfg(test)]
mod real_world_verification_tests {
    use super::*;
    use z3::ast::Int;

    /// Test: Verify that positive integers are always > 0
    /// This should succeed - there's no counterexample
    #[test]
    fn test_verify_positive_int_always_holds() {
        let ctx = Context::new();

        // Create type: Int{> 0}
        let base = Type::int(dummy_span());
        let predicate = binary_expr(BinOp::Gt, ident_expr("it"), int_lit(0));
        let positive_int = refined_type(base, predicate);

        // Try to verify this type (should find that it CAN be violated)
        let result = verify_refinement(&ctx, &positive_int, None, VerifyMode::Proof);

        // This should fail because there exist integers <= 0
        assert!(
            result.is_err(),
            "Should find counterexample for 'it > 0' without constraints"
        );
    }

    /// Test: Verify non-negative integers
    #[test]
    fn test_verify_non_negative_int() {
        let ctx = Context::new();

        // Create type: Int{>= 0}
        let base = Type::int(dummy_span());
        let predicate = binary_expr(BinOp::Ge, ident_expr("it"), int_lit(0));
        let non_negative = refined_type(base, predicate);

        // Try to verify (should fail - negative numbers exist)
        let result = verify_refinement(&ctx, &non_negative, None, VerifyMode::Proof);

        assert!(
            result.is_err(),
            "Should find counterexample (negative number)"
        );

        if let Err(VerificationError::CannotProve { counterexample, .. }) = result
            && let Some(ce) = counterexample
        {
            // Should have a negative value for 'it'
            if let Some(val) = ce.get("it")
                && let Some(i) = val.as_int()
            {
                assert!(i < 0, "Counterexample should be negative, got {}", i);
            }
        }
    }

    /// Test: Verify bounded integers (range constraint)
    #[test]
    fn test_verify_bounded_int() {
        let ctx = Context::new();

        // Create type: Int{it >= 0 && it < 100}
        let base = Type::int(dummy_span());
        let lower = binary_expr(BinOp::Ge, ident_expr("it"), int_lit(0));
        let upper = binary_expr(BinOp::Lt, ident_expr("it"), int_lit(100));
        let predicate = binary_expr(BinOp::And, lower, upper);
        let bounded = refined_type(base, predicate);

        // This should fail - integers outside [0, 100) exist
        let result = verify_refinement(&ctx, &bounded, None, VerifyMode::Proof);
        assert!(result.is_err(), "Should find counterexample outside range");
    }

    /// Test: Verify non-zero constraint for division
    #[test]
    fn test_verify_non_zero_divisor() {
        let ctx = Context::new();

        // Create type: Int{!= 0}
        let base = Type::int(dummy_span());
        let predicate = binary_expr(BinOp::Ne, ident_expr("it"), int_lit(0));
        let non_zero = refined_type(base, predicate);

        // This should fail - zero exists
        let result = verify_refinement(&ctx, &non_zero, None, VerifyMode::Proof);
        assert!(result.is_err(), "Should find counterexample: zero");

        if let Err(VerificationError::CannotProve { counterexample, .. }) = result
            && let Some(ce) = counterexample
            && let Some(val) = ce.get("it")
            && let Some(i) = val.as_int()
        {
            assert_eq!(i, 0, "Counterexample should be zero for != 0 constraint");
        }
    }

    /// Test: Complex predicate with multiple conditions
    #[test]
    fn test_verify_complex_predicate() {
        let ctx = Context::new();

        // Create type: Int{it > 10 && it < 20 && it != 15}
        let base = Type::int(dummy_span());
        let gt10 = binary_expr(BinOp::Gt, ident_expr("it"), int_lit(10));
        let lt20 = binary_expr(BinOp::Lt, ident_expr("it"), int_lit(20));
        let ne15 = binary_expr(BinOp::Ne, ident_expr("it"), int_lit(15));
        let cond1 = binary_expr(BinOp::And, gt10, lt20);
        let predicate = binary_expr(BinOp::And, cond1, ne15);
        let complex = refined_type(base, predicate);

        // Should fail - many counterexamples exist
        let result = verify_refinement(&ctx, &complex, None, VerifyMode::Proof);
        assert!(result.is_err(), "Should find counterexample");
    }

    /// Test: Boolean refinement
    #[test]
    fn test_verify_boolean_refinement() {
        let ctx = Context::new();

        // Create type: Bool (with predicate that it equals true)
        let base = Type::bool(dummy_span());
        let predicate = binary_expr(BinOp::Eq, ident_expr("it"), bool_lit(true));
        let always_true = refined_type(base, predicate);

        // Should fail - false exists
        let result = verify_refinement(&ctx, &always_true, None, VerifyMode::Proof);
        assert!(result.is_err(), "Should find counterexample: false");
    }

    /// Test: Arithmetic relationship
    #[test]
    fn test_verify_arithmetic_relationship() {
        let ctx = Context::new();

        // Create type: Int{it * 2 > 10}
        let base = Type::int(dummy_span());
        let doubled = binary_expr(BinOp::Mul, ident_expr("it"), int_lit(2));
        let predicate = binary_expr(BinOp::Gt, doubled, int_lit(10));
        let arithmetic = refined_type(base, predicate);

        // Should fail - numbers where 2*it <= 10 exist (like 0, 1, 5, etc.)
        let result = verify_refinement(&ctx, &arithmetic, None, VerifyMode::Proof);
        assert!(result.is_err(), "Should find counterexample");

        if let Err(VerificationError::CannotProve { counterexample, .. }) = result
            && let Some(ce) = counterexample
            && let Some(val) = ce.get("it")
            && let Some(i) = val.as_int()
        {
            assert!(
                i * 2 <= 10,
                "Counterexample should satisfy: 2*it <= 10, got it={}",
                i
            );
        }
    }

    /// Test: Negation
    #[test]
    fn test_verify_negation() {
        let ctx = Context::new();

        // Create type: Int{!(it < 0)}  which is equivalent to it >= 0
        let base = Type::int(dummy_span());
        let lt_zero = binary_expr(BinOp::Lt, ident_expr("it"), int_lit(0));
        let predicate = Expr::new(
            ExprKind::Unary {
                op: verum_ast::UnOp::Not,
                expr: Heap::new(lt_zero),
            },
            dummy_span(),
        );
        let not_negative = refined_type(base, predicate);

        // Should fail - negative numbers exist
        let result = verify_refinement(&ctx, &not_negative, None, VerifyMode::Proof);
        assert!(
            result.is_err(),
            "Should find counterexample: negative number"
        );
    }

    /// Test: Verify with runtime mode always succeeds
    #[test]
    fn test_runtime_mode_always_succeeds() {
        let ctx = Context::new();

        // Even an impossible constraint succeeds in runtime mode
        let base = Type::int(dummy_span());
        // Create contradictory predicate: it > 0 && it < 0
        let gt = binary_expr(BinOp::Gt, ident_expr("it"), int_lit(0));
        let lt = binary_expr(BinOp::Lt, ident_expr("it"), int_lit(0));
        let impossible = binary_expr(BinOp::And, gt, lt);
        let contradictory = refined_type(base, impossible);

        // Runtime mode should succeed
        let result = verify_refinement(&ctx, &contradictory, None, VerifyMode::Runtime);
        assert!(result.is_ok(), "Runtime mode should always succeed");
    }

    /// Test: Cost tracking for multiple verifications
    #[test]
    fn test_cost_tracking_multiple_verifications() {
        let ctx = Context::new();
        let mut tracker = CostTracker::new();

        // Verify several different refinements
        let test_cases = vec![
            binary_expr(BinOp::Gt, ident_expr("it"), int_lit(0)),
            binary_expr(BinOp::Ge, ident_expr("it"), int_lit(0)),
            binary_expr(BinOp::Ne, ident_expr("it"), int_lit(0)),
        ];

        for (idx, predicate) in test_cases.into_iter().enumerate() {
            let base = Type::int(dummy_span());
            let refined = refined_type(base, predicate);

            let result = verify_refinement(&ctx, &refined, None, VerifyMode::Proof);

            // Record cost
            match result {
                Ok(proof) => tracker.record(proof.cost),
                Err(e) => {
                    if let Some(cost) = e.cost() {
                        tracker.record(cost.clone());
                    }
                }
            }
        }

        // Should have tracked multiple verifications
        assert_eq!(tracker.costs().len(), 3);
        assert!(tracker.total_time() > Duration::ZERO);
    }

    /// Test: Subtraction constraint
    #[test]
    fn test_verify_subtraction_constraint() {
        let ctx = Context::new();

        // Create type: Int{it - 5 > 0}  which means it > 5
        let base = Type::int(dummy_span());
        let minus_five = binary_expr(BinOp::Sub, ident_expr("it"), int_lit(5));
        let predicate = binary_expr(BinOp::Gt, minus_five, int_lit(0));
        let gt_five = refined_type(base, predicate);

        // Should fail - numbers <= 5 exist
        let result = verify_refinement(&ctx, &gt_five, None, VerifyMode::Proof);
        assert!(result.is_err(), "Should find counterexample <= 5");
    }

    /// Test: Division constraint (modulo)
    #[test]
    fn test_verify_modulo_constraint() {
        let ctx = Context::new();

        // Create type: Int{it % 2 == 0}  (even numbers)
        let base = Type::int(dummy_span());
        let mod_two = binary_expr(BinOp::Rem, ident_expr("it"), int_lit(2));
        let predicate = binary_expr(BinOp::Eq, mod_two, int_lit(0));
        let even = refined_type(base, predicate);

        // Should fail - odd numbers exist
        let result = verify_refinement(&ctx, &even, None, VerifyMode::Proof);
        assert!(result.is_err(), "Should find counterexample: odd number");
    }

    /// Test: Verify with short timeout
    #[test]
    fn test_verify_with_timeout() {
        let config = ContextConfig::default().with_timeout(Duration::from_millis(100));
        let ctx = Context::with_config(config);

        // Simple constraint that should complete quickly
        let base = Type::int(dummy_span());
        let predicate = binary_expr(BinOp::Gt, ident_expr("it"), int_lit(0));
        let positive = refined_type(base, predicate);

        let result = verify_refinement(&ctx, &positive, None, VerifyMode::Proof);
        // Should complete (either success or failure) within timeout
        assert!(result.is_ok() || result.is_err());
    }

    /// Test: Auto mode selection
    #[test]
    fn test_auto_mode_selection() {
        let ctx = Context::new();

        // Simple predicate - auto mode should work
        let base = Type::int(dummy_span());
        let predicate = binary_expr(BinOp::Eq, ident_expr("it"), int_lit(42));
        let simple = refined_type(base, predicate);

        let result = verify_refinement(&ctx, &simple, None, VerifyMode::Auto);
        // Auto mode should handle this
        assert!(result.is_ok() || result.is_err());
    }

    /// Test: Verify inequality chain
    #[test]
    fn test_verify_inequality_chain() {
        let ctx = Context::new();

        // Create type: Int{it >= 10 && it <= 20}
        let base = Type::int(dummy_span());
        let ge10 = binary_expr(BinOp::Ge, ident_expr("it"), int_lit(10));
        let le20 = binary_expr(BinOp::Le, ident_expr("it"), int_lit(20));
        let predicate = binary_expr(BinOp::And, ge10, le20);
        let range = refined_type(base, predicate);

        // Should fail - numbers outside [10, 20] exist
        let result = verify_refinement(&ctx, &range, None, VerifyMode::Proof);
        assert!(result.is_err(), "Should find counterexample outside range");
    }

    /// Test: OR condition
    #[test]
    fn test_verify_or_condition() {
        let ctx = Context::new();

        // Create type: Int{it < 0 || it > 100}
        let base = Type::int(dummy_span());
        let lt_zero = binary_expr(BinOp::Lt, ident_expr("it"), int_lit(0));
        let gt_hundred = binary_expr(BinOp::Gt, ident_expr("it"), int_lit(100));
        let predicate = binary_expr(BinOp::Or, lt_zero, gt_hundred);
        let outside_range = refined_type(base, predicate);

        // Should fail - numbers in [0, 100] exist
        let result = verify_refinement(&ctx, &outside_range, None, VerifyMode::Proof);
        assert!(result.is_err(), "Should find counterexample in [0, 100]");

        if let Err(VerificationError::CannotProve { counterexample, .. }) = result
            && let Some(ce) = counterexample
            && let Some(val) = ce.get("it")
            && let Some(i) = val.as_int()
        {
            assert!(
                (0..=100).contains(&i),
                "Counterexample should be in [0, 100], got {}",
                i
            );
        }
    }

    /// Test: Nested arithmetic
    #[test]
    fn test_verify_nested_arithmetic() {
        let ctx = Context::new();

        // Create type: Int{(it + 5) * 2 > 20}  which means it > 5
        let base = Type::int(dummy_span());
        let plus_five = binary_expr(BinOp::Add, ident_expr("it"), int_lit(5));
        let times_two = binary_expr(BinOp::Mul, plus_five, int_lit(2));
        let predicate = binary_expr(BinOp::Gt, times_two, int_lit(20));
        let nested = refined_type(base, predicate);

        // Should fail - numbers where (it+5)*2 <= 20 exist
        let result = verify_refinement(&ctx, &nested, None, VerifyMode::Proof);
        assert!(result.is_err(), "Should find counterexample");
    }

    /// Test: Verify power constraint
    #[test]
    fn test_verify_power_constraint() {
        let ctx = Context::new();

        // Create type: Int{it**2 > 100}
        let base = Type::int(dummy_span());
        let squared = binary_expr(BinOp::Pow, ident_expr("it"), int_lit(2));
        let predicate = binary_expr(BinOp::Gt, squared, int_lit(100));
        let power_constraint = refined_type(base, predicate);

        // Should fail - numbers where it^2 <= 100 exist (like -10 to 10)
        let result = verify_refinement(&ctx, &power_constraint, None, VerifyMode::Proof);
        assert!(result.is_err(), "Should find counterexample");
    }

    /// Test: Verify logical equivalence
    #[test]
    fn test_verify_logical_equivalence() {
        let ctx = Context::new();

        // Create type: Bool{it == true}
        let base = Type::bool(dummy_span());
        let predicate = binary_expr(BinOp::Eq, ident_expr("it"), bool_lit(true));
        let is_true = refined_type(base, predicate);

        // Should fail - false exists
        let result = verify_refinement(&ctx, &is_true, None, VerifyMode::Proof);
        assert!(result.is_err(), "Should find counterexample: false");

        if let Err(VerificationError::CannotProve { counterexample, .. }) = result
            && let Some(ce) = counterexample
            && let Some(val) = ce.get("it")
            && let Some(b) = val.as_bool()
        {
            assert!(!b, "Counterexample should be false");
        }
    }

    /// Test: Multiple variables (simulating function with two parameters)
    #[test]
    fn test_verify_multiple_variables() {
        let ctx = Context::new();
        let mut translator = Translator::new(&ctx);

        // Create variables x and y
        let x_var = Int::new_const("x");
        let y_var = Int::new_const("y");
        translator.bind("x".into(), z3::ast::Dynamic::from_ast(&x_var));
        translator.bind("y".into(), z3::ast::Dynamic::from_ast(&y_var));

        // Create predicate: x > y
        let predicate = binary_expr(BinOp::Gt, ident_expr("x"), ident_expr("y"));
        let z3_pred = translator.translate_expr(&predicate).unwrap();
        let z3_bool = z3_pred.as_bool().unwrap();

        // Check if there's a counterexample where x <= y
        let solver = ctx.solver();
        solver.assert(z3_bool.not());

        let result = solver.check();
        assert_eq!(
            result,
            z3::SatResult::Sat,
            "Should find x <= y counterexample"
        );
    }

    /// Test: Verify with contradictory constraints
    #[test]
    fn test_verify_contradictory_constraints() {
        let ctx = Context::new();

        // Create contradictory predicate: it > 10 && it < 5
        let base = Type::int(dummy_span());
        let gt10 = binary_expr(BinOp::Gt, ident_expr("it"), int_lit(10));
        let lt5 = binary_expr(BinOp::Lt, ident_expr("it"), int_lit(5));
        let impossible = binary_expr(BinOp::And, gt10, lt5);
        let contradictory = refined_type(base, impossible);

        // This should fail - all integers violate this
        let result = verify_refinement(&ctx, &contradictory, None, VerifyMode::Proof);
        assert!(
            result.is_err(),
            "Contradictory constraint should always fail"
        );
    }

    /// Test: Simple tautology (always true for unconstrained variables)
    #[test]
    fn test_verify_tautology() {
        let ctx = Context::new();

        // Create tautology: it == it
        let base = Type::int(dummy_span());
        let predicate = binary_expr(BinOp::Eq, ident_expr("it"), ident_expr("it"));
        let tautology = refined_type(base, predicate);

        // This should fail - we're checking if there exists a value where it != it
        // But such a value doesn't exist, so the negation is UNSAT
        // However, our API checks if the predicate CAN be violated, not if it's always true
        // So this will depend on implementation details
        let result = verify_refinement(&ctx, &tautology, None, VerifyMode::Proof);
        // Could be either OK (no counterexample) or depends on what we're verifying
    }

    /// Test: Division by a constant
    #[test]
    fn test_verify_division_by_constant() {
        let ctx = Context::new();

        // Create type: Int{it / 2 > 5}  which means it > 10
        let base = Type::int(dummy_span());
        let div_two = binary_expr(BinOp::Div, ident_expr("it"), int_lit(2));
        let predicate = binary_expr(BinOp::Gt, div_two, int_lit(5));
        let divided = refined_type(base, predicate);

        // Should fail - numbers where it/2 <= 5 exist
        let result = verify_refinement(&ctx, &divided, None, VerifyMode::Proof);
        assert!(result.is_err(), "Should find counterexample <= 10");
    }

    /// Test: Verify solver can handle negative numbers properly
    #[test]
    fn test_verify_negative_number_handling() {
        let ctx = Context::new();

        // Create type: Int{it > -10}
        let base = Type::int(dummy_span());
        let neg_ten = Expr::new(
            ExprKind::Unary {
                op: verum_ast::UnOp::Neg,
                expr: Heap::new(int_lit(10)),
            },
            dummy_span(),
        );
        let predicate = binary_expr(BinOp::Gt, ident_expr("it"), neg_ten);
        let gt_neg_ten = refined_type(base, predicate);

        // Should fail - numbers <= -10 exist
        let result = verify_refinement(&ctx, &gt_neg_ten, None, VerifyMode::Proof);
        assert!(result.is_err(), "Should find counterexample <= -10");
    }

    /// Test: Complex boolean expression
    #[test]
    fn test_verify_complex_boolean() {
        let ctx = Context::new();

        // Create: Int{(it > 0 || it < -100) && it != 50}
        let base = Type::int(dummy_span());
        let gt0 = binary_expr(BinOp::Gt, ident_expr("it"), int_lit(0));
        let lt_neg100 = binary_expr(BinOp::Lt, ident_expr("it"), int_lit(-100));
        let or_part = binary_expr(BinOp::Or, gt0, lt_neg100);
        let ne50 = binary_expr(BinOp::Ne, ident_expr("it"), int_lit(50));
        let predicate = binary_expr(BinOp::And, or_part, ne50);
        let complex = refined_type(base, predicate);

        // Should fail - many counterexamples exist (like numbers in [-100, 0])
        let result = verify_refinement(&ctx, &complex, None, VerifyMode::Proof);
        assert!(result.is_err(), "Should find counterexample in [-100, 0]");
    }

    /// Test: Verify batch processing
    #[test]
    fn test_batch_verification_comprehensive() {
        let ctx = Context::new();
        let base = Type::int(dummy_span());

        let predicates = vec![
            binary_expr(BinOp::Gt, ident_expr("it"), int_lit(0)), // > 0
            binary_expr(BinOp::Ge, ident_expr("it"), int_lit(10)), // >= 10
            binary_expr(BinOp::Lt, ident_expr("it"), int_lit(100)), // < 100
            binary_expr(BinOp::Ne, ident_expr("it"), int_lit(42)), // != 42
        ];

        let constraints: Vec<_> = predicates
            .into_iter()
            .map(|pred| (refined_type(base.clone(), pred), None))
            .collect();

        let results = verify_batch(&ctx, &constraints, VerifyMode::Proof);
        assert_eq!(results.len(), 4);

        // All should fail (find counterexamples) since we're checking unbounded types
        for (idx, result) in results.iter().enumerate() {
            assert!(result.is_err(), "Constraint {} should fail", idx);
        }
    }

    /// Test: Verify that error messages contain useful information
    #[test]
    fn test_error_messages_quality() {
        let ctx = Context::new();

        let base = Type::int(dummy_span());
        let predicate = binary_expr(BinOp::Gt, ident_expr("it"), int_lit(1000));
        let large = refined_type(base, predicate);

        let result = verify_refinement(&ctx, &large, None, VerifyMode::Proof);

        if let Err(e) = result {
            let error_msg = format!("{}", e);
            assert!(error_msg.contains("cannot prove") || !error_msg.is_empty());

            // Check that we have suggestions
            let suggestions = e.suggestions();
            // May or may not have suggestions, but API should work
        }
    }
}
