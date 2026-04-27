//! Tier Analysis for VBC Integration
//!
//! This module provides the main tier analysis API that integrates:
//! - Escape analysis (does reference escape?)
//! - Dominance analysis (allocation dominates uses?)
//! - Async/exception path analysis
//!
//! The output is consumed by VBC codegen to emit tier-appropriate instructions.
//!
//! # Architecture
//!
//! ```text
//! CFG → TierAnalyzer → TierAnalysisResult
//!                           │
//!                           ▼
//!                   Map<RefId, ReferenceTier>
//!                           │
//!                           ▼
//!                   VBC Codegen (via to_vbc_tier())
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use verum_cbgr::tier_analysis::{TierAnalyzer, TierAnalysisResult};
//! use verum_cbgr::tier_types::{ReferenceTier, Tier0Reason};
//!
//! let analyzer = TierAnalyzer::new(cfg);
//! let result = analyzer.analyze();
//!
//! for (ref_id, tier) in &result.decisions {
//!     match tier {
//!         ReferenceTier::Tier1 => {
//!             // Emit RefChecked instruction
//!         }
//!         ReferenceTier::Tier0 { reason } => {
//!             // Emit Ref instruction with ChkRef
//!         }
//!         ReferenceTier::Tier2 => {
//!             // Emit RefUnsafe instruction
//!         }
//!     }
//! }
//! ```
//!
//! Integrates escape analysis results into VBC codegen via a unified API. Escape
//! analysis produces Map<ExprId, CbgrTier> decisions; VBC codegen uses these to emit
//! tier-appropriate instructions: Ref (Tier 0, full CBGR ~15ns), RefChecked (Tier 1,
//! compiler-verified 0ns), or RefUnsafe (Tier 2, manual safety). Default is Tier 0
//! if no analysis is available. Statistics track promotion rates and eliminated checks.

use crate::analysis::{BlockId, ControlFlowGraph, EffectInfo, RefId, Span};
use crate::concurrency_analysis::{ConcurrencyAnalysisResult, ConcurrencyAnalyzer};
use crate::dominance_analysis::{DominanceInfo, PromotionDecision as DomPromotionDecision};
use crate::escape_analysis::{EnhancedEscapeAnalyzer, EscapeAnalysisConfig};
use crate::lifetime_analysis::{LifetimeAnalysisResult, LifetimeAnalyzer};
use crate::nll_analysis::{NllAnalysisResult, NllAnalyzer};
use crate::ownership_analysis::{OwnershipAnalysisResult, OwnershipAnalyzer};
use crate::tier_types::{ReferenceTier, Tier0Reason, TierStatistics};
use verum_common::{Map, Set};

// ============================================================================
// Tier Analysis Result
// ============================================================================

/// Result of tier analysis for a function.
///
/// Contains tier decisions keyed by RefId, with optional span mapping for VBC codegen.
/// The span_map allows VBC codegen to look up tier decisions by source location.
///
/// VBC codegen uses ExprId-based lookup but escape analysis uses RefId internally.
/// The ref_to_span map bridges this: VBC codegen looks up tier decisions by source
/// span when direct RefId matching is unavailable, resolving the ExprId/RefId mismatch.
#[derive(Debug, Clone)]
pub struct TierAnalysisResult {
    /// Tier decisions for each reference.
    pub decisions: Map<RefId, ReferenceTier>,
    /// Analysis statistics.
    pub stats: TierStatistics,
    /// RefId to source span mapping for VBC codegen integration.
    /// When present, VBC codegen can use span-based ExprId lookup.
    pub ref_to_span: Map<RefId, Span>,
    /// Ownership analysis result (Phase 6).
    pub ownership: Option<OwnershipAnalysisResult>,
    /// Concurrency analysis result (Phase 6).
    pub concurrency: Option<ConcurrencyAnalysisResult>,
    /// Lifetime analysis result (Phase 7).
    pub lifetime: Option<LifetimeAnalysisResult>,
    /// NLL analysis result (Phase 8).
    pub nll: Option<NllAnalysisResult>,
}

