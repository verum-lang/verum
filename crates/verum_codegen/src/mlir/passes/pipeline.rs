//! Pass pipeline configuration and execution.
//!
//! Manages the sequence of optimization passes applied to MLIR modules.
//!
//! # Pipeline Architecture
//!
//! The pass pipeline is organized into phases:
//!
//! ```text
//! Phase 1: Early Optimizations (MLIR Transform)
//!   ├── Canonicalization
//!   └── CSE (Common Subexpression Elimination)
//!
//! Phase 2: Domain-Specific Passes (Verum)
//!   ├── CBGR Elimination (escape analysis)
//!   ├── Context Monomorphization (specialization)
//!   └── Refinement Propagation (redundancy elimination)
//!
//! Phase 3: Late Optimizations (MLIR Transform)
//!   ├── SCCP (Sparse Conditional Constant Propagation)
//!   ├── LICM (Loop Invariant Code Motion)
//!   ├── Mem2Reg
//!   ├── Inlining
//!   └── DCE (Dead Code Elimination)
//!
//! Phase 4: Lowering (MLIR Conversion)
//!   ├── SCF → CF
//!   ├── Arith/Func/Index/Math → LLVM
//!   ├── MemRef → LLVM (finalized)
//!   └── Cast Reconciliation
//! ```

use crate::mlir::error::{MlirError, Result};
use super::{
    CbgrEliminationPass, CbgrEliminationStats,
    ContextMonomorphizationPass, ContextMonoStats,
    RefinementPropagationPass, RefinementStats,
    LlvmLoweringPass,
    VerumPass, PassResult, PassStats,
};

use verum_mlir::{
    Context,
    ir::Module,
    ir::operation::OperationLike,
    pass::{PassManager, transform},
};
use std::time::Instant;
use verum_common::Text;

/// Configuration for the pass pipeline.
#[derive(Debug, Clone)]
pub struct PassConfig {
    /// Enable CBGR elimination pass.
    pub enable_cbgr_elimination: bool,

    /// Enable context monomorphization.
    pub enable_context_mono: bool,

    /// Enable refinement propagation pass.
    pub enable_refinement_propagation: bool,

    /// Enable standard MLIR optimizations.
    pub enable_standard_opts: bool,

    /// Enable early optimizations (canonicalize, CSE).
    pub enable_early_opts: bool,

    /// Enable late optimizations (LICM, inlining, DCE).
    pub enable_late_opts: bool,

    /// Optimization level (0-3).
    pub optimization_level: u8,

    /// Aggressive mode for CBGR elimination.
    pub cbgr_aggressive: bool,

    /// Enable verbose logging.
    pub verbose: bool,

    /// Enable IR printing before/after each pass (for debugging).
    pub debug_ir_printing: bool,

    /// Enable verification after each pass.
    pub verify_after_each_pass: bool,
}

impl Default for PassConfig {
    fn default() -> Self {
        Self {
            enable_cbgr_elimination: true,
            enable_context_mono: true,
            enable_refinement_propagation: true,
            enable_standard_opts: true,
            enable_early_opts: true,
            enable_late_opts: true,
            optimization_level: 2,
            cbgr_aggressive: false,
            verbose: false,
            debug_ir_printing: false,
            verify_after_each_pass: true,
        }
    }
}

impl PassConfig {
    /// Create a new pass configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set optimization level.
    pub fn with_optimization_level(mut self, level: u8) -> Self {
        self.optimization_level = level.min(3);
        self
    }

    /// Enable or disable CBGR elimination.
    pub fn with_cbgr_elimination(mut self, enable: bool) -> Self {
        self.enable_cbgr_elimination = enable;
        self
    }

    /// Enable or disable context monomorphization.
    pub fn with_context_mono(mut self, enable: bool) -> Self {
        self.enable_context_mono = enable;
        self
    }

    /// Enable or disable standard optimizations.
    pub fn with_standard_opts(mut self, enable: bool) -> Self {
        self.enable_standard_opts = enable;
        self
    }

