//! Advanced Protocol Features Implementation
//!
//! Advanced protocols (future v2.0+): GATs, higher-rank bounds, specialization with lattice ordering, coherence rules — Complete Advanced Protocol System
//!
//! This module implements sophisticated protocol features that enable advanced
//! type-level programming while maintaining zero-cost guarantees:
//!
//! - **Generic Associated Types (GATs)**: Associated types with type parameters
//! - **Specialization**: More specific implementations override general ones
//! - **GenRef Wrapper**: Generation-aware references for lending iterators
//! - **Refinement Integration**: Value-level constraints in protocol signatures
//! - **Higher-Kinded Types**: Type constructors as protocol parameters
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │  GAT System                             │
//! │  - Type parameters on associated types  │
//! │  - Where clauses and bounds             │
//! │  - Higher-kinded type support           │
//! └─────────────────────────────────────────┘
//!           ↓
//! ┌─────────────────────────────────────────┐
//! │  Specialization Lattice                 │
//! │  - Precedence resolution                │
//! │  - Overlap detection                    │
//! │  - Negative reasoning                   │
//! └─────────────────────────────────────────┘
//!           ↓
//! ┌─────────────────────────────────────────┐
//! │  GenRef Wrapper                         │
//! │  - Generation tracking                  │
//! │  - CBGR integration                     │
//! │  - Lending iterator support             │
//! └─────────────────────────────────────────┘
//! ```
//!
//! # Performance Guarantees
//!
//! - **GATs**: Zero-cost (compile to concrete types via monomorphization)
//! - **Specialization**: Zero-cost (resolved at compile-time)
//! - **GenRef**: ~20ns overhead (15ns CBGR + 5ns generation check)
//! - **VTable Dispatch**: <10ns overhead (cache-aligned, direct function pointers)

use verum_ast::{
    span::Span,
    ty::{Ident, Path},
};
use verum_common::{ConstValue, List, Map, Maybe, Set, Text};
use verum_common::well_known_types::WellKnownType as WKT;

use crate::protocol::{ProtocolBound, ProtocolImpl};
use crate::ty::Type;
pub use crate::kind_inference::Kind;
pub use crate::variance::Variance;

// ==================== Generic Associated Types (GATs) ====================

/// Type parameter for a Generic Associated Type
///
/// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .1 lines 116-134
///
/// Example:
/// ```verum
/// protocol Monad {
///     type Wrapped<T>  // GAT with one type parameter
///     fn pure<T>(value: T) -> Self.Wrapped<T>
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct GATTypeParam {
    /// Parameter name (e.g., "T" in `type Item<T>`)
    pub name: Text,

    /// Bounds on this parameter (e.g., `T: Clone + Debug`)
    pub bounds: List<ProtocolBound>,

    /// Default type for this parameter (if any)
    pub default: Maybe<Type>,

    /// Variance of this parameter (covariant, contravariant, invariant)
    pub variance: Variance,
}

// Variance is imported from crate::variance

/// Where clause specific to a GAT (not the protocol itself)
///
/// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .4 lines 441-471
///
/// Example:
/// ```verum
/// protocol Container {
///     type Item<T> where T: Clone + Debug
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct GATWhereClause {
    /// Type parameter being constrained
    pub param: Text,

    /// Protocol bounds that must be satisfied
    pub constraints: List<ProtocolBound>,

    /// Source location
    pub span: Span,
}

/// Kind of associated type
///
/// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .3 lines 410-437
#[derive(Debug, Clone, PartialEq)]
pub enum AssociatedTypeKind {
    /// Regular associated type: `type Item`
    Regular,

    /// Generic associated type: `type Item<T>`
    /// Contains the number of type parameters
    Generic { arity: usize },

    /// Higher-kinded type: `type F<_>`
    /// Arity indicates number of type constructor parameters
    HigherKinded { arity: usize },
}

/// Extended AssociatedType with GAT support
///
/// This extends the basic AssociatedType from protocol.rs with:
/// - Type parameters for GATs
/// - Per-GAT where clauses
/// - Kind tracking (regular, generic, higher-kinded)
///
/// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .1-1.4 lines 112-471
#[derive(Debug, Clone)]
pub struct AssociatedTypeGAT {
    /// Type name
    pub name: Text,

