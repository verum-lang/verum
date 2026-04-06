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
// Comprehensive tests for tensor operations with meta parameters
//
// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Meta parameters for compile-time tensor shapes
// Verum base types: Bool, Int, Float, Text, Unit, plus compound types (Array, Tuple, Record, Function) and Tensor<T, Shape> with compile-time shape parameters — with meta parameters
//
// These tests verify that meta parameters work correctly for tensor shapes
// and compile-time computation.

use verum_ast::{
    expr::{ArrayExpr, Expr, ExprKind},
    literal::Literal,
    span::Span,
};
use verum_common::{ConstValue, List as CoreList, Map, Text};
use verum_types::TypeChecker;

// Helper to create integer literal
fn int_lit(n: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::int(n as i128, Span::dummy())),
        Span::dummy(),
    )
}

// Helper to create shape array
fn shape_array(dims: &[i64]) -> Expr {
    let elements: CoreList<Expr> = dims.iter().map(|&n| int_lit(n)).collect();
    Expr::new(ExprKind::Array(ArrayExpr::List(elements)), Span::dummy())
}

#[test]
fn test_compute_1d_tensor_shape() {
    let mut checker = TypeChecker::new();

    // Shape: [10]
    let shape = shape_array(&[10]);
    let dims = checker.compute_tensor_shape(&shape).unwrap();

    assert_eq!(dims, vec![10].into());
}

#[test]
fn test_compute_2d_tensor_shape() {
    let mut checker = TypeChecker::new();

    // Shape: [2, 3]
    let shape = shape_array(&[2, 3]);
    let dims = checker.compute_tensor_shape(&shape).unwrap();

    assert_eq!(dims, vec![2, 3].into());
}

#[test]
fn test_compute_3d_tensor_shape() {
    let mut checker = TypeChecker::new();

    // Shape: [2, 3, 4]
    let shape = shape_array(&[2, 3, 4]);
    let dims = checker.compute_tensor_shape(&shape).unwrap();

    assert_eq!(dims, vec![2, 3, 4].into());
}

#[test]
fn test_compute_tensor_elements_1d() {
    let mut checker = TypeChecker::new();

    // Shape: [10]
    let shape = shape_array(&[10]);
    let total = checker.compute_tensor_elements(&shape).unwrap();

    assert_eq!(total, 10);
}

#[test]
fn test_compute_tensor_elements_2d() {
    let mut checker = TypeChecker::new();

    // Shape: [2, 3]
    let shape = shape_array(&[2, 3]);
    let total = checker.compute_tensor_elements(&shape).unwrap();

    assert_eq!(total, 6);
}

#[test]
fn test_compute_tensor_elements_3d() {
    let mut checker = TypeChecker::new();

    // Shape: [2, 3, 4]
    let shape = shape_array(&[2, 3, 4]);
    let total = checker.compute_tensor_elements(&shape).unwrap();

    assert_eq!(total, 24);
}

#[test]
fn test_compute_tensor_elements_4d() {
    let mut checker = TypeChecker::new();

    // Shape: [2, 3, 4, 5]
    let shape = shape_array(&[2, 3, 4, 5]);
    let total = checker.compute_tensor_elements(&shape).unwrap();

    assert_eq!(total, 120);
}

#[test]
fn test_validate_tensor_shapes_same() {
    let mut checker = TypeChecker::new();

    // Both shapes: [2, 3]
    let shape1 = shape_array(&[2, 3]);
    let shape2 = shape_array(&[2, 3]);

    let compatible = checker.validate_tensor_shapes(&shape1, &shape2).unwrap();
    assert!(compatible);
}

#[test]
fn test_validate_tensor_shapes_different() {
    let mut checker = TypeChecker::new();

    // shape1: [2, 3], shape2: [3, 4]
    let shape1 = shape_array(&[2, 3]);
    let shape2 = shape_array(&[3, 4]);

    let compatible = checker.validate_tensor_shapes(&shape1, &shape2).unwrap();
    assert!(!compatible);
}

#[test]
fn test_validate_tensor_shapes_different_rank() {
    let mut checker = TypeChecker::new();

    // shape1: [2, 3], shape2: [2, 3, 4]
    let shape1 = shape_array(&[2, 3]);
    let shape2 = shape_array(&[2, 3, 4]);

    let compatible = checker.validate_tensor_shapes(&shape1, &shape2).unwrap();
    assert!(!compatible);
}

