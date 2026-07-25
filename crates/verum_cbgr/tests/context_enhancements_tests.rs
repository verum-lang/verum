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
//! Comprehensive tests for context-sensitive analysis enhancements
//!
//! Tests all three enhancements:
//! 1. Flow-sensitive context tracking
//! 2. Adaptive context depth
//! 3. Context compression

use verum_cbgr::analysis::{BlockId, EscapeResult, FunctionId, RefId};
use verum_cbgr::call_graph::{CallGraph, FunctionSignature, RefFlow};
use verum_cbgr::context_enhancements::*;

// ============================================================================
// FLOW-SENSITIVE CONTEXT TRACKING TESTS
// ============================================================================

#[test]
fn test_dataflow_state_creation() {
    let state = DataflowState::new(RefId(1), BlockId(0));
    assert_eq!(state.reference, RefId(1));
    assert_eq!(state.block, BlockId(0));
    assert_eq!(state.generation, 0);
    assert!(state.predicates.is_empty());
}

#[test]
fn test_dataflow_state_with_predicate() {
    let state =
        DataflowState::new(RefId(1), BlockId(0)).with_predicate(Predicate::BlockTrue(BlockId(1)));

    assert_eq!(state.predicates.len(), 1);
    assert!(state.satisfies(&Predicate::BlockTrue(BlockId(1))));
}

#[test]
fn test_dataflow_state_with_alias_state() {
    let state = DataflowState::new(RefId(1), BlockId(0)).with_alias_state(AliasState::NoAlias);

    assert_eq!(state.alias_state, AliasState::NoAlias);
}

#[test]
fn test_dataflow_state_next_generation() {
    let state1 = DataflowState::new(RefId(1), BlockId(0));
    let state2 = state1.next_generation();

    assert_eq!(state2.generation, 1);
}

#[test]
fn test_dataflow_state_merge_identical() {
    let state1 = DataflowState::new(RefId(1), BlockId(0))
        .with_alias_state(AliasState::NoAlias)
        .with_predicate(Predicate::BlockTrue(BlockId(1)));

    let state2 = DataflowState::new(RefId(1), BlockId(0))
        .with_alias_state(AliasState::NoAlias)
        .with_predicate(Predicate::BlockTrue(BlockId(1)));

    let merged = state1.merge(&state2);
    assert_eq!(merged.alias_state, AliasState::NoAlias);
    assert_eq!(merged.predicates.len(), 1);
}

#[test]
fn test_dataflow_state_merge_different_alias() {
    let state1 = DataflowState::new(RefId(1), BlockId(0)).with_alias_state(AliasState::NoAlias);

    let state2 =
        DataflowState::new(RefId(1), BlockId(0)).with_alias_state(AliasState::MustAlias(RefId(2)));

    let merged = state1.merge(&state2);
    assert_eq!(merged.alias_state, AliasState::Unknown);
}

#[test]
fn test_alias_state_variants() {
    let no_alias = AliasState::NoAlias;
    let must_alias = AliasState::MustAlias(RefId(2));
    let may_alias = AliasState::MayAlias(vec![RefId(2), RefId(3)]);
    let unknown = AliasState::Unknown;

    assert_ne!(no_alias, must_alias);
    assert_ne!(may_alias, unknown);
}

#[test]
fn test_predicate_variants() {
    let p1 = Predicate::BlockTrue(BlockId(1));
    let p2 = Predicate::BlockFalse(BlockId(1));
    let p3 = Predicate::IsNull(RefId(1));
    let p4 = Predicate::IsNotNull(RefId(1));
    let p5 = Predicate::Equal(RefId(1), RefId(2));
    let p6 = Predicate::NotEqual(RefId(1), RefId(2));

    assert_ne!(p1, p2);
    assert_ne!(p3, p4);
    assert_ne!(p5, p6);
}

#[test]
fn test_flow_sensitive_context_creation() {
    let context = FlowSensitiveContext::new(FunctionId(1));

    assert_eq!(context.function, FunctionId(1));
    assert_eq!(context.depth, 0);
    assert_eq!(context.call_chain.len(), 1);
    assert!(context.dataflow_states.is_empty());
}

#[test]
fn test_flow_sensitive_context_extend() {
    let base = FlowSensitiveContext::new(FunctionId(1));
    let extended = base.extend(FunctionId(2));

    assert_eq!(extended.depth, 1);
    assert_eq!(extended.call_chain.len(), 2);
    assert_eq!(extended.call_chain[0], FunctionId(1));
    assert_eq!(extended.call_chain[1], FunctionId(2));
}

