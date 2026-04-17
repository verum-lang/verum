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
//! Comprehensive SMT Solver Tests
//!
//! Exhaustive testing of Z3 integration and SMT verification.
//! Coverage target: 60% → 95%
//!
//! Test categories:
//! - Z3 basic theories (arithmetic, boolean, arrays, bitvectors)
//! - Quantifier elimination and instantiation
//! - Unsat core extraction
//! - Model generation and evaluation
//! - Timeout and resource limits
//! - Tactics and strategies
//! - Parallel solving
//! - Refinement type verification
//!
//! IMPORTANT: These tests use deprecated Z3 API patterns from z3-rs < 0.19.
//!
//! **WHY DISABLED**: Z3-rs 0.19+ changed the API:
//! - OLD: `Int::new_const("x")`
//! - NEW: `Int::new_const(&ctx, "x")` (requires context parameter)
//! - OLD: `x.add(&[&y])`
//! - NEW: `Int::add(&ctx, &[&x, &y])` (static method with context)
//!
//! **TO FIX**: Requires rewriting all 851 lines of tests to:
//! 1. Create Z3 Config and Context in each test
//! 2. Pass context to all Int/Bool/Array construction
//! 3. Use new static method signatures
//!
//! **ESTIMATED EFFORT**: 20-30 hours
//!
//! **DECISION NEEDED**:
//! - Option A: Budget time to rewrite using current Z3 API
//! - Option B: Delete and create focused integration tests when needed
//! - Option C: Create test harness that manages Z3 contexts
//!
//! Tests require Z3 API migration: forall_const, exists_const, Context params (~20-30 hours).

// REQUIRES Z3 API MIGRATION (~20-30 hours): forall_const, exists_const, Context params
#![cfg(feature = "z3_old_api_tests_disabled")]

use std::time::Duration;
use verum_smt::*;
use z3::ast::{Array, Ast, BV, Bool, Datatype, Int, Real};
use z3::*;

// ============================================================================
// Basic Theory Tests
// ============================================================================

#[test]
fn test_linear_integer_arithmetic() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    // Create variables
    let x = Int::new_const("x");
    let y = Int::new_const("y");
    let z = Int::new_const("z");

    // Add constraints: x + y = 10, y + z = 15, z - x = 5
    solver.assert(&x.add(&[&y])._eq(&Int::from_i64(10)));
    solver.assert(&y.add(&[&z])._eq(&Int::from_i64(15)));
    solver.assert(&z.sub(&[&x])._eq(&Int::from_i64(5)));

    assert_eq!(solver.check(), SatResult::Sat);

    let model = solver.get_model().unwrap();
    let x_val = model.eval(&x, true).unwrap().as_i64().unwrap();
    let y_val = model.eval(&y, true).unwrap().as_i64().unwrap();
    let z_val = model.eval(&z, true).unwrap().as_i64().unwrap();

    // Verify solution
    assert_eq!(x_val + y_val, 10);
    assert_eq!(y_val + z_val, 15);
    assert_eq!(z_val - x_val, 5);

    println!("Solution: x={}, y={}, z={}", x_val, y_val, z_val);
}

#[test]
fn test_nonlinear_arithmetic() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    // x² + y² = 25, x + y = 7
    let x_squared = x.mul(&[&x]);
    let y_squared = y.mul(&[&y]);

    solver.assert(&x_squared.add(&[&y_squared])._eq(&Int::from_i64(25)));
    solver.assert(&x.add(&[&y])._eq(&Int::from_i64(7)));

    assert_eq!(solver.check(), SatResult::Sat);

    let model = solver.get_model().unwrap();
    let x_val = model.eval(&x, true).unwrap().as_i64().unwrap();
    let y_val = model.eval(&y, true).unwrap().as_i64().unwrap();

    println!("Solution: x={}, y={}", x_val, y_val);

    // Verify: 3² + 4² = 25, 3 + 4 = 7 (or 4, 3)
    assert!((x_val == 3 && y_val == 4) || (x_val == 4 && y_val == 3));
}

#[test]
fn test_boolean_satisfiability() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let a = Bool::new_const("a");
    let b = Bool::new_const("b");
    let c = Bool::new_const("c");

    // (a ∨ b) ∧ (¬a ∨ c) ∧ (¬b ∨ ¬c)
    solver.assert(&Bool::or(&ctx, &[&a, &b]));
    solver.assert(&Bool::or(&ctx, &[&a.not(), &c]));
    solver.assert(&Bool::or(&ctx, &[&b.not(), &c.not()]));

    assert_eq!(solver.check(), SatResult::Sat);

    let model = solver.get_model().unwrap();
    let a_val = model.eval(&a, true).unwrap().as_bool().unwrap();
    let b_val = model.eval(&b, true).unwrap().as_bool().unwrap();
    let c_val = model.eval(&c, true).unwrap().as_bool().unwrap();

    println!("Solution: a={}, b={}, c={}", a_val, b_val, c_val);

    // Verify solution satisfies all clauses
    assert!(a_val || b_val);
    assert!(!a_val || c_val);
    assert!(!b_val || !c_val);
}

