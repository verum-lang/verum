//! Example: Tensor Shape Computation with Meta Parameters
//!
//! This example demonstrates how to use meta parameters for compile-time
//! tensor shape computation in the Verum type system.
//!
//! Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Unified meta-system for compile-time computation
//! Verum base types: Bool, Int, Float, Text, Unit, plus compound types (Array, Tuple, Record, Function) and Tensor<T, Shape> with compile-time shape parameters — with meta parameters
//!
//! # Key Features Demonstrated
//!
//! 1. **Compile-time shape computation** - Calculate tensor dimensions at compile time
//! 2. **Meta parameter evaluation** - Evaluate arithmetic expressions for shapes
//! 3. **Shape validation** - Verify tensor shapes are compatible
//! 4. **Type-safe tensors** - Prevent dimension mismatches at compile time
//!
//! # Usage
//!
//! ```bash
//! cargo run --example tensor_shapes
//! ```

use verum_ast::{
    Ident,
    expr::{ArrayExpr, BinOp, Expr, ExprKind},
    literal::Literal,
    span::Span,
    ty::Path,
};
use verum_common::{ConstValue, List};
use verum_types::{
    Type, TypeChecker,
    const_eval::ConstEvaluator,
};

fn main() {
    println!("=== Verum Tensor Shape Computation Example ===\n");

    example_1d_tensor();
    example_2d_tensor();
    example_3d_tensor();
    example_computed_shapes();
    example_shape_validation();
    example_meta_parameter_types();
    example_tensor_arithmetic();

    println!("\n=== All examples completed successfully! ===");
}

/// Example 1: 1D Tensor (Vector)
fn example_1d_tensor() {
    println!("Example 1: 1D Tensor (Vector)");
    println!("-------------------------------");

    let mut checker = TypeChecker::new();

    // Shape: [10]
    let shape = create_shape(&[10]);

    let dims = checker.compute_tensor_shape(&shape).unwrap();
    let total = checker.compute_tensor_elements(&shape).unwrap();

    println!("Shape: {:?}", dims);
    println!("Total elements: {}", total);
    println!("Type: Vector<Float, 10>\n");
}

/// Example 2: 2D Tensor (Matrix)
fn example_2d_tensor() {
    println!("Example 2: 2D Tensor (Matrix)");
    println!("-------------------------------");

    let mut checker = TypeChecker::new();

    // Shape: [3, 4]
    let shape = create_shape(&[3, 4]);

    let dims = checker.compute_tensor_shape(&shape).unwrap();
    let total = checker.compute_tensor_elements(&shape).unwrap();

    println!("Shape: {:?}", dims);
    println!("Total elements: {}", total);
    println!("Type: Matrix<Float, 3, 4>\n");
}

/// Example 3: 3D Tensor
fn example_3d_tensor() {
    println!("Example 3: 3D Tensor");
    println!("--------------------");

    let mut checker = TypeChecker::new();

    // Shape: [2, 3, 4]
    let shape = create_shape(&[2, 3, 4]);

    let dims = checker.compute_tensor_shape(&shape).unwrap();
    let total = checker.compute_tensor_elements(&shape).unwrap();

    println!("Shape: {:?}", dims);
    println!("Total elements: {}", total);
    println!("Type: Tensor<Float, [2, 3, 4]>\n");
}

/// Example 4: Computed Shapes (Compile-time arithmetic)
fn example_computed_shapes() {
    println!("Example 4: Computed Shapes");
    println!("--------------------------");

    let mut checker = TypeChecker::new();

    // Shape: [2 * 3, 4 + 1]
    let dim1 = create_binary(BinOp::Mul, 2, 3);
    let dim2 = create_binary(BinOp::Add, 4, 1);
    let mut list = List::new();
    list.push(dim1);
    list.push(dim2);
    let shape = Expr::new(ExprKind::Array(ArrayExpr::List(list)), Span::dummy());

    let dims = checker.compute_tensor_shape(&shape).unwrap();
    let total = checker.compute_tensor_elements(&shape).unwrap();

    println!("Shape expression: [2 * 3, 4 + 1]");
    println!("Computed shape: {:?}", dims);
    println!("Total elements: {}", total);
    println!("Type: Tensor<Float, [6, 5]>\n");
}