impl TierAnalysisResult {
    /// Create empty result.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            decisions: Map::new(),
            stats: TierStatistics::default(),
            ref_to_span: Map::new(),
            ownership: None,
            concurrency: None,
            lifetime: None,
            nll: None,
        }
    }

    /// Create result with span mapping from CfgBuilder.
    #[must_use]
    pub fn with_span_map(
        decisions: Map<RefId, ReferenceTier>,
        stats: TierStatistics,
        ref_to_span: Map<RefId, Span>,
    ) -> Self {
        Self {
            decisions,
            stats,
            ref_to_span,
            ownership: None,
            concurrency: None,
            lifetime: None,
            nll: None,
        }
    }

    /// Check if there are any memory safety warnings.
    #[must_use]
    pub fn has_safety_warnings(&self) -> bool {
        // Check ownership warnings
        if let Some(ref own) = self.ownership {
            if !own.double_free_warnings.is_empty()
                || !own.use_after_free_warnings.is_empty()
                || !own.leak_warnings.is_empty()
            {
                return true;
            }
        }
        // Check concurrency warnings
        if let Some(ref conc) = self.concurrency {
            if conc.has_issues() {
                return true;
            }
        }
        // Check lifetime violations
        if let Some(ref lt) = self.lifetime {
            if lt.has_violations() {
                return true;
            }
        }
        // Check NLL violations
        if let Some(ref nll) = self.nll {
            if nll.has_violations() {
                return true;
            }
        }
        false
    }

    /// Get total warning count from all analyses.
    #[must_use]
    pub fn total_warnings(&self) -> usize {
        let mut count = 0;
        if let Some(ref own) = self.ownership {
            count += own.double_free_warnings.len()
                + own.use_after_free_warnings.len()
                + own.leak_warnings.len();
        }
        if let Some(ref conc) = self.concurrency {
            count += conc.warning_count();
        }
        if let Some(ref lt) = self.lifetime {
            count += lt.violations.len();
        }
        if let Some(ref nll) = self.nll {
            count += nll.violations.len();
        }
        count
    }

    /// Get tier decision for a reference, defaulting to Tier 0 if not found.
    #[must_use]
    pub fn get_tier(&self, ref_id: RefId) -> ReferenceTier {
        self.decisions
            .get(&ref_id)
            .cloned()
            .unwrap_or_else(|| ReferenceTier::tier0(Tier0Reason::NotAnalyzed))
    }

    /// Get tier decision by span (for VBC codegen integration).
    ///
    /// This is the preferred lookup method when source spans are available.
    /// Returns None if no mapping exists for the span.
    #[must_use]
    pub fn get_tier_by_span(&self, span: Span) -> Option<ReferenceTier> {
        // Search ref_to_span for matching span and look up tier
        for (ref_id, ref_span) in &self.ref_to_span {
            if *ref_span == span {
                return Some(self.get_tier(*ref_id));
            }
        }
        None
    }

    /// Get span for a RefId, if tracked.
    #[must_use]
    pub fn get_span(&self, ref_id: RefId) -> Option<Span> {
        self.ref_to_span.get(&ref_id).copied()
    }

    /// Check if a reference is promoted (Tier 1 or Tier 2).
    #[must_use]
    pub fn is_promoted(&self, ref_id: RefId) -> bool {
        self.get_tier(ref_id).is_promoted()
    }

    /// Get number of references analyzed.
    #[must_use]
    pub fn reference_count(&self) -> usize {
        self.decisions.len()
    }

    /// Check if span mapping is available.
    #[must_use]
    pub fn has_span_map(&self) -> bool {
        !self.ref_to_span.is_empty()
    }
}

// ============================================================================
// Tier Analyzer Configuration
// ============================================================================