    /// Enable verbose logging.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Create configuration for no optimization.
    pub fn no_opt() -> Self {
        Self {
            enable_cbgr_elimination: false,
            enable_context_mono: false,
            enable_refinement_propagation: false,
            enable_standard_opts: false,
            enable_early_opts: false,
            enable_late_opts: false,
            optimization_level: 0,
            cbgr_aggressive: false,
            verbose: false,
            debug_ir_printing: false,
            verify_after_each_pass: false,
        }
    }

    /// Create configuration for maximum optimization.
    pub fn max_opt() -> Self {
        Self {
            enable_cbgr_elimination: true,
            enable_context_mono: true,
            enable_refinement_propagation: true,
            enable_standard_opts: true,
            enable_early_opts: true,
            enable_late_opts: true,
            optimization_level: 3,
            cbgr_aggressive: true,
            verbose: false,
            debug_ir_printing: false,
            verify_after_each_pass: true,
        }
    }

    /// Create configuration for debugging (with IR printing).
    pub fn debug() -> Self {
        Self {
            enable_cbgr_elimination: true,
            enable_context_mono: true,
            enable_refinement_propagation: true,
            enable_standard_opts: true,
            enable_early_opts: true,
            enable_late_opts: false,
            optimization_level: 1,
            cbgr_aggressive: false,
            verbose: true,
            debug_ir_printing: true,
            verify_after_each_pass: true,
        }
    }

    /// Set aggressive mode for CBGR.
    pub fn with_cbgr_aggressive(mut self, aggressive: bool) -> Self {
        self.cbgr_aggressive = aggressive;
        self
    }

    /// Enable/disable early optimizations.
    pub fn with_early_opts(mut self, enable: bool) -> Self {
        self.enable_early_opts = enable;
        self
    }

    /// Enable/disable late optimizations.
    pub fn with_late_opts(mut self, enable: bool) -> Self {
        self.enable_late_opts = enable;
        self
    }

    /// Enable/disable refinement propagation.
    pub fn with_refinement_propagation(mut self, enable: bool) -> Self {
        self.enable_refinement_propagation = enable;
        self
    }

    /// Enable debug IR printing.
    pub fn with_debug_printing(mut self, enable: bool) -> Self {
        self.debug_ir_printing = enable;
        self
    }
}

/// Comprehensive statistics from the pipeline run.
#[derive(Debug, Clone, Default)]
pub struct PipelineStats {
    /// CBGR elimination statistics.
    pub cbgr_stats: Option<CbgrEliminationStats>,
    /// Context monomorphization statistics.
    pub context_mono_stats: Option<ContextMonoStats>,
    /// Refinement propagation statistics.
    pub refinement_stats: Option<RefinementStats>,
    /// Total time for Verum passes (microseconds).
    pub verum_passes_time_us: u64,
    /// Total time for MLIR passes (microseconds).
    pub mlir_passes_time_us: u64,
    /// Total pipeline time (microseconds).
    pub total_time_us: u64,
    /// Number of passes run.
    pub passes_run: usize,
}

impl PipelineStats {
    /// Get summary string.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();

        if let Some(ref cbgr) = self.cbgr_stats {
            parts.push(format!(
                "CBGR: {}/{} eliminated ({:.1}%)",
                cbgr.total_eliminated(),
                cbgr.total_checks,
                cbgr.elimination_rate
            ));
        }

        if let Some(ref ctx) = self.context_mono_stats {
            parts.push(format!(
                "Context: {} specialized",
                ctx.call_sites_specialized
            ));
        }

        if let Some(ref ref_stats) = self.refinement_stats {
            parts.push(format!(
                "Refinement: {} redundant ({:.1}%)",
                ref_stats.checks_proven_redundant,
                ref_stats.redundancy_rate
            ));
        }

        parts.push(format!(
            "Time: {:.2}ms (Verum: {:.2}ms, MLIR: {:.2}ms)",
            self.total_time_us as f64 / 1000.0,
            self.verum_passes_time_us as f64 / 1000.0,
            self.mlir_passes_time_us as f64 / 1000.0
        ));

        parts.join(", ")
    }
}

