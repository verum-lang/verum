//! Tensor data type definitions.
//!
//! This module provides the canonical `DType` enum used throughout VBC for
//! tensor operations. It unifies the previously duplicated definitions in
//! `metadata/shape.rs` (compile-time) and `interpreter/tensor.rs` (runtime).
//!
//! # Naming Convention
//!
//! Type names match `core/math/tensor.vr`:
//! - Float types: F16, BF16, F32, F64
//! - Signed integers: I8, I16, I32, I64
//! - Unsigned integers: U8, U16, U32, U64
//! - Boolean: Bool
//! - Complex: Complex64, Complex128

use serde::{Deserialize, Serialize};

/// Tensor element data type.
///
/// Represents the data type of tensor elements, supporting floating-point,
/// integer, boolean, and complex number types.
///
/// # Encoding
///
/// Each variant has a fixed numeric value (0-14) used for serialization
/// and bytecode encoding. The `from_byte` and `to_byte` methods provide
/// conversion between the enum and its byte representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum DType {
    // ========================================================================
    // Floating-Point Types (0-3)
    // ========================================================================
    /// 32-bit floating point (IEEE 754 single precision).
    /// Default for neural network operations due to GPU optimization.
    #[default]
    F32 = 0,

    /// 64-bit floating point (IEEE 754 double precision).
    /// Higher precision for scientific computing.
    F64 = 1,

    /// 16-bit floating point (IEEE 754 half precision).
    /// Memory-efficient for inference, ~3.3 decimal digits precision.
    F16 = 2,

    /// 16-bit brain floating point (bfloat16).
    /// Same exponent range as F32, truncated mantissa.
    /// Preferred for training on TPUs and some GPUs.
    BF16 = 3,

    // ========================================================================
    // Signed Integer Types (4-7)
    // ========================================================================
    /// 64-bit signed integer.
    I64 = 4,

    /// 32-bit signed integer.
    I32 = 5,

    /// 16-bit signed integer.
    I16 = 6,

    /// 8-bit signed integer (quantization).
    I8 = 7,

    // ========================================================================
    // Unsigned Integer Types (8-11)
    // ========================================================================
    /// 64-bit unsigned integer.
    U64 = 8,

    /// 32-bit unsigned integer.
    U32 = 9,

    /// 16-bit unsigned integer.
    U16 = 10,

    /// 8-bit unsigned integer.
    U8 = 11,

    // ========================================================================
    // Boolean (12)
    // ========================================================================
    /// Boolean (1 byte).
    Bool = 12,

    // ========================================================================
    // Complex Types (13-14)
    // ========================================================================
    /// Complex with f32 components (2x f32 = 8 bytes).
    /// Also known as cfloat or complex<float>.
    Complex64 = 13,

    /// Complex with f64 components (2x f64 = 16 bytes).
    /// Also known as cdouble or complex<double>.
    Complex128 = 14,
}

impl DType {
    // ========================================================================
    // Deprecated Aliases (backward compatibility)
    // ========================================================================

    /// Alias for Complex64 (deprecated).
    #[deprecated(since = "0.2.0", note = "Use Complex64 instead")]
    pub const C64: DType = DType::Complex64;

    /// Alias for Complex128 (deprecated).
    #[deprecated(since = "0.2.0", note = "Use Complex128 instead")]
    pub const C128: DType = DType::Complex128;

    // ========================================================================
    // Size and Alignment
    // ========================================================================

    /// Returns the size in bytes of a single element.
    #[inline]
    pub const fn size_bytes(&self) -> usize {
        match self {
            DType::I8 | DType::U8 | DType::Bool => 1,
            DType::I16 | DType::U16 | DType::F16 | DType::BF16 => 2,
            DType::I32 | DType::U32 | DType::F32 => 4,
            DType::I64 | DType::U64 | DType::F64 | DType::Complex64 => 8,
            DType::Complex128 => 16,
        }
    }

    /// Returns the size in bytes (alias for `size_bytes`).
    #[inline]
    pub const fn size(&self) -> usize {
        self.size_bytes()
    }

    /// Returns the alignment in bytes.
    #[inline]
    pub const fn alignment(&self) -> usize {
        match self {
            DType::I8 | DType::U8 | DType::Bool => 1,
            DType::I16 | DType::U16 | DType::F16 | DType::BF16 => 2,
            DType::I32 | DType::U32 | DType::F32 => 4,
            DType::I64 | DType::U64 | DType::F64 | DType::Complex64 | DType::Complex128 => 8,
        }
    }

