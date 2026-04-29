//! CBGR Integration with Compiler Optimization Passes
//!
//! This module integrates CBGR tier analysis into the compiler's optimization pipeline.
//! It runs after MIR lowering and applies automatic reference tier transformations
//! based on proven safety properties from escape analysis.
//!
//! # Pipeline Integration
//!
//! ```text
//! MIR Lowering → CBGR Tier Analysis → Other Optimizations → VBC Codegen
//!                      ↓
//!             ┌────────┴────────┐
//!             │  Escape Analysis │
//!             │  Tier Decisions  │
//!             │  Statistics      │
//!             └─────────────────┘
//! ```
//!
//! # Performance Impact
//!
//! - Analysis overhead: < 50ms per function
//! - Promotion rate: 40-70% typical
//! - Time saved: ~15ns per promoted reference dereference
//!
//! CBGR-VBC integration: inserts generation counter checks into VBC bytecode
//! for memory safety validation. Three reference tiers:
//!   - &T (managed): Runtime CBGR check ~15ns per dereference. ThinRef 16 bytes, FatRef 24 bytes.
//!     Uses epoch-based generation tracking with acquire-release memory ordering.
//!   - &checked T (compiler-proven): 0ns overhead, direct pointer (8 bytes). Requires static
//!     proof via escape analysis or explicit verification that the reference is safe.
//!   - &unsafe T (manual proof): 0ns overhead, direct pointer (8 bytes). Must be within
//!     @unsafe function. No runtime checks, programmer responsibility.
//!     Escape analysis can promote &T to &checked T when safety is provable (40-70% typical rate).

use std::time::Instant;
use verum_cbgr::analysis::{
    BasicBlock as CbgrBasicBlock, BlockId as CbgrBlockId, ControlFlowGraph, DefSite, RefId,
    UseeSite,
};
use verum_cbgr::tier_analysis::{TierAnalysisResult, TierAnalyzer};
use verum_cbgr::tier_types::{ReferenceTier, TierStatistics};
use verum_common::{List, Map, Set, Text};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};

// Import production MIR types from phases/mir_lowering
use crate::phases::mir_lowering::{
    MetadataKind, MirFunction, MirModule, MirStatement, MirType, Operand, ReferenceLayout, Rvalue,
    Terminator,
};

/// CBGR optimization pass configuration
#[derive(Debug, Clone)]
pub struct CbgrPassConfig {
    /// Minimum confidence threshold for promotion (0.0-1.0)
    pub confidence_threshold: f64,
    /// Enable detailed statistics reporting
    pub detailed_stats: bool,
    /// Enable diagnostic messages for promotion decisions
    pub emit_diagnostics: bool,
    /// Maximum analysis time per function (milliseconds)
    pub max_analysis_time_ms: u64,
}

impl Default for CbgrPassConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: 0.95,
            detailed_stats: true,
            emit_diagnostics: false,
            max_analysis_time_ms: 1000,
        }
    }
}

/// CBGR optimization pass for automatic reference tier promotion
///
/// This pass analyzes references in MIR functions and automatically promotes
/// them from `&T` (CBGR-managed, ~15ns overhead) to `&checked T` (0ns overhead)
/// when escape analysis proves safety.
///
/// # Algorithm
///
/// 1. Build CFG from MIR function
/// 2. Run tier analysis (escape + dominance)
/// 3. Apply tier decisions to MIR
/// 4. Generate statistics
///
/// # Example
///
/// ```rust,ignore
/// use verum_compiler::passes::cbgr_integration::CbgrOptimizationPass;
///
/// let mut pass = CbgrOptimizationPass::new(CbgrPassConfig::default());
/// let result = pass.run_on_function(&mut mir_function)?;
///
/// println!("Promoted {} references", result.stats.tier1_count);
/// ```
pub struct CbgrOptimizationPass {
    /// Configuration
    config: CbgrPassConfig,
    /// Accumulated statistics across all functions
    total_stats: CbgrPassStatistics,
    /// Per-function tier decisions
    decisions: Map<Text, Map<RefId, ReferenceTier>>,
}

