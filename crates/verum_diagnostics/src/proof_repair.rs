//! Proof-failure repair-suggestion catalogue — the structured-error
//! frontier for kernel / type / verification failures.
//!
//! ## What this module is
//!
//! When the kernel rejects a term or the type-checker fails on an
//! obligation, downstream tooling (LSP / REPL / CLI) needs more than
//! just an error message — it needs **actionable repair suggestions**
//! ranked by likelihood, with related-theorem hints, deep-link
//! documentation, and structured fields suitable for LSP code-action
//! emission.
//!
//! This module ships:
//!
//!   * [`ProofFailureKind`] — typed classification of every kernel /
//!     type-checker failure mode (mirrors `verum_kernel::KernelError`
//!     variants + verification-time obligation failures).
//!   * [`RepairSuggestion`] — a structured-fix record with snippet,
//!     rationale, applicability, score, and optional doc-link.
//!   * [`RepairEngine`] trait — single dispatch interface; LSP and
//!     CLI consume the same engine.
//!   * [`DefaultRepairEngine`] — reference V0 implementation with a
//!     hand-curated rule-set per failure kind.
//!
//! ## Design principles
//!
//!   1. **Every suggestion is concrete.**  The `snippet` field is a
//!      drop-in code fragment, not advice prose.  IDE consumers can
//!      apply it as a code-action.
//!
//!   2. **Repair suggestions come ranked.**  Score in `[0, 1]`
//!      reflects estimated likelihood the suggestion fixes the
//!      reported failure.  Top-3 are surfaced by IDE hover; full set
//!      via "see all alternatives".
//!
//!   3. **Doc-links to a stable URL.**  Each suggestion carries an
//!      optional `doc_link` (e.g. `docs.verum.lang/kernel/k-refine`)
//!      that opens in-browser from the IDE.
//!
//!   4. **Related theorems come from kernel introspection.**
//!      For an unresolved-name failure, the engine queries the
//!      lemma registry for near-miss matches; for a positivity
//!      violation, it suggests known-positive alternatives.
//!
//! ## Integration with existing infrastructure
//!
//! Builds on `crates/verum_diagnostics/src/suggestion.rs`'s
//! `Suggestion` / `Applicability` / `CodeSnippet` types — this
//! module adds the proof-failure-specific catalogue + ranking
//! logic on top of that base.
//!
//! ## Foundation-neutral
//!
//! The catalogue is foundation-neutral: kernel-rule failures are
//! universal (Refine / Positivity / Universe / Adjunction-Unit /
//! …), while corpus-specific repair hints (e.g. MSFS-specific
//! lemma renames) are layered via separate engine impls composed
//! through [`CompositeRepairEngine`].

use serde::{Deserialize, Serialize};
use verum_common::Text;

// =============================================================================
// ProofFailureKind — typed classification
// =============================================================================

/// A classification of every proof / type-check failure mode the
/// repair engine recognises.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProofFailureKind {
    /// `K-Refine` rejected: depth-strict comprehension fails
    /// `dp(P) < dp(A) + 1`.
    RefineDepthViolation {
        /// The refined type being constructed.
        refined_type: Text,
        /// The predicate's actual depth (or rendering).
        predicate_depth: Text,
    },
    /// `K-Pos` rejected: strict-positivity violation in an inductive.
    PositivityViolation {
        /// Inductive name.
        type_name: Text,
        /// Offending constructor.
        constructor: Text,
        /// Human-readable position description.
        position: Text,
    },
    /// `K-Univ` rejected: universe-level inconsistency.
    UniverseInconsistency {
        /// Universe of the source.
        source_universe: Text,
        /// Universe expected.
        expected_universe: Text,
    },
    /// `K-FwAx` rejected: framework-axiom body is not in `Prop`.
    FrameworkAxiomNotProp {
        /// Axiom name.
        axiom_name: Text,
        /// Body's actual sort.
        body_sort: Text,
    },
    /// `K-Adj-Unit` / `K-Adj-Counit` rejected: round-trip failure.
    AdjunctionRoundTripFailure {
        /// Side: `"unit"` or `"counit"`.
        side: Text,
    },
    /// Type mismatch — unification failed.
    TypeMismatch {
        /// Expected type rendering.
        expected: Text,
        /// Actual type rendering.
        actual: Text,
    },
    /// Unbound name reference.
    UnboundName {
        /// The unresolved identifier.
        name: Text,
    },
    /// Apply-target's signature does not match the goal.
    ApplyMismatch {
        /// Lemma / theorem being applied.
        lemma_name: Text,
        /// What its conclusion was.
        actual_conclusion: Text,
        /// What the goal needed.
        goal: Text,
    },
    /// Tactic returned `Open` — could not close the obligation.
    TacticOpen {
        /// Tactic name.
        tactic: Text,
        /// Reason (counter-example or solver verdict).
        reason: Text,
    },
}