#[test]
fn test_flow_sensitive_context_update_state() {
    let mut context = FlowSensitiveContext::new(FunctionId(1));
    let state = DataflowState::new(RefId(1), BlockId(0));

    context.update_state(BlockId(0), state.clone());

    assert_eq!(context.dataflow_states.len(), 1);
    assert!(context.get_state(BlockId(0)).is_some());
}

#[test]
fn test_flow_sensitive_context_merge_states() {
    let mut context1 = FlowSensitiveContext::new(FunctionId(1));
    let mut context2 = FlowSensitiveContext::new(FunctionId(1));

    let state1 = DataflowState::new(RefId(1), BlockId(0)).with_alias_state(AliasState::NoAlias);
    let state2 = DataflowState::new(RefId(1), BlockId(0)).with_alias_state(AliasState::NoAlias);

    context1.update_state(BlockId(0), state1);
    context2.update_state(BlockId(0), state2);

    context1.merge_states(&context2);

    assert_eq!(context1.dataflow_states.len(), 1);
}

#[test]
fn test_flow_sensitive_context_contains_function() {
    let base = FlowSensitiveContext::new(FunctionId(1));
    let extended = base.extend(FunctionId(2));

    assert!(extended.contains_function(FunctionId(1)));
    assert!(extended.contains_function(FunctionId(2)));
    assert!(!extended.contains_function(FunctionId(3)));
}

// ============================================================================
// ADAPTIVE CONTEXT DEPTH TESTS
// ============================================================================

#[test]
fn test_importance_metrics_creation() {
    let metrics = ImportanceMetrics::new();

    assert_eq!(metrics.call_frequency, 0.5);
    assert_eq!(metrics.escape_probability, 0.5);
    assert_eq!(metrics.code_complexity, 0.5);
    assert_eq!(metrics.num_callers, 0);
    assert_eq!(metrics.num_references, 0);
}

#[test]
fn test_importance_metrics_score_low() {
    let mut metrics = ImportanceMetrics::new();
    metrics.call_frequency = 0.1;
    metrics.escape_probability = 0.1;
    metrics.code_complexity = 0.1;
    metrics.num_callers = 1;
    metrics.num_references = 1;

    let score = metrics.importance_score();
    assert!(score < 0.3, "Expected low score, got {}", score);
}

#[test]
fn test_importance_metrics_score_high() {
    let mut metrics = ImportanceMetrics::new();
    metrics.call_frequency = 0.9;
    metrics.escape_probability = 0.9;
    metrics.code_complexity = 0.9;
    metrics.num_callers = 15;
    metrics.num_references = 25;

    let score = metrics.importance_score();
    assert!(score > 0.8, "Expected high score, got {}", score);
}

#[test]
fn test_importance_metrics_depth_limit_trivial() {
    let mut metrics = ImportanceMetrics::new();
    metrics.call_frequency = 0.2;
    metrics.escape_probability = 0.1;

    assert_eq!(metrics.depth_limit(), 1);
}

#[test]
fn test_importance_metrics_depth_limit_normal() {
    let mut metrics = ImportanceMetrics::new();
    metrics.call_frequency = 0.5;
    metrics.escape_probability = 0.4;

    assert_eq!(metrics.depth_limit(), 3);
}

#[test]
fn test_importance_metrics_depth_limit_important() {
    let mut metrics = ImportanceMetrics::new();
    // To get depth 5, need score >= 0.6 and < 0.8
    // score = 0.30*call_freq + 0.25*escape + 0.20*complexity + 0.15*callers_norm + 0.10*refs_norm
    // Set values to achieve score ~0.7:
    metrics.call_frequency = 0.9;
    metrics.escape_probability = 0.9;
    metrics.code_complexity = 0.5;
    // score = 0.27 + 0.225 + 0.10 = 0.595... Need more
    metrics.num_callers = 5; // normalized_callers = 0.5

    // score = 0.27 + 0.225 + 0.10 + 0.075 = 0.67 → depth 5
    assert_eq!(metrics.depth_limit(), 5);
}

#[test]
fn test_importance_metrics_depth_limit_critical() {
    let mut metrics = ImportanceMetrics::new();
    // To get depth 10, need score >= 0.8
    // Maximize all factors:
    metrics.call_frequency = 1.0;
    metrics.escape_probability = 1.0;
    metrics.code_complexity = 1.0;
    metrics.num_callers = 20; // normalized to 1.0
    metrics.num_references = 30; // normalized to 1.0

    // score = 0.30 + 0.25 + 0.20 + 0.15 + 0.10 = 1.0 → depth 10
    assert_eq!(metrics.depth_limit(), 10);
}

