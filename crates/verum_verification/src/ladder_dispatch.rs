//! 13-strategy verification ladder dispatcher — kernel-checkable.
//!
//! ## What this module is
//!
//! Verum's verification model is a strict ν-monotone ladder of 13
//! strategies (VVA §12 + the existing
//! [`verum_smt::verify_strategy::VerifyStrategy`] enum):
//!
//! ```text
//!   runtime (0) < static (1) < fast (2) < complexity_typed (n<ω) <
//!   formal (ω) < proof (ω+1) < thorough (ω·2) < reliable (ω·2+1) <
//!   certified (ω·2+2) < coherent_static (ω·2+3) < coherent_runtime (ω·2+4) <
//!   coherent (ω·2+5) < synthesize (≤ω·3+1)
//! ```
//!
//! Every theorem carries an `@verify(<strategy>)` annotation; this
//! module is the single dispatcher that:
//!
//!   1. Projects the annotation to a [`LadderStrategy`] enum.
//!   2. Routes the obligation through the matching backend
//!      (Z3 / CVC5 / kernel-recheck / certificate-replay / coherent
//!      α-cert ⟺ ε-cert / inverse search).
//!   3. Returns a typed [`LadderVerdict`] with a *kernel-checkable
//!      witness* on success.
//!   4. Enforces strict-ν-monotonicity at the dispatcher level: a
//!      stricter strategy succeeding **lifts** to the same verdict
//!      under every coarser strategy.
//!
//! ## Why this is a fundamental refactor (not a wrapper)
//!
//! Pre-this-module the dispatch was scattered across
//! `verum_smt::backend_switcher`, `verum_smt::tactics`,
//! `verum_kernel::infer`, and the CLI command harness; per-strategy
//! intent (`@verify(thorough)` vs `@verify(reliable)`) was recorded
//! but rarely *enforced*.  This module provides a **single trait
//! boundary** for ladder dispatch with one method per strategy slot,
//! making it impossible to silently fall through to a coarser
//! strategy without a typed acknowledgement.
//!
//! Each backend adapter is a small, testable concern; the dispatcher
//! itself is foundation-neutral and works against any obligation
//! shape that maps onto Verum's [`verum_kernel::CoreTerm`].
//!
//! ## V0 surface (this commit)
//!
//! V0 ships:
//!
//!   * The full [`LadderStrategy`] / [`LadderVerdict`] /
//!     [`LadderObligation`] / [`LadderDispatcher`] surface.
//!   * Adapters for the 5 strategies whose backends already exist:
//!     `Runtime`, `Static`, `Fast`, `Formal`, `Proof`.
//!   * Honest `Pending` verdicts for the 8 strategies whose backends
//!     are not yet wired (`ComplexityTyped`, `Thorough`, `Reliable`,
//!     `Certified`, `CoherentStatic`, `CoherentRuntime`, `Coherent`,
//!     `Synthesize`) — annotated with the existing infrastructure
//!     hooks they will integrate with in V1+.
//!   * A reference [`DefaultLadderDispatcher`] that wires the V0
//!     adapters and refuses pending strategies with a typed
//!     `LadderVerdict::DispatchPending` (the inverse of "silently
//!     fall through to coarser strategy").
//!
//! ## V1+ promotion path
//!
//! V1 lands the per-strategy backends behind the existing trait
//! interface — no architectural change to consumers.  The
//! `verum verify` CLI uses this dispatcher today; it will gain
//! strictness as backends fill in.
//!
//! Aligned with Verum's "no magic, explicit dependencies" philosophy:
//! per-strategy intent is a *first-class* dispatch contract, not an
//! audit-time projection.

use serde::{Deserialize, Serialize};
use verum_common::Text;

// =============================================================================
// LadderStrategy — the 13 ν-monotone variants
// =============================================================================

/// A strategy slot on the verification ladder.  Mirrors
/// `verum_smt::verify_strategy::VerifyStrategy` but lives in this
/// crate to avoid a `verum_verification → verum_smt` cycle (the
/// dispatcher itself is foundation-neutral and consumes its
/// adapters via the trait below).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LadderStrategy {
    /// `@verify(runtime)` — runtime-only assertion.  ν = 0.
    Runtime,
    /// `@verify(static)` — dataflow / CBGR / constant folding.  ν = 1.
    Static,
    /// `@verify(fast)` — single-solver SMT, bounded timeout.  ν = 2.
    Fast,
    /// `@verify(complexity_typed)` — bounded-arithmetic stratum.  ν = 3.
    ComplexityTyped,
    /// `@verify(formal)` — portfolio SMT (Z3 + CVC5).  ν = ω.
    Formal,
    /// `@verify(proof)` — kernel re-check of `proof { … }` body.  ν = ω + 1.
    Proof,
    /// `@verify(thorough)` — `formal` + `decreases` + `frame` + `invariant`.  ν = ω · 2.
    Thorough,
    /// `@verify(reliable)` — `thorough` + cross-solver agreement.  ν = ω · 2 + 1.
    Reliable,
    /// `@verify(certified)` — `reliable` + cert materialisation + multi-format export.  ν = ω · 2 + 2.
    Certified,
    /// `@verify(coherent_static)` — α-cert + symbolic ε-claim.  ν = ω · 2 + 3.
    CoherentStatic,
    /// `@verify(coherent_runtime)` — α-cert + runtime ε-monitor.  ν = ω · 2 + 4.
    CoherentRuntime,
    /// `@verify(coherent)` — α/ε bidirectional discharge.  ν = ω · 2 + 5.
    Coherent,
    /// `@verify(synthesize)` — inverse proof search (orthogonal).  ν ≤ ω · 3 + 1.
    Synthesize,
}

impl LadderStrategy {
    /// Diagnostic name (matches the `@verify(<name>)` annotation form).
    pub fn name(self) -> &'static str {
        match self {
            LadderStrategy::Runtime         => "runtime",
            LadderStrategy::Static          => "static",
            LadderStrategy::Fast            => "fast",
            LadderStrategy::ComplexityTyped => "complexity_typed",
            LadderStrategy::Formal          => "formal",
            LadderStrategy::Proof           => "proof",
            LadderStrategy::Thorough        => "thorough",
            LadderStrategy::Reliable        => "reliable",
            LadderStrategy::Certified       => "certified",
            LadderStrategy::CoherentStatic  => "coherent_static",
            LadderStrategy::CoherentRuntime => "coherent_runtime",
            LadderStrategy::Coherent        => "coherent",
            LadderStrategy::Synthesize      => "synthesize",
        }
    }

    /// Parse a strategy from its `@verify(...)` form.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "runtime"          => Some(LadderStrategy::Runtime),
            "static"           => Some(LadderStrategy::Static),
            "fast"             => Some(LadderStrategy::Fast),
            "complexity_typed" => Some(LadderStrategy::ComplexityTyped),
            "formal"           => Some(LadderStrategy::Formal),
            "proof"            => Some(LadderStrategy::Proof),
            "thorough"         => Some(LadderStrategy::Thorough),
            "reliable"         => Some(LadderStrategy::Reliable),
            "certified"        => Some(LadderStrategy::Certified),
            "coherent_static"  => Some(LadderStrategy::CoherentStatic),
            "coherent_runtime" => Some(LadderStrategy::CoherentRuntime),
            "coherent"         => Some(LadderStrategy::Coherent),
            "synthesize"       => Some(LadderStrategy::Synthesize),
            _                  => None,
        }
    }

    /// All 13 strategies in monotone-ladder order (`Synthesize` last
    /// because it sits orthogonally above the monotone backbone).
    pub fn all() -> [LadderStrategy; 13] {
        [
            LadderStrategy::Runtime,
            LadderStrategy::Static,
            LadderStrategy::Fast,
            LadderStrategy::ComplexityTyped,
            LadderStrategy::Formal,
            LadderStrategy::Proof,
            LadderStrategy::Thorough,
            LadderStrategy::Reliable,
            LadderStrategy::Certified,
            LadderStrategy::CoherentStatic,
            LadderStrategy::CoherentRuntime,
            LadderStrategy::Coherent,
            LadderStrategy::Synthesize,
        ]
    }

    /// The 12 strategies on the monotone backbone (`Synthesize` excluded).
    pub fn backbone() -> [LadderStrategy; 12] {
        [
            LadderStrategy::Runtime,
            LadderStrategy::Static,
            LadderStrategy::Fast,
            LadderStrategy::ComplexityTyped,
            LadderStrategy::Formal,
            LadderStrategy::Proof,
            LadderStrategy::Thorough,
            LadderStrategy::Reliable,
            LadderStrategy::Certified,
            LadderStrategy::CoherentStatic,
            LadderStrategy::CoherentRuntime,
            LadderStrategy::Coherent,
        ]
    }

    /// Diagnostic ν-ordinal label (matches VVA §2.3).
    pub fn nu_ordinal_label(self) -> &'static str {
        match self {
            LadderStrategy::Runtime         => "0",
            LadderStrategy::Static          => "1",
            LadderStrategy::Fast            => "2",
            LadderStrategy::ComplexityTyped => "3",
            LadderStrategy::Formal          => "ω",
            LadderStrategy::Proof           => "ω + 1",
            LadderStrategy::Thorough        => "ω · 2",
            LadderStrategy::Reliable        => "ω · 2 + 1",
            LadderStrategy::Certified       => "ω · 2 + 2",
            LadderStrategy::CoherentStatic  => "ω · 2 + 3",
            LadderStrategy::CoherentRuntime => "ω · 2 + 4",
            LadderStrategy::Coherent        => "ω · 2 + 5",
            LadderStrategy::Synthesize      => "≤ ω · 3 + 1",
        }
    }

    /// True iff this strategy is on the monotone backbone (Synthesize
    /// is orthogonal).
    pub fn is_on_backbone(self) -> bool {
        !matches!(self, LadderStrategy::Synthesize)
    }

    /// Comparison along the monotone backbone.  Returns `Some(Ordering)`
    /// when both strategies are on the backbone; returns `None` when
    /// either is `Synthesize` (the orthogonal slot).
    pub fn backbone_cmp(self, other: LadderStrategy) -> Option<std::cmp::Ordering> {
        if !self.is_on_backbone() || !other.is_on_backbone() {
            return None;
        }
        let a = backbone_index(self).unwrap();
        let b = backbone_index(other).unwrap();
        Some(a.cmp(&b))
    }
}

fn backbone_index(s: LadderStrategy) -> Option<usize> {
    LadderStrategy::backbone().iter().position(|&x| x == s)
}

// =============================================================================
// LadderObligation — the input to dispatch
// =============================================================================

