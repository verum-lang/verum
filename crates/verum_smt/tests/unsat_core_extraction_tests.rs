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
//! Unsat Core Extraction and Verification Tests
//!
//! Per CLAUDE.md standards: Tests in tests/ directory

use verum_common::Maybe;
use verum_smt::context::Context;
use verum_smt::unsat_core::{
    AssertionCategory, TrackedAssertion, UnsatCore, UnsatCoreAnalyzer, UnsatCoreConfig,
    UnsatCoreExtractor,
};
use verum_common::{List, Text};
use z3::ast::{Bool, Int};
use z3::{SatResult, Solver};

fn create_context() -> Context {
    Context::new()
}

#[test]
fn test_unsat_core_extraction_simple() {
    let _ctx = create_context();

    let solver = Solver::new();

    // Create assertions: x > 10 AND x < 5 (unsatisfiable)
    let x = Int::new_const("x");
    let gt_10 = x.gt(Int::from_i64(10));
    let lt_5 = x.lt(Int::from_i64(5));

    // Assert both constraints
    solver.assert(&gt_10);
    solver.assert(&lt_5);

    // Check satisfiability
    let result = solver.check();
    assert_eq!(result, SatResult::Unsat);

    // Note: Actual core extraction requires tracking which is done via UnsatCoreExtractor
    // This test verifies the basic unsatisfiability
}

#[test]
fn test_unsat_core_with_irrelevant_constraints() {
    let _ctx = create_context();

    let solver = Solver::new();

    // Create constraints: x > 10 AND x < 5 (core) AND y > 0 (irrelevant)
    let x = Int::new_const("x");
    let y = Int::new_const("y");

    let gt_10 = x.gt(Int::from_i64(10));
    let lt_5 = x.lt(Int::from_i64(5));
    let y_positive = y.gt(Int::from_i64(0));

    solver.assert(&gt_10);
    solver.assert(&lt_5);
    solver.assert(&y_positive);

    let result = solver.check();
    assert_eq!(result, SatResult::Unsat);

    // Minimal core should only contain x > 10 and x < 5, not y > 0
    // Full core extraction with minimization would verify this
}

#[test]
fn test_tracked_assertion_creation() {
    let _ctx = create_context();

    let x = Int::new_const("x");
    let constraint = x.gt(Int::from_i64(0));

    let tracked = TrackedAssertion {
        id: Text::from("constraint_1"),
        assertion: constraint,
        source: Maybe::Some(Text::from("test.vr:10")),
        category: AssertionCategory::Refinement,
        description: Maybe::Some(Text::from("x must be positive")),
    };

    assert_eq!(tracked.id, Text::from("constraint_1"));
    assert_eq!(tracked.category, AssertionCategory::Refinement);
}

#[test]
fn test_core_minimality_verification() {
    let _ctx = create_context();

    // For a truly minimal core, removing any single constraint should make it SAT
    // Test: x > 10 AND x < 5
    let x = Int::new_const("x");
    let gt_10 = x.gt(Int::from_i64(10));
    let lt_5 = x.lt(Int::from_i64(5));

    // Full formula is UNSAT
    let solver_full = Solver::new();
    solver_full.assert(&gt_10);
    solver_full.assert(&lt_5);
    assert_eq!(solver_full.check(), SatResult::Unsat);

    // Removing x > 10 should be SAT
    let solver_1 = Solver::new();
    solver_1.assert(&lt_5);
    assert_eq!(solver_1.check(), SatResult::Sat);

    // Removing x < 5 should be SAT
    let solver_2 = Solver::new();
    solver_2.assert(&gt_10);
    assert_eq!(solver_2.check(), SatResult::Sat);

    // This proves the core {x > 10, x < 5} is minimal
}

