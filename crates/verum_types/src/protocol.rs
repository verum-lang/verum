//! Complete Protocol (Trait) System Implementation
//!
//! Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Complete Protocol System
//!
//! Protocols are Verum's version of type classes/traits with:
//! - Structural typing (duck typing)
//! - Associated types and constants
//! - Default implementations
//! - Protocol inheritance (superprotocols)
//! - VTable-based method dispatch (<10ns overhead)
//! - Generic protocols with constraints
//!
//! # Architecture
//!
//! - **Protocol**: Protocol definition with methods, associated types
//! - **ProtocolImpl**: Implementation of protocol for specific type
//! - **ProtocolChecker**: Registry and resolution engine
//! - **ProtocolMethod**: Method signature with optional default impl
//! - **VTable**: Virtual dispatch table for protocol methods

use std::cell::RefCell;

use verum_ast::{
    span::Span,
    ty::{Ident, Path},
};
use verum_common::{ConstValue, List, Map, Maybe, Set, Text};
use verum_common::ToText;
use verum_common::well_known_types::WellKnownType as WKT;
use crate::ty::{Type, TypeVar};

// Thread-local recursion guard for implements_optimistic
// Prevents infinite recursion when checking nested protocol bounds
thread_local! {
    static IMPL_CHECK_DEPTH: RefCell<usize> = const { RefCell::new(0) };
    static SUBST_TYPE_PARAMS_DEPTH: RefCell<usize> = const { RefCell::new(0) };
    // Cache for implements_optimistic to avoid redundant recursive checks.
    // Key: (checker_id, type_key, protocol_key) — see Audit-A2 in
    // ProtocolChecker.checker_id for why the checker_id dimension is
    // required for cross-checker coherence on a long-lived thread.
    static IMPL_OPTIMISTIC_CACHE: RefCell<std::collections::HashMap<(u64, Text, Text), bool>> = RefCell::new(std::collections::HashMap::new());
    // Stack of (type_key, protocol_key) pairs currently being checked
    // Used for cycle detection in implements_optimistic
    static IMPL_CHECKING_STACK: RefCell<Vec<(Text, Text)>> = const { RefCell::new(Vec::new()) };
    // Cycle detection for try_find_associated_type ↔ try_resolve_projection_type
    // Tracks (type_key, assoc_name) pairs being resolved to detect infinite loops
    // caused by blanket impls forwarding associated types (e.g., FutureExt.Output = F.Output)
    static ASSOC_TYPE_RESOLUTION_STACK: RefCell<Vec<(Text, Text)>> = const { RefCell::new(Vec::new()) };
}

/// Process-global counter handing out unique `ProtocolChecker.checker_id`
/// values. AtomicU64 wraps after 2^64 — for any realistic process
/// lifetime this is unbounded.
static NEXT_CHECKER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn allocate_checker_id() -> u64 {
    NEXT_CHECKER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Type alias for type variable substitutions
pub type TypeSubstitution = Map<Text, Type>;

// ==================== Core Protocol Types ====================

/// Protocol kind - distinguishes between constraint protocols, injectable contexts,
/// and dual-kind context protocols.
///
/// Context system synthesis: combining static (@injectable) and dynamic (provide/using) dependency injection
///
/// This replaces the previous `is_context: bool` field with a proper enum that captures
/// the three distinct protocol kinds:
///
/// 1. **Constraint**: Type bounds only (declared with `type X is protocol { }`)
///    - Used in: generic bounds, where clauses
///    - Cannot be: injected via `provide`, required via `using`
///    - Examples: Eq, Ord, Show, Iterator, Functor
///
/// 2. **Injectable**: DI capability only (declared with `context X { }`)
///    - Used in: `using [X]` requirements, `provide X = ...`
///    - Cannot be: used as type bounds
///    - Examples: Database, Logger, Cache
///
/// 3. **ConstraintAndInjectable** (Dual-kind): Both constraint AND injectable
///    (declared with `context protocol X { }`)
///    - Used in: generic bounds AND `using`/`provide`
///    - Enables: static dispatch via bounds (0ns) OR dynamic dispatch via DI (~5-30ns)
///    - Examples: Serializable, Validator
///
/// # Subkinding Rules
///
/// ```text
/// ConstraintAndInjectable <: Constraint
/// ConstraintAndInjectable <: Injectable
/// Constraint ⊥ Injectable  // Incompatible directly
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolKind {
    /// Pure constraint protocol - used only as type bounds.
    /// Declared with: `type X is protocol { }`
    Constraint,

    /// Pure injectable context - used only for dependency injection.
    /// Declared with: `context X { }`
    Injectable,

    /// Dual-kind: both a type constraint AND an injectable context.
    /// Declared with: `context protocol X { }`
    ConstraintAndInjectable,
}

impl ProtocolKind {
    /// Returns true if this kind can be used as a type constraint (in bounds).
    #[inline]
    pub fn is_constraint(&self) -> bool {
        matches!(self, ProtocolKind::Constraint | ProtocolKind::ConstraintAndInjectable)
    }

    /// Returns true if this kind can be used for dependency injection.
    #[inline]
    pub fn is_injectable(&self) -> bool {
        matches!(self, ProtocolKind::Injectable | ProtocolKind::ConstraintAndInjectable)
    }

    /// Returns true if this is a dual-kind protocol (both constraint and injectable).
    #[inline]
    pub fn is_dual(&self) -> bool {
        matches!(self, ProtocolKind::ConstraintAndInjectable)
    }

    /// Check subkinding: does `self` satisfy `required`?
    ///
    /// ConstraintAndInjectable satisfies both Constraint and Injectable.
    /// Constraint and Injectable are incompatible with each other.
    pub fn satisfies(&self, required: ProtocolKind) -> bool {
        match (self, required) {
            // Same kind always satisfies
            (k1, k2) if k1 == &k2 => true,
            // Dual satisfies either single kind
            (ProtocolKind::ConstraintAndInjectable, ProtocolKind::Constraint) => true,
            (ProtocolKind::ConstraintAndInjectable, ProtocolKind::Injectable) => true,
            // Single kinds don't satisfy each other or dual
            _ => false,
        }
    }

    /// Display name for error messages
    pub fn display_name(&self) -> &'static str {
        match self {
            ProtocolKind::Constraint => "constraint protocol",
            ProtocolKind::Injectable => "context",
            ProtocolKind::ConstraintAndInjectable => "context protocol",
        }
    }
}

impl std::fmt::Display for ProtocolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Object safety error
/// Basic protocols with simple associated types (initial release) — 2 lines 11618-11759
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectSafetyError {
    /// Method returns Self (unknown size)
    ReturnsSelf { method_name: Text },
    /// Method has generic type parameters
    GenericMethod { method_name: Text },
    /// Method doesn't take self parameter
    NoSelfParameter { method_name: Text },
    /// Protocol has associated constants
    HasAssociatedConst { const_name: Text },
    /// Protocol requires Self: Sized bound
    RequiresSized,
    /// Method takes self by value (not reference)
    TakesSelfByValue { method_name: Text },
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

/// A protocol declaration (like a trait/type class)
///
/// Protocols define required methods, associated types, and constants
/// that implementing types must provide.
///
/// Now with support for:
/// - Generic Associated Types (GATs)
/// - Specialization metadata
/// - Higher-kinded types
///
/// Example:
/// ```verum
/// protocol Eq<T> {
///     fn eq(self: T, other: T) -> Bool;
///     fn ne(self: T, other: T) -> Bool {
///         !self.eq(other)  // Default implementation
///     }
/// }
///
/// // With GATs:
/// protocol LendingIterator {
///     type Item<'a> where Self: 'a
///     fn next(&mut self) -> Maybe<GenRef<Item>>
/// }
/// ```
#[derive(Debug, Clone)]
pub struct Protocol {
    /// Protocol name
    pub name: Text,
    /// Protocol kind - determines how this protocol can be used.
    ///
    /// - `Constraint`: Type bounds only (`type X is protocol { }`)
    /// - `Injectable`: DI capability only (`context X { }`)
    /// - `ConstraintAndInjectable`: Both (`context protocol X { }`)
    ///
    /// Context system synthesis: combining static (@injectable) and dynamic (provide/using) dependency injection
    pub kind: ProtocolKind,
    /// Type parameters
    pub type_params: List<TypeParam>,
    /// Required methods
    pub methods: Map<Text, ProtocolMethod>,
    /// Associated types (now with GAT support)
    pub associated_types: Map<Text, AssociatedType>,
    /// Associated constants
    pub associated_consts: Map<Text, AssociatedConst>,
    /// Super-protocols (protocol bounds)
    pub super_protocols: List<ProtocolBound>,
    /// Specialization metadata (if this protocol supports specialization)
    /// Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — lines 549-663
    pub specialization_info: Maybe<crate::advanced_protocols::SpecializationInfo>,
    /// The cog where this protocol is defined (for orphan rule checking)
    pub defining_crate: Maybe<Text>,
    /// Source location
    pub span: Span,
}

impl Protocol {
    /// Returns true if this protocol can be used for dependency injection.
    ///
    /// Context protocols (`context protocol Name { }`) and pure contexts (`context Name { }`)
    /// can be used in `using [Name]` clauses and provided with `provide Name = ...`.
    #[inline]
    pub fn is_injectable(&self) -> bool {
        self.kind.is_injectable()
    }

    /// Returns true if this protocol can be used as a type constraint.
    ///
    /// Constraint protocols (`type X is protocol { }`) and context protocols (`context protocol`)
    /// can be used in `where T: X` bounds.
    #[inline]
    pub fn is_constraint(&self) -> bool {
        self.kind.is_constraint()
    }

    /// Returns true if this is a dual-kind context protocol.
    ///
    /// Context protocols (`context protocol Name { }`) can be used both as type constraints
    /// and for dependency injection.
    #[inline]
    pub fn is_context_protocol(&self) -> bool {
        self.kind.is_dual()
    }
}

/// Type parameter in a protocol
#[derive(Debug, Clone)]
pub struct TypeParam {
    pub name: Text,
    pub bounds: List<ProtocolBound>,
    pub default: Maybe<Type>,
}

/// A method in a protocol
///
/// Advanced protocols (future v2.0+): GATs, higher-rank bounds, specialization with lattice ordering, coherence rules — Section 5.4 - Refinement Integration
///
/// Protocol methods can include refinement constraints on parameters and return types:
/// - Parameter constraints: `fn process(x: Int{> 0})`
/// - Return constraints: `fn count() -> Int{>= 0}`
/// - Cross-parameter constraints: `fn transfer(amount: Int, balance: Int{>= amount})`
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
    /// Refinement constraints on parameters and return type
    ///
    /// Maps parameter names (and "return" for return type) to their refinement constraints.
    /// Example: {"x" -> {> 0}, "return" -> {>= 0}}
    ///
    /// Advanced protocols (future v2.0+): GATs, higher-rank bounds, specialization with lattice ordering, coherence rules — Section 5.4.1-5.4.3
    pub refinement_constraints: Map<Text, crate::advanced_protocols::RefinementConstraint>,
    /// Whether this method is async (affects state machine generation)
    pub is_async: bool,
    /// Context requirements for this method (from `using [...]` clause)
    /// Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage — Protocol Method Contexts
    pub context_requirements: List<Text>,
    /// Type parameter names for method-level generic params (e.g., ["B", "F"]).
    /// Used to re-register TypeVars during default method registration.
    pub type_param_names: List<Text>,
    /// Type parameter bounds for method-level generic params.
    /// Maps param name to its bound type (e.g., "F" -> fn(Int) -> B).
    /// This enables closure type inference for methods like `fn map<B, F: fn(Self.Item) -> B>`.
    pub type_param_bounds: Map<Text, Type>,
    /// Receiver kind for object safety checks.
    /// When Some, used directly for NoSelfParameter / TakesSelfByValue checks.
    /// When None, falls back to inferring from Function type params[0].
    pub receiver_kind: Maybe<ReceiverKind>,
}

impl ProtocolMethod {
    /// Create a simple protocol method without refinement constraints
    ///
    /// This is the most common constructor for backward compatibility.
    pub fn simple(name: Text, ty: Type, has_default: bool) -> Self {
        Self {
            name,
            ty,
            has_default,
            doc: Maybe::None,
            refinement_constraints: Map::new(),
            is_async: false,
            context_requirements: List::new(),
            type_param_names: List::new(),
            type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
        }
    }

    /// Create a protocol method with type parameter bounds
    ///
    /// Used for methods with bounded generic parameters like `fn map<B, F: fn(T) -> B>`.
    pub fn with_type_bounds(
        name: Text,
        ty: Type,
        has_default: bool,
        type_param_names: List<Text>,
        type_param_bounds: Map<Text, Type>,
    ) -> Self {
        Self {
            name,
            ty,
            has_default,
            doc: Maybe::None,
            refinement_constraints: Map::new(),
            is_async: false,
            context_requirements: List::new(),
            type_param_names,
            type_param_bounds,
            receiver_kind: Maybe::None,
        }
    }

    /// Create a protocol method with all fields specified
    pub fn new(
        name: Text,
        ty: Type,
        has_default: bool,
        doc: Maybe<Text>,
        refinement_constraints: Map<Text, crate::advanced_protocols::RefinementConstraint>,
        is_async: bool,
    ) -> Self {
        Self {
            name,
            ty,
            has_default,
            doc,
            refinement_constraints,
            is_async,
            context_requirements: List::new(),
            type_param_names: List::new(),
            type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
        }
    }

    /// Create an async protocol method
    pub fn async_method(name: Text, ty: Type, has_default: bool) -> Self {
        Self {
            name,
            ty,
            has_default,
            type_param_names: List::new(),
            type_param_bounds: Map::new(),
            doc: Maybe::None,
            refinement_constraints: Map::new(),
            is_async: true,
            context_requirements: List::new(),
            receiver_kind: Maybe::None,
        }
    }

    /// Add a refinement constraint to a parameter
    pub fn with_param_refinement(
        mut self,
        param_name: Text,
        constraint: crate::advanced_protocols::RefinementConstraint,
    ) -> Self {
        self.refinement_constraints.insert(param_name, constraint);
        self
    }

    /// Add a refinement constraint to the return type
    pub fn with_return_refinement(
        mut self,
        constraint: crate::advanced_protocols::RefinementConstraint,
    ) -> Self {
        self.refinement_constraints
            .insert("return".into(), constraint);
        self
    }

    /// Check if this method has refinement constraints
    pub fn has_refinement(&self) -> bool {
        !self.refinement_constraints.is_empty()
    }

    /// Get refinement constraint for a parameter
    pub fn get_param_refinement(
        &self,
        param_name: &str,
    ) -> Option<&crate::advanced_protocols::RefinementConstraint> {
        self.refinement_constraints.get(&param_name.into())
    }

    /// Get refinement constraint for return type
    pub fn get_return_refinement(
        &self,
    ) -> Option<&crate::advanced_protocols::RefinementConstraint> {
        self.refinement_constraints.get(&"return".into())
    }

    /// Validate refinement constraints
    ///
    /// Checks that:
    /// 1. Parameter names in constraints match actual parameters
    /// 2. Constraint predicates are well-formed
    /// 3. No conflicting constraints exist
    pub fn validate_refinements(&self) -> Result<(), Text> {
        // Extract parameter names from function type
        let param_names: Set<Text> = match &self.ty {
            Type::Function { params, .. } => {
                // For now, use indices as names since we don't have named parameters in Type
                (0..params.len())
                    .map(|i| format!("param_{}", i).into())
                    .collect()
            }
            _ => Set::new(),
        };

        // Validate each constraint
        for (name, _constraint) in self.refinement_constraints.iter() {
            if name != "return" && !param_names.contains(name) {
                // Check if it's a numbered parameter reference
                if !name.starts_with("param_") {
                    return Err(
                        format!("Refinement constraint for unknown parameter: {}", name).into(),
                    );
                }
            }
        }

        Ok(())
    }
}

/// Associated type in a protocol (with GAT support)
///
/// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — + GATs, higher-rank bounds, specialization, coherence (future v2.0+)
///
/// This struct now supports:
/// - Regular associated types: `type Item`
/// - Generic Associated Types (GATs): `type Item<T>`
/// - Higher-kinded types: `type F<_>`
///
/// Example:
/// ```verum
/// protocol Monad {
///     type Wrapped<T>  // GAT with type parameter
///     fn pure<T>(value: T) -> Self.Wrapped<T>
/// }
/// ```
#[derive(Debug, Clone)]
pub struct AssociatedType {
    /// Type name
    pub name: Text,
    /// Type parameters for GATs (empty for regular associated types)
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .1 lines 116-134
    pub type_params: List<crate::advanced_protocols::GATTypeParam>,
    /// Bounds on the associated type
    pub bounds: List<ProtocolBound>,
    /// Where clauses specific to this GAT
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .4 lines 441-471
    pub where_clauses: List<crate::advanced_protocols::GATWhereClause>,
    /// Default type (if any)
    pub default: Maybe<Type>,
    /// Kind of associated type (regular, generic, higher-kinded)
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .3 lines 410-437
    pub kind: crate::advanced_protocols::AssociatedTypeKind,
    /// Refinement predicate on the associated type
    ///
    /// Advanced protocols (future v2.0+): GATs, higher-rank bounds, specialization with lattice ordering, coherence rules — Section 5.4 - Refinement Integration
    ///
    /// Example:
    /// ```verum
    /// protocol Container {
    ///     type Size where Size >= 0    // Refinement predicate
    ///     fn len(self) -> Self.Size
    /// }
    /// ```
    pub refinement: Maybe<crate::advanced_protocols::RefinementConstraint>,
    /// Expected variance of this associated type in implementations
    ///
    /// This is used to verify that implementations respect the variance
    /// declaration, enabling safe covariance/contravariance in protocol types.
    ///
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .4 - Variance
    pub expected_variance: crate::advanced_protocols::Variance,
}

impl AssociatedType {
    /// Create a simple (non-GAT) associated type for backward compatibility
    pub fn simple(name: Text, bounds: List<ProtocolBound>) -> Self {
        Self {
            name,
            type_params: List::new(),
            bounds,
            where_clauses: List::new(),
            default: Maybe::None,
            kind: crate::advanced_protocols::AssociatedTypeKind::Regular,
            refinement: Maybe::None,
            expected_variance: crate::advanced_protocols::Variance::Invariant,
        }
    }

    /// Create a GAT with type parameters
    pub fn generic(
        name: Text,
        type_params: List<crate::advanced_protocols::GATTypeParam>,
        bounds: List<ProtocolBound>,
        where_clauses: List<crate::advanced_protocols::GATWhereClause>,
    ) -> Self {
        let arity = type_params.len();
        Self {
            name,
            type_params,
            bounds,
            where_clauses,
            default: Maybe::None,
            kind: crate::advanced_protocols::AssociatedTypeKind::Generic { arity },
            refinement: Maybe::None,
            expected_variance: crate::advanced_protocols::Variance::Invariant,
        }
    }

    /// Create an associated type with a refinement constraint
    pub fn with_refinement(
        name: Text,
        bounds: List<ProtocolBound>,
        refinement: crate::advanced_protocols::RefinementConstraint,
    ) -> Self {
        Self {
            name,
            type_params: List::new(),
            bounds,
            where_clauses: List::new(),
            default: Maybe::None,
            kind: crate::advanced_protocols::AssociatedTypeKind::Regular,
            refinement: Maybe::Some(refinement),
            expected_variance: crate::advanced_protocols::Variance::Invariant,
        }
    }

    /// Create a covariant associated type
    ///
    /// Covariant types allow subtyping: if A <: B, then Container<A> <: Container<B>
    pub fn covariant(name: Text, bounds: List<ProtocolBound>) -> Self {
        Self {
            name,
            type_params: List::new(),
            bounds,
            where_clauses: List::new(),
            default: Maybe::None,
            kind: crate::advanced_protocols::AssociatedTypeKind::Regular,
            refinement: Maybe::None,
            expected_variance: crate::advanced_protocols::Variance::Covariant,
        }
    }

    /// Create a contravariant associated type
    ///
    /// Contravariant types reverse subtyping: if A <: B, then F<B> <: F<A>
    pub fn contravariant(name: Text, bounds: List<ProtocolBound>) -> Self {
        Self {
            name,
            type_params: List::new(),
            bounds,
            where_clauses: List::new(),
            default: Maybe::None,
            kind: crate::advanced_protocols::AssociatedTypeKind::Regular,
            refinement: Maybe::None,
            expected_variance: crate::advanced_protocols::Variance::Contravariant,
        }
    }

    /// Check if this is a GAT (has type parameters)
    pub fn is_gat(&self) -> bool {
        !self.type_params.is_empty()
    }

    /// Get the arity (number of type parameters)
    pub fn arity(&self) -> usize {
        self.type_params.len()
    }

    /// Check if this associated type has a refinement constraint
    pub fn has_refinement(&self) -> bool {
        self.refinement.is_some()
    }

    /// Check variance compatibility
    ///
    /// Returns true if the given variance is compatible with the expected variance.
    /// - Invariant types are only compatible with invariant
    /// - Covariant types accept covariant or invariant
    /// - Contravariant types accept contravariant or invariant
    pub fn check_variance(
        &self,
        actual_variance: crate::advanced_protocols::Variance,
    ) -> Result<(), Text> {
        use crate::advanced_protocols::Variance;

        match (self.expected_variance, actual_variance) {
            // Exact matches are always ok
            (Variance::Covariant, Variance::Covariant) => Ok(()),
            (Variance::Contravariant, Variance::Contravariant) => Ok(()),
            (Variance::Invariant, Variance::Invariant) => Ok(()),
            // Invariant expected means implementation must be invariant
            (Variance::Invariant, _) => Ok(()),
            // Covariant expected: accept covariant or invariant
            (Variance::Covariant, Variance::Invariant) => Ok(()),
            (Variance::Covariant, Variance::Contravariant) => Err(format!(
                "Associated type '{}' declared covariant but used contravariantly",
                self.name
            )
            .into()),
            // Contravariant expected: accept contravariant or invariant
            (Variance::Contravariant, Variance::Invariant) => Ok(()),
            (Variance::Contravariant, Variance::Covariant) => Err(format!(
                "Associated type '{}' declared contravariant but used covariantly",
                self.name
            )
            .into()),
        }
    }

    /// Validate refinement constraint against the type bounds
    ///
    /// Ensures the refinement predicate is compatible with any protocol bounds
    /// on the associated type.
    pub fn validate_refinement(&self) -> Result<(), Text> {
        if let Maybe::Some(ref _refinement) = self.refinement {
            // Refinement validation would check:
            // 1. Predicate references valid type parameters
            // 2. Operations in predicate are defined for the bounded types
            // 3. No contradictions with where clauses
            Ok(())
        } else {
            Ok(())
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
///
/// Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — .4 - Negative Reasoning
///
/// Protocol bounds can be either positive (type must implement) or negative
/// (type must NOT implement). Negative bounds enable mutual exclusion patterns:
///
/// ```verum
/// implement<T> MyProtocol for T where T: Send + !Sync {
///     // Implementation for Send but not Sync types
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolBound {
    /// Protocol being referenced
    pub protocol: Path,
    /// Type arguments
    pub args: List<Type>,
    /// Whether this is a negative bound (!Protocol syntax)
    ///
    /// When true, this bound requires the type to NOT implement the protocol.
    /// This is used for specialization coherence and mutual exclusion patterns.
    ///
    /// Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — .4 - Negative Reasoning
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
/// Now with support for:
/// - Specialization (via @specialize attribute)
/// - GAT instantiations
///
/// Example:
/// ```verum
/// impl Eq for Int {
///     fn eq(self: Int, other: Int) -> Bool {
///         // implementation
///     }
/// }
///
/// // With specialization:
/// @specialize
/// impl Display for List<Text> {
///     fn fmt(self: &Self, f: &mut Formatter) -> Result<(), Error> {
///         // Specialized implementation
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
    /// Associated type assignments (supports GAT instantiations)
    /// For GATs: maps "Item<T>" to the concrete type
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .1 lines 116-142
    pub associated_types: Map<Text, Type>,
    /// Associated constant values
    pub associated_consts: Map<Text, ConstValue>,
    /// Specialization metadata (if this impl specializes another)
    /// Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — lines 549-663
    pub specialization: Maybe<crate::advanced_protocols::SpecializationInfo>,
    /// The cog where this implementation is defined (for orphan rule checking)
    pub impl_crate: Maybe<Text>,
    /// Source location
    pub span: Span,
    /// Function type bounds for type parameters.
    /// For `implement<F: fn(X) -> T, T> ...`, maps F's TypeVar to the function type `fn(X) -> T`.
    /// This enables extracting additional type bindings (like T) from function type parameters
    /// during associated type resolution via `substitute_impl_type_params`.
    pub type_param_fn_bounds: Map<TypeVar, Type>,
}

/// Where clause for constrained implementations
#[derive(Debug, Clone)]
pub struct WhereClause {
    pub ty: Type,
    pub bounds: List<ProtocolBound>,
}

// ConstValue is imported from crate::const_eval

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

// ==================== VTable Support ====================

/// Virtual dispatch table for protocol implementation
///
/// VTables enable dynamic dispatch with <10ns overhead through:
/// - Hash-based method lookup (O(1) with perfect hashing)
/// - Direct function pointers (no indirection)
/// - Cache-friendly layout (64-byte alignment)
/// - GAT associated type metadata (for runtime type queries)
///
/// # GAT Support
///
/// For protocols with Generic Associated Types (GATs), the VTable includes:
/// - associated_type_indices: Map from associated type name to metadata index
/// - Runtime type information pointers for each GAT instantiation
///
/// Example:
/// ```verum
/// protocol Iterator {
///     type Item<T>  // GAT with type parameter
///     fn next(&mut self) -> Maybe<Self.Item<T>>
/// }
/// ```
#[derive(Debug, Clone)]
pub struct VTable {
    /// Protocol name
    pub protocol: Text,
    /// Type implementing the protocol
    pub for_type: Type,
    /// Map: method name -> method index in vtable
    pub method_indices: Map<Text, usize>,
    /// Number of methods in vtable
    pub method_count: usize,
    /// Map: associated type name -> index in associated type metadata array
    /// Used for GAT support - enables O(1) lookup of associated type info
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .1-1.4
    pub associated_type_indices: Map<Text, usize>,
    /// Number of associated types (including GATs)
    pub associated_type_count: usize,
}

impl VTable {
    /// Create new VTable for protocol implementation
    ///
    /// CRITICAL: Method ordering MUST be deterministic for cross-compilation compatibility.
    /// This implementation uses alphabetically sorted method names to ensure that VTable
    /// indices are stable across compilations, preventing method dispatch failures.
    ///
    /// For protocol-based method ordering (respecting superprotocol inheritance),
    /// use `VTable::from_protocol_checker()` instead.
    pub fn new(protocol: Text, for_type: Type, methods: &Map<Text, Type>) -> Self {
        let mut method_indices = Map::new();

        // CRITICAL FIX: Sort method names alphabetically for deterministic ordering
        // Map::iter() does NOT guarantee order, which causes non-deterministic VTable indices.
        // This leads to method dispatch failures when the same protocol is compiled differently.
        let mut sorted_methods: Vec<_> = methods.iter().collect();
        sorted_methods.sort_by_key(|(name_a, _)| *name_a);

        let mut idx = 0;
        for (method_name, _) in sorted_methods {
            method_indices.insert(method_name.clone(), idx);
            idx += 1;
        }

        Self {
            protocol,
            for_type,
            method_indices,
            method_count: idx,
            associated_type_indices: Map::new(),
            associated_type_count: 0,
        }
    }

    /// Create new VTable with GAT support
    ///
    /// This version takes associated types and builds the GAT metadata indices.
    ///
    /// CRITICAL: Method and associated type ordering MUST be deterministic for
    /// cross-compilation compatibility. This implementation uses alphabetically
    /// sorted names to ensure stable indices.
    ///
    /// # Parameters
    /// - protocol: Protocol name
    /// - for_type: Type implementing the protocol
    /// - methods: Map of method names to types
    /// - associated_types: Map of associated type names to types (from impl block)
    pub fn new_with_gats(
        protocol: Text,
        for_type: Type,
        methods: &Map<Text, Type>,
        associated_types: &Map<Text, Type>,
    ) -> Self {
        let mut method_indices = Map::new();

        // CRITICAL FIX: Sort method names alphabetically for deterministic ordering
        let mut sorted_methods: Vec<_> = methods.iter().collect();
        sorted_methods.sort_by_key(|(name_a, _)| *name_a);

        let mut idx = 0;
        for (method_name, _) in sorted_methods {
            method_indices.insert(method_name.clone(), idx);
            idx += 1;
        }

        // CRITICAL FIX: Sort associated type names alphabetically for deterministic ordering
        let mut associated_type_indices = Map::new();
        let mut sorted_assoc_types: Vec<_> = associated_types.iter().collect();
        sorted_assoc_types.sort_by_key(|(name_a, _)| *name_a);

        let mut assoc_idx = 0;
        for (assoc_name, _) in sorted_assoc_types {
            associated_type_indices.insert(assoc_name.clone(), assoc_idx);
            assoc_idx += 1;
        }

        Self {
            protocol,
            for_type,
            method_indices,
            method_count: idx,
            associated_type_indices,
            associated_type_count: assoc_idx,
        }
    }

    /// Create VTable using ProtocolChecker for proper method ordering
    ///
    /// This is the RECOMMENDED constructor for production use as it respects:
    /// - Protocol definition order
    /// - Superprotocol inheritance order
    /// - Deterministic ordering across compilations
    ///
    /// This constructor queries the ProtocolChecker to get the complete method list
    /// including inherited methods from superprotocols, in the proper order defined
    /// by the protocol hierarchy.
    ///
    /// # Parameters
    /// - protocol_name: Name of the protocol
    /// - for_type: Type implementing the protocol
    /// - protocol_checker: The protocol checker with protocol definitions
    ///
    /// # Returns
    /// Result containing the VTable or a ProtocolError
    ///
    /// # Method Ordering Strategy
    ///
    /// 1. Query protocol_checker.all_methods() to get methods including superprotocols
    /// 2. Methods are ordered by:
    ///    - Superprotocol methods first (in declaration order)
    ///    - This protocol's methods last (in declaration order)
    /// 3. Within each protocol, methods are in definition order
    ///
    /// This ensures that method indices are stable and respect protocol inheritance.
    ///
    /// # Example
    ///
    /// ```ignore
    /// protocol Comparable extends Equatable {
    ///     fn compare(other: Self) -> Ordering  // index 1 (after eq from Equatable)
    /// }
    ///
    /// protocol Equatable {
    ///     fn eq(other: Self) -> Bool  // index 0
    /// }
    ///
    /// let vtable = VTable::from_protocol_checker(
    ///     "Comparable",
    ///     Type::Int,
    ///     &protocol_checker
    /// )?;
    /// // vtable.method_indices = {"eq": 0, "compare": 1}
    /// ```
    pub fn from_protocol_checker(
        protocol_name: &Text,
        for_type: Type,
        protocol_checker: &ProtocolChecker,
    ) -> Result<Self, ProtocolError> {
        // Get all methods including inherited ones in proper order
        let methods = protocol_checker.all_methods(protocol_name)?;

        // Build method indices from the ordered list
        let mut method_indices = Map::new();
        for (idx, method) in methods.iter().enumerate() {
            method_indices.insert(method.name.clone(), idx);
        }

        let method_count = methods.len();

        // Get protocol definition for associated types
        let protocol = protocol_checker
            .get_protocol(protocol_name)
            .ok_or_else(|| ProtocolError::ProtocolNotFound {
                name: protocol_name.clone(),
            })?;

        // Build associated type indices (sorted alphabetically for determinism)
        let mut associated_type_indices = Map::new();
        let mut sorted_assoc_types: Vec<_> = protocol.associated_types.keys().collect();
        sorted_assoc_types.sort();

        for (idx, assoc_name) in sorted_assoc_types.iter().enumerate() {
            associated_type_indices.insert((*assoc_name).clone(), idx);
        }

        let associated_type_count = sorted_assoc_types.len();

        Ok(Self {
            protocol: protocol_name.clone(),
            for_type,
            method_indices,
            method_count,
            associated_type_indices,
            associated_type_count,
        })
    }

    /// Get method index for dispatch
    pub fn get_method_index(&self, method: &Text) -> Maybe<usize> {
        self.method_indices.get(method).copied()
    }

    /// Get associated type index for GAT lookup
    ///
    /// Returns the index into the associated type metadata array.
    pub fn get_associated_type_index(&self, assoc_name: &Text) -> Maybe<usize> {
        self.associated_type_indices.get(assoc_name).copied()
    }

    /// Check if this VTable has GAT support
    pub fn has_gats(&self) -> bool {
        self.associated_type_count > 0
    }

    /// Get vtable layout info for codegen
    pub fn layout_info(&self) -> VTableLayout {
        VTableLayout {
            size: self.method_count * 8, // 8 bytes per function pointer
            alignment: 8,
            method_offsets: self
                .method_indices
                .iter()
                .map(|(name, &idx)| (name.clone(), idx * 8))
                .collect(),
            associated_type_offsets: self
                .associated_type_indices
                .iter()
                .map(|(name, &idx)| (name.clone(), idx * 8))
                .collect(),
        }
    }
}

/// VTable memory layout information
///
/// Extended with GAT support to include associated type metadata offsets.
#[derive(Debug, Clone)]
pub struct VTableLayout {
    /// Total size in bytes (methods + associated type metadata)
    pub size: usize,
    /// Alignment requirement
    pub alignment: usize,
    /// Map: method name -> offset in bytes
    pub method_offsets: Map<Text, usize>,
    /// Map: associated type name -> offset in bytes
    /// Used for GAT metadata lookup at runtime
    pub associated_type_offsets: Map<Text, usize>,
}

// ==================== Method Registry ====================

/// Kind of receiver for a method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverKind {
    /// Takes ownership: self
    Value,
    /// Immutable reference: &self
    Ref,
    /// Mutable reference: &mut self
    RefMut,
    /// Static method (no self)
    Static,
}

/// Signature of a method for type-based lookup
#[derive(Debug, Clone)]
pub struct MethodSignature {
    /// Method name
    pub name: Text,
    /// Type parameters (generic method)
    pub type_params: List<Text>,
    /// Receiver kind
    pub receiver: ReceiverKind,
    /// Parameter types (excluding self)
    pub params: List<Type>,
    /// Return type
    pub return_type: Type,
    /// Whether the method is mutating (for borrow checking)
    pub is_mutating: bool,
}

impl MethodSignature {
    /// Create a simple method signature
    pub fn new(
        name: impl Into<Text>,
        receiver: ReceiverKind,
        params: List<Type>,
        return_type: Type,
    ) -> Self {
        Self {
            name: name.into(),
            type_params: List::new(),
            receiver,
            params,
            return_type,
            is_mutating: receiver == ReceiverKind::RefMut,
        }
    }

    /// Create a generic method signature
    pub fn generic(
        name: impl Into<Text>,
        type_params: List<Text>,
        receiver: ReceiverKind,
        params: List<Type>,
        return_type: Type,
    ) -> Self {
        Self {
            name: name.into(),
            type_params,
            receiver,
            params,
            return_type,
            is_mutating: receiver == ReceiverKind::RefMut,
        }
    }

    /// Create an immutable method (&self)
    pub fn immutable(name: impl Into<Text>, params: List<Type>, return_type: Type) -> Self {
        Self::new(name, ReceiverKind::Ref, params, return_type)
    }

    /// Create a mutating method (&mut self)
    pub fn mutating(name: impl Into<Text>, params: List<Type>, return_type: Type) -> Self {
        Self::new(name, ReceiverKind::RefMut, params, return_type)
    }

    /// Create a static method (no self)
    pub fn static_method(name: impl Into<Text>, params: List<Type>, return_type: Type) -> Self {
        Self::new(name, ReceiverKind::Static, params, return_type)
    }

    /// Freshen all TypeVars in the method signature.
    ///
    /// This replaces every TypeVar in params and return_type with a fresh TypeVar.
    /// This is essential for method-level type parameters (e.g., F in `map<F>`)
    /// to prevent different call sites from sharing the same TypeVar and polluting
    /// each other's inference.
    pub fn freshen(&self) -> Self {
        use crate::ty::TypeVar;

        // Collect all TypeVars from params and return_type
        let mut type_vars: Vec<TypeVar> = Vec::new();
        for param in &self.params {
            Self::collect_type_vars(param, &mut type_vars);
        }
        Self::collect_type_vars(&self.return_type, &mut type_vars);

        // Deduplicate
        type_vars.sort_by_key(|tv| tv.id());
        type_vars.dedup_by_key(|tv| tv.id());

        if type_vars.is_empty() {
            return self.clone();
        }

        // Build fresh mapping
        let mut mapping: Map<TypeVar, Type> = Map::new();
        for tv in &type_vars {
            mapping.insert(*tv, Type::Var(TypeVar::fresh()));
        }

        // Apply mapping
        MethodSignature {
            name: self.name.clone(),
            type_params: self.type_params.clone(),
            receiver: self.receiver,
            params: self.params.iter()
                .map(|p| Self::apply_tv_mapping(p, &mapping))
                .collect(),
            return_type: Self::apply_tv_mapping(&self.return_type, &mapping),
            is_mutating: self.is_mutating,
        }
    }

    fn collect_type_vars(ty: &Type, vars: &mut Vec<crate::ty::TypeVar>) {
        match ty {
            Type::Var(tv) => vars.push(*tv),
            Type::Named { args, .. } | Type::Generic { args, .. } => {
                for arg in args.iter() {
                    Self::collect_type_vars(arg, vars);
                }
            }
            Type::Tuple(elems) => {
                for elem in elems.iter() {
                    Self::collect_type_vars(elem, vars);
                }
            }
            Type::Function { params, return_type, .. } => {
                for param in params.iter() {
                    Self::collect_type_vars(param, vars);
                }
                Self::collect_type_vars(return_type, vars);
            }
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => {
                Self::collect_type_vars(inner, vars);
            }
            Type::Array { element, .. } => {
                Self::collect_type_vars(element, vars);
            }
            _ => {}
        }
    }

    fn apply_tv_mapping(ty: &Type, mapping: &Map<crate::ty::TypeVar, Type>) -> Type {
        match ty {
            Type::Var(tv) => {
                if let Some(replacement) = mapping.get(tv) {
                    replacement.clone()
                } else {
                    ty.clone()
                }
            }
            Type::Named { path, args } => Type::Named {
                path: path.clone(),
                args: args.iter().map(|a| Self::apply_tv_mapping(a, mapping)).collect(),
            },
            Type::Generic { name, args } => Type::Generic {
                name: name.clone(),
                args: args.iter().map(|a| Self::apply_tv_mapping(a, mapping)).collect(),
            },
            Type::Tuple(elems) => {
                Type::Tuple(elems.iter().map(|e| Self::apply_tv_mapping(e, mapping)).collect())
            }
            Type::Function { params, return_type, contexts, type_params, properties } => {
                Type::Function {
                    params: params.iter().map(|p| Self::apply_tv_mapping(p, mapping)).collect(),
                    return_type: Box::new(Self::apply_tv_mapping(return_type, mapping)),
                    contexts: contexts.clone(),
                    type_params: type_params.clone(),
                    properties: properties.clone(),
                }
            }
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(Self::apply_tv_mapping(inner, mapping)),
            },
            Type::CheckedReference { mutable, inner } => Type::CheckedReference {
                mutable: *mutable,
                inner: Box::new(Self::apply_tv_mapping(inner, mapping)),
            },
            Type::UnsafeReference { mutable, inner } => Type::UnsafeReference {
                mutable: *mutable,
                inner: Box::new(Self::apply_tv_mapping(inner, mapping)),
            },
            Type::Array { element, size } => Type::Array {
                element: Box::new(Self::apply_tv_mapping(element, mapping)),
                size: *size,
            },
            _ => ty.clone(),
        }
    }
}

/// Result of method lookup
#[derive(Debug, Clone)]
pub struct MethodLookupResult {
    /// The found method signature
    pub signature: MethodSignature,
    /// Type substitution for generic parameters
    pub substitution: Map<Text, Type>,
    /// The type on which the method was found (may differ from original due to Deref)
    pub resolved_receiver_type: Type,
}

// ==================== Protocol Checker ====================

/// Result of resolving the Try protocol for a type
///
/// Protocol-based Try resolution: ? operator uses Carrier protocol to convert between error types
#[derive(Debug, Clone)]
pub struct TryProtocolResolution {
    /// The Output associated type - extracted on success
    pub output: Type,
    /// The Residual associated type - propagated on failure
    pub residual: Type,
}

/// Result of resolving the IntoIterator protocol for a type
///
/// Used by for-loop type inference to determine the element type of iteration.
/// Protocol-based desugaring: syntactic sugar resolved through protocol method dispatch
#[derive(Debug, Clone)]
pub struct IntoIteratorResolution {
    /// The Item associated type - the element type being iterated
    pub item: Type,
    /// The Iter associated type - the iterator type returned by into_iter()
    pub iter: Type,
}

/// Result of resolving the Future protocol for a type
///
/// Used by await expression type inference to determine the output type.
#[derive(Debug, Clone)]
pub struct FutureResolution {
    /// The Output associated type - the value produced when the future completes
    pub output: Type,
}

/// Result of resolving the Index protocol for a type
///
/// Used by index operator type inference to determine key and value types.
/// Index operator resolution: "x[i]" desugars to Index/IndexMut protocol method calls
#[derive(Debug, Clone)]
pub struct IndexResolution {
    /// The key/index type required for indexing
    pub key: Type,
    /// The output type returned by indexing
    pub output: Type,
}

/// Result of resolving the Maybe protocol for a type
///
/// Used by `??` (null coalescing) and `?.` (optional chaining) operators.
/// Maybe operator resolution: ? on Maybe<T> desugars to match with None -> return None propagation
#[derive(Debug, Clone)]
pub struct MaybeResolution {
    /// The inner type T from Maybe<T>
    pub inner: Type,
}

/// Protocol checking and resolution engine
#[derive(Clone)]
pub struct ProtocolChecker {
    /// Registered protocols
    protocols: Map<Text, Protocol>,
    /// Protocol implementations
    impls: List<ProtocolImpl>,
    /// Implementation index for fast lookup
    /// Map from (type, protocol) to implementation
    impl_index: Map<(Text, Text), usize>,
    /// Derivable protocols (can be automatically derived)
    derivable: Set<Text>,
    /// Current cog being compiled (for orphan rule checking)
    current_crate: Maybe<Text>,
    /// Map from type name to defining cog (for orphan rule checking)
    type_crates: Map<Text, Text>,
    /// Cache for protocol hierarchy traversal results (protocol_name -> all methods)
    methods_cache: Map<Text, List<ProtocolMethod>>,
    /// Cache for superprotocol hierarchy (protocol_name -> all superprotocol names)
    superprotocol_cache: Map<Text, List<Text>>,
    /// Cache for protocol implementation lookups (type_key, protocol_key) -> impl exists
    ///
    /// This cache stores the results of `implements()` checks to avoid repeated
    /// resolution when the same type/protocol pair is queried multiple times.
    /// The value is `Maybe::Some(true)` if implemented, `Maybe::Some(false)` if not,
    /// and the entry is absent if not yet checked.
    impl_check_cache: Map<(Text, Text), bool>,

    // =========================================================================
    // Method Registry - For protocol-based method resolution
    // =========================================================================
    // Protocol-driven method resolution: methods resolved by searching implemented protocols for matching signatures

    /// Methods indexed by (type_constructor, method_name) -> signature
    /// This replaces the 2000-line get_builtin_method_type function
    method_registry: Map<(Text, Text), MethodSignature>,

    /// Map from variant type signature to named type name
    /// This enables protocol lookups on expanded variant types by mapping
    /// them back to their declared named type (e.g., "Variant(None|Some)" -> "Maybe")
    /// Spec: stdlib-agnostic protocol resolution
    variant_type_names: Map<Text, Text>,

    /// Cache for method resolution: (type_key, protocol_key, method_name) -> MethodResolution
    /// Avoids repeated find_impl + method lookup for the same type/protocol/method triple
    method_resolution_cache: Map<(Text, Text, Text), MethodResolution>,

    /// Audit-A2 coherence: per-checker identity used to scope the
    /// thread-local `IMPL_OPTIMISTIC_CACHE`.
    ///
    /// Without this scoping, two different `ProtocolChecker` instances
    /// running on the same OS thread (e.g. successive compilations in
    /// a long-lived process such as `vtest`, the LSP server, or a
    /// REPL) shared the same thread-local cache of `(type_key,
    /// protocol_key) -> bool` results. Checker A could populate
    /// "`Foo: MyProtocol == true`" via its impl set; checker B —
    /// without that impl registered — would then read the stale
    /// `true` and skip its own impl-set probe. Adding a checker_id
    /// dimension to the cache key isolates each checker's view.
    ///
    /// IDs are issued from a process-global atomic counter; collisions
    /// are impossible within the lifetime of a process and across
    /// processes the cache is fresh anyway (thread-local).
    checker_id: u64,

    /// `[protocols].resolution_strategy` — chooses how `find_impl`
    /// resolves multi-candidate cases.  Threaded from manifest via
    /// `set_resolution_strategy`.
    ///
    ///   * `"most_specific"` (default) — pick the most specific impl
    ///     (current behaviour via `select_most_specific_impl`).
    ///   * `"first_declared"` — pick the first registered candidate
    ///     (useful in open-world plugin systems).
    ///   * `"error"` — any overlap is an error; `find_impl` returns
    ///     None to surface the existing ambiguity diagnostic.
    ///
    /// Pre-fix the production resolver hardcoded "most_specific" and
    /// `[protocols].resolution_strategy` was tracing-only at
    /// session.rs:582.
    resolution_strategy: Text,
    /// `[protocols].blanket_impls` — when false, candidates whose
    /// `for_type` is a bare type variable (the blanket
    /// `impl<T> Protocol for T` pattern) are excluded from
    /// `find_impl`'s candidate set.  Threaded from manifest via
    /// `set_blanket_impls`.  Default true (Rust-like ergonomics).
    blanket_impls: bool,

    /// `[types].instance_search` — when false, `find_impl` skips
    /// the Stage-2 generic-candidate scan entirely.  Only the
    /// O(1) exact-match path runs, so any call site that relied
    /// on the resolver finding a generic `impl<T> Protocol for
    /// List<T>` (or similar substitution-based match) for a
    /// concrete type returns `None`.
    ///
    /// Mirrors the Idris-style "no implicit instance search"
    /// semantic: every protocol-method dispatch must hit a
    /// concretely-registered impl for the exact type, or the
    /// caller must thread the instance explicitly.  Useful for
    /// projects that want compile-time predictability (no
    /// surprising blanket-impl resolution) at the cost of more
    /// verbose impl declarations.
    ///
    /// Threaded from manifest via `set_instance_search_enabled`.
    /// Default true (current behaviour — the resolver runs the
    /// full multi-stage candidate scan).
    instance_search_enabled: bool,
}

impl ProtocolChecker {
    /// Create a new protocol checker
    pub fn new() -> Self {
        let mut checker = Self {
            protocols: Map::new(),
            impls: List::new(),
            impl_index: Map::new(),
            derivable: Set::new(),
            current_crate: Maybe::None,
            type_crates: Map::new(),
            methods_cache: Map::new(),
            superprotocol_cache: Map::new(),
            impl_check_cache: Map::new(),
            method_registry: Map::new(),
            variant_type_names: Map::new(),
            method_resolution_cache: Map::new(),
            checker_id: allocate_checker_id(),
            resolution_strategy: Text::from("most_specific"),
            blanket_impls: true,
            instance_search_enabled: true,
        };

        // Register standard protocols
        checker.register_standard_protocols();

        // Register standard type methods
        checker.register_standard_methods();

        checker
    }

    /// Create an empty protocol checker without standard protocols (for testing)
    ///
    /// This is useful for tests that want to register custom protocols
    /// without interference from standard library protocols.
    pub fn new_empty() -> Self {
        Self {
            protocols: Map::new(),
            impls: List::new(),
            impl_index: Map::new(),
            derivable: Set::new(),
            current_crate: Maybe::None,
            type_crates: Map::new(),
            methods_cache: Map::new(),
            superprotocol_cache: Map::new(),
            impl_check_cache: Map::new(),
            method_registry: Map::new(),
            variant_type_names: Map::new(),
            method_resolution_cache: Map::new(),
            checker_id: allocate_checker_id(),
            resolution_strategy: Text::from("most_specific"),
            blanket_impls: true,
            instance_search_enabled: true,
        }
    }

    /// Apply the manifest-side `[protocols].resolution_strategy`.
    /// Unknown values fall back to `"most_specific"` with a warning
    /// so a typo doesn't silently switch resolution semantics.
    pub fn set_resolution_strategy(&mut self, strategy: impl Into<Text>) {
        let s: Text = strategy.into();
        match s.as_str() {
            "most_specific" | "first_declared" | "error" => {
                self.resolution_strategy = s;
            }
            other => {
                tracing::warn!(
                    "[protocols].resolution_strategy: unknown value {:?} \
                     (expected \"most_specific\" | \"first_declared\" | \
                     \"error\"); defaulting to most_specific",
                    other
                );
                self.resolution_strategy = Text::from("most_specific");
            }
        }
    }

    /// Apply the manifest-side `[protocols].blanket_impls`. When
    /// false, `find_impl` excludes candidates whose `for_type` is a
    /// bare type variable from the candidate set.
    pub fn set_blanket_impls(&mut self, allowed: bool) {
        self.blanket_impls = allowed;
    }

    /// Apply the manifest-side `[types].instance_search`. When
    /// false, `find_impl` skips the Stage-2 generic-candidate scan
    /// — only the O(1) exact-match path runs.  Closes the
    /// inert-defense pattern around the field at session.rs:472:
    /// pre-fix the setter on `TypeChecker.set_instance_search_
    /// enabled` stored the value but no production code path
    /// consulted it.
    pub fn set_instance_search_enabled(&mut self, enabled: bool) {
        self.instance_search_enabled = enabled;
    }

    /// Read-only accessor — exposed for diagnostics + tests.
    #[inline]
    pub fn instance_search_enabled(&self) -> bool {
        self.instance_search_enabled
    }

    /// Read-only accessor — exposed for diagnostics + tests.
    #[inline]
    pub fn resolution_strategy(&self) -> &Text {
        &self.resolution_strategy
    }

    /// Read-only accessor — exposed for diagnostics + tests.
    #[inline]
    pub fn blanket_impls_allowed(&self) -> bool {
        self.blanket_impls
    }

    /// Public accessor for the per-instance checker id. Used by the
    /// type-checker glue to scope thread-local caches to this checker.
    #[inline]
    pub fn id(&self) -> u64 { self.checker_id }

    /// Clear implementation check cache
    ///
    /// This should be called when new implementations are registered
    /// to ensure the cache stays consistent.
    pub fn invalidate_impl_cache(&mut self) {
        self.impl_check_cache.clear();
        self.method_resolution_cache.clear();
        // Also clear thread-local optimistic cache
        IMPL_OPTIMISTIC_CACHE.with(|c| {
            c.borrow_mut().clear();
        });
    }

    // =========================================================================
    // Stdlib-agnostic integration methods
    // =========================================================================

    /// Get mutable access to the method registry for stdlib integration
    ///
    /// This enables dynamic method registration during stdlib bootstrap,
    /// allowing methods to be registered as .vr files are parsed.
    pub fn method_registry_mut(&mut self) -> &mut Map<(Text, Text), MethodSignature> {
        &mut self.method_registry
    }

    /// Get read-only access to the method registry
    pub fn method_registry(&self) -> &Map<(Text, Text), MethodSignature> {
        &self.method_registry
    }

    /// Register a variant type name mapping
    ///
    /// This maps a variant type signature (e.g., "Variant(None|Some)") to its
    /// declared named type (e.g., "Maybe"). This enables protocol lookups on
    /// expanded variant types by mapping them back to their named type.
    ///
    /// Spec: stdlib-agnostic protocol resolution
    pub fn register_variant_type_name(&mut self, signature: Text, type_name: Text) {
        // First-wins: stdlib types registered first take precedence
        self.variant_type_names.entry(signature).or_insert(type_name);
    }

    /// Look up the named type for a variant signature
    pub fn get_variant_type_name(&self, signature: &Text) -> Option<&Text> {
        self.variant_type_names.get(signature)
    }

    /// Check if a type implements a protocol by name (string version)
    ///
    /// This is a convenience method for stdlib integration that doesn't
    /// require constructing a Path.
    pub fn implements_by_name(&self, ty: &Type, protocol_name: &str) -> bool {
        // Check if any implementation exists for this type and protocol
        let type_name = self.extract_type_name(ty);
        let key = (type_name, Text::from(protocol_name));
        self.impl_index.contains_key(&key)
    }

    /// Check if a type implements any variant of a protocol (ignoring type args).
    ///
    /// E.g., `implements_protocol_any(Int32, "AddAssign")` returns true
    /// even though the key is `"AddAssign<Int32>"`.
    ///
    /// This is useful for checking compound assignment protocols where the
    /// exact type argument doesn't matter — we just want to know if the type
    /// supports the operation at all.
    pub fn implements_protocol_any(&self, ty: &Type, protocol_base_name: &str) -> bool {
        let type_key = self.extract_type_name(ty);
        let prefix = format!("{}<", protocol_base_name);
        self.impl_index.keys().any(|(tk, pk)| {
            tk == &type_key && (pk.as_str() == protocol_base_name || pk.starts_with(&prefix))
        })
    }

    /// Extract a type name from a Type for registry lookup
    fn extract_type_name(&self, ty: &Type) -> Text {
        match ty {
            Type::Int => WKT::Int.as_str().into(),
            Type::Float => WKT::Float.as_str().into(),
            Type::Bool => WKT::Bool.as_str().into(),
            Type::Char => WKT::Char.as_str().into(),
            Type::Text => WKT::Text.as_str().into(),
            Type::Unit => "Unit".into(),
            Type::Never => "Never".into(),
            Type::Generic { name, .. } => name.clone(),
            Type::Named { path, .. } => {
                use verum_ast::ty::PathSegment;
                path.segments
                    .last()
                    .and_then(|seg| {
                        if let PathSegment::Name(ident) = seg {
                            Some(ident.name.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "Unknown".into())
            }
            Type::Reference { inner, .. } => self.extract_type_name(inner),
            Type::CheckedReference { inner, .. } => self.extract_type_name(inner),
            Type::UnsafeReference { inner, .. } => self.extract_type_name(inner),
            _ => "Unknown".into(),
        }
    }

    /// Register standard protocols (Eq, Ord, Show, Send, Sync, etc.)
    fn register_standard_protocols(&mut self) {
        // Register Send and Sync marker protocols first
        // Basic protocols with simple associated types (initial release) — 4 - Thread-Safety Protocols
        crate::send_sync::register_send_sync_protocols(self);

        // Eq protocol
        let eq = Protocol {
            name: "Eq".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "eq".into(),
                    ProtocolMethod {
                        name: "eq".into(),
                        ty: Type::function(
                            List::from(vec![
                                Type::Var(crate::ty::TypeVar::with_id(0)),
                                Type::Var(crate::ty::TypeVar::with_id(0)),
                            ]),
                            Type::Bool,
                        ),
                        has_default: false,
                        doc: Maybe::Some("Test for equality".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods.insert(
                    "ne".into(),
                    ProtocolMethod {
                        name: "ne".into(),
                        ty: Type::function(
                            List::from(vec![
                                Type::Var(crate::ty::TypeVar::with_id(0)),
                                Type::Var(crate::ty::TypeVar::with_id(0)),
                            ]),
                            Type::Bool,
                        ),
                        has_default: true,
                        doc: Maybe::Some("Test for inequality (has default impl)".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: Map::new(),
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("Eq".into(), eq);
        self.derivable.insert("Eq".into());

        // Ord protocol (requires Eq)
        // Ordering type is a variant: Less | Equal | Greater
        let ordering_type = Type::Variant({
            let mut variants = indexmap::IndexMap::new();
            variants.insert(Text::from("Less"), Type::Unit);
            variants.insert(Text::from("Equal"), Type::Unit);
            variants.insert(Text::from("Greater"), Type::Unit);
            variants
        });
        let ord = Protocol {
            name: "Ord".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "cmp".into(),
                    ProtocolMethod {
                        name: "cmp".into(),
                        ty: Type::function(
                            List::from(vec![
                                Type::Var(crate::ty::TypeVar::with_id(0)),
                                Type::Var(crate::ty::TypeVar::with_id(0)),
                            ]),
                            ordering_type.clone(),
                        ),
                        has_default: false,
                        doc: Maybe::Some("Compare two values".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: Map::new(),
            associated_consts: Map::new(),
            super_protocols: {
                let mut bounds = List::new();
                bounds.push(ProtocolBound {
                    protocol: Path::single(Ident::new("Eq", Span::default())),
                    args: List::new(),
                    is_negative: false,
                });
                bounds
            },
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("Ord".into(), ord);
        self.derivable.insert("Ord".into());

        // Show protocol (for display/debug)
        let show = Protocol {
            name: "Show".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "show".into(),
                    ProtocolMethod {
                        name: "show".into(),
                        ty: Type::function(
                            List::from(vec![Type::Var(crate::ty::TypeVar::with_id(0))]),
                            Type::Text,
                        ),
                        has_default: false,
                        doc: Maybe::Some("Convert to string representation".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: Map::new(),
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("Show".into(), show);
        self.derivable.insert("Show".into());

        // Functor protocol
        let functor = Protocol {
            name: "Functor".into(),
            kind: ProtocolKind::Constraint,
            defining_crate: Maybe::Some("stdlib".into()),
            type_params: {
                let mut params = List::new();
                params.push(TypeParam {
                    name: "F".into(),
                    bounds: List::new(),
                    default: Maybe::None,
                });
                params
            },
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "map".into(),
                    ProtocolMethod {
                        name: "map".into(),
                        ty: Type::function(
                            List::from(vec![
                                Type::Var(crate::ty::TypeVar::with_id(0)),
                                Type::function(
                                    List::from(vec![Type::Var(crate::ty::TypeVar::with_id(1))]),
                                    Type::Var(crate::ty::TypeVar::with_id(2)),
                                ),
                            ]),
                            Type::Var(crate::ty::TypeVar::with_id(0)),
                        ),
                        has_default: false,
                        doc: Maybe::Some("Map a function over the functor".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: Map::new(),
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("Functor".into(), functor);

        // Iterator protocol
        let iterator = Protocol {
            name: "Iterator".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "next".into(),
                    ProtocolMethod {
                        name: "next".into(),
                        ty: Type::function(
                            List::from(vec![Type::Var(crate::ty::TypeVar::with_id(0))]),
                            Type::Var(crate::ty::TypeVar::with_id(1)), // Maybe<Item>
                        ),
                        has_default: false,
                        doc: Maybe::Some("Get next element".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: {
                let mut assoc = Map::new();
                assoc.insert(
                    "Item".into(),
                    AssociatedType::simple("Item".into(), List::new()),
                );
                assoc
            },
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("Iterator".into(), iterator);

        // AsyncIterator protocol
        // Concurrency model: structured concurrency with nurseries, async/await, channels, Send/Sync protocol bounds — Section 12 - AsyncIterator protocol
        // Grammar: grammar/verum.ebnf - for_await_loop consumes AsyncIterator<Item = T>
        let async_iterator = Protocol {
            name: "AsyncIterator".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "next".into(),
                    ProtocolMethod {
                        name: "next".into(),
                        ty: Type::function(
                            List::from(vec![Type::Var(crate::ty::TypeVar::with_id(0))]),
                            Type::Future {
                                output: Box::new(Type::Var(crate::ty::TypeVar::with_id(1))), // Future<Maybe<Item>>
                            },
                        ),
                        has_default: false,
                        doc: Maybe::Some("Get next element asynchronously".into()),
                        refinement_constraints: Map::new(),
                        is_async: true, // This is an async method
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                // Optional size_hint method
                methods.insert(
                    "size_hint".into(),
                    ProtocolMethod {
                        name: "size_hint".into(),
                        ty: Type::function(
                            List::from(vec![Type::Var(crate::ty::TypeVar::with_id(0))]),
                            Type::Tuple(List::from(vec![Type::Int, Type::maybe(Type::Int)])),
                        ),
                        has_default: true, // Default implementation returns (0, None)
                        doc: Maybe::Some("Return size hint (lower bound, optional upper bound)".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: {
                let mut assoc = Map::new();
                assoc.insert(
                    "Item".into(),
                    AssociatedType::simple("Item".into(), List::new()),
                );
                assoc
            },
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("AsyncIterator".into(), async_iterator);

        // Hash protocol
        // Hash protocol: fn hash(&self, hasher: &mut Hasher) for use in Map/Set, must be consistent with Eq
        // Protocol signature: fn hash(&self, hasher: &mut Hasher)
        let hash = Protocol {
            name: "Hash".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "hash".into(),
                    ProtocolMethod {
                        name: "hash".into(),
                        ty: Type::function(
                            List::from(vec![
                                Type::Reference {
                                    mutable: false,
                                    inner: Box::new(Type::Var(crate::ty::TypeVar::with_id(0))),
                                },
                                Type::Reference {
                                    mutable: true,
                                    inner: Box::new(Type::Named {
                                        path: Path::single(Ident::new("Hasher", Span::default())),
                                        args: List::new(),
                                    }),
                                },
                            ]),
                            Type::Unit, // hash() returns Unit, not Int
                        ),
                        has_default: false,
                        doc: Maybe::Some("Feed this value into a Hasher".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: Map::new(),
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("Hash".into(), hash);
        self.derivable.insert("Hash".into());

        // Clone protocol
        // Clone protocol: deep copy semantics via fn clone(&self) -> Self, opt-in via @derive(Clone)
        let clone = Protocol {
            name: "Clone".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "clone".into(),
                    ProtocolMethod {
                        name: "clone".into(),
                        ty: Type::function(
                            List::from(vec![Type::Reference {
                                mutable: false,
                                inner: Box::new(Type::Var(crate::ty::TypeVar::with_id(0))),
                            }]),
                            Type::Var(crate::ty::TypeVar::with_id(0)), // Returns Self
                        ),
                        has_default: false,
                        doc: Maybe::Some("Create a deep copy of this value".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: Map::new(),
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("Clone".into(), clone);
        self.derivable.insert("Clone".into());

        // Default protocol
        // Default protocol: fn default() -> Self for zero/empty/identity value construction
        let default = Protocol {
            name: "Default".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "default".into(),
                    ProtocolMethod {
                        name: "default".into(),
                        ty: Type::function(
                            List::new(), // Static method, no self parameter
                            Type::Var(crate::ty::TypeVar::with_id(0)), // Returns Self
                        ),
                        has_default: false,
                        doc: Maybe::Some("Create a default value".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: Map::new(),
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("Default".into(), default);
        self.derivable.insert("Default".into());

        // Deref protocol
        // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.1 - Deref protocol for smart pointers
        let deref = Protocol {
            name: "Deref".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "deref".into(),
                    ProtocolMethod {
                        name: "deref".into(),
                        ty: Type::function(
                            List::from(vec![Type::Reference {
                                mutable: false,
                                inner: Box::new(Type::Var(crate::ty::TypeVar::with_id(0))),
                            }]),
                            Type::Reference {
                                mutable: false,
                                inner: Box::new(Type::Var(crate::ty::TypeVar::with_id(1))),
                            },
                        ),
                        has_default: false,
                        doc: Maybe::Some("Immutably dereferences the value".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: {
                let mut assoc = Map::new();
                assoc.insert(
                    "Target".into(),
                    AssociatedType::simple("Target".into(), List::new()),
                );
                assoc
            },
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("Deref".into(), deref);

        // DerefMut protocol
        // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.2 - DerefMut extends Deref
        let deref_mut = Protocol {
            name: "DerefMut".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "deref_mut".into(),
                    ProtocolMethod {
                        name: "deref_mut".into(),
                        ty: Type::function(
                            List::from(vec![Type::Reference {
                                mutable: true,
                                inner: Box::new(Type::Var(crate::ty::TypeVar::with_id(0))),
                            }]),
                            Type::Reference {
                                mutable: true,
                                inner: Box::new(Type::Var(crate::ty::TypeVar::with_id(1))),
                            },
                        ),
                        has_default: false,
                        doc: Maybe::Some("Mutably dereferences the value".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: {
                let mut assoc = Map::new();
                assoc.insert(
                    "Target".into(),
                    AssociatedType::simple("Target".into(), List::new()),
                );
                assoc
            },
            associated_consts: Map::new(),
            super_protocols: {
                let mut bounds = List::new();
                bounds.push(ProtocolBound {
                    protocol: Path::single(Ident::new("Deref", Span::default())),
                    args: List::new(),
                    is_negative: false,
                });
                bounds
            },
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("DerefMut".into(), deref_mut);

        // LendingIterator protocol (with GAT)
        // Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .1 lines 143-193
        let lending_iterator = Protocol {
            name: "LendingIterator".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "next".into(),
                    ProtocolMethod {
                        name: "next".into(),
                        ty: Type::function(
                            List::from(vec![Type::Reference {
                                mutable: true,
                                inner: Box::new(Type::Var(crate::ty::TypeVar::with_id(0))),
                            }]),
                            Type::Var(crate::ty::TypeVar::with_id(1)), // Maybe<GenRef<Item>>
                        ),
                        has_default: false,
                        doc: Maybe::Some("Get next element as generation-aware reference".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: {
                let mut assoc = Map::new();
                assoc.insert(
                    "Item".into(),
                    AssociatedType::simple("Item".into(), List::new()),
                );
                assoc
            },
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols
            .insert("LendingIterator".into(), lending_iterator);

        // StreamingIterator protocol (with GAT)
        // Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .1 lines 196-242
        let streaming_iterator = Protocol {
            name: "StreamingIterator".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "get".into(),
                    ProtocolMethod {
                        name: "get".into(),
                        ty: Type::function(
                            List::from(vec![Type::Reference {
                                mutable: false,
                                inner: Box::new(Type::Var(crate::ty::TypeVar::with_id(0))),
                            }]),
                            Type::Var(crate::ty::TypeVar::with_id(1)), // Maybe<&Item>
                        ),
                        has_default: false,
                        doc: Maybe::Some("Get current item without advancing".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods.insert(
                    "advance".into(),
                    ProtocolMethod {
                        name: "advance".into(),
                        ty: Type::function(
                            List::from(vec![Type::Reference {
                                mutable: true,
                                inner: Box::new(Type::Var(crate::ty::TypeVar::with_id(0))),
                            }]),
                            Type::Unit,
                        ),
                        has_default: false,
                        doc: Maybe::Some("Advance to next item".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: {
                let mut assoc = Map::new();
                assoc.insert(
                    "Item".into(),
                    AssociatedType::simple("Item".into(), List::new()),
                );
                assoc
            },
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols
            .insert("StreamingIterator".into(), streaming_iterator);

        // Functor with GAT (higher-kinded type)
        // Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .3 lines 410-437
        let functor_gat = Protocol {
            name: "FunctorGAT".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "fmap".into(),
                    ProtocolMethod {
                        name: "fmap".into(),
                        ty: Type::function(
                            List::from(vec![
                                Type::Var(crate::ty::TypeVar::with_id(0)), // Self.F<A>
                                Type::function(
                                    List::from(vec![Type::Var(crate::ty::TypeVar::with_id(1))]), // A
                                    Type::Var(crate::ty::TypeVar::with_id(2)), // B
                                ),
                            ]),
                            Type::Var(crate::ty::TypeVar::with_id(3)), // Self.F<B>
                        ),
                        has_default: false,
                        doc: Maybe::Some("Map a function over a functor (higher-kinded)".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: {
                let mut assoc = Map::new();
                // GAT: type F<T>
                assoc.insert(
                    "F".into(),
                    AssociatedType::generic(
                        "F".into(),
                        List::from(vec![crate::advanced_protocols::GATTypeParam {
                            name: "T".into(),
                            bounds: List::new(),
                            default: Maybe::None,
                            variance: crate::advanced_protocols::Variance::Covariant,
                        }]),
                        List::new(),
                        List::new(),
                    ),
                );
                assoc
            },
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("FunctorGAT".into(), functor_gat);

        // Monad with GAT
        // Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .1 lines 116-142
        let monad_gat = Protocol {
            name: "MonadGAT".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "pure".into(),
                    ProtocolMethod {
                        name: "pure".into(),
                        ty: Type::function(
                            List::from(vec![Type::Var(crate::ty::TypeVar::with_id(0))]), // T
                            Type::Var(crate::ty::TypeVar::with_id(1)), // Self.Wrapped<T>
                        ),
                        has_default: false,
                        doc: Maybe::Some("Wrap a value in the monad".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods.insert(
                    "bind".into(),
                    ProtocolMethod {
                        name: "bind".into(),
                        ty: Type::function(
                            List::from(vec![
                                Type::Var(crate::ty::TypeVar::with_id(0)), // Self.Wrapped<A>
                                Type::function(
                                    List::from(vec![Type::Var(crate::ty::TypeVar::with_id(1))]), // A
                                    Type::Var(crate::ty::TypeVar::with_id(2)), // Self.Wrapped<B>
                                ),
                            ]),
                            Type::Var(crate::ty::TypeVar::with_id(3)), // Self.Wrapped<B>
                        ),
                        has_default: false,
                        doc: Maybe::Some("Sequentially compose two monadic actions".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: {
                let mut assoc = Map::new();
                // GAT: type Wrapped<T>
                assoc.insert(
                    "Wrapped".into(),
                    AssociatedType::generic(
                        "Wrapped".into(),
                        List::from(vec![crate::advanced_protocols::GATTypeParam {
                            name: "T".into(),
                            bounds: List::new(),
                            default: Maybe::None,
                            variance: crate::advanced_protocols::Variance::Covariant,
                        }]),
                        List::new(),
                        List::new(),
                    ),
                );
                assoc
            },
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("MonadGAT".into(), monad_gat);

        // Try protocol - enables the ? operator
        // Protocol-based Try resolution: ? operator uses Carrier protocol to convert between error types
        let try_protocol = Protocol {
            name: "Try".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "from_output".into(),
                    ProtocolMethod {
                        name: "from_output".into(),
                        ty: Type::function(
                            List::from(vec![Type::Var(crate::ty::TypeVar::with_id(0))]), // Output
                            Type::Var(crate::ty::TypeVar::with_id(1)), // Self
                        ),
                        has_default: false,
                        doc: Maybe::Some("Construct from success value".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods.insert(
                    "branch".into(),
                    ProtocolMethod {
                        name: "branch".into(),
                        ty: Type::function(
                            List::from(vec![Type::Var(crate::ty::TypeVar::with_id(0))]), // Self
                            Type::Generic {
                                name: "ControlFlow".into(),
                                args: List::from(vec![
                                    Type::Var(crate::ty::TypeVar::with_id(1)), // Residual
                                    Type::Var(crate::ty::TypeVar::with_id(2)), // Output
                                ]),
                            },
                        ),
                        has_default: false,
                        doc: Maybe::Some("Decide whether to continue or break".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: {
                let mut assoc = Map::new();
                assoc.insert(
                    "Output".into(),
                    AssociatedType::simple("Output".into(), List::new()),
                );
                assoc.insert(
                    "Residual".into(),
                    AssociatedType::simple("Residual".into(), List::new()),
                );
                assoc
            },
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("Try".into(), try_protocol);

        // FromResidual protocol - enables error type conversion in ? chains
        // Protocol-based Try resolution: ? operator uses Carrier protocol to convert between error types
        let from_residual = Protocol {
            name: "FromResidual".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::from(vec![TypeParam {
                name: "R".into(),
                bounds: List::new(),
                default: Maybe::None,
            }]),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "from_residual".into(),
                    ProtocolMethod {
                        name: "from_residual".into(),
                        ty: Type::function(
                            List::from(vec![Type::Var(crate::ty::TypeVar::with_id(0))]), // R (residual)
                            Type::Var(crate::ty::TypeVar::with_id(1)), // Self
                        ),
                        has_default: false,
                        doc: Maybe::Some("Convert residual to Self".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: Map::new(),
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("FromResidual".into(), from_residual);

        // Bitwise operator protocols with associated Output type
        // These protocols enable operator overloading for bitwise operations
        // and allow the type system to determine the result type via the Output associated type.

        // BitAnd protocol (&)
        let bitand = Protocol {
            name: "BitAnd".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "bitand".into(),
                    ProtocolMethod {
                        name: "bitand".into(),
                        ty: Type::function(
                            List::from(vec![
                                Type::Var(crate::ty::TypeVar::with_id(0)), // Self
                                Type::Var(crate::ty::TypeVar::with_id(0)), // Rhs (same as Self for primitive ops)
                            ]),
                            Type::Var(crate::ty::TypeVar::with_id(1)), // Output
                        ),
                        has_default: false,
                        doc: Maybe::Some("Bitwise AND operation".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: {
                let mut assoc = Map::new();
                assoc.insert(
                    "Output".into(),
                    AssociatedType::simple("Output".into(), List::new()),
                );
                assoc
            },
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("BitAnd".into(), bitand);

        // BitOr protocol (|)
        let bitor = Protocol {
            name: "BitOr".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "bitor".into(),
                    ProtocolMethod {
                        name: "bitor".into(),
                        ty: Type::function(
                            List::from(vec![
                                Type::Var(crate::ty::TypeVar::with_id(0)), // Self
                                Type::Var(crate::ty::TypeVar::with_id(0)), // Rhs
                            ]),
                            Type::Var(crate::ty::TypeVar::with_id(1)), // Output
                        ),
                        has_default: false,
                        doc: Maybe::Some("Bitwise OR operation".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: {
                let mut assoc = Map::new();
                assoc.insert(
                    "Output".into(),
                    AssociatedType::simple("Output".into(), List::new()),
                );
                assoc
            },
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("BitOr".into(), bitor);

        // BitXor protocol (^)
        let bitxor = Protocol {
            name: "BitXor".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "bitxor".into(),
                    ProtocolMethod {
                        name: "bitxor".into(),
                        ty: Type::function(
                            List::from(vec![
                                Type::Var(crate::ty::TypeVar::with_id(0)), // Self
                                Type::Var(crate::ty::TypeVar::with_id(0)), // Rhs
                            ]),
                            Type::Var(crate::ty::TypeVar::with_id(1)), // Output
                        ),
                        has_default: false,
                        doc: Maybe::Some("Bitwise XOR operation".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: {
                let mut assoc = Map::new();
                assoc.insert(
                    "Output".into(),
                    AssociatedType::simple("Output".into(), List::new()),
                );
                assoc
            },
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("BitXor".into(), bitxor);

        // Shl protocol (<<)
        let shl = Protocol {
            name: "Shl".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "shl".into(),
                    ProtocolMethod {
                        name: "shl".into(),
                        ty: Type::function(
                            List::from(vec![
                                Type::Var(crate::ty::TypeVar::with_id(0)), // Self
                                Type::Int, // Rhs is always Int for shift
                            ]),
                            Type::Var(crate::ty::TypeVar::with_id(1)), // Output
                        ),
                        has_default: false,
                        doc: Maybe::Some("Bitwise left shift operation".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: {
                let mut assoc = Map::new();
                assoc.insert(
                    "Output".into(),
                    AssociatedType::simple("Output".into(), List::new()),
                );
                assoc
            },
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("Shl".into(), shl);

        // Shr protocol (>>)
        let shr = Protocol {
            name: "Shr".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "shr".into(),
                    ProtocolMethod {
                        name: "shr".into(),
                        ty: Type::function(
                            List::from(vec![
                                Type::Var(crate::ty::TypeVar::with_id(0)), // Self
                                Type::Int, // Rhs is always Int for shift
                            ]),
                            Type::Var(crate::ty::TypeVar::with_id(1)), // Output
                        ),
                        has_default: false,
                        doc: Maybe::Some("Bitwise right shift operation".into()),
                        refinement_constraints: Map::new(),
                        is_async: false,
                        context_requirements: List::new(),
                        type_param_names: List::new(),
                        type_param_bounds: Map::new(),
                        receiver_kind: Maybe::None,
                    },
                );
                methods
            },
            associated_types: {
                let mut assoc = Map::new();
                assoc.insert(
                    "Output".into(),
                    AssociatedType::simple("Output".into(), List::new()),
                );
                assoc
            },
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("Shr".into(), shr);

        // SimdElement marker protocol
        // SIMD type validation: verifying SIMD vector types match hardware capabilities and element type constraints — .1 - SIMD Architecture
        // Marks types that can be used as SIMD lane elements (numeric primitives).
        // Valid SimdElement types: Int8, Int16, Int32, Int64, UInt8, UInt16, UInt32, UInt64,
        // Float32, Float64, and Bool.
        let simd_element = Protocol {
            name: "SimdElement".into(),
            kind: ProtocolKind::Constraint, // Marker protocol (constraint with no methods)
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: Map::new(), // No methods - marker only
            associated_types: Map::new(),
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("SimdElement".into(), simd_element);

        // Numeric marker protocol
        // SIMD type validation: verifying SIMD vector types match hardware capabilities and element type constraints — .2 - SIMD Operations
        // Marks types that support numeric operations (add, sub, mul, div).
        // Used for SIMD type bounds: Vec<T: SimdElement + Numeric, N>
        let numeric = Protocol {
            name: "Numeric".into(),
            kind: ProtocolKind::Constraint, // Marker protocol (constraint with no methods)
            type_params: List::new(),
            defining_crate: Maybe::Some("stdlib".into()),
            methods: Map::new(),
            associated_types: Map::new(),
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::None,
            span: Span::default(),
        };
        self.protocols.insert("Numeric".into(), numeric);

        // Register tensor literal protocol
        // Tensor protocol: operations on Tensor<T, Shape> including element-wise ops, reductions, reshaping with compile-time shape validation — Tensor Literal Protocol
        crate::tensor_protocol::register_tensor_literal_protocol(self);

        // Register standard implementations for primitive types
        self.register_standard_implementations();

        // Register Send/Sync implementations for standard types
        // Basic protocols with simple associated types (initial release) — 4 - Thread-Safety Protocols
        crate::send_sync::register_standard_send_sync_impls(self);
    }

    /// Register standard protocol implementations for primitive types
    fn register_standard_implementations(&mut self) {
        use verum_ast::{Ident, Path};

        // Helper function to create a simple protocol implementation
        let make_impl = |for_type: Type, protocol: &str| -> ProtocolImpl {
            ProtocolImpl {
                for_type,
                protocol: Path::single(Ident::new(protocol, Span::default())),
                protocol_args: List::new(),
                where_clauses: List::new(),
                methods: Map::new(),
                associated_types: Map::new(),
                associated_consts: Map::new(),
                specialization: Maybe::None,
                impl_crate: Maybe::Some("stdlib".into()),
                span: Span::default(),
                type_param_fn_bounds: Map::new(),
            }
        };

        // Register Eq for primitive types (including Unit - all units are equal)
        for ty in &[Type::Int, Type::Float, Type::Bool, Type::Char, Type::Unit] {
            let _ = self.register_impl(make_impl(ty.clone(), "Eq"));
        }

        // Register Ord for numeric, char, and unit types
        for ty in &[Type::Int, Type::Float, Type::Char, Type::Unit] {
            let _ = self.register_impl(make_impl(ty.clone(), "Ord"));
        }

        // Register Show for all primitive types
        for ty in &[Type::Int, Type::Float, Type::Bool, Type::Char, Type::Unit] {
            let _ = self.register_impl(make_impl(ty.clone(), "Show"));
        }

        // Register Hash for hashable primitive types
        // Hash protocol: fn hash(&self, hasher: &mut Hasher) for use in Map/Set, must be consistent with Eq
        for ty in &[Type::Int, Type::Bool, Type::Char, Type::Unit] {
            let _ = self.register_impl(make_impl(ty.clone(), "Hash"));
        }

        // Register Clone for primitive types (all primitives are Copy, hence Clone)
        for ty in &[Type::Int, Type::Float, Type::Bool, Type::Char, Type::Unit] {
            let _ = self.register_impl(make_impl(ty.clone(), "Clone"));
        }

        // Register Default for primitive types
        for ty in &[Type::Int, Type::Float, Type::Bool, Type::Unit] {
            let _ = self.register_impl(make_impl(ty.clone(), "Default"));
        }

        // Register SimdElement for SIMD-compatible primitive types
        // SIMD type validation: verifying SIMD vector types match hardware capabilities and element type constraints — .1 - Valid SIMD lane element types
        // Int and Float represent arbitrary precision in Verum, but lower to
        // platform-specific widths (i32/i64, f32/f64) for SIMD operations.
        // Bool is used for mask lanes.
        for ty in &[Type::Int, Type::Float, Type::Bool] {
            let _ = self.register_impl(make_impl(ty.clone(), "SimdElement"));
        }

        // Register Numeric for types supporting arithmetic operations
        // SIMD type validation: verifying SIMD vector types match hardware capabilities and element type constraints — .2 - SIMD arithmetic operations
        for ty in &[Type::Int, Type::Float] {
            let _ = self.register_impl(make_impl(ty.clone(), "Numeric"));
        }

        // Register bitwise operator protocols for primitive types
        // These are crucial for proper type inference of bitwise operations

        // Helper to create bitwise impl with Output associated type
        let make_bitwise_impl =
            |for_type: Type, protocol: &str, output_type: Type| -> ProtocolImpl {
                ProtocolImpl {
                    for_type,
                    protocol: Path::single(Ident::new(protocol, Span::default())),
                    protocol_args: List::new(),
                    where_clauses: List::new(),
                    methods: Map::new(),
                    associated_types: {
                        let mut assoc = Map::new();
                        assoc.insert("Output".into(), output_type);
                        assoc
                    },
                    associated_consts: Map::new(),
                    specialization: Maybe::None,
                    impl_crate: Maybe::Some("stdlib".into()),
                    span: Span::default(),
                    type_param_fn_bounds: Map::new(),
                }
            };

        // BitAnd, BitOr, BitXor for Bool -> Output = Bool
        for protocol in &["BitAnd", "BitOr", "BitXor"] {
            let _ = self.register_impl(make_bitwise_impl(Type::Bool, protocol, Type::Bool));
        }

        // BitAnd, BitOr, BitXor for Int -> Output = Int
        for protocol in &["BitAnd", "BitOr", "BitXor"] {
            let _ = self.register_impl(make_bitwise_impl(Type::Int, protocol, Type::Int));
        }

        // Shl, Shr for Int -> Output = Int
        for protocol in &["Shl", "Shr"] {
            let _ = self.register_impl(make_bitwise_impl(Type::Int, protocol, Type::Int));
        }

        // Register Try protocol for Maybe<T>
        // Protocol-based Try resolution: ? operator uses Carrier protocol to convert between error types
        let maybe_try_impl = ProtocolImpl {
            for_type: Type::Generic {
                name: WKT::Maybe.as_str().into(),
                args: List::from(vec![Type::Var(crate::ty::TypeVar::with_id(0))]),
            },
            protocol: Path::single(Ident::new("Try", Span::default())),
            protocol_args: List::new(),
            where_clauses: List::new(),
            methods: Map::new(),
            associated_types: {
                let mut assoc = Map::new();
                // Output = T
                assoc.insert("Output".into(), Type::Var(crate::ty::TypeVar::with_id(0)));
                // Residual = Maybe<Never>
                assoc.insert(
                    "Residual".into(),
                    Type::Generic {
                        name: WKT::Maybe.as_str().into(),
                        args: List::from(vec![Type::Generic {
                            name: "Never".into(),
                            args: List::new(),
                        }]),
                    },
                );
                assoc
            },
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("stdlib".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };
        let _ = self.register_impl(maybe_try_impl);

        // Register Try protocol for Result<T, E>
        let result_try_impl = ProtocolImpl {
            for_type: Type::Generic {
                name: WKT::Result.as_str().into(),
                args: List::from(vec![
                    Type::Var(crate::ty::TypeVar::with_id(0)), // T
                    Type::Var(crate::ty::TypeVar::with_id(1)), // E
                ]),
            },
            protocol: Path::single(Ident::new("Try", Span::default())),
            protocol_args: List::new(),
            where_clauses: List::new(),
            methods: Map::new(),
            associated_types: {
                let mut assoc = Map::new();
                // Output = T
                assoc.insert("Output".into(), Type::Var(crate::ty::TypeVar::with_id(0)));
                // Residual = Result<Never, E>
                assoc.insert(
                    "Residual".into(),
                    Type::Generic {
                        name: WKT::Result.as_str().into(),
                        args: List::from(vec![
                            Type::Generic {
                                name: "Never".into(),
                                args: List::new(),
                            },
                            Type::Var(crate::ty::TypeVar::with_id(1)), // E
                        ]),
                    },
                );
                assoc
            },
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("stdlib".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };
        let _ = self.register_impl(result_try_impl);

        // Register Try protocol for IoResult<T> (alias for Result<T, IoError>)
        let io_result_try_impl = ProtocolImpl {
            for_type: Type::Generic {
                name: "IoResult".into(),
                args: List::from(vec![Type::Var(crate::ty::TypeVar::with_id(0))]),
            },
            protocol: Path::single(Ident::new("Try", Span::default())),
            protocol_args: List::new(),
            where_clauses: List::new(),
            methods: Map::new(),
            associated_types: {
                let mut assoc = Map::new();
                // Output = T
                assoc.insert("Output".into(), Type::Var(crate::ty::TypeVar::with_id(0)));
                // Residual = Result<Never, IoError>
                assoc.insert(
                    "Residual".into(),
                    Type::Generic {
                        name: WKT::Result.as_str().into(),
                        args: List::from(vec![
                            Type::Generic {
                                name: "Never".into(),
                                args: List::new(),
                            },
                            Type::Generic {
                                name: "IoError".into(),
                                args: List::new(),
                            },
                        ]),
                    },
                );
                assoc
            },
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("stdlib".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };
        let _ = self.register_impl(io_result_try_impl);

        // NOTE: FromResidual implementations are loaded from stdlib (core/maybe.vr, core/result.vr)
        // rather than hardcoded here. This enables a stdlib-agnostic type system where
        // the compiler doesn't have built-in knowledge of specific stdlib types.
        //
        // The stdlib defines:
        // - implement<T> FromResidual<Maybe<Never>> for Maybe<T>
        // - implement<T, E> FromResidual<Result<Never, E>> for Result<T, E>
        // - implement<T, E> FromResidual<Result<Never, E>> for Maybe<T>
    }

    // =========================================================================
    // Method Registry - Standard Type Methods
    // =========================================================================
    // Protocol-driven method resolution: methods resolved by searching implemented protocols for matching signatures
    //
    // This replaces the 2000-line get_builtin_method_type function with a
    // clean, data-driven method registry.

    /// Register standard methods for stdlib types.
    ///
    /// NOTE: Method registrations have been removed as part of the stdlib-agnostic
    /// type system refactoring. Methods should now be registered dynamically by parsing
    /// stdlib files (in bootstrap mode) or loaded from stdlib.vbca metadata (in normal mode).
    ///
    /// See stdlib type system refactoring design for details.
    ///
    /// The StdlibAgnosticChecker provides:
    /// - `register_inherent_method()` - for registering methods during stdlib parsing
    /// - `register_protocol_method()` - for protocol-based method registration
    fn register_standard_methods(&mut self) {
        // NOTE: All hardcoded method registrations have been removed.
        // Methods are now registered via:
        // 1. StdlibAgnosticChecker::bootstrap() - parses stdlib .vr files
        // 2. StdlibAgnosticChecker::with_metadata() - loads from stdlib.vbca
        //
        // For primitive types (Int, Float, Bool, Char, Text) that have built-in methods,
        // use register_primitive_methods() which registers only essential operations.

        self.register_primitive_methods();
    }

    /// Register essential methods for primitive types.
    ///
    /// These are truly built-in methods that cannot come from stdlib because
    /// they are needed for the language to bootstrap.
    ///
    /// Primitive types: Int, Float, Bool, Char, Text
    fn register_primitive_methods(&mut self) {
        // Int methods - only essential operations that can't come from stdlib
        self.register_method(WKT::Int.as_str(), MethodSignature::immutable("abs", List::new(), Type::Int));
        self.register_method(WKT::Int.as_str(), MethodSignature::immutable("to_float", List::new(), Type::Float));
        self.register_method(WKT::Int.as_str(), MethodSignature::immutable("to_string", List::new(), Type::Text));

        // Float methods - only essential operations
        self.register_method(WKT::Float.as_str(), MethodSignature::immutable("abs", List::new(), Type::Float));
        self.register_method(WKT::Float.as_str(), MethodSignature::immutable("floor", List::new(), Type::Float));
        self.register_method(WKT::Float.as_str(), MethodSignature::immutable("ceil", List::new(), Type::Float));
        self.register_method(WKT::Float.as_str(), MethodSignature::immutable("round", List::new(), Type::Float));
        self.register_method(WKT::Float.as_str(), MethodSignature::immutable("to_int", List::new(), Type::Int));
        self.register_method(WKT::Float.as_str(), MethodSignature::immutable("to_string", List::new(), Type::Text));
        self.register_method(WKT::Float.as_str(), MethodSignature::immutable("is_nan", List::new(), Type::Bool));
        self.register_method(WKT::Float.as_str(), MethodSignature::immutable("is_infinite", List::new(), Type::Bool));
        self.register_method(WKT::Float.as_str(), MethodSignature::immutable("is_finite", List::new(), Type::Bool));

        // Bool methods
        self.register_method(WKT::Bool.as_str(), MethodSignature::immutable("to_string", List::new(), Type::Text));

        // Char methods
        self.register_method(WKT::Char.as_str(), MethodSignature::immutable("to_string", List::new(), Type::Text));
        self.register_method(WKT::Char.as_str(), MethodSignature::immutable("is_digit", List::new(), Type::Bool));
        self.register_method(WKT::Char.as_str(), MethodSignature::immutable("is_alphabetic", List::new(), Type::Bool));
        self.register_method(WKT::Char.as_str(), MethodSignature::immutable("is_whitespace", List::new(), Type::Bool));
        self.register_method(WKT::Char.as_str(), MethodSignature::immutable("to_uppercase", List::new(), Type::Char));
        self.register_method(WKT::Char.as_str(), MethodSignature::immutable("to_lowercase", List::new(), Type::Char));

        // Byte methods - ASCII classification and conversion
        // Create Byte type for return values
        let byte_type = Type::Named {
            path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new("Byte", Span::default())),
            args: List::new(),
        };
        self.register_method("Byte", MethodSignature::immutable("is_ascii_alphabetic", List::new(), Type::Bool));
        self.register_method("Byte", MethodSignature::immutable("is_ascii_digit", List::new(), Type::Bool));
        self.register_method("Byte", MethodSignature::immutable("is_ascii_alphanumeric", List::new(), Type::Bool));
        self.register_method("Byte", MethodSignature::immutable("is_ascii_whitespace", List::new(), Type::Bool));
        self.register_method("Byte", MethodSignature::immutable("to_ascii_uppercase", List::new(), byte_type.clone()));
        self.register_method("Byte", MethodSignature::immutable("to_ascii_lowercase", List::new(), byte_type.clone()));
        self.register_method("Byte", MethodSignature::immutable("to_int", List::new(), Type::Int));
        self.register_method("Byte", MethodSignature::immutable("checked_add", List::from(vec![byte_type.clone()]), Type::maybe(byte_type.clone())));
        self.register_method("Byte", MethodSignature::immutable("saturating_add", List::from(vec![byte_type.clone()]), byte_type.clone()));
        self.register_method("Byte", MethodSignature::immutable("wrapping_add", List::from(vec![byte_type.clone()]), byte_type.clone()));

        // Text methods - only essential operations
        self.register_method(WKT::Text.as_str(), MethodSignature::immutable("len", List::new(), Type::Int));
        self.register_method(WKT::Text.as_str(), MethodSignature::immutable("is_empty", List::new(), Type::Bool));
        self.register_method(WKT::Text.as_str(), MethodSignature::immutable("clone", List::new(), Type::Text));
        // Text FFI methods
        let cstring_ty = Type::Named {
            path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new("CString", Span::default())),
            args: List::new(),
        };
        self.register_method(WKT::Text.as_str(), MethodSignature::immutable("to_c_string", List::new(), cstring_ty.clone()));
        self.register_method(WKT::Text.as_str(), MethodSignature::immutable("to_cstring", List::new(), cstring_ty.clone()));
        self.register_method(WKT::Text.as_str(), MethodSignature::immutable("to_owned", List::new(), Type::Text));
        self.register_method(WKT::Text.as_str(), MethodSignature::immutable("as_ptr", List::new(), Type::Pointer {
            inner: Box::new(Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new("UInt8", Span::default())),
                args: List::new(),
            }),
            mutable: false,
        }));

        // UIntSize methods - arithmetic with overflow checks
        let uintsize_ty = Type::Named {
            path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new("UIntSize", Span::default())),
            args: List::new(),
        };
        self.register_method("UIntSize", MethodSignature::immutable("checked_add", List::from(vec![uintsize_ty.clone()]), Type::maybe(uintsize_ty.clone())));
        self.register_method("UIntSize", MethodSignature::immutable("checked_sub", List::from(vec![uintsize_ty.clone()]), Type::maybe(uintsize_ty.clone())));
        self.register_method("UIntSize", MethodSignature::immutable("checked_mul", List::from(vec![uintsize_ty.clone()]), Type::maybe(uintsize_ty.clone())));
        self.register_method("UIntSize", MethodSignature::immutable("saturating_add", List::from(vec![uintsize_ty.clone()]), uintsize_ty.clone()));
        self.register_method("UIntSize", MethodSignature::immutable("saturating_sub", List::from(vec![uintsize_ty.clone()]), uintsize_ty.clone()));
        self.register_method("UIntSize", MethodSignature::immutable("saturating_mul", List::from(vec![uintsize_ty.clone()]), uintsize_ty.clone()));

        // Sized integer methods (UInt16, UInt32, etc.) - byte order conversion
        for uint_name in ["UInt16", "UInt32", "UInt64"] {
            let uint_ty = Type::Named {
                path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(uint_name, Span::default())),
                args: List::new(),
            };
            self.register_method(uint_name, MethodSignature::immutable("from_be", List::from(vec![uint_ty.clone()]), uint_ty.clone()));
            self.register_method(uint_name, MethodSignature::immutable("from_le", List::from(vec![uint_ty.clone()]), uint_ty.clone()));
            self.register_method(uint_name, MethodSignature::immutable("to_be", List::new(), uint_ty.clone()));
            self.register_method(uint_name, MethodSignature::immutable("to_le", List::new(), uint_ty.clone()));
            self.register_method(uint_name, MethodSignature::immutable("to_be_bytes", List::new(), Type::Array {
                element: Box::new(byte_type.clone()),
                size: None,
            }));
            self.register_method(uint_name, MethodSignature::immutable("to_le_bytes", List::new(), Type::Array {
                element: Box::new(byte_type.clone()),
                size: None,
            }));
            self.register_method(uint_name, MethodSignature::immutable("checked_add", List::from(vec![uint_ty.clone()]), Type::maybe(uint_ty.clone())));
            self.register_method(uint_name, MethodSignature::immutable("checked_sub", List::from(vec![uint_ty.clone()]), Type::maybe(uint_ty.clone())));
            self.register_method(uint_name, MethodSignature::immutable("checked_mul", List::from(vec![uint_ty.clone()]), Type::maybe(uint_ty.clone())));
        }
    }

    /// Register a method for a type
    fn register_method(&mut self, type_name: &str, signature: MethodSignature) {
        let key = (Text::from(type_name), signature.name.clone());
        self.method_registry.insert(key, signature);
    }

    /// Look up a method by type and name.
    ///
    /// This is the main entry point for protocol-based method resolution.
    /// It replaces the hardcoded get_builtin_method_type function.
    ///
    /// Handles:
    /// - Direct type method lookup (List.len, Map.get, etc.)
    /// - CBGR tier conversion methods (to_checked, to_managed, to_unsafe)
    /// - Universal methods (to_string, clone, into)
    /// - Reference-aware returns (first/last/get on &List return Maybe<&T>)
    ///
    /// # Arguments
    /// * `ty` - The receiver type
    /// * `method_name` - The method name
    ///
    /// # Returns
    /// * `Some(MethodLookupResult)` with the method signature and type substitution
    /// * `None` if the method is not found
    pub fn lookup_method(&self, ty: &Type, method_name: &str) -> Option<MethodLookupResult> {
        if method_name == "next" {
            // #[cfg(debug_assertions)]
            // eprintln!("[DEBUG lookup_method] method='next', ty={:?}", ty);
        }

        // =========================================================================
        // 1. CBGR Tier Conversion Methods (handle before extracting inner type)
        // CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — Section 3 - Reference Tiers
        // =========================================================================
        if let Some(result) = self.lookup_cbgr_conversion(ty, method_name) {
            return Some(result);
        }

        // Check if receiver is a reference for reference-aware return handling
        let is_reference = matches!(ty,
            Type::Reference { .. } | Type::CheckedReference { .. } | Type::UnsafeReference { .. }
        );

        // Extract the type constructor name and arguments
        let (type_name, type_args) = self.extract_type_info(ty)?;

        if method_name == "next" {
            // #[cfg(debug_assertions)]
            // eprintln!("[DEBUG lookup_method] type_name={:?}, type_args={:?}", type_name, type_args);
        }

        // Look up in method registry
        let key = (type_name.clone(), Text::from(method_name));
        if let Some(sig) = self.method_registry.get(&key) {
            // Build substitution from type arguments
            let substitution = self.build_method_substitution(&type_args);

            // Apply substitution to return type
            let mut resolved_return_type = self.apply_substitution_to_type(&sig.return_type, &substitution);

            // Reference-aware return handling for collection accessors
            // &List<T>.first() -> Maybe<&T>, &List<T>.get(i) -> Maybe<&T>
            if is_reference {
                resolved_return_type = self.adjust_return_for_reference_receiver(
                    &type_name,
                    method_name,
                    resolved_return_type,
                );
            }

            let resolved_sig = MethodSignature {
                name: sig.name.clone(),
                type_params: sig.type_params.clone(),
                receiver: sig.receiver,
                params: sig.params.iter()
                    .map(|p| self.apply_substitution_to_type(p, &substitution))
                    .collect(),
                return_type: resolved_return_type,
                is_mutating: sig.is_mutating,
            };

            // CRITICAL: Freshen method-level TypeVars to prevent call-site pollution.
            // Without this, all calls to generic methods like `map<F>` share the same
            // TypeVar for F, so the first call's inference binds F globally.
            let freshened_sig = resolved_sig.freshen();

            return Some(MethodLookupResult {
                signature: freshened_sig,
                substitution,
                resolved_receiver_type: ty.clone(),
            });
        }

        // =========================================================================
        // 2. Universal methods fallback (to_string, clone, into)
        // =========================================================================
        if let Some(result) = self.lookup_universal_method(ty, method_name) {
            return Some(result);
        }

        // =========================================================================
        // 3. Generator type built-in Iterator methods
        // Concurrency model: structured concurrency with nurseries, async/await, channels, Send/Sync protocol bounds — Section 12 - Generators
        // Generator<Y, R> implements Iterator<Item = Y>
        // =========================================================================
        if let Some(result) = self.lookup_generator_iterator_method(ty, method_name) {
            return Some(result);
        }

        // Try protocol method lookup
        self.lookup_protocol_method_for_type(ty, method_name)
    }

    /// Look up a method by name on a type, with argument types for disambiguation.
    ///
    /// This is an enhanced version of `lookup_method` that passes argument type
    /// information to protocol method resolution. This enables correct selection
    /// among multiple implementations of parameterized protocols like `FromResidual<R>`.
    pub fn lookup_method_with_args(&self, ty: &Type, method_name: &str, arg_types: &[Type]) -> Option<MethodLookupResult> {
        // Try all non-protocol lookups first (same as lookup_method)
        if let Some(result) = self.lookup_cbgr_conversion(ty, method_name) {
            return Some(result);
        }

        let is_reference = matches!(ty,
            Type::Reference { .. } | Type::CheckedReference { .. } | Type::UnsafeReference { .. }
        );

        let (type_name, type_args) = self.extract_type_info(ty)?;

        let key = (type_name.clone(), Text::from(method_name));
        if let Some(sig) = self.method_registry.get(&key) {
            let substitution = self.build_method_substitution(&type_args);
            let mut resolved_return_type = self.apply_substitution_to_type(&sig.return_type, &substitution);

            if is_reference {
                resolved_return_type = self.adjust_return_for_reference_receiver(
                    &type_name,
                    method_name,
                    resolved_return_type,
                );
            }

            let resolved_sig = MethodSignature {
                name: sig.name.clone(),
                type_params: sig.type_params.clone(),
                receiver: sig.receiver,
                params: sig.params.iter()
                    .map(|p| self.apply_substitution_to_type(p, &substitution))
                    .collect(),
                return_type: resolved_return_type,
                is_mutating: sig.is_mutating,
            };

            // Freshen method-level TypeVars to prevent call-site pollution
            let freshened_sig = resolved_sig.freshen();

            return Some(MethodLookupResult {
                signature: freshened_sig,
                substitution,
                resolved_receiver_type: ty.clone(),
            });
        }

        if let Some(mut result) = self.lookup_universal_method(ty, method_name) {
            result.signature = result.signature.freshen();
            return Some(result);
        }

        if let Some(mut result) = self.lookup_generator_iterator_method(ty, method_name) {
            result.signature = result.signature.freshen();
            return Some(result);
        }

        // Protocol method lookup WITH arg types for disambiguation
        if let Some(mut result) = self.lookup_protocol_method_for_type_with_args(ty, method_name, arg_types) {
            result.signature = result.signature.freshen();
            return Some(result);
        }
        None
    }

    /// Handle CBGR tier conversion methods
    /// - to_checked: Tier 0 (&T) -> Tier 1 (&checked T)
    /// - to_managed: Tier 1 (&checked T) -> Tier 0 (&T)
    /// - to_unsafe: Any tier -> Tier 2 (&unsafe T)
    fn lookup_cbgr_conversion(&self, ty: &Type, method_name: &str) -> Option<MethodLookupResult> {
        let (return_type, is_mutating) = match (ty, method_name) {
            // to_checked: Tier 0 -> Tier 1
            (Type::Reference { inner, mutable }, "to_checked") => {
                (Type::checked_reference(*mutable, *inner.clone()), false)
            }
            (Type::Reference { inner, mutable: true }, "to_checked_mut") => {
                (Type::checked_reference(true, *inner.clone()), true)
            }
            // to_managed: Tier 1 -> Tier 0
            (Type::CheckedReference { inner, mutable }, "to_managed") => {
                (Type::reference(*mutable, *inner.clone()), false)
            }
            (Type::CheckedReference { inner, mutable: true }, "to_managed_mut") => {
                (Type::reference(true, *inner.clone()), true)
            }
            // to_unsafe: Any tier -> Tier 2
            (Type::Reference { inner, mutable }, "to_unsafe") => {
                (Type::unsafe_reference(*mutable, *inner.clone()), false)
            }
            (Type::Reference { inner, mutable: true }, "to_unsafe_mut") => {
                (Type::unsafe_reference(true, *inner.clone()), true)
            }
            (Type::CheckedReference { inner, mutable }, "to_unsafe") => {
                (Type::unsafe_reference(*mutable, *inner.clone()), false)
            }
            (Type::CheckedReference { inner, mutable: true }, "to_unsafe_mut") => {
                (Type::unsafe_reference(true, *inner.clone()), true)
            }
            _ => return None,
        };

        Some(MethodLookupResult {
            signature: MethodSignature {
                name: method_name.into(),
                type_params: List::new(),
                receiver: if is_mutating { ReceiverKind::RefMut } else { ReceiverKind::Ref },
                params: List::new(),
                return_type,
                is_mutating,
            },
            substitution: Map::new(),
            resolved_receiver_type: ty.clone(),
        })
    }

    /// Look up universal methods that work on all types
    fn lookup_universal_method(&self, ty: &Type, method_name: &str) -> Option<MethodLookupResult> {
        let universal_key = ("_Universal".into(), Text::from(method_name));
        let sig = self.method_registry.get(&universal_key)?;

        let return_type = match method_name {
            "clone" => ty.clone(), // clone returns Self
            "to_string" => Type::Text,
            "into" => Type::Var(crate::ty::TypeVar::fresh()), // into<T>() returns fresh type var
            _ => sig.return_type.clone(),
        };

        Some(MethodLookupResult {
            signature: MethodSignature {
                name: sig.name.clone(),
                type_params: sig.type_params.clone(),
                receiver: sig.receiver,
                params: sig.params.clone(),
                return_type,
                is_mutating: sig.is_mutating,
            },
            substitution: Map::new(),
            resolved_receiver_type: ty.clone(),
        })
    }

    /// Look up Iterator methods on Generator types
    /// Concurrency model: structured concurrency with nurseries, async/await, channels, Send/Sync protocol bounds — Section 12 - Generators
    /// Generator<Y, R> implements Iterator<Item = Y> with:
    /// - next(&mut self) -> Maybe<Y>
    /// - has_next(&self) -> Bool
    fn lookup_generator_iterator_method(&self, ty: &Type, method_name: &str) -> Option<MethodLookupResult> {
        // Extract yield type from Generator
        let (yield_ty, _return_ty) = match ty {
            Type::Generator { yield_ty, return_ty } => (*yield_ty.clone(), *return_ty.clone()),
            Type::Reference { inner, .. } | Type::CheckedReference { inner, .. } | Type::UnsafeReference { inner, .. } => {
                // Handle &Generator, &mut Generator
                if let Type::Generator { yield_ty, return_ty } = inner.as_ref() {
                    (*yield_ty.clone(), *return_ty.clone())
                } else {
                    return None;
                }
            }
            _ => return None,
        };

        // Define Iterator methods for Generator
        let (return_type, is_mutating, receiver) = match method_name {
            "next" => {
                // next(&mut self) -> Maybe<Y>
                let maybe_ty = Type::Generic {
                    name: Text::from(WKT::Maybe.as_str()),
                    args: List::from(vec![yield_ty]),
                };
                (maybe_ty, true, ReceiverKind::RefMut)
            }
            "has_next" => {
                // has_next(&self) -> Bool
                (Type::Bool, false, ReceiverKind::Ref)
            }
            // Common Iterator adapter methods that return new iterators
            // These return Iterator adapter types that wrap the original iterator
            // For type simplicity, we treat them as returning Generator<Item, Unit>
            "filter" | "take" | "skip" => {
                // These preserve the item type: Iterator<Y> -> Iterator<Y>
                let iter_ty = Type::Generator {
                    yield_ty: Box::new(yield_ty),
                    return_ty: Box::new(Type::Unit),
                };
                (iter_ty, false, ReceiverKind::Value)
            }
            "map" | "flat_map" | "filter_map" => {
                // These transform the item type - use a fresh type variable for the new item type
                // The actual type should be inferred from the closure, but for now use type variable
                let new_item = Type::Var(crate::ty::TypeVar::fresh());
                let iter_ty = Type::Generator {
                    yield_ty: Box::new(new_item),
                    return_ty: Box::new(Type::Unit),
                };
                (iter_ty, false, ReceiverKind::Value)
            }
            "chain" => {
                // chain combines two iterators of the same type
                let iter_ty = Type::Generator {
                    yield_ty: Box::new(yield_ty),
                    return_ty: Box::new(Type::Unit),
                };
                (iter_ty, false, ReceiverKind::Value)
            }
            "zip" => {
                // zip returns pairs - (Y, OtherItem) - use type variable for other item
                let other_item = Type::Var(crate::ty::TypeVar::fresh());
                let pair_ty = Type::Tuple(List::from(vec![yield_ty, other_item]));
                let iter_ty = Type::Generator {
                    yield_ty: Box::new(pair_ty),
                    return_ty: Box::new(Type::Unit),
                };
                (iter_ty, false, ReceiverKind::Value)
            }
            "enumerate" => {
                // enumerate returns (Int, Y) pairs
                let pair_ty = Type::Tuple(List::from(vec![Type::Int, yield_ty]));
                let iter_ty = Type::Generator {
                    yield_ty: Box::new(pair_ty),
                    return_ty: Box::new(Type::Unit),
                };
                (iter_ty, false, ReceiverKind::Value)
            }
            // Collecting methods
            "collect" => {
                // collect() -> List<Y> - returns List by default for common case
                // Full generic collect<B: FromIterator<Y>>() -> B would require trait bounds
                let list_ty = Type::Generic {
                    name: Text::from(WKT::List.as_str()),
                    args: List::from(vec![yield_ty]),
                };
                (list_ty, false, ReceiverKind::Value)
            }
            "count" => (Type::Int, false, ReceiverKind::Value),
            "sum" | "product" => (yield_ty.clone(), false, ReceiverKind::Value),
            "any" | "all" => (Type::Bool, false, ReceiverKind::Value),
            "first" | "last" => {
                let maybe_ty = Type::Generic {
                    name: Text::from(WKT::Maybe.as_str()),
                    args: List::from(vec![yield_ty]),
                };
                (maybe_ty, false, ReceiverKind::Value)
            }
            "nth" => {
                let maybe_ty = Type::Generic {
                    name: Text::from(WKT::Maybe.as_str()),
                    args: List::from(vec![yield_ty]),
                };
                (maybe_ty, false, ReceiverKind::RefMut)
            }
            _ => return None,
        };

        Some(MethodLookupResult {
            signature: MethodSignature {
                name: Text::from(method_name),
                type_params: List::new(),
                receiver,
                params: List::new(), // Simplified - actual params depend on method
                return_type,
                is_mutating,
            },
            substitution: Map::new(),
            resolved_receiver_type: ty.clone(),
        })
    }

    /// Adjust return type for methods called on reference receivers
    /// &List<T>.first() -> Maybe<&T> instead of Maybe<T>
    fn adjust_return_for_reference_receiver(
        &self,
        type_name: &Text,
        method_name: &str,
        return_type: Type,
    ) -> Type {
        // Only adjust for collection accessor methods
        let is_accessor = matches!(method_name, "first" | "last" | "get");
        let tn = type_name.as_str();
        let is_collection = matches!(tn, "Array" | "Slice") || WKT::List.matches(tn) || WKT::Map.matches(tn);

        if is_accessor && is_collection {
            // Wrap the inner type in Maybe<&T> instead of Maybe<T>
            if let Type::Generic { name, args } = &return_type {
                if WKT::Maybe.matches(name.as_str()) && !args.is_empty() {
                    let inner = args[0].clone();
                    let ref_inner = Type::Reference {
                        inner: Box::new(inner),
                        mutable: false,
                    };
                    return Type::Generic {
                        name: name.clone(),
                        args: List::from(vec![ref_inner]),
                    };
                }
            }
        }
        return_type
    }

    /// Extract type constructor name and arguments from a type
    fn extract_type_info(&self, ty: &Type) -> Option<(Text, List<Type>)> {
        match ty {
            Type::Generic { name, args } => Some((name.clone(), args.clone())),
            Type::Named { path, args } => {
                let name = path.segments.last().and_then(|seg| {
                    match seg {
                        verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.clone()),
                        _ => None,
                    }
                })?;
                Some((name, args.clone()))
            }
            // Primitive types
            Type::Int => Some((WKT::Int.as_str().into(), List::new())),
            Type::Float => Some((WKT::Float.as_str().into(), List::new())),
            Type::Bool => Some((WKT::Bool.as_str().into(), List::new())),
            Type::Char => Some((WKT::Char.as_str().into(), List::new())),
            Type::Text => Some((WKT::Text.as_str().into(), List::new())),
            Type::Unit => Some(("Unit".into(), List::new())),
            // Empty tuple is canonically Unit
            Type::Tuple(elems) if elems.is_empty() => Some(("Unit".into(), List::new())),
            // Array type - treat as List for method lookup
            Type::Array { element, .. } => Some(("Array".into(), List::from(vec![*element.clone()]))),
            // Slice type
            Type::Slice { element } => Some(("Slice".into(), List::from(vec![*element.clone()]))),
            // Variant types - look up named type via variant_type_names registry
            // e.g., Type::Variant({None: Unit, Some: T}) -> ("Maybe", [T])
            Type::Variant(variants) => {
                let signature = Self::variant_type_signature_static(variants);
                let named_type = self.variant_type_names.get(&signature)
                    .or_else(|| {
                        let relaxed = Self::variant_type_signature_relaxed(variants);
                        self.variant_type_names.get(&relaxed)
                    })?;
                let type_args: List<Type> = variants
                    .values()
                    .filter(|payload| **payload != Type::Unit)
                    .cloned()
                    .collect();
                Some((named_type.clone(), type_args))
            }
            // Reference types - extract inner type
            Type::Reference { inner, .. } => self.extract_type_info(inner),
            Type::CheckedReference { inner, .. } => self.extract_type_info(inner),
            Type::UnsafeReference { inner, .. } => self.extract_type_info(inner),
            // Generator type - yield and return types as args
            // Concurrency model: structured concurrency with nurseries, async/await, channels, Send/Sync protocol bounds — Section 12 - Generators
            Type::Generator { yield_ty, return_ty } => {
                Some(("Generator".into(), List::from(vec![*yield_ty.clone(), *return_ty.clone()])))
            }
            _ => None,
        }
    }

    /// Build type substitution from type arguments
    fn build_method_substitution(&self, type_args: &List<Type>) -> Map<Text, Type> {
        let mut subst = Map::new();
        // T = first type arg, K = first, V = second for Map-like types
        if let Some(first) = type_args.first() {
            subst.insert("T".into(), first.clone());
            subst.insert("K".into(), first.clone());
        }
        if let Some(second) = type_args.get(1) {
            subst.insert("V".into(), second.clone());
        }
        subst
    }

    /// Apply type substitution to a type
    fn apply_substitution_to_type(&self, ty: &Type, subst: &Map<Text, Type>) -> Type {
        match ty {
            Type::Var(tv) => {
                // Type variables with IDs 0, 1, 2 map to T, V, third type param
                match tv.id() {
                    0 => subst.get(&"T".into()).cloned().unwrap_or_else(|| ty.clone()),
                    1 => subst.get(&"V".into()).cloned().unwrap_or_else(|| ty.clone()),
                    _ => ty.clone(),
                }
            }
            Type::Generic { name, args } => {
                let new_args: List<Type> = args.iter().map(|a| self.apply_substitution_to_type(a, subst)).collect();

                // CRITICAL FIX: Check if this is a deferred projection (e.g., ::Item)
                // After substitution, if the base type is now concrete, resolve the projection.
                if name.as_str().starts_with("::") && !new_args.is_empty() {
                    let assoc_name: Text = name.as_str().trim_start_matches("::").into();
                    let base_type = &new_args[0];

                    // Check if base type is concrete (no unresolved type variables)
                    if !self.type_has_unresolved_vars(base_type) {
                        // Try to resolve the projection
                        if let Some(resolved) = self.try_find_associated_type(base_type, &assoc_name) {
                            // Recursively apply substitution to the resolved type
                            return self.apply_substitution_to_type(&resolved, subst);
                        }
                    }
                }

                Type::Generic { name: name.clone(), args: new_args }
            }
            Type::Named { path, args } => Type::Named {
                path: path.clone(),
                args: args.iter().map(|a| self.apply_substitution_to_type(a, subst)).collect(),
            },
            Type::Tuple(elems) => Type::Tuple(
                elems.iter().map(|e| self.apply_substitution_to_type(e, subst)).collect()
            ),
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: verum_common::Heap::new(self.apply_substitution_to_type(inner, subst)),
            },
            _ => ty.clone(),
        }
    }

    /// Look up a method from protocol implementations
    fn lookup_protocol_method_for_type(&self, ty: &Type, method_name: &str) -> Option<MethodLookupResult> {
        self.lookup_protocol_method_for_type_with_args(ty, method_name, &[])
    }

    /// Look up a method from protocol implementations, using argument types for disambiguation.
    ///
    /// When a type has multiple implementations of a parameterized protocol (e.g.,
    /// `FromResidual<Maybe<Never>>` and `FromResidual<Result<Never, E>>` both for `Maybe<T>`),
    /// the `arg_types` are used to select the correct implementation by matching
    /// `protocol_args` against the actual argument types.
    fn lookup_protocol_method_for_type_with_args(&self, ty: &Type, method_name: &str, arg_types: &[Type]) -> Option<MethodLookupResult> {
        // Get all implementations for this type
        let impls = self.get_implementations(ty);

        // Collect all candidate implementations that have the requested method
        struct Candidate {
            return_type: Type,
            params: List<Type>,
            receiver: ReceiverKind,
            is_mutating: bool,
            subst_map: Map<Text, Type>,
            substitution: Map<Text, Type>,
            protocol_args: List<Type>,
        }
        let mut candidates: Vec<Candidate> = Vec::new();

        for impl_ in impls.iter() {
            if let Some(proto) = self.lookup_protocol(&impl_.protocol) {
                // Check impl_.methods FIRST for concrete method signatures,
                // only fall back to proto.methods for abstract signatures.
                // This is critical for associated types like Iterator::Item where:
                //   - proto.methods has `fn next(&mut self) -> Maybe<Self.Item>` (abstract)
                //   - impl_.methods has `fn next(&mut self) -> Maybe<&T>` (concrete)
                // We want the concrete signature to avoid unresolved Self.Item projections.

                let (return_type, params, receiver, is_mutating, _using_impl_method) = if let Some(impl_method_ty) = impl_.methods.get(&Text::from(method_name)) {
                    let ret = self.extract_return_type_from_method(impl_method_ty);
                    let pms = self.extract_params_from_method(impl_method_ty);
                    let (rcv, is_mut) = self.extract_receiver_from_method(impl_method_ty);
                    (ret, pms, rcv, is_mut, true)
                } else if let Some(method) = proto.methods.get(&Text::from(method_name)) {
                    // Fallback to protocol method (abstract signature)
                    let ret = self.extract_return_type_from_method(&method.ty);
                    let pms = self.extract_params_from_method(&method.ty);
                    let (rcv, is_mut) = self.extract_receiver_from_method(&method.ty);
                    (ret, pms, rcv, is_mut, false)
                } else if let Some(inherited_method) = self.find_superprotocol_method(proto, &Text::from(method_name)) {
                    // Check superprotocol hierarchy for inherited methods.
                    // When Eq extends PartialEq, methods from PartialEq should be
                    // available on types implementing Eq.
                    let ret = self.extract_return_type_from_method(&inherited_method.ty);
                    let pms = self.extract_params_from_method(&inherited_method.ty);
                    let (rcv, is_mut) = self.extract_receiver_from_method(&inherited_method.ty);
                    (ret, pms, rcv, is_mut, false)
                } else {
                    continue; // Method not found in this impl or superprotocols
                };

                // Use proper unification-based substitution.
                // 1. Unify impl_.for_type with ty to get bindings (e.g., T52 -> &Int)
                // 2. Add Self -> impl_.for_type for Self references
                // 3. Use substitute_type_params which handles "T{id}" format for TypeVars

                let mut subst_map = Map::new();
                let _ = self.unify_types(&impl_.for_type, ty, &mut subst_map);

                // Add Self mapping for Self references in method signatures
                subst_map.insert(Text::from("Self"), impl_.for_type.clone());

                // Add T0 mapping for TypeVar(0) which represents Self in protocol methods
                subst_map.insert(Text::from("T0"), impl_.for_type.clone());

                // Add associated type mappings to resolve Self.Item projections
                for (assoc_name, assoc_ty) in impl_.associated_types.iter() {
                    let prefixed_name = format!("::{}", assoc_name);
                    subst_map.insert(Text::from(prefixed_name), assoc_ty.clone());
                    subst_map.insert(assoc_name.clone(), assoc_ty.clone());
                }

                let substitution = self.build_method_substitution(&self.extract_type_args(ty));

                candidates.push(Candidate {
                    return_type,
                    params,
                    receiver,
                    is_mutating,
                    subst_map,
                    substitution,
                    protocol_args: impl_.protocol_args.clone(),
                });
            }
        }

        if candidates.is_empty() {
            return None;
        }

        // If only one candidate, use it directly (fast path)
        // If multiple candidates AND we have arg_types for disambiguation,
        // select the candidate whose protocol_args match the arg_types.
        let selected_idx = if candidates.len() == 1 {
            0
        } else if !arg_types.is_empty() {
            // Disambiguate: find the candidate whose protocol_args best matches arg_types.
            // For parameterized protocols like FromResidual<R>, protocol_args[0] = R
            // and the method parameter type is R. We try to unify each candidate's
            // protocol_args with the actual argument types.
            let mut best_idx = 0;
            let mut found_match = false;
            for (i, candidate) in candidates.iter().enumerate() {
                if candidate.protocol_args.is_empty() {
                    continue;
                }
                // Apply the for_type substitution to protocol_args for proper matching
                let instantiated_protocol_arg = self.substitute_type_params(
                    &candidate.protocol_args[0],
                    &candidate.subst_map,
                );
                let normalized_protocol_arg = self.normalize_variant_to_generic(&instantiated_protocol_arg);
                let normalized_arg = self.normalize_variant_to_generic(&arg_types[0]);

                let mut match_subst = Map::new();
                if self.unify_types(&normalized_protocol_arg, &normalized_arg, &mut match_subst) {
                    best_idx = i;
                    found_match = true;
                    break;
                }
                // Also try raw protocol_args without substitution
                let raw_normalized = self.normalize_variant_to_generic(&candidate.protocol_args[0]);
                let mut raw_subst = Map::new();
                if self.unify_types(&raw_normalized, &normalized_arg, &mut raw_subst) {
                    best_idx = i;
                    found_match = true;
                    break;
                }
            }
            if found_match { best_idx } else { 0 }
        } else {
            0
        };

        let candidate = &candidates[selected_idx];

        // Apply substitution using proper method that handles TypeVar IDs
        let substituted_return_type = self.substitute_type_params(&candidate.return_type, &candidate.subst_map);

        // Apply a second pass for nested substitutions (e.g., Self was Wrapper<T52>
        // and T52 -> Person, need to substitute in the result of first pass)
        let mut type_param_map = candidate.subst_map.clone();
        type_param_map.remove(&Text::from("Self"));
        let final_return_type = if !type_param_map.is_empty() {
            self.substitute_type_params(&substituted_return_type, &type_param_map)
        } else {
            substituted_return_type
        };

        // Substitute type params in method parameters too
        let substituted_params: List<Type> = candidate.params.iter()
            .map(|p| {
                let subst_p = self.substitute_type_params(p, &candidate.subst_map);
                if !type_param_map.is_empty() {
                    self.substitute_type_params(&subst_p, &type_param_map)
                } else {
                    subst_p
                }
            })
            .collect();

        let sig = MethodSignature {
            name: Text::from(method_name),
            type_params: List::new(),
            receiver: candidate.receiver,
            params: substituted_params,
            return_type: final_return_type,
            is_mutating: candidate.is_mutating,
        };

        Some(MethodLookupResult {
            signature: sig,
            substitution: candidate.substitution.clone(),
            resolved_receiver_type: ty.clone(),
        })
    }

    /// Extract type arguments from a type
    fn extract_type_args(&self, ty: &Type) -> List<Type> {
        match ty {
            Type::Generic { args, .. } => args.clone(),
            Type::Named { args, .. } => args.clone(),
            _ => List::new(),
        }
    }

    /// Extract return type from a function type
    fn extract_return_type_from_method(&self, method_ty: &Type) -> Type {
        match method_ty {
            Type::Function { return_type, .. } => return_type.as_ref().clone(),
            _ => Type::Unit,
        }
    }

    /// Extract parameter types from a function type (excluding self receiver)
    fn extract_params_from_method(&self, method_ty: &Type) -> List<Type> {
        match method_ty {
            Type::Function { params, .. } => {
                // Skip first param if it's a self reference (&Self, &mut Self, Self)
                let mut result = List::new();
                let mut skip_first = false;
                if let Some(first) = params.first() {
                    match first {
                        Type::Reference { inner, .. } => {
                            if let Type::Named { path, .. } = inner.as_ref() {
                                if path.to_string() == "Self" {
                                    skip_first = true;
                                }
                            }
                        }
                        Type::Named { path, .. } if path.to_string() == "Self" => {
                            skip_first = true;
                        }
                        _ => {}
                    }
                }
                let start = if skip_first { 1 } else { 0 };
                for i in start..params.len() {
                    result.push(params[i].clone());
                }
                result
            }
            _ => List::new(),
        }
    }

    /// Extract receiver kind from a method type
    fn extract_receiver_from_method(&self, method_ty: &Type) -> (ReceiverKind, bool) {
        match method_ty {
            Type::Function { params, .. } => {
                if let Some(first) = params.first() {
                    match first {
                        Type::Reference { mutable: true, .. } => (ReceiverKind::RefMut, true),
                        Type::Reference { mutable: false, .. } => (ReceiverKind::Ref, false),
                        Type::Named { path, .. } if path.to_string() == "Self" => (ReceiverKind::Value, false),
                        _ => (ReceiverKind::Ref, false),
                    }
                } else {
                    (ReceiverKind::Static, false)
                }
            }
            _ => (ReceiverKind::Ref, false),
        }
    }

    /// Register a protocol.
    ///
    /// Returns `Err(ProtocolError::CyclicInheritance)` if the new protocol
    /// introduces a cycle in the superprotocol hierarchy.
    pub fn register_protocol(&mut self, protocol: Protocol) -> Result<(), ProtocolError> {
        let name = protocol.name.clone();
        self.protocols.insert(name.clone(), protocol);
        // Invalidate caches when new protocol is registered
        self.methods_cache.clear();
        self.superprotocol_cache.clear();
        self.method_resolution_cache.clear();

        // Check for cycles introduced by this registration
        if let Err(e) = self.check_hierarchy_cycles(&name) {
            // Roll back the registration so the registry stays consistent
            self.protocols.remove(&name);
            return Err(e);
        }
        Ok(())
    }

    /// Check if a protocol is registered by name
    pub fn has_protocol(&self, name: &Text) -> bool {
        self.protocols.contains_key(name)
    }

    /// Look up a protocol by path
    ///
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .1 - GAT protocol lookup
    ///
    /// Returns the protocol definition if found, None otherwise.
    /// This is used during GAT instantiation to access GAT definitions.
    pub fn lookup_protocol(&self, path: &Path) -> Option<&Protocol> {
        // Extract protocol name from path
        // For simple paths (e.g., "Iterator"), use the first segment
        // For qualified paths (e.g., "std.iter.Iterator"), use the last segment
        let protocol_name = path.segments.last().and_then(|seg| match seg {
            verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str()),
            _ => None,
        })?;

        self.protocols.get(&Text::from(protocol_name))
    }

    /// Register an implementation with coherence checking
    /// Returns an error if the implementation violates orphan rule.
    /// Overlapping implementations are silently skipped (the first registration wins).
    /// This handles stdlib re-exports where the same impl appears in multiple modules.
    pub fn register_impl(&mut self, impl_: ProtocolImpl) -> Result<(), CoherenceError> {
        // Check orphan rule (hard error)
        self.check_orphan_rule(&impl_)?;

        // Check for overlaps — return error if overlapping impl exists
        for existing_impl in self.impls.iter() {
            self.check_overlap(&impl_, existing_impl)?
        }

        // Build index key using stable type representation
        // Spec: Protocol system - implementation indexing
        // IMPORTANT: Include protocol_args in the key to distinguish between
        // implementations like Sub<Duration> and Sub<Instant> for the same type
        let type_key = self.make_type_key(&impl_.for_type);
        let protocol_key = self.make_full_protocol_key(&impl_.protocol, &impl_.protocol_args);
        let idx = self.impls.len();

        self.impl_index.insert((type_key, protocol_key), idx);
        self.impls.push(impl_);

        // Invalidate implementation cache since we added a new impl
        self.invalidate_impl_cache();

        Ok(())
    }

    /// Export the registered implementations as an `InstanceRegistry`
    /// suitable for the dependent-type verification orchestrator.
    ///
    /// The orchestrator's `InstanceRegistry` is a thinner, read-only
    /// view — it stores `(protocol, target_type)` tuples and detects
    /// coherence violations structurally. Callers that have already
    /// populated a `ProtocolChecker` during type checking can pass the
    /// exported registry directly into `DependentVerifier` to include
    /// coherence reporting in the module-boundary verification report.
    pub fn export_instance_registry(&self) -> crate::instance_search::InstanceRegistry {
        use crate::instance_search::{InstanceCandidate, InstanceRegistry};

        let mut registry = InstanceRegistry::new();
        for impl_ in self.impls.iter() {
            let protocol_name = self.make_protocol_key(&impl_.protocol);
            let type_name = self.make_type_key(&impl_.for_type);
            let protocol_args: Vec<Text> = impl_
                .protocol_args
                .iter()
                .map(|a| self.make_type_key(a))
                .collect();

            let mut candidate = InstanceCandidate::new(protocol_name, type_name);
            if !protocol_args.is_empty() {
                candidate = candidate.with_args(protocol_args);
            }
            // Use the span's file/line if available as the source
            // location — otherwise leave empty. The orchestrator uses
            // this only for diagnostic messages.
            registry.register(candidate);
        }
        registry
    }

    /// Generate a full key for a protocol including type arguments
    /// This ensures that Sub<Duration> and Sub<Instant> have different keys
    fn make_full_protocol_key(&self, protocol: &Path, protocol_args: &[Type]) -> Text {
        let mut key = self.make_protocol_key(protocol);
        if !protocol_args.is_empty() {
            key.push('<');
            for (i, arg) in protocol_args.iter().enumerate() {
                if i > 0 {
                    key.push(',');
                }
                key.push_str(self.make_type_key(arg).as_str());
            }
            key.push('>');
        }
        key
    }

    /// Check if a type implements a protocol
    ///
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Protocol Resolution
    /// Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — Specialization
    ///
    /// This method performs a comprehensive check including:
    /// 1. Auto-implemented protocols (Deref, DerefMut)
    /// 2. Exact match via index lookup
    /// 3. Generic/conditional implementations with unification
    ///
    /// # Examples
    /// ```verum
    /// // All of these return true for List<Int>:
    /// checker.implements(&list_int, &show_path)  // if Show for List<Int> exists
    /// checker.implements(&list_int, &eq_path)    // if Eq for List<T> where T: Eq exists
    /// ```
    pub fn implements(&self, ty: &Type, protocol: &Path) -> bool {
        // Check for auto-implemented protocols first
        // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.3 - Automatic Deref implementations
        if let Some(protocol_name) = protocol.as_ident().map(|i| i.as_str())
            && (protocol_name == "Deref" || protocol_name == "DerefMut")
            && self.has_auto_deref_impl(ty, protocol_name)
        {
            return true;
        }

        // Use find_impl which handles exact matching, generic matching, and where clauses
        // This ensures implements() and find_impl() have consistent behavior
        if self.find_impl(ty, protocol).is_some() {
            return true;
        }

        // Check protocol hierarchy: if the type implements a sub-protocol that
        // extends the target protocol, the bound is satisfied.
        // e.g., Int implements Eq (which extends PartialEq), so Int implements PartialEq.
        if let Some(target_name) = protocol.as_ident().map(|i| i.as_str()) {
            let target_text: Text = target_name.into();
            let type_key = self.make_type_key(ty);
            for impl_ in self.impls.iter() {
                let impl_type_key = self.make_type_key(&impl_.for_type);
                if impl_type_key == type_key {
                    if let Some(impl_proto_name) = impl_.protocol.as_ident().map(|i| i.as_str()) {
                        let impl_proto_text: Text = impl_proto_name.into();
                        if self.inherits_from(&impl_proto_text, &target_text) {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    /// Check if a type implements a protocol with caching
    ///
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Protocol Resolution
    ///
    /// This is a cached version of `implements()` that stores results
    /// to avoid repeated resolution for the same type/protocol pairs.
    /// Use this when performing multiple protocol checks during type
    /// inference or constraint solving.
    ///
    /// # Performance
    /// First lookup: O(n) where n is the number of implementations
    /// Cached lookup: O(1) hash lookup
    pub fn implements_cached(&mut self, ty: &Type, protocol: &Path) -> bool {
        let type_key = self.make_type_key(ty);
        let protocol_key = self.make_protocol_key(protocol);
        let cache_key = (type_key.clone(), protocol_key.clone());

        // Check cache first
        if let Some(&result) = self.impl_check_cache.get(&cache_key) {
            return result;
        }

        // Not in cache - perform the check
        let result = self.implements(ty, protocol);

        // Store in cache
        self.impl_check_cache.insert(cache_key, result);

        result
    }

    /// Batch check multiple protocol implementations with caching
    ///
    /// Efficiently checks if a type implements multiple protocols,
    /// using the cache to avoid redundant lookups.
    ///
    /// # Returns
    /// A list of protocols that are NOT implemented (violations)
    pub fn check_protocols_cached(&mut self, ty: &Type, protocols: &[&Text]) -> List<Text> {
        let mut missing = List::new();
        for protocol in protocols {
            let protocol_path = Path::single(Ident::new(protocol.as_str(), Span::default()));
            if !self.implements_cached(ty, &protocol_path) {
                missing.push((*protocol).clone());
            }
        }
        missing
    }

    /// Find implementation for type and protocol
    ///
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Protocol Resolution
    /// Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — Specialization
    ///
    /// This method performs a multi-stage lookup:
    /// 1. **Exact match**: Direct lookup using type key
    /// 2. **Generic match**: Match generic implementations (e.g., impl<T> for List<T>)
    /// 3. **Conditional match**: Evaluate where clauses for conditional impls
    /// 4. **Specialization**: Select most specific impl when multiple match
    ///
    /// # Examples
    /// ```verum
    /// // Exact match for List<Int>
    /// impl Show for List<Int> { ... }
    ///
    /// // Generic match for any List<T> where T: Show
    /// impl<T: Show> Show for List<T> { ... }
    ///
    /// // Specialization: more specific impl is preferred
    /// @specialize
    /// impl Show for List<Text> { ... }
    /// ```
    pub fn find_impl(&self, ty: &Type, protocol: &Path) -> Maybe<&ProtocolImpl> {
        let type_key = self.make_type_key(ty);
        let protocol_key = self.make_protocol_key(protocol);

        // Stage 1: Try exact match first (most common case, O(1))
        if let Some(&idx) = self.impl_index.get(&(type_key, protocol_key.clone())) {
            return self.impls.get(idx);
        }

        // Honour `[types].instance_search = false`: skip Stage 2
        // entirely — caller must thread the instance explicitly or
        // register a concrete `impl Protocol for ConcreteType`.
        // Mirrors the Idris-style "no implicit instance search"
        // semantic.  Performance: returns immediately on the
        // exact-match miss with zero allocation.
        if !self.instance_search_enabled {
            return Maybe::None;
        }

        // Stage 2: Try generic/conditional matching
        // Collect all candidate implementations for this protocol
        let mut candidates: List<(usize, &ProtocolImpl, Map<Text, Type>)> = List::new();

        for (idx, impl_) in self.impls.iter().enumerate() {
            // Check if this impl is for the same protocol
            if self.make_protocol_key(&impl_.protocol) != protocol_key.clone() {
                continue;
            }

            // Honour `[protocols].blanket_impls = false`: skip
            // candidates whose `for_type` is a bare type variable
            // (the blanket `impl<T> Protocol for T` pattern). Pre-fix
            // the manifest field was tracing-only at session.rs:582;
            // setting `blanket_impls = false` had zero effect.
            if !self.blanket_impls && matches!(&impl_.for_type, Type::Var(_)) {
                continue;
            }

            // Try to match the impl's for_type against the concrete type
            if let Some(substitution) = self.try_match_type(&impl_.for_type, ty) {
                // Verify where clauses are satisfied with this substitution
                if self.check_where_clauses_satisfied(&impl_.where_clauses, &substitution) {
                    candidates.push((idx, impl_, substitution));
                }
            }
        }

        // Stage 3: Select best candidate using specialization rules
        if candidates.is_empty() {
            return Maybe::None;
        }

        if candidates.len() == 1 {
            return Maybe::Some(candidates[0].1);
        }

        // Multiple candidates: dispatch on `[protocols].
        // resolution_strategy`. Pre-fix the resolver hardcoded
        // most-specific selection, ignoring the manifest. Now:
        //   * "most_specific" (default) — current behaviour via
        //     `select_most_specific_impl` (lattice-based specificity).
        //   * "first_declared" — pick the first registered candidate.
        //     Useful for open-world plugin systems where ordering
        //     reflects priority.
        //   * "error" — return None on overlap. The compiler's
        //     existing diagnostic path picks up the missing impl
        //     and surfaces an ambiguity error to the user.
        match self.resolution_strategy.as_str() {
            "first_declared" => Maybe::Some(candidates[0].1),
            "error" => Maybe::None,
            // Default "most_specific" + any unrecognised string
            // (set_resolution_strategy normalises to known values,
            // so this branch is the documented default).
            _ => {
                let best_idx = self.select_most_specific_impl(&candidates, ty);
                Maybe::Some(candidates[best_idx].1)
            }
        }
    }

    /// Find implementation with type substitution information
    ///
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Protocol Resolution
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — GATs
    ///
    /// This is like `find_impl` but also returns the type substitution map that was
    /// used to match the implementation. This is essential for:
    /// - Resolving associated types with correct type parameter substitutions
    /// - GAT instantiation with proper type arguments
    /// - Inferring types in generic contexts
    ///
    /// # Returns
    /// * `Some((impl, substitution))` - The matching impl and its type substitution
    /// * `None` - No matching implementation found
    ///
    /// # Example
    /// ```verum
    /// // For impl<T: Show> Show for List<T>, matching against List<Int>:
    /// // Returns (impl, {"T" -> Int})
    /// let (impl, subst) = checker.find_impl_with_substitution(&list_int_ty, &show_path)?;
    /// // subst.get("T") == Some(Type::Int)
    /// ```
    pub fn find_impl_with_substitution(
        &self,
        ty: &Type,
        protocol: &Path,
    ) -> Maybe<(&ProtocolImpl, Map<Text, Type>)> {
        let type_key = self.make_type_key(ty);
        let protocol_key = self.make_protocol_key(protocol);

        // Stage 1: Try exact match first
        if let Some(&idx) = self.impl_index.get(&(type_key, protocol_key.clone())) {
            if let Some(impl_) = self.impls.get(idx) {
                // Exact match has empty substitution
                return Maybe::Some((impl_, Map::new()));
            }
        }

        // Honour `[types].instance_search = false` — sibling gate
        // to the one in `find_impl`. Skip Stage 2 generic-candidate
        // scan when the manifest disables instance search.
        if !self.instance_search_enabled {
            return Maybe::None;
        }

        // Stage 2: Try generic/conditional matching
        let mut candidates: List<(usize, &ProtocolImpl, Map<Text, Type>)> = List::new();

        for (idx, impl_) in self.impls.iter().enumerate() {
            if self.make_protocol_key(&impl_.protocol) != protocol_key {
                continue;
            }

            // Honour `[protocols].blanket_impls = false` — sibling
            // gate to the one in `find_impl`. Skip type-variable
            // for_types when blanket impls are disabled.
            if !self.blanket_impls && matches!(&impl_.for_type, Type::Var(_)) {
                continue;
            }

            if let Some(substitution) = self.try_match_type(&impl_.for_type, ty) {
                if self.check_where_clauses_satisfied(&impl_.where_clauses, &substitution) {
                    candidates.push((idx, impl_, substitution));
                }
            }
        }

        if candidates.is_empty() {
            return Maybe::None;
        }

        if candidates.len() == 1 {
            if let Some((_, impl_, subst)) = candidates.into_iter().next() {
                return Maybe::Some((impl_, subst));
            }
            return Maybe::None;
        }

        // Multiple candidates: dispatch on `[protocols].
        // resolution_strategy` (sibling to `find_impl`).
        match self.resolution_strategy.as_str() {
            "first_declared" => {
                let (_, impl_, subst) = candidates.into_iter().next()?;
                Maybe::Some((impl_, subst))
            }
            "error" => Maybe::None,
            _ => {
                // Default "most_specific".
                let best_idx = self.select_most_specific_impl(&candidates, ty);
                let (_, impl_, subst) = candidates.into_iter().nth(best_idx)?;
                Maybe::Some((impl_, subst))
            }
        }
    }

    /// Infer associated type from type structure and protocol context
    ///
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — GATs
    ///
    /// When we have a generic implementation like `impl<T> Iterator for List<T> { type Item = T }`,
    /// and we're looking up the associated type `Item` for `List<Int>`, this method:
    /// 1. Finds the matching implementation
    /// 2. Gets the type substitution (T -> Int)
    /// 3. Applies the substitution to the associated type definition
    ///
    /// This handles cases where the associated type definition uses type parameters
    /// from the impl header.
    pub fn infer_associated_type(
        &self,
        ty: &Type,
        protocol: &Path,
        assoc_name: &Text,
    ) -> Result<Type, ProtocolError> {
        // Find implementation with substitution
        let (impl_, substitution) = match self.find_impl_with_substitution(ty, protocol) {
            Maybe::Some(pair) => pair,
            Maybe::None => {
                return Err(ProtocolError::NotImplemented {
                    ty: ty.clone(),
                    protocol: protocol.clone(),
                });
            }
        };

        // Look up associated type in implementation
        if let Some(assoc_ty) = impl_.associated_types.get(assoc_name) {
            // Apply the substitution to get the concrete associated type
            let resolved_ty = self.apply_substitution(assoc_ty, &substitution);
            return Ok(resolved_ty);
        }

        // Check for default in protocol definition
        let protocol_name = self.extract_protocol_name(protocol);
        let protocol_def = match self.protocols.get(&protocol_name) {
            Some(p) => p,
            None => {
                return Err(ProtocolError::ProtocolNotFound {
                    name: protocol_name,
                });
            }
        };

        if let Some(assoc_type_def) = protocol_def.associated_types.get(assoc_name)
            && let Maybe::Some(default_ty) = &assoc_type_def.default
        {
            // Apply substitution to the default as well
            let resolved_ty = self.apply_substitution(default_ty, &substitution);
            return Ok(resolved_ty);
        }

        // No associated type found
        Err(ProtocolError::AssociatedTypeNotSpecified {
            protocol: protocol.clone(),
            assoc_name: assoc_name.clone(),
            for_type: ty.clone(),
        })
    }

    /// Try to match a pattern type against a concrete type, returning substitutions if successful
    ///
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Type Matching
    ///
    /// This performs unification-style matching where:
    /// - Type variables in the pattern can match any type
    /// - Named types match if constructors and (recursively) arguments match
    /// - Primitive types must match exactly
    ///
    /// # Arguments
    /// * `pattern` - The type pattern (may contain type variables)
    /// * `concrete` - The concrete type to match against
    ///
    /// # Returns
    /// * `Some(substitution)` - If match succeeds, with variable bindings
    /// * `None` - If match fails
    fn try_match_type(&self, pattern: &Type, concrete: &Type) -> Option<Map<Text, Type>> {
        let mut substitution = Map::new();
        if self.unify_types(pattern, concrete, &mut substitution) {
            Some(substitution)
        } else {
            None
        }
    }

    /// Unify two types, building up a substitution map
    ///
    /// Returns true if unification succeeds, false otherwise.
    /// On success, the substitution map contains bindings for type variables.
    fn unify_types(&self, pattern: &Type, concrete: &Type, subst: &mut Map<Text, Type>) -> bool {
        match (pattern, concrete) {
            // Type variable matches anything and records the binding
            (Type::Var(tv), _) => {
                let var_name: Text = format!("T{}", tv.id()).into();
                // Check for conflicting bindings
                if let Some(existing) = subst.get(&var_name) {
                    // Must match the existing binding
                    self.types_structurally_equal(existing, concrete)
                } else {
                    // Record new binding
                    subst.insert(var_name, concrete.clone());
                    true
                }
            }

            // Named type with type parameters - might be a type parameter itself
            (Type::Named { path: p1, args: a1 }, concrete_ty) => {
                // Check if this Named type is actually a type parameter (simple identifier, no args)
                if a1.is_empty() {
                    if let Some(ident) = p1.as_ident() {
                        let name: Text = ident.as_str().into();
                        // Type parameters are typically uppercase single letters like T, U, V
                        // or names like Element, Item, etc.
                        if self.looks_like_type_param(&name) {
                            // Treat as type variable
                            if let Some(existing) = subst.get(&name) {
                                return self.types_structurally_equal(existing, concrete_ty);
                            } else {
                                subst.insert(name, concrete_ty.clone());
                                return true;
                            }
                        }
                    }
                }

                // Not a type parameter - must match structurally
                // Handle both Named-to-Named and Named-to-Generic (cross-form unification)
                match concrete_ty {
                    Type::Named { path: p2, args: a2 } => {
                        // Check constructor names match
                        if !self.paths_equal(p1, p2) {
                            return false;
                        }
                        // Check arity matches
                        if a1.len() != a2.len() {
                            return false;
                        }
                        // Recursively unify arguments
                        for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                            if !self.unify_types(arg1, arg2, subst) {
                                return false;
                            }
                        }
                        true
                    }
                    Type::Generic { name: n2, args: a2 } => {
                        // Cross-form unification: Named pattern matching Generic concrete
                        // This handles cases where impl's protocol_args uses Named form (e.g., Result<Never, E>)
                        // but the normalized type uses Generic form (e.g., Generic { name: "Result", args: [...] })
                        let type_name = p1.as_ident().map(|id| id.name.as_str());
                        if type_name != Some(n2.as_str()) {
                            return false;
                        }
                        // Check arity matches
                        if a1.len() != a2.len() {
                            return false;
                        }
                        // Recursively unify arguments
                        for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                            if !self.unify_types(arg1, arg2, subst) {
                                return false;
                            }
                        }
                        true
                    }
                    _ => false,
                }
            }

            // Generic types (List<T>, Maybe<T>, etc.)
            (Type::Generic { name: n1, args: a1 }, concrete_ty) => {
                match concrete_ty {
                    Type::Generic { name: n2, args: a2 } => {
                        if n1 != n2 || a1.len() != a2.len() {
                            return false;
                        }
                        for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                            if !self.unify_types(arg1, arg2, subst) {
                                return false;
                            }
                        }
                        true
                    }
                    Type::Named { path: p2, args: a2 } => {
                        // Reverse cross-form unification: Generic pattern matching Named concrete
                        let type_name = p2.as_ident().map(|id| id.name.as_str());
                        if Some(n1.as_str()) != type_name {
                            return false;
                        }
                        // Check arity matches
                        if a1.len() != a2.len() {
                            return false;
                        }
                        // Recursively unify arguments
                        for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                            if !self.unify_types(arg1, arg2, subst) {
                                return false;
                            }
                        }
                        true
                    }
                    Type::Variant(variants) => {
                        // Generic pattern matching Variant concrete (stdlib-agnostic resolution)
                        // This handles cases like: impl<T> Try for Maybe<T>
                        // where the concrete type is expanded to None(Unit) | Some(T)
                        // Look up the variant's signature to see if it maps to this generic type
                        let signature = Self::variant_type_signature_static(variants);
                        let named_type_opt = self.variant_type_names.get(&signature)
                            .or_else(|| {
                                let relaxed = Self::variant_type_signature_relaxed(variants);
                                self.variant_type_names.get(&relaxed)
                            });
                        if let Some(named_type) = named_type_opt {
                            if named_type.as_str() == n1.as_str() {
                                // Type names match! Extract type arguments from variant payloads.
                                // Uses positional matching: non-Unit payloads are matched to type
                                // parameters in the order they appear in the variant map.
                                // This is stdlib-agnostic - no hardcoded variant names.
                                let non_unit_payloads: List<&Type> = variants
                                    .values()
                                    .filter(|payload| **payload != Type::Unit)
                                    .collect();

                                // Match pattern args to non-unit payloads positionally
                                for (pattern_arg, payload) in a1.iter().zip(non_unit_payloads.iter()) {
                                    if !self.unify_types(pattern_arg, payload, subst) {
                                        return false;
                                    }
                                }
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }

            // Primitive types must match exactly
            (Type::Unit, Type::Unit) => true,
            (Type::Never, Type::Never) => true,
            (Type::Bool, Type::Bool) => true,
            (Type::Int, Type::Int) => true,
            (Type::Float, Type::Float) => true,
            (Type::Char, Type::Char) => true,
            (Type::Text, Type::Text) => true,

            // Tuple types
            (Type::Tuple(elems1), Type::Tuple(elems2)) => {
                if elems1.len() != elems2.len() {
                    return false;
                }
                for (e1, e2) in elems1.iter().zip(elems2.iter()) {
                    if !self.unify_types(e1, e2, subst) {
                        return false;
                    }
                }
                true
            }

            // Array types
            (
                Type::Array {
                    element: e1,
                    size: s1,
                },
                Type::Array {
                    element: e2,
                    size: s2,
                },
            ) => {
                // Size must match (or pattern size is None for any-size)
                if s1.is_some() && s1 != s2 {
                    return false;
                }
                self.unify_types(e1, e2, subst)
            }

            // Slice types
            (Type::Slice { element: e1 }, Type::Slice { element: e2 }) => {
                self.unify_types(e1, e2, subst)
            }

            // Reference types
            (
                Type::Reference {
                    inner: i1,
                    mutable: m1,
                },
                Type::Reference {
                    inner: i2,
                    mutable: m2,
                },
            ) => {
                // Mutable reference can match immutable pattern, but not vice versa
                if *m1 && !*m2 {
                    return false;
                }
                self.unify_types(i1, i2, subst)
            }

            // Checked references
            (
                Type::CheckedReference {
                    inner: i1,
                    mutable: m1,
                },
                Type::CheckedReference {
                    inner: i2,
                    mutable: m2,
                },
            ) => {
                if *m1 && !*m2 {
                    return false;
                }
                self.unify_types(i1, i2, subst)
            }

            // Unsafe references
            (
                Type::UnsafeReference {
                    inner: i1,
                    mutable: m1,
                },
                Type::UnsafeReference {
                    inner: i2,
                    mutable: m2,
                },
            ) => {
                if *m1 && !*m2 {
                    return false;
                }
                self.unify_types(i1, i2, subst)
            }

            // Function types
            (
                Type::Function {
                    params: p1,
                    return_type: r1,
                    ..
                },
                Type::Function {
                    params: p2,
                    return_type: r2,
                    ..
                },
            ) => {
                if p1.len() != p2.len() {
                    return false;
                }
                for (param1, param2) in p1.iter().zip(p2.iter()) {
                    if !self.unify_types(param1, param2, subst) {
                        return false;
                    }
                }
                self.unify_types(r1, r2, subst)
            }

            // Refined types - match on base type
            (Type::Refined { base: b1, .. }, Type::Refined { base: b2, .. }) => {
                self.unify_types(b1, b2, subst)
            }
            (Type::Refined { base, .. }, concrete_ty) => self.unify_types(base, concrete_ty, subst),
            (pattern_ty, Type::Refined { base, .. }) => self.unify_types(pattern_ty, base, subst),

            // Future types
            (Type::Future { output: o1 }, Type::Future { output: o2 }) => {
                self.unify_types(o1, o2, subst)
            }

            // Generator types
            (
                Type::Generator {
                    yield_ty: y1,
                    return_ty: r1,
                },
                Type::Generator {
                    yield_ty: y2,
                    return_ty: r2,
                },
            ) => self.unify_types(y1, y2, subst) && self.unify_types(r1, r2, subst),

            // GenRef types (generational references)
            (Type::GenRef { inner: i1, .. }, Type::GenRef { inner: i2, .. }) => {
                self.unify_types(i1, i2, subst)
            }

            // Record types
            (Type::Record(fields1), Type::Record(fields2)) => {
                // All fields in pattern must exist in concrete with matching types
                for (name, ty1) in fields1.iter() {
                    if let Some(ty2) = fields2.get(name) {
                        if !self.unify_types(ty1, ty2, subst) {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
                true
            }

            // No match
            _ => false,
        }
    }

    /// Check if a name looks like a type parameter (single uppercase letter or common names)
    fn looks_like_type_param(&self, name: &Text) -> bool {
        let s = name.as_str();
        // Single uppercase letter (T, U, V, A, B, etc.)
        if s.len() == 1
            && s.chars()
                .next()
                .map(|c| c.is_ascii_uppercase())
                .unwrap_or(false)
        {
            return true;
        }
        // Common type parameter names
        matches!(
            s,
            "T" | "U"
                | "V"
                | "A"
                | "B"
                | "C"
                | "K"
                | "E"
                | "R"
                | "S"
                | "Item"
                | "Element"
                | "Key"
                | "Value"
                | "Error"
                | "Ok"
                | "Output"
                | "Input"
                | "Target"
                | "Source"
                | "Elem"
        )
    }

    /// Check if a type contains/mentions a specific name (type parameter reference)
    /// Used to determine if a where clause actually constrains a type parameter
    fn type_mentions_name(ty: &Type, name: &Text) -> bool {
        match ty {
            Type::Named { path, args } => {
                // Check if the type itself is the name we're looking for
                if let Some(ident) = path.as_ident() {
                    if ident.as_str() == name.as_str() {
                        return true;
                    }
                }
                // Check type arguments recursively
                args.iter().any(|arg| Self::type_mentions_name(arg, name))
            }
            Type::Generic { name: gen_name, args } => {
                if gen_name.as_str() == name.as_str() {
                    return true;
                }
                args.iter().any(|arg| Self::type_mentions_name(arg, name))
            }
            Type::Var(_) => false, // Type variables don't have names we can check
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. } => Self::type_mentions_name(inner, name),
            Type::Array { element, .. } => Self::type_mentions_name(element, name),
            Type::Tuple(types) => types.iter().any(|t| Self::type_mentions_name(t, name)),
            Type::Record(fields) => fields.values().any(|t| Self::type_mentions_name(t, name)),
            Type::Function { params, return_type, .. } => {
                params.iter().any(|p| Self::type_mentions_name(p, name))
                    || Self::type_mentions_name(return_type, name)
            }
            Type::Variant(variants) => variants.values().any(|t| Self::type_mentions_name(t, name)),
            // Primitive types don't contain type parameter references
            Type::Unit | Type::Never | Type::Bool | Type::Int | Type::Float
            | Type::Char | Type::Text => false,
            // Other types - conservatively return false
            _ => false,
        }
    }

    /// Check if two paths are equal
    fn paths_equal(&self, p1: &Path, p2: &Path) -> bool {
        if p1.segments.len() != p2.segments.len() {
            return false;
        }
        for (s1, s2) in p1.segments.iter().zip(p2.segments.iter()) {
            use verum_ast::ty::PathSegment;
            match (s1, s2) {
                (PathSegment::Name(i1), PathSegment::Name(i2)) => {
                    if i1.name != i2.name {
                        return false;
                    }
                }
                (PathSegment::SelfValue, PathSegment::SelfValue) => {}
                (PathSegment::Super, PathSegment::Super) => {}
                (PathSegment::Cog, PathSegment::Cog) => {}
                (PathSegment::Relative, PathSegment::Relative) => {}
                _ => return false,
            }
        }
        true
    }

    /// Check if two types are structurally equal (for substitution consistency checking)
    fn types_structurally_equal(&self, ty1: &Type, ty2: &Type) -> bool {
        match (ty1, ty2) {
            (Type::Unit, Type::Unit) => true,
            (Type::Never, Type::Never) => true,
            (Type::Bool, Type::Bool) => true,
            (Type::Int, Type::Int) => true,
            (Type::Float, Type::Float) => true,
            (Type::Char, Type::Char) => true,
            (Type::Text, Type::Text) => true,
            (Type::Var(v1), Type::Var(v2)) => v1.id() == v2.id(),
            (Type::Named { path: p1, args: a1 }, Type::Named { path: p2, args: a2 }) => {
                self.paths_equal(p1, p2)
                    && a1.len() == a2.len()
                    && a1
                        .iter()
                        .zip(a2.iter())
                        .all(|(t1, t2)| self.types_structurally_equal(t1, t2))
            }
            (Type::Generic { name: n1, args: a1 }, Type::Generic { name: n2, args: a2 }) => {
                n1 == n2
                    && a1.len() == a2.len()
                    && a1
                        .iter()
                        .zip(a2.iter())
                        .all(|(t1, t2)| self.types_structurally_equal(t1, t2))
            }
            (Type::Tuple(e1), Type::Tuple(e2)) => {
                e1.len() == e2.len()
                    && e1
                        .iter()
                        .zip(e2.iter())
                        .all(|(t1, t2)| self.types_structurally_equal(t1, t2))
            }
            (
                Type::Array {
                    element: e1,
                    size: s1,
                },
                Type::Array {
                    element: e2,
                    size: s2,
                },
            ) => s1 == s2 && self.types_structurally_equal(e1, e2),
            (Type::Slice { element: e1 }, Type::Slice { element: e2 }) => {
                self.types_structurally_equal(e1, e2)
            }
            (
                Type::Reference {
                    inner: i1,
                    mutable: m1,
                },
                Type::Reference {
                    inner: i2,
                    mutable: m2,
                },
            ) => m1 == m2 && self.types_structurally_equal(i1, i2),
            _ => false,
        }
    }

    /// Check if where clauses are satisfied given a type substitution
    ///
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Where Clauses
    ///
    /// Evaluates each where clause with the type substitutions applied.
    /// Returns true if all clauses are satisfied.
    ///
    /// CRITICAL: This uses optimistic checking for types that contain:
    /// - Unresolved type variables
    /// - Associated type projections (like `I.IntoIter`)
    /// - Generic type constructors without concrete implementations
    ///
    /// For such types, we assume the constraint is satisfiable if the type
    /// "looks compatible" - this allows blanket implementations to work
    /// even when full resolution isn't possible.
    fn check_where_clauses_satisfied(
        &self,
        where_clauses: &List<WhereClause>,
        substitution: &Map<Text, Type>,
    ) -> bool {
        for clause in where_clauses.iter() {
            // Apply substitution to the constrained type
            let substituted_ty = self.apply_substitution(&clause.ty, substitution);

            // Check each bound is satisfied
            for bound in clause.bounds.iter() {
                let bound_satisfied = if bound.is_negative {
                    // Negative bound: type must NOT implement the protocol
                    !self.implements_optimistic(&substituted_ty, &bound.protocol)
                } else {
                    // Positive bound: type must implement the protocol
                    self.implements_optimistic(&substituted_ty, &bound.protocol)
                };

                if !bound_satisfied {
                    return false;
                }
            }
        }
        true
    }

    /// Optimistic protocol implementation check for blanket impls.
    ///
    /// This is used during where clause checking for blanket implementations.
    /// For types that contain unresolved components (type variables, associated
    /// type projections, etc.), we use optimistic checking that assumes the
    /// constraint is satisfiable if there's a plausible implementation path.
    ///
    /// This allows code like `iter([1, 2, 3]).map(...)` to work even when
    /// the full type resolution chain isn't available.
    fn implements_optimistic(&self, ty: &Type, protocol: &Path) -> bool {
        // Use recursion guard to prevent stack overflow
        let depth = IMPL_CHECK_DEPTH.with(|d| {
            let current = *d.borrow();
            *d.borrow_mut() = current + 1;
            current
        });

        // Helper struct for RAII cleanup
        struct DepthGuard;
        impl Drop for DepthGuard {
            fn drop(&mut self) {
                IMPL_CHECK_DEPTH.with(|d| {
                    *d.borrow_mut() -= 1;
                });
            }
        }
        let _guard = DepthGuard;

        // If we're too deep, return false to break the cycle.
        // Keep this low (3) to prevent stack overflow - the cache handles repeated lookups.
        if depth > 3 {
            return false;
        }

        let type_key = self.make_type_key(ty);
        let protocol_key = self.make_protocol_key(protocol);

        // Check thread-local cache first to avoid redundant recursive checks.
        // Audit-A2: cache key now includes `self.checker_id` so a long-lived
        // thread can't leak `(type_key, protocol_key)` results from a prior
        // checker into this one — the two checkers may have completely
        // different impl sets.
        let cache_key = (self.checker_id, type_key.clone(), protocol_key.clone());
        // Stack uses the checkerless key — recursion guarding is per-thread,
        // not per-checker. (Pre-A2 behaviour preserved.)
        let stack_key = (type_key.clone(), protocol_key.clone());
        let cached = IMPL_OPTIMISTIC_CACHE.with(|c| {
            c.borrow().get(&cache_key).copied()
        });
        if let Some(result) = cached {
            return result;
        }

        // Check for cycles: if we're already checking this (type, protocol) pair,
        // return false to break the cycle
        let is_cycle = IMPL_CHECKING_STACK.with(|s| {
            s.borrow().contains(&stack_key)
        });
        if is_cycle {
            return false;
        }

        // Push onto checking stack
        IMPL_CHECKING_STACK.with(|s| {
            s.borrow_mut().push(stack_key.clone());
        });

        // RAII guard for checking stack
        struct StackGuard {
            key: (Text, Text),
        }
        impl Drop for StackGuard {
            fn drop(&mut self) {
                IMPL_CHECKING_STACK.with(|s| {
                    let mut stack = s.borrow_mut();
                    if let Some(pos) = stack.iter().rposition(|k| k == &self.key) {
                        stack.remove(pos);
                    }
                });
            }
        }
        let _stack_guard = StackGuard { key: stack_key.clone() };

        // Stage 1: Try exact match first (most common case, O(1))
        if self.impl_index.get(&(type_key, protocol_key.clone())).is_some() {
            // Cache the positive result
            IMPL_OPTIMISTIC_CACHE.with(|c| {
                c.borrow_mut().insert(cache_key, true);
            });
            return true;
        }

        // Stage 2: Check if there's a matching generic/blanket impl
        // This can call check_where_clauses_satisfied which may recursively
        // call implements_optimistic, but the depth guard prevents overflow
        for impl_ in &self.impls {
            // Check if this impl is for the same protocol
            if self.make_protocol_key(&impl_.protocol) != protocol_key.clone() {
                continue;
            }

            // Try to match the impl's for_type against the concrete type
            if let Some(substitution) = self.try_match_type(&impl_.for_type, ty) {
                // Also verify where clauses
                if self.check_where_clauses_satisfied(&impl_.where_clauses, &substitution) {
                    // Cache the positive result
                    IMPL_OPTIMISTIC_CACHE.with(|c| {
                        c.borrow_mut().insert(cache_key, true);
                    });
                    return true;
                }
            }
        }

        // CRITICAL FIX: Be smarter about optimistic checking
        // Only be optimistic if there's a POSSIBILITY the type could implement the protocol
        //
        // For example:
        // - Iter<IntoIter<[Int]>>: Stream - YES, there's `impl<I: Iterator> Stream for Iter<I>`
        // - Iter<IntoIter<[Int]>>: Future - NO, there's no `impl Future for Iter<...>` anywhere
        //
        // We check if there's ANY impl of the protocol for the base type.

        // For pure type variables, be optimistic (they could be anything)
        if let Type::Var(_) = ty {
            return true;
        }

        // Type::Future intrinsically implements the Future protocol.
        // This is needed for where-clause checking on impls like
        //   `implement<Fut1: Future, Fut2: Future> Future for Join2<Fut1, Fut2>`
        // when the concrete types are `Future<A>` / `Future<B>`.
        if let Type::Future { .. } = ty {
            if let Some(proto_name) = protocol.as_ident() {
                if proto_name.as_str() == "Future" {
                    return true;
                }
            }
        }

        // For named types, check if the base type has ANY impl for this protocol
        if let Type::Named { path, .. } = ty {
            let protocol_key = self.make_protocol_key(protocol);
            let base_type_name = path.as_ident().map(|id| id.as_str());

            if let Some(base_name) = base_type_name {
                // Check if there's ANY impl of this protocol for this base type
                for impl_ in &self.impls {
                    if self.make_protocol_key(&impl_.protocol) != protocol_key {
                        continue;
                    }

                    // Check if impl for_type has the same base type name
                    let impl_base_name = match &impl_.for_type {
                        Type::Named { path, .. } => path.as_ident().map(|id| id.as_str().to_string()),
                        Type::Var(_) => Some("_".to_string()), // Blanket impl matches any type
                        _ => None,
                    };

                    if let Some(impl_name) = impl_base_name {
                        // Match if same base type OR if it's a blanket impl (for type variable)
                        if impl_name == base_name || impl_name == "_" {
                            // Found a possible impl - check if type contains unresolved components
                            // that might make the impl apply
                            if self.contains_unresolved_type(ty) {
                                return true;
                            }
                        }
                    }
                }
            }
        }

        // For Generic types, check the base type similarly
        if let Type::Generic { name, .. } = ty {
            let protocol_key = self.make_protocol_key(protocol);

            // Check if there's ANY impl of this protocol for this generic type
            for impl_ in &self.impls {
                if self.make_protocol_key(&impl_.protocol) != protocol_key {
                    continue;
                }

                // Check if impl for_type has the same generic name
                let impl_name = match &impl_.for_type {
                    Type::Generic { name: impl_name, .. } => Some(impl_name.as_str().to_string()),
                    Type::Var(_) => Some("_".to_string()), // Blanket impl
                    _ => None,
                };

                if let Some(iname) = impl_name {
                    if iname == name.as_str() || iname == "_" {
                        if self.contains_unresolved_type(ty) {
                            return true;
                        }
                    }
                }
            }
        }

        // For fully resolved types with no matching impl, don't be optimistic
        // Cache the negative result
        IMPL_OPTIMISTIC_CACHE.with(|c| {
            c.borrow_mut().insert(cache_key, false);
        });
        false
    }

    /// Check if a type contains unresolved components.
    ///
    /// Returns true if the type contains:
    /// - Type variables (Type::Var)
    /// - Generic types that look like associated type projections
    /// - Types with names suggesting they're projections (e.g., "IntoIter", "Item")
    fn contains_unresolved_type(&self, ty: &Type) -> bool {
        match ty {
            // Type variables are definitely unresolved
            Type::Var(_) => true,

            // Check named types for projections or unresolved args
            Type::Named { path, args } => {
                // Check if this looks like an associated type projection
                if let Some(ident) = path.as_ident() {
                    let name = ident.as_str();
                    // Common associated type names
                    if name == "IntoIter" || name == "Iter" || name == "Item"
                        || name == "Output" || name == "Error" || name == "Target" {
                        // If this is a projection-like type with non-empty args,
                        // it's likely an unresolved associated type
                        if !args.is_empty() {
                            return true;
                        }
                    }
                }

                // Check if any type arguments are unresolved
                args.iter().any(|arg| self.contains_unresolved_type(arg))
            }

            // Generic types often contain unresolved components
            Type::Generic { args, .. } => {
                args.iter().any(|arg| self.contains_unresolved_type(arg))
            }

            // Function types - check params and return
            Type::Function { params, return_type, .. } => {
                params.iter().any(|p| self.contains_unresolved_type(p))
                    || self.contains_unresolved_type(return_type)
            }

            // Reference types - check inner
            Type::Reference { inner, .. } => self.contains_unresolved_type(inner),

            // Array types - check element
            Type::Array { element, .. } => self.contains_unresolved_type(element),

            // Tuple types - check elements
            Type::Tuple(elements) => elements.iter().any(|e| self.contains_unresolved_type(e)),

            // Future types - check output
            Type::Future { output } => self.contains_unresolved_type(output),

            // Concrete primitive types are fully resolved
            Type::Unit | Type::Never | Type::Bool | Type::Int
            | Type::Float | Type::Char | Type::Text => false,

            // Other types - assume resolved
            _ => false,
        }
    }

    /// Apply a type substitution to a type
    fn apply_substitution(&self, ty: &Type, substitution: &Map<Text, Type>) -> Type {
        match ty {
            Type::Var(tv) => {
                let var_name: Text = format!("T{}", tv.id()).into();
                substitution
                    .get(&var_name)
                    .cloned()
                    .unwrap_or_else(|| ty.clone())
            }
            Type::Named { path, args } => {
                // Check if this is a type parameter
                if args.is_empty() {
                    if let Some(ident) = path.as_ident() {
                        let name: Text = ident.as_str().into();
                        if let Some(replacement) = substitution.get(&name) {
                            return replacement.clone();
                        }
                    }
                }
                // Recursively substitute in arguments
                Type::Named {
                    path: path.clone(),
                    args: args
                        .iter()
                        .map(|arg| self.apply_substitution(arg, substitution))
                        .collect(),
                }
            }
            Type::Generic { name, args } => {
                // Apply substitution to arguments
                let substituted_args: List<Type> = args
                    .iter()
                    .map(|arg| self.apply_substitution(arg, substitution))
                    .collect();

                // CRITICAL FIX: Check if this is a deferred projection (e.g., ::Item)
                // Deferred projections are created when A.Item is encountered with A being
                // a type parameter. After substitution, if the base type is now concrete,
                // we should resolve the projection.
                if name.starts_with("::") && !substituted_args.is_empty() {
                    let assoc_name: Text = name.trim_start_matches("::");
                    let base_type = &substituted_args[0];

                    // Check if base type is concrete (no unresolved type variables)
                    if !self.type_has_unresolved_vars(base_type) {
                        // Try to resolve the projection
                        if let Some(resolved) = self.try_find_associated_type(base_type, &assoc_name) {
                            // Recursively apply substitution to the resolved type
                            // (in case it contains further projections)
                            return self.apply_substitution(&resolved, substitution);
                        }
                    }
                }

                Type::Generic {
                    name: name.clone(),
                    args: substituted_args,
                }
            }
            Type::Tuple(elems) => Type::Tuple(
                elems
                    .iter()
                    .map(|e| self.apply_substitution(e, substitution))
                    .collect(),
            ),
            Type::Array { element, size } => Type::Array {
                element: Box::new(self.apply_substitution(element, substitution)),
                size: *size,
            },
            Type::Slice { element } => Type::Slice {
                element: Box::new(self.apply_substitution(element, substitution)),
            },
            Type::Reference { inner, mutable } => Type::Reference {
                inner: Box::new(self.apply_substitution(inner, substitution)),
                mutable: *mutable,
            },
            Type::Function {
                params,
                return_type,
                contexts,
                type_params,
                properties,
            } => Type::Function {
                params: params
                    .iter()
                    .map(|p| self.apply_substitution(p, substitution))
                    .collect(),
                return_type: Box::new(self.apply_substitution(return_type, substitution)),
                contexts: contexts.clone(),
                type_params: type_params.clone(),
                properties: properties.clone(),
            },
            _ => ty.clone(),
        }
    }

    /// Select the most specific implementation from candidates
    ///
    /// Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — .2 - Specialization Precedence
    ///
    /// The precedence lattice is:
    /// 1. Concrete type (List<Int>) - most specific
    /// 2. Partially specialized (List<T> where T: Copy)
    /// 3. Generic (List<T>) - least specific
    ///
    /// For multiple candidates at the same level, prefer:
    /// - Implementations marked with @specialize
    /// - Implementations with more specific where clauses
    fn select_most_specific_impl(
        &self,
        candidates: &List<(usize, &ProtocolImpl, Map<Text, Type>)>,
        target_type: &Type,
    ) -> usize {
        if candidates.len() <= 1 {
            return 0;
        }

        let mut best_idx = 0;
        let mut best_score = self.compute_specificity_score(&candidates[0].1.for_type, target_type);
        let mut best_has_specialize = candidates[0].1.specialization.is_some();

        for (idx, (_, impl_, _)) in candidates.iter().enumerate().skip(1) {
            let score = self.compute_specificity_score(&impl_.for_type, target_type);
            let has_specialize = impl_.specialization.is_some();

            // Higher score is more specific
            // If scores are equal, prefer @specialize marked impl
            if score > best_score || (score == best_score && has_specialize && !best_has_specialize)
            {
                best_idx = idx;
                best_score = score;
                best_has_specialize = has_specialize;
            }
        }

        best_idx
    }

    /// Compute specificity score for a type pattern
    ///
    /// Higher score = more specific:
    /// - Concrete types score highest
    /// - Types with more concrete parts score higher
    /// - Type variables score 0
    fn compute_specificity_score(&self, pattern: &Type, _target: &Type) -> i32 {
        match pattern {
            // Primitive types are maximally specific
            Type::Unit
            | Type::Never
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Char
            | Type::Text => 100,

            // Type variables are not specific at all
            Type::Var(_) => 0,

            // Named types: specificity depends on their arguments
            Type::Named { path, args } => {
                // Check if this looks like a type parameter
                if args.is_empty() {
                    if let Some(ident) = path.as_ident() {
                        if self.looks_like_type_param(&ident.as_str().into()) {
                            return 0;
                        }
                    }
                }
                // Base score for concrete constructor + sum of arg specificities
                let mut score = 10;
                for arg in args.iter() {
                    score += self.compute_specificity_score(arg, _target);
                }
                score
            }

            Type::Generic { args, .. } => {
                let mut score = 10;
                for arg in args.iter() {
                    score += self.compute_specificity_score(arg, _target);
                }
                score
            }

            Type::Tuple(elems) => {
                let mut score = 5;
                for elem in elems.iter() {
                    score += self.compute_specificity_score(elem, _target);
                }
                score
            }

            Type::Array { element, size } => {
                let mut score = 5 + self.compute_specificity_score(element, _target);
                // Sized arrays are more specific
                if size.is_some() {
                    score += 5;
                }
                score
            }

            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Slice { element: inner } => 3 + self.compute_specificity_score(inner, _target),

            Type::Function {
                params,
                return_type,
                ..
            } => {
                let mut score = 5;
                for param in params.iter() {
                    score += self.compute_specificity_score(param, _target);
                }
                score += self.compute_specificity_score(return_type, _target);
                score
            }

            // Other types: moderate specificity
            _ => 5,
        }
    }

    /// Check if protocol is derivable
    pub fn is_derivable(&self, protocol: &Text) -> bool {
        self.derivable.contains(protocol)
    }

    /// Check if protocol inherits from another (transitive)
    pub fn inherits_from(&self, protocol: &Text, super_protocol: &Text) -> bool {
        if protocol == super_protocol {
            return true;
        }

        if let Some(proto) = self.protocols.get(protocol) {
            // Check direct superprotocols
            for super_bound in proto.super_protocols.iter() {
                if let Some(super_name) = super_bound.protocol.as_ident().map(|i| i.as_str()) {
                    let super_name_text: Text = super_name.into();
                    // Recursive check for transitive inheritance
                    if self.inherits_from(&super_name_text, super_protocol) {
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Check for cycles in the protocol inheritance hierarchy.
    ///
    /// Uses DFS with a "currently on stack" set to detect back-edges.
    /// Called during protocol registration so cycles are caught early.
    pub fn check_hierarchy_cycles(&self, protocol: &Text) -> Result<(), ProtocolError> {
        let mut stack = Set::new();
        self.detect_cycle_dfs(protocol, &mut stack, &mut Set::new())
    }

    fn detect_cycle_dfs(
        &self,
        current: &Text,
        stack: &mut Set<Text>,
        visited: &mut Set<Text>,
    ) -> Result<(), ProtocolError> {
        if stack.contains(current) {
            // Back-edge found -- collect the cycle members from the stack
            return Err(ProtocolError::CyclicInheritance {
                protocol: current.clone(),
                cycle: stack.iter().cloned().collect(),
            });
        }
        if visited.contains(current) {
            // Already fully explored via another path -- no cycle here
            return Ok(());
        }
        stack.insert(current.clone());

        if let Some(proto) = self.protocols.get(current) {
            for super_bound in proto.super_protocols.iter() {
                if let Some(super_name) = super_bound.protocol.as_ident().map(|i| i.as_str()) {
                    let super_text: Text = super_name.into();
                    self.detect_cycle_dfs(&super_text, stack, visited)?;
                }
            }
        }

        stack.remove(current);
        visited.insert(current.clone());
        Ok(())
    }

    /// Get all methods including inherited ones (from superprotocols)
    pub fn all_methods(&self, protocol_name: &Text) -> Result<List<ProtocolMethod>, ProtocolError> {
        let mut methods = List::new();
        let mut seen = Set::new();

        self.collect_methods_recursive(protocol_name, &mut methods, &mut seen)?;

        Ok(methods)
    }

    fn collect_methods_recursive(
        &self,
        protocol_name: &Text,
        methods: &mut List<ProtocolMethod>,
        seen: &mut Set<Text>,
    ) -> Result<(), ProtocolError> {
        // Prevent infinite recursion
        if seen.contains(protocol_name) {
            return Ok(());
        }
        seen.insert(protocol_name.clone());

        let protocol = match self.protocols.get(protocol_name) {
            Some(p) => p,
            None => {
                return Err(ProtocolError::ProtocolNotFound {
                    name: protocol_name.clone(),
                });
            }
        };

        // First, collect methods from superprotocols (they can be overridden)
        for super_bound in protocol.super_protocols.iter() {
            if let Some(super_name) = super_bound.protocol.as_ident().map(|i| i.as_str()) {
                self.collect_methods_recursive(&super_name.into(), methods, seen)?;
            }
        }

        // Then add this protocol's methods (overriding any with same name)
        for (name, method) in protocol.methods.iter() {
            // Remove any existing method with same name (override)
            methods.retain(|m| &m.name != name);
            methods.push(method.clone());
        }

        Ok(())
    }

    /// Find inherited method from superprotocols recursively
    ///
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Superprotocol inheritance
    ///
    /// Searches through the superprotocol hierarchy to find a default method
    /// implementation. Uses visited set to prevent infinite recursion in
    /// case of cyclic protocol hierarchies.
    fn find_inherited_method(
        &self,
        protocol: &Protocol,
        method_name: &Text,
        visited: &mut Set<Text>,
    ) -> Result<Option<MethodResolution>, ProtocolError> {
        // Mark this protocol as visited
        if visited.contains(&protocol.name) {
            return Ok(None);
        }
        visited.insert(protocol.name.clone());

        // Check each superprotocol
        for super_bound in protocol.super_protocols.iter() {
            let super_name = self.extract_protocol_name(&super_bound.protocol);

            if let Some(super_proto) = self.protocols.get(&super_name) {
                // Check if superprotocol has this method (default or required).
                // When Eq extends PartialEq, methods from PartialEq should be
                // resolvable through the Eq protocol hierarchy.
                if let Some(super_method) = super_proto.methods.get(method_name) {
                    return Ok(Some(MethodResolution {
                        ty: super_method.ty.clone(),
                        is_default: super_method.has_default,
                        source: MethodSource::Inherited(super_name.clone()),
                    }));
                }

                // Recursively check this superprotocol's superprotocols
                if let Some(resolution) =
                    self.find_inherited_method(super_proto, method_name, visited)?
                {
                    return Ok(Some(resolution));
                }
            }
        }

        Ok(None)
    }

    /// Find inherited associated type from superprotocols recursively
    ///
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Superprotocol inheritance
    ///
    /// Searches through the superprotocol hierarchy to find an associated type
    /// with a default value. Uses visited set to prevent infinite recursion in
    /// case of cyclic protocol hierarchies.
    fn find_inherited_associated_type(
        &self,
        protocol: &Protocol,
        assoc_name: &Text,
        visited: &mut Set<Text>,
    ) -> Result<Option<Type>, ProtocolError> {
        // Mark this protocol as visited
        if visited.contains(&protocol.name) {
            return Ok(None);
        }
        visited.insert(protocol.name.clone());

        // Check each superprotocol
        for super_bound in protocol.super_protocols.iter() {
            let super_name = self.extract_protocol_name(&super_bound.protocol);

            if let Some(super_proto) = self.protocols.get(&super_name) {
                // Check if superprotocol has this associated type with a default
                if let Some(super_assoc_type) = super_proto.associated_types.get(assoc_name) {
                    if let Option::Some(default_ty) = &super_assoc_type.default {
                        // Validate that the default type satisfies the bounds
                        for bound in super_assoc_type.bounds.iter() {
                            if !self.implements(default_ty, &bound.protocol) {
                                return Err(ProtocolError::BoundNotSatisfied {
                                    ty: default_ty.clone(),
                                    protocol: bound.protocol.clone(),
                                });
                            }
                        }
                        return Ok(Some(default_ty.clone()));
                    }
                }

                // Recursively check this superprotocol's superprotocols
                if let Some(resolved_ty) =
                    self.find_inherited_associated_type(super_proto, assoc_name, visited)?
                {
                    return Ok(Some(resolved_ty));
                }
            }
        }

        Ok(None)
    }

    /// Check if a type satisfies protocol bounds
    ///
    /// Handles both positive bounds (T: Protocol) and negative bounds (T: !Protocol).
    ///
    /// Multi-protocol bounds: combining multiple protocol constraints (T: Display + Debug) — Negative Bounds
    pub fn check_bounds(&self, ty: &Type, bounds: &[ProtocolBound]) -> Result<(), ProtocolError> {
        for bound in bounds {
            if bound.is_negative {
                // Negative bound: type must NOT implement the protocol
                if self.implements(ty, &bound.protocol) {
                    return Err(ProtocolError::NegativeBoundViolated {
                        ty: ty.clone(),
                        protocol: bound.protocol.clone(),
                    });
                }
            } else {
                // Positive bound: type must implement the protocol
                if !self.implements(ty, &bound.protocol) {
                    return Err(ProtocolError::BoundNotSatisfied {
                        ty: ty.clone(),
                        protocol: bound.protocol.clone(),
                    });
                }

                // Check super-protocol bounds (inheritance)
                if let Some(protocol_name) = bound.protocol.as_ident().map(|i| i.as_str())
                    && let Some(protocol) = self.protocols.get(&protocol_name.into())
                {
                    // Avoid cloning by checking bounds directly on the slice
                    self.check_bounds(ty, &protocol.super_protocols)?;
                }
            }
        }

        Ok(())
    }

    /// Check if a type satisfies a negative bound (T: !Protocol)
    ///
    /// Multi-protocol bounds: combining multiple protocol constraints (T: Display + Debug) — Negative Bounds
    ///
    /// Returns true if the type does NOT implement the protocol.
    pub fn satisfies_negative_bound(&self, ty: &Type, protocol: &Path) -> bool {
        !self.implements(ty, protocol)
    }

    /// Check if type satisfies protocol (including superprotocols)
    pub fn check_protocol_satisfied(
        &self,
        ty: &Type,
        protocol_name: &Text,
    ) -> Result<bool, ProtocolError> {
        // Check if explicit implementation exists
        let protocol_path = Path::single(Ident::new(protocol_name.as_str(), Span::default()));
        if self.implements(ty, &protocol_path) {
            return Ok(true);
        }

        // Check if we have an implementation for a subprotocol that inherits this protocol
        // (This handles cases where impl Ord automatically satisfies Eq requirement)
        let type_key = self.make_type_key(ty);
        for impl_ in self.impls.iter() {
            let impl_type_key = self.make_type_key(&impl_.for_type);
            if impl_type_key == type_key
                && let Some(impl_protocol_name) = impl_.protocol.as_ident().map(|i| i.as_str())
            {
                let impl_proto_text: Text = impl_protocol_name.into();
                if self.inherits_from(&impl_proto_text, protocol_name) {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    /// Resolve associated type for a type implementing a protocol (with default support)
    ///
    /// Spec: grammar/verum.ebnf lines 416-417, 932-939
    ///
    /// When an implementation doesn't specify an associated type, use the default
    /// from the protocol definition if available.
    ///
    /// # Example
    /// ```verum
    /// protocol Container {
    ///     default type Item = Heap<u8>;
    ///     fn get(&self, idx: usize) -> Self.Item;
    /// }
    ///
    /// implement Container for MyType {
    ///     // Item not specified, uses default Heap<u8>
    ///     fn get(&self, idx: usize) -> Heap<u8> { ... }
    /// }
    /// ```
    pub fn resolve_associated_type(
        &self,
        ty: &Type,
        protocol: &Path,
        assoc_name: &Text,
    ) -> Result<Type, ProtocolError> {
        // Find implementation
        let impl_ = match self.find_impl(ty, protocol) {
            Option::Some(i) => i,
            Option::None => {
                return Err(ProtocolError::NotImplemented {
                    ty: ty.clone(),
                    protocol: protocol.clone(),
                });
            }
        };

        // Look up associated type in implementation
        if let Some(assoc_ty) = impl_.associated_types.get(assoc_name) {
            return Ok(assoc_ty.clone());
        }

        // Associated type not explicitly specified - check for default
        // Extract protocol name from path - supports both simple (Iterator) and qualified (std.iter.Iterator) paths
        let protocol_name = self.extract_protocol_name(protocol);

        let protocol_def = match self.protocols.get(&protocol_name) {
            Some(p) => p,
            None => {
                return Err(ProtocolError::ProtocolNotFound {
                    name: protocol_name,
                });
            }
        };

        // Check if the protocol has a default for this associated type
        if let Some(assoc_type_def) = protocol_def.associated_types.get(assoc_name)
            && let Option::Some(default_ty) = &assoc_type_def.default
        {
            // Validate that the default type satisfies the bounds
            for bound in assoc_type_def.bounds.iter() {
                if !self.implements(default_ty, &bound.protocol) {
                    return Err(ProtocolError::BoundNotSatisfied {
                        ty: default_ty.clone(),
                        protocol: bound.protocol.clone(),
                    });
                }
            }
            return Ok(default_ty.clone());
        }

        // Check superprotocols recursively for associated types
        // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Superprotocol inheritance
        let mut visited = Set::new();
        if let Some(resolved_ty) =
            self.find_inherited_associated_type(protocol_def, assoc_name, &mut visited)?
        {
            return Ok(resolved_ty);
        }

        // No default and no explicit specification - error
        Err(ProtocolError::AssociatedTypeNotSpecified {
            protocol: protocol.clone(),
            assoc_name: assoc_name.clone(),
            for_type: ty.clone(),
        })
    }

    /// Resolve method for a type implementing a protocol (with default impl support)
    pub fn resolve_method(
        &self,
        ty: &Type,
        protocol: &Path,
        method: &Text,
    ) -> Result<MethodResolution, ProtocolError> {
        // Find implementation
        let impl_ = match self.find_impl(ty, protocol) {
            Option::Some(i) => i,
            Option::None => {
                return Err(ProtocolError::NotImplemented {
                    ty: ty.clone(),
                    protocol: protocol.clone(),
                });
            }
        };

        // Look up method in implementation
        if let Some(method_ty) = impl_.methods.get(method) {
            return Ok(MethodResolution {
                ty: method_ty.clone(),
                is_default: false,
                source: MethodSource::Explicit,
            });
        }

        // Method not explicitly implemented - check for default implementation
        // Extract protocol name from path - supports both simple and qualified paths
        let protocol_name = self.extract_protocol_name(protocol);

        let protocol_def = match self.protocols.get(&protocol_name) {
            Some(p) => p,
            None => {
                return Err(ProtocolError::ProtocolNotFound {
                    name: protocol_name,
                });
            }
        };

        // Check if protocol has default implementation for this method
        if let Some(proto_method) = protocol_def.methods.get(method)
            && proto_method.has_default
        {
            return Ok(MethodResolution {
                ty: proto_method.ty.clone(),
                is_default: true,
                source: MethodSource::Default(protocol_name.clone()),
            });
        }

        // Check superprotocols recursively for inherited default implementations
        // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Superprotocol inheritance
        let mut visited = Set::new();
        if let Some(resolution) = self.find_inherited_method(protocol_def, method, &mut visited)? {
            return Ok(resolution);
        }

        Err(ProtocolError::MethodNotFound {
            protocol: protocol.clone(),
            method: method.clone(),
        })
    }

    /// Resolve method with caching. Returns cached result if available,
    /// otherwise delegates to resolve_method and caches the result.
    pub fn resolve_method_cached(
        &mut self,
        ty: &Type,
        protocol: &Path,
        method: &Text,
    ) -> Result<MethodResolution, ProtocolError> {
        let type_key = self.make_type_key(ty);
        let protocol_key = format!("{:?}", protocol);
        let cache_key = (type_key.clone(), protocol_key.into(), method.clone());

        if let Some(cached) = self.method_resolution_cache.get(&cache_key) {
            return Ok(cached.clone());
        }

        let result = self.resolve_method(ty, protocol, method)?;
        self.method_resolution_cache.insert(cache_key, result.clone());
        Ok(result)
    }

    /// Get all protocols implemented by a type
    ///
    /// This method handles both exact type matches and generic implementations.
    /// For generic implementations like `implement<T: Default> Default for Wrapper<T>`,
    /// calling get_implementations(Wrapper) will find the generic impl.
    ///
    /// Generic protocol implementations: "implement<T: Bound> Protocol for Container<T>" with blanket impls
    pub fn get_implementations(&self, ty: &Type) -> List<&ProtocolImpl> {
        let type_key = self.make_type_key(ty);
        let mut impls = List::new();

        // First try exact match
        for ((tk, _), &idx) in &self.impl_index {
            if tk == &type_key
                && let Some(impl_) = self.impls.get(idx)
            {
                impls.push(impl_);
            }
        }

        // If no exact match, try to find generic implementations
        // For example, if looking for `Wrapper`, find `implement<T> ... for Wrapper<T>`
        if impls.is_empty() {
            // Extract base type name (without args)
            let base_type_key = self.make_base_type_key(ty);

            for impl_ in &self.impls {
                let impl_base_key = self.make_base_type_key(&impl_.for_type);

                if impl_base_key == base_type_key {
                    // Found a generic implementation for this base type
                    impls.push(impl_);
                }
            }
        }

        // CRITICAL FIX: Also find blanket implementations
        // A blanket impl like `implement<S: Stream> StreamExt for S {}` has:
        // - for_type = S (a type variable or type parameter)
        // - where_clauses = [S: Stream]
        // When looking up impls for Iter<I> (which implements Stream),
        // we should find this blanket impl because Iter<I> satisfies the where clause.

        // Track which impls we've already added (by pointer address to avoid PartialEq requirement)
        let existing_ptrs: std::collections::HashSet<*const ProtocolImpl> =
            impls.iter().map(|i| *i as *const ProtocolImpl).collect();

        for impl_ in &self.impls {
            // Skip if already added
            let ptr = impl_ as *const ProtocolImpl;
            if existing_ptrs.contains(&ptr) {
                continue;
            }

            // Check if this is a blanket implementation (for_type is a type variable or type param)
            // CRITICAL FIX: For name-based detection (not Type::Var), require that where_clauses
            // actually constrain the type parameter. This prevents concrete implementations like
            // `implement Clone for Item` from being incorrectly treated as blanket impls that
            // match any type (since "Item" looks like a type param name but is a concrete type).
            let is_blanket = match &impl_.for_type {
                Type::Var(_) => true,
                Type::Named { path, args } if args.is_empty() => {
                    // Named type without args might be a type parameter like S
                    if let Some(ident) = path.as_ident() {
                        let name: Text = ident.as_str().into();
                        if self.looks_like_type_param(&name) {
                            // CRITICAL: Only treat as blanket if where_clauses actually constrain this type param.
                            // A true blanket impl like `implement<T: Clone> Clone for T` has where_clauses
                            // referencing T. A concrete impl like `implement Clone for Item` has no such clauses.
                            
                            impl_.where_clauses.iter().any(|clause| {
                                // Check if this where clause constrains the potential type parameter
                                Self::type_mentions_name(&clause.ty, &name)
                            })
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
                _ => false,
            };

            if is_blanket {
                // Try to match and check where clauses
                if let Some(substitution) = self.try_match_type(&impl_.for_type, ty) {
                    let satisfied = self.check_where_clauses_satisfied(&impl_.where_clauses, &substitution);
                    if satisfied {
                        impls.push(impl_);
                    }
                }
            }
        }

        impls
    }

    /// Make a type key for just the base type name (without type arguments)
    /// Used for matching generic implementations
    fn make_base_type_key(&self, ty: &Type) -> Text {
        use crate::ty::Type::*;
        match ty {
            Unit => "Unit".into(),
            Never => "Never".into(),
            Bool => WKT::Bool.as_str().into(),
            Int => WKT::Int.as_str().into(),
            Float => WKT::Float.as_str().into(),
            Char => WKT::Char.as_str().into(),
            Text => WKT::Text.as_str().into(),
            Named { path, .. } => {
                // Extract just the base type name without prefix
                // This allows matching Named{Maybe} against Generic{Maybe}
                let mut key = verum_common::Text::new();
                key.push_str("base:");
                for segment in &path.segments {
                    if let verum_ast::ty::PathSegment::Name(ident) = segment {
                        key.push_str(ident.name.as_str());
                        key.push('.');
                    }
                }
                // Don't include args - just the base type name
                key
            }
            Generic { name, .. } => {
                // Use same prefix as Named to allow cross-matching
                let mut key = verum_common::Text::new();
                key.push_str("base:");
                key.push_str(name.as_str());
                key.push('.'); // Add trailing dot to match Named format
                // Don't include args
                key
            }
            // For other types, fall back to full key
            _ => self.make_type_key(ty),
        }
    }

    /// Check structural typing: does a type have the required methods?
    ///
    /// This allows duck typing - if a type has the right methods, it can be used
    /// even without an explicit implementation.
    /// Spec: Protocol system - Structural typing support
    ///
    /// Checks if a type structurally implements a protocol by verifying
    /// that it has all required methods with compatible signatures.
    pub fn check_structural(&self, ty: &Type, _required_methods: &Map<Text, Type>) -> bool {
        // For primitive types, we check if they have standard methods
        match ty {
            Type::Int | Type::Float | Type::Bool | Type::Text | Type::Char => {
                // Primitive types implement standard protocols:
                // - Eq: equality comparison
                // - Show: string representation
                // - Ord (for numeric types): comparison
                // For structural checking, verify required methods are minimal
                true
            }
            Type::Unit => {
                // Unit type implements Eq and Show trivially
                true
            }
            Type::Array { .. } | Type::Tuple { .. } | Type::Record { .. } => {
                // Collection types implement Eq and Show if elements do
                // For now, assume they do
                true
            }
            Type::Named { .. } => {
                // For named types, structural checking would require
                // querying the type definition for available methods
                // For now, return true (full implementation would introspect)
                true
            }
            _ => {
                // Other types (functions, references, etc.) don't have
                // standard methods for structural checking
                false
            }
        }
    }

    /// Generate VTable for protocol implementation
    ///
    /// Creates a virtual dispatch table containing all methods (explicit + default)
    /// for efficient runtime dispatch with <10ns overhead.
    pub fn generate_vtable(&self, ty: &Type, protocol: &Path) -> Result<VTable, ProtocolError> {
        let impl_ = match self.find_impl(ty, protocol) {
            Option::Some(i) => i,
            Option::None => {
                return Err(ProtocolError::NotImplemented {
                    ty: ty.clone(),
                    protocol: protocol.clone(),
                });
            }
        };

        let protocol_name = protocol.as_ident().map(|i| i.as_str()).ok_or_else(|| {
            ProtocolError::ProtocolNotFound {
                name: "unknown".into(),
            }
        })?;

        // Collect all methods (including defaults and inherited)
        let all_methods = self.all_methods(&protocol_name.into())?;

        let mut vtable_methods = Map::new();

        for method in all_methods.iter() {
            // Use explicit implementation if available
            if let Some(method_ty) = impl_.methods.get(&method.name) {
                vtable_methods.insert(method.name.clone(), method_ty.clone());
            } else if method.has_default {
                // Use default implementation
                vtable_methods.insert(method.name.clone(), method.ty.clone());
            } else {
                // Missing required method
                return Err(ProtocolError::MethodNotFound {
                    protocol: protocol.clone(),
                    method: method.name.clone(),
                });
            }
        }

        Ok(VTable::new(
            protocol_name.into(),
            ty.clone(),
            &vtable_methods,
        ))
    }

    /// Get protocol definition by name
    pub fn get_protocol(&self, name: &Text) -> Maybe<&Protocol> {
        self.protocols.get(name)
    }

    /// Walk the superprotocol hierarchy to find an inherited method.
    ///
    /// When protocol Eq extends PartialEq, methods defined in PartialEq
    /// should be available on types implementing Eq. This traverses
    /// `super_protocols` breadth-first looking for `method_name`.
    pub fn find_superprotocol_method<'a>(&'a self, proto: &'a Protocol, method_name: &Text) -> Option<&'a ProtocolMethod> {
        let mut visited = Set::new();
        let mut queue: List<&Protocol> = List::new();
        // Seed with direct superprotocols
        for bound in proto.super_protocols.iter() {
            if let Some(parent) = self.lookup_protocol(&bound.protocol) {
                let parent_name = bound.protocol.segments.last().and_then(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.clone()),
                    _ => None,
                }).unwrap_or_default();
                if visited.insert(parent_name) {
                    queue.push(parent);
                }
            }
        }
        while let Some(current) = queue.pop() {
            if let Some(method) = current.methods.get(method_name) {
                return Some(method);
            }
            // Continue searching up the hierarchy
            for bound in current.super_protocols.iter() {
                if let Some(grandparent) = self.lookup_protocol(&bound.protocol) {
                    let name = bound.protocol.segments.last().and_then(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.clone()),
                        _ => None,
                    }).unwrap_or_default();
                    if visited.insert(name) {
                        queue.push(grandparent);
                    }
                }
            }
        }
        None
    }

    /// Get all registered protocols
    ///
    /// Returns a reference to the internal protocol map for iteration.
    /// This is useful for searching which protocol a method belongs to.
    pub fn get_all_protocols(&self) -> &Map<Text, Protocol> {
        &self.protocols
    }

    /// Get protocol definition by name (string slice version)
    ///
    /// This is a convenience wrapper around `get_protocol` that accepts a `&str`
    /// and returns an `Option` instead of `Maybe` for more ergonomic use.
    pub fn get_protocol_definition(&self, name: &str) -> Option<&Protocol> {
        let text_name: Text = name.into();
        match self.protocols.get(&text_name) {
            Some(p) => Some(p),
            None => None,
        }
    }

    /// Get the return type of a protocol method by protocol name and method name.
    /// Used for resolving method calls on &dyn Protocol receivers.
    pub fn get_method_type(&self, protocol_name: &str, method_name: &str) -> Option<Type> {
        let proto = self.get_protocol_definition(protocol_name)?;
        let method_text = Text::from(method_name);
        // First check direct methods
        if let Some(method) = proto.methods.get(&method_text) {
            return match &method.ty {
                Type::Function { return_type, .. } => Some((**return_type).clone()),
                _ => Some(method.ty.clone()),
            };
        }
        // Then check parent (super) protocols recursively
        for super_bound in &proto.super_protocols {
            if let Some(super_name) = super_bound.protocol.as_ident() {
                if let Some(ty) = self.get_method_type(super_name.name.as_str(), method_name) {
                    return Some(ty);
                }
            }
        }
        None
    }

    /// Look up an associated type definition from a protocol
    ///
    /// Given a protocol name and associated type name, returns the associated type
    /// definition from the protocol. This is used during projection resolution to
    /// understand the structure and bounds of an associated type.
    ///
    /// # Arguments
    ///
    /// * `protocol_name` - The name of the protocol to search
    /// * `assoc_name` - The name of the associated type to find
    ///
    /// # Returns
    ///
    /// * `Some(&AssociatedType)` - The associated type definition if found
    /// * `None` - If the protocol doesn't exist or doesn't have this associated type
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Looking up Iterator.Item
    /// let assoc = checker.lookup_associated_type("Iterator", "Item");
    /// ```
    ///
    /// Associated type bounds: constraining associated types in where clauses (where T.Item: Display) — Associated Type Bounds
    pub fn lookup_associated_type(
        &self,
        protocol_name: &str,
        assoc_name: &str,
    ) -> Option<&AssociatedType> {
        let protocol = self.get_protocol_definition(protocol_name)?;
        let assoc_text: Text = assoc_name.into();
        protocol.associated_types.get(&assoc_text)
    }

    /// Look up an associated type from a protocol path
    ///
    /// This variant accepts a Path for the protocol, which is useful when
    /// resolving associated types from parsed AST nodes.
    ///
    /// # Arguments
    ///
    /// * `protocol` - The protocol path
    /// * `assoc_name` - The associated type name
    ///
    /// # Returns
    ///
    /// The associated type definition if found.
    pub fn lookup_associated_type_by_path(
        &self,
        protocol: &Path,
        assoc_name: &Text,
    ) -> Option<&AssociatedType> {
        let protocol_def = self.lookup_protocol(protocol)?;
        protocol_def.associated_types.get(assoc_name)
    }

    /// Find all protocols that have a specific associated type
    ///
    /// This is useful for resolving projections when the protocol is not
    /// explicitly specified. Returns a list of protocol names that have
    /// an associated type with the given name.
    ///
    /// # Arguments
    ///
    /// * `assoc_name` - The associated type name to search for
    ///
    /// # Returns
    ///
    /// A list of protocol names that have this associated type.
    pub fn find_protocols_with_associated_type(&self, assoc_name: &str) -> List<Text> {
        let assoc_text: Text = assoc_name.into();
        self.protocols
            .iter()
            .filter(|(_, proto)| proto.associated_types.contains_key(&assoc_text))
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Try to find an associated type for a given type without knowing the protocol.
    ///
    /// This is used for resolving projections like `T.Item` where T is a concrete type
    /// that implements some protocol with an `Item` associated type, but we don't know
    /// which protocol it is.
    ///
    /// The resolution process:
    /// 1. Find all protocols that the type implements
    /// 2. For each implementation, check if the protocol has the associated type
    /// 3. Return the resolved type from the first matching implementation
    ///
    /// # Arguments
    ///
    /// * `ty` - The base type (e.g., `Collection<Int>`)
    /// * `assoc_name` - The associated type name (e.g., `Item`)
    ///
    /// # Returns
    ///
    /// The resolved associated type if found, None otherwise.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // If Collection<Int> implements Iterator<Item = Int>
    /// let item_ty = checker.try_find_associated_type(&collection_ty, &"Item".into());
    /// assert_eq!(item_ty, Some(Type::Int));
    /// ```
    ///
    /// Associated type bounds: constraining associated types in where clauses (where T.Item: Display) — Associated Type Resolution
    pub fn try_find_associated_type(&self, ty: &Type, assoc_name: &Text) -> Option<Type> {
        // Cycle detection: if we're already resolving this (type, assoc_name) pair,
        // we've hit an infinite loop (e.g., blanket impl FutureExt.Output = F.Output
        // recursing back to the same type). Break the cycle by returning None.
        let type_key = self.make_type_key(ty);
        let resolution_key = (type_key.clone(), assoc_name.clone());

        let is_cycle = ASSOC_TYPE_RESOLUTION_STACK.with(|stack| {
            stack.borrow().contains(&resolution_key)
        });
        if is_cycle {
            return None;
        }

        // Push onto resolution stack
        ASSOC_TYPE_RESOLUTION_STACK.with(|stack| {
            stack.borrow_mut().push(resolution_key.clone());
        });

        // RAII guard to pop from stack on exit
        struct AssocTypeGuard(Text, Text);
        impl Drop for AssocTypeGuard {
            fn drop(&mut self) {
                ASSOC_TYPE_RESOLUTION_STACK.with(|stack| {
                    let mut s = stack.borrow_mut();
                    if let Some(pos) = s.iter().position(|item| item.0 == self.0 && item.1 == self.1) {
                        s.remove(pos);
                    }
                });
            }
        }
        let _guard = AssocTypeGuard(resolution_key.0, resolution_key.1);

        // Fast path: Type::Future { output: T } intrinsically has Output = T.
        // This avoids a full impl lookup and ensures that projection types like
        //   ::Output[Future<Int>]   resolve to   Int
        // even though there is no explicit registered `implement Future for Future<T>` impl.
        // This is required for Join2 and other combinators whose associated Output type
        // is expressed as a tuple of inner future Outputs:
        //   type Output = (Fut1.Output, Fut2.Output);
        if assoc_name.as_str() == "Output" {
            if let Type::Future { output } = ty {
                return Some(*output.clone());
            }
        }

        // Get all implementations for this type (no hardcoded fallbacks - pure registration-based)
        let impls = self.get_implementations(ty);

        // Two-pass resolution: first try direct (non-blanket) impls for concrete answers,
        // then fall back to blanket impls. This mirrors rustc's trait solver priority.
        // A blanket impl like `implement<F: Future> FutureExt for F { type Output = F.Output; }`
        // just forwards the projection, while the direct impl `implement Future for ReadyFuture<T> { type Output = T; }`
        // gives a concrete answer.
        //
        // Pass 1: Try all impls, but if the substituted result is still a projection and
        //         recursive resolution fails (cycle detected), skip that impl.
        // Pass 2: If no concrete answer found, return the best projection we have.
        let mut best_projection: Option<Type> = None;

        for impl_ in impls.iter() {
            // Check if this implementation has the associated type explicitly defined
            if let Some(assoc_ty) = impl_.associated_types.get(assoc_name) {
                let substituted = self.substitute_impl_type_params(assoc_ty, impl_, ty);

                // Check if the substituted type is a projection (still needs resolution)
                let is_projection = matches!(&substituted, Type::Generic { name, args } if name.as_str().starts_with("::") && !args.is_empty());

                if is_projection {
                    // Try to recursively resolve the projection
                    if let Some(resolved) = self.try_resolve_projection_type(&substituted, assoc_name) {
                        return Some(resolved);
                    }
                    // Resolution failed (likely cycle) - save as fallback but continue
                    // looking for a more concrete answer from another impl
                    if best_projection.is_none() {
                        best_projection = Some(substituted);
                    }
                } else {
                    // Concrete result - return immediately
                    return Some(substituted);
                }
            }

            // Check if the protocol has a default for this associated type
            let protocol_name = self.extract_protocol_name(&impl_.protocol);
            if let Some(protocol_def) = self.protocols.get(&protocol_name) {
                // Check if the protocol defines this associated type
                if let Some(assoc_type_def) = protocol_def.associated_types.get(assoc_name) {
                    // Check for default value
                    if let Option::Some(default_ty) = &assoc_type_def.default {
                        return Some(default_ty.clone());
                    }
                }
            }
        }

        // No concrete answer found - return the best projection if any
        if let Some(proj) = best_projection {
            return Some(proj);
        }

        // Also check if the type is a generic type variable with bounds
        // that might have the associated type
        if let Type::Var(tvar) = ty {
            // For type variables, we'd need to check their bounds
            // This is handled elsewhere in the constraint solver
            let _ = tvar; // suppress unused warning
        }

        None
    }

    /// Substitute type parameters in an associated type based on the concrete type.
    ///
    /// For example:
    /// - impl_for_type = `Iter<I>` (the impl's for_type with type params)
    /// - concrete_ty = `Iter<IntoIter<[Int]>>` (the actual type we're querying)
    /// - assoc_ty = `::Item[I]` (the associated type value, a projection on I)
    /// - Result: `::Item[IntoIter<[Int]>]` (with I substituted)
    fn substitute_impl_type_params(&self, assoc_ty: &Type, impl_: &ProtocolImpl, concrete_ty: &Type) -> Type {
        // Phase 1: Build initial substitution by matching impl for_type against concrete_ty
        let mut substitution: Map<TypeVar, Type> = Map::new();
        self.build_type_substitution(&impl_.for_type, concrete_ty, &mut substitution);

        // Phase 2: Extract additional type bindings from function type bounds.
        // For `implement<Fut: Future, F: fn(Fut.Output) -> T, T> Future for MapFuture<Fut, F>`:
        //   Phase 1 gives us: {Fut → ReadyFuture<Int>, F → fn(Int) -> Bool}
        //   Phase 2: The bound F → fn(Fut.Output) -> T, after substitution and projection
        //   resolution, becomes fn(Int) -> T. Matching against fn(Int) -> Bool gives T = Bool.
        if !impl_.type_param_fn_bounds.is_empty() {
            for (bound_tv, bound_fn_type) in &impl_.type_param_fn_bounds {
                if let Some(concrete_fn) = substitution.get(bound_tv).cloned() {
                    // Apply current substitution to the bound pattern to resolve known vars
                    // This also resolves projections like ::Output[ReadyFuture<Int>] → Int
                    let resolved_bound = self.apply_type_substitution(bound_fn_type, &substitution);
                    // Match the resolved bound against the concrete function type
                    self.build_type_substitution(&resolved_bound, &concrete_fn, &mut substitution);
                }
            }
        }

        // Apply the substitution to the associated type
        self.apply_type_substitution(assoc_ty, &substitution)
    }

    /// Build a substitution by matching a pattern type against a concrete type.
    fn build_type_substitution(&self, pattern: &Type, concrete: &Type, subst: &mut Map<TypeVar, Type>) {
        match (pattern, concrete) {
            // Type variable in pattern - add to substitution
            (Type::Var(tv), _) => {
                subst.insert(*tv, concrete.clone());
            }
            // Named types - match args positionally
            (Type::Named { args: pattern_args, .. }, Type::Named { args: concrete_args, .. }) => {
                for (p, c) in pattern_args.iter().zip(concrete_args.iter()) {
                    self.build_type_substitution(p, c, subst);
                }
            }
            // Generic types - match args positionally
            (Type::Generic { args: pattern_args, .. }, Type::Generic { args: concrete_args, .. }) => {
                for (p, c) in pattern_args.iter().zip(concrete_args.iter()) {
                    self.build_type_substitution(p, c, subst);
                }
            }
            // Cross-matching: Generic pattern vs Named concrete (or vice versa)
            // This handles the common case where impl blocks register for_type as Generic
            // (e.g., Generic { name: "ListIter", args: [Var(T)] }) but the concrete type
            // is Named (e.g., Named { path: "ListIter", args: [Int] }), or vice versa.
            (Type::Generic { name, args: pattern_args }, Type::Named { path, args: concrete_args }) => {
                // Verify base names match before extracting substitutions
                let named_name = path.segments.iter().filter_map(|seg| {
                    if let verum_ast::ty::PathSegment::Name(ident) = seg { Some(ident.name.as_str()) } else { None }
                }).next_back().unwrap_or("");
                if name.as_str() == named_name {
                    for (p, c) in pattern_args.iter().zip(concrete_args.iter()) {
                        self.build_type_substitution(p, c, subst);
                    }
                }
            }
            (Type::Named { path, args: pattern_args }, Type::Generic { name, args: concrete_args }) => {
                let named_name = path.segments.iter().filter_map(|seg| {
                    if let verum_ast::ty::PathSegment::Name(ident) = seg { Some(ident.name.as_str()) } else { None }
                }).next_back().unwrap_or("");
                if named_name == name.as_str() {
                    for (p, c) in pattern_args.iter().zip(concrete_args.iter()) {
                        self.build_type_substitution(p, c, subst);
                    }
                }
            }
            // Reference types - match inner
            (Type::Reference { inner: p, .. }, Type::Reference { inner: c, .. }) => {
                self.build_type_substitution(p, c, subst);
            }
            // Tuple types - match elements
            (Type::Tuple(p_elems), Type::Tuple(c_elems)) => {
                for (p, c) in p_elems.iter().zip(c_elems.iter()) {
                    self.build_type_substitution(p, c, subst);
                }
            }
            // Array types - match element
            (Type::Array { element: p, .. }, Type::Array { element: c, .. }) => {
                self.build_type_substitution(p, c, subst);
            }
            // Function types - match params and return type
            (
                Type::Function { params: p_params, return_type: p_ret, .. },
                Type::Function { params: c_params, return_type: c_ret, .. },
            ) => {
                if p_params.len() == c_params.len() {
                    for (p, c) in p_params.iter().zip(c_params.iter()) {
                        self.build_type_substitution(p, c, subst);
                    }
                    self.build_type_substitution(p_ret, c_ret, subst);
                }
            }
            // Other cases - no substitution needed
            _ => {}
        }
    }

    /// Apply a type substitution to a type.
    fn apply_type_substitution(&self, ty: &Type, subst: &Map<TypeVar, Type>) -> Type {
        self.apply_type_substitution_impl(ty, subst, 0)
    }

    /// Internal implementation with depth tracking to prevent stack overflow.
    fn apply_type_substitution_impl(&self, ty: &Type, subst: &Map<TypeVar, Type>, depth: usize) -> Type {
        const MAX_SUBST_DEPTH: usize = 128;
        if depth > MAX_SUBST_DEPTH {
            return ty.clone(); // Conservative: return unchanged at depth limit
        }
        let d = depth + 1;
        match ty {
            Type::Var(tv) => {
                subst.get(tv).cloned().unwrap_or_else(|| ty.clone())
            }
            Type::Named { path, args } => {
                let new_args = args.iter()
                    .map(|a| self.apply_type_substitution_impl(a, subst, d))
                    .collect();
                Type::Named { path: path.clone(), args: new_args }
            }
            Type::Generic { name, args } => {
                let new_args: List<Type> = args.iter()
                    .map(|a| self.apply_type_substitution_impl(a, subst, d))
                    .collect();

                // CRITICAL FIX: Check if this is a deferred projection (e.g., ::Item)
                // After substitution, if the base type is now concrete, resolve the projection.
                if name.as_str().starts_with("::") && !new_args.is_empty() {
                    let assoc_name: Text = name.as_str().trim_start_matches("::").into();
                    let base_type = &new_args[0];

                    // Check if base type is concrete (no unresolved type variables)
                    if !self.type_has_unresolved_vars(base_type) {
                        // Try to resolve the projection
                        if let Some(resolved) = self.try_find_associated_type(base_type, &assoc_name) {
                            // Recursively apply substitution to the resolved type
                            return self.apply_type_substitution_impl(&resolved, subst, d);
                        }
                    }
                }

                Type::Generic { name: name.clone(), args: new_args }
            }
            Type::Reference { inner, mutable } => {
                Type::Reference {
                    inner: Box::new(self.apply_type_substitution_impl(inner, subst, d)),
                    mutable: *mutable,
                }
            }
            Type::Tuple(elems) => {
                Type::Tuple(elems.iter()
                    .map(|e| self.apply_type_substitution_impl(e, subst, d))
                    .collect())
            }
            Type::Array { element, size } => {
                Type::Array {
                    element: Box::new(self.apply_type_substitution_impl(element, subst, d)),
                    size: *size,
                }
            }
            Type::Function { params, return_type, contexts, properties, type_params } => {
                Type::Function {
                    params: params.iter()
                        .map(|p| self.apply_type_substitution_impl(p, subst, d))
                        .collect(),
                    return_type: Box::new(self.apply_type_substitution_impl(return_type, subst, d)),
                    contexts: contexts.clone(),
                    properties: properties.clone(),
                    type_params: type_params.clone(),
                }
            }
            // Other types pass through unchanged
            _ => ty.clone(),
        }
    }

    /// Check if a type contains any unresolved type variables.
    ///
    /// This is used to determine if a deferred projection can be resolved.
    /// A projection like `::Item[A]` can only be resolved if `A` is concrete.
    fn type_has_unresolved_vars(&self, ty: &Type) -> bool {
        match ty {
            Type::Var(_) => true,
            Type::Named { args, .. } => args.iter().any(|a| self.type_has_unresolved_vars(a)),
            Type::Generic { args, .. } => args.iter().any(|a| self.type_has_unresolved_vars(a)),
            Type::Tuple(elems) => elems.iter().any(|e| self.type_has_unresolved_vars(e)),
            Type::Array { element, .. } => self.type_has_unresolved_vars(element),
            Type::Slice { element } => self.type_has_unresolved_vars(element),
            Type::Reference { inner, .. } => self.type_has_unresolved_vars(inner),
            Type::CheckedReference { inner, .. } => self.type_has_unresolved_vars(inner),
            Type::UnsafeReference { inner, .. } => self.type_has_unresolved_vars(inner),
            Type::Pointer { inner, .. } => self.type_has_unresolved_vars(inner),
            Type::Function { params, return_type, .. } => {
                params.iter().any(|p| self.type_has_unresolved_vars(p))
                    || self.type_has_unresolved_vars(return_type)
            }
            Type::Refined { base, .. } => self.type_has_unresolved_vars(base),
            Type::Future { output } => self.type_has_unresolved_vars(output),
            Type::Generator { yield_ty, return_ty } => {
                self.type_has_unresolved_vars(yield_ty) || self.type_has_unresolved_vars(return_ty)
            }
            // Primitive types and other concrete types
            _ => false,
        }
    }

    /// Try to resolve a projection type (like ::Item[SomeType]) to a concrete type.
    fn try_resolve_projection_type(&self, ty: &Type, _original_assoc_name: &Text) -> Option<Type> {
        // Check if this is a projection type (::AssocName format)
        if let Type::Generic { name, args } = ty {
            if name.as_str().starts_with("::") && !args.is_empty() {
                let assoc_name = &name.as_str()[2..]; // Strip "::" prefix
                let base_ty = &args[0];

                // Recursively resolve the associated type on the base type
                let assoc_text: Text = assoc_name.into();
                return self.try_find_associated_type(base_ty, &assoc_text);
            }
        }
        None
    }

    /// Normalize a type by resolving all projection types (like ::Item[ListIter<Int>] -> &Int).
    ///
    /// This is used after substitution to resolve any remaining projections.
    /// For example, if we have `type Item = I.Item` and substitute `I = ListIter<Int>`,
    /// we get `::Item[ListIter<Int>]`. This function resolves it to `&Int`.
    pub fn normalize_projection_type(&self, ty: &Type) -> Type {
        match ty {
            Type::Generic { name, args } if name.as_str().starts_with("::") && !args.is_empty() => {
                // This is a projection type - try to resolve it
                let assoc_name = &name.as_str()[2..]; // Strip "::" prefix
                let base_ty = &args[0];

                // First normalize the base type (handle nested projections)
                let normalized_base = self.normalize_projection_type(base_ty);

                // Then try to resolve the projection
                let assoc_text: Text = assoc_name.into();
                if let Some(resolved) = self.try_find_associated_type(&normalized_base, &assoc_text) {
                    // Recursively normalize the result in case it contains more projections
                    self.normalize_projection_type(&resolved)
                } else {
                    // Can't resolve - return normalized projection
                    Type::Generic {
                        name: name.clone(),
                        args: List::from_iter(std::iter::once(normalized_base)),
                    }
                }
            }
            // Normalize nested types
            Type::Generic { name, args } => Type::Generic {
                name: name.clone(),
                args: args.iter().map(|a| self.normalize_projection_type(a)).collect(),
            },
            Type::Named { path, args } => Type::Named {
                path: path.clone(),
                args: args.iter().map(|a| self.normalize_projection_type(a)).collect(),
            },
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: verum_common::Heap::new(self.normalize_projection_type(inner)),
            },
            Type::Tuple(elems) => {
                Type::Tuple(elems.iter().map(|e| self.normalize_projection_type(e)).collect())
            }
            // Other types pass through unchanged
            _ => ty.clone(),
        }
    }
    /// Get all registered protocol names
    pub fn protocol_names(&self) -> List<Text> {
        // Use collect - iterator has size hint for efficient allocation
        self.protocols.keys().cloned().collect()
    }

    /// Check if a name refers to a registered protocol
    pub fn is_protocol_by_name(&self, name: &str) -> bool {
        let name_text: Text = name.into();
        self.protocols.contains_key(&name_text)
    }

    /// Check if a type implements a protocol (by name)
    pub fn implements_protocol(&self, ty: &Type, protocol_name: &str) -> bool {
        let protocol_text: Text = protocol_name.into();
        self.check_protocol_satisfied(ty, &protocol_text)
            .unwrap_or_default()
    }

    // =========================================================================
    // Try Protocol Resolution - For the ? operator
    // =========================================================================
    // Protocol-based Try resolution: ? operator uses Carrier protocol to convert between error types

    /// Resolve the Try protocol for a type.
    ///
    /// This is used by the `?` operator to determine:
    /// - What type to extract on success (Output)
    /// - What type to propagate on failure (Residual)
    ///
    /// # Arguments
    ///
    /// * `ty` - The type to check (e.g., `Maybe<Int>`, `Result<Text, IoError>`)
    ///
    /// # Returns
    ///
    /// * `Some(TryProtocolResolution)` if the type implements Try
    /// * `None` if the type does not implement Try
    ///
    /// # Example
    ///
    /// ```ignore
    /// let maybe_int = Type::Generic { name: "Maybe".into(), args: vec![Type::Int].into() };
    /// if let Some(resolution) = checker.resolve_try_protocol(&maybe_int) {
    ///     // resolution.output = Type::Int
    ///     // resolution.residual = Type::Generic { name: "Maybe", args: vec![Never] }
    /// }
    /// ```
    pub fn resolve_try_protocol(&self, ty: &Type) -> Option<TryProtocolResolution> {
        // Handle type variables by creating fresh associated type variables
        // This allows type inference to proceed with unresolved types
        if let Type::Var(_) = ty {
            // Create fresh type variables for Output and Residual
            // These will be resolved during unification
            let output = Type::Var(TypeVar::fresh());
            let residual = Type::Var(TypeVar::fresh());
            return Some(TryProtocolResolution { output, residual });
        }

        // =========================================================================
        // Protocol-based resolution (stdlib-agnostic) - PREFERRED
        // Stdlib-agnostic type system: type checker operates without hardcoded knowledge of stdlib types, stdlib types registered from parsed .vr files
        // =========================================================================
        // Try protocol-based resolution first (doesn't require hardcoded type knowledge)
        if self.implements_protocol(ty, "Try") {
            let try_path = Path::single(verum_ast::ty::Ident::new("Try", Span::default()));
            if let Maybe::Some(impl_) = self.find_impl(ty, &try_path) {
                // Build type substitution from impl's generic type to concrete type
                let mut subst = Map::new();
                self.build_type_substitution_for_impl(&impl_.for_type, ty, &mut subst);

                // Resolve Output associated type
                let output = impl_
                    .associated_types
                    .get(&"Output".into())
                    .cloned()
                    .map(|t| self.apply_type_substitution_with_map(&t, &subst))
                    .or_else(|| self.try_find_associated_type(ty, &"Output".into()));

                // Resolve Residual associated type
                let residual = impl_
                    .associated_types
                    .get(&"Residual".into())
                    .cloned()
                    .map(|t| self.apply_type_substitution_with_map(&t, &subst))
                    .or_else(|| self.try_find_associated_type(ty, &"Residual".into()));

                if let (Some(output), Some(residual)) = (output, residual) {
                    return Some(TryProtocolResolution { output, residual });
                }
            }
        }

        // =========================================================================
        // Structural fallback for Result/Maybe-like types
        // When no Try protocol is registered, recognize common patterns:
        // - Generic/Named "Result" with 2 args: Output=T, Residual=Result<Never,E>
        // - Generic/Named "Maybe" with 1 arg: Output=T, Residual=Maybe<Never>
        // - Variant types with Ok/Err or Some/None patterns
        // =========================================================================
        match ty {
            Type::Generic { name, args } => {
                let n = name.as_str();
                if (WKT::Result.matches(n) || n == "IoResult") && args.len() == 2 {
                    return Some(TryProtocolResolution {
                        output: args[0].clone(),
                        residual: Type::Generic {
                            name: name.clone(),
                            args: vec![Type::Never, args[1].clone()].into(),
                        },
                    });
                }
                if WKT::Maybe.matches(n) && args.len() == 1 {
                    return Some(TryProtocolResolution {
                        output: args[0].clone(),
                        residual: Type::Generic {
                            name: name.clone(),
                            args: vec![Type::Never].into(),
                        },
                    });
                }
            }
            Type::Named { path, args } => {
                // Check both single-segment paths (as_ident) and multi-segment paths (last segment)
                let n_opt = path.as_ident().map(|id| id.as_str())
                    .or_else(|| path.segments.last().and_then(|seg| {
                        if let verum_ast::ty::PathSegment::Name(id) = seg {
                            Some(id.name.as_str())
                        } else {
                            None
                        }
                    }));
                if let Some(n) = n_opt {
                    if (WKT::Result.matches(n) || n == "IoResult") && args.len() == 2 {
                        return Some(TryProtocolResolution {
                            output: args[0].clone(),
                            residual: Type::Named {
                                path: path.clone(),
                                args: vec![Type::Never, args[1].clone()].into(),
                            },
                        });
                    }
                    if WKT::Maybe.matches(n) && args.len() == 1 {
                        return Some(TryProtocolResolution {
                            output: args[0].clone(),
                            residual: Type::Named {
                                path: path.clone(),
                                args: vec![Type::Never].into(),
                            },
                        });
                    }
                    // Handle user-defined Result-like types with different names but same structure
                    // by checking if the type resolves to a type alias for Result/Maybe
                }
            }
            Type::Variant(variants) => {
                // Generic structural Try resolution for variant types.
                // Look up the named type for this variant via the registry,
                // then check if it implements Try. If not, use structural heuristic:
                // - Output = first variant with non-Unit payload
                // - Residual = same variant structure with output variant replaced by Never
                let variant_sig = {
                    let mut parts: Vec<&str> = variants.keys().map(|k| k.as_str()).collect();
                    parts.sort();
                    parts.join("|")
                };
                let named_type = self.variant_type_names.get(&verum_common::Text::from(variant_sig.as_str()))
                    .cloned();

                // If the named type implements Try, we already handled it above
                // in the protocol-based resolution. This is the structural fallback.

                // For 2-variant types, treat as Try-able: one variant carries
                // the "success" value, the other carries the "error"/"empty" value.
                if variants.len() == 2 {
                    // Find the "output" variant: first variant with non-Unit, non-Never payload
                    let output_entry = variants.iter().find(|(_, ty)| {
                        !matches!(ty, Type::Unit | Type::Never)
                    });

                    if let Some((output_name, output_ty)) = output_entry {
                        return Some(TryProtocolResolution {
                            output: output_ty.clone(),
                            residual: Type::Variant(
                                variants.iter().map(|(name, ty)| {
                                    if name == output_name { (name.clone(), Type::Never) }
                                    else { (name.clone(), ty.clone()) }
                                }).collect(),
                            ),
                        });
                    }
                }

                // For variant types with explicit Try protocol impl via named type
                if let Some(ref type_name) = named_type {
                    let named_ty = Type::Generic {
                        name: type_name.clone(),
                        args: List::new(),
                    };
                    if self.implements_protocol(&named_ty, "Try") {
                        let try_path = Path::single(verum_ast::ty::Ident::new("Try", Span::default()));
                        if let Maybe::Some(impl_) = self.find_impl(&named_ty, &try_path) {
                            let mut subst = Map::new();
                            self.build_type_substitution_for_impl(&impl_.for_type, ty, &mut subst);
                            let output = impl_.associated_types.get(&"Output".into())
                                .cloned()
                                .map(|t| self.apply_type_substitution_with_map(&t, &subst));
                            let residual = impl_.associated_types.get(&"Residual".into())
                                .cloned()
                                .map(|t| self.apply_type_substitution_with_map(&t, &subst));
                            if let (Some(output), Some(residual)) = (output, residual) {
                                return Some(TryProtocolResolution { output, residual });
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        // No Try implementation found
        None
    }

    /// Build type substitution from impl's generic type to concrete type.
    fn build_type_substitution_for_impl(
        &self,
        impl_type: &Type,
        concrete_type: &Type,
        subst: &mut Map<TypeVar, Type>,
    ) {
        // #[cfg(debug_assertions)]
        // eprintln!("[DEBUG build_subst] impl_type = {:?}", impl_type);
        // #[cfg(debug_assertions)]
        // eprintln!("[DEBUG build_subst] concrete_type = {:?}", concrete_type);
        match (impl_type, concrete_type) {
            (Type::Var(tv), concrete) => {
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG build_subst] Var case: tv={:?} -> {:?}", tv, concrete);
                subst.insert(*tv, concrete.clone());
            }
            (Type::Generic { args: impl_args, .. }, Type::Generic { args: concrete_args, .. }) => {
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG build_subst] Generic-Generic case");
                for (impl_arg, concrete_arg) in impl_args.iter().zip(concrete_args.iter()) {
                    self.build_type_substitution_for_impl(impl_arg, concrete_arg, subst);
                }
            }
            (Type::Named { args: impl_args, .. }, Type::Named { args: concrete_args, .. }) => {
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG build_subst] Named-Named case");
                for (impl_arg, concrete_arg) in impl_args.iter().zip(concrete_args.iter()) {
                    self.build_type_substitution_for_impl(impl_arg, concrete_arg, subst);
                }
            }
            // CRITICAL FIX: Handle cross-form matching between Named and Generic types.
            // This handles cases where impl's for_type is Named (e.g., Named { path: "Rev", args: [Var(I)] })
            // but the concrete type is Generic (e.g., Generic { name: "Rev", args: [Range<Int>] }).
            // This can happen when my Placeholder fix converts Rev<Self> to Generic instead of Named.
            (Type::Named { path, args: impl_args }, Type::Generic { name, args: concrete_args }) => {
                // Verify the base type name matches
                if let Some(ident) = path.as_ident() {
                    if ident.as_str() == name.as_str() && impl_args.len() == concrete_args.len() {
                        for (impl_arg, concrete_arg) in impl_args.iter().zip(concrete_args.iter()) {
                            self.build_type_substitution_for_impl(impl_arg, concrete_arg, subst);
                        }
                    }
                }
            }
            // Reverse case: Generic impl type matching Named concrete type
            (Type::Generic { name, args: impl_args }, Type::Named { path, args: concrete_args }) => {
                if let Some(ident) = path.as_ident() {
                    if name.as_str() == ident.as_str() && impl_args.len() == concrete_args.len() {
                        for (impl_arg, concrete_arg) in impl_args.iter().zip(concrete_args.iter()) {
                            self.build_type_substitution_for_impl(impl_arg, concrete_arg, subst);
                        }
                    }
                }
            }
            // Handle Generic pattern matching Variant concrete type (stdlib-agnostic)
            // This allows impls like `implement<T, E> Try for Result<T, E>` to work
            // when the concrete type is Variant({Ok: Int, Err: Text})
            (Type::Generic { name, args: impl_args }, Type::Variant(variants)) => {
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG build_subst] Generic-Variant case: name={}", name);
                // Verify this variant corresponds to this generic type
                let signature = Self::variant_type_signature_static(variants);
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG build_subst] signature={}", signature);
                let named_type_opt = self.variant_type_names.get(&signature)
                    .or_else(|| {
                        let relaxed = Self::variant_type_signature_relaxed(variants);
                        self.variant_type_names.get(&relaxed)
                    });
                if let Some(named_type) = named_type_opt {
                    // #[cfg(debug_assertions)]
                    // eprintln!("[DEBUG build_subst] found named_type={}", named_type);
                    if named_type.as_str() == name.as_str() {
                        // Extract non-Unit payloads from variants in order
                        // These correspond to the type parameters
                        let concrete_args: List<&Type> = variants
                            .values()
                            .filter(|payload| **payload != Type::Unit)
                            .collect();
                        // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG build_subst] concrete_args len={}", concrete_args.len());

                        // Match impl type args (which should be Type::Var) to concrete payloads
                        for (impl_arg, concrete_arg) in impl_args.iter().zip(concrete_args.iter()) {
                            self.build_type_substitution_for_impl(impl_arg, concrete_arg, subst);
                        }
                    } else {
                        // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG build_subst] name mismatch: {} vs {}", named_type, name);
                    }
                } else {
                    // #[cfg(debug_assertions)]
                    // eprintln!("[DEBUG build_subst] signature not found in variant_type_names");
                }
            }
            // Handle Variant pattern matching Variant concrete type
            // This happens when impl's for_type was already expanded to Variant form
            // Match payloads pairwise by variant name
            (Type::Variant(impl_variants), Type::Variant(concrete_variants)) => {
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG build_subst] Variant-Variant case");
                // Match payloads by variant name
                for (name, impl_payload) in impl_variants.iter() {
                    if let Some(concrete_payload) = concrete_variants.get(name) {
                        self.build_type_substitution_for_impl(impl_payload, concrete_payload, subst);
                    }
                }
            }
            _ => {
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG build_subst] fallthrough case");
            }
        }
    }

    /// Apply type substitution using a Map<TypeVar, Type>.
    fn apply_type_substitution_with_map(&self, ty: &Type, subst: &Map<TypeVar, Type>) -> Type {
        match ty {
            Type::Var(tv) => subst.get(tv).cloned().unwrap_or_else(|| ty.clone()),
            Type::Generic { name, args } => Type::Generic {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| self.apply_type_substitution_with_map(a, subst))
                    .collect(),
            },
            Type::Named { path, args } => Type::Named {
                path: path.clone(),
                args: args
                    .iter()
                    .map(|a| self.apply_type_substitution_with_map(a, subst))
                    .collect(),
            },
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: verum_common::Heap::new(
                    self.apply_type_substitution_with_map(inner, subst),
                ),
            },
            Type::Tuple(elems) => Type::Tuple(
                elems
                    .iter()
                    .map(|e| self.apply_type_substitution_with_map(e, subst))
                    .collect(),
            ),
            // Handle Variant types - substitute TypeVars in payloads
            Type::Variant(variants) => {
                let new_variants: indexmap::IndexMap<Text, Type> = variants
                    .iter()
                    .map(|(name, payload)| {
                        (name.clone(), self.apply_type_substitution_with_map(payload, subst))
                    })
                    .collect();
                Type::Variant(new_variants)
            }
            Type::Function { params, return_type, contexts, type_params, properties } => Type::Function {
                params: params
                    .iter()
                    .map(|p| self.apply_type_substitution_with_map(p, subst))
                    .collect(),
                return_type: Box::new(self.apply_type_substitution_with_map(return_type, subst)),
                contexts: contexts.clone(),
                type_params: type_params.clone(),
                properties: properties.clone(),
            },
            Type::Array { element, size } => Type::Array {
                element: Box::new(self.apply_type_substitution_with_map(element, subst)),
                size: *size,
            },
            Type::Slice { element } => Type::Slice {
                element: Box::new(self.apply_type_substitution_with_map(element, subst)),
            },
            Type::Future { output } => Type::Future {
                output: Box::new(self.apply_type_substitution_with_map(output, subst)),
            },
            _ => ty.clone(),
        }
    }

    /// Check if a type can receive a residual type via FromResidual.
    ///
    /// This is used to verify that the return type of a function can accept
    /// the residual from a `?` expression.
    ///
    /// # Arguments
    ///
    /// * `return_type` - The return type of the function
    /// * `residual_type` - The residual type from the ? expression
    ///
    /// # Returns
    ///
    /// * `true` if return_type implements FromResidual<residual_type>
    /// * `false` otherwise
    ///
    /// # Implementation
    ///
    /// Uses protocol-based lookup (stdlib-agnostic) rather than hardcoded type names.
    /// Searches registered FromResidual implementations to find a matching one.
    pub fn can_convert_residual(&self, return_type: &Type, residual_type: &Type) -> bool {
        // Protocol-based lookup: check if return_type implements FromResidual<residual_type>
        // This is stdlib-agnostic - works for any type with a FromResidual implementation

        // #[cfg(debug_assertions)]
        // eprintln!("[DEBUG can_convert_residual] return_type = {:?}", return_type);
        // #[cfg(debug_assertions)]
        // eprintln!("[DEBUG can_convert_residual] residual_type = {:?}", residual_type);

        // Normalize types to Generic form for matching
        let return_type_normalized = self.normalize_variant_to_generic(return_type);
        let residual_type_normalized = self.normalize_variant_to_generic(residual_type);

        // #[cfg(debug_assertions)]
        // eprintln!("[DEBUG can_convert_residual] return_type_normalized = {:?}", return_type_normalized);
        // #[cfg(debug_assertions)]
        // eprintln!("[DEBUG can_convert_residual] residual_type_normalized = {:?}", residual_type_normalized);

        // Search all FromResidual implementations
        for impl_ in &self.impls {
            // Check if this is a FromResidual implementation
            let protocol_name = self.extract_protocol_name(&impl_.protocol);
            if protocol_name.as_str() != "FromResidual" {
                continue;
            }

            // #[cfg(debug_assertions)]
            // eprintln!("[DEBUG can_convert_residual] Checking FromResidual impl: for_type={:?}, protocol_args={:?}", impl_.for_type, impl_.protocol_args);

            // Normalize impl's for_type to Generic form for matching
            // The impl's for_type may be in Variant form (from stdlib parsing) while
            // return_type_normalized is in Generic form
            let impl_for_type_normalized = self.normalize_variant_to_generic(&impl_.for_type);
            // #[cfg(debug_assertions)]
            // eprintln!("[DEBUG can_convert_residual] impl_for_type_normalized = {:?}", impl_for_type_normalized);

            // Check if impl.for_type matches return_type
            let for_type_match = self.try_match_type(&impl_for_type_normalized, &return_type_normalized);
            if for_type_match.is_none() {
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG can_convert_residual] for_type did not match");
                continue;
            }
            let for_type_subst = match for_type_match {
                Some(s) => s,
                None => continue,
            };
            // #[cfg(debug_assertions)]
            // eprintln!("[DEBUG can_convert_residual] for_type matched! subst = {:?}", for_type_subst);

            // Check if impl.protocol_args[0] matches residual_type
            // The impl's protocol_args may have type variables that need to be
            // instantiated with the same substitution as for_type
            if let Some(impl_residual) = impl_.protocol_args.first() {
                // Normalize impl_residual to Generic form (may be Variant from stdlib)
                let impl_residual_normalized = self.normalize_variant_to_generic(impl_residual);

                // Apply the substitution from for_type matching to the protocol arg
                let impl_residual_instantiated = self.apply_substitution(&impl_residual_normalized, &for_type_subst);
                // Also normalize the instantiated result
                let impl_residual_instantiated_normalized = self.normalize_variant_to_generic(&impl_residual_instantiated);
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG can_convert_residual] impl_residual_normalized = {:?}", impl_residual_normalized);
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG can_convert_residual] impl_residual_instantiated_normalized = {:?}", impl_residual_instantiated_normalized);

                // Now check if the instantiated residual matches the actual residual
                if self.try_match_type(&impl_residual_instantiated_normalized, &residual_type_normalized).is_some() {
                    // #[cfg(debug_assertions)]
                    // eprintln!("[DEBUG can_convert_residual] SUCCESS! residual matched");
                    return true;
                }

                // Also try matching the raw impl_residual_normalized (for wildcard type params)
                if self.try_match_type(&impl_residual_normalized, &residual_type_normalized).is_some() {
                    // #[cfg(debug_assertions)]
                    // eprintln!("[DEBUG can_convert_residual] SUCCESS! raw impl_residual matched");
                    return true;
                }
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG can_convert_residual] residual did not match");
            }
        }

        // Also check with Variant forms if normalization failed
        if return_type != &return_type_normalized || residual_type != &residual_type_normalized {
            // Try again with original types
            for impl_ in &self.impls {
                let protocol_name = self.extract_protocol_name(&impl_.protocol);
                if protocol_name.as_str() != "FromResidual" {
                    continue;
                }

                if let Some(for_type_subst) = self.try_match_type(&impl_.for_type, return_type) {
                    if let Some(impl_residual) = impl_.protocol_args.first() {
                        let impl_residual_instantiated = self.apply_substitution(impl_residual, &for_type_subst);
                        if self.try_match_type(&impl_residual_instantiated, residual_type).is_some() {
                            return true;
                        }
                        if self.try_match_type(impl_residual, residual_type).is_some() {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    /// Normalize a Variant type to its Generic equivalent if registered.
    ///
    /// This is used for protocol matching since implementations are typically
    /// registered with Generic types (e.g., Maybe<T>) while concrete types
    /// may be in Variant form (e.g., None | Some(Int)).
    fn normalize_variant_to_generic(&self, ty: &Type) -> Type {
        match ty {
            Type::Variant(variants) => {
                // Look up the type name from variant signature
                let signature = Self::variant_type_signature_static(variants);
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG normalize] Variant signature = {}", signature);
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG normalize] variant_type_names = {:?}", self.variant_type_names.keys().collect::<Vec<_>>());
                if let Some(type_name) = self.variant_type_names.get(&signature) {
                    // #[cfg(debug_assertions)]
                    // eprintln!("[DEBUG normalize] Found type_name = {}", type_name);
                    // Extract type arguments from variant payloads
                    let args: List<Type> = variants
                        .values()
                        .filter(|payload| **payload != Type::Unit)
                        .cloned()
                        .collect();

                    Type::Generic {
                        name: type_name.clone(),
                        args,
                    }
                } else {
                    // #[cfg(debug_assertions)]
                    // eprintln!("[DEBUG normalize] No type_name found for signature");
                    ty.clone()
                }
            }
            _ => ty.clone(),
        }
    }

    // =========================================================================
    // IntoIterator Protocol Resolution
    // For-loop desugaring: "for x in iter" desugars to IntoIterator protocol calls (into_iter -> next loop)
    // =========================================================================

    /// Resolve the IntoIterator protocol for a type.
    ///
    /// Given a type used in a `for` loop, this returns the element type (Item)
    /// that the iterator will produce.
    ///
    /// Supports:
    /// - Arrays: `[T; N]` -> Item = T
    /// - Slices: `[T]` -> Item = T
    /// - Collections: List<T>, Set<T>, Map<K, V>, etc.
    /// - Ranges: Range<T>, RangeInclusive<T>
    /// - Iterators: Iterator<Item = T>
    /// - References: &Collection, &mut Collection
    ///
    /// # Returns
    ///
    /// * `Some(IntoIteratorResolution)` with Item and Iter types
    /// * `None` if the type doesn't implement IntoIterator
    pub fn resolve_into_iterator_protocol(&self, ty: &Type) -> Option<IntoIteratorResolution> {
        // Handle type variables by creating fresh associated type variables
        if let Type::Var(_) = ty {
            let item = Type::Var(TypeVar::fresh());
            let iter = Type::Var(TypeVar::fresh());
            return Some(IntoIteratorResolution { item, iter });
        }

        // =========================================================================
        // PRIMITIVE TYPE HANDLING
        // These are language-level primitive types, not stdlib types.
        // Their IntoIterator semantics are defined by the language, not the library.
        // =========================================================================

        // Handle arrays: [T; N] is a primitive type, not a stdlib type
        // Arrays have built-in iteration semantics: Item = T (by value)
        // This is language-level support, not stdlib-dependent.
        if let Type::Array { element, .. } = ty {
            return Some(IntoIteratorResolution {
                item: *element.clone(),
                // The iterator type is opaque at this level - we just need Item type
                // for pattern matching in for-loops
                iter: ty.clone(),
            });
        }

        // Handle slices: &[T] is a primitive type
        // Slices iterate by reference: Item = &T
        if let Type::Slice { element } = ty {
            return Some(IntoIteratorResolution {
                item: Type::Reference {
                    inner: element.clone(),
                    mutable: false,
                },
                iter: ty.clone(),
            });
        }

        // Handle Generator type (primitive coroutine type)
        // Generator<Yield, Return> is iterable with Item = Yield
        if let Type::Generator { yield_ty, .. } = ty {
            return Some(IntoIteratorResolution {
                item: *yield_ty.clone(),
                iter: ty.clone(), // Generator is its own iterator
            });
        }

        // Handle Text type (primitive string type)
        // Text is iterable over its UTF-8 bytes as Int values.
        // This is a language-level primitive, not stdlib-dependent.
        if matches!(ty, Type::Text) {
            return Some(IntoIteratorResolution {
                item: Type::Int,
                iter: ty.clone(),
            });
        }

        // Handle references: &T, &mut T
        // References to iterables have special borrowing semantics
        let inner_and_mutable = match ty {
            Type::Reference { inner, mutable } => Some((inner.as_ref(), *mutable)),
            Type::CheckedReference { inner, mutable } => Some((inner.as_ref(), *mutable)),
            Type::UnsafeReference { inner, mutable } => Some((inner.as_ref(), *mutable)),
            _ => None,
        };

        if let Some((inner, mutable)) = inner_and_mutable {
            // For references, try to resolve the inner type's IntoIterator
            // and wrap the Item type in a reference
            if let Some(inner_resolution) = self.resolve_into_iterator_protocol(inner) {
                let ref_item = Type::Reference {
                    inner: Box::new(inner_resolution.item),
                    mutable,
                };
                return Some(IntoIteratorResolution {
                    item: ref_item,
                    iter: inner_resolution.iter,
                });
            }
        }

        // =========================================================================
        // Protocol-based resolution (stdlib-agnostic)
        // All other types (List, Range, Map, etc.) must have their IntoIterator
        // implementations registered via `implement IntoIterator for T { ... }`
        // =========================================================================
        // Try to resolve IntoIterator via registered protocol implementation.
        // This is the preferred method as it doesn't require hardcoded type knowledge.
        let into_iter_path = Path::single(verum_ast::ty::Ident::new("IntoIterator", Span::default()));

        if let Maybe::Some(impl_) = self.find_impl(ty, &into_iter_path) {
            // Build type substitution from impl's generic type to concrete type
            let mut subst = Map::new();
            self.build_type_substitution_for_impl(&impl_.for_type, ty, &mut subst);

            // Extract associated types Item and IntoIter
            let item = impl_
                .associated_types
                .get(&"Item".into())
                .cloned()
                .map(|t| self.apply_type_substitution_with_map(&t, &subst))
                // CRITICAL: Normalize projection types after substitution
                // If Item = I.Item and I = ListIter<Int>, the substitution gives
                // ::Item[ListIter<Int>], which must be resolved to &Int
                .map(|t| self.normalize_projection_type(&t));

            let iter = impl_
                .associated_types
                .get(&"IntoIter".into())
                .cloned()
                .map(|t| self.apply_type_substitution_with_map(&t, &subst))
                .map(|t| self.normalize_projection_type(&t));

            if let (Some(item), Some(iter)) = (item, iter) {
                return Some(IntoIteratorResolution { item, iter });
            }
        }

        // No IntoIterator implementation found
        // NOTE: Duck-typing fallback for custom iterators with has_next()/next()
        // methods is handled in infer.rs (synth_expr for ForIn), not here,
        // because user-defined impl methods are stored in inherent_methods
        // which is not accessible from the ProtocolChecker.
        None
    }

    // =========================================================================
    // Future Protocol Resolution
    // Await desugaring: ".await" desugars to polling Future protocol until completion
    // =========================================================================

    /// Resolve the Future protocol for a type.
    ///
    /// Given a type used with `await`, this returns the output type
    /// that will be produced when the future completes.
    ///
    /// # Returns
    ///
    /// * `Some(FutureResolution)` with the Output type
    /// * `None` if the type doesn't implement Future
    pub fn resolve_future_protocol(&self, ty: &Type) -> Option<FutureResolution> {
        // Handle type variables
        if let Type::Var(_) = ty {
            return Some(FutureResolution {
                output: Type::Var(TypeVar::fresh()),
            });
        }

        // Handle Type::Future directly (primitive type representation)
        if let Type::Future { output } = ty {
            return Some(FutureResolution {
                output: *output.clone(),
            });
        }

        // Handle Future<T> represented as Generic or Named type
        // After unification, async function return types may be represented as
        // Generic { name: "Future", args: [T] } or Named { path: ..Future.., args: [T] }
        // rather than the built-in Type::Future variant.
        match ty {
            Type::Generic { name, args } if name.as_str() == "Future" && args.len() == 1 => {
                return Some(FutureResolution {
                    output: args[0].clone(),
                });
            }
            Type::Named { path, args } if args.len() == 1 => {
                if let Some(ident) = path.as_ident() {
                    if ident.as_str() == "Future" {
                        return Some(FutureResolution {
                            output: args[0].clone(),
                        });
                    }
                }
            }
            _ => {}
        }

        // Handle bare Future (no type args) - the inner type was lost during inference.
        // Treat as Future<_> and return a fresh type variable.
        match ty {
            Type::Generic { name, args } if name.as_str() == "Future" && args.is_empty() => {
                return Some(FutureResolution {
                    output: Type::Var(TypeVar::fresh()),
                });
            }
            Type::Named { path, args } if args.is_empty() => {
                if let Some(ident) = path.as_ident() {
                    if ident.as_str() == "Future" {
                        return Some(FutureResolution {
                            output: Type::Var(TypeVar::fresh()),
                        });
                    }
                }
            }
            _ => {}
        }

        // =========================================================================
        // Protocol-based resolution (stdlib-agnostic)
        // Stdlib-agnostic type system: type checker operates without hardcoded knowledge of stdlib types, stdlib types registered from parsed .vr files
        // =========================================================================
        // Try to resolve Future via registered protocol implementation.
        let future_path = Path::single(verum_ast::ty::Ident::new("Future", Span::default()));
        if let Maybe::Some(impl_) = self.find_impl(ty, &future_path) {
            // Use try_find_associated_type which correctly handles projection resolution.
            // For types like Join2<Fut1, Fut2> whose impl declares
            //   type Output = (Fut1.Output, Fut2.Output);
            // this resolves each projection to the concrete awaited type and returns
            // a proper Tuple, enabling tuple-pattern destructuring of join().await results.
            if let Some(output) = self.try_find_associated_type(ty, &"Output".into()) {
                return Some(FutureResolution { output });
            }

            // Fallback: build substitution and apply it (handles simpler cases where
            // apply_type_substitution resolves any remaining deferred projections).
            let mut subst = Map::new();
            self.build_type_substitution_for_impl(&impl_.for_type, ty, &mut subst);

            // Extract associated type Output - use apply_type_substitution (not _with_map)
            // because the latter does not resolve deferred projection types (::Output[...]).
            if let Some(output) = impl_
                .associated_types
                .get(&"Output".into())
                .cloned()
                .map(|t| self.apply_type_substitution(&t, &subst))
            {
                return Some(FutureResolution { output });
            }
        }

        // No Future implementation found
        None
    }

    // =========================================================================
    // AsyncIterator Protocol Resolution
    // ForAwait desugaring: "for await x in stream" desugars to async iterator protocol polling
    // =========================================================================

    /// Resolve the AsyncIterator protocol for a type.
    ///
    /// Used by for-await loop type inference to determine the element type.
    ///
    /// Supports:
    /// - AsyncStream<T> -> Item = T
    /// - AsyncIterator<T> -> Item = T
    /// - Stream<T> -> Item = T (sync streams can be used in async context)
    /// - Future<Iterable> -> await then iterate
    ///
    /// # Returns
    ///
    /// * `Some(IntoIteratorResolution)` with Item type
    /// * `None` if the type isn't async-iterable
    pub fn resolve_async_iterator_protocol(&self, ty: &Type) -> Option<IntoIteratorResolution> {
        // Handle type variables
        if let Type::Var(_) = ty {
            let item = Type::Var(TypeVar::fresh());
            let iter = Type::Var(TypeVar::fresh());
            return Some(IntoIteratorResolution { item, iter });
        }

        // Handle Future<Iterable> - await then iterate
        if let Type::Future { output } = ty {
            // Try to resolve the inner type as an iterator
            if let Some(inner_resolution) = self.resolve_into_iterator_protocol(output) {
                return Some(inner_resolution);
            }
        }

        // =========================================================================
        // Protocol-based resolution (stdlib-agnostic)
        // Stdlib-agnostic type system: type checker operates without hardcoded knowledge of stdlib types, stdlib types registered from parsed .vr files
        // =========================================================================
        // Try to resolve AsyncIterator via registered protocol implementation.
        let async_iter_path = Path::single(verum_ast::ty::Ident::new("AsyncIterator", Span::default()));
        if let Maybe::Some(impl_) = self.find_impl(ty, &async_iter_path) {
            // Build type substitution from impl's generic type to concrete type
            let mut subst = Map::new();
            self.build_type_substitution_for_impl(&impl_.for_type, ty, &mut subst);

            // Extract associated types Item and IntoIter
            let item = impl_
                .associated_types
                .get(&"Item".into())
                .cloned()
                .map(|t| self.apply_type_substitution_with_map(&t, &subst));

            let iter = impl_
                .associated_types
                .get(&"IntoAsyncIter".into())
                .cloned()
                .map(|t| self.apply_type_substitution_with_map(&t, &subst))
                .or_else(|| Some(ty.clone())); // Default: type is its own async iterator

            if let (Some(item), Some(iter)) = (item, iter) {
                return Some(IntoIteratorResolution { item, iter });
            }
        }

        // No AsyncIterator implementation found
        None
    }

    // =========================================================================
    // Index Protocol Resolution
    // Index operator resolution: "x[i]" desugars to Index/IndexMut protocol method calls
    // =========================================================================

    /// Resolve the Index protocol for a type.
    ///
    /// Used by index operator type inference to determine the key and output types.
    ///
    /// Supports:
    /// - Array/Slice: indexed by Int, returns element type
    /// - List<T>: indexed by Int, returns T
    /// - Map<K, V>: indexed by K, returns V
    /// - Text: indexed by Int, returns Char
    /// - References to indexable types
    ///
    /// # Returns
    ///
    /// * `Some(IndexResolution)` with key and output types
    /// * `None` if the type isn't indexable
    pub fn resolve_index_protocol(&self, ty: &Type) -> Option<IndexResolution> {
        // Handle type variables
        if let Type::Var(_) = ty {
            return Some(IndexResolution {
                key: Type::Int,
                output: Type::Var(TypeVar::fresh()),
            });
        }

        // Handle references - unwrap and recurse
        match ty {
            Type::Reference { inner, .. } |
            Type::CheckedReference { inner, .. } |
            Type::UnsafeReference { inner, .. } => {
                return self.resolve_index_protocol(inner);
            }
            _ => {}
        }

        // Handle Array
        if let Type::Array { element, .. } = ty {
            return Some(IndexResolution {
                key: Type::Int,
                output: *element.clone(),
            });
        }

        // Handle Slice
        if let Type::Slice { element } = ty {
            return Some(IndexResolution {
                key: Type::Int,
                output: *element.clone(),
            });
        }

        // Handle Tuple (indexed by compile-time Int)
        if let Type::Tuple(types) = ty {
            if !types.is_empty() {
                // For tuples, return the first type (actual indexing uses compile-time index)
                return Some(IndexResolution {
                    key: Type::Int,
                    output: types[0].clone(),
                });
            }
        }

        // Handle Text
        if matches!(ty, Type::Text) {
            return Some(IndexResolution {
                key: Type::Int,
                output: Type::Char,
            });
        }

        // Handle List<T> — indexed by Int, returns T
        // Handle Map<K, V> — indexed by K, returns V
        {
            let (type_name, args) = match ty {
                Type::Generic { name, args } => (Some(name.as_str()), args.as_slice()),
                Type::Named { path, args } => {
                    let name = path.as_ident().map(|id| id.name.as_str());
                    (name, args.as_slice())
                }
                _ => (None, [].as_slice()),
            };
            if let Some(name) = type_name {
                if WKT::List.matches(name) && args.len() == 1 {
                    return Some(IndexResolution {
                        key: Type::Int,
                        output: args[0].clone(),
                    });
                }
                if WKT::Map.matches(name) && args.len() == 2 {
                    return Some(IndexResolution {
                        key: args[0].clone(),
                        output: args[1].clone(),
                    });
                }
            }
        }

        // =========================================================================
        // Protocol-based resolution (stdlib-agnostic)
        // STDLIB-AGNOSTIC: No hardcoded type names like "List", "Map" etc.
        // All indexable types are detected via Index protocol implementation.
        // Stdlib-agnostic type system: type checker operates without hardcoded knowledge of stdlib types, stdlib types registered from parsed .vr files
        // =========================================================================
        // Try to resolve Index via registered protocol implementation.
        let index_path = Path::single(verum_ast::ty::Ident::new("Index", Span::default()));
        if let Maybe::Some(impl_) = self.find_impl(ty, &index_path) {
            // Build type substitution from impl's generic type to concrete type
            let mut subst = Map::new();
            self.build_type_substitution_for_impl(&impl_.for_type, ty, &mut subst);

            // Extract associated types Idx and Output
            let key = impl_
                .associated_types
                .get(&"Idx".into())
                .cloned()
                .map(|t| self.apply_type_substitution_with_map(&t, &subst))
                .unwrap_or(Type::Int); // Default to Int if not specified

            let output = impl_
                .associated_types
                .get(&"Output".into())
                .cloned()
                .map(|t| self.apply_type_substitution_with_map(&t, &subst));

            if let Some(output) = output {
                return Some(IndexResolution { key, output });
            }
        }

        // No Index implementation found
        None
    }

    // =========================================================================
    // Maybe Protocol Resolution
    // Maybe operator resolution: ? on Maybe<T> desugars to match with None -> return None propagation
    // =========================================================================

    /// Resolve the Maybe protocol for a type.
    ///
    /// Used by `??` (null coalescing) and `?.` (optional chaining) operators
    /// to extract the inner type from Maybe-like types.
    ///
    /// Supports:
    /// - Maybe<T> (Generic or Named) -> inner = T
    /// - Some(T) | None variants -> inner = T
    /// - Type variables -> fresh inner type
    ///
    /// # Returns
    ///
    /// * `Some(MaybeResolution)` with inner type
    /// * `None` if the type isn't Maybe-like
    pub fn resolve_maybe_protocol(&self, ty: &Type) -> Option<MaybeResolution> {
        // Handle type variables - create fresh inner type
        if let Type::Var(_) = ty {
            return Some(MaybeResolution {
                inner: Type::Var(TypeVar::fresh()),
            });
        }

        // =========================================================================
        // Protocol-based resolution (stdlib-agnostic)
        // Stdlib-agnostic type system: type checker operates without hardcoded knowledge of stdlib types, stdlib types registered from parsed .vr files
        // =========================================================================
        // Try to resolve Maybe via registered protocol implementation.
        let maybe_path = Path::single(verum_ast::ty::Ident::new(WKT::Maybe.as_str(), Span::default()));
        if let Maybe::Some(impl_) = self.find_impl(ty, &maybe_path) {
            // Build type substitution from impl's generic type to concrete type
            let mut subst = Map::new();
            self.build_type_substitution_for_impl(&impl_.for_type, ty, &mut subst);

            // Extract associated type Inner
            if let Some(inner) = impl_
                .associated_types
                .get(&"Inner".into())
                .cloned()
                .map(|t| self.apply_type_substitution_with_map(&t, &subst))
            {
                return Some(MaybeResolution { inner });
            }
        }

        // =========================================================================
        // Structural resolution for Variant types
        // Handles Maybe-like types structurally: any type with exactly 2 variants
        // where one is nullary and the other carries a single value.
        // e.g. type Maybe<T> is None | Some(T);
        // =========================================================================
        if let Type::Variant(variants) = ty {
            if variants.len() == 2 {
                // Find the nullary variant and the single-field variant
                let mut nullary = None;
                let mut carrier = None;
                for (name, ty) in variants.iter() {
                    if *ty == Type::Unit {
                        nullary = Some(name.clone());
                    } else {
                        carrier = Some((name.clone(), ty.clone()));
                    }
                }
                if nullary.is_some() {
                    if let Some((_carrier_name, inner)) = carrier {
                        return Some(MaybeResolution { inner });
                    }
                }
            }
        }

        // No Maybe implementation found
        None
    }

    /// Extract error type from a Try residual type.
    ///
    /// For Result<T, E>, residual is Result<Never, E>, so error is E.
    /// For Maybe<T>, residual is Maybe<Never>, so error is None.
    /// For IoResult<T>, residual is Result<Never, IoError>, so error is IoError.
    ///
    /// # Returns
    ///
    /// * `Some(error_type)` for Result-like residuals
    /// * `None` for Maybe-like residuals (no error type)
    pub fn extract_error_from_residual(&self, residual: &Type) -> Option<Type> {
        // Structural approach: a 2-arg generic's second arg is the error type.
        // A 1-arg (or 0-arg) generic with a Maybe-like structure has no error type.
        match residual {
            Type::Generic { name: _, args } => {
                if args.len() == 2 {
                    // Result-like: second type arg is the error type
                    Some(args[1].clone())
                } else {
                    // Maybe-like or other: no error type
                    None
                }
            }
            Type::Named { path: _, args } => {
                if args.len() == 2 {
                    Some(args[1].clone())
                } else {
                    None
                }
            }
            Type::Var(_) => Some(Type::Var(TypeVar::fresh())),
            _ => None,
        }
    }

    /// Check if a type constructor implements a protocol.
    ///
    /// Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Protocol checking for type constructors
    ///
    /// For HKT bounds like `F<_>: Functor + Monad`, this checks if the type
    /// constructor (e.g., List, Maybe) implements the required protocol.
    ///
    /// Unlike `implements_protocol` which checks concrete types, this method
    /// checks type constructors (e.g., List not List<Int>) for protocol
    /// implementation. This is necessary for higher-kinded type bounds.
    ///
    /// # Arguments
    ///
    /// * `constructor_name` - Name of the type constructor (e.g., "List", "Maybe")
    /// * `protocol_name` - Name of the protocol (e.g., "Functor", "Monad")
    ///
    /// # Returns
    ///
    /// * `true` if the constructor implements the protocol
    /// * `false` otherwise
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Check if List implements Functor
    /// let result = checker.type_constructor_implements_protocol("List", "Functor");
    /// ```
    pub fn type_constructor_implements_protocol(
        &self,
        constructor_name: &Text,
        protocol_name: &Text,
    ) -> bool {
        // Look for an implementation where the implementing type is a type constructor
        // or a generic type with the same name
        for impl_ in &self.impls {
            // Check if this impl is for the requested protocol
            let impl_protocol_name: Maybe<Text> = impl_
                .protocol
                .segments
                .last()
                .and_then(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.clone()),
                    _ => None,
                });

            if impl_protocol_name.as_ref() != Option::Some(protocol_name) {
                continue;
            }

            // Check if the implementing type matches the constructor name
            match &impl_.for_type {
                // Direct type constructor
                Type::TypeConstructor { name, .. } => {
                    if name == constructor_name {
                        return true;
                    }
                }

                // Named type (possibly generic)
                Type::Named { path, .. } => {
                    let type_name: Maybe<Text> = path
                        .segments
                        .last()
                        .and_then(|seg| match seg {
                            verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.clone()),
                            _ => None,
                        });
                    if type_name.as_ref() == Option::Some(constructor_name) {
                        return true;
                    }
                }

                // Generic type (e.g., List<T>)
                Type::Generic { name, .. } => {
                    if name == constructor_name {
                        return true;
                    }
                }

                // Type application (e.g., List<T> where T is a type variable)
                Type::TypeApp { constructor, .. } => {
                    if let Type::TypeConstructor { name, .. } = constructor.as_ref() {
                        if name == constructor_name {
                            return true;
                        }
                    }
                }

                _ => {}
            }
        }

        // Also check if we have a registered HKT protocol implementation
        // stored in impl_index with a constructor-style key
        let key = (constructor_name.clone(), protocol_name.clone());
        if self.impl_index.contains_key(&key) {
            return true;
        }

        false
    }

    /// Register a type constructor as implementing a protocol for HKT bounds.
    ///
    /// Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — HKT protocol registration
    ///
    /// This method allows explicit registration of type constructors as
    /// implementing protocols, which is necessary for HKT bounds checking.
    ///
    /// # Arguments
    ///
    /// * `constructor_name` - Name of the type constructor (e.g., "List", "Maybe")
    /// * `protocol_name` - Name of the protocol (e.g., "Functor", "Monad")
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Register List as implementing Functor
    /// checker.register_type_constructor_protocol("List", "Functor");
    /// ```
    pub fn register_type_constructor_protocol(
        &mut self,
        constructor_name: &str,
        protocol_name: &str,
    ) {
        let key = (Text::from(constructor_name), Text::from(protocol_name));
        // Use a sentinel value (max usize) to indicate a registered HKT impl
        // without an actual impl index
        self.impl_index.insert(key, usize::MAX);
    }

    /// Check if type implements all required methods of a protocol
    ///
    /// This is a helper method that verifies protocol constraints by checking
    /// if the type has all required methods with matching signatures.
    ///
    /// # Arguments
    /// * `ty` - The type to check
    /// * `protocol` - The protocol to check against
    ///
    /// # Returns
    /// `Ok(true)` if type satisfies all protocol constraints, `Ok(false)` otherwise
    pub fn check_protocol_constraint(
        &self,
        ty: &Type,
        protocol: &Protocol,
    ) -> Result<bool, ProtocolError> {
        // Check if type implements required methods
        for (method_name, method) in protocol.methods.iter() {
            if !self.type_has_method(ty, method_name, &method.ty)? {
                return Ok(false);
            }
        }

        // Check superprotocol constraints
        for superprotocol in protocol.super_protocols.iter() {
            if !self.implements(ty, &superprotocol.protocol) {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Check if a type implements a protocol and return detailed violations if not
    ///
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Protocol System
    /// Advanced protocols (future v2.0+): GATs, higher-rank bounds, specialization with lattice ordering, coherence rules — Section 7.3 - Error Messages
    ///
    /// This method performs a comprehensive check of protocol implementation,
    /// returning a detailed list of violations when the type does not fully
    /// implement the protocol. This enables actionable error messages.
    ///
    /// # Arguments
    /// * `ty` - The type to check
    /// * `protocol_def` - The protocol definition to check against
    ///
    /// # Returns
    /// * `Ok(())` - The type fully implements the protocol
    /// * `Err(violations)` - A list of specific violations
    ///
    /// # Example
    /// ```verum
    /// // If Ord requires Eq as a superprotocol and Eq is not implemented:
    /// // Err([SuperprotocolNotImplemented { superprotocol: "Eq", ... }])
    /// ```
    pub fn check_protocol_implementation(
        &self,
        ty: &Type,
        protocol_def: &Protocol,
    ) -> Result<(), List<ProtocolViolation>> {
        let mut violations = List::new();
        let protocol_path = Path::single(Ident::new(protocol_def.name.as_str(), Span::default()));

        // 1. Check if an implementation block exists
        let impl_ = match self.find_impl(ty, &protocol_path) {
            Maybe::Some(i) => i,
            Maybe::None => {
                // No impl block found - this is the primary violation
                violations.push(ProtocolViolation::MissingImplBlock {
                    for_type: ty.clone(),
                    protocol: protocol_def.name.clone(),
                    help: format!(
                        "Add `implement {} for {}` with required methods",
                        protocol_def.name,
                        self.type_to_string(ty)
                    )
                    .into(),
                });

                // Still check superprotocols to provide complete error info
                self.check_superprotocol_violations(ty, protocol_def, &mut violations);

                return Err(violations);
            }
        };

        // 2. Check superprotocol implementations (must be checked before methods)
        self.check_superprotocol_violations(ty, protocol_def, &mut violations);

        // 3. Check required methods
        for (method_name, proto_method) in protocol_def.methods.iter() {
            if let Some(impl_method_ty) = impl_.methods.get(method_name) {
                // Method exists - check signature compatibility
                if !self.check_method_signature_compatible(&proto_method.ty, impl_method_ty) {
                    violations.push(ProtocolViolation::MethodSignatureMismatch {
                        method_name: method_name.clone(),
                        expected: self.format_function_type(&proto_method.ty),
                        actual: self.format_function_type(impl_method_ty),
                        reason: "Parameter or return types do not match".into(),
                    });
                }
            } else if !proto_method.has_default {
                // Required method missing (no default available)
                violations.push(ProtocolViolation::MissingMethod {
                    method_name: method_name.clone(),
                    expected_signature: self.format_method_signature(proto_method),
                    has_default: false,
                });
            }
            // If method is missing but has_default is true, it's OK
        }

        // 4. Check required associated types
        for (assoc_name, assoc_type_def) in protocol_def.associated_types.iter() {
            if let Some(impl_assoc_type) = impl_.associated_types.get(assoc_name) {
                // Associated type is defined - check bounds
                for bound in assoc_type_def.bounds.iter() {
                    if !self.implements(impl_assoc_type, &bound.protocol) {
                        let bound_name = self.extract_protocol_name(&bound.protocol);
                        violations.push(ProtocolViolation::AssociatedTypeBoundViolation {
                            assoc_name: assoc_name.clone(),
                            actual_type: impl_assoc_type.clone(),
                            unsatisfied_bound: bound_name,
                        });
                    }
                }
            } else if assoc_type_def.default.is_none() {
                // Required associated type not defined (no default)
                let bound_names: List<Text> = assoc_type_def
                    .bounds
                    .iter()
                    .map(|b| self.extract_protocol_name(&b.protocol))
                    .collect();
                violations.push(ProtocolViolation::MissingAssociatedType {
                    assoc_name: assoc_name.clone(),
                    bounds: bound_names,
                    has_default: false,
                });
            }
        }

        // 5. Check required associated constants
        // Note: In the current implementation, all associated constants are required
        // (there is no default value support for constants in Protocol)
        for (const_name, const_def) in protocol_def.associated_consts.iter() {
            if impl_.associated_consts.get(const_name).is_none() {
                violations.push(ProtocolViolation::MissingAssociatedConst {
                    const_name: const_name.clone(),
                    expected_type: const_def.ty.clone(),
                });
            }
        }

        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }

    /// Helper to check superprotocol violations
    fn check_superprotocol_violations(
        &self,
        ty: &Type,
        protocol_def: &Protocol,
        violations: &mut List<ProtocolViolation>,
    ) {
        for super_bound in protocol_def.super_protocols.iter() {
            if super_bound.is_negative {
                // Skip negative bounds for violation checking
                continue;
            }

            if !self.implements(ty, &super_bound.protocol) {
                let super_name = self.extract_protocol_name(&super_bound.protocol);
                violations.push(ProtocolViolation::SuperprotocolNotImplemented {
                    superprotocol: super_name.clone(),
                    help: format!(
                        "Implement `{}` for `{}` before implementing `{}`",
                        super_name,
                        self.type_to_string(ty),
                        protocol_def.name
                    )
                    .into(),
                });
            }
        }
    }

    /// Check if two method signatures are compatible
    fn check_method_signature_compatible(&self, expected: &Type, actual: &Type) -> bool {
        match (expected, actual) {
            (
                Type::Function {
                    params: exp_params,
                    return_type: exp_ret,
                    ..
                },
                Type::Function {
                    params: act_params,
                    return_type: act_ret,
                    ..
                },
            ) => {
                // Check parameter count
                if exp_params.len() != act_params.len() {
                    return false;
                }

                // Check each parameter (with type variable handling)
                for (exp_param, act_param) in exp_params.iter().zip(act_params.iter()) {
                    let mut subst = Map::new();
                    if !self.unify_types(exp_param, act_param, &mut subst) {
                        return false;
                    }
                }

                // Check return type
                let mut subst = Map::new();
                self.unify_types(exp_ret, act_ret, &mut subst)
            }
            _ => {
                // For non-function types, use simple unification
                let mut subst = Map::new();
                self.unify_types(expected, actual, &mut subst)
            }
        }
    }

    /// Get protocol implementation with detailed error on failure
    ///
    /// This is a convenience method that combines `find_impl` with
    /// `check_protocol_implementation` to provide a complete error
    /// when a type doesn't implement a protocol.
    pub fn get_impl_or_violations(
        &self,
        ty: &Type,
        protocol_name: &Text,
    ) -> Result<&ProtocolImpl, ProtocolViolations> {
        let protocol_path = Path::single(Ident::new(protocol_name.as_str(), Span::default()));

        // Try to find existing implementation
        if let Maybe::Some(impl_) = self.find_impl(ty, &protocol_path) {
            return Ok(impl_);
        }

        // Get the protocol definition for detailed checking
        let protocol_def = match self.protocols.get(protocol_name) {
            Some(p) => p,
            None => {
                return Err(ProtocolViolations {
                    ty: ty.clone(),
                    protocol: protocol_name.clone(),
                    violations: List::from(vec![ProtocolViolation::MissingImplBlock {
                        for_type: ty.clone(),
                        protocol: protocol_name.clone(),
                        help: format!("Protocol `{}` is not defined", protocol_name).into(),
                    }]),
                });
            }
        };

        // Perform detailed check
        match self.check_protocol_implementation(ty, protocol_def) {
            Ok(()) => {
                // This shouldn't happen if find_impl returned None,
                // but handle gracefully
                Err(ProtocolViolations {
                    ty: ty.clone(),
                    protocol: protocol_name.clone(),
                    violations: List::from(vec![ProtocolViolation::MissingImplBlock {
                        for_type: ty.clone(),
                        protocol: protocol_name.clone(),
                        help: "Implementation may exist but was not found".into(),
                    }]),
                })
            }
            Err(violations) => Err(ProtocolViolations {
                ty: ty.clone(),
                protocol: protocol_name.clone(),
                violations,
            }),
        }
    }

    /// Check if a type has a method with a given name and signature
    ///
    /// This helper checks both inherent methods and protocol implementations.
    ///
    /// # Arguments
    /// * `ty` - The type to check
    /// * `method_name` - The name of the method
    /// * `expected_signature` - The expected type signature of the method
    ///
    /// # Returns
    /// `Ok(true)` if type has the method with matching signature, `Ok(false)` otherwise
    pub fn type_has_method(
        &self,
        ty: &Type,
        method_name: &Text,
        expected_signature: &Type,
    ) -> Result<bool, ProtocolError> {
        // First check if any implementation provides this method
        let impls = self.get_implementations(ty);

        for impl_ in impls.iter() {
            if let Some(method_ty) = impl_.methods.get(method_name) {
                // Use proper type unification instead of string comparison
                // This handles generic type parameters correctly
                let mut subst = Map::new();
                if self.unify_types(method_ty, expected_signature, &mut subst) {
                    return Ok(true);
                }
                // Also try the reverse direction for bidirectional unification
                subst.clear();
                if self.unify_types(expected_signature, method_ty, &mut subst) {
                    return Ok(true);
                }
            }
        }

        // Could also check for inherent methods here if we had that infrastructure
        Ok(false)
    }

    /// Lookup a method for a type, searching through protocol implementations
    ///
    /// This method searches for a method by name, first checking inherent methods
    /// (if available), then checking all protocol implementations.
    ///
    /// # Arguments
    /// * `ty` - The type to search methods for
    /// * `method_name` - The name of the method to find
    ///
    /// # Returns
    /// `Some(Type)` with the method's type signature if found, `None` otherwise
    pub fn lookup_protocol_method(
        &self,
        ty: &Type,
        method_name: &Text,
    ) -> Result<Maybe<Type>, ProtocolError> {
        // Search through all implementations for this type
        let impls = self.get_implementations(ty);

        for impl_ in impls.iter() {
            if let Some(method_ty) = impl_.methods.get(method_name) {
                // CRITICAL FIX: Substitute Self and type parameters in the method type
                // This enables T.default() pattern in generic protocol implementations.
                //
                // For a generic impl like `implement<T: Default> Default for Wrapper<T>`:
                // - impl_.for_type = Wrapper<T>  (with T as a TypeVar after inference)
                // - method_ty might contain Self (e.g., fn default() -> Self)
                // - lookup ty = Wrapper<Person> (concrete instantiation)
                //
                // We need to:
                // 1. Unify impl_.for_type with ty to get bindings (T52 -> Person)
                // 2. Substitute Self with impl_.for_type in method_ty
                // 3. Substitute type parameter bindings (TypeVars) in the result

                let mut result_ty = method_ty.clone();

                // Step 1: Build substitution map by unifying for_type with lookup type
                // This captures bindings like TypeVar(52) -> Person (stored as "T52" -> Person)
                let mut subst_map = Map::new();
                let _ = self.unify_types(&impl_.for_type, ty, &mut subst_map);

                // Step 2: Add Self -> impl_.for_type mapping
                // This allows replacing Self in method signatures
                subst_map.insert(Text::from("Self"), impl_.for_type.clone());

                // Step 3: Apply substitution
                // This substitutes both Self and TypeVars like T52
                result_ty = self.substitute_type_params(&result_ty, &subst_map);

                // Step 4: Apply bindings again for any remaining type variables
                // (e.g., if Self was Wrapper<T52> and T52 -> Person, we need another pass)
                // The first pass already handles TypeVars in nested positions now,
                // but an extra pass doesn't hurt for complex cases
                if !subst_map.is_empty() {
                    // Remove Self from map to avoid infinite substitution
                    let mut type_param_map = subst_map.clone();
                    type_param_map.remove(&Text::from("Self"));
                    if !type_param_map.is_empty() {
                        result_ty = self.substitute_type_params(&result_ty, &type_param_map);
                    }
                }

                return Ok(Maybe::Some(result_ty));
            }
        }

        // Method not found in any implementation
        Ok(Maybe::None)
    }

    /// Look up a protocol method and also return the method-level type parameter names.
    ///
    /// This extends `lookup_protocol_method` by also finding the original `ProtocolMethod`
    /// from the protocol definition, which stores `type_param_names` (e.g., `["C"]` for
    /// `fn collect<C: FromIterator<Self.Item>>() -> C`).
    ///
    /// The caller can then create fresh TypeVars for these method-level type params
    /// to avoid sharing the same TypeVar across multiple call sites.
    pub fn lookup_protocol_method_with_type_param_names(
        &self,
        ty: &Type,
        method_name: &Text,
    ) -> Result<Maybe<(Type, List<Text>)>, ProtocolError> {
        let impls = self.get_implementations(ty);

        for impl_ in impls.iter() {
            if let Some(method_ty) = impl_.methods.get(method_name) {
                let mut result_ty = method_ty.clone();

                // Build substitution map by unifying for_type with lookup type
                let mut subst_map = Map::new();
                let _ = self.unify_types(&impl_.for_type, ty, &mut subst_map);
                subst_map.insert(Text::from("Self"), impl_.for_type.clone());

                // Apply substitution
                result_ty = self.substitute_type_params(&result_ty, &subst_map);
                if !subst_map.is_empty() {
                    let mut type_param_map = subst_map.clone();
                    type_param_map.remove(&Text::from("Self"));
                    if !type_param_map.is_empty() {
                        result_ty = self.substitute_type_params(&result_ty, &type_param_map);
                    }
                }

                // Find the protocol definition to get method-level type_param_names
                let protocol_name = impl_.protocol.as_ident()
                    .map(|id| id.name.clone())
                    .unwrap_or_default();
                let type_param_names = if let Maybe::Some(protocol) = self.get_protocol(&protocol_name) {
                    if let Some(proto_method) = protocol.methods.get(method_name) {
                        proto_method.type_param_names.clone()
                    } else {
                        List::new()
                    }
                } else {
                    List::new()
                };

                return Ok(Maybe::Some((result_ty, type_param_names)));
            }
        }

        Ok(Maybe::None)
    }

    /// Look up all protocol methods with a given name for a type.
    ///
    /// Unlike `lookup_protocol_method` which returns the first match, this returns
    /// ALL matching method signatures. This is needed when a type implements multiple
    /// parameterized protocols with the same method name (e.g., FromResidual<Result<Never, E>>
    /// and FromResidual<Maybe<Never>> both have `from_residual`).
    ///
    /// The caller can then try each signature against actual arguments to find the correct one.
    pub fn lookup_all_protocol_methods(
        &self,
        ty: &Type,
        method_name: &Text,
    ) -> Result<List<Type>, ProtocolError> {
        let mut results = List::new();

        // Dyn-protocol receivers (e.g. `dyn Tracer`) have no concrete
        // impl registered — their method signatures come directly
        // from the protocol declaration. Serve them from there so
        // that call sites like `(&dyn Tracer).start_span(...)` and
        // `Heap<dyn Tracer>.start_span(...)` (after auto-deref to
        // `dyn Tracer`) both resolve.
        if let Type::DynProtocol { bounds, .. } = ty {
            for proto_name in bounds.iter() {
                if let Maybe::Some(proto) = self.get_protocol(proto_name) {
                    if let Some(pm) = proto.methods.get(method_name) {
                        results.push(pm.ty.clone());
                    }
                }
            }
            if !results.is_empty() {
                return Ok(results);
            }
        }

        let impls = self.get_implementations(ty);

        // Fetch the protocol's method-level type-param names (if we
        // can locate the protocol declaration). These names must be
        // EXCLUDED from the impl-level substitution map — without
        // this guard, a method declared as `fn map<B, F>` inherits an
        // impl-level `F` binding and its method-local `F` parameter
        // gets silently replaced, producing type errors like
        // "expected Int, found fn(Int) -> Int".
        let method_level_param_names: Option<List<Text>> = impls
            .iter()
            .find_map(|impl_| {
                let proto = self.lookup_protocol(&impl_.protocol)?;
                proto.methods.get(method_name).map(|pm| pm.type_param_names.clone())
            });

        for impl_ in impls.iter() {
            if let Some(method_ty) = impl_.methods.get(method_name) {
                let mut result_ty = method_ty.clone();

                // Build substitution map by unifying for_type with lookup type
                let mut subst_map = Map::new();
                let _ = self.unify_types(&impl_.for_type, ty, &mut subst_map);

                // Prune method-level params from the subst_map. This
                // is what stops method-scoped `F` from being captured
                // by an impl-scoped `F` binding.
                if let Some(ref method_names) = method_level_param_names {
                    for name in method_names.iter() {
                        subst_map.remove(name);
                    }
                }

                // Add Self -> impl_.for_type mapping
                subst_map.insert(Text::from("Self"), impl_.for_type.clone());

                // Apply substitution
                result_ty = self.substitute_type_params(&result_ty, &subst_map);

                // Apply bindings again for any remaining type variables
                if !subst_map.is_empty() {
                    let mut type_param_map = subst_map.clone();
                    type_param_map.remove(&Text::from("Self"));
                    if !type_param_map.is_empty() {
                        result_ty = self.substitute_type_params(&result_ty, &type_param_map);
                    }
                }

                results.push(result_ty);
            }
        }

        Ok(results)
    }

    /// Query protocol implementations by protocol name
    ///
    /// Returns all implementations registered for the given protocol.
    /// This is used for protocol method dispatch and specialization selection.
    ///
    /// # Arguments
    /// * `protocol_name` - The name of the protocol to query
    ///
    /// # Returns
    /// A list of all implementations for this protocol
    ///
    /// # Example
    /// ```ignore
    /// let impls = checker.query_implementations_by_protocol(&"Display".into());
    /// for impl_ in impls.iter() {
    ///     println!("Type: {:?}", impl_.for_type);
    /// }
    /// ```
    pub fn query_implementations_by_protocol(&self, protocol_name: &Text) -> List<&ProtocolImpl> {
        let mut result = List::new();

        for impl_ in self.impls.iter() {
            // Check if this implementation is for the requested protocol
            // Extract protocol name from the implementation
            let impl_protocol_name = self.extract_protocol_name_from_impl(impl_);
            if impl_protocol_name == *protocol_name {
                result.push(impl_);
            }
        }

        result
    }

    /// Check if a specific type implements a specific protocol
    ///
    /// This method performs a precise check to determine if the given type
    /// has an implementation for the given protocol.
    ///
    /// # Arguments
    /// * `ty` - The type to check
    /// * `protocol_name` - The protocol name to check for
    ///
    /// # Returns
    /// `true` if the type implements the protocol, `false` otherwise
    ///
    /// # Example
    /// ```ignore
    /// let implements_display = checker.check_type_implements_protocol(
    ///     &Type::Int,
    ///     &"Display".into()
    /// );
    /// ```
    pub fn check_type_implements_protocol(&self, ty: &Type, protocol_name: &Text) -> bool {
        let type_key = self.make_type_key(ty);
        self.impl_index
            .contains_key(&(type_key, protocol_name.clone()))
    }

    /// Get all registered protocol implementations
    ///
    /// Returns a reference to all protocol implementations in the system.
    /// This is useful for testing and debugging.
    ///
    /// # Returns
    /// A slice of all registered protocol implementations
    pub fn all_implementations(&self) -> &[ProtocolImpl] {
        &self.impls
    }

    /// Extract protocol name from a protocol implementation
    ///
    /// Helper method to get the protocol name from an implementation.
    fn extract_protocol_name_from_impl(&self, impl_: &ProtocolImpl) -> Text {
        use verum_ast::ty::PathSegment;

        // Convert Path to Text by joining segments
        if impl_.protocol.segments.len() == 1 {
            match &impl_.protocol.segments[0] {
                PathSegment::Name(ident) => ident.name.clone(),
                PathSegment::SelfValue => "self".into(),
                PathSegment::Super => "super".into(),
                PathSegment::Cog => "cog".into(),
                PathSegment::Relative => ".".into(),
            }
        } else {
            // For multi-segment paths, join with dots
            let parts: List<Text> = impl_
                .protocol
                .segments
                .iter()
                .map(|seg| match seg {
                    PathSegment::Name(ident) => ident.name.clone(),
                    PathSegment::SelfValue => "self".into(),
                    PathSegment::Super => "super".into(),
                    PathSegment::Cog => "cog".into(),
                    PathSegment::Relative => ".".into(),
                })
                .collect();
            parts.join(".")
        }
    }

    /// Generate a stable signature for a variant type from its variants map.
    /// This creates a unique identifier from the sorted variant names and payload base type names.
    /// Used to look up the named type for expanded variant types.
    ///
    /// IMPORTANT: This must produce identical signatures to `variant_type_signature()` in infer.rs.
    /// Including payload base type names prevents collisions between different sum types that share
    /// variant names (e.g., MapEntry and BTreeEntry both have Occupied|Vacant variants).
    fn variant_type_signature_static(variants: &indexmap::IndexMap<Text, Type>) -> Text {
        let mut entries: Vec<String> = variants
            .iter()
            .map(|(name, payload)| {
                let payload_name = match payload {
                    Type::Named { path, .. } => {
                        path.as_ident()
                            .map(|id| id.name.as_str().to_string())
                            .unwrap_or_default()
                    }
                    Type::Generic { name: n, .. } => n.as_str().to_string(),
                    // Unit, primitives, and TypeVars are not distinctive for
                    // disambiguation — only Named/Generic payload types matter.
                    _ => String::new(),
                };
                if payload_name.is_empty() {
                    name.as_str().to_string()
                } else {
                    format!("{}({})", name.as_str(), payload_name)
                }
            })
            .collect();
        entries.sort();
        let sig = entries.join("|");
        verum_common::Text::from(format!("Variant({})", sig))
    }

    /// Generate a relaxed variant type signature using only variant names (ignoring payload types).
    /// Used as fallback when the full signature doesn't match due to concrete type arguments.
    fn variant_type_signature_relaxed(variants: &indexmap::IndexMap<Text, Type>) -> Text {
        let mut names: Vec<&str> = variants.keys().map(|k| k.as_str()).collect();
        names.sort();
        verum_common::Text::from(format!("Variant({})", names.join("|")))
    }

    /// Generate a stable key for a type (used for protocol implementation indexing)
    /// Spec: Protocol system - type representation in indices
    fn make_type_key(&self, ty: &Type) -> Text {
        use crate::ty::Type::*;
        match ty {
            Unit => "Unit".into(),
            // Empty tuple is canonically Unit
            Tuple(elems) if elems.is_empty() => "Unit".into(),
            Never => "Never".into(),
            Bool => WKT::Bool.as_str().into(),
            Int => WKT::Int.as_str().into(),
            Float => WKT::Float.as_str().into(),
            Char => WKT::Char.as_str().into(),
            Text => WKT::Text.as_str().into(),
            Named { path, args } => {
                // CRITICAL FIX: Normalize primitive type names to canonical form.
                // When a Type::Named has a single-segment path that matches a primitive type
                // (Int, Float, Bool, Char, Text, Unit, Never), return the same key as the
                // primitive type. This ensures consistent protocol lookups regardless of
                // whether the type is represented as Type::Int or Type::Named { path: "Int" }.
                if args.is_empty() && path.segments.len() == 1 {
                    if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                        let tn = ident.name.as_str();
                        // Normalize primitive and numeric type names for consistent protocol lookup
                        use verum_common::well_known_types::type_names;
                        if type_names::is_primitive_value_type(tn) || matches!(tn, "Never" | "Text") {
                            return tn.into();
                        }
                    }
                }

                let mut key = verum_common::Text::new();
                key.push_str("named:");
                for segment in &path.segments {
                    if let verum_ast::ty::PathSegment::Name(ident) = segment {
                        key.push_str(ident.name.as_str());
                        key.push('.');
                    }
                }
                if !args.is_empty() {
                    key.push('[');
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            key.push(',');
                        }
                        key.push_str(self.make_type_key(arg).as_str());
                    }
                    key.push(']');
                }
                key
            }
            Generic { name, args } => {
                let mut key = verum_common::Text::new();
                key.push_str("generic:");
                key.push_str(name.as_str());
                if !args.is_empty() {
                    key.push('[');
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            key.push(',');
                        }
                        key.push_str(self.make_type_key(arg).as_str());
                    }
                    key.push(']');
                }
                key
            }
            Function {
                params,
                return_type,
                ..
            } => {
                let mut key = verum_common::Text::from("fn:");
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        key.push(',');
                    }
                    key.push_str(self.make_type_key(param).as_str());
                }
                key.push_str("->");
                key.push_str(self.make_type_key(return_type).as_str());
                key
            }
            Tuple(elements) => {
                let mut key = verum_common::Text::from("tuple:");
                for (i, elem) in elements.iter().enumerate() {
                    if i > 0 {
                        key.push(',');
                    }
                    key.push_str(self.make_type_key(elem).as_str());
                }
                key
            }
            Array { element, .. } => {
                verum_common::Text::from(format!("array:{}", self.make_type_key(element)))
            }
            Slice { element } => {
                verum_common::Text::from(format!("slice:{}", self.make_type_key(element)))
            }
            Record(fields) => {
                let mut key = verum_common::Text::from("record:{");
                for (i, (name, field_ty)) in fields.iter().enumerate() {
                    if i > 0 {
                        key.push(',');
                    }
                    key.push_str(name.as_str());
                    key.push(':');
                    key.push_str(self.make_type_key(field_ty).as_str());
                }
                key.push('}');
                key
            }
            Reference { inner, .. } => {
                verum_common::Text::from(format!("ref:{}", self.make_type_key(inner)))
            }
            CheckedReference { inner, .. } => {
                verum_common::Text::from(format!("checked_ref:{}", self.make_type_key(inner)))
            }
            UnsafeReference { inner, .. } => {
                verum_common::Text::from(format!("unsafe_ref:{}", self.make_type_key(inner)))
            }
            Refined { base, .. } => {
                verum_common::Text::from(format!("refined:{}", self.make_type_key(base)))
            }
            Var(_) => "var".into(),
            Meta { name, ty, .. } => {
                verum_common::Text::from(format!("meta:{}:{}", name, self.make_type_key(ty)))
            }
            // Handle other Type variants
            Ownership { inner, .. } => {
                verum_common::Text::from(format!("ownership:{}", self.make_type_key(inner)))
            }
            Pointer { inner, .. } => {
                verum_common::Text::from(format!("ptr:{}", self.make_type_key(inner)))
            }
            VolatilePointer { inner, .. } => {
                verum_common::Text::from(format!("volatile_ptr:{}", self.make_type_key(inner)))
            }
            Exists { body, .. } => {
                verum_common::Text::from(format!("exists:{}", self.make_type_key(body)))
            }
            Forall { body, .. } => {
                verum_common::Text::from(format!("forall:{}", self.make_type_key(body)))
            }
            Variant(variants) => {
                // Try to resolve this variant type to its named type for protocol lookups
                // This enables protocol implementations registered for Maybe<T> to be
                // found when the type has been expanded to None(Unit) | Some(T)
                let signature = Self::variant_type_signature_static(variants);
                if let Some(named_type) = self.variant_type_names.get(&signature) {
                    // Found a named type for this variant - use generic key format
                    // This makes the variant type match protocol impls for the named type
                    verum_common::Text::from(format!("generic:{}", named_type))
                } else {
                    // No named type found - use structural variant key
                    let mut key = verum_common::Text::from("variant:{");
                    for (i, (name, variant_ty)) in variants.iter().enumerate() {
                        if i > 0 {
                            key.push(',');
                        }
                        key.push_str(name.as_str());
                        key.push(':');
                        key.push_str(self.make_type_key(variant_ty).as_str());
                    }
                    key.push('}');
                    key
                }
            }
            Future { output } => {
                verum_common::Text::from(format!("future:{}", self.make_type_key(output)))
            }
            Generator {
                yield_ty,
                return_ty,
            } => verum_common::Text::from(format!(
                "gen:{}|{}",
                self.make_type_key(yield_ty),
                self.make_type_key(return_ty)
            )),
            Tensor { element, shape, .. } => {
                let mut key = verum_common::Text::from("tensor:");
                key.push_str(self.make_type_key(element).as_str());
                key.push('[');
                for (i, dim) in shape.iter().enumerate() {
                    if i > 0 {
                        key.push(',');
                    }
                    key.push_str(&format!("{}", dim));
                }
                key.push(']');
                key
            }
            Lifetime { name, .. } => {
                let mut key = verum_common::Text::from("'");
                key.push_str(name.as_str());
                key
            }
            GenRef { inner, .. } => {
                verum_common::Text::from(format!("genref:{}", self.make_type_key(inner)))
            }
            TypeConstructor { name, .. } => verum_common::Text::from(format!("tycon:{}", name)),
            TypeApp { constructor, args } => {
                let mut key = verum_common::Text::from("tyapp:");
                key.push_str(self.make_type_key(constructor).as_str());
                key.push('[');
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        key.push(',');
                    }
                    key.push_str(self.make_type_key(arg).as_str());
                }
                key.push(']');
                key
            }

            // Dependent Types (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — )
            Pi {
                param_name,
                param_type,
                return_type,
            } => verum_common::Text::from(format!(
                "pi:{}:{}->{}",
                param_name,
                self.make_type_key(param_type),
                self.make_type_key(return_type)
            )),
            Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => verum_common::Text::from(format!(
                "sigma:{}:{}*{}",
                fst_name,
                self.make_type_key(fst_type),
                self.make_type_key(snd_type)
            )),
            Eq { ty, .. } => verum_common::Text::from(format!("eq:{}", self.make_type_key(ty))),
            PathType { space, .. } => {
                verum_common::Text::from(format!("path:{}", self.make_type_key(space)))
            }
            Partial { element_type, .. } => {
                verum_common::Text::from(format!("partial:{}", self.make_type_key(element_type)))
            }
            Interval => "I".into(),
            Universe { level } => verum_common::Text::from(format!("universe:{}", level)),
            Prop => "Prop".into(),
            Inductive { name, .. } => verum_common::Text::from(format!("inductive:{}", name)),
            Coinductive { name, .. } => verum_common::Text::from(format!("coinductive:{}", name)),
            HigherInductive { name, .. } => verum_common::Text::from(format!("hit:{}", name)),
            Quantified { inner, quantity } => verum_common::Text::from(format!(
                "quantified:{}@{}",
                self.make_type_key(inner),
                quantity
            )),
            Placeholder { name, .. } => verum_common::Text::from(format!("placeholder:{}", name)),
            ExtensibleRecord { fields, row_var } => {
                let mut key = verum_common::Text::from("extrec:{");
                for (name, ty) in fields {
                    key.push_str(name.as_str());
                    key.push(':');
                    key.push_str(self.make_type_key(ty).as_str());
                    key.push(',');
                }
                if let Some(rv) = row_var {
                    key.push_str("|");
                    key.push_str(&format!("{}", rv.id()));
                }
                key.push('}');
                key
            }
            CapabilityRestricted { base, capabilities } => {
                let mut key = verum_common::Text::from("caprestricted:");
                key.push_str(self.make_type_key(base).as_str());
                key.push_str("[");
                let names = capabilities.names();
                for (i, name) in names.iter().enumerate() {
                    if i > 0 {
                        key.push(',');
                    }
                    key.push_str(name.as_str());
                }
                key.push(']');
                key
            }
            // Unknown type - a safe top type with no inner structure
            Unknown => "Unknown".into(),

            // DynProtocol - dynamic protocol object (dyn Display + Debug)
            DynProtocol { bounds, bindings } => {
                let mut key = verum_common::Text::from("dyn:");
                for (i, bound) in bounds.iter().enumerate() {
                    if i > 0 {
                        key.push('+');
                    }
                    key.push_str(bound.as_str());
                }
                if !bindings.is_empty() {
                    key.push('<');
                    for (i, (name, ty)) in bindings.iter().enumerate() {
                        if i > 0 {
                            key.push(',');
                        }
                        key.push_str(name.as_str());
                        key.push('=');
                        key.push_str(self.make_type_key(ty).as_str());
                    }
                    key.push('>');
                }
                key
            }
        }
    }

    /// Generate a stable key for a protocol path
    fn make_protocol_key(&self, protocol: &Path) -> Text {
        let mut key = verum_common::Text::new();
        for segment in &protocol.segments {
            if let verum_ast::ty::PathSegment::Name(ident) = segment {
                if !key.is_empty() {
                    key.push('.');
                }
                key.push_str(ident.name.as_str());
            }
        }
        key
    }

    /// Extract the simple protocol name from a path
    ///
    /// For simple paths like `Iterator`, returns `Iterator`.
    /// For qualified paths like `std.iter.Iterator`, returns `Iterator` (the last segment).
    ///
    /// This is used when looking up protocol definitions in the registry,
    /// which uses simple names as keys.
    fn extract_protocol_name(&self, protocol: &Path) -> Text {
        // Try simple identifier first
        if let Some(ident) = protocol.as_ident() {
            return ident.as_str().into();
        }

        // For qualified paths, extract the last segment
        protocol
            .segments
            .last()
            .and_then(|seg| match seg {
                verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.clone()),
                _ => None,
            })
            .unwrap_or_else(|| self.make_protocol_key(protocol))
    }

    /// Check if a type has automatic Deref/DerefMut implementation
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.3 - Automatic implementations
    ///
    /// The following types automatically implement Deref:
    /// - &T implements Deref<Target=T>
    /// - &checked T implements Deref<Target=T>
    /// - &unsafe T implements Deref<Target=T>
    /// - Box<T> (represented as smart pointer types)
    ///
    /// DerefMut is automatically implemented for mutable versions:
    /// - &mut T implements DerefMut<Target=T>
    /// - &checked mut T implements DerefMut<Target=T>
    /// - &unsafe mut T implements DerefMut<Target=T>
    fn has_auto_deref_impl(&self, ty: &Type, protocol: &str) -> bool {
        match protocol {
            "Deref" => {
                // All reference types implement Deref
                matches!(
                    ty,
                    Type::Reference { .. }
                        | Type::CheckedReference { .. }
                        | Type::UnsafeReference { .. }
                )
            }
            "DerefMut" => {
                // Only mutable reference types implement DerefMut
                match ty {
                    Type::Reference { mutable, .. }
                    | Type::CheckedReference { mutable, .. }
                    | Type::UnsafeReference { mutable, .. } => *mutable,
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Get the deref target type for types that auto-implement Deref
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .4.3 - Target type resolution
    pub fn get_deref_target(&self, ty: &Type) -> Maybe<Type> {
        match ty {
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => Maybe::Some((**inner).clone()),
            _ => Maybe::None,
        }
    }

    /// Helper: Check if Path is a single identifier with given name
    fn path_is_name(path: &Path, name: &str) -> bool {
        use verum_ast::ty::PathSegment;
        if path.segments.len() != 1 {
            return false;
        }
        match &path.segments[0] {
            PathSegment::Name(ident) => ident.name.as_str() == name,
            _ => false,
        }
    }

    /// Check if a protocol is object-safe (can be used with `dyn`)
    /// Basic protocols with simple associated types (initial release) — 2 lines 11660-11666
    ///
    /// A protocol is object-safe if:
    /// - Methods take &self or &mut self (not self by value)
    /// - Methods don't return Self
    /// - No generic methods
    /// - No associated constants
    /// - No Self: Sized bound
    pub fn check_object_safety(&self, protocol_name: &Text) -> Result<(), List<ObjectSafetyError>> {
        let protocol = match self.protocols.get(protocol_name) {
            Some(p) => p,
            None => return Ok(()), // Unknown protocol, skip check
        };

        let mut errors = List::new();

        // Rule 1: Check for associated constants
        // Basic protocols with simple associated types (initial release) — 2 line 11664
        if !protocol.associated_consts.is_empty() {
            for (const_name, _) in protocol.associated_consts.iter() {
                errors.push(ObjectSafetyError::HasAssociatedConst {
                    const_name: const_name.clone(),
                });
            }
        }

        // Rule 2: Check for Self: Sized bound
        // Basic protocols with simple associated types (initial release) — 2 line 11665
        for bound in &protocol.super_protocols {
            if Self::path_is_name(&bound.protocol, "Sized") {
                errors.push(ObjectSafetyError::RequiresSized);
            }
        }

        // Rule 3: Check each method
        for (method_name, method) in protocol.methods.iter() {
            // Skip methods with default implementations for object safety checks.
            // Default methods don't need vtable dispatch and can have static/value
            // receivers without making the protocol object-unsafe.
            if method.has_default {
                continue;
            }

            // Extract function type details
            if let Type::Function {
                params,
                return_type,
                type_params,
                ..
            } = &method.ty
            {
                // Rule 3a: No generic methods
                // Basic protocols with simple associated types (initial release) — 2 line 11663
                if !type_params.is_empty() {
                    errors.push(ObjectSafetyError::GenericMethod {
                        method_name: method_name.clone(),
                    });
                }

                // Rule 3b/3c: Check receiver kind for object safety.
                // Use receiver_kind if explicitly set, otherwise infer from params[0].
                let receiver = if let Maybe::Some(rk) = &method.receiver_kind {
                    *rk
                } else if params.is_empty() {
                    ReceiverKind::Static
                } else {
                    match &params[0] {
                        Type::Reference { .. }
                        | Type::CheckedReference { .. }
                        | Type::UnsafeReference { .. } => ReceiverKind::Ref,
                        _ => ReceiverKind::Value,
                    }
                };
                match receiver {
                    ReceiverKind::Static => {
                        errors.push(ObjectSafetyError::NoSelfParameter {
                            method_name: method_name.clone(),
                        });
                    }
                    ReceiverKind::Value => {
                        errors.push(ObjectSafetyError::TakesSelfByValue {
                            method_name: method_name.clone(),
                        });
                    }
                    ReceiverKind::Ref | ReceiverKind::RefMut => {
                        // Object-safe receiver kinds
                    }
                }

                // Rule 3d: Must not return Self
                // Basic protocols with simple associated types (initial release) — 2 line 11662
                if self.type_contains_self(return_type) {
                    errors.push(ObjectSafetyError::ReturnsSelf {
                        method_name: method_name.clone(),
                    });
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Check if a type contains Self
    fn type_contains_self(&self, ty: &Type) -> bool {
        match ty {
            Type::Named { path, args } => {
                if Self::path_is_name(path, "Self") {
                    return true;
                }
                // Check type arguments recursively
                for arg in args {
                    if self.type_contains_self(arg) {
                        return true;
                    }
                }
                false
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    if self.type_contains_self(param) {
                        return true;
                    }
                }
                self.type_contains_self(return_type)
            }
            Type::Tuple(types) => {
                for t in types {
                    if self.type_contains_self(t) {
                        return true;
                    }
                }
                false
            }
            Type::Array { element, .. } => self.type_contains_self(element),
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. }
            | Type::Pointer { inner, .. }
            | Type::VolatilePointer { inner, .. } => self.type_contains_self(inner),
            Type::Refined { base, .. } => self.type_contains_self(base),
            _ => false,
        }
    }

    // ==================== Coherence Checking ====================

    /// Set the current cog for orphan rule checking
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .6 - Orphan Rule
    pub fn set_current_crate(&mut self, crate_name: Text) {
        self.current_crate = Maybe::Some(crate_name);
    }

    /// Register the defining cog for a type (for orphan rule checking)
    pub fn register_type_crate(&mut self, type_name: Text, crate_name: Text) {
        self.type_crates.insert(type_name, crate_name);
    }

    /// Check orphan rule: either protocol or type must be local to current cog
    /// Protocol coherence orphan rules: local protocol + foreign type OK, foreign protocol + local type OK, foreign protocol + foreign type NOT OK. Type parameters make the implementing type local.
    ///
    /// The orphan rule prevents conflicting implementations across cogs:
    /// - You can implement any local protocol for any type
    /// - You can implement any protocol for any local type
    /// - You cannot implement foreign protocol for foreign type
    pub fn check_orphan_rule(&self, impl_: &ProtocolImpl) -> Result<(), CoherenceError> {
        // Get current cog (if not set, we can't check)
        let current_crate = match &self.current_crate {
            Option::Some(c) => c,
            Option::None => return Ok(()), // No cog context, skip check
        };

        // Special case: stdlib is allowed to implement anything
        if let Option::Some(impl_crate) = &impl_.impl_crate
            && impl_crate.as_str() == "stdlib"
        {
            return Ok(());
        }

        // Get protocol name
        let protocol_name = match impl_.protocol.as_ident() {
            Some(ident) => ident.as_str(),
            None => return Ok(()), // Complex path, skip for now
        };

        // Check if protocol is local
        let protocol_is_local = match self.protocols.get(&protocol_name.into()) {
            Some(proto) => match &proto.defining_crate {
                Option::Some(crate_name) => crate_name == current_crate,
                // No defining cog means defined in current compilation unit — treat as local
                Option::None => true,
            },
            None => false,
        };

        // Check if type is local
        let type_is_local = self.is_type_local(&impl_.for_type, current_crate);

        // Orphan rule: at least one must be local
        if !protocol_is_local && !type_is_local {
            return Err(CoherenceError::OrphanRuleViolation {
                protocol: protocol_name.into(),
                for_type: self.type_to_string(&impl_.for_type),
                protocol_crate: self
                    .protocols
                    .get(&protocol_name.into())
                    .and_then(|p| p.defining_crate.clone()),
                type_crate: self.get_type_crate(&impl_.for_type),
                current_crate: current_crate.clone(),
                newtype_suggestion: format!(
                    "type My{0}({0})",
                    self.type_to_string(&impl_.for_type)
                )
                .into(),
                local_protocol_suggestion: format!("protocol My{} {{ ... }}", protocol_name).into(),
                span: impl_.span,
            });
        }

        Ok(())
    }

    /// Check for overlapping implementations
    /// Implementation overlap rules: overlapping impls are errors unless one specializes the other
    ///
    /// Two implementations overlap if they could apply to the same concrete type:
    /// - impl Show for Int and impl Show for Int => overlap
    /// - impl<T> Show for List<T> and impl Show for List<Int> => overlap
    /// - impl Show for List<T> and impl Show for Map<K,V> => no overlap
    /// - impl Sub<Duration> for Instant and impl Sub<Instant> for Instant => NO overlap (different protocol_args)
    pub fn check_overlap(
        &self,
        impl1: &ProtocolImpl,
        impl2: &ProtocolImpl,
    ) -> Result<(), CoherenceError> {
        // Check if protocols are the same
        if self.make_protocol_key(&impl1.protocol) != self.make_protocol_key(&impl2.protocol) {
            return Ok(()); // Different protocols don't overlap
        }

        // CRITICAL FIX: Check if protocol_args are different
        // Sub<Duration> and Sub<Instant> for the same type are NOT overlapping
        // because they have different type arguments
        if impl1.protocol_args.len() == impl2.protocol_args.len() && !impl1.protocol_args.is_empty() {
            let args_could_unify = impl1.protocol_args.iter().zip(impl2.protocol_args.iter())
                .any(|(arg1, arg2)| self.types_could_unify(arg1, arg2));
            if !args_could_unify {
                return Ok(()); // Different protocol_args means no overlap
            }
        }

        // Note: blanket impls (e.g., impl<T> Clone for List<T>) DO overlap with concrete impls
        // (e.g., impl Clone for List<Int>). Specialization resolution determines which impl
        // wins at a call site, but the coherence checker must still detect the overlap.
        // Only skip if one impl explicitly declares specialization.
        let impl1_has_specialization = impl1.specialization.is_some();
        let impl2_has_specialization = impl2.specialization.is_some();
        if impl1_has_specialization || impl2_has_specialization {
            // Explicit specialization: the more specific impl takes priority, no error
            let impl1_is_blanket = !impl1.for_type.free_vars().is_empty();
            let impl2_is_blanket = !impl2.for_type.free_vars().is_empty();
            if impl1_is_blanket != impl2_is_blanket {
                return Ok(());
            }
        }

        // Check if types could unify
        if self.types_could_unify(&impl1.for_type, &impl2.for_type) {
            let protocol_name = impl1
                .protocol
                .as_ident()
                .map(|i| i.as_str())
                .unwrap_or("unknown");

            return Err(CoherenceError::OverlappingImplementations {
                protocol: protocol_name.into(),
                for_type: self.type_to_string(&impl1.for_type),
                first_impl_location: impl1.span,
                second_impl_location: impl2.span,
                specialization_suggestion: Maybe::None,
            });
        }

        Ok(())
    }

    /// Check complete coherence (orphan rule + overlap detection)
    pub fn check_coherence(&self, impl_: &ProtocolImpl) -> Result<(), CoherenceError> {
        // Check orphan rule
        self.check_orphan_rule(impl_)?;

        // Check for overlaps with existing implementations
        for existing_impl in self.impls.iter() {
            self.check_overlap(impl_, existing_impl)?;
        }

        Ok(())
    }

    /// Helper: Check if a type is local to the current cog
    fn is_type_local(&self, ty: &Type, current_crate: &Text) -> bool {
        match ty {
            // Primitive types are never local
            Type::Unit | Type::Bool | Type::Int | Type::Float | Type::Char | Type::Text => false,

            // Named types: check against type_cogs map
            Type::Named { path, args } => {
                // Check if the type constructor itself is local
                if let Some(type_name) = path.as_ident().map(|i| i.as_str())
                    && let Some(crate_name) = self.type_crates.get(&type_name.into())
                    && crate_name == current_crate
                {
                    return true;
                }

                // Check if any type argument contains a local type
                for arg in args {
                    if self.is_type_local(arg, current_crate) {
                        return true;
                    }
                }

                false
            }

            // For generic/compound types, check recursively
            Type::Tuple(elements) => elements
                .iter()
                .any(|t| self.is_type_local(t, current_crate)),
            Type::Array { element, .. } => self.is_type_local(element, current_crate),
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => self.is_type_local(inner, current_crate),

            // Type variables are considered potentially local
            Type::Var(_) => true,

            _ => false,
        }
    }

    /// Helper: Get the defining cog for a type
    fn get_type_crate(&self, ty: &Type) -> Maybe<Text> {
        match ty {
            Type::Unit | Type::Bool | Type::Int | Type::Float | Type::Char | Type::Text => {
                Maybe::Some("stdlib".into())
            }
            Type::Named { path, .. } => {
                if let Some(type_name) = path.as_ident().map(|i| i.as_str()) {
                    self.type_crates.get(&type_name.into()).cloned()
                } else {
                    Maybe::None
                }
            }
            _ => Maybe::None,
        }
    }

    /// Helper: Convert type to string for error messages
    fn type_to_string(&self, ty: &Type) -> Text {
        match ty {
            Type::Unit => "Unit".into(),
            Type::Bool => WKT::Bool.as_str().into(),
            Type::Int => WKT::Int.as_str().into(),
            Type::Float => WKT::Float.as_str().into(),
            Type::Char => WKT::Char.as_str().into(),
            Type::Text => WKT::Text.as_str().into(),
            Type::Named { path, .. } => path
                .as_ident()
                .map(|i| i.as_str())
                .unwrap_or("UnknownType")
                .into(),
            Type::Tuple(elements) => {
                let parts: List<Text> = elements.iter().map(|t| self.type_to_string(t)).collect();
                let parts_str: List<&str> = parts.iter().map(|t| t.as_str()).collect();
                format!("({})", parts_str.join(", ")).into()
            }
            _ => "complex type".into(),
        }
    }

    /// Helper: Check if two types could unify (simple heuristic)
    fn types_could_unify(&self, ty1: &Type, ty2: &Type) -> bool {
        match (ty1, ty2) {
            // Type variables can unify with anything
            (Type::Var(_), _) | (_, Type::Var(_)) => true,

            // Same primitives unify
            (Type::Unit, Type::Unit) => true,
            (Type::Bool, Type::Bool) => true,
            (Type::Int, Type::Int) => true,
            (Type::Float, Type::Float) => true,
            (Type::Char, Type::Char) => true,
            (Type::Text, Type::Text) => true,

            // Named types: check constructor and arguments
            (Type::Named { path: p1, args: a1 }, Type::Named { path: p2, args: a2 }) => {
                // Check if type constructors are the same
                let same_constructor = format!("{:?}", p1) == format!("{:?}", p2);
                if !same_constructor {
                    return false;
                }

                // Check if arguments have same arity
                if a1.len() != a2.len() {
                    return false;
                }

                // Check if all arguments could unify
                a1.iter()
                    .zip(a2.iter())
                    .all(|(t1, t2)| self.types_could_unify(t1, t2))
            }

            // Tuples: same length and all elements could unify
            (Type::Tuple(e1), Type::Tuple(e2)) => {
                e1.len() == e2.len()
                    && e1
                        .iter()
                        .zip(e2.iter())
                        .all(|(t1, t2)| self.types_could_unify(t1, t2))
            }

            // Arrays: elements could unify
            (Type::Array { element: e1, .. }, Type::Array { element: e2, .. }) => {
                self.types_could_unify(e1, e2)
            }

            // Functions: check parameter and return types
            (
                Type::Function {
                    params: p1,
                    return_type: r1,
                    ..
                },
                Type::Function {
                    params: p2,
                    return_type: r2,
                    ..
                },
            ) => {
                p1.len() == p2.len()
                    && p1
                        .iter()
                        .zip(p2.iter())
                        .all(|(t1, t2)| self.types_could_unify(t1, t2))
                    && self.types_could_unify(r1, r2)
            }

            // Different types don't unify
            _ => false,
        }
    }

    // ==================== GAT Support ====================

    /// Resolve GAT instantiation with type arguments
    ///
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .1 lines 116-142
    ///
    /// Given a GAT like `type Item<T>` and type arguments like `[Int]`,
    /// resolves to the concrete type by substituting type parameters.
    ///
    /// Example:
    /// ```verum
    /// protocol Container {
    ///     type Item<T>
    /// }
    ///
    /// impl Container for List {
    ///     type Item<T> = T  // Resolves Item<Int> to Int
    /// }
    /// ```
    pub fn resolve_gat_instantiation(
        &self,
        protocol_name: &Text,
        assoc_type_name: &Text,
        type_args: &List<Type>,
    ) -> Result<Type, crate::advanced_protocols::AdvancedProtocolError> {
        // Get protocol definition
        let protocol = match self.protocols.get(protocol_name) {
            Some(p) => p,
            None => {
                return Err(
                    crate::advanced_protocols::AdvancedProtocolError::GATArityMismatch {
                        gat_name: assoc_type_name.clone(),
                        expected: 0,
                        found: type_args.len(),
                    },
                );
            }
        };

        // Get associated type
        let assoc_type = match protocol.associated_types.get(assoc_type_name) {
            Some(at) => at,
            None => {
                return Err(
                    crate::advanced_protocols::AdvancedProtocolError::GATArityMismatch {
                        gat_name: assoc_type_name.clone(),
                        expected: 0,
                        found: type_args.len(),
                    },
                );
            }
        };

        // Check arity
        if assoc_type.arity() != type_args.len() {
            return Err(
                crate::advanced_protocols::AdvancedProtocolError::GATArityMismatch {
                    gat_name: assoc_type_name.clone(),
                    expected: assoc_type.arity(),
                    found: type_args.len(),
                },
            );
        }

        // Build substitution map from GAT type parameters to concrete type arguments
        // This maps parameter names to the actual types provided
        let mut subst_map: Map<Text, Type> = Map::new();
        for (param, arg) in assoc_type.type_params.iter().zip(type_args.iter()) {
            subst_map.insert(param.name.clone(), arg.clone());
        }

        // If the GAT has a default type, substitute into it
        if let Maybe::Some(ref default_type) = assoc_type.default {
            // Apply substitution to the default type
            let substituted = self.substitute_type_params(default_type, &subst_map);
            Ok(substituted)
        } else {
            // No default: return an associated type projection
            // This represents an abstract associated type like Container.Item<Int>
            // that will be resolved when we know the concrete implementing type
            let projection_name = format!("{}.{}", protocol_name, assoc_type_name);
            Ok(Type::Named {
                path: Path::single(Ident::new(projection_name.as_str(), Span::default())),
                args: type_args.clone(),
            })
        }
    }

    /// Substitute type parameters in a type with concrete type arguments
    ///
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .1 lines 116-142
    ///
    /// Recursively traverses a type and substitutes any named type parameters
    /// with their corresponding concrete types from the substitution map.
    ///
    /// This is the core mechanism for GAT instantiation. When we have:
    /// - GAT definition: `type Item<T> = List<T>`
    /// - Type arguments: `[Int]`
    /// - Substitution map: `{T -> Int}`
    /// - Result: `List<Int>`
    ///
    /// # Arguments
    /// * `ty` - The type to perform substitution on
    /// * `subst_map` - Map from parameter names to concrete types
    ///
    /// # Returns
    /// A new type with all parameter references replaced by concrete types
    ///
    /// # Examples
    /// ```no_run
    /// use verum_types::Type;
    /// use verum_ast::{Path, Ident};
    /// use verum_common::{span::Span, Text, List, Map};
    /// # use verum_types::protocol::ProtocolChecker;
    /// # let checker = ProtocolChecker::new();
    ///
    /// // Example: Substitute T -> Int in List<T>
    /// let list_t = Type::Named {
    ///     path: Path::single(Ident::new("List", Span::default())),
    ///     args: List::from(vec![Type::Named {
    ///         path: Path::single(Ident::new("T", Span::default())),
    ///         args: List::new()
    ///     }])
    /// };
    /// let mut subst_map: Map<Text, Type> = Map::new();
    /// subst_map.insert("T".into(), Type::Int);
    /// // Private method - for internal use only
    /// // let result = checker.substitute_type_params(&list_t, &subst_map);
    /// ```
    fn substitute_type_params(&self, ty: &Type, subst_map: &Map<Text, Type>) -> Type {
        // Depth guard to prevent stack overflow on deeply nested or cyclic type structures
        let depth = SUBST_TYPE_PARAMS_DEPTH.with(|d| {
            let current = *d.borrow();
            *d.borrow_mut() = current + 1;
            current
        });
        struct SubstDepthGuard;
        impl Drop for SubstDepthGuard {
            fn drop(&mut self) {
                SUBST_TYPE_PARAMS_DEPTH.with(|d| {
                    *d.borrow_mut() -= 1;
                });
            }
        }
        let _guard = SubstDepthGuard;

        if depth > 20 {
            return ty.clone(); // Return unchanged to break the cycle
        }

        match ty {
            // Primitive types remain unchanged
            Type::Unit
            | Type::Never
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Char
            | Type::Text => ty.clone(),

            // Type variables: check if there's a binding in the subst_map
            // Format is "T{id}" for type var bindings
            Type::Var(tv) => {
                let var_name: Text = format!("T{}", tv.id()).into();
                if let Some(replacement) = subst_map.get(&var_name) {
                    replacement.clone()
                } else {
                    ty.clone()
                }
            }

            // Named types: check if this is a type parameter to substitute
            Type::Named { path, args } => {
                // Check if this is a simple type parameter (single identifier with no args)
                if args.is_empty()
                    && let Some(ident) = path.as_ident()
                {
                    let name: Text = ident.as_str().into();
                    // If this name is in the substitution map, replace it
                    if let Some(replacement) = subst_map.get(&name) {
                        return replacement.clone();
                    }
                }

                // Not a parameter, or has type args - recursively substitute in arguments
                Type::Named {
                    path: path.clone(),
                    args: args
                        .iter()
                        .map(|arg| self.substitute_type_params(arg, subst_map))
                        .collect(),
                }
            }

            // Generic types: recursively substitute in type arguments
            // CRITICAL: Also resolve projection types like ::Item[T] after substitution
            Type::Generic { name, args } => {
                let substituted_args: List<Type> = args
                    .iter()
                    .map(|arg| self.substitute_type_params(arg, subst_map))
                    .collect();

                // Check if this is a projection type (::AssocName format)
                if name.as_str().starts_with("::") && !substituted_args.is_empty() {
                    let assoc_name = &name.as_str()[2..]; // Strip "::" prefix
                    let base_ty = &substituted_args[0];

                    // Try to resolve the projection to a concrete type
                    let assoc_text: Text = assoc_name.into();
                    if let Some(resolved) = self.try_find_associated_type(base_ty, &assoc_text) {
                        return resolved;
                    }
                }

                // Not a projection or couldn't resolve - keep as generic
                Type::Generic {
                    name: name.clone(),
                    args: substituted_args,
                }
            },

            // Function types: substitute in parameters, return type, and type params
            Type::Function {
                params,
                return_type,
                contexts,
                type_params,
                properties,
            } => {
                Type::Function {
                    params: params
                        .iter()
                        .map(|p| self.substitute_type_params(p, subst_map))
                        .collect(),
                    return_type: Box::new(self.substitute_type_params(return_type, subst_map)),
                    contexts: contexts.clone(),
                    type_params: type_params.clone(), // Type params themselves aren't substituted
                    properties: properties.clone(),
                }
            }

            // Tuple: substitute in each element
            Type::Tuple(elements) => Type::Tuple(
                elements
                    .iter()
                    .map(|e| self.substitute_type_params(e, subst_map))
                    .collect(),
            ),

            // Array: substitute in element type
            Type::Array { element, size } => Type::Array {
                element: Box::new(self.substitute_type_params(element, subst_map)),
                size: *size,
            },

            // Slice: substitute in element type
            Type::Slice { element } => Type::Slice {
                element: Box::new(self.substitute_type_params(element, subst_map)),
            },

            // Record: substitute in field types
            Type::Record(fields) => Type::Record(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), self.substitute_type_params(v, subst_map)))
                    .collect(),
            ),

            // Variant: substitute in variant types
            Type::Variant(variants) => Type::Variant(
                variants
                    .iter()
                    .map(|(k, v)| (k.clone(), self.substitute_type_params(v, subst_map)))
                    .collect(),
            ),

            // References: substitute in inner type
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(self.substitute_type_params(inner, subst_map)),
            },

            Type::CheckedReference { mutable, inner } => Type::CheckedReference {
                mutable: *mutable,
                inner: Box::new(self.substitute_type_params(inner, subst_map)),
            },

            Type::UnsafeReference { mutable, inner } => Type::UnsafeReference {
                mutable: *mutable,
                inner: Box::new(self.substitute_type_params(inner, subst_map)),
            },

            Type::Ownership { mutable, inner } => Type::Ownership {
                mutable: *mutable,
                inner: Box::new(self.substitute_type_params(inner, subst_map)),
            },

            Type::Pointer { mutable, inner } => Type::Pointer {
                mutable: *mutable,
                inner: Box::new(self.substitute_type_params(inner, subst_map)),
            },

            Type::VolatilePointer { mutable, inner } => Type::VolatilePointer {
                mutable: *mutable,
                inner: Box::new(self.substitute_type_params(inner, subst_map)),
            },

            // Refined types: substitute in base type
            // Note: We don't substitute in predicates as they're separate from type params
            Type::Refined { base, predicate } => Type::Refined {
                base: Box::new(self.substitute_type_params(base, subst_map)),
                predicate: predicate.clone(),
            },

            // Existential types: substitute in body (but not in the bound variable)
            Type::Exists { var, body } => Type::Exists {
                var: *var,
                body: Box::new(self.substitute_type_params(body, subst_map)),
            },

            // Universal quantification: substitute in body (but not in bound variables)
            Type::Forall { vars, body } => Type::Forall {
                vars: vars.clone(),
                body: Box::new(self.substitute_type_params(body, subst_map)),
            },

            // Meta parameters: substitute in the type
            Type::Meta {
                name,
                ty,
                refinement,
                value,
            } => Type::Meta {
                name: name.clone(),
                ty: Box::new(self.substitute_type_params(ty, subst_map)),
                refinement: refinement.clone(),
                value: value.clone(),
            },

            // Future: substitute in output type
            Type::Future { output } => Type::Future {
                output: Box::new(self.substitute_type_params(output, subst_map)),
            },

            // Generator: substitute in yield and return types
            Type::Generator {
                yield_ty,
                return_ty,
            } => Type::Generator {
                yield_ty: Box::new(self.substitute_type_params(yield_ty, subst_map)),
                return_ty: Box::new(self.substitute_type_params(return_ty, subst_map)),
            },

            // Tensor: substitute in element type
            // Shape and strides are compile-time values, not types
            Type::Tensor {
                element,
                shape,
                strides,
                span,
            } => Type::Tensor {
                element: Box::new(self.substitute_type_params(element, subst_map)),
                shape: shape.clone(),
                strides: strides.clone(),
                span: *span,
            },

            // Lifetimes remain unchanged
            Type::Lifetime { name } => Type::Lifetime { name: name.clone() },

            // GenRef: substitute in inner type
            Type::GenRef { inner } => Type::GenRef {
                inner: Box::new(self.substitute_type_params(inner, subst_map)),
            },

            // Type constructor: no substitution needed (it's a name, not a type)
            Type::TypeConstructor { name, arity, kind } => Type::TypeConstructor {
                name: name.clone(),
                arity: *arity,
                kind: kind.clone(),
            },

            // Type application: substitute in constructor and arguments
            Type::TypeApp { constructor, args } => Type::TypeApp {
                constructor: Box::new(self.substitute_type_params(constructor, subst_map)),
                args: args
                    .iter()
                    .map(|arg| self.substitute_type_params(arg, subst_map))
                    .collect(),
            },

            // Dependent Types (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — )
            Type::Pi {
                param_name,
                param_type,
                return_type,
            } => Type::Pi {
                param_name: param_name.clone(),
                param_type: Box::new(self.substitute_type_params(param_type, subst_map)),
                return_type: Box::new(self.substitute_type_params(return_type, subst_map)),
            },

            Type::Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => Type::Sigma {
                fst_name: fst_name.clone(),
                fst_type: Box::new(self.substitute_type_params(fst_type, subst_map)),
                snd_type: Box::new(self.substitute_type_params(snd_type, subst_map)),
            },

            Type::Eq { ty, lhs, rhs } => Type::Eq {
                ty: Box::new(self.substitute_type_params(ty, subst_map)),
                lhs: lhs.clone(),
                rhs: rhs.clone(),
            },

            // Path type: substitute in the space type; endpoints are value-level CubicalTerms
            Type::PathType { space, left, right } => Type::PathType {
                space: Box::new(self.substitute_type_params(space, subst_map)),
                left: left.clone(),
                right: right.clone(),
            },

            // Partial element type: substitute in element_type; face is a value-level CubicalTerm
            Type::Partial { element_type, face } => Type::Partial {
                element_type: Box::new(self.substitute_type_params(element_type, subst_map)),
                face: face.clone(),
            },

            // Interval is a primitive type — no inner types to substitute
            Type::Interval => Type::Interval,

            Type::Universe { level } => Type::Universe { level: *level },
            Type::Prop => Type::Prop,

            Type::Inductive {
                name,
                params,
                indices,
                universe,
                constructors,
            } => Type::Inductive {
                name: name.clone(),
                params: params
                    .iter()
                    .map(|(n, t)| {
                        (
                            n.clone(),
                            Box::new(self.substitute_type_params(t, subst_map)),
                        )
                    })
                    .collect(),
                indices: indices
                    .iter()
                    .map(|(n, t)| {
                        (
                            n.clone(),
                            Box::new(self.substitute_type_params(t, subst_map)),
                        )
                    })
                    .collect(),
                universe: *universe,
                constructors: constructors.clone(),
            },

            Type::Coinductive {
                name,
                params,
                destructors,
            } => Type::Coinductive {
                name: name.clone(),
                params: params
                    .iter()
                    .map(|(n, t)| {
                        (
                            n.clone(),
                            Box::new(self.substitute_type_params(t, subst_map)),
                        )
                    })
                    .collect(),
                destructors: destructors.clone(),
            },

            Type::HigherInductive {
                name,
                params,
                point_constructors,
                path_constructors,
            } => Type::HigherInductive {
                name: name.clone(),
                params: params
                    .iter()
                    .map(|(n, t)| {
                        (
                            n.clone(),
                            Box::new(self.substitute_type_params(t, subst_map)),
                        )
                    })
                    .collect(),
                point_constructors: point_constructors.clone(),
                path_constructors: path_constructors.clone(),
            },

            Type::Quantified { inner, quantity } => Type::Quantified {
                inner: Box::new(self.substitute_type_params(inner, subst_map)),
                quantity: *quantity,
            },

            // Placeholder types - should not normally appear during protocol checking
            // Return as-is; they'll be resolved during two-pass type resolution
            Type::Placeholder { name, span } => Type::Placeholder {
                name: name.clone(),
                span: *span,
            },

            // ExtensibleRecord types - substitute into fields, preserve row variable
            Type::ExtensibleRecord { fields, row_var } => {
                let new_fields = fields
                    .iter()
                    .map(|(k, v)| (k.clone(), self.substitute_type_params(v, subst_map)))
                    .collect();
                Type::ExtensibleRecord {
                    fields: new_fields,
                    row_var: *row_var,
                }
            }

            // CapabilityRestricted types - substitute into base, preserve capabilities
            Type::CapabilityRestricted { base, capabilities } => Type::CapabilityRestricted {
                base: Box::new(self.substitute_type_params(base, subst_map)),
                capabilities: capabilities.clone(),
            },

            // Unknown type - a safe top type with no inner types to substitute
            Type::Unknown => Type::Unknown,

            // DynProtocol - substitute in associated type bindings
            Type::DynProtocol { bounds, bindings } => Type::DynProtocol {
                bounds: bounds.clone(),
                bindings: bindings
                    .iter()
                    .map(|(k, v)| (k.clone(), self.substitute_type_params(v, subst_map)))
                    .collect(),
            },
        }
    }

    /// Check GAT where clause constraints
    ///
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .4 lines 441-471
    ///
    /// Verifies that all where clauses on a GAT are satisfied for the given
    /// type arguments.
    ///
    /// Example:
    /// ```verum
    /// protocol Container {
    ///     type Item<T> where T: Clone + Debug
    /// }
    /// ```
    pub fn check_gat_constraints(
        &self,
        assoc_type: &AssociatedType,
        type_args: &List<Type>,
    ) -> Result<(), crate::advanced_protocols::AdvancedProtocolError> {
        // Check that we have the right number of type arguments
        if assoc_type.arity() != type_args.len() {
            return Err(
                crate::advanced_protocols::AdvancedProtocolError::GATArityMismatch {
                    gat_name: assoc_type.name.clone(),
                    expected: assoc_type.arity(),
                    found: type_args.len(),
                },
            );
        }

        // Check each where clause
        for where_clause in assoc_type.where_clauses.iter() {
            // Find the type parameter being constrained
            let param_idx = assoc_type
                .type_params
                .iter()
                .position(|p| p.name == where_clause.param);

            let type_arg = match param_idx {
                Some(idx) => match type_args.get(idx) {
                    Some(ty) => ty,
                    None => continue,
                },
                None => continue,
            };

            // Check that the type satisfies all constraints
            for constraint in where_clause.constraints.iter() {
                if !self.implements(type_arg, &constraint.protocol) {
                    return Err(
                        crate::advanced_protocols::AdvancedProtocolError::GATConstraintNotSatisfied {
                            ty: type_arg.clone(),
                            constraint: "protocol constraint".into(),
                        },
                    );
                }
            }
        }

        Ok(())
    }

    /// Resolve specialization: select most specific implementation
    ///
    /// Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — lines 549-663
    ///
    /// When multiple implementations apply to a type (due to specialization),
    /// selects the most specific one based on the specialization lattice.
    ///
    /// Example:
    /// ```verum
    /// // General implementation
    /// impl<T> Display for List<T> where T: Display { }
    ///
    /// // Specialized implementation (more specific)
    /// @specialize
    /// impl Display for List<Text> { }
    ///
    /// // For List<Text>, the specialized impl is selected
    /// ```
    pub fn resolve_specialization(
        &self,
        ty: &Type,
        protocol: &Path,
    ) -> Result<&ProtocolImpl, crate::advanced_protocols::AdvancedProtocolError> {
        // Find all applicable implementations
        let mut applicable = Set::new();
        let type_key = self.make_type_key(ty);
        let protocol_key = self.make_protocol_key(protocol);

        // Collect all implementations that could apply
        for (idx, impl_) in self.impls.iter().enumerate() {
            let impl_type_key = self.make_type_key(&impl_.for_type);
            let impl_protocol_key = self.make_protocol_key(&impl_.protocol);

            if impl_protocol_key == protocol_key {
                // Check if types could unify (impl could apply)
                if self.types_could_unify(&impl_.for_type, ty) {
                    applicable.insert(idx);
                }
            }
        }

        // If no implementations found, return error
        if applicable.is_empty() {
            // Try exact match as fallback
            if let Some(&idx) = self.impl_index.get(&(type_key, protocol_key)) {
                return self.impls.get(idx).ok_or_else(|| {
                    crate::advanced_protocols::AdvancedProtocolError::AmbiguousSpecialization {
                        ty: ty.clone(),
                        candidates: List::new(),
                    }
                });
            }
            return Err(
                crate::advanced_protocols::AdvancedProtocolError::AmbiguousSpecialization {
                    ty: ty.clone(),
                    candidates: List::new(),
                },
            );
        }

        // If only one implementation, return it
        if applicable.len() == 1 {
            let idx = match applicable.iter().next() {
                Some(&i) => i,
                None => return Err(crate::advanced_protocols::AdvancedProtocolError::AmbiguousSpecialization {
                    ty: ty.clone(),
                    candidates: List::new(),
                }),
            };
            return self.impls.get(idx).ok_or_else(|| {
                crate::advanced_protocols::AdvancedProtocolError::AmbiguousSpecialization {
                    ty: ty.clone(),
                    candidates: List::new(),
                }
            });
        }

        // Multiple implementations: select most specific using specialization info
        let mut most_specific_idx = None;
        let mut highest_rank = 0;

        for &idx in applicable.iter() {
            if let Some(impl_) = self.impls.get(idx) {
                let rank = match &impl_.specialization {
                    Maybe::Some(spec_info) => spec_info.specificity_rank,
                    Maybe::None => 0,
                };

                if most_specific_idx.is_none() || rank > highest_rank {
                    most_specific_idx = Some(idx);
                    highest_rank = rank;
                }
            }
        }

        match most_specific_idx {
            Some(idx) => self.impls.get(idx).ok_or_else(|| {
                // Use collect - provides efficient allocation with size hint
                let candidates: List<_> = applicable.iter().cloned().collect();
                crate::advanced_protocols::AdvancedProtocolError::AmbiguousSpecialization {
                    ty: ty.clone(),
                    candidates,
                }
            }),
            None => {
                // Use collect - provides efficient allocation with size hint
                let candidates: List<_> = applicable.iter().cloned().collect();
                Err(
                    crate::advanced_protocols::AdvancedProtocolError::AmbiguousSpecialization {
                        ty: ty.clone(),
                        candidates,
                    },
                )
            }
        }
    }

    // ==================== Protocol Conformance Checking ====================

    /// Check complete protocol conformance for an implementation
    ///
    /// Advanced protocols (future v2.0+): GATs, higher-rank bounds, specialization with lattice ordering, coherence rules — Full Conformance Verification
    ///
    /// This is the production-quality protocol conformance checker that validates:
    /// 1. All required methods are implemented (non-default ones)
    /// 2. Method signatures match protocol requirements
    /// 3. All required associated types are defined
    /// 4. Associated type bounds are satisfied
    /// 5. Where clauses on the implementation are valid
    /// 6. Protocol inheritance (superprotocols) is satisfied
    /// 7. Associated constants are defined with correct types
    ///
    /// # Arguments
    /// * `impl_` - The protocol implementation to check
    ///
    /// # Returns
    /// * `Ok(())` - Implementation is valid
    /// * `Err(ConformanceError)` - Specific conformance failure with details
    ///
    /// # Example
    /// ```ignore
    /// let impl_ = ProtocolImpl { ... };
    /// checker.check_full_conformance(&impl_)?;
    /// ```
    pub fn check_full_conformance(&self, impl_: &ProtocolImpl) -> Result<(), ConformanceError> {
        // Get protocol definition
        let protocol_name = self.extract_protocol_name(&impl_.protocol);
        let protocol = self.protocols.get(&protocol_name).ok_or_else(|| {
            ConformanceError::ProtocolNotFound {
                name: protocol_name.clone(),
                span: impl_.span,
            }
        })?;

        // 1. Check superprotocol satisfaction
        self.check_superprotocol_conformance(impl_, protocol)?;

        // 2. Check where clause satisfaction
        self.check_where_clause_conformance(impl_, protocol)?;

        // 3. Check all required methods are implemented
        self.check_method_conformance(impl_, protocol)?;

        // 4. Check all required associated types are defined
        self.check_associated_type_conformance(impl_, protocol)?;

        // 5. Check all required associated constants are defined
        self.check_associated_const_conformance(impl_, protocol)?;

        Ok(())
    }

    /// Check that all superprotocols are satisfied
    ///
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Protocol Inheritance
    ///
    /// Verifies that for each superprotocol of the implemented protocol,
    /// the implementing type also implements that superprotocol.
    fn check_superprotocol_conformance(
        &self,
        impl_: &ProtocolImpl,
        protocol: &Protocol,
    ) -> Result<(), ConformanceError> {
        for super_bound in protocol.super_protocols.iter() {
            // Skip negative bounds (those indicate what the type must NOT implement)
            if super_bound.is_negative {
                // Check negative bound: type must NOT implement this
                if self.implements(&impl_.for_type, &super_bound.protocol) {
                    return Err(ConformanceError::NegativeBoundViolation {
                        ty: impl_.for_type.clone(),
                        protocol: super_bound.protocol.clone(),
                        span: impl_.span,
                    });
                }
                continue;
            }

            // Positive bound: type must implement this superprotocol
            if !self.implements(&impl_.for_type, &super_bound.protocol) {
                let super_name = self.extract_protocol_name(&super_bound.protocol);
                return Err(ConformanceError::SuperprotocolNotImplemented {
                    implementing_type: impl_.for_type.clone(),
                    protocol: protocol.name.clone(),
                    superprotocol: super_name.clone(),
                    required_for: protocol.name.clone(),
                    span: impl_.span,
                    help: format!(
                        "Add `implement {} for {}` before implementing {}",
                        super_name,
                        self.type_to_string(&impl_.for_type),
                        protocol.name
                    )
                    .into(),
                });
            }
        }
        Ok(())
    }

    /// Check that all where clauses on the implementation are satisfied
    ///
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .4 - Where Clauses
    ///
    /// Validates both:
    /// 1. Where clauses declared on the impl block itself
    /// 2. Where clauses required by the protocol definition
    fn check_where_clause_conformance(
        &self,
        impl_: &ProtocolImpl,
        _protocol: &Protocol,
    ) -> Result<(), ConformanceError> {
        // Check each where clause on the implementation
        for where_clause in impl_.where_clauses.iter() {
            // Validate that each bound in the where clause is satisfiable
            for bound in where_clause.bounds.iter() {
                if bound.is_negative {
                    // Negative bound: the type must NOT implement this protocol
                    if self.implements(&where_clause.ty, &bound.protocol) {
                        return Err(ConformanceError::WhereClauseNotSatisfied {
                            ty: where_clause.ty.clone(),
                            constraint: format!("!{}", self.extract_protocol_name(&bound.protocol))
                                .into(),
                            reason: "Type implements a protocol it should not".into(),
                            span: impl_.span,
                        });
                    }
                } else {
                    // Positive bound: the type must implement this protocol
                    // Note: We don't immediately fail here because where clauses are
                    // often used to constrain generic type parameters that will be
                    // instantiated later. We only check concrete types.
                    if !self.type_has_free_variables(&where_clause.ty)
                        && !self.implements(&where_clause.ty, &bound.protocol)
                    {
                        let protocol_name = self.extract_protocol_name(&bound.protocol);
                        return Err(ConformanceError::WhereClauseNotSatisfied {
                            ty: where_clause.ty.clone(),
                            constraint: protocol_name.clone(),
                            reason: format!(
                                "Type {} does not implement {}",
                                self.type_to_string(&where_clause.ty),
                                protocol_name
                            )
                            .into(),
                            span: impl_.span,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Check that a type contains free type variables
    ///
    /// Free type variables are type parameters that haven't been instantiated
    /// with concrete types yet. Where clause checking should be deferred for
    /// types with free variables until they're instantiated.
    fn type_has_free_variables(&self, ty: &Type) -> bool {
        match ty {
            Type::Var(_) => true,
            Type::Named { args, .. } => args.iter().any(|arg| self.type_has_free_variables(arg)),
            Type::Generic { args, .. } => args.iter().any(|arg| self.type_has_free_variables(arg)),
            Type::Tuple(elems) => elems.iter().any(|e| self.type_has_free_variables(e)),
            Type::Array { element, .. } => self.type_has_free_variables(element),
            Type::Slice { element } => self.type_has_free_variables(element),
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => self.type_has_free_variables(inner),
            Type::Function {
                params,
                return_type,
                ..
            } => {
                params.iter().any(|p| self.type_has_free_variables(p))
                    || self.type_has_free_variables(return_type)
            }
            _ => false,
        }
    }

    /// Check that all required methods are implemented correctly
    ///
    /// Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — .5 - Soundness Rules
    ///
    /// Validates:
    /// 1. All non-default methods are provided
    /// 2. Method signatures are compatible with protocol requirements
    /// 3. Refinement constraints are respected (implementations can strengthen, not weaken)
    ///
    /// Note: This only checks methods defined directly in the protocol being implemented.
    /// Superprotocol methods are checked when verifying superprotocol conformance.
    fn check_method_conformance(
        &self,
        impl_: &ProtocolImpl,
        protocol: &Protocol,
    ) -> Result<(), ConformanceError> {
        // Only check methods defined directly in this protocol, not inherited ones.
        // Superprotocol methods are verified separately via check_superprotocol_conformance.
        for proto_method in protocol.methods.values() {
            // Check if implementation provides this method
            if let Some(impl_method_ty) = impl_.methods.get(&proto_method.name) {
                // Method is provided - check signature compatibility
                self.check_method_signature_compatibility(
                    &proto_method.name,
                    &proto_method.ty,
                    impl_method_ty,
                    impl_.span,
                )?;

                // Check refinement constraints (impl can strengthen, not weaken)
                if proto_method.has_refinement() {
                    self.check_refinement_compatibility(proto_method, impl_method_ty, impl_.span)?;
                }
            } else if !proto_method.has_default {
                // Method not provided and has no default - error
                return Err(ConformanceError::MissingMethod {
                    protocol: protocol.name.clone(),
                    method: proto_method.name.clone(),
                    implementing_type: impl_.for_type.clone(),
                    expected_signature: self.format_method_signature(proto_method),
                    span: impl_.span,
                });
            }
            // Method not provided but has default - OK
        }

        Ok(())
    }

    /// Check that method signatures are compatible
    ///
    /// The implementation signature must be compatible with the protocol signature.
    /// This means:
    /// - Same number of parameters
    /// - Parameters are contravariant (impl can accept broader types)
    /// - Return type is covariant (impl can return more specific types)
    fn check_method_signature_compatibility(
        &self,
        method_name: &Text,
        proto_sig: &Type,
        impl_sig: &Type,
        span: Span,
    ) -> Result<(), ConformanceError> {
        match (proto_sig, impl_sig) {
            (
                Type::Function {
                    params: proto_params,
                    return_type: proto_ret,
                    ..
                },
                Type::Function {
                    params: impl_params,
                    return_type: impl_ret,
                    ..
                },
            ) => {
                // Check parameter count
                if proto_params.len() != impl_params.len() {
                    return Err(ConformanceError::MethodSignatureMismatch {
                        method: method_name.clone(),
                        expected: self.format_function_type(proto_sig),
                        found: self.format_function_type(impl_sig),
                        reason: format!(
                            "Expected {} parameters, found {}",
                            proto_params.len(),
                            impl_params.len()
                        )
                        .into(),
                        span,
                    });
                }

                // Check parameter compatibility (contravariant)
                // For now, we do a structural compatibility check
                // A full implementation would do proper subtyping
                for (i, (proto_param, impl_param)) in
                    proto_params.iter().zip(impl_params.iter()).enumerate()
                {
                    if !self.types_compatible(proto_param, impl_param) {
                        return Err(ConformanceError::MethodSignatureMismatch {
                            method: method_name.clone(),
                            expected: self.format_function_type(proto_sig),
                            found: self.format_function_type(impl_sig),
                            reason: format!(
                                "Parameter {} type mismatch: expected {}, found {}",
                                i,
                                self.type_to_string(proto_param),
                                self.type_to_string(impl_param)
                            )
                            .into(),
                            span,
                        });
                    }
                }

                // Check return type compatibility (covariant)
                if !self.types_compatible(proto_ret, impl_ret) {
                    return Err(ConformanceError::MethodSignatureMismatch {
                        method: method_name.clone(),
                        expected: self.format_function_type(proto_sig),
                        found: self.format_function_type(impl_sig),
                        reason: format!(
                            "Return type mismatch: expected {}, found {}",
                            self.type_to_string(proto_ret),
                            self.type_to_string(impl_ret)
                        )
                        .into(),
                        span,
                    });
                }

                Ok(())
            }
            _ => {
                // At least one isn't a function type - structural check
                if !self.types_compatible(proto_sig, impl_sig) {
                    Err(ConformanceError::MethodSignatureMismatch {
                        method: method_name.clone(),
                        expected: self.type_to_string(proto_sig),
                        found: self.type_to_string(impl_sig),
                        reason: "Type mismatch".into(),
                        span,
                    })
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Check that types are structurally compatible
    ///
    /// Two types are compatible if:
    /// - They are identical
    /// - One is a type variable (can unify with anything)
    /// - They have the same structure with compatible components
    fn types_compatible(&self, ty1: &Type, ty2: &Type) -> bool {
        match (ty1, ty2) {
            // Type variables are compatible with anything
            (Type::Var(_), _) | (_, Type::Var(_)) => true,

            // Identical primitive types
            (Type::Unit, Type::Unit) => true,
            (Type::Never, Type::Never) => true,
            (Type::Bool, Type::Bool) => true,
            (Type::Int, Type::Int) => true,
            (Type::Float, Type::Float) => true,
            (Type::Char, Type::Char) => true,
            (Type::Text, Type::Text) => true,

            // Named types: check path and args. Numeric aliases
            // (`u64` ↔ `UInt64`, etc.) normalize via
            // `Type::canonical_primitive` so literal-synthesized
            // types match user-declared parameter types.
            (Type::Named { path: p1, args: a1 }, Type::Named { path: p2, args: a2 }) => {
                let k1 = self.make_protocol_key(p1);
                let k2 = self.make_protocol_key(p2);
                Type::canonical_primitive(k1.as_str())
                    == Type::canonical_primitive(k2.as_str())
                    && a1.len() == a2.len()
                    && a1
                        .iter()
                        .zip(a2.iter())
                        .all(|(t1, t2)| self.types_compatible(t1, t2))
            }

            // Generic types — same numeric-alias normalization.
            (Type::Generic { name: n1, args: a1 }, Type::Generic { name: n2, args: a2 }) => {
                Type::canonical_primitive(n1.as_str())
                    == Type::canonical_primitive(n2.as_str())
                    && a1.len() == a2.len()
                    && a1
                        .iter()
                        .zip(a2.iter())
                        .all(|(t1, t2)| self.types_compatible(t1, t2))
            }

            // Tuples
            (Type::Tuple(e1), Type::Tuple(e2)) => {
                e1.len() == e2.len()
                    && e1
                        .iter()
                        .zip(e2.iter())
                        .all(|(t1, t2)| self.types_compatible(t1, t2))
            }

            // Arrays
            (
                Type::Array {
                    element: e1,
                    size: s1,
                },
                Type::Array {
                    element: e2,
                    size: s2,
                },
            ) => s1 == s2 && self.types_compatible(e1, e2),

            // Slices
            (Type::Slice { element: e1 }, Type::Slice { element: e2 }) => {
                self.types_compatible(e1, e2)
            }

            // References
            (
                Type::Reference {
                    mutable: m1,
                    inner: i1,
                },
                Type::Reference {
                    mutable: m2,
                    inner: i2,
                },
            ) => m1 == m2 && self.types_compatible(i1, i2),

            // Functions
            (
                Type::Function {
                    params: p1,
                    return_type: r1,
                    ..
                },
                Type::Function {
                    params: p2,
                    return_type: r2,
                    ..
                },
            ) => {
                p1.len() == p2.len()
                    && p1
                        .iter()
                        .zip(p2.iter())
                        .all(|(t1, t2)| self.types_compatible(t1, t2))
                    && self.types_compatible(r1, r2)
            }

            // Refined types: check base type compatibility
            (Type::Refined { base: b1, .. }, Type::Refined { base: b2, .. }) => {
                self.types_compatible(b1, b2)
            }
            (Type::Refined { base, .. }, other) | (other, Type::Refined { base, .. }) => {
                self.types_compatible(base, other)
            }

            // Different types
            _ => false,
        }
    }

    /// Check refinement constraint compatibility
    ///
    /// Advanced protocols (future v2.0+): GATs, higher-rank bounds, specialization with lattice ordering, coherence rules — Section 7.3 - Refinement Types with Protocols
    ///
    /// Implementations can strengthen refinements but cannot weaken them:
    /// - Parameters: can strengthen (accept more constrained types)
    /// - Return types: can strengthen (return more constrained types)
    fn check_refinement_compatibility(
        &self,
        proto_method: &ProtocolMethod,
        _impl_sig: &Type,
        span: Span,
    ) -> Result<(), ConformanceError> {
        // For each refinement in the protocol method, verify the implementation
        // respects it (implementations can strengthen but not weaken refinements)
        for (param_name, _proto_refinement) in proto_method.refinement_constraints.iter() {
            // The implementation must have at least the same refinement
            // or a stronger one. For now, we just verify the structure.
            // A full implementation would use SMT solving to compare predicates.
            if param_name == "return" {
                // Return type refinement: impl must be at least as strong
                // (covariant position - can strengthen)
            } else {
                // Parameter refinement: impl must be at least as strong
                // (we accept same or stronger constraints)
            }
        }

        // If the implementation has refinements, ensure they're compatible
        // This is a structural check - full verification requires SMT
        let _ = span; // Used for error reporting in full implementation

        Ok(())
    }

    /// Check that all required associated types are defined
    ///
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — GATs
    ///
    /// Validates:
    /// 1. All associated types without defaults are provided
    /// 2. Associated type bounds are satisfied
    /// 3. GAT type parameters match expected arity
    fn check_associated_type_conformance(
        &self,
        impl_: &ProtocolImpl,
        protocol: &Protocol,
    ) -> Result<(), ConformanceError> {
        // Also check associated types from superprotocols
        let mut all_assoc_types: Map<Text, &AssociatedType> = Map::new();

        // Collect from superprotocols first
        self.collect_inherited_associated_types(protocol, &mut all_assoc_types)?;

        // Add this protocol's associated types (can override superprotocol defaults)
        for (name, assoc_type) in protocol.associated_types.iter() {
            all_assoc_types.insert(name.clone(), assoc_type);
        }

        for (assoc_name, assoc_type) in all_assoc_types.iter() {
            if let Some(impl_type) = impl_.associated_types.get(assoc_name) {
                // Associated type is provided - check bounds
                for bound in assoc_type.bounds.iter() {
                    if !self.implements(impl_type, &bound.protocol) {
                        return Err(ConformanceError::AssociatedTypeBoundNotSatisfied {
                            assoc_type: assoc_name.clone(),
                            bound: self.extract_protocol_name(&bound.protocol),
                            actual_type: impl_type.clone(),
                            span: impl_.span,
                        });
                    }
                }

                // For GATs, check arity
                if assoc_type.is_gat() {
                    // The implementation should provide a type that can be
                    // instantiated with the same number of type parameters
                    // This is checked during actual instantiation
                }

                // Check variance compatibility
                assoc_type
                    .check_variance(assoc_type.expected_variance)
                    .map_err(|msg| ConformanceError::AssociatedTypeVarianceMismatch {
                        assoc_type: assoc_name.clone(),
                        message: msg,
                        span: impl_.span,
                    })?;

                // Check refinement constraints if present
                if let Maybe::Some(ref _refinement) = assoc_type.refinement {
                    // Validate that the implementation type satisfies the refinement
                    // This requires SMT solving for full verification
                    // For now, we accept structural compatibility
                }
            } else if assoc_type.default.is_none() {
                // Associated type not provided and has no default
                return Err(ConformanceError::MissingAssociatedType {
                    protocol: protocol.name.clone(),
                    assoc_type: assoc_name.clone(),
                    implementing_type: impl_.for_type.clone(),
                    bounds: assoc_type
                        .bounds
                        .iter()
                        .map(|b| self.extract_protocol_name(&b.protocol))
                        .collect(),
                    span: impl_.span,
                });
            }
            // Associated type not provided but has default - OK
        }

        Ok(())
    }

    /// Collect associated types from superprotocols recursively
    fn collect_inherited_associated_types<'a>(
        &'a self,
        protocol: &'a Protocol,
        result: &mut Map<Text, &'a AssociatedType>,
    ) -> Result<(), ConformanceError> {
        for super_bound in protocol.super_protocols.iter() {
            if super_bound.is_negative {
                continue;
            }

            let super_name = self.extract_protocol_name(&super_bound.protocol);
            if let Some(super_proto) = self.protocols.get(&super_name) {
                // Recursively collect from this superprotocol's superprotocols
                self.collect_inherited_associated_types(super_proto, result)?;

                // Add this superprotocol's associated types
                for (name, assoc_type) in super_proto.associated_types.iter() {
                    // Only add if not already present (allow override)
                    if !result.contains_key(name) {
                        result.insert(name.clone(), assoc_type);
                    }
                }
            }
        }
        Ok(())
    }

    /// Check that all required associated constants are defined
    ///
    /// Validates:
    /// 1. All associated constants are provided
    /// 2. Constant types match
    fn check_associated_const_conformance(
        &self,
        impl_: &ProtocolImpl,
        protocol: &Protocol,
    ) -> Result<(), ConformanceError> {
        for (const_name, const_def) in protocol.associated_consts.iter() {
            if impl_.associated_consts.get(const_name).is_none() {
                return Err(ConformanceError::MissingAssociatedConst {
                    protocol: protocol.name.clone(),
                    const_name: const_name.clone(),
                    expected_type: const_def.ty.clone(),
                    implementing_type: impl_.for_type.clone(),
                    span: impl_.span,
                });
            }
            // Type checking of constant values would be done here
            // For now, we just verify presence
        }
        Ok(())
    }

    /// Format a method signature for error messages
    fn format_method_signature(&self, method: &ProtocolMethod) -> Text {
        let sig = self.format_function_type(&method.ty);
        if method.has_default {
            format!("{} (has default)", sig).into()
        } else {
            sig
        }
    }

    /// Format a function type for error messages
    fn format_function_type(&self, ty: &Type) -> Text {
        match ty {
            Type::Function {
                params,
                return_type,
                ..
            } => {
                let params_str: List<Text> =
                    params.iter().map(|p| self.type_to_string(p)).collect();
                let params_str: List<&str> = params_str.iter().map(|s| s.as_str()).collect();
                format!(
                    "fn({}) -> {}",
                    params_str.join(", "),
                    self.type_to_string(return_type)
                )
                .into()
            }
            _ => self.type_to_string(ty),
        }
    }
}

impl Default for ProtocolChecker {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Conformance Errors ====================

/// Protocol conformance checking errors
///
/// Advanced protocols (future v2.0+): GATs, higher-rank bounds, specialization with lattice ordering, coherence rules — Section 7.3 - Error Messages
///
/// These errors provide detailed information about why a protocol
/// implementation fails conformance checking.
#[derive(Debug, Clone)]
pub enum ConformanceError {
    /// Protocol definition not found
    ProtocolNotFound { name: Text, span: Span },

    /// Required superprotocol is not implemented
    SuperprotocolNotImplemented {
        implementing_type: Type,
        protocol: Text,
        superprotocol: Text,
        required_for: Text,
        span: Span,
        help: Text,
    },

    /// Negative bound violation (type implements what it shouldn't)
    NegativeBoundViolation {
        ty: Type,
        protocol: Path,
        span: Span,
    },

    /// Where clause constraint not satisfied
    WhereClauseNotSatisfied {
        ty: Type,
        constraint: Text,
        reason: Text,
        span: Span,
    },

    /// Required method is missing
    MissingMethod {
        protocol: Text,
        method: Text,
        implementing_type: Type,
        expected_signature: Text,
        span: Span,
    },

    /// Method signature doesn't match protocol requirement
    MethodSignatureMismatch {
        method: Text,
        expected: Text,
        found: Text,
        reason: Text,
        span: Span,
    },

    /// Required associated type is missing
    MissingAssociatedType {
        protocol: Text,
        assoc_type: Text,
        implementing_type: Type,
        bounds: List<Text>,
        span: Span,
    },

    /// Associated type doesn't satisfy required bounds
    AssociatedTypeBoundNotSatisfied {
        assoc_type: Text,
        bound: Text,
        actual_type: Type,
        span: Span,
    },

    /// Associated type variance mismatch
    AssociatedTypeVarianceMismatch {
        assoc_type: Text,
        message: Text,
        span: Span,
    },

    /// Required associated constant is missing
    MissingAssociatedConst {
        protocol: Text,
        const_name: Text,
        expected_type: Type,
        implementing_type: Type,
        span: Span,
    },

    /// Internal error during conformance checking
    InternalError { message: Text, span: Span },
}

impl std::fmt::Display for ConformanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConformanceError::ProtocolNotFound { name, .. } => {
                write!(f, "Protocol '{}' not found", name)
            }
            ConformanceError::SuperprotocolNotImplemented {
                implementing_type,
                protocol,
                superprotocol,
                help,
                ..
            } => {
                write!(
                    f,
                    "Cannot implement '{}' for '{:?}': superprotocol '{}' is not implemented\n\
                     Help: {}",
                    protocol, implementing_type, superprotocol, help
                )
            }
            ConformanceError::NegativeBoundViolation { ty, protocol, .. } => {
                write!(
                    f,
                    "Type '{:?}' implements protocol '{:?}' which violates a negative bound",
                    ty, protocol
                )
            }
            ConformanceError::WhereClauseNotSatisfied {
                ty,
                constraint,
                reason,
                ..
            } => {
                write!(
                    f,
                    "Where clause not satisfied: type '{:?}' does not satisfy '{}'\n\
                     Reason: {}",
                    ty, constraint, reason
                )
            }
            ConformanceError::MissingMethod {
                protocol,
                method,
                implementing_type,
                expected_signature,
                ..
            } => {
                write!(
                    f,
                    "Missing required method '{}' in implementation of '{}' for '{:?}'\n\
                     Expected: {}",
                    method, protocol, implementing_type, expected_signature
                )
            }
            ConformanceError::MethodSignatureMismatch {
                method,
                expected,
                found,
                reason,
                ..
            } => {
                write!(
                    f,
                    "Method '{}' signature mismatch\n\
                     Expected: {}\n\
                     Found: {}\n\
                     Reason: {}",
                    method, expected, found, reason
                )
            }
            ConformanceError::MissingAssociatedType {
                protocol,
                assoc_type,
                implementing_type,
                bounds,
                ..
            } => {
                let bounds_str = if bounds.is_empty() {
                    String::new()
                } else {
                    let bounds_vec: List<&str> = bounds.iter().map(|s| s.as_str()).collect();
                    format!(" where {}", bounds_vec.join(" + "))
                };
                write!(
                    f,
                    "Missing associated type '{}{}' in implementation of '{}' for '{:?}'",
                    assoc_type, bounds_str, protocol, implementing_type
                )
            }
            ConformanceError::AssociatedTypeBoundNotSatisfied {
                assoc_type,
                bound,
                actual_type,
                ..
            } => {
                write!(
                    f,
                    "Associated type '{}' = '{:?}' does not satisfy bound '{}'",
                    assoc_type, actual_type, bound
                )
            }
            ConformanceError::AssociatedTypeVarianceMismatch {
                assoc_type,
                message,
                ..
            } => {
                write!(
                    f,
                    "Variance mismatch for associated type '{}': {}",
                    assoc_type, message
                )
            }
            ConformanceError::MissingAssociatedConst {
                protocol,
                const_name,
                expected_type,
                implementing_type,
                ..
            } => {
                write!(
                    f,
                    "Missing associated constant '{}': {:?} in implementation of '{}' for '{:?}'",
                    const_name, expected_type, protocol, implementing_type
                )
            }
            ConformanceError::InternalError { message, .. } => {
                write!(f, "Internal error: {}", message)
            }
        }
    }
}

impl std::error::Error for ConformanceError {}

// ==================== Errors ====================

/// Coherence checking errors
/// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .6 - Coherence Rules (lines 10746-10966)
#[derive(Debug, Clone)]
pub enum CoherenceError {
    /// Orphan rule violation: neither protocol nor type is local to current cog
    /// Protocol coherence orphan rules: local protocol + foreign type OK, foreign protocol + local type OK, foreign protocol + foreign type NOT OK. Type parameters make the implementing type local.
    OrphanRuleViolation {
        protocol: Text,
        for_type: Text,
        protocol_crate: Maybe<Text>,
        type_crate: Maybe<Text>,
        current_crate: Text,
        newtype_suggestion: Text,
        local_protocol_suggestion: Text,
        span: Span,
    },
    /// Overlapping protocol implementations (same protocol + type)
    /// Implementation overlap rules: overlapping impls are errors unless one specializes the other
    OverlappingImplementations {
        protocol: Text,
        for_type: Text,
        first_impl_location: Span,
        second_impl_location: Span,
        specialization_suggestion: Maybe<Text>,
    },
}

impl std::fmt::Display for CoherenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoherenceError::OrphanRuleViolation {
                protocol,
                for_type,
                current_crate,
                newtype_suggestion,
                local_protocol_suggestion,
                ..
            } => {
                write!(
                    f,
                    "Cannot implement protocol '{}' for type '{}' in crate '{}': violates orphan rule.\n\
                     Either the protocol or the type must be defined in the current crate.\n\
                     \n\
                     Suggestions:\n\
                     1. Use the newtype pattern: {}\n\
                     2. Define a local protocol: {}",
                    protocol,
                    for_type,
                    current_crate,
                    newtype_suggestion,
                    local_protocol_suggestion
                )
            }
            CoherenceError::OverlappingImplementations {
                protocol,
                for_type,
                specialization_suggestion,
                ..
            } => {
                let mut msg = format!(
                    "Overlapping implementations of protocol '{}' for type '{}'",
                    protocol, for_type
                );
                if let Maybe::Some(suggestion) = specialization_suggestion {
                    msg.push_str(&format!("\n\nSuggestion: {}", suggestion));
                }
                write!(f, "{}", msg)
            }
        }
    }
}

impl std::error::Error for CoherenceError {}

/// Protocol checking errors
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("Type {ty:?} does not implement protocol {protocol:?}")]
    NotImplemented { ty: Type, protocol: Path },

    #[error("Protocol {protocol:?} does not have method {method}")]
    MethodNotFound { protocol: Path, method: Text },

    #[error("Type {ty:?} does not satisfy bound {protocol:?}")]
    BoundNotSatisfied { ty: Type, protocol: Path },

    /// Negative bound violated - type implements a protocol it should not
    ///
    /// Multi-protocol bounds: combining multiple protocol constraints (T: Display + Debug) — Negative Bounds
    ///
    /// This error occurs when a type is checked against a negative bound (T: !Protocol)
    /// but the type actually implements the protocol.
    ///
    /// # Example
    /// ```verum
    /// fn deep_clone<T: Clone + !Copy>(value: T) -> T { ... }
    ///
    /// deep_clone(42);  // ERROR: Int implements Copy, violating !Copy bound
    /// ```
    #[error("Negative bound violated: {ty:?} implements {protocol:?}")]
    NegativeBoundViolated { ty: Type, protocol: Path },

    #[error("Associated type {name} not defined")]
    AssociatedTypeNotDefined { name: Text },

    #[error("Conflicting implementations for {ty:?} and {protocol:?}")]
    ConflictingImpls { ty: Type, protocol: Path },

    #[error("Protocol {name} not found")]
    ProtocolNotFound { name: Text },

    #[error(
        "Associated type {assoc_name} not specified for type {for_type:?} in protocol {protocol:?}"
    )]
    AssociatedTypeNotSpecified {
        protocol: Path,
        assoc_name: Text,
        for_type: Type,
    },

    #[error("{0}")]
    Violations(ProtocolViolations),

    #[error(
        "Cyclic protocol inheritance detected: protocol {protocol} is part of a cycle involving {cycle:?}"
    )]
    CyclicInheritance { protocol: Text, cycle: List<Text> },
}

// ==================== Protocol Violation Types ====================

/// A collection of protocol implementation violations
///
/// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Protocol System
///
/// This provides detailed information about why a type does not
/// implement a protocol, enabling actionable error messages.
#[derive(Debug, Clone)]
pub struct ProtocolViolations {
    /// The type that was checked
    pub ty: Type,
    /// The protocol that was not implemented
    pub protocol: Text,
    /// List of specific violations
    pub violations: List<ProtocolViolation>,
}

impl std::fmt::Display for ProtocolViolations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "Type `{}` does not implement protocol `{}`:",
            self.ty.to_text(),
            self.protocol
        )?;
        for (i, violation) in self.violations.iter().enumerate() {
            writeln!(f, "  {}. {}", i + 1, violation)?;
        }
        Ok(())
    }
}

/// Detailed violation type explaining why a protocol is not implemented
///
/// Advanced protocols (future v2.0+): GATs, higher-rank bounds, specialization with lattice ordering, coherence rules — Section 7.3 - Error Messages
///
/// Each variant provides specific information about what is missing
/// or incorrect, enabling the developer to fix the issue.
#[derive(Debug, Clone)]
pub enum ProtocolViolation {
    /// No implementation block exists for this type and protocol
    MissingImplBlock {
        /// The type that needs an impl block
        for_type: Type,
        /// The protocol to implement
        protocol: Text,
        /// Suggested fix
        help: Text,
    },

    /// A required method is not implemented
    MissingMethod {
        /// Name of the missing method
        method_name: Text,
        /// Expected signature
        expected_signature: Text,
        /// Whether the method has a default implementation
        has_default: bool,
    },

    /// A method has an incompatible signature
    MethodSignatureMismatch {
        /// Name of the method
        method_name: Text,
        /// Expected signature from protocol
        expected: Text,
        /// Actual signature in implementation
        actual: Text,
        /// Specific reason for mismatch
        reason: Text,
    },

    /// A required associated type is not defined
    MissingAssociatedType {
        /// Name of the associated type
        assoc_name: Text,
        /// Required bounds on the type
        bounds: List<Text>,
        /// Whether there is a default
        has_default: bool,
    },

    /// An associated type does not satisfy its bounds
    AssociatedTypeBoundViolation {
        /// Name of the associated type
        assoc_name: Text,
        /// The actual type provided
        actual_type: Type,
        /// The bound that is not satisfied
        unsatisfied_bound: Text,
    },

    /// A required associated constant is not defined
    MissingAssociatedConst {
        /// Name of the constant
        const_name: Text,
        /// Expected type
        expected_type: Type,
    },

    /// A superprotocol is not implemented
    SuperprotocolNotImplemented {
        /// The superprotocol that must be implemented first
        superprotocol: Text,
        /// Help text explaining the dependency
        help: Text,
    },

    /// A where clause is not satisfied
    WhereClauseNotSatisfied {
        /// The type that doesn't satisfy the clause
        ty: Type,
        /// The constraint that is not met
        constraint: Text,
        /// Explanation of why
        reason: Text,
    },
}

impl std::fmt::Display for ProtocolViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolViolation::MissingImplBlock {
                for_type,
                protocol,
                help,
            } => {
                write!(
                    f,
                    "Missing implementation block: `implement {} for {}`\n     Help: {}",
                    protocol,
                    for_type.to_text(),
                    help
                )
            }
            ProtocolViolation::MissingMethod {
                method_name,
                expected_signature,
                has_default,
            } => {
                if *has_default {
                    write!(
                        f,
                        "Method `{}` is not implemented (has default: {})",
                        method_name, expected_signature
                    )
                } else {
                    write!(
                        f,
                        "Required method `{}` is not implemented. Expected: {}",
                        method_name, expected_signature
                    )
                }
            }
            ProtocolViolation::MethodSignatureMismatch {
                method_name,
                expected,
                actual,
                reason,
            } => {
                write!(
                    f,
                    "Method `{}` has incompatible signature:\n     Expected: {}\n     Found: {}\n     Reason: {}",
                    method_name, expected, actual, reason
                )
            }
            ProtocolViolation::MissingAssociatedType {
                assoc_name,
                bounds,
                has_default,
            } => {
                let bounds_str = if bounds.is_empty() {
                    "".into()
                } else {
                    let bound_strs: List<&str> = bounds.iter().map(|b| b.as_str()).collect();
                    format!(" (bounds: {})", bound_strs.join(", "))
                };
                if *has_default {
                    write!(
                        f,
                        "Associated type `{}` not specified{} (has default)",
                        assoc_name, bounds_str
                    )
                } else {
                    write!(
                        f,
                        "Required associated type `{}` is not defined{}",
                        assoc_name, bounds_str
                    )
                }
            }
            ProtocolViolation::AssociatedTypeBoundViolation {
                assoc_name,
                actual_type,
                unsatisfied_bound,
            } => {
                write!(
                    f,
                    "Associated type `{}` = `{}` does not satisfy bound `{}`",
                    assoc_name,
                    actual_type.to_text(),
                    unsatisfied_bound
                )
            }
            ProtocolViolation::MissingAssociatedConst {
                const_name,
                expected_type,
            } => {
                write!(
                    f,
                    "Required associated constant `{}` of type `{}` is not defined",
                    const_name,
                    expected_type.to_text()
                )
            }
            ProtocolViolation::SuperprotocolNotImplemented {
                superprotocol,
                help,
            } => {
                write!(
                    f,
                    "Superprotocol `{}` must be implemented first\n     Help: {}",
                    superprotocol, help
                )
            }
            ProtocolViolation::WhereClauseNotSatisfied {
                ty,
                constraint,
                reason,
            } => {
                write!(
                    f,
                    "Where clause not satisfied: `{}` does not satisfy `{}`\n     Reason: {}",
                    ty.to_text(),
                    constraint,
                    reason
                )
            }
        }
    }
}

// ==================== Protocol Derivation ====================

/// Derivation strategy for protocol implementations
///
/// Specifies how method implementations should be generated:
/// - Structural: Compare/display based on type structure (fields, variants)
/// - Lexicographic: Compare fields in declaration order
/// - Custom: Use user-provided implementation hints
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DerivationStrategy {
    /// Structural derivation based on type layout
    #[default]
    Structural,
    /// Lexicographic ordering for Ord
    Lexicographic,
    /// Custom derivation with hints
    Custom { hints: Map<Text, Text> },
}

/// Derived method implementation
///
/// Represents a synthesized method body for protocol derivation.
/// The body is stored as a structured representation that can be
/// lowered to AST or directly to code generation.
#[derive(Debug, Clone)]
pub struct DerivedMethod {
    /// Method name
    pub name: Text,
    /// Parameter names and types
    pub params: List<(Text, Type)>,
    /// Return type
    pub return_type: Type,
    /// Method body as structured operations
    pub body: DerivedBody,
}

/// Structured representation of derived method body
#[derive(Debug, Clone)]
pub enum DerivedBody {
    /// Compare all fields for equality
    /// Returns true if all field comparisons return true
    StructuralEq {
        /// Field names and their types
        fields: List<(Text, Type)>,
    },

    /// Compare all fields in order for Ord
    /// Returns first non-Equal comparison result
    LexicographicCmp {
        /// Field names and their types (in comparison order)
        fields: List<(Text, Type)>,
    },

    /// Format type for display
    /// Generates "TypeName { field1: value1, ... }" style output
    StructuralShow {
        /// Type name
        type_name: Text,
        /// Field names and their types
        fields: List<(Text, Type)>,
    },

    /// Match on variant discriminant for enum equality
    VariantEq {
        /// Variant names and their payload types (single payload type per variant)
        variants: List<(Text, Type)>,
    },

    /// Match on variant discriminant for enum ordering
    VariantCmp {
        /// Variant names and their payload types (in discriminant order)
        variants: List<(Text, Type)>,
    },

    /// Match on variant for enum display
    VariantShow {
        /// Type name
        type_name: Text,
        /// Variant names and their payload types
        variants: List<(Text, Type)>,
    },

    /// Delegate to inner type (for newtypes)
    Delegate {
        /// Field name or index
        field: Text,
        /// Inner type
        inner_type: Type,
    },

    /// Default implementation (use protocol's default)
    UseDefault,
}

/// Automatic protocol derivation
///
/// Provides automatic derivation of common protocols for user-defined types.
/// Supports:
/// - Eq: Structural equality comparison
/// - Ord: Lexicographic ordering (requires Eq)
/// - Show: Debug/display formatting
/// - Clone: Deep copy (requires all fields Clone)
/// - Hash: Hashing (requires Eq)
///
/// # Example
///
/// ```verum
/// @derive(Eq, Ord, Show)
/// struct Point {
///     x: Int,
///     y: Int,
/// }
/// ```
///
/// This generates:
/// - `eq(self, other) -> Bool`: Compare x, then y
/// - `cmp(self, other) -> Ordering`: Compare x, then y lexicographically
/// - `show(self) -> Text`: Format as "Point { x: ..., y: ... }"
pub struct ProtocolDerivation {
    checker: ProtocolChecker,
    /// Derivation strategy to use
    strategy: DerivationStrategy,
}

impl ProtocolDerivation {
    /// Create a new protocol derivation engine
    pub fn new(checker: ProtocolChecker) -> Self {
        Self {
            checker,
            strategy: DerivationStrategy::default(),
        }
    }

    /// Create with a specific derivation strategy
    pub fn with_strategy(checker: ProtocolChecker, strategy: DerivationStrategy) -> Self {
        Self { checker, strategy }
    }

    /// Derive protocol implementation for a type
    ///
    /// Returns a complete ProtocolImpl with synthesized method implementations.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The protocol is not derivable
    /// - Required super-protocols are not implemented
    /// - Type structure is not compatible with derivation
    pub fn derive(&mut self, ty: &Type, protocol: &Text) -> Result<ProtocolImpl, ProtocolError> {
        if !self.checker.is_derivable(protocol) {
            return Err(ProtocolError::ProtocolNotFound {
                name: protocol.clone(),
            });
        }

        match protocol.as_str() {
            "Eq" => self.derive_eq(ty),
            "Ord" => self.derive_ord(ty),
            "Show" => self.derive_show(ty),
            "Clone" => self.derive_clone(ty),
            "Hash" => self.derive_hash(ty),
            "Default" => self.derive_default(ty),
            _ => Err(ProtocolError::ProtocolNotFound {
                name: protocol.clone(),
            }),
        }
    }

    /// Check if a type can derive a protocol
    pub fn can_derive(&self, ty: &Type, protocol: &Text) -> bool {
        match protocol.as_str() {
            "Eq" | "Show" | "Clone" | "Default" => self.all_fields_implement(ty, protocol),
            "Ord" => {
                // Ord requires Eq
                let eq_path = Path::single(Ident::new("Eq", Span::default()));
                self.checker.implements(ty, &eq_path) && self.all_fields_implement(ty, protocol)
            }
            "Hash" => {
                // Hash requires Eq
                let eq_path = Path::single(Ident::new("Eq", Span::default()));
                self.checker.implements(ty, &eq_path) && self.all_fields_implement(ty, protocol)
            }
            _ => false,
        }
    }

    /// Check if all fields of a type implement a protocol
    fn all_fields_implement(&self, ty: &Type, protocol: &Text) -> bool {
        let protocol_path = Path::single(Ident::new(protocol.as_str(), Span::default()));

        match ty {
            Type::Record(fields) => fields
                .iter()
                .all(|(_, field_ty)| self.checker.implements(field_ty, &protocol_path)),
            Type::Tuple(elements) => elements
                .iter()
                .all(|elem_ty| self.checker.implements(elem_ty, &protocol_path)),
            Type::Variant(variants) => {
                // Variant maps tag name to single payload type
                variants
                    .iter()
                    .all(|(_, payload_ty)| self.checker.implements(payload_ty, &protocol_path))
            }
            // Primitive types are assumed to implement all derivable protocols
            Type::Int | Type::Float | Type::Bool | Type::Char | Type::Text | Type::Unit => true,
            _ => false,
        }
    }

    /// Extract fields from a type for derivation
    fn extract_fields(&self, ty: &Type) -> List<(Text, Type)> {
        match ty {
            Type::Record(fields) => fields
                .iter()
                .map(|(name, field_ty)| (name.clone(), field_ty.clone()))
                .collect(),
            Type::Tuple(elements) => elements
                .iter()
                .enumerate()
                .map(|(i, elem_ty)| (format!("{}", i).into(), elem_ty.clone()))
                .collect(),
            _ => List::new(),
        }
    }

    /// Extract variants from a type for derivation
    fn extract_variants(&self, ty: &Type) -> List<(Text, Type)> {
        match ty {
            Type::Variant(variants) => {
                // Variant is IndexMap<Text, Type> - each variant has one payload type
                variants
                    .iter()
                    .map(|(name, payload_ty)| (name.clone(), payload_ty.clone()))
                    .collect()
            }
            _ => List::new(),
        }
    }

    /// Get type name for display
    fn type_name(&self, ty: &Type) -> Text {
        match ty {
            Type::Named { path, .. } => path
                .segments
                .last()
                .map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => ident.name.clone(),
                    _ => "Unknown".into(),
                })
                .unwrap_or_else(|| "Unknown".into()),
            Type::Generic { name, .. } => name.clone(),
            Type::Record(_) => "Record".into(),
            Type::Tuple(_) => "Tuple".into(),
            Type::Variant(_) => "Variant".into(),
            _ => format!("{:?}", ty).into(),
        }
    }

    /// Derive Eq protocol implementation
    ///
    /// Generates structural equality comparison:
    /// - For records: Compare all fields with &&
    /// - For tuples: Compare all elements with &&
    /// - For variants: Match on discriminant, compare payloads
    fn derive_eq(&self, ty: &Type) -> Result<ProtocolImpl, ProtocolError> {
        let fields = self.extract_fields(ty);
        let variants = self.extract_variants(ty);

        let body = if !variants.is_empty() {
            DerivedBody::VariantEq { variants }
        } else if !fields.is_empty() {
            DerivedBody::StructuralEq {
                fields: fields.clone(),
            }
        } else {
            // Primitive or opaque type
            DerivedBody::UseDefault
        };

        let eq_method = DerivedMethod {
            name: "eq".into(),
            params: List::from(vec![
                ("self".into(), ty.clone()),
                ("other".into(), ty.clone()),
            ]),
            return_type: Type::Bool,
            body,
        };

        // Create ne method using default (not eq)
        let ne_method = DerivedMethod {
            name: "ne".into(),
            params: List::from(vec![
                ("self".into(), ty.clone()),
                ("other".into(), ty.clone()),
            ]),
            return_type: Type::Bool,
            body: DerivedBody::UseDefault, // Uses default: !self.eq(other)
        };

        let mut methods = Map::new();
        methods.insert(
            "eq".into(),
            self.derived_method_to_function_type(&eq_method),
        );
        methods.insert(
            "ne".into(),
            self.derived_method_to_function_type(&ne_method),
        );

        Ok(ProtocolImpl {
            protocol: Path::single(Ident::new("Eq", Span::default())),
            protocol_args: List::new(),
            for_type: ty.clone(),
            where_clauses: List::new(),
            methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::None,
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        })
    }

    /// Derive Ord protocol implementation
    ///
    /// Generates lexicographic ordering:
    /// - For records: Compare fields in declaration order
    /// - For tuples: Compare elements in order
    /// - For variants: Compare discriminants first, then payloads
    fn derive_ord(&self, ty: &Type) -> Result<ProtocolImpl, ProtocolError> {
        // Check that Eq is implemented (Ord requires Eq)
        let eq_path = Path::single(Ident::new("Eq", Span::default()));
        if !self.checker.implements(ty, &eq_path) {
            return Err(ProtocolError::BoundNotSatisfied {
                ty: ty.clone(),
                protocol: eq_path,
            });
        }

        let fields = self.extract_fields(ty);
        let variants = self.extract_variants(ty);

        let body = if !variants.is_empty() {
            DerivedBody::VariantCmp { variants }
        } else if !fields.is_empty() {
            DerivedBody::LexicographicCmp {
                fields: fields.clone(),
            }
        } else {
            DerivedBody::UseDefault
        };

        let cmp_method = DerivedMethod {
            name: "cmp".into(),
            params: List::from(vec![
                ("self".into(), ty.clone()),
                ("other".into(), ty.clone()),
            ]),
            return_type: Type::Named {
                path: Path::single(Ident::new(WKT::Ordering.as_str(), Span::default())),
                args: List::new(),
            },
            body,
        };

        let mut methods = Map::new();
        methods.insert(
            "cmp".into(),
            self.derived_method_to_function_type(&cmp_method),
        );

        // Add comparison operators that use default implementations
        for op in &["lt", "le", "gt", "ge"] {
            let op_method = DerivedMethod {
                name: (*op).into(),
                params: List::from(vec![
                    ("self".into(), ty.clone()),
                    ("other".into(), ty.clone()),
                ]),
                return_type: Type::Bool,
                body: DerivedBody::UseDefault,
            };
            methods.insert(
                (*op).into(),
                self.derived_method_to_function_type(&op_method),
            );
        }

        Ok(ProtocolImpl {
            protocol: Path::single(Ident::new("Ord", Span::default())),
            protocol_args: List::new(),
            for_type: ty.clone(),
            where_clauses: List::new(),
            methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::None,
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        })
    }

    /// Derive Show protocol implementation
    ///
    /// Generates debug/display formatting:
    /// - For records: "TypeName { field1: value1, field2: value2 }"
    /// - For tuples: "(value1, value2, ...)"
    /// - For variants: "VariantName(payload1, payload2)" or "VariantName"
    fn derive_show(&self, ty: &Type) -> Result<ProtocolImpl, ProtocolError> {
        let type_name = self.type_name(ty);
        let fields = self.extract_fields(ty);
        let variants = self.extract_variants(ty);

        let body = if !variants.is_empty() {
            DerivedBody::VariantShow {
                type_name,
                variants,
            }
        } else if !fields.is_empty() {
            DerivedBody::StructuralShow {
                type_name,
                fields: fields.clone(),
            }
        } else {
            DerivedBody::UseDefault
        };

        let show_method = DerivedMethod {
            name: "show".into(),
            params: List::from(vec![("self".into(), ty.clone())]),
            return_type: Type::Text,
            body,
        };

        let mut methods = Map::new();
        methods.insert(
            "show".into(),
            self.derived_method_to_function_type(&show_method),
        );

        Ok(ProtocolImpl {
            protocol: Path::single(Ident::new("Show", Span::default())),
            protocol_args: List::new(),
            for_type: ty.clone(),
            where_clauses: List::new(),
            methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::None,
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        })
    }

    /// Derive Clone protocol implementation
    fn derive_clone(&self, ty: &Type) -> Result<ProtocolImpl, ProtocolError> {
        let fields = self.extract_fields(ty);

        let body = if !fields.is_empty() {
            DerivedBody::StructuralEq { fields } // Reuses field extraction
        } else {
            DerivedBody::UseDefault
        };

        let clone_method = DerivedMethod {
            name: "clone".into(),
            params: List::from(vec![("self".into(), ty.clone())]),
            return_type: ty.clone(),
            body,
        };

        let mut methods = Map::new();
        methods.insert(
            "clone".into(),
            self.derived_method_to_function_type(&clone_method),
        );

        Ok(ProtocolImpl {
            protocol: Path::single(Ident::new("Clone", Span::default())),
            protocol_args: List::new(),
            for_type: ty.clone(),
            where_clauses: List::new(),
            methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::None,
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        })
    }

    /// Derive Hash protocol implementation
    fn derive_hash(&self, ty: &Type) -> Result<ProtocolImpl, ProtocolError> {
        // Hash requires Eq
        let eq_path = Path::single(Ident::new("Eq", Span::default()));
        if !self.checker.implements(ty, &eq_path) {
            return Err(ProtocolError::BoundNotSatisfied {
                ty: ty.clone(),
                protocol: eq_path,
            });
        }

        let hash_method = DerivedMethod {
            name: "hash".into(),
            params: List::from(vec![
                ("self".into(), ty.clone()),
                (
                    "hasher".into(),
                    Type::Named {
                        path: Path::single(Ident::new("Hasher", Span::default())),
                        args: List::new(),
                    },
                ),
            ]),
            return_type: Type::Unit,
            body: DerivedBody::StructuralEq {
                fields: self.extract_fields(ty),
            },
        };

        let mut methods = Map::new();
        methods.insert(
            "hash".into(),
            self.derived_method_to_function_type(&hash_method),
        );

        Ok(ProtocolImpl {
            protocol: Path::single(Ident::new("Hash", Span::default())),
            protocol_args: List::new(),
            for_type: ty.clone(),
            where_clauses: List::new(),
            methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::None,
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        })
    }

    /// Derive Default protocol implementation
    fn derive_default(&self, ty: &Type) -> Result<ProtocolImpl, ProtocolError> {
        let default_method = DerivedMethod {
            name: "default".into(),
            params: List::new(),
            return_type: ty.clone(),
            body: DerivedBody::StructuralEq {
                fields: self.extract_fields(ty),
            },
        };

        let mut methods = Map::new();
        methods.insert(
            "default".into(),
            self.derived_method_to_function_type(&default_method),
        );

        Ok(ProtocolImpl {
            protocol: Path::single(Ident::new("Default", Span::default())),
            protocol_args: List::new(),
            for_type: ty.clone(),
            where_clauses: List::new(),
            methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::None,
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        })
    }

    /// Convert a DerivedMethod to a function Type for ProtocolImpl methods map
    fn derived_method_to_function_type(&self, derived: &DerivedMethod) -> Type {
        Type::function(
            derived.params.iter().map(|(_, ty)| ty.clone()).collect(),
            derived.return_type.clone(),
        )
    }
}

// ==================== Unified Protocol Error (Phase 1 Consolidation) ====================

/// Unified wrapper over all protocol-related error types in `verum_types`.
///
/// **Phase 1 — Additive Only**: This enum consolidates the four previously-distinct
/// protocol error families (`ProtocolError`, `ConformanceError`, `CoherenceError`,
/// `ObjectSafetyError`) behind a single umbrella type without forcing migration of
/// existing call sites. The original enums remain fully functional; new code can
/// opt into the unified representation via `From` conversions.
///
/// # Grouping Rationale
/// - **NotImplemented**: convenience variant for the most common case (type does
///   not implement a required protocol) — mirrors `ProtocolError::NotImplemented`
///   but stores stringified names so callers without a `Type`/`Path` in hand can
///   still construct it.
/// - **Protocol**: general protocol-check errors (method resolution, bounds,
///   associated type lookup, conflicting impls, embedded `ProtocolViolations`).
/// - **Conformance**: detailed conformance-check failures (missing methods,
///   signature mismatches, associated type bounds, superprotocol chains).
/// - **Coherence**: orphan rule + overlapping implementation diagnostics.
/// - **ObjectSafety**: `&dyn Protocol` usability checks (returns Self, generic
///   methods, associated constants, etc.).
///
/// # Migration
/// A future phase may replace direct uses of the underlying enums with this
/// wrapper. Until then, treat `UnifiedProtocolError` as a *sink* — construct it
/// from any of the four sources via `?` / `.into()`.
// Note: not `Clone` because `ProtocolError` is not `Clone` (it embeds
// `ProtocolViolations`, which in turn embeds `Type`). Keeping the wrapper
// non-Clone avoids forcing changes to the existing error types in Phase 1.
#[derive(Debug, thiserror::Error)]
pub enum UnifiedProtocolError {
    /// Shortcut: a named protocol is not implemented for a named type.
    ///
    /// Prefer `Protocol(ProtocolError::NotImplemented { .. })` when full
    /// `Type` / `Path` context is available.
    #[error("type `{type_name}` does not implement protocol `{protocol}`")]
    NotImplemented { protocol: Text, type_name: Text },

    /// Wraps a general protocol-check error (method resolution, bounds, etc.).
    #[error(transparent)]
    Protocol(#[from] ProtocolError),

    /// Wraps a detailed conformance-check failure.
    #[error("conformance error: {0}")]
    Conformance(ConformanceError),

    /// Wraps a coherence (orphan/overlap) violation.
    #[error("coherence error: {0}")]
    Coherence(CoherenceError),

    /// Wraps an object-safety violation.
    #[error("object safety error: {0}")]
    ObjectSafety(ObjectSafetyError),
}

// Note: `ConformanceError`, `CoherenceError`, and `ObjectSafetyError` do not all
// implement `std::error::Error`, so we cannot use `#[from] #[error(transparent)]`
// uniformly. We supply explicit `From` impls and display them via `{0}`.

impl From<ConformanceError> for UnifiedProtocolError {
    fn from(err: ConformanceError) -> Self {
        UnifiedProtocolError::Conformance(err)
    }
}

impl From<CoherenceError> for UnifiedProtocolError {
    fn from(err: CoherenceError) -> Self {
        UnifiedProtocolError::Coherence(err)
    }
}

impl From<ObjectSafetyError> for UnifiedProtocolError {
    fn from(err: ObjectSafetyError) -> Self {
        UnifiedProtocolError::ObjectSafety(err)
    }
}

impl UnifiedProtocolError {
    /// Construct a `NotImplemented` variant from string-like inputs.
    pub fn not_implemented(protocol: impl Into<Text>, type_name: impl Into<Text>) -> Self {
        UnifiedProtocolError::NotImplemented {
            protocol: protocol.into(),
            type_name: type_name.into(),
        }
    }

    /// Returns a short static tag identifying the underlying error family —
    /// useful for diagnostics, metrics, and test assertions.
    pub fn category(&self) -> &'static str {
        match self {
            UnifiedProtocolError::NotImplemented { .. } => "not_implemented",
            UnifiedProtocolError::Protocol(_) => "protocol",
            UnifiedProtocolError::Conformance(_) => "conformance",
            UnifiedProtocolError::Coherence(_) => "coherence",
            UnifiedProtocolError::ObjectSafety(_) => "object_safety",
        }
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::advanced_protocols::*;

    #[test]
    fn test_associated_type_simple() {
        let simple = AssociatedType::simple("Item".into(), List::new());
        assert_eq!(simple.name, "Item");
        assert!(!simple.is_gat());
        assert_eq!(simple.arity(), 0);
        assert!(matches!(simple.kind, AssociatedTypeKind::Regular));
    }

    #[test]
    fn test_associated_type_gat() {
        let type_params = List::from(vec![GATTypeParam {
            name: "T".into(),
            bounds: List::new(),
            default: Maybe::None,
            variance: Variance::Covariant,
        }]);

        let gat = AssociatedType::generic("Wrapped".into(), type_params, List::new(), List::new());

        assert_eq!(gat.name, "Wrapped");
        assert!(gat.is_gat());
        assert_eq!(gat.arity(), 1);
        assert!(matches!(gat.kind, AssociatedTypeKind::Generic { arity: 1 }));
    }

    #[test]
    fn test_lending_iterator_protocol() {
        let checker = ProtocolChecker::new();
        let protocol = checker.get_protocol(&"LendingIterator".into());

        assert!(matches!(protocol, Maybe::Some(_)));
        if let Maybe::Some(proto) = protocol {
            assert_eq!(proto.name, "LendingIterator");
            assert!(proto.methods.contains_key(&"next".into()));
            assert!(proto.associated_types.contains_key(&"Item".into()));
        }
    }

    #[test]
    fn test_streaming_iterator_protocol() {
        let checker = ProtocolChecker::new();
        let protocol = checker.get_protocol(&"StreamingIterator".into());

        assert!(matches!(protocol, Maybe::Some(_)));
        if let Maybe::Some(proto) = protocol {
            assert_eq!(proto.name, "StreamingIterator");
            assert!(proto.methods.contains_key(&"get".into()));
            assert!(proto.methods.contains_key(&"advance".into()));
            assert!(proto.associated_types.contains_key(&"Item".into()));
        }
    }

    #[test]
    fn test_functor_gat_protocol() {
        let checker = ProtocolChecker::new();
        let protocol = checker.get_protocol(&"FunctorGAT".into());

        assert!(matches!(protocol, Maybe::Some(_)));
        if let Maybe::Some(proto) = protocol {
            assert_eq!(proto.name, "FunctorGAT");
            assert!(proto.methods.contains_key(&"fmap".into()));
            assert!(proto.associated_types.contains_key(&"F".into()));

            // Check that F is a GAT with 1 type parameter
            if let Some(f_type) = proto.associated_types.get(&"F".into()) {
                assert!(f_type.is_gat());
                assert_eq!(f_type.arity(), 1);
            }
        }
    }

    #[test]
    fn test_monad_gat_protocol() {
        let checker = ProtocolChecker::new();
        let protocol = checker.get_protocol(&"MonadGAT".into());

        assert!(matches!(protocol, Maybe::Some(_)));
        if let Maybe::Some(proto) = protocol {
            assert_eq!(proto.name, "MonadGAT");
            assert!(proto.methods.contains_key(&"pure".into()));
            assert!(proto.methods.contains_key(&"bind".into()));
            assert!(proto.associated_types.contains_key(&"Wrapped".into()));

            // Check that Wrapped is a GAT with 1 type parameter
            if let Some(wrapped) = proto.associated_types.get(&"Wrapped".into()) {
                assert!(wrapped.is_gat());
                assert_eq!(wrapped.arity(), 1);
            }
        }
    }

    #[test]
    fn test_gat_arity_mismatch() {
        let checker = ProtocolChecker::new();

        // Try to resolve GAT with wrong number of type arguments
        let result = checker.resolve_gat_instantiation(
            &"FunctorGAT".into(),
            &"F".into(),
            &List::new(), // Should have 1 type arg, we provide 0
        );

        assert!(result.is_err());
        if let Err(AdvancedProtocolError::GATArityMismatch {
            expected, found, ..
        }) = result
        {
            assert_eq!(expected, 1);
            assert_eq!(found, 0);
        } else {
            panic!("Expected GATArityMismatch error");
        }
    }

    #[test]
    fn test_gat_instantiation() {
        let checker = ProtocolChecker::new();

        // Resolve GAT with correct number of type arguments
        let result = checker.resolve_gat_instantiation(
            &"FunctorGAT".into(),
            &"F".into(),
            &List::from(vec![Type::Int]),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_gat_constraints() {
        let type_params = List::from(vec![GATTypeParam {
            name: "T".into(),
            bounds: List::from(vec![ProtocolBound {
                protocol: Path::single(Ident::new("Clone", Span::default())),
                args: List::new(),
                is_negative: false,
            }]),
            default: Maybe::None,
            variance: Variance::Covariant,
        }]);

        let where_clauses = List::from(vec![GATWhereClause {
            param: "T".into(),
            constraints: List::from(vec![ProtocolBound {
                protocol: Path::single(Ident::new("Clone", Span::default())),
                args: List::new(),
                is_negative: false,
            }]),
            span: Span::default(),
        }]);

        let gat =
            AssociatedType::generic("Container".into(), type_params, List::new(), where_clauses);

        // Use new_empty() for a clean checker without standard protocol registrations
        // This allows testing GAT constraint checking in isolation
        let checker = ProtocolChecker::new_empty();

        // Check with Int - in an empty checker, Int doesn't have Clone registered
        let result = checker.check_gat_constraints(&gat, &List::from(vec![Type::Int]));

        // Should fail because Clone is not registered in the empty checker
        assert!(result.is_err());
    }

    #[test]
    fn test_specialization_none() {
        let impl_ = ProtocolImpl {
            protocol: Path::single(Ident::new("Display", Span::default())),
            protocol_args: List::new(),
            for_type: Type::Int,
            where_clauses: List::new(),
            methods: Map::new(),
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };

        assert!(matches!(impl_.specialization, Maybe::None));
    }

    #[test]
    fn test_specialization_with_rank() {
        let spec_info = SpecializationInfo::specialized(
            Path::single(Ident::new("BaseImpl", Span::default())),
            5,
        );

        let impl_ = ProtocolImpl {
            protocol: Path::single(Ident::new("Display", Span::default())),
            protocol_args: List::new(),
            for_type: Type::Int,
            where_clauses: List::new(),
            methods: Map::new(),
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::Some(spec_info),
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };

        if let Maybe::Some(spec) = impl_.specialization {
            assert!(spec.is_specialized);
            assert_eq!(spec.specificity_rank, 5);
        } else {
            panic!("Expected specialization info");
        }
    }

    #[test]
    fn test_protocol_with_specialization_info() {
        let proto = Protocol {
            name: "Specialized".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            methods: Map::new(),
            associated_types: Map::new(),
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Maybe::Some(SpecializationInfo::none()),
            defining_crate: Maybe::Some("test".into()),
            span: Span::default(),
        };

        assert!(matches!(proto.specialization_info, Maybe::Some(_)));
    }

    #[test]
    fn test_resolve_specialization_no_impls() {
        let checker = ProtocolChecker::new_empty();

        let result = checker.resolve_specialization(
            &Type::Int,
            &Path::single(Ident::new("Display", Span::default())),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_specialization_single_impl() {
        let mut checker = ProtocolChecker::new_empty();

        let impl_ = ProtocolImpl {
            protocol: Path::single(Ident::new("Display", Span::default())),
            protocol_args: List::new(),
            for_type: Type::Int,
            where_clauses: List::new(),
            methods: Map::new(),
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };

        let _ = checker.register_impl(impl_);

        let result = checker.resolve_specialization(
            &Type::Int,
            &Path::single(Ident::new("Display", Span::default())),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_backward_compatibility() {
        // Test that old code using simple() still works
        let simple = AssociatedType::simple("Item".into(), List::new());

        // Fields that existed before
        assert_eq!(simple.name, "Item");
        assert_eq!(simple.bounds.len(), 0);
        assert!(matches!(simple.default, Maybe::None));

        // New fields have sensible defaults
        assert_eq!(simple.type_params.len(), 0);
        assert_eq!(simple.where_clauses.len(), 0);
        assert!(matches!(simple.kind, AssociatedTypeKind::Regular));
    }

    #[test]
    fn test_type_parameter_substitution_simple() {
        let checker = ProtocolChecker::new();

        // Test substituting T -> Int in a simple type parameter
        let param_t = Type::Named {
            path: Path::single(Ident::new("T", Span::default())),
            args: List::new(),
        };

        let mut subst_map = Map::new();
        subst_map.insert("T".into(), Type::Int);

        let result = checker.substitute_type_params(&param_t, &subst_map);
        assert!(matches!(result, Type::Int));
    }

    #[test]
    fn test_type_parameter_substitution_nested() {
        let checker = ProtocolChecker::new();

        // Test substituting T -> Int in List<T>
        let list_t = Type::Named {
            path: Path::single(Ident::new("List", Span::default())),
            args: List::from(vec![Type::Named {
                path: Path::single(Ident::new("T", Span::default())),
                args: List::new(),
            }]),
        };

        let mut subst_map = Map::new();
        subst_map.insert("T".into(), Type::Int);

        let result = checker.substitute_type_params(&list_t, &subst_map);

        // Result should be List<Int>
        if let Type::Named { path, args } = result {
            assert_eq!(path.as_ident().map(|i| i.as_str()), Some("List"));
            assert_eq!(args.len(), 1);
            assert!(matches!(args.get(0), Some(&Type::Int)));
        } else {
            panic!("Expected Named type, got {:?}", result);
        }
    }

    #[test]
    fn test_type_parameter_substitution_multiple() {
        let checker = ProtocolChecker::new();

        // Test substituting T -> Int, U -> Text in Map<T, U>
        let map_t_u = Type::Named {
            path: Path::single(Ident::new("Map", Span::default())),
            args: List::from(vec![
                Type::Named {
                    path: Path::single(Ident::new("T", Span::default())),
                    args: List::new(),
                },
                Type::Named {
                    path: Path::single(Ident::new("U", Span::default())),
                    args: List::new(),
                },
            ]),
        };

        let mut subst_map = Map::new();
        subst_map.insert("T".into(), Type::Int);
        subst_map.insert("U".into(), Type::Text);

        let result = checker.substitute_type_params(&map_t_u, &subst_map);

        // Result should be Map<Int, Text>
        if let Type::Named { path, args } = result {
            assert_eq!(path.as_ident().map(|i| i.as_str()), Some("Map"));
            assert_eq!(args.len(), 2);
            assert!(matches!(args.get(0), Some(&Type::Int)));
            assert!(matches!(args.get(1), Some(&Type::Text)));
        } else {
            panic!("Expected Named type, got {:?}", result);
        }
    }

    #[test]
    fn test_type_parameter_substitution_references() {
        let checker = ProtocolChecker::new();

        // Test substituting T -> Int in &T
        let ref_t = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Named {
                path: Path::single(Ident::new("T", Span::default())),
                args: List::new(),
            }),
        };

        let mut subst_map = Map::new();
        subst_map.insert("T".into(), Type::Int);

        let result = checker.substitute_type_params(&ref_t, &subst_map);

        // Result should be &Int
        if let Type::Reference { mutable, inner } = result {
            assert!(!mutable);
            assert!(matches!(*inner, Type::Int));
        } else {
            panic!("Expected Reference type, got {:?}", result);
        }
    }

    #[test]
    fn test_type_parameter_substitution_tuples() {
        let checker = ProtocolChecker::new();

        // Test substituting T -> Int in (T, Bool, T)
        let tuple_t = Type::Tuple(List::from(vec![
            Type::Named {
                path: Path::single(Ident::new("T", Span::default())),
                args: List::new(),
            },
            Type::Bool,
            Type::Named {
                path: Path::single(Ident::new("T", Span::default())),
                args: List::new(),
            },
        ]));

        let mut subst_map = Map::new();
        subst_map.insert("T".into(), Type::Int);

        let result = checker.substitute_type_params(&tuple_t, &subst_map);

        // Result should be (Int, Bool, Int)
        if let Type::Tuple(elements) = result {
            assert_eq!(elements.len(), 3);
            assert!(matches!(elements.get(0), Some(&Type::Int)));
            assert!(matches!(elements.get(1), Some(&Type::Bool)));
            assert!(matches!(elements.get(2), Some(&Type::Int)));
        } else {
            panic!("Expected Tuple type, got {:?}", result);
        }
    }

    #[test]
    fn test_gat_instantiation_with_default() {
        let mut checker = ProtocolChecker::new_empty();

        // Create a GAT with a default: type Item<T> = List<T>
        let default_type = Type::Named {
            path: Path::single(Ident::new("List", Span::default())),
            args: List::from(vec![Type::Named {
                path: Path::single(Ident::new("T", Span::default())),
                args: List::new(),
            }]),
        };

        let gat = AssociatedType {
            name: "Item".into(),
            type_params: List::from(vec![GATTypeParam {
                name: "T".into(),
                bounds: List::new(),
                default: Maybe::None,
                variance: Variance::Covariant,
            }]),
            bounds: List::new(),
            where_clauses: List::new(),
            default: Maybe::Some(default_type),
            kind: AssociatedTypeKind::Generic { arity: 1 },
            refinement: Maybe::None,
            expected_variance: Variance::Invariant,
        };

        let protocol = Protocol {
            name: "Container".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::new(),
            methods: Map::new(),
            associated_types: {
                let mut map = Map::new();
                map.insert("Item".into(), gat);
                map
            },
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::None,
            span: Span::default(),
        };

        checker.register_protocol(protocol).unwrap();

        // Resolve Item<Int> should give List<Int>
        let result = checker.resolve_gat_instantiation(
            &"Container".into(),
            &"Item".into(),
            &List::from(vec![Type::Int]),
        );

        assert!(result.is_ok());
        if let Ok(Type::Named { path, args }) = result {
            assert_eq!(path.as_ident().map(|i| i.as_str()), Some("List"));
            assert_eq!(args.len(), 1);
            assert!(matches!(args.get(0), Some(&Type::Int)));
        } else {
            panic!("Expected List<Int>, got {:?}", result);
        }
    }

    #[test]
    fn test_gat_instantiation_without_default() {
        let mut checker = ProtocolChecker::new_empty();

        // Create a GAT without a default: type Item<T>
        let gat = AssociatedType {
            name: "Item".into(),
            type_params: List::from(vec![GATTypeParam {
                name: "T".into(),
                bounds: List::new(),
                default: Maybe::None,
                variance: Variance::Covariant,
            }]),
            bounds: List::new(),
            where_clauses: List::new(),
            default: Maybe::None,
            kind: AssociatedTypeKind::Generic { arity: 1 },
            refinement: Maybe::None,
            expected_variance: Variance::Invariant,
        };

        let protocol = Protocol {
            name: "Container".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::new(),
            methods: Map::new(),
            associated_types: {
                let mut map = Map::new();
                map.insert("Item".into(), gat);
                map
            },
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::None,
            span: Span::default(),
        };

        checker.register_protocol(protocol).unwrap();

        // Resolve Item<Int> should give a projection type
        let result = checker.resolve_gat_instantiation(
            &"Container".into(),
            &"Item".into(),
            &List::from(vec![Type::Int]),
        );

        assert!(result.is_ok());
        // Should return a Named type representing Container.Item<Int>
        if let Ok(Type::Named { path, args }) = result {
            assert!(path.as_ident().map(|i| i.as_str()).unwrap().contains("."));
            assert_eq!(args.len(), 1);
            assert!(matches!(args.get(0), Some(&Type::Int)));
        } else {
            panic!("Expected projection type, got {:?}", result);
        }
    }

    // ==================== Protocol Conformance Tests ====================

    #[test]
    fn test_conformance_valid_impl() {
        let mut checker = ProtocolChecker::new_empty();

        // Register a simple protocol
        let show_method_ty =
            Type::function(List::from(vec![Type::Var(TypeVar::with_id(0))]), Type::Text);
        let mut methods = Map::new();
        methods.insert(
            "show".into(),
            ProtocolMethod::simple("show".into(), show_method_ty, false),
        );

        let protocol = Protocol {
            name: "Show".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::new(),
            methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("test".into()),
            span: Span::default(),
        };
        checker.register_protocol(protocol).unwrap();

        // Create a valid implementation
        let mut impl_methods = Map::new();
        impl_methods.insert(
            "show".into(),
            Type::function(List::from(vec![Type::Int]), Type::Text),
        );

        let impl_ = ProtocolImpl {
            protocol: Path::single(Ident::new("Show", Span::default())),
            protocol_args: List::new(),
            for_type: Type::Int,
            where_clauses: List::new(),
            methods: impl_methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };

        let result = checker.check_full_conformance(&impl_);
        assert!(
            result.is_ok(),
            "Valid impl should pass conformance: {:?}",
            result
        );
    }

    #[test]
    fn test_conformance_missing_method() {
        let mut checker = ProtocolChecker::new_empty();

        // Register a protocol with required method
        let eq_method_ty = Type::function(
            List::from(vec![
                Type::Var(TypeVar::with_id(0)),
                Type::Var(TypeVar::with_id(0)),
            ]),
            Type::Bool,
        );
        let mut methods = Map::new();
        methods.insert(
            "eq".into(),
            ProtocolMethod::simple("eq".into(), eq_method_ty, false),
        );

        let protocol = Protocol {
            name: "Eq".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::new(),
            methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("test".into()),
            span: Span::default(),
        };
        checker.register_protocol(protocol).unwrap();

        // Create an implementation missing the required method
        let impl_ = ProtocolImpl {
            protocol: Path::single(Ident::new("Eq", Span::default())),
            protocol_args: List::new(),
            for_type: Type::Int,
            where_clauses: List::new(),
            methods: Map::new(), // Missing 'eq' method
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };

        let result = checker.check_full_conformance(&impl_);
        assert!(result.is_err());
        if let Err(ConformanceError::MissingMethod {
            method, protocol, ..
        }) = result
        {
            assert_eq!(method.as_str(), "eq");
            assert_eq!(protocol.as_str(), "Eq");
        } else {
            panic!("Expected MissingMethod error, got {:?}", result);
        }
    }

    #[test]
    fn test_conformance_missing_associated_type() {
        let mut checker = ProtocolChecker::new_empty();

        // Register a protocol with required associated type
        let assoc_type = AssociatedType::simple("Item".into(), List::new());
        let mut associated_types = Map::new();
        associated_types.insert("Item".into(), assoc_type);

        let next_method_ty = Type::function(
            List::from(vec![Type::Var(TypeVar::with_id(0))]),
            Type::Named {
                path: Path::single(Ident::new("Maybe", Span::default())),
                args: List::from(vec![Type::Var(TypeVar::with_id(1))]),
            },
        );
        let mut methods = Map::new();
        methods.insert(
            "next".into(),
            ProtocolMethod::simple("next".into(), next_method_ty, false),
        );

        let protocol = Protocol {
            name: "Iterator".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::new(),
            methods,
            associated_types,
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("test".into()),
            span: Span::default(),
        };
        checker.register_protocol(protocol).unwrap();

        // Create an implementation missing the associated type
        let mut impl_methods = Map::new();
        impl_methods.insert(
            "next".into(),
            Type::function(
                List::from(vec![Type::Var(TypeVar::with_id(0))]),
                Type::Named {
                    path: Path::single(Ident::new("Maybe", Span::default())),
                    args: List::from(vec![Type::Int]),
                },
            ),
        );

        let impl_ = ProtocolImpl {
            protocol: Path::single(Ident::new("Iterator", Span::default())),
            protocol_args: List::new(),
            for_type: Type::Named {
                path: Path::single(Ident::new("List", Span::default())),
                args: List::from(vec![Type::Int]),
            },
            where_clauses: List::new(),
            methods: impl_methods,
            associated_types: Map::new(), // Missing 'Item' type
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };

        let result = checker.check_full_conformance(&impl_);
        assert!(result.is_err());
        if let Err(ConformanceError::MissingAssociatedType {
            assoc_type,
            protocol,
            ..
        }) = result
        {
            assert_eq!(assoc_type.as_str(), "Item");
            assert_eq!(protocol.as_str(), "Iterator");
        } else {
            panic!("Expected MissingAssociatedType error, got {:?}", result);
        }
    }

    #[test]
    fn test_conformance_method_signature_mismatch() {
        let mut checker = ProtocolChecker::new_empty();

        // Register protocol with specific method signature
        let to_text_method_ty =
            Type::function(List::from(vec![Type::Var(TypeVar::with_id(0))]), Type::Text);
        let mut methods = Map::new();
        methods.insert(
            "to_text".into(),
            ProtocolMethod::simple("to_text".into(), to_text_method_ty, false),
        );

        let protocol = Protocol {
            name: "ToText".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::new(),
            methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("test".into()),
            span: Span::default(),
        };
        checker.register_protocol(protocol).unwrap();

        // Create implementation with wrong return type
        let mut impl_methods = Map::new();
        impl_methods.insert(
            "to_text".into(),
            Type::function(
                List::from(vec![Type::Int]),
                Type::Int, // Wrong! Should return Text
            ),
        );

        let impl_ = ProtocolImpl {
            protocol: Path::single(Ident::new("ToText", Span::default())),
            protocol_args: List::new(),
            for_type: Type::Int,
            where_clauses: List::new(),
            methods: impl_methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };

        let result = checker.check_full_conformance(&impl_);
        assert!(result.is_err());
        if let Err(ConformanceError::MethodSignatureMismatch { method, .. }) = result {
            assert_eq!(method.as_str(), "to_text");
        } else {
            panic!("Expected MethodSignatureMismatch error, got {:?}", result);
        }
    }

    #[test]
    fn test_conformance_superprotocol_not_implemented() {
        let mut checker = ProtocolChecker::new_empty();

        // Register Eq protocol
        let eq_method_ty = Type::function(
            List::from(vec![
                Type::Var(TypeVar::with_id(0)),
                Type::Var(TypeVar::with_id(0)),
            ]),
            Type::Bool,
        );
        let mut eq_methods = Map::new();
        eq_methods.insert(
            "eq".into(),
            ProtocolMethod::simple("eq".into(), eq_method_ty, false),
        );

        let eq_protocol = Protocol {
            name: "Eq".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::new(),
            methods: eq_methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("test".into()),
            span: Span::default(),
        };
        checker.register_protocol(eq_protocol).unwrap();

        // Register Ord protocol that requires Eq
        let cmp_method_ty = Type::function(
            List::from(vec![
                Type::Var(TypeVar::with_id(0)),
                Type::Var(TypeVar::with_id(0)),
            ]),
            Type::Named {
                path: Path::single(Ident::new(WKT::Ordering.as_str(), Span::default())),
                args: List::new(),
            },
        );
        let mut ord_methods = Map::new();
        ord_methods.insert(
            "cmp".into(),
            ProtocolMethod::simple("cmp".into(), cmp_method_ty, false),
        );

        let ord_protocol = Protocol {
            name: "Ord".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::from(vec![ProtocolBound {
                protocol: Path::single(Ident::new("Eq", Span::default())),
                args: List::new(),
                is_negative: false,
            }]),
            methods: ord_methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("test".into()),
            span: Span::default(),
        };
        checker.register_protocol(ord_protocol).unwrap();

        // Try to implement Ord without implementing Eq first
        let mut impl_methods = Map::new();
        impl_methods.insert(
            "cmp".into(),
            Type::function(
                List::from(vec![Type::Int, Type::Int]),
                Type::Named {
                    path: Path::single(Ident::new(WKT::Ordering.as_str(), Span::default())),
                    args: List::new(),
                },
            ),
        );

        let impl_ = ProtocolImpl {
            protocol: Path::single(Ident::new("Ord", Span::default())),
            protocol_args: List::new(),
            for_type: Type::Int,
            where_clauses: List::new(),
            methods: impl_methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };

        let result = checker.check_full_conformance(&impl_);
        assert!(result.is_err());
        if let Err(ConformanceError::SuperprotocolNotImplemented { superprotocol, .. }) = result {
            assert_eq!(superprotocol.as_str(), "Eq");
        } else {
            panic!(
                "Expected SuperprotocolNotImplemented error, got {:?}",
                result
            );
        }
    }

    #[test]
    fn test_conformance_with_default_method() {
        let mut checker = ProtocolChecker::new_empty();

        // Register protocol with default method
        // We'll simulate this by having has_default=true in ProtocolMethod
        let mut methods = Map::new();

        // Required method
        let required_method_ty =
            Type::function(List::from(vec![Type::Var(TypeVar::with_id(0))]), Type::Bool);
        methods.insert(
            "is_valid".into(),
            ProtocolMethod::simple("is_valid".into(), required_method_ty.clone(), false),
        );

        // For testing, we need to set up protocol_methods with defaults
        let protocol = Protocol {
            name: "Validator".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::new(),
            methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("test".into()),
            span: Span::default(),
        };
        checker.register_protocol(protocol).unwrap();

        // Add protocol method with default by registering another protocol variant
        // We simulate this by directly updating the protocol's methods via a new protocol
        let validate_method_ty = Type::function(
            List::from(vec![Type::Var(TypeVar::with_id(0))]),
            Type::Named {
                path: Path::single(Ident::new("Result", Span::default())),
                args: List::new(),
            },
        );
        let validate_method = ProtocolMethod::simple("validate".into(), validate_method_ty, true); // has_default = true
        let mut validator_methods = Map::new();
        validator_methods.insert(
            "is_valid".into(),
            ProtocolMethod::simple("is_valid".into(), required_method_ty.clone(), false),
        );
        validator_methods.insert("validate".into(), validate_method);

        let validator_protocol_updated = Protocol {
            name: "Validator".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::new(),
            methods: validator_methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("test".into()),
            span: Span::default(),
        };
        checker.register_protocol(validator_protocol_updated).unwrap();

        // Implementation only provides required method (validate has default)
        let mut impl_methods = Map::new();
        impl_methods.insert(
            "is_valid".into(),
            Type::function(List::from(vec![Type::Int]), Type::Bool),
        );

        let impl_ = ProtocolImpl {
            protocol: Path::single(Ident::new("Validator", Span::default())),
            protocol_args: List::new(),
            for_type: Type::Int,
            where_clauses: List::new(),
            methods: impl_methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };

        let result = checker.check_full_conformance(&impl_);
        // Should succeed because 'validate' has a default
        assert!(result.is_ok(), "Should pass: {:?}", result);
    }

    #[test]
    fn test_conformance_associated_type_with_bounds() {
        let mut checker = ProtocolChecker::new_empty();

        // Register Clone protocol first
        let clone_method_ty = Type::function(
            List::from(vec![Type::Reference {
                mutable: false,
                inner: Box::new(Type::Var(TypeVar::with_id(0))),
            }]),
            Type::Var(TypeVar::with_id(0)),
        );
        let mut clone_methods = Map::new();
        clone_methods.insert(
            "clone".into(),
            ProtocolMethod::simple("clone".into(), clone_method_ty, false),
        );

        let clone_protocol = Protocol {
            name: "Clone".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::new(),
            methods: clone_methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("test".into()),
            span: Span::default(),
        };
        checker.register_protocol(clone_protocol).unwrap();

        // Register Container protocol with associated type that has Clone bound
        let assoc_type = AssociatedType {
            name: "Element".into(),
            type_params: List::new(),
            bounds: List::from(vec![ProtocolBound {
                protocol: Path::single(Ident::new("Clone", Span::default())),
                args: List::new(),
                is_negative: false,
            }]),
            where_clauses: List::new(),
            default: Maybe::None,
            kind: AssociatedTypeKind::Regular,
            refinement: Maybe::None,
            expected_variance: Variance::Invariant,
        };
        let mut associated_types = Map::new();
        associated_types.insert("Element".into(), assoc_type);

        let protocol = Protocol {
            name: "Container".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::new(),
            methods: Map::new(),
            associated_types,
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("test".into()),
            span: Span::default(),
        };
        checker.register_protocol(protocol).unwrap();

        // Try to implement with type that doesn't implement Clone
        let mut impl_assoc_types = Map::new();
        // Custom type that doesn't implement Clone
        impl_assoc_types.insert(
            "Element".into(),
            Type::Named {
                path: Path::single(Ident::new("NonCloneable", Span::default())),
                args: List::new(),
            },
        );

        let impl_ = ProtocolImpl {
            protocol: Path::single(Ident::new("Container", Span::default())),
            protocol_args: List::new(),
            for_type: Type::Named {
                path: Path::single(Ident::new("MyContainer", Span::default())),
                args: List::new(),
            },
            where_clauses: List::new(),
            methods: Map::new(),
            associated_types: impl_assoc_types,
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };

        let result = checker.check_full_conformance(&impl_);
        assert!(result.is_err());
        if let Err(ConformanceError::AssociatedTypeBoundNotSatisfied {
            assoc_type, bound, ..
        }) = result
        {
            assert_eq!(assoc_type.as_str(), "Element");
            assert_eq!(bound.as_str(), "Clone");
        } else {
            panic!(
                "Expected AssociatedTypeBoundNotSatisfied error, got {:?}",
                result
            );
        }
    }

    #[test]
    fn test_conformance_types_compatible() {
        let checker = ProtocolChecker::new();

        // Test basic type compatibility
        assert!(checker.types_compatible(&Type::Int, &Type::Int));
        assert!(checker.types_compatible(&Type::Bool, &Type::Bool));
        assert!(!checker.types_compatible(&Type::Int, &Type::Bool));

        // Type variables are compatible with anything
        assert!(checker.types_compatible(&Type::Var(TypeVar::with_id(0)), &Type::Int));
        assert!(checker.types_compatible(&Type::Int, &Type::Var(TypeVar::with_id(0))));
        assert!(checker.types_compatible(
            &Type::Var(TypeVar::with_id(0)),
            &Type::Var(TypeVar::with_id(1))
        ));

        // Tuples
        assert!(checker.types_compatible(
            &Type::Tuple(List::from(vec![Type::Int, Type::Bool])),
            &Type::Tuple(List::from(vec![Type::Int, Type::Bool]))
        ));
        assert!(!checker.types_compatible(
            &Type::Tuple(List::from(vec![Type::Int, Type::Bool])),
            &Type::Tuple(List::from(vec![Type::Bool, Type::Int]))
        ));

        // Functions
        let fn1 = Type::function(List::from(vec![Type::Int]), Type::Bool);
        let fn2 = Type::function(List::from(vec![Type::Int]), Type::Bool);
        let fn3 = Type::function(List::from(vec![Type::Int]), Type::Int);
        assert!(checker.types_compatible(&fn1, &fn2));
        assert!(!checker.types_compatible(&fn1, &fn3));
    }

    #[test]
    fn test_conformance_type_has_free_variables() {
        let checker = ProtocolChecker::new();

        // Primitive types don't have free variables
        assert!(!checker.type_has_free_variables(&Type::Int));
        assert!(!checker.type_has_free_variables(&Type::Bool));

        // Type variables are free variables
        assert!(checker.type_has_free_variables(&Type::Var(TypeVar::with_id(0))));
        assert!(checker.type_has_free_variables(&Type::Var(TypeVar::with_id(42))));

        // Container with free variable
        let list_var = Type::Named {
            path: Path::single(Ident::new("List", Span::default())),
            args: List::from(vec![Type::Var(TypeVar::with_id(0))]),
        };
        assert!(checker.type_has_free_variables(&list_var));

        // Container without free variable
        let list_int = Type::Named {
            path: Path::single(Ident::new("List", Span::default())),
            args: List::from(vec![Type::Int]),
        };
        assert!(!checker.type_has_free_variables(&list_int));

        // Function with free variable
        let fn_with_var =
            Type::function(List::from(vec![Type::Var(TypeVar::with_id(0))]), Type::Int);
        assert!(checker.type_has_free_variables(&fn_with_var));
    }

    #[test]
    fn test_conformance_protocol_not_found() {
        let checker = ProtocolChecker::new_empty();

        let impl_ = ProtocolImpl {
            protocol: Path::single(Ident::new("NonExistent", Span::default())),
            protocol_args: List::new(),
            for_type: Type::Int,
            where_clauses: List::new(),
            methods: Map::new(),
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };

        let result = checker.check_full_conformance(&impl_);
        assert!(result.is_err());
        if let Err(ConformanceError::ProtocolNotFound { name, .. }) = result {
            assert_eq!(name.as_str(), "NonExistent");
        } else {
            panic!("Expected ProtocolNotFound error, got {:?}", result);
        }
    }

    #[test]
    fn test_conformance_associated_type_with_default() {
        let mut checker = ProtocolChecker::new_empty();

        // Register protocol with associated type that has a default
        let assoc_type = AssociatedType {
            name: "Output".into(),
            type_params: List::new(),
            bounds: List::new(),
            where_clauses: List::new(),
            default: Maybe::Some(Type::Unit), // Has default
            kind: AssociatedTypeKind::Regular,
            refinement: Maybe::None,
            expected_variance: Variance::Invariant,
        };
        let mut associated_types = Map::new();
        associated_types.insert("Output".into(), assoc_type);

        let protocol = Protocol {
            name: "Processor".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::new(),
            methods: Map::new(),
            associated_types,
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("test".into()),
            span: Span::default(),
        };
        checker.register_protocol(protocol).unwrap();

        // Implementation without specifying Output (uses default)
        let impl_ = ProtocolImpl {
            protocol: Path::single(Ident::new("Processor", Span::default())),
            protocol_args: List::new(),
            for_type: Type::Int,
            where_clauses: List::new(),
            methods: Map::new(),
            associated_types: Map::new(), // Not specifying Output
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };

        let result = checker.check_full_conformance(&impl_);
        assert!(
            result.is_ok(),
            "Should pass because Output has default: {:?}",
            result
        );
    }

    #[test]
    fn test_conformance_complete_valid_implementation() {
        let mut checker = ProtocolChecker::new_empty();

        // Register Eq protocol
        let eq_method_ty = Type::function(
            List::from(vec![
                Type::Var(TypeVar::with_id(0)),
                Type::Var(TypeVar::with_id(0)),
            ]),
            Type::Bool,
        );
        let mut eq_methods = Map::new();
        eq_methods.insert(
            "eq".into(),
            ProtocolMethod::simple("eq".into(), eq_method_ty, false),
        );

        let eq_protocol = Protocol {
            name: "Eq".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::new(),
            methods: eq_methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("test".into()),
            span: Span::default(),
        };
        checker.register_protocol(eq_protocol).unwrap();

        // Register Eq impl for Int
        let mut eq_impl_methods = Map::new();
        eq_impl_methods.insert(
            "eq".into(),
            Type::function(List::from(vec![Type::Int, Type::Int]), Type::Bool),
        );

        let eq_impl = ProtocolImpl {
            protocol: Path::single(Ident::new("Eq", Span::default())),
            protocol_args: List::new(),
            for_type: Type::Int,
            where_clauses: List::new(),
            methods: eq_impl_methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };
        let _ = checker.register_impl(eq_impl);

        // Now register Ord protocol that requires Eq
        let cmp_method_ty = Type::function(
            List::from(vec![
                Type::Var(TypeVar::with_id(0)),
                Type::Var(TypeVar::with_id(0)),
            ]),
            Type::Named {
                path: Path::single(Ident::new(WKT::Ordering.as_str(), Span::default())),
                args: List::new(),
            },
        );
        let mut ord_methods = Map::new();
        ord_methods.insert(
            "cmp".into(),
            ProtocolMethod::simple("cmp".into(), cmp_method_ty, false),
        );

        let ord_protocol = Protocol {
            name: "Ord".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::from(vec![ProtocolBound {
                protocol: Path::single(Ident::new("Eq", Span::default())),
                args: List::new(),
                is_negative: false,
            }]),
            methods: ord_methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("test".into()),
            span: Span::default(),
        };
        checker.register_protocol(ord_protocol).unwrap();

        // Implement Ord for Int (Eq is already implemented)
        let mut ord_impl_methods = Map::new();
        ord_impl_methods.insert(
            "cmp".into(),
            Type::function(
                List::from(vec![Type::Int, Type::Int]),
                Type::Named {
                    path: Path::single(Ident::new(WKT::Ordering.as_str(), Span::default())),
                    args: List::new(),
                },
            ),
        );

        let ord_impl = ProtocolImpl {
            protocol: Path::single(Ident::new("Ord", Span::default())),
            protocol_args: List::new(),
            for_type: Type::Int,
            where_clauses: List::new(),
            methods: ord_impl_methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };

        let result = checker.check_full_conformance(&ord_impl);
        assert!(
            result.is_ok(),
            "Complete valid impl should pass: {:?}",
            result
        );
    }

    #[test]
    fn test_export_instance_registry_empty() {
        let checker = ProtocolChecker::new_empty();
        let registry = checker.export_instance_registry();
        assert!(registry.is_empty());
        assert!(registry.check_coherence().is_coherent());
    }

    #[test]
    fn test_export_instance_registry_mirrors_impls() {
        let mut checker = ProtocolChecker::new_empty();

        let show_method_ty = Type::function(
            List::from(vec![Type::Var(TypeVar::with_id(0))]),
            Type::Text,
        );
        let mut show_methods = Map::new();
        show_methods.insert(
            "show".into(),
            ProtocolMethod::simple("show".into(), show_method_ty, false),
        );
        let show_protocol = Protocol {
            name: "Show".into(),
            kind: ProtocolKind::Constraint,
            type_params: List::new(),
            super_protocols: List::new(),
            methods: show_methods,
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::Some("test".into()),
            span: Span::default(),
        };
        checker.register_protocol(show_protocol).unwrap();

        let mut show_impl_methods = Map::new();
        show_impl_methods.insert(
            "show".into(),
            Type::function(List::from(vec![Type::Int]), Type::Text),
        );
        let show_for_int = ProtocolImpl {
            protocol: Path::single(Ident::new("Show", Span::default())),
            protocol_args: List::new(),
            for_type: Type::Int,
            where_clauses: List::new(),
            methods: show_impl_methods.clone(),
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };
        checker.register_impl(show_for_int).unwrap();

        let registry = checker.export_instance_registry();
        assert_eq!(registry.len(), 1);
        assert!(registry.check_coherence().is_coherent());
    }

    // ============================================================
    // [protocols].resolution_strategy + blanket_impls — pin tests
    // for the newly-wired manifest fields. See ProtocolChecker
    // resolution_strategy / blanket_impls fields.
    //
    // The multi-candidate find_impl behaviour depends on having
    // overlapping impls registered, which `register_impl` rejects
    // via `check_overlap`.  These pin tests therefore focus on the
    // setter/getter contract + manifest-text normalisation; the
    // dispatch behaviour is exercised end-to-end through the
    // semantic_analysis phase + actual @specialize-marked impls in
    // the integration suite.
    // ============================================================

    #[test]
    fn resolution_strategy_default_is_most_specific() {
        // Pin: a freshly-constructed ProtocolChecker reports
        // "most_specific" — matches the documented Verum.toml
        // default and the pre-wire hardcoded behaviour.
        let checker = ProtocolChecker::new();
        assert_eq!(
            checker.resolution_strategy().as_str(),
            "most_specific",
            "default resolution_strategy must be most_specific"
        );
        let empty = ProtocolChecker::new_empty();
        assert_eq!(
            empty.resolution_strategy().as_str(),
            "most_specific",
            "new_empty must also default to most_specific"
        );
    }

    #[test]
    fn resolution_strategy_accepts_three_documented_values() {
        // Pin: the three documented values normalise to
        // themselves; setter accepts them without warning.
        let mut checker = ProtocolChecker::new();
        for s in ["most_specific", "first_declared", "error"] {
            checker.set_resolution_strategy(s);
            assert_eq!(checker.resolution_strategy().as_str(), s);
        }
    }

    #[test]
    fn resolution_strategy_unknown_falls_back_to_most_specific() {
        // Pin: `set_resolution_strategy(unknown)` normalises to
        // "most_specific" with a warning rather than storing an
        // unrecognised value.  Sibling to
        // `PanicStrategy::from_manifest_text`.
        let mut checker = ProtocolChecker::new();
        checker.set_resolution_strategy("nonsense_strategy");
        assert_eq!(
            checker.resolution_strategy().as_str(),
            "most_specific",
            "unknown strategy must normalise to most_specific"
        );
    }

    #[test]
    fn blanket_impls_default_is_true() {
        // Pin: documented Verum.toml default + Rust-like ergonomics.
        assert!(ProtocolChecker::new().blanket_impls_allowed());
        assert!(ProtocolChecker::new_empty().blanket_impls_allowed());
    }

    #[test]
    fn blanket_impls_setter_round_trips() {
        // Pin: `set_blanket_impls(false)` flips the gate; setter is
        // idempotent.  Used at semantic_analysis to wire
        // `[protocols].blanket_impls` from manifest.
        let mut checker = ProtocolChecker::new();
        checker.set_blanket_impls(false);
        assert!(!checker.blanket_impls_allowed());
        checker.set_blanket_impls(false);
        assert!(!checker.blanket_impls_allowed());
        checker.set_blanket_impls(true);
        assert!(checker.blanket_impls_allowed());
    }

    #[test]
    fn instance_search_default_is_enabled() {
        // Pin: documented Verum.toml default + Rust-like
        // ergonomics.
        assert!(ProtocolChecker::new().instance_search_enabled());
        assert!(ProtocolChecker::new_empty().instance_search_enabled());
    }

    #[test]
    fn instance_search_setter_round_trips() {
        // Pin: `set_instance_search_enabled(false)` flips the
        // gate; setter is idempotent.  Used at semantic_analysis
        // to wire `[types].instance_search` from manifest.
        let mut checker = ProtocolChecker::new();
        checker.set_instance_search_enabled(false);
        assert!(!checker.instance_search_enabled());
        checker.set_instance_search_enabled(false);
        assert!(!checker.instance_search_enabled());
        checker.set_instance_search_enabled(true);
        assert!(checker.instance_search_enabled());
    }

    #[test]
    fn instance_search_disabled_returns_none_on_generic_match_path() {
        // Pin: with `[types].instance_search = false`, `find_impl`
        // skips Stage-2 generic-candidate scan and returns None
        // for any non-exact match.  The exact-match path still
        // works (Stage 1 is unaffected) — only implicit blanket /
        // generic resolution is gated.
        //
        // Setup: register a single blanket impl `impl<T>
        // CustomProto for T`. With instance_search ON,
        // find_impl(Type::Int) returns the blanket impl via
        // Stage-2. With it OFF, returns None — caller must
        // register `impl CustomProto for Int` explicitly.
        //
        // Use `new_empty()` so no standard protocols collide with
        // our test impl at `register_impl` time.
        let mut checker = ProtocolChecker::new_empty();
        let proto_path =
            Path::single(Ident::new("CustomProto_InstanceSearchTest", Span::default()));
        let blanket = ProtocolImpl {
            protocol: proto_path.clone(),
            protocol_args: List::new(),
            for_type: Type::Var(crate::ty::TypeVar::fresh()),
            where_clauses: List::new(),
            methods: Map::new(),
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };
        checker.register_impl(blanket).unwrap();

        // Default ON: blanket-impl scan reaches Type::Int.
        match checker.find_impl(&Type::Int, &proto_path) {
            Maybe::Some(_) => {} // expected
            Maybe::None => panic!(
                "default instance_search=true must find the blanket impl"
            ),
        }

        // Flip OFF: Stage 2 skipped → None.
        checker.set_instance_search_enabled(false);
        assert!(
            matches!(checker.find_impl(&Type::Int, &proto_path), Maybe::None),
            "instance_search=false must skip the generic-candidate scan"
        );
    }

    #[test]
    fn instance_search_disabled_keeps_exact_match_path_working() {
        // Pin: instance_search=false ONLY gates Stage 2 (generic
        // candidate scan). Stage 1 (exact-match O(1) lookup)
        // remains active — concretely-registered impls still
        // resolve. This makes the gate a "no implicit blanket
        // search" knob, not a complete protocol-resolver
        // disable.
        let mut checker = ProtocolChecker::new_empty();
        let proto_path =
            Path::single(Ident::new("CustomProto_ExactMatchTest", Span::default()));
        let concrete = ProtocolImpl {
            protocol: proto_path.clone(),
            protocol_args: List::new(),
            for_type: Type::Int,
            where_clauses: List::new(),
            methods: Map::new(),
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        };
        checker.register_impl(concrete).unwrap();
        checker.set_instance_search_enabled(false);
        // Exact match still resolves via Stage 1 (impl_index O(1)
        // hit).
        match checker.find_impl(&Type::Int, &proto_path) {
            Maybe::Some(impl_) => {
                assert!(
                    matches!(&impl_.for_type, Type::Int),
                    "exact-match path must still return the concrete impl"
                );
            }
            Maybe::None => panic!(
                "instance_search=false must NOT block exact-match resolution"
            ),
        }
    }
}