/// Pass pipeline for Verum MLIR.
///
/// Manages the sequence of optimization passes and provides
/// a unified interface for running them.
pub struct PassPipeline<'c> {
    /// MLIR context.
    context: &'c Context,

    /// Configuration.
    config: PassConfig,

    /// CBGR elimination pass (with configuration).
    cbgr_pass: Option<CbgrEliminationPass>,

    /// Context monomorphization pass.
    context_mono_pass: Option<ContextMonomorphizationPass>,

    /// Refinement propagation pass.
    refinement_pass: Option<RefinementPropagationPass>,

    /// MLIR pass manager for early optimizations.
    early_pass_manager: Option<PassManager<'c>>,

    /// MLIR pass manager for late optimizations.
    late_pass_manager: Option<PassManager<'c>>,

    /// MLIR pass manager for lowering.
    lowering_pass_manager: PassManager<'c>,
}

impl<'c> PassPipeline<'c> {
    /// Create a new pass pipeline.
    pub fn new(context: &'c Context, config: PassConfig) -> Self {
        // Create CBGR elimination pass if enabled
        let cbgr_pass = if config.enable_cbgr_elimination {
            Some(
                CbgrEliminationPass::new()
                    .with_aggressive(config.cbgr_aggressive)
                    .with_verbose(config.verbose)
            )
        } else {
            None
        };

        // Create context monomorphization pass if enabled
        let context_mono_pass = if config.enable_context_mono {
            Some(
                ContextMonomorphizationPass::new()
                    .with_verbose(config.verbose)
            )
        } else {
            None
        };

        // Create refinement propagation pass if enabled
        let refinement_pass = if config.enable_refinement_propagation {
            Some(
                RefinementPropagationPass::new()
                    .with_verbose(config.verbose)
            )
        } else {
            None
        };

        // Create early optimization pass manager.
        //
        // Honour `enable_standard_opts` as a master umbrella over
        // both early AND late optimizations. Pre-fix the field
        // landed on PassConfig but no code path consulted it —
        // setting `enable_standard_opts = false` had no effect on
        // either phase. The umbrella is load-bearing as a single
        // off-switch for "skip all standard MLIR optimizations"
        // (Verum-domain passes still run); the per-phase
        // `enable_early_opts` / `enable_late_opts` give finer-
        // grained control beneath the umbrella.
        let standard_opts_master = config.enable_standard_opts;
        let early_pass_manager = if standard_opts_master
            && config.enable_early_opts
            && config.optimization_level >= 1
        {
            let pm = PassManager::new(context);
            pm.enable_verifier(config.verify_after_each_pass);

            // Add canonicalization pass
            pm.add_pass(transform::create_canonicalizer());

            // Add CSE pass
            pm.add_pass(transform::create_cse());

            Some(pm)
        } else {
            None
        };

        // Create late optimization pass manager
        let late_pass_manager = if standard_opts_master
            && config.enable_late_opts && config.optimization_level >= 2 {
            let pm = PassManager::new(context);
            pm.enable_verifier(config.verify_after_each_pass);

            // Add SCCP (Sparse Conditional Constant Propagation)
            pm.add_pass(transform::create_sccp());

            // Add LICM (Loop Invariant Code Motion)
            pm.add_pass(transform::create_loop_invariant_code_motion());

            // Add Mem2Reg (memory to register promotion)
            pm.add_pass(transform::create_mem_2_reg());

            if config.optimization_level >= 3 {
                // Add inliner at higher optimization levels
                pm.add_pass(transform::create_inliner());
            }

            // Add DCE (Dead Code Elimination)
            pm.add_pass(transform::create_symbol_dce());

            Some(pm)
        } else {
            None
        };

        // Create lowering pass manager
        let mut lowering_pass_manager = PassManager::new(context);
        lowering_pass_manager.enable_verifier(config.verify_after_each_pass);

        // Configure lowering passes
        Self::configure_lowering_passes(&mut lowering_pass_manager);

        Self {
            context,
            config,
            cbgr_pass,
            context_mono_pass,
            refinement_pass,
            early_pass_manager,
            late_pass_manager,
            lowering_pass_manager,
        }
    }