/// Configuration for tier analysis.
#[derive(Debug, Clone)]
pub struct TierAnalysisConfig {
    /// Minimum confidence threshold for promotion (0.0-1.0).
    pub confidence_threshold: f64,
    /// Enable async boundary analysis.
    pub analyze_async_boundaries: bool,
    /// Enable exception path analysis.
    pub analyze_exception_paths: bool,
    /// Enable ownership analysis (double-free, UAF detection).
    pub enable_ownership_analysis: bool,
    /// Enable concurrency analysis (data race detection).
    pub enable_concurrency_analysis: bool,
    /// Enable lifetime analysis (borrow checking).
    pub enable_lifetime_analysis: bool,
    /// Enable NLL analysis (non-lexical lifetimes).
    pub enable_nll_analysis: bool,
    /// Maximum iterations for fixpoint computations in sub-analyzers.
    /// Propagated to escape, lifetime, and NLL analyzers to prevent infinite loops.
    pub max_iterations: usize,
    /// Wall-clock timeout in milliseconds for the entire tier analysis of one function.
    /// If exceeded, remaining phases are skipped and all refs default to Tier 0.
    pub timeout_ms: u64,
}

impl Default for TierAnalysisConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: 0.95,
            analyze_async_boundaries: true,
            analyze_exception_paths: true,
            enable_ownership_analysis: true,
            enable_concurrency_analysis: true,
            enable_lifetime_analysis: true,
            enable_nll_analysis: true,
            max_iterations: 1000,
            timeout_ms: 5000,
        }
    }
}

impl TierAnalysisConfig {
    /// Create minimal configuration (fastest, least analysis).
    /// Only runs escape analysis and dominance — no ownership, concurrency,
    /// lifetime, or NLL. Suitable for test execution and rapid iteration.
    #[must_use]
    pub fn minimal() -> Self {
        Self {
            confidence_threshold: 0.95,
            analyze_async_boundaries: false,
            analyze_exception_paths: false,
            enable_ownership_analysis: false,
            enable_concurrency_analysis: false,
            enable_lifetime_analysis: false,
            enable_nll_analysis: false,
            max_iterations: 100,
            timeout_ms: 2000,
        }
    }

    /// Create full configuration (slowest, most thorough).
    #[must_use]
    pub fn full() -> Self {
        Self::default()
    }

    /// Enable only safety-critical analyses.
    #[must_use]
    pub fn safety_only() -> Self {
        Self {
            confidence_threshold: 0.99,
            analyze_async_boundaries: true,
            analyze_exception_paths: true,
            enable_ownership_analysis: true,
            enable_concurrency_analysis: true,
            enable_lifetime_analysis: false,
            enable_nll_analysis: false,
            max_iterations: 1000,
            timeout_ms: 5000,
        }
    }
}

// ============================================================================
// Tier Analyzer
// ============================================================================

/// Tier analyzer that produces tier decisions for VBC codegen.
///
/// This is the main entry point for tier analysis. It coordinates:
/// 1. Escape analysis (does reference escape?)
/// 2. Dominance analysis (allocation dominates uses?)
/// 3. Additional safety checks (async, exceptions)
#[derive(Debug)]
pub struct TierAnalyzer {
    /// Control flow graph.
    cfg: ControlFlowGraph,
    /// Configuration.
    config: TierAnalysisConfig,
    /// Effect information (for async analysis).
    effect_info: Option<EffectInfo>,
}

impl TierAnalyzer {
    /// Create a new tier analyzer.
    #[must_use]
    pub fn new(cfg: ControlFlowGraph) -> Self {
        Self {
            cfg,
            config: TierAnalysisConfig::default(),
            effect_info: None,
        }
    }

    /// Create with custom configuration.
    #[must_use]
    pub fn with_config(cfg: ControlFlowGraph, config: TierAnalysisConfig) -> Self {
        Self {
            cfg,
            config,
            effect_info: None,
        }
    }

    /// Set effect information for async analysis.
    #[must_use]
    pub fn with_effect_info(mut self, effect_info: EffectInfo) -> Self {
        self.effect_info = Some(effect_info);
        self
    }

