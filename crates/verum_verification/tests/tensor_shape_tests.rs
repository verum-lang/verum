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
// Comprehensive test suite for tensor shape verification
//
// Tests cover:
// - Static shape checking
// - Dynamic shape inference with meta parameters
// - Broadcasting compatibility
// - Matrix operations (matmul, transpose, reshape)
// - Reduction operations
// - Concatenation
// - Error cases
//
// Tensor Shape Verification:
// Uses meta parameters for compile-time dimension tracking and SMT-based
// verification for shape compatibility proofs. Key operations:
// - Compile-time shape inference: Tensor<T, Shape: meta [usize]>
// - Matrix multiplication: [M,K] x [K,N] -> [M,N] (inner dimension K must match)
// - Element-wise ops: require matching shapes (same Shape parameter)
// - Broadcasting: NumPy-style rules verified at compile time
// - Reductions: e.g., [M,N] -> [M] (reduce along dimension)
// - Integration with refinement types for constraints (non-empty, square, positive-definite)
// Performance: shape checks eliminated in AOT, meta parameters erased at runtime (0 bytes)

use verum_common::{List, Maybe, Text};
use verum_verification::tensor_shapes::*;

// ============================================================================
// Static Shape Tests
// ============================================================================

#[test]
fn test_static_shape_creation() {
    let shape = TensorShape::from_dims(vec![2, 3, 4]);
    assert_eq!(shape.rank(), 3);
    assert!(shape.is_fully_static());
    assert_eq!(shape.static_dims(), Maybe::Some(List::from(vec![2, 3, 4])));
    assert_eq!(shape.to_string(), "[2, 3, 4]");
}

#[test]
fn test_static_shape_compatibility() {
    let shape1 = TensorShape::from_dims(vec![128, 256]);
    let shape2 = TensorShape::from_dims(vec![128, 256]);
    let shape3 = TensorShape::from_dims(vec![128, 512]);

    assert!(shape1.is_compatible_with(&shape2));
    assert!(!shape1.is_compatible_with(&shape3));
}

#[test]
fn test_static_matmul_success() {
    let verifier = ShapeVerifier::new();
    let shape_a = TensorShape::from_dims(vec![128, 256]);
    let shape_b = TensorShape::from_dims(vec![256, 512]);

    let result = verifier.verify_matmul(&shape_a, &shape_b).unwrap();
    assert_eq!(result.rank(), 2);
    assert_eq!(
        result.static_dims(),
        Maybe::Some(List::from(vec![128, 512]))
    );
}

#[test]
fn test_static_matmul_dimension_mismatch() {
    let verifier = ShapeVerifier::new();
    let shape_a = TensorShape::from_dims(vec![128, 256]);
    let shape_b = TensorShape::from_dims(vec![512, 1024]); // 512 != 256

    let result = verifier.verify_matmul(&shape_a, &shape_b);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ShapeError::DimensionMismatch { .. }
    ));
}

#[test]
fn test_static_matmul_invalid_rank() {
    let verifier = ShapeVerifier::new();
    let shape_a = TensorShape::from_dims(vec![128]); // rank 1, not 2
    let shape_b = TensorShape::from_dims(vec![256, 512]);

    let result = verifier.verify_matmul(&shape_a, &shape_b);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ShapeError::InvalidOperation { .. }
    ));
}

// ============================================================================
// Dynamic Shape Tests (Meta Parameters)
// ============================================================================

#[test]
fn test_dynamic_shape_creation() {
    let mut shape = TensorShape::new();
    shape.add_dynamic_dim("M");
    shape.add_dynamic_dim("N");
    shape.add_dynamic_dim("K");

    assert_eq!(shape.rank(), 3);
    assert!(!shape.is_fully_static());
    assert_eq!(shape.static_dims(), Maybe::None);
    assert_eq!(shape.to_string(), "[M, N, K]");
}

#[test]
fn test_meta_parameter_binding() {
    let mut shape = TensorShape::new();
    shape.add_dynamic_dim("M");
    shape.add_dynamic_dim("N");
    shape.bind_meta_param("M", 128);
    shape.bind_meta_param("N", 256);

    assert_eq!(shape.get_meta_param(&Text::from("M")), Maybe::Some(128));
    assert_eq!(shape.get_meta_param(&Text::from("N")), Maybe::Some(256));
    assert_eq!(shape.meta_params.len(), 2);
}

