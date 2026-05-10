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

/// Discriminator for [`VerificationError`] — zero-sized
/// projection classifying the failure modes of verification
/// passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VerificationErrorKind {
    Failed,
    Timeout,
    Internal,
}

/// Per-variant projection for [`VerificationErrorKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerificationErrorKindMeta {
    /// Lower-snake-case wire form for telemetry surfaces.
    pub name: &'static str,
    /// The pass *concluded* the verification failed —
    /// `Failed` singleton.  The negative verdict.
    pub is_negative_verdict: bool,
    /// The pass ran out of time — `Timeout` singleton.
    /// Distinct from `Failed` (genuine refutation).
    pub is_time_bound_failure: bool,
    /// Internal-error catch-all — `Internal` singleton.
    pub is_internal: bool,
}

impl VerificationErrorKind {
    /// All variants in declaration order.
    pub const ALL: &'static [Self] =
        &[Self::Failed, Self::Timeout, Self::Internal];

    /// Static fact-pack.
    pub const fn meta(self) -> VerificationErrorKindMeta {
        match self {
            VerificationErrorKind::Failed => VerificationErrorKindMeta {
                name: "failed",
                is_negative_verdict: true,
                is_time_bound_failure: false,
                is_internal: false,
            },
            VerificationErrorKind::Timeout => VerificationErrorKindMeta {
                name: "timeout",
                is_negative_verdict: false,
                is_time_bound_failure: true,
                is_internal: false,
            },
            VerificationErrorKind::Internal => VerificationErrorKindMeta {
                name: "internal",
                is_negative_verdict: false,
                is_time_bound_failure: false,
                is_internal: true,
            },
        }
    }
}

impl VerificationError {
    /// Discriminator projection — strip the payload, keep tag.
    pub const fn kind(&self) -> VerificationErrorKind {
        match self {
            VerificationError::Failed(_) => VerificationErrorKind::Failed,
            VerificationError::Timeout(_) => VerificationErrorKind::Timeout,
            VerificationError::Internal(_) => VerificationErrorKind::Internal,
        }
    }

    /// Returns the inner message text — every variant carries
    /// one.  Pinned via the drift test.
    pub fn message(&self) -> &Text {
        match self {
            VerificationError::Failed(t) => t,
            VerificationError::Timeout(t) => t,
            VerificationError::Internal(t) => t,
        }
    }
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

#[cfg(test)]
mod kind_meta_drift_pins {
    use super::*;

    /// Drift-pin: `VerificationErrorKind` discriminator
    /// projection.  Pins variant count, perfect-partition over
    /// the three classifier flags, live-payload kind() +
    /// message() accessors.
    #[test]
    fn meta_pin_verification_error_kind_round_trip_and_partitions() {
        assert_eq!(VerificationErrorKind::ALL.len(), 3);

        // Perfect partition: every variant flips exactly one of
        // {is_negative_verdict, is_time_bound_failure,
        // is_internal}.
        for k in VerificationErrorKind::ALL {
            let m = k.meta();
            let count = (m.is_negative_verdict as u32)
                + (m.is_time_bound_failure as u32)
                + (m.is_internal as u32);
            assert_eq!(count, 1, "{:?}: must flip exactly one classifier", k);
        }

        let mut seen = std::collections::HashSet::new();
        for k in VerificationErrorKind::ALL {
            assert!(seen.insert(k.meta().name));
        }

        // Live-payload kind() + message() routing.
        let f = VerificationError::Failed(Text::from("counterexample at x=0"));
        assert_eq!(f.kind(), VerificationErrorKind::Failed);
        assert_eq!(f.message().as_str(), "counterexample at x=0");

        let to = VerificationError::Timeout(Text::from("z3 timeout 10s"));
        assert_eq!(to.kind(), VerificationErrorKind::Timeout);

        let i = VerificationError::Internal(Text::from("invariant violation"));
        assert_eq!(i.kind(), VerificationErrorKind::Internal);
    }
}
