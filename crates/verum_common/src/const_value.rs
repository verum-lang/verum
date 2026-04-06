//! Unified compile-time constant values
//!
//! Compile-time constant representation for the Verum meta-programming system.
//! Used in @const evaluation, @cfg conditions, compile-time function evaluation,
//! and attribute argument processing. Supports Unit, Bool, Int (i128), UInt (u128),
//! Float (f64), Char, Text, Bytes, Array, Tuple, Maybe, Map, and Set values.
//!
//! This module provides the canonical `ConstValue` type used throughout Verum
//! for representing compile-time constant values. It unifies previously
//! separate definitions from verum_types and verum_protocol_types.
//!
//! # Design Decisions
//!
//! - **Maximum precision**: Uses `i128`/`u128` for integers to avoid precision loss
//! - **Layer-compliant**: No dependency on higher layers (verum_ast, etc.)
//! - **Extensible**: verum_compiler provides AST-extended variant for meta-programming
//!
//! # Architecture
//!
//! ```text
//! verum_common::ConstValue (base type)
//!   - Unit, Bool, Int, UInt, Float, Char, Text, Bytes, Array, Tuple, Maybe
//!   - Used by: verum_types, verum_protocol_types, and as base for verum_compiler
//!
//! verum_compiler::ConstValue (extended type)
//!   - Includes all base variants plus AST variants (Expr, Type, Pattern, Item)
//!   - Used for meta-programming and compile-time code generation
//! ```
//!
//! # Usage
//!
//! ```rust
//! use verum_common::{ConstValue, List, Text};
//!
//! let int_val = ConstValue::Int(42);
//! let text_val = ConstValue::Text(Text::from("hello"));
//! let array_val = ConstValue::Array(List::from_iter([
//!     ConstValue::Int(1),
//!     ConstValue::Int(2),
//! ]));
//! ```

use crate::{Heap, List, Maybe, Text};
use std::fmt;

/// A compile-time constant value (base type)
///
/// This is the unified representation of constant values used throughout
/// the Verum compiler and type system. All compile-time evaluation,
/// constant expression handling, and basic meta-programming use this type.
///
/// # Precision
///
/// Integer variants use `i128`/`u128` for maximum precision. This ensures
/// that compile-time arithmetic doesn't lose precision compared to runtime.
///
/// # AST Variants
///
/// AST-related variants (`Expr`, `Type`, `Pattern`, `Item`) are provided
/// by `verum_compiler::ConstValue` which extends this base type for
/// meta-programming use cases. This separation respects the layer architecture
/// where verum_common cannot depend on verum_ast.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ConstValue {
    /// Unit/void value (the empty tuple)
    #[default]
    Unit,

    /// Boolean value
    Bool(bool),

    /// Signed integer with maximum precision (i128)
    ///
    /// This can represent any integer from -2^127 to 2^127-1.
    /// Narrowing conversions should be explicit.
    Int(i128),

    /// Unsigned integer with maximum precision (u128)
    ///
    /// This can represent any integer from 0 to 2^128-1.
    UInt(u128),

    /// IEEE 754 double-precision floating-point
    Float(f64),

    /// Unicode character
    Char(char),

    /// Text/string value
    Text(Text),

    /// Byte sequence (e.g., b"hello")
    Bytes(Vec<u8>),

    /// Homogeneous array of constant values
    Array(List<ConstValue>),

    /// Heterogeneous tuple of constant values
    Tuple(List<ConstValue>),

    /// Optional value (Maybe<T> in Verum)
    Maybe(Maybe<Heap<ConstValue>>),

    /// Key-value map (Map<Text, T> in Verum)
    ///
    /// Uses Text as keys since this is the most common case in meta-programming.
    /// For more complex key types, use Array of tuples.
    Map(crate::OrderedMap<Text, ConstValue>),

    /// Set of text values (Set<Text> in Verum)
    ///
    /// Uses Text for element type since this is the most common case in meta-programming.
    /// For sets of other types, use Array with deduplication.
    Set(crate::OrderedSet<Text>),
}

// =============================================================================
// Constructors
// =============================================================================

impl ConstValue {
    /// Create a unit value
    #[inline]
    pub const fn unit() -> Self {
        ConstValue::Unit
    }

    /// Create a boolean value
    #[inline]
    pub const fn bool(b: bool) -> Self {
        ConstValue::Bool(b)
    }

    /// Create a signed integer value
    #[inline]
    pub const fn int(n: i128) -> Self {
        ConstValue::Int(n)
    }