#[test]
fn test_meta_param_eval_simple_arithmetic() {
    let mut checker = TypeChecker::new();

    // Compute: 2 + 3
    use verum_ast::expr::BinOp;
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Box::new(int_lit(2)),
            right: Box::new(int_lit(3)),
        },
        Span::dummy(),
    );

    let value = checker.eval_meta_param(&expr).unwrap();
    assert_eq!(value, ConstValue::Int(5));
}

#[test]
fn test_meta_param_eval_complex_arithmetic() {
    let mut checker = TypeChecker::new();

    // Compute: (2 + 3) * 4
    use verum_ast::expr::BinOp;
    let add = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Box::new(int_lit(2)),
            right: Box::new(int_lit(3)),
        },
        Span::dummy(),
    );
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Mul,
            left: Box::new(add),
            right: Box::new(int_lit(4)),
        },
        Span::dummy(),
    );

    let value = checker.eval_meta_param(&expr).unwrap();
    assert_eq!(value, ConstValue::Int(20));
}

#[test]
fn test_array_with_computed_size() {
    let mut checker = TypeChecker::new();
    use verum_types::InferMode;

    // Array: [0; 2 + 3]
    use verum_ast::expr::BinOp;
    let size_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Box::new(int_lit(2)),
            right: Box::new(int_lit(3)),
        },
        Span::dummy(),
    );

    let array_expr = Expr::new(
        ExprKind::Array(ArrayExpr::Repeat {
            value: Box::new(int_lit(0)),
            count: Box::new(size_expr),
        }),
        Span::dummy(),
    );

    let result = checker.infer(&array_expr, InferMode::Synth).unwrap();

    // Should infer array type with size 5
    match result.ty {
        verum_types::ty::Type::Array { element, size } => {
            assert_eq!(*element, verum_types::ty::Type::Int);
            assert_eq!(size, Some(5));
        }
        _ => panic!("Expected Array type"),
    }
}

#[test]
fn test_tensor_shape_with_meta_param() {
    use verum_types::ty::Type;

    let _checker = TypeChecker::new();

    // Create meta parameter: Shape: meta [usize]
    let shape_ty = Type::array(Type::Int, Some(2)); // [usize; 2]
    let meta_ty = Type::meta("Shape".into(), shape_ty, None);

    // Verify structure
    match meta_ty {
        Type::Meta { name, ty, .. } => {
            assert_eq!(name, "Shape");
            match *ty {
                Type::Array { element, size } => {
                    assert_eq!(*element, Type::Int);
                    assert_eq!(size, Some(2));
                }
                _ => panic!("Expected Array type"),
            }
        }
        _ => panic!("Expected Meta type"),
    }
}

#[test]
fn test_meta_param_substitution_in_array() {
    use verum_types::ty::Type;

    let mut checker = TypeChecker::new();

    // Create type with meta parameter: [T; N] where N is meta
    let n_meta = Type::meta("N".into(), Type::Int, None);

    // Substitute N = 10
    let mut env: Map<Text, ConstValue> = Map::new();
    env.insert("N".into(), ConstValue::UInt(10));

    let resolved = checker.substitute_meta(&n_meta, &env).unwrap();

    // Should still be a Meta type (substitution doesn't eliminate meta wrapper)
    assert!(matches!(resolved, Type::Meta { .. }));
}

#[test]
fn test_tensor_type_display() {
    use verum_ast::Ident;
    use verum_ast::ty::Path;
    use verum_types::ty::Type;

    // Create Tensor<Float, [2, 3]> type
    let float_ty = Type::Float;
    let shape_array = Type::array(Type::Int, Some(2));
    let shape_meta = Type::meta("Shape".into(), shape_array, None);

    let tensor_path = Path::single(Ident::new("Tensor", Span::dummy()));
    let tensor_ty = Type::Named {
        path: tensor_path,
        args: vec![float_ty, shape_meta].into(),
    };

    let display = format!("{}", tensor_ty);
    assert!(display.contains("Tensor"));
    assert!(display.contains("Float"));
    assert!(display.contains("Shape"));
}

#[test]
fn test_dynamic_tensor_shape() {
    let mut checker = TypeChecker::new();

    // Create shape with variable (not compile-time constant)
    use verum_ast::ty::{Ident, Path};

    let path = Path::single(Ident::new("n", Span::dummy()));
    let var_expr = Expr::new(ExprKind::Path(path), Span::dummy());

    // This should fail because 'n' is not bound
    let result = checker.compute_tensor_shape(&var_expr);
    assert!(result.is_err());
}

