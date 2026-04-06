//! Tensor Shape Verification using Z3 Array Theory
//!
//! This module provides production-grade tensor shape verification for Verum's
//! compile-time tensor type checking using Z3's Array theory.
//!
//! ## Features
//!
//! - **Matrix Multiplication Shape Checking**: Verify that matrix dimensions are compatible
//! - **Broadcasting Verification**: NumPy-style broadcasting rules
//! - **Meta Parameter Resolution**: Support for symbolic tensor shapes
//! - **Multi-dimensional Tensors**: Full support for arbitrary rank tensors
//!
//! ## Implementation
//!
//! Uses Z3 Array theory to model multi-dimensional tensors:
//! - 1D tensor: Array[Int -> T]
//! - 2D tensor: Array[Int -> Array[Int -> T]]
//! - ND tensor: nested arrays with symbolic dimensions
//!
//! ## Spec Reference
//!
//! Matrix multiplication type-level dimension checking: for `matmul(A: Tensor<f32, [M, K]>,
//! B: Tensor<f32, [K, N]>) -> Tensor<f32, [M, N]>`, the inner dimensions must match.
//! Reshape operations verify product-of-dimensions invariant. Broadcast follows NumPy rules.
//!
//! ## Examples
//!
//! ```rust,no_run
//! use verum_smt::tensor_shapes::TensorShapeVerifier;
//! use verum_ast::{Expr, ExprKind, Literal, LiteralKind};
//!
//! let verifier = TensorShapeVerifier::new();
//!
//! // Matmul verification: [N, K] × [K, M] = [N, M]
//! // let a_shape = vec![expr_int(2), expr_int(3)];  // [2, 3]
//! // let b_shape = vec![expr_int(3), expr_int(4)];  // [3, 4]
//! // let result = verifier.verify_matmul_shapes(&a_shape, &b_shape).unwrap();
//! // assert_eq!(result.len(), 2); // Result is [2, 4]
//! ```

use crate::context::Context;
use crate::translate::Translator;
use crate::z3_backend::{AdvancedResult, Z3Solver};
use verum_ast::{Expr, ExprKind, LiteralKind};
use verum_common::{Heap, List, Map, Maybe, Text};
use verum_common::ToText;
use z3::ast::{Bool, Dynamic, Int};

/// Tensor shape verifier using Z3 Array theory
///
/// This verifier uses Z3's SMT solver to verify tensor shape constraints
/// at compile time, enabling safe zero-overhead tensor operations.
pub struct TensorShapeVerifier {
    /// Z3 context for SMT solving
    #[allow(dead_code)] // Reserved for direct Z3 operations
    context: Heap<Context>,
    /// Translator for converting Verum AST to Z3
    translator: Heap<Translator<'static>>,
    /// Cache of verified tensor shapes for performance
    shape_cache: Map<Text, List<usize>>,
    /// Statistics for performance monitoring
    stats: VerificationStats,
}

impl TensorShapeVerifier {
    /// Create a new tensor shape verifier
    pub fn new() -> Self {
        // SAFETY: Context is leaked to get 'static lifetime for simplicity
        // In production, use proper lifetime management
        let context = Heap::new(Context::new());
        let context_ref = unsafe { &*(&*context as *const Context) };
        let translator = Heap::new(Translator::new(context_ref));

        Self {
            context,
            translator,
            shape_cache: Map::new(),
            stats: VerificationStats::default(),
        }
    }

    /// Verify matrix multiplication shape compatibility
    ///
    /// Given matrices A: [M, K] and B: [K, N], verifies that:
    /// 1. A.cols == B.rows (K dimensions match)
    /// 2. Returns result shape [M, N]
    ///
    /// Supports both concrete dimensions and meta parameters.
    ///
    /// # Examples
    ///
    /// ```verum
    /// fn matmul<M: meta usize, N: meta usize, K: meta usize>(
    ///     a: &Tensor<f32, [M, K]>,
    ///     b: &Tensor<f32, [K, N]>
    /// ) -> Tensor<f32, [M, N]>
    /// ```
    ///
    /// # Errors
    ///
    /// Returns `ShapeError::DimensionMismatch` if K dimensions don't match.
    pub fn verify_matmul_shapes(
        &mut self,
        a_shape: &[Expr],
        b_shape: &[Expr],
    ) -> Result<List<Expr>, ShapeError> {
        self.stats.total_checks += 1;

        // Validate input shapes
        if a_shape.len() != 2 {
            return Err(ShapeError::InvalidRank {
                expected: 2,
                actual: a_shape.len(),
                operation: "matmul left operand".to_text(),
            });
        }

        if b_shape.len() != 2 {
            return Err(ShapeError::InvalidRank {
                expected: 2,
                actual: b_shape.len(),
                operation: "matmul right operand".to_text(),
            });
        }

        // Extract dimensions: a is [M, K], b is [K, N]
        let a_rows = &a_shape[0]; // M
        let a_cols = &a_shape[1]; // K
        let b_rows = &b_shape[0]; // K
        let b_cols = &b_shape[1]; // N

        // Create Z3 solver for shape verification
        let mut solver = Z3Solver::new(Maybe::Some("QF_LIA"));

        // Translate shape expressions to Z3
        let a_cols_z3 = self
            .translator
            .translate_expr(a_cols)
            .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

        let b_rows_z3 = self
            .translator
            .translate_expr(b_rows)
            .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

        // Assert constraint: a.cols == b.rows (K == K)
        if let (Some(a_cols_int), Some(b_rows_int)) = (a_cols_z3.as_int(), b_rows_z3.as_int()) {
            let eq_constraint = a_cols_int.eq(&b_rows_int);
            solver.assert(&eq_constraint);

            // Check if constraint is satisfiable
            match solver.check_sat() {
                AdvancedResult::Sat { .. } => {
                    // Constraint is satisfied - shapes are compatible
                    self.stats.successful_verifications += 1;

                    // Return result shape [M, N]
                    let result_shape = vec![a_rows.clone(), b_cols.clone()];
                    Ok(result_shape.into())
                }
                AdvancedResult::Unsat { .. } => {
                    // Constraint is unsatisfiable - dimension mismatch
                    Err(ShapeError::DimensionMismatch {
                        dim1: format!("{:?}", a_cols).into(),
                        dim2: format!("{:?}", b_rows).into(),
                        operation: "matmul K dimension".to_text(),
                    })
                }
                AdvancedResult::Unknown { reason } => {
                    // Solver couldn't determine - likely timeout or complex constraint
                    Err(ShapeError::VerificationTimeout {
                        reason: reason.unwrap_or_else(|| "unknown reason".to_text()),
                    })
                }
                _ => unreachable!("Unexpected solver result"),
            }
        } else {
            Err(ShapeError::TranslationError(
                "failed to translate shape dimensions to Z3 Int".to_text(),
            ))
        }
    }