    /// Create an unsigned integer value
    #[inline]
    pub const fn uint(n: u128) -> Self {
        ConstValue::UInt(n)
    }

    /// Create a floating-point value
    #[inline]
    pub const fn float(f: f64) -> Self {
        ConstValue::Float(f)
    }

    /// Create a character value
    #[inline]
    pub const fn char(c: char) -> Self {
        ConstValue::Char(c)
    }

    /// Create a text value
    #[inline]
    pub fn text(s: impl Into<Text>) -> Self {
        ConstValue::Text(s.into())
    }

    /// Create a bytes value
    #[inline]
    pub fn bytes(b: impl Into<Vec<u8>>) -> Self {
        ConstValue::Bytes(b.into())
    }

    /// Create an array value
    #[inline]
    pub fn array(values: impl IntoIterator<Item = ConstValue>) -> Self {
        ConstValue::Array(List::from_iter(values))
    }

    /// Create a tuple value
    #[inline]
    pub fn tuple(values: impl IntoIterator<Item = ConstValue>) -> Self {
        ConstValue::Tuple(List::from_iter(values))
    }

    /// Create a Some value
    #[inline]
    pub fn some(value: ConstValue) -> Self {
        ConstValue::Maybe(Some(Heap::new(value)))
    }

    /// Create a None value
    #[inline]
    pub fn none() -> Self {
        ConstValue::Maybe(None)
    }
}

// =============================================================================
// Type Predicates
// =============================================================================

impl ConstValue {
    /// Check if this is a unit value
    #[inline]
    pub fn is_unit(&self) -> bool {
        matches!(self, ConstValue::Unit)
    }

    /// Check if this is a boolean value
    #[inline]
    pub fn is_bool(&self) -> bool {
        matches!(self, ConstValue::Bool(_))
    }

    /// Check if this is an integer (signed or unsigned)
    #[inline]
    pub fn is_integer(&self) -> bool {
        matches!(self, ConstValue::Int(_) | ConstValue::UInt(_))
    }

    /// Check if this is a signed integer
    #[inline]
    pub fn is_int(&self) -> bool {
        matches!(self, ConstValue::Int(_))
    }

    /// Check if this is an unsigned integer
    #[inline]
    pub fn is_uint(&self) -> bool {
        matches!(self, ConstValue::UInt(_))
    }

    /// Check if this is a floating-point value
    #[inline]
    pub fn is_float(&self) -> bool {
        matches!(self, ConstValue::Float(_))
    }

    /// Check if this is a numeric value (int, uint, or float)
    #[inline]
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            ConstValue::Int(_) | ConstValue::UInt(_) | ConstValue::Float(_)
        )
    }

    /// Check if this is a character value
    #[inline]
    pub fn is_char(&self) -> bool {
        matches!(self, ConstValue::Char(_))
    }

    /// Check if this is a text value
    #[inline]
    pub fn is_text(&self) -> bool {
        matches!(self, ConstValue::Text(_))
    }

    /// Check if this is a bytes value
    #[inline]
    pub fn is_bytes(&self) -> bool {
        matches!(self, ConstValue::Bytes(_))
    }

    /// Check if this is an array value
    #[inline]
    pub fn is_array(&self) -> bool {
        matches!(self, ConstValue::Array(_))
    }

    /// Check if this is a tuple value
    #[inline]
    pub fn is_tuple(&self) -> bool {
        matches!(self, ConstValue::Tuple(_))
    }

    /// Check if this is a maybe value
    #[inline]
    pub fn is_maybe(&self) -> bool {
        matches!(self, ConstValue::Maybe(_))
    }

    /// Check if this is Some (for Maybe values)
    #[inline]
    pub fn is_some(&self) -> bool {
        matches!(self, ConstValue::Maybe(Some(_)))
    }

    /// Check if this is None (for Maybe values)
    #[inline]
    pub fn is_none(&self) -> bool {
        matches!(self, ConstValue::Maybe(None))
    }

    /// Check if this is a map value
    #[inline]
    pub fn is_map(&self) -> bool {
        matches!(self, ConstValue::Map(_))
    }

    /// Check if this is a set value
    #[inline]
    pub fn is_set(&self) -> bool {
        matches!(self, ConstValue::Set(_))
    }
}

// =============================================================================
// Extractors
// =============================================================================