#[test]
fn test_adaptive_depth_policy_creation() {
    let policy = AdaptiveDepthPolicy::new(3, 10);

    assert_eq!(policy.depth_for_function(FunctionId(999)), 3);
}

#[test]
fn test_adaptive_depth_policy_set_metrics() {
    let mut policy = AdaptiveDepthPolicy::new(3, 10);

    // Create metrics that result in depth 10 (score >= 0.8)
    let mut metrics = ImportanceMetrics::new();
    metrics.call_frequency = 1.0;
    metrics.escape_probability = 1.0;
    metrics.code_complexity = 1.0;
    metrics.num_callers = 20;
    metrics.num_references = 30;
    // score = 1.0 → depth 10

    policy.set_metrics(FunctionId(1), metrics);

    assert_eq!(policy.depth_for_function(FunctionId(1)), 10);
}

#[test]
fn test_adaptive_depth_policy_max_depth_limit() {
    let mut policy = AdaptiveDepthPolicy::new(3, 5);

    // Create metrics that would give depth 10, but capped at max_depth=5
    let mut metrics = ImportanceMetrics::new();
    metrics.call_frequency = 1.0;
    metrics.escape_probability = 1.0;
    metrics.code_complexity = 1.0;
    metrics.num_callers = 20;
    metrics.num_references = 30;
    // score = 1.0 → depth 10, but capped at 5

    policy.set_metrics(FunctionId(1), metrics);

    assert_eq!(policy.depth_for_function(FunctionId(1)), 5);
}

#[test]
fn test_adaptive_depth_policy_compute_metrics() {
    let mut call_graph = CallGraph::new();
    let func1 = FunctionId(1);
    let func2 = FunctionId(2);

    call_graph.add_function(func1, FunctionSignature::new("func1", 0));
    call_graph.add_function(func2, FunctionSignature::new("func2", 0));
    call_graph.add_call(func2, func1, RefFlow::safe(0));

    let mut policy = AdaptiveDepthPolicy::new(3, 10);
    policy.compute_metrics(&call_graph);

    // func1 has 1 caller, should have some importance
    let depth = policy.depth_for_function(func1);
    assert!(depth >= 1);
}

#[test]
fn test_adaptive_depth_policy_update_from_profile() {
    let mut policy = AdaptiveDepthPolicy::new(3, 10);

    let metrics = ImportanceMetrics::new();
    policy.set_metrics(FunctionId(1), metrics);

    policy.update_from_profile(FunctionId(1), 1000, 800);

    let depth = policy.depth_for_function(FunctionId(1));
    assert!(depth > 3); // High escape rate should increase depth
}

// ============================================================================
// CONTEXT COMPRESSION TESTS
// ============================================================================

#[test]
fn test_abstract_context_creation() {
    let ctx = AbstractContext::new(FunctionId(1));

    assert_eq!(ctx.function, FunctionId(1));
    assert_eq!(ctx.call_pattern, CallPattern::Entry);
    assert!(ctx.abstract_predicates.is_empty());
}

#[test]
fn test_abstract_context_from_concrete() {
    let concrete = FlowSensitiveContext::new(FunctionId(1));
    let abstract_ctx = AbstractContext::from_concrete(&concrete);

    assert_eq!(abstract_ctx.function, FunctionId(1));
    assert_eq!(abstract_ctx.call_pattern, CallPattern::Entry);
}

#[test]
fn test_abstract_context_is_mergeable() {
    let ctx1 = AbstractContext::new(FunctionId(1));
    let ctx2 = AbstractContext::new(FunctionId(1));
    let ctx3 = AbstractContext::new(FunctionId(2));

    assert!(ctx1.is_mergeable_with(&ctx2));
    assert!(!ctx1.is_mergeable_with(&ctx3));
}

#[test]
fn test_call_pattern_entry() {
    let pattern = CallPattern::from_chain(&[]);
    assert_eq!(pattern, CallPattern::Entry);
}

#[test]
fn test_call_pattern_direct() {
    let pattern = CallPattern::from_chain(&[FunctionId(1), FunctionId(2)]);
    assert_eq!(pattern, CallPattern::Direct(FunctionId(2)));
}

#[test]
fn test_call_pattern_recursive() {
    let pattern = CallPattern::from_chain(&[FunctionId(1), FunctionId(1)]);
    assert!(matches!(pattern, CallPattern::Recursive(_)));
}

#[test]
fn test_call_pattern_multiple() {
    let pattern = CallPattern::from_chain(&[FunctionId(1), FunctionId(2), FunctionId(3)]);
    assert!(matches!(pattern, CallPattern::Multiple(_)));
}

