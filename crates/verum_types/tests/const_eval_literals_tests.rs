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
// Tests for const evaluation of Float, Text, and Char literals
//
// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Unified compile-time computation
//
// These tests verify that compile-time constant evaluation works correctly
// for all literal types: Int, Float, Bool, Text (String), and Char.

use verum_ast::{
    expr::{BinOp, Expr, ExprKind, UnOp},
    literal::Literal,
    span::Span,
};
use verum_common::{ConstValue, Text};
use verum_types::const_eval::ConstEvaluator;

// Helper to create float literal
fn float_lit(n: f64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::float(n, Span::dummy())),
        Span::dummy(),
    )
}

// Helper to create text literal
fn text_lit(s: &str) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::string(s.to_string().into(), Span::dummy())),
        Span::dummy(),
    )
}

// Helper to create char literal
fn char_lit(c: char) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::char(c, Span::dummy())),
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
// Float Literal Tests
// ============================================================================

#[test]
fn test_eval_float_literal() {
    let mut eval = ConstEvaluator::new();
    let expr = float_lit(3.14);
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Float(3.14));
}

#[test]
fn test_eval_float_addition() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Add, float_lit(2.5), float_lit(3.7));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Float(6.2));
}

#[test]
fn test_eval_float_subtraction() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Sub, float_lit(10.5), float_lit(3.2));
    let result = eval.eval(&expr).unwrap();
    assert!((result.as_f64().unwrap() - 7.3).abs() < 1e-10);
}

#[test]
fn test_eval_float_multiplication() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Mul, float_lit(2.5), float_lit(4.0));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Float(10.0));
}

#[test]
fn test_eval_float_division() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Div, float_lit(10.0), float_lit(4.0));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Float(2.5));
}

#[test]
fn test_eval_float_remainder() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Rem, float_lit(7.5), float_lit(2.0));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Float(1.5));
}

#[test]
fn test_eval_float_negation() {
    let mut eval = ConstEvaluator::new();
    let expr = unary(UnOp::Neg, float_lit(3.14));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Float(-3.14));
}

#[test]
fn test_eval_float_comparison_lt() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Lt, float_lit(3.14), float_lit(3.15));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_float_comparison_gt() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Gt, float_lit(5.0), float_lit(4.9));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_float_comparison_eq() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Eq, float_lit(2.5), float_lit(2.5));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_float_complex_expression() {
    let mut eval = ConstEvaluator::new();
    // (3.0 + 2.0) * 4.0 / 2.0
    let add = binary(BinOp::Add, float_lit(3.0), float_lit(2.0));
    let mul = binary(BinOp::Mul, add, float_lit(4.0));
    let expr = binary(BinOp::Div, mul, float_lit(2.0));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Float(10.0));
}

// ============================================================================
// Text Literal Tests
// ============================================================================

#[test]
fn test_eval_text_literal() {
    let mut eval = ConstEvaluator::new();
    let expr = text_lit("hello");
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Text(Text::from("hello")));
}

#[test]
fn test_eval_text_empty() {
    let mut eval = ConstEvaluator::new();
    let expr = text_lit("");
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Text(Text::from("")));
}

#[test]
fn test_eval_text_concatenation() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Add, text_lit("hello"), text_lit(" world"));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Text(Text::from("hello world")));
}

#[test]
fn test_eval_text_comparison_eq() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Eq, text_lit("hello"), text_lit("hello"));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_text_comparison_ne() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Ne, text_lit("hello"), text_lit("world"));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_text_comparison_lt() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Lt, text_lit("apple"), text_lit("banana"));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_text_comparison_gt() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Gt, text_lit("zebra"), text_lit("apple"));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_text_display() {
    let value = ConstValue::Text(Text::from("hello"));
    assert_eq!(format!("{}", value), "\"hello\"");
}

#[test]
fn test_eval_text_as_text() {
    let value = ConstValue::Text(Text::from("hello"));
    assert_eq!(value.as_text().unwrap().as_str(), "hello");
}

// ============================================================================
// Char Literal Tests
// ============================================================================

#[test]
fn test_eval_char_literal() {
    let mut eval = ConstEvaluator::new();
    let expr = char_lit('a');
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Char('a'));
}

#[test]
fn test_eval_char_unicode() {
    let mut eval = ConstEvaluator::new();
    let expr = char_lit('λ');
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Char('λ'));
}

