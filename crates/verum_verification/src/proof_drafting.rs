//! Proof-drafting infrastructure — zero-friction goal display +
//! tactic suggestion + obligation auto-completion.
//!
//! ## What this module is
//!
//! The user-facing surface for interactive proof development.  Builds
//! on the existing `tactic_evaluation::{Goal, ProofState}` types and
//! adds a clean trait boundary that LSP / REPL / CLI consumers all
//! drive through:
//!
//!   * [`ProofStateView`] — read-only snapshot of the live proof
//!     state at a given cursor / step.
//!   * [`TacticSuggestion`] — ranked candidate next-step.
//!   * [`SuggestionEngine`] trait — single dispatch for ranked
//!     tactic-suggestion queries.
//!   * [`DefaultSuggestionEngine`] — V0 reference that wires
//!     fuzzy-match against a lemma name registry + tactic-registry
//!     lookup.
//!
//! ## Why a single trait boundary
//!
//! Pre-this-module, IDE/LSP consumed proof state via ad-hoc parsing
//! of compiler diagnostics; REPL had its own goal-printer; CLI had
//! none.  A single typed surface eliminates the per-consumer
//! re-parsing tax and gives LLM-tactic adapters (VERUM-EXPR-2) a
//! clean integration point.
//!
//! ## Invariants
//!
//!   1. Every [`TacticSuggestion`] carries enough metadata for the
//!      kernel to verify the suggested step independently — no
//!      "suggestion that can't be checked".
//!   2. `SuggestionEngine` impls MUST be pure (no I/O); side-effecting
//!      adapters (LLM, network registry) wrap them via composition.
//!   3. `ProofStateView` snapshots are immutable; advancing the proof
//!      produces a new snapshot.
//!
//! ## Foundation-neutral
//!
//! The trait is foundation-neutral: it accepts any goal-shape that
//! reduces to `verum_kernel::CoreTerm` (via `tactic_evaluation::Goal`
//! → CoreTerm projection).  Cubical / classical / paraconsistent
//! corpora share the same suggestion surface.

use serde::{Deserialize, Serialize};
use verum_common::Text;

// =============================================================================
// ProofStateView — immutable read-only snapshot
// =============================================================================

/// A single proof obligation as visible to the developer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProofGoalSummary {
    /// Stable identifier (matches `tactic_evaluation::Goal::id`).
    pub goal_id: usize,
    /// Rendered proposition text (one-line; full term available via
    /// the underlying `Goal::proposition` for kernel-level callers).
    pub proposition: Text,
    /// Hypotheses in scope (name + rendered type).
    pub hypotheses: Vec<HypothesisSummary>,
    /// Whether this goal is *focused* (next to be discharged).
    pub is_focused: bool,
}

/// A single hypothesis in scope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypothesisSummary {
    /// Local name of the hypothesis (`h`, `IH`, etc.).
    pub name: Text,
    /// Rendered type.
    pub ty: Text,
}

/// Read-only snapshot of the proof state at a given step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProofStateView {
    /// Theorem name being proved.
    pub theorem_name: Text,
    /// Open goals in dispatch order (focused goal first).
    pub goals: Vec<ProofGoalSummary>,
    /// Lemmas / theorems / framework axioms reachable from the
    /// proof body (the autocomplete pool).
    pub available_lemmas: Vec<LemmaSummary>,
    /// Tactic invocation history (most-recent last).
    pub history: Vec<Text>,
}

/// A lemma signature visible to the autocomplete suggestion engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LemmaSummary {
    /// Fully-qualified name.
    pub name: Text,
    /// Rendered type signature.
    pub signature: Text,
    /// Lineage tag (`msfs`, `lurie_htt`, `core`, etc.).
    pub lineage: Text,
}

impl ProofStateView {
    /// Construct an empty view (no goals, no lemmas).
    pub fn empty(theorem_name: impl Into<Text>) -> Self {
        Self {
            theorem_name: theorem_name.into(),
            goals: Vec::new(),
            available_lemmas: Vec::new(),
            history: Vec::new(),
        }
    }

    /// True iff every goal is closed (proof discharged).
    pub fn is_complete(&self) -> bool {
        self.goals.is_empty()
    }

    /// The currently-focused goal, if any.
    pub fn focused_goal(&self) -> Option<&ProofGoalSummary> {
        self.goals.iter().find(|g| g.is_focused).or(self.goals.first())
    }
}