#[test]
fn test_context_equivalence_class_creation() {
    let abstract_ctx = AbstractContext::new(FunctionId(1));
    let class = ContextEquivalenceClass::new(abstract_ctx);

    assert_eq!(class.members.len(), 0);
    assert!(class.merged_result.is_none());
}

#[test]
fn test_context_equivalence_class_add_member() {
    let abstract_ctx = AbstractContext::new(FunctionId(1));
    let mut class = ContextEquivalenceClass::new(abstract_ctx);

    let concrete = FlowSensitiveContext::new(FunctionId(1));
    class.add_member(concrete);

    assert_eq!(class.members.len(), 1);
}

#[test]
fn test_context_equivalence_class_compute_merged_result() {
    let abstract_ctx = AbstractContext::new(FunctionId(1));
    let mut class = ContextEquivalenceClass::new(abstract_ctx);

    let ctx1 = FlowSensitiveContext::new(FunctionId(1));
    let ctx2 = FlowSensitiveContext::new(FunctionId(1));

    class.add_member(ctx1);
    class.add_member(ctx2);

    let mut results = std::collections::HashMap::new();
    results.insert(0, EscapeResult::DoesNotEscape);
    results.insert(1, EscapeResult::EscapesViaReturn);

    class.compute_merged_result(&results);

    assert_eq!(class.merged_result, Some(EscapeResult::EscapesViaReturn));
}

#[test]
fn test_context_compressor_creation() {
    let compressor = ContextCompressor::new();

    assert_eq!(compressor.stats().total_contexts, 0);
    assert_eq!(compressor.stats().compressed_contexts, 0);
}

#[test]
fn test_context_compressor_compress_identical() {
    let mut compressor = ContextCompressor::new();

    let ctx1 = FlowSensitiveContext::new(FunctionId(1));
    let ctx2 = FlowSensitiveContext::new(FunctionId(1));

    let compressed = compressor.compress(vec![ctx1, ctx2]);

    assert_eq!(compressed.len(), 1);
    assert_eq!(compressed[0].members.len(), 2);
    assert_eq!(compressor.stats().total_contexts, 2);
    assert_eq!(compressor.stats().compressed_contexts, 1);
}

#[test]
fn test_context_compressor_compress_different() {
    let mut compressor = ContextCompressor::new();

    let ctx1 = FlowSensitiveContext::new(FunctionId(1));
    let ctx2 = FlowSensitiveContext::new(FunctionId(2));

    let compressed = compressor.compress(vec![ctx1, ctx2]);

    assert_eq!(compressed.len(), 2);
    assert_eq!(compressor.stats().total_contexts, 2);
    assert_eq!(compressor.stats().compressed_contexts, 2);
}

#[test]
fn test_compression_stats_savings() {
    let mut stats = CompressionStats::default();
    stats.total_contexts = 100;
    stats.compressed_contexts = 30;
    stats.compression_ratio = 0.3;

    assert_eq!(stats.savings(), 70);
}

#[test]
fn test_compression_stats_compression_percentage() {
    let mut stats = CompressionStats::default();
    stats.total_contexts = 100;
    stats.compressed_contexts = 25;
    stats.compression_ratio = 0.25;

    assert_eq!(stats.compression_percentage(), 75.0);
}

// ============================================================================
// INTEGRATION TESTS
// ============================================================================

#[test]
fn test_enhanced_config_default() {
    let config = EnhancedContextConfig::default();

    assert!(!config.flow_sensitive);
    assert!(!config.adaptive_depth);
    assert!(!config.compression);
    assert_eq!(config.default_depth, 3);
    assert_eq!(config.max_depth, 10);
}

#[test]
fn test_enhanced_config_all_enabled() {
    let config = EnhancedContextConfig::all_enabled();

    assert!(config.flow_sensitive);
    assert!(config.adaptive_depth);
    assert!(config.compression);
}

#[test]
fn test_enhanced_config_builder_pattern() {
    let config = EnhancedContextConfig::default()
        .with_flow_sensitive()
        .with_adaptive_depth()
        .with_compression()
        .with_default_depth(5)
        .with_max_depth(15);

    assert!(config.flow_sensitive);
    assert!(config.adaptive_depth);
    assert!(config.compression);
    assert_eq!(config.default_depth, 5);
    assert_eq!(config.max_depth, 15);
}

#[test]
fn test_enhanced_stats_speedup_ratio() {
    let mut stats = EnhancedStats::default();
    stats.analysis_time_ms = 50.0;

    let speedup = stats.speedup_ratio(100.0);
    assert_eq!(speedup, 2.0);
}

