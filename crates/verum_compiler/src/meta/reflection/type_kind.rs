//! Type classification for compile-time reflection
//!
//! Provides type kind enumeration matching core/meta/reflection.vr TypeKind.

use verum_ast::MetaValue;
use verum_common::Text;

/// Type classification matching stdlib TypeKind
///
/// Provides complete type classification for compile-time reflection.
/// Matches: core/meta/reflection.vr TypeKind
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TypeKind {
    /// Struct/record type: `type Foo is { ... }`
    Struct = 0,
    /// Enum/sum type: `type Foo is A | B | C`
    Enum = 1,
    /// Newtype wrapper: `type Foo is (T)`
    Newtype = 2,
    /// Unit type: `type Foo is ()`
    Unit = 3,
    /// Protocol/trait: `type Foo is protocol { ... }`
    Protocol = 4,
    /// Tuple type: `(A, B, C)`
    Tuple = 5,
    /// Array type: `[T; N]`
    Array = 6,
    /// Slice type: `[T]`
    Slice = 7,
    /// Reference type: `&T`, `&mut T`
    Reference = 8,
    /// Pointer type: `*const T`, `*mut T`
    Pointer = 9,
    /// Function type: `fn(A) -> B`
    Function = 10,
    /// Generic type parameter: `T` in `fn foo<T>()`
    TypeParam = 11,
    /// Associated type: `Self.Item`
    Associated = 12,
    /// Primitive type: Int, Bool, Float, etc.
    Primitive = 13,
    /// Never type: `!`
    Never = 14,
    /// Inferred type: `_`
    Infer = 15,
    /// Unknown (error recovery)
    Unknown = 16,
    /// Type alias: `type Foo is Bar`
    Alias = 17,
}

impl TypeKind {
    /// Check if this is a compound type (has fields or variants)
    #[inline]
    pub fn is_compound(&self) -> bool {
        matches!(self, TypeKind::Struct | TypeKind::Enum | TypeKind::Tuple)
    }

    /// Check if this is a reference type
    #[inline]
    pub fn is_reference(&self) -> bool {
        matches!(self, TypeKind::Reference | TypeKind::Pointer)
    }

    /// Check if this is a primitive type
    #[inline]
    pub fn is_primitive(&self) -> bool {
        matches!(self, TypeKind::Primitive)
    }

    /// Get human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            TypeKind::Struct => "struct",
            TypeKind::Enum => "enum",
            TypeKind::Newtype => "newtype",
            TypeKind::Unit => "unit",
            TypeKind::Protocol => "protocol",
            TypeKind::Tuple => "tuple",
            TypeKind::Array => "array",
            TypeKind::Slice => "slice",
            TypeKind::Reference => "reference",
            TypeKind::Pointer => "pointer",
            TypeKind::Function => "function",
            TypeKind::TypeParam => "type parameter",
            TypeKind::Associated => "associated type",
            TypeKind::Primitive => "primitive",
            TypeKind::Never => "never",
            TypeKind::Infer => "inferred",
            TypeKind::Unknown => "unknown",
            TypeKind::Alias => "alias",
        }
    }

    /// Convert to MetaValue
    pub fn to_meta_value(&self) -> MetaValue {
        MetaValue::Int(*self as i128)
    }

    /// Alias for to_meta_value for backward compatibility
    #[inline]
    pub fn to_const_value(&self) -> MetaValue {
        self.to_meta_value()
    }
}

/// Item visibility matching stdlib Visibility
#[derive(Debug, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum Visibility {
    /// Public: accessible everywhere
    Public = 0,
    /// Private: accessible only in current module
    Private = 1,
    /// Crate-visible: accessible within current crate
    Crate = 2,
    /// Super-visible: accessible in parent module
    Super = 3,
    /// Path-restricted: accessible in specific path
    In(Text) = 4,
}

impl Default for Visibility {
    fn default() -> Self {
        Visibility::Private
    }
}

impl Visibility {
    /// Check if publicly visible
    #[inline]
    pub fn is_public(&self) -> bool {
        matches!(self, Visibility::Public)
    }

    /// Get keyword for this visibility
    pub fn keyword(&self) -> Option<&str> {
        match self {
            Visibility::Public => Some("public"),
            Visibility::Private => None,
            Visibility::Crate => Some("public(crate)"),
            Visibility::Super => Some("public(super)"),
            Visibility::In(_) => None,
        }
    }
}

/// Variant kind matching stdlib VariantKind
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VariantKind {
    /// Unit variant: `None`
    Unit = 0,
    /// Tuple variant: `Some(T)`
    Tuple = 1,
    /// Struct variant: `Point { x: Int, y: Int }`
    Struct = 2,
}
