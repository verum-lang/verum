//! Heuristic "suggested next tactics" for failed proof goals.
//!
//! When a tactic script exhausts its options without closing a goal,
//! the user sees a diagnostic with the residual goal and the exhausted
//! tactic stack. A well-chosen "try this next" list turns that
//! diagnostic from a dead-end into a starting point.
//!
//! The heuristics here are **deliberately simple structural rules**
//! over the goal's proposition shape — they do NOT run the solver,
//! do NOT mutate state, and do NOT guarantee that the suggested
//! tactic will close the goal. They are hints, not proofs.
//!
//! Each suggestion carries a confidence level (high / medium / low)
//! and a one-line rationale, so the user can see WHY a tactic is
//! suggested.
//!
//! # Integration point
//!
//! The proof-search orchestrator in `verum_smt::proof_search` calls
//! [`suggest_next_tactics`] after a user tactic script fails to
//! close a goal. The result flows into the `E501 tactic failed`
//! diagnostic (see `docs/verification/tactic-dsl.md §6.1`).
//!
//! # Extending
//!
//! Add a new rule to [`suggest_next_tactics`] inline or, for rules
//! requiring state, implement [`HeuristicRule`] and register via
//! [`HeuristicRegistry::register`]. The registry is keyed by
//! rule-name so duplicates across crates are caught at registration
//! time rather than producing conflicting suggestions.

use crate::tactic_evaluation::{Goal, Hypothesis};
use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_common::{List, Text};

/// Confidence level on a suggestion — how likely it is to help.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Confidence {
    /// The goal's shape uniquely determines this tactic's applicability.
    /// Example: `forall x. P(x)` goals should start with `intro`.
    High,
    /// The tactic is commonly useful for this goal shape; may need
    /// follow-up.
    Medium,
    /// The tactic is a generic fallback; try if nothing else fits.
    Low,
}

/// One suggested tactic to try after a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TacticSuggestion {
    /// The tactic's surface-syntax name (without `proof by `).
    pub tactic: Text,
    /// Short human-readable rationale — why we think this helps.
    pub rationale: Text,
    /// Confidence in the suggestion.
    pub confidence: Confidence,
}

impl TacticSuggestion {
    /// High-confidence suggestion.
    pub fn high(tactic: &str, rationale: &str) -> Self {
        Self {
            tactic: Text::from(tactic),
            rationale: Text::from(rationale),
            confidence: Confidence::High,
        }
    }

    /// Medium-confidence suggestion.
    pub fn medium(tactic: &str, rationale: &str) -> Self {
        Self {
            tactic: Text::from(tactic),
            rationale: Text::from(rationale),
            confidence: Confidence::Medium,
        }
    }

    /// Low-confidence (generic fallback) suggestion.
    pub fn low(tactic: &str, rationale: &str) -> Self {
        Self {
            tactic: Text::from(tactic),
            rationale: Text::from(rationale),
            confidence: Confidence::Low,
        }
    }
}