#[test]
fn test_nested_array_shape_computation() {
    let mut checker = TypeChecker::new();

    // Create nested array: [[1, 2], [3, 4]]
    let mut row1_list = CoreList::new();
    row1_list.push(int_lit(1));
    row1_list.push(int_lit(2));
    let row1 = Expr::new(ExprKind::Array(ArrayExpr::List(row1_list)), Span::dummy());
    let mut row2_list = CoreList::new();
    row2_list.push(int_lit(3));
    row2_list.push(int_lit(4));
    let row2 = Expr::new(ExprKind::Array(ArrayExpr::List(row2_list)), Span::dummy());
    let mut matrix_list = CoreList::new();
    matrix_list.push(row1);
    matrix_list.push(row2);
    let matrix = Expr::new(ExprKind::Array(ArrayExpr::List(matrix_list)), Span::dummy());

    // Evaluate the nested array
    let value = checker.eval_meta_param(&matrix).unwrap();

    match value {
        ConstValue::Array(rows) => {
            assert_eq!(rows.len(), 2);
            // Each row should be an array
            for row in rows {
                assert!(row.is_array());
                assert_eq!(row.len(), Some(2));
            }
        }
        _ => panic!("Expected nested array"),
    }
}

#[test]
fn test_zero_sized_tensor() {
    let mut checker = TypeChecker::new();

    // Shape: [0]
    let shape = shape_array(&[0]);
    let total = checker.compute_tensor_elements(&shape).unwrap();

    assert_eq!(total, 0);
}

#[test]
fn test_large_tensor_shape() {
    let mut checker = TypeChecker::new();

    // Shape: [100, 100, 100]
    let shape = shape_array(&[100, 100, 100]);
    let total = checker.compute_tensor_elements(&shape).unwrap();

    assert_eq!(total, 1_000_000);
}

#[test]
fn test_meta_param_in_refinement() {
    use verum_ast::expr::BinOp;
    use verum_types::RefinementPredicate;
    use verum_types::ty::Type;

    // Create N: meta usize{> 0}
    // Refinement predicate: N > 0
    let pred_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Gt,
            left: Box::new(int_lit(0)), // N will be substituted here
            right: Box::new(int_lit(0)),
        },
        Span::dummy(),
    );

    let pred = RefinementPredicate::new(pred_expr, "positive".into(), Span::dummy());

    let meta_ty = Type::meta("N".into(), Type::Int, Some(pred));

    // Verify structure
    match meta_ty {
        Type::Meta {
            name, refinement, ..
        } => {
            assert_eq!(name, "N");
            assert!(refinement.is_some());
        }
        _ => panic!("Expected Meta type"),
    }
}

#[test]
fn test_const_eval_with_overflow() {
    use verum_types::const_eval::ConstEvalError;

    let mut checker = TypeChecker::new();

    // Try to compute overflow: i128::MAX + 1
    use verum_ast::expr::BinOp;
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Box::new(Expr::new(
                ExprKind::Literal(Literal::int(i128::MAX, Span::dummy())),
                Span::dummy(),
            )),
            right: Box::new(int_lit(1)),
        },
        Span::dummy(),
    );

    let result = checker.eval_meta_param(&expr);
    assert!(matches!(result, Err(ConstEvalError::Overflow { .. })));
}

#[test]
fn test_tensor_shape_arithmetic() {
    let mut checker = TypeChecker::new();

    // Compute shape: [2 * 3, 4 + 1]
    use verum_ast::expr::BinOp;

    let dim1 = Expr::new(
        ExprKind::Binary {
            op: BinOp::Mul,
            left: Box::new(int_lit(2)),
            right: Box::new(int_lit(3)),
        },
        Span::dummy(),
    );

    let dim2 = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Box::new(int_lit(4)),
            right: Box::new(int_lit(1)),
        },
        Span::dummy(),
    );

    let mut shape_list = CoreList::new();
    shape_list.push(dim1);
    shape_list.push(dim2);
    let shape = Expr::new(ExprKind::Array(ArrayExpr::List(shape_list)), Span::dummy());

    let dims = checker.compute_tensor_shape(&shape).unwrap();
    assert_eq!(dims, vec![6, 5].into());

    let total = checker.compute_tensor_elements(&shape).unwrap();
    assert_eq!(total, 30);
}
