//! Optimization Pass Infrastructure for Verum MLIR.
//!
//! This module provides the pass management infrastructure for Verum's
//! MLIR-based code generation. It includes:
//!
//! - **Domain-specific passes**: CBGR elimination, context monomorphization, refinement propagation
//! - **Standard MLIR passes**: CSE, canonicalization, LICM, inlining
//! - **Pipeline management**: Ordered pass execution with verification
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                     PassPipeline                                 │
//! │  ┌─────────────────────────────────────────────────────────┐    │
//! │  │              Verum Domain Passes                        │    │
//! │  │  ├── CbgrEliminationPass       (escape analysis)       │    │
//! │  │  ├── ContextMonomorphizationPass (specialization)      │    │
//! │  │  └── RefinementPropagationPass (redundancy elim)       │    │
//! │  └─────────────────────────────────────────────────────────┘    │
//! │  ┌─────────────────────────────────────────────────────────┐    │
//! │  │              MLIR Transform Passes                      │    │
//! │  │  ├── Canonicalization                                   │    │
//! │  │  ├── CSE (Common Subexpression Elimination)            │    │
//! │  │  ├── SCCP (Sparse Conditional Constant Propagation)    │    │
//! │  │  ├── DCE (Dead Code Elimination)                       │    │
//! │  │  ├── LICM (Loop Invariant Code Motion)                 │    │
//! │  │  ├── Mem2Reg                                           │    │
//! │  │  └── Inlining                                          │    │
//! │  └─────────────────────────────────────────────────────────┘    │
//! │  ┌─────────────────────────────────────────────────────────┐    │
//! │  │              LLVM Lowering Passes                       │    │
//! │  │  ├── Verum → SCF → CF → LLVM dialect                   │    │
//! │  │  └── Type/Op conversions                               │    │
//! │  └─────────────────────────────────────────────────────────┘    │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Pass Execution Order
//!
//! 1. **Early optimization**: Canonicalization, CSE
//! 2. **Domain passes**: CBGR elimination, context mono, refinement
//! 3. **Late optimization**: LICM, inlining, DCE
//! 4. **Lowering**: Verum → SCF → CF → LLVM
//!
//! # Usage
//!
//! ```rust,ignore
//! use crate::mlir::passes::{PassPipeline, PassConfig};
//!
//! // Create pipeline with default configuration
//! let mut pipeline = PassPipeline::new(&context);
//!
//! // Configure passes
//! pipeline.with_config(PassConfig {
//!     enable_cbgr_elimination: true,
//!     enable_context_mono: true,
//!     enable_standard_opts: true,
//!     optimization_level: 2,
//!     verbose: true,
//! });
//!
//! // Run pipeline
//! let result = pipeline.run(&mut module)?;
//! println!("{}", result.summary());
//! ```

mod cbgr_elimination;
mod context_mono;
pub mod gpu_pipeline;
mod llvm_lowering;
mod pipeline;
mod refinement_propagation;

pub use cbgr_elimination::{
    CbgrEliminationPass, CbgrEliminationStats, EscapeAnalysisEngine, EscapeAnalysisStats,
    EscapeCategory, OptimizationAction, OperationId, OperationInfo, ValueId, ValueInfo,
    is_cbgr_alloc, is_cbgr_check, is_cbgr_operation, may_cause_escape,
};
pub use context_mono::{
    CallSiteId, CallSiteInfo, ContextAnalysisEngine, ContextGetId, ContextGetInfo,
    ContextMonoStats, ContextMonomorphizationPass, ContextResolution, ContextTypeInfo,
    FunctionId, FunctionInfo, is_context_get, is_context_operation, is_context_provide,
};
pub use gpu_pipeline::{GpuPassConfig, GpuPassPipeline, GpuPipelineResult, GpuPipelineStats};
pub use llvm_lowering::LlvmLoweringPass;
pub use pipeline::{PassConfig, PassPipeline, PipelineResult, PipelineStats};
pub use refinement_propagation::{
    CompareOp, Predicate, RefinementAnalysisEngine, RefinementCheckInfo, RefinementId,
    RefinementPropagationPass, RefinementStats, ValuePredicates, is_refinement_check,
};

use crate::mlir::error::Result;
use verum_mlir::ir::Module;

