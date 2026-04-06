//! VBC type system definitions.
//!
//! This module defines the type system used in VBC bytecode, including:
//! - Type identifiers (TypeId, TypeParamId, ProtocolId)
//! - Type references (TypeRef) with support for generics
//! - Type descriptors for record, sum, protocol types
//! - Field and variant descriptors

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

// ============================================================================
// Identifiers
// ============================================================================

/// String identifier - byte offset into string table.
///
/// StringId enables O(1) access to strings: seek to offset, read length, read bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct StringId(pub u32);

impl StringId {
    /// Empty string (by convention at offset 0).
    pub const EMPTY: StringId = StringId(0);
}

/// Type identifier - index into type table.
///
/// Built-in types have predefined IDs (0-15), user types start at 16.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct TypeId(pub u32);

impl TypeId {
    /// Unit type `()`.
    pub const UNIT: TypeId = TypeId(0);
    /// Boolean type.
    pub const BOOL: TypeId = TypeId(1);
    /// 64-bit signed integer.
    pub const INT: TypeId = TypeId(2);
    /// 64-bit floating point.
    pub const FLOAT: TypeId = TypeId(3);
    /// Text (UTF-8 string).
    pub const TEXT: TypeId = TypeId(4);
    /// Never type (bottom type).
    pub const NEVER: TypeId = TypeId(5);
    /// 8-bit unsigned integer.
    pub const U8: TypeId = TypeId(6);
    /// 16-bit unsigned integer.
    pub const U16: TypeId = TypeId(7);
    /// 32-bit unsigned integer.
    pub const U32: TypeId = TypeId(8);
    /// 64-bit unsigned integer.
    pub const U64: TypeId = TypeId(9);
    /// 8-bit signed integer.
    pub const I8: TypeId = TypeId(10);
    /// 16-bit signed integer.
    pub const I16: TypeId = TypeId(11);
    /// 32-bit signed integer.
    pub const I32: TypeId = TypeId(12);
    /// 32-bit floating point.
    pub const F32: TypeId = TypeId(13);
    /// Raw pointer (usize).
    pub const PTR: TypeId = TypeId(14);
    /// Reserved.
    pub const RESERVED: TypeId = TypeId(15);

    // ========================================================================
    // Type Aliases (for semantic clarity)
    // ========================================================================

    /// 64-bit signed integer (alias for INT).
    /// Used when explicitly referring to Int64 rather than the default Int.
    pub const I64: TypeId = TypeId(2);

    /// 64-bit floating point (alias for FLOAT).
    /// Used when explicitly referring to Float64 rather than the default Float.
    pub const F64: TypeId = TypeId(3);

    /// Pointer-sized signed integer (alias for PTR on this platform).
    /// Used for ISize type in Verum.
    pub const ISIZE: TypeId = TypeId(14);

    /// Pointer-sized unsigned integer (alias for PTR on this platform).
    /// Used for USize type in Verum.
    pub const USIZE: TypeId = TypeId(14);

    /// First user-defined type ID.
    pub const FIRST_USER: u32 = 16;

    // ========================================================================
    // Well-Known Meta System Type IDs (256-511)
    // ========================================================================

    /// TokenStream type for meta-programming.
    /// Used by quote expressions to represent generated code as a token sequence.
    pub const TOKEN_STREAM: TypeId = TypeId(256);

    /// Token type - individual token in a TokenStream.
    pub const TOKEN: TypeId = TypeId(257);

    /// TokenKind type - discriminant for token types.
    pub const TOKEN_KIND: TypeId = TypeId(258);

    /// Span type - source location information.
    pub const SPAN: TypeId = TypeId(259);

    // ========================================================================
    // Well-Known Semantic Collection Type IDs (512-1023)
    // ========================================================================
    //
    // These are reserved type IDs for Verum's semantic collection types.
    // They provide O(1) type discrimination for iterator and collection operations.
    //
    // Verum uses semantic type names: List (not Vec), Text (not String), Map (not HashMap).
    // These TypeIds enable O(1) dispatch for collection operations without string comparison.

    /// List<T> - dynamic array with semantic naming.
    /// Layout: [len: i64, cap: i64, backing_ptr: *Value]
    pub const LIST: TypeId = TypeId(512);

    /// Map<K, V> - hash map with semantic naming.
    /// Layout: [count: i64, capacity: i64, entries_ptr: *Entry]
    pub const MAP: TypeId = TypeId(513);

    /// Set<T> - hash set with semantic naming.
    /// Layout: [count: i64, capacity: i64, entries_ptr: *Entry]
    pub const SET: TypeId = TypeId(514);

    /// Maybe<T> - optional value (None | Some(T)).
    /// Layout: [tag: i64, value: T] where tag=0 is None, tag=1 is Some.
    pub const MAYBE: TypeId = TypeId(515);

    /// Result<T, E> - fallible value (Ok(T) | Err(E)).
    /// Layout: [tag: i64, value: T|E] where tag=0 is Ok, tag=1 is Err.
    pub const RESULT: TypeId = TypeId(516);

    /// Range<T> - iterator range (start..end).
    /// Layout: [current: T, end: T, inclusive: bool]
    pub const RANGE: TypeId = TypeId(517);

    /// Array<T, N> - fixed-size array with compile-time length.
    /// Layout: [len: i64, elements: [T; N]]
    pub const ARRAY: TypeId = TypeId(518);

    /// Heap<T> - heap-allocated box with CBGR tracking.
    /// Layout: [inner_ptr: *T]
    pub const HEAP: TypeId = TypeId(519);

    /// Shared<T> - reference-counted shared pointer.
    /// Layout: [inner_ptr: *T, refcount_ptr: *AtomicU64]
    pub const SHARED: TypeId = TypeId(520);

    /// Tuple - anonymous product type.
    /// Layout: [field_0, field_1, ..., field_n]
    pub const TUPLE: TypeId = TypeId(521);

    /// Deque<T> - double-ended queue (ring buffer).
    /// Layout: [len: i64, cap: i64, head: i64, buffer_ptr: *u8]
    pub const DEQUE: TypeId = TypeId(522);

    /// Channel<T> - bounded message queue.
    /// Layout: [len: i64, cap: i64, head: i64, buffer_ptr: *u8, closed: i64]
    pub const CHANNEL: TypeId = TypeId(523);

    /// First semantic type ID (for range checks).
    pub const FIRST_SEMANTIC: u32 = 512;

    /// Last semantic type ID (for range checks).
    pub const LAST_SEMANTIC: u32 = 1023;

    /// Checks if this is a meta-system type (TokenStream, Token, etc.).
    pub fn is_meta_type(self) -> bool {
        matches!(self.0, 256..=259)
    }

    /// Checks if this is a semantic collection type (List, Map, Set, etc.).
    pub fn is_semantic_type(self) -> bool {
        matches!(self.0, Self::FIRST_SEMANTIC..=Self::LAST_SEMANTIC)
    }

    /// Checks if this is an iterable type.
    pub fn is_iterable(self) -> bool {
        matches!(
            self.0,
            512 | 513 | 514 | 517 | 518  // LIST, MAP, SET, RANGE, ARRAY
        )
    }

    /// Checks if this is a built-in type.
    pub fn is_builtin(self) -> bool {
        self.0 < Self::FIRST_USER
    }

    /// Checks if this is a primitive numeric type.
    pub fn is_numeric(self) -> bool {
        matches!(
            self.0,
            2 | 3 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13
        )
    }

    /// Checks if this is an integer type.
    pub fn is_integer(self) -> bool {
        matches!(self.0, 2 | 6 | 7 | 8 | 9 | 10 | 11 | 12)
    }

    /// Checks if this is a floating point type.
    pub fn is_float(self) -> bool {
        matches!(self.0, 3 | 13)
    }
}

/// Type parameter identifier - unique within a function or type definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct TypeParamId(pub u16);

/// Protocol identifier - index into type table (protocols are types).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct ProtocolId(pub u32);

/// Context reference - identifies a context type for dependency injection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct ContextRef(pub u32);

// ============================================================================
// Type References
// ============================================================================

