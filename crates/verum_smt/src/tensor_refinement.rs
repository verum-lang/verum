//! Tensor Refinement Type Integration
//!
//! This module integrates tensor shape verification with Verum's refinement type system,
//! allowing tensor constraints to be verified alongside other refinement predicates.
//!
//! ## Features
//!
//! - Refinement types for tensor shapes: `Tensor<f32, [M, N]>{M > 0 && N > 0}`
//! - Integration with existing refinement verification pipeline
//! - Automatic shape constraint extraction from tensor operations
//! - Compositional verification of tensor programs
//!
//! ## Examples
//!
//! ```verum
//! type PositiveTensor<T, Shape> = Tensor<T, Shape>{
//!     forall i in 0..len(Shape): Shape[i] > 0
//! }
//!
//! fn safe_matmul<M: meta usize, N: meta usize, K: meta usize>(
//!     a: &PositiveTensor<f32, [M, K]>,
//!     b: &PositiveTensor<f32, [K, N]>
//! ) -> PositiveTensor<f32, [M, N]>
//! ```

use crate::context::Context;
use crate::refinement::RefinementVerifier;
use crate::tensor_shapes::{ShapeError, TensorShapeVerifier};
use crate::translate::{TensorSort, Translator};
use verum_ast::{Expr, Type, TypeKind};
use verum_common::{Heap, List, Text};

/// Tensor refinement verifier
///
/// Integrates tensor shape verification with refinement type checking,
/// providing a unified interface for verifying both value constraints
/// and shape constraints on tensors.
pub struct TensorRefinementVerifier {
    /// Underlying refinement verifier for value constraints
    #[allow(dead_code)] // Reserved for unified constraint verification
    refinement_verifier: Heap<RefinementVerifier>,
    /// Tensor shape verifier for dimension constraints
    shape_verifier: Heap<TensorShapeVerifier>,
    /// Z3 context for constraint solving
    context: Heap<Context>,
}

impl TensorRefinementVerifier {
    /// Create a new tensor refinement verifier
    pub fn new() -> Self {
        let context = Heap::new(Context::new());
        let refinement_verifier = Heap::new(RefinementVerifier::new());
        let shape_verifier = Heap::new(TensorShapeVerifier::new());

        Self {
            refinement_verifier,
            shape_verifier,
            context,
        }
    }

    /// Verify a tensor type with refinement constraints
    ///
    /// Checks both:
    /// 1. Shape constraints (dimensions must be compatible)
    /// 2. Value constraints (refinement predicates)
    ///
    /// # Examples
    ///
    /// ```verum
    /// type SquareMatrix<T, N> = Tensor<T, [N, N]>{N > 0}
    /// ```
    pub fn verify_tensor_type(
        &mut self,
        tensor_type: &Type,
    ) -> Result<TensorTypeInfo, TensorRefinementError> {
        match &tensor_type.kind {
            TypeKind::Tensor {
                element,
                shape,
                layout: _,
            } => {
                // Extract shape information
                let tensor_sort = self.extract_tensor_sort(element, shape)?;

                // Check for refinement constraints
                let refinement_constraints = self.extract_refinement_constraints(tensor_type)?;

                Ok(TensorTypeInfo {
                    element_type: element.as_ref().clone(),
                    shape: shape.to_vec().into(),
                    sort: tensor_sort,
                    refinement_predicates: refinement_constraints,
                })
            }

            TypeKind::Refined { base, predicate } => {
                // Refined tensor type: Tensor<T, Shape>{predicate}
                if let TypeKind::Tensor {
                    element,
                    shape,
                    layout: _,
                } = &base.kind
                {
                    let tensor_sort = self.extract_tensor_sort(element, shape)?;

                    // Extract refinement from predicate
                    let refinement_constraints: List<Expr> =
                        List::from(vec![predicate.expr.clone()]);

                    Ok(TensorTypeInfo {
                        element_type: element.as_ref().clone(),
                        shape: shape.to_vec().into(),
                        sort: tensor_sort,
                        refinement_predicates: refinement_constraints,
                    })
                } else {
                    Err(TensorRefinementError::NotATensorType(
                        format!("{:?}", base.kind).into(),
                    ))
                }
            }

            _ => Err(TensorRefinementError::NotATensorType(
                format!("{:?}", tensor_type.kind).into(),
            )),
        }
    }

