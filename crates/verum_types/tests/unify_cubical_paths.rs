//! Unification tests for cubical type constructors (PathType, Partial,
//! Interval) — verifies that the cubical normalizer is exercised on the
//! unification hot path.

use verum_ast::ty::{Ident, Path};
use verum_ast::Span;
use verum_common::{List, Text};
use verum_types::cubical::{CubicalTerm, DimVar, IntervalEndpoint};
use verum_types::ty::Type;
use verum_types::unify::Unifier;

// ---- Helpers -------------------------------------------------------------

fn boxed_val(s: &str) -> Box<CubicalTerm> {
    Box::new(CubicalTerm::Value(Text::from(s)))
}

fn boxed_refl(s: &str) -> Box<CubicalTerm> {
    Box::new(CubicalTerm::Refl(Box::new(CubicalTerm::Value(Text::from(s)))))
}

fn boxed_transport_refl(line: &str, val: &str) -> Box<CubicalTerm> {
    Box::new(CubicalTerm::Transport {
        line: Box::new(CubicalTerm::Refl(Box::new(CubicalTerm::Value(
            Text::from(line),
        )))),
        value: Box::new(CubicalTerm::Value(Text::from(val))),
    })
}

fn path(space: Type, left: Box<CubicalTerm>, right: Box<CubicalTerm>) -> Type {
    Type::PathType {
        space: Box::new(space),
        left,
        right,
    }
}

fn named(name: &str) -> Type {
    let ident = Ident::new(name, Span::dummy());
    Type::Named {
        path: Path::single(ident),
        args: List::new(),
    }
}

fn bool_ty() -> Type {
    named("Bool")
}

fn int_ty() -> Type {
    named("Int")
}

fn span() -> Span {
    Span::dummy()
}

// ---- PathType ------------------------------------------------------------

#[test]
fn pathtype_unifies_identical() {
    let t1 = path(bool_ty(), boxed_val("a"), boxed_val("b"));
    let t2 = path(bool_ty(), boxed_val("a"), boxed_val("b"));
    let mut u = Unifier::new();
    assert!(u.unify(&t1, &t2, span()).is_ok());
}

#[test]
fn pathtype_unifies_up_to_cubical_normalization() {
    // `transport (refl A) x ≡ x`, so these path types should unify.
    let lhs = path(
        bool_ty(),
        boxed_val("x"),
        boxed_transport_refl("A", "x"),
    );
    let rhs = path(bool_ty(), boxed_val("x"), boxed_val("x"));
    let mut u = Unifier::new();
    assert!(
        u.unify(&lhs, &rhs, span()).is_ok(),
        "PathType should unify when right endpoints are cubically equal"
    );
}

#[test]
fn pathtype_rejects_different_endpoints() {
    let t1 = path(bool_ty(), boxed_val("a"), boxed_val("b"));
    let t2 = path(bool_ty(), boxed_val("a"), boxed_val("c"));
    let mut u = Unifier::new();
    assert!(
        u.unify(&t1, &t2, span()).is_err(),
        "distinct right endpoints must fail unification"
    );
}

#[test]
fn pathtype_rejects_different_spaces() {
    let t1 = path(bool_ty(), boxed_val("a"), boxed_val("b"));
    let t2 = path(int_ty(), boxed_val("a"), boxed_val("b"));
    let mut u = Unifier::new();
    assert!(
        u.unify(&t1, &t2, span()).is_err(),
        "mismatched carrier spaces must fail"
    );
}

#[test]
fn pathtype_unifies_refl_with_itself() {
    let t1 = path(bool_ty(), boxed_refl("x"), boxed_refl("x"));
    let t2 = path(bool_ty(), boxed_refl("x"), boxed_refl("x"));
    let mut u = Unifier::new();
    assert!(u.unify(&t1, &t2, span()).is_ok());
}

// ---- Interval ------------------------------------------------------------

#[test]
fn interval_unifies_trivially() {
    let mut u = Unifier::new();
    assert!(u.unify(&Type::Interval, &Type::Interval, span()).is_ok());
}

#[test]
fn interval_does_not_unify_with_bool() {
    let mut u = Unifier::new();
    assert!(u.unify(&Type::Interval, &bool_ty(), span()).is_err());
}

// ---- Partial -------------------------------------------------------------

fn dim_face(name: &str) -> Box<CubicalTerm> {
    Box::new(CubicalTerm::DimVar(DimVar::new(name)))
}

fn i0_face() -> Box<CubicalTerm> {
    Box::new(CubicalTerm::Endpoint(IntervalEndpoint::I0))
}

fn partial(element: Type, face: Box<CubicalTerm>) -> Type {
    Type::Partial {
        element_type: Box::new(element),
        face,
    }
}

#[test]
fn partial_unifies_same_face() {
    let t1 = partial(bool_ty(), dim_face("i"));
    let t2 = partial(bool_ty(), dim_face("i"));
    let mut u = Unifier::new();
    assert!(u.unify(&t1, &t2, span()).is_ok());
}

#[test]
fn partial_rejects_divergent_face() {
    let t1 = partial(bool_ty(), dim_face("i"));
    let t2 = partial(bool_ty(), i0_face());
    let mut u = Unifier::new();
    assert!(u.unify(&t1, &t2, span()).is_err());
}

#[test]
fn partial_rejects_mismatched_element_type() {
    let t1 = partial(bool_ty(), dim_face("i"));
    let t2 = partial(int_ty(), dim_face("i"));
    let mut u = Unifier::new();
    assert!(u.unify(&t1, &t2, span()).is_err());
}
