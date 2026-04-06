//! Meta Value - AST-aware compile-time value representation
//!
//! This module provides `MetaValue`, an extended constant value type for
//! compile-time meta-programming. It extends `verum_common::ConstValue`
//! semantics with AST node variants for code generation.
//!
//! # Architecture
//!
//! ```text
//! verum_common::ConstValue     verum_ast::MetaValue
//! ─────────────────────────    ────────────────────────────────
//! Unit                         Unit
//! Bool(bool)                   Bool(bool)
//! Int(i128)                    Int(i128)
//! UInt(u128)                   UInt(u128)
//! Float(f64)                   Float(f64)
//! Char(char)                   Char(char)
//! Text(Text)                   Text(Text)
//! Bytes(Vec<u8>)               Bytes(Vec<u8>)
//! Array(List<ConstValue>)      Array(List<MetaValue>)  ← can contain AST
//! Tuple(List<ConstValue>)      Tuple(List<MetaValue>)  ← can contain AST
//! Maybe(Maybe<Box<CV>>)        Maybe(Maybe<Box<MV>>)   ← can contain AST
//! Map(OrderedMap<Text, CV>)    Map(OrderedMap<Text, MV>) ← can contain AST
//! Set(OrderedSet<Text>)        Set(OrderedSet<Text>)
//!                              Expr(Expr)              ← AST only
//!                              Type(Type)              ← AST only
//!                              Pattern(Pattern)        ← AST only
//!                              Item(Item)              ← AST only
//! ```
//!
//! # Design Principles
//!
//! 1. **Zero-cost abstraction**: Flat enum, no wrapper indirection
//! 2. **Bidirectional conversion**: `From<ConstValue>` and `TryFrom<MetaValue>`
//! 3. **Mixed collections**: Arrays/tuples can contain both primitives and AST
//! 4. **Maximum precision**: i128/u128 for integers (from ConstValue)
//!
//! # Meta System Integration
//!
//! MetaValue is the runtime representation for the unified meta-system, which is the ONLY
//! compile-time computation mechanism in Verum (no separate const fn or const generics).
//! All compile-time constructs use `@` prefix: `@derive(...)`, `@const`, `@cfg`, etc.
//! MetaValue supports code generation by carrying AST nodes (Expr, Type, Pattern, Item)
//! alongside primitive values, enabling procedural macros and tagged literal handlers
//! to produce and manipulate code at compile time.

use std::fmt;

use verum_common::{ConstValue, Heap, List, Maybe, OrderedMap, OrderedSet, Text};

use crate::expr::Expr;
use crate::literal::{Literal, LiteralKind};
use crate::pattern::Pattern;
use crate::ty::Type;
use crate::Item;

// ============================================================================
// MetaValue Enum
// ============================================================================

/// Compile-time value for meta-programming, extending ConstValue with AST nodes.
///
/// This is the primary value type during meta-function execution. Unlike
/// `ConstValue` which only holds primitives, `MetaValue` can also hold
/// AST nodes for code generation.
///
/// # Performance
///
/// Flat enum layout ensures single-level matching with no indirection.
/// All primitive operations are O(1).
#[derive(Debug, Clone, PartialEq)]
pub enum MetaValue {
    // ─────────────────────────────────────────────────────────────────────
    // Primitive variants (matching ConstValue)
    // ─────────────────────────────────────────────────────────────────────

    /// Unit/void value
    Unit,

    /// Boolean value
    Bool(bool),

    /// Signed integer with maximum precision (i128)
    Int(i128),

    /// Unsigned integer with maximum precision (u128)
    UInt(u128),

    /// IEEE 754 double-precision floating-point
    Float(f64),

    /// Unicode scalar value
    Char(char),

    /// Text/string value (semantic type from verum_common)
    Text(Text),

    /// Raw byte sequence
    Bytes(Vec<u8>),

    /// Homogeneous array (can contain MetaValues including AST)
    Array(List<MetaValue>),

    /// Heterogeneous tuple (can contain MetaValues including AST)
    Tuple(List<MetaValue>),

    /// Optional value (can contain MetaValue including AST)
    Maybe(Maybe<Heap<MetaValue>>),

    /// Key-value map with Text keys (can contain MetaValue including AST)
    Map(OrderedMap<Text, MetaValue>),

    /// Set of Text values
    Set(OrderedSet<Text>),

