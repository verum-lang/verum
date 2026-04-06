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
// Comprehensive SMT solver tests

use verum_common::Heap;
//
// Coverage target: >95% of SMT solver code
//
// Test categories:
// - Basic SAT/UNSAT checking
// - Translation from Verum expressions to Z3
// - Refinement verification
// - Performance
// - Error handling

use verum_ast::{
    expr::{BinOp, Expr, ExprKind, UnOp},
    literal::Literal,
    span::Span,
    ty::{Ident, Path},
};
use verum_smt::solver::{SmtBackend, SmtContext, SmtResult, Z3Backend};
use verum_smt::z3_backend::Z3Config;
use verum_common::Map;

fn dummy_span() -> Span {
    Span::default()
}

fn make_int_literal(value: i64) -> Expr {
    let span = dummy_span();
    Expr::new(ExprKind::Literal(Literal::int(value as i128, span)), span)
}

fn make_bool_literal(value: bool) -> Expr {
    let span = dummy_span();
    Expr::new(ExprKind::Literal(Literal::bool(value, span)), span)
}

fn make_var(name: &str) -> Expr {
    let span = dummy_span();
    Expr::new(ExprKind::Path(Path::single(Ident::new(name, span))), span)
}

fn make_binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        dummy_span(),
    )
}

fn make_unary(op: UnOp, expr: Expr) -> Expr {
    Expr::new(
        ExprKind::Unary {
            op,
            expr: Heap::new(expr),
        },
        dummy_span(),
    )
}

// ==================== Basic SAT/UNSAT Tests ====================

#[test]
fn test_sat_simple_true() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    let expr = make_bool_literal(true);

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Sat => {
            // Expected
        }
        _ => panic!("Expected SAT for true literal"),
    }
}

#[test]
fn test_unsat_false() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    let expr = make_bool_literal(false);

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Unsat(_) => {
            // Expected
        }
        _ => panic!("Expected UNSAT for false literal"),
    }
}

#[test]
fn test_sat_simple_and() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    let expr = make_binary(BinOp::And, make_bool_literal(true), make_bool_literal(true));

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Sat => {
            // Expected
        }
        _ => panic!("Expected SAT for true AND true"),
    }
}

#[test]
fn test_unsat_contradiction() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    let expr = make_binary(
        BinOp::And,
        make_bool_literal(true),
        make_bool_literal(false),
    );

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Unsat(_) => {
            // Expected
        }
        _ => panic!("Expected UNSAT for true AND false"),
    }
}

#[test]
fn test_sat_or() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    let expr = make_binary(BinOp::Or, make_bool_literal(true), make_bool_literal(false));

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Sat => {
            // Expected
        }
        _ => panic!("Expected SAT for true OR false"),
    }
}

#[test]
fn test_sat_not() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    let expr = make_unary(UnOp::Not, make_bool_literal(false));

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Sat => {
            // Expected - NOT false = true
        }
        _ => panic!("Expected SAT for NOT false"),
    }
}

// ==================== Integer Arithmetic Tests ====================

#[test]
fn test_sat_int_comparison_true() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    // 10 < 20
    let expr = make_binary(BinOp::Lt, make_int_literal(10), make_int_literal(20));

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Sat => {
            // Expected
        }
        _ => panic!("Expected SAT for 10 < 20"),
    }
}

#[test]
fn test_unsat_int_comparison_false() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    // 20 < 10
    let expr = make_binary(BinOp::Lt, make_int_literal(20), make_int_literal(10));

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Unsat(_) => {
            // Expected
        }
        _ => panic!("Expected UNSAT for 20 < 10"),
    }
}

#[test]
fn test_sat_int_equality() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    // 42 == 42
    let expr = make_binary(BinOp::Eq, make_int_literal(42), make_int_literal(42));

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Sat => {
            // Expected
        }
        _ => panic!("Expected SAT for 42 == 42"),
    }
}

#[test]
fn test_unsat_int_inequality() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    // 42 == 24
    let expr = make_binary(BinOp::Eq, make_int_literal(42), make_int_literal(24));

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Unsat(_) => {
            // Expected
        }
        _ => panic!("Expected UNSAT for 42 == 24"),
    }
}

