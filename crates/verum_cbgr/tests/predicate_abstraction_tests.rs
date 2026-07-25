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
//! Comprehensive tests for predicate abstraction
//!
//! This test suite validates all aspects of predicate abstraction:
//! - Syntactic similarity detection
//! - Semantic equivalence (via Z3)
//! - Subsumption checking
//! - Widening operator
//! - Path merging
//! - Soundness validation
//! - Precision measurement
//! - Performance characteristics

use verum_cbgr::analysis::{BlockId, PathCondition, PathPredicate};
use verum_cbgr::predicate_abstraction::{
    AbstractPredicate, AbstractionConfig, AbstractorBuilder, PredicateAbstractor,
};
use verum_cbgr::z3_feasibility::Z3FeasibilityCheckerBuilder;
use verum_common::List;

// ============================================================================
// Test 1: Syntactic Similarity Detection
// ============================================================================

#[test]
fn test_syntactic_similarity_same_structure() {
    let mut abstractor = PredicateAbstractor::default();

    // Two BlockTrue predicates with different IDs
    let p1 = PathPredicate::BlockTrue(BlockId(1));
    let p2 = PathPredicate::BlockTrue(BlockId(2));

    // Should be structurally similar
    assert!(abstractor.are_similar(&p1, &p2));
}

#[test]
fn test_syntactic_similarity_different_structure() {
    let mut abstractor = PredicateAbstractor::default();

    let p1 = PathPredicate::BlockTrue(BlockId(1));
    let p2 = PathPredicate::BlockFalse(BlockId(1));

    // Different structure (True vs False)
    assert!(!abstractor.are_similar(&p1, &p2));
}

#[test]
fn test_syntactic_similarity_nested_and() {
    let mut abstractor = PredicateAbstractor::default();

    // (BlockTrue(1) AND BlockTrue(2))
    let p1 = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );

    // (BlockTrue(3) AND BlockTrue(4))
    let p2 = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(3))),
        Box::new(PathPredicate::BlockTrue(BlockId(4))),
    );

    // Same structure (both are AND of two BlockTrue)
    assert!(abstractor.are_similar(&p1, &p2));
}

#[test]
fn test_syntactic_similarity_identical_predicates() {
    let mut abstractor = PredicateAbstractor::default();

    let p1 = PathPredicate::BlockTrue(BlockId(42));
    let p2 = PathPredicate::BlockTrue(BlockId(42));

    // Identical predicates are always similar
    assert!(abstractor.are_similar(&p1, &p2));
}

// ============================================================================
// Test 2: Semantic Equivalence (via Z3)
// ============================================================================

#[test]
fn test_semantic_equivalence_commutative_and() {
    let mut abstractor = PredicateAbstractor::default();
    let mut z3 = Z3FeasibilityCheckerBuilder::new().build();

    // (A AND B)
    let p1 = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );

    // (B AND A) - should be equivalent
    let p2 = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
    );

    // Check via Z3
    let equivalent = abstractor.check_equivalence_z3(&p1, &p2, &mut z3);
    assert!(equivalent, "Commutative AND should be equivalent");
}

#[test]
fn test_semantic_equivalence_de_morgan() {
    let mut abstractor = PredicateAbstractor::default();
    let mut z3 = Z3FeasibilityCheckerBuilder::new().build();

    // NOT (A AND B)
    let p1 = PathPredicate::Not(Box::new(PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    )));

    // (NOT A) OR (NOT B) - De Morgan's law
    let p2 = PathPredicate::Or(
        Box::new(PathPredicate::Not(Box::new(PathPredicate::BlockTrue(
            BlockId(1),
        )))),
        Box::new(PathPredicate::Not(Box::new(PathPredicate::BlockTrue(
            BlockId(2),
        )))),
    );

    let equivalent = abstractor.check_equivalence_z3(&p1, &p2, &mut z3);
    assert!(equivalent, "De Morgan's law should hold");
}

// ============================================================================
// Test 3: Subsumption Checking
// ============================================================================

#[test]
fn test_subsumption_simple() {
    let mut abstractor = PredicateAbstractor::default();
    let mut z3 = Z3FeasibilityCheckerBuilder::new().build();

    // (A AND B) implies A
    let stronger = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );

    let weaker = PathPredicate::BlockTrue(BlockId(1));

    // Stronger subsumes weaker (stronger → weaker)
    let subsumes = abstractor.check_subsumption_z3(&stronger, &weaker, &mut z3);
    assert!(subsumes, "(A AND B) should subsume A");
}

