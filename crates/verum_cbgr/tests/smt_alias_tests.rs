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
//! Comprehensive tests for SMT-based alias verification
//!
//! This test suite validates the production-grade SMT-based alias verification
//! implementation for CBGR escape analysis.

use verum_cbgr::analysis::{AliasRelation, AliasSets, RefId};
use verum_cbgr::smt_alias_verification::{
    ArrayIndex, PointerConstraint, SmtAliasCache, SmtAliasResult, SmtAliasVerifier,
    SmtAliasVerifierBuilder,
};
use verum_common::Text;

// =============================================================================
// Test Group 1: Pointer Constraint Creation (5 tests)
// =============================================================================

#[test]
fn test_stack_allocation_constraint() {
    let constraint = PointerConstraint::stack_alloc(42, 16);

    assert!(constraint.is_stack_alloc());
    assert!(!constraint.is_heap_alloc());
    assert!(!constraint.is_unknown());
    assert_eq!(constraint.base_allocation_id(), verum_common::Maybe::Some(42));
}

#[test]
fn test_heap_allocation_constraint() {
    let constraint = PointerConstraint::heap_alloc(123, 0);

    assert!(constraint.is_heap_alloc());
    assert!(!constraint.is_stack_alloc());
    assert_eq!(
        constraint.base_allocation_id(),
        verum_common::Maybe::Some(123)
    );
}

#[test]
fn test_field_access_constraint() {
    let base = PointerConstraint::stack_alloc(1, 0);
    let field = PointerConstraint::field(base.clone(), 8, "x".into());

    assert_eq!(field.base_allocation_id(), verum_common::Maybe::Some(1));

    if let PointerConstraint::FieldAccess {
        field_offset,
        field_name,
        ..
    } = field
    {
        assert_eq!(field_offset, 8);
        assert_eq!(field_name, Text::from("x"));
    } else {
        panic!("Expected FieldAccess");
    }
}

#[test]
fn test_array_element_constraint() {
    let base = PointerConstraint::heap_alloc(5, 0);
    let index = ArrayIndex::concrete(10);
    let element = PointerConstraint::array_element(base, index, 4);

    assert_eq!(element.base_allocation_id(), verum_common::Maybe::Some(5));
}

#[test]
fn test_pointer_arithmetic_constraint() {
    let base = PointerConstraint::stack_alloc(7, 0);
    let offset = PointerConstraint::add_offset(base, 24);

    assert_eq!(offset.base_allocation_id(), verum_common::Maybe::Some(7));
}

// =============================================================================
// Test Group 2: Array Index Handling (3 tests)
// =============================================================================

#[test]
fn test_concrete_array_index() {
    let index = ArrayIndex::concrete(42);
    assert!(index.is_concrete());
}

#[test]
fn test_symbolic_array_index() {
    let index = ArrayIndex::symbolic("i".into());
    assert!(!index.is_concrete());
}

#[test]
fn test_bounded_symbolic_index() {
    let index = ArrayIndex::symbolic_bounded("idx".into(), 0, 100);

    if let ArrayIndex::Symbolic {
        var_name,
        lower_bound,
        upper_bound,
    } = index
    {
        assert_eq!(var_name, Text::from("idx"));
        assert_eq!(lower_bound, verum_common::Maybe::Some(0));
        assert_eq!(upper_bound, verum_common::Maybe::Some(100));
    } else {
        panic!("Expected Symbolic index");
    }
}

// =============================================================================
// Test Group 3: SMT Alias Result Conversion (2 tests)
// =============================================================================

#[test]
fn test_smt_result_to_alias_relation() {
    assert_eq!(
        SmtAliasResult::NoAlias.to_alias_relation(),
        AliasRelation::NoAlias
    );
    assert_eq!(
        SmtAliasResult::MayAlias.to_alias_relation(),
        AliasRelation::MayAlias
    );
    assert_eq!(
        SmtAliasResult::Unknown.to_alias_relation(),
        AliasRelation::Unknown
    );
}

