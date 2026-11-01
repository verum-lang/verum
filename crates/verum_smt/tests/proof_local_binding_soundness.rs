use verum_ast::{BinOp, Expr, ExprKind, Ident, Literal, Pattern, Span};
use verum_common::{Heap, List, Maybe, Text};
use verum_smt::proof_search::{MatchArm, ProofGoal, ProofSearchEngine, ProofTactic};

fn boolean(value: bool) -> Expr {
    Expr::new(ExprKind::Literal(Literal::bool(value, Span::dummy())), Span::dummy())
}

fn free_equality() -> Expr {
    Expr::new(ExprKind::Binary {
        op: BinOp::Eq,
        left: Heap::new(Expr::ident(Ident::new("x", Span::dummy()))),
        right: Heap::new(boolean(false)),
    }, Span::dummy())
}

#[test]
fn a_local_definition_cannot_manufacture_a_fact_about_an_existing_free_name() {
    let mut engine = ProofSearchEngine::new();
    let goal = ProofGoal::with_hypotheses(free_equality(), List::new());
    let tactic = ProofTactic::Let {
        name: Text::from("x"), value: Heap::new(boolean(false)),
        body: Heap::new(ProofTactic::Assumption),
    };
    let error = engine.execute_tactic(&tactic, &goal).expect_err("x is unconstrained");
    assert!(error.to_string().contains("checked scoped binding semantics"), "{error}");
    assert!(goal.hypotheses.is_empty());
    assert!(engine.execute_tactic(&ProofTactic::Assumption, &goal).is_err());
}

#[test]
fn pattern_bindings_cannot_manufacture_the_same_free_name_equation() {
    let mut engine = ProofSearchEngine::new();
    let goal = ProofGoal::with_hypotheses(free_equality(), List::new());
    let tactic = ProofTactic::Match {
        scrutinee: Heap::new(boolean(false)),
        arms: List::from_iter([MatchArm {
            pattern: Pattern::ident(Ident::new("x", Span::dummy()), false, Span::dummy()),
            guard: Maybe::None, body: ProofTactic::Assumption,
        }]),
    };
    let error = engine.execute_tactic(&tactic, &goal).expect_err("pattern binding must be scoped");
    assert!(error.to_string().contains("checked scoped binding semantics"), "{error}");
    assert!(goal.hypotheses.is_empty());
}

#[test]
fn closed_match_arms_and_existing_hypotheses_keep_their_meaning() {
    let mut engine = ProofSearchEngine::new();
    let tactic = ProofTactic::Match {
        scrutinee: Heap::new(boolean(false)),
        arms: List::from_iter([MatchArm {
            pattern: Pattern::literal(Literal::bool(false, Span::dummy())),
            guard: Maybe::None, body: ProofTactic::Assumption,
        }]),
    };
    assert!(engine.execute_tactic(&tactic,
        &ProofGoal::with_hypotheses(boolean(true), List::from_iter([boolean(true)]))).unwrap().is_empty());
    assert!(engine.execute_tactic(&tactic,
        &ProofGoal::with_hypotheses(boolean(false), List::new())).is_err());
    let existing = ProofGoal::with_hypotheses(free_equality(), List::from_iter([free_equality()]));
    assert!(engine.execute_tactic(&ProofTactic::Assumption, &existing).unwrap().is_empty());
}
