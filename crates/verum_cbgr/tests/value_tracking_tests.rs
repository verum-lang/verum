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
//! Comprehensive tests for value tracking in CBGR escape analysis
//!
//! Tests cover all aspects of concrete value analysis, range tracking,
//! and symbolic execution for more precise escape analysis.

use verum_cbgr::analysis::{BlockId, RefId};
use verum_cbgr::value_tracking::{
    BinaryOp as ValueBinaryOp, ConcreteValue, PathPredicate, PropagationStats, SymbolicValue,
    ValuePropagator, ValueRange, ValueState, ValueTrackingConfig, ValueTrackingResult,
};
use verum_common::{List, Map, Maybe, Set};

// ==================================================================================
// Test 1-5: ConcreteValue Tests
// ==================================================================================

#[test]
fn test_concrete_value_integer() {
    let val = ConcreteValue::Integer(42);
    assert!(val.is_known());
    assert!(val.is_constant());
    assert_eq!(format!("{}", val), "42");
}

#[test]
fn test_concrete_value_boolean() {
    let val = ConcreteValue::Boolean(true);
    assert!(val.is_known());
    assert!(val.is_constant());
    assert_eq!(format!("{}", val), "true");
}

#[test]
fn test_concrete_value_merge_same() {
    let v1 = ConcreteValue::Integer(42);
    let v2 = ConcreteValue::Integer(42);
    assert_eq!(v1.merge(&v2), ConcreteValue::Integer(42));
}

#[test]
fn test_concrete_value_merge_different() {
    let v1 = ConcreteValue::Integer(42);
    let v2 = ConcreteValue::Integer(10);
    assert_eq!(v1.merge(&v2), ConcreteValue::Top);
}

#[test]
fn test_concrete_value_eval_binop() {
    let v1 = ConcreteValue::Integer(10);
    let v2 = ConcreteValue::Integer(5);

    assert_eq!(
        v1.eval_binop(ValueBinaryOp::Add, &v2),
        ConcreteValue::Integer(15)
    );
    assert_eq!(
        v1.eval_binop(ValueBinaryOp::Sub, &v2),
        ConcreteValue::Integer(5)
    );
    assert_eq!(
        v1.eval_binop(ValueBinaryOp::Mul, &v2),
        ConcreteValue::Integer(50)
    );
    assert_eq!(
        v1.eval_binop(ValueBinaryOp::Div, &v2),
        ConcreteValue::Integer(2)
    );
    assert_eq!(
        v1.eval_binop(ValueBinaryOp::Lt, &v2),
        ConcreteValue::Boolean(false)
    );
    assert_eq!(
        v1.eval_binop(ValueBinaryOp::Gt, &v2),
        ConcreteValue::Boolean(true)
    );
}

// ==================================================================================
// Test 6-10: ValueRange Tests
// ==================================================================================

#[test]
fn test_value_range_constant() {
    let range = ValueRange::from_constant(42);
    assert_eq!(range.min, 42);
    assert_eq!(range.max, 42);
    assert!(range.definite);
    assert!(range.contains(42));
    assert!(!range.contains(41));
}

#[test]
fn test_value_range_bounds() {
    let range = ValueRange::from_bounds(0, 10);
    assert_eq!(range.min, 0);
    assert_eq!(range.max, 10);
    assert!(!range.definite);
    assert!(range.contains(5));
    assert!(!range.contains(15));
}

#[test]
fn test_value_range_intersect() {
    let r1 = ValueRange::from_bounds(0, 10);
    let r2 = ValueRange::from_bounds(5, 15);
    let r3 = r1.intersect(&r2);

    assert_eq!(r3.min, 5);
    assert_eq!(r3.max, 10);
    assert!(!r3.definite);
}

#[test]
fn test_value_range_union() {
    let r1 = ValueRange::from_bounds(0, 10);
    let r2 = ValueRange::from_bounds(5, 15);
    let r3 = r1.union(&r2);

    assert_eq!(r3.min, 0);
    assert_eq!(r3.max, 15);
    assert!(!r3.definite);
}