#[test]
fn test_real_arithmetic() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = Real::new_const("x");
    let y = Real::new_const("y");

    // x + y > 0, x < 1, y < 1, x + y < 2
    solver.assert(&x.add(&[&y]).gt(&Real::from_real(0, 1)));
    solver.assert(&x.lt(&Real::from_real(1, 1)));
    solver.assert(&y.lt(&Real::from_real(1, 1)));
    solver.assert(&x.add(&[&y]).lt(&Real::from_real(2, 1)));

    assert_eq!(solver.check(), SatResult::Sat);

    let model = solver.get_model().unwrap();
    println!("Model: {:?}", model);
}

// ============================================================================
// Array Theory Tests
// ============================================================================

#[test]
fn test_array_theory_basic() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    // Create array: Index -> Int
    let arr = Array::new_const(&ctx, "arr", &Sort::int(&ctx), &Sort::int(&ctx));

    let zero = Int::from_i64(0);
    let one = Int::from_i64(1);
    let two = Int::from_i64(2);

    // arr[0] = 42, arr[1] = 100
    solver.assert(&arr.select(&zero)._eq(&Int::from_i64(42)));
    solver.assert(&arr.select(&one)._eq(&Int::from_i64(100)));

    // arr[0] + arr[1] = 142
    solver.assert(
        &arr.select(&zero)
            .add(&[&arr.select(&one)])
            ._eq(&Int::from_i64(142)),
    );

    assert_eq!(solver.check(), SatResult::Sat);

    let model = solver.get_model().unwrap();
    let arr_val = model.eval(&arr, true).unwrap();

    println!("Array model: {:?}", arr_val);
}

#[test]
fn test_array_theory_store() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let arr1 = Array::new_const(&ctx, "arr1", &Sort::int(&ctx), &Sort::int(&ctx));
    let zero = Int::from_i64(0);

    // arr2 = store(arr1, 0, 42)
    let arr2 = arr1.store(&zero, &Int::from_i64(42));

    // arr2[0] should be 42
    solver.assert(&arr2.select(&zero)._eq(&Int::from_i64(42)));

    // For any index i != 0, arr1[i] = arr2[i]
    let i = Int::new_const("i");
    solver.assert(
        &i._eq(&zero)
            .not()
            .implies(&arr1.select(&i)._eq(&arr2.select(&i))),
    );

    assert_eq!(solver.check(), SatResult::Sat);
}

#[test]
fn test_array_extensionality() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let arr1 = Array::new_const(&ctx, "arr1", &Sort::int(&ctx), &Sort::int(&ctx));
    let arr2 = Array::new_const(&ctx, "arr2", &Sort::int(&ctx), &Sort::int(&ctx));

    // ∀i. arr1[i] = arr2[i]
    let i = Int::new_const("i");
    let forall_expr = forall_const(
        &ctx,
        &[&i.into()],
        &[],
        &arr1.select(&i)._eq(&arr2.select(&i)),
    );

    solver.assert(&forall_expr);

    // arr1 ≠ arr2
    solver.assert(&arr1._eq(&arr2).not());

    // Should be UNSAT (extensionality)
    assert_eq!(solver.check(), SatResult::Unsat);
}

// ============================================================================
// Bitvector Tests
// ============================================================================

#[test]
fn test_bitvector_arithmetic() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = BV::new_const("x", 32);
    let y = BV::new_const("y", 32);

    // x + y = 100 (32-bit)
    solver.assert(&x.bvadd(&y)._eq(&BV::from_i64(100, 32)));

    // x < 50
    solver.assert(&x.bvslt(&BV::from_i64(50, 32)));

    assert_eq!(solver.check(), SatResult::Sat);

    let model = solver.get_model().unwrap();
    let x_val = model.eval(&x, true).unwrap();
    let y_val = model.eval(&y, true).unwrap();

    println!("Bitvector solution: x={:?}, y={:?}", x_val, y_val);
}

