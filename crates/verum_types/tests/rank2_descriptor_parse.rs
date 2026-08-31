//! The baked spelling of a rank-2 function type must come back QUANTIFIED
//! (T0997).
//!
//! `fn<R>(Reducer<B, R>) -> Reducer<A, R>` and
//! `fn(Reducer<B, R>) -> Reducer<A, R>` are different types: the first says
//! the CALLEE works for every R, the second says the caller picks one. The
//! decoder had no `fn<` arm, so the spelling fell to the generic-
//! instantiation arm and landed as a rigid
//! `Type::Named { "fn<R>(…) -> Reducer", args: [A, R] }` — a name that
//! unifies with nothing.
//!
//! These are the decoder's own tests. The end-to-end symptom (a second
//! instantiation with a different element type) needs a rebaked stdlib and
//! is pinned separately under `vcs/specs/L3-extended/dependent/`.

use verum_types::infer::parse_descriptor_type_string;
use verum_types::ty::Type;

#[test]
fn a_rank2_spelling_parses_as_a_quantified_type() {
    let ty = parse_descriptor_type_string("fn<R>(Reducer<B, R>) -> Reducer<A, R>");
    match ty {
        Type::Forall { vars, body } => {
            assert_eq!(vars.len(), 1, "one binder was written, so one must be bound");
            assert!(
                matches!(*body, Type::Function { .. }),
                "the body under the quantifier is the function type itself, got {body:?}"
            );
        }
        other => panic!("expected a quantified type, got {other:?}"),
    }
}

#[test]
fn each_parse_binds_a_fresh_variable() {
    // The whole point of the quantifier: two use sites must not share R.
    // Sharing it is exactly the symptom — the first instantiation fixes the
    // element type and the second is measured against it.
    let a = parse_descriptor_type_string("fn<R>(Reducer<B, R>) -> Reducer<A, R>");
    let b = parse_descriptor_type_string("fn<R>(Reducer<B, R>) -> Reducer<A, R>");
    let (va, vb) = match (a, b) {
        (Type::Forall { vars: va, .. }, Type::Forall { vars: vb, .. }) => (va, vb),
        other => panic!("expected two quantified types, got {other:?}"),
    };
    assert_ne!(
        va[0], vb[0],
        "two parses must bind DISTINCT variables, or every use site shares one R"
    );
}

#[test]
fn a_bound_on_the_binder_does_not_end_the_binder_list() {
    // The `>` inside `Reducer<A, B>` must not be read as the end of the
    // binder list — a depth-unaware scan stops at the first one.
    let ty = parse_descriptor_type_string("fn<R: Reducer<A, B>>(Reducer<B, R>) -> R");
    match ty {
        Type::Forall { vars, .. } => assert_eq!(vars.len(), 1),
        other => panic!("expected a quantified type, got {other:?}"),
    }
}

#[test]
fn two_binders_bind_two_variables() {
    let ty = parse_descriptor_type_string("fn<R, Q>(Reducer<B, R>) -> Reducer<Q, R>");
    match ty {
        Type::Forall { vars, .. } => assert_eq!(vars.len(), 2),
        other => panic!("expected a quantified type, got {other:?}"),
    }
}

#[test]
fn a_rank1_function_spelling_is_untouched() {
    // The control. The new arm sits BEFORE the `fn(` arm, so an ordinary
    // function type must still decode as one — a rank-1 type wrongly
    // quantified would let a caller-chosen parameter escape.
    let ty = parse_descriptor_type_string("fn(Reducer<B, R>) -> Reducer<A, R>");
    assert!(
        matches!(ty, Type::Function { .. }),
        "an ordinary function spelling must not become quantified, got {ty:?}"
    );
}
