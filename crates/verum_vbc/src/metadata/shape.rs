//! Shape metadata for compile-time tensor verification.
//!
//! Provides static shape tracking with symbolic dimension support
//! for compile-time shape verification of tensor operations.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Re-export DType from the crate-level dtype module for consistency.
// This unifies the previously duplicated DType definitions between
// metadata/shape.rs (compile-time) and interpreter/tensor.rs (runtime).
pub use crate::dtype::DType;

/// Instruction identifier within a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InstructionId(pub u32);

/// Symbolic dimension identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolId(pub u32);

/// Dimension type for tensor shapes.
///
/// Supports static, symbolic, and dynamic dimensions.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShapeDim {
    /// Statically known dimension (e.g., 1024).
    Static(usize),
    /// Symbolic dimension resolved at specialization time (e.g., batch_size).
    Symbolic(SymbolId),
    /// Fully dynamic dimension (fallback, resolved at runtime).
    Dynamic,
}

impl ShapeDim {
    /// Returns the static value if known.
    pub fn static_value(&self) -> Option<usize> {
        match self {
            ShapeDim::Static(v) => Some(*v),
            _ => None,
        }
    }

    /// Returns true if dimension is statically known.
    pub fn is_static(&self) -> bool {
        matches!(self, ShapeDim::Static(_))
    }

    /// Returns true if dimension is symbolic.
    pub fn is_symbolic(&self) -> bool {
        matches!(self, ShapeDim::Symbolic(_))
    }
}

/// Static shape with dimensions and dtype.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticShape {
    /// Dimensions (may be static, symbolic, or dynamic).
    pub dims: Vec<ShapeDim>,
    /// Data type.
    pub dtype: DType,
}

impl StaticShape {
    /// Creates a new static shape.
    pub fn new(dims: Vec<ShapeDim>, dtype: DType) -> Self {
        Self { dims, dtype }
    }

    /// Creates a static shape with all static dimensions.
    pub fn from_static(dims: &[usize], dtype: DType) -> Self {
        Self {
            dims: dims.iter().map(|&d| ShapeDim::Static(d)).collect(),
            dtype,
        }
    }

    /// Returns the number of dimensions.
    pub fn ndim(&self) -> usize {
        self.dims.len()
    }

    /// Returns the number of elements if all dimensions are static.
    pub fn numel(&self) -> Option<usize> {
        self.dims
            .iter()
            .map(|d| d.static_value())
            .try_fold(1, |acc, d| d.map(|v| acc * v))
    }

    /// Returns true if all dimensions are statically known.
    pub fn is_fully_static(&self) -> bool {
        self.dims.iter().all(|d| d.is_static())
    }

    /// Checks if this shape is compatible with another for broadcasting.
    pub fn broadcast_compatible(&self, other: &StaticShape) -> bool {
        let self_dims = self.dims.iter().rev();
        let other_dims = other.dims.iter().rev();

        for (a, b) in self_dims.zip(other_dims) {
            match (a, b) {
                (ShapeDim::Static(1), _) | (_, ShapeDim::Static(1)) => continue,
                (ShapeDim::Static(x), ShapeDim::Static(y)) if x == y => continue,
                (ShapeDim::Symbolic(x), ShapeDim::Symbolic(y)) if x == y => continue,
                (ShapeDim::Dynamic, _) | (_, ShapeDim::Dynamic) => continue,
                _ => return false,
            }
        }
        true
    }
}

/// Symbolic dimension definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolDef {
    /// Symbol name (e.g., "batch_size", "seq_len").
    pub name: String,
    /// Optional lower bound.
    pub min: Option<usize>,
    /// Optional upper bound.
    pub max: Option<usize>,
    /// Whether this symbol is a power of 2.
    pub is_power_of_2: bool,
}

impl SymbolDef {
    /// Creates a new symbol definition.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            min: None,
            max: None,
            is_power_of_2: false,
        }
    }

    /// Sets bounds on the symbol.
    pub fn with_bounds(mut self, min: usize, max: usize) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self
    }

    /// Marks this symbol as a power of 2.
    pub fn power_of_2(mut self) -> Self {
        self.is_power_of_2 = true;
        self
    }
}

/// Constraint on shapes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeConstraint {
    /// Dimension must be > 0.
    Positive(SymbolId),
    /// Dimension must be a specific value.
    Equal(SymbolId, usize),
    /// Two symbols must be equal.
    SymbolsEqual(SymbolId, SymbolId),
    /// Dimension must be divisible by a value.
    Divisible(SymbolId, usize),
    /// Dimension must be a power of 2.
    PowerOf2(SymbolId),
    /// Inner dimensions of matmul must match.
    MatmulCompat {
        /// The inner dimension of the left operand.
        k_left: SymbolId,
        /// The inner dimension of the right operand.
        k_right: SymbolId,
    },
}

/// Shape metadata for a VBC module.
///
/// Maps instruction IDs to their static shapes and tracks symbolic dimensions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShapeMetadata {
    /// Static shapes per instruction.
    pub static_shapes: HashMap<InstructionId, StaticShape>,
    /// Symbolic dimension definitions.
    pub symbols: HashMap<SymbolId, SymbolDef>,
    /// Shape constraints for verification.
    pub constraints: Vec<ShapeConstraint>,
    /// Next symbol ID.
    next_symbol_id: u32,
}