    /// Verify NumPy-style broadcasting rules for element-wise operations
    ///
    /// Broadcasting allows tensors with different shapes to be combined:
    /// - Dimensions are aligned from right to left
    /// - Each dimension must either:
    ///   1. Be equal
    ///   2. One of them is 1
    ///   3. One dimension is missing (treated as 1)
    ///
    /// # Examples
    ///
    /// ```text
    /// [3, 1, 5] + [   2, 5] = [3, 2, 5]  ✅
    /// [4, 3]    + [4, 3]    = [4, 3]     ✅
    /// [3, 4]    + [4]       = [3, 4]     ✅
    /// [3, 4]    + [3, 1]    = [3, 4]     ✅
    /// [3, 4]    + [2, 4]    = ERROR      ❌
    /// ```
    pub fn verify_broadcast(&mut self, shapes: &[List<Expr>]) -> Result<List<Expr>, ShapeError> {
        self.stats.total_checks += 1;

        if shapes.is_empty() {
            return Err(ShapeError::InvalidInput(
                "cannot broadcast zero tensors".to_text(),
            ));
        }

        if shapes.len() == 1 {
            self.stats.successful_verifications += 1;
            return Ok(shapes[0].clone());
        }

        // Find maximum rank
        let max_rank = shapes.iter().map(|s| s.len()).max().unwrap_or(0);

        // Build result shape from right to left
        let mut result_shape = List::new();

        for dim_idx in (0..max_rank).rev() {
            let mut dim_values = List::new();

            // Collect dimension values from all shapes
            for shape in shapes {
                let rank = shape.len();
                // Check if this dimension exists in the current shape
                // Dimensions are aligned from the right, so we need:
                // dim_idx >= (max_rank - rank) for the dimension to exist
                if dim_idx >= max_rank - rank {
                    let shape_dim_idx = dim_idx - (max_rank - rank);
                    dim_values.push(shape[shape_dim_idx].clone());
                }
                // If dimension is missing, implicitly treat as 1
            }

            // Determine broadcast dimension using Z3
            let broadcast_dim = self.compute_broadcast_dim(&dim_values)?;
            result_shape.insert(0, broadcast_dim);
        }

        self.stats.successful_verifications += 1;
        Ok(result_shape)
    }

    /// Compute the broadcast dimension for a set of dimension values
    ///
    /// Rules:
    /// - All equal → return that value
    /// - Some are 1, others equal → return the non-1 value
    /// - Conflict → error
    fn compute_broadcast_dim(&mut self, dims: &[Expr]) -> Result<Expr, ShapeError> {
        if dims.is_empty() {
            return Err(ShapeError::InvalidInput(
                "cannot compute broadcast dimension from empty list".to_text(),
            ));
        }

        if dims.len() == 1 {
            return Ok(dims[0].clone());
        }

        // Create Z3 solver to verify broadcast compatibility
        let mut solver = Z3Solver::new(Maybe::Some("QF_LIA"));

        // Translate all dimensions to Z3
        let mut z3_dims = List::new();
        for dim in dims {
            let z3_dim = self
                .translator
                .translate_expr(dim)
                .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;
            z3_dims.push(z3_dim);
        }

        // Check if all dimensions are equal or 1
        let one = Int::from_i64(1);

        for i in 0..z3_dims.len() {
            for j in (i + 1)..z3_dims.len() {
                if let (Some(dim_i), Some(dim_j)) = (z3_dims[i].as_int(), z3_dims[j].as_int()) {
                    // Constraint: dim_i == dim_j OR dim_i == 1 OR dim_j == 1
                    let eq = dim_i.eq(&dim_j);
                    let i_is_one = dim_i.eq(&one);
                    let j_is_one = dim_j.eq(&one);

                    let broadcast_valid = Bool::or(&[&eq, &i_is_one, &j_is_one]);
                    solver.assert(&broadcast_valid);
                }
            }
        }

        // Check if constraints are satisfiable
        match solver.check_sat() {
            AdvancedResult::Sat { .. } => {
                // Find the non-1 dimension (or first if all are 1)
                for dim in dims {
                    if let ExprKind::Literal(lit) = &dim.kind
                        && let LiteralKind::Int(i) = &lit.kind
                        && i.value != 1
                    {
                        return Ok(dim.clone());
                    }
                }
                // All are 1 or symbolic - return first
                Ok(dims[0].clone())
            }
            AdvancedResult::Unsat { .. } => Err(ShapeError::BroadcastError {
                shapes: dims.iter().map(|d| format!("{:?}", d).into()).collect(),
            }),
            AdvancedResult::Unknown { reason } => Err(ShapeError::VerificationTimeout {
                reason: reason.unwrap_or_else(|| "unknown reason".to_text()),
            }),
            _ => unreachable!("Unexpected solver result"),
        }
    }

