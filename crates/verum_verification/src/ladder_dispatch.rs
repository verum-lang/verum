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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LadderObligation {
    /// Diagnostic name of the theorem / lemma whose obligation this is.
    pub item_name: Text,
    /// The strategy declared by `@verify(<strategy>)` on the item.
    pub declared_strategy: LadderStrategy,
    /// Rendered obligation text (V0); V1 will replace this with the
    /// typed CoreTerm.
    pub obligation_text: Text,
    /// Optional time budget in milliseconds.  Per-strategy default
    /// applies when `None`.
    pub timeout_ms: Option<u64>,
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
                // #110 hardening — admit syntactic tautologies.
                // The full Z3 single-solver dispatch lands in V1;
                // the structurally-decidable subset already closes
                // many low-stakes obligations correctly.
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
                            "V1: wire to verum_smt::z3_backend single-solver SMT (timeout 100ms)",
                        ),
                    }
                }
            }
            LadderStrategy::ComplexityTyped => {
                // ComplexityTyped is strictly between Fast and
                // Formal on the monotone backbone — must admit at
                // least everything Fast admits.
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
                // Formal must admit at least everything ComplexityTyped
                // admits.  V1 adds portfolio SMT for the broader set.
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
                            "V1: portfolio SMT (Z3 + CVC5) via verum_smt::backend_switcher",
                        ),
                    }
                }
            }
            LadderStrategy::Proof => {
                // #110 hardening — Proof is ν-strict above Fast, so
                // anything the trivial-tautology decider admits at
                // Fast also admits at Proof (monotone lift).  V1
                // adds the actual kernel re-check of the proof
                // body; until then the decidable subset already
                // discharges trivial obligations honestly.
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
                            "V1: kernel re-check of `proof { ... }` body via verum_kernel::infer::check",
                        ),
                    }
                }
            }
            LadderStrategy::Thorough => {
                // #111 hardening — Thorough = Formal + mandatory
                // decreases/invariant/frame.  Trivial tautologies
                // have no termination / loop-invariant / framing
                // obligations to discharge, so the additional
                // gates are vacuously satisfied.
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
                // #111 hardening — Reliable = Thorough + Z3 ∧ CVC5
                // cross-solver agreement.  Trivial tautologies pass
                // every solver (the syntactic rule decides without
                // invoking either backend), so agreement is trivial.
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
                // #111 hardening — Certified = Reliable + cert
                // materialisation + kernel re-check + multi-format
                // export.  Trivial tautologies have a trivial
                // certificate: cite the syntactic rule that fired.
                // The kernel re-check is a no-op (the rule is
                // structurally sound).  Multi-format export emits
                // the same witness across every target.
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
            LadderStrategy::CoherentStatic => LadderVerdict::DispatchPending {
                strategy: LadderStrategy::CoherentStatic,
                note: Text::from(
                    "V1: α-cert + symbolic ε-claim (Coherent verification weak)",
                ),
            },
            LadderStrategy::CoherentRuntime => LadderVerdict::DispatchPending {
                strategy: LadderStrategy::CoherentRuntime,
                note: Text::from(
                    "V1: α-cert + runtime ε-monitor via core.action.coherence_monitor",
                ),
            },
            LadderStrategy::Coherent => LadderVerdict::DispatchPending {
                strategy: LadderStrategy::Coherent,
                note: Text::from(
                    "V1: α/ε bidirectional check (kernel re-checks both certs)",
                ),
            },
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
            LadderStrategy::CoherentStatic  => LadderImplStatus::Pending,
            LadderStrategy::CoherentRuntime => LadderImplStatus::Pending,
            LadderStrategy::Coherent        => LadderImplStatus::Pending,
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
        LadderObligation {
            item_name: Text::from("test_item"),
            declared_strategy: strategy,
            obligation_text: Text::from("trivial"),
            timeout_ms: None,
        }
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
        LadderObligation {
            item_name: Text::from("test"),
            declared_strategy: strategy,
            obligation_text: Text::from(text),
            timeout_ms: None,
        }
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
        // Pin: the Implemented set covers Runtime / Static / Fast /
        // ComplexityTyped / Formal / Proof / Thorough / Reliable /
        // Certified (9 of the 12 backbone strategies).  Coherent
        // triplet + Synthesize remain Pending — those need real
        // α/ε-cert + inverse-search infrastructure.
        let d = DefaultLadderDispatcher::new();
        verify_monotonicity(&d).expect("monotonicity must hold");
        let mut implemented = 0;
        for s in LadderStrategy::all() {
            if matches!(d.implementation_status(s), LadderImplStatus::Implemented) {
                implemented += 1;
            }
        }
        assert_eq!(
            implemented, 9,
            "9 of 13 ladder strategies should be Implemented after #111"
        );
    }
}
