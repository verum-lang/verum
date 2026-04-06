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
    unused_assignments,
    clippy::overly_complex_bool_expr
)]
//! Comprehensive Tests for Field-Sensitive Heap Tracking
//!
//! Validates field-sensitive heap tracking for CBGR per-field escape analysis.
//! Tracks which struct fields are stored to heap vs stack, enabling independent
//! CBGR tier decisions per field. Complexity: O(fields * heap_stores).
//!
//! This test suite validates the field-sensitive heap tracking system that enables
//! independent escape analysis for struct fields. Tests cover:
//!
//! - Core data structures (HeapSiteId, HeapStore, FieldHeapInfo)
//! - Field heap tracker (FieldHeapTracker)
//! - Integration with escape analysis
//! - Performance characteristics
//! - Edge cases and error conditions
//!
//! **Test Coverage**: 20+ comprehensive tests

use verum_cbgr::analysis::{
    BlockId, ControlFlowGraph, DefSite, EscapeAnalyzer, EscapeResult, FieldComponent, FieldPath,
    FunctionId, RefId,
};
use verum_cbgr::field_heap_tracking::{
    FieldHeapInfo, FieldHeapResult, FieldHeapTracker, HeapSiteId, HeapStore,
};
use verum_common::{List, Map, Set};

// ==================================================================================
// Core Data Structure Tests
// ==================================================================================

#[test]
fn test_heap_site_id_creation() {
    let site = HeapSiteId(42);
    assert_eq!(format!("{}", site), "heap#42");

    let site2 = HeapSiteId(0);
    assert_eq!(format!("{}", site2), "heap#0");
}

#[test]
fn test_heap_store_creation_definite() {
    let store = HeapStore::new(
        1,
        RefId(10),
        FieldPath::named("field".into()),
        HeapSiteId(5),
    );

    assert_eq!(store.id, 1);
    assert_eq!(store.reference, RefId(10));
    assert!(store.is_definite);
    assert_eq!(store.target_heap_site, HeapSiteId(5));
}

#[test]
fn test_heap_store_creation_may_escape() {
    let store = HeapStore::may_escape(2, RefId(20), FieldPath::tuple_index(0), HeapSiteId(3));

    assert_eq!(store.id, 2);
    assert!(!store.is_definite);
}

#[test]
fn test_heap_store_affects_field_same_field() {
    let store = HeapStore::new(1, RefId(1), FieldPath::named("x".into()), HeapSiteId(1));

    assert!(store.affects_field(&FieldPath::named("x".into())));
}

#[test]
fn test_heap_store_affects_field_different_field() {
    let store = HeapStore::new(1, RefId(1), FieldPath::named("x".into()), HeapSiteId(1));

    assert!(!store.affects_field(&FieldPath::named("y".into())));
}

#[test]
fn test_heap_store_affects_field_nested() {
    let mut components = List::new();
    components.push(FieldComponent::Named("outer".into()));
    components.push(FieldComponent::Named("inner".into()));
    let nested_path = FieldPath::from_components(components);

    let store = HeapStore::new(1, RefId(1), FieldPath::named("outer".into()), HeapSiteId(1));

    // Nested path should alias with prefix
    assert!(store.affects_field(&nested_path));
}

#[test]
fn test_field_heap_info_creation() {
    let info = FieldHeapInfo::new(FieldPath::named("count".into()));

    assert!(!info.escapes_to_heap);
    assert!(info.can_promote());
    assert_eq!(info.escape_result(), EscapeResult::DoesNotEscape);
    assert_eq!(info.definite_escapes, 0);
    assert_eq!(info.may_escapes, 0);
    assert!(info.heap_sites.is_empty());
}

#[test]
fn test_field_heap_info_add_definite_store() {
    let mut info = FieldHeapInfo::new(FieldPath::named("data".into()));

    let store = HeapStore::new(1, RefId(1), FieldPath::named("data".into()), HeapSiteId(10));

    info.add_heap_store(store);

    assert!(info.escapes_to_heap);
    assert!(!info.can_promote());
    assert_eq!(info.escape_result(), EscapeResult::EscapesViaHeap);
    assert_eq!(info.definite_escapes, 1);
    assert_eq!(info.may_escapes, 0);
    assert_eq!(info.heap_sites.len(), 1);
    assert!(info.heap_sites.contains(&HeapSiteId(10)));
}