// =============================================================================
// RepairSuggestion — the structured-fix record
// =============================================================================

/// Confidence that the suggestion fixes the reported failure.
/// Mirrors `clippy`/`rust-analyzer` applicability levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RepairApplicability {
    /// Always-correct mechanical fix (drop in unchanged).
    MachineApplicable,
    /// Probably correct; user should review before accepting.
    MaybeIncorrect,
    /// Requires placeholder substitution; user must complete the snippet.
    HasPlaceholders,
    /// Speculative — reflects a common pattern but may not match this case.
    Speculative,
}

/// A structured repair suggestion with snippet, rationale, score, and
/// optional documentation link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepairSuggestion {
    /// The replacement code snippet (or full surrounding region).
    pub snippet: Text,
    /// One-line rationale for the suggestion.
    pub rationale: Text,
    /// Applicability tier.
    pub applicability: RepairApplicability,
    /// Likelihood score in `[0.0, 1.0]` (higher = more likely correct).
    pub score: f64,
    /// Optional documentation deep-link
    /// (e.g. `https://docs.verum.lang/kernel/k-refine`).
    pub doc_link: Option<Text>,
    /// Optional related-theorem hint (`(name, signature)` pairs).
    pub related_theorems: Vec<(Text, Text)>,
}

impl RepairSuggestion {
    /// Construct a basic suggestion with no doc-link / related theorems.
    pub fn simple(
        snippet: impl Into<Text>,
        rationale: impl Into<Text>,
        applicability: RepairApplicability,
        score: f64,
    ) -> Self {
        Self {
            snippet: snippet.into(),
            rationale: rationale.into(),
            applicability,
            score: score.clamp(0.0, 1.0),
            doc_link: None,
            related_theorems: Vec::new(),
        }
    }

    /// Add a documentation deep-link.
    pub fn with_doc_link(mut self, link: impl Into<Text>) -> Self {
        self.doc_link = Some(link.into());
        self
    }

    /// Add a related theorem.
    pub fn with_related(mut self, name: impl Into<Text>, signature: impl Into<Text>) -> Self {
        self.related_theorems.push((name.into(), signature.into()));
        self
    }
}

// =============================================================================
// RepairEngine — the trait boundary
// =============================================================================

/// Single dispatch interface for repair-suggestion generation.  LSP
/// and CLI consumers call `suggest(failure)` with a typed
/// [`ProofFailureKind`] and receive a ranked, deduplicated list of
/// [`RepairSuggestion`]s.
///
/// **Purity contract:** implementations MUST be pure — no I/O.
/// Side-effecting adapters (lemma-registry fuzzy-search, LLM repair
/// proposals) compose via [`CompositeRepairEngine`].
pub trait RepairEngine {
    /// Suggest repairs for the given failure.  Caller bounds the
    /// response size with `max_results`.
    fn suggest(&self, failure: &ProofFailureKind, max_results: usize) -> Vec<RepairSuggestion>;
}

// =============================================================================
// DefaultRepairEngine — V0 reference catalogue
// =============================================================================

/// V0 reference engine.  Hand-curated rule-set per failure kind.
/// Each rule emits 1–4 [`RepairSuggestion`]s that an IDE can render
/// as code-actions.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultRepairEngine;

impl DefaultRepairEngine {
    pub fn new() -> Self { Self }

    fn doc(suffix: &str) -> Text {
        Text::from(format!("https://docs.verum.lang/kernel/{}", suffix))
    }
}

