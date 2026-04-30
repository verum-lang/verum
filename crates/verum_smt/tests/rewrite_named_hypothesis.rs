//! Regression: `try_rewrite` must use the user-named hypothesis, not
//! the first equality it finds.
//!
//! Pre-fix the function ignored its `hypothesis: &Text` parameter and
//! scanned `goal.hypotheses` for the FIRST equality, then rewrote
//! using that. Result: `rewrite h2` on a goal where `h0` is also an
//! equality silently used `h0` and produced wrong substitutions.
//!
//! Post-fix the function resolves the hypothesis name through
//! `find_hypothesis_index` (already used by `cases_on` and `destruct`),
//! requires the resolved hypothesis to actually be an equality, and
//! errors out cleanly if not.

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::Literal;
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_ast::{IntLit, LiteralKind};
use verum_common::{Heap, List, Maybe, Text};

use verum_smt::proof_search::{ProofGoal, ProofSearchEngine, ProofTactic};

fn ident_expr(name: &str) -> Expr {
    Expr::path(Path::single(Ident::new(name, Span::dummy())))
}

fn int_lit(value: i64) -> Expr {
    Expr::literal(Literal::new(
        LiteralKind::Int(IntLit::new(value as i128)),
        Span::dummy(),
    ))
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

fn eq(lhs: Expr, rhs: Expr) -> Expr {
    binary(BinOp::Eq, lhs, rhs)
}

#[test]
fn rewrite_uses_named_hypothesis_not_first_equality() {
    // h0: a = b   (first equality, would be picked pre-fix)
    // h1: c = d   (the one the user wants)
    // goal: c + 1 > 0
    // rewrite h1 → d + 1 > 0
    // Pre-fix would have used h0 which doesn't match `c`, returning
    // a "could not match" error — but if h0 happened to match, it
    // would have rewritten with the WRONG equality.
    let mut engine = ProofSearchEngine::new();

    let goal_expr = binary(
        BinOp::Gt,
        binary(BinOp::Add, ident_expr("c"), int_lit(1)),
        int_lit(0),
    );
    let mut hyps = List::new();
    hyps.push(eq(ident_expr("a"), ident_expr("b")));
    hyps.push(eq(ident_expr("c"), ident_expr("d")));
    let goal = ProofGoal::with_hypotheses(goal_expr, hyps);

    let result = engine.execute_tactic(
        &ProofTactic::Rewrite {
            hypothesis: Text::from("h1"),
            reverse: false,
        },
        &goal,
    );
    assert!(
        result.is_ok(),
        "rewrite h1 must succeed using c=d: {:?}",
        result.err()
    );

    let subgoals = result.unwrap();
    assert_eq!(subgoals.len(), 1);
    let new_goal = &subgoals[0];

    // The new goal must contain `d` (not `c`), confirming h1 was used.
    let dump = format!("{:?}", new_goal.goal);
    assert!(
        dump.contains("\"d\""),
        "rewritten goal must reference `d` from h1's RHS. dump: {}",
        dump
    );
    assert!(
        !dump.contains("\"c\""),
        "rewritten goal must not still reference `c`. dump: {}",
        dump
    );
}

#[test]
fn rewrite_rejects_non_equality_hypothesis() {
    // Pre-fix: scan-for-first-equality would skip non-equality
    // hypotheses silently. Post-fix: explicitly named non-equality
    // is an error so the user knows their tactic is incorrect.
    let mut engine = ProofSearchEngine::new();

    let goal_expr = ident_expr("Q");
    let mut hyps = List::new();
    // h0 is NOT an equality.
    hyps.push(ident_expr("P"));
    let goal = ProofGoal::with_hypotheses(goal_expr, hyps);

    let result = engine.execute_tactic(
        &ProofTactic::Rewrite {
            hypothesis: Text::from("h0"),
            reverse: false,
        },
        &goal,
    );
    assert!(
        result.is_err(),
        "rewrite of a non-equality hypothesis must be rejected"
    );
    let msg = format!("{}", result.err().unwrap());
    assert!(
        msg.contains("not an equality"),
        "error must explain why rewrite failed. got: {}",
        msg
    );
}

#[test]
fn rewrite_rejects_dangling_hypothesis_name() {
    let mut engine = ProofSearchEngine::new();

    // Empty hypothesis context.
    let goal_expr = ident_expr("Q");
    let goal = ProofGoal::with_hypotheses(goal_expr, List::new());

    let result = engine.execute_tactic(
        &ProofTactic::Rewrite {
            hypothesis: Text::from("h99"),
            reverse: false,
        },
        &goal,
    );
    assert!(
        result.is_err(),
        "rewrite with dangling hypothesis name must fail"
    );
    let _ = Maybe::<()>::None; // silence unused-import lint
}