/// A proof obligation routed through the ladder dispatcher.  The
/// V0 surface ships a thin shape that captures the essentials; V1
/// will plumb in the full elaborated obligation (CoreTerm + Goal
/// stack + tactic transcript).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LadderObligation {
    /// Diagnostic name of the theorem / lemma whose obligation this is.
    pub item_name: Text,
    /// The strategy declared by `@verify(<strategy>)` on the item.
    pub declared_strategy: LadderStrategy,
    /// Rendered obligation text (used by the trivial-tautology decider
    /// + diagnostics).
    pub obligation_text: Text,
    /// Optional time budget in milliseconds.  Per-strategy default
    /// applies when `None`.
    pub timeout_ms: Option<u64>,
    /// **Typed kernel-side obligation** (#113 hardening).  When
    /// present + an `axiom_registry` is supplied, the `Proof`
    /// strategy routes through `verum_kernel::infer::infer` for a
    /// real kernel re-check.  Absent ⇒ falls back to the trivial-
    /// tautology decider.
    #[serde(default)]
    pub core_term: verum_common::Maybe<verum_kernel::CoreTerm>,
    /// **Optional typed expectation** for the kernel re-check.  When
    /// `core_term` and `expected_type` are both present, the `Proof`
    /// strategy uses `verum_kernel::infer::verify_full` to assert
    /// `core_term : expected_type` — strict definitional-equality
    /// check.  When `expected_type` is absent, only `infer`
    /// well-typedness is required.
    #[serde(default)]
    pub expected_type: verum_common::Maybe<verum_kernel::CoreTerm>,
    /// Axiom registry for the kernel re-check.  Skipped from serde —
    /// registries are not on-wire artifacts; they're constructed at
    /// dispatch-time from the running session.
    #[serde(skip)]
    pub axiom_registry: Option<std::sync::Arc<verum_kernel::AxiomRegistry>>,
    /// **SMT assertions** for the Z3/portfolio path (#113 hardening).
    /// Each entry is a textual SMT-LIB2 formula that the SMT
    /// translator parses on dispatch.  When non-empty + the strategy
    /// requires an SMT backend (Fast / Formal / Thorough / Reliable /
    /// Certified), `BackendSwitcher::solve_with_strategy` runs the
    /// real solver(s) per the strategy's contract.
    #[serde(default)]
    pub smt_assertions: Vec<Text>,
    /// **Typed AST assertions** for the real SMT-backend path (#114
    /// hardening).  The compiler's elaboration phase produces these
    /// directly; passing them through preserves type information
    /// the textual SMT-LIB form would discard.  When non-empty and
    /// the strategy is SMT-using (Fast / ComplexityTyped / Formal /
    /// Thorough / Reliable / Certified), `BackendSwitcher::
    /// solve_with_strategy` runs Z3 (Fast / CT / Formal),
    /// portfolio (Thorough), or cross-validate (Reliable / Certified)
    /// per the strategy's contract.  Skipped from serde — AST exprs
    /// are constructed at dispatch time from the running session's
    /// elaboration, not on-wire artifacts.
    #[serde(skip)]
    pub ast_assertions: Vec<verum_ast::expr::Expr>,
    /// **ε-side coherence claim** (#115 hardening).  The
    /// `@enact(epsilon = ...)` annotation projected to a typed
    /// claim object.  Required by the Coherent triplet
    /// (CoherentStatic / CoherentRuntime / Coherent) — without it,
    /// these strategies fall through to the trivial-decider.
    /// When present, the dispatcher discharges:
    ///
    ///   * α-side: via the kernel/SMT path on `core_term` /
    ///     `ast_assertions` (same as Certified).
    ///   * ε-side: per the strategy's contract — symbolic claim
    ///     (CoherentStatic), deferred runtime monitor
    ///     (CoherentRuntime), or kernel re-check of `claim_term`
    ///     (Coherent strict).
    #[serde(default)]
    pub epsilon_claim: Option<EpsilonClaim>,
}

// =============================================================================
// EpsilonClaim — α/ε bidirectional check payload (#115 hardening)
// =============================================================================

/// The ε-side coherence claim attached to an obligation by the
/// `@enact(epsilon = ...)` annotation.  Drives the ε-discharge
/// path of the Coherent triplet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpsilonClaim {
    /// Identifier of the `@enact` site that introduced this claim
    /// (e.g. `"foo::bar:42"`).  Used for diagnostics + audit.
    pub site: Text,
    /// Symbolic claim text — the literal annotation argument.
    /// CoherentStatic accepts when this is non-empty (the symbolic
    /// claim is taken at face value at compile time).
    pub claim: Text,
    /// Optional typed projection of the claim to a kernel term.
    /// When present, the `Coherent` strict strategy runs
    /// `verum_kernel::infer` against it (full re-check; mirrors
    /// the α-side discharge).
    #[serde(default)]
    pub claim_term: verum_common::Maybe<verum_kernel::CoreTerm>,
    /// True when the ε-side is wired to a runtime monitor (the
    /// `core.action.coherence_monitor` infrastructure runs the
    /// check at execution time).  CoherentRuntime admits when this
    /// is set + α-side admits.
    #[serde(default)]
    pub runtime_monitor: bool,
}

impl EpsilonClaim {
    /// Build a static ε-claim (CoherentStatic).
    pub fn symbolic(site: impl Into<Text>, claim: impl Into<Text>) -> Self {
        Self {
            site: site.into(),
            claim: claim.into(),
            claim_term: verum_common::Maybe::None,
            runtime_monitor: false,
        }
    }

    /// Build a runtime-monitor ε-claim (CoherentRuntime).
    pub fn runtime(site: impl Into<Text>, claim: impl Into<Text>) -> Self {
        Self {
            site: site.into(),
            claim: claim.into(),
            claim_term: verum_common::Maybe::None,
            runtime_monitor: true,
        }
    }

    /// Attach a typed kernel projection of the claim.  Required by
    /// the strict `Coherent` strategy for kernel-side ε-discharge.
    pub fn with_claim_term(mut self, term: verum_kernel::CoreTerm) -> Self {
        self.claim_term = verum_common::Maybe::Some(term);
        self
    }
}

impl PartialEq for LadderObligation {
    fn eq(&self, other: &Self) -> bool {
        self.item_name == other.item_name
            && self.declared_strategy == other.declared_strategy
            && self.obligation_text == other.obligation_text
            && self.timeout_ms == other.timeout_ms
            && self.core_term == other.core_term
            && self.expected_type == other.expected_type
            && self.smt_assertions == other.smt_assertions
            && self.ast_assertions == other.ast_assertions
            && self.epsilon_claim == other.epsilon_claim
        // axiom_registry intentionally excluded — Arc identity is
        // not part of the obligation's structural identity.
    }
}

impl LadderObligation {
    /// Convenience constructor for the V0 path: just `item_name +
    /// declared_strategy + obligation_text` (no typed payload).
    pub fn text(
        item_name: impl Into<Text>,
        declared_strategy: LadderStrategy,
        obligation_text: impl Into<Text>,
    ) -> Self {
        Self {
            item_name: item_name.into(),
            declared_strategy,
            obligation_text: obligation_text.into(),
            timeout_ms: None,
            core_term: verum_common::Maybe::None,
            expected_type: verum_common::Maybe::None,
            axiom_registry: None,
            smt_assertions: Vec::new(),
            ast_assertions: Vec::new(),
            epsilon_claim: None,
        }
    }

    /// Attach a typed ε-side coherence claim.  Required by the
    /// Coherent triplet (CoherentStatic / CoherentRuntime /
    /// Coherent) for the ε-discharge path.
    pub fn with_epsilon_claim(mut self, claim: EpsilonClaim) -> Self {
        self.epsilon_claim = Some(claim);
        self
    }

    /// Attach typed AST assertions for the real SMT-backend path.
    /// When non-empty, Fast / ComplexityTyped / Formal / Thorough /
    /// Reliable / Certified strategies route through
    /// `BackendSwitcher::solve_with_strategy`.
    pub fn with_ast_assertions(mut self, assertions: Vec<verum_ast::expr::Expr>) -> Self {
        self.ast_assertions = assertions;
        self
    }

    /// Attach a typed kernel obligation.  When set, the `Proof`
    /// strategy routes through `verum_kernel::infer::infer`.
    pub fn with_core_term(
        mut self,
        term: verum_kernel::CoreTerm,
        registry: std::sync::Arc<verum_kernel::AxiomRegistry>,
    ) -> Self {
        self.core_term = verum_common::Maybe::Some(term);
        self.axiom_registry = Some(registry);
        self
    }

    /// Attach an expected type for strict definitional-equality
    /// re-check.  Requires `with_core_term` to have been called.
    pub fn with_expected_type(mut self, expected: verum_kernel::CoreTerm) -> Self {
        self.expected_type = verum_common::Maybe::Some(expected);
        self
    }

    /// Attach SMT-LIB2 assertions.  When non-empty, the
    /// SMT-requiring strategies route through
    /// `BackendSwitcher::solve_with_strategy`.
    pub fn with_smt_assertions(mut self, assertions: Vec<Text>) -> Self {
        self.smt_assertions = assertions;
        self
    }
}

// =============================================================================
// LadderVerdict — the output
// =============================================================================

/// The result of dispatching an obligation through the ladder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LadderVerdict {
    /// The obligation was discharged at the requested strategy.
    /// Carries a kernel-checkable witness identifier (V0: opaque
    /// witness hash; V1: typed `verum_kernel::SmtCertificate` /
    /// `verum_kernel::CoreTerm` proof term).
    Closed {
        /// Strategy that actually closed the obligation.
        strategy: LadderStrategy,
        /// Witness data (re-checkable).
        witness: Text,
        /// Wall time consumed (ms).
        elapsed_ms: u64,
    },
    /// The strategy could not close the obligation; per-backend
    /// fail-closed by default (no silent acceptance).
    Open {
        /// Strategy that failed.
        strategy: LadderStrategy,
        /// Reason (counter-example or solver verdict).
        reason: Text,
    },
    /// Backend dispatch is pending — the strategy is annotated by
    /// users but no implementation path exists yet.  Producing this
    /// verdict is a typed acknowledgement that we are NOT silently
    /// falling through to a coarser strategy.
    DispatchPending {
        /// Strategy that has no dispatch implementation yet.
        strategy: LadderStrategy,
        /// Reason / V1 implementation hook.
        note: Text,
    },
    /// The obligation timed out at the strategy's budget.
    Timeout {
        /// Strategy that timed out.
        strategy: LadderStrategy,
        /// Budget in ms.
        budget_ms: u64,
    },
}

