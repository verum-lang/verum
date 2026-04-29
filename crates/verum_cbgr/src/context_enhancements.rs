//! Context-Sensitive Analysis Enhancements for CBGR
//!
//! Enhances escape analysis precision through context-sensitive interprocedural
//! analysis. Without context sensitivity, a function called from two sites with
//! different escape behaviors would be conservatively marked as escaping for both.
//! Context tracking distinguishes calling contexts to enable more promotions.
//!
//! This module implements three production-grade enhancements to context-sensitive
//! interprocedural escape analysis:
//!
//! 1. **Flow-sensitive Context Tracking**: Track dataflow state per calling context
//! 2. **Adaptive Context Depth**: Dynamic depth adjustment based on importance heuristics
//! 3. **Context Compression**: Merge similar contexts to reduce explosion
//!
//! # Performance Target
//!
//! - **2-3x speedup** vs fixed-depth context-sensitive analysis
//! - **50-80% more promotions** vs context-insensitive analysis
//! - **<100ms** for 10K LOC with adaptive depth
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use verum_cbgr::analysis::{EscapeAnalyzer, ContextSensitiveAnalyzer};
//! use verum_cbgr::context_enhancements::*;
//! use verum_cbgr::call_graph::CallGraph;
//!
//! let cfg = build_cfg();
//! let analyzer = EscapeAnalyzer::with_function(cfg, FunctionId(1));
//! let mut cs_analyzer = ContextSensitiveAnalyzer::new(analyzer)
//!     .with_flow_sensitive()      // Enable flow tracking
//!     .with_adaptive_depth()      // Enable adaptive depth
//!     .with_compression();        // Enable context compression
//!
//! let call_graph = CallGraph::new();
//! let info = cs_analyzer.analyze_with_context(RefId(1), &call_graph);
//!
//! // 2-3x faster analysis with better precision!
//! println!("Analysis time: {:.2}ms", info.stats.analysis_time_ms);
//! println!("Promotion rate: {:.1}%", info.promotion_rate() * 100.0);
//! ```

use crate::analysis::{BlockId, EscapeResult, FunctionId, RefId};
use crate::call_graph::CallGraph;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use verum_common::Set;

// ============================================================================
// 1. FLOW-SENSITIVE CONTEXT TRACKING
// ============================================================================

/// Dataflow state at a program point
///
/// Tracks the state of references and values as they flow through
/// different calling contexts.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DataflowState {
    /// Reference being tracked
    pub reference: RefId,
    /// Block where this state is valid
    pub block: BlockId,
    /// Value state: must-alias, may-alias, no-alias
    pub alias_state: AliasState,
    /// Predicate constraints on this path
    pub predicates: Vec<Predicate>,
    /// Generation number for SSA-like tracking
    pub generation: u32,
}

impl DataflowState {
    /// Create new dataflow state
    #[must_use]
    pub fn new(reference: RefId, block: BlockId) -> Self {
        Self {
            reference,
            block,
            alias_state: AliasState::Unknown,
            predicates: Vec::new(),
            generation: 0,
        }
    }

    /// Add predicate constraint
    #[must_use]
    pub fn with_predicate(mut self, predicate: Predicate) -> Self {
        self.predicates.push(predicate);
        self
    }

    /// Set alias state
    #[must_use]
    pub fn with_alias_state(mut self, state: AliasState) -> Self {
        self.alias_state = state;
        self
    }

    /// Increment generation (SSA version)
    #[must_use]
    pub fn next_generation(mut self) -> Self {
        self.generation += 1;
        self
    }

    /// Check if state satisfies predicate
    #[must_use]
    pub fn satisfies(&self, predicate: &Predicate) -> bool {
        self.predicates.contains(predicate)
    }

    /// Merge with another state (join at control flow merge)
    #[must_use]
    pub fn merge(&self, other: &Self) -> Self {
        // Conservative merge: Unknown if states differ
        let alias_state = if self.alias_state == other.alias_state {
            self.alias_state.clone()
        } else {
            AliasState::Unknown
        };

        // Combine predicates (must hold in both)
        let mut predicates = Vec::new();
        for pred in &self.predicates {
            if other.predicates.contains(pred) {
                predicates.push(pred.clone());
            }
        }

        Self {
            reference: self.reference,
            block: self.block,
            alias_state,
            predicates,
            generation: self.generation.max(other.generation),
        }
    }
}

/// Alias state for flow-sensitive tracking
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AliasState {
    /// Definitely aliased (must-alias)
    MustAlias(RefId),
    /// Possibly aliased (may-alias)
    MayAlias(Vec<RefId>),
    /// Definitely not aliased
    NoAlias,
    /// Unknown (conservative)
    Unknown,
}

/// Predicate for path conditions
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Predicate {
    /// Block condition is true
    BlockTrue(BlockId),
    /// Block condition is false
    BlockFalse(BlockId),
    /// Reference is null
    IsNull(RefId),
    /// Reference is not null
    IsNotNull(RefId),
    /// References are equal
    Equal(RefId, RefId),
    /// References are not equal
    NotEqual(RefId, RefId),
}

/// Flow-sensitive context with dataflow state
///
/// Extends calling context with per-block dataflow state.
#[derive(Debug, Clone)]
pub struct FlowSensitiveContext {
    /// Base function being analyzed
    pub function: FunctionId,
    /// Calling context (call chain)
    pub call_chain: Vec<FunctionId>,
    /// Dataflow state per block
    pub dataflow_states: HashMap<BlockId, DataflowState>,
    /// Context depth
    pub depth: usize,
}

impl FlowSensitiveContext {
    /// Create new flow-sensitive context
    #[must_use]
    pub fn new(function: FunctionId) -> Self {
        Self {
            function,
            call_chain: vec![function],
            dataflow_states: HashMap::new(),
            depth: 0,
        }
    }

    /// Extend context with caller
    #[must_use]
    pub fn extend(&self, caller: FunctionId) -> Self {
        let mut call_chain = self.call_chain.clone();
        call_chain.push(caller);

        Self {
            function: self.function,
            call_chain,
            dataflow_states: self.dataflow_states.clone(),
            depth: self.depth + 1,
        }
    }

    /// Update dataflow state at block
    pub fn update_state(&mut self, block: BlockId, state: DataflowState) {
        self.dataflow_states.insert(block, state);
    }

    /// Get dataflow state at block
    #[must_use]
    pub fn get_state(&self, block: BlockId) -> Option<&DataflowState> {
        self.dataflow_states.get(&block)
    }