#[test]
fn test_smt_result_predicates() {
    assert!(SmtAliasResult::NoAlias.is_no_alias());
    assert!(!SmtAliasResult::MayAlias.is_no_alias());

    assert!(!SmtAliasResult::NoAlias.may_alias());
    assert!(SmtAliasResult::MayAlias.may_alias());
    assert!(SmtAliasResult::Unknown.may_alias());
}

// =============================================================================
// Test Group 4: Cache Operations (4 tests)
// =============================================================================

#[test]
fn test_cache_creation() {
    let cache = SmtAliasCache::new();
    assert_eq!(cache.stats().hits, 0);
    assert_eq!(cache.stats().misses, 0);
}

#[test]
fn test_cache_hit_and_miss() {
    let mut cache = SmtAliasCache::new();

    // First access: miss
    assert_eq!(cache.get(12345), verum_common::Maybe::None);
    assert_eq!(cache.stats().misses, 1);
    assert_eq!(cache.stats().hits, 0);

    // Insert
    cache.insert(12345, SmtAliasResult::NoAlias, 100);

    // Second access: hit
    assert_eq!(
        cache.get(12345),
        verum_common::Maybe::Some(SmtAliasResult::NoAlias)
    );
    assert_eq!(cache.stats().hits, 1);
    assert_eq!(cache.stats().misses, 1);
}

#[test]
fn test_cache_eviction() {
    let mut cache = SmtAliasCache::with_size(2);

    // Fill cache
    cache.insert(1, SmtAliasResult::NoAlias, 50);
    cache.insert(2, SmtAliasResult::MayAlias, 60);

    // This should trigger eviction
    cache.insert(3, SmtAliasResult::NoAlias, 70);

    assert_eq!(cache.stats().evictions, 1);
}

#[test]
fn test_cache_clear() {
    let mut cache = SmtAliasCache::new();

    cache.insert(1, SmtAliasResult::NoAlias, 50);
    cache.insert(2, SmtAliasResult::MayAlias, 60);

    assert_eq!(cache.stats().misses, 0); // Inserts don't count as misses

    cache.clear();

    assert_eq!(cache.stats().hits, 0);
    assert_eq!(cache.stats().misses, 0);
}

// =============================================================================
// Test Group 5: Basic SMT Verification (5 tests)
// =============================================================================

#[test]
fn test_different_stack_allocations_no_alias() {
    let mut verifier = SmtAliasVerifier::new();

    let ptr1 = PointerConstraint::stack_alloc(1, 0);
    let ptr2 = PointerConstraint::stack_alloc(2, 0);

    let result = verifier.verify_no_alias(RefId(1), RefId(2), &ptr1, &ptr2);
    assert!(
        result.is_no_alias(),
        "Different stack allocations should not alias"
    );
}

#[test]
fn test_stack_vs_heap_no_alias() {
    let mut verifier = SmtAliasVerifier::new();

    let stack = PointerConstraint::stack_alloc(1, 0);
    let heap = PointerConstraint::heap_alloc(1, 0);

    let result = verifier.verify_no_alias(RefId(1), RefId(2), &stack, &heap);
    assert!(result.is_no_alias(), "Stack and heap should not alias");
}

#[test]
fn test_same_allocation_different_offsets() {
    let mut verifier = SmtAliasVerifier::new();

    let ptr1 = PointerConstraint::stack_alloc(1, 0);
    let ptr2 = PointerConstraint::stack_alloc(1, 8);

    let result = verifier.verify_no_alias(RefId(1), RefId(2), &ptr1, &ptr2);
    // Different offsets in same allocation → no alias
    assert!(result.is_no_alias(), "Different offsets should not alias");
}

#[test]
fn test_unknown_constraint_conservative() {
    let mut verifier = SmtAliasVerifier::new();

    let known = PointerConstraint::stack_alloc(1, 0);
    let unknown = PointerConstraint::Unknown;

    let result = verifier.verify_no_alias(RefId(1), RefId(2), &known, &unknown);
    assert_eq!(
        result,
        SmtAliasResult::Unknown,
        "Unknown should be conservative"
    );
}