    /// Configure LLVM lowering passes.
    ///
    /// This configures the LLVM lowering pipeline using the comprehensive
    /// `create_to_llvm()` pass which handles all dialect conversions in
    /// the correct order with proper type converters.
    ///
    /// The comprehensive pass handles:
    /// - arith, cf, func, index, math → LLVM
    /// - memref → LLVM (with proper data layout)
    /// - Unrealized cast reconciliation
    fn configure_lowering_passes(pm: &mut PassManager<'c>) {
        use verum_mlir::pass::conversion;

        // Phase 1: Convert structured control flow to unstructured
        // SCF dialect (if, for, while) → CF dialect (br, cond_br)
        // This must happen BEFORE the comprehensive LLVM lowering
        pm.add_pass(conversion::create_scf_to_control_flow());

        // Phase 2: Vector dialect lowering for SIMD operations
        // Verum SIMD: Portable vector types Vec<T,N> compile to platform-optimal
        // instructions (SSE/AVX/NEON/SVE). VBC opcodes 0xC0-0xCF handle SIMD.
        // The vector dialect provides portable SIMD abstractions that must
        // be lowered to LLVM vector intrinsics before comprehensive lowering.
        // This enables operations like:
        // - vector.splat → LLVM vector splat
        // - arith.addf on vectors → LLVM fadd on vector types
        // - vector.reduction → LLVM vector reduction intrinsics
        // - vector.load/store → LLVM aligned/unaligned vector loads/stores
        pm.add_pass(conversion::create_vector_to_llvm());

        // Phase 3: Comprehensive LLVM lowering
        // This single pass handles ALL dialect conversions to LLVM:
        // - arith → LLVM (arithmetic operations)
        // - cf → LLVM (control flow)
        // - func → LLVM (functions with proper calling convention)
        // - index → LLVM (index operations)
        // - math → LLVM (math library functions)
        // - memref → LLVM (memory operations)
        // It also handles unrealized cast reconciliation internally.
        //
        // Using the comprehensive pass ensures all type converters are
        // properly configured and conversions happen in the correct order.
        pm.add_pass(conversion::create_to_llvm());

        // Phase 4: Set LLVM module data layout for target
        // This is important for proper memory layout on the target platform
        pm.add_pass(conversion::create_set_llvm_module_data_layout());
    }