// =============================================================================
// TacticSuggestion — ranked candidate next-step
// =============================================================================

/// A ranked candidate tactic that the suggestion engine proposes for
/// the focused goal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TacticSuggestion {
    /// Source-form snippet that the user can type / accept.  Suitable
    /// for direct insertion at the cursor position.
    pub snippet: Text,
    /// Human-readable reason why this is suggested (used by the
    /// "why-this-suggestion" hover in IDE).
    pub rationale: Text,
    /// Ranking score in `[0.0, 1.0]` (higher = more likely correct).
    pub score: f64,
    /// Suggestion category (used by IDE icon rendering and ranking).
    pub category: SuggestionCategory,
}

/// Coarse classification of the suggestion's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuggestionCategory {
    /// `apply <lemma>` — name lifted from the available-lemmas pool.
    LemmaApplication,
    /// Tactic invocation (`auto`, `lia`, `decide`, …).
    TacticInvocation,
    /// `intro` / `intros` / `revert` — proof-state navigation.
    StateNavigation,
    /// `rewrite` / `rw` — equality manipulation.
    Rewriting,
    /// LLM-proposed step (only used when an LLM adapter is wired).
    LlmProposal,
}

impl SuggestionCategory {
    pub fn name(self) -> &'static str {
        match self {
            SuggestionCategory::LemmaApplication => "lemma",
            SuggestionCategory::TacticInvocation => "tactic",
            SuggestionCategory::StateNavigation  => "navigation",
            SuggestionCategory::Rewriting        => "rewrite",
            SuggestionCategory::LlmProposal      => "llm",
        }
    }
}

// =============================================================================
// SuggestionEngine — the trait boundary
// =============================================================================

/// Single dispatch interface for ranked tactic suggestions.  Every
/// IDE / REPL / CLI consumer routes through this one trait.
///
/// **Purity contract:** implementations MUST be pure — no I/O, no
/// network, no time-of-day dependence.  Side-effecting adapters
/// (LLM, registry-fetch) wrap a pure inner engine via composition.
pub trait SuggestionEngine {
    /// Return ranked tactic suggestions for the focused goal.  May
    /// return an empty list if no suggestions match.
    ///
    /// Caller-provided `max_results` bounds the response size; the
    /// engine MUST return at most that many entries.
    fn suggest(&self, view: &ProofStateView, max_results: usize) -> Vec<TacticSuggestion>;
}

// =============================================================================
// DefaultSuggestionEngine — V0 reference
// =============================================================================

/// V0 reference implementation:
///
///   1. **Lemma fuzzy-match.**  For each lemma in `available_lemmas`,
///      compute a similarity score between the lemma's signature and
///      the focused goal's proposition (case-insensitive substring +
///      shared-token count).  Top-k get LemmaApplication suggestions.
///
///   2. **Default tactics.**  Always offers `apply auto`, `apply lia`,
///      `apply decide` as fallback TacticInvocation suggestions
///      (lower score).
///
///   3. **State navigation.**  When the goal is a Π-type (universal
///      quantifier), offers `intro` / `intros` as StateNavigation
///      suggestions.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSuggestionEngine;

impl DefaultSuggestionEngine {
    pub fn new() -> Self {
        Self
    }
}

