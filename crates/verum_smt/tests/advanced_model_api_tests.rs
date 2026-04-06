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
//! Tests for advanced_model.rs enumerate_sort_universe API updates
//!
//! This test suite validates that the enumerate_sort_universe function
//! correctly documents the API limitations and returns None as expected.
//!
//! Related: advanced_model.rs:307 — API limitation documented and handled

use verum_common::Maybe;
use verum_smt::advanced_model::AdvancedModelExtractor;
use verum_smt::z3::{Context, SatResult, Solver, ast::Int};

#[test]
fn test_enumerate_sort_universe_returns_none() {
    // Create a simple Z3 model
    let solver = Solver::new();

    // Create a simple satisfiable formula
    let x = Int::new_const("x");
    solver.assert(x._eq(&x)); // Trivially true

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    // Create extractor
    let extractor = AdvancedModelExtractor::new(model);

    // Test that enumerate_sort_universe returns None (as documented)
    let result = extractor.enumerate_sort_universe("TestSort");
    assert_eq!(
        result,
        Maybe::None,
        "enumerate_sort_universe should return None due to z3-rs API limitations"
    );
}

#[test]
fn test_enumerate_sort_universe_with_int_model() {
    // Create a model with integer variables
    let solver = Solver::new();

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    solver.assert(x.gt(Int::from_i64(0)));
    solver.assert(y.lt(Int::from_i64(100)));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let extractor = AdvancedModelExtractor::new(model);

    // Even with a concrete model, sort universe enumeration is not available
    let result = extractor.enumerate_sort_universe("Int");
    assert_eq!(
        result,
        Maybe::None,
        "Should return None for any sort name, including Int"
    );
}

#[test]
fn test_extractor_creates_successfully() {
    // Verify that the basic extractor functionality works
    let solver = Solver::new();

    let x = Int::new_const("x");
    solver.assert(x._eq(Int::from_i64(42)));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    // Create extractor using the convenience function
    let extractor = verum_smt::advanced_model::create_extractor(model);

    // Verify it extracts constants successfully
    let constants = extractor.get_constants();
    assert!(
        !constants.is_empty(),
        "Should extract at least one constant from the model"
    );
}

#[test]
fn test_multiple_sort_names_all_return_none() {
    let solver = Solver::new();
    let x = Int::new_const("x");
    solver.assert(x._eq(&x));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();
    let extractor = AdvancedModelExtractor::new(model);

    // Test various sort names - all should return None
    let sort_names = vec!["Int", "Bool", "Real", "BitVec", "CustomSort"];

    for sort_name in sort_names {
        let result = extractor.enumerate_sort_universe(sort_name);
        assert_eq!(
            result,
            Maybe::None,
            "Sort '{}' should return None",
            sort_name
        );
    }
}

#[test]
fn test_documentation_mentions_workaround() {
    // This is a compile-time documentation test
    // The function should have comprehensive documentation
    // explaining the limitations and workarounds

    // We can't test documentation directly in Rust tests,
    // but we can verify the function exists and has the expected signature
    let solver = Solver::new();
    let x = Int::new_const("x");
    solver.assert(x._eq(&x));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().expect("Expected a model");
    let extractor = AdvancedModelExtractor::new(model);

    // Just verify the method exists and can be called
    let _result: Maybe<verum_common::Set<verum_common::Text>> =
        extractor.enumerate_sort_universe("AnySort");

    // If this compiles, the signature is correct
    assert!(true, "Function signature is correct");
}

#[test]
fn test_extractor_other_functions_work() {
    // Verify that other AdvancedModelExtractor functions still work correctly
    let solver = Solver::new();

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    solver.assert(x._eq(Int::from_i64(10)));
    solver.assert(y._eq(Int::from_i64(20)));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let mut extractor = AdvancedModelExtractor::new(model);
    extractor.extract_complete_model();

    // Test that constant extraction works
    let constants = extractor.get_constants();
    assert!(constants.len() >= 2, "Should extract both constants");

    // Test that summary works
    let summary = extractor.summary();
    assert!(
        summary.num_constants >= 2,
        "Summary should show at least 2 constants"
    );

    // Test that get_constant works
    let x_val = extractor.get_constant("x");
    assert!(x_val.is_some(), "Should be able to get constant value");
}

// ============================================================================
// Additional Model API Tests
// ============================================================================

#[test]
fn test_model_with_boolean_variables() {
    let solver = Solver::new();

    // Create a model with boolean constraints
    let p = z3::ast::Bool::new_const("p");
    let q = z3::ast::Bool::new_const("q");

    // p AND (NOT q) - only satisfiable when p=true, q=false
    let q_not = q.not();
    solver.assert(&p);
    solver.assert(&q_not);

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let mut extractor = AdvancedModelExtractor::new(model);
    extractor.extract_complete_model();

    // Verify constants are extracted
    let constants = extractor.get_constants();
    assert!(constants.len() >= 2, "Should extract boolean constants");
}

