//! Integer Type Hierarchy
//!
//! Integer type hierarchy: all fixed-size integers (i8..i128, u8..u128) are refinement types of Int with range predicates
//!
//! Verum provides a comprehensive integer type hierarchy with explicit bounds
//! and overflow semantics. All integer types are refinements of the base `Int` type.
//!
//! # Semantic Type Names (Primary)
//!
//! Following Verum's **Semantic Honesty** principle, numeric types use descriptive names:
//!
//! ```text
//! Int (arbitrary precision, default for integer literals)
//! ├── Signed Fixed-Width
//! │   ├── Int8   = Int{>= -128 && <= 127}
//! │   ├── Int16  = Int{>= -32768 && <= 32767}
//! │   ├── Int32  = Int{>= -2147483648 && <= 2147483647}
//! │   ├── Int64  = Int{>= -9223372036854775808 && <= 9223372036854775807}
//! │   ├── Int128 = Int{>= -(2^127) && <= (2^127 - 1)}
//! │   └── ISize  = platform-dependent (Int32 or Int64)
//! │
//! └── Unsigned Fixed-Width
//!     ├── UInt8   = Int{>= 0 && <= 255}
//!     ├── UInt16  = Int{>= 0 && <= 65535}
//!     ├── UInt32  = Int{>= 0 && <= 4294967295}
//!     ├── UInt64  = Int{>= 0 && <= 18446744073709551615}
//!     ├── UInt128 = Int{>= 0 && <= (2^128 - 1)}
//!     └── USize   = platform-dependent (UInt32 or UInt64)
//! ```
//!
//! # Compatibility Aliases (FFI)
//!
//! For interoperability with Rust/C, aliases are provided:
//! - i8, i16, i32, i64, i128, isize → Int8, Int16, Int32, Int64, Int128, ISize
//! - u8, u16, u32, u64, u128, usize → UInt8, UInt16, UInt32, UInt64, UInt128, USize
//!
//! # Overflow Semantics
//!
//! Three overflow modes (controlled by annotations):
//! - **checked** (default): Runtime overflow detection, panic on overflow
//! - **wrapping**: Two's complement wraparound
//! - **saturating**: Clamp to min/max bounds
//!
//! # Examples
//!
//! ```verum
//! // Type inference from literals
//! let a: i32 = 42;        // Inferred as i32
//! let b = 42_i64;         // Explicit i64 suffix
//! let c: u8 = 255;        // Maximum u8 value
//!
//! // Overflow modes
//! @overflow(checked)
//! fn safe_add(x: i32, y: i32) -> i32 {
//!     x + y  // Panics on overflow
//! }
//!
//! @overflow(wrapping)
//! fn wrapping_add(x: u8, y: u8) -> u8 {
//!     x + y  // Wraps: 255 + 1 = 0
//! }
//!
//! @overflow(saturating)
//! fn saturating_add(x: u8, y: u8) -> u8 {
//!     x + y  // Saturates: 255 + 1 = 255
//! }
//! ```

use crate::ty::Type;
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{List, Map, Maybe, Text};

/// Overflow behavior for integer operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OverflowMode {
    /// Runtime overflow detection (default)
    #[default]
    Checked,
    /// Two's complement wraparound
    Wrapping,
    /// Clamp to min/max bounds
    Saturating,
}

/// Integer type variant
///
/// Primary names follow Verum's Semantic Honesty principle (Int8, UInt64, etc.)
/// Compatibility aliases (i8, u64) are also supported for FFI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntegerKind {
    // Unbounded (arbitrary precision)
    Int,

    // Signed fixed-width (semantic names: Int8, Int16, Int32, Int64, Int128, ISize)
    Int8,
    Int16,
    Int32,
    Int64,
    Int128,
    ISize,

    // Unsigned fixed-width (semantic names: UInt8, UInt16, UInt32, UInt64, UInt128, USize)
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    UInt128,
    USize,
}

