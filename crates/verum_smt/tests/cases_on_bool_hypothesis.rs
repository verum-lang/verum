//! Locks the metadata-driven Bool case-analysis path in `try_cases_on`.
//!
//! Pre-fix `is_boolean_hypothesis` was a permanently-false stub:
//! ```ignore
//! fn is_boolean_hypothesis(&self, _hyp: &Expr) -> bool { false }
//! ```
//! That made the `cases_on` arm for plain Bool-typed Path hypotheses
//! unreachable. Any `cases_on h` where `h` was a Bool variable fell
//! through to the catch-all "not case-analyzable" error.
//!
//! Post-fix the engine carries an externally-populated
//! `bool_typed_hypotheses` set; the case-split arm produces two subgoals
//! whose labels reflect the chosen branch and whose hypotheses include
//! `h == true` / `h == false` equalities so downstream SMT discharge can
//! substitute the value into the goal.
//!
//! The engine still ships with **zero** hardcoded knowledge — names like
//! `b`, `flag`, `cond` are NOT recognized as Bool until the caller calls
//! `register_bool_hypothesis`.

use verum_ast::expr::{Expr, ExprKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{List, Maybe, Text};

use verum_smt::proof_search::{ProofGoal, ProofSearchEngine, ProofTactic};

fn ident_expr(name: &str) -> Expr {
    Expr::path(Path::single(Ident::new(name, Span::dummy())))
}

fn nontrivial_goal_with_hyp(hyp: Expr) -> ProofGoal {
    let mut hyps = List::new();
    hyps.push(hyp);
    // Plain identifier `goal_marker` so the trivial-close fast path
    // doesn't short-circuit before `try_cases_on` runs.
    ProofGoal::with_hypotheses(ident_expr("goal_marker"), hyps)
}

#[test]
fn unregistered_path_hypothesis_is_not_recognized_as_bool() {
    let mut engine = ProofSearchEngine::new();
    let goal = nontrivial_goal_with_hyp(ident_expr("b"));

    let result = engine.execute_tactic(
        &ProofTactic::CasesOn {
            hypothesis: Text::from("h0"),
        },
        &goal,
    );
    assert!(
        result.is_err(),
        "engine must not assume any Path-shaped hypothesis is Bool — \
         metadata must come from register_bool_hypothesis",
    );
}

#[test]
fn registered_bool_hypothesis_splits_into_two_subgoals() {
    let mut engine = ProofSearchEngine::new();
    engine.register_bool_hypothesis(Text::from("b"));

    let goal = nontrivial_goal_with_hyp(ident_expr("b"));
    let subgoals = engine
        .execute_tactic(
            &ProofTactic::CasesOn {
                hypothesis: Text::from("h0"),
            },
            &goal,
        )
        .expect("registered Bool hypothesis must dispatch through cases_on");

    assert_eq!(subgoals.len(), 2, "Bool case-split produces 2 subgoals");

    let labels: Vec<String> = subgoals
        .iter()
        .filter_map(|g| match &g.label {
            Maybe::Some(t) => Some(t.as_str().to_string()),
            Maybe::None => None,
        })
        .collect();
    assert_eq!(labels.len(), 2, "all subgoals must be labeled");
    assert!(labels.iter().any(|l| l.starts_with("case_true_")));
    assert!(labels.iter().any(|l| l.starts_with("case_false_")));
}

#[test]
fn each_subgoal_carries_an_equality_hypothesis() {
    // The two subgoals must record which branch they're in by adding
    // `b == true` / `b == false` as hypotheses. Pre-fix the subgoals only
    // got bare `true`/`false` literals, which the SMT solver couldn't
    // connect back to the variable being analyzed.
    let mut engine = ProofSearchEngine::new();
    engine.register_bool_hypothesis(Text::from("b"));

    let goal = nontrivial_goal_with_hyp(ident_expr("b"));
    let subgoals = engine
        .execute_tactic(
            &ProofTactic::CasesOn {
                hypothesis: Text::from("h0"),
            },
            &goal,
        )
        .unwrap();

    for sub in subgoals.iter() {
        // Each subgoal must contain at least one equality hypothesis
        // referencing the original Path expression.
        let has_eq_hyp = sub.hypotheses.iter().any(|h| {
            matches!(
                &h.kind,
                ExprKind::Binary {
                    op: verum_ast::BinOp::Eq,
                    ..
                }
            )
        });
        assert!(
            has_eq_hyp,
            "subgoal {:?} must include an equality hypothesis recording the chosen branch",
            sub.label
        );
    }
}
