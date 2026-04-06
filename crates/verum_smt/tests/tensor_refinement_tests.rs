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
//! Integration tests for tensor refinement type system
//!
//! Tests the integration between tensor shape verification and
//! Verum's refinement type system.
//!
// REQUIRES API MIGRATION: TensorSort.dimensions type changed to List<usize>

#![cfg(feature = "tensor_refinement_tests_disabled")]

use verum_ast::{Expr, ExprKind, IntLit, Literal, LiteralKind, Span, Type, TypeKind};
use verum_common::Heap;
use verum_smt::TensorSort;
use verum_smt::tensor_refinement::{
    TensorOperation, TensorRefinementError, TensorRefinementVerifier, TensorTypeInfo,
};
use verum_common::List;

/// Helper to create integer literal expression
fn expr_int(value: i128) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value,
                suffix: None,
            }),
            span: Span::dummy(),
        }),
        Span::dummy(),
    )
}

/// Helper to create a simple tensor type
fn tensor_type(element: Type, shape: List<Expr>) -> Type {
    let shape_vec: Vec<Heap<Expr>> = shape.iter().map(|e| Heap::new(e.clone())).collect();
    Type::new(
        TypeKind::Tensor {
            element: Heap::new(element),
            shape: shape_vec.into(),
            layout: verum_common::Maybe::None,
        },
        Span::dummy(),
    )
}

/// Helper to create TensorTypeInfo
fn tensor_info(element: Type, shape: List<Expr>) -> TensorTypeInfo {
    let ndim = shape.len();
    TensorTypeInfo {
        element_type: element.clone(),
        shape: shape,
        sort: TensorSort {
            element_type: "Real".into(),
            dimensions: List::from_iter(vec![0i128; ndim]),
            ndim,
        },
        refinement_predicates: List::new(),
    }
}

#[test]
fn test_verify_tensor_type_basic() {
    let mut verifier = TensorRefinementVerifier::new();

    // Create a simple tensor type: Tensor<f32, [2, 3]>
    let element_type = Type::float(Span::dummy());
    let shape: List<Expr> = List::from_iter(vec![expr_int(2), expr_int(3)]);
    let tensor = tensor_type(element_type, shape);

    let result = verifier.verify_tensor_type(&tensor);
    assert!(result.is_ok(), "Basic tensor type should verify");

    let info = result.unwrap();
    assert_eq!(info.shape.len(), 2, "Shape should have 2 dimensions");
    assert_eq!(info.sort.ndim, 2, "Sort should indicate 2D tensor");
}

#[test]
fn test_verify_tensor_type_not_tensor() {
    let mut verifier = TensorRefinementVerifier::new();

    // Try to verify a non-tensor type
    let int_type = Type::int(Span::dummy());

    let result = verifier.verify_tensor_type(&int_type);
    assert!(result.is_err(), "Int type should not verify as tensor");

    match result.unwrap_err() {
        TensorRefinementError::NotATensorType(_) => {
            // Expected error
        }
        other => panic!("Expected NotATensorType, got {:?}", other),
    }
}

#[test]
fn test_verify_matmul_operation() {
    let mut verifier = TensorRefinementVerifier::new();

    // Create operands: [2, 3] × [3, 4]
    let element_type = Type::float(Span::dummy());

    let a_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(2), expr_int(3)]),
    );
    let b_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(3), expr_int(4)]),
    );

    let result = verifier.verify_tensor_operation(TensorOperation::MatMul, &[a_info, b_info]);

    assert!(result.is_ok(), "Valid matmul operation should succeed");

    let result_info = result.unwrap();
    assert_eq!(result_info.shape.len(), 2, "Result should be 2D tensor");
}

#[test]
fn test_verify_matmul_dimension_mismatch() {
    let mut verifier = TensorRefinementVerifier::new();

    // Create operands with mismatched dimensions: [2, 3] × [5, 4]
    let element_type = Type::float(Span::dummy());

    let a_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(2), expr_int(3)]),
    );
    let b_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(5), expr_int(4)]),
    );

    let result = verifier.verify_tensor_operation(TensorOperation::MatMul, &[a_info, b_info]);

    assert!(result.is_err(), "Mismatched matmul should fail");
}

#[test]
fn test_verify_elementwise_operation() {
    let mut verifier = TensorRefinementVerifier::new();

    // Create operands with same shape: [3, 4] + [3, 4]
    let element_type = Type::float(Span::dummy());

    let a_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(3), expr_int(4)]),
    );
    let b_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(3), expr_int(4)]),
    );

    let result = verifier.verify_tensor_operation(TensorOperation::Elementwise, &[a_info, b_info]);

    assert!(
        result.is_ok(),
        "Elementwise with matching shapes should succeed"
    );

    let result_info = result.unwrap();
    assert_eq!(result_info.shape.len(), 2, "Result should match input");
}

#[test]
fn test_verify_broadcast_operation() {
    let mut verifier = TensorRefinementVerifier::new();

    // Create operands for broadcasting: [3, 1, 5] + [2, 5]
    let element_type = Type::float(Span::dummy());

    let a_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(3), expr_int(1), expr_int(5)]),
    );

    let b_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(2), expr_int(5)]),
    );

    let result = verifier.verify_tensor_operation(TensorOperation::Broadcast, &[a_info, b_info]);

    assert!(result.is_ok(), "Broadcasting should succeed");

    let result_info = result.unwrap();
    assert_eq!(
        result_info.shape.len(),
        3,
        "Result should have 3 dimensions"
    );
}