#[test]
fn test_dynamic_shape_resolution() {
    let mut shape = TensorShape::new();
    shape.add_dynamic_dim("M");
    shape.add_static_dim(256);
    shape.add_dynamic_dim("K");
    shape.bind_meta_param("M", 128);
    shape.bind_meta_param("K", 64);

    let resolved = shape.resolve().unwrap();
    assert!(resolved.is_fully_static());
    assert_eq!(
        resolved.static_dims(),
        Maybe::Some(List::from(vec![128, 256, 64]))
    );
}

#[test]
fn test_dynamic_shape_unresolved_error() {
    let mut shape = TensorShape::new();
    shape.add_dynamic_dim("M");
    shape.add_dynamic_dim("N");
    shape.bind_meta_param("M", 128);
    // N is not bound

    let result = shape.resolve();
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ShapeError::UnresolvedDimension { .. }
    ));
}

#[test]
fn test_dynamic_matmul() {
    let verifier = ShapeVerifier::new();

    // [M, K] × [K, N] → [M, N]
    let mut shape_a = TensorShape::new();
    shape_a.add_dynamic_dim("M");
    shape_a.add_dynamic_dim("K");

    let mut shape_b = TensorShape::new();
    shape_b.add_dynamic_dim("K");
    shape_b.add_dynamic_dim("N");

    let result = verifier.verify_matmul(&shape_a, &shape_b).unwrap();
    assert_eq!(result.rank(), 2);
    assert_eq!(result.dimensions[0], Dimension::Dynamic(Text::from("M")));
    assert_eq!(result.dimensions[1], Dimension::Dynamic(Text::from("N")));
}

#[test]
fn test_mixed_static_dynamic_matmul() {
    let verifier = ShapeVerifier::new();

    // [128, K] × [K, 512] → [128, 512]
    let mut shape_a = TensorShape::new();
    shape_a.add_static_dim(128);
    shape_a.add_dynamic_dim("K");

    let mut shape_b = TensorShape::new();
    shape_b.add_dynamic_dim("K");
    shape_b.add_static_dim(512);

    let result = verifier.verify_matmul(&shape_a, &shape_b).unwrap();
    assert_eq!(result.rank(), 2);
    assert_eq!(result.dimensions[0], Dimension::Static(128));
    assert_eq!(result.dimensions[1], Dimension::Static(512));
}

// ============================================================================
// Broadcasting Tests
// ============================================================================

#[test]
fn test_broadcasting_scalar_to_matrix() {
    let verifier = ShapeVerifier::new();
    let scalar = TensorShape::from_dims(vec![1, 1]);
    let matrix = TensorShape::from_dims(vec![3, 4]);

    let result = verifier.verify_broadcast(&scalar, &matrix).unwrap();
    assert_eq!(result.static_dims(), Maybe::Some(List::from(vec![3, 4])));
}

#[test]
fn test_broadcasting_vector_to_matrix() {
    let verifier = ShapeVerifier::new();
    let vector = TensorShape::from_dims(vec![4]);
    let matrix = TensorShape::from_dims(vec![3, 4]);

    let result = verifier.verify_broadcast(&vector, &matrix).unwrap();
    assert_eq!(result.static_dims(), Maybe::Some(List::from(vec![3, 4])));
}

#[test]
fn test_broadcasting_incompatible_shapes() {
    let verifier = ShapeVerifier::new();
    let shape1 = TensorShape::from_dims(vec![3, 4]);
    let shape2 = TensorShape::from_dims(vec![3, 5]); // 4 != 5

    let result = verifier.verify_broadcast(&shape1, &shape2);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ShapeError::IncompatibleBroadcast { .. }
    ));
}

#[test]
fn test_broadcasting_matching_dimensions() {
    let verifier = ShapeVerifier::new();
    let shape1 = TensorShape::from_dims(vec![3, 1]);
    let shape2 = TensorShape::from_dims(vec![1, 4]);

    let result = verifier.verify_broadcast(&shape1, &shape2).unwrap();
    assert_eq!(result.static_dims(), Maybe::Some(List::from(vec![3, 4])));
}

#[test]
fn test_broadcasting_disabled() {
    let mut config = VerificationConfig::default();
    config.allow_broadcast = false;
    let verifier = ShapeVerifier::with_config(config);

    let shape1 = TensorShape::from_dims(vec![3, 1]);
    let shape2 = TensorShape::from_dims(vec![1, 4]);

    let result = verifier.verify_broadcast(&shape1, &shape2);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ShapeError::InvalidOperation { .. }
    ));
}

