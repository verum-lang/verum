//! Plain associated-type resolvers cannot erase a family's applied arguments.
use std::sync::Arc;
use verum_ast::Span;
use verum_common::{List, Text};
use verum_types::{Type, protocol::ProtocolChecker, unify::Unifier};

fn rows(argument: Type) -> Type {
    Type::Generic { name: Text::from("::Rows"), args: List::from_iter([Type::Bool, argument]) }
}

#[test]
fn public_projection_normalizer_preserves_distinct_family_applications() {
    let checker = ProtocolChecker::new();
    let int = rows(Type::Int);
    let text = rows(Type::Text);
    let normalized_int = checker.normalize_projection_type(&int);
    let normalized_text = checker.normalize_projection_type(&text);
    assert_eq!(normalized_int, int);
    assert_eq!(normalized_text, text);
    assert_ne!(normalized_int, normalized_text, "Rows<Int> and Rows<Text> must remain distinct");
}

#[test]
fn a_plain_projection_resolver_cannot_authorize_an_applied_family() {
    let mut unifier = Unifier::new();
    unifier.set_projection_resolver(Arc::new(|_, _| Some(Type::Text)));
    assert!(unifier.unify(&rows(Type::Int), &Type::Text, Span::dummy()).is_err(),
        "a resolver without the family argument cannot prove its result");
    assert!(unifier.unify(&rows(Type::Int), &rows(Type::Text), Span::dummy()).is_err());
}

#[test]
fn plain_projection_resolution_keeps_its_existing_exact_behavior() {
    let mut unifier = Unifier::new();
    unifier.set_projection_resolver(Arc::new(|base, member| {
        (base == &Type::Bool && member == "Item").then_some(Type::Text)
    }));
    let plain = Type::Generic { name: Text::from("::Item"), args: List::from_single(Type::Bool) };
    assert!(unifier.unify(&plain, &Type::Text, Span::dummy()).is_ok());
}