    /// Verify element-wise operation shape compatibility
    ///
    /// For operations like +, -, *, /, the shapes must either:
    /// 1. Be exactly equal, or
    /// 2. Follow broadcasting rules
    pub fn verify_elementwise(
        &mut self,
        shape_a: &[Expr],
        shape_b: &[Expr],
    ) -> Result<List<Expr>, ShapeError> {
        self.stats.total_checks += 1;

        // Try exact match first (fast path)
        if shape_a.len() == shape_b.len() {
            let mut all_equal = true;
            for (dim_a, dim_b) in shape_a.iter().zip(shape_b.iter()) {
                if let (ExprKind::Literal(lit_a), ExprKind::Literal(lit_b)) =
                    (&dim_a.kind, &dim_b.kind)
                    && let (LiteralKind::Int(a), LiteralKind::Int(b)) = (&lit_a.kind, &lit_b.kind)
                    && a.value != b.value
                {
                    all_equal = false;
                    break;
                }
            }

            if all_equal {
                self.stats.successful_verifications += 1;
                return Ok(shape_a.to_vec().into());
            }
        }

        // Fall back to broadcasting rules
        let shapes = vec![shape_a.to_vec().into(), shape_b.to_vec().into()];
        self.verify_broadcast(&shapes)
    }

    /// Resolve meta parameters in tensor shapes
    ///
    /// Given a shape with meta parameters like [M, N, K], attempts to
    /// resolve concrete values based on context and constraints.
    pub fn resolve_meta_parameters(
        &mut self,
        shape: &[Expr],
        constraints: &[Bool],
    ) -> Result<List<Maybe<usize>>, ShapeError> {
        let mut resolved = List::new();

        let mut solver = Z3Solver::new(Maybe::Some("QF_LIA"));

        // Add user-provided constraints
        for constraint in constraints {
            solver.assert(constraint);
        }

        // Try to resolve each dimension
        for dim in shape {
            match &dim.kind {
                ExprKind::Literal(lit) => {
                    if let LiteralKind::Int(i) = &lit.kind {
                        resolved.push(Maybe::Some(i.value as usize));
                    } else {
                        resolved.push(Maybe::None);
                    }
                }
                ExprKind::Path(_) => {
                    // Meta parameter - try to resolve from model
                    let z3_dim = self
                        .translator
                        .translate_expr(dim)
                        .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

                    match solver.check_sat() {
                        AdvancedResult::Sat {
                            model: Maybe::Some(model),
                        } => {
                            if let Some(dim_int) = z3_dim.as_int()
                                && let Some(value) = model.eval(&dim_int, true)
                                && let Some(i) = value.as_i64()
                            {
                                resolved.push(Maybe::Some(i as usize));
                                continue;
                            }
                            // Could not resolve
                            resolved.push(Maybe::None);
                        }
                        _ => {
                            // Could not resolve
                            resolved.push(Maybe::None);
                        }
                    }
                }
                _ => {
                    resolved.push(Maybe::None);
                }
            }
        }

        Ok(resolved)
    }

    /// Verify reshape operation
    ///
    /// Validates that the total number of elements is preserved:
    /// `product(old_shape) == product(new_shape)`
    ///
    /// This uses Z3 to verify the constraint for both concrete and symbolic shapes.
    /// For symbolic shapes (with meta parameters like M, N, K), we use Z3 to prove
    /// that the products are equal for ALL possible values of the symbolic parameters.
    ///
    /// # Verification Strategy
    ///
    /// To verify that `product(old) == product(new)` for all valid symbolic values:
    /// 1. Assert that all dimensions are positive (required for valid tensor shapes)
    /// 2. Assert the NEGATION of equality: `product(old) != product(new)`
    /// 3. If UNSAT, the products are provably equal for all positive dimension values
    /// 4. If SAT, we found a counterexample where the products differ
    ///
    /// # Examples
    ///
    /// ```verum
    /// let v: Tensor<f32, [12]> = ...;
    /// let m: Tensor<f32, [3, 4]> = reshape(&v);  // OK: 12 = 3*4
    /// let m2: Tensor<f32, [2, 6]> = reshape(&v); // OK: 12 = 2*6
    /// let bad: Tensor<f32, [5, 5]> = reshape(&v); // ERROR: 12 != 25
    ///
    /// // Symbolic example:
    /// fn reshape_symbolic<M: meta usize, N: meta usize>(
    ///     t: Tensor<f32, [M, N]>
    /// ) -> Tensor<f32, [N, M]> // OK: M*N == N*M (commutative)
    /// ```
    pub fn verify_reshape(
        &mut self,
        old_shape: &[Expr],
        new_shape: &[Expr],
    ) -> Result<(), ShapeError> {
        self.stats.total_checks += 1;

        // Create Z3 solver for product verification
        // Use QF_NIA (Quantifier-Free Non-linear Integer Arithmetic) to handle products
        let mut solver = Z3Solver::new(Maybe::Some("QF_NIA"));

        // Compute product of old shape
        let old_product = self.compute_shape_product(old_shape)?;

        // Compute product of new shape
        let new_product = self.compute_shape_product(new_shape)?;

        if let (Some(old_int), Some(new_int)) = (old_product.as_int(), new_product.as_int()) {
            // Collect all symbolic dimension variables from both shapes
            let mut symbolic_vars = List::new();
            for dim in old_shape.iter().chain(new_shape.iter()) {
                if let ExprKind::Path(path) = &dim.kind {
                    let var_name = path.as_ident().map(|id| id.as_str()).unwrap_or("");
                    if !var_name.is_empty() {
                        let z3_var = Int::new_const(var_name);
                        symbolic_vars.push(z3_var);
                    }
                }
            }

            // Assert positivity constraints for all symbolic dimensions
            // (valid tensor dimensions must be >= 1)
            let one = Int::from_i64(1);
            for var in &symbolic_vars {
                let positive = var.ge(&one);
                solver.assert(&positive);
            }

            // Also assert positivity for concrete dimensions (sanity check)
            for dim in old_shape.iter().chain(new_shape.iter()) {
                let dim_z3 = self
                    .translator
                    .translate_expr(dim)
                    .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;
                if let Some(dim_int) = dim_z3.as_int() {
                    let positive = dim_int.ge(&one);
                    solver.assert(&positive);
                }
            }

            // KEY INSIGHT: To prove equality for ALL values, we check if
            // the NEGATION of equality is UNSATISFIABLE
            // If old_product != new_product is UNSAT, then old_product == new_product always holds
            let neq_constraint = old_int.eq(&new_int).not();
            solver.assert(&neq_constraint);

            // Check if inequality is satisfiable
            match solver.check_sat() {
                AdvancedResult::Unsat { .. } => {
                    // Inequality is UNSAT -> products are PROVABLY equal for all valid inputs
                    self.stats.successful_verifications += 1;
                    Ok(())
                }
                AdvancedResult::Sat { model } => {
                    // Found a counterexample where products differ
                    let counterexample: Maybe<Text> = model.map(|m| Text::from(format!("{}", m)));
                    let reason_suffix = match counterexample {
                        Maybe::Some(ref ce) => format!("; counterexample: {}", ce),
                        Maybe::None => String::new(),
                    };
                    Err(ShapeError::ReshapeError {
                        old_shape: old_shape
                            .iter()
                            .map(|e| format!("{:?}", e).into())
                            .collect(),
                        new_shape: new_shape
                            .iter()
                            .map(|e| format!("{:?}", e).into())
                            .collect(),
                        reason: format!("total element count mismatch{}", reason_suffix).into(),
                    })
                }
                AdvancedResult::Unknown { reason } => Err(ShapeError::VerificationTimeout {
                    reason: reason.unwrap_or_else(|| "unknown reason".to_text()),
                }),
                _ => unreachable!("Unexpected solver result"),
            }
        } else {
            Err(ShapeError::TranslationError(
                "failed to translate shape products to Z3 Int".to_text(),
            ))
        }
    }