// ============================================================================
// Element-wise Operation Tests
// ============================================================================

#[test]
fn test_elementwise_matching_shapes() {
    let verifier = ShapeVerifier::new();
    let shape1 = TensorShape::from_dims(vec![128, 256]);
    let shape2 = TensorShape::from_dims(vec![128, 256]);

    let result = verifier.verify_elementwise(&shape1, &shape2).unwrap();
    assert_eq!(
        result.static_dims(),
        Maybe::Some(List::from(vec![128, 256]))
    );
}

#[test]
fn test_elementwise_mismatched_shapes() {
    let verifier = ShapeVerifier::new();
    let shape1 = TensorShape::from_dims(vec![128, 256]);
    let shape2 = TensorShape::from_dims(vec![128, 512]);

    let result = verifier.verify_elementwise(&shape1, &shape2);
    assert!(result.is_err());
}

#[test]
fn test_elementwise_mismatched_rank() {
    let verifier = ShapeVerifier::new();
    let shape1 = TensorShape::from_dims(vec![128, 256]);
    let shape2 = TensorShape::from_dims(vec![128, 256, 3]);

    let result = verifier.verify_elementwise(&shape1, &shape2);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ShapeError::ShapeMismatch { .. }
    ));
}

// ============================================================================
// Transpose Tests
// ============================================================================

#[test]
fn test_transpose_default() {
    let verifier = ShapeVerifier::new();
    let shape = TensorShape::from_dims(vec![2, 3, 4]);

    let result = verifier.verify_transpose(&shape, Maybe::None).unwrap();
    assert_eq!(result.static_dims(), Maybe::Some(List::from(vec![4, 3, 2])));
}

#[test]
fn test_transpose_custom_axes() {
    let verifier = ShapeVerifier::new();
    let shape = TensorShape::from_dims(vec![2, 3, 4]);

    let result = verifier
        .verify_transpose(&shape, Maybe::Some(List::from(vec![1, 0, 2])))
        .unwrap();
    assert_eq!(result.static_dims(), Maybe::Some(List::from(vec![3, 2, 4])));
}

#[test]
fn test_transpose_invalid_axes_length() {
    let verifier = ShapeVerifier::new();
    let shape = TensorShape::from_dims(vec![2, 3, 4]);

    let result = verifier.verify_transpose(&shape, Maybe::Some(List::from(vec![0, 1]))); // Too few
    assert!(result.is_err());
}

#[test]
fn test_transpose_duplicate_axes() {
    let verifier = ShapeVerifier::new();
    let shape = TensorShape::from_dims(vec![2, 3, 4]);

    let result = verifier.verify_transpose(&shape, Maybe::Some(List::from(vec![0, 0, 2]))); // Duplicate 0
    assert!(result.is_err());
}

// ============================================================================
// Reshape Tests
// ============================================================================

#[test]
fn test_reshape_compatible() {
    let verifier = ShapeVerifier::new();
    let input = TensorShape::from_dims(vec![2, 3, 4]);
    let target = TensorShape::from_dims(vec![6, 4]);

    let result = verifier.verify_reshape(&input, &target).unwrap();
    assert_eq!(result.static_dims(), Maybe::Some(List::from(vec![6, 4])));
}

#[test]
fn test_reshape_incompatible() {
    let verifier = ShapeVerifier::new();
    let input = TensorShape::from_dims(vec![2, 3, 4]); // 24 elements
    let target = TensorShape::from_dims(vec![5, 6]); // 30 elements

    let result = verifier.verify_reshape(&input, &target);
    assert!(result.is_err());
}

// ============================================================================
// Reduction Tests
// ============================================================================

#[test]
fn test_reduction_with_keep_dims() {
    let verifier = ShapeVerifier::new();
    let shape = TensorShape::from_dims(vec![2, 3, 4]);

    let result = verifier.verify_reduction(&shape, 1, true).unwrap();
    assert_eq!(result.static_dims(), Maybe::Some(List::from(vec![2, 1, 4])));
}

#[test]
fn test_reduction_without_keep_dims() {
    let verifier = ShapeVerifier::new();
    let shape = TensorShape::from_dims(vec![2, 3, 4]);

    let result = verifier.verify_reduction(&shape, 1, false).unwrap();
    assert_eq!(result.static_dims(), Maybe::Some(List::from(vec![2, 4])));
}