#[test]
fn test_bitvector_overflow() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = BV::new_const("x", 8); // 8-bit

    // x = 255
    solver.assert(&x._eq(&BV::from_i64(255, 8)));

    // x + 1 = 0 (overflow)
    let one = BV::from_i64(1, 8);
    solver.assert(&x.bvadd(&one)._eq(&BV::from_i64(0, 8)));

    assert_eq!(solver.check(), SatResult::Sat);
}

#[test]
fn test_bitvector_shifts() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = BV::new_const("x", 8);
    let shift_amt = BV::from_i64(2, 8);

    // x << 2 = 8, so x = 2
    solver.assert(&x.bvshl(&shift_amt)._eq(&BV::from_i64(8, 8)));

    assert_eq!(solver.check(), SatResult::Sat);

    let model = solver.get_model().unwrap();
    let x_val = model.eval(&x, true).unwrap();

    println!("x = {:?}", x_val);
}

// ============================================================================
// Quantifier Tests
// ============================================================================

#[test]
fn test_universal_quantifier() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = Int::new_const("x");

    // ∀x. x + 0 = x
    let forall_expr = forall_const(
        &ctx,
        &[&x.into()],
        &[],
        &x.add(&[&Int::from_i64(0)])._eq(&x),
    );

    solver.assert(&forall_expr);

    // Should be SAT (tautology)
    assert_eq!(solver.check(), SatResult::Sat);
}

#[test]
fn test_existential_quantifier() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = Int::new_const("x");

    // ∃x. (x > 0 ∧ x < 10)
    let exists_body = Bool::and(&ctx, &[&x.gt(&Int::from_i64(0)), &x.lt(&Int::from_i64(10))]);

    let exists_expr = exists_const(&ctx, &[&x.into()], &[], &exists_body);

    solver.assert(&exists_expr);

    assert_eq!(solver.check(), SatResult::Sat);

    let model = solver.get_model().unwrap();
    let x_val = model.eval(&x, true).unwrap().as_i64().unwrap();

    println!("Witness: x = {}", x_val);
    assert!(x_val > 0 && x_val < 10);
}

#[test]
fn test_nested_quantifiers() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    // ∀x. ∃y. y > x
    let inner = exists_const(&ctx, &[&y.into()], &[], &y.gt(&x));
    let outer = forall_const(&ctx, &[&x.into()], &[], &inner);

    solver.assert(&outer);

    assert_eq!(solver.check(), SatResult::Sat);
}

#[test]
fn test_quantifier_elimination() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    // ∃x. (x > y ∧ x < y + 10)
    let body = Bool::and(&ctx, &[&x.gt(&y), &x.lt(&y.add(&[&Int::from_i64(10)]))]);

    let exists_expr = exists_const(&ctx, &[&x.into()], &[], &body);

    // Simplify should eliminate quantifier
    let simplified = exists_expr.simplify();

    println!("Original: {}", exists_expr);
    println!("Simplified: {}", simplified);

    // After elimination, should be equivalent to "true" since x can always be y+1
    let solver = Solver::new();
    solver.assert(&simplified);
    assert_eq!(solver.check(), SatResult::Sat);
}

// ============================================================================
// Unsat Core Tests
// ============================================================================

#[test]
fn test_unsat_core_extraction() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = Int::new_const("x");

    // Add tracked assertions
    let c1 = Bool::new_const("c1");
    let c2 = Bool::new_const("c2");
    let c3 = Bool::new_const("c3");

    // x > 10
    solver.assert_and_track(&x.gt(&Int::from_i64(10)), &c1);

    // x < 5
    solver.assert_and_track(&x.lt(&Int::from_i64(5)), &c2);

    // x = 0
    solver.assert_and_track(&x._eq(&Int::from_i64(0)), &c3);

    assert_eq!(solver.check(), SatResult::Unsat);

    let core = solver.get_unsat_core();

    println!("Unsat core: {:?}", core);

    // Core should contain at least c1 and c2 (conflicting)
    assert!(core.len() >= 2);
}

#[test]
fn test_minimal_unsat_core() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = Int::new_const("x");

    let c1 = Bool::new_const("c1");
    let c2 = Bool::new_const("c2");
    let c3 = Bool::new_const("c3");
    let c4 = Bool::new_const("c4");

    // x > 10
    solver.assert_and_track(&x.gt(&Int::from_i64(10)), &c1);

    // x < 5 (conflicts with c1)
    solver.assert_and_track(&x.lt(&Int::from_i64(5)), &c2);

    // x >= 0 (not needed for conflict)
    solver.assert_and_track(&x.ge(&Int::from_i64(0)), &c3);

    // x < 100 (not needed for conflict)
    solver.assert_and_track(&x.lt(&Int::from_i64(100)), &c4);

    assert_eq!(solver.check(), SatResult::Unsat);

    let core = solver.get_unsat_core();

    println!("Unsat core size: {}", core.len());
    println!("Unsat core: {:?}", core);

    // Minimal core should be just c1 and c2
    assert_eq!(core.len(), 2);
}

