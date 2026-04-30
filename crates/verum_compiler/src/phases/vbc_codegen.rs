//! VBC Code Generation Phase
//!
//! This phase compiles typed AST to VBC bytecode with tier-aware instructions.
//! It runs CBGR tier analysis before codegen to determine optimal reference tiers.
//!
//! # Architecture
//!
//! ```text
//! Typed AST (from Phase 4)
//!       │
//!       ▼
//! ┌─────────────────────────────────────────┐
//! │           VBC CODEGEN PHASE              │
//! │                                          │
//! │  1. Build CFG for tier analysis          │
//! │  2. Run escape analysis (verum_cbgr)     │
//! │  3. Convert TierDecision → TierContext   │
//! │  4. Run VBC codegen with tier context    │
//! │  5. Output VbcModule with tier stats     │
//! └─────────────────────────────────────────┘
//!       │
//!       ▼
//! VbcModuleData (bytecode + tier stats)
//! ```
//!
//! # Tier Analysis Integration
//!
//! The phase uses `verum_cbgr::tier_analysis` to determine which references
//! can be promoted from Tier 0 (runtime checked) to Tier 1 (zero-overhead).
//!
//! - Tier 0: ~15ns overhead per dereference (CBGR validation)
//! - Tier 1: 0ns overhead (compiler proven safe)
//! - Tier 2: 0ns overhead (manual unsafe)
//!
//! VBC codegen with CBGR: TypedAST to VBC bytecode with CBGR safety checks.

use std::time::{Duration, Instant};

use super::{
    CompilationPhase, PhaseData, PhaseInput, PhaseMetrics, PhaseOutput, VbcModuleData,
    VbcTierStats,
};
use verum_ast::Module;
use verum_cbgr::tier_analysis::TierAnalysisConfig;
use verum_common::{List, Text};
use verum_diagnostics::Diagnostic;
use verum_vbc::codegen::{CodegenConfig, TierContext, VbcCodegen};

/// VBC code generation phase.
///
/// Compiles typed AST modules to VBC bytecode with tier-aware instructions.
pub struct VbcCodegenPhase {
    /// Configuration for tier analysis.
    tier_config: TierAnalysisConfig,

    /// Configuration for VBC codegen.
    codegen_config: CodegenConfig,

    /// Performance metrics.
    metrics: VbcCodegenMetrics,
}

/// Metrics collected during VBC codegen.
#[derive(Debug, Clone, Default)]
struct VbcCodegenMetrics {
    /// Time spent in tier analysis.
    tier_analysis_time: Duration,
    /// Time spent in VBC codegen.
    codegen_time: Duration,
    /// Number of modules processed.
    modules_processed: usize,
    /// Total tier stats across all modules.
    total_tier_stats: VbcTierStats,
}

impl VbcCodegenPhase {
    /// Creates a new VBC codegen phase with default configuration.
    pub fn new() -> Self {
        Self {
            tier_config: TierAnalysisConfig::default(),
            codegen_config: CodegenConfig::default(),
            metrics: VbcCodegenMetrics::default(),
        }
    }

    /// Creates a new VBC codegen phase with custom configuration.
    pub fn with_config(tier_config: TierAnalysisConfig, codegen_config: CodegenConfig) -> Self {
        Self {
            tier_config,
            codegen_config,
            metrics: VbcCodegenMetrics::default(),
        }
    }

    /// Sets the module name for codegen.
    pub fn with_module_name(mut self, name: impl Into<String>) -> Self {
        self.codegen_config.module_name = name.into();
        self
    }

    /// Enables debug info generation.
    pub fn with_debug_info(mut self) -> Self {
        self.codegen_config.debug_info = true;
        self
    }

    /// Sets optimization level (0-3).
    pub fn with_optimization_level(mut self, level: u8) -> Self {
        self.codegen_config.optimization_level = level.min(3);
        self
    }