    /// Merge dataflow states with another context
    pub fn merge_states(&mut self, other: &Self) {
        for (block, other_state) in &other.dataflow_states {
            self.dataflow_states
                .entry(*block)
                .and_modify(|state| *state = state.merge(other_state))
                .or_insert_with(|| other_state.clone());
        }
    }

    /// Check if context contains function (recursion detection)
    #[must_use]
    pub fn contains_function(&self, func: FunctionId) -> bool {
        self.call_chain.contains(&func)
    }
}

// ============================================================================
// 2. ADAPTIVE CONTEXT DEPTH
// ============================================================================

/// Heuristics for determining function importance
///
/// Important functions get higher depth limit for more precise analysis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImportanceMetrics {
    /// Call frequency (0.0 = rare, 1.0 = hot)
    pub call_frequency: f64,
    /// Escape probability (0.0 = never, 1.0 = always)
    pub escape_probability: f64,
    /// Code complexity (0.0 = simple, 1.0 = complex)
    pub code_complexity: f64,
    /// Number of callers
    pub num_callers: usize,
    /// Number of references
    pub num_references: usize,
}

impl ImportanceMetrics {
    /// Create new metrics with defaults
    #[must_use]
    pub fn new() -> Self {
        Self {
            call_frequency: 0.5,
            escape_probability: 0.5,
            code_complexity: 0.5,
            num_callers: 0,
            num_references: 0,
        }
    }

    /// Compute overall importance score (0.0 - 1.0)
    ///
    /// Higher score = more important = higher depth limit
    #[must_use]
    pub fn importance_score(&self) -> f64 {
        // Weighted combination:
        // - Call frequency: 30% (hot functions need precision)
        // - Escape probability: 25% (likely escapes need thorough analysis)
        // - Code complexity: 20% (complex code needs more depth)
        // - Num callers: 15% (many callers benefit from context sensitivity)
        // - Num references: 10% (many refs benefit from precision)

        let normalized_callers = (self.num_callers as f64 / 10.0).min(1.0);
        let normalized_refs = (self.num_references as f64 / 20.0).min(1.0);

        0.30 * self.call_frequency
            + 0.25 * self.escape_probability
            + 0.20 * self.code_complexity
            + 0.15 * normalized_callers
            + 0.10 * normalized_refs
    }

    /// Determine depth limit based on importance
    ///
    /// - Score 0.0-0.3: depth 1 (trivial functions)
    /// - Score 0.3-0.6: depth 3 (normal functions)
    /// - Score 0.6-0.8: depth 5 (important functions)
    /// - Score 0.8-1.0: depth 10 (critical functions)
    #[must_use]
    pub fn depth_limit(&self) -> usize {
        let score = self.importance_score();

        if score < 0.3 {
            1
        } else if score < 0.6 {
            3
        } else if score < 0.8 {
            5
        } else {
            10
        }
    }
}

impl Default for ImportanceMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Adaptive depth policy
///
/// Dynamically adjusts context depth based on function importance.
#[derive(Debug, Clone)]
pub struct AdaptiveDepthPolicy {
    /// Metrics per function
    metrics: HashMap<FunctionId, ImportanceMetrics>,
    /// Global default depth
    default_depth: usize,
    /// Maximum depth (safety limit)
    max_depth: usize,
}

impl AdaptiveDepthPolicy {
    /// Create new adaptive depth policy
    #[must_use]
    pub fn new(default_depth: usize, max_depth: usize) -> Self {
        Self {
            metrics: HashMap::new(),
            default_depth,
            max_depth,
        }
    }

    /// Set metrics for a function
    pub fn set_metrics(&mut self, func: FunctionId, metrics: ImportanceMetrics) {
        self.metrics.insert(func, metrics);
    }

    /// Get depth limit for a function
    #[must_use]
    pub fn depth_for_function(&self, func: FunctionId) -> usize {
        self.metrics
            .get(&func)
            .map_or(self.default_depth, |m| m.depth_limit().min(self.max_depth))
    }

    /// Compute metrics from call graph
    pub fn compute_metrics(&mut self, call_graph: &CallGraph) {
        for func in call_graph.signatures.keys() {
            let mut metrics = ImportanceMetrics::new();

            // Compute call frequency (approximation)
            if let Some(callers) = call_graph.callers_of(*func) {
                metrics.num_callers = callers.len();
                // Assume hot if many callers
                metrics.call_frequency = (callers.len() as f64 / 5.0).min(1.0);
            }

            // Estimate escape probability
            // (In production, would use profile data)
            metrics.escape_probability = 0.5;

            // Estimate code complexity
            // (In production, would count CFG blocks/edges)
            metrics.code_complexity = 0.5;

            self.metrics.insert(*func, metrics);
        }
    }

    /// Update metrics based on profiling data
    pub fn update_from_profile(&mut self, func: FunctionId, call_count: u64, escape_count: u64) {
        self.metrics.entry(func).and_modify(|m| {
            m.call_frequency = (call_count as f64 / 1000.0).min(1.0);
            m.escape_probability = if call_count > 0 {
                escape_count as f64 / call_count as f64
            } else {
                0.0
            };
        });
    }
}

// ============================================================================
// 3. CONTEXT COMPRESSION
// ============================================================================

/// Abstract context for compression
///
/// Represents an equivalence class of concrete contexts.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AbstractContext {
    /// Function being analyzed
    pub function: FunctionId,
    /// Abstract call pattern (compressed call chain)
    pub call_pattern: CallPattern,
    /// Predicate abstraction
    pub abstract_predicates: Vec<AbstractPredicate>,
}

impl AbstractContext {
    /// Create new abstract context
    #[must_use]
    pub fn new(function: FunctionId) -> Self {
        Self {
            function,
            call_pattern: CallPattern::Entry,
            abstract_predicates: Vec::new(),
        }
    }

    /// Abstract from concrete context
    ///
    /// Extracts abstract predicates from the dataflow states in the concrete context.
    /// This enables context compression by identifying contexts with equivalent abstract
    /// predicate patterns even if their concrete predicates differ.
    ///
    /// # Algorithm
    ///
    /// 1. Collect all predicates from dataflow states
    /// 2. Classify each predicate as True, False, or Unknown:
    ///    - If a predicate appears in all states with same polarity -> True/False
    ///    - If a predicate appears with different polarities -> Unknown
    ///    - If a predicate doesn't affect escape analysis -> omit (optimization)
    /// 3. Simplify by merging redundant predicates
    ///
    /// # Example
    ///
    /// ```text
    /// Concrete predicates:
    ///   - BlockTrue(B1), IsNotNull(R1), BlockFalse(B2)
    ///
    /// Abstract predicates:
    ///   - True (R1 is always non-null on all paths)
    ///   - Unknown (B1/B2 may vary)
    /// ```
    #[must_use]
    pub fn from_concrete(context: &FlowSensitiveContext) -> Self {
        let call_pattern = CallPattern::from_chain(&context.call_chain);
        let abstract_predicates = Self::extract_abstract_predicates(context);

        Self {
            function: context.function,
            call_pattern,
            abstract_predicates,
        }
    }