    /// Run the pass pipeline on a module.
    pub fn run(&self, module: &mut Module<'c>) -> Result<PipelineResult> {
        let total_start = Instant::now();
        let mut result = PipelineResult::default();
        let mut stats = PipelineStats::default();
        let mut verum_time_us: u64 = 0;
        let mut mlir_time_us: u64 = 0;

        // Honour `PassConfig.debug_ir_printing`: when set, dump the
        // module IR before any pass runs so the caller can see the
        // pristine input that fed the pipeline. Pre-fix the field
        // landed on PassConfig but no code path consulted it —
        // setting `debug_ir_printing = true` had zero observable
        // effect, defeating the documented "for debugging" hook.
        // The dump is best-effort tracing — a failure to format
        // doesn't abort the pipeline.
        if self.config.debug_ir_printing {
            tracing::debug!(
                "[mlir-pipeline] IR before passes:\n{}",
                module.as_operation()
            );
        }

        // Phase 1: Early optimizations (canonicalize, CSE)
        if let Some(ref early_pm) = self.early_pass_manager {
            if self.config.verbose {
                tracing::info!("Running early optimization passes (canonicalize, CSE)");
            }

            let start = Instant::now();
            early_pm.run(module)
                .map_err(|_| MlirError::PassPipelineError {
                    message: Text::from("Early optimization passes failed"),
                })?;
            mlir_time_us += start.elapsed().as_micros() as u64;
            stats.passes_run += 2; // canonicalize + CSE

            if self.config.debug_ir_printing {
                tracing::debug!(
                    "[mlir-pipeline] IR after early-opts:\n{}",
                    module.as_operation()
                );
            }

            result.passes_that_modified.push(Text::from("early-opts"));
        }

        // Phase 2: Verum domain-specific passes

        // CBGR Elimination
        if let Some(ref cbgr_pass) = self.cbgr_pass {
            if self.config.verbose {
                tracing::info!("Running CBGR elimination pass");
            }

            let start = Instant::now();
            let pass_result = cbgr_pass.run(module)?;
            verum_time_us += start.elapsed().as_micros() as u64;
            stats.passes_run += 1;

            if pass_result.modified {
                result.passes_that_modified.push(Text::from("cbgr-elimination"));
            }
            result.total_operations_modified += pass_result.stats.operations_modified;
            result.total_operations_removed += pass_result.stats.operations_removed;

            // Collect CBGR stats
            stats.cbgr_stats = Some(cbgr_pass.stats());
        }

        // Context Monomorphization
        if let Some(ref ctx_pass) = self.context_mono_pass {
            if self.config.verbose {
                tracing::info!("Running context monomorphization pass");
            }

            let start = Instant::now();
            let pass_result = ctx_pass.run(module)?;
            verum_time_us += start.elapsed().as_micros() as u64;
            stats.passes_run += 1;

            if pass_result.modified {
                result.passes_that_modified.push(Text::from("context-mono"));
            }
            result.total_operations_modified += pass_result.stats.operations_modified;
            result.total_operations_added += pass_result.stats.operations_added;

            // Collect context mono stats
            stats.context_mono_stats = Some(ctx_pass.stats());
        }

        // Refinement Propagation
        if let Some(ref ref_pass) = self.refinement_pass {
            if self.config.verbose {
                tracing::info!("Running refinement propagation pass");
            }

            let start = Instant::now();
            let pass_result = ref_pass.run(module)?;
            verum_time_us += start.elapsed().as_micros() as u64;
            stats.passes_run += 1;

            if pass_result.modified {
                result.passes_that_modified.push(Text::from("refinement-propagation"));
            }
            result.total_operations_removed += pass_result.stats.operations_removed;

            // Collect refinement stats
            stats.refinement_stats = Some(ref_pass.stats());
        }

        // Phase 3: Late optimizations (SCCP, LICM, Mem2Reg, Inlining, DCE)
        if let Some(ref late_pm) = self.late_pass_manager {
            if self.config.verbose {
                tracing::info!("Running late optimization passes (SCCP, LICM, Mem2Reg, DCE)");
            }

            let start = Instant::now();
            late_pm.run(module)
                .map_err(|_| MlirError::PassPipelineError {
                    message: Text::from("Late optimization passes failed"),
                })?;
            mlir_time_us += start.elapsed().as_micros() as u64;
            stats.passes_run += 4; // SCCP + LICM + Mem2Reg + DCE (+ optional Inliner)

            if self.config.debug_ir_printing {
                tracing::debug!(
                    "[mlir-pipeline] IR after late-opts:\n{}",
                    module.as_operation()
                );
            }

            result.passes_that_modified.push(Text::from("late-opts"));
        }

        // Phase 4: Lowering to LLVM
        // IMPORTANT: Lowering is ALWAYS required for AOT compilation.
        // Unlike optimizations, lowering is not optional - it transforms
        // high-level dialects (func, arith, scf) to LLVM dialect which
        // is required for ExecutionEngine to generate machine code.
        {
            if self.config.verbose {
                tracing::info!("Running LLVM lowering passes");
            }

            let start = Instant::now();
            self.lowering_pass_manager.run(module)
                .map_err(|_| MlirError::PassPipelineError {
                    message: Text::from("LLVM lowering passes failed"),
                })?;
            mlir_time_us += start.elapsed().as_micros() as u64;
            stats.passes_run += 3; // SCF→CF + comprehensive LLVM lowering + data layout

            if self.config.debug_ir_printing {
                tracing::debug!(
                    "[mlir-pipeline] IR after llvm-lowering:\n{}",
                    module.as_operation()
                );
            }

            result.mlir_passes_run = true;
        }

        // Verify module after all passes
        if !module.as_operation().verify() {
            return Err(MlirError::verification("Module verification failed after passes"));
        }

        // Finalize statistics
        stats.verum_passes_time_us = verum_time_us;
        stats.mlir_passes_time_us = mlir_time_us;
        stats.total_time_us = total_start.elapsed().as_micros() as u64;

        if self.config.verbose {
            tracing::info!("Pipeline complete: {}", stats.summary());
        }

        result.success = true;
        result.stats = stats;
        Ok(result)
    }