/// Type reference that may contain generic parameters.
///
/// TypeRef represents types as they appear in function signatures and expressions,
/// potentially containing unresolved generic parameters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TypeRef {
    /// Concrete type (no generics).
    Concrete(TypeId),

    /// Generic type parameter (T, K, V, etc.).
    /// References a TypeParam in the enclosing function/type.
    Generic(TypeParamId),

    /// Instantiated generic type: `List<Int>`, `Map<Text, Int>`.
    Instantiated {
        /// Base generic type.
        base: TypeId,
        /// Type arguments (Vec to avoid infinite size due to recursion).
        args: Vec<TypeRef>,
    },

    /// Function type.
    Function {
        /// Parameter types (Vec to avoid infinite size).
        params: Vec<TypeRef>,
        /// Return type.
        return_type: Box<TypeRef>,
        /// Required contexts.
        contexts: SmallVec<[ContextRef; 2]>,
    },

    /// Rank-2 polymorphic function type: fn<R>(params) -> return_type
    ///
    /// Type parameters are universally quantified inside the function type,
    /// meaning the function must work for ALL instantiations of the type parameters.
    /// This enables patterns like transducers: fn<R>(Reducer<B, R>) -> Reducer<A, R>
    ///
    /// Spec: grammar/verum.ebnf - rank2_function_type
    Rank2Function {
        /// Number of universally quantified type parameters (local to this function type).
        /// These are indexed starting from 0 within this type's scope.
        type_param_count: u16,
        /// Parameter types (may reference the local type params via TypeParamId).
        params: Vec<TypeRef>,
        /// Return type (may reference the local type params).
        return_type: Box<TypeRef>,
        /// Required contexts.
        contexts: SmallVec<[ContextRef; 2]>,
    },

    /// Reference type.
    Reference {
        /// Inner type.
        inner: Box<TypeRef>,
        /// Mutability.
        mutability: Mutability,
        /// CBGR tier.
        tier: CbgrTier,
    },

    /// Tuple type (Vec to avoid infinite size).
    Tuple(Vec<TypeRef>),

    /// Array type with static size.
    Array {
        /// Element type.
        element: Box<TypeRef>,
        /// Array length.
        length: u64,
    },

    /// Slice type (dynamic size).
    Slice(Box<TypeRef>),
}

impl Default for TypeRef {
    fn default() -> Self {
        TypeRef::Concrete(TypeId::UNIT)
    }
}

impl TypeRef {
    /// Creates a concrete type reference.
    pub fn concrete(id: TypeId) -> Self {
        TypeRef::Concrete(id)
    }

    /// Creates a generic type parameter reference.
    pub fn generic(id: TypeParamId) -> Self {
        TypeRef::Generic(id)
    }

    /// Creates an instantiated generic type.
    pub fn instantiated(base: TypeId, args: impl IntoIterator<Item = TypeRef>) -> Self {
        TypeRef::Instantiated {
            base,
            args: args.into_iter().collect(),
        }
    }

    /// Creates a reference type.
    pub fn reference(inner: TypeRef, mutability: Mutability, tier: CbgrTier) -> Self {
        TypeRef::Reference {
            inner: Box::new(inner),
            mutability,
            tier,
        }
    }

    /// Checks if this type contains any generic parameters.
    pub fn is_generic(&self) -> bool {
        match self {
            TypeRef::Concrete(_) => false,
            TypeRef::Generic(_) => true,
            TypeRef::Instantiated { args, .. } => args.iter().any(|a| a.is_generic()),
            TypeRef::Function {
                params,
                return_type,
                ..
            } => params.iter().any(|p| p.is_generic()) || return_type.is_generic(),
            TypeRef::Rank2Function {
                params,
                return_type,
                ..
            } => params.iter().any(|p| p.is_generic()) || return_type.is_generic(),
            TypeRef::Reference { inner, .. } => inner.is_generic(),
            TypeRef::Tuple(elems) => elems.iter().any(|e| e.is_generic()),
            TypeRef::Array { element, .. } => element.is_generic(),
            TypeRef::Slice(inner) => inner.is_generic(),
        }
    }

    /// Returns the base type ID if this is a concrete or instantiated type.
    pub fn base_type_id(&self) -> Option<TypeId> {
        match self {
            TypeRef::Concrete(id) => Some(*id),
            TypeRef::Instantiated { base, .. } => Some(*base),
            _ => None,
        }
    }
}

/// Mutability of a reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[repr(u8)]
pub enum Mutability {
    /// Immutable reference `&T`.
    #[default]
    Immutable = 0,
    /// Mutable reference `&mut T`.
    Mutable = 1,
}

/// CBGR tier for reference safety checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[repr(u8)]
pub enum CbgrTier {
    /// Tier 0: Runtime checked (~15ns overhead).
    #[default]
    Tier0 = 0,
    /// Tier 1: Compiler proven safe (0ns overhead).
    Tier1 = 1,
    /// Tier 2: Manual proof required (0ns overhead, unsafe).
    Tier2 = 2,
}

/// Reference capability flags for CBGR permission checking.
///
/// CBGR capability checks ensure references have required permissions before access.
/// Read/Write/Execute capabilities are verified at dereference points. The `subsumes()`
/// method checks permission inclusion (e.g., ReadWrite subsumes Read). Methods
/// `for_deref()`, `for_store()`, `for_call()` return the required capability for each operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ReferenceCapability {
    /// Read-only access.
    Read = 0x01,
    /// Write-only access.
    Write = 0x02,
    /// Read and write access.
    ReadWrite = 0x03,
    /// Execute access (for function pointers).
    Execute = 0x04,
}

impl ReferenceCapability {
    /// Convert capability to raw flags.
    #[must_use]
    pub fn to_flags(self) -> u8 {
        self as u8
    }

    /// Check if capability includes read permission.
    #[must_use]
    pub fn can_read(self) -> bool {
        matches!(self, Self::Read | Self::ReadWrite)
    }

    /// Check if capability includes write permission.
    #[must_use]
    pub fn can_write(self) -> bool {
        matches!(self, Self::Write | Self::ReadWrite)
    }

    /// Check if capability includes execute permission.
    #[must_use]
    pub fn can_execute(self) -> bool {
        matches!(self, Self::Execute)
    }

    /// Check if one capability subsumes another.
    ///
    /// Returns true if `self` provides all permissions required by `required`.
    /// Used to verify that a reference has sufficient capabilities for an operation.
    #[must_use]
    pub fn subsumes(self, required: Self) -> bool {
        match required {
            Self::Read => self.can_read(),
            Self::Write => self.can_write(),
            Self::ReadWrite => self.can_read() && self.can_write(),
            Self::Execute => self.can_execute(),
        }
    }

    /// Get the name of this capability for diagnostics.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::ReadWrite => "read-write",
            Self::Execute => "execute",
        }
    }

    /// Required capability for a dereference operation.
    #[must_use]
    pub fn for_deref(is_mut: bool) -> Self {
        if is_mut {
            Self::ReadWrite
        } else {
            Self::Read
        }
    }

    /// Required capability for a store operation.
    #[must_use]
    pub fn for_store() -> Self {
        Self::Write
    }

    /// Required capability for a function pointer call.
    #[must_use]
    pub fn for_call() -> Self {
        Self::Execute
    }
}

// ============================================================================
// Type Descriptors
// ============================================================================

/// Type kind classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[repr(u8)]
pub enum TypeKind {
    /// Primitive type (Int, Float, Bool, etc.).
    #[default]
    Primitive = 0,
    /// Record/struct type.
    Record = 1,
    /// Sum/enum type.
    Sum = 2,
    /// Protocol (trait).
    Protocol = 3,
    /// Newtype wrapper.
    Newtype = 4,
    /// Tuple type.
    Tuple = 5,
    /// Unit type.
    Unit = 6,
    /// Array type.
    Array = 7,
    /// Tensor type.
    Tensor = 8,
}

impl TryFrom<u8> for TypeKind {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(TypeKind::Primitive),
            1 => Ok(TypeKind::Record),
            2 => Ok(TypeKind::Sum),
            3 => Ok(TypeKind::Protocol),
            4 => Ok(TypeKind::Newtype),
            5 => Ok(TypeKind::Tuple),
            6 => Ok(TypeKind::Unit),
            7 => Ok(TypeKind::Array),
            8 => Ok(TypeKind::Tensor),
            _ => Err(value),
        }
    }
}

/// Visibility of types, fields, and functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[repr(u8)]
pub enum Visibility {
    /// Private to the module.
    Private = 0,
    /// Public to all.
    #[default]
    Public = 1,
    /// Public to cog only.
    Cog = 2,
}

impl TryFrom<u8> for Visibility {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Visibility::Private),
            1 => Ok(Visibility::Public),
            2 => Ok(Visibility::Cog),
            _ => Err(value),
        }
    }
}

/// Variance of a type parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[repr(u8)]
pub enum Variance {
    /// Covariant: `List<Cat>` is subtype of `List<Animal>`.
    Covariant = 0,
    /// Contravariant: `Fn(Animal)` is subtype of `Fn(Cat)`.
    Contravariant = 1,
    /// Invariant: No subtyping.
    #[default]
    Invariant = 2,
}

impl TryFrom<u8> for Variance {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Variance::Covariant),
            1 => Ok(Variance::Contravariant),
            2 => Ok(Variance::Invariant),
            _ => Err(value),
        }
    }
}