#[test]
fn test_build_flow_sensitive_contexts() {
    let mut call_graph = CallGraph::new();
    let func1 = FunctionId(1);
    let func2 = FunctionId(2);

    call_graph.add_function(func1, FunctionSignature::new("func1", 0));
    call_graph.add_function(func2, FunctionSignature::new("func2", 0));
    call_graph.add_call(func2, func1, RefFlow::safe(0));

    let contexts = build_flow_sensitive_contexts(func1, &call_graph, 3);

    assert!(!contexts.is_empty());
}

#[test]
fn test_compute_importance_metrics() {
    let mut call_graph = CallGraph::new();
    let func1 = FunctionId(1);
    let func2 = FunctionId(2);

    call_graph.add_function(func1, FunctionSignature::new("func1", 0));
    call_graph.add_function(func2, FunctionSignature::new("func2", 0));
    call_graph.add_call(func2, func1, RefFlow::safe(0));

    let metrics = compute_importance_metrics(&call_graph);

    assert!(metrics.contains_key(&func1));
    assert!(metrics.contains_key(&func2));
}

#[test]
fn test_flow_sensitive_context_multi_level() {
    let base = FlowSensitiveContext::new(FunctionId(1));
    let level1 = base.extend(FunctionId(2));
    let level2 = level1.extend(FunctionId(3));

    assert_eq!(level2.depth, 2);
    assert_eq!(level2.call_chain.len(), 3);
}

#[test]
fn test_adaptive_depth_with_many_callers() {
    let mut call_graph = CallGraph::new();
    let target = FunctionId(1);

    call_graph.add_function(target, FunctionSignature::new("target", 0));

    // Add many callers
    for i in 2..12 {
        let caller = FunctionId(i);
        call_graph.add_function(caller, FunctionSignature::new(format!("caller{}", i), 0));
        call_graph.add_call(caller, target, RefFlow::safe(0));
    }

    let mut policy = AdaptiveDepthPolicy::new(3, 10);
    policy.compute_metrics(&call_graph);

    let depth = policy.depth_for_function(target);
    assert!(depth > 3); // Many callers should increase importance
}

#[test]
fn test_context_compression_with_different_patterns() {
    let mut compressor = ContextCompressor::new();

    let ctx1 = FlowSensitiveContext::new(FunctionId(1));
    let ctx2 = FlowSensitiveContext::new(FunctionId(1)).extend(FunctionId(2));
    let ctx3 = FlowSensitiveContext::new(FunctionId(1)).extend(FunctionId(3));

    let compressed = compressor.compress(vec![ctx1, ctx2, ctx3]);

    // Different call patterns should create different equivalence classes
    assert!(compressed.len() >= 2);
}

#[test]
fn test_dataflow_state_complex_merge() {
    let state1 = DataflowState::new(RefId(1), BlockId(0))
        .with_predicate(Predicate::IsNotNull(RefId(1)))
        .with_predicate(Predicate::BlockTrue(BlockId(1)))
        .next_generation();

    let state2 = DataflowState::new(RefId(1), BlockId(0))
        .with_predicate(Predicate::IsNotNull(RefId(1)))
        .with_predicate(Predicate::BlockFalse(BlockId(2)))
        .next_generation()
        .next_generation();

    let merged = state1.merge(&state2);

    // Only common predicates should remain
    assert_eq!(merged.predicates.len(), 1);
    assert!(merged.satisfies(&Predicate::IsNotNull(RefId(1))));

    // Generation should be maximum
    assert_eq!(merged.generation, 2);
}

#[test]
fn test_importance_metrics_all_factors() {
    let mut metrics = ImportanceMetrics::new();
    metrics.call_frequency = 0.8;
    metrics.escape_probability = 0.6;
    metrics.code_complexity = 0.7;
    metrics.num_callers = 8;
    metrics.num_references = 15;

    let score = metrics.importance_score();

    // With high values across all factors, should be high importance
    assert!(score > 0.65, "Expected score > 0.65, got {}", score);
    assert!(score < 0.85, "Expected score < 0.85, got {}", score);
}

#[test]
fn test_context_equivalence_merge_all_promote() {
    let abstract_ctx = AbstractContext::new(FunctionId(1));
    let mut class = ContextEquivalenceClass::new(abstract_ctx);

    class.add_member(FlowSensitiveContext::new(FunctionId(1)));
    class.add_member(FlowSensitiveContext::new(FunctionId(1)));

    let mut results = std::collections::HashMap::new();
    results.insert(0, EscapeResult::DoesNotEscape);
    results.insert(1, EscapeResult::DoesNotEscape);

    class.compute_merged_result(&results);

    assert_eq!(class.merged_result, Some(EscapeResult::DoesNotEscape));
}