impl ConstValue {
    /// Extract boolean value
    #[inline]
    pub fn as_bool_value(&self) -> Option<bool> {
        match self {
            ConstValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Extract as i128 if possible (with widening from unsigned)
    pub fn as_i128(&self) -> Option<i128> {
        match self {
            ConstValue::Int(n) => Some(*n),
            ConstValue::UInt(n) => (*n).try_into().ok(),
            _ => None,
        }
    }

    /// Extract as u128 if possible (with widening from signed if positive)
    pub fn as_u128(&self) -> Option<u128> {
        match self {
            ConstValue::UInt(n) => Some(*n),
            ConstValue::Int(n) => (*n).try_into().ok(),
            _ => None,
        }
    }

    /// Extract as i64 (narrowing conversion)
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            ConstValue::Int(n) => (*n).try_into().ok(),
            ConstValue::UInt(n) => (*n).try_into().ok(),
            _ => None,
        }
    }

    /// Extract as u64 (narrowing conversion)
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            ConstValue::UInt(n) => (*n).try_into().ok(),
            ConstValue::Int(n) => (*n).try_into().ok(),
            _ => None,
        }
    }

    /// Extract as usize (narrowing conversion)
    pub fn as_usize(&self) -> Option<usize> {
        match self {
            ConstValue::UInt(n) => (*n).try_into().ok(),
            ConstValue::Int(n) => (*n).try_into().ok(),
            _ => None,
        }
    }

    /// Extract as f64
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ConstValue::Float(f) => Some(*f),
            ConstValue::Int(n) => Some(*n as f64),
            ConstValue::UInt(n) => Some(*n as f64),
            _ => None,
        }
    }

    /// Extract character value
    #[inline]
    pub fn as_char_value(&self) -> Option<char> {
        match self {
            ConstValue::Char(c) => Some(*c),
            _ => None,
        }
    }

    /// Extract text reference
    #[inline]
    pub fn as_text(&self) -> Option<&Text> {
        match self {
            ConstValue::Text(s) => Some(s),
            _ => None,
        }
    }

    /// Extract bytes reference
    #[inline]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            ConstValue::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Extract array reference
    #[inline]
    pub fn as_array(&self) -> Option<&List<ConstValue>> {
        match self {
            ConstValue::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Extract tuple reference
    #[inline]
    pub fn as_tuple(&self) -> Option<&List<ConstValue>> {
        match self {
            ConstValue::Tuple(tup) => Some(tup),
            _ => None,
        }
    }

    /// Extract inner value from Maybe
    #[inline]
    pub fn as_maybe(&self) -> Option<&Maybe<Heap<ConstValue>>> {
        match self {
            ConstValue::Maybe(m) => Some(m),
            _ => None,
        }
    }

    /// Unwrap the inner value from a Some, returns None if None or not a Maybe
    #[inline]
    pub fn unwrap_maybe(&self) -> Option<&ConstValue> {
        match self {
            ConstValue::Maybe(Some(inner)) => Some(inner.as_ref()),
            _ => None,
        }
    }

    /// Extract map reference
    #[inline]
    pub fn as_map(&self) -> Option<&crate::OrderedMap<Text, ConstValue>> {
        match self {
            ConstValue::Map(m) => Some(m),
            _ => None,
        }
    }

    /// Extract set reference
    #[inline]
    pub fn as_set(&self) -> Option<&crate::OrderedSet<Text>> {
        match self {
            ConstValue::Set(s) => Some(s),
            _ => None,
        }
    }

    /// Get the array/tuple/map/set length if applicable
    pub fn len(&self) -> Option<usize> {
        match self {
            ConstValue::Array(arr) => Some(arr.len()),
            ConstValue::Tuple(tup) => Some(tup.len()),
            ConstValue::Text(s) => Some(s.len()),
            ConstValue::Bytes(b) => Some(b.len()),
            ConstValue::Map(m) => Some(m.len()),
            ConstValue::Set(s) => Some(s.len()),
            _ => None,
        }
    }

    /// Check if the collection is empty (for Array, Tuple, Text, Bytes)
    pub fn is_empty(&self) -> Option<bool> {
        self.len().map(|l| l == 0)
    }
}

// =============================================================================
// Truthiness (for conditionals)
// =============================================================================

impl ConstValue {
    /// Convert to boolean for conditional evaluation
    ///
    /// This follows Verum's truthiness rules:
    /// - `Unit` is false
    /// - `Bool(false)` is false
    /// - `Int(0)` and `UInt(0)` are false
    /// - `Float(0.0)` is false
    /// - Empty text, array, tuple, bytes are false
    /// - `Maybe(None)` is false
    /// - Everything else is true
    pub fn is_truthy(&self) -> bool {
        match self {
            ConstValue::Unit => false,
            ConstValue::Bool(b) => *b,
            ConstValue::Int(n) => *n != 0,
            ConstValue::UInt(n) => *n != 0,
            ConstValue::Float(f) => *f != 0.0,
            ConstValue::Char(_) => true,
            ConstValue::Text(s) => !s.is_empty(),
            ConstValue::Bytes(b) => !b.is_empty(),
            ConstValue::Array(arr) => !arr.is_empty(),
            ConstValue::Tuple(tup) => !tup.is_empty(),
            ConstValue::Maybe(opt) => opt.is_some(),
            ConstValue::Map(m) => !m.is_empty(),
            ConstValue::Set(s) => !s.is_empty(),
        }
    }
}

// =============================================================================
// Type Name
// =============================================================================

impl ConstValue {
    /// Get the type name for error messages and debugging
    pub fn type_name(&self) -> &'static str {
        match self {
            ConstValue::Unit => "Unit",
            ConstValue::Bool(_) => "Bool",
            ConstValue::Int(_) => "Int",
            ConstValue::UInt(_) => "UInt",
            ConstValue::Float(_) => "Float",
            ConstValue::Char(_) => "Char",
            ConstValue::Text(_) => "Text",
            ConstValue::Bytes(_) => "Bytes",
            ConstValue::Array(_) => "Array",
            ConstValue::Tuple(_) => "Tuple",
            ConstValue::Maybe(_) => "Maybe",
            ConstValue::Map(_) => "Map",
            ConstValue::Set(_) => "Set",
        }
    }
}