    /// Run tier analysis and produce decisions.
    pub fn analyze(&self) -> TierAnalysisResult {
        use std::time::Instant;

        let start = Instant::now();
        let timeout = std::time::Duration::from_millis(self.config.timeout_ms);
        let mut decisions = Map::new();
        let mut stats = TierStatistics::new();
        let mut ref_to_span = Map::new();

        // Phase 1: Collect all references and build span map
        let all_refs = self.collect_references_with_spans(&mut ref_to_span);

        if all_refs.is_empty() {
            return TierAnalysisResult::empty();
        }

        // Helper: check if wall-clock timeout exceeded
        let timed_out = |start: Instant, timeout: std::time::Duration| -> bool {
            self.config.timeout_ms > 0 && start.elapsed() > timeout
        };

        // Phase 2: Run escape analysis (with propagated max_iterations)
        let mut escape_config = EscapeAnalysisConfig::default();
        escape_config.max_iterations = self.config.max_iterations;
        let mut escape_analyzer = EnhancedEscapeAnalyzer::new(self.cfg.clone());
        escape_analyzer.set_config(escape_config);
        escape_analyzer.analyze();

        if timed_out(start, timeout) {
            return self.fallback_all_tier0(all_refs, ref_to_span, start);
        }

        // Phase 3: Run dominance analysis
        let dominance_info = DominanceInfo::compute(&self.cfg);

        if timed_out(start, timeout) {
            return self.fallback_all_tier0(all_refs, ref_to_span, start);
        }

        // Phase 4: Compute async boundary info if needed
        let async_info = if self.config.analyze_async_boundaries && !timed_out(start, timeout) {
            self.effect_info.as_ref().map(|ei| {
                crate::promotion_decision::AsyncBoundaryInfo::compute(&self.cfg, ei)
            })
        } else {
            None
        };

        // Phase 5: Compute exception path info if needed
        let exception_info = if self.config.analyze_exception_paths && !timed_out(start, timeout) {
            Some(crate::promotion_decision::ExceptionPathInfo::compute(
                &self.cfg,
            ))
        } else {
            None
        };

        // Phase 6: Run ownership analysis (double-free, use-after-free detection)
        let ownership_result = if self.config.enable_ownership_analysis && !timed_out(start, timeout) {
            let ownership_analyzer = OwnershipAnalyzer::new(self.cfg.clone());
            Some(ownership_analyzer.analyze())
        } else {
            None
        };

        // Phase 7: Run concurrency analysis (data race detection)
        let concurrency_result = if self.config.enable_concurrency_analysis && !timed_out(start, timeout) {
            let concurrency_analyzer = ConcurrencyAnalyzer::new(self.cfg.clone());
            Some(concurrency_analyzer.analyze())
        } else {
            None
        };

        // Phase 8: Run lifetime analysis (borrow checking, with propagated max_iterations)
        let lifetime_result = if self.config.enable_lifetime_analysis && !timed_out(start, timeout) {
            let mut lt_config = crate::lifetime_analysis::LifetimeAnalysisConfig::default();
            lt_config.max_iterations = self.config.max_iterations;
            let lifetime_analyzer = LifetimeAnalyzer::new(self.cfg.clone()).with_config(lt_config);
            Some(lifetime_analyzer.analyze())
        } else {
            None
        };

        // Phase 9: Run NLL analysis (non-lexical lifetimes, with propagated max_iterations)
        let nll_result = if self.config.enable_nll_analysis && !timed_out(start, timeout) {
            let mut nll_config = crate::nll_analysis::NllConfig::default();
            nll_config.max_iterations = self.config.max_iterations;
            let nll_analyzer = NllAnalyzer::new(self.cfg.clone()).with_config(nll_config);
            Some(nll_analyzer.analyze())
        } else {
            None
        };

        // Phase 10: Make tier decisions for each reference
        for ref_id in all_refs {
            let tier = self.decide_tier_enhanced(
                ref_id,
                &escape_analyzer,
                &dominance_info,
                async_info.as_ref(),
                exception_info.as_ref(),
                ownership_result.as_ref(),
                lifetime_result.as_ref(),
                nll_result.as_ref(),
            );

            // Record in statistics
            stats.record(&tier);

            decisions.insert(ref_id, tier);
        }

        stats.functions_analyzed = 1;
        stats.analysis_duration_us = start.elapsed().as_micros() as u64;

        TierAnalysisResult {
            decisions,
            stats,
            ref_to_span,
            ownership: ownership_result,
            concurrency: concurrency_result,
            lifetime: lifetime_result,
            nll: nll_result,
        }
    }

