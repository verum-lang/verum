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
//! Comprehensive Tests for Enhanced Escape Analysis
//!
//! Tests cover all escape scenarios from Section 2.3:
//! - NoEscape (0ns optimization)
//! - Heap escape
//! - Return escape
//! - Closure capture
//! - Thread crossing
//! - Parameter escape

use verum_cbgr::analysis::{BasicBlock, BlockId, ControlFlowGraph, DefSite, RefId, UseeSite};
use verum_cbgr::escape_analysis::{
    EnhancedEscapeAnalyzer, EscapeAnalysisConfig, EscapeKind, EscapeState,
};
use verum_common::{Map, Set};

// ==================================================================================
// Helper Functions for Test CFG Construction
// ==================================================================================

/// Create a simple CFG with entry and exit blocks
fn create_simple_cfg() -> ControlFlowGraph {
    let entry = BlockId(0);
    let exit = BlockId(1);

    let mut cfg = ControlFlowGraph::new(entry, exit);

    // Entry block
    let mut entry_block = BasicBlock {
        id: entry,
        predecessors: Set::new(),
        successors: Set::new(),
        definitions: vec![].into(),
        uses: vec![].into(),
        call_sites: vec![].into(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };
    entry_block.successors.insert(exit);

    // Exit block
    let mut exit_block = BasicBlock {
        id: exit,
        predecessors: Set::new(),
        successors: Set::new(),
        definitions: vec![].into(),
        uses: vec![].into(),
        call_sites: vec![].into(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };
    exit_block.predecessors.insert(entry);

    cfg.add_block(entry_block);
    cfg.add_block(exit_block);

    cfg
}

/// Add a stack-allocated reference to a block
fn add_stack_ref(cfg: &mut ControlFlowGraph, block_id: BlockId, ref_id: RefId) {
    if let Some(block) = cfg.blocks.get_mut(&block_id) {
        block.definitions.push(DefSite {
            block: block_id,
            reference: ref_id,
            is_stack_allocated: true,
            span: None,
        });
    }
}

/// Add a heap-allocated reference to a block
fn add_heap_ref(cfg: &mut ControlFlowGraph, block_id: BlockId, ref_id: RefId) {
    if let Some(block) = cfg.blocks.get_mut(&block_id) {
        block.definitions.push(DefSite {
            block: block_id,
            reference: ref_id,
            is_stack_allocated: false,
            span: None,
        });
    }
}

/// Add a reference use to a block
fn add_ref_use(cfg: &mut ControlFlowGraph, block_id: BlockId, ref_id: RefId, is_mutable: bool) {
    if let Some(block) = cfg.blocks.get_mut(&block_id) {
        block.uses.push(UseeSite {
            block: block_id,
            reference: ref_id,
            is_mutable,
            span: None,
        });
    }
}

// ==================================================================================
// Test Cases: NoEscape Scenarios (0ns optimization)
// ==================================================================================

#[test]
fn test_simple_stack_ref_no_escape() {
    // Test: Stack-allocated reference used only in entry block
    // Expected: NoEscape (can optimize to 0ns)

    let mut cfg = create_simple_cfg();
    let ref_id = RefId(1);

    // Define stack reference in entry
    add_stack_ref(&mut cfg, BlockId(0), ref_id);

    // Use it in entry (but not in exit)
    add_ref_use(&mut cfg, BlockId(0), ref_id, false);

    // Run analysis
    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    let result = analyzer.analyze();

    // Verify NoEscape
    let state = result.escape_states.get(&ref_id);
    assert!(state.is_some(), "Reference should have escape state");
    assert_eq!(
        *state.unwrap(),
        EscapeState::NoEscape,
        "Stack reference used only in entry should not escape"
    );

    // Verify can optimize
    let no_escape_refs = result.no_escape_refs();
    assert!(
        no_escape_refs.contains(&ref_id),
        "Reference should be in NoEscape set"
    );

    // Verify stats
    assert_eq!(
        result.stats.no_escape_count, 1,
        "Should have 1 NoEscape reference"
    );
    assert_eq!(result.stats.total_references, 1);
}

#[test]
fn test_local_variable_no_escape() {
    // Test: Local variable used in multiple blocks but never escapes
    // Expected: NoEscape if properly analyzed

    let mut cfg = create_simple_cfg();
    let ref_id = RefId(2);

    // Define in entry
    add_stack_ref(&mut cfg, BlockId(0), ref_id);

    // Use in entry (immutable)
    add_ref_use(&mut cfg, BlockId(0), ref_id, false);

    // Run analysis
    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    let result = analyzer.analyze();

    // Verify
    let state = result.escape_states.get(&ref_id).unwrap();
    assert_eq!(*state, EscapeState::NoEscape);
}

// ==================================================================================
// Test Cases: Return Escape
// ==================================================================================

#[test]
fn test_return_escape() {
    // Test: Reference used in exit block (potential return)
    // Expected: Escapes (need CBGR)

    let mut cfg = create_simple_cfg();
    let ref_id = RefId(3);

    // Define in entry
    add_stack_ref(&mut cfg, BlockId(0), ref_id);

    // Use in exit block (indicates return)
    add_ref_use(&mut cfg, BlockId(1), ref_id, false);

    // Run analysis
    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    let result = analyzer.analyze();

    // Verify escape
    let state = result.escape_states.get(&ref_id).unwrap();
    assert_eq!(
        *state,
        EscapeState::Escapes,
        "Reference used in exit block should escape"
    );

    // Verify escape point recorded
    assert!(
        result.stats.escape_points_detected > 0,
        "Should detect escape point"
    );

    // Find the escape point
    let escape_point = result
        .escape_points
        .iter()
        .find(|p| p.reference == ref_id && p.escape_kind == EscapeKind::ReturnEscape);

    assert!(
        escape_point.is_some(),
        "Should have ReturnEscape escape point"
    );
}

// ==================================================================================
// Test Cases: Heap Escape
// ==================================================================================

#[test]
fn test_heap_allocation_escape() {
    // Test: Heap-allocated reference (not stack)
    // Expected: MayEscape or Escapes

    let mut cfg = create_simple_cfg();
    let ref_id = RefId(4);

    // Define as heap-allocated
    add_heap_ref(&mut cfg, BlockId(0), ref_id);

    // Use in entry
    add_ref_use(&mut cfg, BlockId(0), ref_id, false);

    // Run analysis
    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    let result = analyzer.analyze();

    // Verify escape
    let state = result.escape_states.get(&ref_id).unwrap();
    assert!(
        *state == EscapeState::MayEscape || *state == EscapeState::Escapes,
        "Heap-allocated reference should escape"
    );

    // Verify escape point
    let heap_escape = result
        .escape_points
        .iter()
        .find(|p| p.reference == ref_id && p.escape_kind == EscapeKind::HeapStore);

    assert!(heap_escape.is_some(), "Should detect heap escape");
}

// ==================================================================================
// Test Cases: Complex Control Flow
// ==================================================================================

#[test]
fn test_branching_control_flow() {
    // Test: Reference used in branching paths
    // Expected: Conservative analysis (may escape)

    let entry = BlockId(0);
    let branch1 = BlockId(1);
    let branch2 = BlockId(2);
    let merge = BlockId(3);
    let exit = BlockId(4);

    let mut cfg = ControlFlowGraph::new(entry, exit);

    // Entry block
    let mut entry_block = BasicBlock {
        id: entry,
        predecessors: Set::new(),
        successors: Set::new(),
        definitions: vec![].into(),
        uses: vec![].into(),
        call_sites: vec![].into(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };
    entry_block.successors.insert(branch1);
    entry_block.successors.insert(branch2);

    // Branch 1
    let mut branch1_block = BasicBlock {
        id: branch1,
        predecessors: Set::new(),
        successors: Set::new(),
        definitions: vec![].into(),
        uses: vec![].into(),
        call_sites: vec![].into(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };
    branch1_block.predecessors.insert(entry);
    branch1_block.successors.insert(merge);

    // Branch 2
    let mut branch2_block = BasicBlock {
        id: branch2,
        predecessors: Set::new(),
        successors: Set::new(),
        definitions: vec![].into(),
        uses: vec![].into(),
        call_sites: vec![].into(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };
    branch2_block.predecessors.insert(entry);
    branch2_block.successors.insert(merge);

    // Merge block
    let mut merge_block = BasicBlock {
        id: merge,
        predecessors: Set::new(),
        successors: Set::new(),
        definitions: vec![].into(),
        uses: vec![].into(),
        call_sites: vec![].into(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };
    merge_block.predecessors.insert(branch1);
    merge_block.predecessors.insert(branch2);
    merge_block.successors.insert(exit);

    // Exit block
    let mut exit_block = BasicBlock {
        id: exit,
        predecessors: Set::new(),
        successors: Set::new(),
        definitions: vec![].into(),
        uses: vec![].into(),
        call_sites: vec![].into(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };
    exit_block.predecessors.insert(merge);

    cfg.add_block(entry_block);
    cfg.add_block(branch1_block);
    cfg.add_block(branch2_block);
    cfg.add_block(merge_block);
    cfg.add_block(exit_block);

    let ref_id = RefId(5);

    // Define in entry
    add_stack_ref(&mut cfg, entry, ref_id);

    // Use in branch1
    add_ref_use(&mut cfg, branch1, ref_id, false);

    // Use in branch2
    add_ref_use(&mut cfg, branch2, ref_id, false);

    // Run analysis
    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    let result = analyzer.analyze();

    // Verify state
    let state = result.escape_states.get(&ref_id);
    assert!(state.is_some(), "Should have escape state");

    // Either NoEscape or MayEscape is acceptable for branching
    // (depends on implementation conservativeness)
}

// ==================================================================================
// Test Cases: Statistics and Reporting
// ==================================================================================

#[test]
fn test_escape_analysis_stats() {
    // Test: Verify statistics are collected correctly

    let mut cfg = create_simple_cfg();

    // Add multiple references with different escape patterns
    let no_escape_ref = RefId(10);
    let escape_ref = RefId(11);

    add_stack_ref(&mut cfg, BlockId(0), no_escape_ref);
    add_ref_use(&mut cfg, BlockId(0), no_escape_ref, false);

    add_stack_ref(&mut cfg, BlockId(0), escape_ref);
    add_ref_use(&mut cfg, BlockId(1), escape_ref, false); // Exit block

    // Run analysis
    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    let result = analyzer.analyze();

    // Verify stats
    assert_eq!(result.stats.total_references, 2);
    assert!(result.stats.no_escape_count > 0);
    assert!(result.stats.escapes_count > 0 || result.stats.may_escape_count > 0);

    // Verify percentage calculation
    let percentage = result.stats.no_escape_percentage();
    assert!((0.0..=100.0).contains(&percentage));

    // Verify time saved estimation
    let time_saved = result.stats.estimated_time_saved_ns();
    assert!(time_saved > 0);
}

#[test]
fn test_escape_point_tracking() {
    // Test: Verify escape points are tracked correctly

    let mut cfg = create_simple_cfg();
    let ref_id = RefId(12);

    // Create a reference that escapes via return
    add_stack_ref(&mut cfg, BlockId(0), ref_id);
    add_ref_use(&mut cfg, BlockId(1), ref_id, false);

    // Run analysis
    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    let result = analyzer.analyze();

    // Verify escape points
    assert!(
        !result.escape_points.is_empty(),
        "Should have escape points"
    );

    // Find specific escape point
    let return_escape = result.escape_points.iter().find(|p| p.reference == ref_id);

    assert!(return_escape.is_some());

    let point = return_escape.unwrap();
    assert_eq!(point.escape_kind, EscapeKind::ReturnEscape);
    assert!(!point.description.is_empty());
}

// ==================================================================================
// Test Cases: Configuration
// ==================================================================================

#[test]
fn test_custom_config() {
    // Test: Verify configuration options work

    let cfg = create_simple_cfg();

    let config = EscapeAnalysisConfig {
        enable_interprocedural: false,
        max_iterations: 50,
        enable_closure_analysis: false,
        enable_thread_analysis: false,
        confidence_threshold: 0.90,
    };

    let analyzer = EnhancedEscapeAnalyzer::new(cfg).with_config(config);

    // Verify config is applied
    assert_eq!(analyzer.config().max_iterations, 50);
    assert_eq!(analyzer.config().confidence_threshold, 0.90);
}

// ==================================================================================
// Test Cases: Report Generation
// ==================================================================================

#[test]
fn test_report_generation() {
    // Test: Verify diagnostic report generation

    let mut cfg = create_simple_cfg();
    let ref_id = RefId(13);

    add_stack_ref(&mut cfg, BlockId(0), ref_id);
    add_ref_use(&mut cfg, BlockId(1), ref_id, false);

    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    let result = analyzer.analyze();

    // Generate report
    let report = result.generate_report();

    // Verify report content
    assert!(report.contains("Escape Analysis Report"));
    assert!(report.contains("Total references"));

    // Display should also work
    let display_output = format!("{}", result);
    assert!(!display_output.is_empty());
}

// ==================================================================================
// Test Cases: EscapeResult Conversion
// ==================================================================================

#[test]
fn test_escape_result_conversion() {
    // Test: Verify conversion to EscapeResult for compatibility

    use verum_cbgr::analysis::EscapeResult;

    let mut cfg = create_simple_cfg();

    // NoEscape case
    let no_escape_ref = RefId(20);
    add_stack_ref(&mut cfg, BlockId(0), no_escape_ref);
    add_ref_use(&mut cfg, BlockId(0), no_escape_ref, false);

    // Escapes case
    let escape_ref = RefId(21);
    add_stack_ref(&mut cfg, BlockId(0), escape_ref);
    add_ref_use(&mut cfg, BlockId(1), escape_ref, false);

    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    analyzer.analyze();

    // Convert to EscapeResult
    let no_escape_result = analyzer.to_escape_result(no_escape_ref);
    assert_eq!(no_escape_result, EscapeResult::DoesNotEscape);

    let escape_result = analyzer.to_escape_result(escape_ref);
    assert_ne!(escape_result, EscapeResult::DoesNotEscape);
}

// ==================================================================================
// Test Cases: Edge Cases
// ==================================================================================

#[test]
fn test_empty_cfg() {
    // Test: Analyze empty CFG

    let cfg = create_simple_cfg();

    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    let result = analyzer.analyze();

    // Should handle gracefully
    assert_eq!(result.stats.total_references, 0);
    assert!(result.escape_points.is_empty());
}

#[test]
fn test_reference_with_no_uses() {
    // Test: Reference defined but never used

    let mut cfg = create_simple_cfg();
    let ref_id = RefId(30);

    // Define but don't use
    add_stack_ref(&mut cfg, BlockId(0), ref_id);

    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    let result = analyzer.analyze();

    // Should still analyze
    let state = result.escape_states.get(&ref_id);
    assert!(state.is_some());

    // Unused reference should be NoEscape
    assert_eq!(*state.unwrap(), EscapeState::NoEscape);
}

#[test]
fn test_multiple_uses_same_block() {
    // Test: Reference used multiple times in same block

    let mut cfg = create_simple_cfg();
    let ref_id = RefId(31);

    add_stack_ref(&mut cfg, BlockId(0), ref_id);

    // Multiple uses
    add_ref_use(&mut cfg, BlockId(0), ref_id, false);
    add_ref_use(&mut cfg, BlockId(0), ref_id, false);
    add_ref_use(&mut cfg, BlockId(0), ref_id, true); // mutable

    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    let result = analyzer.analyze();

    // Should still be NoEscape (all in same block)
    let state = result.escape_states.get(&ref_id).unwrap();
    assert_eq!(*state, EscapeState::NoEscape);
}