#[test]
fn test_reduction_invalid_axis() {
    let verifier = ShapeVerifier::new();
    let shape = TensorShape::from_dims(vec![2, 3, 4]);

    let result = verifier.verify_reduction(&shape, 5, false); // axis > rank
    assert!(result.is_err());
}

// ============================================================================
// Concatenation Tests
// ============================================================================

#[test]
fn test_concat_along_axis_0() {
    let verifier = ShapeVerifier::new();
    let shapes = vec![
        TensorShape::from_dims(vec![2, 3]),
        TensorShape::from_dims(vec![4, 3]),
        TensorShape::from_dims(vec![1, 3]),
    ];

    let result = verifier.verify_concat(&shapes, 0).unwrap();
    assert_eq!(result.static_dims(), Maybe::Some(List::from(vec![7, 3]))); // 2+4+1 = 7
}

#[test]
fn test_concat_along_axis_1() {
    let verifier = ShapeVerifier::new();
    let shapes = vec![
        TensorShape::from_dims(vec![3, 2]),
        TensorShape::from_dims(vec![3, 5]),
    ];

    let result = verifier.verify_concat(&shapes, 1).unwrap();
    assert_eq!(result.static_dims(), Maybe::Some(List::from(vec![3, 7]))); // 2+5 = 7
}

#[test]
fn test_concat_mismatched_dimensions() {
    let verifier = ShapeVerifier::new();
    let shapes = vec![
        TensorShape::from_dims(vec![2, 3]),
        TensorShape::from_dims(vec![2, 4]), // Different on non-concat axis
    ];

    let result = verifier.verify_concat(&shapes, 0);
    assert!(result.is_err());
}