    /// Verify tensor operation constraints
    ///
    /// Given an operation (matmul, elementwise, etc.) and operand types,
    /// verifies that:
    /// 1. Shapes are compatible
    /// 2. Refinement predicates are preserved
    ///
    /// Returns the result type with propagated constraints.
    pub fn verify_tensor_operation(
        &mut self,
        operation: TensorOperation,
        operands: &[TensorTypeInfo],
    ) -> Result<TensorTypeInfo, TensorRefinementError> {
        match operation {
            TensorOperation::MatMul => {
                if operands.len() != 2 {
                    return Err(TensorRefinementError::InvalidOperandCount {
                        expected: 2,
                        actual: operands.len(),
                    });
                }

                // Verify shapes are compatible for matmul
                let result_shape = self
                    .shape_verifier
                    .verify_matmul_shapes(&operands[0].shape, &operands[1].shape)
                    .map_err(TensorRefinementError::ShapeVerificationError)?;

                // Propagate refinement constraints
                let mut result_predicates = List::new();
                result_predicates.extend(operands[0].refinement_predicates.clone());
                result_predicates.extend(operands[1].refinement_predicates.clone());

                Ok(TensorTypeInfo {
                    element_type: operands[0].element_type.clone(),
                    shape: result_shape,
                    sort: TensorSort {
                        element_type: operands[0].sort.element_type.clone(),
                        dimensions: vec![0, 0].into(), // Will be resolved later
                        ndim: 2,
                    },
                    refinement_predicates: result_predicates,
                })
            }

            TensorOperation::Elementwise => {
                if operands.len() != 2 {
                    return Err(TensorRefinementError::InvalidOperandCount {
                        expected: 2,
                        actual: operands.len(),
                    });
                }

                // Verify shapes are compatible for elementwise operations
                let result_shape = self
                    .shape_verifier
                    .verify_elementwise(&operands[0].shape, &operands[1].shape)
                    .map_err(TensorRefinementError::ShapeVerificationError)?;

                // Propagate refinement constraints
                let mut result_predicates = List::new();
                result_predicates.extend(operands[0].refinement_predicates.clone());
                result_predicates.extend(operands[1].refinement_predicates.clone());

                Ok(TensorTypeInfo {
                    element_type: operands[0].element_type.clone(),
                    shape: result_shape,
                    sort: operands[0].sort.clone(),
                    refinement_predicates: result_predicates,
                })
            }

            TensorOperation::Broadcast => {
                // Verify broadcasting for all operands
                let shapes: List<List<Expr>> = operands.iter().map(|op| op.shape.clone()).collect();

                let result_shape = self
                    .shape_verifier
                    .verify_broadcast(&shapes)
                    .map_err(TensorRefinementError::ShapeVerificationError)?;

                // Merge refinement constraints
                let mut result_predicates = List::new();
                for operand in operands {
                    result_predicates.extend(operand.refinement_predicates.clone());
                }

                Ok(TensorTypeInfo {
                    element_type: operands[0].element_type.clone(),
                    shape: result_shape,
                    sort: operands[0].sort.clone(),
                    refinement_predicates: result_predicates,
                })
            }

            TensorOperation::Reshape => {
                // Reshape preserves total elements but changes shape
                // Refinement constraints on elements are preserved
                if operands.len() != 1 {
                    return Err(TensorRefinementError::InvalidOperandCount {
                        expected: 1,
                        actual: operands.len(),
                    });
                }

                let operand = &operands[0];
                let old_shape = &operand.shape;

                // Get the target shape from operation metadata
                // Note: In a full implementation, this would extract from op.metadata
                // For now, we require the shape to be provided via a separate method
                let new_shape = self.get_reshape_target_shape()?;

                // Verify element count constraint: prod(old_shape) == prod(new_shape)
                // Use SMT to verify this constraint symbolically
                self.verify_reshape_compatibility(old_shape, &new_shape)?;

                // Create the reshaped tensor type
                Ok(TensorTypeInfo {
                    element_type: operand.element_type.clone(),
                    shape: new_shape,
                    sort: operand.sort.clone(),
                    // Preserve refinement predicates - they apply to elements, not shape
                    refinement_predicates: operand.refinement_predicates.clone(),
                })
            }

            TensorOperation::Transpose => {
                // Transpose swaps dimensions
                if operands.len() != 1 {
                    return Err(TensorRefinementError::InvalidOperandCount {
                        expected: 1,
                        actual: operands.len(),
                    });
                }

                let operand = &operands[0];

                // Reverse shape dimensions for transpose
                let transposed_shape: List<Expr> = operand.shape.iter().rev().cloned().collect();

                Ok(TensorTypeInfo {
                    element_type: operand.element_type.clone(),
                    shape: transposed_shape,
                    sort: operand.sort.clone(),
                    refinement_predicates: operand.refinement_predicates.clone(),
                })
            }
        }
    }

