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
//! Tests for Z3 Interpolation Engine
//!
//! Verifies Craig interpolation implementation for compositional verification.
//!
// FIXED (Session 23): Tests enabled - Updated Z3 API for z3-rs 0.19+

#![allow(deprecated)] // Allow _eq until migration to eq
#![allow(unused_variables)]

use verum_smt::z3_backend::InterpolationEngine;
use z3::ast::{Bool, Int};

#[test]
fn test_basic_interpolation() {
    // Test basic interpolation: A => I, I ∧ B => false
    // A: x > 0
    // B: x < 0
    // Expected interpolant: I relates to x's sign

    let mut engine = InterpolationEngine::new();

    // Partition A: x > 0
    let x = Int::new_const("x");
    let a_formula = x.gt(Int::from_i64(0));
    engine.add_partition(&[a_formula]);

    // Partition B: x < 0
    let b_formula = x.lt(Int::from_i64(0));
    engine.add_partition(&[b_formula]);

    // Compute interpolants
    let result = engine.compute_interpolants();

    // Should return Some interpolants since A ∧ B is UNSAT
    assert!(
        result.is_some(),
        "Interpolation should succeed for UNSAT formula"
    );

    if let verum_common::Maybe::Some(interpolants) = result {
        assert_eq!(
            interpolants.len(),
            1,
            "Should have one interpolant between two partitions"
        );
        println!("Computed interpolant: {:?}", interpolants[0]);
    }
}

#[test]
fn test_interpolation_sat_returns_none() {
    // Test that SAT formulas return None (no interpolant)
    // A: x > 0
    // B: x > -10
    // A ∧ B is SAT, so no interpolant exists

    let mut engine = InterpolationEngine::new();

    let x = Int::new_const("x");

    // Partition A: x > 0
    let a_formula = x.gt(Int::from_i64(0));
    engine.add_partition(&[a_formula]);

    // Partition B: x > -10 (compatible with A)
    let b_formula = x.gt(Int::from_i64(-10));
    engine.add_partition(&[b_formula]);

    // Compute interpolants
    let result = engine.compute_interpolants();

    // Should return None since A ∧ B is SAT
    assert!(
        result.is_none(),
        "Interpolation should return None for SAT formula"
    );
}

#[test]
fn test_interpolation_three_partitions() {
    // Test interpolation with three partitions
    // A: x = 1
    // B: x = 2
    // C: x = 3
    // All three together are UNSAT

    let mut engine = InterpolationEngine::new();

    let x = Int::new_const("x");

    // Partition A: x = 1
    engine.add_partition(&[x._eq(Int::from_i64(1))]);

    // Partition B: x = 2
    engine.add_partition(&[x._eq(Int::from_i64(2))]);

    // Partition C: x = 3
    engine.add_partition(&[x._eq(Int::from_i64(3))]);

    // Compute interpolants
    let result = engine.compute_interpolants();

    // Should return Some interpolants
    if let verum_common::Maybe::Some(interpolants) = result {
        assert_eq!(
            interpolants.len(),
            2,
            "Should have two interpolants for three partitions"
        );
        println!("Interpolant 1: {:?}", interpolants[0]);
        println!("Interpolant 2: {:?}", interpolants[1]);
    } else {
        panic!("Expected interpolants for UNSAT formula");
    }
}

#[test]
fn test_interpolation_boolean_formulas() {
    // Test with pure boolean formulas
    // A: p ∧ q
    // B: ¬p ∨ ¬q
    // A ∧ B implies: (p ∧ q) ∧ (¬p ∨ ¬q) which simplifies to false

    let mut engine = InterpolationEngine::new();

    let p = Bool::new_const("p");
    let q = Bool::new_const("q");

    // Partition A: p ∧ q
    let a_formula = Bool::and(&[&p, &q]);
    engine.add_partition(&[a_formula]);

    // Partition B: ¬p ∨ ¬q
    let b_formula = Bool::or(&[&p.not(), &q.not()]);
    engine.add_partition(&[b_formula]);

    // Compute interpolants
    let result = engine.compute_interpolants();

    assert!(
        result.is_some(),
        "Should compute interpolant for boolean UNSAT"
    );

    if let verum_common::Maybe::Some(interpolants) = result {
        println!("Boolean interpolant: {:?}", interpolants[0]);
    }
}

#[test]
fn test_interpolation_arithmetic_relations() {
    // Test with arithmetic relations
    // A: x + y = 10
    // B: x + y = 20
    // Clearly UNSAT

    let mut engine = InterpolationEngine::new();

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    // Partition A: x + y = 10
    let sum = Int::add(&[&x, &y]);
    let a_formula = sum._eq(Int::from_i64(10));
    engine.add_partition(&[a_formula]);

    // Partition B: x + y = 20
    let sum2 = Int::add(&[&x, &y]);
    let b_formula = sum2._eq(Int::from_i64(20));
    engine.add_partition(&[b_formula]);

    // Compute interpolants
    let result = engine.compute_interpolants();

    assert!(
        result.is_some(),
        "Should compute interpolant for arithmetic UNSAT"
    );

    if let verum_common::Maybe::Some(interpolants) = result {
        assert_eq!(interpolants.len(), 1);
        println!("Arithmetic interpolant: {:?}", interpolants[0]);
    }
}

#[test]
fn test_interpolation_empty_partitions() {
    // Test with no partitions - should return None
    let mut engine = InterpolationEngine::new();

    let result = engine.compute_interpolants();
    assert!(result.is_none(), "Should return None for no partitions");
}

#[test]
fn test_interpolation_single_partition() {
    // Test with single partition - should return None
    let mut engine = InterpolationEngine::new();

    let x = Int::new_const("x");
    engine.add_partition(&[x._eq(Int::from_i64(5))]);

    let result = engine.compute_interpolants();
    assert!(result.is_none(), "Should return None for single partition");
}