    /// Returns the alignment in bytes (alias for `alignment`).
    #[inline]
    pub const fn align(&self) -> usize {
        self.alignment()
    }

    // ========================================================================
    // Type Classification
    // ========================================================================

    /// Returns true if this is a floating-point type (F16, BF16, F32, F64).
    #[inline]
    pub const fn is_float(&self) -> bool {
        matches!(self, DType::F16 | DType::BF16 | DType::F32 | DType::F64)
    }

    /// Returns true if this is a signed integer type (I8, I16, I32, I64).
    #[inline]
    pub const fn is_signed_int(&self) -> bool {
        matches!(self, DType::I8 | DType::I16 | DType::I32 | DType::I64)
    }

    /// Returns true if this is a signed integer type (alias for `is_signed_int`).
    #[inline]
    pub const fn is_signed_integer(&self) -> bool {
        self.is_signed_int()
    }

    /// Returns true if this is an unsigned integer type (U8, U16, U32, U64).
    #[inline]
    pub const fn is_unsigned_int(&self) -> bool {
        matches!(self, DType::U8 | DType::U16 | DType::U32 | DType::U64)
    }

    /// Returns true if this is an unsigned integer type (alias for `is_unsigned_int`).
    #[inline]
    pub const fn is_unsigned_integer(&self) -> bool {
        self.is_unsigned_int()
    }

    /// Returns true if this is any integer type (signed or unsigned).
    #[inline]
    pub const fn is_integer(&self) -> bool {
        self.is_signed_int() || self.is_unsigned_int()
    }

    /// Returns true if this is a complex type (Complex64, Complex128).
    #[inline]
    pub const fn is_complex(&self) -> bool {
        matches!(self, DType::Complex64 | DType::Complex128)
    }

    /// Returns true if this is a half-precision type (F16, BF16).
    #[inline]
    pub const fn is_half(&self) -> bool {
        matches!(self, DType::F16 | DType::BF16)
    }

    /// Returns true if this is a boolean type.
    #[inline]
    pub const fn is_bool(&self) -> bool {
        matches!(self, DType::Bool)
    }

    /// Returns true if this is a numeric type (not bool).
    #[inline]
    pub const fn is_numeric(&self) -> bool {
        !self.is_bool()
    }

    // ========================================================================
    // Bit Width
    // ========================================================================

    /// Returns the bit width of the type.
    #[inline]
    pub const fn bit_width(&self) -> usize {
        self.size_bytes() * 8
    }

    // ========================================================================
    // Byte Encoding
    // ========================================================================

    /// Converts to a byte representation.
    #[inline]
    pub const fn to_byte(&self) -> u8 {
        *self as u8
    }

