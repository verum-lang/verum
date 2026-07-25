//! Primitive type information for compile-time reflection
//!
//! Provides primitive type metadata matching core/meta/reflection.vr PrimitiveType.

/// Primitive type information
///
/// Matches: core/meta/reflection.vr PrimitiveType
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PrimitiveType {
    Bool = 0,
    Char = 1,
    Int8 = 2,
    Int16 = 3,
    Int32 = 4,
    Int64 = 5,
    Int = 6,
    UInt8 = 7,
    UInt16 = 8,
    UInt32 = 9,
    UInt64 = 10,
    ISize = 11,
    USize = 12,
    Float32 = 13,
    Float64 = 14,
    Text = 15,
    Unit = 16,
}

impl PrimitiveType {
    /// Get type name
    pub fn name(&self) -> &'static str {
        match self {
            PrimitiveType::Bool => "Bool",
            PrimitiveType::Char => "Char",
            PrimitiveType::Int8 => "Int8",
            PrimitiveType::Int16 => "Int16",
            PrimitiveType::Int32 => "Int32",
            PrimitiveType::Int64 => "Int64",
            PrimitiveType::Int => "Int",
            PrimitiveType::UInt8 => "UInt8",
            PrimitiveType::UInt16 => "UInt16",
            PrimitiveType::UInt32 => "UInt32",
            PrimitiveType::UInt64 => "UInt64",
            PrimitiveType::ISize => "ISize",
            PrimitiveType::USize => "USize",
            PrimitiveType::Float32 => "Float32",
            PrimitiveType::Float64 => "Float64",
            PrimitiveType::Text => "Text",
            PrimitiveType::Unit => "()",
        }
    }

    /// Get size in bytes
    pub fn size(&self) -> i64 {
        match self {
            PrimitiveType::Bool => 1,
            PrimitiveType::Char => 4,
            PrimitiveType::Int8 | PrimitiveType::UInt8 => 1,
            PrimitiveType::Int16 | PrimitiveType::UInt16 => 2,
            PrimitiveType::Int32 | PrimitiveType::UInt32 | PrimitiveType::Float32 => 4,
            PrimitiveType::Int64
            | PrimitiveType::UInt64
            | PrimitiveType::Float64
            | PrimitiveType::Int
            | PrimitiveType::ISize
            | PrimitiveType::USize => 8,
            PrimitiveType::Text => 24,
            PrimitiveType::Unit => 0,
        }
    }

    /// Check if this is a numeric type
    #[inline]
    pub fn is_numeric(&self) -> bool {
        !matches!(
            self,
            PrimitiveType::Bool | PrimitiveType::Char | PrimitiveType::Text | PrimitiveType::Unit
        )
    }

    /// Check if this is a signed integer
    #[inline]
    pub fn is_signed(&self) -> bool {
        matches!(
            self,
            PrimitiveType::Int8
                | PrimitiveType::Int16
                | PrimitiveType::Int32
                | PrimitiveType::Int64
                | PrimitiveType::Int
                | PrimitiveType::ISize
        )
    }

    /// Check if this is an unsigned integer
    #[inline]
    pub fn is_unsigned(&self) -> bool {
        matches!(
            self,
            PrimitiveType::UInt8
                | PrimitiveType::UInt16
                | PrimitiveType::UInt32
                | PrimitiveType::UInt64
                | PrimitiveType::USize
        )
    }

    /// Check if this is a floating point type
    #[inline]
    pub fn is_float(&self) -> bool {
        matches!(self, PrimitiveType::Float32 | PrimitiveType::Float64)
    }
}