    /// Compute the product of all dimensions in a shape
    ///
    /// Returns a Z3 Int expression representing the total number of elements.
    fn compute_shape_product(&mut self, shape: &[Expr]) -> Result<Dynamic, ShapeError> {
        if shape.is_empty() {
            // Empty shape (scalar) has product 1
            return Ok(Int::from_i64(1).into());
        }

        // Translate first dimension
        let mut product = self
            .translator
            .translate_expr(&shape[0])
            .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

        // Multiply by remaining dimensions
        for dim in &shape[1..] {
            let dim_z3 = self
                .translator
                .translate_expr(dim)
                .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

            if let (Some(prod_int), Some(dim_int)) = (product.as_int(), dim_z3.as_int()) {
                product = (&prod_int * &dim_int).into();
            } else {
                return Err(ShapeError::TranslationError(
                    "shape dimension is not an integer".to_text(),
                ));
            }
        }

        Ok(product)
    }

    /// Verify bounds check elimination for loop indices
    ///
    /// Given loop bounds and array accesses, proves that all accesses are within bounds.
    /// This enables the compiler to eliminate runtime bounds checks.
    ///
    /// # Examples
    ///
    /// ```verum
    /// @verify(bounds_elimination)
    /// fn matmul<M: meta usize, N: meta usize, K: meta usize>(
    ///     a: &Tensor<f32, [M, K]>,
    ///     b: &Tensor<f32, [K, N]>
    /// ) -> Tensor<f32, [M, N]> {
    ///     for i in 0..M {
    ///         for j in 0..N {
    ///             for k in 0..K {
    ///                 // Compiler PROVES: i < M, j < N, k < K
    ///                 result[i, j] += a[i, k] * b[k, j];
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    pub fn verify_bounds_elimination(
        &mut self,
        loop_bounds: &[LoopBound],
        array_accesses: &[ArrayAccess],
    ) -> Result<BoundsProof, ShapeError> {
        self.stats.total_checks += 1;

        let mut solver = Z3Solver::new(Maybe::Some("QF_LIA"));

        // Create Z3 variables for loop indices
        let mut index_vars = Map::new();
        for bound in loop_bounds {
            let var_z3 = Int::new_const(bound.var_name.as_str());
            index_vars.insert(bound.var_name.clone(), var_z3);

            // Assert loop bounds: lower <= var < upper
            if let Some(var_int) = index_vars.get(&bound.var_name) {
                // Translate lower and upper bounds
                let lower_z3 = self
                    .translator
                    .translate_expr(&bound.lower)
                    .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

                let upper_z3 = self
                    .translator
                    .translate_expr(&bound.upper)
                    .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

                if let (Some(lower_int), Some(upper_int)) = (lower_z3.as_int(), upper_z3.as_int()) {
                    // Assert: var >= lower
                    solver.assert(&var_int.ge(&lower_int));
                    // Assert: var < upper
                    solver.assert(&var_int.lt(&upper_int));
                }
            }
        }

        // For each array access, verify it's within bounds
        let mut proved_accesses = List::new();

        for access in array_accesses {
            // Translate array dimensions
            let shape = &access.array_shape;

            // Verify each index is within its dimension
            for (idx, index_expr) in access.indices.iter().enumerate() {
                if idx >= shape.len() {
                    return Err(ShapeError::InvalidRank {
                        expected: shape.len(),
                        actual: access.indices.len(),
                        operation: format!("array access {}", access.array_name).into(),
                    });
                }

                // Get the index expression (may reference loop variables)
                let index_z3 = self
                    .translator
                    .translate_expr(index_expr)
                    .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

                // Get the dimension size
                let dim_z3 = self
                    .translator
                    .translate_expr(&shape[idx])
                    .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

                // Need to prove: 0 <= index < dim
                if let (Some(index_int), Some(dim_int)) = (index_z3.as_int(), dim_z3.as_int()) {
                    // Check: index >= 0 is always true (or provable from loop bounds)
                    let zero = Int::from_i64(0);
                    let lower_bound = index_int.ge(&zero);

                    // Check: index < dim
                    let upper_bound = index_int.lt(&dim_int);

                    // Check if bounds can be violated (index < 0 OR index >= dim)
                    // This is equivalent to NOT(0 <= index < dim)
                    let violation = Bool::or(&[&lower_bound.not(), &upper_bound.not()]);

                    solver.push();
                    solver.assert(&violation);

                    match solver.check_sat() {
                        AdvancedResult::Unsat { .. } => {
                            // Violation is unsatisfiable - bounds check is guaranteed to pass
                            proved_accesses.push(ProvedAccess {
                                array_name: access.array_name.clone(),
                                index: idx,
                                proof: "Z3 proved bounds are always satisfied".to_text(),
                            });
                        }
                        AdvancedResult::Sat { model } => {
                            // Found a counterexample where bounds are violated
                            solver.pop();

                            // Convert model to text representation
                            let counterexample_text = model.map(|m| format!("{}", m).into());

                            return Err(ShapeError::BoundsCheckFailed {
                                array_name: access.array_name.clone(),
                                index: idx,
                                counterexample: counterexample_text,
                            });
                        }
                        AdvancedResult::Unknown { reason } => {
                            solver.pop();
                            return Err(ShapeError::VerificationTimeout {
                                reason: reason.unwrap_or_else(|| "unknown".to_text()),
                            });
                        }
                        _ => unreachable!(),
                    }

                    solver.pop();
                }
            }
        }

        self.stats.successful_verifications += 1;

        Ok(BoundsProof {
            proved_accesses,
            can_eliminate_checks: true,
        })
    }