    // ─────────────────────────────────────────────────────────────────────
    // AST variants (meta-programming only)
    // ─────────────────────────────────────────────────────────────────────

    /// AST expression for code generation
    Expr(Expr),

    /// AST type for type construction
    Type(Type),

    /// AST pattern for pattern construction
    Pattern(Pattern),

    /// AST item (single declaration)
    Item(Item),

    /// List of items (for generating multiple declarations)
    ///
    /// Meta-functions that generate multiple declarations return this variant.
    Items(List<MetaValue>),
}

// ============================================================================
// Constructors
// ============================================================================

impl MetaValue {
    /// Create unit value.
    #[inline]
    pub const fn unit() -> Self {
        Self::Unit
    }

    /// Create boolean value.
    #[inline]
    pub const fn bool(v: bool) -> Self {
        Self::Bool(v)
    }

    /// Create signed integer value.
    #[inline]
    pub const fn int(v: i128) -> Self {
        Self::Int(v)
    }

    /// Create unsigned integer value.
    #[inline]
    pub const fn uint(v: u128) -> Self {
        Self::UInt(v)
    }

    /// Create floating-point value.
    #[inline]
    pub const fn float(v: f64) -> Self {
        Self::Float(v)
    }

    /// Create character value.
    #[inline]
    pub const fn char(v: char) -> Self {
        Self::Char(v)
    }

    /// Create text value.
    #[inline]
    pub fn text(v: impl Into<Text>) -> Self {
        Self::Text(v.into())
    }

    /// Create bytes value.
    #[inline]
    pub fn bytes(v: Vec<u8>) -> Self {
        Self::Bytes(v)
    }

    /// Create array value.
    #[inline]
    pub fn array(v: impl Into<List<MetaValue>>) -> Self {
        Self::Array(v.into())
    }

    /// Create tuple value.
    #[inline]
    pub fn tuple(v: impl Into<List<MetaValue>>) -> Self {
        Self::Tuple(v.into())
    }

    /// Create None value.
    #[inline]
    pub const fn none() -> Self {
        Self::Maybe(Maybe::None)
    }

    /// Create Some value.
    #[inline]
    pub fn some(v: MetaValue) -> Self {
        Self::Maybe(Maybe::Some(Heap::new(v)))
    }

    /// Create empty map value.
    #[inline]
    pub fn map(v: impl Into<OrderedMap<Text, MetaValue>>) -> Self {
        Self::Map(v.into())
    }

    /// Create empty set value.
    #[inline]
    pub fn set(v: impl Into<OrderedSet<Text>>) -> Self {
        Self::Set(v.into())
    }

    /// Create expression value.
    #[inline]
    pub fn expr(v: Expr) -> Self {
        Self::Expr(v)
    }

    /// Create type value.
    #[inline]
    pub fn ty(v: Type) -> Self {
        Self::Type(v)
    }

    /// Create pattern value.
    #[inline]
    pub fn pattern(v: Pattern) -> Self {
        Self::Pattern(v)
    }

    /// Create item value.
    #[inline]
    pub fn item(v: Item) -> Self {
        Self::Item(v)
    }
}

// ============================================================================
// Type Predicates
// ============================================================================

impl MetaValue {
    #[inline]
    pub const fn is_unit(&self) -> bool {
        matches!(self, Self::Unit)
    }

    #[inline]
    pub const fn is_bool(&self) -> bool {
        matches!(self, Self::Bool(_))
    }

    #[inline]
    pub const fn is_int(&self) -> bool {
        matches!(self, Self::Int(_))
    }

    #[inline]
    pub const fn is_uint(&self) -> bool {
        matches!(self, Self::UInt(_))
    }

    #[inline]
    pub const fn is_integer(&self) -> bool {
        matches!(self, Self::Int(_) | Self::UInt(_))
    }

    #[inline]
    pub const fn is_float(&self) -> bool {
        matches!(self, Self::Float(_))
    }

    #[inline]
    pub const fn is_numeric(&self) -> bool {
        matches!(self, Self::Int(_) | Self::UInt(_) | Self::Float(_))
    }

    #[inline]
    pub const fn is_char(&self) -> bool {
        matches!(self, Self::Char(_))
    }