    /// Fallback: assign all references to Tier 0 when analysis times out.
    /// This is the safe default — Tier 0 has full CBGR runtime checks (~15ns).
    fn fallback_all_tier0(
        &self,
        all_refs: Set<RefId>,
        ref_to_span: Map<RefId, Span>,
        start: std::time::Instant,
    ) -> TierAnalysisResult {
        let mut decisions = Map::new();
        let mut stats = TierStatistics::new();
        for ref_id in all_refs {
            let tier = ReferenceTier::tier0(Tier0Reason::AnalysisTimeout);
            stats.record(&tier);
            decisions.insert(ref_id, tier);
        }
        stats.functions_analyzed = 1;
        stats.analysis_duration_us = start.elapsed().as_micros() as u64;
        TierAnalysisResult {
            decisions,
            stats,
            ref_to_span,
            ownership: None,
            concurrency: None,
            lifetime: None,
            nll: None,
        }
    }

    /// Collect all reference IDs from the CFG.
    fn collect_references(&self) -> Set<RefId> {
        let mut refs = Set::new();

        for (_block_id, block) in &self.cfg.blocks {
            for def in &block.definitions {
                refs.insert(def.reference);
            }
            for use_site in &block.uses {
                refs.insert(use_site.reference);
            }
        }

        refs
    }

    /// Collect all reference IDs from the CFG and build span mapping.
    ///
    /// This is the preferred method for VBC codegen integration as it
    /// extracts span information from DefSite/UseeSite for later lookup.
    fn collect_references_with_spans(&self, ref_to_span: &mut Map<RefId, Span>) -> Set<RefId> {
        let mut refs = Set::new();

        for (_block_id, block) in &self.cfg.blocks {
            for def in &block.definitions {
                refs.insert(def.reference);
                // Record span if available (prefer definition site span)
                if let Some(span) = def.span {
                    ref_to_span.insert(def.reference, span);
                }
            }
            for use_site in &block.uses {
                refs.insert(use_site.reference);
                // Only add span if not already recorded from definition
                if !ref_to_span.contains_key(&use_site.reference) {
                    if let Some(span) = use_site.span {
                        ref_to_span.insert(use_site.reference, span);
                    }
                }
            }
        }

        refs
    }

    /// Decide tier for a single reference with all analyses (enhanced version).
    fn decide_tier_enhanced(
        &self,
        ref_id: RefId,
        escape_analyzer: &EnhancedEscapeAnalyzer,
        dominance_info: &DominanceInfo,
        async_info: Option<&crate::promotion_decision::AsyncBoundaryInfo>,
        exception_info: Option<&crate::promotion_decision::ExceptionPathInfo>,
        ownership_result: Option<&OwnershipAnalysisResult>,
        lifetime_result: Option<&LifetimeAnalysisResult>,
        nll_result: Option<&NllAnalysisResult>,
    ) -> ReferenceTier {
        use crate::dominance_analysis::decide_promotion;
        use crate::escape_analysis::EscapeState;

        // Check ownership analysis warnings (highest priority)
        if let Some(ownership) = ownership_result {
            // If reference has use-after-free warning, keep at Tier 0
            for uaf in &ownership.use_after_free_warnings {
                if uaf.ref_id == ref_id {
                    return ReferenceTier::tier0(Tier0Reason::UseAfterFree);
                }
            }
        }

        // Check lifetime analysis violations
        if let Some(lifetime) = lifetime_result {
            for violation in &lifetime.violations {
                if violation.ref_id == ref_id {
                    return ReferenceTier::tier0(Tier0Reason::LifetimeViolation);
                }
            }
        }

        // Check NLL analysis violations
        if let Some(nll) = nll_result {
            for _violation in &nll.violations {
                // NLL violations affect all refs in the function (conservative)
                // In real impl, would check specific ref involvement
                if !nll.violations.is_empty() {
                    return ReferenceTier::tier0(Tier0Reason::BorrowViolation);
                }
            }
        }

        // Get escape state (default to Unknown if not found)
        let escape_state = escape_analyzer
            .get_state(ref_id)
            .unwrap_or(EscapeState::Unknown);

        // Check async boundaries
        if let Some(ai) = async_info {
            if ai.is_async_function {
                // References used across await points can't be promoted
                if self.crosses_async_boundary(ref_id, ai) {
                    return ReferenceTier::tier0(Tier0Reason::AsyncBoundary);
                }
            }
        }

        // Check exception paths
        if let Some(ei) = exception_info {
            if self.on_exception_path(ref_id, ei) {
                return ReferenceTier::tier0(Tier0Reason::ExceptionPath);
            }
        }

        // Convert escape state to escape category
        let escape_category = self.escape_state_to_category(&escape_state);

        // Use dominance analysis for final decision
        let ref_info = crate::dominance_analysis::ReferenceInfo {
            ref_id,
            allocation_site: self.find_definition_block(ref_id),
            use_sites: self.find_use_blocks(ref_id),
            is_stack_allocated: true, // Conservative assumption
            escape_category,
        };

        let dom_decision = decide_promotion(&ref_info, dominance_info);

        // Map dominance decision to tier
        match dom_decision {
            DomPromotionDecision::PromoteToChecked => ReferenceTier::tier1(),
            DomPromotionDecision::KeepManagedEscape => {
                ReferenceTier::tier0(Tier0Reason::Escapes)
            }
            DomPromotionDecision::KeepManagedDominance => {
                ReferenceTier::tier0(Tier0Reason::DominanceFailure)
            }
            DomPromotionDecision::KeepManagedConservative => {
                ReferenceTier::tier0(Tier0Reason::Conservative)
            }
        }
    }

