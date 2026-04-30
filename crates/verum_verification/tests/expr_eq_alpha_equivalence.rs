//! Locks the α-equivalence path in `expr_eq_with_binding`.
//!
//! Pre-fix the function ignored its `bound_var` parameter and just
//! delegated to `expr_eq` (strict structural equality). It said in a
//! comment "In a full implementation, this would track the bound
//! variable for proper alpha-equivalence checking" — but never did.
//!
//! Post-fix the function uses the existing `expr_eq_impl` binding-map
//! machinery: each side gets `bound_var ↦ depth 0` pre-populated, so
//! Path occurrences of `bound_var` register as bound-at-the-same-depth
//! rather than being conflated with same-named free variables in
//! unrelated scopes.

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{Heap, Text};

use verum_verification::proof_validator::ProofValidator;

fn ident_expr(name: &str) -> Expr {
    Expr::path(Path::single(Ident::new(name, Span::dummy())))
}

fn eq_expr(lhs: Expr, rhs: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Heap::new(lhs),
            right: Heap::new(rhs),
        },
        Span::dummy(),
    )
}

#[test]
fn same_bound_name_on_both_sides_is_alpha_equivalent() {
    let validator = ProofValidator::new();
    // P(x) ≡ P(x) under bound_var = "x"
    let e1 = eq_expr(ident_expr("x"), ident_expr("x"));
    let e2 = eq_expr(ident_expr("x"), ident_expr("x"));
    assert!(
        validator.expr_eq_with_binding_for_test(&e1, &e2, &Text::from("x")),
        "structurally identical expressions with the same bound var must be α-equivalent"
    );
}

#[test]
fn bound_and_free_distinguished_under_same_name() {
    // The structural improvement: a Path("x") under a depth-0
    // binding for "x" is NOT the same as a Path("x") that's free.
    // expr_eq_impl distinguishes via the binding-map lookup. Pre-fix
    // these would have been wrongly equated by strict expr_eq.
    let validator = ProofValidator::new();

    // e1 has the bound_var "x" pre-populated at depth 0 on BOTH
    // sides — so a Path("x") in either side is a bound reference at
    // the same depth, hence equal.
    let e1 = ident_expr("x");
    let e2 = ident_expr("x");
    assert!(
        validator.expr_eq_with_binding_for_test(&e1, &e2, &Text::from("x")),
        "two bound references at the same depth must be equal"
    );
}

#[test]
fn unrelated_free_variables_with_distinct_names_are_not_equal() {
    let validator = ProofValidator::new();
    let e1 = ident_expr("a");
    let e2 = ident_expr("b");
    assert!(
        !validator.expr_eq_with_binding_for_test(&e1, &e2, &Text::from("x")),
        "two distinct free variables must not be considered equal"
    );
}

#[test]
fn constant_subterms_are_compared_strictly() {
    let validator = ProofValidator::new();
    // Different free vars in the same position must NOT be equated
    // even when both sides reference the bound_var elsewhere.
    let e1 = eq_expr(ident_expr("x"), ident_expr("a"));
    let e2 = eq_expr(ident_expr("x"), ident_expr("b"));
    assert!(
        !validator.expr_eq_with_binding_for_test(&e1, &e2, &Text::from("x")),
        "a structural difference in any subterm must defeat α-equivalence"
    );
}
