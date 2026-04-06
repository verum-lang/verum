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
//! Tests for array types with const generic size parameters.
//!
//! This test suite verifies that arrays with explicit sizes work correctly
//! in type inference and unification.

use verum_ast::{expr::*, literal::Literal, span::Span};
use verum_types::{Type, TypeChecker, Unifier};

#[test]
fn test_array_literal_infers_size() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Create array literal: [1.0, 2.0, 3.0, 4.0, 5.0]
    let elements = vec![
        Expr::literal(Literal::float(1.0, span)),
        Expr::literal(Literal::float(2.0, span)),
        Expr::literal(Literal::float(3.0, span)),
        Expr::literal(Literal::float(4.0, span)),
        Expr::literal(Literal::float(5.0, span)),
    ];

    let array_expr = Expr::new(ExprKind::Array(ArrayExpr::List(elements.into())), span);

    let result = checker.synth_expr(&array_expr).unwrap();

    // Check that the array type has the correct size
    match result.ty {
        Type::Array { element, size } => {
            assert_eq!(*element, Type::float());
            assert_eq!(size, Some(5), "Array literal should infer size as 5");
        }
        _ => panic!("Expected Array type, got {:?}", result.ty),
    }
}

#[test]
fn test_array_unify_same_size() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let ty1 = Type::array(Type::float(), Some(5));
    let ty2 = Type::array(Type::float(), Some(5));

    let result = unifier.unify(&ty1, &ty2, span);
    assert!(
        result.is_ok(),
        "Arrays with same size should unify: {:?}",
        result.err()
    );
}

#[test]
fn test_array_unify_different_sizes() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let ty1 = Type::array(Type::float(), Some(5));
    let ty2 = Type::array(Type::float(), Some(3));

    let result = unifier.unify(&ty1, &ty2, span);
    assert!(
        result.is_err(),
        "Arrays with different sizes should not unify"
    );

    let err = result.err().unwrap();
    let err_text = format!("{:?}", err);
    assert!(
        err_text.contains("5") && err_text.contains("3"),
        "Error should mention both sizes: {}",
        err_text
    );
}

#[test]
fn test_array_unify_known_with_unknown_size() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let ty1 = Type::array(Type::float(), Some(5));
    let ty2 = Type::array(Type::float(), None);

    // Should unify: one has known size, other doesn't
    let result = unifier.unify(&ty1, &ty2, span);
    assert!(
        result.is_ok(),
        "Array with known size should unify with unknown size: {:?}",
        result.err()
    );

    // Try the other way around
    let result = unifier.unify(&ty2, &ty1, span);
    assert!(
        result.is_ok(),
        "Array with unknown size should unify with known size: {:?}",
        result.err()
    );
}

#[test]
fn test_array_unify_both_unknown() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let ty1 = Type::array(Type::float(), None);
    let ty2 = Type::array(Type::float(), None);

    let result = unifier.unify(&ty1, &ty2, span);
    assert!(
        result.is_ok(),
        "Arrays with unknown sizes should unify: {:?}",
        result.err()
    );
}

#[test]
fn test_array_repeat_syntax() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Create array repeat: [0; 10]
    let value = Box::new(Expr::literal(Literal::int(0, span)));
    let count = Box::new(Expr::literal(Literal::int(10, span)));

    let array_expr = Expr::new(ExprKind::Array(ArrayExpr::Repeat { value, count }), span);

    let result = checker.synth_expr(&array_expr).unwrap();

    // Check that the array type has the correct size
    match result.ty {
        Type::Array { element, size } => {
            assert_eq!(*element, Type::int());
            assert_eq!(size, Some(10), "Array repeat should infer size as 10");
        }
        _ => panic!("Expected Array type, got {:?}", result.ty),
    }
}

#[test]
fn test_array_index_access() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Create array: [1, 2, 3]
    let elements = vec![
        Expr::literal(Literal::int(1, span)),
        Expr::literal(Literal::int(2, span)),
        Expr::literal(Literal::int(3, span)),
    ];
    let array_expr = Expr::new(ExprKind::Array(ArrayExpr::List(elements.into())), span);

    // Index: arr[0]
    let index_expr = Expr::new(
        ExprKind::Index {
            expr: Box::new(array_expr),
            index: Box::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let result = checker.synth_expr(&index_expr).unwrap();

    // Check that indexing returns the element type
    assert_eq!(
        result.ty,
        Type::int(),
        "Array indexing should return element type"
    );
}

#[test]
fn test_array_empty() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Create empty array: []
    let array_expr = Expr::new(ExprKind::Array(ArrayExpr::List(vec![].into())), span);

    let result = checker.synth_expr(&array_expr).unwrap();

    // Check that empty array has size 0
    match result.ty {
        Type::Array { size, .. } => {
            assert_eq!(size, Some(0), "Empty array should have size 0");
        }
        _ => panic!("Expected Array type, got {:?}", result.ty),
    }
}

#[test]
fn test_array_single_element() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Create single element array: [42]
    let array_expr = Expr::new(
        ExprKind::Array(ArrayExpr::List(
            vec![Expr::literal(Literal::int(42, span))].into(),
        )),
        span,
    );

    let result = checker.synth_expr(&array_expr).unwrap();

    // Check that single element array has size 1
    match result.ty {
        Type::Array { element, size } => {
            assert_eq!(*element, Type::int());
            assert_eq!(size, Some(1), "Single element array should have size 1");
        }
        _ => panic!("Expected Array type, got {:?}", result.ty),
    }
}
