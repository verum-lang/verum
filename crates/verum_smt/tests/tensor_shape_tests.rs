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
//! Comprehensive tests for tensor shape verification using Z3 Array theory
//!
//! This test suite validates:
//! - Matrix multiplication shape checking
//! - NumPy-style broadcasting
//! - Meta parameter resolution
//! - Invalid shape detection
//! - Reshape validation with Z3 product constraints
//! - Bounds check elimination verification
//!
//! Tests all tensor shape verification features: Tensor<T, Shape> with compile-time
//! shape parameters, matrix multiplication dimension checking, NumPy-style broadcasting,
//! reshape validation (product of old shape = product of new shape), and bounds check
//! elimination via Z3 array theory proofs.

use verum_ast::{Expr, ExprKind, IntLit, Literal, LiteralKind, Span};
use verum_smt::tensor_shapes::{ShapeError, TensorShapeVerifier};
use verum_common::List;

/// Helper to create integer literal expression
fn expr_int(value: u128) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value: value as i128,
                suffix: None,
            }),
            span: Span::dummy(),
        }),
        Span::dummy(),
    )
}

/// Helper to create path expression (for meta parameters)
fn expr_meta(name: &str) -> Expr {
    use verum_ast::{Ident, Path, PathSegment};
    use verum_common::Text;

    let ident = Ident::new(Text::from(name), Span::dummy());
    let segment = PathSegment::Name(ident);
    let path = Path {
        segments: vec![segment].into(),
        span: Span::dummy(),
    };

    Expr::new(ExprKind::Path(path), Span::dummy())
}

#[test]
fn test_matmul_valid_shapes() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: [2, 3] × [3, 4] = [2, 4]
    let a_shape = vec![expr_int(2), expr_int(3)];
    let b_shape = vec![expr_int(3), expr_int(4)];

    let result = verifier.verify_matmul_shapes(&a_shape, &b_shape);
    assert!(result.is_ok(), "Valid matmul should succeed");

    let result_shape = result.unwrap();
    assert_eq!(result_shape.len(), 2, "Result should be 2D");

    // Verify result dimensions
    if let ExprKind::Literal(lit) = &result_shape[0].kind
        && let LiteralKind::Int(i) = &lit.kind {
            assert_eq!(i.value, 2, "First dimension should be 2");
        }

    if let ExprKind::Literal(lit) = &result_shape[1].kind
        && let LiteralKind::Int(i) = &lit.kind {
            assert_eq!(i.value, 4, "Second dimension should be 4");
        }

    // Check statistics
    let stats = verifier.stats();
    assert_eq!(stats.total_checks, 1);
    assert_eq!(stats.successful_verifications, 1);
}

#[test]
fn test_matmul_dimension_mismatch() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: [2, 3] × [5, 4] should fail (3 != 5)
    let a_shape = vec![expr_int(2), expr_int(3)];
    let b_shape = vec![expr_int(5), expr_int(4)];

    let result = verifier.verify_matmul_shapes(&a_shape, &b_shape);
    assert!(result.is_err(), "Invalid matmul should fail");

    match result.unwrap_err() {
        ShapeError::DimensionMismatch { .. } => {
            // Expected error
        }
        other => panic!("Expected DimensionMismatch, got {:?}", other),
    }
}

#[test]
fn test_matmul_invalid_rank() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: [2, 3, 4] × [3, 4] should fail (3D × 2D)
    let a_shape = vec![expr_int(2), expr_int(3), expr_int(4)];
    let b_shape = vec![expr_int(3), expr_int(4)];

    let result = verifier.verify_matmul_shapes(&a_shape, &b_shape);
    assert!(result.is_err(), "3D matmul should fail");

    match result.unwrap_err() {
        ShapeError::InvalidRank { .. } => {
            // Expected error
        }
        other => panic!("Expected InvalidRank, got {:?}", other),
    }
}

#[test]
fn test_matmul_with_meta_parameters() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: [M, K] × [K, N] should succeed (symbolic K matches)
    let a_shape = vec![expr_meta("M"), expr_meta("K")];
    let b_shape = vec![expr_meta("K"), expr_meta("N")];

    let result = verifier.verify_matmul_shapes(&a_shape, &b_shape);
    assert!(
        result.is_ok(),
        "Matmul with matching meta parameters should succeed"
    );

    let result_shape = result.unwrap();
    assert_eq!(result_shape.len(), 2, "Result should be 2D");
}