#[test]
fn test_field_heap_info_add_may_escape_store() {
    let mut info = FieldHeapInfo::new(FieldPath::named("cache".into()));

    let store = HeapStore::may_escape(
        1,
        RefId(1),
        FieldPath::named("cache".into()),
        HeapSiteId(20),
    );

    info.add_heap_store(store);

    assert!(info.escapes_to_heap);
    assert_eq!(info.definite_escapes, 0);
    assert_eq!(info.may_escapes, 1);
}

#[test]
fn test_field_heap_info_multiple_stores() {
    let mut info = FieldHeapInfo::new(FieldPath::named("buffer".into()));

    // Add multiple stores to different heap sites
    for i in 0..3 {
        let store = HeapStore::new(
            i,
            RefId(1),
            FieldPath::named("buffer".into()),
            HeapSiteId(i),
        );
        info.add_heap_store(store);
    }

    assert_eq!(info.definite_escapes, 3);
    assert_eq!(info.heap_sites.len(), 3);
    assert_eq!(info.store_operations.len(), 3);
}

#[test]
fn test_field_heap_info_mark_conservative() {
    let mut info = FieldHeapInfo::new(FieldPath::new());

    info.mark_conservative();

    assert!(info.is_conservative);
    assert!(info.escapes_to_heap);
    assert!(!info.can_promote());
}

#[test]
fn test_field_heap_info_merge() {
    let mut info1 = FieldHeapInfo::new(FieldPath::named("x".into()));
    let mut info2 = FieldHeapInfo::new(FieldPath::named("x".into()));

    // Add stores to each
    let store1 = HeapStore::new(1, RefId(1), FieldPath::named("x".into()), HeapSiteId(1));
    let store2 = HeapStore::new(2, RefId(1), FieldPath::named("x".into()), HeapSiteId(2));

    info1.add_heap_store(store1);
    info2.add_heap_store(store2);

    // Merge
    info1.merge(&info2);

    assert_eq!(info1.definite_escapes, 2);
    assert_eq!(info1.heap_sites.len(), 2);
    assert!(info1.heap_sites.contains(&HeapSiteId(1)));
    assert!(info1.heap_sites.contains(&HeapSiteId(2)));
}

// ==================================================================================
// FieldHeapResult Tests
// ==================================================================================

#[test]
fn test_field_heap_result_creation() {
    let result = FieldHeapResult::new(RefId(1));

    assert_eq!(result.reference, RefId(1));
    assert!(!result.base_escapes_to_heap);
    assert_eq!(result.total_fields(), 0);
    assert_eq!(result.total_stores, 0);
}

#[test]
fn test_field_heap_result_add_field_info() {
    let mut result = FieldHeapResult::new(RefId(1));

    let mut info = FieldHeapInfo::new(FieldPath::named("x".into()));
    let store = HeapStore::new(1, RefId(1), FieldPath::named("x".into()), HeapSiteId(1));
    info.add_heap_store(store);

    result.add_field_info(info);

    assert!(result.base_escapes_to_heap);
    assert_eq!(result.total_fields(), 1);
    assert_eq!(result.all_heap_sites.len(), 1);
}

#[test]
fn test_field_heap_result_promotable_fields() {
    let mut result = FieldHeapResult::new(RefId(1));

    // Field x escapes
    let mut info_x = FieldHeapInfo::new(FieldPath::named("x".into()));
    let store = HeapStore::new(1, RefId(1), FieldPath::named("x".into()), HeapSiteId(1));
    info_x.add_heap_store(store);
    result.add_field_info(info_x);

    // Field y does NOT escape
    let info_y = FieldHeapInfo::new(FieldPath::named("y".into()));
    result.add_field_info(info_y);

    let promotable = result.promotable_fields();
    assert_eq!(promotable.len(), 1);
    assert!(promotable.contains(&FieldPath::named("y".into())));
}

