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
// Comprehensive tests for const evaluation
//
// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Unified compile-time computation
//
// These tests verify that compile-time constant evaluation works correctly
// for meta parameters, enabling features like tensor shape computation.

use verum_ast::{
    expr::{ArrayExpr, BinOp, Expr, ExprKind, UnOp},
    literal::Literal,
    span::Span,
};
use verum_common::{ConstValue, List};
use verum_types::const_eval::{ConstEvalError, ConstEvaluator};

// Helper to create integer literal
fn int_lit(n: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::int(n as i128, Span::dummy())),
        Span::dummy(),
    )
}

// Helper to create boolean literal
fn bool_lit(b: bool) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::bool(b, Span::dummy())),
        Span::dummy(),
    )
}

// Helper to create binary expression
fn binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        Span::dummy(),
    )
}

// Helper to create unary expression
fn unary(op: UnOp, expr: Expr) -> Expr {
    Expr::new(
        ExprKind::Unary {
            op,
            expr: Box::new(expr),
        },
        Span::dummy(),
    )
}

// ============================================================================
// Basic Arithmetic Tests (10 tests)
// ============================================================================

#[test]
fn test_eval_integer_literal() {
    let mut eval = ConstEvaluator::new();
    let expr = int_lit(42);
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(42));
}

#[test]
fn test_eval_addition() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Add, int_lit(2), int_lit(3));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(5));
}

#[test]
fn test_eval_subtraction() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Sub, int_lit(10), int_lit(3));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(7));
}

#[test]
fn test_eval_multiplication() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Mul, int_lit(6), int_lit(7));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(42));
}

#[test]
fn test_eval_division() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Div, int_lit(20), int_lit(4));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(5));
}

#[test]
fn test_eval_remainder() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Rem, int_lit(17), int_lit(5));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(2));
}

#[test]
fn test_eval_complex_arithmetic() {
    let mut eval = ConstEvaluator::new();
    // (2 + 3) * 4
    let add = binary(BinOp::Add, int_lit(2), int_lit(3));
    let expr = binary(BinOp::Mul, add, int_lit(4));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(20));
}

#[test]
fn test_eval_nested_arithmetic() {
    let mut eval = ConstEvaluator::new();
    // 2 * (3 + 4 * 5)
    let inner_mul = binary(BinOp::Mul, int_lit(4), int_lit(5));
    let add = binary(BinOp::Add, int_lit(3), inner_mul);
    let expr = binary(BinOp::Mul, int_lit(2), add);
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(46));
}

#[test]
fn test_eval_negation() {
    let mut eval = ConstEvaluator::new();
    let expr = unary(UnOp::Neg, int_lit(42));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(-42));
}

#[test]
fn test_eval_double_negation() {
    let mut eval = ConstEvaluator::new();
    let inner = unary(UnOp::Neg, int_lit(42));
    let expr = unary(UnOp::Neg, inner);
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(42));
}

// ============================================================================
// Comparison Tests (10 tests)
// ============================================================================

#[test]
fn test_eval_equality() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Eq, int_lit(5), int_lit(5));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_inequality() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Ne, int_lit(5), int_lit(3));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_less_than() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Lt, int_lit(3), int_lit(5));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_less_than_false() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Lt, int_lit(5), int_lit(3));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(false));
}