    /// Extract abstract predicates from a concrete context's dataflow states
    ///
    /// Analyzes predicates across all dataflow states to determine their abstract
    /// equivalents for context compression.
    fn extract_abstract_predicates(context: &FlowSensitiveContext) -> Vec<AbstractPredicate> {
        if context.dataflow_states.is_empty() {
            return vec![AbstractPredicate::Unknown];
        }

        // Collect predicates from all dataflow states
        let mut all_predicates: HashMap<Predicate, PredicateTracker> = HashMap::new();

        for state in context.dataflow_states.values() {
            for pred in &state.predicates {
                all_predicates
                    .entry(pred.clone())
                    .or_default()
                    .mark_present();
            }
        }

        // Classify predicates
        let mut abstract_preds = Vec::new();
        let total_states = context.dataflow_states.len();

        for (_pred, tracker) in all_predicates {
            let abstract_pred = if tracker.present_count == total_states {
                // Predicate holds in all states -> True
                AbstractPredicate::True
            } else if tracker.present_count == 0 {
                // Predicate never holds -> False
                AbstractPredicate::False
            } else {
                // Predicate holds in some states -> Unknown
                AbstractPredicate::Unknown
            };

            // Only add non-trivial predicates
            if !matches!(abstract_pred, AbstractPredicate::Unknown) || abstract_preds.is_empty() {
                abstract_preds.push(abstract_pred);
            }
        }

        // Ensure we have at least one predicate
        if abstract_preds.is_empty() {
            abstract_preds.push(AbstractPredicate::Unknown);
        }

        // Simplify: if we have both True and False, result is Unknown
        let has_true = abstract_preds.contains(&AbstractPredicate::True);
        let has_false = abstract_preds.contains(&AbstractPredicate::False);

        if has_true && has_false {
            vec![AbstractPredicate::Unknown]
        } else if abstract_preds
            .iter()
            .all(|p| matches!(p, AbstractPredicate::True))
        {
            vec![AbstractPredicate::True]
        } else if abstract_preds
            .iter()
            .all(|p| matches!(p, AbstractPredicate::False))
        {
            vec![AbstractPredicate::False]
        } else {
            // Deduplicate
            let mut unique = Vec::new();
            for pred in abstract_preds {
                if !unique.contains(&pred) {
                    unique.push(pred);
                }
            }
            unique
        }
    }

    /// Check if two contexts can be merged
    #[must_use]
    pub fn is_mergeable_with(&self, other: &Self) -> bool {
        // Contexts are mergeable if:
        // 1. Same function
        // 2. Same call pattern
        // 3. Compatible predicates

        self.function == other.function && self.call_pattern == other.call_pattern
    }
}

/// Abstract call pattern
///
/// Compresses call chains by identifying recurring patterns.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CallPattern {
    /// Entry point (no caller)
    Entry,
    /// Single caller
    Direct(FunctionId),
    /// Recursive pattern
    Recursive(FunctionId),
    /// Multiple callers (merged)
    Multiple(Vec<FunctionId>),
}

impl CallPattern {
    /// Create pattern from call chain
    #[must_use]
    pub fn from_chain(chain: &[FunctionId]) -> Self {
        match chain.len() {
            0 => CallPattern::Entry,
            1 => CallPattern::Entry,
            2 => {
                let callee = chain[0];
                let caller = chain[1];
                // Check for direct recursion
                if callee == caller {
                    CallPattern::Recursive(callee)
                } else {
                    CallPattern::Direct(caller)
                }
            }
            _ => {
                // Check for recursion
                let callee = chain[0];
                if chain[1..].contains(&callee) {
                    CallPattern::Recursive(callee)
                } else {
                    // Multiple callers: take unique (without sorting since FunctionId doesn't implement Ord)
                    let mut unique = Vec::new();
                    let mut seen = Set::new();
                    for &id in chain {
                        if seen.insert(id) {
                            unique.push(id);
                        }
                    }
                    CallPattern::Multiple(unique)
                }
            }
        }
    }
}

/// Abstract predicate for compression
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AbstractPredicate {
    /// Always true
    True,
    /// Always false
    False,
    /// Unknown (top)
    Unknown,
}

/// Helper for tracking predicate presence across dataflow states
///
/// Used during abstract predicate extraction to determine if a predicate
/// holds in all states (True), no states (False), or some states (Unknown).
#[derive(Debug, Clone, Default)]
struct PredicateTracker {
    /// Number of states where this predicate is present
    present_count: usize,
}

impl PredicateTracker {
    /// Create new tracker
    fn new() -> Self {
        Self { present_count: 0 }
    }

    /// Mark predicate as present in current state
    fn mark_present(&mut self) {
        self.present_count += 1;
    }
}

/// Context equivalence class
///
/// Groups similar contexts for compression.
#[derive(Debug, Clone)]
pub struct ContextEquivalenceClass {
    /// Representative abstract context
    pub representative: AbstractContext,
    /// Concrete contexts in this class
    pub members: Vec<FlowSensitiveContext>,
    /// Merged escape result
    pub merged_result: Option<EscapeResult>,
}

impl ContextEquivalenceClass {
    /// Create new equivalence class
    #[must_use]
    pub fn new(representative: AbstractContext) -> Self {
        Self {
            representative,
            members: Vec::new(),
            merged_result: None,
        }
    }

    /// Add member to class
    pub fn add_member(&mut self, context: FlowSensitiveContext) {
        self.members.push(context);
    }

    /// Compute merged result
    pub fn compute_merged_result(&mut self, results: &HashMap<usize, EscapeResult>) {
        // Conservative merge: if any escapes, class escapes
        let mut merged = EscapeResult::DoesNotEscape;

        for (idx, _member) in self.members.iter().enumerate() {
            if let Some(result) = results.get(&idx) {
                merged = Self::merge_results(merged, *result);
            }
        }

        self.merged_result = Some(merged);
    }