    /// Type parameters (empty for non-GATs)
    pub type_params: List<GATTypeParam>,

    /// Protocol bounds on the associated type itself
    pub bounds: List<ProtocolBound>,

    /// Where clauses specific to this GAT
    pub where_clauses: List<GATWhereClause>,

    /// Default type (if any)
    pub default: Maybe<Type>,

    /// Kind of associated type
    pub kind: AssociatedTypeKind,

    /// Documentation
    pub doc: Maybe<Text>,

    /// Source location
    pub span: Span,
}

impl AssociatedTypeGAT {
    /// Create a simple (non-GAT) associated type
    pub fn simple(name: Text, bounds: List<ProtocolBound>) -> Self {
        Self {
            name,
            type_params: List::new(),
            bounds,
            where_clauses: List::new(),
            default: Maybe::None,
            kind: AssociatedTypeKind::Regular,
            doc: Maybe::None,
            span: Span::default(),
        }
    }

    /// Create a GAT with type parameters
    pub fn generic(
        name: Text,
        type_params: List<GATTypeParam>,
        bounds: List<ProtocolBound>,
        where_clauses: List<GATWhereClause>,
    ) -> Self {
        let arity = type_params.len();
        Self {
            name,
            type_params,
            bounds,
            where_clauses,
            default: Maybe::None,
            kind: AssociatedTypeKind::Generic { arity },
            doc: Maybe::None,
            span: Span::default(),
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

    /// Instantiate a GAT with concrete types
    ///
    /// Creates a concrete instantiation of this GAT by substituting type parameters
    /// with the provided concrete types. Returns the resulting concrete type.
    ///
    /// # Arguments
    ///
    /// * `concrete_types` - The concrete types to substitute for each type parameter
    ///
    /// # Returns
    ///
    /// * `Ok(Type)` - The instantiated concrete type
    /// * `Err(String)` - Error if arity mismatch or instantiation fails
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Given: type Item<T> in Iterator protocol
    /// let gat = AssociatedTypeGAT::generic("Item", vec![...], ...);
    /// let concrete = gat.instantiate(&[Type::Int])?;
    /// // Result: Item<Int>
    /// ```
    pub fn instantiate(&self, concrete_types: &[Type]) -> Result<Type, Text> {
        // Validate arity
        if concrete_types.len() != self.type_params.len() {
            return Err(format!(
                "GAT '{}' expects {} type argument(s), got {}",
                self.name,
                self.type_params.len(),
                concrete_types.len()
            )
            .into());
        }

        // For non-GATs, just return a simple named type
        if !self.is_gat() {
            return Ok(Type::Named {
                path: verum_ast::ty::Path::from_ident(verum_ast::ty::Ident::new(
                    self.name.as_str(),
                    Span::default(),
                )),
                args: List::new(),
            });
        }

        // Create a Generic type with the concrete type arguments
        Ok(Type::Generic {
            name: self.name.clone(),
            args: List::from_iter(concrete_types.iter().cloned()),
        })
    }

    /// Instantiate a GAT with concrete types and validate constraints
    ///
    /// Like `instantiate`, but also validates that where clause constraints
    /// are satisfied by the concrete types.
    ///
    /// # Arguments
    ///
    /// * `concrete_types` - The concrete types to substitute
    /// * `check_constraint` - Callback to check if a type satisfies a protocol bound
    ///
    /// # Returns
    ///
    /// * `Ok(Type)` - The instantiated concrete type if all constraints are satisfied
    /// * `Err(String)` - Error if arity mismatch or constraints not satisfied
    pub fn instantiate_checked<F>(
        &self,
        concrete_types: &[Type],
        check_constraint: F,
    ) -> Result<Type, Text>
    where
        F: Fn(&Type, &ProtocolBound) -> bool,
    {
        // First validate arity
        if concrete_types.len() != self.type_params.len() {
            return Err(format!(
                "GAT '{}' expects {} type argument(s), got {}",
                self.name,
                self.type_params.len(),
                concrete_types.len()
            )
            .into());
        }

        // Check where clause constraints
        for (i, param) in self.type_params.iter().enumerate() {
            let concrete_type = &concrete_types[i];

            // Check bounds on the type parameter itself
            for bound in &param.bounds {
                if !check_constraint(concrete_type, bound) {
                    return Err(format!(
                        "Type '{}' does not satisfy bound '{}' required by GAT type parameter '{}'",
                        format_type(concrete_type),
                        bound.protocol,
                        param.name
                    )
                    .into());
                }
            }
        }

        // Check additional where clauses
        for where_clause in &self.where_clauses {
            // Find the concrete type for this parameter
            let param_idx = self
                .type_params
                .iter()
                .position(|p| p.name == where_clause.param);

            if let Some(idx) = param_idx {
                let concrete_type = &concrete_types[idx];
                for constraint in &where_clause.constraints {
                    if !check_constraint(concrete_type, constraint) {
                        return Err(format!(
                            "Type '{}' does not satisfy where clause constraint '{}' for parameter '{}'",
                            format_type(concrete_type),
                            constraint.protocol,
                            where_clause.param
                        )
                        .into());
                    }
                }
            }
        }

        // All constraints satisfied, create the instantiated type
        self.instantiate(concrete_types)
    }
}

/// Helper function to format a type for error messages
fn format_type(ty: &Type) -> Text {
    match ty {
        Type::Int => WKT::Int.as_str().into(),
        Type::Float => WKT::Float.as_str().into(),
        Type::Bool => WKT::Bool.as_str().into(),
        Type::Text => WKT::Text.as_str().into(),
        Type::Char => WKT::Char.as_str().into(),
        Type::Unit => "()".into(),
        Type::Never => "Never".into(),
        Type::Generic { name, args } if args.is_empty() => name.clone(),
        Type::Generic { name, args } => {
            let args_str: Vec<String> = args.iter().map(|a| format_type(a).to_string()).collect();
            format!("{}<{}>", name, args_str.join(", ")).into()
        }
        Type::Named { path, .. } => {
            if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.last() {
                ident.name.as_str().into()
            } else {
                "unknown".into()
            }
        }
        _ => format!("{:?}", ty).into(),
    }
}

// ==================== GenRef: Generation-Aware References ====================

/// Generation-aware reference wrapper for CBGR
///
/// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .2 lines 143-193, 533-547
///
/// GenRef wraps a CBGR reference with explicit generation tracking,
/// enabling lending iterators and self-referential types without lifetime annotations.
///
/// # Memory Layout
///
/// ```text
/// GenRef<T> {
///     ptr: *const T      // 8 bytes
///     generation: u64    // 8 bytes
/// }
/// Total: 16 bytes
/// ```
///
/// # Example
///
/// ```verum
/// type WindowIterator<T> {
///     data: GenRef<List<T>>,
///     window_size: usize,
///     position: usize
/// }
///
/// implement<T> Iterator for WindowIterator<T> {
///     type Item is [T]
///
///     fn next(&mut self) -> Maybe<GenRef<&[T]>> {
///         let data = self.data.deref()?;
///         if self.position + self.window_size <= data.len() {
///             let slice = &data[self.position..self.position + self.window_size];
///             self.position += 1;
///             Some(GenRef.borrow(slice))
///         } else {
///             None
///         }
///     }
/// }
/// ```
#[derive(Debug, Clone)]
pub struct GenRefType {
    /// Inner type being referenced
    pub inner: Box<Type>,

    /// Source location
    pub span: Span,
}

impl GenRefType {
    /// Create a new GenRef type
    pub fn new(inner: Type) -> Self {
        Self {
            inner: Box::new(inner),
            span: Span::default(),
        }
    }

    /// Get the inner type
    pub fn inner(&self) -> &Type {
        &self.inner
    }
}

/// Generation tracking predicates for refinement types
///
/// Higher-rank protocol bounds: for<T> quantification in protocol bounds for universal requirements — .2 lines 515-532
///
/// These predicates are available in ensures/requires clauses:
/// - `generation(ref)` - Get generation counter
/// - `epoch(ref)` - Get epoch counter
/// - `valid(ref)` - Check if reference is still valid
/// - `same_allocation(a, b)` - Check if both point to same allocation
#[derive(Debug, Clone, PartialEq)]
pub enum GenerationPredicate {
    /// Get generation counter: `generation(ref: &T) -> u64`
    Generation { ref_expr: Box<Type> },

    /// Get epoch counter: `epoch(ref: &T) -> u16`
    Epoch { ref_expr: Box<Type> },

    /// Check if reference is valid: `valid(ref: &T) -> Bool`
    Valid { ref_expr: Box<Type> },

    /// Check same allocation: `same_allocation(a: &T, b: &U) -> Bool`
    SameAllocation { ref_a: Box<Type>, ref_b: Box<Type> },
}

// ==================== Specialization ====================

/// Specialization metadata for protocol implementations
///
/// Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — lines 549-663
///
/// Enables more specific implementations to override more general ones
/// with compile-time resolution based on specificity lattice.
///
/// # Precedence Lattice (most specific wins)
///
/// 1. Concrete type: `impl Show for List<Int>`
/// 2. Partially specialized: `impl<T> Show for List<T> where T: Copy`
/// 3. Generic: `impl<T> Show for List<T> where T: Display`
///
/// # Example
///
/// ```verum
/// // General implementation
/// implement<T> Display for List<T> where T: Display {
///     fn fmt(self: &Self, f: &mut Formatter) -> Result<(), Error> {
///         // Generic formatting
///     }
/// }
///
/// // Specialized implementation (more specific)
/// @specialize
/// implement Display for List<Text> {
///     fn fmt(self: &Self, f: &mut Formatter) -> Result<(), Error> {
///         // Optimized for List<Text>
///     }
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct SpecializationInfo {
    /// Whether this impl is marked with @specialize
    pub is_specialized: bool,

    /// Which implementation does this specialize (if any)
    pub specializes: Maybe<Path>,

    /// Specificity rank for precedence resolution
    /// Higher rank = more specific
    pub specificity_rank: usize,

    /// Whether this contains default methods that can be overridden
    pub is_default: bool,

    /// Source location
    pub span: Span,
}

impl SpecializationInfo {
    /// Create specialization info for a non-specialized impl
    pub fn none() -> Self {
        Self {
            is_specialized: false,
            specializes: Maybe::None,
            specificity_rank: 0,
            is_default: false,
            span: Span::default(),
        }
    }

    /// Create specialization info for a specialized impl
    pub fn specialized(specializes: Path, rank: usize) -> Self {
        Self {
            is_specialized: true,
            specializes: Maybe::Some(specializes),
            specificity_rank: rank,
            is_default: false,
            span: Span::default(),
        }
    }
}

/// Negative protocol bound for mutual exclusion
///
/// Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — .4 lines 623-638
///
/// Example:
/// ```verum
/// // These are mutually exclusive:
/// implement<T> MyProtocol for T where T: Send + Sync { }
///
/// @specialize
/// implement<T> MyProtocol for T where T: Send + !Sync { }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum ProtocolBoundPolarity {
    /// Positive bound: T: Protocol
    Positive { protocol: Path, args: List<Type> },

    /// Negative bound: T: !Protocol
    Negative { protocol: Path },
}

// ==================== Refinement Integration ====================

/// Refinement constraint in protocol method signature
///
/// Advanced protocols (future v2.0+): GATs, higher-rank bounds, specialization with lattice ordering, coherence rules — Section 5.4 lines 801-937
///
/// Tracks refinements on parameters and return types for:
/// - Inline syntax: `Int{> 0}`
/// - Declarative syntax: `Int where is_positive`
/// - Sigma-type syntax: `x: Int where x > 0`
#[derive(Debug, Clone, PartialEq)]
pub struct RefinementConstraint {
    /// Parameter or return value name
    pub name: Text,