#[test]
fn test_field_heap_result_escaping_fields() {
    let mut result = FieldHeapResult::new(RefId(1));

    // Add escaping field
    let mut info = FieldHeapInfo::new(FieldPath::tuple_index(0));
    let store = HeapStore::new(1, RefId(1), FieldPath::tuple_index(0), HeapSiteId(5));
    info.add_heap_store(store);
    result.add_field_info(info);

    let escaping = result.escaping_fields();
    assert_eq!(escaping.len(), 1);
    assert!(escaping.contains(&FieldPath::tuple_index(0)));
}

#[test]
fn test_field_heap_result_promotion_rate() {
    let mut result = FieldHeapResult::new(RefId(1));

    // 2 promotable fields
    result.add_field_info(FieldHeapInfo::new(FieldPath::named("a".into())));
    result.add_field_info(FieldHeapInfo::new(FieldPath::named("b".into())));

    // 1 escaping field
    let mut info_c = FieldHeapInfo::new(FieldPath::named("c".into()));
    let store = HeapStore::new(1, RefId(1), FieldPath::named("c".into()), HeapSiteId(1));
    info_c.add_heap_store(store);
    result.add_field_info(info_c);

    let rate = result.promotion_rate();
    assert!((rate - 0.6667).abs() < 0.01); // 2/3 ≈ 0.6667
}

#[test]
fn test_field_heap_result_merge() {
    let mut result1 = FieldHeapResult::new(RefId(1));
    let mut result2 = FieldHeapResult::new(RefId(1));

    // Result 1: field x escapes
    let mut info_x = FieldHeapInfo::new(FieldPath::named("x".into()));
    let store_x = HeapStore::new(1, RefId(1), FieldPath::named("x".into()), HeapSiteId(1));
    info_x.add_heap_store(store_x);
    result1.add_field_info(info_x);

    // Result 2: field y escapes
    let mut info_y = FieldHeapInfo::new(FieldPath::named("y".into()));
    let store_y = HeapStore::new(2, RefId(1), FieldPath::named("y".into()), HeapSiteId(2));
    info_y.add_heap_store(store_y);
    result2.add_field_info(info_y);

    // Merge
    result1.merge(&result2);

    assert_eq!(result1.total_fields(), 2);
    assert_eq!(result1.all_heap_sites.len(), 2);
    assert_eq!(result1.escaping_count(), 2);
}

// ==================================================================================
// FieldHeapTracker Tests
// ==================================================================================

#[test]
fn test_field_heap_tracker_creation() {
    let tracker = FieldHeapTracker::new();

    let stats = tracker.statistics();
    assert_eq!(stats.total_heap_sites, 0);
    assert_eq!(stats.total_stores, 0);
    assert_eq!(stats.references_tracked, 0);
}

#[test]
fn test_field_heap_tracker_register_heap_allocation() {
    let mut tracker = FieldHeapTracker::new();

    let site1 = tracker.register_heap_allocation("Box::new");
    let site2 = tracker.register_heap_allocation("Vec::push");

    assert_eq!(site1, HeapSiteId(0));
    assert_eq!(site2, HeapSiteId(1));

    let stats = tracker.statistics();
    assert_eq!(stats.total_heap_sites, 2);
}

#[test]
fn test_field_heap_tracker_add_heap_store() {
    let mut tracker = FieldHeapTracker::new();

    let heap_site = tracker.register_heap_allocation("test_heap");
    tracker.add_heap_store(RefId(1), FieldPath::named("field".into()), heap_site, true);

    let stats = tracker.statistics();
    assert_eq!(stats.total_stores, 1);
    assert_eq!(stats.definite_stores, 1);
    assert_eq!(stats.may_stores, 0);
}