#[test]
fn test_eval_less_equal() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Le, int_lit(5), int_lit(5));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_greater_than() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Gt, int_lit(7), int_lit(3));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_greater_equal() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Ge, int_lit(5), int_lit(5));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_comparison_chain() {
    let mut eval = ConstEvaluator::new();
    // (3 < 5) && (5 < 10)
    let left = binary(BinOp::Lt, int_lit(3), int_lit(5));
    let right = binary(BinOp::Lt, int_lit(5), int_lit(10));
    let expr = binary(BinOp::And, left, right);
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_refinement_predicate() {
    let mut eval = ConstEvaluator::new();
    // N > 0 where N = 10
    eval.bind("N".to_string(), ConstValue::Int(10));

    use verum_ast::ty::{Ident, Path};
    let path = Path::single(Ident::new("N".to_string(), Span::dummy()));
    let n_var = Expr::new(ExprKind::Path(path), Span::dummy());

    let expr = binary(BinOp::Gt, n_var, int_lit(0));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_refinement_complex() {
    let mut eval = ConstEvaluator::new();
    // N > 0 && N < 100 where N = 50
    eval.bind("N".to_string(), ConstValue::Int(50));

    use verum_ast::ty::{Ident, Path};
    let path1 = Path::single(Ident::new("N".to_string(), Span::dummy()));
    let n_var1 = Expr::new(ExprKind::Path(path1), Span::dummy());

    let path2 = Path::single(Ident::new("N".to_string(), Span::dummy()));
    let n_var2 = Expr::new(ExprKind::Path(path2), Span::dummy());

    let gt = binary(BinOp::Gt, n_var1, int_lit(0));
    let lt = binary(BinOp::Lt, n_var2, int_lit(100));
    let expr = binary(BinOp::And, gt, lt);

    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

// ============================================================================
// Logical Operations Tests (5 tests)
// ============================================================================

#[test]
fn test_eval_boolean_literal() {
    let mut eval = ConstEvaluator::new();
    let expr = bool_lit(true);
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_logical_and() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::And, bool_lit(true), bool_lit(true));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_logical_or() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Or, bool_lit(false), bool_lit(true));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_logical_not() {
    let mut eval = ConstEvaluator::new();
    let expr = unary(UnOp::Not, bool_lit(false));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_logical_complex() {
    let mut eval = ConstEvaluator::new();
    // !(true && false) || true
    let and_expr = binary(BinOp::And, bool_lit(true), bool_lit(false));
    let not_expr = unary(UnOp::Not, and_expr);
    let expr = binary(BinOp::Or, not_expr, bool_lit(true));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

// ============================================================================
// Array Operations Tests (8 tests)
// ============================================================================

#[test]
fn test_eval_array_literal() {
    let mut eval = ConstEvaluator::new();
    let elements: List<Expr> = vec![int_lit(1), int_lit(2), int_lit(3)].into();
    let expr = Expr::new(ExprKind::Array(ArrayExpr::List(elements)), Span::dummy());
    let result = eval.eval(&expr).unwrap();

    match result {
        ConstValue::Array(arr) => {
            assert_eq!(arr.len(), 3);
            assert_eq!(arr[0], ConstValue::Int(1));
            assert_eq!(arr[1], ConstValue::Int(2));
            assert_eq!(arr[2], ConstValue::Int(3));
        }
        _ => panic!("Expected array"),
    }
}

#[test]
fn test_eval_array_index() {
    let mut eval = ConstEvaluator::new();

    // Create array [10, 20, 30]
    let elements: List<Expr> = vec![int_lit(10), int_lit(20), int_lit(30)].into();
    let array = Expr::new(ExprKind::Array(ArrayExpr::List(elements)), Span::dummy());

    // Index: array[1]
    let expr = Expr::new(
        ExprKind::Index {
            expr: Box::new(array),
            index: Box::new(int_lit(1)),
        },
        Span::dummy(),
    );

    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(20));
}

#[test]
fn test_eval_array_shape_2d() {
    let mut eval = ConstEvaluator::new();

    // Shape: [2, 3]
    let elements: List<Expr> = vec![int_lit(2), int_lit(3)].into();
    let expr = Expr::new(ExprKind::Array(ArrayExpr::List(elements)), Span::dummy());

    let result = eval.eval(&expr).unwrap();

    match result {
        ConstValue::Array(arr) => {
            assert_eq!(arr.len(), 2);
            assert_eq!(arr[0], ConstValue::Int(2));
            assert_eq!(arr[1], ConstValue::Int(3));
        }
        _ => panic!("Expected array"),
    }
}

#[test]
fn test_eval_array_computed_size() {
    let mut eval = ConstEvaluator::new();

    // Array size: 2 + 3 = 5
    let size_expr = binary(BinOp::Add, int_lit(2), int_lit(3));

    // [0; 2 + 3]
    let expr = Expr::new(
        ExprKind::Array(ArrayExpr::Repeat {
            value: Box::new(int_lit(0)),
            count: Box::new(size_expr),
        }),
        Span::dummy(),
    );

    let result = eval.eval(&expr).unwrap();

    match result {
        ConstValue::Array(arr) => {
            assert_eq!(arr.len(), 5);
            assert!(arr.iter().all(|v| *v == ConstValue::Int(0)));
        }
        _ => panic!("Expected array"),
    }
}

#[test]
fn test_eval_nested_array() {
    let mut eval = ConstEvaluator::new();

    // [[1, 2], [3, 4]]
    let row1 = Expr::new(
        ExprKind::Array(ArrayExpr::List(vec![int_lit(1), int_lit(2)].into())),
        Span::dummy(),
    );
    let row2 = Expr::new(
        ExprKind::Array(ArrayExpr::List(vec![int_lit(3), int_lit(4)].into())),
        Span::dummy(),
    );
    let expr = Expr::new(
        ExprKind::Array(ArrayExpr::List(vec![row1, row2].into())),
        Span::dummy(),
    );

    let result = eval.eval(&expr).unwrap();

    match result {
        ConstValue::Array(rows) => {
            assert_eq!(rows.len(), 2);
            match &rows[0] {
                ConstValue::Array(row) => {
                    assert_eq!(row.len(), 2);
                    assert_eq!(row[0], ConstValue::Int(1));
                }
                _ => panic!("Expected nested array"),
            }
        }
        _ => panic!("Expected array"),
    }
}

#[test]
fn test_eval_tuple() {
    let mut eval = ConstEvaluator::new();

    let elements: List<Expr> = vec![int_lit(1), int_lit(2), int_lit(3)].into();
    let expr = Expr::new(ExprKind::Tuple(elements), Span::dummy());

    let result = eval.eval(&expr).unwrap();

    match result {
        ConstValue::Tuple(tup) => {
            assert_eq!(tup.len(), 3);
            assert_eq!(tup[0], ConstValue::Int(1));
            assert_eq!(tup[1], ConstValue::Int(2));
            assert_eq!(tup[2], ConstValue::Int(3));
        }
        _ => panic!("Expected tuple"),
    }
}

#[test]
fn test_eval_tuple_index() {
    let mut eval = ConstEvaluator::new();

    // Create tuple (10, 20, 30)
    let elements: List<Expr> = vec![int_lit(10), int_lit(20), int_lit(30)].into();
    let tuple = Expr::new(ExprKind::Tuple(elements), Span::dummy());

    // Index: tuple.1
    let expr = Expr::new(
        ExprKind::TupleIndex {
            expr: Box::new(tuple),
            index: 1,
        },
        Span::dummy(),
    );

    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(20));
}

#[test]
fn test_eval_parenthesized() {
    let mut eval = ConstEvaluator::new();

    // (42)
    let inner = int_lit(42);
    let expr = Expr::new(ExprKind::Paren(Box::new(inner)), Span::dummy());

    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Int(42));
}

// ============================================================================
// Error Cases Tests (7 tests)
// ============================================================================

#[test]
fn test_eval_division_by_zero() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Div, int_lit(10), int_lit(0));
    let result = eval.eval(&expr);
    assert!(matches!(result, Err(ConstEvalError::DivisionByZero)));
}