impl IntegerKind {
    /// Get the bit width of this integer type
    pub fn bit_width(&self) -> Option<u32> {
        match self {
            IntegerKind::Int => None, // Unbounded (arbitrary precision)
            IntegerKind::Int8 | IntegerKind::UInt8 => Some(8),
            IntegerKind::Int16 | IntegerKind::UInt16 => Some(16),
            IntegerKind::Int32 | IntegerKind::UInt32 => Some(32),
            IntegerKind::Int64 | IntegerKind::UInt64 => Some(64),
            IntegerKind::Int128 | IntegerKind::UInt128 => Some(128),
            IntegerKind::ISize | IntegerKind::USize => {
                // Platform-dependent: 32 or 64 bits
                if cfg!(target_pointer_width = "32") {
                    Some(32)
                } else {
                    Some(64)
                }
            }
        }
    }

    /// Check if this is a signed integer type
    pub fn is_signed(&self) -> bool {
        matches!(
            self,
            IntegerKind::Int
                | IntegerKind::Int8
                | IntegerKind::Int16
                | IntegerKind::Int32
                | IntegerKind::Int64
                | IntegerKind::Int128
                | IntegerKind::ISize
        )
    }

    /// Check if this is an unsigned integer type
    pub fn is_unsigned(&self) -> bool {
        matches!(
            self,
            IntegerKind::UInt8
                | IntegerKind::UInt16
                | IntegerKind::UInt32
                | IntegerKind::UInt64
                | IntegerKind::UInt128
                | IntegerKind::USize
        )
    }

    /// Get the minimum value for this integer type
    pub fn min_value(&self) -> i128 {
        match self {
            IntegerKind::Int => i128::MIN, // Approximation for arbitrary precision
            IntegerKind::Int8 => i8::MIN as i128,
            IntegerKind::Int16 => i16::MIN as i128,
            IntegerKind::Int32 => i32::MIN as i128,
            IntegerKind::Int64 => i64::MIN as i128,
            IntegerKind::Int128 => i128::MIN,
            IntegerKind::ISize => {
                if cfg!(target_pointer_width = "32") {
                    i32::MIN as i128
                } else {
                    i64::MIN as i128
                }
            }
            IntegerKind::UInt8 => 0,
            IntegerKind::UInt16 => 0,
            IntegerKind::UInt32 => 0,
            IntegerKind::UInt64 => 0,
            IntegerKind::UInt128 => 0,
            IntegerKind::USize => 0,
        }
    }

    /// Get the maximum value for this integer type
    pub fn max_value(&self) -> i128 {
        match self {
            IntegerKind::Int => i128::MAX, // Approximation for arbitrary precision
            IntegerKind::Int8 => i8::MAX as i128,
            IntegerKind::Int16 => i16::MAX as i128,
            IntegerKind::Int32 => i32::MAX as i128,
            IntegerKind::Int64 => i64::MAX as i128,
            IntegerKind::Int128 => i128::MAX,
            IntegerKind::ISize => {
                if cfg!(target_pointer_width = "32") {
                    i32::MAX as i128
                } else {
                    i64::MAX as i128
                }
            }
            IntegerKind::UInt8 => u8::MAX as i128,
            IntegerKind::UInt16 => u16::MAX as i128,
            IntegerKind::UInt32 => u32::MAX as i128,
            IntegerKind::UInt64 => u64::MAX as i128,
            IntegerKind::UInt128 => {
                // u128::MAX doesn't fit in i128, use max i128 as approximation
                i128::MAX
            }
            IntegerKind::USize => {
                if cfg!(target_pointer_width = "32") {
                    u32::MAX as i128
                } else {
                    u64::MAX as i128
                }
            }
        }
    }