    /// Creates from a byte representation.
    ///
    /// Returns `None` if the byte doesn't correspond to a valid DType.
    #[inline]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(DType::F32),
            1 => Some(DType::F64),
            2 => Some(DType::F16),
            3 => Some(DType::BF16),
            4 => Some(DType::I64),
            5 => Some(DType::I32),
            6 => Some(DType::I16),
            7 => Some(DType::I8),
            8 => Some(DType::U64),
            9 => Some(DType::U32),
            10 => Some(DType::U16),
            11 => Some(DType::U8),
            12 => Some(DType::Bool),
            13 => Some(DType::Complex64),
            14 => Some(DType::Complex128),
            _ => None,
        }
    }

    /// Creates from a VBC type id.
    ///
    /// Type IDs match the enum discriminants for direct mapping.
    /// Returns F32 for unknown type IDs.
    #[inline]
    pub const fn from_type_id(id: u8) -> Self {
        match Self::from_byte(id) {
            Some(dtype) => dtype,
            None => DType::F32, // Default fallback
        }
    }

    // ========================================================================
    // Component Types (for complex numbers)
    // ========================================================================

    /// Returns the component type for complex dtypes.
    ///
    /// For Complex64, returns F32. For Complex128, returns F64.
    /// For non-complex types, returns self.
    #[inline]
    pub const fn component_type(&self) -> Self {
        match self {
            DType::Complex64 => DType::F32,
            DType::Complex128 => DType::F64,
            _ => *self,
        }
    }

    // ========================================================================
    // Type Promotion
    // ========================================================================

    /// Returns type priority for promotion (higher = wins).
    ///
    /// Priority ordering: Complex > Float > Integer > Bool
    #[inline]
    const fn type_priority(&self) -> u8 {
        match self {
            DType::Bool => 0,
            DType::I8 | DType::U8 => 1,
            DType::I16 | DType::U16 => 2,
            DType::I32 | DType::U32 => 3,
            DType::I64 | DType::U64 => 4,
            DType::F16 | DType::BF16 => 5,
            DType::F32 => 6,
            DType::F64 => 7,
            DType::Complex64 => 8,
            DType::Complex128 => 9,
        }
    }

    /// Returns the larger float type between two types.
    #[inline]
    const fn larger_float(a: DType, b: DType) -> DType {
        if matches!(a, DType::F64) || matches!(b, DType::F64) {
            DType::F64
        } else if matches!(a, DType::F32) || matches!(b, DType::F32) {
            DType::F32
        } else if matches!(a, DType::BF16) || matches!(b, DType::BF16) {
            DType::BF16
        } else {
            DType::F16
        }
    }

    /// Returns the larger integer type between two types.
    #[inline]
    const fn larger_integer(a: DType, b: DType) -> DType {
        if matches!(a, DType::I64) || matches!(b, DType::I64) {
            DType::I64
        } else if matches!(a, DType::U64) || matches!(b, DType::U64) {
            DType::U64
        } else if matches!(a, DType::I32) || matches!(b, DType::I32) {
            DType::I32
        } else if matches!(a, DType::U32) || matches!(b, DType::U32) {
            DType::U32
        } else if matches!(a, DType::I16) || matches!(b, DType::I16) {
            DType::I16
        } else if matches!(a, DType::U16) || matches!(b, DType::U16) {
            DType::U16
        } else if matches!(a, DType::U8) {
            // Mixed signed/unsigned I8/U8: promote to I16 for safety
            DType::I16
        } else {
            DType::I8
        }
    }

    /// Returns the promoted type for binary operations (instance method).
    ///
    /// Follows NumPy-compatible type promotion rules:
    /// - Bool promotes to any numeric type
    /// - Integer promotes to float when mixed
    /// - Smaller types promote to larger types within category
    /// - Complex wins over real
    #[inline]
    pub const fn promote(&self, other: &Self) -> Self {
        Self::promote_static(*self, *other)
    }

    /// Returns the promoted dtype for binary operations (static method).
    ///
    /// Type promotion rules (NumPy-compatible):
    /// - Bool promotes to any numeric type
    /// - Integer promotes to float when mixed
    /// - Smaller types promote to larger types within category
    /// - Complex wins over real
    #[inline]
    pub const fn promote_static(a: DType, b: DType) -> DType {
        if a.to_byte() == b.to_byte() {
            return a;
        }

        // Priority ordering (higher wins): Complex > Float > Integer > Bool
        let priority_a = a.type_priority();
        let priority_b = b.type_priority();

        if priority_a >= priority_b {
            // a has higher or equal priority
            if a.is_complex() || b.is_complex() {
                // Both complex: pick larger
                if matches!(a, DType::Complex128) || matches!(b, DType::Complex128) {
                    DType::Complex128
                } else {
                    DType::Complex64
                }
            } else if a.is_float() || b.is_float() {
                // At least one float: pick larger float
                Self::larger_float(a, b)
            } else {
                // Both integers: pick larger
                Self::larger_integer(a, b)
            }
        } else {
            // b has higher priority
            Self::promote_static(b, a)
        }
    }

    // ========================================================================
    // String Representation
    // ========================================================================

    /// Returns the canonical string name.
    #[inline]
    pub const fn name(&self) -> &'static str {
        match self {
            DType::F32 => "F32",
            DType::F64 => "F64",
            DType::F16 => "F16",
            DType::BF16 => "BF16",
            DType::I64 => "I64",
            DType::I32 => "I32",
            DType::I16 => "I16",
            DType::I8 => "I8",
            DType::U64 => "U64",
            DType::U32 => "U32",
            DType::U16 => "U16",
            DType::U8 => "U8",
            DType::Bool => "Bool",
            DType::Complex64 => "Complex64",
            DType::Complex128 => "Complex128",
        }
    }

    /// Parses from a string name.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "F32" | "f32" | "Float32" | "float32" => Some(DType::F32),
            "F64" | "f64" | "Float64" | "float64" | "Float" | "float" => Some(DType::F64),
            "F16" | "f16" | "Float16" | "float16" => Some(DType::F16),
            "BF16" | "bf16" | "BFloat16" | "bfloat16" => Some(DType::BF16),
            "I64" | "i64" | "Int64" | "int64" | "Int" | "int" => Some(DType::I64),
            "I32" | "i32" | "Int32" | "int32" => Some(DType::I32),
            "I16" | "i16" | "Int16" | "int16" => Some(DType::I16),
            "I8" | "i8" | "Int8" | "int8" => Some(DType::I8),
            "U64" | "u64" | "UInt64" | "uint64" => Some(DType::U64),
            "U32" | "u32" | "UInt32" | "uint32" => Some(DType::U32),
            "U16" | "u16" | "UInt16" | "uint16" => Some(DType::U16),
            "U8" | "u8" | "UInt8" | "uint8" => Some(DType::U8),
            "Bool" | "bool" | "Boolean" | "boolean" => Some(DType::Bool),
            "Complex64" | "complex64" | "C64" | "c64" => Some(DType::Complex64),
            "Complex128" | "complex128" | "C128" | "c128" => Some(DType::Complex128),
            _ => None,
        }
    }
}