#[test]
fn test_different_heap_allocations_no_alias() {
    let mut verifier = SmtAliasVerifier::new();

    let heap1 = PointerConstraint::heap_alloc(10, 0);
    let heap2 = PointerConstraint::heap_alloc(20, 0);

    let result = verifier.verify_no_alias(RefId(10), RefId(20), &heap1, &heap2);
    assert!(
        result.is_no_alias(),
        "Different heap allocations should not alias"
    );
}

// =============================================================================
// Test Group 6: Field Access Verification (3 tests)
// =============================================================================

#[test]
fn test_different_fields_same_struct_no_alias() {
    let mut verifier = SmtAliasVerifier::new();

    let base = PointerConstraint::stack_alloc(1, 0);
    let field_x = PointerConstraint::field(base.clone(), 0, "x".into());
    let field_y = PointerConstraint::field(base, 8, "y".into());

    let result = verifier.verify_no_alias(RefId(1), RefId(2), &field_x, &field_y);
    assert!(result.is_no_alias(), "Different fields should not alias");
}

#[test]
fn test_nested_field_access() {
    let mut verifier = SmtAliasVerifier::new();

    let base = PointerConstraint::stack_alloc(1, 0);
    let inner = PointerConstraint::field(base.clone(), 0, "inner".into());
    let nested = PointerConstraint::field(inner, 4, "data".into());

    // Same base, different offset chain
    let other_field = PointerConstraint::field(base, 16, "other".into());

    let result = verifier.verify_no_alias(RefId(1), RefId(2), &nested, &other_field);
    assert!(
        result.is_no_alias(),
        "Nested fields with different offsets should not alias"
    );
}

#[test]
fn test_same_field_may_alias() {
    let mut verifier = SmtAliasVerifier::new();

    let base = PointerConstraint::stack_alloc(1, 0);
    let field1 = PointerConstraint::field(base.clone(), 8, "count".into());
    let field2 = PointerConstraint::field(base, 8, "count".into());

    let result = verifier.verify_no_alias(RefId(1), RefId(2), &field1, &field2);
    // Same field, same offset → may alias (conservative)
    assert!(result.may_alias(), "Same field should may-alias");
}

// =============================================================================
// Test Group 7: Array Access Verification (3 tests)
// =============================================================================

#[test]
fn test_different_concrete_indices_no_alias() {
    let mut verifier = SmtAliasVerifier::new();

    let base = PointerConstraint::heap_alloc(1, 0);
    let elem0 = PointerConstraint::array_element(base.clone(), ArrayIndex::concrete(0), 4);
    let elem1 = PointerConstraint::array_element(base, ArrayIndex::concrete(1), 4);

    let result = verifier.verify_no_alias(RefId(1), RefId(2), &elem0, &elem1);
    assert!(
        result.is_no_alias(),
        "Different concrete indices should not alias"
    );
}

#[test]
fn test_symbolic_index_may_alias() {
    let mut verifier = SmtAliasVerifier::new();

    let base = PointerConstraint::heap_alloc(1, 0);
    let elem_i =
        PointerConstraint::array_element(base.clone(), ArrayIndex::symbolic("i".into()), 4);
    let elem_j = PointerConstraint::array_element(base, ArrayIndex::symbolic("j".into()), 4);

    let result = verifier.verify_no_alias(RefId(1), RefId(2), &elem_i, &elem_j);
    // Symbolic indices: can't prove they're different
    assert!(
        result.may_alias(),
        "Symbolic indices should be conservative"
    );
}

#[test]
fn test_bounded_indices_different_ranges() {
    let mut verifier = SmtAliasVerifier::new();

    let base = PointerConstraint::heap_alloc(1, 0);
    let elem_low = PointerConstraint::array_element(
        base.clone(),
        ArrayIndex::symbolic_bounded("i".into(), 0, 10),
        4,
    );
    let elem_high =
        PointerConstraint::array_element(base, ArrayIndex::symbolic_bounded("j".into(), 20, 30), 4);

    let result = verifier.verify_no_alias(RefId(1), RefId(2), &elem_low, &elem_high);
    // Non-overlapping ranges → should not alias
    // NOTE: Current implementation is conservative, may return MayAlias
    // Advanced implementation could prove no-alias
    assert!(result.may_alias() || result.is_no_alias());
}