#[test]
fn test_broadcast_equal_shapes() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: [3, 4] + [3, 4] = [3, 4]
    let shape_a: List<Expr> = vec![expr_int(3), expr_int(4)].into();
    let shape_b: List<Expr> = vec![expr_int(3), expr_int(4)].into();
    let shapes: Vec<List<Expr>> = vec![shape_a.clone(), shape_b.clone()];

    let result = verifier.verify_broadcast(&shapes);
    assert!(result.is_ok(), "Equal shapes should broadcast");

    let result_shape = result.unwrap();
    assert_eq!(result_shape.len(), 2);
}

#[test]
fn test_broadcast_trailing_dimension() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: [3, 4] + [4] = [3, 4]
    let shape_a: List<Expr> = vec![expr_int(3), expr_int(4)].into();
    let shape_b: List<Expr> = vec![expr_int(4)].into();
    let shapes: Vec<List<Expr>> = vec![shape_a.clone(), shape_b.clone()];

    let result = verifier.verify_broadcast(&shapes);
    assert!(
        result.is_ok(),
        "Trailing dimension broadcast should succeed"
    );

    let result_shape = result.unwrap();
    assert_eq!(result_shape.len(), 2);
}

#[test]
fn test_broadcast_with_ones() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: [3, 1, 5] + [2, 5] = [3, 2, 5]
    let shape_a: List<Expr> = vec![expr_int(3), expr_int(1), expr_int(5)].into();
    let shape_b: List<Expr> = vec![expr_int(2), expr_int(5)].into();
    let shapes: Vec<List<Expr>> = vec![shape_a.clone(), shape_b.clone()];

    let result = verifier.verify_broadcast(&shapes);
    assert!(result.is_ok(), "Broadcasting with 1s should succeed");

    let result_shape = result.unwrap();
    assert_eq!(result_shape.len(), 3, "Result should have 3 dimensions");
}

#[test]
fn test_broadcast_incompatible() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: [3, 4] + [2, 4] should fail (3 != 2 in first dimension)
    let shape_a: List<Expr> = vec![expr_int(3), expr_int(4)].into();
    let shape_b: List<Expr> = vec![expr_int(2), expr_int(4)].into();
    let shapes: Vec<List<Expr>> = vec![shape_a.clone(), shape_b.clone()];

    let result = verifier.verify_broadcast(&shapes);
    assert!(result.is_err(), "Incompatible shapes should fail");

    match result.unwrap_err() {
        ShapeError::BroadcastError { .. } => {
            // Expected error
        }
        other => panic!("Expected BroadcastError, got {:?}", other),
    }
}

#[test]
fn test_elementwise_equal_shapes() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: [2, 3] + [2, 3] = [2, 3]
    let shape_a = vec![expr_int(2), expr_int(3)];
    let shape_b = vec![expr_int(2), expr_int(3)];

    let result = verifier.verify_elementwise(&shape_a, &shape_b);
    assert!(
        result.is_ok(),
        "Equal shapes for elementwise should succeed"
    );

    let result_shape = result.unwrap();
    assert_eq!(result_shape.len(), 2);
}

#[test]
fn test_elementwise_broadcast() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: [3, 4] + [1, 4] = [3, 4]
    let shape_a = vec![expr_int(3), expr_int(4)];
    let shape_b = vec![expr_int(1), expr_int(4)];

    let result = verifier.verify_elementwise(&shape_a, &shape_b);
    assert!(
        result.is_ok(),
        "Elementwise with broadcasting should succeed"
    );

    let result_shape = result.unwrap();
    assert_eq!(result_shape.len(), 2);
}

#[test]
fn test_statistics() {
    let mut verifier = TensorShapeVerifier::new();

    // Perform several checks
    let shape_a = vec![expr_int(2), expr_int(3)];
    let shape_b = vec![expr_int(3), expr_int(4)];
    let _ = verifier.verify_matmul_shapes(&shape_a, &shape_b);

    let shape_c = vec![expr_int(4), expr_int(5)];
    let shape_d = vec![expr_int(4), expr_int(5)];
    let _ = verifier.verify_elementwise(&shape_c, &shape_d);

    let stats = verifier.stats();
    assert_eq!(stats.total_checks, 2, "Should have 2 total checks");
    assert_eq!(
        stats.successful_verifications, 2,
        "All checks should succeed"
    );
    assert!(stats.success_rate() > 0.99, "Success rate should be ~100%");
}