#[test]
fn test_context_equivalence_merge_mixed() {
    let abstract_ctx = AbstractContext::new(FunctionId(1));
    let mut class = ContextEquivalenceClass::new(abstract_ctx);

    class.add_member(FlowSensitiveContext::new(FunctionId(1)));
    class.add_member(FlowSensitiveContext::new(FunctionId(1)));
    class.add_member(FlowSensitiveContext::new(FunctionId(1)));

    let mut results = std::collections::HashMap::new();
    results.insert(0, EscapeResult::DoesNotEscape);
    results.insert(1, EscapeResult::EscapesViaHeap);
    results.insert(2, EscapeResult::DoesNotEscape);

    class.compute_merged_result(&results);

    // Conservative merge: if any escapes, class escapes
    assert_eq!(class.merged_result, Some(EscapeResult::EscapesViaHeap));
}

#[test]
fn test_large_context_compression() {
    let mut compressor = ContextCompressor::new();

    let mut contexts = Vec::new();

    // Create 100 contexts, 50 for func1, 50 for func2
    for _i in 0..50 {
        contexts.push(FlowSensitiveContext::new(FunctionId(1)));
        contexts.push(FlowSensitiveContext::new(FunctionId(2)));
    }

    let compressed = compressor.compress(contexts);

    assert_eq!(compressed.len(), 2);
    assert_eq!(compressor.stats().total_contexts, 100);
    assert_eq!(compressor.stats().compressed_contexts, 2);
    assert_eq!(compressor.stats().compression_ratio, 0.02);
    assert_eq!(compressor.stats().compression_percentage(), 98.0);
}

// ============================================================================
// PARALLEL ANALYSIS TESTS
// ============================================================================

#[test]
fn test_parallel_config_creation() {
    let config = ParallelConfig::new(20);
    assert_eq!(config.threshold, 20);
    assert_eq!(config.max_threads, 0);
    assert!(config.work_stealing);
}

#[test]
fn test_parallel_config_default() {
    let config = ParallelConfig::default();
    assert_eq!(config.threshold, 10);
    assert_eq!(config.max_threads, 0);
    assert!(config.work_stealing);
}

#[test]
fn test_parallel_config_builder() {
    let config = ParallelConfig::new(15)
        .with_max_threads(4)
        .with_work_stealing(false);

    assert_eq!(config.threshold, 15);
    assert_eq!(config.max_threads, 4);
    assert!(!config.work_stealing);
}

#[test]
fn test_parallel_config_should_parallelize() {
    let config = ParallelConfig::new(10);

    assert!(!config.should_parallelize(5));
    assert!(!config.should_parallelize(9));
    assert!(config.should_parallelize(10));
    assert!(config.should_parallelize(100));
}

#[test]
fn test_parallel_stats_creation() {
    let stats = ParallelStats::default();
    assert_eq!(stats.total_contexts, 0);
    assert_eq!(stats.threads_used, 0);
    assert_eq!(stats.parallel_time_ms, 0.0);
}

#[test]
fn test_parallel_stats_total_time() {
    let mut stats = ParallelStats::default();
    stats.parallel_time_ms = 50.0;
    stats.sequential_time_ms = 5.0;

    assert_eq!(stats.total_time_ms(), 55.0);
}

#[test]
fn test_parallel_stats_efficiency_percentage() {
    let mut stats = ParallelStats::default();
    stats.parallel_efficiency = 0.75;

    assert_eq!(stats.efficiency_percentage(), 75.0);
}

#[test]
fn test_parallel_stats_is_beneficial() {
    let mut stats = ParallelStats::default();

    stats.speedup_ratio = 1.05;
    assert!(!stats.is_beneficial()); // Less than 10% improvement

    stats.speedup_ratio = 1.5;
    assert!(stats.is_beneficial()); // 50% improvement
}

#[test]
fn test_parallel_stats_compute_speedup() {
    let mut stats = ParallelStats::default();
    stats.parallel_time_ms = 25.0;
    stats.sequential_time_ms = 5.0;
    stats.threads_used = 4;

    stats.compute_speedup(100.0);

    assert!(stats.speedup_ratio > 3.0); // Should be around 3.3x (100/30)
    assert!(stats.parallel_efficiency > 0.7); // Around 0.83 (3.3/4)
}