impl std::fmt::Display for DType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_bytes() {
        assert_eq!(DType::Bool.size_bytes(), 1);
        assert_eq!(DType::I8.size_bytes(), 1);
        assert_eq!(DType::U8.size_bytes(), 1);
        assert_eq!(DType::I16.size_bytes(), 2);
        assert_eq!(DType::F16.size_bytes(), 2);
        assert_eq!(DType::BF16.size_bytes(), 2);
        assert_eq!(DType::I32.size_bytes(), 4);
        assert_eq!(DType::F32.size_bytes(), 4);
        assert_eq!(DType::I64.size_bytes(), 8);
        assert_eq!(DType::F64.size_bytes(), 8);
        assert_eq!(DType::Complex64.size_bytes(), 8);
        assert_eq!(DType::Complex128.size_bytes(), 16);
    }

    #[test]
    fn test_type_classification() {
        assert!(DType::F32.is_float());
        assert!(DType::F64.is_float());
        assert!(DType::F16.is_float());
        assert!(DType::BF16.is_float());
        assert!(!DType::I32.is_float());

        assert!(DType::I8.is_signed_int());
        assert!(DType::I16.is_signed_int());
        assert!(DType::I32.is_signed_int());
        assert!(DType::I64.is_signed_int());
        assert!(!DType::U32.is_signed_int());

        assert!(DType::U8.is_unsigned_int());
        assert!(DType::U64.is_unsigned_int());
        assert!(!DType::I32.is_unsigned_int());

        assert!(DType::Complex64.is_complex());
        assert!(DType::Complex128.is_complex());
        assert!(!DType::F64.is_complex());

        assert!(DType::Bool.is_bool());
        assert!(!DType::I32.is_bool());
    }

    #[test]
    fn test_byte_roundtrip() {
        for byte in 0..=14 {
            let dtype = DType::from_byte(byte).expect("valid byte");
            assert_eq!(dtype.to_byte(), byte);
        }
        assert!(DType::from_byte(15).is_none());
        assert!(DType::from_byte(255).is_none());
    }

    #[test]
    fn test_name_roundtrip() {
        for byte in 0..=14 {
            let dtype = DType::from_byte(byte).unwrap();
            let name = dtype.name();
            let parsed = DType::from_name(name).expect("valid name");
            assert_eq!(dtype, parsed);
        }
    }

    #[test]
    fn test_promote() {
        // Float promotes integer
        assert_eq!(DType::F32.promote(&DType::I32), DType::F32);
        assert_eq!(DType::I32.promote(&DType::F64), DType::F64);

        // Wider float wins
        assert_eq!(DType::F32.promote(&DType::F64), DType::F64);

        // Complex dominates
        assert_eq!(DType::Complex64.promote(&DType::F64), DType::Complex64);
        assert_eq!(DType::F32.promote(&DType::Complex128), DType::Complex128);
    }

    #[test]
    fn test_component_type() {
        assert_eq!(DType::Complex64.component_type(), DType::F32);
        assert_eq!(DType::Complex128.component_type(), DType::F64);
        assert_eq!(DType::F32.component_type(), DType::F32);
    }

    #[test]
    fn test_default() {
        assert_eq!(DType::default(), DType::F32);
    }
}