// =============================================================================
// Display
// =============================================================================

impl fmt::Display for ConstValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConstValue::Unit => write!(f, "()"),
            ConstValue::Bool(b) => write!(f, "{}", b),
            ConstValue::Int(n) => write!(f, "{}", n),
            ConstValue::UInt(n) => write!(f, "{}u", n),
            ConstValue::Float(fl) => {
                // Ensure floats always have decimal point
                if fl.fract() == 0.0 {
                    write!(f, "{}.0", fl)
                } else {
                    write!(f, "{}", fl)
                }
            }
            ConstValue::Char(c) => write!(f, "'{}'", c.escape_default()),
            ConstValue::Text(s) => write!(f, "\"{}\"", s.escape_default()),
            ConstValue::Bytes(bytes) => {
                write!(f, "b\"")?;
                for b in bytes {
                    if b.is_ascii_graphic() || *b == b' ' {
                        write!(f, "{}", *b as char)?;
                    } else {
                        write!(f, "\\x{:02x}", b)?;
                    }
                }
                write!(f, "\"")
            }
            ConstValue::Array(arr) => {
                write!(f, "[")?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            ConstValue::Tuple(tup) => {
                write!(f, "(")?;
                for (i, v) in tup.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                if tup.len() == 1 {
                    write!(f, ",")?; // Single-element tuple needs trailing comma
                }
                write!(f, ")")
            }
            ConstValue::Maybe(None) => write!(f, "None"),
            ConstValue::Maybe(Some(inner)) => write!(f, "Some({})", inner),
            ConstValue::Map(map) => {
                write!(f, "{{")?;
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{}\": {}", k, v)?;
                }
                write!(f, "}}")
            }
            ConstValue::Set(set) => {
                write!(f, "Set{{")?;
                for (i, v) in set.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{}\"", v)?;
                }
                write!(f, "}}")
            }
        }
    }
}

// =============================================================================
// From Implementations
// =============================================================================

impl From<bool> for ConstValue {
    fn from(b: bool) -> Self {
        ConstValue::Bool(b)
    }
}

impl From<i8> for ConstValue {
    fn from(n: i8) -> Self {
        ConstValue::Int(n as i128)
    }
}

impl From<i16> for ConstValue {
    fn from(n: i16) -> Self {
        ConstValue::Int(n as i128)
    }
}

impl From<i32> for ConstValue {
    fn from(n: i32) -> Self {
        ConstValue::Int(n as i128)
    }
}

impl From<i64> for ConstValue {
    fn from(n: i64) -> Self {
        ConstValue::Int(n as i128)
    }
}

impl From<i128> for ConstValue {
    fn from(n: i128) -> Self {
        ConstValue::Int(n)
    }
}

impl From<isize> for ConstValue {
    fn from(n: isize) -> Self {
        ConstValue::Int(n as i128)
    }
}

