//! The comparison behind the protocol-signature instrument (T1029).
//!
//! Conformance today checks the method NAME and never its signature, so an
//! implementation may promise one thing and be another. The instrument that
//! measures how often that happens in `core/` is only as good as the
//! comparison underneath it, and a comparison that reports everything is
//! worse than none: it hands the repair a number made of noise.
//!
//! So both directions are pinned here — what it MUST report, and what it
//! must deliberately decline to report.

use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::List;
use verum_types::infer::TypeChecker;
use verum_types::ty::Type;

fn func(params: Vec<Type>, ret: Type) -> Type {
    Type::Function {
        params: params.into_iter().collect::<List<Type>>(),
        return_type: Box::new(ret),
        contexts: None,
        type_params: List::new(),
        properties: None,
    }
}

fn named(n: &str) -> Type {
    Type::Named {
        path: Path::single(Ident::new(n, Span::default())),
        args: vec![].into(),
    }
}

#[test]
fn identical_signatures_do_not_disagree() {
    let a = func(vec![named("Int")], named("Int"));
    let b = func(vec![named("Int")], named("Int"));
    assert!(TypeChecker::signature_disagreement(&a, &b).is_none());
}

#[test]
fn a_different_parameter_type_is_reported_with_its_position() {
    // The measured defect: protocol says `greet(times: Int) -> Int`,
    // implementation says `greet(name: Text) -> Bool`.
    let proto = func(vec![named("Int")], named("Int"));
    let imp = func(vec![named("Text")], named("Bool"));
    let d = TypeChecker::signature_disagreement(&proto, &imp)
        .expect("a parameter type mismatch must be reported");
    assert!(d.contains("parameter 1"), "should name the position: {d}");
    assert!(d.contains("Int") && d.contains("Text"), "should name both sides: {d}");
}

#[test]
fn a_different_arity_is_reported_as_arity() {
    let proto = func(vec![named("Int"), named("Int")], named("Int"));
    let imp = func(vec![named("Int")], named("Int"));
    let d = TypeChecker::signature_disagreement(&proto, &imp)
        .expect("an arity mismatch must be reported");
    assert!(d.contains("arity"), "should say arity: {d}");
}

#[test]
fn a_different_return_type_is_reported_as_the_return_type() {
    let proto = func(vec![named("Int")], named("Int"));
    let imp = func(vec![named("Int")], named("Bool"));
    let d = TypeChecker::signature_disagreement(&proto, &imp)
        .expect("a return mismatch must be reported");
    assert!(d.contains("return type"), "should say return type: {d}");
}

#[test]
fn a_pair_mentioning_self_is_declined_on_purpose() {
    // Substituting `Self` by the implementing type is the real repair's
    // job. Until it exists, reporting these would fill the count with
    // pairs that are not defects — which is exactly what makes a
    // measurement useless for deciding reject-vs-warn.
    let proto = func(vec![named("Self")], named("Int"));
    let imp = func(vec![named("Point")], named("Int"));
    assert!(
        TypeChecker::signature_disagreement(&proto, &imp).is_none(),
        "a Self-mentioning pair must be declined, not reported"
    );
}

#[test]
fn a_non_function_pair_is_declined_rather_than_guessed_at() {
    let a = named("Int");
    let b = named("Text");
    assert!(TypeChecker::signature_disagreement(&a, &b).is_none());
}