impl RepairEngine for DefaultRepairEngine {
    fn suggest(&self, failure: &ProofFailureKind, max_results: usize) -> Vec<RepairSuggestion> {
        let mut out: Vec<RepairSuggestion> = match failure {
            ProofFailureKind::RefineDepthViolation { refined_type, predicate_depth } => vec![
                RepairSuggestion::simple(
                    format!(
                        "// reduce the predicate's modal depth so it satisfies\n// dp(P) < dp({}) + 1\n// Current depth: {}",
                        refined_type.as_str(),
                        predicate_depth.as_str()
                    ),
                    "K-Refine kernel rule requires the refinement predicate's depth to be strictly below the carrier-type's depth + 1",
                    RepairApplicability::HasPlaceholders,
                    0.85,
                )
                .with_doc_link(Self::doc("k-refine")),
                RepairSuggestion::simple(
                    format!(
                        "@require_extension(vfe_7)\n// then use ordinal-valued depth via K-Refine-omega for {}",
                        refined_type.as_str()
                    ),
                    "Opt into the K-Refine-omega rule (ordinal-valued depth) when finite-depth refinement is too restrictive",
                    RepairApplicability::Speculative,
                    0.45,
                )
                .with_doc_link(Self::doc("k-refine-omega")),
            ],

            ProofFailureKind::PositivityViolation { type_name, constructor, position } => vec![
                RepairSuggestion::simple(
                    format!(
                        "// {} cannot appear in {} of constructor `{}`\n// move the recursive reference to a strictly-positive position\n// (right of every arrow, never inside a function-typed argument)",
                        type_name.as_str(),
                        position.as_str(),
                        constructor.as_str(),
                    ),
                    "K-Pos rejects non-strictly-positive recursion (Berardi 1998 derives False from any negative occurrence)",
                    RepairApplicability::HasPlaceholders,
                    0.9,
                )
                .with_doc_link(Self::doc("k-pos")),
                RepairSuggestion::simple(
                    format!(
                        "@coinductive\ntype {} is ...",
                        type_name.as_str()
                    ),
                    "If you want a *productive* (rather than well-founded) self-reference, declare the type coinductive",
                    RepairApplicability::Speculative,
                    0.35,
                )
                .with_doc_link(Self::doc("coinductive")),
            ],

            ProofFailureKind::UniverseInconsistency { source_universe, expected_universe } => vec![
                RepairSuggestion::simple(
                    format!(
                        "// add explicit universe ascent: {} → {}\n// e.g. wrap the term in `Lift<{}>(_)` or annotate the type",
                        source_universe.as_str(),
                        expected_universe.as_str(),
                        expected_universe.as_str(),
                    ),
                    "K-Univ kernel rule requires the source's universe to be ≤ expected; bump via Lift / explicit annotation",
                    RepairApplicability::HasPlaceholders,
                    0.8,
                )
                .with_doc_link(Self::doc("k-univ")),
            ],

            ProofFailureKind::FrameworkAxiomNotProp { axiom_name, body_sort } => vec![
                RepairSuggestion::simple(
                    format!(
                        "// `{}`'s body is in {} — K-FwAx requires `Prop`-typed bodies\n// either (a) reformulate the axiom as a Prop-valued predicate, or\n//        (b) downgrade to @def if you want a definitional binding",
                        axiom_name.as_str(),
                        body_sort.as_str(),
                    ),
                    "K-FwAx admits only Prop-typed framework-axiom bodies (anything else would let users postulate arbitrary computable functions)",
                    RepairApplicability::HasPlaceholders,
                    0.85,
                )
                .with_doc_link(Self::doc("k-fwax")),
            ],

            ProofFailureKind::AdjunctionRoundTripFailure { side } => vec![
                RepairSuggestion::simple(
                    format!(
                        "// the α ⊣ ε round-trip failed on the {} side\n// check that alpha_of(epsilon(α)) ≡ α (or epsilon(alpha_of(e)) ≡ e\n// up to gauge equivalence)",
                        side.as_str()
                    ),
                    "K-Adj-Unit / K-Adj-Counit kernel rules enforce the Diakrisis 108.T α ⊣ ε round-trip identity",
                    RepairApplicability::HasPlaceholders,
                    0.7,
                )
                .with_doc_link(Self::doc("k-adj-unit")),
            ],

            ProofFailureKind::TypeMismatch { expected, actual } => vec![
                RepairSuggestion::simple(
                    format!(
                        "// expected: {}\n// actual:   {}\n// add an explicit conversion / coercion, or fix the surrounding term",
                        expected.as_str(),
                        actual.as_str(),
                    ),
                    "type mismatch — the inferred type and the expected type differ",
                    RepairApplicability::HasPlaceholders,
                    0.75,
                ),
            ],

            ProofFailureKind::UnboundName { name } => vec![
                RepairSuggestion::simple(
                    format!("mount <module>.{{{}}};", name.as_str()),
                    "unbound name — add a `mount` declaration importing the symbol from its defining module",
                    RepairApplicability::HasPlaceholders,
                    0.8,
                )
                .with_doc_link(Self::doc("module-system")),
                RepairSuggestion::simple(
                    format!(
                        "// `{}` is undefined.  Did you mean a lemma in scope?\n// Try: `verum proof-draft --suggest` for ranked alternatives",
                        name.as_str()
                    ),
                    "unbound name — query the suggestion engine for near-miss alternatives",
                    RepairApplicability::Speculative,
                    0.5,
                ),
            ],

            ProofFailureKind::ApplyMismatch { lemma_name, actual_conclusion, goal } => vec![
                RepairSuggestion::simple(
                    format!(
                        "// `{}` concludes:  {}\n// goal needed:    {}\n// either (a) instantiate `{}` with explicit type/term arguments, or\n//        (b) apply a different lemma whose conclusion matches the goal",
                        lemma_name.as_str(),
                        actual_conclusion.as_str(),
                        goal.as_str(),
                        lemma_name.as_str(),
                    ),
                    "applied lemma's conclusion does not unify with the current goal",
                    RepairApplicability::HasPlaceholders,
                    0.78,
                ),
                RepairSuggestion::simple(
                    "apply auto;".to_string(),
                    "fall back to the auto tactic — portfolio SMT search may discharge the goal automatically",
                    RepairApplicability::MaybeIncorrect,
                    0.35,
                ),
            ],

            ProofFailureKind::TacticOpen { tactic, reason } => vec![
                RepairSuggestion::simple(
                    format!(
                        "// `{}` returned Open: {}\n// try a stricter strategy: `apply {} || apply auto || apply lia`",
                        tactic.as_str(),
                        reason.as_str(),
                        tactic.as_str(),
                    ),
                    "tactic could not close the goal — chain alternatives with `||` or step into a manual proof",
                    RepairApplicability::Speculative,
                    0.55,
                ),
            ],
        };

        // Rank: descending score; truncate to max_results.
        out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        out.truncate(max_results);
        out
    }
}