/// Produce a ranked list of tactic suggestions for `goal`, skipping
/// any tactic already in `exhausted`.
///
/// The returned list is **ordered by confidence** (High → Medium →
/// Low); inside a confidence class, rules fire in registration
/// order. Callers typically show the top 3–5 to the user.
///
/// # Guarantees
///
/// - Deterministic: same goal + same exhausted set → same list.
/// - Side-effect-free: no solver invocation, no mutation.
/// - No false negatives in the shape analysis — if a rule matches
///   the goal's outer structure, it fires.
///
/// # Example
///
/// ```rust,ignore
/// use verum_verification::tactic_heuristics::suggest_next_tactics;
/// let goal = /* forall x. P(x) */;
/// let hints = suggest_next_tactics(&goal, &[]);
/// assert_eq!(hints[0].tactic.as_str(), "intro");
/// ```
pub fn suggest_next_tactics(goal: &Goal, exhausted: &[&str]) -> Vec<TacticSuggestion> {
    let mut out: Vec<TacticSuggestion> = Vec::new();
    let prop = &*goal.proposition;

    // -- Rule 1: trivially-true bool literal (`true`) ----------------
    if is_bool_true(prop) {
        out.push(TacticSuggestion::high(
            "refl",
            "goal is `true` — refl closes trivially",
        ));
    }

    // -- Rule 2: syntactically-identical equality --------------------
    if let ExprKind::Binary { op: BinOp::Eq, left, right } = &prop.kind {
        if expr_structurally_eq(left, right) {
            out.push(TacticSuggestion::high(
                "refl",
                "lhs and rhs are syntactically identical",
            ));
        } else if is_arithmetic(left) && is_arithmetic(right) {
            out.push(TacticSuggestion::medium(
                "omega",
                "both sides are linear arithmetic — omega decides LIA",
            ));
            out.push(TacticSuggestion::medium(
                "ring",
                "both sides are polynomial — ring normalises algebraically",
            ));
        } else {
            out.push(TacticSuggestion::medium(
                "simp",
                "normalise both sides, then try refl",
            ));
            out.push(TacticSuggestion::medium(
                "rewrite",
                "if a lemma equates lhs with rhs, rewrite closes directly",
            ));
        }
    }

    // -- Rule 3: goal is an assumed hypothesis -----------------------
    if goal.hypotheses.iter().any(|h| hyp_matches_goal(h, prop)) {
        out.push(TacticSuggestion::high(
            "assumption",
            "a hypothesis in scope has the same type as the goal",
        ));
    }

    // -- Rule 4: conjunction goal (P && Q) ---------------------------
    if let ExprKind::Binary { op: BinOp::And, .. } = &prop.kind {
        out.push(TacticSuggestion::high(
            "split",
            "goal is a conjunction — split into subgoals",
        ));
    }

    // -- Rule 5: disjunction goal (P || Q) ---------------------------
    if let ExprKind::Binary { op: BinOp::Or, .. } = &prop.kind {
        out.push(TacticSuggestion::medium(
            "left",
            "goal is a disjunction — prove the left side if it holds",
        ));
        out.push(TacticSuggestion::medium(
            "right",
            "goal is a disjunction — prove the right side if it holds",
        ));
    }

    // -- Rule 6: implication goal (P -> Q) ---------------------------
    if matches!(&prop.kind, ExprKind::Binary { op: BinOp::Imply, .. }) {
        out.push(TacticSuggestion::high(
            "intro",
            "goal is an implication — introduce the hypothesis",
        ));
    }

    // -- Rule 7: forall-shaped goal ----------------------------------
    if is_forall_call(prop) {
        out.push(TacticSuggestion::high(
            "intro",
            "goal is universally quantified — introduce the binder",
        ));
    }

    // -- Rule 8: exists-shaped goal ----------------------------------
    if is_exists_call(prop) {
        out.push(TacticSuggestion::high(
            "witness",
            "goal is existentially quantified — provide a witness",
        ));
    }

    // -- Rule 9: match-shaped scrutinee in hypotheses ----------------
    // If a hypothesis has an inductive (sum) type, induction may
    // close sub-goals by case analysis. Medium confidence because
    // we don't know the exact inductive shape here.
    for h in goal.hypotheses.iter() {
        if hyp_looks_inductive(h) {
            out.push(TacticSuggestion::medium(
                "induction",
                "hypothesis has an inductive type — case analysis may help",
            ));
            break;
        }
    }

    // -- Rule 10: generic fallbacks ----------------------------------
    out.push(TacticSuggestion::low(
        "auto",
        "generic fallback — tries structural + SMT + simp chain",
    ));
    out.push(TacticSuggestion::low(
        "smt",
        "send the goal to the SMT backend directly; may timeout",
    ));

    // Filter out exhausted tactics, preserving suggestion order.
    let exhausted_set: std::collections::HashSet<&str> =
        exhausted.iter().copied().collect();
    out.retain(|s| !exhausted_set.contains(s.tactic.as_str()));

    // Sort by confidence (High > Medium > Low). Stable sort preserves
    // registration order within each confidence tier.
    out.sort_by_key(|s| match s.confidence {
        Confidence::High => 0,
        Confidence::Medium => 1,
        Confidence::Low => 2,
    });

    out
}

