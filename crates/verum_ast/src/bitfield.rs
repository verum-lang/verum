//! Bitfield types for the Verum AST.
//!
//! This module defines AST nodes for Verum's first-class bitfield system,
//! which provides type-safe bit-level data manipulation for hardware drivers,
//! network protocols, and embedded systems.
//!
//! # Design Philosophy
//!
//! Verum bitfields follow the language's core principles:
//!
//! - **Semantic Honesty**: Types describe meaning (`BitWidth`, `ByteOrder`)
//!   rather than implementation details
//! - **Type Safety**: Bit widths are validated at compile time
//! - **Portability**: Explicit byte order specification eliminates undefined behavior
//! - **Zero-Cost**: Compiles to optimal bit manipulation instructions
//! - **Verifiable**: SMT integration for constraint verification
//!
//! # Grammar
//!
//! Per `grammar/verum.ebnf`, bitfield attributes use standard attribute syntax:
//!
//! ```text
//! @bits(N)           - Field bit width
//! @bitfield          - Type uses packed bitfield layout
//! @endian(mode)      - Byte order specification (big, little, native)
//! @offset(N)         - Explicit bit offset (optional)
//! ```
//!
//! # Examples
//!
//! ```verum
//! // Network packet header with explicit bit layout
//! @bitfield
//! @endian(big)  // Network byte order
//! type IpHeader is {
//!     @bits(4)  version: U8,
//!     @bits(4)  ihl: U8,
//!     @bits(8)  dscp_ecn: U8,
//!     @bits(16) total_length: U16,
//!     @bits(16) identification: U16,
//!     @bits(3)  flags: U8,
//!     @bits(13) fragment_offset: U16,
//!     @bits(8)  ttl: U8,
//!     @bits(8)  protocol: U8,
//!     @bits(16) header_checksum: U16,
//!     @bits(32) source_addr: U32,
//!     @bits(32) dest_addr: U32,
//! };
//!
//! // Hardware register with read-only and write-only fields
//! @bitfield
//! @endian(little)
//! type StatusRegister is {
//!     @bits(1)  busy: Bool,      // Read-only status
//!     @bits(1)  error: Bool,     // Read-only status
//!     @bits(6)  _reserved: U8,   // Padding
//!     @bits(8)  error_code: U8,  // Read-only error code
//! };
//! ```
//!
//! # Compile-Time Verification
//!
//! The type checker validates:
//! - Field bit width does not exceed storage type width
//! - Total bits fit within specified container size
//! - No overlapping fields (unless explicitly specified)
//! - Alignment constraints are satisfied

use crate::span::{Span, Spanned};
use serde::{Deserialize, Serialize};
use verum_common::Maybe;

/// Bit width specification for a bitfield member.
///
/// Represents the number of bits occupied by a field within a bitfield type.
/// The width must be positive and cannot exceed the storage type's bit width.
///
/// # Validation Rules
///
/// - `width > 0` (zero-width fields are invalid)
/// - `width <= storage_type.bits` (e.g., `@bits(9)` on `U8` is invalid)
/// - For boolean fields, `width == 1` is enforced
///
/// # Examples
///
/// ```verum
/// @bits(4) version: U8,   // Valid: 4 <= 8
/// @bits(16) port: U16,    // Valid: 16 <= 16
/// @bits(1) flag: Bool,    // Valid: boolean as single bit
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BitWidth {
    /// The number of bits this field occupies
    pub bits: u32,
    /// Source location for error reporting
    pub span: Span,
}

impl BitWidth {
    /// Create a new bit width specification.
    pub fn new(bits: u32, span: Span) -> Self {
        Self { bits, span }
    }

    /// Check if this width is valid for the given storage type bit width.
    pub fn is_valid_for(&self, storage_bits: u32) -> bool {
        self.bits > 0 && self.bits <= storage_bits
    }

    /// Get the bit width value.
    pub fn value(&self) -> u32 {
        self.bits
    }
}