#[test]
fn test_value_range_empty() {
    let r1 = ValueRange::from_bounds(10, 5);
    assert!(r1.is_empty());

    let r2 = ValueRange::from_bounds(5, 10);
    assert!(!r2.is_empty());
}

// ==================================================================================
// Test 11-15: SymbolicValue Tests
// ==================================================================================

#[test]
fn test_symbolic_value_concrete() {
    let sym = SymbolicValue::from_concrete(ConcreteValue::Integer(42));
    let env = Map::new();
    assert_eq!(sym.evaluate(&env), ConcreteValue::Integer(42));
}

#[test]
fn test_symbolic_value_variable() {
    let sym = SymbolicValue::variable(0);
    let mut env = Map::new();
    env.insert(0, ConcreteValue::Integer(10));

    assert_eq!(sym.evaluate(&env), ConcreteValue::Integer(10));
}

#[test]
fn test_symbolic_value_binop() {
    let sym = SymbolicValue::binop(
        ValueBinaryOp::Add,
        SymbolicValue::variable(0),
        SymbolicValue::from_concrete(ConcreteValue::Integer(10)),
    );

    let mut env = Map::new();
    env.insert(0, ConcreteValue::Integer(32));

    assert_eq!(sym.evaluate(&env), ConcreteValue::Integer(42));
}

#[test]
fn test_symbolic_value_is_definitely_true() {
    let sym = SymbolicValue::from_concrete(ConcreteValue::Boolean(true));
    let env = Map::new();
    assert!(sym.is_definitely_true(&env));
    assert!(!sym.is_definitely_false(&env));
}

#[test]
fn test_symbolic_value_is_definitely_false() {
    let sym = SymbolicValue::from_concrete(ConcreteValue::Boolean(false));
    let env = Map::new();
    assert!(!sym.is_definitely_true(&env));
    assert!(sym.is_definitely_false(&env));
}

// ==================================================================================
// Test 16-20: ValueState Tests
// ==================================================================================

#[test]
fn test_value_state_concrete() {
    let mut state = ValueState::new();
    state.set_concrete(0, ConcreteValue::Integer(42));

    assert_eq!(
        state.get_concrete(0),
        Maybe::Some(ConcreteValue::Integer(42))
    );
    assert_eq!(state.get_concrete(1), Maybe::None);
}

#[test]
fn test_value_state_range() {
    let mut state = ValueState::new();
    let range = ValueRange::from_bounds(0, 10);
    state.set_range(0, range.clone());

    if let Maybe::Some(r) = state.get_range(0) {
        assert_eq!(r.min, 0);
        assert_eq!(r.max, 10);
    } else {
        panic!("Expected range");
    }
}

#[test]
fn test_value_state_symbolic() {
    let mut state = ValueState::new();
    let sym = SymbolicValue::variable(0);
    state.set_symbolic(1, sym.clone());

    assert!(matches!(state.get_symbolic(1), Maybe::Some(_)));
}

#[test]
fn test_value_state_merge() {
    let mut state1 = ValueState::new();
    state1.set_concrete(0, ConcreteValue::Integer(42));

    let mut state2 = ValueState::new();
    state2.set_concrete(0, ConcreteValue::Integer(42));

    let merged = state1.merge(&state2);
    assert_eq!(
        merged.get_concrete(0),
        Maybe::Some(ConcreteValue::Integer(42))
    );
}

#[test]
fn test_value_state_refine_with_condition() {
    let mut state = ValueState::new();
    state.set_range(0, ValueRange::from_bounds(0, 100));

    // Refine with x < 10
    let predicate = SymbolicValue::binop(
        ValueBinaryOp::Lt,
        SymbolicValue::variable(0),
        SymbolicValue::from_concrete(ConcreteValue::Integer(10)),
    );

    let refined = state.refine_with_condition(&predicate, true);
    if let Maybe::Some(r) = refined.get_range(0) {
        assert_eq!(r.max, 9);
    } else {
        panic!("Expected refined range");
    }
}

// ==================================================================================
// Test 21-25: ValuePropagator Tests
// ==================================================================================