#[test]
fn test_verify_transpose_operation() {
    let mut verifier = TensorRefinementVerifier::new();

    // Create a 2D tensor: [3, 4]
    let element_type = Type::float(Span::dummy());
    let a_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(3), expr_int(4)]),
    );

    let result = verifier.verify_tensor_operation(TensorOperation::Transpose, &[a_info]);

    assert!(result.is_ok(), "Transpose should succeed");

    let result_info = result.unwrap();
    assert_eq!(result_info.shape.len(), 2, "Result should be 2D");

    // Verify dimensions are swapped: [4, 3]
    if let ExprKind::Literal(lit) = &result_info.shape[0].kind {
        if let LiteralKind::Int(i) = &lit.kind {
            assert_eq!(i.value, 4, "First dimension should be swapped to 4");
        }
    }

    if let ExprKind::Literal(lit) = &result_info.shape[1].kind {
        if let LiteralKind::Int(i) = &lit.kind {
            assert_eq!(i.value, 3, "Second dimension should be swapped to 3");
        }
    }
}

#[test]
fn test_verify_invalid_operand_count() {
    let mut verifier = TensorRefinementVerifier::new();

    let element_type = Type::float(Span::dummy());
    let a_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(2), expr_int(3)]),
    );

    // MatMul requires exactly 2 operands
    let result = verifier.verify_tensor_operation(TensorOperation::MatMul, &[a_info]);

    assert!(result.is_err(), "MatMul with 1 operand should fail");

    match result.unwrap_err() {
        TensorRefinementError::InvalidOperandCount { expected, actual } => {
            assert_eq!(expected, 2);
            assert_eq!(actual, 1);
        }
        other => panic!("Expected InvalidOperandCount, got {:?}", other),
    }
}

#[test]
fn test_verification_statistics() {
    let mut verifier = TensorRefinementVerifier::new();

    // Perform several operations
    let element_type = Type::float(Span::dummy());

    let a_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(2), expr_int(3)]),
    );
    let b_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(3), expr_int(4)]),
    );

    let _ = verifier.verify_tensor_operation(TensorOperation::MatMul, &[a_info.clone(), b_info]);

    let c_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(5), expr_int(6)]),
    );
    let _ = verifier.verify_tensor_operation(TensorOperation::Transpose, &[c_info]);

    let stats = verifier.stats();
    assert!(
        stats.shape_checks >= 2,
        "Should have performed at least 2 shape checks"
    );
}

#[test]
fn test_matmul_chain_verification() {
    let mut verifier = TensorRefinementVerifier::new();

    let element_type = Type::float(Span::dummy());

    // Chain: A[2,3] × B[3,4] × C[4,5]

    // Step 1: A × B
    let a_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(2), expr_int(3)]),
    );
    let b_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(3), expr_int(4)]),
    );

    let ab_result = verifier.verify_tensor_operation(TensorOperation::MatMul, &[a_info, b_info]);
    assert!(ab_result.is_ok());

    // Step 2: (A × B) × C
    let ab_info = ab_result.unwrap();
    let c_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(4), expr_int(5)]),
    );

    let abc_result = verifier.verify_tensor_operation(TensorOperation::MatMul, &[ab_info, c_info]);
    assert!(abc_result.is_ok());

    let result = abc_result.unwrap();
    assert_eq!(result.shape.len(), 2);

    // Verify final shape: [2, 5]
    if let ExprKind::Literal(lit) = &result.shape[0].kind {
        if let LiteralKind::Int(i) = &lit.kind {
            assert_eq!(i.value, 2);
        }
    }

    if let ExprKind::Literal(lit) = &result.shape[1].kind {
        if let LiteralKind::Int(i) = &lit.kind {
            assert_eq!(i.value, 5);
        }
    }
}

#[test]
fn test_combined_operations() {
    let mut verifier = TensorRefinementVerifier::new();

    let element_type = Type::float(Span::dummy());

    // Create tensors
    let a_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(2), expr_int(3)]),
    );
    let b_info = tensor_info(
        element_type.clone(),
        List::from_iter(vec![expr_int(3), expr_int(4)]),
    );

    // MatMul: [2,3] × [3,4] = [2,4]
    let matmul_result =
        verifier.verify_tensor_operation(TensorOperation::MatMul, &[a_info, b_info]);
    assert!(matmul_result.is_ok());

    let matmul_info = matmul_result.unwrap();

    // Transpose: [2,4] -> [4,2]
    let transpose_result =
        verifier.verify_tensor_operation(TensorOperation::Transpose, &[matmul_info]);
    assert!(transpose_result.is_ok());

    let final_info = transpose_result.unwrap();
    assert_eq!(final_info.shape.len(), 2);

    // Verify final shape: [4, 2]
    if let ExprKind::Literal(lit) = &final_info.shape[0].kind {
        if let LiteralKind::Int(i) = &lit.kind {
            assert_eq!(i.value, 4);
        }
    }

    if let ExprKind::Literal(lit) = &final_info.shape[1].kind {
        if let LiteralKind::Int(i) = &lit.kind {
            assert_eq!(i.value, 2);
        }
    }
}