impl From<u8> for ConstValue {
    fn from(n: u8) -> Self {
        ConstValue::UInt(n as u128)
    }
}

impl From<u16> for ConstValue {
    fn from(n: u16) -> Self {
        ConstValue::UInt(n as u128)
    }
}

impl From<u32> for ConstValue {
    fn from(n: u32) -> Self {
        ConstValue::UInt(n as u128)
    }
}

impl From<u64> for ConstValue {
    fn from(n: u64) -> Self {
        ConstValue::UInt(n as u128)
    }
}

impl From<u128> for ConstValue {
    fn from(n: u128) -> Self {
        ConstValue::UInt(n)
    }
}

impl From<usize> for ConstValue {
    fn from(n: usize) -> Self {
        ConstValue::UInt(n as u128)
    }
}

impl From<f32> for ConstValue {
    fn from(f: f32) -> Self {
        ConstValue::Float(f as f64)
    }
}

impl From<f64> for ConstValue {
    fn from(f: f64) -> Self {
        ConstValue::Float(f)
    }
}

impl From<char> for ConstValue {
    fn from(c: char) -> Self {
        ConstValue::Char(c)
    }
}

impl From<&str> for ConstValue {
    fn from(s: &str) -> Self {
        ConstValue::Text(Text::from(s))
    }
}

impl From<String> for ConstValue {
    fn from(s: String) -> Self {
        ConstValue::Text(Text::from(s))
    }
}

impl From<Text> for ConstValue {
    fn from(s: Text) -> Self {
        ConstValue::Text(s)
    }
}

impl<T: Into<ConstValue>> From<Vec<T>> for ConstValue {
    fn from(v: Vec<T>) -> Self {
        ConstValue::Array(List::from_iter(v.into_iter().map(Into::into)))
    }
}