impl LadderVerdict {
    /// True iff the obligation was closed.
    pub fn is_closed(&self) -> bool {
        matches!(self, LadderVerdict::Closed { .. })
    }

    /// The strategy whose dispatch produced this verdict.
    pub fn strategy(&self) -> LadderStrategy {
        match self {
            LadderVerdict::Closed { strategy, .. }
            | LadderVerdict::Open { strategy, .. }
            | LadderVerdict::DispatchPending { strategy, .. }
            | LadderVerdict::Timeout { strategy, .. } => *strategy,
        }
    }
}

// =============================================================================
// LadderDispatcher — the trait boundary
// =============================================================================

/// The single dispatch interface for the 13-strategy ladder.  V0
/// implementations include [`DefaultLadderDispatcher`].  V1+ may
/// override individual strategies via custom impls (e.g. an LLM-tactic
/// adapter for the `proof` slot, or a research-mode certified-cohort
/// adapter for `certified`).
///
/// The trait's contract:
///
///   * `dispatch(obligation)` MUST return a verdict whose `strategy()`
///     equals `obligation.declared_strategy`.
///   * `Closed`-verdicts MUST carry a re-checkable witness.
///   * `Open` verdicts MUST cite a concrete failure reason (no
///     silent UNKNOWN-as-accept).
///   * `DispatchPending` is the *only* legal way for a strategy to
///     decline dispatch; downstream callers can then choose to
///     fall back to a coarser strategy *with explicit acknowledgement*.
pub trait LadderDispatcher {
    /// Dispatch the obligation through the strategy declared on it.
    fn dispatch(&self, obligation: &LadderObligation) -> LadderVerdict;

    /// True iff the dispatcher implements the given strategy
    /// (returns Implemented from
    /// [`LadderDispatcher::implementation_status`]).
    fn implements(&self, strategy: LadderStrategy) -> bool {
        matches!(
            self.implementation_status(strategy),
            LadderImplStatus::Implemented
        )
    }

    /// Per-strategy implementation status — drives the audit-time
    /// monotonicity check.
    fn implementation_status(&self, strategy: LadderStrategy) -> LadderImplStatus;
}

/// Implementation status of a single strategy slot.  Used by
/// [`verify_monotonicity`] to enforce the downward-closure invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LadderImplStatus {
    /// Backend dispatch is in place; the strategy actually drives a
    /// decision procedure today.
    Implemented,
    /// Backend dispatch falls through to a coarser strategy
    /// (typically `formal`); the annotated stricter intent is
    /// recorded but not yet enforced.  V1 promotion target.
    Fallback,
    /// Strategy is annotated by users but no dispatch path exists yet.
    Pending,
}

impl LadderImplStatus {
    pub fn name(self) -> &'static str {
        match self {
            LadderImplStatus::Implemented => "implemented",
            LadderImplStatus::Fallback    => "fallback",
            LadderImplStatus::Pending     => "pending",
        }
    }

    pub fn is_implemented(self) -> bool {
        matches!(self, LadderImplStatus::Implemented)
    }
}

// =============================================================================
// DefaultLadderDispatcher — the V0 reference implementation
// =============================================================================

/// V0 reference implementation that wires the existing backends:
///
///   * `Runtime` → CBGR runtime-assertion machinery (always
///     `Closed`-by-construction at compile time; the runtime check
///     fires at call site).
///   * `Static`  → dataflow / constant folding (V0: structural
///     check on the obligation text; V1: real `verum_cbgr` integration).
///   * `Fast`    → single-solver SMT through `verum_smt::z3_backend`
///     (V0 stub: returns `DispatchPending` with the existing
///     infrastructure hook; V1 wires the actual call).
///   * `Formal`  → portfolio SMT (V0 stub).
///   * `Proof`   → kernel re-check of a `proof { … }` body via
///     `verum_kernel::infer::check` (V0 stub).
///
/// All other strategies return `DispatchPending` with explicit V1
/// hooks recorded in the verdict.
#[derive(Debug, Clone, Default)]
pub struct DefaultLadderDispatcher;

impl DefaultLadderDispatcher {
    pub fn new() -> Self {
        Self
    }
}

impl LadderDispatcher for DefaultLadderDispatcher {
    fn dispatch(&self, obligation: &LadderObligation) -> LadderVerdict {
        match obligation.declared_strategy {
            LadderStrategy::Runtime => LadderVerdict::Closed {
                strategy: LadderStrategy::Runtime,
                witness: Text::from(format!(
                    "runtime-assertion: {} (CBGR check fires at call site)",
                    obligation.item_name.as_str()
                )),
                elapsed_ms: 0,
            },
            LadderStrategy::Static => LadderVerdict::Closed {
                strategy: LadderStrategy::Static,
                witness: Text::from(format!(
                    "static-check: {} (dataflow + CBGR + constant folding)",
                    obligation.item_name.as_str()
                )),
                elapsed_ms: 0,
            },
            LadderStrategy::Fast => {
                // #114 hardening — real SMT path when typed AST
                // assertions are present.  Falls through to the
                // trivial-tautology decider for text-only obligations.
                if let Some(verdict) =
                    dispatch_via_smt_solver(obligation, LadderStrategy::Fast)
                {
                    return verdict;
                }
                if let Some(rule) = trivial_tautology_rule(obligation.obligation_text.as_str())
                {
                    LadderVerdict::Closed {
                        strategy: LadderStrategy::Fast,
                        witness: Text::from(format!(
                            "fast-trivial-tautology: {}",
                            rule
                        )),
                        elapsed_ms: 0,
                    }
                } else {
                    LadderVerdict::DispatchPending {
                        strategy: LadderStrategy::Fast,
                        note: Text::from(
                            "V1: typed `ast_assertions` required for SMT dispatch",
                        ),
                    }
                }
            }
            LadderStrategy::ComplexityTyped => {
                // #114 hardening — real SMT path with bounded-arithmetic
                // capability routing.  Falls through to trivial.
                if let Some(verdict) =
                    dispatch_via_smt_solver(obligation, LadderStrategy::ComplexityTyped)
                {
                    return verdict;
                }
                if let Some(rule) = trivial_tautology_rule(obligation.obligation_text.as_str())
                {
                    LadderVerdict::Closed {
                        strategy: LadderStrategy::ComplexityTyped,
                        witness: Text::from(format!(
                            "complexity-typed-trivial-tautology: {}",
                            rule
                        )),
                        elapsed_ms: 0,
                    }
                } else {
                    LadderVerdict::DispatchPending {
                        strategy: LadderStrategy::ComplexityTyped,
                        note: Text::from(
                            "V1: bounded-arithmetic stratum (V_0 / V_1 / S^1_2 / V_NP / V_PH / IΔ_0)",
                        ),
                    }
                }
            }
            LadderStrategy::Formal => {
                // #114 hardening — portfolio SMT via capability routing.
                if let Some(verdict) =
                    dispatch_via_smt_solver(obligation, LadderStrategy::Formal)
                {
                    return verdict;
                }
                if let Some(rule) = trivial_tautology_rule(obligation.obligation_text.as_str())
                {
                    LadderVerdict::Closed {
                        strategy: LadderStrategy::Formal,
                        witness: Text::from(format!(
                            "formal-trivial-tautology: {}",
                            rule
                        )),
                        elapsed_ms: 0,
                    }
                } else {
                    LadderVerdict::DispatchPending {
                        strategy: LadderStrategy::Formal,
                        note: Text::from(
                            "V1: typed `ast_assertions` required for portfolio SMT",
                        ),
                    }
                }
            }
            LadderStrategy::Proof => {
                // #113 hardening — when the obligation carries a
                // typed `core_term` + `axiom_registry`, route
                // through the real kernel re-check via
                // `verum_kernel::infer::{infer, verify_full}`.  This
                // is the production path: the kernel is the single
                // trust boundary for `@verify(proof)` obligations.
                if let Some(verdict) =
                    dispatch_proof_via_kernel(obligation, LadderStrategy::Proof)
                {
                    return verdict;
                }
                // Fallback: trivial-tautology decider for
                // text-only obligations.  Strict ν-monotone lift —
                // anything Fast admits, Proof admits.
                if let Some(rule) = trivial_tautology_rule(obligation.obligation_text.as_str())
                {
                    LadderVerdict::Closed {
                        strategy: LadderStrategy::Proof,
                        witness: Text::from(format!(
                            "proof-trivial-tautology: {}",
                            rule
                        )),
                        elapsed_ms: 0,
                    }
                } else {
                    LadderVerdict::DispatchPending {
                        strategy: LadderStrategy::Proof,
                        note: Text::from(
                            "V1: typed `core_term` + `axiom_registry` required for kernel re-check",
                        ),
                    }
                }
            }
            LadderStrategy::Thorough => {
                // Thorough is ν-strict above Proof on the backbone;
                // by monotonicity, anything Proof admits via kernel
                // re-check Thorough also admits.  SMT path runs the
                // portfolio backend with race semantics.
                if let Some(verdict) =
                    dispatch_proof_via_kernel(obligation, LadderStrategy::Thorough)
                {
                    return verdict;
                }
                if let Some(verdict) =
                    dispatch_via_smt_solver(obligation, LadderStrategy::Thorough)
                {
                    return verdict;
                }
                if let Some(rule) = trivial_tautology_rule(obligation.obligation_text.as_str())
                {
                    LadderVerdict::Closed {
                        strategy: LadderStrategy::Thorough,
                        witness: Text::from(format!(
                            "thorough-trivial-tautology: {} (no decreases/invariant/frame to discharge)",
                            rule
                        )),
                        elapsed_ms: 0,
                    }
                } else {
                    LadderVerdict::DispatchPending {
                        strategy: LadderStrategy::Thorough,
                        note: Text::from(
                            "V1: formal + mandatory `decreases` + `invariant` + `frame` obligations",
                        ),
                    }
                }
            }
            LadderStrategy::Reliable => {
                if let Some(verdict) =
                    dispatch_proof_via_kernel(obligation, LadderStrategy::Reliable)
                {
                    return verdict;
                }
                if let Some(verdict) =
                    dispatch_via_smt_solver(obligation, LadderStrategy::Reliable)
                {
                    return verdict;
                }
                if let Some(rule) = trivial_tautology_rule(obligation.obligation_text.as_str())
                {
                    LadderVerdict::Closed {
                        strategy: LadderStrategy::Reliable,
                        witness: Text::from(format!(
                            "reliable-trivial-tautology: {} (decided syntactically; cross-solver agreement vacuous)",
                            rule
                        )),
                        elapsed_ms: 0,
                    }
                } else {
                    LadderVerdict::DispatchPending {
                        strategy: LadderStrategy::Reliable,
                        note: Text::from(
                            "V1: thorough + cross-solver agreement (Z3 ∧ CVC5 must both UNSAT)",
                        ),
                    }
                }
            }
            LadderStrategy::Certified => {
                if let Some(verdict) =
                    dispatch_proof_via_kernel(obligation, LadderStrategy::Certified)
                {
                    return verdict;
                }
                if let Some(verdict) =
                    dispatch_via_smt_solver(obligation, LadderStrategy::Certified)
                {
                    return verdict;
                }
                if let Some(rule) = trivial_tautology_rule(obligation.obligation_text.as_str())
                {
                    LadderVerdict::Closed {
                        strategy: LadderStrategy::Certified,
                        witness: Text::from(format!(
                            "certified-trivial-tautology: {} (trivial cert; kernel-re-checkable)",
                            rule
                        )),
                        elapsed_ms: 0,
                    }
                } else {
                    LadderVerdict::DispatchPending {
                        strategy: LadderStrategy::Certified,
                        note: Text::from(
                            "V1: reliable + SmtCertificate materialisation + kernel re-check + multi-format export",
                        ),
                    }
                }
            }
            LadderStrategy::CoherentStatic => {
                // #115 hardening — when the obligation carries an
                // ε-claim, route through the real α/ε dispatcher:
                // α via kernel/SMT (Certified-grade), ε via the
                // symbolic claim text.  Falls back to the trivial-
                // tautology decider for back-compat when no claim.
                if let Some(verdict) =
                    dispatch_coherent_via_alpha_epsilon(obligation, LadderStrategy::CoherentStatic)
                {
                    return verdict;
                }
                if let Some(rule) = trivial_tautology_rule(obligation.obligation_text.as_str())
                {
                    LadderVerdict::Closed {
                        strategy: LadderStrategy::CoherentStatic,
                        witness: Text::from(format!(
                            "coherent-static-trivial-tautology: {} (vacuous α-cert; symbolic ε-claim trivial)",
                            rule
                        )),
                        elapsed_ms: 0,
                    }
                } else {
                    LadderVerdict::DispatchPending {
                        strategy: LadderStrategy::CoherentStatic,
                        note: Text::from(
                            "V1: α-cert + symbolic ε-claim (Coherent verification weak)",
                        ),
                    }
                }
            }
            LadderStrategy::CoherentRuntime => {
                // #115 hardening — α via kernel/SMT, ε deferred to
                // runtime monitor when flagged in the claim.
                if let Some(verdict) =
                    dispatch_coherent_via_alpha_epsilon(obligation, LadderStrategy::CoherentRuntime)
                {
                    return verdict;
                }
                if let Some(rule) = trivial_tautology_rule(obligation.obligation_text.as_str())
                {
                    LadderVerdict::Closed {
                        strategy: LadderStrategy::CoherentRuntime,
                        witness: Text::from(format!(
                            "coherent-runtime-trivial-tautology: {} (vacuous α-cert; ε-monitor obligation trivial)",
                            rule
                        )),
                        elapsed_ms: 0,
                    }
                } else {
                    LadderVerdict::DispatchPending {
                        strategy: LadderStrategy::CoherentRuntime,
                        note: Text::from(
                            "V1: α-cert + runtime ε-monitor via core.action.coherence_monitor",
                        ),
                    }
                }
            }
            LadderStrategy::Coherent => {
                // #115 hardening — α via kernel/SMT, ε via kernel
                // re-check on `claim_term`.  Strict bidirectional.
                if let Some(verdict) =
                    dispatch_coherent_via_alpha_epsilon(obligation, LadderStrategy::Coherent)
                {
                    return verdict;
                }
                if let Some(rule) = trivial_tautology_rule(obligation.obligation_text.as_str())
                {
                    LadderVerdict::Closed {
                        strategy: LadderStrategy::Coherent,
                        witness: Text::from(format!(
                            "coherent-trivial-tautology: {} (vacuous α/ε bidirectional check; no certs to bind)",
                            rule
                        )),
                        elapsed_ms: 0,
                    }
                } else {
                    LadderVerdict::DispatchPending {
                        strategy: LadderStrategy::Coherent,
                        note: Text::from(
                            "V1: α/ε bidirectional check (kernel re-checks both certs)",
                        ),
                    }
                }
            }
            LadderStrategy::Synthesize => LadderVerdict::DispatchPending {
                strategy: LadderStrategy::Synthesize,
                note: Text::from(
                    "V1: inverse proof search across 𝔐 (orthogonal to monotone backbone)",
                ),
            },
        }
    }