    /// Compiles a single AST module to VBC.
    fn compile_module(&mut self, module: &Module) -> Result<VbcModuleData, List<Diagnostic>> {
        // Step 1: Build CFG for tier analysis
        let tier_start = Instant::now();
        let tier_context = self.run_tier_analysis(module)?;
        self.metrics.tier_analysis_time += tier_start.elapsed();

        // Step 2: Run VBC codegen with tier context
        let codegen_start = Instant::now();
        let mut codegen = VbcCodegen::with_config(self.codegen_config.clone());
        codegen.set_tier_context(tier_context);

        let vbc_module = codegen.compile_module(module).map_err(|e| {
            let diagnostic = verum_diagnostics::DiagnosticBuilder::error()
                .code("E0701")
                .message(format!("VBC codegen error: {}", e))
                .build();
            {
                let mut list = List::new();
                list.push(diagnostic);
                list
            }
        })?;

        self.metrics.codegen_time += codegen_start.elapsed();

        // Step 3: Collect tier statistics
        let (tier0, tier1, tier2) = codegen.tier_stats();
        let total = tier0 + tier1 + tier2;
        let promotion_rate = if total > 0 {
            tier1 as f64 / total as f64
        } else {
            0.0
        };

        let tier_stats = VbcTierStats {
            tier0_refs: tier0,
            tier1_refs: tier1,
            tier2_refs: tier2,
            promotion_rate,
        };

        // Update total metrics
        self.metrics.total_tier_stats.tier0_refs += tier0;
        self.metrics.total_tier_stats.tier1_refs += tier1;
        self.metrics.total_tier_stats.tier2_refs += tier2;
        self.metrics.modules_processed += 1;

        Ok(VbcModuleData {
            module: vbc_module,
            tier_stats,
        })
    }

    /// Runs tier analysis on a module and returns TierContext.
    ///
    /// This method:
    /// 1. Builds CFGs from all functions in the module using CfgConstructor
    /// 2. Runs TierAnalyzer on each CFG to determine reference tiers
    /// 3. Aggregates results into a TierContext for VBC codegen
    ///
    /// The tier decisions enable VBC codegen to emit tier-appropriate instructions:
    /// - Tier 0: ChkRef + Deref (runtime CBGR validation, ~15ns)
    /// - Tier 1: Deref directly (compiler-proven safe, 0ns)
    /// - Tier 2: DerefUnsafe (manual safety, 0ns)
    fn run_tier_analysis(&self, module: &Module) -> Result<TierContext, List<Diagnostic>> {
        use super::cfg_constructor::CfgConstructor;
        use verum_cbgr::tier_analysis::TierAnalyzer;

        // Step 1: Build CFGs for all functions in the module
        let module_cfg = CfgConstructor::from_module(module);

        // Step 2: Run tier analysis on each function's CFG.
        //
        // ModuleCfg.functions is verum_common::Map (HashMap-backed), so
        // raw iteration order leaks Rust's per-process random hasher
        // seed into TierContext merge order — which in turn leaks into
        // VBC bytecode emission.  Sort by FunctionId so the bytecode
        // is byte-identical across runs.
        // See #143 / project_loom_quality_pivot_2026-04-25.md.
        let mut tier_context = TierContext::new();

        let mut sorted_funcs: Vec<_> = module_cfg.functions.iter().collect();
        sorted_funcs.sort_by_key(|(id, _)| id.0);

        for (_func_id, func_cfg) in sorted_funcs {
            // Create analyzer for this function's CFG
            let analyzer = TierAnalyzer::with_config(
                func_cfg.cfg.clone(),
                self.tier_config.clone(),
            );

            // Run 9-phase analysis (escape, dominance, ownership, concurrency,
            // lifetime, NLL, tier determination, cross-function, final)
            let analysis_result = analyzer.analyze();

            // Merge results into tier context using span-based ExprId mapping
            // TierContext::from_analysis_result handles RefId→ExprId conversion
            // via the span mappings preserved in analysis_result.
            let func_tier_context = TierContext::from_analysis_result(&analysis_result);

            // Merge function-level decisions into module-level context.
            //
            // #118 — `func_tier_context.decisions` is keyed by
            // span-encoded ExprIds `(start<<32)|end`, NOT 0..N.
            // The pre-#118 `0..decision_count()` loop constructed
            // `ExprId(i)` and looked up Tier1 decisions that were
            // never stored at those keys, silently dropping every
            // promotion. The canonical merge uses `merge_from`,
            // exposing the actual span-encoded keys directly.
            tier_context.merge_from(&func_tier_context);
        }

        // Enable tier context with collected decisions
        tier_context.enabled = true;

        Ok(tier_context)
    }
}

