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
//! Comprehensive test suite for Z3 feasibility checking
//!
//! This test suite validates the Z3 SMT solver integration for path
//! feasibility checking in CBGR escape analysis.

use verum_cbgr::analysis::{BlockId, PathCondition, PathPredicate};
use verum_cbgr::z3_feasibility::{
    FeasibilityResult, Z3FeasibilityChecker, Z3FeasibilityCheckerBuilder,
};

// ==================== Basic Satisfiability Tests ====================

#[test]
fn test_true_is_satisfiable() {
    let mut checker = Z3FeasibilityChecker::new();
    let result = checker.check_feasible(&PathPredicate::True);
    assert_eq!(result, FeasibilityResult::Satisfiable);
    assert!(result.is_feasible());
    assert!(!result.is_infeasible());
}

#[test]
fn test_false_is_unsatisfiable() {
    let mut checker = Z3FeasibilityChecker::new();
    let result = checker.check_feasible(&PathPredicate::False);
    assert_eq!(result, FeasibilityResult::Unsatisfiable);
    assert!(!result.is_feasible());
    assert!(result.is_infeasible());
}

#[test]
fn test_simple_block_true() {
    let mut checker = Z3FeasibilityChecker::new();
    let pred = PathPredicate::BlockTrue(BlockId(42));
    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

#[test]
fn test_simple_block_false() {
    let mut checker = Z3FeasibilityChecker::new();
    let pred = PathPredicate::BlockFalse(BlockId(42));
    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

// ==================== Contradiction Tests ====================

#[test]
fn test_simple_contradiction() {
    let mut checker = Z3FeasibilityChecker::new();
    // block_42 AND !block_42 is a contradiction
    let pred = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(42))),
        Box::new(PathPredicate::BlockFalse(BlockId(42))),
    );
    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Unsatisfiable);
    assert!(result.is_infeasible());
}

#[test]
fn test_nested_contradiction() {
    let mut checker = Z3FeasibilityChecker::new();
    // (block_1 AND block_2) AND (block_1 AND !block_1)
    let left = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );
    let right = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockFalse(BlockId(1))),
    );
    let pred = PathPredicate::And(Box::new(left), Box::new(right));
    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Unsatisfiable);
}

#[test]
fn test_triple_contradiction() {
    let mut checker = Z3FeasibilityChecker::new();
    // block_1 AND !block_1 AND block_2
    let pred = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::And(
            Box::new(PathPredicate::BlockFalse(BlockId(1))),
            Box::new(PathPredicate::BlockTrue(BlockId(2))),
        )),
    );
    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Unsatisfiable);
}

// ==================== Tautology Tests ====================

#[test]
fn test_simple_tautology() {
    let mut checker = Z3FeasibilityChecker::new();
    // block_42 OR !block_42 is a tautology
    let pred = PathPredicate::Or(
        Box::new(PathPredicate::BlockTrue(BlockId(42))),
        Box::new(PathPredicate::BlockFalse(BlockId(42))),
    );
    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

#[test]
fn test_complex_tautology() {
    let mut checker = Z3FeasibilityChecker::new();
    // (block_1 AND block_2) OR (!block_1 OR !block_2)
    let left = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );
    let right = PathPredicate::Or(
        Box::new(PathPredicate::BlockFalse(BlockId(1))),
        Box::new(PathPredicate::BlockFalse(BlockId(2))),
    );
    let pred = PathPredicate::Or(Box::new(left), Box::new(right));
    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

// ==================== Complex Conjunction Tests ====================

#[test]
fn test_satisfiable_conjunction() {
    let mut checker = Z3FeasibilityChecker::new();
    // block_1 AND block_2 AND block_3 (all different blocks)
    let pred = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::And(
            Box::new(PathPredicate::BlockTrue(BlockId(2))),
            Box::new(PathPredicate::BlockTrue(BlockId(3))),
        )),
    );
    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

#[test]
fn test_large_conjunction() {
    let mut checker = Z3FeasibilityChecker::new();
    // block_1 AND block_2 AND ... AND block_10
    let mut pred = PathPredicate::BlockTrue(BlockId(1));
    for i in 2..=10 {
        pred = PathPredicate::And(
            Box::new(pred),
            Box::new(PathPredicate::BlockTrue(BlockId(i))),
        );
    }
    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

// ==================== Complex Disjunction Tests ====================

#[test]
fn test_satisfiable_disjunction() {
    let mut checker = Z3FeasibilityChecker::new();
    // block_1 OR block_2 OR block_3
    let pred = PathPredicate::Or(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::Or(
            Box::new(PathPredicate::BlockTrue(BlockId(2))),
            Box::new(PathPredicate::BlockTrue(BlockId(3))),
        )),
    );
    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

#[test]
fn test_unsatisfiable_disjunction() {
    let mut checker = Z3FeasibilityChecker::new();
    // false OR false OR false
    let pred = PathPredicate::Or(
        Box::new(PathPredicate::False),
        Box::new(PathPredicate::Or(
            Box::new(PathPredicate::False),
            Box::new(PathPredicate::False),
        )),
    );
    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Unsatisfiable);
}