    /// Extract tensor sort from element type and shape
    fn extract_tensor_sort(
        &self,
        element: &Type,
        shape: &[Expr],
    ) -> Result<TensorSort, TensorRefinementError> {
        // Create a temporary translator
        let context_ref = unsafe { &*(&*self.context as *const Context) };
        let translator = Translator::new(context_ref);

        translator
            .translate_tensor_type(element, shape)
            .map_err(|e| TensorRefinementError::TranslationError(format!("{:?}", e).into()))
    }

    /// Extract refinement constraints from a tensor type
    fn extract_refinement_constraints(
        &self,
        tensor_type: &Type,
    ) -> Result<List<Expr>, TensorRefinementError> {
        match &tensor_type.kind {
            TypeKind::Refined { predicate, .. } => {
                // Extract the refinement predicate
                let constraints: List<Expr> = List::from(vec![predicate.expr.clone()]);
                Ok(constraints)
            }
            _ => {
                // No refinement constraints
                Ok(List::new())
            }
        }
    }

    /// Get verification statistics
    pub fn stats(&self) -> TensorRefinementStats {
        let shape_stats = self.shape_verifier.stats();

        TensorRefinementStats {
            shape_checks: shape_stats.total_checks,
            refinement_checks: 0, // Would come from refinement_verifier
            combined_checks: shape_stats.total_checks,
            success_rate: shape_stats.success_rate(),
        }
    }

    /// Get the target shape for a reshape operation
    ///
    /// The target shape should be stored in the operation metadata or
    /// extracted from the operation's type annotation.
    ///
    /// Note: In a full implementation, this would be called with operation
    /// metadata containing the target shape. For now, returns an error.
    fn get_reshape_target_shape(&self) -> Result<List<Expr>, TensorRefinementError> {
        // In a full implementation, this would extract the target shape from:
        // 1. Operation attributes/metadata
        // 2. Return type annotation
        // 3. Explicit shape argument
        //
        // For now, return an error indicating the shape must be provided
        Err(TensorRefinementError::ReshapeTargetShapeRequired)
    }

    /// Verify that reshape preserves total element count
    ///
    /// Checks that prod(old_shape) == prod(new_shape) using SMT solving.
    fn verify_reshape_compatibility(
        &self,
        old_shape: &List<Expr>,
        new_shape: &List<Expr>,
    ) -> Result<(), TensorRefinementError> {
        use z3::{SatResult, Solver};

        // Create Z3 solver
        let solver = Solver::new();

        // Compute product of old shape dimensions
        let old_product = self.compute_shape_product(old_shape)?;

        // Compute product of new shape dimensions
        let new_product = self.compute_shape_product(new_shape)?;

        // Assert that products must be equal using eq() method
        let equality = old_product.eq(&new_product);

        // Check if the inequality (old_product != new_product) is satisfiable
        // If SAT, then there exists an assignment where products differ (invalid reshape)
        // If UNSAT, then products are always equal (valid reshape)
        solver.assert(equality.not());

        match solver.check() {
            SatResult::Unsat => {
                // Products are always equal - reshape is valid
                Ok(())
            }
            SatResult::Sat => {
                // Found counterexample where products differ
                Err(TensorRefinementError::ReshapeIncompatible {
                    old_shape: format!("{:?}", old_shape).into(),
                    new_shape: format!("{:?}", new_shape).into(),
                })
            }
            SatResult::Unknown => {
                // Solver couldn't determine - treat as error
                Err(TensorRefinementError::ReshapeVerificationFailed {
                    reason: Text::from("SMT solver returned unknown"),
                })
            }
        }
    }