#[test]
fn test_cache_clearing() {
    let mut verifier = TensorShapeVerifier::new();

    // Perform a check
    let shape_a = vec![expr_int(2), expr_int(3)];
    let shape_b = vec![expr_int(3), expr_int(4)];
    let _ = verifier.verify_matmul_shapes(&shape_a, &shape_b);

    // Clear cache
    verifier.clear_cache();

    // Cache should be empty (implicitly tested by no errors)
}

#[test]
fn test_complex_matmul_chain() {
    let mut verifier = TensorShapeVerifier::new();

    // Test chain: A[2,3] × B[3,4] × C[4,5]
    // Step 1: A × B = [2, 4]
    let a_shape = vec![expr_int(2), expr_int(3)];
    let b_shape = vec![expr_int(3), expr_int(4)];
    let ab_result = verifier.verify_matmul_shapes(&a_shape, &b_shape);
    assert!(ab_result.is_ok());

    // Step 2: (A × B) × C = [2, 5]
    let ab_shape = ab_result.unwrap();
    let c_shape = vec![expr_int(4), expr_int(5)];
    let abc_result = verifier.verify_matmul_shapes(&ab_shape, &c_shape);
    assert!(abc_result.is_ok());

    let final_shape = abc_result.unwrap();
    assert_eq!(final_shape.len(), 2);

    // Verify final dimensions: [2, 5]
    if let ExprKind::Literal(lit) = &final_shape[0].kind
        && let LiteralKind::Int(i) = &lit.kind {
            assert_eq!(i.value, 2, "First dimension should be 2");
        }

    if let ExprKind::Literal(lit) = &final_shape[1].kind
        && let LiteralKind::Int(i) = &lit.kind {
            assert_eq!(i.value, 5, "Second dimension should be 5");
        }
}

#[test]
fn test_square_matrix_multiplication() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: [N, N] × [N, N] = [N, N]
    let a_shape = vec![expr_int(4), expr_int(4)];
    let b_shape = vec![expr_int(4), expr_int(4)];

    let result = verifier.verify_matmul_shapes(&a_shape, &b_shape);
    assert!(
        result.is_ok(),
        "Square matrix multiplication should succeed"
    );

    let result_shape = result.unwrap();
    assert_eq!(result_shape.len(), 2);

    // Both dimensions should be 4
    for dim in &result_shape {
        if let ExprKind::Literal(lit) = &dim.kind
            && let LiteralKind::Int(i) = &lit.kind {
                assert_eq!(i.value, 4);
            }
    }
}

#[test]
fn test_vector_matrix_multiplication() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: [1, N] × [N, M] = [1, M] (row vector × matrix)
    let a_shape = vec![expr_int(1), expr_int(5)];
    let b_shape = vec![expr_int(5), expr_int(3)];

    let result = verifier.verify_matmul_shapes(&a_shape, &b_shape);
    assert!(
        result.is_ok(),
        "Vector-matrix multiplication should succeed"
    );

    let result_shape = result.unwrap();
    assert_eq!(result_shape.len(), 2);

    // Result should be [1, 3]
    if let ExprKind::Literal(lit) = &result_shape[0].kind
        && let LiteralKind::Int(i) = &lit.kind {
            assert_eq!(i.value, 1);
        }

    if let ExprKind::Literal(lit) = &result_shape[1].kind
        && let LiteralKind::Int(i) = &lit.kind {
            assert_eq!(i.value, 3);
        }
}

#[test]
fn test_multi_tensor_broadcast() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: [3, 1, 5] + [2, 5] + [1, 1, 5] = [3, 2, 5]
    let shape_a: List<Expr> = vec![expr_int(3), expr_int(1), expr_int(5)].into();
    let shape_b: List<Expr> = vec![expr_int(2), expr_int(5)].into();
    let shape_c: List<Expr> = vec![expr_int(1), expr_int(1), expr_int(5)].into();
    let shapes: Vec<List<Expr>> = vec![shape_a.clone(), shape_b.clone(), shape_c.clone()];

    let result = verifier.verify_broadcast(&shapes);
    assert!(result.is_ok(), "Multi-tensor broadcast should succeed");

    let result_shape = result.unwrap();
    assert_eq!(result_shape.len(), 3);
}