/// Example 5: Shape Validation
fn example_shape_validation() {
    println!("Example 5: Shape Validation");
    println!("---------------------------");

    let mut checker = TypeChecker::new();

    // Compatible shapes
    let shape1 = create_shape(&[3, 4]);
    let shape2 = create_shape(&[3, 4]);

    let compatible = checker.validate_tensor_shapes(&shape1, &shape2).unwrap();
    println!("Shapes [3, 4] and [3, 4] compatible: {}", compatible);

    // Incompatible shapes
    let shape3 = create_shape(&[3, 4]);
    let shape4 = create_shape(&[4, 5]);

    let compatible = checker.validate_tensor_shapes(&shape3, &shape4).unwrap();
    println!("Shapes [3, 4] and [4, 5] compatible: {}", compatible);
    println!();
}

/// Example 6: Meta Parameter Types
fn example_meta_parameter_types() {
    println!("Example 6: Meta Parameter Types");
    println!("--------------------------------");

    // Create meta parameter: Shape: meta [usize]
    let shape_ty = Type::array(Type::Int, Some(2));
    let meta_ty = Type::meta("Shape".into(), shape_ty, None);

    println!("Meta parameter type: {}", meta_ty);

    // Create Tensor<Float, Shape>
    let tensor_path = Path::single(Ident::new("Tensor".to_string(), Span::dummy()));
    let tensor_ty = Type::Named {
        path: tensor_path,
        args: vec![Type::Float, meta_ty].into(),
    };

    println!("Tensor type: {}", tensor_ty);
    println!();
}

/// Example 7: Tensor Arithmetic with Meta Parameters
fn example_tensor_arithmetic() {
    println!("Example 7: Tensor Arithmetic with Meta Parameters");
    println!("-------------------------------------------------");

    let mut eval = ConstEvaluator::new();

    // Bind meta parameters
    eval.bind("N".to_string(), ConstValue::UInt(10));
    eval.bind("M".to_string(), ConstValue::UInt(20));

    println!("Meta parameters: N = 10, M = 20");

    // Compute: N + M
    
    let n_path = Path::single(Ident::new("N".to_string(), Span::dummy()));
    let m_path = Path::single(Ident::new("M".to_string(), Span::dummy()));

    let n_var = Expr::new(ExprKind::Path(n_path), Span::dummy());
    let m_var = Expr::new(ExprKind::Path(m_path), Span::dummy());

    let sum = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Box::new(n_var),
            right: Box::new(m_var),
        },
        Span::dummy(),
    );

    let result = eval.eval(&sum).unwrap();
    println!("N + M = {}", result);

    // Compute: N * M
    let n_path2 = Path::single(Ident::new("N".to_string(), Span::dummy()));
    let m_path2 = Path::single(Ident::new("M".to_string(), Span::dummy()));

    let n_var2 = Expr::new(ExprKind::Path(n_path2), Span::dummy());
    let m_var2 = Expr::new(ExprKind::Path(m_path2), Span::dummy());

    let product = Expr::new(
        ExprKind::Binary {
            op: BinOp::Mul,
            left: Box::new(n_var2),
            right: Box::new(m_var2),
        },
        Span::dummy(),
    );

    let result = eval.eval(&product).unwrap();
    println!("N * M = {}", result);
    println!();
}

// Helper functions

fn create_int_lit(n: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::int(n as i128, Span::dummy())),
        Span::dummy(),
    )
}

fn create_shape(dims: &[i64]) -> Expr {
    let elements: List<Expr> = dims.iter().map(|&n| create_int_lit(n)).collect();
    Expr::new(ExprKind::Array(ArrayExpr::List(elements)), Span::dummy())
}

fn create_binary(op: BinOp, left: i64, right: i64) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(create_int_lit(left)),
            right: Box::new(create_int_lit(right)),
        },
        Span::dummy(),
    )
}
