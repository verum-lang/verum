//! Signed integer induction retains preconditions at each target and covers
//! zero, positive successors and negative predecessors. Constructor metadata
//! alone must never be mistaken for a checked structural eliminator.

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::{IntLit, Literal, LiteralKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{Heap, List, Maybe, Text};

use verum_smt::proof_search::{ProofGoal, ProofSearchEngine, ProofTactic};

fn ident_expr(name: &str) -> Expr {
    Expr::path(Path::single(Ident::new(name, Span::dummy())))
}

fn int_expr(v: i128) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit::new(v)),
            Span::dummy(),
        )),
        Span::dummy(),
    )
}

fn binary(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Heap::new(lhs),
            right: Heap::new(rhs),
        },
        Span::dummy(),
    )
}

/// `n * 2 >= 0` — a goal in which `n` genuinely occurs, so a missing
/// substitution is visible in the subgoal's structure rather than only
/// in its rendering.
fn goal_over_n() -> ProofGoal {
    let body = binary(
        BinOp::Ge,
        binary(BinOp::Mul, ident_expr("n"), int_expr(2)),
        int_expr(0),
    );
    let mut hyps = List::new();
    hyps.push(binary(BinOp::Ge, ident_expr("n"), int_expr(0)));
    ProofGoal::with_hypotheses(body, hyps)
}

fn label_of(g: &ProofGoal) -> String {
    match &g.label {
        Maybe::Some(l) => l.as_str().to_string(),
        Maybe::None => String::new(),
    }
}

/// The left operand of a `Binary` goal, for structural assertions.
fn goal_lhs(g: &ProofGoal) -> Expr {
    match &g.goal.kind {
        ExprKind::Binary { left, .. } => (**left).clone(),
        other => panic!("expected a binary goal, got {:?}", other),
    }
}

#[test]
fn integer_variable_yields_base_and_both_signed_steps() {
    let mut engine = ProofSearchEngine::new();
    engine.register_integer_variable(Text::from("n"));

    let subgoals = engine
        .execute_tactic(
            &ProofTactic::Induction {
                var: Text::from("n"),
            },
            &goal_over_n(),
        )
        .expect("induction on a registered integer variable must run");

    assert_eq!(
        subgoals.len(),
        3,
        "signed induction needs base, upward and downward obligations; got labels {:?}",
        subgoals.iter().map(label_of).collect::<Vec<_>>()
    );
    assert_eq!(label_of(&subgoals[0]), "base_case_0");
    assert_eq!(label_of(&subgoals[1]), "inductive_case_succ");
    assert_eq!(label_of(&subgoals[2]), "inductive_case_pred");
}

#[test]
fn base_case_substitutes_zero_into_goal_and_hypotheses() {
    let mut engine = ProofSearchEngine::new();
    engine.register_integer_variable(Text::from("n"));
    let subgoals = engine
        .execute_tactic(
            &ProofTactic::Induction {
                var: Text::from("n"),
            },
            &goal_over_n(),
        )
        .expect("induction runs");

    // Goal `n * 2 >= 0` becomes `0 * 2 >= 0`: the multiplication's own
    // left operand must no longer be a Path.
    let mul = goal_lhs(&subgoals[0]);
    let mul_lhs = match &mul.kind {
        ExprKind::Binary { left, .. } => (**left).clone(),
        other => panic!("expected `n * 2`, got {:?}", other),
    };
    assert!(
        matches!(mul_lhs.kind, ExprKind::Literal(_)),
        "base case must substitute n := 0 in the goal, found {:?}",
        mul_lhs.kind
    );

    // The precondition is instantiated too.  Leaving `n >= 0` standing
    // while proving `P(0)` would let a precondition that does NOT hold
    // at the base be used to discharge it.
    for h in subgoals[0].hypotheses.iter() {
        if let ExprKind::Binary { left, .. } = &h.kind {
            assert!(
                !matches!(left.kind, ExprKind::Path(_)),
                "hypothesis kept an un-instantiated `n`: {:?}",
                h.kind
            );
        }
    }
}

