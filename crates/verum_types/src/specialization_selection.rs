//! Specialization Selection for Type Inference
//!
//! Advanced protocols (future v2.0+): GATs, higher-rank bounds, specialization with lattice ordering, coherence rules — Section 9.1 - Automatic Specialization Selection
//!
//! Implements automatic selection of the most specific protocol implementation during
//! type inference. When multiple implementations exist for a protocol, this module
//! uses the specialization lattice to select the most appropriate one.
//!
//! # Core Algorithm
//!
//! 1. **Find Candidates**: Identify all implementations that could apply to the type
//! 2. **Check Constraints**: Verify where clauses and protocol bounds are satisfied
//! 3. **Rank by Lattice**: Order candidates by specialization specificity
//! 4. **Check Ambiguity**: Ensure exactly one maximal element exists
//! 5. **Cache Result**: Store decision for fast subsequent lookups
//!
//! # Performance
//!
//! - Selection (uncached): <5ms
//! - Selection (cached): <1ms
//! - Lattice construction: <50ms (one-time per protocol)
//! - Coherence checking: <100ms (compile-time only)
//!
//! # Example
//!
//! ```verum
//! // Default implementation
//! implement<T> Display for T {
//!     fn display(self) -> Text { "..." }
//! }
//!
//! // Specialized for Int
//! @specialize
//! implement Display for Int {
//!     fn display(self) -> Text { format!("{}", self) }
//! }
//!
//! fn show<T: Display>(x: T) {
//!     x.display()  // Select appropriate impl based on T
//! }
//!
//! show(42);        // Uses Int specialization
//! show("hello");   // Uses default implementation
//! ```

use thiserror::Error;
use verum_ast::span::Span;
use verum_ast::ty::{Path, PathSegment, TypeBound, TypeBoundKind};
use verum_common::{List, Map, Maybe, Set, Text};
use verum_common::well_known_types::WellKnownType as WKT;
use verum_common::well_known_types::WellKnownProtocol as WKP;
use verum_common::primitive_implements_protocol;

use crate::TypeError;
use crate::advanced_protocols::{SpecializationInfo, SpecializationLattice};
use crate::protocol::{Protocol, ProtocolBound, ProtocolChecker, ProtocolImpl, WhereClause};
use crate::ty::{Type, TypeVar};
use crate::unify::Unifier;

// ==================== Core Types ====================

/// Implementation identifier (index into global impl list)
pub type ImplementId = usize;

/// Specialization selection error
#[derive(Debug, Clone, Error)]
pub enum SpecializationError {
    /// Multiple maximal implementations (ambiguity)
    #[error("ambiguous specialization: multiple implementations apply")]
    Ambiguous {
        candidates: List<ImplementId>,
        protocol: Text,
        self_type: Type,
        suggestion: Text,
    },

    /// Overlapping implementations without specialization relationship
    #[error("overlapping implementations without specialization")]
    Overlap {
        impl1_id: ImplementId,
        impl2_id: ImplementId,
        protocol: Text,
        suggestion: Text,
    },

    /// No applicable implementation found
    #[error("no implementation found for protocol {protocol} on type {self_type}")]
    NoApplicableImpl {
        protocol: Text,
        self_type: Type,
        suggestion: Text,
    },

    /// Coherence violation (negative specialization)
    #[error("coherence violation in negative specialization")]
    CoherenceViolation { impl_id: ImplementId, reason: Text },

    /// Negative bound violated - type implements a protocol it should not
    ///
    /// Multi-protocol bounds: combining multiple protocol constraints (T: Display + Debug) — Negative Bounds
    ///
    /// This error occurs when a type T has a bound `T: !Protocol` but T actually
    /// implements Protocol. For example:
    /// ```verum
    /// implement<T: Clone + !Copy> DeepClone for T { ... }
    /// // Applied to Int (which is Copy) - ERROR!
    /// ```
    #[error("negative bound violated: {ty} implements {protocol}")]
    NegativeBoundViolated {
        /// The type that violated the negative bound
        ty: Type,
        /// The protocol that should NOT be implemented
        protocol: Text,
        /// The impl id where the negative bound is declared
        impl_id: ImplementId,
        /// Span for error reporting
        span: Maybe<Span>,
    },

    /// Implementation status unknown for negative bound check
    ///
    /// When checking a generic type variable against a negative bound,
    /// we cannot determine if it satisfies the bound until instantiation.
    #[error("cannot verify negative bound for unresolved type")]
    NegativeBoundUnknown {
        /// The type variable being checked
        ty: Type,
        /// The protocol in the negative bound
        protocol: Text,
    },
}

impl SpecializationError {
    /// Convert to TypeError for reporting
    pub fn to_type_error(self, span: Span) -> TypeError {
        match self {
            SpecializationError::Ambiguous {
                candidates,
                protocol,
                self_type,
                suggestion,
            } => {
                let candidates_str = candidates
                    .iter()
                    .map(|id| format!("impl #{}", id))
                    .collect::<Vec<_>>()
                    .join(", ");
                let type_str = match &self_type {
                    Type::Named { path, .. } => path.to_string(),
                    Type::Var(v) => format!("type variable {}", v.id()),
                    _ => "unknown type".to_string(),
                };
                TypeError::Other(
                    format!(
                        "ambiguous method call: protocol `{}` has multiple applicable \
                         implementations for type `{}`\n  \
                         candidates: {}\n  \
                         help: {}",
                        protocol, type_str, candidates_str, suggestion
                    )
                    .into(),
                )
            }
            SpecializationError::Overlap {
                impl1_id,
                impl2_id,
                protocol,
                suggestion,
            } => TypeError::Other(
                format!(
                    "overlapping implementations of protocol `{}`\n  \
                     impl #{} and impl #{} overlap without specialization relationship\n  \
                     help: {}",
                    protocol, impl1_id, impl2_id, suggestion
                )
                .into(),
            ),
            SpecializationError::NoApplicableImpl {
                protocol,
                self_type,
                suggestion,
            } => {
                let type_str = match &self_type {
                    Type::Named { path, .. } => path.to_string(),
                    Type::Var(v) => format!("type variable {}", v.id()),
                    _ => "unknown type".to_string(),
                };
                TypeError::ProtocolNotSatisfied {
                    ty: type_str.into(),
                    protocol,
                    span,
                }
            }
            SpecializationError::CoherenceViolation { impl_id, reason } => TypeError::Other(
                format!(
                    "coherence violation in impl #{}\n  reason: {}",
                    impl_id, reason
                )
                .into(),
            ),
            SpecializationError::NegativeBoundViolated {
                ty,
                protocol,
                impl_id,
                span: error_span,
            } => {
                let type_str = match &ty {
                    Type::Named { path, .. } => path.to_string(),
                    Type::Var(v) => format!("type variable {}", v.id()),
                    _ => format!("{:?}", ty),
                };
                TypeError::NegativeBoundViolated {
                    ty: type_str.into(),
                    protocol,
                    span: error_span.unwrap_or(span),
                }
            }
            SpecializationError::NegativeBoundUnknown { ty, protocol } => {
                let type_str = match &ty {
                    Type::Var(v) => format!("type variable {}", v.id()),
                    _ => format!("{:?}", ty),
                };
                TypeError::Other(
                    format!(
                        "cannot verify negative bound `!{}` for unresolved type `{}`\n  \
                         help: the type variable must be instantiated before checking negative bounds",
                        protocol, type_str
                    )
                    .into(),
                )
            }
        }
    }
}

// ==================== Negative Bound Result ====================

/// Result of checking a negative bound
///
/// Multi-protocol bounds: combining multiple protocol constraints (T: Display + Debug) — Negative Bounds
///
/// Used to represent the three possible outcomes when checking if a type
/// satisfies a negative bound (T: !Protocol):
///
/// - `Satisfied`: Type does NOT implement the protocol (bound satisfied)
/// - `Violated`: Type DOES implement the protocol (bound violated)
/// - `Unknown`: Cannot determine (type variable or unknown type)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NegativeBoundResult {
    /// Type does NOT implement the protocol (bound satisfied)
    ///
    /// Example: `Text: !Copy` is Satisfied because Text is not Copy
    Satisfied,

    /// Type DOES implement the protocol (bound violated)
    ///
    /// Example: `Int: !Copy` is Violated because Int IS Copy
    Violated,

    /// Cannot determine implementation status
    ///
    /// This occurs when the type is a type variable or otherwise unresolved.
    /// The bound will be checked again when the type is instantiated.
    ///
    /// Example: `T: !Copy` where T is a type parameter is Unknown
    Unknown,
}

