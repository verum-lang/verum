//! Escape Analysis Integration with CBGR Codegen
//!
//! When escape analysis proves NoEscape, CBGR codegen replaces ThinRef/FatRef
//! structures with direct pointers, eliminating the generation check against
//! AllocationHeader. This saves ~15ns per dereference on hot paths by bypassing
//! the atomic load + compare of generation and epoch fields.
//!
//! This module integrates escape analysis results with CBGR code generation,
//! enabling automatic elimination of CBGR checks for `NoEscape` references.
//!
//! # Purpose
//!
//! When escape analysis proves a reference doesn't escape (`NoEscape` state),
//! we can safely skip CBGR generation checks and use direct pointers instead,
//! achieving 0ns overhead instead of ~15ns.
//!
//! # Integration Points
//!
//! 1. **Reference Dereference**: Skip `cbgr_check()` for `NoEscape` refs
//! 2. **Reference Creation**: Use direct pointers instead of ThinRef/FatRef
//! 3. **Function Calls**: Pass raw pointers for `NoEscape` params
//! 4. **IDE Hints**: Show `[0ns - escape analysis]` for optimized refs
//!
//! # Safety
//!
//! All optimizations are conservative:
//! - Only apply when escape analysis **proves** safety
//! - Fall back to CBGR if any uncertainty
//! - Never compromise memory safety for performance

use crate::analysis::RefId;
use crate::escape_analysis::{EnhancedEscapeAnalyzer, EscapeState};
use verum_common::{Map, Maybe, Text};

/// Escape-aware codegen optimizer
///
/// This component consults escape analysis results during code generation
/// to eliminate unnecessary CBGR checks.
#[derive(Debug)]
pub struct EscapeAwareCodegen {
    /// Escape analyzer with analysis results
    analyzer: Maybe<EnhancedEscapeAnalyzer>,

    /// Optimization decisions cache
    optimization_cache: Map<RefId, OptimizationDecision>,

    /// Statistics
    stats: CodegenOptimizationStats,
}

/// Optimization decision for code generation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizationDecision {
    /// Skip CBGR checks (0ns) - reference proven `NoEscape`
    SkipCbgrChecks,

    /// Use CBGR checks (~15ns) - reference may escape
    UseCbgrChecks,

    /// Unknown - conservative fallback to CBGR
    Unknown,
}

impl OptimizationDecision {
    /// Check if can skip CBGR checks
    #[must_use]
    pub fn can_skip_checks(&self) -> bool {
        matches!(self, OptimizationDecision::SkipCbgrChecks)
    }

    /// Get expected overhead
    #[must_use]
    pub fn expected_overhead_ns(&self) -> u32 {
        match self {
            OptimizationDecision::SkipCbgrChecks => 0,
            OptimizationDecision::UseCbgrChecks => 15,
            OptimizationDecision::Unknown => 15,
        }
    }

    /// Get IDE hint text
    #[must_use]
    pub fn ide_hint(&self) -> &'static str {
        match self {
            OptimizationDecision::SkipCbgrChecks => "[0ns - escape analysis optimized]",
            OptimizationDecision::UseCbgrChecks => "[~15ns CBGR check]",
            OptimizationDecision::Unknown => "[~15ns CBGR check - unknown]",
        }
    }
}

/// Statistics for codegen optimizations
#[derive(Debug, Clone, Default)]
pub struct CodegenOptimizationStats {
    /// Total dereferences analyzed
    pub total_dereferences: u32,

    /// CBGR checks skipped (0ns)
    pub checks_skipped: u32,

    /// CBGR checks kept (~15ns)
    pub checks_kept: u32,

    /// Unknown decisions (conservative)
    pub unknown_decisions: u32,

    /// Estimated time saved per execution (nanoseconds)
    pub estimated_time_saved_ns: u64,
}

impl CodegenOptimizationStats {
    /// Calculate optimization rate
    #[must_use]
    pub fn optimization_rate(&self) -> f64 {
        if self.total_dereferences == 0 {
            0.0
        } else {
            (f64::from(self.checks_skipped) / f64::from(self.total_dereferences)) * 100.0
        }
    }