// ==================== Negation Tests ====================

#[test]
fn test_double_negation() {
    let mut checker = Z3FeasibilityChecker::new();
    // NOT(NOT(block_42)) should be equivalent to block_42
    let pred = PathPredicate::Not(Box::new(PathPredicate::Not(Box::new(
        PathPredicate::BlockTrue(BlockId(42)),
    ))));
    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

#[test]
fn test_negation_of_true() {
    let mut checker = Z3FeasibilityChecker::new();
    // NOT(true) = false
    let pred = PathPredicate::Not(Box::new(PathPredicate::True));
    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Unsatisfiable);
}

#[test]
fn test_negation_of_false() {
    let mut checker = Z3FeasibilityChecker::new();
    // NOT(false) = true
    let pred = PathPredicate::Not(Box::new(PathPredicate::False));
    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

#[test]
fn test_de_morgans_law_and() {
    let mut checker = Z3FeasibilityChecker::new();
    // NOT(block_1 AND block_2) should be equivalent to (!block_1 OR !block_2)
    let not_and = PathPredicate::Not(Box::new(PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    )));
    let or_not = PathPredicate::Or(
        Box::new(PathPredicate::BlockFalse(BlockId(1))),
        Box::new(PathPredicate::BlockFalse(BlockId(2))),
    );

    let result1 = checker.check_feasible(&not_and);
    let result2 = checker.check_feasible(&or_not);

    // Both should be satisfiable
    assert_eq!(result1, FeasibilityResult::Satisfiable);
    assert_eq!(result2, FeasibilityResult::Satisfiable);
}

// ==================== Nested Boolean Expression Tests ====================

#[test]
fn test_deeply_nested_expression() {
    let mut checker = Z3FeasibilityChecker::new();
    // ((block_1 AND block_2) OR (block_3 AND block_4)) AND block_5
    let left_and = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );
    let right_and = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(3))),
        Box::new(PathPredicate::BlockTrue(BlockId(4))),
    );
    let or_expr = PathPredicate::Or(Box::new(left_and), Box::new(right_and));
    let pred = PathPredicate::And(
        Box::new(or_expr),
        Box::new(PathPredicate::BlockTrue(BlockId(5))),
    );

    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

#[test]
fn test_nested_contradiction_in_disjunction() {
    let mut checker = Z3FeasibilityChecker::new();
    // (block_1 AND !block_1) OR block_2
    // This should be satisfiable (because of block_2)
    let contradiction = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockFalse(BlockId(1))),
    );
    let pred = PathPredicate::Or(
        Box::new(contradiction),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );

    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

#[test]
fn test_nested_tautology_in_conjunction() {
    let mut checker = Z3FeasibilityChecker::new();
    // (block_1 OR !block_1) AND block_2
    // This should be satisfiable (equivalent to block_2)
    let tautology = PathPredicate::Or(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockFalse(BlockId(1))),
    );
    let pred = PathPredicate::And(
        Box::new(tautology),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );

    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

// ==================== Cache Tests ====================

#[test]
fn test_cache_hit() {
    let mut checker = Z3FeasibilityChecker::new();
    let pred = PathPredicate::BlockTrue(BlockId(42));

    // First check - cache miss
    let result1 = checker.check_feasible(&pred);
    assert_eq!(checker.stats().misses, 1);
    assert_eq!(checker.stats().hits, 0);

    // Second check - cache hit
    let result2 = checker.check_feasible(&pred);
    assert_eq!(result1, result2);
    assert_eq!(checker.stats().misses, 1);
    assert_eq!(checker.stats().hits, 1);
    assert_eq!(checker.stats().hit_rate(), 0.5);
}

#[test]
fn test_cache_multiple_predicates() {
    let mut checker = Z3FeasibilityChecker::new();

    // Check multiple predicates
    let pred1 = PathPredicate::BlockTrue(BlockId(1));
    let pred2 = PathPredicate::BlockTrue(BlockId(2));
    let pred3 = PathPredicate::And(Box::new(pred1.clone()), Box::new(pred2.clone()));

    checker.check_feasible(&pred1);
    checker.check_feasible(&pred2);
    checker.check_feasible(&pred3);

    // Re-check pred1 - should be cached
    checker.check_feasible(&pred1);

    assert_eq!(checker.stats().misses, 3); // pred1, pred2, pred3
    assert_eq!(checker.stats().hits, 1); // pred1 second time
}

#[test]
fn test_cache_eviction() {
    // Small cache for testing eviction
    let mut checker = Z3FeasibilityChecker::with_config(2, 100);

    // Fill cache with 2 entries
    checker.check_feasible(&PathPredicate::BlockTrue(BlockId(1)));
    checker.check_feasible(&PathPredicate::BlockTrue(BlockId(2)));

    // This should evict the LRU entry (block_1)
    checker.check_feasible(&PathPredicate::BlockTrue(BlockId(3)));

    assert_eq!(checker.stats().evictions, 1);

    // Check block_1 again - should be a miss (was evicted)
    checker.check_feasible(&PathPredicate::BlockTrue(BlockId(1)));
    assert_eq!(checker.stats().misses, 4); // 1, 2, 3, 1 again
}

