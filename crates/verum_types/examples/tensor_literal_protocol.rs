//! Example: FromTensorLiteral Protocol Usage
//!
//! This example demonstrates the FromTensorLiteral protocol implementation
//! for compile-time tensor literal construction with shape validation.
//!
//! Tensor protocol: operations on Tensor<T, Shape> including element-wise ops, reductions, reshaping with compile-time shape validation — Tensor Literal Protocol
//!
//! # Key Features Demonstrated
//!
//! 1. **Protocol registration** - FromTensorLiteral protocol in type system
//! 2. **Shape validation** - Compile-time verification of tensor dimensions
//! 3. **Element count checking** - Ensure correct number of elements
//! 4. **Nesting validation** - Verify proper multi-dimensional structure
//! 5. **Broadcasting** - Single element expansion to fill shape
//!
//! # Usage
//!
//! ```bash
//! cargo run --example tensor_literal_protocol
//! ```

use verum_ast::span::Span;
use verum_common::ConstValue;
use verum_types::{
    protocol::ProtocolChecker,
    tensor_protocol::{TensorLiteralValidator, create_from_tensor_literal_protocol},
};

fn main() {
    println!("=== FromTensorLiteral Protocol Example ===\n");

    example_protocol_registration();
    example_protocol_structure();
    example_shape_validation();
    example_nesting_validation();
    example_broadcasting();
    example_error_messages();
    example_complex_tensors();

    println!("\n=== All examples completed successfully! ===");
}