#[test]
fn test_eval_overflow_addition() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(
        BinOp::Add,
        Expr::new(
            ExprKind::Literal(Literal::int(i128::MAX, Span::dummy())),
            Span::dummy(),
        ),
        int_lit(1),
    );
    let result = eval.eval(&expr);
    assert!(matches!(result, Err(ConstEvalError::Overflow { .. })));
}

#[test]
fn test_eval_unbound_variable() {
    let mut eval = ConstEvaluator::new();

    use verum_ast::ty::{Ident, Path};
    let path = Path::single(Ident::new("undefined".to_string(), Span::dummy()));
    let expr = Expr::new(ExprKind::Path(path), Span::dummy());

    let result = eval.eval(&expr);
    assert!(matches!(
        result,
        Err(ConstEvalError::UnboundVariable { .. })
    ));
}

#[test]
fn test_eval_index_out_of_bounds() {
    let mut eval = ConstEvaluator::new();

    // Create array [1, 2, 3]
    let elements: List<Expr> = vec![int_lit(1), int_lit(2), int_lit(3)].into();
    let array = Expr::new(ExprKind::Array(ArrayExpr::List(elements)), Span::dummy());

    // Try to access index 10
    let expr = Expr::new(
        ExprKind::Index {
            expr: Box::new(array),
            index: Box::new(int_lit(10)),
        },
        Span::dummy(),
    );

    let result = eval.eval(&expr);
    assert!(matches!(
        result,
        Err(ConstEvalError::IndexOutOfBounds { .. })
    ));
}