    #[inline]
    pub const fn is_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }

    #[inline]
    pub const fn is_bytes(&self) -> bool {
        matches!(self, Self::Bytes(_))
    }

    #[inline]
    pub const fn is_array(&self) -> bool {
        matches!(self, Self::Array(_))
    }

    #[inline]
    pub const fn is_tuple(&self) -> bool {
        matches!(self, Self::Tuple(_))
    }

    #[inline]
    pub const fn is_maybe(&self) -> bool {
        matches!(self, Self::Maybe(_))
    }

    #[inline]
    pub const fn is_map(&self) -> bool {
        matches!(self, Self::Map(_))
    }

    #[inline]
    pub const fn is_set(&self) -> bool {
        matches!(self, Self::Set(_))
    }

    #[inline]
    pub const fn is_expr(&self) -> bool {
        matches!(self, Self::Expr(_))
    }

    #[inline]
    pub const fn is_type(&self) -> bool {
        matches!(self, Self::Type(_))
    }

    #[inline]
    pub const fn is_pattern(&self) -> bool {
        matches!(self, Self::Pattern(_))
    }

    #[inline]
    pub const fn is_item(&self) -> bool {
        matches!(self, Self::Item(_))
    }

    /// Check if this is an items list.
    #[inline]
    pub const fn is_items(&self) -> bool {
        matches!(self, Self::Items(_))
    }

    /// Check if this is an AST node (non-primitive).
    #[inline]
    pub const fn is_ast(&self) -> bool {
        matches!(
            self,
            Self::Expr(_) | Self::Type(_) | Self::Pattern(_) | Self::Item(_) | Self::Items(_)
        )
    }

    /// Check if this is a primitive value (convertible to ConstValue).
    #[inline]
    pub const fn is_primitive(&self) -> bool {
        !self.is_ast()
    }
}

// ============================================================================
// Value Extractors
// ============================================================================

impl MetaValue {
    #[inline]
    pub const fn as_bool_value(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Get signed integer as i64 (truncates large values).
    #[inline]
    pub const fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(n) => Some(*n as i64),
            Self::UInt(n) if *n <= i64::MAX as u128 => Some(*n as i64),
            _ => None,
        }
    }

    /// Get signed integer as i128 (full precision).
    #[inline]
    pub const fn as_i128(&self) -> Option<i128> {
        match self {
            Self::Int(n) => Some(*n),
            Self::UInt(n) if *n <= i128::MAX as u128 => Some(*n as i128),
            _ => None,
        }
    }

    /// Get unsigned integer as u64 (truncates large values).
    #[inline]
    pub const fn as_u64(&self) -> Option<u64> {
        match self {
            Self::UInt(n) => Some(*n as u64),
            Self::Int(n) if *n >= 0 => Some(*n as u64),
            _ => None,
        }
    }

    /// Get unsigned integer as u128 (full precision).
    #[inline]
    pub const fn as_u128(&self) -> Option<u128> {
        match self {
            Self::UInt(n) => Some(*n),
            Self::Int(n) if *n >= 0 => Some(*n as u128),
            _ => None,
        }
    }

    #[inline]
    pub const fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(n) => Some(*n as f64),
            Self::UInt(n) => Some(*n as f64),
            _ => None,
        }
    }

    #[inline]
    pub const fn as_char_value(&self) -> Option<char> {
        match self {
            Self::Char(c) => Some(*c),
            _ => None,
        }
    }

    #[inline]
    pub fn as_text(&self) -> Option<&Text> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }

    #[inline]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }

    #[inline]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(b) => Some(b),
            _ => None,
        }
    }

    #[inline]
    pub fn as_array(&self) -> Option<&List<MetaValue>> {
        match self {
            Self::Array(arr) => Some(arr),
            _ => None,
        }
    }

    #[inline]
    pub fn as_tuple(&self) -> Option<&List<MetaValue>> {
        match self {
            Self::Tuple(tup) => Some(tup),
            _ => None,
        }
    }

    #[inline]
    pub fn as_maybe(&self) -> Option<&Maybe<Heap<MetaValue>>> {
        match self {
            Self::Maybe(m) => Some(m),
            _ => None,
        }
    }

    #[inline]
    pub fn as_map(&self) -> Option<&OrderedMap<Text, MetaValue>> {
        match self {
            Self::Map(m) => Some(m),
            _ => None,
        }
    }

    #[inline]
    pub fn as_set(&self) -> Option<&OrderedSet<Text>> {
        match self {
            Self::Set(s) => Some(s),
            _ => None,
        }
    }

    #[inline]
    pub fn as_expr(&self) -> Option<&Expr> {
        match self {
            Self::Expr(e) => Some(e),
            _ => None,
        }
    }

    #[inline]
    pub fn as_type(&self) -> Option<&Type> {
        match self {
            Self::Type(t) => Some(t),
            _ => None,
        }
    }

    #[inline]
    pub fn as_pattern(&self) -> Option<&Pattern> {
        match self {
            Self::Pattern(p) => Some(p),
            _ => None,
        }
    }

    #[inline]
    pub fn as_item(&self) -> Option<&Item> {
        match self {
            Self::Item(i) => Some(i),
            _ => None,
        }
    }

    #[inline]
    pub fn as_items(&self) -> Option<&List<MetaValue>> {
        match self {
            Self::Items(items) => Some(items),
            _ => None,
        }
    }

    // --- Consuming extractors ---

    #[inline]
    pub fn into_text(self) -> Option<Text> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }

    #[inline]
    pub fn into_bytes(self) -> Option<Vec<u8>> {
        match self {
            Self::Bytes(b) => Some(b),
            _ => None,
        }
    }

    #[inline]
    pub fn into_array(self) -> Option<List<MetaValue>> {
        match self {
            Self::Array(arr) => Some(arr),
            _ => None,
        }
    }

    #[inline]
    pub fn into_tuple(self) -> Option<List<MetaValue>> {
        match self {
            Self::Tuple(tup) => Some(tup),
            _ => None,
        }
    }

    #[inline]
    pub fn into_map(self) -> Option<OrderedMap<Text, MetaValue>> {
        match self {
            Self::Map(m) => Some(m),
            _ => None,
        }
    }

    #[inline]
    pub fn into_set(self) -> Option<OrderedSet<Text>> {
        match self {
            Self::Set(s) => Some(s),
            _ => None,
        }
    }

    #[inline]
    pub fn into_expr(self) -> Option<Expr> {
        match self {
            Self::Expr(e) => Some(e),
            _ => None,
        }
    }

    #[inline]
    pub fn into_type(self) -> Option<Type> {
        match self {
            Self::Type(t) => Some(t),
            _ => None,
        }
    }

    #[inline]
    pub fn into_pattern(self) -> Option<Pattern> {
        match self {
            Self::Pattern(p) => Some(p),
            _ => None,
        }
    }

    #[inline]
    pub fn into_item(self) -> Option<Item> {
        match self {
            Self::Item(i) => Some(i),
            _ => None,
        }
    }

    #[inline]
    pub fn into_items(self) -> Option<List<MetaValue>> {
        match self {
            Self::Items(items) => Some(items),
            _ => None,
        }
    }
}