impl SuggestionEngine for DefaultSuggestionEngine {
    fn suggest(&self, view: &ProofStateView, max_results: usize) -> Vec<TacticSuggestion> {
        let goal = match view.focused_goal() {
            Some(g) => g,
            None => return Vec::new(),
        };

        let mut suggestions: Vec<TacticSuggestion> = Vec::new();

        // 1) Lemma fuzzy-match.
        let goal_text = goal.proposition.as_str().to_lowercase();
        for lemma in &view.available_lemmas {
            let score = similarity_score(&goal_text, &lemma.signature.as_str().to_lowercase());
            if score > 0.0 {
                suggestions.push(TacticSuggestion {
                    snippet: Text::from(format!("apply {};", lemma.name.as_str())),
                    rationale: Text::from(format!(
                        "lemma {} ({}) shares {:.0}% structural overlap with the goal",
                        lemma.name.as_str(),
                        lemma.lineage.as_str(),
                        score * 100.0
                    )),
                    score,
                    category: SuggestionCategory::LemmaApplication,
                });
            }
        }

        // 2) State navigation for Π-shaped goals.
        let goal_text_lower = goal.proposition.as_str().to_lowercase();
        if goal_text_lower.starts_with("forall")
            || goal_text_lower.starts_with("∀")
            || goal_text_lower.contains("->")
            || goal_text_lower.contains("→")
        {
            suggestions.push(TacticSuggestion {
                snippet: Text::from("intro h;"),
                rationale: Text::from(
                    "goal is a Π-type / implication — introduce the hypothesis as `h`",
                ),
                score: 0.6,
                category: SuggestionCategory::StateNavigation,
            });
        }

        // 3) Default tactic fallbacks (lower score so they trail
        //    lemma-matches but always available).
        suggestions.push(TacticSuggestion {
            snippet: Text::from("apply auto;"),
            rationale: Text::from("portfolio SMT + tactic-registry auto-search (verify=formal)"),
            score: 0.20,
            category: SuggestionCategory::TacticInvocation,
        });
        suggestions.push(TacticSuggestion {
            snippet: Text::from("apply lia;"),
            rationale: Text::from(
                "linear-integer-arithmetic decision procedure (kernel-checkable witness)",
            ),
            score: 0.18,
            category: SuggestionCategory::TacticInvocation,
        });
        suggestions.push(TacticSuggestion {
            snippet: Text::from("apply decide;"),
            rationale: Text::from(
                "boolean-tautology truth-table decision (kernel-checkable witness)",
            ),
            score: 0.16,
            category: SuggestionCategory::TacticInvocation,
        });

        // Rank: descending score, stable; truncate to max_results.
        suggestions.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        suggestions.truncate(max_results);
        suggestions
    }
}

/// Crude similarity score: shared-token count / max-token-count.
/// Returns a value in `[0.0, 1.0]`.  V1 promotion: typed unification
/// score against the goal's proposition.
fn similarity_score(a: &str, b: &str) -> f64 {
    let tokens_a: std::collections::HashSet<&str> =
        a.split(|c: char| !c.is_alphanumeric() && c != '_').filter(|s| !s.is_empty()).collect();
    let tokens_b: std::collections::HashSet<&str> =
        b.split(|c: char| !c.is_alphanumeric() && c != '_').filter(|s| !s.is_empty()).collect();
    if tokens_a.is_empty() || tokens_b.is_empty() {
        return 0.0;
    }
    let shared = tokens_a.intersection(&tokens_b).count();
    let denom = tokens_a.len().max(tokens_b.len());
    shared as f64 / denom as f64
}

// =============================================================================
// CompositeEngine — runs multiple engines and merges suggestions
// =============================================================================

/// Combines multiple [`SuggestionEngine`]s into one — useful for
/// composing the default engine with an LLM adapter, registry-search
/// adapter, etc.  Suggestions are merged + re-sorted by score; ties
/// broken by source order.
pub struct CompositeEngine {
    /// Component engines, called in order; their suggestions are
    /// merged.
    pub engines: Vec<Box<dyn SuggestionEngine>>,
}

impl std::fmt::Debug for CompositeEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CompositeEngine {{ engines: <{}> }}", self.engines.len())
    }
}

impl CompositeEngine {
    pub fn new(engines: Vec<Box<dyn SuggestionEngine>>) -> Self {
        Self { engines }
    }
}

