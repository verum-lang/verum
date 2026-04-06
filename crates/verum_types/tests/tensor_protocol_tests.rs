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
//! Comprehensive tests for FromTensorLiteral protocol
//!
//! Tensor protocol: operations on Tensor<T, Shape> including element-wise ops, reductions, reshaping with compile-time shape validation — Tensor Literal Protocol
//!
//! These tests verify:
//! - Protocol registration in the type checker
//! - Shape validation at compile-time
//! - Element count checking
//! - Nesting structure validation
//! - Broadcasting behavior
//! - Error messages and diagnostics

use verum_ast::span::Span;
use verum_common::{ConstValue, Text};
use verum_types::{
    Type,
    protocol::ProtocolChecker,
    tensor_protocol::{NestedArray, TensorLiteralValidator, create_from_tensor_literal_protocol},
};

#[test]
fn test_protocol_registration() {
    let checker = ProtocolChecker::new();

    // Protocol should be registered automatically via register_standard_protocols
    let _protocol_name: Text = "FromTensorLiteral".into();
    let protocol = checker.lookup_protocol(&verum_ast::ty::Path::single(verum_ast::Ident::new(
        "FromTensorLiteral".to_string(),
        Span::default(),
    )));

    assert!(
        protocol.is_some(),
        "FromTensorLiteral protocol should be registered"
    );

    let protocol = protocol.unwrap();
    assert_eq!(protocol.name.as_str(), "FromTensorLiteral");
    assert_eq!(protocol.type_params.len(), 2);
    assert!(
        protocol
            .methods
            .contains_key(&Text::from("from_tensor_literal"))
    );
}

#[test]
fn test_protocol_creation() {
    let protocol = create_from_tensor_literal_protocol();

    assert_eq!(protocol.name.as_str(), "FromTensorLiteral");
    assert_eq!(protocol.type_params.len(), 2);

    // Check type parameters
    assert_eq!(protocol.type_params[0].name.as_str(), "Shape");
    assert_eq!(protocol.type_params[1].name.as_str(), "T");

    // Check method
    assert!(
        protocol
            .methods
            .contains_key(&Text::from("from_tensor_literal"))
    );
    let method = protocol
        .methods
        .get(&Text::from("from_tensor_literal"))
        .unwrap();
    assert_eq!(method.name.as_str(), "from_tensor_literal");
    assert!(!method.has_default);
}

#[test]
fn test_nested_array_1d() {
    let arr = NestedArray::from_shape(Type::Float, &[4]);

    assert_eq!(arr.depth, 1);
    assert_eq!(arr.element_count, 4);
    assert_eq!(arr.element_ty, Type::Float);
}

#[test]
fn test_nested_array_2d() {
    let arr = NestedArray::from_shape(Type::Float, &[2, 3]);

    assert_eq!(arr.depth, 2);
    assert_eq!(arr.element_count, 6);
    assert_eq!(arr.element_ty, Type::Float);
}

#[test]
fn test_nested_array_3d() {
    let arr = NestedArray::from_shape(Type::Int, &[2, 3, 4]);

    assert_eq!(arr.depth, 3);
    assert_eq!(arr.element_count, 24);
    assert_eq!(arr.element_ty, Type::Int);
}

#[test]
fn test_validate_shape_exact_match_1d() {
    let mut validator = TensorLiteralValidator::new();

    // Shape [4] with 4 elements
    let result = validator.validate_shape(&[4], 4, Span::default());
    assert!(result.is_ok());
}

#[test]
fn test_validate_shape_exact_match_2d() {
    let mut validator = TensorLiteralValidator::new();

    // Shape [2, 3] with 6 elements
    let result = validator.validate_shape(&[2, 3], 6, Span::default());
    assert!(result.is_ok());
}

#[test]
fn test_validate_shape_exact_match_3d() {
    let mut validator = TensorLiteralValidator::new();

    // Shape [2, 3, 4] with 24 elements
    let result = validator.validate_shape(&[2, 3, 4], 24, Span::default());
    assert!(result.is_ok());
}

#[test]
fn test_validate_shape_mismatch_too_few() {
    let mut validator = TensorLiteralValidator::new();

    // Shape [4] with only 3 elements
    let result = validator.validate_shape(&[4], 3, Span::default());
    assert!(result.is_err());

    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(err_str.contains("size mismatch"));
    assert!(err_str.contains("expected 4"));
    assert!(err_str.contains("got 3"));
}

#[test]
fn test_validate_shape_mismatch_too_many() {
    let mut validator = TensorLiteralValidator::new();

    // Shape [2, 3] with 7 elements (instead of 6)
    let result = validator.validate_shape(&[2, 3], 7, Span::default());
    assert!(result.is_err());

    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(err_str.contains("size mismatch"));
    assert!(err_str.contains("expected 6"));
    assert!(err_str.contains("got 7"));
}