// =============================================================================
// Shape-inspection helpers. Kept conservative — any predicate that
// would require semantic analysis (type checking, substitution) is
// avoided in favour of syntactic-shape recognition.
// =============================================================================

fn is_bool_true(e: &Expr) -> bool {
    matches!(
        &e.kind,
        ExprKind::Literal(lit)
            if matches!(&lit.kind, verum_ast::literal::LiteralKind::Bool(b) if *b)
    )
}

fn is_arithmetic(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Literal(lit) => {
            matches!(
                &lit.kind,
                verum_ast::literal::LiteralKind::Int(_)
                    | verum_ast::literal::LiteralKind::Float(_)
            )
        }
        ExprKind::Binary { op, left, right } => {
            matches!(
                op,
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem
            ) && is_arithmetic(left)
                && is_arithmetic(right)
        }
        ExprKind::Unary { op: verum_ast::expr::UnOp::Neg, expr } => is_arithmetic(expr),
        ExprKind::Path(_) => true, // a bare variable could be Int/Real
        _ => false,
    }
}

/// Syntactic structural equality — ignores spans, respects operator
/// symmetry only for strictly commutative ops. Conservative: a `false`
/// result does NOT rule out semantic equality, just says the refl
/// tactic wouldn't close the goal by refl alone.
fn expr_structurally_eq(a: &Expr, b: &Expr) -> bool {
    match (&a.kind, &b.kind) {
        (ExprKind::Literal(la), ExprKind::Literal(lb)) => la.kind == lb.kind,
        (ExprKind::Path(pa), ExprKind::Path(pb)) => {
            pa.segments.len() == pb.segments.len()
                && pa.segments.iter().zip(pb.segments.iter()).all(|(s1, s2)| {
                    match (s1, s2) {
                        (
                            verum_ast::ty::PathSegment::Name(i1),
                            verum_ast::ty::PathSegment::Name(i2),
                        ) => i1.name == i2.name,
                        _ => false,
                    }
                })
        }
        (
            ExprKind::Binary { op: o1, left: l1, right: r1 },
            ExprKind::Binary { op: o2, left: l2, right: r2 },
        ) => {
            o1 == o2
                && expr_structurally_eq(l1, l2)
                && expr_structurally_eq(r1, r2)
        }
        _ => false,
    }
}

fn is_forall_call(e: &Expr) -> bool {
    // `forall x in ... . ...` or `forall<T>(...)` — parse-level shape
    // varies; for now we check for a call whose callee name is `forall`
    // or a Quantifier-flavoured node if one exists in the AST.
    if let ExprKind::Call { func, .. } = &e.kind {
        if let ExprKind::Path(p) = &func.kind {
            if let Some(last) = p.segments.last() {
                if let verum_ast::ty::PathSegment::Name(id) = last {
                    return id.name.as_str() == "forall";
                }
            }
        }
    }
    false
}

fn is_exists_call(e: &Expr) -> bool {
    if let ExprKind::Call { func, .. } = &e.kind {
        if let ExprKind::Path(p) = &func.kind {
            if let Some(last) = p.segments.last() {
                if let verum_ast::ty::PathSegment::Name(id) = last {
                    return id.name.as_str() == "exists";
                }
            }
        }
    }
    false
}

fn hyp_matches_goal(h: &Hypothesis, goal: &Expr) -> bool {
    // Conservative: exact structural match between hypothesis's
    // proposition and the goal.
    expr_structurally_eq(&h.proposition, goal)
}

fn hyp_looks_inductive(_h: &Hypothesis) -> bool {
    // Without access to the type environment we cannot decisively
    // determine inductive-ness. Return false here; a richer version
    // (task #74) consults the type registry to detect Variant /
    // Inductive / HIT types.
    false
}