impl SuggestionEngine for CompositeEngine {
    fn suggest(&self, view: &ProofStateView, max_results: usize) -> Vec<TacticSuggestion> {
        let mut all: Vec<TacticSuggestion> = Vec::new();
        for e in &self.engines {
            all.extend(e.suggest(view, max_results));
        }
        all.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        all.truncate(max_results);
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_view() -> ProofStateView {
        ProofStateView {
            theorem_name: Text::from("test_thm"),
            goals: vec![ProofGoalSummary {
                goal_id: 0,
                proposition: Text::from("forall x. x > 0 -> x + 1 > 0"),
                hypotheses: vec![],
                is_focused: true,
            }],
            available_lemmas: vec![
                LemmaSummary {
                    name: Text::from("nat_succ_pos"),
                    signature: Text::from("forall x. x > 0 -> succ(x) > 0"),
                    lineage: Text::from("core"),
                },
                LemmaSummary {
                    name: Text::from("unrelated_lemma"),
                    signature: Text::from("List<Int> append associative"),
                    lineage: Text::from("core"),
                },
            ],
            history: Vec::new(),
        }
    }

    // ----- ProofStateView -----

    #[test]
    fn empty_view_is_complete() {
        let v = ProofStateView::empty("foo");
        assert!(v.is_complete());
        assert!(v.focused_goal().is_none());
    }

    #[test]
    fn focused_goal_returns_first_focused() {
        let v = sample_view();
        let f = v.focused_goal().unwrap();
        assert_eq!(f.goal_id, 0);
    }

    #[test]
    fn focused_goal_falls_back_to_first_when_no_focus_flag() {
        let mut v = sample_view();
        v.goals[0].is_focused = false;
        // Falls back to first goal even if not flagged focused.
        let f = v.focused_goal().unwrap();
        assert_eq!(f.goal_id, 0);
    }

    // ----- DefaultSuggestionEngine -----

    #[test]
    fn engine_suggests_relevant_lemma_first() {
        let v = sample_view();
        let engine = DefaultSuggestionEngine::new();
        let s = engine.suggest(&v, 5);
        assert!(!s.is_empty());
        // The relevant lemma `nat_succ_pos` should rank above the
        // unrelated `unrelated_lemma`.
        let nat_pos = s.iter().position(|x| x.snippet.as_str().contains("nat_succ_pos"));
        let unrel = s.iter().position(|x| x.snippet.as_str().contains("unrelated_lemma"));
        assert!(nat_pos.is_some(), "relevant lemma should appear");
        if let (Some(np), Some(un)) = (nat_pos, unrel) {
            assert!(np < un, "relevant lemma must rank above unrelated");
        }
    }

    #[test]
    fn engine_offers_intro_for_pi_shaped_goals() {
        let v = sample_view();
        let engine = DefaultSuggestionEngine::new();
        let s = engine.suggest(&v, 10);
        let has_intro = s.iter().any(|x| x.snippet.as_str().starts_with("intro"));
        assert!(has_intro, "Π-shaped goal should suggest `intro`");
    }

    #[test]
    fn engine_always_offers_auto_lia_decide() {
        let v = sample_view();
        let engine = DefaultSuggestionEngine::new();
        let s = engine.suggest(&v, 10);
        assert!(s.iter().any(|x| x.snippet.as_str().contains("auto")));
        assert!(s.iter().any(|x| x.snippet.as_str().contains("lia")));
        assert!(s.iter().any(|x| x.snippet.as_str().contains("decide")));
    }

    #[test]
    fn engine_respects_max_results() {
        let v = sample_view();
        let engine = DefaultSuggestionEngine::new();
        let s = engine.suggest(&v, 2);
        assert!(s.len() <= 2);
    }

    #[test]
    fn engine_returns_empty_for_complete_proof() {
        let v = ProofStateView::empty("done");
        let engine = DefaultSuggestionEngine::new();
        assert!(engine.suggest(&v, 5).is_empty());
    }

    // ----- CompositeEngine -----

    #[test]
    fn composite_merges_and_reranks() {
        let v = sample_view();
        let engines: Vec<Box<dyn SuggestionEngine>> = vec![
            Box::new(DefaultSuggestionEngine::new()),
            Box::new(DefaultSuggestionEngine::new()),
        ];
        let composite = CompositeEngine::new(engines);
        let s = composite.suggest(&v, 3);
        assert!(s.len() <= 3);
        // Composite result is the union (with possible duplicates),
        // re-sorted by score.
        let mut prev = f64::INFINITY;
        for entry in &s {
            assert!(entry.score <= prev, "results must be score-descending");
            prev = entry.score;
        }
    }

    // ----- Similarity scoring -----

    #[test]
    fn similarity_full_match_is_one() {
        assert_eq!(similarity_score("foo bar baz", "foo bar baz"), 1.0);
    }

    #[test]
    fn similarity_no_overlap_is_zero() {
        assert_eq!(similarity_score("alpha beta", "gamma delta"), 0.0);
    }

    #[test]
    fn similarity_empty_is_zero() {
        assert_eq!(similarity_score("", "foo"), 0.0);
        assert_eq!(similarity_score("foo", ""), 0.0);
    }

    #[test]
    fn suggestion_category_names_distinct() {
        use std::collections::HashSet;
        let names: HashSet<&str> = [
            SuggestionCategory::LemmaApplication,
            SuggestionCategory::TacticInvocation,
            SuggestionCategory::StateNavigation,
            SuggestionCategory::Rewriting,
            SuggestionCategory::LlmProposal,
        ]
        .iter()
        .map(|c| c.name())
        .collect();
        assert_eq!(names.len(), 5);
    }
}
