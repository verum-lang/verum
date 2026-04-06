//! FromTensorLiteral Protocol Implementation
//!
//! Tensor protocol: operations on Tensor<T, Shape> including element-wise ops, reductions, reshaping with compile-time shape validation — Tensor Literal Protocol
//!
//! This module implements the `FromTensorLiteral` protocol for compile-time
//! tensor literal construction with shape validation.
//!
//! # Protocol Definition
//!
//! ```verum
//! protocol FromTensorLiteral<Shape: meta [usize], T> {
//!     fn from_tensor_literal(elements: NestedArray<T, Shape>) -> Self
//!         where const_eval;  // Must be compile-time evaluable
//! }
//! ```
//!
//! # Key Features
//!
//! - **Compile-time shape validation** - Verify element count matches shape
//! - **Type-safe construction** - Ensure element types match tensor type
//! - **Zero-cost abstraction** - All validation at compile-time, no runtime overhead
//! - **Nested array support** - Handle multi-dimensional tensor literals
//!
//! # Examples
//!
//! ```verum
//! // 1D tensor (vector)
//! let vec = tensor<4>i32{1, 2, 3, 4};
//! // Calls: FromTensorLiteral<[4], i32>::from_tensor_literal([1, 2, 3, 4])
//!
//! // 2D tensor (matrix)
//! let mat = tensor<2, 2>f32{{1.0, 2.0}, {3.0, 4.0}};
//! // Calls: FromTensorLiteral<[2, 2], f32>::from_tensor_literal([[1.0, 2.0], [3.0, 4.0]])
//!
//! // Broadcasting (single element repeated)
//! let ones = tensor<8>f32{1.0};  // Expands to {1.0, 1.0, ..., 1.0}
//! ```
//!
//! # Architecture
//!
//! The protocol integrates with:
//! - **const_eval**: Compile-time shape computation and validation
//! - **type checker**: Shape type inference and element type checking
//! - **diagnostics**: Rich error messages for shape/type mismatches
//!
//! # Performance
//!
//! All operations happen at compile-time:
//! - Shape validation: 0ns runtime overhead
//! - Element count checking: 0ns runtime overhead
//! - Type checking: Standard type inference cost
//!
//! Result: Direct LLVM constant initialization (optimal codegen)

use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{List, Map, Maybe, Text};

use crate::TypeError;
use crate::const_eval::ConstEvaluator;
use verum_common::ConstValue;
use crate::protocol::{
    AssociatedConst, AssociatedType, Protocol, ProtocolBound, ProtocolMethod, TypeParam,
};
use crate::ty::Type;

/// NestedArray type representation
///
/// Represents compile-time nested arrays for tensor literal validation.
/// This is a type-level construct used during type checking.
#[derive(Debug, Clone, PartialEq)]
pub struct NestedArray {
    /// Element type
    pub element_ty: Type,
    /// Nesting depth (number of dimensions)
    pub depth: usize,
    /// Total element count (flattened)
    pub element_count: usize,
}

impl NestedArray {
    /// Create a new nested array type
    pub fn new(element_ty: Type, depth: usize, element_count: usize) -> Self {
        Self {
            element_ty,
            depth,
            element_count,
        }
    }

    /// Create from shape dimensions
    pub fn from_shape(element_ty: Type, shape: &[usize]) -> Self {
        let depth = shape.len();
        let element_count = shape.iter().product();
        Self::new(element_ty, depth, element_count)
    }
}

/// Tensor literal validator
///
/// Validates tensor literals at compile-time according to FromTensorLiteral protocol.
pub struct TensorLiteralValidator {
    /// Const evaluator for shape computation
    evaluator: ConstEvaluator,
}

impl TensorLiteralValidator {
    /// Create a new tensor literal validator
    pub fn new() -> Self {
        Self {
            evaluator: ConstEvaluator::new(),
        }
    }