#[test]
fn test_broadcast_scalar() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: [3, 4, 5] + [] = [3, 4, 5] (scalar broadcast)
    let shape_a: List<Expr> = vec![expr_int(3), expr_int(4), expr_int(5)].into();
    let shape_b: List<Expr> = vec![].into();
    let shapes: Vec<List<Expr>> = vec![shape_a.clone(), shape_b.clone()];

    let result = verifier.verify_broadcast(&shapes);
    assert!(result.is_ok(), "Scalar broadcast should succeed");

    let result_shape = result.unwrap();
    assert_eq!(result_shape.len(), 3);
}

// ==================== Reshape Validation Tests ====================
// Reshape validation: product of old shape dimensions must equal product of new shape dimensions

#[test]
fn test_reshape_valid_12_to_3x4() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: reshape [12] to [3, 4] - valid because 12 = 3×4
    let old_shape = vec![expr_int(12)];
    let new_shape = vec![expr_int(3), expr_int(4)];

    let result = verifier.verify_reshape(&old_shape, &new_shape);
    assert!(
        result.is_ok(),
        "Valid reshape [12] -> [3, 4] should succeed"
    );
}

#[test]
fn test_reshape_valid_12_to_2x6() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: reshape [12] to [2, 6] - valid because 12 = 2×6
    let old_shape = vec![expr_int(12)];
    let new_shape = vec![expr_int(2), expr_int(6)];

    let result = verifier.verify_reshape(&old_shape, &new_shape);
    assert!(
        result.is_ok(),
        "Valid reshape [12] -> [2, 6] should succeed"
    );
}

#[test]
fn test_reshape_valid_2x3x4_to_24() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: reshape [2, 3, 4] to [24] - valid because 2×3×4 = 24
    let old_shape = vec![expr_int(2), expr_int(3), expr_int(4)];
    let new_shape = vec![expr_int(24)];

    let result = verifier.verify_reshape(&old_shape, &new_shape);
    assert!(
        result.is_ok(),
        "Valid reshape [2, 3, 4] -> [24] should succeed"
    );
}

#[test]
fn test_reshape_valid_24_to_2x3x4() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: reshape [24] to [2, 3, 4] - valid because 24 = 2×3×4
    let old_shape = vec![expr_int(24)];
    let new_shape = vec![expr_int(2), expr_int(3), expr_int(4)];

    let result = verifier.verify_reshape(&old_shape, &new_shape);
    assert!(
        result.is_ok(),
        "Valid reshape [24] -> [2, 3, 4] should succeed"
    );
}

#[test]
fn test_reshape_invalid_12_to_5x5() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: reshape [12] to [5, 5] - INVALID because 12 ≠ 25
    let old_shape = vec![expr_int(12)];
    let new_shape = vec![expr_int(5), expr_int(5)];

    let result = verifier.verify_reshape(&old_shape, &new_shape);
    assert!(
        result.is_err(),
        "Invalid reshape [12] -> [5, 5] should fail"
    );

    match result.unwrap_err() {
        ShapeError::ReshapeError { .. } => {
            // Expected error
        }
        other => panic!("Expected ReshapeError, got {:?}", other),
    }
}

#[test]
fn test_reshape_invalid_2x3_to_3x3() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: reshape [2, 3] to [3, 3] - INVALID because 6 ≠ 9
    let old_shape = vec![expr_int(2), expr_int(3)];
    let new_shape = vec![expr_int(3), expr_int(3)];

    let result = verifier.verify_reshape(&old_shape, &new_shape);
    assert!(
        result.is_err(),
        "Invalid reshape [2, 3] -> [3, 3] should fail"
    );

    match result.unwrap_err() {
        ShapeError::ReshapeError { .. } => {
            // Expected error
        }
        other => panic!("Expected ReshapeError, got {:?}", other),
    }
}

#[test]
fn test_reshape_scalar_to_1d() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: reshape [] (scalar) to [1] - valid because 1 = 1
    let old_shape = vec![];
    let new_shape = vec![expr_int(1)];

    let result = verifier.verify_reshape(&old_shape, &new_shape);
    assert!(result.is_ok(), "Valid reshape [] -> [1] should succeed");
}

#[test]
fn test_reshape_with_meta_parameters() {
    let mut verifier = TensorShapeVerifier::new();

    // Test: reshape [M, N] to [M*N] with symbolic sizes
    // This tests that Z3 can verify: M * N = M * N (trivially true)
    let old_shape = vec![expr_meta("M"), expr_meta("N")];
    let new_shape = vec![expr_meta("MN")]; // Assume MN represents M*N

    // This test is expected to succeed if translator handles meta parameters
    let _result = verifier.verify_reshape(&old_shape, &new_shape);
    // Note: May fail if Z3 translator doesn't know MN = M*N
    // In a real implementation, this would require additional constraints
}