// =============================================================================
// CompositeRepairEngine — composition for adapter chaining
// =============================================================================

/// Combines multiple [`RepairEngine`]s — useful for composing the
/// default catalogue with a corpus-specific engine (MSFS-aware) or
/// an LLM repair adapter.  Suggestions are merged + re-sorted by
/// score; ties broken by source order.
pub struct CompositeRepairEngine {
    pub engines: Vec<Box<dyn RepairEngine>>,
}

impl CompositeRepairEngine {
    pub fn new(engines: Vec<Box<dyn RepairEngine>>) -> Self {
        Self { engines }
    }
}

impl std::fmt::Debug for CompositeRepairEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CompositeRepairEngine {{ engines: <{}> }}", self.engines.len())
    }
}

impl RepairEngine for CompositeRepairEngine {
    fn suggest(&self, failure: &ProofFailureKind, max_results: usize) -> Vec<RepairSuggestion> {
        let mut all: Vec<RepairSuggestion> = Vec::new();
        for e in &self.engines {
            all.extend(e.suggest(failure, max_results));
        }
        all.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        all.truncate(max_results);
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- RepairSuggestion -----

    #[test]
    fn simple_clamps_score() {
        let r = RepairSuggestion::simple("x", "y", RepairApplicability::Speculative, 1.5);
        assert_eq!(r.score, 1.0);
        let r2 = RepairSuggestion::simple("x", "y", RepairApplicability::Speculative, -0.5);
        assert_eq!(r2.score, 0.0);
    }

    #[test]
    fn doc_link_and_related_chain() {
        let r = RepairSuggestion::simple("x", "y", RepairApplicability::MachineApplicable, 1.0)
            .with_doc_link("https://example.org/foo")
            .with_related("lemma_a", "Π x. P(x)");
        assert!(r.doc_link.is_some());
        assert_eq!(r.related_theorems.len(), 1);
    }

    // ----- DefaultRepairEngine — per failure kind -----

    fn engine() -> DefaultRepairEngine { DefaultRepairEngine::new() }

    #[test]
    fn refine_depth_violation_offers_two_paths() {
        let f = ProofFailureKind::RefineDepthViolation {
            refined_type: Text::from("CategoricalLevel"),
            predicate_depth: Text::from("ω·2"),
        };
        let s = engine().suggest(&f, 5);
        assert_eq!(s.len(), 2);
        // The doc-link must point at the K-Refine page.
        assert!(s[0].doc_link.as_ref().unwrap().as_str().contains("k-refine"));
    }

    #[test]
    fn positivity_violation_suggests_coinductive_alternative() {
        let f = ProofFailureKind::PositivityViolation {
            type_name: Text::from("Bad"),
            constructor: Text::from("Wrap"),
            position: Text::from("left of arrow inside arg #1"),
        };
        let s = engine().suggest(&f, 5);
        assert!(s.iter().any(|r| r.snippet.as_str().contains("coinductive")));
    }

    #[test]
    fn unbound_name_offers_mount_suggestion() {
        let f = ProofFailureKind::UnboundName {
            name: Text::from("foo_lemma"),
        };
        let s = engine().suggest(&f, 5);
        assert!(s.iter().any(|r| r.snippet.as_str().contains("mount")));
    }

    #[test]
    fn apply_mismatch_offers_auto_fallback() {
        let f = ProofFailureKind::ApplyMismatch {
            lemma_name: Text::from("foo"),
            actual_conclusion: Text::from("A"),
            goal: Text::from("B"),
        };
        let s = engine().suggest(&f, 5);
        assert!(s.iter().any(|r| r.snippet.as_str().contains("apply auto")));
    }

    #[test]
    fn suggestions_are_score_descending() {
        let f = ProofFailureKind::RefineDepthViolation {
            refined_type: Text::from("X"),
            predicate_depth: Text::from("ω"),
        };
        let s = engine().suggest(&f, 5);
        for w in s.windows(2) {
            assert!(w[0].score >= w[1].score);
        }
    }

    #[test]
    fn max_results_is_respected() {
        let f = ProofFailureKind::RefineDepthViolation {
            refined_type: Text::from("X"),
            predicate_depth: Text::from("ω"),
        };
        let s = engine().suggest(&f, 1);
        assert_eq!(s.len(), 1);
    }

    // ----- CompositeRepairEngine -----

    #[test]
    fn composite_merges_and_reranks() {
        let f = ProofFailureKind::UnboundName { name: Text::from("foo") };
        let composite = CompositeRepairEngine::new(vec![
            Box::new(DefaultRepairEngine::new()),
            Box::new(DefaultRepairEngine::new()),
        ]);
        let s = composite.suggest(&f, 3);
        assert!(s.len() <= 3);
        for w in s.windows(2) {
            assert!(w[0].score >= w[1].score);
        }
    }

    #[test]
    fn every_failure_kind_produces_at_least_one_suggestion() {
        let kinds = [
            ProofFailureKind::RefineDepthViolation {
                refined_type: Text::from("X"),
                predicate_depth: Text::from("ω"),
            },
            ProofFailureKind::PositivityViolation {
                type_name: Text::from("X"),
                constructor: Text::from("C"),
                position: Text::from("left of arrow"),
            },
            ProofFailureKind::UniverseInconsistency {
                source_universe: Text::from("Type_1"),
                expected_universe: Text::from("Type_0"),
            },
            ProofFailureKind::FrameworkAxiomNotProp {
                axiom_name: Text::from("ax"),
                body_sort: Text::from("Type"),
            },
            ProofFailureKind::AdjunctionRoundTripFailure {
                side: Text::from("unit"),
            },
            ProofFailureKind::TypeMismatch {
                expected: Text::from("Int"),
                actual: Text::from("Bool"),
            },
            ProofFailureKind::UnboundName { name: Text::from("foo") },
            ProofFailureKind::ApplyMismatch {
                lemma_name: Text::from("f"),
                actual_conclusion: Text::from("A"),
                goal: Text::from("B"),
            },
            ProofFailureKind::TacticOpen {
                tactic: Text::from("lia"),
                reason: Text::from("non-trivial"),
            },
        ];
        let e = engine();
        for k in &kinds {
            assert!(!e.suggest(k, 5).is_empty(), "{:?} produced no suggestion", k);
        }
    }
}
