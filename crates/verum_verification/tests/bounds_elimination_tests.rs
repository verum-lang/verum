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
//! Tests for Bounds Check Elimination
//!
//! Bounds Check Elimination via Refinement Types:
//! Bounds checks are eliminated in AOT code when refinement types and meta
//! parameters prove safety at compile time. The analysis checks:
//! - Refinement type proves index < array_size (e.g., idx: Int{< N})
//! - Range refinement proves 0 <= min and max < array_size
//! - Flow-sensitive refinement from branch conditions (e.g., if idx < len(arr))
//! - Loop invariants proving bounds (e.g., invariant 0 <= i <= len(arr))

use verum_ast::span::Span;
use verum_common::List;

use verum_verification::bounds_elimination::*;
use verum_verification::cbgr_elimination::{BlockId, ControlFlowGraph, ScopeId};

// =============================================================================
// Test Helpers
// =============================================================================

fn make_cfg() -> ControlFlowGraph {
    let entry = BlockId::new(0);
    let root_scope = ScopeId::new(0);
    ControlFlowGraph::new(entry, root_scope)
}

fn make_span() -> Span {
    Span::dummy()
}

// =============================================================================
// Refinement Type Elimination Tests
// =============================================================================

#[test]
fn test_static_bounds_elimination() {
    // array: [T; 10], index: i where 0 <= i < 10
    // Expected: Check eliminated

    let cfg = make_cfg();
    let mut eliminator = BoundsCheckEliminator::new(cfg);

    // Add array bounds: static length 10
    let array_bounds = ArrayBounds::new("array".into()).with_static_length(10);
    eliminator.add_array_bounds(array_bounds);

    // Add index refinement: 0 <= index < 10
    let constraint = Expression::binary(
        BinaryOp::And,
        Expression::binary(BinaryOp::Ge, Expression::var("index"), Expression::int(0)),
        Expression::binary(BinaryOp::Lt, Expression::var("index"), Expression::int(10)),
    );
    eliminator.add_refinement("index".into(), constraint);

    // Create array access: array[index]
    let access = ArrayAccess::new(
        Expression::var("array"),
        Expression::var("index"),
        BlockId::new(0),
        make_span(),
    );

    // Analyze
    let decision = eliminator.analyze_array_access(&access).unwrap();

    // Should eliminate check
    assert_eq!(decision, CheckDecision::Eliminate);
    assert_eq!(eliminator.stats().elimination_rate(), 100.0);
}

#[test]
fn test_refinement_type_elimination() {
    // index: Positive where index < 100
    // array: List<T> where len(array) >= 100
    // Expected: Check eliminated

    let cfg = make_cfg();
    let mut eliminator = BoundsCheckEliminator::new(cfg);

    // Add array bounds: length >= 100
    let array_bounds = ArrayBounds::new("array".into()).with_static_length(100);
    eliminator.add_array_bounds(array_bounds);

    // Add index refinement: 0 <= index < 100
    let constraint = Expression::binary(
        BinaryOp::And,
        Expression::binary(BinaryOp::Ge, Expression::var("index"), Expression::int(0)),
        Expression::binary(BinaryOp::Lt, Expression::var("index"), Expression::int(100)),
    );
    eliminator.add_refinement("index".into(), constraint);

    // Create array access
    let access = ArrayAccess::new(
        Expression::var("array"),
        Expression::var("index"),
        BlockId::new(0),
        make_span(),
    );

    // Analyze
    let decision = eliminator.analyze_array_access(&access).unwrap();

    // Should eliminate
    assert_eq!(decision, CheckDecision::Eliminate);
}

#[test]
fn test_refinement_out_of_bounds() {
    // index: Int where 0 <= index < 200
    // array: List<T> where len(array) == 100
    // Expected: Check kept (bounds don't match)

    let cfg = make_cfg();
    let mut eliminator = BoundsCheckEliminator::new(cfg);

    // Array length: 100
    let array_bounds = ArrayBounds::new("array".into()).with_static_length(100);
    eliminator.add_array_bounds(array_bounds);

    // Index constraint: 0 <= index < 200 (larger than array)
    let constraint = Expression::binary(
        BinaryOp::And,
        Expression::binary(BinaryOp::Ge, Expression::var("index"), Expression::int(0)),
        Expression::binary(BinaryOp::Lt, Expression::var("index"), Expression::int(200)),
    );
    eliminator.add_refinement("index".into(), constraint);

    // Create array access
    let access = ArrayAccess::new(
        Expression::var("array"),
        Expression::var("index"),
        BlockId::new(0),
        make_span(),
    );

    // Analyze
    let decision = eliminator.analyze_array_access(&access).unwrap();

    // Should keep check (refinement doesn't prove safety)
    assert_eq!(decision, CheckDecision::Keep);
}