// =============================================================================
// Extensible registry — for cross-crate rules that need more than
// structural shape analysis.
// =============================================================================

/// A pluggable heuristic rule. Implementations walk the goal and
/// produce zero or more suggestions without interacting with the
/// proof-search state.
pub trait HeuristicRule: Send + Sync {
    /// Human-readable rule name (for deduplication and diagnostics).
    fn name(&self) -> &'static str;

    /// Emit suggestions for this goal. Returning an empty vector is
    /// valid and means "this rule does not apply."
    fn suggest(&self, goal: &Goal) -> Vec<TacticSuggestion>;
}

/// Registry of heuristic rules. Deduplicates by name at registration
/// time so conflicting rules from different crates are caught early.
#[derive(Default)]
pub struct HeuristicRegistry {
    rules: Vec<Box<dyn HeuristicRule>>,
}

impl std::fmt::Debug for HeuristicRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeuristicRegistry")
            .field("rule_count", &self.rules.len())
            .field(
                "rule_names",
                &self.rules.iter().map(|r| r.name()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl HeuristicRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a rule. Returns `Err` if a rule with the same name
    /// is already registered.
    pub fn register(
        &mut self,
        rule: Box<dyn HeuristicRule>,
    ) -> Result<(), Text> {
        let name = rule.name();
        if self.rules.iter().any(|r| r.name() == name) {
            return Err(Text::from(format!(
                "heuristic rule '{}' already registered",
                name
            )));
        }
        self.rules.push(rule);
        Ok(())
    }

    /// Run every registered rule against the goal, collecting all
    /// emitted suggestions.
    pub fn suggest_all(&self, goal: &Goal) -> Vec<TacticSuggestion> {
        let mut out: Vec<TacticSuggestion> = Vec::new();
        for r in &self.rules {
            out.extend(r.suggest(goal));
        }
        out
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tactic_evaluation::{Goal, Hypothesis};
    use verum_ast::expr::{BinOp, Expr, ExprKind};
    use verum_ast::literal::{Literal, LiteralKind};
    use verum_ast::Span;
    use verum_common::Heap;

    fn int_lit(n: i64) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::int(n as i128, Span::dummy())),
            Span::dummy(),
        )
    }

    fn bool_lit(b: bool) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::bool(b, Span::dummy())),
            Span::dummy(),
        )
    }

    fn binop(op: BinOp, l: Expr, r: Expr) -> Expr {
        Expr::new(
            ExprKind::Binary {
                op,
                left: Heap::new(l),
                right: Heap::new(r),
            },
            Span::dummy(),
        )
    }

    #[test]
    fn refl_suggested_for_bool_true() {
        let g = Goal::new(0, bool_lit(true));
        let sug = suggest_next_tactics(&g, &[]);
        assert!(sug.iter().any(|s| s.tactic.as_str() == "refl"));
        // Refl should outrank generic fallbacks.
        let refl_pos = sug.iter().position(|s| s.tactic.as_str() == "refl").unwrap();
        let auto_pos = sug.iter().position(|s| s.tactic.as_str() == "auto").unwrap();
        assert!(refl_pos < auto_pos);
    }

    #[test]
    fn refl_suggested_for_x_eq_x() {
        let eq = binop(BinOp::Eq, int_lit(42), int_lit(42));
        let g = Goal::new(0, eq);
        let sug = suggest_next_tactics(&g, &[]);
        assert!(sug
            .iter()
            .find(|s| s.tactic.as_str() == "refl")
            .map(|s| s.confidence == Confidence::High)
            .unwrap_or(false));
    }

    #[test]
    fn omega_suggested_for_arithmetic_equality() {
        let lhs = binop(BinOp::Add, int_lit(2), int_lit(3));
        let rhs = int_lit(5);
        let eq = binop(BinOp::Eq, lhs, rhs);
        let g = Goal::new(0, eq);
        let sug = suggest_next_tactics(&g, &[]);
        assert!(sug.iter().any(|s| s.tactic.as_str() == "omega"));
    }

    #[test]
    fn split_suggested_for_conjunction() {
        let conj = binop(BinOp::And, bool_lit(true), bool_lit(true));
        let g = Goal::new(0, conj);
        let sug = suggest_next_tactics(&g, &[]);
        assert!(sug
            .iter()
            .find(|s| s.tactic.as_str() == "split")
            .map(|s| s.confidence == Confidence::High)
            .unwrap_or(false));
    }

    #[test]
    fn left_and_right_suggested_for_disjunction() {
        let disj = binop(BinOp::Or, bool_lit(true), bool_lit(false));
        let g = Goal::new(0, disj);
        let sug = suggest_next_tactics(&g, &[]);
        let tactics: Vec<&str> =
            sug.iter().map(|s| s.tactic.as_str()).collect();
        assert!(tactics.contains(&"left"));
        assert!(tactics.contains(&"right"));
    }

    #[test]
    fn intro_suggested_for_implication() {
        let imp = binop(BinOp::Imply, bool_lit(true), bool_lit(true));
        let g = Goal::new(0, imp);
        let sug = suggest_next_tactics(&g, &[]);
        assert!(sug
            .iter()
            .find(|s| s.tactic.as_str() == "intro")
            .map(|s| s.confidence == Confidence::High)
            .unwrap_or(false));
    }

    #[test]
    fn exhausted_tactics_filtered_out() {
        let g = Goal::new(0, bool_lit(true));
        let sug = suggest_next_tactics(&g, &["refl"]);
        assert!(!sug.iter().any(|s| s.tactic.as_str() == "refl"));
        // auto fallback should still be present.
        assert!(sug.iter().any(|s| s.tactic.as_str() == "auto"));
    }

    #[test]
    fn assumption_suggested_when_hypothesis_matches() {
        let prop = bool_lit(true);
        let h = Hypothesis::new(
            verum_common::Text::from("h"),
            prop.clone(),
        );
        let mut g = Goal::new(0, prop);
        g.add_hypothesis(h);
        let sug = suggest_next_tactics(&g, &[]);
        assert!(sug.iter().any(|s| s.tactic.as_str() == "assumption"));
    }

    #[test]
    fn registry_rejects_duplicate_rules() {
        struct Dummy;
        impl HeuristicRule for Dummy {
            fn name(&self) -> &'static str {
                "dummy"
            }
            fn suggest(&self, _: &Goal) -> Vec<TacticSuggestion> {
                Vec::new()
            }
        }
        let mut reg = HeuristicRegistry::new();
        assert!(reg.register(Box::new(Dummy)).is_ok());
        let second = reg.register(Box::new(Dummy));
        assert!(second.is_err());
        let err = second.unwrap_err();
        assert!(err.as_str().contains("dummy"));
    }

    #[test]
    fn suggestions_are_deterministic() {
        let prop = binop(BinOp::And, bool_lit(true), bool_lit(true));
        let g1 = Goal::new(0, prop.clone());
        let g2 = Goal::new(42, prop); // different goal id, same shape
        let s1 = suggest_next_tactics(&g1, &[]);
        let s2 = suggest_next_tactics(&g2, &[]);
        assert_eq!(s1.len(), s2.len());
        for (a, b) in s1.iter().zip(s2.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn high_confidence_ranked_above_low() {
        // Trivial bool-true goal: refl(High), auto(Low), smt(Low).
        let g = Goal::new(0, bool_lit(true));
        let sug = suggest_next_tactics(&g, &[]);
        let first_low = sug
            .iter()
            .position(|s| s.confidence == Confidence::Low)
            .unwrap();
        // Every suggestion before the first Low should be High or Medium.
        for s in &sug[..first_low] {
            assert!(matches!(
                s.confidence,
                Confidence::High | Confidence::Medium
            ));
        }
    }
}

// Silence unused-import warnings when tests are disabled.
#[allow(dead_code)]
fn _keep_list_import() {
    let _: List<()> = List::new();
}
