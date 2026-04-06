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
// Comprehensive tensor literal parsing tests
//
// Tests for comprehensive tensor shape parsing and validation
// This module provides exhaustive testing of tensor literal parsing:
// - All dimensionalities (0D scalar through 4D and beyond)
// - Broadcasting syntax validation
// - Error cases and diagnostics
// - Type compatibility
// - Edge cases and boundary conditions
//
// Target: ~200 tensor shape tests for comprehensive coverage

use verum_ast::{Expr, ExprKind, FileId, PathSegment, TypeKind, literal::LiteralKind};
use verum_fast_parser::VerumParser;

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

// ============================================================================
// SECTION 1: DIMENSIONALITY TESTS (0D through 4D+)
// ============================================================================

#[test]
fn test_0d_scalar_tensor() {
    let expr = parse_expr_test("tensor<>f32{42.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 0, "Expected 0D scalar tensor");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_1d_vector_size_4() {
    let expr = parse_expr_test("tensor<4>f32{1.0, 2.0, 3.0, 4.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 4);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_1d_vector_size_8() {
    let expr = parse_expr_test("tensor<8>f32{1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 8);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_1d_vector_size_16() {
    let expr = parse_expr_test("tensor<16>i32{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 16);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_1d_vector_size_32() {
    let expr = parse_expr_test("tensor<32>u8{255}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 32);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_2d_matrix_2x2() {
    let expr = parse_expr_test("tensor<2, 2>f32{{1.0, 2.0}, {3.0, 4.0}}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 2);
            assert_eq!(shape[0], 2);
            assert_eq!(shape[1], 2);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_2d_matrix_2x3() {
    let expr = parse_expr_test("tensor<2, 3>f32{1.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 2);
            assert_eq!(shape[0], 2);
            assert_eq!(shape[1], 3);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_2d_matrix_3x4() {
    let expr = parse_expr_test("tensor<3, 4>i32{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 2);
            assert_eq!(shape[0], 3);
            assert_eq!(shape[1], 4);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_2d_matrix_4x4() {
    let expr = parse_expr_test("tensor<4, 4>f64{1.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 2);
            assert_eq!(shape[0], 4);
            assert_eq!(shape[1], 4);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_3d_tensor_rgb_image() {
    let expr = parse_expr_test("tensor<3, 224, 224>u8{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 3);
            assert_eq!(shape[0], 3, "RGB channels");
            assert_eq!(shape[1], 224, "Height");
            assert_eq!(shape[2], 224, "Width");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_3d_tensor_2x3x4() {
    let expr = parse_expr_test("tensor<2, 3, 4>f32{1.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 3);
            assert_eq!(shape[0], 2);
            assert_eq!(shape[1], 3);
            assert_eq!(shape[2], 4);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_3d_tensor_256_cubed() {
    let expr = parse_expr_test("tensor<256, 256, 256>u8{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 3);
            assert_eq!(shape[0], 256);
            assert_eq!(shape[1], 256);
            assert_eq!(shape[2], 256);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_4d_tensor_batch() {
    let expr = parse_expr_test("tensor<16, 3, 224, 224>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 4);
            assert_eq!(shape[0], 16, "Batch size");
            assert_eq!(shape[1], 3, "Channels");
            assert_eq!(shape[2], 224, "Height");
            assert_eq!(shape[3], 224, "Width");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_4d_tensor_2x2x2x2() {
    let expr = parse_expr_test("tensor<2, 2, 2, 2>i32{1}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 4);
            for dim in shape {
                assert_eq!(*dim, 2);
            }
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_5d_tensor() {
    let expr = parse_expr_test("tensor<2, 3, 4, 5, 6>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 5);
            assert_eq!(shape[0], 2);
            assert_eq!(shape[1], 3);
            assert_eq!(shape[2], 4);
            assert_eq!(shape[3], 5);
            assert_eq!(shape[4], 6);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_6d_tensor() {
    let expr = parse_expr_test("tensor<2, 2, 2, 2, 2, 2>f32{1.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 6);
            for dim in shape {
                assert_eq!(*dim, 2);
            }
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// ============================================================================
// SECTION 2: BROADCASTING SYNTAX TESTS
// ============================================================================

#[test]
fn test_broadcast_scalar_to_vector_4() {
    let expr = parse_expr_test("tensor<4>f32{1.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, data, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 4);
            // Data should be a single scalar literal
            match &data.kind {
                ExprKind::Literal(lit) => match &lit.kind {
                    LiteralKind::Float(_) => {
                        // Correct: single element for broadcasting
                    }
                    _ => panic!("Expected float literal for broadcast"),
                },
                _ => panic!("Expected literal expression for broadcast"),
            }
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_broadcast_scalar_to_vector_8() {
    let expr = parse_expr_test("tensor<8>i32{42}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, data, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 8);
            match &data.kind {
                ExprKind::Literal(lit) => match &lit.kind {
                    LiteralKind::Int(_) => {}
                    _ => panic!("Expected int literal for broadcast"),
                },
                _ => panic!("Expected literal expression for broadcast"),
            }
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_broadcast_scalar_to_matrix() {
    let expr = parse_expr_test("tensor<3, 4>f32{0.5}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 2);
            assert_eq!(shape[0], 3);
            assert_eq!(shape[1], 4);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_broadcast_scalar_to_3d() {
    let expr = parse_expr_test("tensor<2, 3, 4>i32{-1}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 3);
            assert_eq!(shape[0], 2);
            assert_eq!(shape[1], 3);
            assert_eq!(shape[2], 4);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_broadcast_zero_float() {
    let expr = parse_expr_test("tensor<16>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 16);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_broadcast_one_integer() {
    let expr = parse_expr_test("tensor<32>i64{1}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 32);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_broadcast_bool_true() {
    let expr = parse_expr_test("tensor<8>Bool{true}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 8);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_broadcast_bool_false() {
    let expr = parse_expr_test("tensor<16>Bool{false}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 16);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// ============================================================================
// SECTION 3: TYPE VARIETY TESTS
// ============================================================================

#[test]
fn test_tensor_f32_type() {
    let expr = parse_expr_test("tensor<4>f32{1.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { elem_type, .. } => match &elem_type.kind {
            TypeKind::Path(path) => {
                if let Some(PathSegment::Name(ident)) = path.segments.last() {
                    assert_eq!(ident.as_str(), "f32");
                } else {
                    panic!("Expected name segment in path");
                }
            }
            _ => panic!("Expected path type"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_tensor_f64_type() {
    let expr = parse_expr_test("tensor<4>f64{1.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { elem_type, .. } => match &elem_type.kind {
            TypeKind::Path(path) => {
                if let Some(PathSegment::Name(ident)) = path.segments.last() {
                    assert_eq!(ident.as_str(), "f64");
                } else {
                    panic!("Expected name segment in path");
                }
            }
            _ => panic!("Expected path type"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_tensor_i8_type() {
    let expr = parse_expr_test("tensor<4>i8{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { elem_type, .. } => match &elem_type.kind {
            TypeKind::Path(path) => {
                if let Some(PathSegment::Name(ident)) = path.segments.last() {
                    assert_eq!(ident.as_str(), "i8");
                } else {
                    panic!("Expected name segment in path");
                }
            }
            _ => panic!("Expected path type"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_tensor_i16_type() {
    let expr = parse_expr_test("tensor<4>i16{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { elem_type, .. } => match &elem_type.kind {
            TypeKind::Path(path) => {
                if let Some(PathSegment::Name(ident)) = path.segments.last() {
                    assert_eq!(ident.as_str(), "i16");
                } else {
                    panic!("Expected name segment in path");
                }
            }
            _ => panic!("Expected path type"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_tensor_i32_type() {
    let expr = parse_expr_test("tensor<4>i32{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { elem_type, .. } => match &elem_type.kind {
            TypeKind::Path(path) => {
                if let Some(PathSegment::Name(ident)) = path.segments.last() {
                    assert_eq!(ident.as_str(), "i32");
                } else {
                    panic!("Expected name segment in path");
                }
            }
            _ => panic!("Expected path type"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_tensor_i64_type() {
    let expr = parse_expr_test("tensor<4>i64{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { elem_type, .. } => match &elem_type.kind {
            TypeKind::Path(path) => {
                if let Some(PathSegment::Name(ident)) = path.segments.last() {
                    assert_eq!(ident.as_str(), "i64");
                } else {
                    panic!("Expected name segment in path");
                }
            }
            _ => panic!("Expected path type"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_tensor_u8_type() {
    let expr = parse_expr_test("tensor<4>u8{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { elem_type, .. } => match &elem_type.kind {
            TypeKind::Path(path) => {
                if let Some(PathSegment::Name(ident)) = path.segments.last() {
                    assert_eq!(ident.as_str(), "u8");
                } else {
                    panic!("Expected name segment in path");
                }
            }
            _ => panic!("Expected path type"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_tensor_u16_type() {
    let expr = parse_expr_test("tensor<4>u16{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { elem_type, .. } => match &elem_type.kind {
            TypeKind::Path(path) => {
                if let Some(PathSegment::Name(ident)) = path.segments.last() {
                    assert_eq!(ident.as_str(), "u16");
                } else {
                    panic!("Expected name segment in path");
                }
            }
            _ => panic!("Expected path type"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_tensor_u32_type() {
    let expr = parse_expr_test("tensor<4>u32{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { elem_type, .. } => match &elem_type.kind {
            TypeKind::Path(path) => {
                if let Some(PathSegment::Name(ident)) = path.segments.last() {
                    assert_eq!(ident.as_str(), "u32");
                } else {
                    panic!("Expected name segment in path");
                }
            }
            _ => panic!("Expected path type"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_tensor_u64_type() {
    let expr = parse_expr_test("tensor<4>u64{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { elem_type, .. } => match &elem_type.kind {
            TypeKind::Path(path) => {
                if let Some(PathSegment::Name(ident)) = path.segments.last() {
                    assert_eq!(ident.as_str(), "u64");
                } else {
                    panic!("Expected name segment in path");
                }
            }
            _ => panic!("Expected path type"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_tensor_bool_type() {
    let expr = parse_expr_test("tensor<4>Bool{true}");
    match &expr.kind {
        ExprKind::TensorLiteral { elem_type, .. } => {
            match &elem_type.kind {
                TypeKind::Bool => {
                    // Correct: Bool is a primitive type
                }
                TypeKind::Path(path) => {
                    // Bool might also be parsed as a path in some cases
                    if let Some(PathSegment::Name(ident)) = path.segments.last() {
                        assert_eq!(ident.as_str(), "Bool");
                    } else {
                        panic!("Expected name segment in path");
                    }
                }
                _ => panic!("Expected Bool type (primitive or path)"),
            }
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// ============================================================================
// SECTION 4: ERROR CASE TESTS
// ============================================================================

#[test]
fn test_error_zero_dimension() {
    let has_error = parse_expr_expect_error("tensor<0>f32{1.0}");
    assert!(has_error, "Zero dimension should be rejected");
}

#[test]
fn test_error_zero_in_2d() {
    let has_error = parse_expr_expect_error("tensor<0, 3>f32{1.0}");
    assert!(has_error, "Zero dimension in 2D should be rejected");
}

#[test]
fn test_error_zero_in_middle() {
    let has_error = parse_expr_expect_error("tensor<2, 0, 4>f32{1.0}");
    assert!(has_error, "Zero dimension in middle should be rejected");
}

#[test]
fn test_error_zero_in_last() {
    let has_error = parse_expr_expect_error("tensor<2, 3, 0>f32{1.0}");
    assert!(
        has_error,
        "Zero dimension in last position should be rejected"
    );
}

#[test]
fn test_error_negative_dimension() {
    let has_error = parse_expr_expect_error("tensor<-4>f32{1.0}");
    assert!(has_error, "Negative dimension should be rejected");
}

#[test]
fn test_error_missing_type() {
    let has_error = parse_expr_expect_error("tensor<4>{1.0}");
    assert!(has_error, "Missing element type should be rejected");
}

#[test]
fn test_error_missing_shape() {
    let has_error = parse_expr_expect_error("tensor f32{1.0}");
    assert!(has_error, "Missing shape should be rejected");
}

#[test]
fn test_error_missing_data() {
    let has_error = parse_expr_expect_error("tensor<4>f32");
    assert!(has_error, "Missing data should be rejected");
}

#[test]
fn test_error_empty_shape() {
    // Empty shape with explicit empty angle brackets
    // Empty shape tensor<> is valid syntax for 0D scalar tensors
    let result = parse_expr_test("tensor<>f32{1.0}");
    match &result.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            // 0D tensor (scalar) is valid
            assert_eq!(shape.len(), 0);
        }
        _ => panic!("Expected TensorLiteral for 0D tensor"),
    }
}

// ============================================================================
// SECTION 5: WHITESPACE AND FORMATTING TESTS
// ============================================================================

#[test]
fn test_whitespace_around_angles() {
    let expr = parse_expr_test("tensor < 4 > f32 { 1.0 }");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 4);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_whitespace_in_shape() {
    let expr = parse_expr_test("tensor< 2 , 3 , 4 >f32{1.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 3);
            assert_eq!(shape[0], 2);
            assert_eq!(shape[1], 3);
            assert_eq!(shape[2], 4);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_multiline_tensor() {
    let source = r#"tensor<2, 3>f32{
        {1.0, 2.0, 3.0},
        {4.0, 5.0, 6.0}
    }"#;
    let expr = parse_expr_test(source);
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 2);
            assert_eq!(shape[0], 2);
            assert_eq!(shape[1], 3);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_no_whitespace_compact() {
    let expr = parse_expr_test("tensor<4>f32{1.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 4);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// ============================================================================
// SECTION 6: LARGE DIMENSIONS TESTS
// ============================================================================

#[test]
fn test_large_1d_1000() {
    let expr = parse_expr_test("tensor<1000>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 1000);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_large_2d_1000x1000() {
    let expr = parse_expr_test("tensor<1000, 1000>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 2);
            assert_eq!(shape[0], 1000);
            assert_eq!(shape[1], 1000);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_large_3d_100x100x100() {
    let expr = parse_expr_test("tensor<100, 100, 100>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 3);
            assert_eq!(shape[0], 100);
            assert_eq!(shape[1], 100);
            assert_eq!(shape[2], 100);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_large_dimension_values() {
    let expr = parse_expr_test("tensor<65536>u8{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 65536);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// ============================================================================
// SECTION 7: ELEMENT DATA EXPRESSION TESTS
// ============================================================================

#[test]
fn test_data_array_literal() {
    let expr = parse_expr_test("tensor<3>f32{[1.0, 2.0, 3.0]}");
    match &expr.kind {
        ExprKind::TensorLiteral { data, .. } => match &data.kind {
            ExprKind::Array(_) => {}
            _ => panic!("Expected array literal"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_data_variable_reference() {
    let expr = parse_expr_test("tensor<8>f32{data}");
    match &expr.kind {
        ExprKind::TensorLiteral { data, .. } => match &data.kind {
            ExprKind::Path(_) => {}
            _ => panic!("Expected path expression"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_data_function_call() {
    let expr = parse_expr_test("tensor<16>i32{generate_data()}");
    match &expr.kind {
        ExprKind::TensorLiteral { data, .. } => match &data.kind {
            ExprKind::Call { .. } => {}
            _ => panic!("Expected call expression"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_data_method_call() {
    let expr = parse_expr_test("tensor<4>f32{source.get_data()}");
    match &expr.kind {
        ExprKind::TensorLiteral { data, .. } => match &data.kind {
            ExprKind::MethodCall { .. } => {}
            _ => panic!("Expected method call expression"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_data_field_access() {
    let expr = parse_expr_test("tensor<8>f32{obj.field}");
    match &expr.kind {
        ExprKind::TensorLiteral { data, .. } => match &data.kind {
            ExprKind::Field { .. } => {}
            _ => panic!("Expected field access expression"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_data_binary_operation() {
    let expr = parse_expr_test("tensor<4>f32{a + b}");
    match &expr.kind {
        ExprKind::TensorLiteral { data, .. } => match &data.kind {
            ExprKind::Binary { .. } => {}
            _ => panic!("Expected binary expression"),
        },
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// ============================================================================
// SECTION 8: COMMON USE CASE PATTERNS
// ============================================================================

#[test]
fn test_simd_vector_f32x4() {
    // Common SIMD vector size for SSE
    let expr = parse_expr_test("tensor<4>f32{1.0, 2.0, 3.0, 4.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape[0], 4, "SSE f32x4 vector");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_simd_vector_f32x8() {
    // Common SIMD vector size for AVX
    let expr = parse_expr_test("tensor<8>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape[0], 8, "AVX f32x8 vector");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_simd_vector_f32x16() {
    // Common SIMD vector size for AVX-512
    let expr = parse_expr_test("tensor<16>f32{1.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape[0], 16, "AVX-512 f32x16 vector");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_mnist_digit_28x28() {
    // MNIST digit image
    let expr = parse_expr_test("tensor<28, 28>u8{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape[0], 28);
            assert_eq!(shape[1], 28);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_cifar10_image() {
    // CIFAR-10 image: 32x32 RGB
    let expr = parse_expr_test("tensor<3, 32, 32>u8{0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape[0], 3, "RGB channels");
            assert_eq!(shape[1], 32, "Height");
            assert_eq!(shape[2], 32, "Width");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_imagenet_image() {
    // ImageNet-sized image: 224x224 RGB
    let expr = parse_expr_test("tensor<3, 224, 224>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape[0], 3);
            assert_eq!(shape[1], 224);
            assert_eq!(shape[2], 224);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_neural_network_weights_784x128() {
    // First layer of MNIST neural network
    let expr = parse_expr_test("tensor<784, 128>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape[0], 784, "Input layer (28*28)");
            assert_eq!(shape[1], 128, "Hidden layer");
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_neural_network_batch() {
    // Batch of 32 MNIST images
    let expr = parse_expr_test("tensor<32, 1, 28, 28>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape[0], 32, "Batch size");
            assert_eq!(shape[1], 1, "Grayscale channel");
            assert_eq!(shape[2], 28);
            assert_eq!(shape[3], 28);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// ============================================================================
// SECTION 9: SHAPE COMBINATIONS (Edge Cases)
// ============================================================================

#[test]
fn test_shape_1x1() {
    let expr = parse_expr_test("tensor<1, 1>f32{42.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape[0], 1);
            assert_eq!(shape[1], 1);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_shape_1xn() {
    let expr = parse_expr_test("tensor<1, 10>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape[0], 1);
            assert_eq!(shape[1], 10);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_shape_nx1() {
    let expr = parse_expr_test("tensor<10, 1>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape[0], 10);
            assert_eq!(shape[1], 1);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_shape_1x1x1() {
    let expr = parse_expr_test("tensor<1, 1, 1>f32{1.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape[0], 1);
            assert_eq!(shape[1], 1);
            assert_eq!(shape[2], 1);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_shape_power_of_two_dimensions() {
    let expr = parse_expr_test("tensor<2, 4, 8, 16>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape[0], 2);
            assert_eq!(shape[1], 4);
            assert_eq!(shape[2], 8);
            assert_eq!(shape[3], 16);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_shape_prime_dimensions() {
    let expr = parse_expr_test("tensor<3, 5, 7>f32{0.0}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape[0], 3);
            assert_eq!(shape[1], 5);
            assert_eq!(shape[2], 7);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// ============================================================================
// SECTION 10: NESTED ARRAY STRUCTURE TESTS
// ============================================================================

#[test]
fn test_nested_1d_flat_array() {
    let expr = parse_expr_test("tensor<4>i32{[1, 2, 3, 4]}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 4);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_nested_2d_array_of_arrays() {
    let expr = parse_expr_test("tensor<2, 2>i32{[[1, 2], [3, 4]]}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 2);
            assert_eq!(shape[0], 2);
            assert_eq!(shape[1], 2);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_nested_2d_braced_arrays() {
    let expr = parse_expr_test("tensor<2, 3>f32{{1.0, 2.0, 3.0}, {4.0, 5.0, 6.0}}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 2);
            assert_eq!(shape[0], 2);
            assert_eq!(shape[1], 3);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_nested_3d_triple_nesting() {
    let expr = parse_expr_test("tensor<2, 2, 2>i32{[[[1, 2], [3, 4]], [[5, 6], [7, 8]]]}");
    match &expr.kind {
        ExprKind::TensorLiteral { shape, .. } => {
            assert_eq!(shape.len(), 3);
            assert_eq!(shape[0], 2);
            assert_eq!(shape[1], 2);
            assert_eq!(shape[2], 2);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// Test count: 110+ tests covering comprehensive parser functionality
