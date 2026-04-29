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
        let prop = goal.proposition.as_str();
        let prop_lower = prop.to_lowercase();
        let goal_head = head_token(prop);

        // 1) Lemma match.  Score = base structural overlap (Jaccard
        //    over identifier tokens) + head-match bonus.  The head-
        //    bonus rewards lemmas whose signature head is the same
        //    head as the goal — a textual proxy for typed-head
        //    unification.
        for lemma in &view.available_lemmas {
            let score =
                lemma_score(prop, lemma.signature.as_str(), &goal_head);
            if score > 0.0 {
                suggestions.push(TacticSuggestion {
                    snippet: Text::from(format!("apply {};", lemma.name.as_str())),
                    rationale: Text::from(format!(
                        "lemma {} ({}) — score {:.2}, signature {}",
                        lemma.name.as_str(),
                        lemma.lineage.as_str(),
                        score,
                        lemma.signature.as_str()
                    )),
                    score,
                    category: SuggestionCategory::LemmaApplication,
                });
            }
        }

        // 2) Shape-aware tactics (#102 hardening).  Each fires only
        //    when the proposition's textual shape suggests it; each
        //    carries a structured `rationale` citing why.

        // 2a) Hypothesis match — `assumption` / `apply h`.
        for h in &goal.hypotheses {
            if h.ty.as_str() == prop {
                suggestions.push(TacticSuggestion {
                    snippet: Text::from("assumption;"),
                    rationale: Text::from(format!(
                        "hypothesis `{}` matches the goal exactly",
                        h.name.as_str()
                    )),
                    score: 0.95,
                    category: SuggestionCategory::TacticInvocation,
                });
                break;
            }
        }
        // 2b) Goal head matches a hypothesis head — `apply <h>`.
        for h in &goal.hypotheses {
            let h_head = head_token(h.ty.as_str());
            if h_head == goal_head && !goal_head.is_empty() {
                suggestions.push(TacticSuggestion {
                    snippet: Text::from(format!("apply {};", h.name.as_str())),
                    rationale: Text::from(format!(
                        "hypothesis `{}` has the same head as the goal (`{}`)",
                        h.name.as_str(),
                        goal_head
                    )),
                    score: 0.78,
                    category: SuggestionCategory::TacticInvocation,
                });
            }
        }
        // 2c) Reflexivity — `x = x` / `Path A x x` / `x ≡ x`.
        if looks_like_reflexive(prop) {
            suggestions.push(TacticSuggestion {
                snippet: Text::from("refl;"),
                rationale: Text::from(
                    "goal has shape `t = t` / `Path A t t` — discharged by reflexivity",
                ),
                score: 0.92,
                category: SuggestionCategory::TacticInvocation,
            });
        }
        // 2d) False-elim — `False` / `⊥` goal with a hypothesis in scope.
        if looks_like_false(&prop_lower) && !goal.hypotheses.is_empty() {
            suggestions.push(TacticSuggestion {
                snippet: Text::from("contradiction;"),
                rationale: Text::from(
                    "goal is `False`/`⊥` and the local context is non-empty — \
                     try contradiction (ex falso quodlibet)",
                ),
                score: 0.88,
                category: SuggestionCategory::TacticInvocation,
            });
        }
        // 2e) Top-level conjunction — `split` / `constructor`.
        if has_top_level(prop, &["/\\", "∧", "&&"]) {
            suggestions.push(TacticSuggestion {
                snippet: Text::from("split;"),
                rationale: Text::from(
                    "goal is a top-level conjunction — split into separate sub-goals",
                ),
                score: 0.80,
                category: SuggestionCategory::StateNavigation,
            });
        }
        // 2f) Top-level disjunction — `left` / `right`.
        if has_top_level(prop, &["\\/", "∨", "||"]) {
            for (snippet, rationale, score) in [
                (
                    "left;",
                    "goal is a disjunction — try the left branch first",
                    0.45,
                ),
                (
                    "right;",
                    "goal is a disjunction — try the right branch",
                    0.40,
                ),
            ] {
                suggestions.push(TacticSuggestion {
                    snippet: Text::from(snippet),
                    rationale: Text::from(rationale),
                    score,
                    category: SuggestionCategory::StateNavigation,
                });
            }
        }
        // 2g) Inductive hypothesis — `induction <name>` when the
        //     hypothesis's type head looks like an inductive
        //     (Nat / List / Tree).
        for h in &goal.hypotheses {
            let h_head = head_token(h.ty.as_str());
            if matches!(
                h_head.as_str(),
                "Nat" | "Int" | "List" | "Tree" | "Vec" | "Maybe"
            ) {
                suggestions.push(TacticSuggestion {
                    snippet: Text::from(format!("induction {};", h.name.as_str())),
                    rationale: Text::from(format!(
                        "hypothesis `{}` has inductive type `{}` — try structural induction",
                        h.name.as_str(),
                        h_head
                    )),
                    score: 0.55,
                    category: SuggestionCategory::StateNavigation,
                });
            }
        }

        // 3) State navigation for Π-shaped goals.  Promote when the
        //    arrow is at the *top level* (parenthesis-aware via the
        //    `has_top_level` helper) so `(A -> B) -> C` ranks intro
        //    over a structurally-internal arrow.
        let is_universal = prop_lower.starts_with("forall")
            || prop_lower.starts_with("∀");
        if is_universal || has_top_level(prop, &["->", "→"]) {
            suggestions.push(TacticSuggestion {
                snippet: Text::from("intro h;"),
                rationale: Text::from(
                    "goal is a Π-type / implication — introduce the hypothesis as `h`",
                ),
                score: 0.60,
                category: SuggestionCategory::StateNavigation,
            });
        }

        // 4) Default tactic fallbacks (lower score so they trail
        //    lemma-matches + shape-aware suggestions; always available).
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
        suggestions.sort_by(|a, b| {
            b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal)
        });
        suggestions.truncate(max_results);
        suggestions
    }
}

