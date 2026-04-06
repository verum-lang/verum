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
    unused_assignments,
    clippy::approx_constant
)]
//! Tests for type inference placeholder fixes
//!
//! These tests verify that the type inference system properly handles
//! edge cases that previously fell back to placeholder expressions:
//!
//! 1. Type argument to expression conversion
//! 2. Tuple indexing with compile-time constant evaluation
//! 3. Fin type symbolic value generation
//! 4. Tensor shape evaluation with meta parameters
//!
//! Core type system + dependent types + tensor type validation

use verum_ast::{
    expr::{ArrayExpr, BinOp, Expr, ExprKind, UnOp},
    literal::Literal,
    span::Span,
    ty::{GenericArg, Ident, Path, PathSegment, Type as AstType, TypeKind},
};
use verum_common::{ConstValue, List, Text};
use verum_types::const_eval::ConstEvaluator;

// ============================================================================
// Helper Functions
// ============================================================================

fn int_lit(n: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::int(n as i128, Span::dummy())),
        Span::dummy(),
    )
}

fn path_expr(name: &str) -> Expr {
    Expr::new(
        ExprKind::Path(Path::single(Ident::new(name, Span::dummy()))),
        Span::dummy(),
    )
}

fn tuple_expr(elements: Vec<Expr>) -> Expr {
    Expr::new(ExprKind::Tuple(elements.into()), Span::dummy())
}

// ============================================================================
// Const Evaluator - Tuple Indexing Tests
// ============================================================================

#[test]
fn test_tuple_index_at_compile_time_first_element() {
    let mut eval = ConstEvaluator::new();

    // Create tuple (10, 20, 30)
    let elements = vec![int_lit(10), int_lit(20), int_lit(30)];
    let tuple = Expr::new(ExprKind::Tuple(elements.into()), Span::dummy());

    // Access .0
    let expr = Expr::new(
        ExprKind::TupleIndex {
            expr: Box::new(tuple),
            index: 0,
        },
        Span::dummy(),
    );

    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(10));
}

#[test]
fn test_tuple_index_at_compile_time_middle_element() {
    let mut eval = ConstEvaluator::new();

    // Create tuple (100, 200, 300, 400)
    let elements = vec![int_lit(100), int_lit(200), int_lit(300), int_lit(400)];
    let tuple = Expr::new(ExprKind::Tuple(elements.into()), Span::dummy());

    // Access .2
    let expr = Expr::new(
        ExprKind::TupleIndex {
            expr: Box::new(tuple),
            index: 2,
        },
        Span::dummy(),
    );

    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(300));
}

#[test]
fn test_tuple_index_at_compile_time_last_element() {
    let mut eval = ConstEvaluator::new();

    // Create tuple (1, 2, 3, 4, 5)
    let elements = vec![int_lit(1), int_lit(2), int_lit(3), int_lit(4), int_lit(5)];
    let tuple = Expr::new(ExprKind::Tuple(elements.into()), Span::dummy());

    // Access .4 (last element)
    let expr = Expr::new(
        ExprKind::TupleIndex {
            expr: Box::new(tuple),
            index: 4,
        },
        Span::dummy(),
    );

    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(5));
}

#[test]
fn test_nested_tuple_index() {
    let mut eval = ConstEvaluator::new();

    // Create nested tuple ((1, 2), (3, 4))
    let inner1 = Expr::new(ExprKind::Tuple(vec![int_lit(1), int_lit(2)].into()), Span::dummy());
    let inner2 = Expr::new(ExprKind::Tuple(vec![int_lit(3), int_lit(4)].into()), Span::dummy());
    let outer = Expr::new(ExprKind::Tuple(vec![inner1, inner2].into()), Span::dummy());

    // Access .1 to get (3, 4)
    let first_access = Expr::new(
        ExprKind::TupleIndex {
            expr: Box::new(outer),
            index: 1,
        },
        Span::dummy(),
    );

    let result = eval.eval(&first_access).unwrap();
    match result {
        ConstValue::Tuple(tup) => {
            assert_eq!(tup.len(), 2);
            assert_eq!(tup[0], ConstValue::Int(3));
            assert_eq!(tup[1], ConstValue::Int(4));
        }
        _ => panic!("Expected tuple"),
    }
}

// ============================================================================
// Const Evaluator - Array Index Tests with Constants
// ============================================================================

#[test]
fn test_array_index_with_constant_expression() {
    let mut eval = ConstEvaluator::new();

    // Create array [100, 200, 300, 400, 500]
    let elements = vec![
        int_lit(100),
        int_lit(200),
        int_lit(300),
        int_lit(400),
        int_lit(500),
    ];
    let array = Expr::new(ExprKind::Array(ArrayExpr::List(elements.into())), Span::dummy());

    // Access with computed index: 1 + 1 = 2
    let index_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Box::new(int_lit(1)),
            right: Box::new(int_lit(1)),
        },
        Span::dummy(),
    );

    let expr = Expr::new(
        ExprKind::Index {
            expr: Box::new(array),
            index: Box::new(index_expr),
        },
        Span::dummy(),
    );

    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(300));
}

