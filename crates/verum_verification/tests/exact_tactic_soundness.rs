//! Soundness regression: `apply_exact(term)` must verify the term
//! actually discharges the goal — pre-fix it just called
//! `prove_current_goal()` unconditionally and accepted ANY term as
//! a proof of ANY goal:
//!
//! ```ignore
//! fn apply_exact(&mut self, _proof: &Heap<Expr>) -> TacticResult<()> {
//!     // For now, just mark as proven
//!     self.state.prove_current_goal()?;
//!     Ok(())
//! }
//! ```
//!
//! Post-fix the term must be one of:
//! - A Path naming a hypothesis whose proposition equals the goal.
//! - The goal expression itself (covers literal `true`, reflexive
//!   equalities, and verbatim re-statement).

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::Literal;
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_ast::{IntLit, LiteralKind};
use verum_common::Heap;

use verum_ast::decl::TacticExpr;
use verum_verification::tactic_evaluation::{Hypothesis, TacticEvaluator};

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

#[test]
fn exact_with_matching_hypothesis_succeeds() {
    // Goal: x > 0. Hypothesis h0: x > 0. exact h0 succeeds.
    let goal = binary(BinOp::Gt, ident_expr("x"), int_lit(0));
    let mut evaluator = TacticEvaluator::with_goal(goal.clone());
    {
        let cur = evaluator.state_mut().current_goal_mut().unwrap();
        cur.add_hypothesis(Hypothesis::new(verum_common::Text::from("h0"), goal));
    }

    let proof = Heap::new(ident_expr("h0"));
    let result = evaluator.apply_tactic(&TacticExpr::Exact(proof));
    assert!(
        result.is_ok(),
        "exact h0 with matching hypothesis must succeed: {:?}",
        result
    );
}

#[test]
fn exact_with_mismatched_hypothesis_is_rejected() {
    // Goal: x > 0. Hypothesis h0: x < 0. exact h0 must fail.
    let goal = binary(BinOp::Gt, ident_expr("x"), int_lit(0));
    let other = binary(BinOp::Lt, ident_expr("x"), int_lit(0));
    let mut evaluator = TacticEvaluator::with_goal(goal);
    {
        let cur = evaluator.state_mut().current_goal_mut().unwrap();
        cur.add_hypothesis(Hypothesis::new(verum_common::Text::from("h0"), other));
    }

    let proof = Heap::new(ident_expr("h0"));
    let result = evaluator.apply_tactic(&TacticExpr::Exact(proof));
    assert!(
        result.is_err(),
        "exact h0 must fail when h0's content doesn't match the goal — \
         pre-fix this silently passed"
    );
}

#[test]
fn exact_with_arbitrary_unrelated_term_is_rejected() {
    // Goal: x > 0. Proof: literal 42. Pre-fix: silently accepted.
    // Post-fix: rejected because 42 is neither a hypothesis name nor
    // structurally equal to the goal.
    let goal = binary(BinOp::Gt, ident_expr("x"), int_lit(0));
    let mut evaluator = TacticEvaluator::with_goal(goal);

    let proof = Heap::new(int_lit(42));
    let result = evaluator.apply_tactic(&TacticExpr::Exact(proof));
    assert!(
        result.is_err(),
        "exact 42 must NOT discharge a non-trivially-true goal — \
         pre-fix this silently passed for ANY proof term"
    );
}

#[test]
fn exact_with_dangling_hypothesis_name_is_rejected() {
    // Goal: x > 0. Proof: path `h_made_up` not in hypothesis context.
    let goal = binary(BinOp::Gt, ident_expr("x"), int_lit(0));
    let mut evaluator = TacticEvaluator::with_goal(goal);

    let proof = Heap::new(ident_expr("h_made_up"));
    let result = evaluator.apply_tactic(&TacticExpr::Exact(proof));
    assert!(
        result.is_err(),
        "exact h_made_up must fail when h_made_up isn't in the hypothesis context"
    );
}

#[test]
fn exact_with_goal_verbatim_succeeds() {
    // Goal: 5 == 5 (a reflexive equality). Proof: 5 == 5 (same expr).
    // Structural equality between proof and goal accepts.
    let goal = binary(BinOp::Eq, int_lit(5), int_lit(5));
    let mut evaluator = TacticEvaluator::with_goal(goal.clone());

    let proof = Heap::new(goal);
    let result = evaluator.apply_tactic(&TacticExpr::Exact(proof));
    assert!(
        result.is_ok(),
        "exact <goal verbatim> must succeed for reflexive cases: {:?}",
        result
    );
}