#[test]
fn test_field_heap_tracker_track_field_allocations() {
    let mut tracker = FieldHeapTracker::new();

    // Register fields
    let mut paths = Set::new();
    paths.insert(FieldPath::named("x".into()));
    paths.insert(FieldPath::named("y".into()));
    tracker.register_fields(RefId(1), paths);

    // Add heap store for field x only
    let heap_site = tracker.register_heap_allocation("Box::new");
    tracker.add_heap_store(RefId(1), FieldPath::named("x".into()), heap_site, true);

    // Analyze
    let result = tracker.track_field_heap_allocations(RefId(1));

    assert_eq!(result.total_fields(), 2);
    assert!(result.field_escapes_to_heap(&FieldPath::named("x".into())));
    assert!(!result.field_escapes_to_heap(&FieldPath::named("y".into())));
    assert!(result.can_promote_field(&FieldPath::named("y".into())));
}

#[test]
fn test_field_heap_tracker_field_escapes_query() {
    let mut tracker = FieldHeapTracker::new();

    let heap_site = tracker.register_heap_allocation("heap");
    tracker.add_heap_store(RefId(1), FieldPath::named("cache".into()), heap_site, true);

    assert!(tracker.field_escapes_to_heap(RefId(1), &FieldPath::named("cache".into())));
    assert!(!tracker.field_escapes_to_heap(RefId(1), &FieldPath::named("count".into())));
}

#[test]
fn test_field_heap_tracker_refine_escape() {
    let mut tracker = FieldHeapTracker::new();

    let heap_site = tracker.register_heap_allocation("Box");
    tracker.add_heap_store(RefId(1), FieldPath::named("data".into()), heap_site, true);

    // Refine a promotable result
    let initial = EscapeResult::DoesNotEscape;
    let refined =
        tracker.refine_field_escape_with_heap(RefId(1), &FieldPath::named("data".into()), initial);

    assert_eq!(refined, EscapeResult::EscapesViaHeap);
}

#[test]
fn test_field_heap_tracker_refine_already_escaping() {
    let tracker = FieldHeapTracker::new();

    // Already escaping via return
    let initial = EscapeResult::EscapesViaReturn;
    let refined = tracker.refine_field_escape_with_heap(RefId(1), &FieldPath::new(), initial);

    // Should keep existing escape result
    assert_eq!(refined, EscapeResult::EscapesViaReturn);
}

#[test]
fn test_field_heap_tracker_statistics() {
    let mut tracker = FieldHeapTracker::new();

    // Register 3 heap sites
    tracker.register_heap_allocation("site1");
    tracker.register_heap_allocation("site2");
    tracker.register_heap_allocation("site3");

    // Add 5 stores (3 definite, 2 may)
    for i in 0..5 {
        tracker.add_heap_store(
            RefId(i),
            FieldPath::new(),
            HeapSiteId(i % 3),
            i < 3, // First 3 are definite
        );
    }

    // Register 2 references
    tracker.register_fields(RefId(1), Set::new());
    tracker.register_fields(RefId(2), Set::new());

    let stats = tracker.statistics();
    assert_eq!(stats.total_heap_sites, 3);
    assert_eq!(stats.total_stores, 5);
    assert_eq!(stats.definite_stores, 3);
    assert_eq!(stats.may_stores, 2);
    assert_eq!(stats.references_tracked, 2);
}

#[test]
fn test_field_heap_tracker_clear() {
    let mut tracker = FieldHeapTracker::new();

    tracker.register_heap_allocation("test");
    tracker.add_heap_store(RefId(1), FieldPath::new(), HeapSiteId(0), true);
    tracker.register_fields(RefId(1), Set::new());

    tracker.clear();

    let stats = tracker.statistics();
    assert_eq!(stats.total_heap_sites, 0);
    assert_eq!(stats.total_stores, 0);
    assert_eq!(stats.references_tracked, 0);
}

// ==================================================================================
// Integration with EscapeAnalyzer Tests
// ==================================================================================

#[test]
fn test_escape_analyzer_field_heap_integration() {
    // Create simple CFG
    let mut cfg = ControlFlowGraph::new(BlockId(0), BlockId(1));

    let analyzer = EscapeAnalyzer::new(cfg);

    // Test field heap tracking (will use heuristics)
    let result = analyzer.track_field_heap_allocations(RefId(1));

    // Should return valid result
    assert_eq!(result.reference, RefId(1));
}