    /// The refinement predicate
    pub predicate: RefinementPredicate,

    /// Which syntax form was used
    pub kind: RefinementKind,

    /// Source location
    pub span: Span,
}

/// Refinement predicate expression
#[derive(Debug, Clone, PartialEq)]
pub enum RefinementPredicate {
    /// Named predicate: `is_positive`, `is_sorted`, etc.
    Named { name: Text },

    /// Binary comparison: `> 0`, `!= 0`, `<= 100`
    BinaryOp { op: BinaryOp, value: ConstValue },

    /// Logical combination: `x > 0 && x < 100`
    And {
        left: Box<RefinementPredicate>,
        right: Box<RefinementPredicate>,
    },

    /// Logical disjunction: `x < 0 || x > 100`
    Or {
        left: Box<RefinementPredicate>,
        right: Box<RefinementPredicate>,
    },

    /// Negation: `!(x == 0)`
    Not { inner: Box<RefinementPredicate> },
}

/// Binary operator in refinement predicate
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Eq, // ==
    Ne, // !=
    Lt, // <
    Le, // <=
    Gt, // >
    Ge, // >=
}

// ConstValue is imported from crate::const_eval

/// Refinement syntax kind
///
/// Advanced protocols (future v2.0+): GATs, higher-rank bounds, specialization with lattice ordering, coherence rules — Section 5.4.1-5.4.3
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefinementKind {
    /// Inline: `Int{> 0}`
    Inline,

    /// Declarative: `Int where is_positive`
    Declarative,

    /// Sigma-type: `x: Int where x > 0`
    SigmaType,
}