impl ShapeMetadata {
    /// Creates empty shape metadata.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a static shape for an instruction.
    pub fn add_static_shape(&mut self, instr: InstructionId, shape: StaticShape) {
        self.static_shapes.insert(instr, shape);
    }

    /// Gets the shape for an instruction.
    pub fn get_shape(&self, instr: InstructionId) -> Option<&StaticShape> {
        self.static_shapes.get(&instr)
    }

    /// Defines a new symbolic dimension.
    pub fn define_symbol(&mut self, def: SymbolDef) -> SymbolId {
        let id = SymbolId(self.next_symbol_id);
        self.next_symbol_id += 1;
        self.symbols.insert(id, def);
        id
    }

    /// Gets a symbol definition.
    pub fn get_symbol(&self, id: SymbolId) -> Option<&SymbolDef> {
        self.symbols.get(&id)
    }

    /// Adds a shape constraint.
    pub fn add_constraint(&mut self, constraint: ShapeConstraint) {
        self.constraints.push(constraint);
    }

    /// Returns true if this metadata is empty (no shapes, symbols, or constraints).
    pub fn is_empty(&self) -> bool {
        self.static_shapes.is_empty() && self.symbols.is_empty() && self.constraints.is_empty()
    }

    /// Verifies all constraints (returns violations).
    pub fn verify_constraints(
        &self,
        bindings: &HashMap<SymbolId, usize>,
    ) -> Vec<(ShapeConstraint, String)> {
        let mut violations = Vec::new();

        for constraint in &self.constraints {
            if let Some(error) = self.check_constraint(constraint, bindings) {
                violations.push((constraint.clone(), error));
            }
        }

        violations
    }

    fn check_constraint(
        &self,
        constraint: &ShapeConstraint,
        bindings: &HashMap<SymbolId, usize>,
    ) -> Option<String> {
        match constraint {
            ShapeConstraint::Positive(sym) => {
                if let Some(&val) = bindings.get(sym)
                    && val == 0 {
                        return Some(format!("Symbol {:?} must be positive, got 0", sym));
                    }
            }
            ShapeConstraint::Equal(sym, expected) => {
                if let Some(&val) = bindings.get(sym)
                    && val != *expected {
                        return Some(format!(
                            "Symbol {:?} must be {}, got {}",
                            sym, expected, val
                        ));
                    }
            }
            ShapeConstraint::SymbolsEqual(a, b) => {
                if let (Some(&va), Some(&vb)) = (bindings.get(a), bindings.get(b))
                    && va != vb {
                        return Some(format!(
                            "Symbols {:?} and {:?} must be equal: {} != {}",
                            a, b, va, vb
                        ));
                    }
            }
            ShapeConstraint::Divisible(sym, divisor) => {
                if let Some(&val) = bindings.get(sym)
                    && val % divisor != 0 {
                        return Some(format!(
                            "Symbol {:?} must be divisible by {}, got {}",
                            sym, divisor, val
                        ));
                    }
            }
            ShapeConstraint::PowerOf2(sym) => {
                if let Some(&val) = bindings.get(sym)
                    && (val == 0 || (val & (val - 1)) != 0) {
                        return Some(format!(
                            "Symbol {:?} must be a power of 2, got {}",
                            sym, val
                        ));
                    }
            }
            ShapeConstraint::MatmulCompat { k_left, k_right } => {
                if let (Some(&kl), Some(&kr)) = (bindings.get(k_left), bindings.get(k_right))
                    && kl != kr {
                        return Some(format!(
                            "Matmul inner dimensions must match: {} != {}",
                            kl, kr
                        ));
                    }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_static_shape() {
        let shape = StaticShape::from_static(&[32, 64, 128], DType::F32);
        assert_eq!(shape.ndim(), 3);
        assert_eq!(shape.numel(), Some(32 * 64 * 128));
        assert!(shape.is_fully_static());
    }

    #[test]
    fn test_symbolic_shape() {
        let mut meta = ShapeMetadata::new();
        let batch = meta.define_symbol(SymbolDef::new("batch_size").with_bounds(1, 256));

        let shape = StaticShape::new(
            vec![
                ShapeDim::Symbolic(batch),
                ShapeDim::Static(1024),
                ShapeDim::Static(768),
            ],
            DType::F32,
        );

        assert!(!shape.is_fully_static());
        assert_eq!(shape.numel(), None);
        assert_eq!(shape.ndim(), 3);
    }

    #[test]
    fn test_constraint_verification() {
        let mut meta = ShapeMetadata::new();
        let batch = meta.define_symbol(SymbolDef::new("batch_size"));
        meta.add_constraint(ShapeConstraint::Positive(batch));
        meta.add_constraint(ShapeConstraint::Divisible(batch, 8));

        let mut bindings = HashMap::new();

        // Valid binding
        bindings.insert(batch, 32);
        assert!(meta.verify_constraints(&bindings).is_empty());

        // Invalid: not divisible by 8
        bindings.insert(batch, 33);
        assert_eq!(meta.verify_constraints(&bindings).len(), 1);

        // Invalid: zero (fails Positive constraint only - 0 % 8 == 0 so Divisible passes)
        bindings.insert(batch, 0);
        assert_eq!(meta.verify_constraints(&bindings).len(), 1);
    }
}