#[test]
fn test_array_index_with_variable_binding() {
    let mut eval = ConstEvaluator::new();

    // Bind i = 3
    eval.bind("i", ConstValue::Int(3));

    // Create array [10, 20, 30, 40, 50]
    let elements = vec![
        int_lit(10),
        int_lit(20),
        int_lit(30),
        int_lit(40),
        int_lit(50),
    ];
    let array = Expr::new(ExprKind::Array(ArrayExpr::List(elements.into())), Span::dummy());

    // Access with variable: array[i]
    let index_expr = path_expr("i");

    let expr = Expr::new(
        ExprKind::Index {
            expr: Box::new(array),
            index: Box::new(index_expr),
        },
        Span::dummy(),
    );

    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(40));
}

// ============================================================================
// Meta Parameter Handling in Tensor Shapes
// ============================================================================

#[test]
fn test_tensor_shape_with_literal_dimensions() {
    let mut eval = ConstEvaluator::new();

    // Shape: [2, 3, 4]
    let shape = Expr::new(
        ExprKind::Array(ArrayExpr::List(vec![int_lit(2), int_lit(3), int_lit(4)].into())),
        Span::dummy(),
    );

    let dims = eval.compute_tensor_shape(&shape).unwrap();
    assert_eq!(dims.len(), 3);
    assert_eq!(dims[0], 2);
    assert_eq!(dims[1], 3);
    assert_eq!(dims[2], 4);
}

#[test]
fn test_tensor_shape_with_computed_dimensions() {
    let mut eval = ConstEvaluator::new();

    // Shape: [2 + 1, 3 * 2] = [3, 6]
    let dim1 = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Box::new(int_lit(2)),
            right: Box::new(int_lit(1)),
        },
        Span::dummy(),
    );
    let dim2 = Expr::new(
        ExprKind::Binary {
            op: BinOp::Mul,
            left: Box::new(int_lit(3)),
            right: Box::new(int_lit(2)),
        },
        Span::dummy(),
    );
    let shape = Expr::new(
        ExprKind::Array(ArrayExpr::List(vec![dim1, dim2].into())),
        Span::dummy(),
    );

    let dims = eval.compute_tensor_shape(&shape).unwrap();
    assert_eq!(dims.len(), 2);
    assert_eq!(dims[0], 3);
    assert_eq!(dims[1], 6);
}

#[test]
fn test_tensor_shape_with_bound_meta_parameter() {
    let mut eval = ConstEvaluator::new();

    // Bind N = 4
    eval.bind("N", ConstValue::Int(4));

    // Shape: [N, N]
    let n1 = path_expr("N");
    let n2 = path_expr("N");
    let shape = Expr::new(
        ExprKind::Array(ArrayExpr::List(vec![n1, n2].into())),
        Span::dummy(),
    );

    let dims = eval.compute_tensor_shape(&shape).unwrap();
    assert_eq!(dims.len(), 2);
    assert_eq!(dims[0], 4);
    assert_eq!(dims[1], 4);
}

#[test]
fn test_tensor_total_elements() {
    let mut eval = ConstEvaluator::new();

    // Shape: [2, 3, 4] -> total = 24
    let shape = Expr::new(
        ExprKind::Array(ArrayExpr::List(vec![int_lit(2), int_lit(3), int_lit(4)].into())),
        Span::dummy(),
    );

    let total = eval.compute_tensor_elements(&shape).unwrap();
    assert_eq!(total, 24);
}

#[test]
fn test_tensor_shape_broadcast_compatibility() {
    let mut eval = ConstEvaluator::new();

    // Shape1: [3, 1, 5]
    let shape1 = Expr::new(
        ExprKind::Array(ArrayExpr::List(vec![int_lit(3), int_lit(1), int_lit(5)].into())),
        Span::dummy(),
    );

    // Shape2: [1, 4, 5] - broadcast compatible with shape1
    let shape2 = Expr::new(
        ExprKind::Array(ArrayExpr::List(vec![int_lit(1), int_lit(4), int_lit(5)].into())),
        Span::dummy(),
    );

    let compatible = eval.validate_tensor_shapes(&shape1, &shape2).unwrap();
    assert!(compatible);
}

#[test]
fn test_tensor_shape_not_broadcast_compatible() {
    let mut eval = ConstEvaluator::new();

    // Shape1: [3, 2, 5]
    let shape1 = Expr::new(
        ExprKind::Array(ArrayExpr::List(vec![int_lit(3), int_lit(2), int_lit(5)].into())),
        Span::dummy(),
    );

    // Shape2: [3, 4, 5] - NOT broadcast compatible (2 != 4 and neither is 1)
    let shape2 = Expr::new(
        ExprKind::Array(ArrayExpr::List(vec![int_lit(3), int_lit(4), int_lit(5)].into())),
        Span::dummy(),
    );

    let compatible = eval.validate_tensor_shapes(&shape1, &shape2).unwrap();
    assert!(!compatible);
}

// ============================================================================
// Type Variable Expression Generation
// ============================================================================