// ============================================================================
// Collection Operations
// ============================================================================

impl MetaValue {
    /// Get length of collection (array, tuple, text, bytes, map, set, items).
    pub fn len(&self) -> Option<usize> {
        match self {
            Self::Array(arr) => Some(arr.len()),
            Self::Tuple(tup) => Some(tup.len()),
            Self::Text(s) => Some(s.len()),
            Self::Bytes(b) => Some(b.len()),
            Self::Map(m) => Some(m.len()),
            Self::Set(s) => Some(s.len()),
            Self::Items(items) => Some(items.len()),
            _ => None,
        }
    }

    /// Check if collection is empty.
    pub fn is_empty(&self) -> Option<bool> {
        self.len().map(|n| n == 0)
    }
}

// ============================================================================
// Literal Conversion
// ============================================================================

impl MetaValue {
    /// Create a MetaValue from an AST literal.
    ///
    /// This converts AST literals to their corresponding MetaValue representation.
    /// Used during constant folding and meta-function evaluation.
    pub fn from_literal(lit: &Literal) -> Self {
        match &lit.kind {
            LiteralKind::Bool(b) => Self::Bool(*b),
            LiteralKind::Int(i) => Self::Int(i.value),
            LiteralKind::Float(f) => Self::Float(f.value),
            LiteralKind::Text(s) => Self::Text(s.as_str().to_string().into()),
            LiteralKind::Char(c) => Self::Char(*c),
            LiteralKind::ByteChar(b) => Self::Int(*b as i128),
            LiteralKind::ByteString(bytes) => Self::Bytes(bytes.clone()),
            // Complex literals convert to text representation
            LiteralKind::Tagged { .. } => Self::Text("<tagged_literal>".to_string().into()),
            LiteralKind::InterpolatedString(_) => {
                Self::Text("<interpolated_string>".to_string().into())
            }
            LiteralKind::Contract(_) => Self::Text("<contract>".to_string().into()),
            LiteralKind::Composite(_) => Self::Text("<composite>".to_string().into()),
            LiteralKind::ContextAdaptive(_) => Self::Text("<context_adaptive>".to_string().into()),
        }
    }
}