// =============================================================================
// Lemma scoring + textual shape predicates (#102 hardening)
// =============================================================================

/// Lemma similarity score combining structural-overlap (Jaccard
/// over identifier tokens) with a head-match bonus.  Range:
/// `[0.0, 1.0]`.
fn lemma_score(goal: &str, lemma_sig: &str, goal_head: &str) -> f64 {
    let base = similarity_score(&goal.to_lowercase(), &lemma_sig.to_lowercase());
    if base == 0.0 {
        return 0.0;
    }
    let lemma_head = head_token(lemma_sig);
    let head_bonus = if !goal_head.is_empty()
        && !lemma_head.is_empty()
        && goal_head.eq_ignore_ascii_case(&lemma_head)
    {
        0.25
    } else {
        0.0
    };
    (base + head_bonus).min(1.0)
}

/// Crude similarity score: shared-token count / max-token-count.
/// Returns a value in `[0.0, 1.0]`.
fn similarity_score(a: &str, b: &str) -> f64 {
    let tokens_a: std::collections::HashSet<&str> = a
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .collect();
    let tokens_b: std::collections::HashSet<&str> = b
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .collect();
    if tokens_a.is_empty() || tokens_b.is_empty() {
        return 0.0;
    }
    let shared = tokens_a.intersection(&tokens_b).count();
    let denom = tokens_a.len().max(tokens_b.len());
    shared as f64 / denom as f64
}

/// First identifier-shaped token of `s`.  Used as a textual proxy
/// for the typed head of a proposition (`P x y` → `P`,
/// `Path A x y` → `Path`, `forall n, ...` → `forall`).
fn head_token(s: &str) -> String {
    let trimmed = s.trim_start_matches(|c: char| !c.is_alphanumeric() && c != '_');
    let end = trimmed
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(trimmed.len());
    trimmed[..end].to_string()
}

/// True iff the proposition is textually `False` / `⊥` / `false`.
fn looks_like_false(prop_lower: &str) -> bool {
    let t = prop_lower.trim();
    t == "false" || t == "⊥" || t == "bot" || t == "⊥;"
}

/// True iff the proposition has shape `t = t` / `Path A t t` /
/// `t ≡ t` — recognises `=`, `≡`, and `Path A t t` shapes.  Best-
/// effort textual heuristic; the kernel still rechecks before the
/// suggestion is acted on.
fn looks_like_reflexive(prop: &str) -> bool {
    let s = prop.trim();
    // ASCII / Unicode equality: `lhs = rhs` or `lhs ≡ rhs`.
    for sep in [" ≡ ", " = "] {
        if let Some((lhs, rhs)) = s.split_once(sep) {
            if lhs.trim() == rhs.trim() && !lhs.trim().is_empty() {
                return true;
            }
        }
    }
    // Path shape: `Path A x y` or `Path<A>(x, y)`.  Match when the
    // last two whitespace-separated identifiers coincide.
    if s.starts_with("Path") {
        let tokens: Vec<&str> = s
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|t| !t.is_empty())
            .collect();
        // `["Path", "A", "x", "y"]` — last two are the endpoints.
        if tokens.len() >= 4 {
            return tokens[tokens.len() - 1] == tokens[tokens.len() - 2];
        }
    }
    false
}

