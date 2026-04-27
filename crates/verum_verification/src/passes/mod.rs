//! Verification Compiler Passes
//!
//! Implements compiler passes for gradual verification:
//! - Verification level inference (`level_inference.rs`)
//! - Kernel-rule re-checking (`kernel_recheck.rs`)
//! - Boundary detection (`boundary_detection.rs`)
//! - Transition recommendation (`transition_recommendation.rs`)
//! - SMT-based contract verification (`smt.rs`)
//! - Verification pipeline composition (`pipeline.rs`)
//!
//! These passes run during compilation to: (1) infer verification levels from
//! annotations and code context, (2) detect boundaries between verification
//! levels, (3) generate proof obligations at boundaries, and (4) recommend
//! transitions to higher verification levels based on code metrics.
//!
//! # Module structure (#199 split — pass-per-module)
//!
//! Pre-split, all six pass implementations + pipeline + result/error types
//! lived in a single `passes.rs` (1130 LOC). The split establishes one file
//! per pass for auditability + reduces per-file complexity for maintainers.
//! Public API is preserved via re-exports below — no caller-visible change.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use verum_common::{List, Text};

use crate::cost::VerificationCost;
use crate::level::VerificationLevel;

// =============================================================================
// Verification pass trait + result/error types (shared across all passes)
// =============================================================================

/// Verification pass errors
#[derive(Debug, Error)]
pub enum VerificationError {
    #[error("verification failed: {0}")]
    Failed(Text),

    #[error("timeout: {0}")]
    Timeout(Text),

    #[error("internal error: {0}")]
    Internal(Text),
}

/// Result of a verification pass
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    /// Whether verification succeeded
    pub success: bool,

    /// Verification level used
    pub level: VerificationLevel,

    /// Time taken
    pub duration: Duration,

    /// Costs per function
    pub costs: List<VerificationCost>,

    /// Number of functions verified
    pub functions_verified: usize,

    /// Number of boundaries detected
    pub boundaries_detected: usize,

    /// Number of proof obligations generated
    pub obligations_generated: usize,
}

impl VerificationResult {
    /// Create a successful result
    pub fn success(
        level: VerificationLevel,
        duration: Duration,
        costs: List<VerificationCost>,
    ) -> Self {
        let functions_verified = costs.len();
        Self {
            success: true,
            level,
            duration,
            costs,
            functions_verified,
            boundaries_detected: 0,
            obligations_generated: 0,
        }
    }

    /// Create a failure result
    pub fn failure(level: VerificationLevel, duration: Duration) -> Self {
        Self {
            success: false,
            level,
            duration,
            costs: List::new(),
            functions_verified: 0,
            boundaries_detected: 0,
            obligations_generated: 0,
        }
    }
}

/// pass classification for the
/// fail-fast / aggregate decision.
///
/// The default classification is [`PassClassification::SoundnessCritical`]
/// so unmodified passes preserve pre-V8 fail-fast semantics. Each
/// pass implementer can opt into [`PassClassification::Informational`]
/// to indicate "diagnostic-only — don't halt the pipeline if I
/// fail". The pipeline composer reads this classification together
/// with its [`crate::passes::pipeline::PipelineMode`] to decide
/// whether to halt on failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PassClassification {
    /// Halts the pipeline on failure when run under
    /// `PipelineMode::Default`. Soundness-critical passes are those
    /// whose failure invalidates the assumptions of every
    /// downstream pass — kernel formation rules, refinement-
    /// stratification rules, framework-conflict R3/R4 errors.
    SoundnessCritical,
    /// Never halts the pipeline. Diagnostic-only passes whose
    /// failure-shape is informative but not soundness-fatal:
    /// boundary detection (the boundary is wrong, not the
    /// program), transition recommendation (advisory).
    Informational,
}

/// Verification pass trait
pub trait VerificationPass {
    /// Run the pass on a module
    fn run(
        &mut self,
        module: &verum_ast::Module,
        ctx: &mut crate::context::VerificationContext,
    ) -> Result<VerificationResult, VerificationError>;

    /// Name of this pass
    fn name(&self) -> &str;

    /// pass classification. Default
    /// `SoundnessCritical` to preserve pre-V8 fail-fast semantics
    /// for unmodified passes; passes that want their failures to
    /// be aggregated rather than halt the pipeline override to
    /// [`PassClassification::Informational`].
    fn classification(&self) -> PassClassification {
        PassClassification::SoundnessCritical
    }
}

// =============================================================================
// Sub-modules (one pass per file, plus the pipeline composer)
// =============================================================================

pub mod boundary_detection;
pub mod kernel_recheck;
pub mod level_inference;
pub mod pipeline;
pub mod smt;
pub mod transition_recommendation;

// =============================================================================
// Re-exports — preserve pre-split public API
// =============================================================================

pub use boundary_detection::BoundaryDetectionPass;
pub use kernel_recheck::KernelRecheckPass;
pub use level_inference::LevelInferencePass;
pub use pipeline::VerificationPipeline;
pub use smt::{
    SmtVerificationPass, SmtVerificationResult, SmtVerificationStats, VCStatus,
    VCVerificationResult,
};
pub use transition_recommendation::{TransitionRecommendation, TransitionRecommendationPass};