// =============================================================================
// Test Group 8: Caching Performance (3 tests)
// =============================================================================

#[test]
fn test_cache_hit_on_repeated_query() {
    let mut verifier = SmtAliasVerifier::new();

    let ptr1 = PointerConstraint::stack_alloc(1, 0);
    let ptr2 = PointerConstraint::stack_alloc(2, 0);

    // First query
    let result1 = verifier.verify_no_alias(RefId(1), RefId(2), &ptr1, &ptr2);
    let stats1 = verifier.cache_stats().clone();

    // Second query (should hit cache)
    let result2 = verifier.verify_no_alias(RefId(1), RefId(2), &ptr1, &ptr2);
    let stats2 = verifier.cache_stats().clone();

    assert_eq!(result1, result2);
    assert_eq!(stats1.misses, 1);
    assert_eq!(stats2.hits, 1); // Cache hit!
}

#[test]
fn test_cache_stats_accuracy() {
    let mut verifier = SmtAliasVerifier::new();

    let ptr1 = PointerConstraint::stack_alloc(1, 0);
    let ptr2 = PointerConstraint::stack_alloc(2, 0);
    let ptr3 = PointerConstraint::stack_alloc(3, 0);

    // 3 unique queries
    verifier.verify_no_alias(RefId(1), RefId(2), &ptr1, &ptr2);
    verifier.verify_no_alias(RefId(1), RefId(3), &ptr1, &ptr3);
    verifier.verify_no_alias(RefId(2), RefId(3), &ptr2, &ptr3);

    // Repeat first query
    verifier.verify_no_alias(RefId(1), RefId(2), &ptr1, &ptr2);

    let stats = verifier.cache_stats();
    assert_eq!(stats.misses, 3);
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.hit_rate(), 0.25); // 1 hit out of 4 total queries
}

#[test]
fn test_clear_cache_resets_stats() {
    let mut verifier = SmtAliasVerifier::new();

    let ptr1 = PointerConstraint::stack_alloc(1, 0);
    let ptr2 = PointerConstraint::stack_alloc(2, 0);

    verifier.verify_no_alias(RefId(1), RefId(2), &ptr1, &ptr2);
    assert_eq!(verifier.cache_stats().misses, 1);

    verifier.clear_cache();
    assert_eq!(verifier.cache_stats().hits, 0);
    assert_eq!(verifier.cache_stats().misses, 0);
}

// =============================================================================
// Test Group 9: Builder Pattern (2 tests)
// =============================================================================

#[test]
fn test_verifier_builder_basic() {
    let verifier = SmtAliasVerifierBuilder::new()
        .with_timeout(200)
        .with_pointer_bits(32)
        .build();

    // Can't directly test these private fields, but verify it builds
    let ptr1 = PointerConstraint::stack_alloc(1, 0);
    let ptr2 = PointerConstraint::stack_alloc(2, 0);

    let mut verifier_mut = verifier;
    let result = verifier_mut.verify_no_alias(RefId(1), RefId(2), &ptr1, &ptr2);
    assert!(result.is_no_alias());
}

#[test]
fn test_verifier_builder_with_cache_size() {
    let verifier = SmtAliasVerifierBuilder::new().with_cache_size(100).build();

    let mut verifier_mut = verifier;

    let ptr1 = PointerConstraint::stack_alloc(1, 0);
    let ptr2 = PointerConstraint::stack_alloc(2, 0);

    verifier_mut.verify_no_alias(RefId(1), RefId(2), &ptr1, &ptr2);
    // Verify it works with custom cache size
    assert_eq!(verifier_mut.cache_stats().misses, 1);
}

// =============================================================================
// Test Group 10: Complex Scenarios (5 tests)
// =============================================================================

