//! Verification pipeline composer.
//!
//! `VerificationPipeline` runs an ordered list of [`VerificationPass`]
//! implementations on a module, with **fail-fast** semantics: when
//! any pass returns `result.success == false`, the pipeline records
//! that failure and skips the remaining passes.
//!
//! Two built-in pipelines:
//!
//!   • [`VerificationPipeline::static_analysis_pipeline`] — 5
//!     lightweight passes (level inference + kernel-recheck +
//!     hygiene-recheck + boundary detection + transition
//!     recommendation). No SMT.
//!   • [`VerificationPipeline::full_verification_pipeline`] — adds
//!     `SmtVerificationPass` after the static-analysis chain.

use std::fmt;

use verum_ast::Module;
use verum_common::List;

use crate::context::VerificationContext;
use crate::level::VerificationLevel;
use crate::transition::TransitionStrategy;

use super::{
    BoundaryDetectionPass, KernelRecheckPass, LevelInferencePass, PassClassification,
    SmtVerificationPass, TransitionRecommendationPass, VerificationError, VerificationPass,
    VerificationResult,
};

/// V8 (#208, B7) — pipeline-level halt policy.
///
/// Mediates between the two valid contracts that the V0 pipeline
/// could not separate:
///
///   * **Default** — halt on `SoundnessCritical` failure; continue
///     through `Informational` failures. This is the new V8 default
///     and matches the practical separation between formation
///     errors (which downstream passes depend on) and advisory
///     diagnostics (which don't).
///   * **StrictFailFast** — halt on *any* failure regardless of
///     classification. Equivalent to pre-V8 behaviour. Useful for
///     CI gates that want a single first-failure stop.
///   * **Aggregate** — never halt. Run every pass, accumulate every
///     diagnostic. Useful for IDE / batch-report workflows where
///     surfacing all errors at once is more valuable than the
///     short-circuit speedup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineMode {
    /// V8 default — halt only on SoundnessCritical pass failure.
    Default,
    /// Pre-V8 — halt on any pass failure (regardless of class).
    StrictFailFast,
    /// Never halt — accumulate every diagnostic.
    Aggregate,
}

impl PipelineMode {
    /// Decide whether a pass result should halt the pipeline.
    pub fn should_halt(self, classification: PassClassification, success: bool) -> bool {
        if success {
            return false;
        }
        match self {
            PipelineMode::Default => {
                classification == PassClassification::SoundnessCritical
            }
            PipelineMode::StrictFailFast => true,
            PipelineMode::Aggregate => false,
        }
    }
}

/// Verification pipeline combining all passes.
pub struct VerificationPipeline {
    passes: List<Box<dyn VerificationPass>>,
    /// V8 (#208, B7) — halt policy. Default
    /// [`PipelineMode::Default`] gates fail-fast on
    /// SoundnessCritical class only.
    mode: PipelineMode,
}

impl VerificationPipeline {
    /// Create a new (empty) verification pipeline.
    pub fn new() -> Self {
        Self {
            passes: List::new(),
            mode: PipelineMode::Default,
        }
    }

    /// V8 (#208, B7) — configure the halt policy. Builder-style;
    /// returns `self` so call-sites read naturally:
    /// `VerificationPipeline::full_verification_pipeline().with_mode(PipelineMode::Aggregate)`.
    pub fn with_mode(mut self, mode: PipelineMode) -> Self {
        self.mode = mode;
        self
    }

    /// V8 (#208, B7) — read-only accessor for the configured halt
    /// policy.
    pub fn mode(&self) -> PipelineMode {
        self.mode
    }

    /// Add a pass to the pipeline.
    pub fn add_pass(&mut self, pass: Box<dyn VerificationPass>) {
        self.passes.push(pass);
    }