    /// Get the maximum value as u128 for unsigned types (full precision)
    pub fn max_value_u128(&self) -> u128 {
        match self {
            IntegerKind::Int => u128::MAX, // Approximation
            IntegerKind::Int8 => i8::MAX as u128,
            IntegerKind::Int16 => i16::MAX as u128,
            IntegerKind::Int32 => i32::MAX as u128,
            IntegerKind::Int64 => i64::MAX as u128,
            IntegerKind::Int128 => i128::MAX as u128,
            IntegerKind::ISize => {
                if cfg!(target_pointer_width = "32") {
                    i32::MAX as u128
                } else {
                    i64::MAX as u128
                }
            }
            IntegerKind::UInt8 => u8::MAX as u128,
            IntegerKind::UInt16 => u16::MAX as u128,
            IntegerKind::UInt32 => u32::MAX as u128,
            IntegerKind::UInt64 => u64::MAX as u128,
            IntegerKind::UInt128 => u128::MAX,
            IntegerKind::USize => {
                if cfg!(target_pointer_width = "32") {
                    u32::MAX as u128
                } else {
                    u64::MAX as u128
                }
            }
        }
    }

    /// Get the semantic type name (primary name following Verum conventions)
    pub fn name(&self) -> Text {
        Text::from(self.semantic_name())
    }

    /// Get the semantic type name as &'static str
    pub fn semantic_name(&self) -> &'static str {
        match self {
            IntegerKind::Int => "Int",
            IntegerKind::Int8 => "Int8",
            IntegerKind::Int16 => "Int16",
            IntegerKind::Int32 => "Int32",
            IntegerKind::Int64 => "Int64",
            IntegerKind::Int128 => "Int128",
            IntegerKind::ISize => "ISize",
            IntegerKind::UInt8 => "UInt8",
            IntegerKind::UInt16 => "UInt16",
            IntegerKind::UInt32 => "UInt32",
            IntegerKind::UInt64 => "UInt64",
            IntegerKind::UInt128 => "UInt128",
            IntegerKind::USize => "USize",
        }
    }

    /// Get the compatibility alias name (Rust-style, for FFI)
    pub fn compat_name(&self) -> &'static str {
        match self {
            IntegerKind::Int => "Int",
            IntegerKind::Int8 => "i8",
            IntegerKind::Int16 => "i16",
            IntegerKind::Int32 => "i32",
            IntegerKind::Int64 => "i64",
            IntegerKind::Int128 => "i128",
            IntegerKind::ISize => "isize",
            IntegerKind::UInt8 => "u8",
            IntegerKind::UInt16 => "u16",
            IntegerKind::UInt32 => "u32",
            IntegerKind::UInt64 => "u64",
            IntegerKind::UInt128 => "u128",
            IntegerKind::USize => "usize",
        }
    }

    /// Parse an integer type from a string (accepts both semantic and compat names)
    pub fn from_name(name: &str) -> Option<IntegerKind> {
        match name {
            // Base unbounded type
            "Int" => Some(IntegerKind::Int),

            // Semantic names (primary)
            "Int8" => Some(IntegerKind::Int8),
            "Int16" => Some(IntegerKind::Int16),
            "Int32" => Some(IntegerKind::Int32),
            "Int64" => Some(IntegerKind::Int64),
            "Int128" => Some(IntegerKind::Int128),
            "ISize" => Some(IntegerKind::ISize),
            "UInt8" => Some(IntegerKind::UInt8),
            "UInt16" => Some(IntegerKind::UInt16),
            "UInt32" => Some(IntegerKind::UInt32),
            "UInt64" => Some(IntegerKind::UInt64),
            "UInt128" => Some(IntegerKind::UInt128),
            "USize" => Some(IntegerKind::USize),

            // Compatibility aliases (FFI)
            "i8" => Some(IntegerKind::Int8),
            "i16" => Some(IntegerKind::Int16),
            "i32" => Some(IntegerKind::Int32),
            "i64" => Some(IntegerKind::Int64),
            "i128" => Some(IntegerKind::Int128),
            "isize" => Some(IntegerKind::ISize),
            "u8" => Some(IntegerKind::UInt8),
            "u16" => Some(IntegerKind::UInt16),
            "u32" => Some(IntegerKind::UInt32),
            "u64" => Some(IntegerKind::UInt64),
            "u128" => Some(IntegerKind::UInt128),
            "usize" => Some(IntegerKind::USize),

            // Byte type (alias for UInt8)
            "Byte" => Some(IntegerKind::UInt8),

            _ => None,
        }
    }

    /// Check if the given name is a valid integer type name
    pub fn is_valid_name(name: &str) -> bool {
        Self::from_name(name).is_some()
    }
}