// ==================== Specialization Selector ====================

/// Selects the most specific protocol implementation during type inference
pub struct SpecializationSelector {
    /// Cache: (protocol_name, type) -> selected implementation
    pub cache: Map<(Text, Text), ImplementId>,

    /// Specialization lattices for each protocol
    /// Map: protocol_name -> lattice
    pub lattices: Map<Text, SpecializationLattice>,

    /// Statistics for performance monitoring
    stats: SelectionStats,
}

/// Performance statistics for specialization selection
#[derive(Debug, Clone, Default)]
pub struct SelectionStats {
    /// Number of cache hits
    pub cache_hits: usize,
    /// Number of cache misses
    pub cache_misses: usize,
    /// Number of selections performed
    pub selections: usize,
    /// Total time spent in microseconds
    pub time_us: u64,
}

impl SpecializationSelector {
    /// Create a new specialization selector
    pub fn new() -> Self {
        Self {
            cache: Map::new(),
            lattices: Map::new(),
            stats: SelectionStats::default(),
        }
    }

    /// Select the most specific implementation for a protocol on a type
    ///
    /// This is the main entry point for specialization selection.
    ///
    /// # Algorithm
    ///
    /// 1. Check cache for previous decision
    /// 2. Find all candidate implementations
    /// 3. Rank candidates by specialization lattice
    /// 4. Verify no ambiguity (single maximal element)
    /// 5. Cache and return result
    pub fn select_implementation(
        &mut self,
        protocol: &Protocol,
        self_type: &Type,
        protocol_checker: &ProtocolChecker,
        unifier: &mut Unifier,
    ) -> Result<ImplementId, SpecializationError> {
        let start = std::time::Instant::now();
        self.stats.selections += 1;

        // 1. Check cache
        let type_key = self.type_to_cache_key(self_type);
        if let Some(&impl_id) = self.cache.get(&(protocol.name.clone(), type_key.clone())) {
            self.stats.cache_hits += 1;
            return Ok(impl_id);
        }

        self.stats.cache_misses += 1;

        // 2. Find all candidate implementations
        let candidates = self.find_candidates(protocol, self_type, protocol_checker, unifier)?;

        if candidates.is_empty() {
            return Err(SpecializationError::NoApplicableImpl {
                protocol: protocol.name.clone(),
                self_type: self_type.clone(),
                suggestion: {
                    let type_str = match self_type {
                        Type::Named { path, .. } => path.to_string(),
                        Type::Var(v) => format!("type variable {}", v.id()),
                        _ => "the target type".to_string(),
                    };
                    format!(
                        "implement protocol `{}` for type `{}`",
                        protocol.name, type_str
                    )
                    .into()
                },
            });
        }

        // 3. Get or build specialization lattice
        let lattice = self
            .get_or_build_lattice(protocol, protocol_checker)
            .clone();

        // 4. Rank candidates by lattice
        let ranked = self.rank_by_lattice(&candidates, &lattice)?;

        // 5. Check for ambiguity (multiple maximal elements)
        if ranked.len() > 1 {
            return Err(SpecializationError::Ambiguous {
                candidates: ranked.clone(),
                protocol: protocol.name.clone(),
                self_type: self_type.clone(),
                suggestion: "add more specific type constraints or use explicit type annotations"
                    .into(),
            });
        }

        let selected = ranked[0];

        // 6. Cache result
        self.cache_selection(protocol.name.clone(), type_key, selected);

        self.stats.time_us += start.elapsed().as_micros() as u64;

        Ok(selected)
    }

    /// Find all implementations that could apply to the given type
    fn find_candidates(
        &self,
        protocol: &Protocol,
        self_type: &Type,
        protocol_checker: &ProtocolChecker,
        unifier: &mut Unifier,
    ) -> Result<List<ImplementId>, SpecializationError> {
        let mut candidates = List::new();

        // Get all implementations for this protocol
        let impls = protocol_checker.get_implementations_for_protocol(&protocol.name);

        for (impl_id, impl_info) in impls.iter().enumerate() {
            // Check if self_type matches impl pattern
            if self.matches_impl_pattern(self_type, &impl_info.for_type, unifier) {
                // Verify where clauses are satisfied
                if self.check_where_clauses(self_type, &impl_info.where_clauses, protocol_checker) {
                    // Check negative specialization constraints
                    if self.check_negative_constraints(impl_info, protocol_checker) {
                        candidates.push(impl_id);
                    }
                }
            }
        }

        Ok(candidates)
    }

    /// Check if a type matches an implementation pattern
    pub fn matches_impl_pattern(
        &self,
        self_type: &Type,
        pattern: &Type,
        unifier: &mut Unifier,
    ) -> bool {
        // Try to unify self_type with the impl's for_type
        // This handles cases like:
        // - impl for Int matches Int
        // - impl<T> for List<T> matches List<Int>
        // - impl<T: Clone> for T matches any Clone type

        match (self_type, pattern) {
            // Exact type match
            (Type::Named { path: p1, .. }, Type::Named { path: p2, .. }) if p1 == p2 => true,

            // Type variable in pattern (generic impl)
            (_, Type::Var(_)) => true,

            // Compound types (recursive check)
            (
                Type::Named {
                    path: p1,
                    args: args1,
                },
                Type::Named {
                    path: p2,
                    args: args2,
                },
            ) if p1 == p2 => {
                args1.len() == args2.len()
                    && args1
                        .iter()
                        .zip(args2.iter())
                        .all(|(a1, a2)| self.matches_impl_pattern(a1, a2, unifier))
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
                p1.len() == p2.len()
                    && p1
                        .iter()
                        .zip(p2.iter())
                        .all(|(param1, param2)| self.matches_impl_pattern(param1, param2, unifier))
                    && self.matches_impl_pattern(r1.as_ref(), r2.as_ref(), unifier)
            }

            // Reference types
            (Type::Reference { inner: t1, .. }, Type::Reference { inner: t2, .. }) => {
                self.matches_impl_pattern(t1.as_ref(), t2.as_ref(), unifier)
            }

            _ => false,
        }
    }