impl Default for VbcCodegenPhase {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilationPhase for VbcCodegenPhase {
    fn name(&self) -> &str {
        "VBC Codegen"
    }

    fn description(&self) -> &str {
        "Compiles typed AST to VBC bytecode with tier-aware instructions"
    }

    fn execute(&self, input: PhaseInput) -> Result<PhaseOutput, List<Diagnostic>> {
        // Extract AST modules from input
        let modules = match input.data {
            PhaseData::AstModules(modules) => modules,
            PhaseData::AstModulesWithContracts { modules, .. } => modules,
            _ => {
                let diagnostic = verum_diagnostics::DiagnosticBuilder::error()
                    .code("E0700")
                    .message("VBC codegen phase requires AST modules as input")
                    .build();
                return Err({
                let mut list = List::new();
                list.push(diagnostic);
                list
            });
            }
        };

        // Compile each module
        let mut phase = Self::with_config(self.tier_config.clone(), self.codegen_config.clone());
        let mut vbc_modules = List::new();

        for module in modules.iter() {
            let vbc_data = phase.compile_module(module)?;
            vbc_modules.push(vbc_data);
        }

        // Calculate overall promotion rate
        let total = phase.metrics.total_tier_stats.tier0_refs
            + phase.metrics.total_tier_stats.tier1_refs
            + phase.metrics.total_tier_stats.tier2_refs;
        if total > 0 {
            phase.metrics.total_tier_stats.promotion_rate =
                phase.metrics.total_tier_stats.tier1_refs as f64 / total as f64;
        }

        // Log statistics
        tracing::info!(
            "VBC codegen complete: {} modules, {} refs (T0: {}, T1: {}, T2: {}), {:.1}% promoted",
            phase.metrics.modules_processed,
            total,
            phase.metrics.total_tier_stats.tier0_refs,
            phase.metrics.total_tier_stats.tier1_refs,
            phase.metrics.total_tier_stats.tier2_refs,
            phase.metrics.total_tier_stats.promotion_rate * 100.0
        );

        Ok(PhaseOutput {
            data: PhaseData::Vbc(vbc_modules),
            warnings: List::new(),
            metrics: self.metrics(),
        })
    }

    fn can_parallelize(&self) -> bool {
        // VBC codegen can be parallelized per module
        true
    }

    fn metrics(&self) -> PhaseMetrics {
        let mut custom_metrics = List::new();
        custom_metrics.push((
            Text::from("tier0_refs"),
            Text::from(self.metrics.total_tier_stats.tier0_refs.to_string()),
        ));
        custom_metrics.push((
            Text::from("tier1_refs"),
            Text::from(self.metrics.total_tier_stats.tier1_refs.to_string()),
        ));
        custom_metrics.push((
            Text::from("tier2_refs"),
            Text::from(self.metrics.total_tier_stats.tier2_refs.to_string()),
        ));
        custom_metrics.push((
            Text::from("promotion_rate"),
            Text::from(format!(
                "{:.1}%",
                self.metrics.total_tier_stats.promotion_rate * 100.0
            )),
        ));

        PhaseMetrics {
            phase_name: Text::from("VBC Codegen"),
            duration: self.metrics.tier_analysis_time + self.metrics.codegen_time,
            items_processed: self.metrics.modules_processed,
            memory_allocated: 0,
            custom_metrics,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vbc_codegen_phase_creation() {
        let phase = VbcCodegenPhase::new();
        assert_eq!(phase.name(), "VBC Codegen");
    }

    #[test]
    fn test_vbc_codegen_phase_with_config() {
        let phase = VbcCodegenPhase::new()
            .with_module_name("test_module")
            .with_debug_info()
            .with_optimization_level(2);

        assert_eq!(phase.codegen_config.module_name, "test_module");
        assert!(phase.codegen_config.debug_info);
        assert_eq!(phase.codegen_config.optimization_level, 2);
    }

    #[test]
    fn test_tier_stats_default() {
        let stats = VbcTierStats::default();
        assert_eq!(stats.tier0_refs, 0);
        assert_eq!(stats.tier1_refs, 0);
        assert_eq!(stats.tier2_refs, 0);
        assert_eq!(stats.promotion_rate, 0.0);
    }
}