/// Integer type hierarchy manager
#[derive(Debug, Clone)]
pub struct IntegerHierarchy {
    /// Cache of predefined integer types
    types: Map<IntegerKind, Type>,
}

impl IntegerHierarchy {
    /// Create a new integer hierarchy with all predefined types
    ///
    /// Integer types are represented as `Type::Named` with their semantic names.
    /// For example, `Int32` becomes `Type::Named { path: "Int32", args: [] }`.
    /// Refinement checking (bounds validation) happens later in the type checker.
    pub fn new() -> Self {
        let mut types = Map::new();

        // Base unbounded Int type
        types.insert(IntegerKind::Int, Type::Int);

        // Create all fixed-width integer types as Type::Named
        // Using semantic names (Int8, UInt64, etc.)
        for kind in [
            IntegerKind::Int8,
            IntegerKind::Int16,
            IntegerKind::Int32,
            IntegerKind::Int64,
            IntegerKind::Int128,
            IntegerKind::ISize,
            IntegerKind::UInt8,
            IntegerKind::UInt16,
            IntegerKind::UInt32,
            IntegerKind::UInt64,
            IntegerKind::UInt128,
            IntegerKind::USize,
        ] {
            // Create a Type::Named with the semantic type name
            let name = kind.name();
            let ident = Ident::new(name.as_str(), Span::dummy());
            let path = Path::single(ident);
            let ty = Type::Named {
                path,
                args: vec![].into(),
            };
            types.insert(kind, ty);
        }

        IntegerHierarchy { types }
    }

    /// Get the type for a given integer kind
    pub fn get_type(&self, kind: IntegerKind) -> Option<&Type> {
        match self.types.get(&kind) {
            Some(ty) => Some(ty),
            None => None,
        }
    }

    /// Check if a literal value fits in the given integer type
    pub fn check_literal_fits(&self, value: i128, kind: IntegerKind) -> bool {
        let min = kind.min_value();
        let max = kind.max_value();
        value >= min && value <= max
    }

    /// Infer integer type from literal suffix
    ///
    /// Examples:
    /// - `42` → None (context-dependent)
    /// - `42_i32` → Some(I32)
    /// - `255_u8` → Some(U8)
    pub fn infer_from_suffix(&self, suffix: &str) -> Option<IntegerKind> {
        IntegerKind::from_name(suffix)
    }

    /// Get default integer type for unsuffixed literals
    ///
    /// Spec: Default is Int32 for signed, UInt32 for unsigned
    pub fn default_signed() -> IntegerKind {
        IntegerKind::Int32
    }

    pub fn default_unsigned() -> IntegerKind {
        IntegerKind::UInt32
    }

    /// Check if type1 is a subtype of type2 in the integer hierarchy
    ///
    /// Examples:
    /// - i8 <: i16 (smaller range is subtype of larger)
    /// - u8 <: u16 (smaller range is subtype of larger)
    /// - i32 <: Int (bounded is subtype of unbounded)
    /// - u8 NOT <: i8 (unsigned not subtype of signed)
    pub fn is_subtype(&self, sub: IntegerKind, sup: IntegerKind) -> bool {
        // Int is the top type
        if sup == IntegerKind::Int {
            return true;
        }

        // Same type
        if sub == sup {
            return true;
        }

        // Check range subsumption
        let sub_min = sub.min_value();
        let sub_max = sub.max_value();
        let sup_min = sup.min_value();
        let sup_max = sup.max_value();

        // Subtype if range is contained: [sub_min, sub_max] ⊆ [sup_min, sup_max]
        sub_min >= sup_min && sub_max <= sup_max
    }