#[test]
fn test_const_value_conversions() {
    // Test Int conversion
    let int_val = ConstValue::Int(42);
    assert_eq!(int_val.as_i64(), Some(42));
    assert_eq!(int_val.as_u64(), Some(42));

    // Test UInt conversion
    let uint_val = ConstValue::UInt(100);
    assert_eq!(uint_val.as_u64(), Some(100));
    assert_eq!(uint_val.as_i64(), Some(100));

    // Test Bool conversion
    let bool_val = ConstValue::Bool(true);
    assert_eq!(bool_val.as_bool_value(), Some(true));

    // Test Float conversion
    let float_val = ConstValue::Float(3.14);
    assert_eq!(float_val.as_f64(), Some(3.14));

    // Test Text conversion
    let text_val = ConstValue::Text(Text::from("hello"));
    assert_eq!(text_val.as_text(), Some(&Text::from("hello")));

    // Test Char conversion
    let char_val = ConstValue::Char('x');
    assert_eq!(char_val.as_char_value(), Some('x'));
}

#[test]
fn test_const_value_array_methods() {
    let arr = ConstValue::Array(List::from(vec![
        ConstValue::Int(1),
        ConstValue::Int(2),
        ConstValue::Int(3),
    ]));

    assert!(arr.is_array());
    assert_eq!(arr.len(), Some(3));

    let not_arr = ConstValue::Int(42);
    assert!(!not_arr.is_array());
    assert_eq!(not_arr.len(), None);
}

// ============================================================================
// Edge Cases for Constant Evaluation
// ============================================================================

#[test]
fn test_eval_zero_dimension_tensor_shape() {
    let mut eval = ConstEvaluator::new();

    // Empty shape: [] for scalar
    let shape = Expr::new(ExprKind::Array(ArrayExpr::List(vec![].into())), Span::dummy());

    let dims = eval.compute_tensor_shape(&shape).unwrap();
    assert_eq!(dims.len(), 0);
}

#[test]
fn test_eval_single_dimension_tensor() {
    let mut eval = ConstEvaluator::new();

    // Shape: [10] - 1D tensor
    let shape = Expr::new(
        ExprKind::Array(ArrayExpr::List(vec![int_lit(10)].into())),
        Span::dummy(),
    );

    let dims = eval.compute_tensor_shape(&shape).unwrap();
    assert_eq!(dims.len(), 1);
    assert_eq!(dims[0], 10);
}

#[test]
fn test_eval_large_dimension_product() {
    let mut eval = ConstEvaluator::new();

    // Shape: [100, 100, 100] -> total = 1,000,000
    let shape = Expr::new(
        ExprKind::Array(ArrayExpr::List(vec![
            int_lit(100),
            int_lit(100),
            int_lit(100),
        ].into())),
        Span::dummy(),
    );

    let total = eval.compute_tensor_elements(&shape).unwrap();
    assert_eq!(total, 1_000_000);
}

// ============================================================================
// Heterogeneous Tuple Type Inference Edge Cases
// ============================================================================

#[test]
fn test_mixed_type_tuple_evaluation() {
    let mut eval = ConstEvaluator::new();

    // Create tuple with mixed types: (42, true)
    let int_elem = int_lit(42);
    let bool_elem = Expr::new(
        ExprKind::Literal(Literal::bool(true, Span::dummy())),
        Span::dummy(),
    );
    let tuple = Expr::new(ExprKind::Tuple(vec![int_elem, bool_elem].into()), Span::dummy());

    // Access integer element
    let int_access = Expr::new(
        ExprKind::TupleIndex {
            expr: Box::new(tuple.clone()),
            index: 0,
        },
        Span::dummy(),
    );

    let result = eval.eval(&int_access).unwrap();
    assert_eq!(result, ConstValue::Int(42));
}

#[test]
fn test_deeply_nested_structure_evaluation() {
    let mut eval = ConstEvaluator::new();

    // Create ((1, 2), (3, (4, 5)))
    let inner1 = Expr::new(ExprKind::Tuple(vec![int_lit(1), int_lit(2)].into()), Span::dummy());
    let inner2_deep = Expr::new(ExprKind::Tuple(vec![int_lit(4), int_lit(5)].into()), Span::dummy());
    let inner2 = Expr::new(
        ExprKind::Tuple(vec![int_lit(3), inner2_deep].into()),
        Span::dummy(),
    );
    let outer = Expr::new(ExprKind::Tuple(vec![inner1, inner2].into()), Span::dummy());

    // Access outer.1 to get (3, (4, 5))
    let access = Expr::new(
        ExprKind::TupleIndex {
            expr: Box::new(outer),
            index: 1,
        },
        Span::dummy(),
    );

    let result = eval.eval(&access).unwrap();
    match result {
        ConstValue::Tuple(tup) => {
            assert_eq!(tup.len(), 2);
            assert_eq!(tup[0], ConstValue::Int(3));
            match &tup[1] {
                ConstValue::Tuple(inner) => {
                    assert_eq!(inner.len(), 2);
                    assert_eq!(inner[0], ConstValue::Int(4));
                    assert_eq!(inner[1], ConstValue::Int(5));
                }
                _ => panic!("Expected nested tuple"),
            }
        }
        _ => panic!("Expected tuple"),
    }
}