    /// Check if a reference crosses async boundaries.
    fn crosses_async_boundary(
        &self,
        ref_id: RefId,
        async_info: &crate::promotion_decision::AsyncBoundaryInfo,
    ) -> bool {
        let def_block = self.find_definition_block(ref_id);
        let use_blocks = self.find_use_blocks(ref_id);

        // If defined before await and used after, it crosses
        for use_block in use_blocks {
            if async_info.is_continuation_block(use_block)
                && !async_info.is_continuation_block(def_block)
            {
                return true;
            }
        }

        false
    }

    /// Check if a reference is on an exception path.
    fn on_exception_path(
        &self,
        ref_id: RefId,
        exception_info: &crate::promotion_decision::ExceptionPathInfo,
    ) -> bool {
        let use_blocks = self.find_use_blocks(ref_id);

        for use_block in use_blocks {
            if exception_info.is_handler_block(use_block)
                || exception_info.is_cleanup_block(use_block)
            {
                return true;
            }
        }

        false
    }

    /// Find the block where a reference is defined.
    fn find_definition_block(&self, ref_id: RefId) -> BlockId {
        for (block_id, block) in &self.cfg.blocks {
            for def in &block.definitions {
                if def.reference == ref_id {
                    return *block_id;
                }
            }
        }
        self.cfg.entry // Default to entry block
    }

    /// Find all blocks where a reference is used.
    fn find_use_blocks(&self, ref_id: RefId) -> Set<BlockId> {
        let mut blocks = Set::new();

        for (block_id, block) in &self.cfg.blocks {
            for use_site in &block.uses {
                if use_site.reference == ref_id {
                    blocks.insert(*block_id);
                }
            }
        }

        blocks
    }

    /// Convert escape state to escape category for dominance analysis.
    fn escape_state_to_category(
        &self,
        escape_state: &crate::escape_analysis::EscapeState,
    ) -> crate::dominance_analysis::EscapeCategory {
        use crate::dominance_analysis::EscapeCategory;
        use crate::escape_analysis::EscapeState;

        match escape_state {
            EscapeState::NoEscape => EscapeCategory::NoEscape,
            EscapeState::MayEscape => EscapeCategory::MayEscape,
            EscapeState::Escapes => EscapeCategory::Escapes,
            EscapeState::Unknown => EscapeCategory::Unknown,
        }
    }
}

// ============================================================================
// Convenience Functions
// ============================================================================