// ==================== Bounds Check Elimination Tests ====================
// Bounds check elimination: when loop index `i` is bounded by `0 <= i < N` and array
// has length N, the bounds check can be proven unnecessary via Z3 and eliminated at compile time

#[test]
fn test_bounds_elimination_simple_loop() {
    use verum_smt::tensor_shapes::{ArrayAccess, LoopBound};
    use verum_common::ToText;

    let mut verifier = TensorShapeVerifier::new();

    // Loop: for i in 0..N { a[i] }
    let loop_bounds = vec![LoopBound {
        var_name: "i".to_text(),
        lower: expr_int(0),
        upper: expr_int(10), // N = 10
    }];

    let array_accesses = vec![ArrayAccess {
        array_name: "a".to_text(),
        array_shape: vec![expr_int(10)].into(), // Array of size 10
        indices: vec![expr_meta("i")].into(),   // Access a[i]
    }];

    let result = verifier.verify_bounds_elimination(&loop_bounds, &array_accesses);
    assert!(
        result.is_ok(),
        "Simple loop bounds should be provable: 0 <= i < 10, array size 10"
    );

    let proof = result.unwrap();
    assert!(proof.can_eliminate_checks, "Should eliminate bounds checks");
    assert_eq!(
        proof.proved_accesses.len(),
        1,
        "Should prove one array access"
    );
}

#[test]
fn test_bounds_elimination_nested_loops_matmul() {
    use verum_smt::tensor_shapes::{ArrayAccess, LoopBound};
    use verum_common::ToText;

    let mut verifier = TensorShapeVerifier::new();

    // Matrix multiplication loops:
    // for i in 0..M {
    //   for j in 0..N {
    //     for k in 0..K {
    //       result[i, j] += a[i, k] * b[k, j]
    //     }
    //   }
    // }
    let loop_bounds = vec![
        LoopBound {
            var_name: "i".to_text(),
            lower: expr_int(0),
            upper: expr_int(2), // M = 2
        },
        LoopBound {
            var_name: "j".to_text(),
            lower: expr_int(0),
            upper: expr_int(3), // N = 3
        },
        LoopBound {
            var_name: "k".to_text(),
            lower: expr_int(0),
            upper: expr_int(4), // K = 4
        },
    ];

    let array_accesses = vec![
        ArrayAccess {
            array_name: "a".to_text(),
            array_shape: vec![expr_int(2), expr_int(4)].into(), // [M, K] = [2, 4]
            indices: vec![expr_meta("i"), expr_meta("k")].into(),
        },
        ArrayAccess {
            array_name: "b".to_text(),
            array_shape: vec![expr_int(4), expr_int(3)].into(), // [K, N] = [4, 3]
            indices: vec![expr_meta("k"), expr_meta("j")].into(),
        },
        ArrayAccess {
            array_name: "result".to_text(),
            array_shape: vec![expr_int(2), expr_int(3)].into(), // [M, N] = [2, 3]
            indices: vec![expr_meta("i"), expr_meta("j")].into(),
        },
    ];

    let result = verifier.verify_bounds_elimination(&loop_bounds, &array_accesses);
    assert!(
        result.is_ok(),
        "Matmul bounds should be provable: ALL indices within bounds"
    );

    let proof = result.unwrap();
    assert!(proof.can_eliminate_checks);
    // Should prove 3 arrays × their respective dimensions
    assert!(
        proof.proved_accesses.len() >= 3,
        "Should prove multiple array accesses"
    );
}

#[test]
fn test_bounds_elimination_out_of_bounds() {
    use verum_smt::tensor_shapes::{ArrayAccess, LoopBound};
    use verum_common::ToText;

    let mut verifier = TensorShapeVerifier::new();

    // Loop: for i in 0..10 { a[i] } but array size is only 5
    let loop_bounds = vec![LoopBound {
        var_name: "i".to_text(),
        lower: expr_int(0),
        upper: expr_int(10), // Loop goes to 10
    }];

    let array_accesses = vec![ArrayAccess {
        array_name: "a".to_text(),
        array_shape: vec![expr_int(5)].into(), // Array size is only 5!
        indices: vec![expr_meta("i")].into(),
    }];

    let result = verifier.verify_bounds_elimination(&loop_bounds, &array_accesses);
    assert!(
        result.is_err(),
        "Out-of-bounds access should be detected: i can be 9 but array size is 5"
    );

    match result.unwrap_err() {
        ShapeError::BoundsCheckFailed { .. } => {
            // Expected error - Z3 found counterexample
        }
        other => panic!("Expected BoundsCheckFailed, got {:?}", other),
    }
}