#[test]
fn step_assumes_the_implication_not_the_bare_precondition() {
    let mut engine = ProofSearchEngine::new();
    engine.register_integer_variable(Text::from("n"));
    let subgoals = engine
        .execute_tactic(
            &ProofTactic::Induction {
                var: Text::from("n"),
            },
            &goal_over_n(),
        )
        .expect("induction runs");

    let step = &subgoals[1];

    // The induction hypothesis is `H(k) => P(k)`, an implication.
    // Assuming `P(k)` with `H(k)` admitted unconditionally is the
    // unsound shortcut this asserts against.
    let has_imply = step.hypotheses.iter().any(|h| {
        matches!(
            &h.kind,
            ExprKind::Binary {
                op: BinOp::Imply,
                ..
            }
        )
    });
    assert!(
        has_imply,
        "the step must assume the implication H(k) => P(k); hypotheses: {:?}",
        step.hypotheses
            .iter()
            .map(|h| format!("{:?}", h.kind))
            .collect::<Vec<_>>()
    );

    // And the goal advanced to `k + 1`.
    let mul = goal_lhs(step);
    let mul_lhs = match &mul.kind {
        ExprKind::Binary { left, .. } => (**left).clone(),
        other => panic!("expected `(n+1) * 2`, got {:?}", other),
    };
    assert!(
        matches!(
            mul_lhs.kind,
            ExprKind::Binary {
                op: BinOp::Add,
                ..
            }
        ),
        "step goal must be P(k + 1), found {:?}",
        mul_lhs.kind
    );
}

/// Negative control: without the registration the tactic must still
/// fail, and fail for the REASON it actually has — the fix must not
/// turn every un-inferrable variable into an integer.
#[test]
fn unregistered_variable_still_fails_with_the_real_reason() {
    let mut engine = ProofSearchEngine::new();
    let err = engine
        .execute_tactic(
            &ProofTactic::Induction {
                var: Text::from("n"),
            },
            &goal_over_n(),
        )
        .expect_err("an unregistered variable has no induction principle");
    let msg = format!("{}", err);
    assert!(
        msg.contains("cannot infer type"),
        "the failure must name type inference, not something else: {}",
        msg
    );
}

#[test]
fn names_of_constructors_are_not_a_typed_eliminator() {
    let mut engine = ProofSearchEngine::new();
    engine.register_variant_type("Color".into(), vec!["Red".into(), "Green".into(), "Blue".into()]);
    let goal = ProofGoal::new(binary(BinOp::Eq, ident_expr("c"), ident_expr("Red")));
    let error = engine.execute_tactic(&ProofTactic::Induction { var: "c".into() }, &goal)
        .expect_err("constructor names alone cannot justify elimination");
    assert!(error.to_string().contains("checked typed eliminator"), "{error}");
}

#[test]
fn nonnegative_induction_cannot_claim_all_integers_are_nonnegative() {
    let mut engine = ProofSearchEngine::new();
    engine.register_integer_variable("n".into());
    let goal = ProofGoal::new(binary(BinOp::Ge, ident_expr("n"), int_expr(0)));
    let obligations = engine.execute_tactic(&ProofTactic::Induction { var: "n".into() }, &goal).unwrap();
    for obligation in obligations.iter().take(2) {
        assert!(engine.execute_tactic(&ProofTactic::Auto, obligation).unwrap().is_empty());
    }
    // k = 0 falsifies the downward obligation: P(0) does not imply P(-1).
    let down = &obligations[2];
    assert!(engine.execute_tactic(&ProofTactic::Auto, down).is_err());
    let ExprKind::Binary { left, .. } = &down.goal.kind else { panic!("comparison") };
    assert!(matches!(left.kind, ExprKind::Binary { op: BinOp::Sub, .. }));
    assert!(down.hypotheses.iter().any(|h| matches!(h.kind, ExprKind::Binary { op: BinOp::Le, .. })));
}

#[test]
fn domain_restricted_and_signed_neighboring_proofs_remain_valid() {
    let mut engine = ProofSearchEngine::new();
    engine.register_integer_variable("n".into());
    for goal in [goal_over_n(), ProofGoal::new(binary(
        BinOp::Ge, binary(BinOp::Mul, ident_expr("n"), ident_expr("n")), int_expr(0),
    ))] {
        let obligations = engine.execute_tactic(&ProofTactic::Induction { var: "n".into() }, &goal).unwrap();
        assert_eq!(obligations.len(), 3);
        for obligation in &obligations {
            assert!(engine.execute_tactic(&ProofTactic::Auto, obligation).unwrap().is_empty(), "{obligation:?}");
        }
    }
}

#[test]
fn methods_cannot_discard_strong_or_well_founded_evidence() {
    let mut engine = ProofSearchEngine::new();
    engine.register_integer_variable("n".into());
    let true_goal = ProofGoal::new(Expr::new(ExprKind::Literal(Literal::bool(true, Span::dummy())), Span::dummy()));
    for tactic in [
        ProofTactic::StrongInduction { var: "n".into() },
        ProofTactic::WellFoundedInduction { var: "n".into(), relation: "arbitrary_relation".into() },
    ] {
        let error = engine.execute_tactic(&tactic, &true_goal).expect_err("no method substitution");
        assert!(error.to_string().contains("evidence"), "{error}");
    }
}