#[test]
fn test_sat_int_arithmetic() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    // (10 + 5) == 15
    let left = make_binary(BinOp::Add, make_int_literal(10), make_int_literal(5));
    let expr = make_binary(BinOp::Eq, left, make_int_literal(15));

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Sat => {
            // Expected
        }
        _ => panic!("Expected SAT for (10 + 5) == 15"),
    }
}

#[test]
fn test_unsat_int_arithmetic_wrong() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    // (10 + 5) == 20
    let left = make_binary(BinOp::Add, make_int_literal(10), make_int_literal(5));
    let expr = make_binary(BinOp::Eq, left, make_int_literal(20));

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Unsat(_) => {
            // Expected
        }
        _ => panic!("Expected UNSAT for (10 + 5) == 20"),
    }
}

// ==================== Variable Tests ====================

#[test]
fn test_sat_with_variable() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    // x > 0 (should be SAT - can find x = 1)
    let expr = make_binary(BinOp::Gt, make_var("x"), make_int_literal(0));

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Sat => {
            // Expected - SAT with x = any positive number
        }
        _ => panic!("Expected SAT for x > 0"),
    }
}

#[test]
fn test_unsat_contradictory_constraints() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    // x > 10 AND x < 5 (UNSAT)
    let c1 = make_binary(BinOp::Gt, make_var("x"), make_int_literal(10));
    let c2 = make_binary(BinOp::Lt, make_var("x"), make_int_literal(5));
    let expr = make_binary(BinOp::And, c1, c2);

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Unsat(_) => {
            // Expected
        }
        _ => panic!("Expected UNSAT for x > 10 AND x < 5"),
    }
}

#[test]
fn test_sat_consistent_constraints() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    // x > 5 AND x < 10 (SAT - e.g., x = 7)
    let c1 = make_binary(BinOp::Gt, make_var("x"), make_int_literal(5));
    let c2 = make_binary(BinOp::Lt, make_var("x"), make_int_literal(10));
    let expr = make_binary(BinOp::And, c1, c2);

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Sat => {
            // Expected
        }
        _ => panic!("Expected SAT for x > 5 AND x < 10"),
    }
}

// ==================== Refinement Type Tests ====================

#[test]
fn test_verify_positive_refinement_valid() {
    let backend = Z3Backend::new(Z3Config::default());

    // Verify: 42 > 0
    let predicate = make_binary(BinOp::Gt, make_var("it"), make_int_literal(0));

    let mut bindings = Map::new();
    bindings.insert("it".into(), Literal::int(42i128, dummy_span()));

    let result = backend.verify_predicate(&predicate, &bindings);

    assert!(result.is_ok());
    assert!(result.unwrap(), "42 should satisfy > 0");
}

#[test]
fn test_verify_positive_refinement_invalid() {
    let backend = Z3Backend::new(Z3Config::default());

    // Verify: -5 > 0
    let predicate = make_binary(BinOp::Gt, make_var("it"), make_int_literal(0));

    let mut bindings = Map::new();
    bindings.insert("it".into(), Literal::int(-5i128, dummy_span()));

    let result = backend.verify_predicate(&predicate, &bindings);

    assert!(result.is_ok());
    assert!(!result.unwrap(), "-5 should not satisfy > 0");
}

#[test]
fn test_verify_range_refinement_valid() {
    let backend = Z3Backend::new(Z3Config::default());

    // Verify: 50 in range [0, 100]
    // 0 <= it AND it <= 100
    let lower = make_binary(BinOp::Ge, make_var("it"), make_int_literal(0));
    let upper = make_binary(BinOp::Le, make_var("it"), make_int_literal(100));
    let predicate = make_binary(BinOp::And, lower, upper);

    let mut bindings = Map::new();
    bindings.insert("it".into(), Literal::int(50i128, dummy_span()));

    let result = backend.verify_predicate(&predicate, &bindings);

    assert!(result.is_ok());
    assert!(result.unwrap(), "50 should be in range [0, 100]");
}

#[test]
fn test_verify_range_refinement_invalid() {
    let backend = Z3Backend::new(Z3Config::default());

    // Verify: 150 in range [0, 100]
    let lower = make_binary(BinOp::Ge, make_var("it"), make_int_literal(0));
    let upper = make_binary(BinOp::Le, make_var("it"), make_int_literal(100));
    let predicate = make_binary(BinOp::And, lower, upper);

    let mut bindings = Map::new();
    bindings.insert("it".into(), Literal::int(150i128, dummy_span()));

    let result = backend.verify_predicate(&predicate, &bindings);

    assert!(result.is_ok());
    assert!(!result.unwrap(), "150 should not be in range [0, 100]");
}