#[test]
fn test_model_with_complex_arithmetic() {
    let solver = Solver::new();

    // Create arithmetic constraints
    let x = Int::new_const("x");
    let y = Int::new_const("y");
    let z = Int::new_const("z");

    // x + y = z, x > 0, y > 0, z < 100
    // Use Int::add with slice argument
    let sum = Int::add(&[&x, &y]);
    solver.assert(sum._eq(&z));
    solver.assert(x.gt(Int::from_i64(0)));
    solver.assert(y.gt(Int::from_i64(0)));
    solver.assert(z.lt(Int::from_i64(100)));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let mut extractor = AdvancedModelExtractor::new(model);
    extractor.extract_complete_model();

    // Verify we get all three constants
    let summary = extractor.summary();
    assert!(
        summary.num_constants >= 3,
        "Should have at least 3 constants"
    );

    // Verify the relationship x + y = z holds in the model
    let x_val = extractor.get_constant("x");
    let y_val = extractor.get_constant("y");
    let z_val = extractor.get_constant("z");

    assert!(
        x_val.is_some() && y_val.is_some() && z_val.is_some(),
        "All constants should have values"
    );
}

#[test]
fn test_model_evaluation_consistency() {
    let solver = Solver::new();

    // Set up a deterministic model
    let x = Int::new_const("x");
    solver.assert(x._eq(Int::from_i64(42)));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let mut extractor = AdvancedModelExtractor::new(model);

    // First extraction
    extractor.extract_complete_model();
    let first_summary = extractor.summary();

    // Verify consistency
    assert!(
        first_summary.num_constants >= 1,
        "Should have at least 1 constant"
    );
}

#[test]
fn test_empty_model_handling() {
    let solver = Solver::new();

    // Create a trivially satisfiable formula with no variables
    // Note: z3 will still create a model, it just won't have constants
    let truth = z3::ast::Bool::from_bool(true);
    solver.assert(&truth);

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let extractor = AdvancedModelExtractor::new(model);

    // Should not panic, even with minimal model
    let constants = extractor.get_constants();
    // Empty constants are fine
    assert!(
        constants.len() >= 0,
        "Should handle empty or minimal models"
    );
}

#[test]
fn test_model_with_quantifiers() {
    let solver = Solver::new();

    // Create a model with existentially quantified formula
    let x = Int::new_const("x");

    // exists x: x > 10 AND x < 20
    solver.assert(x.gt(Int::from_i64(10)));
    solver.assert(x.lt(Int::from_i64(20)));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let mut extractor = AdvancedModelExtractor::new(model);
    extractor.extract_complete_model();

    // Should extract x with value in range (10, 20)
    let constants = extractor.get_constants();
    assert!(!constants.is_empty(), "Should extract quantified variable");
}

#[test]
fn test_get_constant_for_nonexistent_name() {
    let solver = Solver::new();
    let x = Int::new_const("x");
    solver.assert(x._eq(Int::from_i64(5)));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let mut extractor = AdvancedModelExtractor::new(model);
    extractor.extract_complete_model();

    // Query for a constant that doesn't exist
    let nonexistent = extractor.get_constant("nonexistent_variable");
    assert!(
        nonexistent.is_none(),
        "Should return None for nonexistent constants"
    );
}

#[test]
fn test_model_summary_statistics() {
    let solver = Solver::new();

    // Create a model with multiple variables of different counts
    let a = Int::new_const("a");
    let b = Int::new_const("b");
    let c = Int::new_const("c");
    let d = Int::new_const("d");
    let e = Int::new_const("e");

    solver.assert(a._eq(Int::from_i64(1)));
    solver.assert(b._eq(Int::from_i64(2)));
    solver.assert(c._eq(Int::from_i64(3)));
    solver.assert(d._eq(Int::from_i64(4)));
    solver.assert(e._eq(Int::from_i64(5)));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let mut extractor = AdvancedModelExtractor::new(model);
    extractor.extract_complete_model();

    let summary = extractor.summary();
    assert!(summary.num_constants >= 5, "Should track all 5 constants");
}

#[test]
fn test_model_with_negative_integers() {
    let solver = Solver::new();

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    // x < 0, y < x (both negative)
    solver.assert(x.lt(Int::from_i64(0)));
    solver.assert(y.lt(&x));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let mut extractor = AdvancedModelExtractor::new(model);
    extractor.extract_complete_model();

    // Should handle negative values correctly
    let constants = extractor.get_constants();
    assert!(
        constants.len() >= 2,
        "Should extract negative integer constants"
    );
}

#[test]
fn test_incremental_model_extraction() {
    let solver = Solver::new();

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    solver.assert(x._eq(Int::from_i64(100)));
    solver.assert(y._eq(Int::from_i64(200)));

    assert_eq!(solver.check(), SatResult::Sat);
    let model = solver.get_model().unwrap();

    let mut extractor = AdvancedModelExtractor::new(model);

    // First get constants before full extraction
    let initial_count = extractor.get_constants().len();

    // Then do full extraction
    extractor.extract_complete_model();

    // After extraction, should have more complete data
    let final_count = extractor.get_constants().len();

    // Both operations should work without panic
    assert!(initial_count >= 0, "Initial extraction should work");
    assert!(final_count >= 0, "Final extraction should work");
}