    /// Get total estimated savings
    #[must_use]
    pub fn total_savings_ns(&self) -> u64 {
        self.estimated_time_saved_ns
    }
}

impl EscapeAwareCodegen {
    /// Create new escape-aware codegen
    #[must_use]
    pub fn new() -> Self {
        Self {
            analyzer: Maybe::None,
            optimization_cache: Map::new(),
            stats: CodegenOptimizationStats::default(),
        }
    }

    /// Create with escape analyzer
    #[must_use]
    pub fn with_analyzer(analyzer: EnhancedEscapeAnalyzer) -> Self {
        Self {
            analyzer: Maybe::Some(analyzer),
            optimization_cache: Map::new(),
            stats: CodegenOptimizationStats::default(),
        }
    }

    /// Set escape analyzer
    pub fn set_analyzer(&mut self, analyzer: EnhancedEscapeAnalyzer) {
        self.analyzer = Maybe::Some(analyzer);
        self.optimization_cache.clear();
    }

    /// Check if CBGR checks can be skipped for a reference
    ///
    /// This is the main decision point during code generation.
    ///
    /// # Returns
    ///
    /// - `true`: Skip CBGR checks (0ns overhead)
    /// - `false`: Keep CBGR checks (~15ns overhead)
    pub fn can_skip_cbgr_check(&mut self, reference: RefId) -> bool {
        let decision = self.get_optimization_decision(reference);
        decision.can_skip_checks()
    }

    /// Get optimization decision for a reference
    ///
    /// Uses cached result if available, otherwise consults escape analyzer.
    pub fn get_optimization_decision(&mut self, reference: RefId) -> OptimizationDecision {
        // Check cache first
        if let Some(&cached) = self.optimization_cache.get(&reference) {
            return cached;
        }

        // Consult escape analyzer
        let decision = if let Maybe::Some(ref analyzer) = self.analyzer {
            match analyzer.get_state(reference) {
                Maybe::Some(EscapeState::NoEscape) => OptimizationDecision::SkipCbgrChecks,
                Maybe::Some(
                    EscapeState::Escapes | EscapeState::MayEscape | EscapeState::Unknown,
                ) => OptimizationDecision::UseCbgrChecks,
                Maybe::None => OptimizationDecision::Unknown,
            }
        } else {
            // No analyzer: conservative fallback
            OptimizationDecision::Unknown
        };

        // Cache decision
        self.optimization_cache.insert(reference, decision);

        // Update stats
        self.stats.total_dereferences += 1;
        match decision {
            OptimizationDecision::SkipCbgrChecks => {
                self.stats.checks_skipped += 1;
                self.stats.estimated_time_saved_ns += 15;
            }
            OptimizationDecision::UseCbgrChecks => {
                self.stats.checks_kept += 1;
            }
            OptimizationDecision::Unknown => {
                self.stats.unknown_decisions += 1;
                self.stats.checks_kept += 1;
            }
        }

        decision
    }

    /// Get IDE hint for reference dereference
    ///
    /// Provides user-facing cost transparency in IDE
    pub fn get_ide_hint(&mut self, reference: RefId) -> Text {
        let decision = self.get_optimization_decision(reference);
        decision.ide_hint().to_string().into()
    }

    /// Get expected overhead for reference dereference
    pub fn get_expected_overhead(&mut self, reference: RefId) -> u32 {
        let decision = self.get_optimization_decision(reference);
        decision.expected_overhead_ns()
    }

    /// Get optimization statistics
    #[must_use]
    pub fn statistics(&self) -> &CodegenOptimizationStats {
        &self.stats
    }

