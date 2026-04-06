//! Example demonstrating tensor support through Z3 Array theory.
//!
//! This example shows how Verum tensors are translated to nested Z3 Arrays
//! and verified using the SMT solver.

use z3::ast::{Array, Ast, Int};
use z3::{SatResult, Solver, Sort};

fn main() {
    println!("=== Tensor Array Theory Example ===\n");

    // Example 1: 1D Tensor - Array[Int -> Int]
    example_1d_tensor();

    // Example 2: 2D Tensor - Array[Int -> Array[Int -> Real]]
    example_2d_tensor();

    // Example 3: Tensor bounds verification
    example_bounds_verification();

    // Example 4: Matrix multiplication constraint
    example_matmul_constraint();

    println!("=== All Examples Complete ===");
}

/// Example 1: 1D Tensor represented as Z3 Array
fn example_1d_tensor() {
    println!("--- Example 1: 1D Tensor as Z3 Array ---");

    // Int -> Int array (represents Tensor<i32, [N]>)
    let int_sort = Sort::int();
    let _array_sort = Sort::array(&int_sort, &int_sort);

    // Create a symbolic 1D tensor
    let tensor = Array::fresh_const("vec", &int_sort, &int_sort);

    println!("Created 1D tensor: Array[Int -> Int]");

    // Store value at index 5
    let idx5 = Int::from_i64(5);
    let val42 = Int::from_i64(42);
    let tensor_updated = tensor.store(&idx5, &val42);

    // Retrieve value at index 5
    let retrieved = tensor_updated.select(&idx5);

    // Verify: tensor[5] == 42
    let solver = Solver::new();
    solver.assert(&Ast::eq(&retrieved, &val42));

    match solver.check() {
        SatResult::Sat => println!("✓ Verified: tensor[5] == 42"),
        _ => println!("✗ Verification failed"),
    }

    println!();
}

/// Example 2: 2D Tensor (matrix) as nested arrays
fn example_2d_tensor() {
    println!("--- Example 2: 2D Tensor (Matrix) ---");

    // Create sorts for 2D array: Int -> (Int -> Real)
    let int_sort = Sort::int();
    let real_sort = Sort::real();
    let row_sort = Sort::array(&int_sort, &real_sort);
    let _matrix_sort = Sort::array(&int_sort, &row_sort);

    println!("Created 2D matrix: Array[Int -> Array[Int -> Real]]");
    println!("This represents Tensor<f32, [rows, cols]>");

    // Create symbolic matrix
    let matrix = Array::fresh_const("matrix", &int_sort, &row_sort);

    // Access element at [1, 2]
    let row_idx = Int::from_i64(1);
    let _col_idx = Int::from_i64(2);

    // matrix[1] gives us a row (Array[Int -> Real])
    let _row = matrix.select(&row_idx);

    println!("✓ matrix[1] returns a row array");
    println!("✓ matrix[1][2] accesses element at row 1, column 2");

    println!();
}

/// Example 3: Tensor bounds verification
fn example_bounds_verification() {
    println!("--- Example 3: Tensor Bounds Verification ---");

    let solver = Solver::new();

    // Symbolic index
    let idx = Int::fresh_const("idx");

    // Tensor dimensions
    let zero = Int::from_i64(0);
    let size = Int::from_i64(10);

    // Assert: 0 <= idx < 10 (valid bounds)
    solver.assert(&idx.ge(&zero));
    solver.assert(&idx.lt(&size));

    // Check if there exists a valid index
    match solver.check() {
        SatResult::Sat => {
            println!("✓ Valid indices exist in range [0, 10)");
            if let Some(model) = solver.get_model() {
                if let Some(val) = model.eval(&idx, true) {
                    println!("  Example valid index: {}", val);
                }
            }
        }
        SatResult::Unsat => println!("✗ No valid indices (impossible)"),
        SatResult::Unknown => println!("? Solver returned unknown"),
    }

    // Now check for out-of-bounds
    let solver2 = Solver::new();
    let bad_idx = Int::fresh_const("bad_idx");

    // Assert: idx < 0 OR idx >= 10 (out of bounds)
    let too_small = bad_idx.lt(&zero);
    let too_large = bad_idx.ge(&size);

    // Use z3's Bool or
    solver2.assert(&z3::ast::Bool::or(&[&too_small, &too_large]));

    match solver2.check() {
        SatResult::Sat => {
            println!("✓ Out-of-bounds indices exist (correctly detected)");
            if let Some(model) = solver2.get_model() {
                if let Some(val) = model.eval(&bad_idx, true) {
                    println!("  Example out-of-bounds index: {}", val);
                }
            }
        }
        _ => println!("? Unexpected result"),
    }

    println!();
}

/// Example 4: Matrix multiplication dimension constraint
fn example_matmul_constraint() {
    println!("--- Example 4: Matrix Multiplication Constraint ---");

    let solver = Solver::new();

    // Matrix A: [m x n]
    // Matrix B: [p x q]
    // For A @ B to be valid: n == p

    let m = Int::fresh_const("m");
    let n = Int::fresh_const("n");
    let p = Int::fresh_const("p");
    let q = Int::fresh_const("q");

    let zero = Int::from_i64(0);

    // All dimensions must be positive
    solver.assert(&m.gt(&zero));
    solver.assert(&n.gt(&zero));
    solver.assert(&p.gt(&zero));
    solver.assert(&q.gt(&zero));

    // Matmul constraint: n == p
    solver.assert(&Ast::eq(&n, &p));

    // Result matrix is [m x q]
    println!("Constraint: A[m×n] @ B[p×q] requires n == p");
    println!("Result matrix: C[m×q]");

    match solver.check() {
        SatResult::Sat => {
            println!("✓ Valid matrix multiplication configuration exists");
            if let Some(model) = solver.get_model() {
                let m_val = model
                    .eval(&m, true)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let n_val = model
                    .eval(&n, true)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let p_val = model
                    .eval(&p, true)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let q_val = model
                    .eval(&q, true)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                println!(
                    "  Example: A[{}×{}] @ B[{}×{}] -> C[{}×{}]",
                    m_val, n_val, p_val, q_val, m_val, q_val
                );
            }
        }
        _ => println!("✗ No valid configuration"),
    }

    // Check invalid case: n != p
    let solver2 = Solver::new();
    let n2 = Int::from_i64(3);
    let p2 = Int::from_i64(5);

    solver2.assert(&Ast::eq(&n2, &p2).not());

    match solver2.check() {
        SatResult::Sat => {
            println!("✓ Correctly detected: 3 != 5, matmul invalid");
        }
        _ => println!("? Unexpected"),
    }

    println!();
}