#[test]
fn test_parallel_result_accumulator_creation() {
    let accumulator: ParallelResultAccumulator<i32> = ParallelResultAccumulator::new();
    assert_eq!(accumulator.result_count(), 0);
    assert_eq!(accumulator.error_count(), 0);
}

#[test]
fn test_parallel_result_accumulator_add_result() {
    let accumulator = ParallelResultAccumulator::new();

    accumulator.add_result(0, 42);
    accumulator.add_result(1, 100);

    assert_eq!(accumulator.result_count(), 2);
}

#[test]
fn test_parallel_result_accumulator_add_error() {
    let accumulator: ParallelResultAccumulator<i32> = ParallelResultAccumulator::new();

    accumulator.add_error();
    accumulator.add_error();

    assert_eq!(accumulator.error_count(), 2);
}

#[test]
fn test_parallel_result_accumulator_into_results() {
    let accumulator = ParallelResultAccumulator::new();

    accumulator.add_result(0, "test1".to_string());
    accumulator.add_result(1, "test2".to_string());
    accumulator.add_error();

    let Some((results, errors)) = accumulator.into_results() else {
        panic!("Failed to get results");
    };

    assert_eq!(results.len(), 2);
    assert_eq!(errors, 1);
    assert_eq!(results.get(&0).unwrap(), "test1");
    assert_eq!(results.get(&1).unwrap(), "test2");
}

#[test]
fn test_parallel_context_analyzer_creation() {
    let analyzer = ParallelContextAnalyzer::with_default();
    let config = analyzer.config();
    assert_eq!(config.threshold, 10);
}

#[test]
fn test_parallel_context_analyzer_with_threshold() {
    let analyzer = ParallelContextAnalyzer::with_threshold(25);
    let config = analyzer.config();
    assert_eq!(config.threshold, 25);
}

#[test]
fn test_parallel_analyze_sequential_fallback() {
    let analyzer = ParallelContextAnalyzer::with_threshold(100);

    // Only 5 contexts - should fall back to sequential
    let contexts: Vec<_> = (0..5)
        .map(|i| FlowSensitiveContext::new(FunctionId(i)))
        .collect();

    let results = analyzer.analyze_parallel(&contexts, |ctx| ctx.function.0 * 2);

    assert_eq!(results.len(), 5);
    assert_eq!(results.get(&0).unwrap(), &0);
    assert_eq!(results.get(&2).unwrap(), &4);

    let stats = analyzer.stats();
    assert_eq!(stats.total_contexts, 5);
    assert_eq!(stats.threads_used, 1); // Sequential
}

#[test]
fn test_parallel_analyze_actually_parallel() {
    let analyzer = ParallelContextAnalyzer::with_threshold(10);

    // 50 contexts - should use parallelism
    let contexts: Vec<_> = (0..50)
        .map(|i| FlowSensitiveContext::new(FunctionId(i)))
        .collect();

    let results = analyzer.analyze_parallel(&contexts, |ctx| ctx.function.0 * 3);

    assert_eq!(results.len(), 50);

    // Verify all results are correct
    for i in 0u64..50 {
        assert_eq!(results.get(&(i as usize)).unwrap(), &(i * 3));
    }

    let stats = analyzer.stats();
    assert_eq!(stats.total_contexts, 50);
    assert!(stats.threads_used > 1); // Should use multiple threads
}

#[test]
fn test_parallel_analyze_with_complex_computation() {
    let analyzer = ParallelContextAnalyzer::with_threshold(5);

    let contexts: Vec<_> = (0..20)
        .map(|i| {
            let mut ctx = FlowSensitiveContext::new(FunctionId(i));
            // Add some state
            for j in 0..10 {
                let state = DataflowState::new(RefId(j), BlockId(j));
                ctx.update_state(BlockId(j), state);
            }
            ctx
        })
        .collect();

    let results = analyzer.analyze_parallel(&contexts, |ctx| {
        // Simulate complex computation
        ctx.dataflow_states.len() + ctx.depth
    });

    assert_eq!(results.len(), 20);
    for i in 0..20 {
        assert_eq!(results.get(&i).unwrap(), &10); // 10 states + 0 depth
    }
}

#[test]
fn test_parallel_analyze_equivalence_classes() {
    let analyzer = ParallelContextAnalyzer::with_threshold(5);

    // Create some equivalence classes
    let mut classes = Vec::new();
    for i in 0..15 {
        let abstract_ctx = AbstractContext::new(FunctionId(i));
        let mut class = ContextEquivalenceClass::new(abstract_ctx);
        class.add_member(FlowSensitiveContext::new(FunctionId(i)));
        classes.push(class);
    }

    let results = analyzer.analyze_equivalence_classes(&classes, |cls| cls.members.len());

    assert_eq!(results.len(), 15);
    for i in 0..15 {
        assert_eq!(results.get(&i).unwrap(), &1);
    }

    let stats = analyzer.stats();
    assert_eq!(stats.total_contexts, 15);
}

