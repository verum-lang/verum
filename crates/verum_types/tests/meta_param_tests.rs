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
// Tests for Meta Parameters
//
// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Unified compile-time computation
// Meta parameters replace const generics with unified meta-system.
//
// Examples:
// - `N: meta usize` - compile-time usize value
// - `Shape: meta [usize]` - compile-time usize array
// - `N: meta usize{> 0}` - with refinement constraint

use verum_ast::{
    expr::{Expr, ExprKind},
    span::Span,
};
use verum_common::Text;
use verum_types::refinement::RefinementPredicate;
use verum_types::ty::Type;

#[test]
fn test_meta_param_creation() {
    // Test creating a simple meta parameter: N: meta usize
    let meta_ty = Type::meta("N".into(), Type::Int, None);

    match meta_ty {
        Type::Meta {
            name,
            ty,
            refinement,
        } => {
            assert_eq!(name, "N");
            assert_eq!(*ty, Type::Int);
            assert!(refinement.is_none());
        }
        _ => panic!("Expected Meta type"),
    }
}

#[test]
fn test_meta_param_with_refinement() {
    // Test creating meta parameter with refinement: N: meta usize{> 0}
    let pred_expr = Expr::new(
        ExprKind::Literal(verum_ast::literal::Literal::int(0, Span::dummy())),
        Span::dummy(),
    );
    let pred = RefinementPredicate::inline(pred_expr, Span::dummy());

    let meta_ty = Type::meta("N".into(), Type::Int, Some(pred));

    match meta_ty {
        Type::Meta {
            name,
            ty,
            refinement,
        } => {
            assert_eq!(name, "N");
            assert_eq!(*ty, Type::Int);
            assert!(refinement.is_some());
        }
        _ => panic!("Expected Meta type"),
    }
}

#[test]
fn test_meta_param_array_shape() {
    // Test creating meta parameter for array shape: Shape: meta [usize]
    let array_ty = Type::array(Type::Int, None);
    let meta_ty = Type::meta("Shape".into(), array_ty, None);

    match meta_ty {
        Type::Meta { name, ty, .. } => {
            assert_eq!(name, "Shape");
            match *ty {
                Type::Array { element, size } => {
                    assert_eq!(*element, Type::Int);
                    assert_eq!(size, None);
                }
                _ => panic!("Expected Array type"),
            }
        }
        _ => panic!("Expected Meta type"),
    }
}

#[test]
fn test_meta_param_display() {
    // Test Display implementation for meta parameters
    let meta_ty = Type::meta("N".into(), Type::Int, None);

    let display = format!("{}", meta_ty);
    assert_eq!(display, "N: meta Int");
}

#[test]
fn test_meta_param_with_refinement_display() {
    // Test Display with refinement
    let pred_expr = Expr::new(
        ExprKind::Literal(verum_ast::literal::Literal::int(0, Span::dummy())),
        Span::dummy(),
    );
    let pred = RefinementPredicate::lambda(pred_expr, Text::from("positive"), Span::dummy());

    let meta_ty = Type::meta("N".into(), Type::Int, Some(pred));

    let display = format!("{}", meta_ty);
    assert!(display.contains("N: meta Int"));
    assert!(display.contains("{"));
}

#[test]
fn test_meta_param_free_vars() {
    // Meta parameters should track type variables in their base type
    use verum_types::ty::TypeVar;

    let type_var = TypeVar::fresh();
    let meta_ty = Type::meta("N".into(), Type::Var(type_var), None);

    let free_vars = meta_ty.free_vars();
    assert_eq!(free_vars.len(), 1);
    assert!(free_vars.contains(&type_var));
}

#[test]
fn test_meta_param_substitution() {
    // Test that substitution works correctly for meta parameters
    use verum_types::ty::{Substitution, TypeVar};

    let type_var = TypeVar::fresh();
    let meta_ty = Type::meta("N".into(), Type::Var(type_var), None);

    let mut subst = Substitution::new();
    subst.insert(type_var, Type::Int);

    let substituted = meta_ty.apply_subst(&subst);

    match substituted {
        Type::Meta { name, ty, .. } => {
            assert_eq!(name, "N");
            assert_eq!(*ty, Type::Int);
        }
        _ => panic!("Expected Meta type after substitution"),
    }
}

#[test]
fn test_meta_param_tensor_shape() {
    // Test realistic Tensor shape example: Tensor<Float, [2, 3]>
    // Shape parameter: Shape: meta [usize]

    let shape_array = Type::array(Type::Int, Some(2)); // [usize; 2] for [2, 3]
    let shape_meta = Type::meta("Shape".into(), shape_array, None);

    match shape_meta {
        Type::Meta { name, ty, .. } => {
            assert_eq!(name, "Shape");
            match *ty {
                Type::Array { element, size } => {
                    assert_eq!(*element, Type::Int);
                    assert_eq!(size, Some(2));
                }
                _ => panic!("Expected Array type for tensor shape"),
            }
        }
        _ => panic!("Expected Meta type"),
    }
}

#[test]
fn test_meta_param_nested() {
    // Test nested meta parameters: Matrix<T, Rows: meta usize, Cols: meta usize>
    let rows_meta = Type::meta("Rows".into(), Type::Int, None);
    let cols_meta = Type::meta("Cols".into(), Type::Int, None);

    // Both should be valid meta types
    assert!(matches!(rows_meta, Type::Meta { .. }));
    assert!(matches!(cols_meta, Type::Meta { .. }));
}

#[test]
fn test_meta_param_is_monotype() {
    // Meta parameters without type variables should be monotypes
    let meta_ty = Type::meta("N".into(), Type::Int, None);

    assert!(meta_ty.is_monotype());
}

#[test]
fn test_meta_param_not_monotype() {
    // Meta parameters with type variables should not be monotypes
    use verum_types::ty::TypeVar;

    let type_var = TypeVar::fresh();
    let meta_ty = Type::meta("N".into(), Type::Var(type_var), None);

    assert!(!meta_ty.is_monotype());
}