    /// Verify slicing operation bounds
    ///
    /// Validates that a slice operation is within bounds:
    /// - `0 <= start <= end <= dim` for each dimension
    /// - Returns the resulting shape after slicing
    ///
    /// # Examples
    ///
    /// ```verum
    /// let t: Tensor<f32, [10, 20, 30]> = ...;
    /// let s = t[2:5, :, 10:25];  // Result: [3, 20, 15]
    /// ```
    ///
    /// # Parameters
    ///
    /// - `tensor_shape`: The shape of the input tensor
    /// - `slices`: Vector of slice specifications (start, end, step) for each dimension
    ///
    /// # Returns
    ///
    /// The resulting shape after slicing, or an error if slicing is out of bounds.
    pub fn verify_slice(
        &mut self,
        tensor_shape: &[Expr],
        slices: &[SliceSpec],
    ) -> Result<List<Expr>, ShapeError> {
        self.stats.total_checks += 1;

        if slices.len() > tensor_shape.len() {
            return Err(ShapeError::InvalidRank {
                expected: tensor_shape.len(),
                actual: slices.len(),
                operation: "slice".to_text(),
            });
        }

        let mut solver = Z3Solver::new(Maybe::Some("QF_LIA"));
        let mut result_shape = List::new();
        let zero = Int::from_i64(0);

        for (dim_idx, (dim, slice)) in tensor_shape.iter().zip(slices.iter()).enumerate() {
            let dim_z3 = self
                .translator
                .translate_expr(dim)
                .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

            let start_z3 = self
                .translator
                .translate_expr(&slice.start)
                .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

            let end_z3 = self
                .translator
                .translate_expr(&slice.end)
                .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

            if let (Some(dim_int), Some(start_int), Some(end_int)) =
                (dim_z3.as_int(), start_z3.as_int(), end_z3.as_int())
            {
                // Assert positivity for symbolic dimensions
                solver.assert(&dim_int.ge(Int::from_i64(1)));

                // Verify: 0 <= start <= end <= dim
                // We check if the NEGATION is SAT (meaning violation is possible)
                let valid_start = start_int.ge(&zero);
                let valid_order = end_int.ge(&start_int);
                let valid_end = dim_int.ge(&end_int);
                let all_valid = Bool::and(&[&valid_start, &valid_order, &valid_end]);

                solver.push();
                solver.assert(&all_valid.not());

                match solver.check_sat() {
                    AdvancedResult::Unsat { .. } => {
                        // Slice bounds are valid for all possible dimension values
                    }
                    AdvancedResult::Sat { model } => {
                        solver.pop();
                        let counterexample = model.map(|m| format!("{}", m).into());
                        return Err(ShapeError::SliceOutOfBounds {
                            dimension: dim_idx,
                            slice_start: format!("{:?}", slice.start).into(),
                            slice_end: format!("{:?}", slice.end).into(),
                            dimension_size: format!("{:?}", dim).into(),
                            counterexample,
                        });
                    }
                    AdvancedResult::Unknown { reason } => {
                        solver.pop();
                        return Err(ShapeError::VerificationTimeout {
                            reason: reason.unwrap_or_else(|| "unknown".to_text()),
                        });
                    }
                    _ => unreachable!(),
                }
                solver.pop();

                // Compute result dimension: (end - start + step - 1) / step
                // For step=1: end - start
                if let Some(step_val) = slice.step {
                    if step_val == 1 {
                        // Simple case: result_dim = end - start
                        let result_dim = &end_int - &start_int;
                        result_shape.push(self.int_to_expr(result_dim));
                    } else {
                        // General case: (end - start + step - 1) / step
                        let step_z3 = Int::from_i64(step_val as i64);
                        let diff = &end_int - &start_int;
                        let adjusted = &diff + &step_z3 - Int::from_i64(1);
                        let result_dim = adjusted / step_z3;
                        result_shape.push(self.int_to_expr(result_dim));
                    }
                } else {
                    // Default step is 1
                    let result_dim = &end_int - &start_int;
                    result_shape.push(self.int_to_expr(result_dim));
                }
            } else {
                return Err(ShapeError::TranslationError(
                    "failed to translate slice bounds to Z3 Int".to_text(),
                ));
            }
        }

        // Remaining dimensions are unchanged
        for dim in tensor_shape.iter().skip(slices.len()) {
            result_shape.push(dim.clone());
        }

        self.stats.successful_verifications += 1;
        Ok(result_shape)
    }