// ============================================================================
// Pass Trait and Types
// ============================================================================

/// Result of running a single pass.
#[derive(Debug, Clone)]
pub struct PassResult {
    /// Whether the pass modified the IR.
    pub modified: bool,
    /// Statistics from the pass.
    pub stats: PassStats,
}

impl Default for PassResult {
    fn default() -> Self {
        Self {
            modified: false,
            stats: PassStats::default(),
        }
    }
}

impl PassResult {
    /// Create a result indicating no modification.
    pub fn unmodified() -> Self {
        Self::default()
    }

    /// Create a result indicating modification with stats.
    pub fn modified_with(stats: PassStats) -> Self {
        Self {
            modified: true,
            stats,
        }
    }
}

/// Generic statistics for a pass.
#[derive(Debug, Clone, Default)]
pub struct PassStats {
    /// Operations analyzed by the pass.
    pub operations_analyzed: usize,
    /// Operations modified by the pass.
    pub operations_modified: usize,
    /// Operations removed by the pass.
    pub operations_removed: usize,
    /// Operations added by the pass.
    pub operations_added: usize,
}

impl PassStats {
    /// Check if any changes were made.
    pub fn has_changes(&self) -> bool {
        self.operations_modified > 0 || self.operations_removed > 0 || self.operations_added > 0
    }

    /// Total operations affected.
    pub fn total_affected(&self) -> usize {
        self.operations_modified + self.operations_removed + self.operations_added
    }

    /// Merge with another stats.
    pub fn merge(&mut self, other: &PassStats) {
        self.operations_analyzed += other.operations_analyzed;
        self.operations_modified += other.operations_modified;
        self.operations_removed += other.operations_removed;
        self.operations_added += other.operations_added;
    }

    /// Format as summary string.
    pub fn summary(&self) -> String {
        format!(
            "analyzed={}, modified={}, removed={}, added={}",
            self.operations_analyzed,
            self.operations_modified,
            self.operations_removed,
            self.operations_added
        )
    }
}

/// Trait for Verum optimization passes.
///
/// All custom optimization passes should implement this trait.
/// The trait provides a standard interface for pass execution
/// with proper error handling and statistics collection.
pub trait VerumPass {
    /// Get the pass name (for logging and debugging).
    fn name(&self) -> &str;

    /// Run the pass on a module.
    ///
    /// Returns a result indicating whether the IR was modified
    /// and statistics about the pass execution.
    fn run(&self, module: &mut Module<'_>) -> Result<PassResult>;

    /// Get a description of what this pass does.
    fn description(&self) -> &str {
        "Verum optimization pass"
    }

    /// Whether this pass requires verification after running.
    fn requires_verification(&self) -> bool {
        true
    }

    /// Dependencies on other passes (by name).
    fn dependencies(&self) -> &[&str] {
        &[]
    }

    /// Passes that should run after this one (by name).
    fn invalidates(&self) -> &[&str] {
        &[]
    }
}

// ============================================================================
// Standard Pass Wrappers
// ============================================================================

/// Wrapper for MLIR's Canonicalization pass.
///
/// This pass applies canonicalization patterns to simplify the IR.
/// It's typically run early in the pipeline to normalize the IR.
pub struct CanonicalizationPass;

impl CanonicalizationPass {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CanonicalizationPass {
    fn default() -> Self {
        Self::new()
    }
}

impl VerumPass for CanonicalizationPass {
    fn name(&self) -> &str {
        "canonicalize"
    }

    fn description(&self) -> &str {
        "Apply canonicalization patterns to simplify IR"
    }

    fn run(&self, _module: &mut Module<'_>) -> Result<PassResult> {
        // This is run via MLIR PassManager, not directly
        Ok(PassResult::unmodified())
    }
}

/// Wrapper for MLIR's CSE (Common Subexpression Elimination) pass.
pub struct CsePass;

impl CsePass {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CsePass {
    fn default() -> Self {
        Self::new()
    }
}

impl VerumPass for CsePass {
    fn name(&self) -> &str {
        "cse"
    }

    fn description(&self) -> &str {
        "Common subexpression elimination"
    }