impl Spanned for BitWidth {
    fn span(&self) -> Span {
        self.span
    }
}

impl std::fmt::Display for BitWidth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "@bits({})", self.bits)
    }
}

/// Byte order specification for multi-byte bitfield types.
///
/// Determines how multi-byte values are laid out in memory. This is critical
/// for hardware interfaces and network protocols where byte order matters.
///
/// # Semantic Naming
///
/// Uses `ByteOrder` instead of "endianness" for clearer semantic meaning:
/// - Big: Most significant byte first (network byte order)
/// - Little: Least significant byte first (x86, ARM default)
/// - Native: Platform-specific (use with caution for portability)
///
/// # Examples
///
/// ```verum
/// @endian(big)     // Network protocols (TCP/IP, etc.)
/// @endian(little)  // x86/ARM hardware registers
/// @endian(native)  // Platform-specific (rarely recommended)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ByteOrder {
    /// Big-endian: Most significant byte at lowest address.
    /// Standard for network protocols (network byte order).
    Big,

    /// Little-endian: Least significant byte at lowest address.
    /// Common for x86 and ARM processors.
    #[default]
    Little,

    /// Native byte order: Uses the target platform's default.
    /// Avoid for portable code or network protocols.
    Native,
}

impl ByteOrder {
    /// Parse byte order from a string identifier.
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "big" | "Big" | "BIG" => Maybe::Some(ByteOrder::Big),
            "little" | "Little" | "LITTLE" => Maybe::Some(ByteOrder::Little),
            "native" | "Native" | "NATIVE" => Maybe::Some(ByteOrder::Native),
            _ => Maybe::None,
        }
    }

    /// Get the string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            ByteOrder::Big => "big",
            ByteOrder::Little => "little",
            ByteOrder::Native => "native",
        }
    }

    /// Check if this is network byte order (big-endian).
    pub fn is_network_order(&self) -> bool {
        matches!(self, ByteOrder::Big)
    }
}

impl std::fmt::Display for ByteOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Complete specification for a bitfield member.
///
/// Combines bit width with optional explicit offset for fields that
/// require precise bit positioning within the container.
///
/// # Layout Calculation
///
/// If `offset` is `None`, the field is placed immediately after the
/// previous field. If `offset` is `Some(n)`, the field starts at bit `n`
/// from the beginning of the container (0-indexed).
///
/// # Examples
///
/// ```verum
/// // Automatic layout (sequential)
/// @bits(4) version: U8,  // bits 0-3
/// @bits(4) ihl: U8,      // bits 4-7
///
/// // Explicit offset (for reserved gaps)
/// @bits(8) @offset(0)  low_byte: U8,
/// @bits(8) @offset(24) high_byte: U8,  // Skip bits 8-23
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BitSpec {
    /// The bit width of this field
    pub width: BitWidth,

    /// Optional explicit bit offset from start of container.
    /// If None, the field is placed sequentially after the previous field.
    pub offset: Maybe<u32>,

    /// Source location for error reporting
    pub span: Span,
}

impl BitSpec {
    /// Create a bit specification with automatic offset.
    pub fn new(width: BitWidth, span: Span) -> Self {
        Self {
            width,
            offset: Maybe::None,
            span,
        }
    }

    /// Create a bit specification with explicit offset.
    pub fn with_offset(width: BitWidth, offset: u32, span: Span) -> Self {
        Self {
            width,
            offset: Maybe::Some(offset),
            span,
        }
    }

    /// Get the bit width.
    pub fn bit_width(&self) -> u32 {
        self.width.bits
    }

    /// Check if this field has an explicit offset.
    pub fn has_explicit_offset(&self) -> bool {
        matches!(self.offset, Maybe::Some(_))
    }

    /// Get the explicit offset, if specified.
    pub fn explicit_offset(&self) -> Maybe<u32> {
        self.offset
    }
}

impl Spanned for BitSpec {
    fn span(&self) -> Span {
        self.span
    }
}