impl<T: Into<ConstValue>> From<Option<T>> for ConstValue {
    fn from(opt: Option<T>) -> Self {
        match opt {
            Some(v) => ConstValue::some(v.into()),
            None => ConstValue::none(),
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constructors() {
        assert_eq!(ConstValue::unit(), ConstValue::Unit);
        assert_eq!(ConstValue::bool(true), ConstValue::Bool(true));
        assert_eq!(ConstValue::int(42), ConstValue::Int(42));
        assert_eq!(ConstValue::uint(42), ConstValue::UInt(42));
        assert_eq!(ConstValue::float(2.5), ConstValue::Float(2.5));
        assert_eq!(ConstValue::char('x'), ConstValue::Char('x'));
    }

    #[test]
    fn test_type_predicates() {
        assert!(ConstValue::Unit.is_unit());
        assert!(ConstValue::Bool(true).is_bool());
        assert!(ConstValue::Int(42).is_int());
        assert!(ConstValue::UInt(42).is_uint());
        assert!(ConstValue::Int(42).is_integer());
        assert!(ConstValue::UInt(42).is_integer());
        assert!(ConstValue::Float(2.5).is_float());
        assert!(ConstValue::Int(42).is_numeric());
        assert!(ConstValue::Char('x').is_char());
    }

    #[test]
    fn test_extractors() {
        assert_eq!(ConstValue::Bool(true).as_bool_value(), Some(true));
        assert_eq!(ConstValue::Int(42).as_i128(), Some(42));
        assert_eq!(ConstValue::UInt(42).as_u128(), Some(42));
        assert_eq!(ConstValue::Int(42).as_i64(), Some(42));
        assert_eq!(ConstValue::Float(2.5).as_f64(), Some(2.5));
        assert_eq!(ConstValue::Char('x').as_char_value(), Some('x'));
    }

    #[test]
    fn test_integer_widening() {
        // Signed to unsigned (positive)
        assert_eq!(ConstValue::Int(100).as_u128(), Some(100));
        // Unsigned to signed (within range)
        assert_eq!(ConstValue::UInt(100).as_i128(), Some(100));
        // Signed negative to unsigned fails
        assert_eq!(ConstValue::Int(-1).as_u128(), None);
        // Large unsigned to signed fails
        assert_eq!(ConstValue::UInt(u128::MAX).as_i128(), None);
    }

    #[test]
    fn test_truthiness() {
        assert!(!ConstValue::Unit.is_truthy());
        assert!(!ConstValue::Bool(false).is_truthy());
        assert!(ConstValue::Bool(true).is_truthy());
        assert!(!ConstValue::Int(0).is_truthy());
        assert!(ConstValue::Int(1).is_truthy());
        assert!(ConstValue::Int(-1).is_truthy());
        assert!(!ConstValue::UInt(0).is_truthy());
        assert!(ConstValue::UInt(1).is_truthy());
        assert!(!ConstValue::Float(0.0).is_truthy());
        assert!(ConstValue::Float(0.1).is_truthy());
        assert!(!ConstValue::text("").is_truthy());
        assert!(ConstValue::text("hello").is_truthy());
        assert!(!ConstValue::none().is_truthy());
        assert!(ConstValue::some(ConstValue::int(42)).is_truthy());
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", ConstValue::Unit), "()");
        assert_eq!(format!("{}", ConstValue::Bool(true)), "true");
        assert_eq!(format!("{}", ConstValue::Int(42)), "42");
        assert_eq!(format!("{}", ConstValue::UInt(42)), "42u");
        assert_eq!(format!("{}", ConstValue::Float(3.0)), "3.0");
        assert_eq!(format!("{}", ConstValue::Float(2.5)), "2.5");
        assert_eq!(format!("{}", ConstValue::Char('x')), "'x'");
        assert_eq!(format!("{}", ConstValue::text("hello")), "\"hello\"");
        assert_eq!(
            format!(
                "{}",
                ConstValue::array([ConstValue::int(1), ConstValue::int(2)])
            ),
            "[1, 2]"
        );
        assert_eq!(format!("{}", ConstValue::none()), "None");
        assert_eq!(
            format!("{}", ConstValue::some(ConstValue::int(42))),
            "Some(42)"
        );
    }

    #[test]
    fn test_from_impls() {
        assert_eq!(ConstValue::from(true), ConstValue::Bool(true));
        assert_eq!(ConstValue::from(42i32), ConstValue::Int(42));
        assert_eq!(ConstValue::from(42u32), ConstValue::UInt(42));
        assert_eq!(ConstValue::from(2.5f64), ConstValue::Float(2.5));
        assert_eq!(ConstValue::from('x'), ConstValue::Char('x'));
        assert_eq!(ConstValue::from("hello"), ConstValue::text("hello"));
    }

    #[test]
    fn test_type_name() {
        assert_eq!(ConstValue::Unit.type_name(), "Unit");
        assert_eq!(ConstValue::Bool(true).type_name(), "Bool");
        assert_eq!(ConstValue::Int(42).type_name(), "Int");
        assert_eq!(ConstValue::UInt(42).type_name(), "UInt");
        assert_eq!(ConstValue::Float(2.5).type_name(), "Float");
        assert_eq!(ConstValue::Char('x').type_name(), "Char");
        assert_eq!(ConstValue::text("hello").type_name(), "Text");
        assert_eq!(ConstValue::array([]).type_name(), "Array");
        assert_eq!(ConstValue::tuple([]).type_name(), "Tuple");
        assert_eq!(ConstValue::none().type_name(), "Maybe");
    }

    #[test]
    fn test_len() {
        assert_eq!(
            ConstValue::array([ConstValue::int(1), ConstValue::int(2)]).len(),
            Some(2)
        );
        assert_eq!(ConstValue::tuple([ConstValue::int(1)]).len(), Some(1));
        assert_eq!(ConstValue::text("hello").len(), Some(5));
        assert_eq!(ConstValue::bytes(vec![1, 2, 3]).len(), Some(3));
        assert_eq!(ConstValue::Int(42).len(), None);
    }

    #[test]
    fn test_maybe() {
        let some_val = ConstValue::some(ConstValue::int(42));
        let none_val = ConstValue::none();

        assert!(some_val.is_some());
        assert!(!some_val.is_none());
        assert!(!none_val.is_some());
        assert!(none_val.is_none());

        assert_eq!(some_val.unwrap_maybe(), Some(&ConstValue::Int(42)));
        assert_eq!(none_val.unwrap_maybe(), None);
    }

    #[test]
    fn test_max_precision() {
        // Test that i128 range works
        let max_i128 = ConstValue::Int(i128::MAX);
        let min_i128 = ConstValue::Int(i128::MIN);
        assert_eq!(max_i128.as_i128(), Some(i128::MAX));
        assert_eq!(min_i128.as_i128(), Some(i128::MIN));

        // Test that u128 range works
        let max_u128 = ConstValue::UInt(u128::MAX);
        assert_eq!(max_u128.as_u128(), Some(u128::MAX));

        // Narrowing should fail for out-of-range values
        assert_eq!(max_i128.as_i64(), None);
        assert_eq!(max_u128.as_u64(), None);
    }
}
