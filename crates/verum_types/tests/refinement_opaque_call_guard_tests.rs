//! A refinement predicate that CALLS a function must not be hard-refused
//! on the strength of the solver's verdict alone (A113).
//!
//! The checker has a fast path that hard-errors when the SMT layer answers
//! `Invalid` AND the checked expression is a literal AND the predicate was
//! written by a human. That branch exists for a real case — a quantifier
//! false for every value is decided, and the syntactic evaluator cannot
//! reproduce it — but its premise is that the predicate is fully
//! INTERPRETED.
//!
//! A free function call is not. The solver sees an uninterpreted symbol,
//! cannot unfold it, cannot prove the verification condition, and answers
//! `Invalid` — meaning "not proven", not "false". Read as "violated", it
//! refuses correct programs and blames the constraint. Measured 2026-09-12
//! at both polarities, with the same arithmetic written two ways:
//!
//! ```text
//!     Int{it * 2 >= 10}      v: 20  clean     v: 1  error<E500>
//!     Int{twice(it) >= 10}   v: 20  E500      v: 1  error<E500>
//! ```
//!
//! The second row refuses `40 >= 10`. So did `Int{ident(it) >= 0}` and
//! `Int{twice(it) >= -1000}`, which no value can violate.
//!
//! These tests pin the DETECTOR that separates the two cases. The
//! end-to-end behaviour is pinned by the spec suite; what can go wrong
//! here and nowhere else is the detector quietly answering `false` for a
//! shape it does not walk into — a nested call inside a comparison, inside
//! a `!`, inside parentheses — which would restore the false rejection
//! without failing anything.

use verum_ast::{expr::*, literal::Literal, span::Span, ty::Ident};
use verum_common::List;
use verum_types::refinement::RefinementChecker;

fn it_ident(span: Span) -> Expr {
    Expr::ident(Ident::new("it", span))
}

fn int_lit(n: i128, span: Span) -> Expr {
    Expr::literal(Literal::int(n, span))
}

fn call_of(name: &str, args: Vec<Expr>, span: Span) -> Expr {
    Expr::new(
        ExprKind::Call {
            func: Box::new(Expr::ident(Ident::new(name, span))),
            type_args: List::new(),
            args: args.into(),
        },
        span,
    )
}

fn binary(op: BinOp, left: Expr, right: Expr, span: Span) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        span,
    )
}

#[test]
fn a_bare_call_is_opaque() {
    let s = Span::dummy();
    let pred = call_of("twice", vec![it_ident(s)], s);
    assert!(RefinementChecker::predicate_calls_an_opaque_function(&pred));
}

#[test]
fn a_call_under_a_comparison_is_opaque() {
    // The shape that was actually measured: `twice(it) >= 10`. A detector
    // that only looked at the predicate's own node would answer `false`
    // here and leave the false rejection in place.
    let s = Span::dummy();
    let pred = binary(
        BinOp::Ge,
        call_of("twice", vec![it_ident(s)], s),
        int_lit(10, s),
        s,
    );
    assert!(RefinementChecker::predicate_calls_an_opaque_function(&pred));
}

#[test]
fn a_call_on_the_right_hand_side_is_opaque() {
    // `it >= ten()` refused just as `twice(it) >= 10` did, so the walk
    // must not stop at the left operand.
    let s = Span::dummy();
    let pred = binary(BinOp::Ge, it_ident(s), call_of("ten", vec![], s), s);
    assert!(RefinementChecker::predicate_calls_an_opaque_function(&pred));
}

#[test]
fn a_call_nested_two_deep_is_opaque() {
    let s = Span::dummy();
    let inner = binary(
        BinOp::Ge,
        call_of("twice", vec![it_ident(s)], s),
        int_lit(10, s),
        s,
    );
    let pred = Expr::new(
        ExprKind::Unary {
            op: UnOp::Not,
            expr: Box::new(Expr::new(ExprKind::Paren(Box::new(inner)), s)),
        },
        s,
    );
    assert!(RefinementChecker::predicate_calls_an_opaque_function(&pred));
}

#[test]
fn plain_arithmetic_is_not_opaque() {
    // THE CONTROL, and it is the whole point: `it * 2 >= 10` decides
    // correctly at both polarities today, so the guard must leave it
    // alone. A detector that answered `true` for everything would make
    // this file pass and would silence T0967's quantifier case.
    let s = Span::dummy();
    let pred = binary(
        BinOp::Ge,
        binary(BinOp::Mul, it_ident(s), int_lit(2, s), s),
        int_lit(10, s),
        s,
    );
    assert!(!RefinementChecker::predicate_calls_an_opaque_function(&pred));
}

#[test]
fn a_method_call_is_not_opaque() {
    // `Text{it.len() > 0}` is clean on "abc" and refuses on "" — the
    // syntactic evaluator folds `len`, so the solver never gets the last
    // word and the guard must not intervene.
    let s = Span::dummy();
    let recv = it_ident(s);
    let meth = Expr::new(
        ExprKind::MethodCall {
            receiver: Box::new(recv),
            method: Ident::new("len", s),
            type_args: List::new(),
            args: List::new(),
        },
        s,
    );
    let pred = binary(BinOp::Gt, meth, int_lit(0, s), s);
    assert!(!RefinementChecker::predicate_calls_an_opaque_function(&pred));
}

#[test]
fn a_bare_name_as_the_whole_predicate_is_opaque() {
    // `Int{is_positive}` and `Int where is_positive` both parse to a lone
    // `Path`. They refused a SATISFYING value until 2026-09-12 for exactly
    // the reason the call form did, and the call walk could not see them.
    let s = Span::dummy();
    let pred = Expr::ident(Ident::new("is_positive", s));
    assert!(RefinementChecker::predicate_calls_an_opaque_function(&pred));
}

#[test]
fn a_bare_name_in_parentheses_is_still_opaque() {
    let s = Span::dummy();
    let pred = Expr::new(
        ExprKind::Paren(Box::new(Expr::ident(Ident::new("is_positive", s)))),
        s,
    );
    assert!(RefinementChecker::predicate_calls_an_opaque_function(&pred));
}

#[test]
fn a_name_INSIDE_a_comparison_is_not_opaque() {
    // THE CONTROL FOR THE POSITIONAL RULE. `it` is a name, and it appears
    // in every predicate; a test on "does a name occur" rather than "is
    // the ROOT a name" would make `it >= 10` undecidable and undo the
    // decided cases entirely.
    let s = Span::dummy();
    let pred = binary(BinOp::Ge, it_ident(s), int_lit(10, s), s);
    assert!(!RefinementChecker::predicate_calls_an_opaque_function(&pred));
}

#[test]
fn a_bare_comparison_is_not_opaque() {
    let s = Span::dummy();
    let pred = binary(BinOp::Ge, it_ident(s), int_lit(10, s), s);
    assert!(!RefinementChecker::predicate_calls_an_opaque_function(&pred));
}