impl CbgrOptimizationPass {
    /// Create new CBGR optimization pass
    pub fn new(config: CbgrPassConfig) -> Self {
        Self {
            config,
            total_stats: CbgrPassStatistics::default(),
            decisions: Map::new(),
        }
    }

    /// Run the CBGR optimization pass on a single function
    ///
    /// # Returns
    ///
    /// - `Ok(PassResult)`: Success with tier analysis statistics
    /// - `Err(List<Diagnostic>)`: Analysis failed with diagnostics
    pub fn run_on_function(
        &mut self,
        func: &mut MirFunction,
    ) -> Result<PassResult, List<Diagnostic>> {
        let start = Instant::now();

        // Build CFG from MIR function
        let cfg = self.build_cfg(func)?;

        // Create tier analyzer with config (full analysis enabled)
        let tier_config = verum_cbgr::tier_analysis::TierAnalysisConfig {
            confidence_threshold: self.config.confidence_threshold,
            analyze_async_boundaries: true,
            analyze_exception_paths: true,
            enable_ownership_analysis: true,
            enable_concurrency_analysis: true,
            enable_lifetime_analysis: true,
            enable_nll_analysis: true,
            max_iterations: 1000,
            timeout_ms: 5000,
        };
        let analyzer = TierAnalyzer::with_config(cfg, tier_config);

        // Run tier analysis
        let result = analyzer.analyze();

        // Apply tier decisions to MIR
        self.apply_tier_decisions(func, &result)?;

        // Store decisions
        self.decisions
            .insert(func.name.clone(), result.decisions.clone());

        // Update total statistics
        self.total_stats.functions_analyzed += 1;
        self.total_stats.total_promotions += result.stats.tier1_count;
        self.total_stats.total_references += result.stats.total_refs;
        self.total_stats.total_time_saved_ns += result.stats.estimated_savings_ns;

        let duration = start.elapsed();

        // Check timeout
        if duration.as_millis() as u64 > self.config.max_analysis_time_ms {
            let warning = DiagnosticBuilder::new(Severity::Warning)
                .message(format!(
                    "CBGR analysis for '{}' took {}ms (limit: {}ms)",
                    func.name,
                    duration.as_millis(),
                    self.config.max_analysis_time_ms
                ))
                .help("Consider simplifying function or increasing timeout")
                .build();

            return Ok(PassResult {
                stats: result.stats,
                duration_ms: duration.as_millis() as u64,
                warnings: List::from(vec![warning]),
            });
        }

        Ok(PassResult {
            stats: result.stats,
            duration_ms: duration.as_millis() as u64,
            warnings: List::new(),
        })
    }

    /// Run the pass on all functions in a module
    pub fn run_on_module(
        &mut self,
        module: &mut MirModule,
    ) -> Result<ModulePassResult, List<Diagnostic>> {
        let mut all_warnings = List::new();
        let mut function_results = Map::new();

        for func in module.functions.iter_mut() {
            match self.run_on_function(func) {
                Ok(result) => {
                    all_warnings.extend(result.warnings.clone());
                    // Honour `detailed_stats`: when false, suppress
                    // the per-function map and ship only the
                    // aggregate `total_stats`.  Pre-fix the flag was
                    // a config field with no readers — every caller
                    // got the full per-function map regardless of
                    // whether they asked for it, paying the
                    // per-function entry cost in the returned Map
                    // unconditionally.
                    if self.config.detailed_stats {
                        function_results.insert(func.name.clone(), result);
                    }
                }
                Err(diagnostics) => {
                    // Continue with other functions even if one fails
                    all_warnings.extend(diagnostics);
                }
            }
        }

        Ok(ModulePassResult {
            function_results,
            total_stats: self.total_stats.clone(),
            warnings: all_warnings,
        })
    }