#[test]
fn test_subsumption_not_implied() {
    let mut abstractor = PredicateAbstractor::default();
    let mut z3 = Z3FeasibilityCheckerBuilder::new().build();

    let p1 = PathPredicate::BlockTrue(BlockId(1));
    let p2 = PathPredicate::BlockTrue(BlockId(2));

    // p1 does not imply p2
    let subsumes = abstractor.check_subsumption_z3(&p1, &p2, &mut z3);
    assert!(!subsumes, "BlockTrue(1) should not subsume BlockTrue(2)");
}

// ============================================================================
// Test 4: Widening Operator
// ============================================================================

#[test]
fn test_widening_after_threshold() {
    let config = AbstractionConfig {
        widening_threshold: 2,
        ..Default::default()
    };
    let mut abstractor = PredicateAbstractor::new(config);

    let pred = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );

    // First abstraction - below threshold
    let result1 = abstractor.abstract_predicate(&pred, 3);
    assert!(!result1.is_true(), "First widening should not go to True");

    // Second abstraction - at threshold
    let result2 = abstractor.abstract_predicate(&pred, 3);
    assert!(!result2.is_true(), "Second widening should not go to True");

    // Third abstraction - above threshold
    let result3 = abstractor.abstract_predicate(&pred, 3);
    assert!(
        result3.is_true(),
        "Third widening should widen simple AND to True"
    );
}

#[test]
fn test_widening_iteration_tracking() {
    let mut abstractor = PredicateAbstractor::default();

    let pred1 = PathPredicate::BlockTrue(BlockId(1));
    let pred2 = PathPredicate::BlockTrue(BlockId(2));

    // Different predicates should have independent iteration counts
    abstractor.abstract_predicate(&pred1, 3);
    abstractor.abstract_predicate(&pred1, 3);
    abstractor.abstract_predicate(&pred2, 3);

    // pred1 has 2 iterations, pred2 has 1
    let stats = abstractor.stats();
    assert!(stats.widening_operations >= 3);
}

// ============================================================================
// Test 5: Path Merging
// ============================================================================

#[test]
fn test_path_merging_reduces_count() {
    // Use config with low threshold so merging triggers
    let config = AbstractionConfig {
        path_threshold: 5,
        ..Default::default()
    };
    let mut abstractor = PredicateAbstractor::new(config);

    // Create 10 similar paths (above threshold of 5)
    let mut paths = List::new();
    for i in 0..10 {
        let pred = PathPredicate::BlockTrue(BlockId(i as u64));
        paths.push(PathCondition::with_predicate(pred));
    }

    // Merge should reduce to fewer paths
    let merged = abstractor.merge_similar_paths(paths.clone());

    // Should merge similar BlockTrue predicates
    assert!(
        merged.len() < paths.len(),
        "Merging should reduce path count (got {}, expected < {})",
        merged.len(),
        paths.len()
    );
}

#[test]
fn test_path_merging_below_threshold() {
    let config = AbstractionConfig {
        path_threshold: 100,
        ..Default::default()
    };
    let mut abstractor = PredicateAbstractor::new(config);

    // Create 10 paths (below threshold of 100)
    let mut paths = List::new();
    for i in 0..10 {
        let pred = PathPredicate::BlockTrue(BlockId(i as u64));
        paths.push(PathCondition::with_predicate(pred));
    }

    // Should not merge (below threshold)
    let merged = abstractor.merge_similar_paths(paths.clone());
    assert_eq!(
        merged.len(),
        paths.len(),
        "Should not merge below threshold"
    );
}

#[test]
fn test_path_merging_empty_input() {
    let mut abstractor = PredicateAbstractor::default();

    let paths = List::new();
    let merged = abstractor.merge_similar_paths(paths);

    assert_eq!(merged.len(), 0, "Empty input should yield empty output");
}