    /// Check if where clauses are satisfied for a type
    ///
    /// Handles both positive bounds (T: Protocol) and negative bounds (T: !Protocol).
    ///
    /// Multi-protocol bounds: combining multiple protocol constraints (T: Display + Debug) — Negative Bounds
    fn check_where_clauses(
        &self,
        self_type: &Type,
        where_clauses: &[WhereClause],
        protocol_checker: &ProtocolChecker,
    ) -> bool {
        for clause in where_clauses {
            // Check if the type satisfies all bounds in the where clause
            for bound in &clause.bounds {
                // Check negative bounds differently from positive bounds
                if bound.is_negative {
                    // For negative bounds (T: !Protocol), verify T does NOT implement Protocol
                    match self.check_negative_bound(&clause.ty, bound, protocol_checker) {
                        NegativeBoundResult::Satisfied => {
                            // Type correctly does NOT implement the protocol
                            continue;
                        }
                        NegativeBoundResult::Violated => {
                            // Type DOES implement the protocol - bound violated
                            return false;
                        }
                        NegativeBoundResult::Unknown => {
                            // For generic type variables, we're conservative - assume satisfied
                            // The bound will be checked again when the type is instantiated
                            continue;
                        }
                    }
                } else {
                    // For positive bounds, use standard check
                    if !protocol_checker.check_protocol_bound(self_type, bound) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Check if a type satisfies a negative bound (T: !Protocol)
    ///
    /// Multi-protocol bounds: combining multiple protocol constraints (T: Display + Debug) — Negative Bounds
    ///
    /// # Algorithm
    ///
    /// 1. For concrete types: Check if there's an implementation of Protocol for T
    ///    - If yes -> Violated (type DOES implement)
    ///    - If no  -> Satisfied (type does NOT implement)
    ///
    /// 2. For type variables: Cannot determine until instantiation
    ///    - Return Unknown, defer checking to instantiation time
    ///
    /// 3. For built-in types: Check against known implementations
    ///
    /// # Examples
    ///
    /// ```verum
    /// // T: !Copy
    /// // For T = Text -> Satisfied (Text is not Copy)
    /// // For T = Int  -> Violated (Int is Copy)
    /// // For T = ?V   -> Unknown (type variable)
    /// ```
    pub fn check_negative_bound(
        &self,
        ty: &Type,
        bound: &ProtocolBound,
        protocol_checker: &ProtocolChecker,
    ) -> NegativeBoundResult {
        // Extract the protocol name from the bound
        let protocol_name = self.protocol_path_to_string(&bound.protocol);

        // Handle different type cases
        match ty {
            // Type variables: cannot determine until instantiation
            Type::Var(_) => NegativeBoundResult::Unknown,

            // Generic types with unresolved parameters
            Type::Generic { .. } => NegativeBoundResult::Unknown,

            // Concrete named types: check for implementation
            Type::Named { path, args } => {
                // Check if there's an implementation of the protocol for this type
                let type_implements = self.concrete_type_implements_protocol(
                    ty,
                    &protocol_name,
                    protocol_checker,
                );

                if type_implements {
                    NegativeBoundResult::Violated
                } else {
                    NegativeBoundResult::Satisfied
                }
            }

            // Built-in types have known protocol implementations
            Type::Int | Type::Float | Type::Bool | Type::Char => {
                // Check built-in protocol implementations
                let type_implements = self.builtin_implements_protocol(ty, &protocol_name);
                if type_implements {
                    NegativeBoundResult::Violated
                } else {
                    NegativeBoundResult::Satisfied
                }
            }

            Type::Text => {
                // Text has specific protocol implementations (Clone, Eq, Ord, Hash)
                // but NOT Copy
                let type_implements = self.builtin_implements_protocol(ty, &protocol_name);
                if type_implements {
                    NegativeBoundResult::Violated
                } else {
                    NegativeBoundResult::Satisfied
                }
            }

            Type::Unit => {
                // Unit implements Copy, Clone, Eq, etc.
                let type_implements = self.builtin_implements_protocol(ty, &protocol_name);
                if type_implements {
                    NegativeBoundResult::Violated
                } else {
                    NegativeBoundResult::Satisfied
                }
            }

            // Reference types: check inner type
            Type::Reference { inner, .. } => {
                // References have specific protocol implementations
                // &T: Copy if T: Sized (always true for concrete types)
                // &mut T: !Copy (mutable references are not Copy)
                self.check_negative_bound(inner, bound, protocol_checker)
            }

            // Tuple types: check component types
            Type::Tuple(elems) => {
                // Tuples implement protocols if all elements do
                // For negative bounds, check each element
                for elem in elems.iter() {
                    match self.check_negative_bound(elem, bound, protocol_checker) {
                        NegativeBoundResult::Violated => {
                            // If any element implements the protocol, the tuple might too
                            // (depending on the protocol's derivation rules)
                        }
                        NegativeBoundResult::Unknown => return NegativeBoundResult::Unknown,
                        NegativeBoundResult::Satisfied => {}
                    }
                }
                // All elements checked - check the tuple type itself
                let type_implements = self.concrete_type_implements_protocol(
                    ty,
                    &protocol_name,
                    protocol_checker,
                );
                if type_implements {
                    NegativeBoundResult::Violated
                } else {
                    NegativeBoundResult::Satisfied
                }
            }

            // Function types: typically don't implement most protocols
            Type::Function { .. } => {
                // Functions implement Fn traits, but rarely Copy/Clone
                if matches!(protocol_name.as_str(), "Fn" | "FnOnce" | "FnMut") {
                    NegativeBoundResult::Violated
                } else {
                    NegativeBoundResult::Satisfied
                }
            }

            // Array types: check element type
            Type::Array { element, .. } => {
                self.check_negative_bound(element, bound, protocol_checker)
            }

            // Slice types: similar to arrays
            Type::Slice { element } => {
                self.check_negative_bound(element, bound, protocol_checker)
            }

            // Never type: implements everything vacuously
            Type::Never => NegativeBoundResult::Violated,

            // Pointer types: have specific implementations
            Type::Pointer { .. } => {
                // Raw pointers implement Copy/Clone but not much else
                if WKP::from_name(protocol_name.as_str())
                    .is_some_and(|p| matches!(p, WKP::Copy | WKP::Clone))
                {
                    NegativeBoundResult::Violated
                } else {
                    NegativeBoundResult::Satisfied
                }
            }

            // Future types
            Type::Future { .. } => {
                // Futures typically implement specific async protocols
                if WKP::from_name(protocol_name.as_str())
                    .is_some_and(|p| matches!(p, WKP::Future))
                {
                    NegativeBoundResult::Violated
                } else {
                    NegativeBoundResult::Satisfied
                }
            }

            // Refinement types: check base type
            Type::Refined { base, .. } => {
                self.check_negative_bound(base, bound, protocol_checker)
            }

            // Existential types: unknown until opened
            Type::Exists { .. } => NegativeBoundResult::Unknown,

            // Default case: unknown
            _ => NegativeBoundResult::Unknown,
        }
    }

    /// Check if a built-in type implements a protocol.
    ///
    /// Delegates to the centralized `primitive_implements_protocol` registry in
    /// `verum_common`, which is the single source of truth for primitive type
    /// protocol implementations.
    fn builtin_implements_protocol(&self, ty: &Type, protocol: &Text) -> bool {
        let type_name = match ty.primitive_name() {
            Some(name) => name,
            None => return false,
        };
        primitive_implements_protocol(type_name, protocol.as_str()).unwrap_or(false)
    }

    /// Check if a concrete type implements a protocol by querying the protocol checker
    fn concrete_type_implements_protocol(
        &self,
        ty: &Type,
        protocol_name: &Text,
        protocol_checker: &ProtocolChecker,
    ) -> bool {
        // First check built-in implementations
        if self.builtin_implements_protocol(ty, protocol_name) {
            return true;
        }

        // Then check registered implementations
        protocol_checker
            .check_protocol_satisfied(ty, protocol_name)
            .unwrap_or(false)
    }

    /// Check negative specialization constraints
    ///
    /// For @specialize(negative), verify that excluded bounds are NOT satisfied
    ///
    /// # Specification
    ///
    /// Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — .4 lines 623-638
    ///
    /// Negative specialization allows implementations for types that DON'T satisfy
    /// certain bounds. For example:
    /// ```verum
    /// @specialize(negative)
    /// implement<T: !Clone> MyProtocol for List<T> { }
    /// ```
    ///
    /// This implementation applies ONLY when T does NOT implement Clone.
    ///
    /// # Algorithm
    ///
    /// 1. Check if implementation has specialization metadata
    /// 2. If specialization is marked as `is_default`, check for negative bounds
    /// 3. For each where clause, check for negative protocol bounds (T: !Protocol)
    /// 4. Verify that the type does NOT satisfy the negative bounds
    /// 5. Return false if any negative bound IS satisfied (impl should not apply)
    ///
    /// # Returns
    ///
    /// - `true`: Implementation is valid and should be considered
    /// - `false`: Implementation should be excluded (negative bound satisfied)
    fn check_negative_constraints(
        &self,
        impl_info: &ProtocolImpl,
        protocol_checker: &ProtocolChecker,
    ) -> bool {
        // Check if this is a specialization
        let spec_info = match &impl_info.specialization {
            Maybe::Some(info) => info,
            Maybe::None => return true, // No specialization, allow by default
        };

        // For negative specialization, check if it's marked as default
        // Default implementations can have negative bounds
        if !spec_info.is_default {
            return true; // Not a default impl, no negative checking needed
        }

        // Check all where clauses for negative bounds
        for where_clause in impl_info.where_clauses.iter() {
            // Extract the type being constrained
            let constrained_type = &where_clause.ty;

            // Check each bound in this where clause
            for bound in where_clause.bounds.iter() {
                // Check if this is a negative bound by examining the protocol path
                // Negative bounds are indicated by special marker or bound type
                if self.is_negative_bound(bound) {
                    // Extract the protocol from the negative bound
                    let protocol_path = &bound.protocol;

                    // Convert ProtocolBound to a checkable format
                    // For negative bounds, we need to verify the type does NOT satisfy it
                    let type_satisfies_protocol = self.check_type_satisfies_protocol(
                        constrained_type,
                        protocol_path,
                        protocol_checker,
                    );

                    if type_satisfies_protocol {
                        // Type DOES satisfy the negative bound (!Protocol)
                        // This means the negative impl should NOT apply
                        return false;
                    }
                }
            }
        }

        // Additionally check for negative bounds in AST TypeBound format
        // This handles where clauses that may use the AST TypeBound structure
        if let Maybe::Some(ref_type) = self.extract_impl_type_for_negative_check(impl_info)
            && let Some(negative_bounds) = self.extract_negative_type_bounds(&ref_type)
        {
            for neg_bound in negative_bounds.iter() {
                // Check if the impl's for_type satisfies this negative bound
                if self.check_negative_type_bound(&impl_info.for_type, neg_bound, protocol_checker)
                {
                    // Type satisfies the bound that should be negated
                    return false;
                }
            }
        }

        // All negative constraints are satisfied (type does NOT satisfy negative bounds)
        true
    }

    /// Check if a ProtocolBound represents a negative bound
    ///
    /// Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — .4 - Negative Reasoning
    ///
    /// Negative bounds are indicated in two ways:
    /// 1. The ProtocolBound's `is_negative` flag (preferred, set by parser)
    /// 2. Legacy: protocol path starting with '!' (for backward compatibility)
    ///
    /// Negative bounds enable mutual exclusion patterns in specialization:
    /// ```verum
    /// implement<T> MyProtocol for T where T: Send + !Sync {
    ///     // Implementation for types that are Send but not Sync
    /// }
    /// ```
    fn is_negative_bound(&self, bound: &ProtocolBound) -> bool {
        // Primary check: use the is_negative flag from ProtocolBound
        if bound.is_negative_bound() {
            return true;
        }

        // Legacy fallback: check if protocol name starts with '!'
        // This supports older code that used string-based negative bounds
        let protocol_str = self.protocol_path_to_string(&bound.protocol);
        protocol_str.starts_with("!")
    }

    /// Convert a protocol Path to a string for checking
    fn protocol_path_to_string(&self, path: &Path) -> Text {
        let temp_segments: Vec<Text> = path
            .segments
            .iter()
            .map(|seg| match seg {
                PathSegment::Name(ident) => ident.name.clone(),
                PathSegment::SelfValue => "Self".into(),
                PathSegment::Super => "super".into(),
                PathSegment::Cog => "cog".into(),
                PathSegment::Relative => ".".into(),
            })
            .collect();
        let segments: List<Text> = temp_segments.into();

        segments.join(".")
    }

    /// Check if a type satisfies a protocol (for negative bound checking)
    fn check_type_satisfies_protocol(
        &self,
        ty: &Type,
        protocol_path: &Path,
        protocol_checker: &ProtocolChecker,
    ) -> bool {
        // Get protocol name (remove '!' prefix if present for lookup)
        let protocol_str = self.protocol_path_to_string(protocol_path);
        let protocol_name = if protocol_str.starts_with("!") {
            protocol_str[1..].into()
        } else {
            protocol_str
        };

        // Check if type has an implementation for this protocol
        let impls = protocol_checker.get_implementations_for_protocol(&protocol_name);

        for impl_info in impls.iter() {
            // Check if impl's for_type matches the constrained type
            if self.types_match(ty, &impl_info.for_type) {
                return true; // Type does implement the protocol
            }
        }

        false // Type does not implement the protocol
    }

    /// Check if two types match for implementation purposes
    ///
    /// Production implementation using proper unification semantics:
    /// - Type variables are treated as universally quantified in the implementation
    /// - Concrete types must match structurally
    /// - Handles function types, reference types, and compound types
    fn types_match(&self, ty1: &Type, ty2: &Type) -> bool {
        self.types_match_with_bindings(ty1, ty2, &mut Map::new())
    }

    /// Check type matching with binding accumulator for occurs check
    fn types_match_with_bindings(
        &self,
        ty1: &Type,
        ty2: &Type,
        bindings: &mut Map<TypeVar, Type>,
    ) -> bool {
        match (ty1, ty2) {
            // Named types - paths and args must match
            (
                Type::Named {
                    path: p1,
                    args: args1,
                },
                Type::Named {
                    path: p2,
                    args: args2,
                },
            ) => {
                p1 == p2
                    && args1.len() == args2.len()
                    && args1
                        .iter()
                        .zip(args2.iter())
                        .all(|(a1, a2)| self.types_match_with_bindings(a1, a2, bindings))
            }

            // Type variables in impl signature match anything (instantiation)
            (Type::Var(var), concrete) => {
                if let Some(existing) = bindings.get(var) {
                    // Variable already bound - check consistency
                    let existing_clone = existing.clone();
                    self.types_match_with_bindings(&existing_clone, concrete, bindings)
                } else {
                    // Occurs check: concrete type should not contain var
                    if self.type_contains_var(concrete, var) {
                        false
                    } else {
                        bindings.insert(*var, concrete.clone());
                        true
                    }
                }
            }

            // Symmetric case
            (concrete, Type::Var(var)) => {
                if let Some(existing) = bindings.get(var) {
                    let existing_clone = existing.clone();
                    self.types_match_with_bindings(concrete, &existing_clone, bindings)
                } else if self.type_contains_var(concrete, var) {
                    false
                } else {
                    bindings.insert(*var, concrete.clone());
                    true
                }
            }

            // Function types - parameter and return types must match
            (
                Type::Function {
                    params: p1,
                    return_type: r1,
                    type_params: _,
                    contexts: c1,
                    properties: _,
                },
                Type::Function {
                    params: p2,
                    return_type: r2,
                    type_params: _,
                    contexts: c2,
                    properties: _,
                },
            ) => {
                // Check parameter count and types (contravariant)
                p1.len() == p2.len()
                    && p1.iter().zip(p2.iter()).all(|(a, b)| {
                        self.types_match_with_bindings(b, a, bindings)
                    })
                    // Check return type (covariant)
                    && self.types_match_with_bindings(r1, r2, bindings)
                    // Contexts should be compatible
                    && c1.is_some() == c2.is_some()
            }

            // Tuple types - all elements must match
            (Type::Tuple(elems1), Type::Tuple(elems2)) => {
                elems1.len() == elems2.len()
                    && elems1
                        .iter()
                        .zip(elems2.iter())
                        .all(|(e1, e2)| self.types_match_with_bindings(e1, e2, bindings))
            }

            // Reference types - inner types and mutability must match
            (
                Type::Reference {
                    inner: i1,
                    mutable: m1,
                    ..
                },
                Type::Reference {
                    inner: i2,
                    mutable: m2,
                    ..
                },
            ) => m1 == m2 && self.types_match_with_bindings(i1, i2, bindings),

            // Refinement types - base types must match (refinement predicates are checked separately)
            (Type::Refined { base: b1, .. }, Type::Refined { base: b2, .. }) => {
                self.types_match_with_bindings(b1, b2, bindings)
            }

            // Refinement vs non-refinement - extract base type
            (Type::Refined { base, .. }, other) | (other, Type::Refined { base, .. }) => {
                self.types_match_with_bindings(base, other, bindings)
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
            ) => self.types_match_with_bindings(e1, e2, bindings) && s1 == s2,

            // Slice types
            (Type::Slice { element: e1 }, Type::Slice { element: e2 }) => {
                self.types_match_with_bindings(e1, e2, bindings)
            }

            // Existential types match their body type
            (Type::Exists { body: b1, .. }, other) | (other, Type::Exists { body: b1, .. }) => {
                self.types_match_with_bindings(b1, other, bindings)
            }

            // Unit types
            (Type::Unit, Type::Unit) => true,

            // Never types
            (Type::Never, Type::Never) => true,

            // Primitive types must be exactly equal
            (Type::Bool, Type::Bool)
            | (Type::Int, Type::Int)
            | (Type::Float, Type::Float)
            | (Type::Char, Type::Char)
            | (Type::Text, Type::Text) => true,

            // Pointer types
            (
                Type::Pointer {
                    inner: i1,
                    mutable: m1,
                },
                Type::Pointer {
                    inner: i2,
                    mutable: m2,
                },
            ) => m1 == m2 && self.types_match_with_bindings(i1, i2, bindings),

            // Fallback to structural equality
            _ => ty1 == ty2,
        }
    }

    /// Check if a type contains a given type variable (for occurs check)
    fn type_contains_var(&self, ty: &Type, var: &TypeVar) -> bool {
        match ty {
            Type::Var(v) => v == var,
            Type::Named { args, .. } => args.iter().any(|a| self.type_contains_var(a, var)),
            Type::Function {
                params,
                return_type,
                ..
            } => {
                params.iter().any(|p| self.type_contains_var(p, var))
                    || self.type_contains_var(return_type, var)
            }
            Type::Tuple(elems) => elems.iter().any(|e| self.type_contains_var(e, var)),
            Type::Reference { inner, .. } => self.type_contains_var(inner, var),
            Type::Refined { base, .. } => self.type_contains_var(base, var),
            Type::Array { element, .. } => self.type_contains_var(element, var),
            Type::Slice { element } => self.type_contains_var(element, var),
            Type::Pointer { inner, .. } => self.type_contains_var(inner, var),
            Type::Exists { body, .. } => self.type_contains_var(body, var),
            _ => false,
        }
    }

    /// Extract implementation type for negative bound checking
    ///
    /// Returns the type that should be checked against negative bounds
    fn extract_impl_type_for_negative_check(&self, impl_info: &ProtocolImpl) -> Maybe<Type> {
        // For negative specialization, we check the for_type
        Maybe::Some(impl_info.for_type.clone())
    }

    /// Extract negative TypeBounds from a type if present
    ///
    /// Checks if the type has any negative protocol bounds in its structure
    fn extract_negative_type_bounds(&self, _ty: &Type) -> Option<List<TypeBound>> {
        // In the current implementation, negative bounds are stored in where clauses
        // rather than directly on types. This is a hook for future enhancement.
        None
    }

    /// Check if a type satisfies a negative TypeBound
    ///
    /// Returns true if the type DOES satisfy the bound (meaning negative check fails)
    fn check_negative_type_bound(
        &self,
        ty: &Type,
        bound: &TypeBound,
        protocol_checker: &ProtocolChecker,
    ) -> bool {
        match &bound.kind {
            TypeBoundKind::NegativeProtocol(protocol_path) => {
                // For negative protocol bound, check if type implements the protocol
                self.check_type_satisfies_protocol(ty, protocol_path, protocol_checker)
            }
            TypeBoundKind::Protocol(protocol_path) => {
                // Regular positive bound in negative context - this shouldn't happen
                // but handle it defensively
                false
            }
            TypeBoundKind::Equality(_) => {
                // Equality bounds are not relevant for negative checks
                false
            }
            TypeBoundKind::AssociatedTypeBound { .. } => {
                // Associated type bounds are not relevant for negative checks
                false
            }
            TypeBoundKind::AssociatedTypeEquality { .. } => {
                // Associated type equality bounds are not relevant for negative checks
                false
            }
            TypeBoundKind::GenericProtocol(generic_ty) => {
                // Generic protocol bound: Iterator<Item = T>
                // Extract the base protocol path and check
                use verum_ast::ty::TypeKind;
                if let TypeKind::Generic { base, .. } = &generic_ty.kind {
                    if let TypeKind::Path(path) = &base.kind {
                        return self.check_type_satisfies_protocol(ty, path, protocol_checker);
                    }
                }
                false
            }
        }
    }

    /// Rank candidates by specialization lattice
    ///
    /// Returns maximal elements (most specific implementations)
    fn rank_by_lattice(
        &self,
        candidates: &[ImplementId],
        lattice: &SpecializationLattice,
    ) -> Result<List<ImplementId>, SpecializationError> {
        if candidates.is_empty() {
            return Ok(List::new());
        }

        if candidates.len() == 1 {
            return Ok(vec![candidates[0]].into());
        }

        // Find maximal elements in the candidate set
        let mut maximal = List::new();

        for &candidate in candidates {
            let mut is_maximal = true;

            // Check if any other candidate is more specific
            for &other in candidates {
                if candidate != other && lattice.is_more_specific(other, candidate) {
                    is_maximal = false;
                    break;
                }
            }

            if is_maximal {
                maximal.push(candidate);
            }
        }

        Ok(maximal)
    }

    /// Get or build specialization lattice for a protocol
    fn get_or_build_lattice(
        &mut self,
        protocol: &Protocol,
        protocol_checker: &ProtocolChecker,
    ) -> &SpecializationLattice {
        let protocol_name = protocol.name.clone();

        if !self.lattices.contains_key(&protocol_name) {
            let lattice = self.build_lattice(protocol, protocol_checker);
            self.lattices.insert(protocol_name.clone(), lattice);
        }

        // SAFETY: insert above ensures key exists
        &self.lattices[&protocol_name]
    }

    /// Build specialization lattice for a protocol
    fn build_lattice(
        &self,
        protocol: &Protocol,
        protocol_checker: &ProtocolChecker,
    ) -> SpecializationLattice {
        let mut lattice = SpecializationLattice::new();

        // Get all implementations for this protocol
        let impls = protocol_checker.get_implementations_for_protocol(&protocol.name);

        // Add all implementations to lattice
        for (impl_id, _) in impls.iter().enumerate() {
            lattice.add_impl(impl_id);
        }

        // Build partial order based on specialization relationships
        for (i, impl1) in impls.iter().enumerate() {
            for (j, impl2) in impls.iter().enumerate() {
                if i != j && self.is_specialized(impl1, impl2) {
                    // impl1 is more specific than impl2
                    lattice.ordering.insert((i, j), true);
                }
            }
        }

        // Find maximal element (most general implementation)
        for (impl_id, _) in impls.iter().enumerate() {
            let mut is_maximal = true;
            for (other_id, _) in impls.iter().enumerate() {
                if impl_id != other_id && lattice.is_more_specific(impl_id, other_id) {
                    is_maximal = false;
                    break;
                }
            }
            if is_maximal {
                lattice.max_element = Maybe::Some(impl_id);
                break;
            }
        }

        // Find minimal elements (most specific implementations)
        for (impl_id, _) in impls.iter().enumerate() {
            let mut is_minimal = true;
            for (other_id, _) in impls.iter().enumerate() {
                if impl_id != other_id && lattice.is_more_specific(other_id, impl_id) {
                    is_minimal = false;
                    break;
                }
            }
            if is_minimal {
                lattice.min_elements.insert(impl_id);
            }
        }

        lattice
    }

    /// Check if impl1 specializes impl2
    fn is_specialized(&self, impl1: &ProtocolImpl, impl2: &ProtocolImpl) -> bool {
        // Check if impl1 explicitly declares it specializes impl2
        if let Maybe::Some(spec_info) = &impl1.specialization
            && spec_info.is_specialized
        {
            // Compare specificity ranks
            if let Maybe::Some(spec_info2) = &impl2.specialization {
                return spec_info.specificity_rank > spec_info2.specificity_rank;
            }
            return true;
        }

        // Otherwise, check structural specificity
        // impl1 is more specific if:
        // 1. It has more concrete types (fewer type variables)
        // 2. It has more where clause constraints
        // 3. Its for_type is more specific

        self.is_more_specific_type(&impl1.for_type, &impl2.for_type)
    }

    /// Check if type1 is more specific than type2
    fn is_more_specific_type(&self, type1: &Type, type2: &Type) -> bool {
        match (type1, type2) {
            // Concrete type is more specific than type variable
            (Type::Named { .. }, Type::Var(_)) => true,
            (Type::Var(_), Type::Named { .. }) => false,

            // Compare compound types recursively
            (
                Type::Named {
                    path: p1,
                    args: args1,
                },
                Type::Named {
                    path: p2,
                    args: args2,
                },
            ) if p1 == p2 => {
                // More specific if any argument is more specific
                args1
                    .iter()
                    .zip(args2.iter())
                    .any(|(a1, a2)| self.is_more_specific_type(a1, a2))
            }

            _ => false,
        }
    }

    /// Cache a specialization decision
    pub fn cache_selection(&mut self, protocol: Text, type_key: Text, impl_id: ImplementId) {
        self.cache.insert((protocol, type_key), impl_id);
    }

    /// Convert a type to a cache key
    pub fn type_to_cache_key(&self, ty: &Type) -> Text {
        // Generate a stable string representation for caching
        match ty {
            Type::Named { path, args } => {
                let mut key = path.to_string();
                if !args.is_empty() {
                    key.push('<');
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            key.push_str(", ");
                        }
                        key.push_str(self.type_to_cache_key(arg).as_str());
                    }
                    key.push('>');
                }
                key.into()
            }
            Type::Var(v) => format!("Var{}", v.id()).into(),
            Type::Int => WKT::Int.as_str().into(),
            Type::Float => WKT::Float.as_str().into(),
            Type::Bool => WKT::Bool.as_str().into(),
            Type::Char => WKT::Char.as_str().into(),
            Type::Text => WKT::Text.as_str().into(),
            Type::Unit => "Unit".into(),
            _ => "ComplexType".into(),
        }
    }

    /// Get performance statistics
    pub fn stats(&self) -> &SelectionStats {
        &self.stats
    }

    /// Clear cache (useful for testing)
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

impl Default for SpecializationSelector {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Coherence Checking ====================

/// Coherence checker for specialization
pub struct CoherenceChecker {
    /// Detected violations
    violations: List<CoherenceViolation>,
}

/// A coherence violation
#[derive(Debug, Clone)]
pub struct CoherenceViolation {
    pub impl1_id: ImplementId,
    pub impl2_id: ImplementId,
    pub protocol: Text,
    pub reason: Text,
}

impl CoherenceChecker {
    /// Create a new coherence checker
    pub fn new() -> Self {
        Self {
            violations: List::new(),
        }
    }

    /// Check coherence for a protocol
    ///
    /// Verifies that no overlapping implementations exist without a
    /// specialization relationship between them.
    pub fn check_protocol(
        &mut self,
        protocol: &Protocol,
        protocol_checker: &ProtocolChecker,
    ) -> Result<(), List<CoherenceViolation>> {
        self.violations.clear();

        let impls = protocol_checker.get_implementations_for_protocol(&protocol.name);

        // Check all pairs of implementations for overlap
        for (i, impl1) in impls.iter().enumerate() {
            for (j, impl2) in impls.iter().enumerate() {
                if i < j && self.overlaps(impl1, impl2) {
                    // Check if one specializes the other
                    if !self.has_specialization_relationship(impl1, impl2) {
                        self.violations.push(CoherenceViolation {
                            impl1_id: i,
                            impl2_id: j,
                            protocol: protocol.name.clone(),
                            reason: "overlapping implementations without specialization".into(),
                        });
                    }
                }
            }
        }

        if self.violations.is_empty() {
            Ok(())
        } else {
            Err(self.violations.clone())
        }
    }

    /// Check if two implementations overlap
    ///
    /// Two implementations overlap if there exists a concrete type that could
    /// match both implementation patterns. However, implementations with
    /// complementary negative bounds do NOT overlap.
    ///
    /// Multi-protocol bounds: combining multiple protocol constraints (T: Display + Debug) — Negative Bounds
    ///
    /// # Negative Bounds and Mutual Exclusion
    ///
    /// Negative bounds create mutual exclusion between implementations:
    /// ```verum
    /// implement<T: Clone + !Copy> DeepClone for T { ... }  // impl1
    /// implement<T: Copy> DeepClone for T { ... }           // impl2
    /// ```
    ///
    /// These implementations do NOT overlap because:
    /// - impl1 requires T: !Copy (T must NOT implement Copy)
    /// - impl2 requires T: Copy (T MUST implement Copy)
    ///
    /// No type can satisfy both constraints simultaneously, so they are
    /// mutually exclusive and can coexist without ambiguity.
    pub fn overlaps(&self, impl1: &ProtocolImpl, impl2: &ProtocolImpl) -> bool {
        // First, check if negative bounds create mutual exclusion
        if self.has_mutually_exclusive_bounds(impl1, impl2) {
            return false; // No overlap due to negative bounds
        }

        // Two impls overlap if there exists a type that matches both
        // Simplified check: compare for_type patterns

        match (&impl1.for_type, &impl2.for_type) {
            // Same concrete type = overlap
            (
                Type::Named {
                    path: p1,
                    args: args1,
                },
                Type::Named {
                    path: p2,
                    args: args2,
                },
            ) if p1 == p2 && args1 == args2 => true,

            // One is generic (type variable) and other is concrete = overlap
            (Type::Var(_), Type::Named { .. }) | (Type::Named { .. }, Type::Var(_)) => true,

            // Both are generic = overlap
            (Type::Var(_), Type::Var(_)) => true,

            _ => false,
        }
    }

    /// Check if two implementations have mutually exclusive bounds
    ///
    /// Multi-protocol bounds: combining multiple protocol constraints (T: Display + Debug) — Negative Bounds
    ///
    /// Returns true if the implementations have complementary bounds that
    /// make it impossible for any type to satisfy both.
    ///
    /// # Algorithm
    ///
    /// 1. Collect all bounds from both implementations' where clauses
    /// 2. For each positive bound P in impl1, check if impl2 has !P
    /// 3. For each negative bound !P in impl1, check if impl2 has P
    /// 4. If any such pair exists, the implementations are mutually exclusive
    pub fn has_mutually_exclusive_bounds(
        &self,
        impl1: &ProtocolImpl,
        impl2: &ProtocolImpl,
    ) -> bool {
        // Collect all bounds from impl1
        let bounds1 = self.collect_all_bounds(impl1);
        // Collect all bounds from impl2
        let bounds2 = self.collect_all_bounds(impl2);

        // Check for complementary bounds
        for (ty1, bound1) in bounds1.iter() {
            for (ty2, bound2) in bounds2.iter() {
                // Only check bounds on the same type parameter
                if !self.types_refer_to_same_param(ty1, ty2) {
                    continue;
                }

                // Check if one is positive and one is negative for the same protocol
                if self.are_complementary_bounds(bound1, bound2) {
                    return true; // Mutually exclusive
                }
            }
        }

        false
    }

    /// Collect all bounds from an implementation's where clauses
    fn collect_all_bounds(&self, impl_info: &ProtocolImpl) -> List<(Type, ProtocolBound)> {
        let mut result = List::new();

        for clause in impl_info.where_clauses.iter() {
            for bound in clause.bounds.iter() {
                result.push((clause.ty.clone(), bound.clone()));
            }
        }

        result
    }

    /// Check if two types refer to the same type parameter
    fn types_refer_to_same_param(&self, ty1: &Type, ty2: &Type) -> bool {
        match (ty1, ty2) {
            // Same type variable
            (Type::Var(v1), Type::Var(v2)) => v1 == v2,

            // Same named type with same name (for generic parameters)
            (
                Type::Named { path: p1, args: a1 },
                Type::Named { path: p2, args: a2 },
            ) => p1 == p2 && a1 == a2,

            // Generic types with same name
            (Type::Generic { name: n1, .. }, Type::Generic { name: n2, .. }) => n1 == n2,

            // Same primitive types
            (Type::Int, Type::Int) => true,
            (Type::Float, Type::Float) => true,
            (Type::Bool, Type::Bool) => true,
            (Type::Char, Type::Char) => true,
            (Type::Text, Type::Text) => true,
            (Type::Unit, Type::Unit) => true,

            _ => false,
        }
    }

    /// Check if two bounds are complementary (one positive, one negative, same protocol)
    ///
    /// Example:
    /// - T: Copy and T: !Copy are complementary
    /// - T: Clone and T: !Clone are complementary
    /// - T: Clone and T: !Copy are NOT complementary (different protocols)
    fn are_complementary_bounds(&self, bound1: &ProtocolBound, bound2: &ProtocolBound) -> bool {
        // Check if one is negative and one is positive
        if bound1.is_negative == bound2.is_negative {
            return false; // Both positive or both negative - not complementary
        }

        // Check if they refer to the same protocol
        self.same_protocol(&bound1.protocol, &bound2.protocol)
    }

    /// Check if two protocol paths refer to the same protocol
    fn same_protocol(&self, p1: &Path, p2: &Path) -> bool {
        // Compare path segments
        if p1.segments.len() != p2.segments.len() {
            return false;
        }

        for (seg1, seg2) in p1.segments.iter().zip(p2.segments.iter()) {
            match (seg1, seg2) {
                (PathSegment::Name(id1), PathSegment::Name(id2)) => {
                    if id1.name != id2.name {
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

    /// Check if implementations have a specialization relationship
    pub fn has_specialization_relationship(
        &self,
        impl1: &ProtocolImpl,
        impl2: &ProtocolImpl,
    ) -> bool {
        // Check if either impl declares it specializes the other
        if let Maybe::Some(spec1) = &impl1.specialization
            && spec1.is_specialized
        {
            return true;
        }
        if let Maybe::Some(spec2) = &impl2.specialization
            && spec2.is_specialized
        {
            return true;
        }

        // Also check if negative bounds create mutual exclusion
        // which is a form of specialization relationship
        self.has_mutually_exclusive_bounds(impl1, impl2)
    }

    /// Get detected violations
    pub fn violations(&self) -> &[CoherenceViolation] {
        &self.violations
    }
}

impl Default for CoherenceChecker {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Extensions for ProtocolChecker ====================

/// Extension trait for ProtocolChecker to support specialization selection
pub trait ProtocolCheckerExt {
    /// Get all implementations for a protocol
    fn get_implementations_for_protocol(&self, protocol: &Text) -> List<ProtocolImpl>;

    /// Check if a protocol bound is satisfied
    fn check_protocol_bound(&self, ty: &Type, bound: &ProtocolBound) -> bool;
}

impl ProtocolCheckerExt for ProtocolChecker {
    fn get_implementations_for_protocol(&self, protocol: &Text) -> List<ProtocolImpl> {
        // Use the new public API to query implementations by protocol name
        self.query_implementations_by_protocol(protocol)
            .iter()
            .map(|&impl_ref| impl_ref.clone())
            .collect()
    }

    fn check_protocol_bound(&self, ty: &Type, bound: &ProtocolBound) -> bool {
        // Check if a type satisfies a protocol bound
        // This uses the existing protocol checker infrastructure

        // Helper function to convert type to string for lookup
        let type_str = match ty {
            Type::Unit => "Unit".into(),
            Type::Bool => WKT::Bool.as_str().into(),
            Type::Int => WKT::Int.as_str().into(),
            Type::Float => WKT::Float.as_str().into(),
            Type::Char => WKT::Char.as_str().into(),
            Type::Text => WKT::Text.as_str().into(),
            Type::Named { path, .. } => path
                .as_ident()
                .map(|i| i.name.clone())
                .unwrap_or_else(|| "Unknown".into()),
            Type::Generic { name, .. } => name.clone(),
            _ => "ComplexType".into(),
        };

        // Helper function to convert protocol path to string
        let protocol_name: Text = bound
            .protocol
            .segments
            .iter()
            .map(|seg| match seg {
                PathSegment::Name(ident) => ident.name.clone(),
                _ => "".into(),
            })
            .collect::<Vec<Text>>()
            .join(".")
            .into();

        // Use the existing check_protocol_satisfied method from ProtocolChecker
        // This delegates to the actual implementation which has access to private fields
        match self.check_protocol_satisfied(ty, &protocol_name) {
            Ok(_) => true,
            Err(_) => {
                // Fallback: check centralized primitive protocol registry
                let type_name = match ty.primitive_name() {
                    Some(name) => name,
                    None => return false,
                };
                primitive_implements_protocol(type_name, protocol_name.as_str())
                    .unwrap_or(false)
            }
        }
    }
}

// ==================== Integration with TypeChecker ====================

/// Integration point for TypeChecker
impl SpecializationSelector {
    /// Resolve protocol method call with specialization
    ///
    /// This is called during type inference when resolving a method call
    /// on a protocol.
    pub fn resolve_protocol_method(
        &mut self,
        receiver_type: &Type,
        protocol: &Protocol,
        method_name: &Text,
        protocol_checker: &ProtocolChecker,
        unifier: &mut Unifier,
    ) -> Result<(ImplementId, Type), SpecializationError> {
        // 1. Select specialized implementation
        let impl_id =
            self.select_implementation(protocol, receiver_type, protocol_checker, unifier)?;

        // 2. Get method from selected impl
        let impls = protocol_checker.get_implementations_for_protocol(&protocol.name);
        if let Some(impl_info) = impls.get(impl_id)
            && let Some(method_ty) = impl_info.methods.get(method_name)
        {
            return Ok((impl_id, method_ty.clone()));
        }

        // Method not found in implementation
        Err(SpecializationError::NoApplicableImpl {
            protocol: protocol.name.clone(),
            self_type: receiver_type.clone(),
            suggestion: format!(
                "method `{}` not found in selected implementation",
                method_name
            )
            .into(),
        })
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::span::{FileId, Span};
    use verum_ast::ty::{Ident, Path};

    fn make_span() -> Span {
        Span::new(0, 10, FileId::new(0))
    }

    fn make_copy_bound(is_negative: bool) -> ProtocolBound {
        ProtocolBound {
            protocol: Path::single(Ident::new("Copy", make_span())),
            args: List::new(),
            is_negative,
        }
    }

    fn make_clone_bound(is_negative: bool) -> ProtocolBound {
        ProtocolBound {
            protocol: Path::single(Ident::new("Clone", make_span())),
            args: List::new(),
            is_negative,
        }
    }

    fn make_where_clause(ty: Type, bounds: List<ProtocolBound>) -> WhereClause {
        WhereClause { ty, bounds }
    }

    fn make_impl(
        for_type: Type,
        where_clauses: List<WhereClause>,
    ) -> ProtocolImpl {
        ProtocolImpl {
            protocol: Path::single(Ident::new("DeepClone", make_span())),
            protocol_args: List::new(),
            for_type,
            where_clauses,
            methods: Map::new(),
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("test".into()),
            span: make_span(),
            type_param_fn_bounds: Map::new(),
        }
    }

    // ==================== NegativeBoundResult Tests ====================

    #[test]
    fn test_negative_bound_result_variants() {
        assert_ne!(NegativeBoundResult::Satisfied, NegativeBoundResult::Violated);
        assert_ne!(NegativeBoundResult::Satisfied, NegativeBoundResult::Unknown);
        assert_ne!(NegativeBoundResult::Violated, NegativeBoundResult::Unknown);
    }

    // ==================== check_negative_bound Tests ====================

    #[test]
    fn test_check_negative_bound_int_is_copy() {
        let selector = SpecializationSelector::new();
        let protocol_checker = ProtocolChecker::new_empty();
        let copy_bound = make_copy_bound(true); // T: !Copy

        // Int implements Copy, so !Copy is violated
        let result = selector.check_negative_bound(&Type::Int, &copy_bound, &protocol_checker);
        assert_eq!(result, NegativeBoundResult::Violated);
    }

    #[test]
    fn test_check_negative_bound_text_not_copy() {
        let selector = SpecializationSelector::new();
        let protocol_checker = ProtocolChecker::new_empty();
        let copy_bound = make_copy_bound(true); // T: !Copy

        // Text does NOT implement Copy, so !Copy is satisfied
        let result = selector.check_negative_bound(&Type::Text, &copy_bound, &protocol_checker);
        assert_eq!(result, NegativeBoundResult::Satisfied);
    }

    #[test]
    fn test_check_negative_bound_type_var_unknown() {
        let selector = SpecializationSelector::new();
        let protocol_checker = ProtocolChecker::new_empty();
        let copy_bound = make_copy_bound(true); // T: !Copy

        // Type variable cannot be determined
        let type_var = Type::Var(TypeVar::with_id(0));
        let result = selector.check_negative_bound(&type_var, &copy_bound, &protocol_checker);
        assert_eq!(result, NegativeBoundResult::Unknown);
    }

    #[test]
    fn test_check_negative_bound_bool_is_clone() {
        let selector = SpecializationSelector::new();
        let protocol_checker = ProtocolChecker::new_empty();
        let clone_bound = make_clone_bound(true); // T: !Clone

        // Bool implements Clone, so !Clone is violated
        let result = selector.check_negative_bound(&Type::Bool, &clone_bound, &protocol_checker);
        assert_eq!(result, NegativeBoundResult::Violated);
    }

    // ==================== Coherence Tests (Mutual Exclusion) ====================

    #[test]
    fn test_complementary_bounds_detection() {
        let checker = CoherenceChecker::new();

        let copy_positive = make_copy_bound(false); // T: Copy
        let copy_negative = make_copy_bound(true);  // T: !Copy

        // Copy and !Copy are complementary
        assert!(checker.are_complementary_bounds(&copy_positive, &copy_negative));
        assert!(checker.are_complementary_bounds(&copy_negative, &copy_positive));
    }

    #[test]
    fn test_non_complementary_bounds() {
        let checker = CoherenceChecker::new();

        let copy_positive = make_copy_bound(false);  // T: Copy
        let clone_negative = make_clone_bound(true); // T: !Clone

        // Copy and !Clone are NOT complementary (different protocols)
        assert!(!checker.are_complementary_bounds(&copy_positive, &clone_negative));
    }

    #[test]
    fn test_same_polarity_not_complementary() {
        let checker = CoherenceChecker::new();

        let copy_positive1 = make_copy_bound(false); // T: Copy
        let copy_positive2 = make_copy_bound(false); // T: Copy

        // Both positive - not complementary
        assert!(!checker.are_complementary_bounds(&copy_positive1, &copy_positive2));

        let copy_negative1 = make_copy_bound(true); // T: !Copy
        let copy_negative2 = make_copy_bound(true); // T: !Copy

        // Both negative - not complementary
        assert!(!checker.are_complementary_bounds(&copy_negative1, &copy_negative2));
    }

    #[test]
    fn test_mutually_exclusive_impls() {
        let checker = CoherenceChecker::new();

        // impl<T: Clone + !Copy> DeepClone for T
        let impl1 = make_impl(
            Type::Var(TypeVar::with_id(0)),
            List::from(vec![make_where_clause(
                Type::Var(TypeVar::with_id(0)),
                List::from(vec![
                    make_clone_bound(false), // T: Clone
                    make_copy_bound(true),   // T: !Copy
                ]),
            )]),
        );

        // impl<T: Copy> DeepClone for T
        let impl2 = make_impl(
            Type::Var(TypeVar::with_id(0)),
            List::from(vec![make_where_clause(
                Type::Var(TypeVar::with_id(0)),
                List::from(vec![
                    make_copy_bound(false), // T: Copy
                ]),
            )]),
        );

        // These impls are mutually exclusive due to Copy / !Copy
        assert!(checker.has_mutually_exclusive_bounds(&impl1, &impl2));
    }

    #[test]
    fn test_overlapping_impls_without_exclusion() {
        let checker = CoherenceChecker::new();

        // impl<T: Clone> DeepClone for T
        let impl1 = make_impl(
            Type::Var(TypeVar::with_id(0)),
            List::from(vec![make_where_clause(
                Type::Var(TypeVar::with_id(0)),
                List::from(vec![make_clone_bound(false)]), // T: Clone
            )]),
        );

        // impl<T: Clone> DeepClone for T (same bounds)
        let impl2 = make_impl(
            Type::Var(TypeVar::with_id(0)),
            List::from(vec![make_where_clause(
                Type::Var(TypeVar::with_id(0)),
                List::from(vec![make_clone_bound(false)]), // T: Clone
            )]),
        );

        // These impls are NOT mutually exclusive
        assert!(!checker.has_mutually_exclusive_bounds(&impl1, &impl2));
    }

    #[test]
    fn test_no_overlap_with_mutual_exclusion() {
        let checker = CoherenceChecker::new();

        // impl<T: !Copy> DeepClone for T
        let impl1 = make_impl(
            Type::Var(TypeVar::with_id(0)),
            List::from(vec![make_where_clause(
                Type::Var(TypeVar::with_id(0)),
                List::from(vec![make_copy_bound(true)]), // T: !Copy
            )]),
        );

        // impl<T: Copy> DeepClone for T
        let impl2 = make_impl(
            Type::Var(TypeVar::with_id(0)),
            List::from(vec![make_where_clause(
                Type::Var(TypeVar::with_id(0)),
                List::from(vec![make_copy_bound(false)]), // T: Copy
            )]),
        );

        // These impls should NOT overlap (mutual exclusion)
        assert!(!checker.overlaps(&impl1, &impl2));
    }

    // ==================== Error Variant Tests ====================

    #[test]
    fn test_negative_bound_violated_error() {
        let error = SpecializationError::NegativeBoundViolated {
            ty: Type::Int,
            protocol: "Copy".into(),
            impl_id: 0,
            span: Maybe::Some(make_span()),
        };

        let type_error = error.to_type_error(make_span());
        match type_error {
            TypeError::NegativeBoundViolated { ty, protocol, .. } => {
                assert_eq!(ty, Text::from("Int"));
                assert_eq!(protocol, Text::from("Copy"));
            }
            _ => panic!("Expected NegativeBoundViolated error"),
        }
    }

    #[test]
    fn test_negative_bound_unknown_error() {
        let error = SpecializationError::NegativeBoundUnknown {
            ty: Type::Var(TypeVar::with_id(0)),
            protocol: "Copy".into(),
        };

        let type_error = error.to_type_error(make_span());
        match type_error {
            TypeError::Other(msg) => {
                assert!(msg.as_str().contains("cannot verify negative bound"));
            }
            _ => panic!("Expected Other error"),
        }
    }

    // ==================== Builtin Type Tests ====================

    #[test]
    fn test_builtin_implements_copy() {
        let selector = SpecializationSelector::new();

        // Types that implement Copy
        assert!(selector.builtin_implements_protocol(&Type::Int, &"Copy".into()));
        assert!(selector.builtin_implements_protocol(&Type::Float, &"Copy".into()));
        assert!(selector.builtin_implements_protocol(&Type::Bool, &"Copy".into()));
        assert!(selector.builtin_implements_protocol(&Type::Char, &"Copy".into()));
        assert!(selector.builtin_implements_protocol(&Type::Unit, &"Copy".into()));

        // Text does NOT implement Copy
        assert!(!selector.builtin_implements_protocol(&Type::Text, &"Copy".into()));
    }

    #[test]
    fn test_builtin_implements_clone() {
        let selector = SpecializationSelector::new();

        // All these types implement Clone
        assert!(selector.builtin_implements_protocol(&Type::Int, &"Clone".into()));
        assert!(selector.builtin_implements_protocol(&Type::Float, &"Clone".into()));
        assert!(selector.builtin_implements_protocol(&Type::Bool, &"Clone".into()));
        assert!(selector.builtin_implements_protocol(&Type::Char, &"Clone".into()));
        assert!(selector.builtin_implements_protocol(&Type::Text, &"Clone".into()));
        assert!(selector.builtin_implements_protocol(&Type::Unit, &"Clone".into()));
    }

    #[test]
    fn test_builtin_implements_eq() {
        let selector = SpecializationSelector::new();

        assert!(selector.builtin_implements_protocol(&Type::Int, &"Eq".into()));
        assert!(selector.builtin_implements_protocol(&Type::Bool, &"Eq".into()));
        assert!(selector.builtin_implements_protocol(&Type::Char, &"Eq".into()));
        assert!(selector.builtin_implements_protocol(&Type::Text, &"Eq".into()));

        // Float doesn't fully implement Eq due to NaN
        assert!(!selector.builtin_implements_protocol(&Type::Float, &"Eq".into()));
    }
}