#[test]
fn test_value_propagator_constant() {
    let mut propagator = ValuePropagator::new();
    let mut state = ValueState::new();

    propagator.propagate_constant(&mut state, 0, ConcreteValue::Integer(42));
    assert_eq!(
        state.get_concrete(0),
        Maybe::Some(ConcreteValue::Integer(42))
    );
}

#[test]
fn test_value_propagator_binop() {
    let mut propagator = ValuePropagator::new();
    let mut state = ValueState::new();

    state.set_concrete(1, ConcreteValue::Integer(10));
    state.set_concrete(2, ConcreteValue::Integer(5));

    propagator.propagate_binop(&mut state, 0, ValueBinaryOp::Add, 1, 2);
    assert_eq!(
        state.get_concrete(0),
        Maybe::Some(ConcreteValue::Integer(15))
    );
}

#[test]
fn test_value_propagator_phi() {
    let mut propagator = ValuePropagator::new();
    let mut state = ValueState::new();

    let incoming: List<(BlockId, u32)> = vec![(BlockId(0), 1u32), (BlockId(1), 2u32)].into();

    propagator.propagate_phi(&mut state, 0, BlockId(2), &incoming);
    assert!(matches!(state.get_symbolic(0), Maybe::Some(_)));
}

#[test]
fn test_value_propagator_stats() {
    let propagator = ValuePropagator::new();
    let stats = propagator.stats();

    assert_eq!(stats.concrete_propagated, 0);
    assert_eq!(stats.ranges_refined, 0);
    assert_eq!(stats.symbolic_created, 0);
}

#[test]
fn test_value_propagator_merge_predecessors() {
    let propagator = ValuePropagator::new();
    let mut predecessors = Set::new();
    predecessors.insert(BlockId(0));
    predecessors.insert(BlockId(1));

    let merged = propagator.merge_predecessor_states(&predecessors);
    // Should create empty state since no exit states are set
    assert_eq!(merged.get_concrete(0), Maybe::None);
}

// ==================================================================================
// Test 26-30: PathPredicate Tests
// ==================================================================================

#[test]
fn test_path_predicate_evaluate_true() {
    let condition = SymbolicValue::from_concrete(ConcreteValue::Boolean(true));
    let predicate = PathPredicate::new(condition, true, BlockId(0));

    let state = ValueState::new();
    assert_eq!(predicate.evaluate(&state), Maybe::Some(true));
}

#[test]
fn test_path_predicate_evaluate_false() {
    let condition = SymbolicValue::from_concrete(ConcreteValue::Boolean(false));
    let predicate = PathPredicate::new(condition, true, BlockId(0));

    let state = ValueState::new();
    assert_eq!(predicate.evaluate(&state), Maybe::Some(false));
}

#[test]
fn test_path_predicate_is_satisfiable() {
    let condition = SymbolicValue::from_concrete(ConcreteValue::Boolean(true));
    let predicate = PathPredicate::new(condition, true, BlockId(0));

    let state = ValueState::new();
    assert!(predicate.is_satisfiable(&state));
}

#[test]
fn test_path_predicate_refine_state() {
    let condition = SymbolicValue::binop(
        ValueBinaryOp::Lt,
        SymbolicValue::variable(0),
        SymbolicValue::from_concrete(ConcreteValue::Integer(100)),
    );
    let predicate = PathPredicate::new(condition, true, BlockId(0));

    let mut state = ValueState::new();
    state.set_range(0, ValueRange::unbounded());

    let refined = predicate.refine_state(&state);
    if let Maybe::Some(r) = refined.get_range(0) {
        assert_eq!(r.max, 99);
    } else {
        panic!("Expected refined range");
    }
}

#[test]
fn test_path_predicate_negation() {
    let condition = SymbolicValue::from_concrete(ConcreteValue::Boolean(true));
    let predicate = PathPredicate::new(condition, false, BlockId(0));

    let state = ValueState::new();
    assert_eq!(predicate.evaluate(&state), Maybe::Some(false));
}