#[test]
fn test_eval_type_error_add_bool() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Add, bool_lit(true), bool_lit(false));
    let result = eval.eval(&expr);
    assert!(matches!(result, Err(ConstEvalError::TypeError { .. })));
}

#[test]
fn test_eval_type_error_not_int() {
    let mut eval = ConstEvaluator::new();
    let expr = unary(UnOp::Not, int_lit(42));
    let result = eval.eval(&expr);
    assert!(matches!(result, Err(ConstEvalError::TypeError { .. })));
}

#[test]
fn test_eval_not_constant() {
    let mut eval = ConstEvaluator::new();

    // Function call to unknown function returns NotConstant or FunctionNotFound
    // (depending on whether meta interpreter is configured)
    use verum_ast::ty::{Ident, Path};
    let path = Path::single(Ident::new("func".to_string(), Span::dummy()));
    let func = Expr::new(ExprKind::Path(path), Span::dummy());
    let expr = Expr::new(
        ExprKind::Call { type_args: vec![].into(),
            func: Box::new(func),
            args: List::new(),
        },
        Span::dummy(),
    );

    let result = eval.eval(&expr);
    // With meta interpreter integration, unknown functions return UndefinedFunction
    // Previously this was NotConstant, but now we properly track user functions
    assert!(
        matches!(result, Err(ConstEvalError::NotConstant))
            || matches!(result, Err(ConstEvalError::UndefinedFunction { .. })),
        "Expected NotConstant or UndefinedFunction, got {:?}",
        result
    );
}

// ============================================================================
// Display Tests (1 test)
// ============================================================================

#[test]
fn test_const_value_display() {
    use verum_common::Text;

    assert_eq!(format!("{}", ConstValue::Int(42)), "42");
    assert_eq!(format!("{}", ConstValue::UInt(100)), "100u");
    assert_eq!(format!("{}", ConstValue::Float(3.14)), "3.14");
    assert_eq!(format!("{}", ConstValue::Bool(true)), "true");
    assert_eq!(
        format!("{}", ConstValue::Text(Text::from("hello"))),
        "\"hello\""
    );
    assert_eq!(format!("{}", ConstValue::Char('x')), "'x'");
    assert_eq!(
        format!(
            "{}",
            ConstValue::Array(List::from(vec![
                ConstValue::Int(1),
                ConstValue::Int(2),
                ConstValue::Int(3)
            ]))
        ),
        "[1, 2, 3]"
    );
    assert_eq!(
        format!(
            "{}",
            ConstValue::Tuple(List::from(vec![ConstValue::Int(1), ConstValue::Bool(true)]))
        ),
        "(1, true)"
    );
}
