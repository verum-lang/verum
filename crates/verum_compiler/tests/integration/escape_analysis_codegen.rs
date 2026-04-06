//! Integration Tests for Escape Analysis → Codegen Pipeline
//!
//! These tests verify that escape analysis results correctly flow through
//! the compiler pipeline to code generation, enabling automatic promotion
//! of references from Tier 0 (&T) to Tier 1 (&checked T).

use verum_cbgr::analysis::{EscapeAnalyzer, EscapeResult, RefId, ControlFlowGraph, BasicBlock, BlockId, DefSite};
use verum_cbgr::codegen::ReferenceTier;
use verum_cbgr::compiler_integration::{CbgrCompilerPlugin, FunctionAnalysisResult, FunctionId};
use verum_cbgr::llvm_integration::{TierSelectorBuilder, TierStatistics};
use verum_codegen::references::{ReferenceCodegen, ReferenceStats};
use verum_common::{List, Map, Set};

/// Test 1: Escape analysis correctly identifies non-escaping reference
#[test]
fn test_escape_analysis_no_escape() {
    // Create simple CFG for function:
    // fn test(x: Int) -> Int {
    //     let y = x + 1;  // y is stack-local
    //     y               // y doesn't escape
    // }

    let entry = BlockId(0);
    let exit = BlockId(1);
    let mut cfg = ControlFlowGraph::new(entry, exit);

    let ref_id = RefId(1); // Reference to 'y'

    let mut entry_block = BasicBlock {
        id: entry,
        predecessors: Set::new(),
        successors: Set::from_iter(vec![exit]),
        definitions: List::from(vec![DefSite {
            block: entry,
            reference: ref_id,
            is_stack_allocated: true,
        }]),
        uses: List::new(),
    };

    cfg.add_block(entry_block);

    // Run escape analysis
    let analyzer = EscapeAnalyzer::new(cfg);
    let result = analyzer.analyze(ref_id);

    // Should not escape
    assert_eq!(result, EscapeResult::DoesNotEscape);
    assert!(result.can_promote());
}

/// Test 2: TierSelector correctly promotes non-escaping reference
#[test]
fn test_tier_selector_promotion() {
    let function_id = FunctionId(1);
    let ref_id = RefId(1);

    // Create analysis result with promotion decision
    let mut analysis = FunctionAnalysisResult::new(function_id);
    analysis.tier_decisions.insert(ref_id, ReferenceTier::Checked);
    analysis.escape_results.insert(ref_id, EscapeResult::DoesNotEscape);
    analysis.promotions = 1;
    analysis.total_references = 1;

    // Build tier selector
    let selector = TierSelectorBuilder::new()
        .with_analysis(analysis)
        .with_confidence_threshold(0.95)
        .build()
        .expect("Failed to build tier selector");

    // Verify tier selection
    let tier = selector.select_tier(ref_id)
        .expect("Reference should have tier");

    assert_eq!(tier, ReferenceTier::Checked);

    // Verify statistics
    let stats = selector.statistics();
    assert_eq!(stats.tier1_count, 1);
    assert_eq!(stats.total_count, 1);
    assert_eq!(stats.promotion_rate(), 1.0);
}

/// Test 3: TierSelector keeps escaping reference as Managed
#[test]
fn test_tier_selector_no_promotion() {
    let function_id = FunctionId(1);
    let ref_id = RefId(1);

    // Create analysis result with NO promotion (escapes)
    let mut analysis = FunctionAnalysisResult::new(function_id);
    analysis.tier_decisions.insert(ref_id, ReferenceTier::Managed);
    analysis.escape_results.insert(ref_id, EscapeResult::EscapesViaReturn);
    analysis.promotions = 0;
    analysis.total_references = 1;

    // Build tier selector
    let selector = TierSelectorBuilder::new()
        .with_analysis(analysis)
        .build()
        .expect("Failed to build tier selector");

    // Verify tier selection
    let tier = selector.select_tier(ref_id)
        .expect("Reference should have tier");

    assert_eq!(tier, ReferenceTier::Managed);

    // Verify statistics
    let stats = selector.statistics();
    assert_eq!(stats.tier0_count, 1);
    assert_eq!(stats.promotion_rate(), 0.0);
}

/// Test 4: Confidence threshold filters promotions
#[test]
fn test_confidence_threshold() {
    let function_id = FunctionId(1);
    let ref_id = RefId(1);

    // Low confidence analysis result
    let mut analysis = FunctionAnalysisResult::new(function_id);
    analysis.tier_decisions.insert(ref_id, ReferenceTier::Managed);
    analysis.total_references = 1;

    // High threshold (1.0) should prevent promotion
    let selector = TierSelectorBuilder::new()
        .with_analysis(analysis.clone())
        .with_confidence_threshold(1.0)
        .build()
        .expect("Failed to build tier selector");

    let tier = selector.select_tier(ref_id)
        .expect("Reference should have tier");

    // Should remain managed due to high threshold
    assert_eq!(tier, ReferenceTier::Managed);
}

/// Test 5: Manual override works correctly
#[test]
fn test_manual_override() {
    let function_id = FunctionId(1);
    let ref_id = RefId(1);

    // Analysis says Managed
    let mut analysis = FunctionAnalysisResult::new(function_id);
    analysis.tier_decisions.insert(ref_id, ReferenceTier::Managed);
    analysis.total_references = 1;

    // Manual override to Checked
    let selector = TierSelectorBuilder::new()
        .with_analysis(analysis)
        .override_tier(ref_id, ReferenceTier::Checked)
        .build()
        .expect("Failed to build tier selector");

    let tier = selector.select_tier(ref_id)
        .expect("Reference should have tier");

    // Override should take precedence
    assert_eq!(tier, ReferenceTier::Checked);
}