    /// Validate tensor literal shape against expected dimensions
    ///
    /// Returns Ok(()) if shape matches, Err with diagnostic if mismatch.
    ///
    /// # Spec Compliance
    ///
    /// Implements compile-time validation from tensor protocol specification:
    /// ```verum
    /// meta {
    ///     let expected_size = Shape.iter().product();
    ///     let actual_size = count_elements(elements);
    ///     if actual_size != expected_size {
    ///         compile_error!("Tensor size mismatch...");
    ///     }
    /// }
    /// ```
    pub fn validate_shape(
        &mut self,
        expected_shape: &[usize],
        actual_elements: usize,
        span: Span,
    ) -> Result<(), TypeError> {
        // Compute expected element count
        let expected_size: usize = expected_shape.iter().product();

        // Check for size mismatch
        if actual_elements != expected_size {
            // Special case: broadcasting (single element)
            if actual_elements == 1 && expected_size > 1 {
                // Broadcasting is allowed - single element will be repeated
                return Ok(());
            }

            let shape_str = expected_shape
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(TypeError::Other(
                format!(
                    "Tensor size mismatch: expected {} elements for shape [{}], got {} elements\n  \
                     at: {}\n  \
                     help: ensure the number of literal elements matches the tensor shape\n  \
                     help: for shape [{}], provide exactly {} elements\n  \
                     help: or provide a single element for broadcasting",
                    expected_size, shape_str, actual_elements, span.start, shape_str, expected_size
                )
                .into(),
            ));
        }

        Ok(())
    }

    /// Validate nested array structure matches expected shape
    ///
    /// For 2D tensors, validates that the literal has correct nesting structure.
    /// Example: `{{1, 2}, {3, 4}}` for shape [2, 2]
    pub fn validate_nesting(
        &self,
        expected_shape: &[usize],
        actual_depth: usize,
        span: Span,
    ) -> Result<(), TypeError> {
        let expected_depth = expected_shape.len();

        if actual_depth != expected_depth {
            let shape_str = expected_shape
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(TypeError::Other(
                format!(
                    "Tensor nesting mismatch: expected {}D structure for shape [{}], got {}D structure\n  \
                     at: {}\n  \
                     help: for shape [{}], use {} levels of nesting\n  \
                     help: example: {}",
                    expected_depth,
                    shape_str,
                    actual_depth,
                    span.start,
                    shape_str,
                    expected_depth,
                    self.nesting_example(expected_shape)
                )
                .into(),
            ));
        }

        Ok(())
    }

    /// Generate example nesting structure for error messages
    ///
    /// # Visibility
    ///
    /// This method is `pub` to enable external testing but is not part of the stable API.
    pub fn nesting_example(&self, shape: &[usize]) -> String {
        match shape.len() {
            1 => format!("tensor<{}>T{{a, b, c, ...}}", shape[0]),
            2 => format!("tensor<{}, {}>T{{{{a, b}}, {{c, d}}}}", shape[0], shape[1]),
            3 => format!(
                "tensor<{}, {}, {}>T{{{{{{a, b}}, {{c, d}}}}, {{{{e, f}}, {{g, h}}}}}}",
                shape[0], shape[1], shape[2]
            ),
            _ => "tensor<...>T{{...}}".to_string(),
        }
    }

    /// Compute element count from ConstValue shape array
    pub fn compute_element_count(&mut self, shape: &[ConstValue]) -> Result<usize, TypeError> {
        let mut count = 1usize;

        for dim in shape {
            let dim_val = dim.as_u128().ok_or_else(|| {
                TypeError::Other(
                    format!("Shape dimension must be a positive integer, found: {}", dim).into(),
                )
            })?;

            count = count.checked_mul(dim_val as usize).ok_or_else(|| {
                TypeError::Other(
                    "Tensor shape overflow: total size exceeds usize::MAX"
                        .to_string()
                        .into(),
                )
            })?;
        }

        Ok(count)
    }