/// True iff `s` contains any of `needles` at depth 0 (parenthesis-
/// aware).  Used to recognise top-level `/\` / `∨` / `->` operators
/// for shape-aware suggestions.  Byte-safe: needle comparison runs
/// against `s.as_bytes()` so multi-byte UTF-8 (∧, ∨, →) doesn't
/// panic on char-boundary slicing.
fn has_top_level(s: &str, needles: &[&str]) -> bool {
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            for n in needles {
                let nb = n.as_bytes();
                if i + nb.len() <= bytes.len() && &bytes[i..i + nb.len()] == nb {
                    return true;
                }
            }
        }
    }
    false
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

    // =========================================================================
    // Shape-aware suggestions (#102 hardening)
    // =========================================================================

    fn view_with(prop: &str, hyps: Vec<(&str, &str)>) -> ProofStateView {
        ProofStateView {
            theorem_name: Text::from("t"),
            goals: vec![ProofGoalSummary {
                goal_id: 0,
                proposition: Text::from(prop),
                hypotheses: hyps
                    .into_iter()
                    .map(|(n, ty)| HypothesisSummary {
                        name: Text::from(n),
                        ty: Text::from(ty),
                    })
                    .collect(),
                is_focused: true,
            }],
            available_lemmas: Vec::new(),
            history: Vec::new(),
        }
    }

    #[test]
    fn head_token_pulls_first_identifier() {
        assert_eq!(head_token("P x y"), "P");
        assert_eq!(head_token("Path A x y"), "Path");
        assert_eq!(head_token("forall n, P n"), "forall");
        assert_eq!(head_token("(P x)"), "P");
        assert_eq!(head_token(""), "");
    }

    #[test]
    fn looks_like_reflexive_recognises_equality() {
        assert!(looks_like_reflexive("x = x"));
        assert!(looks_like_reflexive("a + b = a + b"));
        assert!(looks_like_reflexive("foo ≡ foo"));
        assert!(looks_like_reflexive("Path A x x"));
        assert!(!looks_like_reflexive("x = y"));
        assert!(!looks_like_reflexive("Path A x y"));
    }

    #[test]
    fn has_top_level_paren_aware() {
        assert!(has_top_level("A -> B", &["->"]));
        assert!(has_top_level("(P x) /\\ (Q y)", &["/\\"]));
        // Nested arrow inside parens does NOT count as top-level.
        assert!(!has_top_level("(A -> B)", &["->"]));
    }

    #[test]
    fn suggests_assumption_when_hypothesis_matches_goal() {
        let v = view_with("P x", vec![("h", "P x")]);
        let s = DefaultSuggestionEngine::new().suggest(&v, 20);
        assert!(s.iter().any(|x| x.snippet.as_str() == "assumption;"));
    }

    #[test]
    fn suggests_apply_h_when_hypothesis_head_matches_goal_head() {
        let v = view_with("P x y", vec![("h", "P a b")]);
        let s = DefaultSuggestionEngine::new().suggest(&v, 20);
        assert!(s.iter().any(|x| x.snippet.as_str() == "apply h;"));
    }

    #[test]
    fn suggests_refl_when_goal_is_x_eq_x() {
        let v = view_with("x = x", vec![]);
        let s = DefaultSuggestionEngine::new().suggest(&v, 20);
        assert!(s.iter().any(|x| x.snippet.as_str() == "refl;"));
    }

    #[test]
    fn suggests_refl_for_path_a_x_x() {
        let v = view_with("Path Nat zero zero", vec![]);
        let s = DefaultSuggestionEngine::new().suggest(&v, 20);
        assert!(s.iter().any(|x| x.snippet.as_str() == "refl;"));
    }

    #[test]
    fn suggests_contradiction_when_false_with_hypothesis() {
        let v = view_with("False", vec![("h", "P /\\ ~P")]);
        let s = DefaultSuggestionEngine::new().suggest(&v, 20);
        assert!(s.iter().any(|x| x.snippet.as_str() == "contradiction;"));
    }

    #[test]
    fn does_not_suggest_contradiction_when_no_hypothesis() {
        let v = view_with("False", vec![]);
        let s = DefaultSuggestionEngine::new().suggest(&v, 20);
        assert!(!s.iter().any(|x| x.snippet.as_str() == "contradiction;"));
    }

    #[test]
    fn suggests_split_for_top_level_conjunction() {
        let v = view_with("P /\\ Q", vec![]);
        let s = DefaultSuggestionEngine::new().suggest(&v, 20);
        assert!(s.iter().any(|x| x.snippet.as_str() == "split;"));
    }

    #[test]
    fn suggests_split_for_unicode_conjunction() {
        let v = view_with("P ∧ Q", vec![]);
        let s = DefaultSuggestionEngine::new().suggest(&v, 20);
        assert!(s.iter().any(|x| x.snippet.as_str() == "split;"));
    }

    #[test]
    fn does_not_split_when_conjunction_is_inside_parens() {
        let v = view_with("(P /\\ Q) -> R", vec![]);
        let s = DefaultSuggestionEngine::new().suggest(&v, 20);
        assert!(!s.iter().any(|x| x.snippet.as_str() == "split;"));
    }

    #[test]
    fn suggests_left_and_right_for_top_level_disjunction() {
        let v = view_with("P \\/ Q", vec![]);
        let s = DefaultSuggestionEngine::new().suggest(&v, 20);
        assert!(s.iter().any(|x| x.snippet.as_str() == "left;"));
        assert!(s.iter().any(|x| x.snippet.as_str() == "right;"));
    }

    #[test]
    fn suggests_induction_for_inductive_hypothesis() {
        let v = view_with("P n", vec![("n", "Nat")]);
        let s = DefaultSuggestionEngine::new().suggest(&v, 20);
        assert!(s.iter().any(|x| x.snippet.as_str() == "induction n;"));
    }

    #[test]
    fn does_not_suggest_induction_for_non_inductive_hypothesis() {
        let v = view_with("P x", vec![("x", "Bool")]);
        let s = DefaultSuggestionEngine::new().suggest(&v, 20);
        assert!(!s
            .iter()
            .any(|x| x.snippet.as_str() == "induction x;"));
    }

    #[test]
    fn assumption_outranks_default_tactics() {
        // The exact-hypothesis match (score 0.95) must come before
        // the auto/lia/decide fallbacks (score ≤ 0.20).
        let v = view_with("P x", vec![("h", "P x")]);
        let s = DefaultSuggestionEngine::new().suggest(&v, 20);
        let assume_pos = s
            .iter()
            .position(|x| x.snippet.as_str() == "assumption;")
            .unwrap();
        let auto_pos = s
            .iter()
            .position(|x| x.snippet.as_str().contains("auto"))
            .unwrap();
        assert!(
            assume_pos < auto_pos,
            "assumption ({}) must rank before auto ({})",
            assume_pos,
            auto_pos
        );
    }

    #[test]
    fn lemma_score_head_match_bonus_lifts_targeted_lemma() {
        // Two lemmas with the same Jaccard overlap on shared tokens,
        // but only one's head matches the goal head.  The head-match
        // lemma must rank higher.
        let v = ProofStateView {
            theorem_name: Text::from("t"),
            goals: vec![ProofGoalSummary {
                goal_id: 0,
                proposition: Text::from("P x y"),
                hypotheses: vec![],
                is_focused: true,
            }],
            available_lemmas: vec![
                LemmaSummary {
                    name: Text::from("p_intro"),
                    signature: Text::from("P x y"),
                    lineage: Text::from("core"),
                },
                LemmaSummary {
                    name: Text::from("q_intro"),
                    signature: Text::from("Q x y"),
                    lineage: Text::from("core"),
                },
            ],
            history: Vec::new(),
        };
        let s = DefaultSuggestionEngine::new().suggest(&v, 20);
        let p_pos = s
            .iter()
            .position(|x| x.snippet.as_str().contains("p_intro"))
            .unwrap();
        let q_pos = s
            .iter()
            .position(|x| x.snippet.as_str().contains("q_intro"))
            .unwrap();
        assert!(
            p_pos < q_pos,
            "head-match lemma must rank above non-head-match"
        );
    }

    #[test]
    fn task_102_shape_aware_suggestions_carry_structured_rationale() {
        // Pin: every shape-aware suggestion carries a non-empty
        // rationale that cites *why* it fired.  Empty rationales
        // would defeat the IDE's hover-explanation panel.
        let v = view_with(
            "x = x",
            vec![("h", "P x"), ("n", "Nat")],
        );
        let s = DefaultSuggestionEngine::new().suggest(&v, 20);
        for sug in &s {
            assert!(
                !sug.rationale.as_str().is_empty(),
                "every suggestion must carry a non-empty rationale: {:?}",
                sug.snippet.as_str()
            );
        }
    }
}
