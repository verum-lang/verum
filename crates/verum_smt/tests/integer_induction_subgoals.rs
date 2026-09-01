//! Regression: `induction n` on an INTEGER variable.
//!
//! Structural induction needs a variant type with constructors.  An
//! `Int` has neither, so `infer_variable_type` refused it — correctly —
//! and `try_induction` then failed outright.  The result was that the
//! single most common induction in mathematics could not be written:
//!
//!     theorem t(n: Int) requires n >= 0 ensures P(n) {
//!         proof by induction on n {
//!             case 0 => trivial
//!             case succ(k) => { have ih: P(k); conclude by auto }
//!         }
//!     }
//!
//! reported "Proof method 'induction' failed — verify the induction
//! variable or case split is correct", advice naming a cause the author
//! had no way to act on: the variable was fine, the principle was
//! missing.
//!
//! Post-fix an integer-sorted variable (registered by the caller from
//! its DECLARED type, never guessed from its name) takes the Peano
//! path, whose two subgoals line up with the `case 0` / `case succ(k)`
//! arms the proof text already writes.

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
fn integer_variable_yields_base_and_step() {
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
        2,
        "Peano induction produces exactly a base case and a step; got labels {:?}",
        subgoals.iter().map(label_of).collect::<Vec<_>>()
    );
    assert_eq!(label_of(&subgoals[0]), "base_case_0");
    assert_eq!(label_of(&subgoals[1]), "inductive_case_succ");
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

/// Control: the structural path is untouched.  A variable resolvable
/// through `variant_map` still splits on constructors.
#[test]
fn variant_variable_still_takes_the_structural_path() {
    let mut engine = ProofSearchEngine::new();
    engine.register_variant_type(
        Text::from("Color"),
        vec![Text::from("Red"), Text::from("Green"), Text::from("Blue")],
    );

    let goal = ProofGoal::with_hypotheses(
        binary(BinOp::Eq, ident_expr("c"), ident_expr("Red")),
        List::new(),
    );
    let subgoals = engine
        .execute_tactic(
            &ProofTactic::Induction {
                var: Text::from("c"),
            },
            &goal,
        )
        .expect("structural induction still runs");

    assert_eq!(
        subgoals.len(),
        3,
        "one subgoal per constructor; got {:?}",
        subgoals.iter().map(label_of).collect::<Vec<_>>()
    );
    assert!(
        subgoals.iter().all(|g| label_of(g).contains("case")),
        "structural labels must not have been replaced by the Peano ones: {:?}",
        subgoals.iter().map(label_of).collect::<Vec<_>>()
    );
}