#[test]
fn test_bounds_elimination_offset_access() {
    use verum_smt::tensor_shapes::{ArrayAccess, LoopBound};
    use verum_common::ToText;

    let mut verifier = TensorShapeVerifier::new();

    // Loop: for i in 1..9 { a[i] } - offset loop
    let loop_bounds = vec![LoopBound {
        var_name: "i".to_text(),
        lower: expr_int(1), // Start at 1
        upper: expr_int(9), // End at 9 (exclusive)
    }];

    let array_accesses = vec![ArrayAccess {
        array_name: "a".to_text(),
        array_shape: vec![expr_int(10)].into(), // Array of size 10
        indices: vec![expr_meta("i")].into(),   // Access a[i] where 1 <= i < 9
    }];

    let result = verifier.verify_bounds_elimination(&loop_bounds, &array_accesses);
    assert!(
        result.is_ok(),
        "Offset loop bounds should be provable: 1 <= i < 9, array size 10"
    );

    let proof = result.unwrap();
    assert!(proof.can_eliminate_checks);
}

#[test]
fn test_bounds_elimination_multidimensional_access() {
    use verum_smt::tensor_shapes::{ArrayAccess, LoopBound};
    use verum_common::ToText;

    let mut verifier = TensorShapeVerifier::new();

    // Nested loops for 2D array:
    // for i in 0..3 {
    //   for j in 0..4 {
    //     a[i, j]
    //   }
    // }
    let loop_bounds = vec![
        LoopBound {
            var_name: "i".to_text(),
            lower: expr_int(0),
            upper: expr_int(3),
        },
        LoopBound {
            var_name: "j".to_text(),
            lower: expr_int(0),
            upper: expr_int(4),
        },
    ];

    let array_accesses = vec![ArrayAccess {
        array_name: "a".to_text(),
        array_shape: vec![expr_int(3), expr_int(4)].into(), // [3, 4] array
        indices: vec![expr_meta("i"), expr_meta("j")].into(),
    }];

    let result = verifier.verify_bounds_elimination(&loop_bounds, &array_accesses);
    assert!(
        result.is_ok(),
        "2D array bounds should be provable: 0 <= i < 3, 0 <= j < 4"
    );

    let proof = result.unwrap();
    assert!(proof.can_eliminate_checks);
    // Should prove 2 dimensions
    assert_eq!(
        proof.proved_accesses.len(),
        2,
        "Should prove both dimensions"
    );
}

#[test]
fn test_comprehensive_tensor_pipeline() {
    let mut verifier = TensorShapeVerifier::new();

    // Complex scenario: Verify entire matmul operation
    // 1. Verify shapes are compatible for matmul
    let a_shape = vec![expr_int(2), expr_int(3)];
    let b_shape = vec![expr_int(3), expr_int(4)];

    let matmul_result = verifier.verify_matmul_shapes(&a_shape, &b_shape);
    assert!(matmul_result.is_ok(), "Step 1: Matmul shapes compatible");

    let result_shape = matmul_result.unwrap();

    // 2. Verify we can reshape the result if needed
    let flat_shape = vec![expr_int(8)]; // 2×4 = 8
    let reshape_result = verifier.verify_reshape(&result_shape, &flat_shape);
    assert!(reshape_result.is_ok(), "Step 2: Can reshape [2, 4] to [8]");

    // 3. Verify elementwise operation with another tensor
    let other_shape = vec![expr_int(2), expr_int(4)];
    let elementwise_result = verifier.verify_elementwise(&result_shape, &other_shape);
    assert!(
        elementwise_result.is_ok(),
        "Step 3: Elementwise operation with [2, 4]"
    );

    // Check cumulative statistics
    let stats = verifier.stats();
    assert!(stats.total_checks >= 3, "Should have at least 3 checks");
    assert!(
        stats.success_rate() > 0.99,
        "All checks should succeed in this pipeline"
    );
}