    /// Build control flow graph from MIR function
    fn build_cfg(&self, func: &MirFunction) -> Result<ControlFlowGraph, List<Diagnostic>> {
        // Build CFG from MIR basic blocks
        let entry = CbgrBlockId(func.entry_block.0 as u64);

        // Find exit blocks (blocks that return or are unreachable)
        let exit_block_id = func
            .blocks
            .iter()
            .find(|b| matches!(b.terminator, Terminator::Return | Terminator::Unreachable))
            .map(|b| CbgrBlockId(b.id.0 as u64))
            .unwrap_or(CbgrBlockId(func.blocks.len() as u64));

        let mut cfg = ControlFlowGraph::new(entry, exit_block_id);

        // Build BasicBlocks with definitions and uses
        for block in func.blocks.iter() {
            let block_id = CbgrBlockId(block.id.0 as u64);

            // Build predecessors and successors sets
            let mut predecessors = Set::new();
            for p in block.predecessors.iter() {
                predecessors.insert(CbgrBlockId(p.0 as u64));
            }

            let mut successors = Set::new();
            for s in block.successors.iter() {
                successors.insert(CbgrBlockId(s.0 as u64));
            }

            // Collect definitions and uses
            let mut definitions = List::new();
            let mut uses = List::new();

            // Extract reference definitions and uses from statements
            for stmt in block.statements.iter() {
                match stmt {
                    MirStatement::Assign(place, rvalue) => {
                        // LHS is a definition
                        if place.projections.is_empty() {
                            definitions.push(DefSite {
                                block: block_id,
                                reference: RefId(place.local.0 as u64),
                                is_stack_allocated: true,
                                span: None,
                            });
                        }
                        // Collect uses from RHS
                        self.collect_rvalue_uses(&mut uses, block_id, rvalue);
                    }
                    MirStatement::GenerationCheck(place)
                    | MirStatement::EpochCheck(place)
                    | MirStatement::Drop(place)
                    | MirStatement::DropInPlace(place) => {
                        if place.projections.is_empty() {
                            uses.push(UseeSite {
                                block: block_id,
                                reference: RefId(place.local.0 as u64),
                                is_mutable: false,
                                span: None,
                            });
                        }
                    }
                    MirStatement::CapabilityCheck { place, .. } => {
                        if place.projections.is_empty() {
                            uses.push(UseeSite {
                                block: block_id,
                                reference: RefId(place.local.0 as u64),
                                is_mutable: false,
                                span: None,
                            });
                        }
                    }
                    MirStatement::Retag { place, .. } => {
                        if place.projections.is_empty() {
                            definitions.push(DefSite {
                                block: block_id,
                                reference: RefId(place.local.0 as u64),
                                is_stack_allocated: true,
                                span: None,
                            });
                        }
                    }
                    _ => {}
                }
            }

            // Extract uses from terminator
            match &block.terminator {
                Terminator::Call { args, .. } | Terminator::AsyncCall { args, .. } => {
                    for arg in args.iter() {
                        if let Operand::Copy(place) | Operand::Move(place) = arg {
                            if place.projections.is_empty() {
                                uses.push(UseeSite {
                                    block: block_id,
                                    reference: RefId(place.local.0 as u64),
                                    is_mutable: false,
                                    span: None,
                                });
                            }
                        }
                    }
                }
                Terminator::Branch { condition, .. } => {
                    if let Operand::Copy(place) | Operand::Move(place) = condition {
                        if place.projections.is_empty() {
                            uses.push(UseeSite {
                                block: block_id,
                                reference: RefId(place.local.0 as u64),
                                is_mutable: false,
                                span: None,
                            });
                        }
                    }
                }
                _ => {}
            }

            // Create and add the basic block
            let cbgr_block = CbgrBasicBlock {
                id: block_id,
                predecessors,
                successors,
                definitions,
                uses,
                call_sites: List::new(),
                has_await_point: false,
                is_exception_handler: false,
                is_cleanup_handler: false,
                may_throw: false,
            };
            cfg.add_block(cbgr_block);
        }

        Ok(cfg)
    }