// ============================================================================
// Type Name
// ============================================================================

impl MetaValue {
    /// Get the type name for error messages and debugging.
    #[inline]
    pub fn type_name(&self) -> Text {
        Text::from(match self {
            Self::Unit => "Unit",
            Self::Bool(_) => "Bool",
            Self::Int(_) => "Int",
            Self::UInt(_) => "UInt",
            Self::Float(_) => "Float",
            Self::Char(_) => "Char",
            Self::Text(_) => "Text",
            Self::Bytes(_) => "Bytes",
            Self::Array(_) => "Array",
            Self::Tuple(_) => "Tuple",
            Self::Maybe(_) => "Maybe",
            Self::Map(_) => "Map",
            Self::Set(_) => "Set",
            Self::Expr(_) => "Expr",
            Self::Type(_) => "Type",
            Self::Pattern(_) => "Pattern",
            Self::Item(_) => "Item",
            Self::Items(_) => "Items",
        })
    }

    /// Convert to boolean for conditionals (non-Option version).
    ///
    /// This is equivalent to `is_truthy().unwrap_or(false)` but more convenient
    /// for use in conditional contexts.
    #[inline]
    pub fn as_bool(&self) -> bool {
        self.is_truthy().unwrap_or(false)
    }

    /// Alias for `as_i128()` for backward compatibility.
    #[inline]
    pub fn as_int(&self) -> Option<i128> {
        self.as_i128()
    }

    /// Alias for `as_u128()` for backward compatibility.
    #[inline]
    pub fn as_uint(&self) -> Option<u128> {
        self.as_u128()
    }
}

// ============================================================================
// Truthiness
// ============================================================================

impl MetaValue {
    /// Evaluate truthiness.
    ///
    /// - `Bool(b)` → `b`
    /// - `Int(0)` / `UInt(0)` → `false`
    /// - `Int(_)` / `UInt(_)` → `true`
    /// - `Text("")` → `false`
    /// - `Text(_)` → `true`
    /// - `Array([])` / `Tuple([])` / `Map({})` / `Set({})` → `false`
    /// - `Array(_)` / `Tuple(_)` / `Map(_)` / `Set(_)` → `true`
    /// - `Maybe(None)` → `false`
    /// - `Maybe(Some(_))` → `true`
    /// - `Unit` → `false`
    /// - AST nodes → `true`
    pub fn is_truthy(&self) -> Option<bool> {
        Some(match self {
            Self::Unit => false,
            Self::Bool(b) => *b,
            Self::Int(n) => *n != 0,
            Self::UInt(n) => *n != 0,
            Self::Float(f) => *f != 0.0,
            Self::Char(_) => true,
            Self::Text(s) => !s.is_empty(),
            Self::Bytes(b) => !b.is_empty(),
            Self::Array(arr) => !arr.is_empty(),
            Self::Tuple(tup) => !tup.is_empty(),
            Self::Maybe(m) => m.is_some(),
            Self::Map(m) => !m.is_empty(),
            Self::Set(s) => !s.is_empty(),
            Self::Expr(_) | Self::Type(_) | Self::Pattern(_) | Self::Item(_) => true,
            Self::Items(items) => !items.is_empty(),
        })
    }
}

// ============================================================================
// ConstValue Conversion
// ============================================================================

impl From<ConstValue> for MetaValue {
    fn from(cv: ConstValue) -> Self {
        match cv {
            ConstValue::Unit => Self::Unit,
            ConstValue::Bool(b) => Self::Bool(b),
            ConstValue::Int(n) => Self::Int(n),
            ConstValue::UInt(n) => Self::UInt(n),
            ConstValue::Float(f) => Self::Float(f),
            ConstValue::Char(c) => Self::Char(c),
            ConstValue::Text(s) => Self::Text(s),
            ConstValue::Bytes(b) => Self::Bytes(b),
            ConstValue::Array(arr) => Self::Array(arr.into_iter().map(MetaValue::from).collect()),
            ConstValue::Tuple(tup) => Self::Tuple(tup.into_iter().map(MetaValue::from).collect()),
            ConstValue::Maybe(m) => Self::Maybe(m.map(|inner| Heap::new(MetaValue::from(*inner)))),
            ConstValue::Map(map) => Self::Map(
                map.into_iter()
                    .map(|(k, v)| (k, MetaValue::from(v)))
                    .collect(),
            ),
            ConstValue::Set(set) => Self::Set(set),
        }
    }
}