// ==================================================================================
// Test 31-35: ValueTrackingConfig Tests
// ==================================================================================

#[test]
fn test_value_tracking_config_default() {
    let config = ValueTrackingConfig::default();
    assert!(config.enable_constant_propagation);
    assert!(config.enable_range_analysis);
    assert!(config.enable_symbolic_execution);
    assert_eq!(config.max_iterations, 100);
}

#[test]
fn test_value_tracking_config_custom() {
    let config = ValueTrackingConfig {
        enable_constant_propagation: true,
        enable_range_analysis: false,
        enable_symbolic_execution: true,
        max_iterations: 50,
    };

    assert!(config.enable_constant_propagation);
    assert!(!config.enable_range_analysis);
    assert!(config.enable_symbolic_execution);
    assert_eq!(config.max_iterations, 50);
}

#[test]
fn test_value_tracking_result_default() {
    let result = ValueTrackingResult::default();
    assert!(result.block_states.is_empty());
    assert!(result.infeasible_paths.is_empty());
}

#[test]
fn test_value_tracking_result_get_state() {
    let mut result = ValueTrackingResult::new();
    let state = ValueState::new();
    result.block_states.insert(BlockId(0), state);

    assert!(result.get_state(BlockId(0)).is_some());
    assert!(result.get_state(BlockId(1)).is_none());
}

#[test]
fn test_value_tracking_result_is_path_feasible() {
    let result = ValueTrackingResult::new();
    let path: List<BlockId> = vec![BlockId(0), BlockId(1), BlockId(2)].into();

    assert!(result.is_path_feasible(&path));
}

// ==================================================================================
// Test 36-40: Integration Tests
// ==================================================================================

#[test]
fn test_constant_propagation_chain() {
    let mut state = ValueState::new();
    let mut propagator = ValuePropagator::new();

    // x = 10
    propagator.propagate_constant(&mut state, 0, ConcreteValue::Integer(10));
    // y = 5
    propagator.propagate_constant(&mut state, 1, ConcreteValue::Integer(5));
    // z = x + y
    propagator.propagate_binop(&mut state, 2, ValueBinaryOp::Add, 0, 1);

    assert_eq!(
        state.get_concrete(2),
        Maybe::Some(ConcreteValue::Integer(15))
    );
}

#[test]
fn test_range_propagation_chain() {
    let mut state = ValueState::new();
    let mut propagator = ValuePropagator::new();

    // x ∈ [0, 10]
    state.set_range(0, ValueRange::from_bounds(0, 10));
    // y ∈ [5, 15]
    state.set_range(1, ValueRange::from_bounds(5, 15));
    // z = x + y
    propagator.propagate_binop(&mut state, 2, ValueBinaryOp::Add, 0, 1);

    if let Maybe::Some(r) = state.get_range(2) {
        assert_eq!(r.min, 5);
        assert_eq!(r.max, 25);
    } else {
        panic!("Expected range");
    }
}

#[test]
fn test_condition_refinement_less_than() {
    let mut state = ValueState::new();
    state.set_range(0, ValueRange::from_bounds(0, 100));

    let predicate = SymbolicValue::binop(
        ValueBinaryOp::Lt,
        SymbolicValue::variable(0),
        SymbolicValue::from_concrete(ConcreteValue::Integer(50)),
    );

    let refined = state.refine_with_condition(&predicate, true);
    if let Maybe::Some(r) = refined.get_range(0) {
        assert_eq!(r.max, 49);
    } else {
        panic!("Expected refined range");
    }
}

#[test]
fn test_condition_refinement_greater_equal() {
    let mut state = ValueState::new();
    state.set_range(0, ValueRange::from_bounds(0, 100));

    let predicate = SymbolicValue::binop(
        ValueBinaryOp::Ge,
        SymbolicValue::variable(0),
        SymbolicValue::from_concrete(ConcreteValue::Integer(50)),
    );

    let refined = state.refine_with_condition(&predicate, true);
    if let Maybe::Some(r) = refined.get_range(0) {
        assert_eq!(r.min, 50);
    } else {
        panic!("Expected refined range");
    }
}