#[test]
fn test_clear_cache() {
    let mut checker = Z3FeasibilityChecker::new();

    // Add some entries
    checker.check_feasible(&PathPredicate::BlockTrue(BlockId(1)));
    checker.check_feasible(&PathPredicate::BlockTrue(BlockId(2)));

    // Clear cache
    checker.clear_cache();

    assert_eq!(checker.stats().hits, 0);
    assert_eq!(checker.stats().misses, 0);
}

// ==================== PathCondition Integration Tests ====================

#[test]
fn test_path_condition_feasibility() {
    let mut checker = Z3FeasibilityChecker::new();

    let path = PathCondition {
        predicate: PathPredicate::BlockTrue(BlockId(42)),
        blocks: verum_common::List::new(),
    };

    assert!(checker.check_path_condition_feasible(&path));
}

#[test]
fn test_infeasible_path_condition() {
    let mut checker = Z3FeasibilityChecker::new();

    let path = PathCondition {
        predicate: PathPredicate::And(
            Box::new(PathPredicate::BlockTrue(BlockId(42))),
            Box::new(PathPredicate::BlockFalse(BlockId(42))),
        ),
        blocks: verum_common::List::new(),
    };

    assert!(!checker.check_path_condition_feasible(&path));
}

// ==================== Builder Tests ====================

#[test]
fn test_builder_default() {
    let _checker = Z3FeasibilityCheckerBuilder::new().build();
    // Builder creates checker with default configuration
    // (max_cache_size=1000, timeout_ms=100)
}

#[test]
fn test_builder_custom_config() {
    let _checker = Z3FeasibilityCheckerBuilder::new()
        .with_cache_size(5000)
        .with_timeout(500)
        .build();
    // Builder creates checker with custom configuration
    // (max_cache_size=5000, timeout_ms=500)
}

// ==================== Performance Tests ====================

#[test]
fn test_large_predicate() {
    let mut checker = Z3FeasibilityChecker::new();

    // Create a large conjunction of different blocks
    let mut pred = PathPredicate::BlockTrue(BlockId(1));
    for i in 2..=100 {
        pred = PathPredicate::And(
            Box::new(pred),
            Box::new(PathPredicate::BlockTrue(BlockId(i))),
        );
    }

    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

#[test]
fn test_deep_nesting() {
    let mut checker = Z3FeasibilityChecker::new();

    // Create deeply nested NOT expressions
    let mut pred = PathPredicate::BlockTrue(BlockId(42));
    for _ in 0..20 {
        pred = PathPredicate::Not(Box::new(PathPredicate::Not(Box::new(pred))));
    }

    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

// ==================== Edge Cases ====================

#[test]
fn test_mixed_blocks_satisfiable() {
    let mut checker = Z3FeasibilityChecker::new();
    // block_1 AND !block_2 AND block_3
    // This is satisfiable (different blocks)
    let pred = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::And(
            Box::new(PathPredicate::BlockFalse(BlockId(2))),
            Box::new(PathPredicate::BlockTrue(BlockId(3))),
        )),
    );
    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

#[test]
fn test_same_block_different_paths() {
    let mut checker = Z3FeasibilityChecker::new();
    // (block_42 AND block_1) OR (block_42 AND block_2)
    let left = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(42))),
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
    );
    let right = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(42))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );
    let pred = PathPredicate::Or(Box::new(left), Box::new(right));

    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

#[test]
fn test_distributive_law() {
    let mut checker = Z3FeasibilityChecker::new();
    // (block_1 OR block_2) AND block_3
    let or_expr = PathPredicate::Or(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );
    let pred = PathPredicate::And(
        Box::new(or_expr),
        Box::new(PathPredicate::BlockTrue(BlockId(3))),
    );

    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

// ==================== Stress Tests ====================

#[test]
fn test_many_disjuncts() {
    let mut checker = Z3FeasibilityChecker::new();

    // Create OR of 50 different blocks
    let mut pred = PathPredicate::BlockTrue(BlockId(1));
    for i in 2..=50 {
        pred = PathPredicate::Or(
            Box::new(pred),
            Box::new(PathPredicate::BlockTrue(BlockId(i))),
        );
    }

    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}

#[test]
fn test_alternating_and_or() {
    let mut checker = Z3FeasibilityChecker::new();

    // (block_1 AND block_2) OR (block_3 AND block_4) OR (block_5 AND block_6)
    let and1 = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );
    let and2 = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(3))),
        Box::new(PathPredicate::BlockTrue(BlockId(4))),
    );
    let and3 = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(5))),
        Box::new(PathPredicate::BlockTrue(BlockId(6))),
    );

    let pred = PathPredicate::Or(
        Box::new(and1),
        Box::new(PathPredicate::Or(Box::new(and2), Box::new(and3))),
    );

    let result = checker.check_feasible(&pred);
    assert_eq!(result, FeasibilityResult::Satisfiable);
}