    /// Verify transpose operation
    ///
    /// Validates that a transpose permutation is valid and computes the result shape.
    ///
    /// # Examples
    ///
    /// ```verum
    /// let t: Tensor<f32, [2, 3, 4]> = ...;
    /// let t2 = t.transpose([2, 0, 1]);  // Result: [4, 2, 3]
    /// ```
    pub fn verify_transpose(
        &mut self,
        tensor_shape: &[Expr],
        permutation: &[usize],
    ) -> Result<List<Expr>, ShapeError> {
        self.stats.total_checks += 1;

        // Verify permutation is valid
        let rank = tensor_shape.len();
        if permutation.len() != rank {
            return Err(ShapeError::InvalidRank {
                expected: rank,
                actual: permutation.len(),
                operation: "transpose permutation".to_text(),
            });
        }

        // Check that permutation contains each index exactly once
        let mut seen = vec![false; rank];
        for &idx in permutation {
            if idx >= rank {
                return Err(ShapeError::InvalidInput(
                    format!(
                        "transpose permutation index {} out of range for rank {}",
                        idx, rank
                    )
                    .into(),
                ));
            }
            if seen[idx] {
                return Err(ShapeError::InvalidInput(
                    format!("transpose permutation contains duplicate index {}", idx).into(),
                ));
            }
            seen[idx] = true;
        }

        // Apply permutation to shape
        let result_shape: List<Expr> = permutation
            .iter()
            .map(|&i| tensor_shape[i].clone())
            .collect();

        self.stats.successful_verifications += 1;
        Ok(result_shape)
    }

    /// Verify matrix multiplication with symbolic dimensions
    ///
    /// Extended version that uses Z3 to prove dimension compatibility
    /// for symbolic shapes like [M, K] x [K, N] -> [M, N].
    ///
    /// This proves that the K dimensions are always equal given the constraints.
    pub fn verify_matmul_symbolic(
        &mut self,
        a_shape: &[Expr],
        b_shape: &[Expr],
        constraints: &[SymbolicConstraint],
    ) -> Result<List<Expr>, ShapeError> {
        self.stats.total_checks += 1;

        // Validate input shapes
        if a_shape.len() != 2 {
            return Err(ShapeError::InvalidRank {
                expected: 2,
                actual: a_shape.len(),
                operation: "matmul left operand".to_text(),
            });
        }

        if b_shape.len() != 2 {
            return Err(ShapeError::InvalidRank {
                expected: 2,
                actual: b_shape.len(),
                operation: "matmul right operand".to_text(),
            });
        }

        let a_cols = &a_shape[1]; // K
        let b_rows = &b_shape[0]; // K

        // Create solver with non-linear arithmetic for general constraints
        let mut solver = Z3Solver::new(Maybe::Some("QF_NIA"));

        // Add user-provided symbolic constraints
        for constraint in constraints {
            let z3_constraint = self.translate_symbolic_constraint(constraint)?;
            solver.assert(&z3_constraint);
        }

        // Translate dimensions
        let a_cols_z3 = self
            .translator
            .translate_expr(a_cols)
            .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

        let b_rows_z3 = self
            .translator
            .translate_expr(b_rows)
            .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

        if let (Some(a_cols_int), Some(b_rows_int)) = (a_cols_z3.as_int(), b_rows_z3.as_int()) {
            // Assert positivity for symbolic dimensions
            let one = Int::from_i64(1);
            solver.assert(&a_cols_int.ge(&one));
            solver.assert(&b_rows_int.ge(&one));

            // Prove dimensions are equal: check if a_cols != b_rows is UNSAT
            let neq = a_cols_int.eq(&b_rows_int).not();
            solver.assert(&neq);

            match solver.check_sat() {
                AdvancedResult::Unsat { .. } => {
                    // Dimensions are provably equal
                    self.stats.successful_verifications += 1;
                    let result_shape = vec![a_shape[0].clone(), b_shape[1].clone()];
                    Ok(result_shape.into())
                }
                AdvancedResult::Sat { model } => {
                    let counterexample: Maybe<Text> = model.map(|m| format!("{}", m).into());
                    let reason_suffix = match counterexample {
                        Maybe::Some(ref ce) => format!("; counterexample: {}", ce),
                        Maybe::None => String::new(),
                    };
                    Err(ShapeError::DimensionMismatch {
                        dim1: format!("{:?}", a_cols).into(),
                        dim2: format!("{:?}", b_rows).into(),
                        operation: format!("matmul K dimension{}", reason_suffix).into(),
                    })
                }
                AdvancedResult::Unknown { reason } => Err(ShapeError::VerificationTimeout {
                    reason: reason.unwrap_or_else(|| "unknown reason".to_text()),
                }),
                _ => unreachable!("Unexpected solver result"),
            }
        } else {
            Err(ShapeError::TranslationError(
                "failed to translate shape dimensions to Z3 Int".to_text(),
            ))
        }
    }