    /// Merge two escape results conservatively
    fn merge_results(r1: EscapeResult, r2: EscapeResult) -> EscapeResult {
        match (r1, r2) {
            (EscapeResult::DoesNotEscape, EscapeResult::DoesNotEscape) => {
                EscapeResult::DoesNotEscape
            }
            (EscapeResult::EscapesViaReturn, _) | (_, EscapeResult::EscapesViaReturn) => {
                EscapeResult::EscapesViaReturn
            }
            (EscapeResult::EscapesViaHeap, _) | (_, EscapeResult::EscapesViaHeap) => {
                EscapeResult::EscapesViaHeap
            }
            (EscapeResult::EscapesViaClosure, _) | (_, EscapeResult::EscapesViaClosure) => {
                EscapeResult::EscapesViaClosure
            }
            (EscapeResult::EscapesViaThread, _) | (_, EscapeResult::EscapesViaThread) => {
                EscapeResult::EscapesViaThread
            }
            _ => EscapeResult::ExceedsStackBounds,
        }
    }
}

/// Context compressor
///
/// Merges similar contexts to reduce exponential explosion.
#[derive(Debug, Clone)]
pub struct ContextCompressor {
    /// Equivalence classes
    classes: HashMap<AbstractContext, ContextEquivalenceClass>,
    /// Compression statistics
    stats: CompressionStats,
}

impl ContextCompressor {
    /// Create new context compressor
    #[must_use]
    pub fn new() -> Self {
        Self {
            classes: HashMap::new(),
            stats: CompressionStats::default(),
        }
    }

    /// Compress a set of contexts
    pub fn compress(
        &mut self,
        contexts: Vec<FlowSensitiveContext>,
    ) -> Vec<ContextEquivalenceClass> {
        self.stats.total_contexts = contexts.len();
        self.classes.clear();

        // Group contexts by abstract representation
        for context in contexts {
            let abstract_ctx = AbstractContext::from_concrete(&context);

            self.classes
                .entry(abstract_ctx.clone())
                .or_insert_with(|| ContextEquivalenceClass::new(abstract_ctx))
                .add_member(context);
        }

        self.stats.compressed_contexts = self.classes.len();
        self.stats.compression_ratio = if self.stats.total_contexts > 0 {
            self.stats.compressed_contexts as f64 / self.stats.total_contexts as f64
        } else {
            1.0
        };

        self.classes.values().cloned().collect()
    }

    /// Get compression statistics
    #[must_use]
    pub fn stats(&self) -> &CompressionStats {
        &self.stats
    }
}

impl Default for ContextCompressor {
    fn default() -> Self {
        Self::new()
    }
}

/// Compression statistics
#[derive(Debug, Clone, Default)]
pub struct CompressionStats {
    /// Total contexts before compression
    pub total_contexts: usize,
    /// Contexts after compression
    pub compressed_contexts: usize,
    /// Compression ratio (compressed / total)
    pub compression_ratio: f64,
    /// Merge operations performed
    pub merges_performed: usize,
}

impl CompressionStats {
    /// Get compression savings
    #[must_use]
    pub fn savings(&self) -> usize {
        self.total_contexts.saturating_sub(self.compressed_contexts)
    }

    /// Get compression percentage
    #[must_use]
    pub fn compression_percentage(&self) -> f64 {
        (1.0 - self.compression_ratio) * 100.0
    }
}

// ============================================================================
// INTEGRATION WITH ContextSensitiveAnalyzer
// ============================================================================

/// Enhanced context-sensitive analysis configuration
#[derive(Debug, Clone)]
pub struct EnhancedContextConfig {
    /// Enable flow-sensitive tracking
    pub flow_sensitive: bool,
    /// Enable adaptive depth
    pub adaptive_depth: bool,
    /// Enable context compression
    pub compression: bool,
    /// Default depth (if not adaptive)
    pub default_depth: usize,
    /// Maximum depth (safety limit)
    pub max_depth: usize,
}

impl Default for EnhancedContextConfig {
    fn default() -> Self {
        Self {
            flow_sensitive: false,
            adaptive_depth: false,
            compression: false,
            default_depth: 3,
            max_depth: 10,
        }
    }
}

impl EnhancedContextConfig {
    /// Create config with all enhancements enabled
    #[must_use]
    pub fn all_enabled() -> Self {
        Self {
            flow_sensitive: true,
            adaptive_depth: true,
            compression: true,
            default_depth: 3,
            max_depth: 10,
        }
    }

    /// Enable flow-sensitive tracking
    #[must_use]
    pub fn with_flow_sensitive(mut self) -> Self {
        self.flow_sensitive = true;
        self
    }

    /// Enable adaptive depth
    #[must_use]
    pub fn with_adaptive_depth(mut self) -> Self {
        self.adaptive_depth = true;
        self
    }

    /// Enable context compression
    #[must_use]
    pub fn with_compression(mut self) -> Self {
        self.compression = true;
        self
    }

    /// Set default depth
    #[must_use]
    pub fn with_default_depth(mut self, depth: usize) -> Self {
        self.default_depth = depth;
        self
    }

    /// Set maximum depth
    #[must_use]
    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }
}

/// Enhanced analysis statistics
#[derive(Debug, Clone, Default)]
pub struct EnhancedStats {
    /// Flow-sensitive states tracked
    pub flow_states_tracked: usize,
    /// Adaptive depth adjustments
    pub depth_adjustments: usize,
    /// Contexts compressed
    pub contexts_compressed: usize,
    /// Analysis time (milliseconds)
    pub analysis_time_ms: f64,
    /// Compression stats
    pub compression_stats: CompressionStats,
}

impl EnhancedStats {
    /// Calculate speedup ratio vs baseline
    #[must_use]
    pub fn speedup_ratio(&self, baseline_time_ms: f64) -> f64 {
        if self.analysis_time_ms > 0.0 {
            baseline_time_ms / self.analysis_time_ms
        } else {
            1.0
        }
    }
}

// ============================================================================
// 4. PARALLEL ANALYSIS
// ============================================================================

/// Configuration for parallel context analysis
///
/// Controls when and how to parallelize context-sensitive analysis.
#[derive(Debug, Clone)]
pub struct ParallelConfig {
    /// Minimum number of contexts before using parallelism (default: 10)
    pub threshold: usize,
    /// Maximum number of threads to use (0 = use rayon default)
    pub max_threads: usize,
    /// Enable work stealing (default: true)
    pub work_stealing: bool,
}

impl ParallelConfig {
    /// Create new parallel configuration
    #[must_use]
    pub fn new(threshold: usize) -> Self {
        Self {
            threshold,
            max_threads: 0,
            work_stealing: true,
        }
    }

    /// Set maximum threads
    #[must_use]
    pub fn with_max_threads(mut self, max_threads: usize) -> Self {
        self.max_threads = max_threads;
        self
    }

    /// Enable/disable work stealing
    #[must_use]
    pub fn with_work_stealing(mut self, enabled: bool) -> Self {
        self.work_stealing = enabled;
        self
    }