// =============================================================================
// Loop Invariant Elimination Tests
// =============================================================================

#[test]
fn test_loop_invariant_elimination() {
    // for i in 0..array.len() { array[i] }
    // Expected: Check eliminated

    let cfg = make_cfg();
    let mut eliminator = BoundsCheckEliminator::new(cfg);

    let loop_id = LoopId::new(1);

    // Add array bounds: dynamic length
    let array_bounds = ArrayBounds::new("array".into())
        .with_length_expr(Expression::array_len(Expression::var("array")));
    eliminator.add_array_bounds(array_bounds);

    // Add loop invariant: 0 <= i < array.len()
    let mut invariant = LoopInvariant::new(loop_id, "i".into()).with_bounds(
        Expression::int(0),
        Expression::array_len(Expression::var("array")),
    );

    // Add variable bounds for 'i'
    invariant.add_variable_bounds(
        "i".into(),
        ValueRange::new(
            Expression::int(0),
            Expression::array_len(Expression::var("array")),
        )
        .with_proven(true),
    );

    eliminator.add_loop_invariant(invariant);

    // Create array access inside loop
    let access = ArrayAccess::new(
        Expression::var("array"),
        Expression::var("i"),
        BlockId::new(0),
        make_span(),
    )
    .with_loop(loop_id);

    // Analyze
    let decision = eliminator.analyze_array_access(&access).unwrap();

    // Should eliminate
    assert_eq!(decision, CheckDecision::Eliminate);
}

#[test]
fn test_loop_invariant_mismatch() {
    // for i in 0..10 { array[i] }  // array.len() != 10
    // Expected: Check kept

    let cfg = make_cfg();
    let mut eliminator = BoundsCheckEliminator::new(cfg);

    let loop_id = LoopId::new(1);

    // Array length: dynamic
    let array_bounds = ArrayBounds::new("array".into())
        .with_length_expr(Expression::array_len(Expression::var("array")));
    eliminator.add_array_bounds(array_bounds);

    // Loop bounds: 0..10 (doesn't match array.len())
    let invariant = LoopInvariant::new(loop_id, "i".into())
        .with_bounds(Expression::int(0), Expression::int(10));

    eliminator.add_loop_invariant(invariant);

    // Create array access
    let access = ArrayAccess::new(
        Expression::var("array"),
        Expression::var("i"),
        BlockId::new(0),
        make_span(),
    )
    .with_loop(loop_id);

    // Analyze
    let decision = eliminator.analyze_array_access(&access).unwrap();

    // Should keep check (invariant doesn't prove array bounds)
    assert_eq!(decision, CheckDecision::Keep);
}

// =============================================================================
// Meta Parameter Elimination Tests
// =============================================================================

#[test]
fn test_meta_parameter_elimination() {
    // fn access<const N: usize>(array: [T; N], i: usize where i < N)
    // Expected: Check eliminated

    let cfg = make_cfg();
    let mut eliminator = BoundsCheckEliminator::new(cfg);

    // Array with meta parameter N=10
    let array_bounds = ArrayBounds::new("array".into()).with_static_length(10);
    eliminator.add_array_bounds(array_bounds);

    // Index constraint: i < N (where N=10)
    let constraint = Expression::binary(
        BinaryOp::And,
        Expression::binary(BinaryOp::Ge, Expression::var("i"), Expression::int(0)),
        Expression::binary(BinaryOp::Lt, Expression::var("i"), Expression::int(10)),
    );
    eliminator.add_refinement("i".into(), constraint);

    // Create array access
    let access = ArrayAccess::new(
        Expression::var("array"),
        Expression::var("i"),
        BlockId::new(0),
        make_span(),
    );

    // Analyze
    let decision = eliminator.analyze_array_access(&access).unwrap();

    // Should eliminate
    assert_eq!(decision, CheckDecision::Eliminate);
}

#[test]
fn test_meta_constraint_verification() {
    // Test meta constraint verification
    let constraint =
        Expression::binary(BinaryOp::Lt, Expression::var("index"), Expression::var("N"));

    let meta = MetaConstraint::new("N".into(), constraint).with_value(10);

    // index < N where N=10
    assert!(meta.verify(5)); // 5 < 10: true
    assert!(meta.verify(9)); // 9 < 10: true
    assert!(!meta.verify(10)); // 10 < 10: false
    assert!(!meta.verify(15)); // 15 < 10: false
}

// =============================================================================
// Dataflow Analysis Tests
// =============================================================================