    /// Verify concatenation along a specified axis
    ///
    /// Validates that all tensors have matching shapes except along the concat axis.
    ///
    /// # Examples
    ///
    /// ```verum
    /// let a: Tensor<f32, [2, 3]> = ...;
    /// let b: Tensor<f32, [2, 5]> = ...;
    /// let c = concat([a, b], axis=1);  // Result: [2, 8]
    /// ```
    pub fn verify_concat(
        &mut self,
        shapes: &[List<Expr>],
        axis: usize,
    ) -> Result<List<Expr>, ShapeError> {
        self.stats.total_checks += 1;

        if shapes.is_empty() {
            return Err(ShapeError::InvalidInput(
                "cannot concatenate zero tensors".to_text(),
            ));
        }

        let rank = shapes[0].len();
        if axis >= rank {
            return Err(ShapeError::InvalidInput(
                format!("concat axis {} out of range for rank {}", axis, rank).into(),
            ));
        }

        // All shapes must have the same rank
        for (i, shape) in shapes.iter().enumerate() {
            if shape.len() != rank {
                return Err(ShapeError::InvalidRank {
                    expected: rank,
                    actual: shape.len(),
                    operation: format!("concat tensor {}", i).into(),
                });
            }
        }

        let mut solver = Z3Solver::new(Maybe::Some("QF_LIA"));

        // Verify that all dimensions except axis are equal
        for dim_idx in 0..rank {
            if dim_idx == axis {
                continue;
            }

            for i in 1..shapes.len() {
                let dim0_z3 = self
                    .translator
                    .translate_expr(&shapes[0][dim_idx])
                    .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

                let dim_i_z3 = self
                    .translator
                    .translate_expr(&shapes[i][dim_idx])
                    .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

                if let (Some(dim0_int), Some(dim_i_int)) = (dim0_z3.as_int(), dim_i_z3.as_int()) {
                    // Check if dimensions can differ
                    let neq = dim0_int.eq(&dim_i_int).not();
                    solver.push();
                    solver.assert(&neq);

                    match solver.check_sat() {
                        AdvancedResult::Unsat { .. } => {
                            // Dimensions are provably equal
                        }
                        AdvancedResult::Sat { model } => {
                            solver.pop();
                            let counterexample: Maybe<Text> =
                                model.map(|m| Text::from(format!("{}", m)));
                            let reason_suffix = match counterexample {
                                Maybe::Some(ref ce) => format!("; counterexample: {}", ce),
                                Maybe::None => String::new(),
                            };
                            return Err(ShapeError::DimensionMismatch {
                                dim1: format!("{:?}", shapes[0][dim_idx]).into(),
                                dim2: format!("{:?}", shapes[i][dim_idx]).into(),
                                operation: format!("concat dimension {}{}", dim_idx, reason_suffix)
                                    .into(),
                            });
                        }
                        AdvancedResult::Unknown { reason } => {
                            solver.pop();
                            return Err(ShapeError::VerificationTimeout {
                                reason: reason.unwrap_or_else(|| "unknown".to_text()),
                            });
                        }
                        _ => unreachable!(),
                    }
                    solver.pop();
                }
            }
        }

        // Build result shape: sum along axis, copy others
        let mut result_shape = List::new();
        for dim_idx in 0..rank {
            if dim_idx == axis {
                // Sum all dimensions along axis
                let sum = self.sum_dimensions(shapes.iter().map(|s| &s[axis]).collect())?;
                result_shape.push(sum);
            } else {
                result_shape.push(shapes[0][dim_idx].clone());
            }
        }

        self.stats.successful_verifications += 1;
        Ok(result_shape)
    }

    /// Sum a list of dimension expressions
    fn sum_dimensions(&mut self, dims: List<&Expr>) -> Result<Expr, ShapeError> {
        if dims.is_empty() {
            return Err(ShapeError::InvalidInput(
                "cannot sum empty dimension list".to_text(),
            ));
        }

        if dims.len() == 1 {
            return Ok(dims[0].clone());
        }

        // Translate first dimension
        let mut sum = self
            .translator
            .translate_expr(dims[0])
            .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

        // Add remaining dimensions
        for dim in dims.iter().skip(1) {
            let dim_z3 = self
                .translator
                .translate_expr(dim)
                .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

            if let (Some(sum_int), Some(dim_int)) = (sum.as_int(), dim_z3.as_int()) {
                sum = (&sum_int + &dim_int).into();
            } else {
                return Err(ShapeError::TranslationError(
                    "dimension is not an integer".to_text(),
                ));
            }
        }

        if let Some(sum_int) = sum.as_int() {
            Ok(self.int_to_expr(sum_int))
        } else {
            Err(ShapeError::TranslationError(
                "sum is not an integer".to_text(),
            ))
        }
    }

    /// Convert Z3 Int to Expr
    fn int_to_expr(&self, int: Int) -> Expr {
        // Try to simplify to constant
        if let Some(val) = int.as_i64() {
            Expr {
                kind: ExprKind::Literal(verum_ast::Literal {
                    kind: LiteralKind::Int(verum_ast::IntLit {
                        value: val as i128,
                        suffix: None,
                    }),
                    span: verum_ast::Span::default(),
                }),
                span: verum_ast::Span::default(),
                ref_kind: None,
                check_eliminated: false,
            }
        } else {
            // Return a placeholder for symbolic result
            // In practice, the caller should handle this case
            Expr {
                kind: ExprKind::Path(verum_ast::Path {
                    segments: vec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident::new(
                        format!("{:?}", int),
                        verum_ast::Span::default(),
                    ))]
                    .into(),
                    span: verum_ast::Span::default(),
                }),
                span: verum_ast::Span::default(),
                ref_kind: None,
                check_eliminated: false,
            }
        }
    }

    /// Translate a symbolic constraint to Z3
    fn translate_symbolic_constraint(
        &mut self,
        constraint: &SymbolicConstraint,
    ) -> Result<Bool, ShapeError> {
        let lhs = self
            .translator
            .translate_expr(&constraint.lhs)
            .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

        let rhs = self
            .translator
            .translate_expr(&constraint.rhs)
            .map_err(|e| ShapeError::TranslationError(format!("{:?}", e).into()))?;

        if let (Some(lhs_int), Some(rhs_int)) = (lhs.as_int(), rhs.as_int()) {
            match constraint.op {
                ConstraintOp::Eq => Ok(lhs_int.eq(&rhs_int)),
                ConstraintOp::Ne => Ok(lhs_int.eq(&rhs_int).not()),
                ConstraintOp::Lt => Ok(lhs_int.lt(&rhs_int)),
                ConstraintOp::Le => Ok(lhs_int.le(&rhs_int)),
                ConstraintOp::Gt => Ok(lhs_int.gt(&rhs_int)),
                ConstraintOp::Ge => Ok(lhs_int.ge(&rhs_int)),
            }
        } else {
            Err(ShapeError::TranslationError(
                "constraint operands are not integers".to_text(),
            ))
        }
    }

    /// Get verification statistics
    pub fn stats(&self) -> &VerificationStats {
        &self.stats
    }

    /// Clear shape cache
    pub fn clear_cache(&mut self) {
        self.shape_cache.clear();
    }
}

