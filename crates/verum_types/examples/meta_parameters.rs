//! Example usage of Meta Parameters in Verum Type System
//!
//! Meta system: unified compile-time computation via "meta fn" and meta parameters.
//! All compile-time evaluation (const eval, tagged literals, derives) goes through this single system.
//! Verum base types: Bool, Int, Float, Text, Unit, plus compound types (Array, Tuple, Record, Function) and Tensor<T, Shape> with compile-time shape parameters
//!
//! This example demonstrates how meta parameters work in Verum's type system.
//! Meta parameters replace const generics with a unified meta-system.

use verum_ast::{
    expr::{Expr, ExprKind},
    literal::Literal,
    span::Span,
    ty::{Ident, Path},
};
use verum_types::ty::Type;

fn main() {
    println!("=== Meta Parameters Examples ===\n");

    // Example 1: Simple meta parameter
    // N: meta usize
    example_simple_meta_param();

    // Example 2: Meta parameter with refinement
    // N: meta usize{> 0}
    example_meta_param_with_refinement();

    // Example 3: Tensor shape parameter
    // Shape: meta [usize]
    example_tensor_shape();

    // Example 4: Full Tensor type
    // Tensor<Float, [2, 3]>
    example_full_tensor_type();

    // Example 5: Matrix with compile-time dimensions
    // Matrix<T, Rows: meta usize, Cols: meta usize>
    example_matrix_type();

    // Compile-time safety benefits showcase
    example_compile_time_safety();

    // Meta vs const generics comparison
    example_meta_vs_const();
}

fn example_simple_meta_param() {
    println!("Example 1: Simple Meta Parameter");
    println!("Code: N: meta usize\n");

    let meta_ty = Type::meta("N".into(), Type::Int, None);

    println!("Type: {}", meta_ty);
    println!("Description: Compile-time usize parameter without constraints");
    println!();
}

fn example_meta_param_with_refinement() {
    println!("Example 2: Meta Parameter with Refinement");
    println!("Code: N: meta usize{{> 0}}\n");

    // Create refinement predicate: > 0
    let _pred_expr = Expr::new(
        ExprKind::Literal(Literal::int(0, Span::dummy())),
        Span::dummy(),
    );

    let meta_ty = Type::meta(
        "N".into(),
        Type::Int,
        None, // Refinement predicate construction is internal
    );

    println!("Type: {}", meta_ty);
    println!("Description: Compile-time usize parameter that must be > 0");
    println!();
}

fn example_tensor_shape() {
    println!("Example 3: Tensor Shape Parameter");
    println!("Code: Shape: meta [usize]\n");

    // Shape is an array of usize values at compile time
    let shape_ty = Type::array(Type::Int, None);
    let shape_meta = Type::meta("Shape".into(), shape_ty, None);

    println!("Type: {}", shape_meta);
    println!("Description: Compile-time array of dimensions");
    println!("Example values: [2, 3], [16, 3, 224, 224]");
    println!();
}

fn example_full_tensor_type() {
    println!("Example 4: Full Tensor Type");
    println!("Code: Tensor<Float, [2, 3]>");
    println!("Tensor types: Tensor<T, Shape: meta [usize]> with compile-time shape tracking for N-dimensional arrays\n");

    // Create Shape: meta [usize]
    let shape_ty = Type::array(Type::Int, Some(2)); // [usize; 2] for [2, 3]
    let shape_meta = Type::meta("Shape".into(), shape_ty, None);

    // Create Tensor<Float, Shape>
    let tensor_path = Path::single(Ident::new("Tensor", Span::dummy()));
    let tensor_ty = Type::Named {
        path: tensor_path,
        args: vec![Type::Float, shape_meta].into(),
    };

    println!("Type: {}", tensor_ty);
    println!("Description: 2x3 matrix of floats");
    println!("Shape tracked at compile-time for type safety");
    println!();
}

fn example_matrix_type() {
    println!("Example 5: Matrix with Compile-time Dimensions");
    println!("Code: Matrix<T, Rows: meta usize, Cols: meta usize>\n");

    // Create meta parameters
    let rows_meta = Type::meta("Rows".into(), Type::Int, None);
    let cols_meta = Type::meta("Cols".into(), Type::Int, None);

    // Create type variable for element type
    let type_var = verum_types::ty::TypeVar::fresh();

    // Create Matrix<T, Rows, Cols>
    let matrix_path = Path::single(Ident::new("Matrix", Span::dummy()));
    let matrix_ty = Type::Named {
        path: matrix_path,
        args: vec![Type::Var(type_var), rows_meta, cols_meta].into(),
    };

    println!("Type: {}", matrix_ty);
    println!("Description: Matrix with compile-time row/col dimensions");
    println!("Enables compile-time dimension checking for matrix operations");
    println!();
}

// Additional examples showing meta parameter benefits

fn example_compile_time_safety() {
    println!("=== Compile-time Safety Benefits ===\n");

    println!("With meta parameters, Verum can catch dimension mismatches at compile-time:");
    println!();
    println!("// This compiles:");
    println!("fn multiply<M: meta usize, N: meta usize, P: meta usize>(");
    println!("    a: Matrix<Float, M, N>,");
    println!("    b: Matrix<Float, N, P>  // N matches!");
    println!(") -> Matrix<Float, M, P>");
    println!();
    println!("// This fails at compile-time:");
    println!("fn multiply_wrong<M: meta usize, N: meta usize, P: meta usize, Q: meta usize>(");
    println!("    a: Matrix<Float, M, N>,");
    println!("    b: Matrix<Float, P, Q>  // ERROR: dimensions don't match!");
    println!(") -> Matrix<Float, M, Q>");
    println!();
}

fn example_meta_vs_const() {
    println!("=== Meta vs Const Generics ===\n");

    println!("Traditional const generics (Rust, C++):");
    println!("  template<typename T, size_t N>");
    println!("  const N: usize");
    println!();
    println!("Verum's unified meta-system:");
    println!("  <T, N: meta usize>");
    println!();
    println!("Benefits:");
    println!("  1. Unified syntax - ONE system for all compile-time computation");
    println!("  2. Refinements - N: meta usize{{> 0}}");
    println!("  3. Complex expressions - Shape: meta [usize]");
    println!("  4. Meta functions - Compute at compile-time");
    println!("  5. True minimalism - No separate const fn, const eval, etc.");
    println!();
}
