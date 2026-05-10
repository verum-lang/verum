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
///  These rules ensure dynamic dispatch is possible without knowing the concrete type.
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
///  fn eq(self: T, other: T) -> Bool;
///  fn ne(self: T, other: T) -> Bool {
///  !self.eq(other) // Default implementation
///  }
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
///  type Item
///  fn get(&self, index: Int) -> Maybe<Self.Item>
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
///  fn eq(self: Int, other: Int) -> Bool {
///  // implementation
///  }
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

/// Source of a method implementation.
///
/// Canonical home for the protocol-side method-source taxonomy.
/// `verum_types::protocol::MethodSource` re-exports this type
/// rather than defining its own — pre-collapse the two were
/// structural duplicates with bit-identical doc comments and
/// payload shapes, kept in sync only by convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MethodSource {
    /// Explicitly implemented in the impl block
    Explicit,
    /// Default implementation from protocol definition
    Default(Text),
    /// Inherited from superprotocol
    Inherited(Text),
}

/// Discriminator for [`MethodSource`] — the zero-sized
/// projection used by metrics consumers that classify the
/// method-source band without cloning the protocol-name string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MethodSourceKind {
    Explicit,
    Default,
    Inherited,
}

/// Static fact-pack for a [`MethodSourceKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodSourceKindMeta {
    /// Lower-snake-case wire form — used in telemetry surfaces.
    pub name: &'static str,
    /// Whether the implementation lives on the *current* impl
    /// block (true for `Explicit`).  False means the body was
    /// resolved from a different declaration site —
    /// either the protocol's default or a superprotocol's impl.
    pub is_explicit_impl: bool,
    /// Whether the implementation comes from a *protocol*
    /// (default body) or from a *superprotocol* (inherited).
    /// True means *protocol-side* — Default + Inherited both
    /// flip this; Explicit does not.
    pub is_protocol_provided: bool,
    /// Whether the variant carries a *protocol-name* payload
    /// (the name of the providing protocol or superprotocol).
    /// True for Default + Inherited; false for Explicit.
    pub carries_protocol_name: bool,
}

impl MethodSourceKind {
    /// All variants in declaration order.
    pub const ALL: &'static [MethodSourceKind] = &[
        MethodSourceKind::Explicit,
        MethodSourceKind::Default,
        MethodSourceKind::Inherited,
    ];

    /// Static fact-pack.
    pub const fn meta(self) -> MethodSourceKindMeta {
        match self {
            MethodSourceKind::Explicit => MethodSourceKindMeta {
                name: "explicit",
                is_explicit_impl: true,
                is_protocol_provided: false,
                carries_protocol_name: false,
            },
            MethodSourceKind::Default => MethodSourceKindMeta {
                name: "default",
                is_explicit_impl: false,
                is_protocol_provided: true,
                carries_protocol_name: true,
            },
            MethodSourceKind::Inherited => MethodSourceKindMeta {
                name: "inherited",
                is_explicit_impl: false,
                is_protocol_provided: true,
                carries_protocol_name: true,
            },
        }
    }
}

impl MethodSource {
    /// Discriminator projection — strip the protocol-name
    /// payload, keep the tag.
    pub const fn kind(&self) -> MethodSourceKind {
        match self {
            MethodSource::Explicit => MethodSourceKind::Explicit,
            MethodSource::Default(_) => MethodSourceKind::Default,
            MethodSource::Inherited(_) => MethodSourceKind::Inherited,
        }
    }

    /// Returns the providing protocol name for the protocol-
    /// provided variants (Default + Inherited).  Pinned via
    /// `meta().carries_protocol_name` so consumers can decide
    /// whether to call this without per-variant matching.
    pub fn protocol_name(&self) -> Option<&Text> {
        match self {
            MethodSource::Explicit => None,
            MethodSource::Default(name) => Some(name),
            MethodSource::Inherited(name) => Some(name),
        }
    }
}

#[cfg(test)]
mod method_source_meta_drift_pins {
    use super::*;

    /// Drift-pin: `MethodSourceKind` discriminator projection +
    /// classifier flags.  Pre-collapse the `MethodSource` enum
    /// lived as a structural duplicate in `verum_types::protocol`;
    /// now `verum_types` re-exports this canonical home so a new
    /// variant added here flows through automatically.
    #[test]
    fn meta_pin_method_source_kind_round_trip_and_partitions() {
        // 1. Variant count.
        assert_eq!(MethodSourceKind::ALL.len(), 3);

        // 2. Names + uniqueness.
        let mut seen = std::collections::HashSet::new();
        for k in MethodSourceKind::ALL {
            let m = k.meta();
            assert!(
                m.name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{:?}: name not snake_case: {}",
                k,
                m.name
            );
            assert!(seen.insert(m.name), "{:?}: duplicate name", k);
        }

        // 3. is_explicit_impl partition — Explicit singleton.
        let explicit: Vec<_> = MethodSourceKind::ALL
            .iter()
            .filter(|k| k.meta().is_explicit_impl)
            .copied()
            .collect();
        assert_eq!(explicit, vec![MethodSourceKind::Explicit]);

        // 4. is_protocol_provided partition — Default + Inherited.
        let pp: Vec<_> = MethodSourceKind::ALL
            .iter()
            .filter(|k| k.meta().is_protocol_provided)
            .copied()
            .collect();
        assert_eq!(
            pp,
            vec![MethodSourceKind::Default, MethodSourceKind::Inherited],
        );

        // 5. carries_protocol_name partition — Default + Inherited.
        let cn: Vec<_> = MethodSourceKind::ALL
            .iter()
            .filter(|k| k.meta().carries_protocol_name)
            .copied()
            .collect();
        assert_eq!(
            cn,
            vec![MethodSourceKind::Default, MethodSourceKind::Inherited],
        );

        // 6. Cross-cutting: is_explicit_impl ⊕ is_protocol_provided
        //    (a method body comes from exactly one of: the impl
        //    block itself, or a protocol/superprotocol).
        for k in MethodSourceKind::ALL {
            let m = k.meta();
            assert!(
                m.is_explicit_impl ^ m.is_protocol_provided,
                "{:?}: must flip exactly one of is_explicit_impl / is_protocol_provided",
                k
            );
        }

        // 7. is_protocol_provided ⇔ carries_protocol_name —
        //    only protocol-provided variants carry the protocol-
        //    name payload.
        for k in MethodSourceKind::ALL {
            let m = k.meta();
            assert_eq!(
                m.is_protocol_provided, m.carries_protocol_name,
                "{:?}: protocol-provided iff carries-protocol-name",
                k
            );
        }

        // 8. Live-payload kind() projection + protocol_name()
        //    accessor.
        let exp = MethodSource::Explicit;
        assert_eq!(exp.kind(), MethodSourceKind::Explicit);
        assert!(exp.protocol_name().is_none());

        let def = MethodSource::Default(Text::from("Display"));
        assert_eq!(def.kind(), MethodSourceKind::Default);
        assert_eq!(def.protocol_name().unwrap().as_str(), "Display");

        let inh = MethodSource::Inherited(Text::from("Eq"));
        assert_eq!(inh.kind(), MethodSourceKind::Inherited);
        assert_eq!(inh.protocol_name().unwrap().as_str(), "Eq");
    }
}