    /// Collect reference uses from an rvalue
    fn collect_rvalue_uses(
        &self,
        uses: &mut List<UseeSite>,
        block_id: CbgrBlockId,
        rvalue: &Rvalue,
    ) {
        match rvalue {
            Rvalue::Use(operand)
            | Rvalue::Unary(_, operand)
            | Rvalue::Cast(_, operand, _)
            | Rvalue::ShallowInitBox(operand, _)
            | Rvalue::Repeat(operand, _) => {
                if let Operand::Copy(place) | Operand::Move(place) = operand {
                    if place.projections.is_empty() {
                        uses.push(UseeSite {
                            block: block_id,
                            reference: RefId(place.local.0 as u64),
                            is_mutable: false,
                            span: None,
                        });
                    }
                }
            }
            Rvalue::Binary(_, left, right) | Rvalue::CheckedBinary(_, left, right) => {
                for operand in [left, right] {
                    if let Operand::Copy(place) | Operand::Move(place) = operand {
                        if place.projections.is_empty() {
                            uses.push(UseeSite {
                                block: block_id,
                                reference: RefId(place.local.0 as u64),
                                is_mutable: false,
                                span: None,
                            });
                        }
                    }
                }
            }
            Rvalue::Ref(_, place) | Rvalue::Deref(place) | Rvalue::AddressOf(_, place) => {
                if place.projections.is_empty() {
                    uses.push(UseeSite {
                        block: block_id,
                        reference: RefId(place.local.0 as u64),
                        is_mutable: false,
                        span: None,
                    });
                }
            }
            Rvalue::Aggregate(_, operands) => {
                for operand in operands.iter() {
                    if let Operand::Copy(place) | Operand::Move(place) = operand {
                        if place.projections.is_empty() {
                            uses.push(UseeSite {
                                block: block_id,
                                reference: RefId(place.local.0 as u64),
                                is_mutable: false,
                                span: None,
                            });
                        }
                    }
                }
            }
            Rvalue::Discriminant(place) | Rvalue::Len(place) | Rvalue::CopyForDeref(place) => {
                if place.projections.is_empty() {
                    uses.push(UseeSite {
                        block: block_id,
                        reference: RefId(place.local.0 as u64),
                        is_mutable: false,
                        span: None,
                    });
                }
            }
            Rvalue::NullConstant | Rvalue::ThreadLocalRef(_) => {}
        }
    }

    /// Apply tier decisions to MIR
    ///
    /// Updates reference layouts based on tier analysis:
    /// - Tier 1: Mark as promoted (CBGR checks eliminated)
    /// - Tier 0: Keep ThinRef with CBGR validation
    fn apply_tier_decisions(
        &self,
        func: &mut MirFunction,
        result: &TierAnalysisResult,
    ) -> Result<(), List<Diagnostic>> {
        let mut modified = 0;

        for local in func.locals.iter_mut() {
            let ref_id = RefId(local.id.0 as u64);
            let decision = result.get_tier(ref_id);

            if decision.is_promoted() {
                // Transform ThinRef to optimized layout
                if let MirType::Ref {
                    inner,
                    mutable,
                    layout,
                } = &local.ty
                {
                    let promoted_layout = match layout {
                        ReferenceLayout::ThinRef => {
                            if matches!(&**inner, MirType::Slice(_)) {
                                ReferenceLayout::FatRef(MetadataKind::Length)
                            } else {
                                ReferenceLayout::ThinRef
                            }
                        }
                        ReferenceLayout::FatRef(kind) => ReferenceLayout::FatRef(*kind),
                    };

                    local.ty = MirType::Ref {
                        inner: inner.clone(),
                        mutable: *mutable,
                        layout: promoted_layout,
                    };
                    modified += 1;
                }
            }
        }

        if self.config.emit_diagnostics && modified > 0 {
            let note = DiagnosticBuilder::new(Severity::Note)
                .message(format!(
                    "CBGR: Promoted {} references in '{}' (CBGR checks eliminated)",
                    modified, func.name
                ))
                .build();
            let _ = note;
        }

        Ok(())
    }

    /// Get tier decision for a specific reference
    pub fn get_decision(&self, func_name: &Text, ref_id: RefId) -> Option<&ReferenceTier> {
        self.decisions
            .get(func_name)
            .and_then(|decisions| decisions.get(&ref_id))
    }

    /// Get all accumulated statistics
    pub fn statistics(&self) -> &CbgrPassStatistics {
        &self.total_stats
    }

    /// Reset statistics
    pub fn reset_statistics(&mut self) {
        self.total_stats = CbgrPassStatistics::default();
        self.decisions.clear();
    }
}

