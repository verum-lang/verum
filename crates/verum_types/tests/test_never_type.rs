#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Bottom elimination is a subtype rule, not symmetric type equality.

use verum_ast::span::Span;
use verum_types::ty::Type;
use verum_types::unify::Unifier;
use verum_types::subtype::Subtyping;

fn span() -> Span {
    Span::default()
}

#[test]
fn test_never_is_subtype_of_int_but_not_equal() {
    let mut unifier = Unifier::new();
    let never = Type::never();
    let int = Type::int();

    // Equality cannot coerce bottom.
    let result = unifier.unify(&never, &int, span());
    assert!(result.is_err(), "bottom is not equal to Int");
    assert!(Subtyping::new().is_subtype(&never, &int));
    assert!(!Subtyping::new().is_subtype(&int, &never));

    // The reverse equality cannot introduce bottom.
    let mut unifier2 = Unifier::new();
    let result2 = unifier2.unify(&int, &never, span());
    assert!(result2.is_err(), "Int cannot manufacture bottom");
}

#[test]
fn test_never_is_subtype_of_bool_but_not_equal() {
    let mut unifier = Unifier::new();
    let never = Type::never();
    let bool = Type::bool();

    // Bottom may be eliminated into Bool, without equating the types.
    let result = unifier.unify(&never, &bool, span());
    assert!(result.is_err());
    assert!(Subtyping::new().is_subtype(&never, &bool));
}

#[test]
fn test_never_is_subtype_of_text_but_not_equal() {
    let mut unifier = Unifier::new();
    let never = Type::never();
    let text = Type::text();

    // Bottom may be eliminated into Text, without equating the types.
    let result = unifier.unify(&never, &text, span());
    assert!(result.is_err());
    assert!(Subtyping::new().is_subtype(&never, &text));
}

#[test]
fn test_never_unifies_with_never() {
    let mut unifier = Unifier::new();
    let never1 = Type::never();
    let never2 = Type::never();

    // Never should unify with Never
    let result = unifier.unify(&never1, &never2, span());
    assert!(result.is_ok(), "Never should unify with Never");
}

#[test]
fn test_never_is_subtype_of_unit_but_not_equal() {
    let mut unifier = Unifier::new();
    let never = Type::never();
    let unit = Type::unit();

    // A diverging path coerces to Unit; a fallthrough Unit path is inhabited.
    let result = unifier.unify(&never, &unit, span());
    assert!(result.is_err());
    assert!(Subtyping::new().is_subtype(&never, &unit));
}

#[test]
fn test_never_display() {
    let never = Type::never();
    assert_eq!(
        format!("{}", never),
        "!",
        "Never type should display as '!'"
    );
}