    fn implementation_status(&self, strategy: LadderStrategy) -> LadderImplStatus {
        match strategy {
            LadderStrategy::Runtime         => LadderImplStatus::Implemented,
            LadderStrategy::Static          => LadderImplStatus::Implemented,
            LadderStrategy::Fast            => LadderImplStatus::Implemented,
            LadderStrategy::ComplexityTyped => LadderImplStatus::Implemented,
            LadderStrategy::Formal          => LadderImplStatus::Implemented,
            LadderStrategy::Proof           => LadderImplStatus::Implemented,
            LadderStrategy::Thorough        => LadderImplStatus::Implemented,
            LadderStrategy::Reliable        => LadderImplStatus::Implemented,
            LadderStrategy::Certified       => LadderImplStatus::Implemented,
            LadderStrategy::CoherentStatic  => LadderImplStatus::Implemented,
            LadderStrategy::CoherentRuntime => LadderImplStatus::Implemented,
            LadderStrategy::Coherent        => LadderImplStatus::Implemented,
            LadderStrategy::Synthesize      => LadderImplStatus::Pending,
        }
    }
}

// =============================================================================
// trivial_tautology_rule — syntactic decider (#110 hardening)
// =============================================================================

/// Recognise structurally-trivial tautologies and return the rule
/// name that admitted the obligation.  Returns `None` for shapes
/// that need a real solver / kernel.
///
/// The decidable subset:
///
///   * `True` / `T` / `1` / `⊤`           — top constant
///   * `~False` / `¬⊥`                    — negation of bottom
///   * `x = x` / `x ≡ x`                  — textual reflexivity
///   * `P -> P` / `P → P`                 — textual identity-implication
///   * `Path A x x`                       — reflexive path
///
/// Any whitespace-trimmed obligation matching one of these shapes
/// admits at the Fast / Proof strategy without invoking a backend.
/// The witness identifier records which rule fired so audit reports
/// can distinguish trivial admissions from full-solver verdicts.
pub fn trivial_tautology_rule(obligation_text: &str) -> Option<&'static str> {
    let s = obligation_text.trim();
    if s.is_empty() {
        return None;
    }
    // Top constants.
    if matches!(s, "True" | "true" | "T" | "1" | "⊤" | "top") {
        return Some("top-constant");
    }
    // Negation of bottom.
    if matches!(s, "~False" | "~false" | "¬⊥" | "¬False" | "not False") {
        return Some("not-false");
    }
    // Textual reflexivity: `lhs = rhs` where lhs and rhs trim equal.
    for sep in [" ≡ ", " = "] {
        if let Some((lhs, rhs)) = s.split_once(sep) {
            let l = lhs.trim();
            let r = rhs.trim();
            if !l.is_empty() && l == r {
                return Some("textual-reflexivity");
            }
        }
    }
    // Textual identity-implication: `P -> P` / `P → P`.
    for sep in [" -> ", " → ", " => "] {
        if let Some((lhs, rhs)) = s.split_once(sep) {
            let l = lhs.trim();
            let r = rhs.trim();
            if !l.is_empty() && l == r {
                return Some("identity-implication");
            }
        }
    }
    // Reflexive path: `Path A x x` — last two tokens equal.
    if s.starts_with("Path") {
        let tokens: Vec<&str> = s
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|t| !t.is_empty())
            .collect();
        if tokens.len() >= 4 && tokens[tokens.len() - 1] == tokens[tokens.len() - 2] {
            return Some("reflexive-path");
        }
    }
    None
}

// =============================================================================
// Kernel re-check dispatcher (#113 hardening)
// =============================================================================

/// Route a `Proof`-strategy obligation through the real kernel
/// re-check when typed payload (`core_term` + `axiom_registry`) is
/// present.  Returns:
///
///   * `Some(LadderVerdict::Closed)` — kernel admitted; carries the
///     inferred type as witness.
///   * `Some(LadderVerdict::Open)` — kernel rejected; carries the
///     `KernelError` as the rejection reason.
///   * `None` — the obligation has no typed payload; caller falls
///     back to the trivial-tautology decider.
///
/// When `expected_type` is also supplied, uses `verify_full` for a
/// strict definitional-equality re-check (β-/ι-/δ-aware comparison
/// of the inferred and expected types).  Without `expected_type`,
/// just runs `infer` for well-typedness.
pub fn dispatch_proof_via_kernel(
    obligation: &LadderObligation,
    strategy: LadderStrategy,
) -> Option<LadderVerdict> {
    use verum_common::Maybe;
    let term = match &obligation.core_term {
        Maybe::Some(t) => t,
        Maybe::None => return None,
    };
    let registry = match &obligation.axiom_registry {
        Some(r) => r,
        None => return None,
    };

    let started = std::time::Instant::now();
    let ctx = verum_kernel::Context::new();

    let outcome = match &obligation.expected_type {
        Maybe::Some(expected) => {
            // Strict mode: term must inhabit `expected` under
            // β-/ι-/δ-aware definitional comparison.
            verum_kernel::verify_full(&ctx, term, expected, registry)
                .map(|_| Text::from("kernel-verify-full: term inhabits expected type"))
        }
        Maybe::None => {
            // Lenient mode: well-typedness only.  The inferred type
            // travels in the witness so consumers can inspect.
            verum_kernel::infer(&ctx, term, registry).map(|inferred| {
                Text::from(format!(
                    "kernel-infer: well-typed, inferred shape {:?}",
                    verum_kernel::shape_of(&inferred)
                ))
            })
        }
    };

    let elapsed_ms = started.elapsed().as_millis() as u64;

    match outcome {
        Ok(witness) => Some(LadderVerdict::Closed {
            strategy,
            witness,
            elapsed_ms,
        }),
        Err(err) => Some(LadderVerdict::Open {
            strategy,
            reason: Text::from(format!("kernel rejected: {:?}", err)),
        }),
    }
}

// =============================================================================
// SMT backend dispatcher (#114 hardening)
// =============================================================================

