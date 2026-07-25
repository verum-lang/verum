//! Demonstration of Tensor Shape Verification
//!
//! This example shows how to use the tensor shape verification system
//! to verify tensor operations at compile time.
//!
//! Run with: cargo run --example tensor_shape_demo

use verum_common::{List, Maybe};
use verum_verification::tensor_shapes::*;

fn main() {
    println!("=== Tensor Shape Verification Demo ===\n");

    // Example 1: Static Matrix Multiplication
    println!("Example 1: Static Matrix Multiplication");
    let verifier = ShapeVerifier::new();

    let shape_a = TensorShape::from_dims(vec![128, 256]);
    let shape_b = TensorShape::from_dims(vec![256, 512]);

    println!("  A: {}", shape_a);
    println!("  B: {}", shape_b);

    match verifier.verify_matmul(&shape_a, &shape_b) {
        Ok(result) => {
            println!("  Result: {} ✓", result);
            assert_eq!(
                result.static_dims(),
                Maybe::Some(List::from(vec![128, 512]))
            );
        }
        Err(e) => println!("  Error: {}", e),
    }
    println!();

    // Example 2: Dynamic Shape with Meta Parameters
    println!("Example 2: Dynamic Matrix Multiplication [M, K] × [K, N] → [M, N]");

    let mut shape_a = TensorShape::new();
    shape_a.add_dynamic_dim("M");
    shape_a.add_dynamic_dim("K");
    shape_a.bind_meta_param("M", 128);
    shape_a.bind_meta_param("K", 256);

    let mut shape_b = TensorShape::new();
    shape_b.add_dynamic_dim("K");
    shape_b.add_dynamic_dim("N");
    shape_b.bind_meta_param("K", 256);
    shape_b.bind_meta_param("N", 512);

    println!("  A: {} (M=128, K=256)", shape_a);
    println!("  B: {} (K=256, N=512)", shape_b);

    match verifier.verify_matmul(&shape_a, &shape_b) {
        Ok(result) => {
            println!("  Result: {} ✓", result);
            let resolved = result.resolve().unwrap();
            println!("  Resolved: {}", resolved);
            assert_eq!(
                resolved.static_dims(),
                Maybe::Some(List::from(vec![128, 512]))
            );
        }
        Err(e) => println!("  Error: {}", e),
    }
    println!();

    // Example 3: Broadcasting
    println!("Example 3: Broadcasting [3, 1] + [1, 4] → [3, 4]");

    let shape1 = TensorShape::from_dims(vec![3, 1]);
    let shape2 = TensorShape::from_dims(vec![1, 4]);

    println!("  Shape1: {}", shape1);
    println!("  Shape2: {}", shape2);

    match verifier.verify_broadcast(&shape1, &shape2) {
        Ok(result) => {
            println!("  Result: {} ✓", result);
            assert_eq!(result.static_dims(), Maybe::Some(List::from(vec![3, 4])));
        }
        Err(e) => println!("  Error: {}", e),
    }
    println!();

    // Example 4: Shape Mismatch Error
    println!("Example 4: Shape Mismatch (Error Case)");

    let shape_a = TensorShape::from_dims(vec![128, 256]);
    let shape_b = TensorShape::from_dims(vec![512, 1024]); // 512 != 256

    println!("  A: {}", shape_a);
    println!("  B: {} (incompatible!)", shape_b);

    match verifier.verify_matmul(&shape_a, &shape_b) {
        Ok(result) => println!("  Result: {}", result),
        Err(e) => println!("  Error: {} ✓ (Expected)", e),
    }
    println!();

    // Example 5: Complex Pipeline
    println!("Example 5: Complex Pipeline");
    println!(
        "  [128, 784] → reshape → [128, 28, 28] → transpose → [28, 28, 128] → reduce → [28, 128]"
    );

    let input = TensorShape::from_dims(vec![128, 784]);
    println!("  Input: {}", input);

    let reshaped_target = TensorShape::from_dims(vec![128, 28, 28]);
    let reshaped = verifier.verify_reshape(&input, &reshaped_target).unwrap();
    println!("  After reshape: {}", reshaped);

    let transposed = verifier
        .verify_transpose(&reshaped, Maybe::Some(List::from(vec![1, 2, 0])))
        .unwrap();
    println!("  After transpose: {}", transposed);

    let reduced = verifier.verify_reduction(&transposed, 0, false).unwrap();
    println!("  After reduction: {}", reduced);

    assert_eq!(
        reduced.static_dims(),
        Maybe::Some(List::from(vec![28, 128]))
    );
    println!("  ✓ Pipeline complete");
    println!();

    // Example 6: Concatenation
    println!("Example 6: Concatenation");

    let shapes = vec![
        TensorShape::from_dims(vec![2, 3]),
        TensorShape::from_dims(vec![4, 3]),
        TensorShape::from_dims(vec![1, 3]),
    ];

    println!("  Shapes: [2, 3], [4, 3], [1, 3]");

    match verifier.verify_concat(&shapes, 0) {
        Ok(result) => {
            println!("  Concat along axis 0: {} ✓", result);
            assert_eq!(result.static_dims(), Maybe::Some(List::from(vec![7, 3]))); // 2+4+1 = 7
        }
        Err(e) => println!("  Error: {}", e),
    }
    println!();

    println!("=== All examples completed successfully! ===");
}