// Kind is imported from crate::kind_inference

// ==================== Specialization Lattice ====================

/// Specialization lattice for coherence checking
///
/// Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — .2 lines 572-602
///
/// Orders implementations by specificity to select the most specific one.
#[derive(Debug, Clone)]
pub struct SpecializationLattice {
    /// All implementations for a protocol
    pub impls: List<usize>, // Indices into global impl list

    /// Partial order: impl_i < impl_j if impl_i is more specific
    pub ordering: Map<(usize, usize), bool>,

    /// Maximum element (most general implementation)
    pub max_element: Maybe<usize>,

    /// Minimal elements (most specific implementations)
    pub min_elements: Set<usize>,
}

impl SpecializationLattice {
    /// Create an empty lattice
    pub fn new() -> Self {
        Self {
            impls: List::new(),
            ordering: Map::new(),
            max_element: Maybe::None,
            min_elements: Set::new(),
        }
    }

    /// Add an implementation to the lattice
    pub fn add_impl(&mut self, impl_idx: usize) {
        self.impls.push(impl_idx);
    }

    /// Check if impl1 is more specific than impl2
    pub fn is_more_specific(&self, impl1: usize, impl2: usize) -> bool {
        self.ordering.get(&(impl1, impl2)).cloned().unwrap_or(false)
    }