    /// Run all passes, halting per the configured
    /// [`PipelineMode`].
    ///
    /// Halt semantics by mode (V8, B7):
    ///   * `Default` (V8 default) — halt on `SoundnessCritical`
    ///     pass failure; continue through `Informational` ones.
    ///   * `StrictFailFast` (pre-V8) — halt on any pass failure.
    ///   * `Aggregate` — never halt; collect every diagnostic.
    ///
    /// The failed pass's result IS pushed into the returned list so
    /// callers can read the diagnostic; only the *subsequent*
    /// passes are skipped (or run, depending on mode).
    pub fn run_all(
        &mut self,
        module: &Module,
        ctx: &mut VerificationContext,
    ) -> Result<List<VerificationResult>, VerificationError> {
        let mut results = List::new();

        for pass in &mut self.passes {
            let classification = pass.classification();
            let result = pass.run(module, ctx)?;
            let halt = self.mode.should_halt(classification, result.success);
            results.push(result);
            if halt {
                break;
            }
        }

        Ok(results)
    }

    /// Create the **static-analysis** pipeline: 5 lightweight
    /// passes (level inference + kernel-recheck + hygiene-recheck +
    /// boundary detection + transition recommendation). Does **not**
    /// include `SmtVerificationPass` — SMT is a heavy dependency
    /// and not always available; callers that want the full
    /// pipeline should use [`Self::full_verification_pipeline`].
    ///
    /// Renamed from `default_pipeline` (#202): the original name
    /// was misleading because users reasonably expected "default
    /// verification" to include SMT discharge. The 5-pass
    /// composition is the right default for AOT/build paths that
    /// want kernel + hygiene + transition advice without paying
    /// the SMT round-trip cost.
    pub fn static_analysis_pipeline() -> Self {
        let mut pipeline = Self::new();

        pipeline.add_pass(Box::new(LevelInferencePass::new(
            VerificationLevel::Runtime,
        )));
        // KernelRecheckPass runs *after* level inference (which
        // sets per-function scopes) but *before* boundary
        // detection / transition recommendation — kernel-rule
        // failures are formation errors that should short-circuit
        // the rest of the pipeline (#187 V0).
        pipeline.add_pass(Box::new(KernelRecheckPass::new()));
        // HygieneRecheckPass (#190) — framework-author discipline
        // (R1 brand-prefix names, R2 ε-coordinate canonicalisable,
        // R3 meta-classifier uniqueness). R1/R2 are Warnings;
        // R3 is Error and triggers fail-fast. Runs after
        // KernelRecheckPass so kernel formation failures take
        // precedence in the diagnostic stream.
        pipeline.add_pass(Box::new(crate::framework_hygiene::HygieneRecheckPass::new()));
        pipeline.add_pass(Box::new(BoundaryDetectionPass::new()));
        pipeline.add_pass(Box::new(TransitionRecommendationPass::new(
            TransitionStrategy::Balanced,
        )));

        pipeline
    }

    /// Backwards-compat alias for [`Self::static_analysis_pipeline`].
    /// New callers should use the explicit name to make the
    /// SMT-absence intentional. This alias will be removed in a
    /// future major version.
    #[deprecated(
        since = "0.2.0",
        note = "Use static_analysis_pipeline() — this name was misleading because \
                no SMT pass is included. See task #202 for the rationale."
    )]
    pub fn default_pipeline() -> Self {
        Self::static_analysis_pipeline()
    }

    /// Create the **full-verification** pipeline: static-analysis
    /// passes + `SmtVerificationPass` for actual SMT discharge of
    /// refinement obligations. Fail-fast applies (#187 contract):
    /// any pass returning `success == false` halts the rest.
    ///
    /// SMT verification is the default-on terminal pass; modules
    /// passing the static-analysis chain have their refinement
    /// types subjected to Z3 portfolio dispatch.
    pub fn full_verification_pipeline() -> Self {
        let mut pipeline = Self::static_analysis_pipeline();
        pipeline.add_pass(Box::new(SmtVerificationPass::new()));
        pipeline
    }
}

impl Default for VerificationPipeline {
    fn default() -> Self {
        // Default = static-analysis pipeline (no SMT). The full
        // verification path with SMT discharge is opt-in via
        // [`Self::full_verification_pipeline`] — see #202.
        Self::static_analysis_pipeline()
    }
}

impl fmt::Debug for VerificationPipeline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerificationPipeline")
            .field("passes", &format!("{} passes", self.passes.len()))
            .finish()
    }
}