/// Bitfield layout specification for a type.
///
/// Marks a type as using packed bitfield layout rather than standard
/// struct layout with natural alignment.
///
/// # Layout Semantics
///
/// When a type has `@bitfield` annotation:
/// - Fields are packed according to their `@bits(N)` specifications
/// - No implicit padding between fields (unless via `@offset`)
/// - Byte order determined by `@endian` attribute
/// - Total size is rounded up to byte boundary
///
/// # Accessor Generation
///
/// The compiler generates type-safe accessor methods:
/// - `get_field() -> T`: Extract field value with proper masking/shifting
/// - `set_field(value: T)`: Update field with bounds checking
/// - `with_field(value: T) -> Self`: Builder pattern for immutable updates
///
/// # Examples
///
/// ```verum
/// @bitfield
/// @endian(big)
/// type Flags is {
///     @bits(1) enabled: Bool,
///     @bits(3) priority: U8,
///     @bits(4) mode: U8,
/// };
///
/// let f = Flags { enabled: true, priority: 5, mode: 2 };
/// assert(f.get_enabled() == true);
/// assert(f.get_priority() == 5);
///
/// let f2 = f.with_priority(7);  // Immutable update
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BitLayout {
    /// Byte order for multi-byte containers
    pub byte_order: ByteOrder,

    /// Optional explicit total size in bits.
    /// If None, calculated from field specifications.
    pub total_bits: Maybe<u32>,

    /// Whether to allow overlapping fields.
    /// Default is false (overlapping fields are an error).
    pub allow_overlap: bool,

    /// Source location of the @bitfield attribute
    pub span: Span,
}

impl BitLayout {
    /// Create a new bitfield layout with default settings.
    pub fn new(span: Span) -> Self {
        Self {
            byte_order: ByteOrder::default(),
            total_bits: Maybe::None,
            allow_overlap: false,
            span,
        }
    }

    /// Create a bitfield layout with specified byte order.
    pub fn with_byte_order(byte_order: ByteOrder, span: Span) -> Self {
        Self {
            byte_order,
            total_bits: Maybe::None,
            allow_overlap: false,
            span,
        }
    }

    /// Set the total bit size explicitly.
    pub fn with_total_bits(mut self, bits: u32) -> Self {
        self.total_bits = Maybe::Some(bits);
        self
    }

    /// Allow overlapping fields (for union-like semantics).
    pub fn with_overlap_allowed(mut self) -> Self {
        self.allow_overlap = true;
        self
    }

    /// Get the byte order.
    pub fn byte_order(&self) -> ByteOrder {
        self.byte_order
    }

    /// Check if fields can overlap.
    pub fn allows_overlap(&self) -> bool {
        self.allow_overlap
    }
}

impl Spanned for BitLayout {
    fn span(&self) -> Span {
        self.span
    }
}

impl Default for BitLayout {
    fn default() -> Self {
        Self {
            byte_order: ByteOrder::default(),
            total_bits: Maybe::None,
            allow_overlap: false,
            span: Span::dummy(),
        }
    }
}

/// Computed layout information for a resolved bitfield.
///
/// This is produced by the type checker after validating all field
/// specifications and computing the final layout.
///
/// # Usage
///
/// This struct is used during code generation to:
/// - Generate correct accessor masks and shifts
/// - Determine container type and alignment
/// - Produce optimal bit manipulation code
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedBitLayout {
    /// Total size in bits (rounded up to byte boundary for storage)
    pub total_bits: u32,

    /// Total size in bytes (ceil(total_bits / 8))
    pub total_bytes: u32,

    /// Byte order for the container
    pub byte_order: ByteOrder,

    /// Resolved field layouts with computed offsets
    pub fields: Vec<ResolvedBitField>,
}

impl ResolvedBitLayout {
    /// Create a new resolved layout.
    pub fn new(byte_order: ByteOrder) -> Self {
        Self {
            total_bits: 0,
            total_bytes: 0,
            byte_order,
            fields: Vec::new(),
        }
    }