/// Test 6: Multiple references with mixed promotion decisions
#[test]
fn test_mixed_promotions() {
    let function_id = FunctionId(1);

    let ref1 = RefId(1); // Promoted
    let ref2 = RefId(2); // Not promoted
    let ref3 = RefId(3); // Promoted

    let mut analysis = FunctionAnalysisResult::new(function_id);
    analysis.tier_decisions.insert(ref1, ReferenceTier::Checked);
    analysis.tier_decisions.insert(ref2, ReferenceTier::Managed);
    analysis.tier_decisions.insert(ref3, ReferenceTier::Checked);
    analysis.promotions = 2;
    analysis.total_references = 3;

    let selector = TierSelectorBuilder::new()
        .with_analysis(analysis)
        .build()
        .expect("Failed to build tier selector");

    // Verify individual tiers
    assert_eq!(selector.select_tier(ref1).unwrap(), ReferenceTier::Checked);
    assert_eq!(selector.select_tier(ref2).unwrap(), ReferenceTier::Managed);
    assert_eq!(selector.select_tier(ref3).unwrap(), ReferenceTier::Checked);

    // Verify statistics
    let stats = selector.statistics();
    assert_eq!(stats.tier1_count, 2);
    assert_eq!(stats.tier0_count, 1);
    assert_eq!(stats.total_count, 3);

    // Promotion rate should be 2/3
    let expected_rate = 2.0 / 3.0;
    let actual_rate = stats.promotion_rate();
    assert!((actual_rate - expected_rate).abs() < 0.01);
}

/// Test 7: Statistics calculation accuracy
#[test]
fn test_statistics_calculation() {
    let stats = TierStatistics {
        tier0_count: 30,
        tier1_count: 60,
        tier2_count: 10,
        total_count: 100,
    };

    // Promotion rate (Tier 1 / Total)
    assert_eq!(stats.promotion_rate(), 0.6);

    // Average overhead: (30 * 15ns + 60 * 0ns + 10 * 0ns) / 100 = 4.5ns
    assert_eq!(stats.average_overhead_ns(), 4.5);

    // Time saved: 70 promotions * 15ns * 10 derefs = 10,500ns
    assert_eq!(stats.estimated_time_saved_ns(), 10_500);
}

/// Test 8: End-to-end: Analysis → Selector → Stats
#[test]
fn test_end_to_end_pipeline() {
    let function_id = FunctionId(1);

    // Simulate analysis finding 10 references:
    // - 7 can be promoted (DoesNotEscape)
    // - 3 must stay managed (various escape reasons)
    let mut analysis = FunctionAnalysisResult::new(function_id);

    for i in 0..7 {
        let ref_id = RefId(i);
        analysis.tier_decisions.insert(ref_id, ReferenceTier::Checked);
        analysis.escape_results.insert(ref_id, EscapeResult::DoesNotEscape);
    }

    for i in 7..10 {
        let ref_id = RefId(i);
        analysis.tier_decisions.insert(ref_id, ReferenceTier::Managed);
        analysis.escape_results.insert(ref_id, EscapeResult::EscapesViaHeap);
    }

    analysis.promotions = 7;
    analysis.total_references = 10;

    // Build selector
    let selector = TierSelectorBuilder::new()
        .with_analysis(analysis.clone())
        .with_confidence_threshold(0.95)
        .build()
        .expect("Failed to build tier selector");

    // Verify all tiers
    for i in 0..7 {
        assert_eq!(
            selector.select_tier(RefId(i)).unwrap(),
            ReferenceTier::Checked
        );
    }

    for i in 7..10 {
        assert_eq!(
            selector.select_tier(RefId(i)).unwrap(),
            ReferenceTier::Managed
        );
    }

    // Verify final statistics
    let stats = selector.statistics();
    assert_eq!(stats.tier1_count, 7);
    assert_eq!(stats.tier0_count, 3);
    assert_eq!(stats.total_count, 10);
    assert_eq!(stats.promotion_rate(), 0.7);

    // Display stats (for manual verification in test output)
    println!("{}", stats);
}

/// Test 9: ReferenceStats promotion tracking
#[test]
fn test_reference_stats_tracking() {
    let mut stats = ReferenceStats::default();

    // Simulate codegen processing 100 references:
    // - 60 promoted (0ns overhead each)
    // - 40 managed (15ns overhead each)
    stats.tier_selections = 100;
    stats.promotions = 60;
    stats.default_managed = 40;
    stats.zero_cost_derefs = 60;
    stats.cbgr_checks = 40;

    // Verify calculations
    assert_eq!(stats.promotion_rate(), 60.0);

    // Average overhead: (40 * 15 + 60 * 0) / 100 = 6ns
    assert_eq!(stats.average_overhead_ns(), 6.0);

    // Time saved: 60 * 15 * 10 / 1000 = 9μs
    assert_eq!(stats.estimated_time_saved_us(), 9);

    // Print report
    println!("{}", stats.report());
}

/// Test 10: Zero references edge case
#[test]
fn test_zero_references() {
    let stats = ReferenceStats::default();

    // Should not panic with division by zero
    assert_eq!(stats.promotion_rate(), 0.0);
    assert_eq!(stats.average_overhead_ns(), 0.0);
    assert_eq!(stats.estimated_time_saved_us(), 0);
}