// ============================================================================
// Model Generation Tests
// ============================================================================

#[test]
fn test_model_evaluation() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    solver.assert(&x.add(&[&y])._eq(&Int::from_i64(10)));
    solver.assert(&x.gt(&Int::from_i64(5)));

    assert_eq!(solver.check(), SatResult::Sat);

    let model = solver.get_model().unwrap();

    // Evaluate x, y
    let x_val = model.eval(&x, true).unwrap().as_i64().unwrap();
    let y_val = model.eval(&y, true).unwrap().as_i64().unwrap();

    println!("Model: x={}, y={}", x_val, y_val);

    // Evaluate compound expression x + y
    let sum = x.add(&[&y]);
    let sum_val = model.eval(&sum, true).unwrap().as_i64().unwrap();

    assert_eq!(sum_val, 10);
    assert_eq!(x_val + y_val, 10);
    assert!(x_val > 5);
}

#[test]
fn test_model_completion() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = Int::new_const("x");
    let y = Int::new_const("y");
    let z = Int::new_const("z");

    // Only constrain x
    solver.assert(&x._eq(&Int::from_i64(42)));

    assert_eq!(solver.check(), SatResult::Sat);

    let model = solver.get_model().unwrap();

    // y and z should have default values (model completion)
    let x_val = model.eval(&x, true).unwrap().as_i64().unwrap();
    let y_val = model.eval(&y, true);
    let z_val = model.eval(&z, true);

    assert_eq!(x_val, 42);

    println!("x={}, y={:?}, z={:?}", x_val, y_val, z_val);

    // y and z get arbitrary values from model completion
    assert!(y_val.is_some());
    assert!(z_val.is_some());
}

// ============================================================================
// Timeout and Resource Limit Tests
// ============================================================================

#[test]
fn test_solver_timeout() {
    let mut cfg = Config::new();
    cfg.set_timeout_msec(100); // 100ms timeout

    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    // Create a hard problem (lots of quantifiers)
    for i in 0..100 {
        let x = Int::new_const(format!("x{}", i));
        let y = Int::new_const(format!("y{}", i));

        let body = x.add(&[&y])._eq(&Int::from_i64(i));
        let forall = forall_const(&ctx, &[&x.into(), &y.into()], &[], &body);

        solver.assert(&forall);
    }

    let result = solver.check();

    // Should timeout or return unknown
    println!("Result with timeout: {:?}", result);

    assert!(matches!(result, SatResult::Unknown));
}

#[test]
fn test_resource_limits() {
    let mut cfg = Config::new();
    cfg.set_param_value("max_memory", "100"); // 100MB limit

    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    // Try to create a problem that exceeds memory
    // (This is heuristic - may not trigger on all systems)

    for i in 0..10_000 {
        let x = Int::new_const(format!("x{}", i));
        solver.assert(&x.gt(&Int::from_i64(i)));
    }

    let _ = solver.check();

    // Test passes if we don't crash
}

// ============================================================================
// Tactics and Strategies Tests
// ============================================================================

#[test]
fn test_solver_with_tactics() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);

    // Use specific tactics
    let tactic = Tactic::new("simplify");
    let solver = tactic.solver();

    let x = Int::new_const("x");

    // x + 0 = x should simplify
    solver.assert(&x.add(&[&Int::from_i64(0)])._eq(&x));

    assert_eq!(solver.check(), SatResult::Sat);
}

#[test]
fn test_combined_tactics() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);

    // Combine tactics: simplify then solve
    let simplify = Tactic::new("simplify");
    let solve = Tactic::new("smt");

    let combined = simplify.and_then(&solve);
    let solver = combined.solver();

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    solver.assert(&x.add(&[&y])._eq(&Int::from_i64(10)));
    solver.assert(&x._eq(&Int::from_i64(3)));

    assert_eq!(solver.check(), SatResult::Sat);

    let model = solver.get_model().unwrap();
    let y_val = model.eval(&y, true).unwrap().as_i64().unwrap();

    assert_eq!(y_val, 7);
}

// ============================================================================
// Refinement Type Verification Tests
// ============================================================================