/// Example 1: Protocol Registration
fn example_protocol_registration() {
    println!("Example 1: Protocol Registration");
    println!("---------------------------------");

    let checker = ProtocolChecker::new();

    // Look up the FromTensorLiteral protocol
    let path = verum_ast::ty::Path::single(verum_ast::Ident::new(
        "FromTensorLiteral".to_string(),
        Span::default(),
    ));
    let protocol = checker.lookup_protocol(&path);

    if let Some(protocol) = protocol {
        println!("✓ FromTensorLiteral protocol registered");
        println!("  Name: {}", protocol.name);
        println!("  Type params: {}", protocol.type_params.len());
        println!(
            "  Methods: {}",
            protocol
                .methods
                .keys()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    } else {
        println!("✗ Protocol not found");
    }

    println!();
}

/// Example 2: Protocol Structure
fn example_protocol_structure() {
    println!("Example 2: Protocol Structure");
    println!("------------------------------");

    let protocol = create_from_tensor_literal_protocol();

    println!("Protocol: {}", protocol.name);
    println!("Type Parameters:");
    for param in &protocol.type_params {
        println!("  - {} (bounds: {})", param.name, param.bounds.len());
    }

    println!("Methods:");
    for (name, method) in &protocol.methods {
        println!("  - {}: {}", name, method.name);
        println!(
            "    has_default: {}, doc: {}",
            method.has_default,
            method.doc.is_some()
        );
    }

    println!();
}

/// Example 3: Shape Validation
fn example_shape_validation() {
    println!("Example 3: Shape Validation");
    println!("----------------------------");

    let mut validator = TensorLiteralValidator::new();

    // Valid shapes
    let valid_cases = vec![
        (&[4][..], 4, "1D vector with 4 elements"),
        (&[2, 3][..], 6, "2D matrix with 2×3 elements"),
        (&[2, 3, 4][..], 24, "3D tensor with 2×3×4 elements"),
    ];

    for (shape, elements, desc) in valid_cases {
        match validator.validate_shape(shape, elements, Span::default()) {
            Ok(_) => println!("✓ Valid: {}", desc),
            Err(e) => println!("✗ Error: {} - {}", desc, e),
        }
    }

    println!();
}

/// Example 4: Nesting Validation
fn example_nesting_validation() {
    println!("Example 4: Nesting Validation");
    println!("------------------------------");

    let validator = TensorLiteralValidator::new();

    let nesting_cases = vec![
        (&[4][..], 1, "1D tensor (single level)"),
        (&[2, 3][..], 2, "2D tensor (two levels)"),
        (&[2, 3, 4][..], 3, "3D tensor (three levels)"),
    ];

    for (shape, depth, desc) in nesting_cases {
        match validator.validate_nesting(shape, depth, Span::default()) {
            Ok(_) => println!("✓ Valid nesting: {}", desc),
            Err(e) => println!("✗ Nesting error: {} - {}", desc, e),
        }
    }

    println!();
}

/// Example 5: Broadcasting
fn example_broadcasting() {
    println!("Example 5: Broadcasting Support");
    println!("--------------------------------");

    let mut validator = TensorLiteralValidator::new();

    // Broadcasting: single element can fill any shape
    let broadcast_cases = vec![
        (&[4][..], "tensor<4>f32{1.0}"),
        (&[2, 3][..], "tensor<2, 3>f32{1.0}"),
        (&[8][..], "tensor<8>i32{0}"),
    ];

    for (shape, syntax) in broadcast_cases {
        match validator.validate_shape(shape, 1, Span::default()) {
            Ok(_) => println!("✓ Broadcasting allowed: {}", syntax),
            Err(e) => println!("✗ Broadcasting error: {} - {}", syntax, e),
        }
    }

    println!();
}

/// Example 6: Error Messages
fn example_error_messages() {
    println!("Example 6: Error Messages");
    println!("-------------------------");

    let mut validator = TensorLiteralValidator::new();

    // Shape mismatch
    println!("Shape mismatch error:");
    match validator.validate_shape(&[4], 3, Span::default()) {
        Ok(_) => println!("  Unexpected success"),
        Err(e) => {
            let err_str = e.to_string();
            // Show first line of error
            if let Some(first_line) = err_str.lines().next() {
                println!("  {}", first_line);
            }
        }
    }

    // Nesting mismatch
    println!("\nNesting mismatch error:");
    match validator.validate_nesting(&[2, 3], 1, Span::default()) {
        Ok(_) => println!("  Unexpected success"),
        Err(e) => {
            let err_str = e.to_string();
            if let Some(first_line) = err_str.lines().next() {
                println!("  {}", first_line);
            }
        }
    }

    println!();
}

/// Example 7: Complex Tensors
fn example_complex_tensors() {
    println!("Example 7: Complex Tensor Shapes");
    println!("---------------------------------");

    let mut validator = TensorLiteralValidator::new();

    // Image tensor: [3, 224, 224] (RGB channels, 224×224 resolution)
    let image_shape = vec![
        ConstValue::UInt(3),
        ConstValue::UInt(224),
        ConstValue::UInt(224),
    ];
    let image_elements = validator.compute_element_count(&image_shape).unwrap();
    println!("Image tensor [3, 224, 224]:");
    println!("  Total elements: {}", image_elements);

    let shape_array = validator.shape_to_usize_array(&image_shape).unwrap();
    match validator.validate_shape(&shape_array, image_elements, Span::default()) {
        Ok(_) => println!("  ✓ Valid shape"),
        Err(e) => println!("  ✗ Error: {}", e),
    }

    // Batch of images: [32, 3, 224, 224]
    let batch_shape = vec![
        ConstValue::UInt(32),
        ConstValue::UInt(3),
        ConstValue::UInt(224),
        ConstValue::UInt(224),
    ];
    let batch_elements = validator.compute_element_count(&batch_shape).unwrap();
    println!("\nBatch tensor [32, 3, 224, 224]:");
    println!("  Total elements: {}", batch_elements);

    // Weight matrix: [512, 512]
    let weight_shape = vec![ConstValue::UInt(512), ConstValue::UInt(512)];
    let weight_elements = validator.compute_element_count(&weight_shape).unwrap();
    println!("\nWeight matrix [512, 512]:");
    println!("  Total elements: {}", weight_elements);

    println!();
}
