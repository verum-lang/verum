//! Locks the architectural invariant that `try_cases_on` resolves variant
//! constructors via the externally-populated `variant_map` registry, with no
//! hardcoded stdlib type names like `"Some"` or `"None"`.
//!
//! Pre-fix the engine matched literal `"Some" | "None"` to detect Maybe
//! constructors and emitted a fixed pair of placeholder subgoals. That
//! violated the no-stdlib-knowledge-in-compiler rule (CLAUDE.md) and silently
//! gave any non-Maybe variant type ("Result", "Color", user types) an
//! unhelpful catch-all error from `cases_on`.
//!
//! These tests verify the generalized version:
//! 1. Maybe-shaped types still split into 2 subgoals (None / Some).
//! 2. 3-way variants (Color = Red | Green | Blue) split into 3 subgoals —
//!    arity is no longer fixed at 2.
//! 3. The engine has zero knowledge until `register_variant_type` is called —
//!    an unregistered constructor falls through to the catch-all error arm.

use verum_ast::expr::{Expr, ExprKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{Heap, List, Maybe, Text};

use verum_smt::proof_search::{ProofGoal, ProofSearchEngine, ProofTactic};

fn ident_expr(name: &str) -> Expr {
    Expr::path(Path::single(Ident::new(name, Span::dummy())))
}

fn ctor_call(name: &str) -> Expr {
    Expr::new(
        ExprKind::Call {
            func: Heap::new(ident_expr(name)),
            type_args: List::new(),
            args: List::new(),
        },
        Span::dummy(),
    )
}

fn nontrivial_goal_with_hyp(hyp: Expr) -> ProofGoal {
    let mut hyps = List::new();
    hyps.push(hyp);
    // Plain identifier `foo` — not a literal `true`, so the trivial-close
    // fast path doesn't short-circuit dispatch. Forces `try_cases_on` to run.
    ProofGoal::with_hypotheses(ident_expr("foo"), hyps)
}

#[test]
fn unregistered_constructor_falls_through_to_error() {
    let mut engine = ProofSearchEngine::new();
    let goal = nontrivial_goal_with_hyp(ctor_call("Some"));

    let result = engine.execute_tactic(
        &ProofTactic::CasesOn { hypothesis: Text::from("h0") },
        &goal,
    );
    assert!(
        result.is_err(),
        "engine must NOT recognize 'Some' until register_variant_type runs — \
         proves zero hardcoded stdlib type knowledge",
    );
}

#[test]
fn two_way_variant_emits_two_subgoals() {
    let mut engine = ProofSearchEngine::new();
    engine.register_variant_type(
        Text::from("Maybe"),
        vec![Text::from("None"), Text::from("Some")],
    );

    let goal = nontrivial_goal_with_hyp(ctor_call("Some"));
    let subgoals = engine
        .execute_tactic(
            &ProofTactic::CasesOn { hypothesis: Text::from("h0") },
            &goal,
        )
        .expect("registered constructor must dispatch through cases_on");

    assert_eq!(subgoals.len(), 2);
}

#[test]
fn three_way_variant_emits_three_subgoals_with_constructor_labels() {
    // Pre-fix: 3-way variants were silently misclassified as not
    // case-analyzable because the dispatcher only handled the 2-way Maybe
    // shape. Generic version emits one subgoal per registered constructor,
    // labelled with the constructor's lowercased name.
    let mut engine = ProofSearchEngine::new();
    engine.register_variant_type(
        Text::from("Color"),
        vec![
            Text::from("Red"),
            Text::from("Green"),
            Text::from("Blue"),
        ],
    );

    let goal = nontrivial_goal_with_hyp(ctor_call("Green"));
    let subgoals = engine
        .execute_tactic(
            &ProofTactic::CasesOn { hypothesis: Text::from("h0") },
            &goal,
        )
        .expect("3-way variant must dispatch through cases_on");

    assert_eq!(
        subgoals.len(),
        3,
        "arity is no longer hardcoded to 2 — proves the generalization is real",
    );

    let labels: Vec<String> = subgoals
        .iter()
        .filter_map(|g| match &g.label {
            Maybe::Some(t) => Some(t.as_str().to_string()),
            Maybe::None => None,
        })
        .collect();
    assert_eq!(labels.len(), 3, "all subgoals must be labeled");
    assert!(labels.iter().any(|l| l.contains("red")));
    assert!(labels.iter().any(|l| l.contains("green")));
    assert!(labels.iter().any(|l| l.contains("blue")));
}
