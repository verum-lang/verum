//! Tensor Shape Checker
//!
//! SIMD and tensor system: unified Tensor<T, Shape> type with compile-time shape validation, SIMD acceleration (SSE/AVX/NEON), auto-differentiation — Tensor Type System
//!
//! This module provides comprehensive tensor shape validation including:
//! - Shape inference and validation for binary/ternary operations
//! - Broadcasting compatibility checking (NumPy-style)
//! - Matrix multiplication shape verification
//! - Reduce operation shape computation
//! - Reshape validation
//! - Transpose shape computation
//!
//! # Performance
//!
//! All operations are compile-time with 0ns runtime overhead.

use verum_ast::span::Span;
use verum_common::well_known_types::WellKnownType as WKT;
use verum_common::{ConstValue, List};
use crate::ty::Type;

/// Error type for tensor shape checking
#[derive(Debug, Clone)]
pub struct TensorShapeError {
    /// Error message
    pub message: String,
    /// Span where error occurred
    pub span: Span,
}

impl TensorShapeError {
    /// Create a new error with message
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span: Span::default(),
        }
    }

    /// Create error with span
    pub fn with_span(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

impl std::fmt::Display for TensorShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TensorShapeError {}

/// Check if type is a scalar (0-dimensional)
fn is_scalar_type(ty: &Type) -> bool {
    matches!(ty, Type::Int | Type::Float | Type::Bool | Type::Char)
}

/// Format a type name for error messages
fn format_type_name(ty: &Type) -> String {
    match ty {
        Type::Int => WKT::Int.as_str().to_string(),
        Type::Float => WKT::Float.as_str().to_string(),
        Type::Bool => WKT::Bool.as_str().to_string(),
        Type::Char => WKT::Char.as_str().to_string(),
        Type::Text => WKT::Text.as_str().to_string(),
        Type::Unit => "Unit".to_string(),
        Type::Tensor { element, shape, .. } => {
            let elem_str = format_type_name(element);
            let shape_str: Vec<String> = shape
                .iter()
                .map(|v| match v {
                    crate::ConstValue::UInt(n) => n.to_string(),
                    crate::ConstValue::Int(n) => n.to_string(),
                    _ => "?".to_string(),
                })
                .collect();
            format!("Tensor<{}, [{}]>", elem_str, shape_str.join(", "))
        }
        Type::Named { path, .. } => path.to_string(),
        _ => "unknown type".to_string(),
    }
}

/// Tensor shape checker for type validation
///
/// Provides compile-time shape validation for tensor operations following
/// NumPy broadcasting rules and standard linear algebra conventions.
pub struct TensorShapeChecker {
    /// Whether to enable strict mode (no implicit broadcasting)
    strict_mode: bool,
}

impl Default for TensorShapeChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl TensorShapeChecker {
    /// Create a new tensor shape checker
    pub fn new() -> Self {
        Self { strict_mode: false }
    }

    /// Create a shape checker with strict mode (no broadcasting)
    pub fn strict() -> Self {
        Self { strict_mode: true }
    }

    /// Extract shape dimensions from a tensor type
    fn extract_shape(&self, ty: &Type) -> Result<Vec<usize>, TensorShapeError> {
        match ty {
            Type::Tensor { shape, .. } => {
                let mut dims = Vec::new();
                for dim in shape.iter() {
                    match dim {
                        ConstValue::UInt(n) => dims.push(*n as usize),
                        ConstValue::Int(n) if *n >= 0 => dims.push(*n as usize),
                        _ => {
                            return Err(TensorShapeError::new(
                                "Invalid tensor dimension: must be a non-negative integer",
                            ));
                        }
                    }
                }
                Ok(dims)
            }
            // Scalars are 0-dimensional tensors
            ty if is_scalar_type(ty) => Ok(vec![]),
            _ => Err(TensorShapeError::new(format!(
                "Expected tensor type, got {}",
                format_type_name(ty)
            ))),
        }
    }

    /// Extract element type from a tensor type
    fn extract_element_type(&self, ty: &Type) -> Result<Type, TensorShapeError> {
        match ty {
            Type::Tensor { element, .. } => Ok((**element).clone()),
            // Scalar types are their own element type
            ty if is_scalar_type(ty) => Ok(ty.clone()),
            _ => Err(TensorShapeError::new(format!(
                "Expected tensor type, got {}",
                format_type_name(ty)
            ))),
        }
    }

    /// Create a tensor type from shape and element type
    fn create_tensor_type(&self, element: Type, shape: Vec<usize>, span: Span) -> Type {
        if shape.is_empty() {
            // 0-dimensional -> return scalar element type directly
            element
        } else {
            let shape_const: List<ConstValue> =
                shape.iter().map(|&s| ConstValue::UInt(s as u128)).collect();

            // Compute strides for row-major layout
            let mut strides = vec![1usize; shape.len()];
            for i in (0..shape.len().saturating_sub(1)).rev() {
                strides[i] = strides[i + 1] * shape[i + 1];
            }

            Type::Tensor {
                element: Box::new(element),
                shape: shape_const,
                strides: strides.into_iter().collect(),
                span,
            }
        }
    }

    /// Check if two shapes are broadcast-compatible (NumPy rules)
    ///
    /// Broadcasting rules:
    /// 1. If shapes have different lengths, prepend 1s to the shorter shape
    /// 2. Dimensions are compatible if they are equal OR one of them is 1
    /// 3. Result shape takes the maximum of each dimension
    pub fn check_broadcast_compatible(&self, t1: &Type, t2: &Type) -> Result<(), TensorShapeError> {
        let shape1 = self.extract_shape(t1)?;
        let shape2 = self.extract_shape(t2)?;

        if self.strict_mode && shape1 != shape2 {
            return Err(TensorShapeError::new(format!(
                "Strict mode: shapes must match exactly, got [{}] and [{}]",
                shape1
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                shape2
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }

        // Align shapes from the right
        let max_len = shape1.len().max(shape2.len());
        let padded1: Vec<usize> = std::iter::repeat_n(1, max_len - shape1.len())
            .chain(shape1.iter().copied())
            .collect();
        let padded2: Vec<usize> = std::iter::repeat_n(1, max_len - shape2.len())
            .chain(shape2.iter().copied())
            .collect();

        // Check compatibility
        for (i, (d1, d2)) in padded1.iter().zip(padded2.iter()).enumerate() {
            if *d1 != *d2 && *d1 != 1 && *d2 != 1 {
                return Err(TensorShapeError::new(format!(
                    "Shapes [{}] and [{}] are not broadcast-compatible at dimension {}",
                    shape1
                        .iter()
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    shape2
                        .iter()
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    i
                )));
            }
        }

        Ok(())
    }

    /// Compute broadcast result shape
    fn compute_broadcast_shape(
        &self,
        shape1: &[usize],
        shape2: &[usize],
    ) -> Result<Vec<usize>, TensorShapeError> {
        let max_len = shape1.len().max(shape2.len());
        let padded1: Vec<usize> = std::iter::repeat_n(1, max_len - shape1.len())
            .chain(shape1.iter().copied())
            .collect();
        let padded2: Vec<usize> = std::iter::repeat_n(1, max_len - shape2.len())
            .chain(shape2.iter().copied())
            .collect();

        let mut result = Vec::with_capacity(max_len);
        for (d1, d2) in padded1.iter().zip(padded2.iter()) {
            if *d1 == *d2 {
                result.push(*d1);
            } else if *d1 == 1 {
                result.push(*d2);
            } else if *d2 == 1 {
                result.push(*d1);
            } else {
                return Err(TensorShapeError::new(format!(
                    "Cannot broadcast shapes [{}] and [{}]",
                    shape1
                        .iter()
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    shape2
                        .iter()
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }

        Ok(result)
    }

    /// Check binary operation shape and return result type
    ///
    /// Supports element-wise operations: add, sub, mul, div, pow, etc.
    /// Comparison operations (eq, ne, lt, le, gt, ge) return Bool element type.
    pub fn check_binary_op_shape(
        &self,
        t1: &Type,
        t2: &Type,
        op: &str,
    ) -> Result<Type, TensorShapeError> {
        let shape1 = self.extract_shape(t1)?;
        let shape2 = self.extract_shape(t2)?;
        let elem1 = self.extract_element_type(t1)?;
        let elem2 = self.extract_element_type(t2)?;

        // Check element type compatibility
        if elem1 != elem2 {
            return Err(TensorShapeError::new(format!(
                "Binary op '{}' requires matching element types, got {} and {}",
                op,
                format_type_name(&elem1),
                format_type_name(&elem2)
            )));
        }

        // Check broadcast compatibility and compute result shape
        self.check_broadcast_compatible(t1, t2)?;
        let result_shape = self.compute_broadcast_shape(&shape1, &shape2)?;

        // Comparison operations return Bool, arithmetic ops preserve element type
        let result_elem = match op {
            "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "==" | "!=" | "<" | "<=" | ">" | ">=" => {
                Type::Bool
            }
            _ => elem1,
        };

        Ok(self.create_tensor_type(result_elem, result_shape, Span::default()))
    }

    /// Check matrix multiplication shape
    ///
    /// For matrices A[m, k] @ B[k, n] -> C[m, n]
    /// Also supports batched matmul with broadcasting on batch dimensions
    pub fn check_matmul_shape(&self, a: &Type, b: &Type) -> Result<Vec<usize>, TensorShapeError> {
        let shape_a = self.extract_shape(a)?;
        let shape_b = self.extract_shape(b)?;

        if shape_a.len() < 2 || shape_b.len() < 2 {
            return Err(TensorShapeError::new(format!(
                "Matrix multiplication requires at least 2D tensors, got shapes [{}] and [{}]",
                shape_a
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                shape_b
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }

        // Get matrix dimensions (last 2 dimensions)
        let m = shape_a[shape_a.len() - 2];
        let k1 = shape_a[shape_a.len() - 1];
        let k2 = shape_b[shape_b.len() - 2];
        let n = shape_b[shape_b.len() - 1];

        // Check inner dimensions match
        if k1 != k2 {
            return Err(TensorShapeError::new(format!(
                "Matrix multiplication inner dimensions must match: {}x{} @ {}x{}",
                m, k1, k2, n
            )));
        }

        // Handle batch dimensions with broadcasting
        let batch_a = &shape_a[..shape_a.len() - 2];
        let batch_b = &shape_b[..shape_b.len() - 2];
        let batch_result = self.compute_broadcast_shape(batch_a, batch_b)?;

        // Result shape: batch_dims + [m, n]
        let mut result = batch_result;
        result.push(m);
        result.push(n);

        Ok(result)
    }

    /// Check reduce operation and return result type
    ///
    /// Supports: sum, prod, mean, max, min, any, all, argmax, argmin
    pub fn check_reduce_op(
        &self,
        tensor: &Type,
        op: &str,
        axis: Option<usize>,
    ) -> Result<Type, TensorShapeError> {
        let shape = self.extract_shape(tensor)?;
        let elem = self.extract_element_type(tensor)?;

        let result_shape = match axis {
            Some(ax) => {
                if ax >= shape.len() {
                    return Err(TensorShapeError::new(format!(
                        "Axis {} out of bounds for tensor with {} dimensions",
                        ax,
                        shape.len()
                    )));
                }
                // Remove the reduced axis
                let mut new_shape = shape.clone();
                new_shape.remove(ax);
                new_shape
            }
            None => {
                // Reduce over all axes -> scalar
                vec![]
            }
        };

        // argmax/argmin return index type (Int)
        let result_elem = if op == "argmax" || op == "argmin" {
            Type::Int
        } else {
            elem
        };

        Ok(self.create_tensor_type(result_elem, result_shape, Span::default()))
    }

    /// Check reshape operation
    ///
    /// The new shape must have the same total element count as the original
    pub fn check_reshape(
        &self,
        tensor: &Type,
        new_shape: &[usize],
    ) -> Result<Type, TensorShapeError> {
        let old_shape = self.extract_shape(tensor)?;
        let elem = self.extract_element_type(tensor)?;

        let old_count: usize = old_shape.iter().product();
        let new_count: usize = new_shape.iter().product();

        if old_count != new_count {
            return Err(TensorShapeError::new(format!(
                "Cannot reshape tensor with {} elements to shape [{}] ({} elements)",
                old_count,
                new_shape
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                new_count
            )));
        }

        Ok(self.create_tensor_type(elem, new_shape.to_vec(), Span::default()))
    }

    /// Check transpose operation
    ///
    /// For 2D: swaps dimensions
    /// For nD: reverses all dimensions
    pub fn check_transpose(&self, tensor: &Type) -> Result<Type, TensorShapeError> {
        let shape = self.extract_shape(tensor)?;
        let elem = self.extract_element_type(tensor)?;

        let new_shape: Vec<usize> = shape.into_iter().rev().collect();

        Ok(self.create_tensor_type(elem, new_shape, Span::default()))
    }

    /// Check transpose with explicit permutation
    pub fn check_transpose_perm(
        &self,
        tensor: &Type,
        perm: &[usize],
    ) -> Result<Type, TensorShapeError> {
        let shape = self.extract_shape(tensor)?;
        let elem = self.extract_element_type(tensor)?;

        if perm.len() != shape.len() {
            return Err(TensorShapeError::new(format!(
                "Permutation length {} doesn't match tensor dimensions {}",
                perm.len(),
                shape.len()
            )));
        }

        // Validate permutation is valid
        let mut seen = vec![false; shape.len()];
        for &p in perm {
            if p >= shape.len() {
                return Err(TensorShapeError::new(format!(
                    "Permutation index {} out of bounds",
                    p
                )));
            }
            if seen[p] {
                return Err(TensorShapeError::new(format!(
                    "Duplicate index {} in permutation",
                    p
                )));
            }
            seen[p] = true;
        }

        let new_shape: Vec<usize> = perm.iter().map(|&p| shape[p]).collect();

        Ok(self.create_tensor_type(elem, new_shape, Span::default()))
    }

    /// Check ternary operation shape (e.g., fused multiply-add)
    pub fn check_ternary_op_shape(
        &self,
        a: &Type,
        b: &Type,
        c: &Type,
        op: &str,
    ) -> Result<Type, TensorShapeError> {
        let shape_a = self.extract_shape(a)?;
        let shape_b = self.extract_shape(b)?;
        let shape_c = self.extract_shape(c)?;
        let elem_a = self.extract_element_type(a)?;

        // Check all element types match
        let elem_b = self.extract_element_type(b)?;
        let elem_c = self.extract_element_type(c)?;
        if elem_a != elem_b || elem_a != elem_c {
            return Err(TensorShapeError::new(format!(
                "Ternary op '{}' requires matching element types",
                op
            )));
        }

        // Broadcast all three shapes together
        let ab_shape = self.compute_broadcast_shape(&shape_a, &shape_b)?;
        let result_shape = self.compute_broadcast_shape(&ab_shape, &shape_c)?;

        Ok(self.create_tensor_type(elem_a, result_shape, Span::default()))
    }

    /// Check select/where operation: select(mask, a, b)
    ///
    /// mask must be boolean tensor, a and b must have matching element types
    pub fn check_select_op(
        &self,
        mask: &Type,
        a: &Type,
        b: &Type,
    ) -> Result<Type, TensorShapeError> {
        let mask_shape = self.extract_shape(mask)?;
        let shape_a = self.extract_shape(a)?;
        let shape_b = self.extract_shape(b)?;
        let elem_a = self.extract_element_type(a)?;
        let elem_b = self.extract_element_type(b)?;

        // Check mask is boolean
        let mask_elem = self.extract_element_type(mask)?;
        if mask_elem != Type::Bool {
            return Err(TensorShapeError::new("Select mask must be boolean tensor"));
        }

        // Check a and b have matching element types
        if elem_a != elem_b {
            return Err(TensorShapeError::new(format!(
                "Select requires matching element types for true/false branches, got {} and {}",
                format_type_name(&elem_a),
                format_type_name(&elem_b)
            )));
        }

        // Broadcast all three shapes
        let ab_shape = self.compute_broadcast_shape(&shape_a, &shape_b)?;
        let result_shape = self.compute_broadcast_shape(&mask_shape, &ab_shape)?;

        Ok(self.create_tensor_type(elem_a, result_shape, Span::default()))
    }

    /// Check concatenation operation
    pub fn check_concat(&self, tensors: &[Type], axis: usize) -> Result<Type, TensorShapeError> {
        if tensors.is_empty() {
            return Err(TensorShapeError::new("Cannot concatenate empty list"));
        }

        let first_shape = self.extract_shape(&tensors[0])?;
        let elem = self.extract_element_type(&tensors[0])?;

        if axis >= first_shape.len() {
            return Err(TensorShapeError::new(format!(
                "Axis {} out of bounds for {}-dimensional tensor",
                axis,
                first_shape.len()
            )));
        }

        let mut concat_dim = first_shape[axis];

        for (i, t) in tensors.iter().skip(1).enumerate() {
            let shape = self.extract_shape(t)?;
            let t_elem = self.extract_element_type(t)?;

            if t_elem != elem {
                return Err(TensorShapeError::new(format!(
                    "All tensors must have same element type, tensor {} differs",
                    i + 1
                )));
            }

            if shape.len() != first_shape.len() {
                return Err(TensorShapeError::new(format!(
                    "All tensors must have same number of dimensions, tensor {} has {} vs {}",
                    i + 1,
                    shape.len(),
                    first_shape.len()
                )));
            }

            for (j, (&d1, &d2)) in first_shape.iter().zip(shape.iter()).enumerate() {
                if j != axis && d1 != d2 {
                    return Err(TensorShapeError::new(format!(
                        "Dimensions must match except on concat axis, dimension {} differs: {} vs {}",
                        j, d1, d2
                    )));
                }
            }

            concat_dim += shape[axis];
        }

        let mut result_shape = first_shape;
        result_shape[axis] = concat_dim;

        Ok(self.create_tensor_type(elem, result_shape, Span::default()))
    }

    /// Check slice operation
    pub fn check_slice(
        &self,
        tensor: &Type,
        starts: &[usize],
        ends: &[usize],
        steps: &[usize],
    ) -> Result<Type, TensorShapeError> {
        let shape = self.extract_shape(tensor)?;
        let elem = self.extract_element_type(tensor)?;

        if starts.len() != shape.len() || ends.len() != shape.len() || steps.len() != shape.len() {
            return Err(TensorShapeError::new(
                "Slice parameters must match tensor dimensions",
            ));
        }

        let mut result_shape = Vec::new();
        for i in 0..shape.len() {
            if starts[i] > ends[i] {
                return Err(TensorShapeError::new(format!(
                    "Slice start {} > end {} at dimension {}",
                    starts[i], ends[i], i
                )));
            }
            if ends[i] > shape[i] {
                return Err(TensorShapeError::new(format!(
                    "Slice end {} > dimension size {} at dimension {}",
                    ends[i], shape[i], i
                )));
            }
            if steps[i] == 0 {
                return Err(TensorShapeError::new(format!(
                    "Slice step cannot be 0 at dimension {}",
                    i
                )));
            }

            let size = (ends[i] - starts[i]).div_ceil(steps[i]);
            result_shape.push(size);
        }

        Ok(self.create_tensor_type(elem, result_shape, Span::default()))
    }

    /// Check squeeze operation (remove dimensions of size 1)
    pub fn check_squeeze(
        &self,
        tensor: &Type,
        axes: Option<&[usize]>,
    ) -> Result<Type, TensorShapeError> {
        let shape = self.extract_shape(tensor)?;
        let elem = self.extract_element_type(tensor)?;

        let result_shape: Vec<usize> = match axes {
            Some(squeeze_axes) => {
                for &ax in squeeze_axes {
                    if ax >= shape.len() {
                        return Err(TensorShapeError::new(format!(
                            "Squeeze axis {} out of bounds",
                            ax
                        )));
                    }
                    if shape[ax] != 1 {
                        return Err(TensorShapeError::new(format!(
                            "Cannot squeeze axis {} with size {} (must be 1)",
                            ax, shape[ax]
                        )));
                    }
                }
                shape
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !squeeze_axes.contains(i))
                    .map(|(_, &d)| d)
                    .collect()
            }
            None => shape.into_iter().filter(|&d| d != 1).collect(),
        };

        Ok(self.create_tensor_type(elem, result_shape, Span::default()))
    }

    /// Check unsqueeze operation (add dimension of size 1)
    pub fn check_unsqueeze(&self, tensor: &Type, axis: usize) -> Result<Type, TensorShapeError> {
        let shape = self.extract_shape(tensor)?;
        let elem = self.extract_element_type(tensor)?;

        if axis > shape.len() {
            return Err(TensorShapeError::new(format!(
                "Unsqueeze axis {} out of bounds for {}-dimensional tensor",
                axis,
                shape.len()
            )));
        }

        let mut result_shape = shape;
        result_shape.insert(axis, 1);

        Ok(self.create_tensor_type(elem, result_shape, Span::default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tensor(elem: Type, shape: Vec<usize>) -> Type {
        let shape_const: List<ConstValue> =
            shape.iter().map(|&s| ConstValue::UInt(s as u128)).collect();
        let strides: List<usize> = {
            let mut strides = vec![1; shape.len()];
            for i in (0..shape.len().saturating_sub(1)).rev() {
                strides[i] = strides[i + 1] * shape[i + 1];
            }
            strides.into_iter().collect()
        };
        Type::Tensor {
            element: Box::new(elem),
            shape: shape_const,
            strides,
            span: Span::default(),
        }
    }

    #[test]
    fn test_broadcast_compatible() {
        let checker = TensorShapeChecker::new();

        // Same shapes
        let t1 = make_tensor(Type::Float, vec![3, 4]);
        let t2 = make_tensor(Type::Float, vec![3, 4]);
        assert!(checker.check_broadcast_compatible(&t1, &t2).is_ok());

        // Broadcasting with 1
        let t1 = make_tensor(Type::Float, vec![3, 4]);
        let t2 = make_tensor(Type::Float, vec![1, 4]);
        assert!(checker.check_broadcast_compatible(&t1, &t2).is_ok());

        // Broadcasting with different ranks
        let t1 = make_tensor(Type::Float, vec![3, 4]);
        let t2 = make_tensor(Type::Float, vec![4]);
        assert!(checker.check_broadcast_compatible(&t1, &t2).is_ok());

        // Incompatible shapes
        let t1 = make_tensor(Type::Float, vec![3, 4]);
        let t2 = make_tensor(Type::Float, vec![3, 5]);
        assert!(checker.check_broadcast_compatible(&t1, &t2).is_err());
    }

    #[test]
    fn test_matmul_shape() {
        let checker = TensorShapeChecker::new();

        // Basic 2D matmul
        let a = make_tensor(Type::Float, vec![3, 4]);
        let b = make_tensor(Type::Float, vec![4, 5]);
        let result = checker.check_matmul_shape(&a, &b).unwrap();
        assert_eq!(result, vec![3, 5]);

        // Batched matmul
        let a = make_tensor(Type::Float, vec![2, 3, 4]);
        let b = make_tensor(Type::Float, vec![2, 4, 5]);
        let result = checker.check_matmul_shape(&a, &b).unwrap();
        assert_eq!(result, vec![2, 3, 5]);

        // Inner dimension mismatch
        let a = make_tensor(Type::Float, vec![3, 4]);
        let b = make_tensor(Type::Float, vec![5, 6]);
        assert!(checker.check_matmul_shape(&a, &b).is_err());
    }

    #[test]
    fn test_reshape() {
        let checker = TensorShapeChecker::new();

        let t = make_tensor(Type::Float, vec![2, 3, 4]);
        let result = checker.check_reshape(&t, &[6, 4]).unwrap();

        match result {
            Type::Tensor { shape, .. } => {
                let dims: Vec<usize> = shape
                    .iter()
                    .map(|c| match c {
                        ConstValue::UInt(n) => *n as usize,
                        _ => panic!("unexpected"),
                    })
                    .collect();
                assert_eq!(dims, vec![6, 4]);
            }
            _ => panic!("Expected tensor type"),
        }

        // Invalid reshape (different element count)
        assert!(checker.check_reshape(&t, &[5, 5]).is_err());
    }

    #[test]
    fn test_transpose() {
        let checker = TensorShapeChecker::new();

        let t = make_tensor(Type::Float, vec![2, 3, 4]);
        let result = checker.check_transpose(&t).unwrap();

        match result {
            Type::Tensor { shape, .. } => {
                let dims: Vec<usize> = shape
                    .iter()
                    .map(|c| match c {
                        ConstValue::UInt(n) => *n as usize,
                        _ => panic!("unexpected"),
                    })
                    .collect();
                assert_eq!(dims, vec![4, 3, 2]);
            }
            _ => panic!("Expected tensor type"),
        }
    }

    #[test]
    fn test_reduce() {
        let checker = TensorShapeChecker::new();

        let t = make_tensor(Type::Float, vec![2, 3, 4]);

        // Reduce along axis 1
        let result = checker.check_reduce_op(&t, "sum", Some(1)).unwrap();
        match result {
            Type::Tensor { shape, .. } => {
                let dims: Vec<usize> = shape
                    .iter()
                    .map(|c| match c {
                        ConstValue::UInt(n) => *n as usize,
                        _ => panic!("unexpected"),
                    })
                    .collect();
                assert_eq!(dims, vec![2, 4]);
            }
            _ => panic!("Expected tensor type"),
        }

        // Reduce all -> scalar
        let result = checker.check_reduce_op(&t, "sum", None).unwrap();
        assert!(matches!(result, Type::Float));
    }

    #[test]
    fn test_scalar_operations() {
        let checker = TensorShapeChecker::new();

        // Scalar + Tensor broadcasting
        let scalar = Type::Float;
        let tensor = make_tensor(Type::Float, vec![3, 4]);

        let result = checker
            .check_binary_op_shape(&scalar, &tensor, "add")
            .unwrap();
        match result {
            Type::Tensor { shape, .. } => {
                let dims: Vec<usize> = shape
                    .iter()
                    .map(|c| match c {
                        ConstValue::UInt(n) => *n as usize,
                        _ => panic!("unexpected"),
                    })
                    .collect();
                assert_eq!(dims, vec![3, 4]);
            }
            _ => panic!("Expected tensor type"),
        }
    }

    #[test]
    fn test_squeeze_unsqueeze() {
        let checker = TensorShapeChecker::new();

        // Unsqueeze
        let t = make_tensor(Type::Float, vec![3, 4]);
        let result = checker.check_unsqueeze(&t, 0).unwrap();
        match result {
            Type::Tensor { shape, .. } => {
                let dims: Vec<usize> = shape
                    .iter()
                    .map(|c| match c {
                        ConstValue::UInt(n) => *n as usize,
                        _ => panic!("unexpected"),
                    })
                    .collect();
                assert_eq!(dims, vec![1, 3, 4]);
            }
            _ => panic!("Expected tensor type"),
        }

        // Squeeze
        let t_with_ones = make_tensor(Type::Float, vec![1, 3, 1, 4]);
        let result = checker.check_squeeze(&t_with_ones, None).unwrap();
        match result {
            Type::Tensor { shape, .. } => {
                let dims: Vec<usize> = shape
                    .iter()
                    .map(|c| match c {
                        ConstValue::UInt(n) => *n as usize,
                        _ => panic!("unexpected"),
                    })
                    .collect();
                assert_eq!(dims, vec![3, 4]);
            }
            _ => panic!("Expected tensor type"),
        }
    }

    #[test]
    fn test_concat() {
        let checker = TensorShapeChecker::new();

        let t1 = make_tensor(Type::Float, vec![2, 4]);
        let t2 = make_tensor(Type::Float, vec![3, 4]);
        let t3 = make_tensor(Type::Float, vec![5, 4]);

        let result = checker.check_concat(&[t1, t2, t3], 0).unwrap();
        match result {
            Type::Tensor { shape, .. } => {
                let dims: Vec<usize> = shape
                    .iter()
                    .map(|c| match c {
                        ConstValue::UInt(n) => *n as usize,
                        _ => panic!("unexpected"),
                    })
                    .collect();
                assert_eq!(dims, vec![10, 4]); // 2+3+5 = 10
            }
            _ => panic!("Expected tensor type"),
        }
    }

    #[test]
    fn test_slice() {
        let checker = TensorShapeChecker::new();

        let t = make_tensor(Type::Float, vec![10, 20, 30]);

        let result = checker
            .check_slice(&t, &[0, 5, 10], &[5, 15, 25], &[1, 2, 3])
            .unwrap();

        match result {
            Type::Tensor { shape, .. } => {
                let dims: Vec<usize> = shape
                    .iter()
                    .map(|c| match c {
                        ConstValue::UInt(n) => *n as usize,
                        _ => panic!("unexpected"),
                    })
                    .collect();
                // (5-0+1-1)/1 = 5, (15-5+2-1)/2 = 5, (25-10+3-1)/3 = 6
                assert_eq!(dims, vec![5, 5, 5]);
            }
            _ => panic!("Expected tensor type"),
        }
    }

    #[test]
    fn test_transpose_perm() {
        let checker = TensorShapeChecker::new();

        let t = make_tensor(Type::Float, vec![2, 3, 4]);
        let result = checker.check_transpose_perm(&t, &[1, 2, 0]).unwrap();

        match result {
            Type::Tensor { shape, .. } => {
                let dims: Vec<usize> = shape
                    .iter()
                    .map(|c| match c {
                        ConstValue::UInt(n) => *n as usize,
                        _ => panic!("unexpected"),
                    })
                    .collect();
                assert_eq!(dims, vec![3, 4, 2]);
            }
            _ => panic!("Expected tensor type"),
        }
    }
}
