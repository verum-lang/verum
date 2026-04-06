//! Core Protocol Type Definitions
//!
//! Verum Protocol System Core Types:
//! Protocols (analogous to traits) declare required methods, associated types,
//! and associated constants. Implementations provide concrete definitions.
//! Protocol bounds constrain generic type parameters. Object safety rules
//! determine which protocols can be used as dynamic dispatch (dyn Protocol).
//!
//! This module contains the foundational protocol (trait) type definitions
//! without any verification logic. These types are used by both verum_types
//! and verum_smt.
//!
//! # Design
//!
//! These are pure data structures representing:
//! - Protocol declarations (methods, associated types, constants)
//! - Protocol implementations
//! - Protocol bounds and constraints
//! - Method resolution metadata

use verum_ast::{span::Span, ty::Path};
use verum_common::{ConstValue, List, Map, Maybe, Text};

// Forward declare Type - this will be from verum_ast in the actual usage
// For now we use a placeholder since Type is defined in verum_types
// The actual Type will be imported by the using crates
use verum_ast::ty::Type as AstType;

/// Type alias to represent types - consumers will provide the actual Type
pub type Type = AstType;

// ==================== Object Safety ====================

/// Object safety error
///
/// Object safety errors that prevent a protocol from being used as `dyn Protocol`.
/// A protocol is object-safe when all methods can be dispatched through a vtable:
/// - Methods must not return Self (unknown size through vtable)
/// - Methods must not have generic type parameters (can't monomorphize through vtable)
/// - Methods must take self by reference (&self or &mut self), not by value
/// - Protocol must not have associated constants (no vtable slot)
/// - Protocol must not require Self: Sized bound
/// These rules ensure dynamic dispatch is possible without knowing the concrete type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectSafetyError {
    /// Method returns Self (unknown size)
    ReturnsSelf {
        /// Method name
        method_name: Text,
    },
    /// Method has generic type parameters
    GenericMethod {
        /// Method name
        method_name: Text,
    },
    /// Method doesn't take self parameter
    NoSelfParameter {
        /// Method name
        method_name: Text,
    },
    /// Protocol has associated constants
    HasAssociatedConst {
        /// Constant name
        const_name: Text,
    },
    /// Protocol requires Self: Sized bound
    RequiresSized,
    /// Method takes self by value (not reference)
    TakesSelfByValue {
        /// Method name
        method_name: Text,
    },
}

impl std::fmt::Display for ObjectSafetyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjectSafetyError::ReturnsSelf { method_name } => {
                write!(
                    f,
                    "method '{}' returns Self (unknown size at runtime)",
                    method_name
                )
            }
            ObjectSafetyError::GenericMethod { method_name } => {
                write!(
                    f,
                    "method '{}' has generic parameters (cannot be called through vtable)",
                    method_name
                )
            }
            ObjectSafetyError::NoSelfParameter { method_name } => {
                write!(
                    f,
                    "method '{}' has no self parameter (cannot be called on protocol object)",
                    method_name
                )
            }
            ObjectSafetyError::HasAssociatedConst { const_name } => {
                write!(
                    f,
                    "protocol has associated constant '{}' (incompatible with dynamic dispatch)",
                    const_name
                )
            }
            ObjectSafetyError::RequiresSized => {
                write!(
                    f,
                    "protocol requires Self: Sized bound (prevents use as protocol object)"
                )
            }
            ObjectSafetyError::TakesSelfByValue { method_name } => {
                write!(
                    f,
                    "method '{}' takes self by value (must use &self or &mut self)",
                    method_name
                )
            }
        }
    }
}

// ==================== Protocol Definition ====================

/// A protocol declaration (like a trait/type class)
///
/// Protocols define required methods, associated types, and constants
/// that implementing types must provide.
///
/// Example:
/// ```verum
/// protocol Eq<T> {
///     fn eq(self: T, other: T) -> Bool;
///     fn ne(self: T, other: T) -> Bool {
///         !self.eq(other)  // Default implementation
///     }
/// }
/// ```
#[derive(Debug, Clone)]
pub struct Protocol {
    /// Protocol name
    pub name: Text,
    /// Type parameters
    pub type_params: List<TypeParam>,
    /// Required methods
    pub methods: Map<Text, ProtocolMethod>,
    /// Associated types
    pub associated_types: Map<Text, AssociatedType>,
    /// Associated constants
    pub associated_consts: Map<Text, AssociatedConst>,
    /// Super-protocols (protocol bounds)
    pub super_protocols: List<ProtocolBound>,
    /// The crate where this protocol is defined (for orphan rule checking)
    pub defining_crate: Maybe<Text>,
    /// Source location
    pub span: Span,
}

