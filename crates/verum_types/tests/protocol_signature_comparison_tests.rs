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

#[test]
fn an_unresolved_side_is_declined_rather_than_counted() {
    // Measured on core/base: `Hasher::write` reported protocol `&[Byte]`
    // against implementation `&Unknown`. `Unknown` is a side that did not
    // resolve, not a different type, and reporting it inflates the count
    // with non-defects — the same error this whole task is about, aimed
    // the other way.
    let proto = func(vec![named("Byte")], Type::Unit);
    let imp = func(vec![Type::Unknown], Type::Unit);
    assert!(
        TypeChecker::signature_disagreement(&proto, &imp).is_none(),
        "an Unknown side must be declined"
    );
}

#[test]
fn an_inference_hole_is_declined_rather_than_counted() {
    // Measured on core/base: `Iterator::fold` reported protocol `_`
    // against implementation `B`. `_` is a hole, not a type.
    let proto = func(vec![named("_")], Type::Unit);
    let imp = func(vec![named("B")], Type::Unit);
    assert!(
        TypeChecker::signature_disagreement(&proto, &imp).is_none(),
        "an inference hole must be declined"
    );
}

#[test]
fn a_qualified_path_and_a_bare_one_are_the_same_type() {
    // Measured on core/base: `Hash::hash` reported protocol
    // `&mut core.base.protocols.Hasher` against implementation
    // `&mut Hasher` — one type, two spellings.
    let proto = func(vec![named("core.base.protocols.Hasher")], Type::Unit);
    let imp = func(vec![named("Hasher")], Type::Unit);
    assert!(
        TypeChecker::signature_disagreement(&proto, &imp).is_none(),
        "a module prefix is not a difference"
    );
}

#[test]
fn generic_parameters_differing_only_by_name_agree() {
    // Measured on core/base: `Into::into` reported protocol `T` against
    // implementation `U`.
    let proto = func(vec![named("Int")], named("T"));
    let imp = func(vec![named("Int")], named("U"));
    assert!(
        TypeChecker::signature_disagreement(&proto, &imp).is_none(),
        "type parameters are alpha-equivalent"
    );
}

#[test]
fn two_different_concrete_types_still_disagree_after_all_the_leniency() {
    // The control for the four declines above: after Self, Unknown, `_`,
    // path-stripping and alpha-equivalence, a genuine mismatch must
    // still be reported. Without this the leniency could have grown
    // until the instrument reported nothing at all — which is the
    // failure mode every other test here is blind to.
    let proto = func(vec![named("Int")], named("Int"));
    let imp = func(vec![named("Text")], named("Int"));
    assert!(
        TypeChecker::signature_disagreement(&proto, &imp).is_some(),
        "Int against Text must survive every decline"
    );
}

#[test]
fn a_protocols_own_parameter_instantiated_by_the_impl_is_not_a_disagreement() {
    // `Module::forward(In)` implemented for `AttentionInput` is what
    // implementing a generic protocol MEANS. The earlier heuristic
    // declined only pairs where BOTH sides were one or two uppercase
    // characters, so this slipped through and would have inflated a
    // corpus-wide count by every generic protocol in core/.
    let params: std::collections::HashSet<String> = ["In".to_string()].into_iter().collect();
    let proto = func(vec![named("In")], Type::Unit);
    let imp = func(vec![named("AttentionInput")], Type::Unit);
    assert!(
        TypeChecker::signature_disagreement_with_params(&proto, &imp, &params).is_none(),
        "an instantiated protocol parameter is not a disagreement"
    );
}

#[test]
fn a_name_that_is_not_a_protocol_parameter_still_disagrees() {
    // The control for the decline above: knowing the parameter list must
    // not turn the comparison into a blanket yes. `Int` is not a
    // parameter of this protocol, so `Int` against `Text` survives.
    let params: std::collections::HashSet<String> = ["In".to_string()].into_iter().collect();
    let proto = func(vec![named("Int")], Type::Unit);
    let imp = func(vec![named("Text")], Type::Unit);
    assert!(
        TypeChecker::signature_disagreement_with_params(&proto, &imp, &params).is_some(),
        "a non-parameter mismatch must survive"
    );
}