    /// Convert ConstValue shape to usize array
    pub fn shape_to_usize_array(&self, shape: &[ConstValue]) -> Result<List<usize>, TypeError> {
        let mut result = List::new();

        for dim in shape {
            let dim_val = dim.as_u128().ok_or_else(|| {
                TypeError::Other(
                    format!("Shape dimension must be a positive integer, found: {}", dim).into(),
                )
            })?;

            result.push(dim_val as usize);
        }

        Ok(result)
    }
}

impl Default for TensorLiteralValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Create the FromTensorLiteral protocol definition
///
/// This protocol is used for compile-time tensor literal construction.
///
/// # Spec Compliance
///
/// Implements protocol from tensor protocol specification:
/// ```verum
/// protocol FromTensorLiteral<Shape: meta [usize], T> {
///     fn from_tensor_literal(elements: NestedArray<T, Shape>) -> Self
///         where const_eval;
/// }
/// ```
pub fn create_from_tensor_literal_protocol() -> Protocol {
    // Type parameter: T (element type)
    let element_param = TypeParam {
        name: "T".into(),
        bounds: List::new(),
        default: Maybe::None,
    };

    // Meta type parameter: Shape: meta [usize]
    // This is represented as a regular type parameter in the protocol definition
    // but marked as meta during type checking
    let shape_param = TypeParam {
        name: "Shape".into(),
        bounds: List::new(),
        default: Maybe::None,
    };

    // Method: fn from_tensor_literal(elements: NestedArray<T, Shape>) -> Self
    let mut methods = Map::new();

    // Create NestedArray<T, Shape> type
    // In practice, this would be the actual nested array literal type
    let nested_array_ty = Type::Named {
        path: Path::single(Ident::new("NestedArray".to_string(), Span::default())),
        args: vec![
            Type::Var(crate::ty::TypeVar::with_id(0)), // T
            Type::Var(crate::ty::TypeVar::with_id(1)), // Shape (as type for now)
        ]
        .into(),
    };

    methods.insert(
        "from_tensor_literal".into(),
        ProtocolMethod {
            name: "from_tensor_literal".into(),
            ty: Type::function(
                vec![nested_array_ty].into(),
                Type::Var(crate::ty::TypeVar::with_id(2)), // Self
            ),
            has_default: false,
            doc: Maybe::Some(
                "Construct tensor from compile-time literal with shape validation".into(),
            ),
            refinement_constraints: Map::new(),
            is_async: false,
            context_requirements: List::new(),
            type_param_names: List::new(),
            type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
        },
    );

    Protocol {
        name: "FromTensorLiteral".into(),
        kind: crate::protocol::ProtocolKind::Constraint,
        type_params: vec![shape_param, element_param].into(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some("stdlib".into()),
        span: Span::default(),
    }
}

/// Register FromTensorLiteral protocol in the protocol checker
///
/// This should be called during standard protocol registration.
pub fn register_tensor_literal_protocol(checker: &mut crate::protocol::ProtocolChecker) {
    let protocol = create_from_tensor_literal_protocol();
    let _ = checker.register_protocol(protocol);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nested_array_from_shape() {
        let arr = NestedArray::from_shape(Type::Float, &[4]);
        assert_eq!(arr.depth, 1);
        assert_eq!(arr.element_count, 4);

        let arr2d = NestedArray::from_shape(Type::Float, &[2, 3]);
        assert_eq!(arr2d.depth, 2);
        assert_eq!(arr2d.element_count, 6);

        let arr3d = NestedArray::from_shape(Type::Float, &[2, 3, 4]);
        assert_eq!(arr3d.depth, 3);
        assert_eq!(arr3d.element_count, 24);
    }

    #[test]
    fn test_validate_shape_exact_match() {
        let mut validator = TensorLiteralValidator::new();

        // 1D: [4] with 4 elements
        assert!(validator.validate_shape(&[4], 4, Span::default()).is_ok());

        // 2D: [2, 3] with 6 elements
        assert!(
            validator
                .validate_shape(&[2, 3], 6, Span::default())
                .is_ok()
        );

        // 3D: [2, 3, 4] with 24 elements
        assert!(
            validator
                .validate_shape(&[2, 3, 4], 24, Span::default())
                .is_ok()
        );
    }

    #[test]
    fn test_validate_shape_mismatch() {
        let mut validator = TensorLiteralValidator::new();

        // Too few elements
        let result = validator.validate_shape(&[4], 3, Span::default());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("size mismatch"));

        // Too many elements
        let result = validator.validate_shape(&[2, 3], 7, Span::default());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("size mismatch"));
    }

    #[test]
    fn test_validate_shape_broadcasting() {
        let mut validator = TensorLiteralValidator::new();

        // Broadcasting: single element for any shape
        assert!(validator.validate_shape(&[4], 1, Span::default()).is_ok());
        assert!(
            validator
                .validate_shape(&[2, 3], 1, Span::default())
                .is_ok()
        );
        assert!(
            validator
                .validate_shape(&[2, 3, 4], 1, Span::default())
                .is_ok()
        );
    }

    #[test]
    fn test_validate_nesting() {
        let validator = TensorLiteralValidator::new();

        // Correct nesting
        assert!(validator.validate_nesting(&[4], 1, Span::default()).is_ok());
        assert!(
            validator
                .validate_nesting(&[2, 3], 2, Span::default())
                .is_ok()
        );
        assert!(
            validator
                .validate_nesting(&[2, 3, 4], 3, Span::default())
                .is_ok()
        );
    }

    #[test]
    fn test_validate_nesting_mismatch() {
        let validator = TensorLiteralValidator::new();

        // Wrong nesting depth
        let result = validator.validate_nesting(&[2, 3], 1, Span::default());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("nesting mismatch"));

        let result = validator.validate_nesting(&[4], 2, Span::default());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("nesting mismatch"));
    }

    #[test]
    fn test_compute_element_count() {
        let mut validator = TensorLiteralValidator::new();

        // 1D
        let shape = vec![ConstValue::UInt(10)];
        assert_eq!(validator.compute_element_count(&shape).unwrap(), 10);

        // 2D
        let shape2d = vec![ConstValue::UInt(2), ConstValue::UInt(3)];
        assert_eq!(validator.compute_element_count(&shape2d).unwrap(), 6);

        // 3D
        let shape3d = vec![
            ConstValue::UInt(2),
            ConstValue::UInt(3),
            ConstValue::UInt(4),
        ];
        assert_eq!(validator.compute_element_count(&shape3d).unwrap(), 24);
    }

    #[test]
    fn test_shape_to_usize_array() {
        let validator = TensorLiteralValidator::new();

        let shape = vec![ConstValue::UInt(2), ConstValue::UInt(3)];
        let result = validator.shape_to_usize_array(&shape).unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0], 2);
        assert_eq!(result[1], 3);
    }

    #[test]
    fn test_protocol_creation() {
        let protocol = create_from_tensor_literal_protocol();

        assert_eq!(protocol.name.as_str(), "FromTensorLiteral");
        assert_eq!(protocol.type_params.len(), 2);
        assert_eq!(protocol.type_params[0].name.as_str(), "Shape");
        assert_eq!(protocol.type_params[1].name.as_str(), "T");
        assert!(protocol.methods.contains_key(&"from_tensor_literal".into()));
    }

    #[test]
    fn test_nesting_example_generation() {
        let validator = TensorLiteralValidator::new();

        let ex1d = validator.nesting_example(&[4]);
        assert!(ex1d.contains("tensor<4>T{"));

        let ex2d = validator.nesting_example(&[2, 2]);
        assert!(ex2d.contains("tensor<2, 2>T{"));

        let ex3d = validator.nesting_example(&[2, 2, 2]);
        assert!(ex3d.contains("tensor<2, 2, 2>T{"));
    }
}