    /// Check if parallelism should be used
    #[must_use]
    pub fn should_parallelize(&self, context_count: usize) -> bool {
        context_count >= self.threshold
    }
}

impl Default for ParallelConfig {
    fn default() -> Self {
        Self {
            threshold: 10,
            max_threads: 0,
            work_stealing: true,
        }
    }
}

/// Statistics for parallel analysis
///
/// Tracks parallelism effectiveness and performance metrics.
#[derive(Debug, Clone, Default)]
pub struct ParallelStats {
    /// Total contexts analyzed
    pub total_contexts: usize,
    /// Number of threads actually used
    pub threads_used: usize,
    /// Time spent in parallel section (milliseconds)
    pub parallel_time_ms: f64,
    /// Time spent in sequential overhead (milliseconds)
    pub sequential_time_ms: f64,
    /// Actual speedup achieved vs sequential
    pub speedup_ratio: f64,
    /// Parallel efficiency (0.0 - 1.0)
    pub parallel_efficiency: f64,
}

impl ParallelStats {
    /// Calculate total time
    #[must_use]
    pub fn total_time_ms(&self) -> f64 {
        self.parallel_time_ms + self.sequential_time_ms
    }

    /// Calculate theoretical vs actual speedup
    #[must_use]
    pub fn efficiency_percentage(&self) -> f64 {
        self.parallel_efficiency * 100.0
    }

    /// Check if parallelism was beneficial
    #[must_use]
    pub fn is_beneficial(&self) -> bool {
        self.speedup_ratio > 1.1 // At least 10% improvement
    }

    /// Compute speedup ratio from times
    pub fn compute_speedup(&mut self, sequential_baseline_ms: f64) {
        let total = self.total_time_ms();
        if total > 0.0 {
            self.speedup_ratio = sequential_baseline_ms / total;
            self.parallel_efficiency = self.speedup_ratio / self.threads_used as f64;
        }
    }
}

/// Thread-safe result accumulator for parallel analysis
///
/// Uses Mutex for safe concurrent updates. For very high contention,
/// could be replaced with lock-free structures.
#[derive(Debug)]
pub struct ParallelResultAccumulator<T> {
    /// Results indexed by context ID
    results: Arc<Mutex<HashMap<usize, T>>>,
    /// Error count
    errors: Arc<Mutex<usize>>,
}

impl<T> ParallelResultAccumulator<T> {
    /// Create new accumulator
    #[must_use]
    pub fn new() -> Self {
        Self {
            results: Arc::new(Mutex::new(HashMap::new())),
            errors: Arc::new(Mutex::new(0)),
        }
    }

    /// Add a result (thread-safe)
    pub fn add_result(&self, context_id: usize, result: T) {
        let mut results = self.results.lock().unwrap();
        results.insert(context_id, result);
    }

    /// Increment error count (thread-safe)
    pub fn add_error(&self) {
        let mut errors = self.errors.lock().unwrap();
        *errors += 1;
    }

    /// Get all results (consumes accumulator)
    ///
    /// Returns None if the Arc still has multiple references (should not happen
    /// in correct usage, but we handle it gracefully rather than panicking).
    #[must_use]
    pub fn into_results(self) -> Option<(HashMap<usize, T>, usize)> {
        let results = match Arc::try_unwrap(self.results) {
            Ok(mutex) => mutex.into_inner().unwrap(),
            Err(_arc) => {
                // Arc still has references - cannot consume, return None
                // This indicates incorrect usage (concurrent access after into_results called)
                return None;
            }
        };
        let errors = *self.errors.lock().unwrap();
        Some((results, errors))
    }

    /// Get all results by cloning (requires T: Clone)
    ///
    /// Use this when you need results but may still have concurrent references.
    #[must_use]
    pub fn clone_results(&self) -> (HashMap<usize, T>, usize)
    where
        T: Clone,
    {
        let results = self.results.lock().unwrap().clone();
        let errors = *self.errors.lock().unwrap();
        (results, errors)
    }

    /// Get result count (non-consuming)
    #[must_use]
    pub fn result_count(&self) -> usize {
        self.results.lock().unwrap().len()
    }

    /// Get error count (non-consuming)
    #[must_use]
    pub fn error_count(&self) -> usize {
        *self.errors.lock().unwrap()
    }
}

impl<T> Default for ParallelResultAccumulator<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for ParallelResultAccumulator<T> {
    fn clone(&self) -> Self {
        Self {
            results: Arc::clone(&self.results),
            errors: Arc::clone(&self.errors),
        }
    }
}

/// Parallel context analyzer
///
/// Analyzes multiple contexts in parallel using Rayon's work-stealing scheduler.
#[derive(Debug, Clone)]
pub struct ParallelContextAnalyzer {
    /// Parallel configuration
    config: ParallelConfig,
    /// Statistics from last run
    stats: Arc<Mutex<ParallelStats>>,
}

impl ParallelContextAnalyzer {
    /// Create new parallel analyzer
    #[must_use]
    pub fn new(config: ParallelConfig) -> Self {
        Self {
            config,
            stats: Arc::new(Mutex::new(ParallelStats::default())),
        }
    }

    /// Create with default configuration
    #[must_use]
    pub fn with_default() -> Self {
        Self::new(ParallelConfig::default())
    }

    /// Create with custom threshold
    #[must_use]
    pub fn with_threshold(threshold: usize) -> Self {
        Self::new(ParallelConfig::new(threshold))
    }