#[test]
fn test_eval_char_comparison_eq() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Eq, char_lit('a'), char_lit('a'));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_char_comparison_ne() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Ne, char_lit('a'), char_lit('b'));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_char_comparison_lt() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Lt, char_lit('a'), char_lit('b'));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_char_comparison_gt() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Gt, char_lit('z'), char_lit('a'));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_char_comparison_le() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Le, char_lit('a'), char_lit('a'));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_char_comparison_ge() {
    let mut eval = ConstEvaluator::new();
    let expr = binary(BinOp::Ge, char_lit('b'), char_lit('b'));
    let result = eval.eval(&expr).unwrap();
    assert_eq!(result, ConstValue::Bool(true));
}

#[test]
fn test_eval_char_display() {
    let value = ConstValue::Char('x');
    assert_eq!(format!("{}", value), "'x'");
}

#[test]
fn test_eval_char_as_char() {
    let value = ConstValue::Char('x');
    assert_eq!(value.as_char_value().unwrap(), 'x');
}

// ============================================================================
// Mixed Type Tests
// ============================================================================

#[test]
fn test_const_value_display_all_types() {
    assert_eq!(format!("{}", ConstValue::Int(42)), "42");
    assert_eq!(format!("{}", ConstValue::UInt(100)), "100u");
    assert_eq!(format!("{}", ConstValue::Float(3.14)), "3.14");
    assert_eq!(format!("{}", ConstValue::Bool(true)), "true");
    assert_eq!(
        format!("{}", ConstValue::Text(Text::from("hello"))),
        "\"hello\""
    );
    assert_eq!(format!("{}", ConstValue::Char('a')), "'a'");
}

#[test]
fn test_as_float_conversions() {
    // Float to float
    let f = ConstValue::Float(3.14);
    assert_eq!(f.as_f64().unwrap(), 3.14);

    // Int to float
    let i = ConstValue::Int(42);
    assert_eq!(i.as_f64().unwrap(), 42.0);

    // UInt to float
    let u = ConstValue::UInt(100);
    assert_eq!(u.as_f64().unwrap(), 100.0);

    // Non-numeric types
    let b = ConstValue::Bool(true);
    assert!(b.as_f64().is_none());

    let t = ConstValue::Text(Text::from("hello"));
    assert!(t.as_f64().is_none());

    let c = ConstValue::Char('a');
    assert!(c.as_f64().is_none());
}

// ============================================================================
// Array with Mixed Literal Types
// ============================================================================

#[test]
fn test_eval_float_array() {
    let mut eval = ConstEvaluator::new();

    use verum_ast::expr::ArrayExpr;
    let elements = vec![float_lit(1.1), float_lit(2.2), float_lit(3.3)];
    let expr = Expr::new(ExprKind::Array(ArrayExpr::List(elements.into())), Span::dummy());

    let result = eval.eval(&expr).unwrap();

    match result {
        ConstValue::Array(arr) => {
            assert_eq!(arr.len(), 3);
            assert_eq!(arr[0], ConstValue::Float(1.1));
            assert_eq!(arr[1], ConstValue::Float(2.2));
            assert_eq!(arr[2], ConstValue::Float(3.3));
        }
        _ => panic!("Expected array"),
    }
}

#[test]
fn test_eval_text_array() {
    let mut eval = ConstEvaluator::new();

    use verum_ast::expr::ArrayExpr;
    let elements = vec![text_lit("hello"), text_lit("world"), text_lit("!")];
    let expr = Expr::new(ExprKind::Array(ArrayExpr::List(elements.into())), Span::dummy());

    let result = eval.eval(&expr).unwrap();

    match result {
        ConstValue::Array(arr) => {
            assert_eq!(arr.len(), 3);
            assert_eq!(arr[0], ConstValue::Text(Text::from("hello")));
            assert_eq!(arr[1], ConstValue::Text(Text::from("world")));
            assert_eq!(arr[2], ConstValue::Text(Text::from("!")));
        }
        _ => panic!("Expected array"),
    }
}

#[test]
fn test_eval_char_array() {
    let mut eval = ConstEvaluator::new();

    use verum_ast::expr::ArrayExpr;
    let elements = vec![char_lit('a'), char_lit('b'), char_lit('c')];
    let expr = Expr::new(ExprKind::Array(ArrayExpr::List(elements.into())), Span::dummy());

    let result = eval.eval(&expr).unwrap();

    match result {
        ConstValue::Array(arr) => {
            assert_eq!(arr.len(), 3);
            assert_eq!(arr[0], ConstValue::Char('a'));
            assert_eq!(arr[1], ConstValue::Char('b'));
            assert_eq!(arr[2], ConstValue::Char('c'));
        }
        _ => panic!("Expected array"),
    }
}