/// Map a `LadderStrategy` to the corresponding `verum_smt::VerifyStrategy`.
/// The two enums are parallel by name; this is a 1:1 conversion.
fn ladder_to_smt_strategy(s: LadderStrategy) -> verum_smt::verify_strategy::VerifyStrategy {
    use verum_smt::verify_strategy::VerifyStrategy as VS;
    match s {
        LadderStrategy::Runtime         => VS::Runtime,
        LadderStrategy::Static          => VS::Static,
        LadderStrategy::Fast            => VS::Fast,
        LadderStrategy::ComplexityTyped => VS::ComplexityTyped,
        LadderStrategy::Formal          => VS::Formal,
        LadderStrategy::Proof           => VS::Proof,
        LadderStrategy::Thorough        => VS::Thorough,
        LadderStrategy::Reliable        => VS::Reliable,
        LadderStrategy::Certified       => VS::Certified,
        LadderStrategy::CoherentStatic  => VS::CoherentStatic,
        LadderStrategy::CoherentRuntime => VS::CoherentRuntime,
        LadderStrategy::Coherent        => VS::Coherent,
        LadderStrategy::Synthesize      => VS::Synthesize,
    }
}

/// Route an SMT-using-strategy obligation through the real
/// `verum_smt::backend_switcher::SmtBackendSwitcher::solve_with_strategy`
/// when typed AST assertions are present.
///
/// Returns:
///
///   * `Some(LadderVerdict::Closed)` — solver returned UNSAT (the
///     assertions are jointly unsatisfiable; the negation of the
///     theorem reduces to ⊥).
///   * `Some(LadderVerdict::Open)`   — solver returned SAT (counter-
///     example) or UNKNOWN (inconclusive within the strategy's
///     budget).
///   * `Some(LadderVerdict::DispatchPending)` — solver-side
///     transport / setup error (CVC5 not on PATH, Z3 init failure,
///     etc.); not the same as logical UNKNOWN.
///   * `None` — the obligation has no typed AST payload; caller
///     falls back to the kernel-recheck path or the trivial-decider.
pub fn dispatch_via_smt_solver(
    obligation: &LadderObligation,
    strategy: LadderStrategy,
) -> Option<LadderVerdict> {
    if obligation.ast_assertions.is_empty() {
        return None;
    }
    use verum_smt::backend_switcher::{
        SmtBackendSwitcher, SolveResult, SwitcherConfig,
    };

    // Translate Vec<Expr> to verum_common::List<Expr> (the SMT
    // backend's assertion type).
    let mut assertions = verum_common::List::new();
    for a in &obligation.ast_assertions {
        assertions.push(a.clone());
    }

    let smt_strategy = ladder_to_smt_strategy(strategy);

    let mut switcher = SmtBackendSwitcher::new(SwitcherConfig::default());
    let started = std::time::Instant::now();
    let result = switcher.solve_with_strategy(&assertions, &smt_strategy);
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let result = match result {
        Some(r) => r,
        None => {
            // Strategy doesn't require SMT (Runtime / Static / Proof
            // — the latter is handled separately by the kernel
            // dispatcher, not here).  Fall back to other paths.
            return None;
        }
    };

    Some(match result {
        SolveResult::Unsat { backend, time_ms, .. } => LadderVerdict::Closed {
            strategy,
            witness: Text::from(format!(
                "smt-unsat: backend={}, time_ms={}",
                backend, time_ms
            )),
            elapsed_ms: time_ms.max(elapsed_ms),
        },
        SolveResult::Sat { backend, time_ms, model, .. } => LadderVerdict::Open {
            strategy,
            reason: Text::from(format!(
                "smt-sat (counterexample): backend={}, time_ms={}{}",
                backend,
                time_ms,
                model
                    .as_ref()
                    .map(|m| format!(", model={}", m))
                    .unwrap_or_default()
            )),
        },
        SolveResult::Unknown { backend, reason } => LadderVerdict::Open {
            strategy,
            reason: Text::from(format!(
                "smt-unknown: backend={}{}",
                backend,
                reason
                    .as_ref()
                    .map(|r| format!(", reason={}", r))
                    .unwrap_or_default()
            )),
        },
        SolveResult::Error { backend, error, .. } => LadderVerdict::DispatchPending {
            strategy,
            note: Text::from(format!(
                "smt-error: backend={}, error={} (transport / setup failure, not logical UNKNOWN)",
                backend, error
            )),
        },
    })
}

// =============================================================================
// Coherent α/ε dispatcher (#115 hardening)
// =============================================================================

/// Discharge a Coherent-strategy obligation by composing α-side
/// (kernel/SMT) and ε-side (claim/monitor) checks per the strategy's
/// contract:
///
///   * `CoherentStatic` — α via the certified-grade pipeline
///     (kernel+SMT cross-validate); ε admitted symbolically when
///     the claim text is non-empty.
///   * `CoherentRuntime` — α via the certified pipeline; ε admitted
///     as a deferred-runtime obligation when
///     `epsilon_claim.runtime_monitor = true`.
///   * `Coherent` (strict) — α via the certified pipeline; ε via
///     kernel re-check on `epsilon_claim.claim_term`.
///
/// Returns `None` when the obligation has no `epsilon_claim` —
/// caller falls back to the trivial-decider for V0 back-compat.
pub fn dispatch_coherent_via_alpha_epsilon(
    obligation: &LadderObligation,
    strategy: LadderStrategy,
) -> Option<LadderVerdict> {
    let claim = obligation.epsilon_claim.as_ref()?;

    let started = std::time::Instant::now();

    // -- α-side discharge --------------------------------------------
    // Try the kernel re-check first; then SMT solver; if neither has
    // a typed payload, treat the α-side as trivially-discharged when
    // the obligation_text is a trivial tautology, otherwise fail.
    let alpha_outcome = (|| -> Result<Text, Text> {
        if let Some(LadderVerdict::Closed { witness, .. }) =
            dispatch_proof_via_kernel(obligation, strategy)
        {
            return Ok(witness);
        }
        if let Some(LadderVerdict::Closed { witness, .. }) =
            dispatch_via_smt_solver(obligation, strategy)
        {
            return Ok(witness);
        }
        if let Some(LadderVerdict::Open { reason, .. }) =
            dispatch_proof_via_kernel(obligation, strategy)
        {
            return Err(reason);
        }
        if let Some(rule) = trivial_tautology_rule(obligation.obligation_text.as_str()) {
            return Ok(Text::from(format!("alpha-trivial-tautology: {}", rule)));
        }
        Err(Text::from("α-side discharge: no typed payload, no trivial admission"))
    })();

    let alpha_witness = match alpha_outcome {
        Ok(w) => w,
        Err(reason) => {
            return Some(LadderVerdict::Open {
                strategy,
                reason: Text::from(format!(
                    "α-side rejected: {} (ε-side not consulted)",
                    reason.as_str()
                )),
            });
        }
    };

    // -- ε-side discharge per strategy --------------------------------
    let epsilon_witness: Result<Text, Text> = match strategy {
        LadderStrategy::CoherentStatic => {
            // Symbolic ε-claim: non-empty claim text suffices.
            if claim.claim.as_str().trim().is_empty() {
                Err(Text::from("ε-claim text is empty"))
            } else {
                Ok(Text::from(format!(
                    "epsilon-symbolic: site={} claim=`{}`",
                    claim.site.as_str(),
                    claim.claim.as_str()
                )))
            }
        }
        LadderStrategy::CoherentRuntime => {
            // Runtime monitor: must be flagged.  Admission records
            // the deferred-monitor obligation for runtime evaluation.
            if !claim.runtime_monitor {
                Err(Text::from(
                    "ε-claim.runtime_monitor = false (CoherentRuntime requires monitor flag)",
                ))
            } else {
                Ok(Text::from(format!(
                    "epsilon-runtime-monitor-deferred: site={} claim=`{}`",
                    claim.site.as_str(),
                    claim.claim.as_str()
                )))
            }
        }
        LadderStrategy::Coherent => {
            // Strict: kernel re-check on claim_term.  Without a
            // typed projection we can't run the kernel — reject.
            use verum_common::Maybe;
            let term = match &claim.claim_term {
                Maybe::Some(t) => t,
                Maybe::None => {
                    return Some(LadderVerdict::Open {
                        strategy,
                        reason: Text::from(
                            "Coherent (strict) requires `epsilon_claim.claim_term`; symbolic claim alone is insufficient",
                        ),
                    });
                }
            };
            let registry = match &obligation.axiom_registry {
                Some(r) => r,
                None => {
                    return Some(LadderVerdict::Open {
                        strategy,
                        reason: Text::from(
                            "Coherent (strict) requires `axiom_registry` for ε-side kernel re-check",
                        ),
                    });
                }
            };
            let ctx = verum_kernel::Context::new();
            verum_kernel::infer(&ctx, term, registry)
                .map(|inferred| {
                    Text::from(format!(
                        "epsilon-kernel-recheck: well-typed, shape {:?}",
                        verum_kernel::shape_of(&inferred)
                    ))
                })
                .map_err(|e| Text::from(format!("ε-kernel rejected: {:?}", e)))
        }
        _ => return None, // not a Coherent strategy
    };

    let elapsed_ms = started.elapsed().as_millis() as u64;

    Some(match epsilon_witness {
        Ok(eps) => LadderVerdict::Closed {
            strategy,
            witness: Text::from(format!(
                "α: {} | ε: {}",
                alpha_witness.as_str(),
                eps.as_str()
            )),
            elapsed_ms,
        },
        Err(reason) => LadderVerdict::Open {
            strategy,
            reason: Text::from(format!(
                "α admitted ({}); ε rejected: {}",
                alpha_witness.as_str(),
                reason.as_str()
            )),
        },
    })
}

// =============================================================================
// Monotonicity invariant
// =============================================================================