#[test]
fn test_concat_empty_list() {
    let verifier = ShapeVerifier::new();
    let shapes: Vec<TensorShape> = vec![];

    let result = verifier.verify_concat(&shapes, 0);
    assert!(result.is_err());
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_complex_pipeline() {
    let verifier = ShapeVerifier::new();

    // Start with [128, 784]
    let input = TensorShape::from_dims(vec![128, 784]);

    // Reshape to [128, 28, 28]
    let reshaped_target = TensorShape::from_dims(vec![128, 28, 28]);
    let reshaped = verifier.verify_reshape(&input, &reshaped_target).unwrap();
    assert_eq!(
        reshaped.static_dims(),
        Maybe::Some(List::from(vec![128, 28, 28]))
    );

    // Transpose to [28, 28, 128]
    let transposed = verifier
        .verify_transpose(&reshaped, Maybe::Some(List::from(vec![1, 2, 0])))
        .unwrap();
    assert_eq!(
        transposed.static_dims(),
        Maybe::Some(List::from(vec![28, 28, 128]))
    );

    // Reduce along axis 0: [28, 28, 128] → [28, 128]
    let reduced = verifier.verify_reduction(&transposed, 0, false).unwrap();
    assert_eq!(
        reduced.static_dims(),
        Maybe::Some(List::from(vec![28, 128]))
    );
}

#[test]
fn test_broadcast_with_meta_parameters() {
    let verifier = ShapeVerifier::new();

    // [N, 1] + [1, M] → [N, M]
    let mut shape1 = TensorShape::new();
    shape1.add_dynamic_dim("N");
    shape1.add_static_dim(1);

    let mut shape2 = TensorShape::new();
    shape2.add_static_dim(1);
    shape2.add_dynamic_dim("M");

    let result = verifier.verify_broadcast(&shape1, &shape2).unwrap();
    assert_eq!(result.rank(), 2);
    assert_eq!(result.dimensions[0], Dimension::Dynamic(Text::from("N")));
    assert_eq!(result.dimensions[1], Dimension::Dynamic(Text::from("M")));
}

#[test]
fn test_batch_matrix_multiply() {
    let verifier = ShapeVerifier::new();

    // Batch matmul: We need to handle this specially
    // For now, verify basic 2D case
    let batch1 = TensorShape::from_dims(vec![32, 128, 256]);
    let batch2 = TensorShape::from_dims(vec![32, 256, 512]);

    // This would fail with current 2D matmul check - that's expected
    // Full implementation would need batch matmul support
    let result = verifier.verify_matmul(&batch1, &batch2);
    assert!(result.is_err()); // Expected: rank must be 2
}

// ============================================================================
// Error Message Tests
// ============================================================================

#[test]
fn test_dimension_mismatch_error_format() {
    let verifier = ShapeVerifier::new();
    let shape_a = TensorShape::from_dims(vec![128, 256]);
    let shape_b = TensorShape::from_dims(vec![512, 1024]);

    let err = verifier.verify_matmul(&shape_a, &shape_b).unwrap_err();
    let error_msg = format!("{}", err);
    assert!(error_msg.contains("dimension mismatch"));
}

#[test]
fn test_shape_mismatch_error_format() {
    let verifier = ShapeVerifier::new();
    let shape1 = TensorShape::from_dims(vec![128, 256]);
    let shape2 = TensorShape::from_dims(vec![128, 256, 3]);

    let err = verifier.verify_elementwise(&shape1, &shape2).unwrap_err();
    let error_msg = format!("{}", err);
    assert!(error_msg.contains("shape mismatch"));
    assert!(error_msg.contains("rank"));
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_scalar_shape() {
    let shape = TensorShape::from_dims(vec![]);
    assert_eq!(shape.rank(), 0);
    assert!(shape.is_fully_static());
    assert_eq!(shape.static_dims(), Maybe::Some(List::from(vec![])));
}

#[test]
fn test_single_dimension() {
    let shape = TensorShape::from_dims(vec![128]);
    assert_eq!(shape.rank(), 1);
    assert_eq!(shape.static_dims(), Maybe::Some(List::from(vec![128])));
}

#[test]
fn test_large_tensor() {
    let shape = TensorShape::from_dims(vec![1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(shape.rank(), 8);
    assert!(shape.is_fully_static());
}

#[test]
fn test_zero_dimension() {
    let shape = TensorShape::from_dims(vec![0, 3, 4]);
    assert_eq!(shape.rank(), 3);
    assert!(shape.is_fully_static());
    assert_eq!(shape.static_dims(), Maybe::Some(List::from(vec![0, 3, 4])));
}

// ============================================================================
// Wire-up Pin Tests
// ============================================================================

#[test]
fn max_rank_caps_verify_matmul() {
    // Pin: VerificationConfig.max_rank actually rejects shapes
    // whose rank exceeds the configured cap. Before the wire-up
    // the field was inert — verify_matmul accepted any rank-2
    // pair regardless of cap (and other operations accepted any
    // rank). Setting max_rank = 1 must reject every rank-2
    // matmul operand.
    let cfg = VerificationConfig {
        max_rank: 1,
        ..VerificationConfig::default()
    };
    let v = ShapeVerifier::with_config(cfg);
    let a = TensorShape::from_dims(vec![3, 4]);
    let b = TensorShape::from_dims(vec![4, 5]);
    let res = v.verify_matmul(&a, &b);
    assert!(
        res.is_err(),
        "max_rank=1 must reject rank-2 matmul operands"
    );
    let err = format!("{:?}", res.unwrap_err());
    assert!(
        err.contains("max_rank") || err.contains("rank"),
        "rejection must name the rank cap: {err}"
    );
}

#[test]
fn max_rank_caps_verify_reshape_output() {
    // Pin: verify_reshape gates BOTH input and output rank.
    // A reshape from rank-2 to rank-5 with max_rank = 3 must be
    // rejected even though the input is within the cap.
    let cfg = VerificationConfig {
        max_rank: 3,
        ..VerificationConfig::default()
    };
    let v = ShapeVerifier::with_config(cfg);
    let input = TensorShape::from_dims(vec![6, 4]);
    let new_shape = TensorShape::from_dims(vec![2, 2, 2, 1, 3]);
    let res = v.verify_reshape(&input, &new_shape);
    assert!(
        res.is_err(),
        "max_rank=3 must reject a reshape that produces rank 5"
    );
}

#[test]
fn max_rank_unlimited_default_accepts_high_rank() {
    // Pin: default config (max_rank = 8) accepts any rank up to
    // and including 8 — preserves prior behaviour for the
    // "Maximum tensor rank to verify (default: 8)" contract.
    let v = ShapeVerifier::new();
    let a = TensorShape::from_dims(vec![1, 2]);
    let b = TensorShape::from_dims(vec![2, 3]);
    assert!(v.verify_matmul(&a, &b).is_ok());
}