#[test]
fn test_complex_symbolic_expression() {
    // (x * 2) + 10
    let x = SymbolicValue::variable(0);
    let two = SymbolicValue::from_concrete(ConcreteValue::Integer(2));
    let ten = SymbolicValue::from_concrete(ConcreteValue::Integer(10));

    let mul = SymbolicValue::binop(ValueBinaryOp::Mul, x, two);
    let result = SymbolicValue::binop(ValueBinaryOp::Add, mul, ten);

    let mut env = Map::new();
    env.insert(0, ConcreteValue::Integer(5));

    assert_eq!(result.evaluate(&env), ConcreteValue::Integer(20));
}

// ==================================================================================
// Test 41-45: Edge Cases and Error Handling
// ==================================================================================

#[test]
fn test_division_by_zero() {
    let v1 = ConcreteValue::Integer(10);
    let v2 = ConcreteValue::Integer(0);

    let result = v1.eval_binop(ValueBinaryOp::Div, &v2);
    assert_eq!(result, ConcreteValue::Top);
}

#[test]
fn test_integer_overflow() {
    let v1 = ConcreteValue::Integer(i64::MAX);
    let v2 = ConcreteValue::Integer(1);

    let result = v1.eval_binop(ValueBinaryOp::Add, &v2);
    assert_eq!(result, ConcreteValue::Top);
}

#[test]
fn test_unknown_value_merge() {
    let v1 = ConcreteValue::Unknown;
    let v2 = ConcreteValue::Integer(42);

    assert_eq!(v1.merge(&v2), ConcreteValue::Integer(42));
}

#[test]
fn test_top_value_merge() {
    let v1 = ConcreteValue::Top;
    let v2 = ConcreteValue::Integer(42);

    assert_eq!(v1.merge(&v2), ConcreteValue::Top);
}

#[test]
fn test_empty_range_intersect() {
    let r1 = ValueRange::from_bounds(0, 10);
    let r2 = ValueRange::from_bounds(20, 30);
    let r3 = r1.intersect(&r2);

    assert!(r3.is_empty());
}

// ==================================================================================
// Test 46-50: Performance-oriented Tests
// ==================================================================================

#[test]
fn test_large_range_union() {
    let r1 = ValueRange::from_bounds(i64::MIN, 0);
    let r2 = ValueRange::from_bounds(0, i64::MAX);
    let r3 = r1.union(&r2);

    assert_eq!(r3.min, i64::MIN);
    assert_eq!(r3.max, i64::MAX);
}

#[test]
fn test_many_concrete_values() {
    let mut state = ValueState::new();

    for i in 0..1000 {
        state.set_concrete(i, ConcreteValue::Integer(i as i64));
    }

    for i in 0..1000 {
        assert_eq!(
            state.get_concrete(i),
            Maybe::Some(ConcreteValue::Integer(i as i64))
        );
    }
}

#[test]
fn test_many_ranges() {
    let mut state = ValueState::new();

    for i in 0..1000 {
        state.set_range(i, ValueRange::from_bounds(i as i64, i as i64 + 10));
    }

    assert_eq!(state.get_range(500).unwrap().min, 500);
}

#[test]
fn test_propagation_stats_accumulation() {
    let mut propagator = ValuePropagator::new();
    let mut state = ValueState::new();

    for i in 0..10 {
        propagator.propagate_constant(&mut state, i, ConcreteValue::Integer(i as i64));
    }

    assert_eq!(propagator.stats().concrete_propagated, 10);
}

#[test]
fn test_value_state_merge_performance() {
    let mut state1 = ValueState::new();
    let mut state2 = ValueState::new();

    for i in 0..100 {
        state1.set_concrete(i, ConcreteValue::Integer(i as i64));
        state2.set_concrete(i, ConcreteValue::Integer(i as i64));
    }

    let merged = state1.merge(&state2);
    for i in 0..100 {
        assert_eq!(
            merged.get_concrete(i),
            Maybe::Some(ConcreteValue::Integer(i as i64))
        );
    }
}