    /// Add a resolved field to the layout.
    pub fn add_field(&mut self, field: ResolvedBitField) {
        let end_bit = field.offset + field.width;
        if end_bit > self.total_bits {
            self.total_bits = end_bit;
            self.total_bytes = end_bit.div_ceil(8);
        }
        self.fields.push(field);
    }

    /// Get the mask for extracting a field at the given index.
    pub fn field_mask(&self, index: usize) -> Maybe<u64> {
        self.fields.get(index).map(|f| f.mask())
    }

    /// Get the shift amount for a field at the given index.
    pub fn field_shift(&self, index: usize) -> Maybe<u32> {
        self.fields.get(index).map(|f| f.offset)
    }
}

/// Resolved layout information for a single bitfield member.
///
/// Contains all information needed to generate accessor code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedBitField {
    /// Field name for error messages and codegen
    pub name: String,

    /// Bit offset from start of container (0-indexed)
    pub offset: u32,

    /// Bit width of the field
    pub width: u32,

    /// Index of this field in the parent struct
    pub field_index: usize,
}

impl ResolvedBitField {
    /// Create a new resolved bit field.
    pub fn new(name: String, offset: u32, width: u32, field_index: usize) -> Self {
        Self {
            name,
            offset,
            width,
            field_index,
        }
    }

    /// Compute the extraction mask for this field.
    ///
    /// The mask has `width` bits set starting at bit 0.
    /// To extract the field: `(value >> offset) & mask`
    pub fn mask(&self) -> u64 {
        if self.width >= 64 {
            u64::MAX
        } else {
            (1u64 << self.width) - 1
        }
    }

    /// Compute the in-place mask for this field.
    ///
    /// The mask has `width` bits set at the field's position.
    /// To clear the field: `value & !in_place_mask`
    pub fn in_place_mask(&self) -> u64 {
        self.mask() << self.offset
    }

    /// Get the bit range as (start, end) exclusive.
    pub fn bit_range(&self) -> (u32, u32) {
        (self.offset, self.offset + self.width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::FileId;

    fn test_span() -> Span {
        Span::new(0, 10, FileId::new(0))
    }

    #[test]
    fn test_bit_width_validation() {
        let w = BitWidth::new(4, test_span());
        assert!(w.is_valid_for(8)); // 4 bits fits in U8
        assert!(w.is_valid_for(4)); // exactly fits
        assert!(!w.is_valid_for(3)); // too small
    }

    #[test]
    fn test_byte_order_parsing() {
        assert_eq!(ByteOrder::from_str("big"), Maybe::Some(ByteOrder::Big));
        assert_eq!(
            ByteOrder::from_str("little"),
            Maybe::Some(ByteOrder::Little)
        );
        assert_eq!(
            ByteOrder::from_str("native"),
            Maybe::Some(ByteOrder::Native)
        );
        assert_eq!(ByteOrder::from_str("invalid"), Maybe::None);
    }

    #[test]
    fn test_resolved_field_mask() {
        let field = ResolvedBitField::new("version".into(), 0, 4, 0);
        assert_eq!(field.mask(), 0b1111);
        assert_eq!(field.in_place_mask(), 0b1111);

        let field2 = ResolvedBitField::new("flags".into(), 4, 4, 1);
        assert_eq!(field2.mask(), 0b1111);
        assert_eq!(field2.in_place_mask(), 0b11110000);
    }

    #[test]
    fn test_resolved_layout() {
        let mut layout = ResolvedBitLayout::new(ByteOrder::Big);
        layout.add_field(ResolvedBitField::new("version".into(), 0, 4, 0));
        layout.add_field(ResolvedBitField::new("ihl".into(), 4, 4, 1));

        assert_eq!(layout.total_bits, 8);
        assert_eq!(layout.total_bytes, 1);
        assert_eq!(layout.fields.len(), 2);
    }
}