/// Type descriptor in the type table.
///
/// Contains all metadata about a type including generic parameters,
/// fields, variants, and protocol implementations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeDescriptor {
    /// Unique type ID.
    pub id: TypeId,
    /// Type name (index into string table).
    pub name: StringId,
    /// Type kind.
    pub kind: TypeKind,
    /// Generic type parameters (empty for concrete types).
    pub type_params: SmallVec<[TypeParamDescriptor; 2]>,
    /// Fields (for record types).
    pub fields: SmallVec<[FieldDescriptor; 4]>,
    /// Variants (for sum types).
    pub variants: SmallVec<[VariantDescriptor; 4]>,
    /// Size in bytes (0 for generic types - computed at instantiation).
    pub size: u32,
    /// Alignment in bytes.
    pub alignment: u32,
    /// Drop function (if type needs cleanup).
    pub drop_fn: Option<u32>, // FunctionId.0
    /// Clone function (if type implements Clone).
    pub clone_fn: Option<u32>, // FunctionId.0
    /// Protocol implementations.
    pub protocols: SmallVec<[ProtocolImpl; 2]>,
    /// Visibility.
    pub visibility: Visibility,
}

impl Default for TypeDescriptor {
    fn default() -> Self {
        Self {
            id: TypeId::UNIT,
            name: StringId::EMPTY,
            kind: TypeKind::Unit,
            type_params: SmallVec::new(),
            fields: SmallVec::new(),
            variants: SmallVec::new(),
            size: 0,
            alignment: 1,
            drop_fn: None,
            clone_fn: None,
            protocols: SmallVec::new(),
            visibility: Visibility::Public,
        }
    }
}

/// Generic type parameter descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeParamDescriptor {
    /// Parameter name (T, K, V, etc.).
    pub name: StringId,
    /// Parameter ID (unique within function/type).
    pub id: TypeParamId,
    /// Protocol bounds (T: Ord + Clone).
    pub bounds: SmallVec<[ProtocolId; 2]>,
    /// Default type (if any).
    pub default: Option<TypeRef>,
    /// Variance.
    pub variance: Variance,
}

impl Default for TypeParamDescriptor {
    fn default() -> Self {
        Self {
            name: StringId::EMPTY,
            id: TypeParamId(0),
            bounds: SmallVec::new(),
            default: None,
            variance: Variance::Invariant,
        }
    }
}

/// Field descriptor for record types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDescriptor {
    /// Field name.
    pub name: StringId,
    /// Field type.
    pub type_ref: TypeRef,
    /// Offset within struct (0 for generic types).
    pub offset: u32,
    /// Field visibility.
    pub visibility: Visibility,
}

impl Default for FieldDescriptor {
    fn default() -> Self {
        Self {
            name: StringId::EMPTY,
            type_ref: TypeRef::Concrete(TypeId::UNIT),
            offset: 0,
            visibility: Visibility::Public,
        }
    }
}

/// Variant kind for sum types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[repr(u8)]
pub enum VariantKind {
    /// No payload: `None`, `Empty`.
    #[default]
    Unit = 0,
    /// Tuple payload: `Some(T)`, `Point(Int, Int)`.
    Tuple = 1,
    /// Record payload: `Node { left: T, right: T }`.
    Record = 2,
}

impl TryFrom<u8> for VariantKind {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(VariantKind::Unit),
            1 => Ok(VariantKind::Tuple),
            2 => Ok(VariantKind::Record),
            _ => Err(value),
        }
    }
}

/// Variant descriptor for sum types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantDescriptor {
    /// Variant name.
    pub name: StringId,
    /// Variant tag (discriminant).
    pub tag: u32,
    /// Payload type (None for unit variants).
    pub payload: Option<TypeRef>,
    /// Payload kind.
    pub kind: VariantKind,
    /// Tuple arity (for tuple variants).
    pub arity: u8,
    /// Fields (for record variants).
    pub fields: SmallVec<[FieldDescriptor; 4]>,
}

impl Default for VariantDescriptor {
    fn default() -> Self {
        Self {
            name: StringId::EMPTY,
            tag: 0,
            payload: None,
            kind: VariantKind::Unit,
            arity: 0,
            fields: SmallVec::new(),
        }
    }
}

/// Protocol implementation for a type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolImpl {
    /// Protocol being implemented.
    pub protocol: ProtocolId,
    /// Methods implementing the protocol (function IDs).
    pub methods: Vec<u32>, // FunctionId.0
}

// ============================================================================
// Computational Properties
// ============================================================================