/// Error when converting MetaValue to ConstValue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaValueConversionError {
    /// Cannot convert AST variant.
    ContainsAst,
}

impl fmt::Display for MetaValueConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContainsAst => write!(f, "MetaValue contains AST node(s)"),
        }
    }
}

impl std::error::Error for MetaValueConversionError {}

impl TryFrom<MetaValue> for ConstValue {
    type Error = MetaValueConversionError;

    fn try_from(mv: MetaValue) -> Result<Self, Self::Error> {
        match mv {
            MetaValue::Unit => Ok(ConstValue::Unit),
            MetaValue::Bool(b) => Ok(ConstValue::Bool(b)),
            MetaValue::Int(n) => Ok(ConstValue::Int(n)),
            MetaValue::UInt(n) => Ok(ConstValue::UInt(n)),
            MetaValue::Float(f) => Ok(ConstValue::Float(f)),
            MetaValue::Char(c) => Ok(ConstValue::Char(c)),
            MetaValue::Text(s) => Ok(ConstValue::Text(s)),
            MetaValue::Bytes(b) => Ok(ConstValue::Bytes(b)),
            MetaValue::Array(arr) => {
                let converted: Result<List<ConstValue>, _> =
                    arr.into_iter().map(ConstValue::try_from).collect();
                converted.map(ConstValue::Array)
            }
            MetaValue::Tuple(tup) => {
                let converted: Result<List<ConstValue>, _> =
                    tup.into_iter().map(ConstValue::try_from).collect();
                converted.map(ConstValue::Tuple)
            }
            MetaValue::Maybe(m) => match m {
                Maybe::None => Ok(ConstValue::Maybe(Maybe::None)),
                Maybe::Some(inner) => {
                    let converted = ConstValue::try_from(*inner)?;
                    Ok(ConstValue::Maybe(Maybe::Some(Heap::new(converted))))
                }
            },
            MetaValue::Map(map) => {
                let mut converted = verum_common::OrderedMap::new();
                for (k, v) in map {
                    let value = ConstValue::try_from(v)?;
                    converted.insert(k, value);
                }
                Ok(ConstValue::Map(converted))
            }
            MetaValue::Set(set) => Ok(ConstValue::Set(set)),
            MetaValue::Expr(_)
            | MetaValue::Type(_)
            | MetaValue::Pattern(_)
            | MetaValue::Item(_)
            | MetaValue::Items(_) => Err(MetaValueConversionError::ContainsAst),
        }
    }
}

// ============================================================================
// Primitive From Implementations
// ============================================================================