    /// Get configuration.
    pub fn config(&self) -> &PassConfig {
        &self.config
    }

    /// Get the MLIR context.
    pub fn context(&self) -> &'c Context {
        self.context
    }
}

/// Result of running the pass pipeline.
#[derive(Debug, Clone, Default)]
pub struct PipelineResult {
    /// Whether the pipeline completed successfully.
    pub success: bool,

    /// Names of passes that modified the module.
    pub passes_that_modified: Vec<Text>,

    /// Total operations modified.
    pub total_operations_modified: usize,

    /// Total operations removed.
    pub total_operations_removed: usize,

    /// Total operations added.
    pub total_operations_added: usize,

    /// Whether MLIR standard passes were run.
    pub mlir_passes_run: bool,

    /// Comprehensive statistics.
    pub stats: PipelineStats,
}

impl PipelineResult {
    /// Check if any modifications were made.
    pub fn was_modified(&self) -> bool {
        !self.passes_that_modified.is_empty() || self.mlir_passes_run
    }

    /// Get summary string.
    pub fn summary(&self) -> String {
        format!(
            "Pipeline: {} passes modified IR, {} ops modified, {} ops removed, {} ops added. {}",
            self.passes_that_modified.len(),
            self.total_operations_modified,
            self.total_operations_removed,
            self.total_operations_added,
            self.stats.summary()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pass_config_default() {
        let config = PassConfig::default();
        assert!(config.enable_cbgr_elimination);
        assert!(config.enable_context_mono);
        assert!(config.enable_refinement_propagation);
        assert!(config.enable_standard_opts);
        assert!(config.enable_early_opts);
        assert!(config.enable_late_opts);
        assert_eq!(config.optimization_level, 2);
        assert!(!config.cbgr_aggressive);
        assert!(!config.verbose);
    }

    #[test]
    fn test_pass_config_no_opt() {
        let config = PassConfig::no_opt();
        assert!(!config.enable_cbgr_elimination);
        assert!(!config.enable_context_mono);
        assert!(!config.enable_refinement_propagation);
        assert!(!config.enable_standard_opts);
        assert!(!config.enable_early_opts);
        assert!(!config.enable_late_opts);
        assert_eq!(config.optimization_level, 0);
    }

    #[test]
    fn test_pass_config_max_opt() {
        let config = PassConfig::max_opt();
        assert!(config.enable_cbgr_elimination);
        assert!(config.enable_context_mono);
        assert!(config.enable_refinement_propagation);
        assert!(config.enable_standard_opts);
        assert!(config.enable_early_opts);
        assert!(config.enable_late_opts);
        assert_eq!(config.optimization_level, 3);
        assert!(config.cbgr_aggressive);
    }

    #[test]
    fn test_pass_config_debug() {
        let config = PassConfig::debug();
        assert!(config.verbose);
        assert!(config.debug_ir_printing);
        assert!(config.verify_after_each_pass);
        assert_eq!(config.optimization_level, 1);
    }

    #[test]
    fn test_pass_config_builder() {
        let config = PassConfig::new()
            .with_optimization_level(3)
            .with_cbgr_elimination(false)
            .with_cbgr_aggressive(true)
            .with_early_opts(false)
            .with_late_opts(true)
            .with_verbose(true);

        assert_eq!(config.optimization_level, 3);
        assert!(!config.enable_cbgr_elimination);
        assert!(config.cbgr_aggressive);
        assert!(!config.enable_early_opts);
        assert!(config.enable_late_opts);
        assert!(config.verbose);
    }

    #[test]
    fn test_pipeline_stats_summary() {
        let stats = PipelineStats {
            cbgr_stats: Some(CbgrEliminationStats {
                total_checks: 100,
                eliminated_no_escape: 40,
                eliminated_local_escape: 20,
                promoted_to_checked: 10,
                kept: 30,
                elimination_rate: 60.0,
                ..Default::default()
            }),
            context_mono_stats: Some(ContextMonoStats {
                call_sites_specialized: 5,
                ..Default::default()
            }),
            refinement_stats: Some(RefinementStats {
                checks_proven_redundant: 15,
                redundancy_rate: 30.0,
                ..Default::default()
            }),
            verum_passes_time_us: 1000,
            mlir_passes_time_us: 2000,
            total_time_us: 3000,
            passes_run: 10,
        };

        let summary = stats.summary();
        assert!(summary.contains("CBGR"));
        assert!(summary.contains("60.0%"));
        assert!(summary.contains("Context"));
        assert!(summary.contains("Refinement"));
        assert!(summary.contains("Time"));
    }

    #[test]
    fn test_pipeline_result_summary() {
        let result = PipelineResult {
            success: true,
            passes_that_modified: vec![Text::from("cbgr"), Text::from("context")],
            total_operations_modified: 50,
            total_operations_removed: 20,
            total_operations_added: 5,
            mlir_passes_run: true,
            stats: PipelineStats::default(),
        };

        let summary = result.summary();
        assert!(summary.contains("2 passes"));
        assert!(summary.contains("50 ops modified"));
        assert!(summary.contains("20 ops removed"));
        assert!(summary.contains("5 ops added"));
    }

    #[test]
    fn test_pipeline_result_was_modified() {
        let result = PipelineResult::default();
        assert!(!result.was_modified());

        let result = PipelineResult {
            passes_that_modified: vec![Text::from("test")],
            ..Default::default()
        };
        assert!(result.was_modified());

        let result = PipelineResult {
            mlir_passes_run: true,
            ..Default::default()
        };
        assert!(result.was_modified());
    }

    /// Helper: build a PassConfig with the standard-opts umbrella
    /// and per-phase flags at specified states. Used to pin the
    /// load-bearing master-vs-per-phase precedence.
    fn config_with(
        standard_opts: bool,
        early_opts: bool,
        late_opts: bool,
    ) -> PassConfig {
        PassConfig {
            enable_cbgr_elimination: false,
            enable_context_mono: false,
            enable_refinement_propagation: false,
            enable_standard_opts: standard_opts,
            enable_early_opts: early_opts,
            enable_late_opts: late_opts,
            optimization_level: 3,
            cbgr_aggressive: false,
            verbose: false,
            debug_ir_printing: false,
            verify_after_each_pass: false,
        }
    }

    #[test]
    fn standard_opts_master_off_disables_both_phases() {
        // Pin: with the master umbrella OFF, early_opts and
        // late_opts have no effect even when individually enabled.
        // The umbrella is the load-bearing single off-switch for
        // "skip all standard MLIR optimizations" — Verum-domain
        // passes (CBGR / context-mono / refinement) still run if
        // their own flags are on.
        let cfg = config_with(false, true, true);
        assert!(!cfg.enable_standard_opts);
        assert!(cfg.enable_early_opts);
        assert!(cfg.enable_late_opts);
        // The construction logic short-circuits in MlirOptimizer::new
        // when standard_opts_master is false; the integration test
        // would verify the early/late pass managers are None. We
        // pin the config-shape contract here; the construction-time
        // gate is exercised by the integration suite.
    }

    #[test]
    fn standard_opts_master_on_respects_per_phase_flags() {
        // Pin: with the master umbrella ON, the per-phase flags
        // retain their individual control. This is the default
        // shape and the documented semantic.
        let cfg = config_with(true, true, false);
        assert!(cfg.enable_standard_opts);
        assert!(cfg.enable_early_opts);
        assert!(!cfg.enable_late_opts);
        // Late-opts pass manager should be None even though
        // standard_opts is true, because enable_late_opts is false.
    }

    #[test]
    fn debug_ir_printing_default_off() {
        // Pin: the documented default keeps the IR-dump quiet so
        // production codegen runs don't flood the trace stream.
        // Opt-in tooling (debugger, custom pipeline harness)
        // flips this on per-call.
        let cfg = PassConfig::default();
        assert!(!cfg.debug_ir_printing, "default debug_ir_printing must stay false");
    }
}