    /// Get all integer types in the hierarchy
    pub fn all_kinds(&self) -> List<IntegerKind> {
        List::from(vec![
            IntegerKind::Int,
            IntegerKind::Int8,
            IntegerKind::Int16,
            IntegerKind::Int32,
            IntegerKind::Int64,
            IntegerKind::Int128,
            IntegerKind::ISize,
            IntegerKind::UInt8,
            IntegerKind::UInt16,
            IntegerKind::UInt32,
            IntegerKind::UInt64,
            IntegerKind::UInt128,
            IntegerKind::USize,
        ])
    }
}

impl Default for IntegerHierarchy {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Overflow Mode Traits
// Integer type hierarchy: all fixed-size integers (i8..i128, u8..u128) are refinement types of Int with range predicates — .3
// ============================================================================

/// Trait for checked arithmetic operations (returns Maybe<T>)
///
/// Checked operations detect overflow/underflow and return None on failure.
/// This is the default behavior for arithmetic operators in Verum.
///
/// # Examples
/// ```
/// use verum_types::CheckedOps;
/// use verum_common::Maybe;
///
/// let x: i32 = i32::MAX;
/// assert_eq!(x.checked_add(1), None); // Overflow detected
///
/// let y: i32 = 100;
/// assert_eq!(y.checked_add(50), Some(150));
/// ```
pub trait CheckedOps: Sized {
    fn checked_add(self, rhs: Self) -> Maybe<Self>;
    fn checked_sub(self, rhs: Self) -> Maybe<Self>;
    fn checked_mul(self, rhs: Self) -> Maybe<Self>;
    fn checked_div(self, rhs: Self) -> Maybe<Self>;
    fn checked_rem(self, rhs: Self) -> Maybe<Self>;
    fn checked_neg(self) -> Maybe<Self>;
    fn checked_shl(self, rhs: u32) -> Maybe<Self>;
    fn checked_shr(self, rhs: u32) -> Maybe<Self>;
    fn checked_pow(self, exp: u32) -> Maybe<Self>;
}

/// Trait for wrapping arithmetic operations (two's complement)
///
/// Wrapping operations perform modular arithmetic, wrapping around on overflow.
/// Useful for bit manipulation, cryptography, and hash functions.
///
/// # Examples
/// ```
/// use verum_types::WrappingOps;
///
/// let x: i32 = i32::MAX;
/// assert_eq!(x.wrapping_add(1), i32::MIN); // Wraps to minimum
///
/// let y: u8 = 255;
/// assert_eq!(y.wrapping_add(1), 0); // Wraps to 0
/// ```
pub trait WrappingOps: Sized {
    fn wrapping_add(self, rhs: Self) -> Self;
    fn wrapping_sub(self, rhs: Self) -> Self;
    fn wrapping_mul(self, rhs: Self) -> Self;
    fn wrapping_div(self, rhs: Self) -> Self;
    fn wrapping_rem(self, rhs: Self) -> Self;
    fn wrapping_neg(self) -> Self;
    fn wrapping_shl(self, rhs: u32) -> Self;
    fn wrapping_shr(self, rhs: u32) -> Self;
    fn wrapping_pow(self, exp: u32) -> Self;
}

/// Trait for saturating arithmetic operations (clamp to bounds)
///
/// Saturating operations clamp results to min/max values on overflow.
/// Useful for graphics, audio, signal processing, and embedded systems.
///
/// # Examples
/// ```
/// use verum_types::SaturatingOps;
///
/// let x: i32 = i32::MAX;
/// assert_eq!(x.saturating_add(1), i32::MAX); // Clamps to max
///
/// let y: u8 = 10;
/// assert_eq!(y.saturating_sub(20), 0); // Clamps to 0
/// ```
pub trait SaturatingOps: Sized {
    fn saturating_add(self, rhs: Self) -> Self;
    fn saturating_sub(self, rhs: Self) -> Self;
    fn saturating_mul(self, rhs: Self) -> Self;
    fn saturating_pow(self, exp: u32) -> Self;
}

// ============================================================================
// Trait Implementations for All Integer Types
// ============================================================================

macro_rules! impl_overflow_modes {
    ($($t:ty),* $(,)?) => {
        $(
            impl CheckedOps for $t {
                #[inline]
                fn checked_add(self, rhs: Self) -> Maybe<Self> {
                    <$t>::checked_add(self, rhs).into()
                }

                #[inline]
                fn checked_sub(self, rhs: Self) -> Maybe<Self> {
                    <$t>::checked_sub(self, rhs).into()
                }

                #[inline]
                fn checked_mul(self, rhs: Self) -> Maybe<Self> {
                    <$t>::checked_mul(self, rhs).into()
                }

                #[inline]
                fn checked_div(self, rhs: Self) -> Maybe<Self> {
                    <$t>::checked_div(self, rhs).into()
                }

                #[inline]
                fn checked_rem(self, rhs: Self) -> Maybe<Self> {
                    <$t>::checked_rem(self, rhs).into()
                }

                #[inline]
                fn checked_neg(self) -> Maybe<Self> {
                    <$t>::checked_neg(self).into()
                }

                #[inline]
                fn checked_shl(self, rhs: u32) -> Maybe<Self> {
                    <$t>::checked_shl(self, rhs).into()
                }

                #[inline]
                fn checked_shr(self, rhs: u32) -> Maybe<Self> {
                    <$t>::checked_shr(self, rhs).into()
                }

                #[inline]
                fn checked_pow(self, exp: u32) -> Maybe<Self> {
                    <$t>::checked_pow(self, exp).into()
                }
            }

            impl WrappingOps for $t {
                #[inline]
                fn wrapping_add(self, rhs: Self) -> Self {
                    <$t>::wrapping_add(self, rhs)
                }

                #[inline]
                fn wrapping_sub(self, rhs: Self) -> Self {
                    <$t>::wrapping_sub(self, rhs)
                }

                #[inline]
                fn wrapping_mul(self, rhs: Self) -> Self {
                    <$t>::wrapping_mul(self, rhs)
                }

                #[inline]
                fn wrapping_div(self, rhs: Self) -> Self {
                    self.wrapping_div(rhs)
                }

                #[inline]
                fn wrapping_rem(self, rhs: Self) -> Self {
                    self.wrapping_rem(rhs)
                }

                #[inline]
                fn wrapping_neg(self) -> Self {
                    <$t>::wrapping_neg(self)
                }

                #[inline]
                fn wrapping_shl(self, rhs: u32) -> Self {
                    <$t>::wrapping_shl(self, rhs)
                }

                #[inline]
                fn wrapping_shr(self, rhs: u32) -> Self {
                    <$t>::wrapping_shr(self, rhs)
                }

                #[inline]
                fn wrapping_pow(self, exp: u32) -> Self {
                    self.wrapping_pow(exp)
                }
            }

            impl SaturatingOps for $t {
                #[inline]
                fn saturating_add(self, rhs: Self) -> Self {
                    <$t>::saturating_add(self, rhs)
                }

                #[inline]
                fn saturating_sub(self, rhs: Self) -> Self {
                    <$t>::saturating_sub(self, rhs)
                }

                #[inline]
                fn saturating_mul(self, rhs: Self) -> Self {
                    <$t>::saturating_mul(self, rhs)
                }

                #[inline]
                fn saturating_pow(self, exp: u32) -> Self {
                    <$t>::saturating_pow(self, exp)
                }
            }
        )*
    };
}

// Apply to all 12 integer types
impl_overflow_modes!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

// Tests moved to tests/integer_hierarchy_tests.rs