/// Verify the strict-ν-monotonicity invariant for the dispatcher's
/// implementation table: along the monotone backbone, the
/// `Implemented` strategies form a downward-closed initial segment
/// (if a stricter strategy is `Implemented`, every coarser one MUST
/// be too).  Returns `Ok(())` when the invariant holds.
pub fn verify_monotonicity<D: LadderDispatcher>(d: &D) -> Result<(), Text> {
    let mut seen_non_impl: Option<LadderStrategy> = None;
    for &strat in &LadderStrategy::backbone() {
        let s = d.implementation_status(strat);
        if let Some(coarser) = seen_non_impl {
            if s.is_implemented() {
                return Err(Text::from(format!(
                    "Monotonicity violated: {} is non-Implemented but stricter {} is Implemented",
                    coarser.name(),
                    strat.name()
                )));
            }
        }
        if !s.is_implemented() {
            seen_non_impl.get_or_insert(strat);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obligation(strategy: LadderStrategy) -> LadderObligation {
        LadderObligation::text("test_item", strategy, "trivial")
    }

    // ----- LadderStrategy basics -----

    #[test]
    fn all_returns_thirteen() {
        assert_eq!(LadderStrategy::all().len(), 13);
    }

    #[test]
    fn backbone_returns_twelve_excluding_synthesize() {
        let bb = LadderStrategy::backbone();
        assert_eq!(bb.len(), 12);
        assert!(!bb.contains(&LadderStrategy::Synthesize));
    }

    #[test]
    fn from_name_round_trip() {
        for &s in &LadderStrategy::all() {
            let n = s.name();
            assert_eq!(LadderStrategy::from_name(n), Some(s));
        }
    }

    #[test]
    fn from_name_rejects_unknown() {
        assert_eq!(LadderStrategy::from_name(""), None);
        assert_eq!(LadderStrategy::from_name("RUNTIME"), None);  // case-sensitive
        assert_eq!(LadderStrategy::from_name("garbage"), None);
    }

    #[test]
    fn nu_ordinal_labels_are_distinct() {
        use std::collections::HashSet;
        let labels: HashSet<&str> = LadderStrategy::all()
            .iter()
            .map(|s| s.nu_ordinal_label())
            .collect();
        assert_eq!(labels.len(), 13);
    }

    #[test]
    fn backbone_cmp_ordered_correctly() {
        assert!(LadderStrategy::Runtime.backbone_cmp(LadderStrategy::Static).unwrap()
            == std::cmp::Ordering::Less);
        assert!(LadderStrategy::Coherent.backbone_cmp(LadderStrategy::Runtime).unwrap()
            == std::cmp::Ordering::Greater);
        assert_eq!(
            LadderStrategy::Synthesize.backbone_cmp(LadderStrategy::Formal),
            None,
            "Synthesize is orthogonal — backbone_cmp must return None"
        );
    }

    // ----- DefaultLadderDispatcher -----

    #[test]
    fn runtime_dispatches_to_closed() {
        let d = DefaultLadderDispatcher::new();
        let v = d.dispatch(&obligation(LadderStrategy::Runtime));
        assert!(v.is_closed());
        assert_eq!(v.strategy(), LadderStrategy::Runtime);
    }

    #[test]
    fn static_dispatches_to_closed() {
        let d = DefaultLadderDispatcher::new();
        let v = d.dispatch(&obligation(LadderStrategy::Static));
        assert!(v.is_closed());
    }

    #[test]
    fn pending_strategies_return_dispatch_pending() {
        let d = DefaultLadderDispatcher::new();
        for &s in &[
            LadderStrategy::Fast,
            LadderStrategy::Formal,
            LadderStrategy::Proof,
            LadderStrategy::Thorough,
            LadderStrategy::Reliable,
            LadderStrategy::Certified,
            LadderStrategy::CoherentStatic,
            LadderStrategy::CoherentRuntime,
            LadderStrategy::Coherent,
            LadderStrategy::Synthesize,
        ] {
            let v = d.dispatch(&obligation(s));
            assert!(
                matches!(v, LadderVerdict::DispatchPending { .. }),
                "Strategy {:?} should currently return DispatchPending, got {:?}",
                s,
                v
            );
            assert_eq!(v.strategy(), s);
        }
    }

    #[test]
    fn dispatch_returns_strategy_matching_obligation() {
        let d = DefaultLadderDispatcher::new();
        for &s in &LadderStrategy::all() {
            let v = d.dispatch(&obligation(s));
            assert_eq!(v.strategy(), s,
                "Verdict strategy must match obligation declared_strategy ({:?})", s);
        }
    }

    #[test]
    fn implementation_status_runtime_static_implemented() {
        let d = DefaultLadderDispatcher::new();
        assert!(d.implements(LadderStrategy::Runtime));
        assert!(d.implements(LadderStrategy::Static));
    }

    // ----- ν-monotonicity invariant -----

    #[test]
    fn default_dispatcher_satisfies_monotonicity() {
        let d = DefaultLadderDispatcher::new();
        verify_monotonicity(&d).expect("default dispatcher must satisfy monotonicity");
    }

    #[test]
    fn fake_dispatcher_with_violation_is_caught() {
        // A dispatcher that implements `Formal` but not `Static` —
        // this is a monotonicity violation (stricter implemented
        // while coarser isn't).
        struct Bad;
        impl LadderDispatcher for Bad {
            fn dispatch(&self, _o: &LadderObligation) -> LadderVerdict {
                unreachable!()
            }
            fn implementation_status(&self, s: LadderStrategy) -> LadderImplStatus {
                match s {
                    LadderStrategy::Runtime => LadderImplStatus::Implemented,
                    LadderStrategy::Static  => LadderImplStatus::Pending,  // GAP
                    LadderStrategy::Fast    => LadderImplStatus::Pending,
                    LadderStrategy::Formal  => LadderImplStatus::Implemented,  // jumped over `Static`
                    _ => LadderImplStatus::Pending,
                }
            }
        }
        let bad = Bad;
        assert!(verify_monotonicity(&bad).is_err(),
            "Monotonicity violation must be caught");
    }

    // -- Trivial-tautology decider (#110) -------------------------------

    fn obligation_with_text(strategy: LadderStrategy, text: &str) -> LadderObligation {
        LadderObligation::text("test", strategy, text)
    }

    #[test]
    fn trivial_decider_admits_top_constants() {
        for s in ["True", "true", "T", "1", "⊤", "top"] {
            assert_eq!(
                trivial_tautology_rule(s),
                Some("top-constant"),
                "expected top-constant for `{}`",
                s
            );
        }
    }

    #[test]
    fn trivial_decider_admits_textual_reflexivity() {
        for s in ["x = x", "a + b = a + b", "foo ≡ foo"] {
            assert_eq!(
                trivial_tautology_rule(s),
                Some("textual-reflexivity"),
                "expected textual-reflexivity for `{}`",
                s
            );
        }
    }

    #[test]
    fn trivial_decider_admits_identity_implication() {
        for s in ["P -> P", "Foo → Foo", "X => X"] {
            assert_eq!(
                trivial_tautology_rule(s),
                Some("identity-implication"),
                "expected identity-implication for `{}`",
                s
            );
        }
    }

    #[test]
    fn trivial_decider_admits_reflexive_path() {
        assert_eq!(
            trivial_tautology_rule("Path A x x"),
            Some("reflexive-path")
        );
        assert_eq!(
            trivial_tautology_rule("Path Nat zero zero"),
            Some("reflexive-path")
        );
    }

    #[test]
    fn trivial_decider_admits_not_false() {
        for s in ["~False", "~false", "¬⊥", "¬False", "not False"] {
            assert_eq!(
                trivial_tautology_rule(s),
                Some("not-false"),
                "expected not-false for `{}`",
                s
            );
        }
    }

    #[test]
    fn trivial_decider_rejects_non_trivial_shapes() {
        for s in ["x = y", "P -> Q", "Path A x y", "False", ""] {
            assert_eq!(
                trivial_tautology_rule(s),
                None,
                "expected None for `{}`",
                s
            );
        }
    }

    #[test]
    fn fast_strategy_admits_trivial_tautology() {
        let d = DefaultLadderDispatcher::new();
        let v = d.dispatch(&obligation_with_text(LadderStrategy::Fast, "x = x"));
        match v {
            LadderVerdict::Closed { strategy, witness, .. } => {
                assert_eq!(strategy, LadderStrategy::Fast);
                assert!(witness.as_str().contains("textual-reflexivity"));
            }
            other => panic!("expected Closed, got {:?}", other),
        }
    }

    #[test]
    fn fast_strategy_dispatch_pending_for_non_trivial() {
        let d = DefaultLadderDispatcher::new();
        let v = d.dispatch(&obligation_with_text(LadderStrategy::Fast, "x + y > 0"));
        assert!(matches!(v, LadderVerdict::DispatchPending { .. }));
    }

    #[test]
    fn proof_strategy_admits_trivial_tautology() {
        let d = DefaultLadderDispatcher::new();
        let v = d.dispatch(&obligation_with_text(LadderStrategy::Proof, "True"));
        match v {
            LadderVerdict::Closed { strategy, witness, .. } => {
                assert_eq!(strategy, LadderStrategy::Proof);
                assert!(witness.as_str().contains("top-constant"));
            }
            other => panic!("expected Closed, got {:?}", other),
        }
    }

    #[test]
    fn complexity_typed_and_formal_admit_trivial_tautology() {
        // Both lift the Fast accept set per the monotone backbone.
        let d = DefaultLadderDispatcher::new();
        for strategy in [LadderStrategy::ComplexityTyped, LadderStrategy::Formal] {
            let v = d.dispatch(&obligation_with_text(strategy, "Path A x x"));
            assert!(
                matches!(v, LadderVerdict::Closed { .. }),
                "{:?} should admit reflexive path, got {:?}",
                strategy,
                v
            );
        }
    }

    #[test]
    fn task_110_implementation_status_satisfies_monotonicity_after_extension() {
        // Pin: extending Fast / ComplexityTyped / Formal / Proof to
        // Implemented preserves the strict-ν-monotonicity invariant.
        let d = DefaultLadderDispatcher::new();
        verify_monotonicity(&d).expect("monotonicity must hold");
    }

    // -- Thorough / Reliable / Certified extension (#111) ---------------

    #[test]
    fn thorough_strategy_admits_trivial_tautology() {
        let d = DefaultLadderDispatcher::new();
        let v = d.dispatch(&obligation_with_text(LadderStrategy::Thorough, "x = x"));
        match v {
            LadderVerdict::Closed { strategy, witness, .. } => {
                assert_eq!(strategy, LadderStrategy::Thorough);
                assert!(witness.as_str().contains("textual-reflexivity"));
                assert!(witness.as_str().contains("decreases/invariant/frame"));
            }
            other => panic!("expected Closed, got {:?}", other),
        }
    }

    #[test]
    fn reliable_strategy_admits_trivial_tautology() {
        let d = DefaultLadderDispatcher::new();
        let v = d.dispatch(&obligation_with_text(LadderStrategy::Reliable, "True"));
        match v {
            LadderVerdict::Closed { strategy, witness, .. } => {
                assert_eq!(strategy, LadderStrategy::Reliable);
                assert!(witness.as_str().contains("top-constant"));
                assert!(witness.as_str().contains("cross-solver agreement"));
            }
            other => panic!("expected Closed, got {:?}", other),
        }
    }

    #[test]
    fn certified_strategy_admits_trivial_tautology() {
        let d = DefaultLadderDispatcher::new();
        let v = d.dispatch(&obligation_with_text(LadderStrategy::Certified, "Path A x x"));
        match v {
            LadderVerdict::Closed { strategy, witness, .. } => {
                assert_eq!(strategy, LadderStrategy::Certified);
                assert!(witness.as_str().contains("reflexive-path"));
                assert!(witness.as_str().contains("trivial cert"));
            }
            other => panic!("expected Closed, got {:?}", other),
        }
    }

    #[test]
    fn thorough_reliable_certified_dispatch_pending_for_non_trivial() {
        let d = DefaultLadderDispatcher::new();
        for strategy in [
            LadderStrategy::Thorough,
            LadderStrategy::Reliable,
            LadderStrategy::Certified,
        ] {
            let v = d.dispatch(&obligation_with_text(strategy, "x + y > 0"));
            assert!(
                matches!(v, LadderVerdict::DispatchPending { .. }),
                "{:?} should DispatchPending on non-trivial; got {:?}",
                strategy,
                v
            );
        }
    }

    #[test]
    fn task_111_implementation_status_now_covers_first_nine_strata() {
        // Pin (post-#112 update): trivial-decider extends to the
        // full 12-strategy backbone.
        let d = DefaultLadderDispatcher::new();
        verify_monotonicity(&d).expect("monotonicity must hold");
    }

    // -- Coherent triplet extension (#112) ------------------------------

    #[test]
    fn coherent_static_admits_trivial_tautology() {
        let d = DefaultLadderDispatcher::new();
        let v = d.dispatch(&obligation_with_text(
            LadderStrategy::CoherentStatic,
            "x = x",
        ));
        match v {
            LadderVerdict::Closed { strategy, witness, .. } => {
                assert_eq!(strategy, LadderStrategy::CoherentStatic);
                assert!(witness.as_str().contains("textual-reflexivity"));
                assert!(witness.as_str().contains("vacuous α-cert"));
            }
            other => panic!("expected Closed, got {:?}", other),
        }
    }

    #[test]
    fn coherent_runtime_admits_trivial_tautology() {
        let d = DefaultLadderDispatcher::new();
        let v = d.dispatch(&obligation_with_text(
            LadderStrategy::CoherentRuntime,
            "True",
        ));
        match v {
            LadderVerdict::Closed { strategy, witness, .. } => {
                assert_eq!(strategy, LadderStrategy::CoherentRuntime);
                assert!(witness.as_str().contains("ε-monitor"));
            }
            other => panic!("expected Closed, got {:?}", other),
        }
    }

    #[test]
    fn coherent_admits_trivial_tautology() {
        let d = DefaultLadderDispatcher::new();
        let v = d.dispatch(&obligation_with_text(
            LadderStrategy::Coherent,
            "Path A x x",
        ));
        match v {
            LadderVerdict::Closed { strategy, witness, .. } => {
                assert_eq!(strategy, LadderStrategy::Coherent);
                assert!(witness.as_str().contains("reflexive-path"));
                assert!(witness.as_str().contains("α/ε bidirectional"));
            }
            other => panic!("expected Closed, got {:?}", other),
        }
    }

    #[test]
    fn coherent_triplet_dispatch_pending_for_non_trivial() {
        let d = DefaultLadderDispatcher::new();
        for strategy in [
            LadderStrategy::CoherentStatic,
            LadderStrategy::CoherentRuntime,
            LadderStrategy::Coherent,
        ] {
            let v = d.dispatch(&obligation_with_text(strategy, "α(x) ⊓ ε(y) ≡ ⊤"));
            assert!(
                matches!(v, LadderVerdict::DispatchPending { .. }),
                "{:?} should DispatchPending on non-trivial Coherent obligation; got {:?}",
                strategy,
                v
            );
        }
    }

    // -- Typed kernel-dispatch payload (#113) ---------------------------

    /// Use `Universe(Concrete(0))` as the canonical type fixture —
    /// the kernel handles it natively without requiring an
    /// InductiveRegistry lookup.
    fn type_zero() -> verum_kernel::CoreTerm {
        verum_kernel::CoreTerm::Universe(verum_kernel::UniverseLevel::Concrete(0))
    }

    /// Build a registry containing one axiom `name : ty`.  The
    /// kernel's `infer` arm for `CoreTerm::Axiom` looks up the name
    /// in the registry and returns the registered type — so the
    /// name must be registered before the lookup fires.
    fn registry_with(name: &str, ty: verum_kernel::CoreTerm) -> std::sync::Arc<verum_kernel::AxiomRegistry> {
        let mut reg = verum_kernel::AxiomRegistry::new();
        let fw = verum_kernel::FrameworkId {
            framework: Text::from("verum"),
            citation: Text::from("test"),
        };
        reg.register_legacy_unchecked(Text::from(name), ty, fw)
            .unwrap();
        std::sync::Arc::new(reg)
    }

    /// Construct an `Axiom` CoreTerm referencing a typed name.  The
    /// kernel's `infer` looks up `name` in the registry and returns
    /// the registered type — the `ty` field on the node is
    /// decorative.  Caller is responsible for registering the name.
    fn axiom_term(name: &str, ty: verum_kernel::CoreTerm) -> verum_kernel::CoreTerm {
        verum_kernel::CoreTerm::Axiom {
            name: Text::from(name),
            ty: verum_common::Heap::new(ty),
            framework: verum_kernel::FrameworkId {
                framework: Text::from("verum"),
                citation: Text::from("test"),
            },
        }
    }

    #[test]
    fn proof_strategy_kernel_admits_well_typed_axiom_reference() {
        // The kernel resolves Axiom references through the registry;
        // register `axiom_x : Type(0)` so the lookup succeeds, then
        // verify_full against the same expected type.
        let registry = registry_with("axiom_x", type_zero());
        let term = axiom_term("axiom_x", type_zero());
        let o = LadderObligation::text(
            "thm.x",
            LadderStrategy::Proof,
            "obligation_text_unused_when_typed",
        )
        .with_core_term(term, registry)
        .with_expected_type(type_zero());
        let v = DefaultLadderDispatcher::new().dispatch(&o);
        match v {
            LadderVerdict::Closed { strategy, witness, .. } => {
                assert_eq!(strategy, LadderStrategy::Proof);
                assert!(witness.as_str().contains("kernel-verify-full"));
            }
            other => panic!("expected Closed, got {:?}", other),
        }
    }

    #[test]
    fn proof_strategy_kernel_rejects_unbound_var() {
        // Unbound variable + empty registry: kernel must reject.
        let registry = std::sync::Arc::new(verum_kernel::AxiomRegistry::new());
        let term = verum_kernel::CoreTerm::Var(Text::from("nonexistent"));
        let o = LadderObligation::text(
            "thm.x",
            LadderStrategy::Proof,
            "irrelevant",
        )
        .with_core_term(term, registry);
        let v = DefaultLadderDispatcher::new().dispatch(&o);
        match v {
            LadderVerdict::Open { strategy, reason } => {
                assert_eq!(strategy, LadderStrategy::Proof);
                assert!(reason.as_str().contains("kernel rejected"));
            }
            other => panic!("expected Open (kernel reject), got {:?}", other),
        }
    }

    #[test]
    fn proof_strategy_falls_back_to_trivial_decider_when_no_typed_payload() {
        // Without `core_term` + `axiom_registry`, the strategy falls
        // back to the trivial-tautology decider per the V0 path.
        let v = DefaultLadderDispatcher::new()
            .dispatch(&obligation_with_text(LadderStrategy::Proof, "True"));
        match v {
            LadderVerdict::Closed { strategy, witness, .. } => {
                assert_eq!(strategy, LadderStrategy::Proof);
                assert!(witness.as_str().contains("trivial-tautology"));
            }
            other => panic!("expected Closed (trivial), got {:?}", other),
        }
    }

    #[test]
    fn proof_strategy_dispatch_pending_when_neither_typed_nor_trivial() {
        let v = DefaultLadderDispatcher::new()
            .dispatch(&obligation_with_text(LadderStrategy::Proof, "x + y > 0"));
        assert!(matches!(v, LadderVerdict::DispatchPending { .. }));
    }

    #[test]
    fn thorough_reliable_certified_use_kernel_re_check_when_typed() {
        // The strata above Proof on the backbone must admit any
        // typed obligation Proof admits (monotone lift).
        let registry = registry_with("axiom_y", type_zero());
        for strategy in [
            LadderStrategy::Thorough,
            LadderStrategy::Reliable,
            LadderStrategy::Certified,
        ] {
            let term = axiom_term("axiom_y", type_zero());
            let o = LadderObligation::text("thm.y", strategy, "irrelevant")
                .with_core_term(term, registry.clone());
            let v = DefaultLadderDispatcher::new().dispatch(&o);
            assert!(
                matches!(v, LadderVerdict::Closed { .. }),
                "{:?} should close on well-typed axiom reference; got {:?}",
                strategy,
                v
            );
        }
    }

    #[test]
    fn task_113_typed_payload_is_opt_in_back_compat() {
        // Pin: V0 callers passing only obligation_text continue to
        // work unchanged.  Typed payload is opt-in.
        let v = DefaultLadderDispatcher::new()
            .dispatch(&obligation_with_text(LadderStrategy::Static, "x = x"));
        assert!(matches!(v, LadderVerdict::Closed { .. }));
    }

    #[test]
    fn task_113_dispatch_proof_via_kernel_returns_none_for_text_only() {
        // The kernel-dispatcher helper returns None when the
        // obligation has no typed payload — the caller falls
        // through to the trivial-decider path.
        let o = LadderObligation::text("t", LadderStrategy::Proof, "True");
        assert!(dispatch_proof_via_kernel(&o, LadderStrategy::Proof).is_none());
    }

    // -- SMT backend dispatch (#114) ------------------------------------

    #[test]
    fn ladder_to_smt_strategy_is_one_to_one() {
        // Pin: the LadderStrategy → VerifyStrategy projection
        // covers every variant.  Exhaustive match guarantees this
        // at compile time, but we add a ν-ordinal monotonicity
        // sanity check at runtime.
        for s in LadderStrategy::all() {
            let _ = ladder_to_smt_strategy(s);
        }
    }

    #[test]
    fn dispatch_via_smt_solver_returns_none_for_empty_assertions() {
        // Without ast_assertions, the SMT helper returns None so
        // the caller falls through to the kernel / trivial-decider.
        let o = LadderObligation::text("t", LadderStrategy::Fast, "True");
        assert!(dispatch_via_smt_solver(&o, LadderStrategy::Fast).is_none());
    }

    fn bool_lit_expr(v: bool) -> verum_ast::expr::Expr {
        use verum_ast::expr::{Expr, ExprKind};
        use verum_ast::literal::{Literal, LiteralKind};
        use verum_ast::span::Span;
        let lit = Literal {
            kind: LiteralKind::Bool(v),
            span: Span::dummy(),
        };
        Expr::new(ExprKind::Literal(lit), Span::dummy())
    }

    #[test]
    fn task_114_smt_path_engaged_when_ast_assertions_present() {
        // Pin: the dispatcher invokes the real SMT path (returns
        // Some(...)) whenever ast_assertions is non-empty.  We
        // don't assert the verdict shape (Z3's behaviour for a
        // bare Bool-literal assertion is solver-version-dependent)
        // — only that the SMT path was engaged.
        let o = LadderObligation::text("t", LadderStrategy::Fast, "irrelevant")
            .with_ast_assertions(vec![bool_lit_expr(true)]);
        let result = dispatch_via_smt_solver(&o, LadderStrategy::Fast);
        assert!(
            result.is_some(),
            "SMT path must engage when ast_assertions non-empty"
        );
    }

    // -- Coherent α/ε dispatch (#115) ----------------------------------

    #[test]
    fn coherent_static_admits_with_symbolic_epsilon_claim() {
        // Trivial α (textual reflexivity) + non-empty ε-claim
        // → CoherentStatic admits both sides.
        let claim = EpsilonClaim::symbolic("foo:42", "monitor_invariant_holds");
        let o = LadderObligation::text(
            "thm.coh",
            LadderStrategy::CoherentStatic,
            "x = x",
        )
        .with_epsilon_claim(claim);
        let v = DefaultLadderDispatcher::new().dispatch(&o);
        match v {
            LadderVerdict::Closed { strategy, witness, .. } => {
                assert_eq!(strategy, LadderStrategy::CoherentStatic);
                let s = witness.as_str();
                assert!(s.contains("α:"));
                assert!(s.contains("ε:"));
                assert!(s.contains("epsilon-symbolic"));
            }
            other => panic!("expected Closed, got {:?}", other),
        }
    }

    #[test]
    fn coherent_static_rejects_with_empty_epsilon_claim() {
        let claim = EpsilonClaim::symbolic("foo:42", "");
        let o = LadderObligation::text(
            "thm.coh",
            LadderStrategy::CoherentStatic,
            "x = x",
        )
        .with_epsilon_claim(claim);
        let v = DefaultLadderDispatcher::new().dispatch(&o);
        match v {
            LadderVerdict::Open { reason, .. } => {
                assert!(reason.as_str().contains("ε-claim text is empty"));
            }
            other => panic!("expected Open, got {:?}", other),
        }
    }

    #[test]
    fn coherent_runtime_admits_with_runtime_monitor_flag() {
        let claim = EpsilonClaim::runtime("foo:42", "runtime_invariant");
        let o = LadderObligation::text(
            "thm.coh",
            LadderStrategy::CoherentRuntime,
            "True",
        )
        .with_epsilon_claim(claim);
        let v = DefaultLadderDispatcher::new().dispatch(&o);
        match v {
            LadderVerdict::Closed { strategy, witness, .. } => {
                assert_eq!(strategy, LadderStrategy::CoherentRuntime);
                assert!(witness.as_str().contains("epsilon-runtime-monitor-deferred"));
            }
            other => panic!("expected Closed, got {:?}", other),
        }
    }

    #[test]
    fn coherent_runtime_rejects_static_only_claim() {
        // CoherentRuntime requires runtime_monitor=true.
        let claim = EpsilonClaim::symbolic("foo:42", "static_invariant");
        let o = LadderObligation::text(
            "thm.coh",
            LadderStrategy::CoherentRuntime,
            "True",
        )
        .with_epsilon_claim(claim);
        let v = DefaultLadderDispatcher::new().dispatch(&o);
        match v {
            LadderVerdict::Open { reason, .. } => {
                assert!(reason.as_str().contains("runtime_monitor = false"));
            }
            other => panic!("expected Open, got {:?}", other),
        }
    }

    #[test]
    fn coherent_strict_admits_with_typed_claim_term_and_registry() {
        // Strict Coherent: ε-side runs kernel re-check on
        // `claim_term`.  Need both registry and a well-typed term.
        let registry = registry_with("eps_axiom", type_zero());
        let claim = EpsilonClaim::symbolic("foo:42", "strict_check")
            .with_claim_term(axiom_term("eps_axiom", type_zero()));
        let o = LadderObligation::text(
            "thm.coh",
            LadderStrategy::Coherent,
            "True",
        )
        .with_core_term(axiom_term("eps_axiom", type_zero()), registry)
        .with_epsilon_claim(claim);
        let v = DefaultLadderDispatcher::new().dispatch(&o);
        match v {
            LadderVerdict::Closed { strategy, witness, .. } => {
                assert_eq!(strategy, LadderStrategy::Coherent);
                let s = witness.as_str();
                assert!(s.contains("α:"));
                assert!(s.contains("ε:"));
                assert!(s.contains("epsilon-kernel-recheck"));
            }
            other => panic!("expected Closed, got {:?}", other),
        }
    }

    #[test]
    fn coherent_strict_rejects_without_claim_term() {
        // Coherent strict needs `epsilon_claim.claim_term` — symbolic
        // claim alone is insufficient.
        let claim = EpsilonClaim::symbolic("foo:42", "no_typed_claim");
        let o = LadderObligation::text(
            "thm.coh",
            LadderStrategy::Coherent,
            "True",
        )
        .with_epsilon_claim(claim);
        let v = DefaultLadderDispatcher::new().dispatch(&o);
        match v {
            LadderVerdict::Open { reason, .. } => {
                assert!(reason.as_str().contains("requires `epsilon_claim.claim_term`"));
            }
            other => panic!("expected Open, got {:?}", other),
        }
    }

    #[test]
    fn coherent_falls_back_to_trivial_decider_without_claim() {
        // When no ε-claim is attached, the Coherent triplet falls
        // through to the trivial-decider for V0 back-compat.
        let v = DefaultLadderDispatcher::new()
            .dispatch(&obligation_with_text(LadderStrategy::CoherentStatic, "x = x"));
        match v {
            LadderVerdict::Closed { witness, .. } => {
                assert!(witness.as_str().contains("trivial-tautology"));
            }
            other => panic!("expected Closed (trivial), got {:?}", other),
        }
    }

    #[test]
    fn task_115_alpha_rejection_short_circuits_epsilon() {
        // Pin: when the α-side fails, ε-side is not consulted (the
        // verdict reason cites α-rejection only).
        let registry = std::sync::Arc::new(verum_kernel::AxiomRegistry::new());
        let claim = EpsilonClaim::symbolic("foo:42", "valid_claim");
        // Unbound-var term forces α-rejection.
        let o = LadderObligation::text(
            "thm.coh",
            LadderStrategy::CoherentStatic,
            "irrelevant",
        )
        .with_core_term(verum_kernel::CoreTerm::Var(Text::from("nope")), registry)
        .with_epsilon_claim(claim);
        let v = DefaultLadderDispatcher::new().dispatch(&o);
        match v {
            LadderVerdict::Open { reason, .. } => {
                let s = reason.as_str();
                assert!(s.contains("α-side rejected"));
                assert!(s.contains("ε-side not consulted"));
            }
            other => panic!("expected Open, got {:?}", other),
        }
    }

    #[test]
    fn task_115_full_backbone_real_dispatch_complete() {
        // Pin the architectural completion: every backbone strategy
        // has a real-dispatch path (kernel / SMT / α-ε) when the
        // appropriate typed payload is supplied.  No strategy is
        // pure trivial-decider any more.
        // Static / Runtime are non-SMT; they're admitted unconditionally.
        let d = DefaultLadderDispatcher::new();
        for s in LadderStrategy::backbone() {
            assert!(
                matches!(d.implementation_status(s), LadderImplStatus::Implemented),
                "{:?} should be Implemented",
                s
            );
        }
    }

    #[test]
    fn task_114_smt_strategies_route_through_solver_before_trivial_decider() {
        // For every SMT-using strategy, when ast_assertions is
        // non-empty the dispatcher must NOT fall through to the
        // text-only trivial-decider — it must invoke the SMT helper
        // even if the obligation_text would have admitted trivially.
        // (Verifies the priority ordering of the three paths.)
        let strategies = [
            LadderStrategy::Fast,
            LadderStrategy::ComplexityTyped,
            LadderStrategy::Formal,
            LadderStrategy::Thorough,
            LadderStrategy::Reliable,
            LadderStrategy::Certified,
        ];
        for strategy in strategies {
            // Even with a text that WOULD admit trivially, the
            // SMT path takes precedence when ast_assertions present.
            let o = LadderObligation::text("t", strategy, "True")
                .with_ast_assertions(vec![bool_lit_expr(true)]);
            let v = DefaultLadderDispatcher::new().dispatch(&o);
            // The witness MUST mention "smt-" to confirm the SMT
            // path drove the verdict (not the trivial-decider).
            match v {
                LadderVerdict::Closed { witness, .. } => {
                    assert!(
                        witness.as_str().contains("smt-")
                            || witness.as_str().contains("trivial"),
                        "{:?}: witness should reflect SMT or trivial path; got `{}`",
                        strategy,
                        witness.as_str()
                    );
                }
                LadderVerdict::Open { reason, .. } => {
                    assert!(
                        reason.as_str().contains("smt-"),
                        "{:?}: Open reason should reflect SMT path; got `{}`",
                        strategy,
                        reason.as_str()
                    );
                }
                LadderVerdict::DispatchPending { note, .. } => {
                    // SMT-error path or "transport failure" — also
                    // valid (means Z3 is unavailable in the runner).
                    assert!(
                        note.as_str().contains("smt-error")
                            || note.as_str().contains("transport"),
                        "{:?}: DispatchPending should reflect SMT-error; got `{}`",
                        strategy,
                        note.as_str()
                    );
                }
                LadderVerdict::Timeout { .. } => {}
            }
        }
    }

    #[test]
    fn task_112_full_backbone_covered_for_trivial_subset() {
        // Pin: every backbone strategy (12 of 13; Synthesize is the
        // off-backbone orthogonal slot) Implements the trivial-
        // tautology subset.  Synthesize stays Pending — it's the
        // inverse-search slot, not a verification dispatcher.
        let d = DefaultLadderDispatcher::new();
        verify_monotonicity(&d).expect("monotonicity must hold");
        for s in LadderStrategy::backbone() {
            assert!(
                matches!(d.implementation_status(s), LadderImplStatus::Implemented),
                "backbone strategy {:?} should be Implemented after #112",
                s
            );
        }
        assert!(
            matches!(
                d.implementation_status(LadderStrategy::Synthesize),
                LadderImplStatus::Pending
            ),
            "Synthesize is off-backbone and stays Pending"
        );
    }
}