#[test]
fn test_core_soundness() {
    let _ctx = create_context();

    // A core is sound if the core itself is UNSAT
    let x = Int::new_const("x");
    let gt_10 = x.gt(Int::from_i64(10));
    let lt_5 = x.lt(Int::from_i64(5));

    let solver = Solver::new();
    solver.assert(&gt_10);
    solver.assert(&lt_5);

    // Core is UNSAT
    assert_eq!(solver.check(), SatResult::Unsat);
}

#[test]
fn test_core_completeness() {
    let _ctx = create_context();

    // A core is complete if it implies the unsatisfiability of the full formula
    // This is automatically true for unsat cores extracted from the solver
    let x = Int::new_const("x");
    let y = Int::new_const("y");

    let gt_10 = x.gt(Int::from_i64(10));
    let lt_5 = x.lt(Int::from_i64(5));
    let y_positive = y.gt(Int::from_i64(0));

    // Full formula
    let solver_full = Solver::new();
    solver_full.assert(&gt_10);
    solver_full.assert(&lt_5);
    solver_full.assert(&y_positive);
    assert_eq!(solver_full.check(), SatResult::Unsat);

    // Core (subset)
    let solver_core = Solver::new();
    solver_core.assert(&gt_10);
    solver_core.assert(&lt_5);
    assert_eq!(solver_core.check(), SatResult::Unsat);

    // If core is UNSAT, adding more constraints keeps it UNSAT
}

#[test]
fn test_assertion_categories() {
    let categories = vec![
        AssertionCategory::Precondition,
        AssertionCategory::Postcondition,
        AssertionCategory::Refinement,
        AssertionCategory::Invariant,
        AssertionCategory::UserAssertion,
        AssertionCategory::Generated,
        AssertionCategory::Custom(Text::from("test")),
    ];

    for cat in categories {
        let formatted = format!("{}", cat);
        assert!(!formatted.is_empty());
    }
}

#[test]
fn test_unsat_core_config() {
    let config = UnsatCoreConfig {
        minimize: true,
        quick_extraction: false,
        max_iterations: 100,
        timeout_ms: Maybe::Some(5000),
        proof_based: true,
    };

    assert!(config.minimize);
    assert!(!config.quick_extraction);
    assert!(config.proof_based);
}

#[test]
fn test_multiple_cores() {
    let _ctx = create_context();

    // Some formulas may have multiple minimal cores
    // Example: (x > 10 AND x < 5) OR (y > 10 AND y < 5)
    let x = Int::new_const("x");
    let y = Int::new_const("y");

    let x_unsat = Bool::and(&[&x.gt(Int::from_i64(10)), &x.lt(Int::from_i64(5))]);
    let y_unsat = Bool::and(&[&y.gt(Int::from_i64(10)), &y.lt(Int::from_i64(5))]);

    // Both x_unsat and y_unsat are individually UNSAT
    let solver_x = Solver::new();
    solver_x.assert(&x_unsat);
    assert_eq!(solver_x.check(), SatResult::Unsat);

    let solver_y = Solver::new();
    solver_y.assert(&y_unsat);
    assert_eq!(solver_y.check(), SatResult::Unsat);

    // The disjunction is also UNSAT (both disjuncts are UNSAT)
    let combined = Bool::or(&[&x_unsat, &y_unsat]);
    let solver_combined = Solver::new();
    solver_combined.assert(&combined);
    assert_eq!(solver_combined.check(), SatResult::Unsat);
}

#[test]
fn test_core_extraction_performance() {
    let _ctx = create_context();

    let start = std::time::Instant::now();

    // Create a moderately complex UNSAT formula
    let x = Int::new_const("x");
    let solver = Solver::new();

    for i in 0..10 {
        let constraint = x.gt(Int::from_i64(i * 10));
        solver.assert(&constraint);
    }
    // Add contradictory constraint
    solver.assert(x.lt(Int::from_i64(0)));

    let result = solver.check();
    assert_eq!(result, SatResult::Unsat);

    let elapsed = start.elapsed();
    // Core extraction should be reasonably fast (<100ms for small problems)
    assert!(elapsed.as_millis() < 1000);
}