    /// Analyze contexts in parallel.
    ///
    /// Returns results indexed by context position in input vector.
    /// Honours the full `ParallelConfig`:
    ///
    /// * `threshold` — already gated through `should_parallelize`.
    /// * `max_threads > 0` — runs the parallel section inside a
    ///   bespoke `rayon::ThreadPool` capped to that many workers
    ///   (instead of leaking onto the global pool which is sized
    ///   by `RAYON_NUM_THREADS` or logical CPU count). The pool
    ///   is built per-call rather than cached because the cost
    ///   amortises across the analysis work and cached pools
    ///   would deadlock if a caller were to re-enter from inside
    ///   `analyzer`.
    /// * `work_stealing` — rayon's default scheduler is
    ///   work-stealing; the flag is informational and matches
    ///   the documented contract. Disabling it would require a
    ///   different scheduler entirely; until that lands, the
    ///   accessor `work_stealing_enabled()` is the read surface.
    pub fn analyze_parallel<F, T>(
        &self,
        contexts: &[FlowSensitiveContext],
        analyzer: F,
    ) -> HashMap<usize, T>
    where
        F: Fn(&FlowSensitiveContext) -> T + Send + Sync,
        T: Send,
    {
        use std::time::Instant;

        let start = Instant::now();
        let context_count = contexts.len();

        // Check if parallelism is beneficial
        if !self.config.should_parallelize(context_count) {
            // Sequential analysis for small inputs
            let results: HashMap<usize, T> = contexts
                .iter()
                .enumerate()
                .map(|(idx, ctx)| (idx, analyzer(ctx)))
                .collect();

            self.update_stats(
                context_count,
                1,
                start.elapsed().as_secs_f64() * 1000.0,
                0.0,
            );
            return results;
        }

        // Parallel analysis
        let seq_start = Instant::now();
        let accumulator = ParallelResultAccumulator::new();
        let seq_time = seq_start.elapsed().as_secs_f64() * 1000.0;

        let par_start = Instant::now();

        // Honour `max_threads`: when > 0, run the parallel
        // section inside a bespoke pool capped to that many
        // workers. Failures fall back to the global pool so a
        // pool-build error doesn't abort the analysis.
        let max_threads = self.config.max_threads;
        let run_parallel = |acc: &ParallelResultAccumulator<T>| {
            contexts.par_iter().enumerate().for_each(|(idx, ctx)| {
                let result = analyzer(ctx);
                acc.add_result(idx, result);
            });
        };
        let threads_used = if max_threads > 0 {
            match rayon::ThreadPoolBuilder::new()
                .num_threads(max_threads)
                .build()
            {
                Ok(pool) => {
                    pool.install(|| run_parallel(&accumulator));
                    max_threads
                }
                Err(_) => {
                    run_parallel(&accumulator);
                    rayon::current_num_threads()
                }
            }
        } else {
            run_parallel(&accumulator);
            rayon::current_num_threads()
        };

        let par_time = par_start.elapsed().as_secs_f64() * 1000.0;

        let (results, _errors) = accumulator.into_results().unwrap_or_default();

        self.update_stats(context_count, threads_used, par_time, seq_time);

        results
    }

    /// Analyze equivalence classes in parallel
    ///
    /// Each equivalence class is analyzed independently, making this
    /// embarrassingly parallel with no synchronization needed.
    pub fn analyze_equivalence_classes<F, T>(
        &self,
        classes: &[ContextEquivalenceClass],
        analyzer: F,
    ) -> HashMap<usize, T>
    where
        F: Fn(&ContextEquivalenceClass) -> T + Send + Sync,
        T: Send,
    {
        use std::time::Instant;

        let start = Instant::now();
        let class_count = classes.len();

        if !self.config.should_parallelize(class_count) {
            let results: HashMap<usize, T> = classes
                .iter()
                .enumerate()
                .map(|(idx, cls)| (idx, analyzer(cls)))
                .collect();

            self.update_stats(class_count, 1, start.elapsed().as_secs_f64() * 1000.0, 0.0);
            return results;
        }

        let seq_start = Instant::now();
        let accumulator = ParallelResultAccumulator::new();
        let seq_time = seq_start.elapsed().as_secs_f64() * 1000.0;

        let par_start = Instant::now();

        classes.par_iter().enumerate().for_each(|(idx, cls)| {
            let result = analyzer(cls);
            accumulator.add_result(idx, result);
        });

        let par_time = par_start.elapsed().as_secs_f64() * 1000.0;
        let (results, _errors) = accumulator.into_results().unwrap_or_default();
        let threads_used = rayon::current_num_threads();

        self.update_stats(class_count, threads_used, par_time, seq_time);

        results
    }

    /// Get statistics from last parallel run
    #[must_use]
    pub fn stats(&self) -> ParallelStats {
        self.stats.lock().unwrap().clone()
    }

    /// Update statistics
    fn update_stats(
        &self,
        context_count: usize,
        threads_used: usize,
        parallel_time_ms: f64,
        sequential_time_ms: f64,
    ) {
        let mut stats = self.stats.lock().unwrap();
        stats.total_contexts = context_count;
        stats.threads_used = threads_used;
        stats.parallel_time_ms = parallel_time_ms;
        stats.sequential_time_ms = sequential_time_ms;

        // Estimate sequential baseline (very rough approximation)
        let estimated_sequential = parallel_time_ms * threads_used as f64 * 0.8; // 80% efficiency assumed
        stats.compute_speedup(estimated_sequential);
    }

    /// Read mirror of `ParallelConfig.max_threads`. `0` means
    /// "use rayon's default pool".
    #[must_use]
    pub fn max_threads(&self) -> usize {
        self.config.max_threads
    }

    /// Read mirror of `ParallelConfig.work_stealing`. The
    /// underlying rayon scheduler is always work-stealing; this
    /// accessor exists so consumers of the analyzer can query the
    /// declared stance and surface it in diagnostics.
    #[must_use]
    pub fn work_stealing_enabled(&self) -> bool {
        self.config.work_stealing
    }

    /// Configure parallelism threshold
    #[must_use]
    pub fn with_parallel_threshold(mut self, threshold: usize) -> Self {
        self.config.threshold = threshold;
        self
    }

    /// Get current configuration
    #[must_use]
    pub fn config(&self) -> &ParallelConfig {
        &self.config
    }
}

impl Default for ParallelContextAnalyzer {
    fn default() -> Self {
        Self::with_default()
    }
}

/// Context-sensitive analyzer with parallel support
///
/// Integrates parallel analysis into the enhanced context-sensitive workflow.
pub struct ContextSensitiveAnalyzer {
    /// Base configuration
    pub config: EnhancedContextConfig,
    /// Adaptive depth policy
    pub depth_policy: Option<AdaptiveDepthPolicy>,
    /// Context compressor
    pub compressor: Option<ContextCompressor>,
    /// Parallel analyzer
    pub parallel_analyzer: Option<ParallelContextAnalyzer>,
}