    /// Compute the product of shape dimensions as Z3 Int
    ///
    /// For now, this returns a constant value by extracting literal dimensions.
    /// Full implementation would handle symbolic dimensions.
    fn compute_shape_product(
        &self,
        shape: &List<Expr>,
    ) -> Result<z3::ast::Int, TensorRefinementError> {
        use z3::ast::Int;

        if shape.is_empty() {
            // Empty shape (scalar) has product 1
            return Ok(Int::from_i64(1));
        }

        // Compute product of constant dimensions
        let mut product: i64 = 1;
        let mut has_symbolic = false;

        for dim_expr in shape.iter() {
            match &dim_expr.kind {
                verum_ast::ExprKind::Literal(lit) => {
                    if let verum_ast::literal::LiteralKind::Int(int_lit) = &lit.kind {
                        // Extract integer value from literal
                        product = product.saturating_mul(int_lit.value as i64);
                    } else {
                        has_symbolic = true;
                    }
                }
                _ => {
                    // Non-literal dimension - need symbolic handling
                    has_symbolic = true;
                }
            }
        }

        if has_symbolic {
            // For symbolic dimensions, create a fresh symbolic variable
            // A full implementation would build the product expression symbolically
            let sym_name = format!("shape_product_{}", shape.len());
            Ok(Int::new_const(sym_name.as_str()))
        } else {
            Ok(Int::from_i64(product))
        }
    }
}

impl Default for TensorRefinementVerifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Information about a tensor type
#[derive(Debug, Clone)]
pub struct TensorTypeInfo {
    /// Element type (f32, i32, etc.)
    pub element_type: Type,
    /// Shape dimensions as expressions
    pub shape: List<Expr>,
    /// Z3 sort information
    pub sort: TensorSort,
    /// Refinement predicates on the tensor
    pub refinement_predicates: List<Expr>,
}

/// Tensor operations that can be verified
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TensorOperation {
    /// Matrix multiplication: [M, K] × [K, N] → [M, N]
    MatMul,
    /// Element-wise operations: [Shape] ⊕ [Shape] → [Shape]
    Elementwise,
    /// Broadcasting: [Shape1] + [Shape2] → [BroadcastedShape]
    Broadcast,
    /// Reshape: [M, N] → [K] (where M*N = K)
    Reshape,
    /// Transpose: [M, N] → [N, M]
    Transpose,
}

/// Tensor refinement verification errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum TensorRefinementError {
    /// Not a tensor type
    #[error("not a tensor type: {0}")]
    NotATensorType(Text),

    /// Shape verification error
    #[error("shape verification error: {0}")]
    ShapeVerificationError(#[from] ShapeError),

    /// Translation error
    #[error("translation error: {0}")]
    TranslationError(Text),

    /// Invalid operand count
    #[error("invalid operand count: expected {expected}, got {actual}")]
    InvalidOperandCount { expected: usize, actual: usize },

    /// Refinement verification error
    #[error("refinement verification error: {0}")]
    RefinementError(Text),

    /// Reshape target shape must be provided
    #[error("reshape target shape must be provided via operation metadata or type annotation")]
    ReshapeTargetShapeRequired,

    /// Reshape shapes are incompatible (different element counts)
    #[error(
        "reshape incompatible: old shape {old_shape} and new shape {new_shape} have different element counts"
    )]
    ReshapeIncompatible { old_shape: Text, new_shape: Text },

    /// Reshape verification failed
    #[error("reshape verification failed: {reason}")]
    ReshapeVerificationFailed { reason: Text },
}

/// Tensor refinement verification statistics
#[derive(Debug, Clone, Default)]
pub struct TensorRefinementStats {
    /// Number of shape constraint checks
    pub shape_checks: usize,
    /// Number of refinement predicate checks
    pub refinement_checks: usize,
    /// Combined verification checks
    pub combined_checks: usize,
    /// Success rate
    pub success_rate: f64,
}