    fn run(&self, _module: &mut Module<'_>) -> Result<PassResult> {
        Ok(PassResult::unmodified())
    }
}

/// Wrapper for MLIR's SCCP pass.
pub struct SccpPass;

impl SccpPass {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SccpPass {
    fn default() -> Self {
        Self::new()
    }
}

impl VerumPass for SccpPass {
    fn name(&self) -> &str {
        "sccp"
    }

    fn description(&self) -> &str {
        "Sparse conditional constant propagation"
    }

    fn run(&self, _module: &mut Module<'_>) -> Result<PassResult> {
        Ok(PassResult::unmodified())
    }
}

/// Wrapper for MLIR's DCE pass.
pub struct DcePass;

impl DcePass {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DcePass {
    fn default() -> Self {
        Self::new()
    }
}

impl VerumPass for DcePass {
    fn name(&self) -> &str {
        "dce"
    }

    fn description(&self) -> &str {
        "Dead code elimination"
    }

    fn run(&self, _module: &mut Module<'_>) -> Result<PassResult> {
        Ok(PassResult::unmodified())
    }
}

/// Wrapper for MLIR's LICM pass.
pub struct LicmPass;

impl LicmPass {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LicmPass {
    fn default() -> Self {
        Self::new()
    }
}

impl VerumPass for LicmPass {
    fn name(&self) -> &str {
        "licm"
    }

    fn description(&self) -> &str {
        "Loop invariant code motion"
    }

    fn run(&self, _module: &mut Module<'_>) -> Result<PassResult> {
        Ok(PassResult::unmodified())
    }
}

/// Wrapper for MLIR's Mem2Reg pass.
pub struct Mem2RegPass;

impl Mem2RegPass {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Mem2RegPass {
    fn default() -> Self {
        Self::new()
    }
}

impl VerumPass for Mem2RegPass {
    fn name(&self) -> &str {
        "mem2reg"
    }

    fn description(&self) -> &str {
        "Promote memory to registers"
    }

    fn run(&self, _module: &mut Module<'_>) -> Result<PassResult> {
        Ok(PassResult::unmodified())
    }
}

/// Wrapper for MLIR's Inliner pass.
pub struct InlinerPass {
    /// Threshold for inlining (cost limit).
    threshold: usize,
}

impl InlinerPass {
    pub fn new() -> Self {
        Self { threshold: 100 }
    }

    pub fn with_threshold(mut self, threshold: usize) -> Self {
        self.threshold = threshold;
        self
    }
}

impl Default for InlinerPass {
    fn default() -> Self {
        Self::new()
    }
}

impl VerumPass for InlinerPass {
    fn name(&self) -> &str {
        "inline"
    }

    fn description(&self) -> &str {
        "Function inlining"
    }

    fn run(&self, _module: &mut Module<'_>) -> Result<PassResult> {
        Ok(PassResult::unmodified())
    }
}

/// Wrapper for MLIR's Symbol DCE pass.
pub struct SymbolDcePass;

impl SymbolDcePass {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SymbolDcePass {
    fn default() -> Self {
        Self::new()
    }
}

impl VerumPass for SymbolDcePass {
    fn name(&self) -> &str {
        "symbol-dce"
    }

    fn description(&self) -> &str {
        "Symbol dead code elimination"
    }

    fn run(&self, _module: &mut Module<'_>) -> Result<PassResult> {
        Ok(PassResult::unmodified())
    }
}

// ============================================================================
// Pass Registry
// ============================================================================

/// Registry of available passes.
#[derive(Debug, Default)]
pub struct PassRegistry {
    /// Registered pass names.
    pass_names: Vec<&'static str>,
}

impl PassRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a registry with all standard passes.
    pub fn with_all_passes() -> Self {
        let mut registry = Self::new();
        registry.register_all();
        registry
    }

    /// Register all available passes.
    pub fn register_all(&mut self) {
        self.pass_names.extend(&[
            // Domain passes
            "cbgr-elimination",
            "context-monomorphization",
            "refinement-propagation",
            // Standard passes
            "canonicalize",
            "cse",
            "sccp",
            "dce",
            "licm",
            "mem2reg",
            "inline",
            "symbol-dce",
            // Lowering passes
            "llvm-lowering",
        ]);
    }

    /// Check if a pass is registered.
    pub fn is_registered(&self, name: &str) -> bool {
        self.pass_names.iter().any(|&n| n == name)
    }