    /// Generate optimization report
    #[must_use]
    pub fn generate_report(&self) -> Text {
        let mut report = Text::new();

        report.push_str("=== CBGR Codegen Optimization Report ===\n\n");

        report.push_str(&format!(
            "Total dereferences:  {}\n",
            self.stats.total_dereferences
        ));
        report.push_str(&format!(
            "CBGR checks skipped: {} ({:.1}%)\n",
            self.stats.checks_skipped,
            self.stats.optimization_rate()
        ));
        report.push_str(&format!(
            "CBGR checks kept:    {}\n",
            self.stats.checks_kept
        ));
        report.push_str(&format!(
            "Unknown decisions:   {}\n\n",
            self.stats.unknown_decisions
        ));

        report.push_str(&format!(
            "Estimated time saved: ~{}ns per execution\n",
            self.stats.total_savings_ns()
        ));
        report.push_str(&format!(
            "                      ~{}μs per 1000 calls\n",
            self.stats.total_savings_ns() / 1000
        ));

        if self.stats.checks_skipped > 0 {
            report.push_str(&format!(
                "\n✅ {} references optimized to 0ns overhead\n",
                self.stats.checks_skipped
            ));
        }

        if self.stats.unknown_decisions > 0 {
            report.push_str(&format!(
                "\n⚠️  {} references have unknown escape state (conservative CBGR)\n",
                self.stats.unknown_decisions
            ));
            report.push_str("    Consider improving escape analysis for better optimization.\n");
        }

        report
    }
}

impl Default for EscapeAwareCodegen {
    fn default() -> Self {
        Self::new()
    }
}

/// LLVM IR generation helpers for escape-aware codegen
///
/// These functions integrate with the LLVM codegen pipeline to emit
/// optimized code based on escape analysis.
pub mod llvm_helpers {
    use super::{EscapeAwareCodegen, OptimizationDecision, RefId, Text};

    /// Generate CBGR check or skip based on escape analysis
    ///
    /// # Pseudocode
    ///
    /// ```text
    /// fn generate_reference_deref(ref_id, codegen):
    ///     if codegen.can_skip_cbgr_check(ref_id):
    ///         // NoEscape: Direct pointer access (0ns)
    ///         return emit_direct_load(ref_id)
    ///     else:
    ///         // Escapes: Full CBGR check (~15ns)
    ///         return emit_cbgr_deref(ref_id)
    /// ```
    pub fn should_emit_cbgr_check(codegen: &mut EscapeAwareCodegen, reference: RefId) -> bool {
        !codegen.can_skip_cbgr_check(reference)
    }