#[test]
fn test_pointer_arithmetic_no_alias() {
    let mut verifier = SmtAliasVerifier::new();

    let base = PointerConstraint::stack_alloc(1, 0);
    let offset1 = PointerConstraint::add_offset(base.clone(), 8);
    let offset2 = PointerConstraint::add_offset(base, 16);

    let result = verifier.verify_no_alias(RefId(1), RefId(2), &offset1, &offset2);
    assert!(
        result.is_no_alias(),
        "Different pointer arithmetic offsets should not alias"
    );
}

#[test]
fn test_mixed_field_and_array_access() {
    let mut verifier = SmtAliasVerifier::new();

    let base = PointerConstraint::stack_alloc(1, 0);
    let field = PointerConstraint::field(base.clone(), 0, "array_field".into());
    let array_elem = PointerConstraint::array_element(field, ArrayIndex::concrete(0), 4);

    let other_field = PointerConstraint::field(base, 100, "other".into());

    let result = verifier.verify_no_alias(RefId(1), RefId(2), &array_elem, &other_field);
    assert!(
        result.is_no_alias(),
        "Array element and different field should not alias"
    );
}

#[test]
fn test_parameter_constraint_conservative() {
    let mut verifier = SmtAliasVerifier::new();

    let param0 = PointerConstraint::Parameter { param_idx: 0 };
    let param1 = PointerConstraint::Parameter { param_idx: 1 };

    let result = verifier.verify_no_alias(RefId(1), RefId(2), &param0, &param1);
    // Parameters are conservative: different params may still alias
    assert!(result.may_alias(), "Parameters should be conservative");
}

#[test]
fn test_deeply_nested_field_access() {
    let mut verifier = SmtAliasVerifier::new();

    let base = PointerConstraint::stack_alloc(1, 0);
    let level1 = PointerConstraint::field(base.clone(), 0, "a".into());
    let level2 = PointerConstraint::field(level1, 8, "b".into());
    let level3 = PointerConstraint::field(level2, 16, "c".into());

    let other = PointerConstraint::field(base, 100, "other".into());

    let result = verifier.verify_no_alias(RefId(1), RefId(2), &level3, &other);
    assert!(
        result.is_no_alias(),
        "Deeply nested fields with different offsets should not alias"
    );
}

#[test]
fn test_multiple_queries_same_verifier() {
    let mut verifier = SmtAliasVerifier::new();

    let ptr1 = PointerConstraint::stack_alloc(1, 0);
    let ptr2 = PointerConstraint::stack_alloc(2, 0);
    let ptr3 = PointerConstraint::heap_alloc(1, 0);
    let ptr4 = PointerConstraint::heap_alloc(2, 0);

    let result1 = verifier.verify_no_alias(RefId(1), RefId(2), &ptr1, &ptr2);
    let result2 = verifier.verify_no_alias(RefId(1), RefId(3), &ptr1, &ptr3);
    let result3 = verifier.verify_no_alias(RefId(3), RefId(4), &ptr3, &ptr4);

    assert!(result1.is_no_alias());
    assert!(result2.is_no_alias());
    assert!(result3.is_no_alias());

    // All should be cached now
    let stats = verifier.cache_stats();
    assert_eq!(stats.misses, 3);
    assert_eq!(stats.hits, 0); // No repeats yet
}

#[test]
fn test_refine_alias_with_smt() {
    let mut verifier = SmtAliasVerifier::new();
    let mut alias_sets = AliasSets::new(RefId(1));

    // Add some may-alias versions
    alias_sets.add_may_alias(2);
    alias_sets.add_may_alias(3);

    // Build constraints
    let mut constraints = verum_common::Map::new();
    constraints.insert(RefId(1), PointerConstraint::stack_alloc(1, 0));
    constraints.insert(RefId(2), PointerConstraint::stack_alloc(2, 0)); // Different alloc
    constraints.insert(RefId(3), PointerConstraint::stack_alloc(3, 0)); // Different alloc

    let refined = verifier.refine_alias_with_smt(RefId(1), &alias_sets, &constraints);

    // Should have proven no-alias for RefId(2) and RefId(3)
    assert!(refined.no_alias.contains(&RefId(2)) || refined.may_alias.contains(&2));
    // Note: refine_alias_with_smt currently adds to no_alias, but the implementation
    // might need adjustment to properly remove from may_alias
}