#[test]
fn test_path_merging_single_path() {
    let mut abstractor = PredicateAbstractor::default();

    let mut paths = List::new();
    paths.push(PathCondition::with_predicate(PathPredicate::BlockTrue(
        BlockId(1),
    )));

    let merged = abstractor.merge_similar_paths(paths.clone());
    assert_eq!(merged.len(), 1, "Single path should remain unchanged");
}

// ============================================================================
// Test 6: Soundness Validation
// ============================================================================

#[test]
fn test_soundness_no_false_negatives() {
    let mut abstractor = PredicateAbstractor::default();

    // Create paths with different escape behaviors
    let escaping_path = PathCondition::with_predicate(PathPredicate::BlockTrue(BlockId(1)));
    let non_escaping_path = PathCondition::with_predicate(PathPredicate::BlockFalse(BlockId(1)));

    let mut paths = List::new();
    paths.push(escaping_path);
    paths.push(non_escaping_path);

    // Merge should keep both paths (different predicates)
    let merged = abstractor.merge_similar_paths(paths.clone());

    // Should not merge contradictory paths
    assert_eq!(merged.len(), 2, "Contradictory paths should not be merged");
}

#[test]
fn test_soundness_abstraction_preserves_feasibility() {
    let mut abstractor = PredicateAbstractor::default();

    // Feasible predicate
    let feasible = PathPredicate::BlockTrue(BlockId(42));

    // Abstract at various levels
    for level in 0..=4 {
        let abstracted = abstractor.abstract_predicate(&feasible, level);

        // Abstraction should not make feasible predicate infeasible
        assert!(
            !abstracted.is_false(),
            "Abstraction at level {} should not make feasible predicate infeasible",
            level
        );
    }
}

#[test]
fn test_soundness_infeasible_stays_infeasible() {
    let mut abstractor = PredicateAbstractor::default();

    // Infeasible predicate: (A AND NOT A)
    let infeasible = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockFalse(BlockId(1))),
    );

    // Abstract at level 1 (should simplify to False)
    let abstracted = abstractor.abstract_predicate(&infeasible, 1);

    // Should remain infeasible (or be detected as such)
    let simplified = abstracted.simplify();
    assert!(
        simplified.is_false(),
        "Infeasible predicate should remain infeasible after abstraction"
    );
}

// ============================================================================
// Test 7: Precision Measurement
// ============================================================================

#[test]
fn test_precision_level0_preserves_exact() {
    let mut abstractor = PredicateAbstractor::default();

    let pred = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );

    // Level 0 should preserve exact predicate
    let abstracted = abstractor.abstract_predicate(&pred, 0);
    assert_eq!(abstracted, pred, "Level 0 should preserve exact predicate");
}

#[test]
fn test_precision_level1_normalizes() {
    let mut abstractor = PredicateAbstractor::default();

    // Double negation
    let pred = PathPredicate::Not(Box::new(PathPredicate::Not(Box::new(
        PathPredicate::BlockTrue(BlockId(1)),
    ))));

    // Level 1 should normalize (eliminate double negation)
    let abstracted = abstractor.abstract_predicate(&pred, 1);
    let expected = PathPredicate::BlockTrue(BlockId(1));

    assert_eq!(abstracted, expected, "Level 1 should normalize predicates");
}

#[test]
fn test_precision_level4_abstracts_to_top() {
    let mut abstractor = PredicateAbstractor::default();

    let pred = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );

    // Level 4 should abstract to True (top)
    let abstracted = abstractor.abstract_predicate(&pred, 4);
    assert!(abstracted.is_true(), "Level 4 should abstract to True");
}

// ============================================================================
// Test 8: Abstraction Levels
// ============================================================================

#[test]
fn test_abstraction_level_progression() {
    let mut abstractor = PredicateAbstractor::default();

    let pred = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );

    // Each level should be at least as abstract as previous
    let l0 = abstractor.abstract_predicate(&pred, 0);
    let l1 = abstractor.abstract_predicate(&pred, 1);

    // Level 0 should be most precise
    assert_eq!(l0, pred, "Level 0 should be concrete");

    // Level 1 should be normalized
    // (can't easily test monotonicity without Z3, but check it doesn't error)
    let _l1_check = l1;
}