    /// Get optimization annotation for LLVM metadata
    ///
    /// This can be used to add optimization metadata to LLVM IR for
    /// better debugging and profiling.
    pub fn get_llvm_annotation(codegen: &mut EscapeAwareCodegen, reference: RefId) -> Text {
        let decision = codegen.get_optimization_decision(reference);

        match decision {
            OptimizationDecision::SkipCbgrChecks => {
                format!("cbgr.escape_analysis.no_escape ref_{}", reference.0).into()
            }
            OptimizationDecision::UseCbgrChecks => {
                format!("cbgr.escape_analysis.escapes ref_{}", reference.0).into()
            }
            OptimizationDecision::Unknown => {
                format!("cbgr.escape_analysis.unknown ref_{}", reference.0).into()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{BasicBlock, BlockId, ControlFlowGraph, DefSite};
    use crate::escape_analysis::EnhancedEscapeAnalyzer;
    use verum_common::{List, Set};

    fn create_test_analyzer_with_no_escape_ref(ref_id: RefId) -> EnhancedEscapeAnalyzer {
        let entry = BlockId(0);
        let exit = BlockId(1);

        let mut cfg = ControlFlowGraph::new(entry, exit);

        // Entry block
        let mut entry_block = BasicBlock {
            id: entry,
            predecessors: Set::new(),
            successors: Set::new(),
            definitions: vec![DefSite {
                block: entry,
                reference: ref_id,
                is_stack_allocated: true,
                span: None,
            }]
            .into(),
            uses: List::new(),
            call_sites: List::new(),
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
            definitions: List::new(),
            uses: List::new(),
            call_sites: List::new(),
            has_await_point: false,
            is_exception_handler: false,
            is_cleanup_handler: false,
            may_throw: false,
        };
        exit_block.predecessors.insert(entry);

        cfg.add_block(entry_block);
        cfg.add_block(exit_block);

        let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
        analyzer.analyze();

        analyzer
    }

    #[test]
    fn test_skip_cbgr_check_for_no_escape() {
        let ref_id = RefId(1);
        let analyzer = create_test_analyzer_with_no_escape_ref(ref_id);

        let mut codegen = EscapeAwareCodegen::with_analyzer(analyzer);

        // Should skip CBGR check for NoEscape reference
        assert!(codegen.can_skip_cbgr_check(ref_id));

        // Decision should be SkipCbgrChecks
        let decision = codegen.get_optimization_decision(ref_id);
        assert_eq!(decision, OptimizationDecision::SkipCbgrChecks);

        // Overhead should be 0ns
        assert_eq!(codegen.get_expected_overhead(ref_id), 0);
    }

    #[test]
    fn test_codegen_without_analyzer() {
        let mut codegen = EscapeAwareCodegen::new();

        let ref_id = RefId(2);

        // Without analyzer, should be conservative (use CBGR)
        assert!(!codegen.can_skip_cbgr_check(ref_id));

        let decision = codegen.get_optimization_decision(ref_id);
        assert_eq!(decision, OptimizationDecision::Unknown);
    }

    #[test]
    fn test_optimization_stats() {
        let ref_id = RefId(3);
        let analyzer = create_test_analyzer_with_no_escape_ref(ref_id);

        let mut codegen = EscapeAwareCodegen::with_analyzer(analyzer);

        // Make several optimization decisions
        codegen.can_skip_cbgr_check(ref_id);
        codegen.can_skip_cbgr_check(ref_id); // Cached
        codegen.can_skip_cbgr_check(RefId(99)); // Unknown

        let stats = codegen.statistics();

        assert_eq!(stats.total_dereferences, 2); // One cached, doesn't count twice
        assert_eq!(stats.checks_skipped, 1);
        assert!(stats.estimated_time_saved_ns > 0);
    }

    #[test]
    fn test_ide_hints() {
        let ref_id = RefId(4);
        let analyzer = create_test_analyzer_with_no_escape_ref(ref_id);

        let mut codegen = EscapeAwareCodegen::with_analyzer(analyzer);

        let hint = codegen.get_ide_hint(ref_id);
        assert!(hint.contains("0ns"));
        assert!(hint.contains("escape analysis"));
    }

    #[test]
    fn test_optimization_report() {
        let ref_id = RefId(5);
        let analyzer = create_test_analyzer_with_no_escape_ref(ref_id);

        let mut codegen = EscapeAwareCodegen::with_analyzer(analyzer);

        codegen.can_skip_cbgr_check(ref_id);

        let report = codegen.generate_report();

        assert!(report.contains("CBGR Codegen Optimization Report"));
        assert!(report.contains("references optimized"));
    }

    #[test]
    fn test_llvm_helpers() {
        let ref_id = RefId(6);
        let analyzer = create_test_analyzer_with_no_escape_ref(ref_id);

        let mut codegen = EscapeAwareCodegen::with_analyzer(analyzer);

        // Should NOT emit CBGR check for NoEscape
        assert!(!llvm_helpers::should_emit_cbgr_check(&mut codegen, ref_id));

        // Should have annotation
        let annotation = llvm_helpers::get_llvm_annotation(&mut codegen, ref_id);
        assert!(annotation.contains("no_escape"));
    }

    #[test]
    fn test_decision_caching() {
        let ref_id = RefId(7);
        let analyzer = create_test_analyzer_with_no_escape_ref(ref_id);

        let mut codegen = EscapeAwareCodegen::with_analyzer(analyzer);

        // First call
        let decision1 = codegen.get_optimization_decision(ref_id);

        // Second call should use cache
        let decision2 = codegen.get_optimization_decision(ref_id);

        assert_eq!(decision1, decision2);

        // Stats should only count once
        assert_eq!(codegen.stats.total_dereferences, 1);
    }
}