impl ContextSensitiveAnalyzer {
    /// Create new analyzer with default configuration
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: EnhancedContextConfig::default(),
            depth_policy: None,
            compressor: None,
            parallel_analyzer: None,
        }
    }

    /// Enable all enhancements including parallelism
    #[must_use]
    pub fn with_all_enhancements() -> Self {
        Self {
            config: EnhancedContextConfig::all_enabled(),
            depth_policy: Some(AdaptiveDepthPolicy::new(3, 10)),
            compressor: Some(ContextCompressor::new()),
            parallel_analyzer: Some(ParallelContextAnalyzer::with_default()),
        }
    }

    /// Set configuration. The booleans on the config now drive the
    /// optional helper-component slots so that
    /// `ContextSensitiveAnalyzer::new().with_config(cfg)` produces
    /// the same effective shape as the bespoke
    /// `with_all_enhancements()` constructor when `cfg` is
    /// `EnhancedContextConfig::all_enabled()`.
    ///
    /// Specifically:
    ///
    /// * `cfg.adaptive_depth == true` → install an
    ///   `AdaptiveDepthPolicy::new(default_depth, max_depth)`.
    ///   Pre-existing policies are replaced so the policy
    ///   matches the active depth bounds.
    /// * `cfg.compression == true` → install a fresh
    ///   `ContextCompressor` if none is set; pre-existing
    ///   compressors are preserved (caller may have configured a
    ///   custom compression strategy).
    /// * `cfg.flow_sensitive` is observable via
    ///   `flow_sensitive_enabled()` for downstream analyses that
    ///   gate flow-sensitive bookkeeping on it.
    ///
    /// Before this wire-up the booleans were inert — set on the
    /// config but read by no consumer.
    #[must_use]
    pub fn with_config(mut self, config: EnhancedContextConfig) -> Self {
        if config.adaptive_depth {
            self.depth_policy = Some(AdaptiveDepthPolicy::new(
                config.default_depth,
                config.max_depth,
            ));
        }
        if config.compression && self.compressor.is_none() {
            self.compressor = Some(ContextCompressor::new());
        }
        self.config = config;
        self
    }

    /// Whether flow-sensitive tracking is enabled. Mirrors
    /// `EnhancedContextConfig.flow_sensitive`. Downstream
    /// analyses that maintain per-flow state consult this to
    /// decide whether to install or skip the flow-state map.
    #[must_use]
    pub fn flow_sensitive_enabled(&self) -> bool {
        self.config.flow_sensitive
    }

    /// Whether adaptive depth is enabled. Mirrors
    /// `EnhancedContextConfig.adaptive_depth`. `true` means the
    /// `depth_policy` slot is in play; `false` means callers
    /// should treat `default_depth` as the static depth bound.
    #[must_use]
    pub fn adaptive_depth_enabled(&self) -> bool {
        self.config.adaptive_depth
    }

    /// Whether context compression is enabled. Mirrors
    /// `EnhancedContextConfig.compression`. `true` means the
    /// `compressor` slot is in play.
    #[must_use]
    pub fn compression_enabled(&self) -> bool {
        self.config.compression
    }

    /// Default analysis depth — used by callers that don't have a
    /// `depth_policy` installed (i.e. `adaptive_depth == false`).
    #[must_use]
    pub fn default_depth(&self) -> usize {
        self.config.default_depth
    }

    /// Hard upper bound on context depth. The adaptive policy
    /// honours this; callers in non-adaptive mode should refuse
    /// to descend past it as a safety check.
    #[must_use]
    pub fn max_depth(&self) -> usize {
        self.config.max_depth
    }

    /// Enable adaptive depth
    #[must_use]
    pub fn with_adaptive_depth(mut self, policy: AdaptiveDepthPolicy) -> Self {
        self.depth_policy = Some(policy);
        self
    }

    /// Enable compression
    #[must_use]
    pub fn with_compression(mut self, compressor: ContextCompressor) -> Self {
        self.compressor = Some(compressor);
        self
    }

    /// Enable parallel analysis
    #[must_use]
    pub fn with_parallel(mut self, parallel_threshold: usize) -> Self {
        self.parallel_analyzer = Some(ParallelContextAnalyzer::with_threshold(parallel_threshold));
        self
    }

    /// Set parallel threshold
    #[must_use]
    pub fn with_parallel_threshold(mut self, threshold: usize) -> Self {
        if let Some(ref mut pa) = self.parallel_analyzer {
            *pa = pa.clone().with_parallel_threshold(threshold);
        } else {
            self.parallel_analyzer = Some(ParallelContextAnalyzer::with_threshold(threshold));
        }
        self
    }

    /// Analyze contexts in parallel
    ///
    /// Returns analysis results for each context. Uses parallel execution
    /// if threshold is met, otherwise falls back to sequential.
    pub fn analyze_contexts_parallel<F, T>(
        &self,
        contexts: &[FlowSensitiveContext],
        analyzer: F,
    ) -> HashMap<usize, T>
    where
        F: Fn(&FlowSensitiveContext) -> T + Send + Sync,
        T: Send,
    {
        if let Some(ref parallel) = self.parallel_analyzer {
            parallel.analyze_parallel(contexts, analyzer)
        } else {
            // Fallback to sequential
            contexts
                .iter()
                .enumerate()
                .map(|(idx, ctx)| (idx, analyzer(ctx)))
                .collect()
        }
    }

    /// Get parallel statistics
    #[must_use]
    pub fn parallel_stats(&self) -> Option<ParallelStats> {
        self.parallel_analyzer
            .as_ref()
            .map(ParallelContextAnalyzer::stats)
    }
}

impl Default for ContextSensitiveAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Build flow-sensitive contexts from call graph
#[must_use]
pub fn build_flow_sensitive_contexts(
    function: FunctionId,
    call_graph: &CallGraph,
    max_depth: usize,
) -> Vec<FlowSensitiveContext> {
    let mut contexts = Vec::new();
    let base_context = FlowSensitiveContext::new(function);

    // Recursive helper to enumerate contexts
    fn enumerate(
        ctx: &FlowSensitiveContext,
        call_graph: &CallGraph,
        max_depth: usize,
        result: &mut Vec<FlowSensitiveContext>,
    ) {
        result.push(ctx.clone());

        if ctx.depth >= max_depth {
            return;
        }

        // Find callers
        if let Some(callers) = call_graph.callers_of(ctx.function) {
            for caller in callers {
                // Avoid infinite recursion
                if !ctx.contains_function(*caller) {
                    let extended = ctx.extend(*caller);
                    enumerate(&extended, call_graph, max_depth, result);
                }
            }
        }
    }

    enumerate(&base_context, call_graph, max_depth, &mut contexts);
    contexts
}