    /// Get all registered pass names.
    pub fn pass_names(&self) -> &[&'static str] {
        &self.pass_names
    }
}

// ============================================================================
// Optimization Level Presets
// ============================================================================

/// Optimization level presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizationLevel {
    /// No optimization (O0).
    None = 0,
    /// Basic optimization (O1).
    Basic = 1,
    /// Standard optimization (O2).
    Standard = 2,
    /// Aggressive optimization (O3).
    Aggressive = 3,
    /// Size optimization (Os).
    Size = 4,
}

impl OptimizationLevel {
    /// Get enabled passes for this level.
    pub fn enabled_passes(&self) -> Vec<&'static str> {
        match self {
            Self::None => vec![],
            Self::Basic => vec![
                "canonicalize",
                "cse",
            ],
            Self::Standard => vec![
                "canonicalize",
                "cse",
                "cbgr-elimination",
                "context-monomorphization",
                "refinement-propagation",
                "sccp",
                "dce",
            ],
            Self::Aggressive => vec![
                "canonicalize",
                "cse",
                "cbgr-elimination",
                "context-monomorphization",
                "refinement-propagation",
                "sccp",
                "licm",
                "inline",
                "mem2reg",
                "dce",
                "symbol-dce",
            ],
            Self::Size => vec![
                "canonicalize",
                "cse",
                "cbgr-elimination",
                "dce",
                "symbol-dce",
            ],
        }
    }

    /// Get the LLVM optimization level.
    pub fn llvm_level(&self) -> usize {
        match self {
            Self::None => 0,
            Self::Basic => 1,
            Self::Standard | Self::Size => 2,
            Self::Aggressive => 3,
        }
    }
}

impl Default for OptimizationLevel {
    fn default() -> Self {
        Self::Standard
    }
}

impl From<usize> for OptimizationLevel {
    fn from(level: usize) -> Self {
        match level {
            0 => Self::None,
            1 => Self::Basic,
            2 => Self::Standard,
            3 => Self::Aggressive,
            _ => Self::Aggressive,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pass_stats_merge() {
        let mut stats1 = PassStats {
            operations_analyzed: 10,
            operations_modified: 2,
            operations_removed: 1,
            operations_added: 0,
        };

        let stats2 = PassStats {
            operations_analyzed: 5,
            operations_modified: 1,
            operations_removed: 0,
            operations_added: 1,
        };

        stats1.merge(&stats2);

        assert_eq!(stats1.operations_analyzed, 15);
        assert_eq!(stats1.operations_modified, 3);
        assert_eq!(stats1.operations_removed, 1);
        assert_eq!(stats1.operations_added, 1);
    }

    #[test]
    fn test_pass_stats_has_changes() {
        let stats = PassStats {
            operations_analyzed: 10,
            operations_modified: 0,
            operations_removed: 0,
            operations_added: 0,
        };
        assert!(!stats.has_changes());

        let stats = PassStats {
            operations_analyzed: 10,
            operations_modified: 1,
            operations_removed: 0,
            operations_added: 0,
        };
        assert!(stats.has_changes());
    }

    #[test]
    fn test_optimization_level_passes() {
        assert!(OptimizationLevel::None.enabled_passes().is_empty());
        assert!(OptimizationLevel::Basic.enabled_passes().contains(&"canonicalize"));
        assert!(OptimizationLevel::Standard.enabled_passes().contains(&"cbgr-elimination"));
        assert!(OptimizationLevel::Aggressive.enabled_passes().contains(&"inline"));
    }

    #[test]
    fn test_optimization_level_llvm() {
        assert_eq!(OptimizationLevel::None.llvm_level(), 0);
        assert_eq!(OptimizationLevel::Basic.llvm_level(), 1);
        assert_eq!(OptimizationLevel::Standard.llvm_level(), 2);
        assert_eq!(OptimizationLevel::Aggressive.llvm_level(), 3);
    }

    #[test]
    fn test_pass_registry() {
        let registry = PassRegistry::with_all_passes();
        assert!(registry.is_registered("cbgr-elimination"));
        assert!(registry.is_registered("canonicalize"));
        assert!(registry.is_registered("cse"));
        assert!(!registry.is_registered("unknown-pass"));
    }
}