impl From<bool> for MetaValue {
    #[inline]
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<i8> for MetaValue {
    #[inline]
    fn from(v: i8) -> Self {
        Self::Int(v as i128)
    }
}

impl From<i16> for MetaValue {
    #[inline]
    fn from(v: i16) -> Self {
        Self::Int(v as i128)
    }
}

impl From<i32> for MetaValue {
    #[inline]
    fn from(v: i32) -> Self {
        Self::Int(v as i128)
    }
}

impl From<i64> for MetaValue {
    #[inline]
    fn from(v: i64) -> Self {
        Self::Int(v as i128)
    }
}

impl From<i128> for MetaValue {
    #[inline]
    fn from(v: i128) -> Self {
        Self::Int(v)
    }
}

impl From<isize> for MetaValue {
    #[inline]
    fn from(v: isize) -> Self {
        Self::Int(v as i128)
    }
}

impl From<u8> for MetaValue {
    #[inline]
    fn from(v: u8) -> Self {
        Self::UInt(v as u128)
    }
}

impl From<u16> for MetaValue {
    #[inline]
    fn from(v: u16) -> Self {
        Self::UInt(v as u128)
    }
}

impl From<u32> for MetaValue {
    #[inline]
    fn from(v: u32) -> Self {
        Self::UInt(v as u128)
    }
}

impl From<u64> for MetaValue {
    #[inline]
    fn from(v: u64) -> Self {
        Self::UInt(v as u128)
    }
}

impl From<u128> for MetaValue {
    #[inline]
    fn from(v: u128) -> Self {
        Self::UInt(v)
    }
}

impl From<usize> for MetaValue {
    #[inline]
    fn from(v: usize) -> Self {
        Self::UInt(v as u128)
    }
}

impl From<f32> for MetaValue {
    #[inline]
    fn from(v: f32) -> Self {
        Self::Float(v as f64)
    }
}

impl From<f64> for MetaValue {
    #[inline]
    fn from(v: f64) -> Self {
        Self::Float(v)
    }
}

impl From<char> for MetaValue {
    #[inline]
    fn from(v: char) -> Self {
        Self::Char(v)
    }
}

impl From<&str> for MetaValue {
    #[inline]
    fn from(v: &str) -> Self {
        Self::Text(v.into())
    }
}

impl From<String> for MetaValue {
    #[inline]
    fn from(v: String) -> Self {
        Self::Text(v.into())
    }
}

impl From<Text> for MetaValue {
    #[inline]
    fn from(v: Text) -> Self {
        Self::Text(v)
    }
}

impl From<Vec<u8>> for MetaValue {
    #[inline]
    fn from(v: Vec<u8>) -> Self {
        Self::Bytes(v)
    }
}

impl From<&[u8]> for MetaValue {
    #[inline]
    fn from(v: &[u8]) -> Self {
        Self::Bytes(v.to_vec())
    }
}

// ============================================================================
// AST From Implementations
// ============================================================================

impl From<Expr> for MetaValue {
    #[inline]
    fn from(v: Expr) -> Self {
        Self::Expr(v)
    }
}

impl From<Type> for MetaValue {
    #[inline]
    fn from(v: Type) -> Self {
        Self::Type(v)
    }
}

impl From<Pattern> for MetaValue {
    #[inline]
    fn from(v: Pattern) -> Self {
        Self::Pattern(v)
    }
}

impl From<Item> for MetaValue {
    #[inline]
    fn from(v: Item) -> Self {
        Self::Item(v)
    }
}

// ============================================================================
// Display
// ============================================================================

impl fmt::Display for MetaValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => write!(f, "()"),
            Self::Bool(b) => write!(f, "{}", b),
            Self::Int(n) => write!(f, "{}", n),
            Self::UInt(n) => write!(f, "{}", n),
            Self::Float(n) => write!(f, "{}", n),
            Self::Char(c) => write!(f, "'{}'", c),
            Self::Text(s) => write!(f, "\"{}\"", s),
            Self::Bytes(b) => write!(f, "b\"{}\"", String::from_utf8_lossy(b)),
            Self::Array(arr) => {
                write!(f, "[")?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            Self::Tuple(tup) => {
                write!(f, "(")?;
                for (i, v) in tup.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, ")")
            }
            Self::Maybe(m) => match m {
                Maybe::None => write!(f, "None"),
                Maybe::Some(v) => write!(f, "Some({})", v),
            },
            Self::Map(map) => {
                write!(f, "{{")?;
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{}\": {}", k, v)?;
                }
                write!(f, "}}")
            }
            Self::Set(set) => {
                write!(f, "Set{{")?;
                for (i, v) in set.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{}\"", v)?;
                }
                write!(f, "}}")
            }
            Self::Expr(_) => write!(f, "<expr>"),
            Self::Type(_) => write!(f, "<type>"),
            Self::Pattern(_) => write!(f, "<pattern>"),
            Self::Item(_) => write!(f, "<item>"),
            Self::Items(items) => write!(f, "[{} items]", items.len()),
        }
    }
}

// ============================================================================
// Default
// ============================================================================