#[test]
fn test_validate_shape_broadcasting_1d() {
    let mut validator = TensorLiteralValidator::new();

    // Single element can broadcast to any shape
    let result = validator.validate_shape(&[4], 1, Span::default());
    assert!(result.is_ok());
}

#[test]
fn test_validate_shape_broadcasting_2d() {
    let mut validator = TensorLiteralValidator::new();

    // Single element can broadcast to 2D shape
    let result = validator.validate_shape(&[2, 3], 1, Span::default());
    assert!(result.is_ok());
}

#[test]
fn test_validate_shape_broadcasting_3d() {
    let mut validator = TensorLiteralValidator::new();

    // Single element can broadcast to 3D shape
    let result = validator.validate_shape(&[2, 3, 4], 1, Span::default());
    assert!(result.is_ok());
}

#[test]
fn test_validate_nesting_correct_1d() {
    let validator = TensorLiteralValidator::new();

    // 1D tensor requires depth 1
    let result = validator.validate_nesting(&[4], 1, Span::default());
    assert!(result.is_ok());
}

#[test]
fn test_validate_nesting_correct_2d() {
    let validator = TensorLiteralValidator::new();

    // 2D tensor requires depth 2
    let result = validator.validate_nesting(&[2, 3], 2, Span::default());
    assert!(result.is_ok());
}

#[test]
fn test_validate_nesting_correct_3d() {
    let validator = TensorLiteralValidator::new();

    // 3D tensor requires depth 3
    let result = validator.validate_nesting(&[2, 3, 4], 3, Span::default());
    assert!(result.is_ok());
}

#[test]
fn test_validate_nesting_mismatch_too_shallow() {
    let validator = TensorLiteralValidator::new();

    // 2D shape but only 1D nesting
    let result = validator.validate_nesting(&[2, 3], 1, Span::default());
    assert!(result.is_err());

    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(err_str.contains("nesting mismatch"));
    assert!(err_str.contains("expected 2D"));
    assert!(err_str.contains("got 1D"));
}

#[test]
fn test_validate_nesting_mismatch_too_deep() {
    let validator = TensorLiteralValidator::new();

    // 1D shape but 2D nesting
    let result = validator.validate_nesting(&[4], 2, Span::default());
    assert!(result.is_err());

    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(err_str.contains("nesting mismatch"));
    assert!(err_str.contains("expected 1D"));
    assert!(err_str.contains("got 2D"));
}

#[test]
fn test_compute_element_count_1d() {
    let mut validator = TensorLiteralValidator::new();

    let shape = vec![ConstValue::UInt(10)];
    let count = validator.compute_element_count(&shape).unwrap();
    assert_eq!(count, 10);
}

#[test]
fn test_compute_element_count_2d() {
    let mut validator = TensorLiteralValidator::new();

    let shape = vec![ConstValue::UInt(2), ConstValue::UInt(3)];
    let count = validator.compute_element_count(&shape).unwrap();
    assert_eq!(count, 6);
}

#[test]
fn test_compute_element_count_3d() {
    let mut validator = TensorLiteralValidator::new();

    let shape = vec![
        ConstValue::UInt(2),
        ConstValue::UInt(3),
        ConstValue::UInt(4),
    ];
    let count = validator.compute_element_count(&shape).unwrap();
    assert_eq!(count, 24);
}

#[test]
fn test_compute_element_count_4d() {
    let mut validator = TensorLiteralValidator::new();

    let shape = vec![
        ConstValue::UInt(2),
        ConstValue::UInt(3),
        ConstValue::UInt(4),
        ConstValue::UInt(5),
    ];
    let count = validator.compute_element_count(&shape).unwrap();
    assert_eq!(count, 120);
}

#[test]
fn test_compute_element_count_invalid_dimension() {
    let mut validator = TensorLiteralValidator::new();

    // Bool instead of UInt
    let shape = vec![ConstValue::Bool(true)];
    let result = validator.compute_element_count(&shape);
    assert!(result.is_err());

    let err = result.unwrap_err();
    assert!(err.to_string().contains("positive integer"));
}

#[test]
fn test_shape_to_usize_array() {
    let validator = TensorLiteralValidator::new();

    let shape = vec![
        ConstValue::UInt(2),
        ConstValue::UInt(3),
        ConstValue::UInt(4),
    ];
    let result = validator.shape_to_usize_array(&shape).unwrap();

    assert_eq!(result.len(), 3);
    assert_eq!(result[0], 2);
    assert_eq!(result[1], 3);
    assert_eq!(result[2], 4);
}