#[test]
fn test_abstraction_clamps_max_level() {
    let config = AbstractionConfig {
        max_abstraction_level: 2,
        ..Default::default()
    };
    let mut abstractor = PredicateAbstractor::new(config);

    let pred = PathPredicate::BlockTrue(BlockId(1));

    // Request level 4 but max is 2
    let abstracted = abstractor.abstract_predicate(&pred, 4);

    // Should not go to True (level 4), should clamp at level 2
    assert!(
        !abstracted.is_true(),
        "Should clamp to max_abstraction_level"
    );
}

// ============================================================================
// Test 9: Caching
// ============================================================================

#[test]
fn test_cache_hit_performance() {
    let mut abstractor = PredicateAbstractor::default();

    let pred = PathPredicate::BlockTrue(BlockId(42));

    // First call - cache miss
    abstractor.abstract_predicate(&pred, 1);
    let stats1 = abstractor.stats();
    assert_eq!(stats1.cache_misses, 1);
    assert_eq!(stats1.cache_hits, 0);

    // Second call - cache hit
    abstractor.abstract_predicate(&pred, 1);
    let stats2 = abstractor.stats();
    assert_eq!(stats2.cache_hits, 1);
}

#[test]
fn test_cache_different_levels() {
    let mut abstractor = PredicateAbstractor::default();

    let pred = PathPredicate::BlockTrue(BlockId(42));

    // Abstract at level 1
    abstractor.abstract_predicate(&pred, 1);

    // Abstract at level 2 - different cache key
    abstractor.abstract_predicate(&pred, 2);

    let stats = abstractor.stats();
    assert_eq!(
        stats.cache_misses, 2,
        "Different levels should use different cache keys"
    );
}

#[test]
fn test_cache_size_limit() {
    let config = AbstractionConfig {
        max_cache_size: 2,
        ..Default::default()
    };
    let mut abstractor = PredicateAbstractor::new(config);

    // Fill cache beyond limit
    for i in 0..10 {
        let pred = PathPredicate::BlockTrue(BlockId(i as u64));
        abstractor.abstract_predicate(&pred, 1);
    }

    // Cache should not grow beyond limit
    // (internal detail, can't directly test, but verify no crash)
    let stats = abstractor.stats();
    assert!(stats.total_abstractions > 0);
}

// ============================================================================
// Test 10: Loop Handling
// ============================================================================

#[test]
fn test_loop_path_explosion_prevented() {
    let mut abstractor = PredicateAbstractor::default();

    // Simulate loop: create many paths with similar structure
    let mut paths = List::new();
    for i in 0..100 {
        // Loop iteration i: BlockTrue(loop_header) AND BlockTrue(i)
        let pred = PathPredicate::And(
            Box::new(PathPredicate::BlockTrue(BlockId(0))), // loop header
            Box::new(PathPredicate::BlockTrue(BlockId(i as u64))), // iteration
        );
        paths.push(PathCondition::with_predicate(pred));
    }

    // Merge should drastically reduce
    let merged = abstractor.merge_similar_paths(paths.clone());

    assert!(
        merged.len() < paths.len() / 2,
        "Loop paths should be heavily merged (got {}, expected < {})",
        merged.len(),
        paths.len() / 2
    );
}

// ============================================================================
// Test 11: Builder Pattern
// ============================================================================

#[test]
fn test_builder_configuration() {
    let abstractor = AbstractorBuilder::new()
        .max_abstraction_level(3)
        .path_threshold(25)
        .use_z3_equivalence(false)
        .use_subsumption(true)
        .use_widening(true)
        .widening_threshold(5)
        .max_cache_size(5000)
        .incremental_merging(false)
        .build();

    let stats = abstractor.stats();
    assert_eq!(
        stats.total_abstractions, 0,
        "New abstractor should have 0 abstractions"
    );
}

#[test]
fn test_builder_default() {
    let abstractor = AbstractorBuilder::default().build();

    let stats = abstractor.stats();
    assert_eq!(stats.total_abstractions, 0);
}

// ============================================================================
// Test 12: Abstract Predicate
// ============================================================================

#[test]
fn test_abstract_predicate_canonicalization() {
    // Commutative AND: (A AND B) should equal (B AND A)
    let p1 = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
    );

    let p2 = PathPredicate::And(
        Box::new(PathPredicate::BlockTrue(BlockId(2))),
        Box::new(PathPredicate::BlockTrue(BlockId(1))),
    );

    let abs1 = AbstractPredicate::new(p1, 1);
    let abs2 = AbstractPredicate::new(p2, 1);

    // Should have same canonical form and hash
    assert_eq!(
        abs1.hash_value(),
        abs2.hash_value(),
        "Commutative predicates should have same canonical hash"
    );
}