#[test]
fn test_context_sensitive_analyzer_creation() {
    let analyzer = ContextSensitiveAnalyzer::new();
    assert!(analyzer.depth_policy.is_none());
    assert!(analyzer.compressor.is_none());
    assert!(analyzer.parallel_analyzer.is_none());
}

#[test]
fn test_context_sensitive_analyzer_with_all_enhancements() {
    let analyzer = ContextSensitiveAnalyzer::with_all_enhancements();
    assert!(analyzer.depth_policy.is_some());
    assert!(analyzer.compressor.is_some());
    assert!(analyzer.parallel_analyzer.is_some());
}

#[test]
fn test_context_sensitive_analyzer_with_parallel() {
    let analyzer = ContextSensitiveAnalyzer::new().with_parallel(15);

    assert!(analyzer.parallel_analyzer.is_some());

    let config = analyzer.parallel_analyzer.as_ref().unwrap().config();
    assert_eq!(config.threshold, 15);
}

#[test]
fn test_context_sensitive_analyzer_with_parallel_threshold() {
    let analyzer = ContextSensitiveAnalyzer::new().with_parallel_threshold(20);

    assert!(analyzer.parallel_analyzer.is_some());

    let config = analyzer.parallel_analyzer.as_ref().unwrap().config();
    assert_eq!(config.threshold, 20);
}

#[test]
fn test_context_sensitive_analyzer_analyze_contexts_parallel() {
    let analyzer = ContextSensitiveAnalyzer::new().with_parallel(5);

    let contexts: Vec<_> = (0..10)
        .map(|i| FlowSensitiveContext::new(FunctionId(i)))
        .collect();

    let results = analyzer.analyze_contexts_parallel(&contexts, |ctx| ctx.function.0 + 100);

    assert_eq!(results.len(), 10);
    assert_eq!(results.get(&0).unwrap(), &100);
    assert_eq!(results.get(&9).unwrap(), &109);
}

#[test]
fn test_context_sensitive_analyzer_parallel_stats() {
    let analyzer = ContextSensitiveAnalyzer::new().with_parallel(5);

    let contexts: Vec<_> = (0..20)
        .map(|i| FlowSensitiveContext::new(FunctionId(i)))
        .collect();

    let _results = analyzer.analyze_contexts_parallel(&contexts, |ctx| ctx.function.0);

    let stats = analyzer.parallel_stats();
    assert!(stats.is_some());

    let stats = stats.unwrap();
    assert_eq!(stats.total_contexts, 20);
    assert!(stats.threads_used >= 1);
}

#[test]
fn test_parallel_analyzer_thread_safety() {
    use std::sync::Arc;
    use std::thread;

    let analyzer = Arc::new(ParallelContextAnalyzer::with_threshold(5));

    let contexts: Vec<_> = (0..30)
        .map(|i| FlowSensitiveContext::new(FunctionId(i)))
        .collect();

    let contexts = Arc::new(contexts);

    // Spawn multiple threads using the same analyzer
    let handles: Vec<_> = (0..3)
        .map(|thread_id| {
            let analyzer = Arc::clone(&analyzer);
            let contexts = Arc::clone(&contexts);

            thread::spawn(move || {
                analyzer.analyze_parallel(&contexts, |ctx| ctx.function.0 * thread_id)
            })
        })
        .collect();

    // Wait for all threads
    for handle in handles {
        let results = handle.join().unwrap();
        assert_eq!(results.len(), 30);
    }
}

#[test]
fn test_parallel_with_compression_integration() {
    let mut compressor = ContextCompressor::new();
    let analyzer = ParallelContextAnalyzer::with_threshold(5);

    // Create 50 contexts, but only 5 unique patterns
    let mut contexts = Vec::new();
    for _ in 0..10 {
        for func_id in 0..5 {
            contexts.push(FlowSensitiveContext::new(FunctionId(func_id)));
        }
    }

    // Compress first
    let compressed = compressor.compress(contexts);
    assert_eq!(compressed.len(), 5);

    // Then analyze in parallel
    let results = analyzer.analyze_equivalence_classes(&compressed, |cls| cls.members.len());

    assert_eq!(results.len(), 5);
    for i in 0..5 {
        assert_eq!(results.get(&i).unwrap(), &10); // Each class has 10 members
    }
}
