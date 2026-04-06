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
// Tests for tensor literal parsing
//
// Tests for tensor literal parsing: `tensor<shape>dtype{values}`
// This module tests parsing of tensor literal expressions:
// - 1D tensors: tensor<4>f32{1.0, 2.0, 3.0, 4.0}
// - 2D tensors: tensor<2, 3>i32{{1, 2, 3}, {4, 5, 6}}
// - 3D tensors: tensor<3, 224, 224>u8{...}
// - Validation: dimensions > 0

use verum_ast::{Expr, ExprKind, FileId};
use verum_parser::VerumParser;

fn parse_expr_test(source: &str) -> Expr {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_expr_str(source, file_id)
        .unwrap_or_else(|_| panic!("Failed to parse: {}", source))
}

fn parse_expr_expect_error(source: &str) -> bool {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser.parse_expr_str(source, file_id).is_err()
}

// === 1D TENSOR TESTS ===

#[test]
fn test_parse_1d_tensor_f32() {
    let expr = parse_expr_test("tensor<4>f32{1.0}");
    match &expr.kind {
        ExprKind::TensorLiteral {
            shape,
            elem_type: _,
            data: _,
        } => {
            assert_eq!(shape.len(), 1, "Expected 1D tensor");
            assert_eq!(shape[0], 4, "Expected dimension 4");
            // elem_type and data are validated separately
        }
        _ => panic!("Expected TensorLiteral expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_1d_tensor_i32() {
    let expr = parse_expr_test("tensor<8>i32{42}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1, "Expected 1D tensor");
            assert_eq!(shape[0], 8, "Expected dimension 8");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_parse_1d_tensor_bool() {
    let expr = parse_expr_test("tensor<10>Bool{true}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1, "Expected 1D tensor");
            assert_eq!(shape[0], 10, "Expected dimension 10");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// === 2D TENSOR TESTS ===

#[test]
fn test_parse_2d_tensor() {
    let expr = parse_expr_test("tensor<2, 3>f32{1.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 2, "Expected 2D tensor");
            assert_eq!(shape[0], 2, "Expected first dimension 2");
            assert_eq!(shape[1], 3, "Expected second dimension 3");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_parse_2d_tensor_nested_array() {
    let source = "tensor<2, 3>i32{[1, 2, 3]}";
    let expr = parse_expr_test(source);
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 2, "Expected 2D tensor");
            assert_eq!(shape[0], 2, "Expected first dimension 2");
            assert_eq!(shape[1], 3, "Expected second dimension 3");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// === 3D TENSOR TESTS ===

#[test]
fn test_parse_3d_tensor() {
    let expr = parse_expr_test("tensor<3, 224, 224>u8{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 3, "Expected 3D tensor");
            assert_eq!(shape[0], 3, "Expected first dimension 3");
            assert_eq!(shape[1], 224, "Expected second dimension 224");
            assert_eq!(shape[2], 224, "Expected third dimension 224");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_parse_3d_tensor_rgb_image() {
    let expr = parse_expr_test("tensor<3, 256, 256>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 3, "Expected 3D tensor");
            assert_eq!(shape[0], 3, "Expected 3 color channels");
            assert_eq!(shape[1], 256, "Expected height 256");
            assert_eq!(shape[2], 256, "Expected width 256");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// === HIGH-DIMENSIONAL TENSOR TESTS ===

#[test]
fn test_parse_4d_tensor() {
    let expr = parse_expr_test("tensor<32, 3, 64, 64>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 4, "Expected 4D tensor");
            assert_eq!(shape[0], 32, "Expected batch size 32");
            assert_eq!(shape[1], 3, "Expected 3 channels");
            assert_eq!(shape[2], 64, "Expected height 64");
            assert_eq!(shape[3], 64, "Expected width 64");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// === DATA EXPRESSION TESTS ===

#[test]
fn test_parse_tensor_with_array_literal() {
    let expr = parse_expr_test("tensor<3>f32{[1.0, 2.0, 3.0]}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, data, .. } => {
            assert_eq!(shape.len(), 1, "Expected 1D tensor");
            assert_eq!(shape[0], 3, "Expected dimension 3");
            // Data should be an array expression
            match &data.kind {
                ExprKind::Array(_) => {
                    // Correct: data is an array literal
                }
                _ => panic!("Expected array expression for data"),
            }
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_parse_tensor_with_variable() {
    let expr = parse_expr_test("tensor<4>f32{data}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, data, .. } => {
            assert_eq!(shape.len(), 1, "Expected 1D tensor");
            assert_eq!(shape[0], 4, "Expected dimension 4");
            // Data should be a path expression (variable reference)
            match &data.kind {
                ExprKind::Path(_) => {
                    // Correct: data is a variable reference
                }
                _ => panic!("Expected path expression for data"),
            }
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_parse_tensor_with_function_call() {
    let expr = parse_expr_test("tensor<10>i32{generate_data()}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, data, .. } => {
            assert_eq!(shape.len(), 1, "Expected 1D tensor");
            assert_eq!(shape[0], 10, "Expected dimension 10");
            // Data should be a call expression
            match &data.kind {
                ExprKind::Call { .. } => {
                    // Correct: data is a function call
                }
                _ => panic!("Expected call expression for data"),
            }
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// === VALIDATION TESTS ===

#[test]
fn test_tensor_zero_dimension_error() {
    // Dimension 0 should be rejected
    let has_error = parse_expr_expect_error("tensor<0>f32{1.0}");
    assert!(has_error, "Expected error for zero dimension");
}

#[test]
fn test_tensor_zero_in_multidim_error() {
    // Zero dimension in multi-dimensional tensor should be rejected
    let has_error = parse_expr_expect_error("tensor<2, 0, 3>f32{1.0}");
    assert!(
        has_error,
        "Expected error for zero dimension in multi-dim tensor"
    );
}

// === TYPE TESTS ===

#[test]
fn test_parse_tensor_various_types() {
    let type_tests = vec![
        ("tensor<4>f32{0.0}", 4),
        ("tensor<4>f64{0.0}", 4),
        ("tensor<4>i8{0}", 4),
        ("tensor<4>i16{0}", 4),
        ("tensor<4>i32{0}", 4),
        ("tensor<4>i64{0}", 4),
        ("tensor<4>u8{0}", 4),
        ("tensor<4>u16{0}", 4),
        ("tensor<4>u32{0}", 4),
        ("tensor<4>u64{0}", 4),
        ("tensor<4>Bool{true}", 4),
    ];

    for (source, expected_dim) in type_tests {
        let expr = parse_expr_test(source);
        match &expr.kind {
            ExprKind::TensorLiteral { shape, .. } => {
                assert_eq!(shape.len(), 1, "Expected 1D tensor for {}", source);
                assert_eq!(
                    shape[0], expected_dim,
                    "Expected dimension {} for {}",
                    expected_dim, source
                );
            }
            _ => panic!("Expected TensorLiteral expression for {}", source),
        }
    }
}

// === LARGE DIMENSION TESTS ===

#[test]
fn test_parse_tensor_large_dimensions() {
    let expr = parse_expr_test("tensor<1000, 1000>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 2, "Expected 2D tensor");
            assert_eq!(shape[0], 1000, "Expected dimension 1000");
            assert_eq!(shape[1], 1000, "Expected dimension 1000");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// === WHITESPACE HANDLING ===

#[test]
fn test_parse_tensor_with_whitespace() {
    let expr = parse_expr_test("tensor < 4 > f32 { 1.0 }");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1, "Expected 1D tensor");
            assert_eq!(shape[0], 4, "Expected dimension 4");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_parse_tensor_multiline() {
    let source = r#"tensor<2, 3>f32{
        [1.0, 2.0, 3.0]
    }"#;
    let expr = parse_expr_test(source);
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 2, "Expected 2D tensor");
            assert_eq!(shape[0], 2, "Expected dimension 2");
            assert_eq!(shape[1], 3, "Expected dimension 3");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}
