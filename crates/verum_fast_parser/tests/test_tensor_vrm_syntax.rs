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
// Test tensor literal parsing with full Verum syntax
use verum_ast::{FileId, expr::ExprKind};
use verum_fast_parser::VerumParser;

fn parse_test(code: &str) -> Result<verum_ast::expr::Expr, verum_common::List<verum_fast_parser::error::ParseError>> {
    let parser = VerumParser::new();
    parser.parse_expr_str(code, FileId::dummy())
}

#[test]
fn test_tensor_1d_float_comma_sep() {
    let code = "tensor<4>Float { 1.0, 2.0, 3.0, 4.0 }";
    match parse_test(code) {
        Ok(expr) => match &expr.kind {
            ExprKind::TensorLiteral { shape, .. } => {
                assert_eq!(shape.len(), 1);
                assert_eq!(shape[0], 4);
            }
            _ => panic!("Expected TensorLiteral, got {:?}", expr.kind),
        },
        Err(errors) => {
            eprintln!("Parse failed with {} errors:", errors.len());
            for (i, err) in errors.iter().enumerate() {
                eprintln!("  Error {}: {}", i + 1, err);
            }
            panic!("Parsing failed");
        }
    }
}

#[test]
fn test_tensor_2d_float_comma_sep() {
    let code = "tensor<2, 3>Float { 1.0, 2.0, 3.0, 4.0, 5.0, 6.0 }";
    match parse_test(code) {
        Ok(expr) => match &expr.kind {
            ExprKind::TensorLiteral { shape, .. } => {
                assert_eq!(shape.len(), 2);
                assert_eq!(shape[0], 2);
                assert_eq!(shape[1], 3);
            }
            _ => panic!("Expected TensorLiteral, got {:?}", expr.kind),
        },
        Err(errors) => {
            eprintln!("Parse failed with {} errors:", errors.len());
            for (i, err) in errors.iter().enumerate() {
                eprintln!("  Error {}: {}", i + 1, err);
            }
            panic!("Parsing failed");
        }
    }
}

#[test]
fn test_tensor_2d_nested_braces() {
    let code = "tensor<2, 3>Float { { 1.0, 2.0, 3.0 }, { 4.0, 5.0, 6.0 } }";
    match parse_test(code) {
        Ok(expr) => match &expr.kind {
            ExprKind::TensorLiteral { shape, .. } => {
                assert_eq!(shape.len(), 2);
                assert_eq!(shape[0], 2);
                assert_eq!(shape[1], 3);
            }
            _ => panic!("Expected TensorLiteral, got {:?}", expr.kind),
        },
        Err(errors) => {
            eprintln!("Parse failed with {} errors:", errors.len());
            for (i, err) in errors.iter().enumerate() {
                eprintln!("  Error {}: {}", i + 1, err);
            }
            panic!("Parsing failed");
        }
    }
}

#[test]
fn test_tensor_int() {
    let code = "tensor<4>Int { 1, 2, 3, 4 }";
    match parse_test(code) {
        Ok(expr) => match &expr.kind {
            ExprKind::TensorLiteral { shape, .. } => {
                assert_eq!(shape.len(), 1);
                assert_eq!(shape[0], 4);
            }
            _ => panic!("Expected TensorLiteral, got {:?}", expr.kind),
        },
        Err(errors) => {
            eprintln!("Parse failed with {} errors:", errors.len());
            for (i, err) in errors.iter().enumerate() {
                eprintln!("  Error {}: {}", i + 1, err);
            }
            panic!("Parsing failed");
        }
    }
}