impl Default for MetaValue {
    #[inline]
    fn default() -> Self {
        Self::Unit
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::ExprKind;
    use crate::span::Span;

    #[test]
    fn test_constructors() {
        assert!(MetaValue::unit().is_unit());
        assert!(MetaValue::bool(true).is_bool());
        assert!(MetaValue::int(42).is_int());
        assert!(MetaValue::uint(42).is_uint());
        assert!(MetaValue::float(2.5).is_float());
        assert!(MetaValue::char('x').is_char());
        assert!(MetaValue::text("hello").is_text());
        assert!(MetaValue::bytes(vec![1, 2, 3]).is_bytes());
    }

    #[test]
    fn test_extractors() {
        assert_eq!(MetaValue::bool(true).as_bool_value(), Some(true));
        assert_eq!(MetaValue::int(42).as_i64(), Some(42));
        assert_eq!(MetaValue::int(42).as_i128(), Some(42));
        assert_eq!(MetaValue::uint(100).as_u64(), Some(100));
        assert_eq!(MetaValue::float(2.5).as_f64(), Some(2.5));
        assert_eq!(MetaValue::char('x').as_char_value(), Some('x'));
        assert_eq!(MetaValue::text("hello").as_str(), Some("hello"));
    }

    #[test]
    fn test_from_const_value() {
        let cv = ConstValue::Int(42);
        let mv: MetaValue = cv.into();
        assert!(mv.is_int());
        assert_eq!(mv.as_i128(), Some(42));
    }

    #[test]
    fn test_try_from_primitive() {
        let mv = MetaValue::int(42);
        let cv: Result<ConstValue, _> = mv.try_into();
        assert!(cv.is_ok());
        assert_eq!(cv.unwrap(), ConstValue::Int(42));
    }

    #[test]
    fn test_try_from_ast() {
        let span = Span::dummy();
        let expr = Expr::new(
            ExprKind::Path(crate::ty::Path::single(crate::ty::Ident::new(
                "x".to_string(),
                span,
            ))),
            span,
        );
        let mv = MetaValue::Expr(expr);
        let cv: Result<ConstValue, _> = mv.try_into();
        assert!(cv.is_err());
    }

    #[test]
    fn test_nested_array_conversion() {
        // Array of primitives - should convert
        let arr = MetaValue::array(vec![MetaValue::int(1), MetaValue::int(2)]);
        let cv: Result<ConstValue, _> = arr.try_into();
        assert!(cv.is_ok());

        // Array with AST - should fail
        let span = Span::dummy();
        let expr = Expr::new(
            ExprKind::Path(crate::ty::Path::single(crate::ty::Ident::new(
                "x".to_string(),
                span,
            ))),
            span,
        );
        let arr = MetaValue::array(vec![MetaValue::int(1), MetaValue::Expr(expr)]);
        let cv: Result<ConstValue, _> = arr.try_into();
        assert!(cv.is_err());
    }

    #[test]
    fn test_truthiness() {
        assert_eq!(MetaValue::bool(true).is_truthy(), Some(true));
        assert_eq!(MetaValue::bool(false).is_truthy(), Some(false));
        assert_eq!(MetaValue::int(0).is_truthy(), Some(false));
        assert_eq!(MetaValue::int(1).is_truthy(), Some(true));
        assert_eq!(MetaValue::text("").is_truthy(), Some(false));
        assert_eq!(MetaValue::text("hello").is_truthy(), Some(true));
        assert_eq!(MetaValue::unit().is_truthy(), Some(false));
    }

    #[test]
    fn test_collection_length() {
        let arr = MetaValue::array(vec![MetaValue::int(1), MetaValue::int(2)]);
        assert_eq!(arr.len(), Some(2));
        assert_eq!(MetaValue::text("hello").len(), Some(5));
        assert_eq!(MetaValue::bytes(vec![1, 2, 3]).len(), Some(3));
    }

    #[test]
    fn test_from_primitives() {
        let _: MetaValue = true.into();
        let _: MetaValue = 42i32.into();
        let _: MetaValue = 42u64.into();
        let _: MetaValue = 2.5f64.into();
        let _: MetaValue = 'x'.into();
        let _: MetaValue = "hello".into();
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", MetaValue::int(42)), "42");
        assert_eq!(format!("{}", MetaValue::bool(true)), "true");
        assert_eq!(format!("{}", MetaValue::text("hi")), "\"hi\"");
        assert_eq!(format!("{}", MetaValue::unit()), "()");
    }

    #[test]
    fn test_is_ast() {
        assert!(!MetaValue::int(42).is_ast());
        assert!(MetaValue::int(42).is_primitive());

        let span = Span::dummy();
        let expr = Expr::new(
            ExprKind::Path(crate::ty::Path::single(crate::ty::Ident::new(
                "x".to_string(),
                span,
            ))),
            span,
        );
        assert!(MetaValue::Expr(expr).is_ast());
    }

    #[test]
    fn test_default() {
        assert!(MetaValue::default().is_unit());
    }
}