/// Type parameter in a protocol
#[derive(Debug, Clone)]
pub struct TypeParam {
    /// Parameter name
    pub name: Text,
    /// Protocol bounds on this parameter
    pub bounds: List<ProtocolBound>,
    /// Default type (if any)
    pub default: Maybe<Type>,
}

/// A method in a protocol
#[derive(Debug, Clone)]
pub struct ProtocolMethod {
    /// Method name
    pub name: Text,
    /// Method type (function type)
    pub ty: Type,
    /// Whether this method has a default implementation
    pub has_default: bool,
    /// Documentation
    pub doc: Maybe<Text>,
}

/// Associated type in a protocol
///
/// Associated type in a protocol. Implementations must provide a concrete type.
/// Example: `type Item` in Iterator protocol, implemented as `type Item is Int`.
///
/// This is the base version without GAT support.
/// For GAT support, see gat_types::AssociatedTypeGAT.
///
/// Example:
/// ```verum
/// protocol Container {
///     type Item
///     fn get(&self, index: Int) -> Maybe<Self.Item>
/// }
/// ```
#[derive(Debug, Clone)]
pub struct AssociatedType {
    /// Type name
    pub name: Text,
    /// Bounds on the associated type
    pub bounds: List<ProtocolBound>,
    /// Default type (if any)
    pub default: Maybe<Type>,
}

impl AssociatedType {
    /// Create a simple associated type
    pub fn new(name: Text, bounds: List<ProtocolBound>) -> Self {
        Self {
            name,
            bounds,
            default: None,
        }
    }

    /// Create with default type
    pub fn with_default(name: Text, bounds: List<ProtocolBound>, default: Type) -> Self {
        Self {
            name,
            bounds,
            default: Some(default),
        }
    }
}

/// Associated constant in a protocol
#[derive(Debug, Clone)]
pub struct AssociatedConst {
    /// Constant name
    pub name: Text,
    /// Constant type
    pub ty: Type,
}

/// A protocol bound (constraint)
///
/// Example: `T: Eq + Ord`
#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolBound {
    /// Protocol being referenced
    pub protocol: Path,
    /// Type arguments to the protocol
    pub args: List<Type>,
    /// Whether this is a negative bound (!Protocol syntax)
    ///
    /// When true, this bound requires the type to NOT implement the protocol.
    /// This is used for specialization coherence and mutual exclusion patterns.
    pub is_negative: bool,
}

impl ProtocolBound {
    /// Create a new positive protocol bound
    pub fn positive(protocol: Path, args: List<Type>) -> Self {
        Self {
            protocol,
            args,
            is_negative: false,
        }
    }

    /// Create a new negative protocol bound (!Protocol)
    pub fn negative(protocol: Path, args: List<Type>) -> Self {
        Self {
            protocol,
            args,
            is_negative: true,
        }
    }

    /// Create a simple bound with just a protocol name (positive, no args)
    pub fn simple(protocol: Path) -> Self {
        Self::positive(protocol, List::new())
    }

    /// Check if this is a negative bound
    pub fn is_negative_bound(&self) -> bool {
        self.is_negative
    }
}

// ==================== Protocol Implementation ====================

/// An implementation of a protocol for a specific type
///
/// Example:
/// ```verum
/// impl Eq for Int {
///     fn eq(self: Int, other: Int) -> Bool {
///         // implementation
///     }
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ProtocolImpl {
    /// The protocol being implemented
    pub protocol: Path,
    /// Type arguments to the protocol
    pub protocol_args: List<Type>,
    /// The type implementing the protocol
    pub for_type: Type,
    /// Where clauses (additional constraints)
    pub where_clauses: List<WhereClause>,
    /// Method implementations
    pub methods: Map<Text, Type>,
    /// Associated type assignments
    pub associated_types: Map<Text, Type>,
    /// Associated constant values
    pub associated_consts: Map<Text, ConstValue>,
    /// The crate where this implementation is defined (for orphan rule checking)
    pub impl_crate: Maybe<Text>,
    /// Source location
    pub span: Span,
}

/// Where clause for constrained implementations
#[derive(Debug, Clone)]
pub struct WhereClause {
    /// Type being constrained
    pub ty: Type,
    /// Protocol bounds on the type
    pub bounds: List<ProtocolBound>,
}

// ConstValue is re-exported from verum_common for protocol constants

// ==================== Method Resolution ====================

/// Result of method resolution
#[derive(Debug, Clone)]
pub struct MethodResolution {
    /// Type of the resolved method
    pub ty: Type,
    /// Whether this is a default implementation
    pub is_default: bool,
    /// Source of the method implementation
    pub source: MethodSource,
}

/// Source of a method implementation
#[derive(Debug, Clone)]
pub enum MethodSource {
    /// Explicitly implemented in the impl block
    Explicit,
    /// Default implementation from protocol definition
    Default(Text),
    /// Inherited from superprotocol
    Inherited(Text),
}