#[test]
fn test_abstract_predicate_equivalence_class() {
    let pred = PathPredicate::BlockTrue(BlockId(42));
    let mut abs = AbstractPredicate::new(pred.clone(), 0);

    assert_eq!(abs.equivalence_class.len(), 1);
    assert!(abs.equivalence_class.contains(&pred));

    // Add another predicate
    let pred2 = PathPredicate::BlockTrue(BlockId(43));
    abs.add_to_equivalence_class(pred2.clone());

    assert_eq!(abs.equivalence_class.len(), 2);
    assert!(abs.equivalence_class.contains(&pred2));
}

// ============================================================================
// Test 13: Edge Cases
// ============================================================================

#[test]
fn test_edge_case_always_true() {
    let mut abstractor = PredicateAbstractor::default();

    let pred = PathPredicate::True;

    // Abstracting True at any level should remain True
    for level in 0..=4 {
        let abstracted = abstractor.abstract_predicate(&pred, level);
        assert!(
            abstracted.is_true(),
            "True should remain True at all levels"
        );
    }
}

#[test]
fn test_edge_case_always_false() {
    let mut abstractor = PredicateAbstractor::default();

    let pred = PathPredicate::False;

    // Abstracting False at any level should remain False
    for level in 0..=4 {
        let abstracted = abstractor.abstract_predicate(&pred, level);
        assert!(
            abstracted.is_false(),
            "False should remain False at all levels"
        );
    }
}

#[test]
fn test_edge_case_deeply_nested() {
    let mut abstractor = PredicateAbstractor::default();

    // Create deeply nested predicate: (((A AND B) AND C) AND D)
    let mut pred = PathPredicate::BlockTrue(BlockId(1));
    for i in 2..=10 {
        pred = PathPredicate::And(
            Box::new(pred),
            Box::new(PathPredicate::BlockTrue(BlockId(i as u64))),
        );
    }

    // Should handle without stack overflow
    let abstracted = abstractor.abstract_predicate(&pred, 1);

    // Should not crash
    let _check = abstracted;
}

// ============================================================================
// Test 14: Statistics
// ============================================================================

#[test]
fn test_statistics_tracking() {
    let mut abstractor = PredicateAbstractor::default();

    let pred1 = PathPredicate::BlockTrue(BlockId(1));
    let pred2 = PathPredicate::BlockTrue(BlockId(2));

    // Perform operations
    abstractor.abstract_predicate(&pred1, 1);
    abstractor.abstract_predicate(&pred2, 2);
    abstractor.are_similar(&pred1, &pred2);

    let stats = abstractor.stats();

    assert_eq!(stats.total_abstractions, 2);
    assert!(stats.time_ns > 0, "Should track time");
}

#[test]
fn test_statistics_reset() {
    let mut abstractor = PredicateAbstractor::default();

    let pred = PathPredicate::BlockTrue(BlockId(1));
    abstractor.abstract_predicate(&pred, 1);

    assert!(abstractor.stats().total_abstractions > 0);

    // Reset stats
    abstractor.reset_stats();

    assert_eq!(abstractor.stats().total_abstractions, 0);
}

// ============================================================================
// Test 15: Cache Clearing
// ============================================================================

#[test]
fn test_clear_caches() {
    let mut abstractor = PredicateAbstractor::default();

    let pred = PathPredicate::BlockTrue(BlockId(1));

    // Populate caches
    abstractor.abstract_predicate(&pred, 1);
    abstractor.abstract_predicate(&pred, 1); // Cache hit

    let stats_before = abstractor.stats().clone();
    assert!(stats_before.cache_hits > 0);

    // Clear caches
    abstractor.clear_caches();

    // Reset stats to test cache miss after clear
    abstractor.reset_stats();

    // Should be cache miss now
    abstractor.abstract_predicate(&pred, 1);
    let stats_after = abstractor.stats();
    assert_eq!(stats_after.cache_misses, 1);
    assert_eq!(stats_after.cache_hits, 0);
}