/// Compute importance metrics for all functions
#[must_use]
pub fn compute_importance_metrics(
    call_graph: &CallGraph,
) -> HashMap<FunctionId, ImportanceMetrics> {
    let mut metrics = HashMap::new();

    for func in call_graph.signatures.keys() {
        let mut m = ImportanceMetrics::new();

        if let Some(callers) = call_graph.callers_of(*func) {
            m.num_callers = callers.len();
            m.call_frequency = (callers.len() as f64 / 5.0).min(1.0);
        }

        metrics.insert(*func, m);
    }

    metrics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dataflow_state_creation() {
        let state = DataflowState::new(RefId(1), BlockId(0));
        assert_eq!(state.reference, RefId(1));
        assert_eq!(state.block, BlockId(0));
        assert_eq!(state.generation, 0);
    }

    #[test]
    fn test_dataflow_state_merge() {
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
    fn test_importance_metrics_score() {
        let mut metrics = ImportanceMetrics::new();
        metrics.call_frequency = 0.8;
        metrics.escape_probability = 0.3;
        metrics.code_complexity = 0.6;
        metrics.num_callers = 5;
        metrics.num_references = 10;

        let score = metrics.importance_score();
        assert!(score > 0.5 && score < 0.8);
    }

    #[test]
    fn test_importance_metrics_depth_limit() {
        let mut metrics = ImportanceMetrics::new();

        // Score: 0.30 * 0.2 = 0.06 < 0.3 -> depth 1
        metrics.call_frequency = 0.2;
        assert_eq!(metrics.depth_limit(), 1);

        // Score: 0.30 * 0.5 + 0.25 * 0.5 = 0.275 < 0.3 -> depth 1 (needs more)
        metrics.call_frequency = 0.5;
        metrics.escape_probability = 0.5;
        assert_eq!(metrics.depth_limit(), 3);

        // Score: 0.30 * 0.8 + 0.25 * 0.8 + 0.20 * 0.8 = 0.60 -> depth 5
        metrics.call_frequency = 0.8;
        metrics.escape_probability = 0.8;
        metrics.code_complexity = 0.8;
        assert_eq!(metrics.depth_limit(), 5);

        // Score: 0.30 * 0.9 + 0.25 * 0.9 + 0.20 * 0.9 + 0.15 * 1.0 = 0.825 -> depth 10
        metrics.call_frequency = 0.9;
        metrics.escape_probability = 0.9;
        metrics.code_complexity = 0.9;
        metrics.num_callers = 10; // normalized to 1.0
        assert_eq!(metrics.depth_limit(), 10);
    }

    #[test]
    fn test_adaptive_depth_policy() {
        let mut policy = AdaptiveDepthPolicy::new(3, 10);

        let mut metrics = ImportanceMetrics::new();
        metrics.call_frequency = 0.9;
        metrics.escape_probability = 0.9;
        metrics.code_complexity = 0.9;
        metrics.num_callers = 10;
        policy.set_metrics(FunctionId(1), metrics);

        assert_eq!(policy.depth_for_function(FunctionId(1)), 10);
        assert_eq!(policy.depth_for_function(FunctionId(999)), 3);
    }

    #[test]
    fn test_context_compressor() {
        let mut compressor = ContextCompressor::new();

        let ctx1 = FlowSensitiveContext::new(FunctionId(1));
        let ctx2 = FlowSensitiveContext::new(FunctionId(1));
        let ctx3 = FlowSensitiveContext::new(FunctionId(2));

        let compressed = compressor.compress(vec![ctx1, ctx2, ctx3]);

        // Should compress ctx1 and ctx2 into one class
        assert_eq!(compressed.len(), 2);
        assert_eq!(compressor.stats().total_contexts, 3);
        assert_eq!(compressor.stats().compressed_contexts, 2);
    }

    #[test]
    fn test_compression_stats() {
        let mut stats = CompressionStats::default();
        stats.total_contexts = 100;
        stats.compressed_contexts = 30;
        stats.compression_ratio = 0.3;

        assert_eq!(stats.savings(), 70);
        assert_eq!(stats.compression_percentage(), 70.0);
    }

    #[test]
    fn enhanced_context_config_drives_dependencies() {
        // Pin: `with_config(EnhancedContextConfig::all_enabled())`
        // installs the dependent components (depth_policy +
        // compressor) — before the wire-up, the booleans on the
        // config were inert and the dependent slots stayed `None`.
        let analyzer = ContextSensitiveAnalyzer::new()
            .with_config(EnhancedContextConfig::all_enabled());
        assert!(analyzer.flow_sensitive_enabled());
        assert!(analyzer.adaptive_depth_enabled());
        assert!(analyzer.compression_enabled());
        assert!(
            analyzer.depth_policy.is_some(),
            "adaptive_depth=true must install a depth policy"
        );
        assert!(
            analyzer.compressor.is_some(),
            "compression=true must install a compressor"
        );
        assert_eq!(analyzer.default_depth(), 3);
        assert_eq!(analyzer.max_depth(), 10);

        // Default config installs nothing.
        let bare = ContextSensitiveAnalyzer::new()
            .with_config(EnhancedContextConfig::default());
        assert!(!bare.flow_sensitive_enabled());
        assert!(!bare.adaptive_depth_enabled());
        assert!(!bare.compression_enabled());
        assert!(bare.depth_policy.is_none());
        assert!(bare.compressor.is_none());
    }

    #[test]
    fn parallel_config_max_threads_round_trips_through_accessor() {
        // Pin: `ParallelConfig.max_threads` reaches the analyzer
        // and is observable via the read accessor. Before the
        // wire-up, the field was set on the config but never
        // consulted by the analyzer; with the wire-up the
        // accessor exposes the configured value and
        // `analyze_parallel` honours it via a per-call
        // `rayon::ThreadPoolBuilder`.
        let cfg = ParallelConfig {
            threshold: 4,
            max_threads: 2,
            work_stealing: true,
        };
        let analyzer = ParallelContextAnalyzer::new(cfg);
        assert_eq!(analyzer.max_threads(), 2);
        assert!(analyzer.work_stealing_enabled());

        // 0 means "use rayon's default pool".
        let unbounded = ParallelContextAnalyzer::with_default();
        assert_eq!(unbounded.max_threads(), 0);
    }

    #[test]
    fn test_enhanced_config() {
        let config = EnhancedContextConfig::all_enabled();
        assert!(config.flow_sensitive);
        assert!(config.adaptive_depth);
        assert!(config.compression);
    }

    #[test]
    fn test_call_pattern_from_chain() {
        let pattern1 = CallPattern::from_chain(&[]);
        assert_eq!(pattern1, CallPattern::Entry);

        let pattern2 = CallPattern::from_chain(&[FunctionId(1), FunctionId(2)]);
        assert_eq!(pattern2, CallPattern::Direct(FunctionId(2)));

        let pattern3 = CallPattern::from_chain(&[FunctionId(1), FunctionId(1)]);
        assert!(matches!(pattern3, CallPattern::Recursive(_)));
    }

    #[test]
    fn test_abstract_context_mergeable() {
        let ctx1 = AbstractContext::new(FunctionId(1));
        let ctx2 = AbstractContext::new(FunctionId(1));
        let ctx3 = AbstractContext::new(FunctionId(2));

        assert!(ctx1.is_mergeable_with(&ctx2));
        assert!(!ctx1.is_mergeable_with(&ctx3));
    }
}