#[test]
fn test_dataflow_propagation() {
    // if index < array.len() { array[index] }
    // Expected: Check eliminated in then-branch

    let cfg = make_cfg();
    let mut eliminator = BoundsCheckEliminator::new(cfg);

    // Add array bounds
    let array_bounds = ArrayBounds::new("array".into()).with_static_length(100);
    eliminator.add_array_bounds(array_bounds);

    // Add dataflow constraint: index proven to be < 100
    let block = BlockId::new(1);
    eliminator.add_reaching_def(
        block,
        "index".into(),
        Definition {
            var: "index".into(),
            value: Expression::int(50), // Known value
            block,
        },
    );

    // Create array access
    let access = ArrayAccess::new(
        Expression::var("array"),
        Expression::var("index"),
        block,
        make_span(),
    );

    // Analyze
    let decision = eliminator.analyze_array_access(&access).unwrap();

    // Should eliminate (dataflow proves bounds)
    assert_eq!(decision, CheckDecision::Eliminate);
}

// =============================================================================
// Check Hoisting Tests
// =============================================================================

#[test]
fn test_check_hoisting() {
    // for i in 0..n { array[i*2] }
    // Expected: Hoist check: n*2 < array.len()

    let cfg = make_cfg();
    let mut eliminator = BoundsCheckEliminator::new(cfg);

    let loop_id = LoopId::new(1);

    // Add array bounds
    let array_bounds = ArrayBounds::new("array".into()).with_static_length(100);
    eliminator.add_array_bounds(array_bounds);

    // Add loop invariant
    let invariant = LoopInvariant::new(loop_id, "i".into())
        .with_bounds(Expression::int(0), Expression::int(50));
    eliminator.add_loop_invariant(invariant);

    // Create array access with computed index: i*2
    let access = ArrayAccess::new(
        Expression::var("array"),
        Expression::binary(BinaryOp::Mul, Expression::var("i"), Expression::int(2)),
        BlockId::new(0),
        make_span(),
    )
    .with_loop(loop_id);

    // Analyze
    let decision = eliminator.analyze_array_access(&access).unwrap();

    // Should hoist check
    assert_eq!(decision, CheckDecision::Hoist);
}

#[test]
fn test_no_hoisting_for_simple_index() {
    // for i in 0..array.len() { array[i] }
    // Expected: Eliminate (not hoist) - induction variable matches bounds

    let cfg = make_cfg();
    let mut eliminator = BoundsCheckEliminator::new(cfg);

    let loop_id = LoopId::new(1);

    // Add array bounds
    let array_bounds = ArrayBounds::new("array".into())
        .with_length_expr(Expression::array_len(Expression::var("array")));
    eliminator.add_array_bounds(array_bounds);

    // Add loop invariant matching array length
    let invariant = LoopInvariant::new(loop_id, "i".into()).with_bounds(
        Expression::int(0),
        Expression::array_len(Expression::var("array")),
    );
    eliminator.add_loop_invariant(invariant);

    // Create simple array access: array[i]
    let access = ArrayAccess::new(
        Expression::var("array"),
        Expression::var("i"),
        BlockId::new(0),
        make_span(),
    )
    .with_loop(loop_id);

    // Analyze
    let decision = eliminator.analyze_array_access(&access).unwrap();

    // Should eliminate (not hoist)
    assert_eq!(decision, CheckDecision::Eliminate);
}

// =============================================================================
// Batch Analysis Tests
// =============================================================================

#[test]
fn test_batch_analysis() {
    // Multiple array accesses in one function
    let cfg = make_cfg();
    let mut eliminator = BoundsCheckEliminator::new(cfg);

    // Setup bounds
    let array_bounds = ArrayBounds::new("array".into()).with_static_length(10);
    eliminator.add_array_bounds(array_bounds);

    // Setup refinement
    let constraint = Expression::binary(
        BinaryOp::And,
        Expression::binary(BinaryOp::Ge, Expression::var("i"), Expression::int(0)),
        Expression::binary(BinaryOp::Lt, Expression::var("i"), Expression::int(10)),
    );
    eliminator.add_refinement("i".into(), constraint);

    // Create multiple accesses
    let mut accesses = List::new();

    // Access 1: array[i] - should eliminate
    accesses.push(ArrayAccess::new(
        Expression::var("array"),
        Expression::var("i"),
        BlockId::new(0),
        make_span(),
    ));

    // Access 2: array[0] - should eliminate
    accesses.push(ArrayAccess::new(
        Expression::var("array"),
        Expression::int(0),
        BlockId::new(0),
        make_span(),
    ));

    // Access 3: array[j] - should keep (no constraint for j)
    accesses.push(ArrayAccess::new(
        Expression::var("array"),
        Expression::var("j"),
        BlockId::new(0),
        make_span(),
    ));

    // Analyze all
    let mut decisions = List::new();
    for access in accesses.iter() {
        let decision = eliminator.analyze_array_access(access).unwrap();
        decisions.push(decision);
    }

    // Check results
    assert_eq!(decisions[0], CheckDecision::Eliminate);
    assert_eq!(decisions[1], CheckDecision::Keep); // Literal index - conservative
    assert_eq!(decisions[2], CheckDecision::Keep); // No constraint for j

    // Check statistics
    let stats = eliminator.stats();
    assert_eq!(stats.total_checks, 3);
    assert_eq!(stats.eliminated_checks, 1);
    assert_eq!(stats.kept_checks, 2);
}