/// Result of running CBGR pass on a single function
#[derive(Debug, Clone)]
pub struct PassResult {
    /// Tier analysis statistics
    pub stats: TierStatistics,
    /// Analysis duration (milliseconds)
    pub duration_ms: u64,
    /// Warnings generated
    pub warnings: List<Diagnostic>,
}

/// Result of running CBGR pass on a module
#[derive(Debug, Clone)]
pub struct ModulePassResult {
    /// Per-function results
    pub function_results: Map<Text, PassResult>,
    /// Accumulated statistics
    pub total_stats: CbgrPassStatistics,
    /// All warnings
    pub warnings: List<Diagnostic>,
}

/// Accumulated statistics across all analyzed functions
#[derive(Debug, Clone, Default)]
pub struct CbgrPassStatistics {
    /// Number of functions analyzed
    pub functions_analyzed: u64,
    /// Total references analyzed
    pub total_references: u64,
    /// Total promotions applied
    pub total_promotions: u64,
    /// Total estimated time saved (nanoseconds)
    pub total_time_saved_ns: u64,
}

impl CbgrPassStatistics {
    /// Calculate overall promotion rate
    pub fn promotion_rate(&self) -> f64 {
        if self.total_references == 0 {
            0.0
        } else {
            (self.total_promotions as f64) / (self.total_references as f64)
        }
    }

    /// Calculate average promotions per function
    pub fn avg_promotions_per_function(&self) -> f64 {
        if self.functions_analyzed == 0 {
            0.0
        } else {
            (self.total_promotions as f64) / (self.functions_analyzed as f64)
        }
    }

    /// Estimate total time saved per execution (microseconds)
    pub fn estimated_time_saved_us(&self) -> u64 {
        self.total_time_saved_ns / 1000
    }
}

impl std::fmt::Display for CbgrPassStatistics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "CBGR Tier Analysis Pass Statistics:")?;
        writeln!(f, "  Functions analyzed:     {}", self.functions_analyzed)?;
        writeln!(f, "  References analyzed:    {}", self.total_references)?;
        writeln!(
            f,
            "  Promotions applied:     {} (Tier 0 → Tier 1)",
            self.total_promotions
        )?;
        writeln!(
            f,
            "  Promotion rate:         {:.1}%",
            self.promotion_rate() * 100.0
        )?;
        writeln!(
            f,
            "  Avg promotions/func:    {:.1}",
            self.avg_promotions_per_function()
        )?;
        writeln!(
            f,
            "  Est. time saved/exec:   ~{}μs",
            self.estimated_time_saved_us()
        )?;
        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phases::mir_lowering::{BasicBlock, BlockId, LocalId, LocalKind, MirLocal};
    use verum_ast::span::Span as AstSpan;

    #[test]
    fn test_cbgr_pass_creation() {
        let config = CbgrPassConfig::default();
        let pass = CbgrOptimizationPass::new(config);

        assert_eq!(pass.statistics().functions_analyzed, 0);
        assert_eq!(pass.statistics().total_promotions, 0);
    }

    #[test]
    fn test_cbgr_pass_config_defaults() {
        let config = CbgrPassConfig::default();

        assert_eq!(config.confidence_threshold, 0.95);
        assert!(config.detailed_stats);
        assert!(!config.emit_diagnostics);
        assert_eq!(config.max_analysis_time_ms, 1000);
    }

    #[test]
    fn test_statistics_promotion_rate() {
        let stats = CbgrPassStatistics {
            functions_analyzed: 10,
            total_references: 100,
            total_promotions: 60,
            total_time_saved_ns: 9000,
        };

        assert_eq!(stats.promotion_rate(), 0.6);
        assert_eq!(stats.avg_promotions_per_function(), 6.0);
        assert_eq!(stats.estimated_time_saved_us(), 9);
    }

    #[test]
    fn test_statistics_zero_division() {
        let stats = CbgrPassStatistics::default();

        assert_eq!(stats.promotion_rate(), 0.0);
        assert_eq!(stats.avg_promotions_per_function(), 0.0);
    }
}
