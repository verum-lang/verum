//! Soundness regression: `recheck_with_smt` must translate hypothesis
//! propositions, not bind them as opaque fresh booleans.
//!
//! Pre-fix the loop bound a `Bool::new_const(name)` for each
//! hypothesis and discarded the actual `prop` expression. Z3 thus saw
//! every hypothesis as a vacuous `h0 := true` regardless of whether
//! the proposition was `x > 5`, `is_sorted(xs)`, or any other concrete
//! claim. Soundness consequence: a re-check that should have found a
//! counterexample (because the assumed hypotheses don't actually
//! entail the conclusion) silently passed.
//!
//! Post-fix the loop translates `prop` to a Z3 Bool and asserts it.
//! When the prop carries the actual claim `x = 0`, the rechecker now
//! sees that constraint and correctly disproves `x > 0`.

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::Literal;
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_ast::{IntLit, LiteralKind};
use verum_common::Heap;

use verum_verification::proof_validator::ProofValidator;

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
fn recheck_uses_hypothesis_proposition_not_just_name() {
    // Hypothesis: x = 0
    // Formula being rechecked: x > 0
    // Pre-fix the rechecker would silently succeed because it would
    // see only `h0 := true` instead of `x = 0`. Z3 would then accept
    // any formula consistent with that vacuous context, including
    // `x > 0` (which is satisfiable when x is a free variable).
    let mut validator = ProofValidator::new();
    let x_eq_zero = binary(BinOp::Eq, ident_expr("x"), int_lit(0));
    validator.register_hypothesis("h0", x_eq_zero);

    let x_gt_zero = binary(BinOp::Gt, ident_expr("x"), int_lit(0));
    let result = validator.recheck_with_smt_for_test("z3", &x_gt_zero);

    assert!(
        result.is_err(),
        "rechecker must reject `x > 0` under hypothesis `x = 0` — pre-fix \
         this silently passed because only the hypothesis NAME (not its \
         proposition) was asserted to Z3"
    );
}

#[test]
fn recheck_accepts_formulas_entailed_by_hypothesis() {
    // Negative control: when the hypothesis actually entails the
    // formula, the rechecker should accept. Pre-fix this also passed
    // (vacuously, for the wrong reason). Post-fix the test pins that
    // we still accept genuinely-entailed formulas.
    //
    // Hypothesis: x = 5, formula: x > 0 — entailed.
    let mut validator = ProofValidator::new();
    let x_eq_five = binary(BinOp::Eq, ident_expr("x"), int_lit(5));
    validator.register_hypothesis("h0", x_eq_five);

    let x_gt_zero = binary(BinOp::Gt, ident_expr("x"), int_lit(0));
    let result = validator.recheck_with_smt_for_test("z3", &x_gt_zero);

    assert!(
        result.is_ok(),
        "rechecker must accept `x > 0` under hypothesis `x = 5` — \
         the hypothesis must be translated and asserted, not skipped: {:?}",
        result
    );
}