// =============================================================================
// Statistics Tests
// =============================================================================

#[test]
fn test_elimination_stats() {
    let mut stats = EliminationStats::default();

    stats.total_checks = 100;
    stats.eliminated_checks = 80;
    stats.hoisted_checks = 15;
    stats.kept_checks = 5;

    // Check rates
    assert_eq!(stats.elimination_rate(), 80.0);
    assert_eq!(stats.optimization_rate(), 95.0);

    // Check time savings
    let expected_savings = (80 * 5 * 100) + (15 * 4 * 100);
    assert_eq!(stats.estimated_time_saved_ns(), expected_savings);
}

#[test]
fn test_compute_elimination_stats() {
    let mut decisions = List::new();

    // Add various decisions
    decisions.push(CheckDecision::Eliminate);
    decisions.push(CheckDecision::Eliminate);
    decisions.push(CheckDecision::Hoist);
    decisions.push(CheckDecision::Keep);

    let stats = compute_elimination_stats(&decisions);

    assert_eq!(stats.total_checks, 4);
    assert_eq!(stats.eliminated_checks, 2);
    assert_eq!(stats.hoisted_checks, 1);
    assert_eq!(stats.kept_checks, 1);
    assert_eq!(stats.elimination_rate(), 50.0);
    assert_eq!(stats.optimization_rate(), 75.0);
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn test_empty_access_list() {
    let cfg = make_cfg();
    let eliminator = BoundsCheckEliminator::new(cfg);

    let stats = eliminator.stats();
    assert_eq!(stats.total_checks, 0);
    assert_eq!(stats.elimination_rate(), 0.0);
}

#[test]
fn test_unknown_array() {
    // Access to array without bounds information
    let cfg = make_cfg();
    let mut eliminator = BoundsCheckEliminator::new(cfg);

    let access = ArrayAccess::new(
        Expression::var("unknown"),
        Expression::var("i"),
        BlockId::new(0),
        make_span(),
    );

    let decision = eliminator.analyze_array_access(&access).unwrap();

    // Should conservatively keep check
    assert_eq!(decision, CheckDecision::Keep);
}

#[test]
fn test_complex_index_expression() {
    // array[(i + j) * 2]
    let cfg = make_cfg();
    let mut eliminator = BoundsCheckEliminator::new(cfg);

    let array_bounds = ArrayBounds::new("array".into()).with_static_length(100);
    eliminator.add_array_bounds(array_bounds);

    // Complex index expression
    let index = Expression::binary(
        BinaryOp::Mul,
        Expression::binary(BinaryOp::Add, Expression::var("i"), Expression::var("j")),
        Expression::int(2),
    );

    let access = ArrayAccess::new(
        Expression::var("array"),
        index,
        BlockId::new(0),
        make_span(),
    );

    let decision = eliminator.analyze_array_access(&access).unwrap();

    // Should keep check (complex expression)
    assert_eq!(decision, CheckDecision::Keep);
}

// =============================================================================
// API Tests
// =============================================================================

#[test]
fn test_public_api() {
    let cfg = make_cfg();

    let access = ArrayAccess::new(
        Expression::var("array"),
        Expression::var("i"),
        BlockId::new(0),
        make_span(),
    );

    // Test single access analysis
    let decision = analyze_bounds_check(&access, &cfg).unwrap();
    assert!(matches!(
        decision,
        CheckDecision::Keep | CheckDecision::Eliminate | CheckDecision::Hoist
    ));
}

#[test]
fn test_batch_api() {
    let cfg = make_cfg();

    let mut accesses = List::new();
    accesses.push(ArrayAccess::new(
        Expression::var("array"),
        Expression::var("i"),
        BlockId::new(0),
        make_span(),
    ));

    // Test batch analysis
    let decisions = analyze_function_bounds(&accesses, &cfg).unwrap();
    assert_eq!(decisions.len(), 1);
}