    /// Get the most specific implementation applicable to a type
    /// Returns None if there's no unique most specific implementation (ambiguous)
    pub fn select_most_specific(&self, applicable: &Set<usize>) -> Maybe<usize> {
        if applicable.is_empty() {
            return Maybe::None;
        }

        if applicable.len() == 1 {
            return match applicable.iter().next() {
                Some(&idx) => Maybe::Some(idx),
                None => Maybe::None,
            };
        }

        // Find unique minimal element in applicable set
        // Must be more specific than all others (not just "not less specific")
        let mut minimal_candidate: Maybe<usize> = Maybe::None;

        for &candidate in applicable.iter() {
            let mut is_most_specific = true;

            for &other in applicable.iter() {
                if candidate != other {
                    // If other is more specific than candidate, candidate is not minimal
                    if self.is_more_specific(other, candidate) {
                        is_most_specific = false;
                        break;
                    }
                    // If neither is more specific than the other, this is ambiguous
                    // candidate must be more specific than other to be the unique minimal
                    if !self.is_more_specific(candidate, other) {
                        is_most_specific = false;
                        break;
                    }
                }
            }

            if is_most_specific {
                if minimal_candidate.is_some() {
                    // More than one minimal element = ambiguous
                    return Maybe::None;
                }
                minimal_candidate = Maybe::Some(candidate);
            }
        }

        minimal_candidate
    }
}