#[test]
fn test_verify_even_refinement() {
    let backend = Z3Backend::new(Z3Config::default());

    // Verify: it % 2 == 0
    let modulo = make_binary(BinOp::Rem, make_var("it"), make_int_literal(2));
    let predicate = make_binary(BinOp::Eq, modulo, make_int_literal(0));

    let mut bindings_even = Map::new();
    bindings_even.insert("it".into(), Literal::int(42i128, dummy_span()));

    let result_even = backend.verify_predicate(&predicate, &bindings_even);
    assert!(result_even.is_ok());
    assert!(result_even.unwrap(), "42 should be even");

    let mut bindings_odd = Map::new();
    bindings_odd.insert("it".into(), Literal::int(43i128, dummy_span()));

    let result_odd = backend.verify_predicate(&predicate, &bindings_odd);
    assert!(result_odd.is_ok());
    assert!(!result_odd.unwrap(), "43 should not be even");
}

// ==================== Complex Formula Tests ====================

#[test]
fn test_complex_nested_formula() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    // ((x > 0) AND (y > 0)) => (x + y > 0)
    let x_pos = make_binary(BinOp::Gt, make_var("x"), make_int_literal(0));
    let y_pos = make_binary(BinOp::Gt, make_var("y"), make_int_literal(0));
    let antecedent = make_binary(BinOp::And, x_pos, y_pos);

    let x_plus_y = make_binary(BinOp::Add, make_var("x"), make_var("y"));
    let consequent = make_binary(BinOp::Gt, x_plus_y, make_int_literal(0));

    // Negate implication to check validity: NOT (A => B) should be UNSAT
    // A => B is equivalent to NOT A OR B
    // So NOT (A => B) is equivalent to A AND NOT B
    let negated_consequent = make_unary(UnOp::Not, consequent);
    let expr = make_binary(BinOp::And, antecedent, negated_consequent);

    let result = backend.check_sat(&expr, &context);

    match result {
        SmtResult::Unsat(_) => {
            // Expected - implication is valid
        }
        _ => panic!("Expected UNSAT for validity check of (x > 0 ∧ y > 0) => (x + y > 0)"),
    }
}

// ==================== Performance Tests ====================

#[test]
fn test_performance_simple_query() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    let expr = make_binary(BinOp::Lt, make_int_literal(10), make_int_literal(20));

    let start = std::time::Instant::now();
    let _result = backend.check_sat(&expr, &context);
    let elapsed = start.elapsed();

    // Allow more slack for CI and different machine configurations
    // Z3 initialization can have variable latency
    assert!(
        elapsed.as_millis() < 100,
        "Simple query took too long: {}ms (expected < 100ms)",
        elapsed.as_millis()
    );
}

#[test]
fn test_performance_complex_query() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    // Build a complex formula: x1 > 0 AND x2 > 0 AND ... AND x10 > 0
    let mut expr = make_binary(BinOp::Gt, make_var("x0"), make_int_literal(0));

    for i in 1..10 {
        let var_name = format!("x{}", i);
        let constraint = make_binary(BinOp::Gt, make_var(&var_name), make_int_literal(0));
        expr = make_binary(BinOp::And, expr, constraint);
    }

    let start = std::time::Instant::now();
    let _result = backend.check_sat(&expr, &context);
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_millis() < 100,
        "Complex query took too long: {}ms (target < 100ms)",
        elapsed.as_millis()
    );
}

#[test]
fn test_performance_target_average() {
    let backend = Z3Backend::new(Z3Config::default());
    let context = SmtContext::default();

    let iterations = 100;
    let mut total_time = std::time::Duration::ZERO;

    for i in 0..iterations {
        let expr = make_binary(BinOp::Lt, make_int_literal(i), make_int_literal(i + 1));

        let start = std::time::Instant::now();
        let _result = backend.check_sat(&expr, &context);
        total_time += start.elapsed();
    }

    let average_ms = total_time.as_micros() as f64 / iterations as f64 / 1000.0;

    println!("Average query time: {:.2}ms", average_ms);

    // Target: < 10ms average (spec requirement)
    assert!(
        average_ms < 10.0,
        "Average query time {}ms exceeds 10ms target",
        average_ms
    );
}