/// Analyze a function and produce tier decisions.
///
/// This is a convenience function for simple use cases.
///
/// # Example
///
/// ```rust,ignore
/// let result = analyze_tiers(&cfg);
/// for (ref_id, tier) in &result.decisions {
///     println!("{:?}: {}", ref_id, tier);
/// }
/// ```
pub fn analyze_tiers(cfg: &ControlFlowGraph) -> TierAnalysisResult {
    TierAnalyzer::new(cfg.clone()).analyze()
}

/// Analyze with custom configuration.
pub fn analyze_tiers_with_config(
    cfg: &ControlFlowGraph,
    config: TierAnalysisConfig,
) -> TierAnalysisResult {
    TierAnalyzer::with_config(cfg.clone(), config).analyze()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{BasicBlock, DefSite, UseeSite};

    fn make_simple_cfg() -> ControlFlowGraph {
        let entry = BlockId(0);
        let exit = BlockId(1);
        let mut cfg = ControlFlowGraph::new(entry, exit);

        // Entry block with definition
        let mut entry_successors = Set::new();
        entry_successors.insert(exit);

        let mut entry_block = BasicBlock {
            id: entry,
            predecessors: Set::new(),
            successors: entry_successors,
            definitions: verum_common::List::new(),
            uses: verum_common::List::new(),
            call_sites: verum_common::List::new(),
            has_await_point: false,
            is_exception_handler: false,
            is_cleanup_handler: false,
            may_throw: false,
        };
        entry_block.definitions.push(DefSite {
            block: entry,
            reference: RefId(0),
            is_stack_allocated: true,
            span: None, // Test CFG, no real source span
        });
        cfg.add_block(entry_block);

        // Exit block with use
        let mut exit_predecessors = Set::new();
        exit_predecessors.insert(entry);

        let mut exit_block = BasicBlock {
            id: exit,
            predecessors: exit_predecessors,
            successors: Set::new(),
            definitions: verum_common::List::new(),
            uses: verum_common::List::new(),
            call_sites: verum_common::List::new(),
            has_await_point: false,
            is_exception_handler: false,
            is_cleanup_handler: false,
            may_throw: false,
        };
        exit_block.uses.push(UseeSite {
            block: exit,
            reference: RefId(0),
            is_mutable: false,
            span: None, // Test CFG, no real source span
        });
        cfg.add_block(exit_block);

        cfg
    }

    #[test]
    fn test_tier_analysis_config_default() {
        let config = TierAnalysisConfig::default();
        assert_eq!(config.confidence_threshold, 0.95);
        assert!(config.analyze_async_boundaries);
        assert!(config.analyze_exception_paths);
    }

    #[test]
    fn test_empty_cfg() {
        let entry = BlockId(0);
        let exit = BlockId(0);
        let cfg = ControlFlowGraph::new(entry, exit);

        let result = analyze_tiers(&cfg);
        assert!(result.decisions.is_empty());
        assert_eq!(result.stats.total_refs, 0);
    }

    #[test]
    fn test_simple_cfg_analysis() {
        let cfg = make_simple_cfg();
        let result = analyze_tiers(&cfg);

        assert_eq!(result.stats.total_refs, 1);
        // Simple local reference - check tier is valid
        let tier = result.get_tier(RefId(0));
        assert!(tier.tier_number() <= 2);
    }

    #[test]
    fn test_tier_result_get_tier_default() {
        let result = TierAnalysisResult::empty();
        let tier = result.get_tier(RefId(999));

        // Should default to Tier 0 with NotAnalyzed reason
        assert_eq!(tier.tier_number(), 0);
        assert_eq!(tier.reason(), Some(&Tier0Reason::NotAnalyzed));
    }

    #[test]
    fn test_tier_result_is_promoted() {
        let mut result = TierAnalysisResult::empty();
        result.decisions.insert(RefId(0), ReferenceTier::tier1());
        result.decisions.insert(RefId(1), ReferenceTier::tier0(Tier0Reason::Escapes));

        assert!(result.is_promoted(RefId(0)));
        assert!(!result.is_promoted(RefId(1)));
    }
}