impl Default for SpecializationLattice {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Error Types ====================

/// Errors specific to advanced protocol features
#[derive(Debug, Clone, thiserror::Error)]
pub enum AdvancedProtocolError {
    /// GAT arity mismatch
    #[error("GAT {gat_name} expects {expected} type arguments, found {found}")]
    GATArityMismatch {
        gat_name: Text,
        expected: usize,
        found: usize,
    },

    /// GAT where clause not satisfied
    #[error("Type {ty:?} does not satisfy GAT constraint {constraint}")]
    GATConstraintNotSatisfied { ty: Type, constraint: Text },

    /// Specialization conflict (ambiguous)
    #[error("Ambiguous specialization: multiple equally-specific implementations for {ty:?}")]
    AmbiguousSpecialization { ty: Type, candidates: List<usize> },

    /// Invalid specialization (not more specific)
    #[error("Implementation marked @specialize but not more specific than base")]
    InvalidSpecialization { specialized: Path, base: Path },

    /// Refinement variance violation
    #[error("Implementation weakens refinement: {message}")]
    RefinementVarianceViolation { message: Text },

    /// Kind mismatch
    #[error("Kind mismatch: expected {expected:?}, found {found:?}")]
    KindMismatch { expected: Kind, found: Kind },

    /// GenRef generation mismatch
    #[error("GenRef generation mismatch: reference invalidated")]
    GenerationMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gat_type_param_creation() {
        let param = GATTypeParam {
            name: "T".into(),
            bounds: List::new(),
            default: Maybe::None,
            variance: Variance::Covariant,
        };

        assert_eq!(param.name, "T");
        assert_eq!(param.variance, Variance::Covariant);
    }

    #[test]
    fn test_associated_type_gat_simple() {
        let simple = AssociatedTypeGAT::simple("Item".into(), List::new());

        assert_eq!(simple.name, "Item");
        assert!(!simple.is_gat());
        assert_eq!(simple.arity(), 0);
        assert!(matches!(simple.kind, AssociatedTypeKind::Regular));
    }

    #[test]
    fn test_associated_type_gat_generic() {
        let type_params = List::from(vec![GATTypeParam {
            name: "T".into(),
            bounds: List::new(),
            default: Maybe::None,
            variance: Variance::Covariant,
        }]);

        let gat =
            AssociatedTypeGAT::generic("Wrapped".into(), type_params, List::new(), List::new());

        assert_eq!(gat.name, "Wrapped");
        assert!(gat.is_gat());
        assert_eq!(gat.arity(), 1);
        assert!(matches!(gat.kind, AssociatedTypeKind::Generic { arity: 1 }));
    }

    #[test]
    fn test_kind_arity() {
        let type_kind = Kind::type_kind();
        assert_eq!(type_kind.arity(), 0);

        let unary = Kind::unary_constructor();
        assert_eq!(unary.arity(), 1);

        let binary = Kind::binary_constructor();
        assert_eq!(binary.arity(), 2);
    }

    #[test]
    fn test_specialization_lattice() {
        let mut lattice = SpecializationLattice::new();

        lattice.add_impl(0); // General
        lattice.add_impl(1); // Specialized
        lattice.add_impl(2); // Most specialized

        // Set up ordering: 2 < 1 < 0 (2 is most specific)
        lattice.ordering.insert((2, 1), true);
        lattice.ordering.insert((1, 0), true);
        lattice.ordering.insert((2, 0), true); // Transitivity

        let applicable = Set::from_iter(vec![0, 1, 2]);
        let selected = lattice.select_most_specific(&applicable);

        assert_eq!(selected, Maybe::Some(2)); // Most specific wins
    }

    #[test]
    fn test_genref_type() {
        let inner = Type::Int;
        let genref = GenRefType::new(inner.clone());

        assert_eq!(genref.inner(), &Type::Int);
    }

    #[test]
    fn test_specialization_info() {
        let none = SpecializationInfo::none();
        assert!(!none.is_specialized);
        assert_eq!(none.specificity_rank, 0);

        let specialized = SpecializationInfo::specialized(
            Path::single(Ident::new("BaseImpl", Span::default())),
            5,
        );
        assert!(specialized.is_specialized);
        assert_eq!(specialized.specificity_rank, 5);
    }
}