bitflags! {
    /// Computational properties (inferred, not effects).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
    pub struct PropertySet: u16 {
        /// No side effects.
        const PURE = 0b0000_0001;
        /// Performs I/O.
        const IO = 0b0000_0010;
        /// Async function.
        const ASYNC = 0b0000_0100;
        /// Can fail (returns Result).
        const FALLIBLE = 0b0000_1000;
        /// Mutates references.
        const MUTATES = 0b0001_0000;
        /// Heap allocations.
        const ALLOCATES = 0b0010_0000;
        /// May not return (panic, loop).
        const DIVERGES = 0b0100_0000;
        /// Uses GPU operations.
        const GPU = 0b1000_0000;
        /// Generator function (fn*). Functions with this property use Yield to suspend
        /// execution and produce values lazily via the Iterator protocol.
        const GENERATOR = 0b0001_0000_0000;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // StringId Tests
    // ========================================================================

    #[test]
    fn test_string_id_empty_constant() {
        assert_eq!(StringId::EMPTY, StringId(0));
    }

    #[test]
    fn test_string_id_default() {
        assert_eq!(StringId::default(), StringId(0));
    }

    #[test]
    fn test_string_id_equality() {
        assert_eq!(StringId(42), StringId(42));
        assert_ne!(StringId(1), StringId(2));
    }

    #[test]
    fn test_string_id_clone() {
        let id = StringId(100);
        let cloned = id;
        assert_eq!(id, cloned);
    }

    #[test]
    fn test_string_id_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(StringId(1));
        set.insert(StringId(2));
        set.insert(StringId(1)); // duplicate
        assert_eq!(set.len(), 2);
    }

    // ========================================================================
    // TypeId Constants Tests
    // ========================================================================

    #[test]
    fn test_type_id_constants_values() {
        assert_eq!(TypeId::UNIT.0, 0);
        assert_eq!(TypeId::BOOL.0, 1);
        assert_eq!(TypeId::INT.0, 2);
        assert_eq!(TypeId::FLOAT.0, 3);
        assert_eq!(TypeId::TEXT.0, 4);
        assert_eq!(TypeId::NEVER.0, 5);
        assert_eq!(TypeId::U8.0, 6);
        assert_eq!(TypeId::U16.0, 7);
        assert_eq!(TypeId::U32.0, 8);
        assert_eq!(TypeId::U64.0, 9);
        assert_eq!(TypeId::I8.0, 10);
        assert_eq!(TypeId::I16.0, 11);
        assert_eq!(TypeId::I32.0, 12);
        assert_eq!(TypeId::F32.0, 13);
        assert_eq!(TypeId::PTR.0, 14);
        assert_eq!(TypeId::RESERVED.0, 15);
    }

    #[test]
    fn test_type_id_first_user() {
        assert_eq!(TypeId::FIRST_USER, 16);
    }

    #[test]
    fn test_builtin_types() {
        // All built-in types should return true
        assert!(TypeId::UNIT.is_builtin());
        assert!(TypeId::BOOL.is_builtin());
        assert!(TypeId::INT.is_builtin());
        assert!(TypeId::FLOAT.is_builtin());
        assert!(TypeId::TEXT.is_builtin());
        assert!(TypeId::NEVER.is_builtin());
        assert!(TypeId::U8.is_builtin());
        assert!(TypeId::U16.is_builtin());
        assert!(TypeId::U32.is_builtin());
        assert!(TypeId::U64.is_builtin());
        assert!(TypeId::I8.is_builtin());
        assert!(TypeId::I16.is_builtin());
        assert!(TypeId::I32.is_builtin());
        assert!(TypeId::F32.is_builtin());
        assert!(TypeId::PTR.is_builtin());
        assert!(TypeId::RESERVED.is_builtin());

        // User types should return false
        assert!(!TypeId(16).is_builtin());
        assert!(!TypeId(100).is_builtin());
        assert!(!TypeId(u32::MAX).is_builtin());
    }

    #[test]
    fn test_numeric_types() {
        // Integer types
        assert!(TypeId::INT.is_numeric());
        assert!(TypeId::U8.is_numeric());
        assert!(TypeId::U16.is_numeric());
        assert!(TypeId::U32.is_numeric());
        assert!(TypeId::U64.is_numeric());
        assert!(TypeId::I8.is_numeric());
        assert!(TypeId::I16.is_numeric());
        assert!(TypeId::I32.is_numeric());

        // Float types
        assert!(TypeId::FLOAT.is_numeric());
        assert!(TypeId::F32.is_numeric());

        // Non-numeric types
        assert!(!TypeId::UNIT.is_numeric());
        assert!(!TypeId::BOOL.is_numeric());
        assert!(!TypeId::TEXT.is_numeric());
        assert!(!TypeId::NEVER.is_numeric());
        assert!(!TypeId::PTR.is_numeric());
        assert!(!TypeId::RESERVED.is_numeric());
        assert!(!TypeId(100).is_numeric());
    }

    #[test]
    fn test_integer_types() {
        // Integer types
        assert!(TypeId::INT.is_integer());
        assert!(TypeId::U8.is_integer());
        assert!(TypeId::U16.is_integer());
        assert!(TypeId::U32.is_integer());
        assert!(TypeId::U64.is_integer());
        assert!(TypeId::I8.is_integer());
        assert!(TypeId::I16.is_integer());
        assert!(TypeId::I32.is_integer());

        // Non-integer types
        assert!(!TypeId::FLOAT.is_integer());
        assert!(!TypeId::F32.is_integer());
        assert!(!TypeId::UNIT.is_integer());
        assert!(!TypeId::BOOL.is_integer());
        assert!(!TypeId::TEXT.is_integer());
    }

    #[test]
    fn test_float_types() {
        assert!(TypeId::FLOAT.is_float());
        assert!(TypeId::F32.is_float());

        assert!(!TypeId::INT.is_float());
        assert!(!TypeId::U8.is_float());
        assert!(!TypeId::BOOL.is_float());
    }

    #[test]
    fn test_type_id_default() {
        assert_eq!(TypeId::default(), TypeId(0));
    }

    #[test]
    fn test_type_id_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(TypeId::INT);
        set.insert(TypeId::FLOAT);
        set.insert(TypeId::INT); // duplicate
        assert_eq!(set.len(), 2);
    }

    // ========================================================================
    // TypeParamId Tests
    // ========================================================================

    #[test]
    fn test_type_param_id_default() {
        assert_eq!(TypeParamId::default(), TypeParamId(0));
    }

    #[test]
    fn test_type_param_id_equality() {
        assert_eq!(TypeParamId(0), TypeParamId(0));
        assert_ne!(TypeParamId(0), TypeParamId(1));
    }

    // ========================================================================
    // ProtocolId Tests
    // ========================================================================

    #[test]
    fn test_protocol_id_default() {
        assert_eq!(ProtocolId::default(), ProtocolId(0));
    }

    #[test]
    fn test_protocol_id_equality() {
        assert_eq!(ProtocolId(42), ProtocolId(42));
        assert_ne!(ProtocolId(1), ProtocolId(2));
    }

    // ========================================================================
    // ContextRef Tests
    // ========================================================================

    #[test]
    fn test_context_ref_default() {
        assert_eq!(ContextRef::default(), ContextRef(0));
    }

    #[test]
    fn test_context_ref_equality() {
        assert_eq!(ContextRef(10), ContextRef(10));
        assert_ne!(ContextRef(1), ContextRef(2));
    }

    // ========================================================================
    // TypeRef Variants Tests
    // ========================================================================

    #[test]
    fn test_type_ref_concrete() {
        let tr = TypeRef::Concrete(TypeId::INT);
        assert_eq!(tr, TypeRef::concrete(TypeId::INT));
        assert!(!tr.is_generic());
        assert_eq!(tr.base_type_id(), Some(TypeId::INT));
    }

    #[test]
    fn test_type_ref_generic() {
        let tr = TypeRef::Generic(TypeParamId(0));
        assert_eq!(tr, TypeRef::generic(TypeParamId(0)));
        assert!(tr.is_generic());
        assert_eq!(tr.base_type_id(), None);
    }

    #[test]
    fn test_type_ref_instantiated() {
        let tr = TypeRef::instantiated(TypeId(100), vec![TypeRef::Concrete(TypeId::INT)]);
        match &tr {
            TypeRef::Instantiated { base, args } => {
                assert_eq!(*base, TypeId(100));
                assert_eq!(args.len(), 1);
                assert_eq!(args[0], TypeRef::Concrete(TypeId::INT));
            }
            _ => panic!("Expected Instantiated variant"),
        }
        assert!(!tr.is_generic());
        assert_eq!(tr.base_type_id(), Some(TypeId(100)));
    }

    #[test]
    fn test_type_ref_instantiated_with_generic_args() {
        let tr = TypeRef::Instantiated {
            base: TypeId(100),
            args: vec![TypeRef::Generic(TypeParamId(0))],
        };
        assert!(tr.is_generic());
    }

    #[test]
    fn test_type_ref_function() {
        let tr = TypeRef::Function {
            params: vec![TypeRef::Concrete(TypeId::INT)],
            return_type: Box::new(TypeRef::Concrete(TypeId::BOOL)),
            contexts: SmallVec::new(),
        };
        assert!(!tr.is_generic());
        assert_eq!(tr.base_type_id(), None);
    }

    #[test]
    fn test_type_ref_function_with_generic_param() {
        let tr = TypeRef::Function {
            params: vec![TypeRef::Generic(TypeParamId(0))],
            return_type: Box::new(TypeRef::Concrete(TypeId::BOOL)),
            contexts: SmallVec::new(),
        };
        assert!(tr.is_generic());
    }

    #[test]
    fn test_type_ref_function_with_generic_return() {
        let tr = TypeRef::Function {
            params: vec![TypeRef::Concrete(TypeId::INT)],
            return_type: Box::new(TypeRef::Generic(TypeParamId(0))),
            contexts: SmallVec::new(),
        };
        assert!(tr.is_generic());
    }

    #[test]
    fn test_type_ref_function_with_contexts() {
        let mut contexts = SmallVec::new();
        contexts.push(ContextRef(1));
        contexts.push(ContextRef(2));
        let tr = TypeRef::Function {
            params: vec![],
            return_type: Box::new(TypeRef::Concrete(TypeId::UNIT)),
            contexts,
        };
        match &tr {
            TypeRef::Function { contexts, .. } => {
                assert_eq!(contexts.len(), 2);
            }
            _ => panic!("Expected Function variant"),
        }
    }

    #[test]
    fn test_type_ref_reference() {
        let tr = TypeRef::reference(
            TypeRef::Concrete(TypeId::INT),
            Mutability::Immutable,
            CbgrTier::Tier0,
        );
        match &tr {
            TypeRef::Reference {
                inner,
                mutability,
                tier,
            } => {
                assert_eq!(**inner, TypeRef::Concrete(TypeId::INT));
                assert_eq!(*mutability, Mutability::Immutable);
                assert_eq!(*tier, CbgrTier::Tier0);
            }
            _ => panic!("Expected Reference variant"),
        }
        assert!(!tr.is_generic());
        assert_eq!(tr.base_type_id(), None);
    }

    #[test]
    fn test_type_ref_reference_with_generic_inner() {
        let tr = TypeRef::Reference {
            inner: Box::new(TypeRef::Generic(TypeParamId(0))),
            mutability: Mutability::Mutable,
            tier: CbgrTier::Tier1,
        };
        assert!(tr.is_generic());
    }

    #[test]
    fn test_type_ref_tuple() {
        let tr = TypeRef::Tuple(vec![
            TypeRef::Concrete(TypeId::INT),
            TypeRef::Concrete(TypeId::FLOAT),
        ]);
        assert!(!tr.is_generic());
        assert_eq!(tr.base_type_id(), None);
    }

    #[test]
    fn test_type_ref_tuple_empty() {
        let tr = TypeRef::Tuple(vec![]);
        assert!(!tr.is_generic());
    }

    #[test]
    fn test_type_ref_tuple_with_generic() {
        let tr = TypeRef::Tuple(vec![
            TypeRef::Concrete(TypeId::INT),
            TypeRef::Generic(TypeParamId(0)),
        ]);
        assert!(tr.is_generic());
    }

    #[test]
    fn test_type_ref_array() {
        let tr = TypeRef::Array {
            element: Box::new(TypeRef::Concrete(TypeId::INT)),
            length: 10,
        };
        assert!(!tr.is_generic());
        assert_eq!(tr.base_type_id(), None);
    }

    #[test]
    fn test_type_ref_array_with_generic_element() {
        let tr = TypeRef::Array {
            element: Box::new(TypeRef::Generic(TypeParamId(0))),
            length: 5,
        };
        assert!(tr.is_generic());
    }

    #[test]
    fn test_type_ref_slice() {
        let tr = TypeRef::Slice(Box::new(TypeRef::Concrete(TypeId::U8)));
        assert!(!tr.is_generic());
        assert_eq!(tr.base_type_id(), None);
    }

    #[test]
    fn test_type_ref_slice_with_generic() {
        let tr = TypeRef::Slice(Box::new(TypeRef::Generic(TypeParamId(0))));
        assert!(tr.is_generic());
    }

    #[test]
    fn test_type_ref_default() {
        let tr = TypeRef::default();
        assert_eq!(tr, TypeRef::Concrete(TypeId::UNIT));
    }

    #[test]
    fn test_type_ref_is_generic() {
        let concrete = TypeRef::Concrete(TypeId::INT);
        assert!(!concrete.is_generic());

        let generic = TypeRef::Generic(TypeParamId(0));
        assert!(generic.is_generic());

        let instantiated = TypeRef::Instantiated {
            base: TypeId(100),
            args: vec![TypeRef::Concrete(TypeId::INT)],
        };
        assert!(!instantiated.is_generic());

        let generic_instantiated = TypeRef::Instantiated {
            base: TypeId(100),
            args: vec![TypeRef::Generic(TypeParamId(0))],
        };
        assert!(generic_instantiated.is_generic());
    }

    #[test]
    fn test_type_ref_nested_generics() {
        // List<Map<K, V>> where K and V are generic
        let inner = TypeRef::Instantiated {
            base: TypeId(101), // Map
            args: vec![
                TypeRef::Generic(TypeParamId(0)),
                TypeRef::Generic(TypeParamId(1)),
            ],
        };
        let outer = TypeRef::Instantiated {
            base: TypeId(100), // List
            args: vec![inner],
        };
        assert!(outer.is_generic());
    }

    #[test]
    fn test_type_ref_equality() {
        let a = TypeRef::Concrete(TypeId::INT);
        let b = TypeRef::Concrete(TypeId::INT);
        let c = TypeRef::Concrete(TypeId::FLOAT);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_type_ref_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(TypeRef::Concrete(TypeId::INT));
        set.insert(TypeRef::Concrete(TypeId::FLOAT));
        set.insert(TypeRef::Concrete(TypeId::INT)); // duplicate
        assert_eq!(set.len(), 2);
    }

    // ========================================================================
    // Mutability Tests
    // ========================================================================

    #[test]
    fn test_mutability_values() {
        assert_eq!(Mutability::Immutable as u8, 0);
        assert_eq!(Mutability::Mutable as u8, 1);
    }

    #[test]
    fn test_mutability_default() {
        assert_eq!(Mutability::default(), Mutability::Immutable);
    }

    #[test]
    fn test_mutability_equality() {
        assert_eq!(Mutability::Immutable, Mutability::Immutable);
        assert_eq!(Mutability::Mutable, Mutability::Mutable);
        assert_ne!(Mutability::Immutable, Mutability::Mutable);
    }

    #[test]
    fn test_mutability_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Mutability::Immutable);
        set.insert(Mutability::Mutable);
        assert_eq!(set.len(), 2);
    }

    // ========================================================================
    // CbgrTier Tests
    // ========================================================================

    #[test]
    fn test_cbgr_tier_values() {
        assert_eq!(CbgrTier::Tier0 as u8, 0);
        assert_eq!(CbgrTier::Tier1 as u8, 1);
        assert_eq!(CbgrTier::Tier2 as u8, 2);
    }

    #[test]
    fn test_cbgr_tier() {
        assert_eq!(CbgrTier::default(), CbgrTier::Tier0);
        assert_eq!(CbgrTier::Tier0 as u8, 0);
        assert_eq!(CbgrTier::Tier1 as u8, 1);
        assert_eq!(CbgrTier::Tier2 as u8, 2);
    }

    #[test]
    fn test_cbgr_tier_equality() {
        assert_eq!(CbgrTier::Tier0, CbgrTier::Tier0);
        assert_ne!(CbgrTier::Tier0, CbgrTier::Tier1);
        assert_ne!(CbgrTier::Tier1, CbgrTier::Tier2);
    }

    #[test]
    fn test_cbgr_tier_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(CbgrTier::Tier0);
        set.insert(CbgrTier::Tier1);
        set.insert(CbgrTier::Tier2);
        assert_eq!(set.len(), 3);
    }

    // ========================================================================
    // TypeKind Tests
    // ========================================================================

    #[test]
    fn test_type_kind_values() {
        assert_eq!(TypeKind::Primitive as u8, 0);
        assert_eq!(TypeKind::Record as u8, 1);
        assert_eq!(TypeKind::Sum as u8, 2);
        assert_eq!(TypeKind::Protocol as u8, 3);
        assert_eq!(TypeKind::Newtype as u8, 4);
        assert_eq!(TypeKind::Tuple as u8, 5);
        assert_eq!(TypeKind::Unit as u8, 6);
        assert_eq!(TypeKind::Array as u8, 7);
        assert_eq!(TypeKind::Tensor as u8, 8);
    }

    #[test]
    fn test_type_kind_default() {
        assert_eq!(TypeKind::default(), TypeKind::Primitive);
    }

    #[test]
    fn test_type_kind_try_from_valid() {
        assert_eq!(TypeKind::try_from(0), Ok(TypeKind::Primitive));
        assert_eq!(TypeKind::try_from(1), Ok(TypeKind::Record));
        assert_eq!(TypeKind::try_from(2), Ok(TypeKind::Sum));
        assert_eq!(TypeKind::try_from(3), Ok(TypeKind::Protocol));
        assert_eq!(TypeKind::try_from(4), Ok(TypeKind::Newtype));
        assert_eq!(TypeKind::try_from(5), Ok(TypeKind::Tuple));
        assert_eq!(TypeKind::try_from(6), Ok(TypeKind::Unit));
        assert_eq!(TypeKind::try_from(7), Ok(TypeKind::Array));
        assert_eq!(TypeKind::try_from(8), Ok(TypeKind::Tensor));
    }

    #[test]
    fn test_type_kind_try_from_invalid() {
        assert_eq!(TypeKind::try_from(9), Err(9));
        assert_eq!(TypeKind::try_from(100), Err(100));
        assert_eq!(TypeKind::try_from(255), Err(255));
    }

    #[test]
    fn test_type_kind_equality() {
        assert_eq!(TypeKind::Record, TypeKind::Record);
        assert_ne!(TypeKind::Record, TypeKind::Sum);
    }

    #[test]
    fn test_type_kind_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(TypeKind::Primitive);
        set.insert(TypeKind::Record);
        set.insert(TypeKind::Sum);
        assert_eq!(set.len(), 3);
    }

    // ========================================================================
    // VariantKind Tests
    // ========================================================================

    #[test]
    fn test_variant_kind_values() {
        assert_eq!(VariantKind::Unit as u8, 0);
        assert_eq!(VariantKind::Tuple as u8, 1);
        assert_eq!(VariantKind::Record as u8, 2);
    }

    #[test]
    fn test_variant_kind_default() {
        assert_eq!(VariantKind::default(), VariantKind::Unit);
    }

    #[test]
    fn test_variant_kind_try_from_valid() {
        assert_eq!(VariantKind::try_from(0), Ok(VariantKind::Unit));
        assert_eq!(VariantKind::try_from(1), Ok(VariantKind::Tuple));
        assert_eq!(VariantKind::try_from(2), Ok(VariantKind::Record));
    }

    #[test]
    fn test_variant_kind_try_from_invalid() {
        assert_eq!(VariantKind::try_from(3), Err(3));
        assert_eq!(VariantKind::try_from(100), Err(100));
        assert_eq!(VariantKind::try_from(255), Err(255));
    }

    #[test]
    fn test_variant_kind_equality() {
        assert_eq!(VariantKind::Tuple, VariantKind::Tuple);
        assert_ne!(VariantKind::Tuple, VariantKind::Record);
    }

    #[test]
    fn test_variant_kind_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(VariantKind::Unit);
        set.insert(VariantKind::Tuple);
        set.insert(VariantKind::Record);
        assert_eq!(set.len(), 3);
    }

    // ========================================================================
    // Visibility Tests
    // ========================================================================

    #[test]
    fn test_visibility_values() {
        assert_eq!(Visibility::Private as u8, 0);
        assert_eq!(Visibility::Public as u8, 1);
        assert_eq!(Visibility::Cog as u8, 2);
    }

    #[test]
    fn test_visibility_default() {
        assert_eq!(Visibility::default(), Visibility::Public);
    }

    #[test]
    fn test_visibility_try_from_valid() {
        assert_eq!(Visibility::try_from(0), Ok(Visibility::Private));
        assert_eq!(Visibility::try_from(1), Ok(Visibility::Public));
        assert_eq!(Visibility::try_from(2), Ok(Visibility::Cog));
    }

    #[test]
    fn test_visibility_try_from_invalid() {
        assert_eq!(Visibility::try_from(3), Err(3));
        assert_eq!(Visibility::try_from(100), Err(100));
        assert_eq!(Visibility::try_from(255), Err(255));
    }

    #[test]
    fn test_visibility_equality() {
        assert_eq!(Visibility::Public, Visibility::Public);
        assert_ne!(Visibility::Public, Visibility::Private);
    }

    #[test]
    fn test_visibility_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Visibility::Private);
        set.insert(Visibility::Public);
        set.insert(Visibility::Cog);
        assert_eq!(set.len(), 3);
    }

    // ========================================================================
    // Variance Tests
    // ========================================================================

    #[test]
    fn test_variance_values() {
        assert_eq!(Variance::Covariant as u8, 0);
        assert_eq!(Variance::Contravariant as u8, 1);
        assert_eq!(Variance::Invariant as u8, 2);
    }

    #[test]
    fn test_variance_default() {
        assert_eq!(Variance::default(), Variance::Invariant);
    }

    #[test]
    fn test_variance_try_from_valid() {
        assert_eq!(Variance::try_from(0), Ok(Variance::Covariant));
        assert_eq!(Variance::try_from(1), Ok(Variance::Contravariant));
        assert_eq!(Variance::try_from(2), Ok(Variance::Invariant));
    }

    #[test]
    fn test_variance_try_from_invalid() {
        assert_eq!(Variance::try_from(3), Err(3));
        assert_eq!(Variance::try_from(100), Err(100));
        assert_eq!(Variance::try_from(255), Err(255));
    }

    #[test]
    fn test_variance_equality() {
        assert_eq!(Variance::Covariant, Variance::Covariant);
        assert_ne!(Variance::Covariant, Variance::Contravariant);
    }

    #[test]
    fn test_variance_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Variance::Covariant);
        set.insert(Variance::Contravariant);
        set.insert(Variance::Invariant);
        assert_eq!(set.len(), 3);
    }

    // ========================================================================
    // PropertySet Tests
    // ========================================================================

    #[test]
    fn test_property_set_individual_flags() {
        assert_eq!(PropertySet::PURE.bits(), 0b0000_0001);
        assert_eq!(PropertySet::IO.bits(), 0b0000_0010);
        assert_eq!(PropertySet::ASYNC.bits(), 0b0000_0100);
        assert_eq!(PropertySet::FALLIBLE.bits(), 0b0000_1000);
        assert_eq!(PropertySet::MUTATES.bits(), 0b0001_0000);
        assert_eq!(PropertySet::ALLOCATES.bits(), 0b0010_0000);
        assert_eq!(PropertySet::DIVERGES.bits(), 0b0100_0000);
        assert_eq!(PropertySet::GPU.bits(), 0b1000_0000);
    }

    #[test]
    fn test_property_set_default() {
        assert_eq!(PropertySet::default(), PropertySet::empty());
        assert!(PropertySet::default().is_empty());
    }

    #[test]
    fn test_property_set() {
        let props = PropertySet::PURE | PropertySet::ALLOCATES;
        assert!(props.contains(PropertySet::PURE));
        assert!(props.contains(PropertySet::ALLOCATES));
        assert!(!props.contains(PropertySet::IO));
    }

    #[test]
    fn test_property_set_union() {
        let a = PropertySet::PURE | PropertySet::IO;
        let b = PropertySet::ASYNC | PropertySet::FALLIBLE;
        let union = a | b;
        assert!(union.contains(PropertySet::PURE));
        assert!(union.contains(PropertySet::IO));
        assert!(union.contains(PropertySet::ASYNC));
        assert!(union.contains(PropertySet::FALLIBLE));
    }

    #[test]
    fn test_property_set_intersection() {
        let a = PropertySet::PURE | PropertySet::IO | PropertySet::ASYNC;
        let b = PropertySet::IO | PropertySet::ASYNC | PropertySet::FALLIBLE;
        let intersection = a & b;
        assert!(!intersection.contains(PropertySet::PURE));
        assert!(intersection.contains(PropertySet::IO));
        assert!(intersection.contains(PropertySet::ASYNC));
        assert!(!intersection.contains(PropertySet::FALLIBLE));
    }

    #[test]
    fn test_property_set_difference() {
        let a = PropertySet::PURE | PropertySet::IO | PropertySet::ASYNC;
        let b = PropertySet::IO;
        let diff = a - b;
        assert!(diff.contains(PropertySet::PURE));
        assert!(!diff.contains(PropertySet::IO));
        assert!(diff.contains(PropertySet::ASYNC));
    }

    #[test]
    fn test_property_set_complement() {
        let a = PropertySet::PURE;
        let complement = !a;
        assert!(!complement.contains(PropertySet::PURE));
        assert!(complement.contains(PropertySet::IO));
        assert!(complement.contains(PropertySet::ASYNC));
    }

    #[test]
    fn test_property_set_all_flags() {
        let all = PropertySet::PURE
            | PropertySet::IO
            | PropertySet::ASYNC
            | PropertySet::FALLIBLE
            | PropertySet::MUTATES
            | PropertySet::ALLOCATES
            | PropertySet::DIVERGES
            | PropertySet::GPU;
        assert!(all.contains(PropertySet::PURE));
        assert!(all.contains(PropertySet::IO));
        assert!(all.contains(PropertySet::ASYNC));
        assert!(all.contains(PropertySet::FALLIBLE));
        assert!(all.contains(PropertySet::MUTATES));
        assert!(all.contains(PropertySet::ALLOCATES));
        assert!(all.contains(PropertySet::DIVERGES));
        assert!(all.contains(PropertySet::GPU));
    }

    #[test]
    fn test_property_set_empty() {
        let empty = PropertySet::empty();
        assert!(empty.is_empty());
        assert!(!empty.contains(PropertySet::PURE));
    }

    #[test]
    fn test_property_set_insert_remove() {
        let mut props = PropertySet::empty();
        props.insert(PropertySet::PURE);
        assert!(props.contains(PropertySet::PURE));
        props.remove(PropertySet::PURE);
        assert!(!props.contains(PropertySet::PURE));
    }

    #[test]
    fn test_property_set_toggle() {
        let mut props = PropertySet::PURE;
        props.toggle(PropertySet::PURE);
        assert!(!props.contains(PropertySet::PURE));
        props.toggle(PropertySet::PURE);
        assert!(props.contains(PropertySet::PURE));
    }

    #[test]
    fn test_property_set_equality() {
        let a = PropertySet::PURE | PropertySet::IO;
        let b = PropertySet::PURE | PropertySet::IO;
        let c = PropertySet::PURE;
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_property_set_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(PropertySet::PURE);
        set.insert(PropertySet::IO);
        set.insert(PropertySet::PURE); // duplicate
        assert_eq!(set.len(), 2);
    }

    // ========================================================================
    // TypeDescriptor Tests
    // ========================================================================

    #[test]
    fn test_type_descriptor_default() {
        let td = TypeDescriptor::default();
        assert_eq!(td.id, TypeId::UNIT);
        assert_eq!(td.name, StringId::EMPTY);
        assert_eq!(td.kind, TypeKind::Unit);
        assert!(td.type_params.is_empty());
        assert!(td.fields.is_empty());
        assert!(td.variants.is_empty());
        assert_eq!(td.size, 0);
        assert_eq!(td.alignment, 1);
        assert!(td.drop_fn.is_none());
        assert!(td.clone_fn.is_none());
        assert!(td.protocols.is_empty());
        assert_eq!(td.visibility, Visibility::Public);
    }

    #[test]
    fn test_type_descriptor_with_fields() {
        let mut td = TypeDescriptor::default();
        td.kind = TypeKind::Record;
        td.fields.push(FieldDescriptor {
            name: StringId(1),
            type_ref: TypeRef::Concrete(TypeId::INT),
            offset: 0,
            visibility: Visibility::Public,
        });
        td.fields.push(FieldDescriptor {
            name: StringId(2),
            type_ref: TypeRef::Concrete(TypeId::FLOAT),
            offset: 8,
            visibility: Visibility::Private,
        });
        assert_eq!(td.fields.len(), 2);
    }

    #[test]
    fn test_type_descriptor_with_variants() {
        let mut td = TypeDescriptor::default();
        td.kind = TypeKind::Sum;
        td.variants.push(VariantDescriptor {
            name: StringId(1),
            tag: 0,
            payload: None,
            kind: VariantKind::Unit,
            arity: 0,
            fields: SmallVec::new(),
        });
        td.variants.push(VariantDescriptor {
            name: StringId(2),
            tag: 1,
            payload: Some(TypeRef::Concrete(TypeId::INT)),
            kind: VariantKind::Tuple,
            arity: 1,
            fields: SmallVec::new(),
        });
        assert_eq!(td.variants.len(), 2);
    }

    #[test]
    fn test_type_descriptor_with_type_params() {
        let mut td = TypeDescriptor::default();
        td.type_params.push(TypeParamDescriptor {
            name: StringId(10),
            id: TypeParamId(0),
            bounds: SmallVec::new(),
            default: None,
            variance: Variance::Covariant,
        });
        assert_eq!(td.type_params.len(), 1);
    }

    #[test]
    fn test_type_descriptor_with_protocols() {
        let mut td = TypeDescriptor::default();
        td.protocols.push(ProtocolImpl {
            protocol: ProtocolId(100),
            methods: vec![1, 2, 3],
        });
        assert_eq!(td.protocols.len(), 1);
        assert_eq!(td.protocols[0].methods.len(), 3);
    }

    #[test]
    fn test_type_descriptor_equality() {
        let a = TypeDescriptor::default();
        let b = TypeDescriptor::default();
        assert_eq!(a, b);
    }

    // ========================================================================
    // TypeParamDescriptor Tests
    // ========================================================================

    #[test]
    fn test_type_param_descriptor_default() {
        let tpd = TypeParamDescriptor::default();
        assert_eq!(tpd.name, StringId::EMPTY);
        assert_eq!(tpd.id, TypeParamId(0));
        assert!(tpd.bounds.is_empty());
        assert!(tpd.default.is_none());
        assert_eq!(tpd.variance, Variance::Invariant);
    }

    #[test]
    fn test_type_param_descriptor_with_bounds() {
        let mut tpd = TypeParamDescriptor::default();
        tpd.bounds.push(ProtocolId(1));
        tpd.bounds.push(ProtocolId(2));
        assert_eq!(tpd.bounds.len(), 2);
    }

    #[test]
    fn test_type_param_descriptor_with_default() {
        let mut tpd = TypeParamDescriptor::default();
        tpd.default = Some(TypeRef::Concrete(TypeId::INT));
        assert!(tpd.default.is_some());
    }

    // ========================================================================
    // FieldDescriptor Tests
    // ========================================================================

    #[test]
    fn test_field_descriptor_default() {
        let fd = FieldDescriptor::default();
        assert_eq!(fd.name, StringId::EMPTY);
        assert_eq!(fd.type_ref, TypeRef::Concrete(TypeId::UNIT));
        assert_eq!(fd.offset, 0);
        assert_eq!(fd.visibility, Visibility::Public);
    }

    #[test]
    fn test_field_descriptor_custom() {
        let fd = FieldDescriptor {
            name: StringId(42),
            type_ref: TypeRef::Concrete(TypeId::TEXT),
            offset: 16,
            visibility: Visibility::Private,
        };
        assert_eq!(fd.name, StringId(42));
        assert_eq!(fd.offset, 16);
        assert_eq!(fd.visibility, Visibility::Private);
    }

    // ========================================================================
    // VariantDescriptor Tests
    // ========================================================================

    #[test]
    fn test_variant_descriptor_default() {
        let vd = VariantDescriptor::default();
        assert_eq!(vd.name, StringId::EMPTY);
        assert_eq!(vd.tag, 0);
        assert!(vd.payload.is_none());
        assert_eq!(vd.kind, VariantKind::Unit);
        assert_eq!(vd.arity, 0);
        assert!(vd.fields.is_empty());
    }

    #[test]
    fn test_variant_descriptor_unit() {
        let vd = VariantDescriptor {
            name: StringId(1),
            tag: 0,
            payload: None,
            kind: VariantKind::Unit,
            arity: 0,
            fields: SmallVec::new(),
        };
        assert_eq!(vd.kind, VariantKind::Unit);
        assert!(vd.payload.is_none());
    }

    #[test]
    fn test_variant_descriptor_tuple() {
        let vd = VariantDescriptor {
            name: StringId(2),
            tag: 1,
            payload: Some(TypeRef::Tuple(vec![
                TypeRef::Concrete(TypeId::INT),
                TypeRef::Concrete(TypeId::FLOAT),
            ])),
            kind: VariantKind::Tuple,
            arity: 2,
            fields: SmallVec::new(),
        };
        assert_eq!(vd.kind, VariantKind::Tuple);
        assert_eq!(vd.arity, 2);
    }

    #[test]
    fn test_variant_descriptor_record() {
        let mut fields = SmallVec::new();
        fields.push(FieldDescriptor {
            name: StringId(10),
            type_ref: TypeRef::Concrete(TypeId::INT),
            offset: 0,
            visibility: Visibility::Public,
        });
        let vd = VariantDescriptor {
            name: StringId(3),
            tag: 2,
            payload: None,
            kind: VariantKind::Record,
            arity: 0,
            fields,
        };
        assert_eq!(vd.kind, VariantKind::Record);
        assert_eq!(vd.fields.len(), 1);
    }

    // ========================================================================
    // ProtocolImpl Tests
    // ========================================================================

    #[test]
    fn test_protocol_impl() {
        let pi = ProtocolImpl {
            protocol: ProtocolId(50),
            methods: vec![100, 101, 102],
        };
        assert_eq!(pi.protocol, ProtocolId(50));
        assert_eq!(pi.methods.len(), 3);
    }

    #[test]
    fn test_protocol_impl_empty_methods() {
        let pi = ProtocolImpl {
            protocol: ProtocolId(1),
            methods: vec![],
        };
        assert!(pi.methods.is_empty());
    }

    // ========================================================================
    // Serde Serialization Tests
    // ========================================================================

    #[test]
    fn test_string_id_serde() {
        let id = StringId(42);
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: StringId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn test_type_id_serde() {
        let id = TypeId::INT;
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: TypeId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn test_type_param_id_serde() {
        let id = TypeParamId(5);
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: TypeParamId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn test_protocol_id_serde() {
        let id = ProtocolId(100);
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: ProtocolId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn test_context_ref_serde() {
        let cr = ContextRef(25);
        let json = serde_json::to_string(&cr).unwrap();
        let deserialized: ContextRef = serde_json::from_str(&json).unwrap();
        assert_eq!(cr, deserialized);
    }

    #[test]
    fn test_type_ref_concrete_serde() {
        let tr = TypeRef::Concrete(TypeId::INT);
        let json = serde_json::to_string(&tr).unwrap();
        let deserialized: TypeRef = serde_json::from_str(&json).unwrap();
        assert_eq!(tr, deserialized);
    }

    #[test]
    fn test_type_ref_generic_serde() {
        let tr = TypeRef::Generic(TypeParamId(0));
        let json = serde_json::to_string(&tr).unwrap();
        let deserialized: TypeRef = serde_json::from_str(&json).unwrap();
        assert_eq!(tr, deserialized);
    }

    #[test]
    fn test_type_ref_instantiated_serde() {
        let tr = TypeRef::Instantiated {
            base: TypeId(100),
            args: vec![TypeRef::Concrete(TypeId::INT), TypeRef::Concrete(TypeId::FLOAT)],
        };
        let json = serde_json::to_string(&tr).unwrap();
        let deserialized: TypeRef = serde_json::from_str(&json).unwrap();
        assert_eq!(tr, deserialized);
    }

    #[test]
    fn test_type_ref_function_serde() {
        let mut contexts = SmallVec::new();
        contexts.push(ContextRef(1));
        let tr = TypeRef::Function {
            params: vec![TypeRef::Concrete(TypeId::INT)],
            return_type: Box::new(TypeRef::Concrete(TypeId::BOOL)),
            contexts,
        };
        let json = serde_json::to_string(&tr).unwrap();
        let deserialized: TypeRef = serde_json::from_str(&json).unwrap();
        assert_eq!(tr, deserialized);
    }

    #[test]
    fn test_type_ref_reference_serde() {
        let tr = TypeRef::Reference {
            inner: Box::new(TypeRef::Concrete(TypeId::INT)),
            mutability: Mutability::Mutable,
            tier: CbgrTier::Tier1,
        };
        let json = serde_json::to_string(&tr).unwrap();
        let deserialized: TypeRef = serde_json::from_str(&json).unwrap();
        assert_eq!(tr, deserialized);
    }

    #[test]
    fn test_type_ref_tuple_serde() {
        let tr = TypeRef::Tuple(vec![
            TypeRef::Concrete(TypeId::INT),
            TypeRef::Concrete(TypeId::FLOAT),
        ]);
        let json = serde_json::to_string(&tr).unwrap();
        let deserialized: TypeRef = serde_json::from_str(&json).unwrap();
        assert_eq!(tr, deserialized);
    }

    #[test]
    fn test_type_ref_array_serde() {
        let tr = TypeRef::Array {
            element: Box::new(TypeRef::Concrete(TypeId::U8)),
            length: 256,
        };
        let json = serde_json::to_string(&tr).unwrap();
        let deserialized: TypeRef = serde_json::from_str(&json).unwrap();
        assert_eq!(tr, deserialized);
    }

    #[test]
    fn test_type_ref_slice_serde() {
        let tr = TypeRef::Slice(Box::new(TypeRef::Concrete(TypeId::U8)));
        let json = serde_json::to_string(&tr).unwrap();
        let deserialized: TypeRef = serde_json::from_str(&json).unwrap();
        assert_eq!(tr, deserialized);
    }

    #[test]
    fn test_mutability_serde() {
        for m in [Mutability::Immutable, Mutability::Mutable] {
            let json = serde_json::to_string(&m).unwrap();
            let deserialized: Mutability = serde_json::from_str(&json).unwrap();
            assert_eq!(m, deserialized);
        }
    }

    #[test]
    fn test_cbgr_tier_serde() {
        for tier in [CbgrTier::Tier0, CbgrTier::Tier1, CbgrTier::Tier2] {
            let json = serde_json::to_string(&tier).unwrap();
            let deserialized: CbgrTier = serde_json::from_str(&json).unwrap();
            assert_eq!(tier, deserialized);
        }
    }

    #[test]
    fn test_type_kind_serde() {
        for kind in [
            TypeKind::Primitive,
            TypeKind::Record,
            TypeKind::Sum,
            TypeKind::Protocol,
            TypeKind::Newtype,
            TypeKind::Tuple,
            TypeKind::Unit,
            TypeKind::Array,
            TypeKind::Tensor,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let deserialized: TypeKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, deserialized);
        }
    }

    #[test]
    fn test_visibility_serde() {
        for v in [Visibility::Private, Visibility::Public, Visibility::Cog] {
            let json = serde_json::to_string(&v).unwrap();
            let deserialized: Visibility = serde_json::from_str(&json).unwrap();
            assert_eq!(v, deserialized);
        }
    }

    #[test]
    fn test_variance_serde() {
        for v in [Variance::Covariant, Variance::Contravariant, Variance::Invariant] {
            let json = serde_json::to_string(&v).unwrap();
            let deserialized: Variance = serde_json::from_str(&json).unwrap();
            assert_eq!(v, deserialized);
        }
    }

    #[test]
    fn test_variant_kind_serde() {
        for vk in [VariantKind::Unit, VariantKind::Tuple, VariantKind::Record] {
            let json = serde_json::to_string(&vk).unwrap();
            let deserialized: VariantKind = serde_json::from_str(&json).unwrap();
            assert_eq!(vk, deserialized);
        }
    }

    #[test]
    fn test_property_set_serde() {
        let props = PropertySet::PURE | PropertySet::ASYNC | PropertySet::FALLIBLE;
        let json = serde_json::to_string(&props).unwrap();
        let deserialized: PropertySet = serde_json::from_str(&json).unwrap();
        assert_eq!(props, deserialized);
    }

    #[test]
    fn test_type_descriptor_serde() {
        let mut td = TypeDescriptor::default();
        td.id = TypeId(42);
        td.name = StringId(10);
        td.kind = TypeKind::Record;
        td.size = 16;
        td.alignment = 8;
        td.fields.push(FieldDescriptor {
            name: StringId(1),
            type_ref: TypeRef::Concrete(TypeId::INT),
            offset: 0,
            visibility: Visibility::Public,
        });

        let json = serde_json::to_string(&td).unwrap();
        let deserialized: TypeDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(td, deserialized);
    }

    #[test]
    fn test_type_param_descriptor_serde() {
        let mut tpd = TypeParamDescriptor::default();
        tpd.name = StringId(5);
        tpd.id = TypeParamId(0);
        tpd.variance = Variance::Covariant;
        tpd.bounds.push(ProtocolId(1));

        let json = serde_json::to_string(&tpd).unwrap();
        let deserialized: TypeParamDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(tpd, deserialized);
    }

    #[test]
    fn test_field_descriptor_serde() {
        let fd = FieldDescriptor {
            name: StringId(42),
            type_ref: TypeRef::Concrete(TypeId::TEXT),
            offset: 8,
            visibility: Visibility::Private,
        };

        let json = serde_json::to_string(&fd).unwrap();
        let deserialized: FieldDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(fd, deserialized);
    }

    #[test]
    fn test_variant_descriptor_serde() {
        let vd = VariantDescriptor {
            name: StringId(1),
            tag: 0,
            payload: Some(TypeRef::Concrete(TypeId::INT)),
            kind: VariantKind::Tuple,
            arity: 1,
            fields: SmallVec::new(),
        };

        let json = serde_json::to_string(&vd).unwrap();
        let deserialized: VariantDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(vd, deserialized);
    }

    #[test]
    fn test_protocol_impl_serde() {
        let pi = ProtocolImpl {
            protocol: ProtocolId(50),
            methods: vec![1, 2, 3],
        };

        let json = serde_json::to_string(&pi).unwrap();
        let deserialized: ProtocolImpl = serde_json::from_str(&json).unwrap();
        assert_eq!(pi, deserialized);
    }

    // ========================================================================
    // Complex Type Ref Tests (Nested Structures)
    // ========================================================================

    #[test]
    fn test_deeply_nested_type_ref() {
        // List<Map<Text, Option<&mut T>>>
        let inner_ref = TypeRef::Reference {
            inner: Box::new(TypeRef::Generic(TypeParamId(0))),
            mutability: Mutability::Mutable,
            tier: CbgrTier::Tier0,
        };
        let option = TypeRef::Instantiated {
            base: TypeId(200), // Option
            args: vec![inner_ref],
        };
        let map = TypeRef::Instantiated {
            base: TypeId(101), // Map
            args: vec![TypeRef::Concrete(TypeId::TEXT), option],
        };
        let list = TypeRef::Instantiated {
            base: TypeId(100), // List
            args: vec![map],
        };

        assert!(list.is_generic());
        assert_eq!(list.base_type_id(), Some(TypeId(100)));

        // Verify serialization roundtrip
        let json = serde_json::to_string(&list).unwrap();
        let deserialized: TypeRef = serde_json::from_str(&json).unwrap();
        assert_eq!(list, deserialized);
    }

    #[test]
    fn test_function_type_with_all_features() {
        let mut contexts = SmallVec::new();
        contexts.push(ContextRef(1));
        contexts.push(ContextRef(2));

        let fn_type = TypeRef::Function {
            params: vec![
                TypeRef::Reference {
                    inner: Box::new(TypeRef::Generic(TypeParamId(0))),
                    mutability: Mutability::Immutable,
                    tier: CbgrTier::Tier1,
                },
                TypeRef::Tuple(vec![
                    TypeRef::Concrete(TypeId::INT),
                    TypeRef::Concrete(TypeId::FLOAT),
                ]),
            ],
            return_type: Box::new(TypeRef::Instantiated {
                base: TypeId(200), // Result
                args: vec![
                    TypeRef::Generic(TypeParamId(1)),
                    TypeRef::Concrete(TypeId::TEXT),
                ],
            }),
            contexts,
        };

        assert!(fn_type.is_generic());

        // Verify serialization roundtrip
        let json = serde_json::to_string(&fn_type).unwrap();
        let deserialized: TypeRef = serde_json::from_str(&json).unwrap();
        assert_eq!(fn_type, deserialized);
    }

    #[test]
    fn test_type_ref_clone() {
        let original = TypeRef::Instantiated {
            base: TypeId(100),
            args: vec![
                TypeRef::Concrete(TypeId::INT),
                TypeRef::Generic(TypeParamId(0)),
            ],
        };
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    // ========================================================================
    // Edge Cases and Boundary Tests
    // ========================================================================

    #[test]
    fn test_type_id_boundary_builtin() {
        // TypeId 15 is the last builtin
        assert!(TypeId(15).is_builtin());
        // TypeId 16 is first user type
        assert!(!TypeId(16).is_builtin());
    }

    #[test]
    fn test_property_set_all_bits_set() {
        let all = PropertySet::all();
        assert!(all.contains(PropertySet::PURE));
        assert!(all.contains(PropertySet::IO));
        assert!(all.contains(PropertySet::ASYNC));
        assert!(all.contains(PropertySet::FALLIBLE));
        assert!(all.contains(PropertySet::MUTATES));
        assert!(all.contains(PropertySet::ALLOCATES));
        assert!(all.contains(PropertySet::DIVERGES));
        assert!(all.contains(PropertySet::GPU));
    }

    #[test]
    fn test_empty_tuple_type() {
        let unit_tuple = TypeRef::Tuple(vec![]);
        assert!(!unit_tuple.is_generic());
        assert_eq!(unit_tuple.base_type_id(), None);
    }

    #[test]
    fn test_zero_length_array() {
        let zero_array = TypeRef::Array {
            element: Box::new(TypeRef::Concrete(TypeId::INT)),
            length: 0,
        };
        assert!(!zero_array.is_generic());
    }

    #[test]
    fn test_large_array_length() {
        let large_array = TypeRef::Array {
            element: Box::new(TypeRef::Concrete(TypeId::U8)),
            length: u64::MAX,
        };
        match large_array {
            TypeRef::Array { length, .. } => assert_eq!(length, u64::MAX),
            _ => panic!("Expected Array variant"),
        }
    }

    #[test]
    fn test_type_descriptor_with_drop_and_clone() {
        let mut td = TypeDescriptor::default();
        td.drop_fn = Some(100);
        td.clone_fn = Some(101);
        assert_eq!(td.drop_fn, Some(100));
        assert_eq!(td.clone_fn, Some(101));
    }
}