#[test]
fn test_shape_to_usize_array_empty() {
    let validator = TensorLiteralValidator::new();

    let shape = vec![];
    let result = validator.shape_to_usize_array(&shape).unwrap();

    assert_eq!(result.len(), 0);
}

#[test]
fn test_shape_to_usize_array_large_dimensions() {
    let validator = TensorLiteralValidator::new();

    let shape = vec![
        ConstValue::UInt(100),
        ConstValue::UInt(200),
        ConstValue::UInt(300),
    ];
    let result = validator.shape_to_usize_array(&shape).unwrap();

    assert_eq!(result.len(), 3);
    assert_eq!(result[0], 100);
    assert_eq!(result[1], 200);
    assert_eq!(result[2], 300);
}

#[test]
fn test_error_message_quality_size_mismatch() {
    let mut validator = TensorLiteralValidator::new();

    let result = validator.validate_shape(&[2, 3], 5, Span::default());
    assert!(result.is_err());

    let err_str = result.unwrap_err().to_string();

    // Should contain clear information
    assert!(err_str.contains("Tensor size mismatch"));
    assert!(err_str.contains("expected 6 elements"));
    assert!(err_str.contains("got 5 elements"));
    assert!(err_str.contains("shape [2, 3]"));

    // Should contain helpful suggestions
    assert!(err_str.contains("help:"));
    assert!(err_str.contains("provide exactly 6 elements"));
    assert!(err_str.contains("broadcasting"));
}

#[test]
fn test_error_message_quality_nesting_mismatch() {
    let validator = TensorLiteralValidator::new();

    let result = validator.validate_nesting(&[2, 2], 1, Span::default());
    assert!(result.is_err());

    let err_str = result.unwrap_err().to_string();

    // Should contain clear information
    assert!(err_str.contains("nesting mismatch"));
    assert!(err_str.contains("expected 2D"));
    assert!(err_str.contains("got 1D"));

    // Should contain helpful example
    assert!(err_str.contains("example:"));
    assert!(err_str.contains("tensor<2, 2>T"));
}

#[test]
fn test_nesting_example_1d() {
    let validator = TensorLiteralValidator::new();
    let example = validator.nesting_example(&[4]);

    assert!(example.contains("tensor<4>T"));
    assert!(example.contains("{"));
}

#[test]
fn test_nesting_example_2d() {
    let validator = TensorLiteralValidator::new();
    let example = validator.nesting_example(&[2, 2]);

    assert!(example.contains("tensor<2, 2>T"));
    assert!(example.contains("{{"));
}

#[test]
fn test_nesting_example_3d() {
    let validator = TensorLiteralValidator::new();
    let example = validator.nesting_example(&[2, 2, 2]);

    assert!(example.contains("tensor<2, 2, 2>T"));
    assert!(example.contains("{{{"));
}

#[test]
fn test_protocol_integration_with_checker() {
    let checker = ProtocolChecker::new();

    // Verify protocol is registered
    let path = verum_ast::ty::Path::single(verum_ast::Ident::new(
        "FromTensorLiteral".to_string(),
        Span::default(),
    ));
    let protocol = checker.lookup_protocol(&path);

    assert!(protocol.is_some());
    let protocol = protocol.unwrap();

    // Verify it has correct structure
    assert_eq!(protocol.name.as_str(), "FromTensorLiteral");
    assert_eq!(protocol.type_params.len(), 2);
    assert!(
        protocol
            .methods
            .contains_key(&Text::from("from_tensor_literal"))
    );
}

#[test]
fn test_validator_default_construction() {
    let _validator = TensorLiteralValidator::default();
    // Should construct successfully
}

#[test]
fn test_nested_array_equality() {
    let arr1 = NestedArray::new(Type::Float, 2, 6);
    let arr2 = NestedArray::new(Type::Float, 2, 6);
    let arr3 = NestedArray::new(Type::Int, 2, 6);

    assert_eq!(arr1, arr2);
    assert_ne!(arr1, arr3);
}

#[test]
fn test_complex_shape_validation() {
    let mut validator = TensorLiteralValidator::new();

    // Large multidimensional tensor: [16, 16, 3]
    let shape = &[16, 16, 3];
    let expected_elements = 16 * 16 * 3; // 768

    let result = validator.validate_shape(shape, expected_elements, Span::default());
    assert!(result.is_ok());

    // Wrong element count
    let result = validator.validate_shape(shape, 767, Span::default());
    assert!(result.is_err());
}

#[test]
fn test_broadcasting_only_single_element() {
    let mut validator = TensorLiteralValidator::new();

    // Broadcasting only works with exactly 1 element
    let result = validator.validate_shape(&[4], 1, Span::default());
    assert!(result.is_ok());

    // 2 elements is not broadcasting
    let result = validator.validate_shape(&[4], 2, Span::default());
    assert!(result.is_err());
}