#[test]
fn test_refinement_type_positive() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    // Type: {x: Int | x > 0}
    let x = Int::new_const("x");

    // Assume x has refinement type
    solver.assert(&x.gt(&Int::from_i64(0)));

    // Check: x + 1 > 0 (should hold)
    let result = Bool::new_const("result");
    solver.assert(&result._eq(&x.add(&[&Int::from_i64(1)]).gt(&Int::from_i64(0))));

    assert_eq!(solver.check(), SatResult::Sat);

    let model = solver.get_model().unwrap();
    let result_val = model.eval(&result, true).unwrap().as_bool().unwrap();

    assert!(result_val); // x + 1 > 0 holds
}

#[test]
fn test_refinement_type_array_bounds() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let arr_len = Int::new_const("arr_len");
    let idx = Int::new_const("idx");

    // Assume array has length arr_len
    solver.assert(&arr_len.gt(&Int::from_i64(0)));

    // Assume idx has refinement: {i: Int | i >= 0 && i < arr_len}
    solver.assert(&idx.ge(&Int::from_i64(0)));
    solver.assert(&idx.lt(&arr_len));

    // Check: accessing arr[idx] is safe (always true by refinement)
    assert_eq!(solver.check(), SatResult::Sat);

    // Now try invalid access: arr[arr_len]
    solver.push();
    solver.assert(&arr_len.lt(&arr_len)); // False
    assert_eq!(solver.check(), SatResult::Unsat);
    solver.pop(1);
}

#[test]
fn test_refinement_type_division_by_zero() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    // Type: {y: Int | y != 0}
    solver.assert(&y._eq(&Int::from_i64(0)).not());

    // Check: x / y is defined
    assert_eq!(solver.check(), SatResult::Sat);

    // Verify y is never 0
    solver.push();
    solver.assert(&y._eq(&Int::from_i64(0)));
    assert_eq!(solver.check(), SatResult::Unsat);
    solver.pop(1);
}

// ============================================================================
// Parallel Solving Tests
// ============================================================================

#[test]
fn test_parallel_independent_queries() {
    use std::thread;

    let handles: Vec<_> = (0..10)
        .map(|i| {
            thread::spawn(move || {
                let cfg = Config::new();
                let ctx = Context::new(&cfg);
                let solver = Solver::new();

                let x = Int::new_const("x");

                // Each thread solves x = i
                solver.assert(&x._eq(&Int::from_i64(i)));

                assert_eq!(solver.check(), SatResult::Sat);

                let model = solver.get_model().unwrap();
                let x_val = model.eval(&x, true).unwrap().as_i64().unwrap();

                assert_eq!(x_val, i);
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }
}

// ============================================================================
// Edge Cases and Regression Tests
// ============================================================================

#[test]
fn test_empty_solver() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    // No assertions - should be SAT
    assert_eq!(solver.check(), SatResult::Sat);
}

#[test]
fn test_trivially_unsat() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    // false
    solver.assert(&Bool::from_bool(&ctx, false));

    assert_eq!(solver.check(), SatResult::Unsat);
}

#[test]
fn test_push_pop_scopes() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = Int::new_const("x");

    // Level 0: x > 0
    solver.assert(&x.gt(&Int::from_i64(0)));
    assert_eq!(solver.check(), SatResult::Sat);

    // Level 1: x < 10
    solver.push();
    solver.assert(&x.lt(&Int::from_i64(10)));
    assert_eq!(solver.check(), SatResult::Sat);

    // Level 2: x = 100 (conflicts with x < 10)
    solver.push();
    solver.assert(&x._eq(&Int::from_i64(100)));
    assert_eq!(solver.check(), SatResult::Unsat);

    // Pop back to level 1
    solver.pop(1);
    assert_eq!(solver.check(), SatResult::Sat);

    // Pop back to level 0
    solver.pop(1);
    assert_eq!(solver.check(), SatResult::Sat);
}

#[test]
fn test_large_constants() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = Int::new_const("x");

    // Very large constant
    let large = Int::from_i64(i64::MAX);

    solver.assert(&x._eq(&large));

    assert_eq!(solver.check(), SatResult::Sat);

    let model = solver.get_model().unwrap();
    let x_val = model.eval(&x, true).unwrap().as_i64().unwrap();

    assert_eq!(x_val, i64::MAX);
}

#[test]
fn test_solver_reset() {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new();

    let x = Int::new_const("x");

    // Add assertion
    solver.assert(&x._eq(&Int::from_i64(42)));
    assert_eq!(solver.check(), SatResult::Sat);

    // Reset
    solver.reset();

    // Should be SAT again (no assertions)
    assert_eq!(solver.check(), SatResult::Sat);

    // Add conflicting assertion
    solver.assert(&x._eq(&Int::from_i64(100)));
    assert_eq!(solver.check(), SatResult::Sat); // New constraint
}