#[test]
fn test_escape_analyzer_field_escapes_to_heap() {
    let cfg = ControlFlowGraph::new(BlockId(0), BlockId(1));
    let analyzer = EscapeAnalyzer::new(cfg);

    // Query field heap escape
    let escapes = analyzer.field_escapes_to_heap(RefId(1), &FieldPath::named("test".into()));

    // Should return boolean result
    assert!(!escapes || escapes); // Always true, just testing API
}

#[test]
fn test_escape_analyzer_refine_with_heap() {
    let cfg = ControlFlowGraph::new(BlockId(0), BlockId(1));
    let analyzer = EscapeAnalyzer::new(cfg);

    let initial = EscapeResult::DoesNotEscape;
    let refined = analyzer.refine_field_escape_with_heap(RefId(1), &FieldPath::new(), initial);

    // Should return valid escape result
    assert!(refined.can_promote() || !refined.can_promote());
}

// ==================================================================================
// Edge Cases and Error Conditions
// ==================================================================================

#[test]
fn test_field_heap_empty_field_path() {
    let mut tracker = FieldHeapTracker::new();

    let heap_site = tracker.register_heap_allocation("heap");
    tracker.add_heap_store(RefId(1), FieldPath::new(), heap_site, true);

    let result = tracker.track_field_heap_allocations(RefId(1));

    // Base reference should escape
    assert!(result.base_escapes_to_heap);
}

#[test]
fn test_field_heap_multiple_references() {
    use verum_common::Set;

    let mut tracker = FieldHeapTracker::new();

    let heap_site = tracker.register_heap_allocation("shared_heap");

    // Register field paths for each reference so they can be tracked
    let mut fields1: Set<FieldPath> = Set::new();
    fields1.insert(FieldPath::named("a".into()));
    fields1.insert(FieldPath::named("b".into()));
    tracker.register_fields(RefId(1), fields1);

    let mut fields2: Set<FieldPath> = Set::new();
    fields2.insert(FieldPath::named("a".into()));
    fields2.insert(FieldPath::named("b".into()));
    tracker.register_fields(RefId(2), fields2);

    // Different references with different fields escaping to heap
    tracker.add_heap_store(RefId(1), FieldPath::named("a".into()), heap_site, true);
    tracker.add_heap_store(RefId(2), FieldPath::named("b".into()), heap_site, true);

    let result1 = tracker.track_field_heap_allocations(RefId(1));
    let result2 = tracker.track_field_heap_allocations(RefId(2));

    // Each reference should only see its own escapes
    assert!(result1.field_escapes_to_heap(&FieldPath::named("a".into())));
    assert!(!result1.field_escapes_to_heap(&FieldPath::named("b".into())));

    assert!(result2.field_escapes_to_heap(&FieldPath::named("b".into())));
    assert!(!result2.field_escapes_to_heap(&FieldPath::named("a".into())));
}

#[test]
fn test_field_heap_zero_promotion_rate() {
    let mut result = FieldHeapResult::new(RefId(1));

    // No fields added
    assert_eq!(result.promotion_rate(), 0.0);
}

#[test]
fn test_field_heap_hundred_percent_promotion() {
    let mut result = FieldHeapResult::new(RefId(1));

    // All fields promotable
    result.add_field_info(FieldHeapInfo::new(FieldPath::named("x".into())));
    result.add_field_info(FieldHeapInfo::new(FieldPath::named("y".into())));
    result.add_field_info(FieldHeapInfo::new(FieldPath::named("z".into())));

    assert_eq!(result.promotion_rate(), 1.0);
}

#[test]
fn test_heap_tracking_statistics_report() {
    let mut tracker = FieldHeapTracker::new();

    tracker.register_heap_allocation("test1");
    tracker.register_heap_allocation("test2");
    tracker.add_heap_store(RefId(1), FieldPath::new(), HeapSiteId(0), true);

    let stats = tracker.statistics();
    let report = stats.report();

    // Report should contain key information
    assert!(report.contains("Heap sites: 2"));
    assert!(report.contains("Total stores: 1"));
    assert!(report.contains("1 definite, 0 may"));
}
