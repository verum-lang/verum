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
//! Test for Never type unification
//!
//! This test verifies that the Never type (!) properly unifies with all types,
//! which is necessary for early return, break, and continue statements.

use verum_ast::span::Span;
use verum_types::ty::Type;
use verum_types::unify::Unifier;

fn span() -> Span {
    Span::default()
}

#[test]
fn test_never_unifies_with_int() {
    let mut unifier = Unifier::new();
    let never = Type::never();
    let int = Type::int();

    // Never should unify with Int
    let result = unifier.unify(&never, &int, span());
    assert!(result.is_ok(), "Never should unify with Int");

    // Int should unify with Never
    let mut unifier2 = Unifier::new();
    let result2 = unifier2.unify(&int, &never, span());
    assert!(result2.is_ok(), "Int should unify with Never");
}

#[test]
fn test_never_unifies_with_bool() {
    let mut unifier = Unifier::new();
    let never = Type::never();
    let bool = Type::bool();

    // Never should unify with Bool
    let result = unifier.unify(&never, &bool, span());
    assert!(result.is_ok(), "Never should unify with Bool");
}

#[test]
fn test_never_unifies_with_text() {
    let mut unifier = Unifier::new();
    let never = Type::never();
    let text = Type::text();

    // Never should unify with Text
    let result = unifier.unify(&never, &text, span());
    assert!(result.is_ok(), "Never should unify with Text");
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
fn test_never_unifies_with_unit() {
    let mut unifier = Unifier::new();
    let never = Type::never();
    let unit = Type::unit();

    // Never should unify with Unit (important for if statements with no else)
    let result = unifier.unify(&never, &unit, span());
    assert!(result.is_ok(), "Never should unify with Unit");
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