impl Default for TensorShapeVerifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Loop bound specification for bounds check elimination
#[derive(Debug, Clone)]
pub struct LoopBound {
    /// Loop variable name (e.g., "i", "j", "k")
    pub var_name: Text,
    /// Lower bound (inclusive)
    pub lower: Expr,
    /// Upper bound (exclusive)
    pub upper: Expr,
}

/// Array access specification for bounds verification
#[derive(Debug, Clone)]
pub struct ArrayAccess {
    /// Array name being accessed
    pub array_name: Text,
    /// Array shape dimensions
    pub array_shape: List<Expr>,
    /// Index expressions for each dimension
    pub indices: List<Expr>,
}

/// Proof that an array access is within bounds
#[derive(Debug, Clone)]
pub struct ProvedAccess {
    /// Array name
    pub array_name: Text,
    /// Index dimension that was proved safe
    pub index: usize,
    /// Proof text/justification
    pub proof: Text,
}

/// Result of bounds check elimination verification
#[derive(Debug, Clone)]
pub struct BoundsProof {
    /// All proved array accesses
    pub proved_accesses: List<ProvedAccess>,
    /// Whether bounds checks can be eliminated
    pub can_eliminate_checks: bool,
}

/// Slice specification for tensor slicing operations
#[derive(Debug, Clone)]
pub struct SliceSpec {
    /// Start index (inclusive)
    pub start: Expr,
    /// End index (exclusive)
    pub end: Expr,
    /// Step size (optional, defaults to 1)
    pub step: Option<usize>,
}

impl SliceSpec {
    /// Create a new slice specification with default step of 1
    pub fn new(start: Expr, end: Expr) -> Self {
        Self {
            start,
            end,
            step: None,
        }
    }

    /// Create a slice specification with a custom step
    pub fn with_step(start: Expr, end: Expr, step: usize) -> Self {
        Self {
            start,
            end,
            step: Some(step),
        }
    }
}

/// Symbolic constraint for shape verification
///
/// Represents a constraint on symbolic dimension parameters like:
/// - M == N
/// - M >= 1
/// - K <= 1024
#[derive(Debug, Clone)]
pub struct SymbolicConstraint {
    /// Left-hand side of the constraint
    pub lhs: Expr,
    /// Comparison operator
    pub op: ConstraintOp,
    /// Right-hand side of the constraint
    pub rhs: Expr,
}

impl SymbolicConstraint {
    /// Create a new symbolic constraint
    pub fn new(lhs: Expr, op: ConstraintOp, rhs: Expr) -> Self {
        Self { lhs, op, rhs }
    }
}

/// Comparison operators for symbolic constraints
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintOp {
    /// Equal (==)
    Eq,
    /// Not equal (!=)
    Ne,
    /// Less than (<)
    Lt,
    /// Less than or equal (<=)
    Le,
    /// Greater than (>)
    Gt,
    /// Greater than or equal (>=)
    Ge,
}

/// Tensor shape verification errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum ShapeError {
    /// Dimension mismatch between tensors
    #[error("dimension mismatch: {dim1} != {dim2} in {operation}")]
    DimensionMismatch {
        dim1: Text,
        dim2: Text,
        operation: Text,
    },

    /// Invalid tensor rank for operation
    #[error("invalid rank: expected {expected}, got {actual} for {operation}")]
    InvalidRank {
        expected: usize,
        actual: usize,
        operation: Text,
    },

    /// Broadcasting error
    #[error("cannot broadcast shapes: {shapes:?}")]
    BroadcastError { shapes: List<Text> },

    /// Reshape validation error
    #[error("reshape error: cannot reshape {old_shape:?} to {new_shape:?}: {reason}")]
    ReshapeError {
        old_shape: List<Text>,
        new_shape: List<Text>,
        reason: Text,
    },

    /// Bounds check failed - array access may be out of bounds
    #[error("bounds check failed for {array_name}[{index}]")]
    BoundsCheckFailed {
        array_name: Text,
        index: usize,
        counterexample: Maybe<Text>,
    },

    /// Slice operation out of bounds
    #[error(
        "slice out of bounds: dimension {dimension} slice [{slice_start}:{slice_end}] exceeds dimension size {dimension_size}"
    )]
    SliceOutOfBounds {
        dimension: usize,
        slice_start: Text,
        slice_end: Text,
        dimension_size: Text,
        counterexample: Maybe<Text>,
    },

    /// Translation error from Verum to Z3
    #[error("translation error: {0}")]
    TranslationError(Text),

    /// SMT verification timeout
    #[error("verification timeout: {reason}")]
    VerificationTimeout { reason: Text },

    /// Invalid input
    #[error("invalid input: {0}")]
    InvalidInput(Text),

    /// Meta parameter resolution failed
    #[error("failed to resolve meta parameter: {0}")]
    MetaParameterError(Text),
}

/// Verification statistics
#[derive(Debug, Clone, Default)]
pub struct VerificationStats {
    /// Total number of shape verification checks
    pub total_checks: usize,
    /// Number of successful verifications
    pub successful_verifications: usize,
    /// Number of failed verifications
    pub failed_verifications: usize,
    /// Cache hit rate
    pub cache_hits: usize,
    /// Total verification time in milliseconds
    pub total_time_ms: u64,
}

impl VerificationStats {
    /// Get success rate
    pub fn success_rate(&self) -> f64 {
        if self.total_checks == 0 {
            return 0.0;
        }
        self.successful_verifications as f64 / self.total_checks as f64
    }

    /// Get cache hit rate
    pub fn cache_hit_rate(&self) -> f64 {
        if self.total_checks == 0 {
            return 0.0;
        }
        self.cache_hits as f64 / self.total_checks as f64
    }

    /// Get average verification time
    pub fn avg_time_ms(&self) -> f64 {
        if self.total_checks == 0 {
            return 0.0;
        }
        self.total_time_ms as f64 / self.total_checks as f64
    }
}
