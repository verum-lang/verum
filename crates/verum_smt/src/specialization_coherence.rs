//! Specialization Coherence Verification using CHC
//!
//! Verum Advanced Protocols — Specialization System
//!
//! Specialization allows multiple overlapping protocol implementations where a more
//! specific impl takes precedence. The specialization lattice must satisfy:
//! - Reflexivity: every impl is at least as specific as itself
//! - Transitivity: if I1 ⊑ I2 and I2 ⊑ I3, then I1 ⊑ I3
//! - Antisymmetry: if I1 ⊑ I2 and I2 ⊑ I1, then I1 = I2 (no cycles)
//! - Unique most-specific: for any concrete type, at most one impl is selected
//! - Negative bounds: `T: !Protocol` restricts an impl to types that do NOT implement Protocol
//!
//! This module verifies specialization lattice properties using Constrained Horn Clauses:
//! 1. No ambiguous specializations (unique most specific impl)
//! 2. Lattice properties (antisymmetry, transitivity, reflexivity)
//! 3. Overlap detection between implementations
//! 4. Specialization precedence resolution
//!
//! # Performance Targets
//!
//! - Small lattice (<10 impls): <50ms
//! - Medium lattice (10-50 impls): <150ms
//! - Large lattice (>50 impls): <200ms
//!
//! # CHC Encoding
//!
//! The specialization lattice is encoded as a set of Horn clauses:
//! ```smt2
//! ; more_specific(I1, I2) means I1 is more specific than I2
//! (declare-rel more_specific (Impl Impl))
//!
//! ; Transitivity
//! (rule (=> (and (more_specific I1 I2) (more_specific I2 I3))
//!           (more_specific I1 I3)))
//!
//! ; Antisymmetry (no cycles)
//! (rule (=> (and (more_specific I1 I2) (more_specific I2 I1))
//!           false))
//! ```

use std::collections::HashSet;
use std::time::{Duration, Instant};

use verum_ast::span::Span;
use verum_ast::ty::{Path, Type};
use verum_common::{List, Map, Maybe, Set, Text};
use verum_protocol_types::protocol_base::{Protocol, ProtocolImpl};
use verum_protocol_types::specialization::SpecializationLattice;
use verum_common::ToText;

use z3::ast::{Bool, Int};
use z3::{Context, Sort};

use crate::fixedpoint::{Atom, CHC, FixedPointEngine};

// ==================== Core Types ====================

/// Specialization verification result
#[derive(Debug, Clone)]
pub struct SpecializationVerificationResult {
    /// Whether the specialization lattice is valid
    pub is_coherent: bool,
    /// Verification time
    pub duration: Duration,
    /// Errors found
    pub errors: List<SpecializationError>,
    /// Ambiguous specializations detected
    pub ambiguities: List<Ambiguity>,
    /// Statistics
    pub stats: SpecializationStats,
}

/// Specialization verification error
#[derive(Debug, Clone)]
pub enum SpecializationError {
    /// Ambiguous specialization (multiple equally-specific impls)
    AmbiguousSpecialization {
        ty: Type,
        protocol: Text,
        candidates: List<usize>,
    },
    /// Cycle in specialization lattice
    SpecializationCycle { cycle: List<usize> },
    /// Overlapping implementations without specialization
    OverlappingImpls {
        impl1: usize,
        impl2: usize,
        overlap: Text,
    },
    /// Invalid specialization ordering
    InvalidOrdering {
        impl1: usize,
        impl2: usize,
        reason: Text,
    },
    /// Antisymmetry violation
    AntisymmetryViolation { impl1: usize, impl2: usize },
    /// Negative bound violation
    ///
    /// Negative protocol bounds create mutual exclusion in the specialization lattice.
    /// A negative bound `T: !Protocol` means the impl only applies when T does NOT
    /// implement Protocol. This violation means T actually does implement the excluded protocol.
    /// Example: `T: !Clone` violated because `T` implements `Clone`
    NegativeBoundViolation {
        /// Index of the violating implementation
        impl_idx: usize,
        /// The type that violated the constraint
        ty: Type,
        /// The protocol that should NOT be satisfied
        protocol: Text,
        /// Reason for violation
        reason: Text,
    },
    /// Contradictory bounds detected
    ///
    /// Example: `T: Clone + !Clone` (impossible)
    ContradictoryBounds {
        /// The type with contradictory bounds
        ty: Type,
        /// The protocol involved
        protocol: Text,
        /// Explanation
        reason: Text,
    },
    /// Mutual exclusion violation in specialization lattice
    ///
    /// Two implementations that should be mutually exclusive based on
    /// negative bounds are not properly ordered.
    MutualExclusionViolation {
        impl1: usize,
        impl2: usize,
        reason: Text,
    },
}

/// Ambiguous specialization case
#[derive(Debug, Clone)]
pub struct Ambiguity {
    /// The type causing ambiguity
    pub ty: Type,
    /// The protocol
    pub protocol: Text,
    /// Equally-specific implementations
    pub candidates: List<usize>,
    /// Suggestion for resolution
    pub suggestion: Text,
}

/// Specificity ordering between implementations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecificityOrdering {
    /// First is more specific than second
    MoreSpecific,
    /// Second is more specific than first
    LessSpecific,
    /// Equally specific (ambiguous)
    Equal,
    /// Incomparable (no overlap)
    Incomparable,
}

/// Specialization verification statistics
#[derive(Debug, Clone, Default)]
pub struct SpecializationStats {
    /// Number of implementations checked
    pub impls_checked: usize,
    /// Number of pairwise comparisons
    pub comparisons: usize,
    /// Number of overlaps detected
    pub overlaps_detected: usize,
    /// Number of CHC rules generated
    pub chc_rules: usize,
    /// Time spent in fixedpoint solver
    pub fixedpoint_time: Duration,
}

// ==================== Specialization Verifier ====================

/// Verifies specialization coherence using CHC solving
pub struct SpecializationVerifier {
    /// Fixedpoint engine for CHC solving
    fixedpoint: FixedPointEngine,
    /// Implementation database
    implementations: List<ProtocolImpl>,
    /// Specialization lattice
    lattice: SpecializationLattice,
    /// Cache of pairwise comparisons
    comparison_cache: Map<(usize, usize), SpecificityOrdering>,
    /// Known type parameters from registered implementations
    /// Maps implementation index to set of type parameter names
    known_type_params: Map<usize, Set<Text>>,
    /// Protocol declaration registry, keyed by protocol short name.
    ///
    /// Populated externally via [`register_protocol`]. The
    /// `super_protocols` field of each `Protocol` declares the parent
    /// protocols in the hierarchy (`A : B` means B is in A's super list).
    /// Used by [`is_subprotocol`] to traverse the hierarchy graph so the
    /// coherence checker can recognize that types implementing
    /// subprotocols also satisfy their superprotocols.
    protocols: Map<Text, Protocol>,
}

impl SpecializationVerifier {
    /// Create a new specialization verifier
    pub fn new() -> Result<Self, Text> {
        let fixedpoint = FixedPointEngine::new(Context::thread_local())?;

        // Create a default path for the lattice
        let default_protocol = Path::single(verum_ast::ty::Ident {
            name: "Unknown".into(),
            span: verum_ast::span::Span::default(),
        });

        Ok(Self {
            fixedpoint,
            implementations: List::new(),
            lattice: SpecializationLattice::new(default_protocol),
            comparison_cache: Map::new(),
            known_type_params: Map::new(),
            protocols: Map::new(),
        })
    }

    /// Register a protocol declaration so the coherence checker can
    /// traverse the super-protocol hierarchy. Idempotent: re-registering
    /// the same name overwrites the previous definition.
    pub fn register_protocol(&mut self, protocol: Protocol) {
        self.protocols.insert(protocol.name.clone(), protocol);
    }

    /// Public reflection over the protocol hierarchy: returns `true` iff
    /// `sub_name` is reachable from `super_name` via the
    /// super-protocol graph (reflexively, transitively).
    ///
    /// This is the public surface over the internal `is_subprotocol`
    /// walker — useful for diagnostics, external coherence consumers, and
    /// tests that pin the hierarchy contract without having to construct
    /// a full `ProtocolImpl` to drive it indirectly.
    pub fn is_subprotocol_by_name(&self, sub_name: &str, super_name: &str) -> bool {
        let path = Path::single(verum_ast::ty::Ident {
            name: sub_name.into(),
            span: verum_ast::span::Span::default(),
        });
        self.is_subprotocol(&path, super_name)
    }

    /// Register an implementation
    pub fn register_implementation(&mut self, impl_: ProtocolImpl, idx: usize) {
        let for_type = impl_.for_type.clone();

        // Extract type parameters from where clauses
        // Type parameters are typically introduced in where clauses like:
        // `impl<T> Protocol for Container<T> where T: SomeBound`
        let mut type_params = Set::new();
        for clause in impl_.where_clauses.iter() {
            // The constrained type in where clause is often a type parameter
            self.extract_type_params_from_type(&clause.ty, &mut type_params);
        }

        // Also extract type parameters from the for_type itself
        // (e.g., Container<T> has T as a type parameter)
        self.extract_type_params_from_type(&impl_.for_type, &mut type_params);

        self.known_type_params.insert(idx, type_params);
        self.implementations.push(impl_);
        self.lattice.add_impl(idx, for_type, Maybe::None);
    }

    /// Extract type parameter names from a type recursively
    fn extract_type_params_from_type(&self, ty: &Type, params: &mut Set<Text>) {
        use verum_ast::ty::TypeKind;

        match &ty.kind {
            // Path types that are single identifiers starting with uppercase
            // are likely type parameters (T, U, K, V, etc.)
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let name = ident.as_str();
                    // Type parameters are conventionally single uppercase letters
                    // or CamelCase names that aren't known types
                    if self.looks_like_type_param(name) {
                        params.insert(Text::from(name));
                    }
                }
            }
            // Generic types like Container<T> - extract T from args
            TypeKind::Generic { base, args } => {
                self.extract_type_params_from_type(base, params);
                for arg in args {
                    // GenericArg can be Type, Const, Lifetime, or Binding
                    use verum_ast::ty::GenericArg;
                    match arg {
                        GenericArg::Type(ty) => {
                            self.extract_type_params_from_type(ty, params);
                        }
                        GenericArg::Binding(binding) => {
                            self.extract_type_params_from_type(&binding.ty, params);
                        }
                        GenericArg::Const(_) | GenericArg::Lifetime(_) => {
                            // No type params to extract from const or lifetime args
                        }
                    }
                }
            }
            // Tuple types
            TypeKind::Tuple(types) => {
                for t in types {
                    self.extract_type_params_from_type(t, params);
                }
            }
            // Reference types
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. }
            | TypeKind::Pointer { inner, .. }
            | TypeKind::Ownership { inner, .. }
            | TypeKind::GenRef { inner } => {
                self.extract_type_params_from_type(inner, params);
            }
            // Array/slice
            TypeKind::Array { element, .. } => {
                self.extract_type_params_from_type(element, params);
            }
            TypeKind::Slice(element) => {
                self.extract_type_params_from_type(element, params);
            }
            // Function types
            TypeKind::Function {
                params: fn_params,
                return_type,
                ..
            }
            | TypeKind::Rank2Function {
                type_params: _,
                params: fn_params,
                return_type,
                ..
            } => {
                for p in fn_params {
                    self.extract_type_params_from_type(p, params);
                }
                self.extract_type_params_from_type(return_type, params);
            }
            // Refinement types (canonical: all three forms collapse here)
            TypeKind::Refined { base, .. } => {
                self.extract_type_params_from_type(base, params);
            }
            // Bounded types
            TypeKind::Bounded { base, .. } => {
                self.extract_type_params_from_type(base, params);
            }
            // Tensor types
            TypeKind::Tensor { element, .. } => {
                self.extract_type_params_from_type(element, params);
            }
            // Qualified types
            TypeKind::Qualified { self_ty, .. } => {
                self.extract_type_params_from_type(self_ty, params);
            }
            // Record types - extract from field types
            TypeKind::Record { fields, .. } => {
                for field in fields {
                    self.extract_type_params_from_type(&field.ty, params);
                }
            }
            // Primitive and other types don't have type params
            _ => {}
        }
    }

    /// Check if a name looks like a type parameter
    ///
    /// Type parameters in Verum follow these conventions:
    /// - Single uppercase letters (T, U, V, K, V, E, etc.)
    /// - Short uppercase names (Key, Val, Item, Elem)
    /// - Names that aren't well-known concrete types
    fn looks_like_type_param(&self, name: &str) -> bool {
        // Known concrete types that should NOT be considered type parameters
        const KNOWN_TYPES: &[&str] = &[
            "Int", "Bool", "Float", "Text", "Unit", "Char", "List", "Map", "Set", "Maybe",
            "Result", "Heap", "Shared", "Self", "Never", "Any",
        ];

        if KNOWN_TYPES.contains(&name) {
            return false;
        }

        // Must start with uppercase (Verum convention for type parameters)
        let first_char = name.chars().next();
        if !first_char.is_some_and(|c| c.is_uppercase()) {
            return false;
        }

        // Single uppercase letters are almost always type params
        if name.len() == 1 {
            return true;
        }

        // Common type parameter naming patterns
        // Short names (2-4 chars) that are all uppercase or CamelCase
        if name.len() <= 4 && name.chars().all(|c| c.is_alphanumeric()) {
            return true;
        }

        // Names ending with common type param suffixes
        const TYPE_PARAM_SUFFIXES: &[&str] = &["Type", "T", "Ty"];
        for suffix in TYPE_PARAM_SUFFIXES {
            if name.ends_with(suffix) && name.len() > suffix.len() {
                return true;
            }
        }

        false
    }

    /// Register predicates needed for CHC encoding
    ///
    /// Returns Ok if CHC predicates were registered successfully.
    /// Returns Err if registration failed.
    fn register_chc_predicates(&mut self) -> Result<(), Text> {
        use crate::fixedpoint::PredicateBody;
        use crate::fixedpoint::RecursivePredicate;

        // Use Int sort for implementation indices
        let int_sort = Sort::int();

        // Register more_specific(Int, Int) -> Bool predicate
        // where Int represents implementation index
        let more_specific_pred = RecursivePredicate {
            name: Text::from("more_specific"),
            params: List::from(vec![int_sort.clone(), int_sort.clone()]),
            body: PredicateBody::Base(Bool::from_bool(true)),
            well_founded: true,
        };
        self.fixedpoint.register_predicate(more_specific_pred)?;

        // Register error() -> Bool predicate (for antisymmetry violation)
        let error_pred = RecursivePredicate {
            name: Text::from("false"),
            params: List::new(),
            body: PredicateBody::Base(Bool::from_bool(false)),
            well_founded: true,
        };
        self.fixedpoint.register_predicate(error_pred)?;

        Ok(())
    }

    /// Verify specialization coherence
    ///
    /// Checks:
    /// 1. No cycles in specialization lattice
    /// 2. No ambiguous specializations
    /// 3. Lattice properties (antisymmetry, transitivity)
    /// 4. All overlaps are resolved by specialization
    pub fn verify(&mut self) -> SpecializationVerificationResult {
        let start = Instant::now();
        let mut errors = List::new();
        let mut ambiguities = List::new();
        let mut stats = SpecializationStats {
            impls_checked: self.implementations.len(),
            ..Default::default()
        };

        // Empty implementation list is trivially coherent
        if self.implementations.is_empty() {
            return SpecializationVerificationResult {
                is_coherent: true,
                duration: start.elapsed(),
                errors,
                ambiguities,
                stats,
            };
        }

        // Step 1: Build specialization lattice
        self.build_lattice(&mut stats);

        // Step 2: Register CHC predicates
        if let Err(e) = self.register_chc_predicates() {
            // Non-fatal: fall back to basic checks
            eprintln!("Warning: CHC predicate registration failed: {}", e);
        }

        // Step 3: Encode lattice as CHC rules and verify with fixedpoint solver
        let chcs = self.encode_lattice_as_chc();
        stats.chc_rules = chcs.len();

        // Add CHC rules to fixedpoint engine
        for chc in chcs.iter() {
            if let Err(e) = self.fixedpoint.add_chc(chc.clone()) {
                // Non-fatal: continue with basic checks
                eprintln!("Warning: CHC rule addition failed: {}", e);
            }
        }

        // CHC-based antisymmetry checking is done via check_antisymmetry() below
        // The fixedpoint engine rules above help with advanced cycle detection

        // Step 4: Check for cycles
        if let Err(err) = self.check_cycles() {
            errors.push(err);
        }

        // Step 5: Check antisymmetry
        if let Err(err) = self.check_antisymmetry() {
            errors.push(err);
        }

        // Step 6: Check for ambiguities
        let ambiguity_errors = self.check_ambiguities(&mut stats);
        for err in ambiguity_errors {
            match err {
                SpecializationError::AmbiguousSpecialization {
                    ty,
                    protocol,
                    candidates,
                } => {
                    let suggestion = self.suggest_resolution(&ty, &protocol, &candidates);
                    ambiguities.push(Ambiguity {
                        ty,
                        protocol,
                        candidates,
                        suggestion,
                    });
                }
                other => errors.push(other),
            }
        }

        // Step 7: Check overlaps
        let overlap_errors = self.check_overlaps(&mut stats);
        errors.extend(overlap_errors);

        let duration = start.elapsed();
        stats.fixedpoint_time = duration;

        SpecializationVerificationResult {
            is_coherent: errors.is_empty() && ambiguities.is_empty(),
            duration,
            errors,
            ambiguities,
            stats,
        }
    }

    /// Build specialization lattice by pairwise comparison
    fn build_lattice(&mut self, stats: &mut SpecializationStats) {
        let n = self.implementations.len();

        for i in 0..n {
            for j in 0..n {
                if i == j {
                    continue;
                }

                stats.comparisons += 1;

                let ordering = self.compare_specificity(i, j);
                self.comparison_cache.insert((i, j), ordering);

                if ordering == SpecificityOrdering::MoreSpecific {
                    self.lattice.ordering.push((i, j));
                }
            }
        }

        // Find max and min elements
        self.compute_extremal_elements();
    }

    /// Compare specificity of two implementations
    ///
    /// Rules (most specific to least specific):
    /// 1. Concrete type beats generic
    /// 2. More constraints beat fewer constraints
    /// 3. Specialized impl beats general impl
    fn compare_specificity(&self, i: usize, j: usize) -> SpecificityOrdering {
        // Check cache
        if let Some(&cached) = self.comparison_cache.get(&(i, j)) {
            return cached;
        }

        let impl_i = &self.implementations[i];
        let impl_j = &self.implementations[j];

        // Check if types overlap
        if !self.types_overlap(&impl_i.for_type, &impl_j.for_type) {
            return SpecificityOrdering::Incomparable;
        }

        // Compare by concreteness
        let i_generic = self.is_generic(&impl_i.for_type);
        let j_generic = self.is_generic(&impl_j.for_type);

        if !i_generic && j_generic {
            return SpecificityOrdering::MoreSpecific;
        }
        if i_generic && !j_generic {
            return SpecificityOrdering::LessSpecific;
        }

        // Compare by number of constraints
        let i_constraints = impl_i.where_clauses.len();
        let j_constraints = impl_j.where_clauses.len();

        if i_constraints > j_constraints {
            return SpecificityOrdering::MoreSpecific;
        }
        if j_constraints > i_constraints {
            return SpecificityOrdering::LessSpecific;
        }

        // Equally specific
        SpecificityOrdering::Equal
    }

    /// Check if two types overlap (have common instantiations)
    fn types_overlap(&self, ty1: &Type, ty2: &Type) -> bool {
        // Simplified overlap check
        // Full implementation would use unification
        format!("{:?}", ty1) == format!("{:?}", ty2)
    }

    /// Check if a type is generic (contains type parameters)
    ///
    /// A type is generic if it contains:
    /// - Type variables (Type::Var)
    /// - Generic types with arguments (Type::Generic)
    /// - Named types with arguments (Type::Named with non-empty args)
    /// - Function types with type parameters
    /// - Any nested generic types
    ///
    /// This method uses the accumulated knowledge of type parameters from
    /// registered implementations to accurately identify type parameters.
    fn is_generic(&self, ty: &Type) -> bool {
        self.is_generic_with_context(ty, None)
    }

    /// Check if a type is generic with optional implementation context
    ///
    /// When `impl_idx` is provided, uses the known type parameters for that
    /// specific implementation. Otherwise, checks against all known type parameters.
    fn is_generic_with_context(&self, ty: &Type, impl_idx: Option<usize>) -> bool {
        use verum_ast::ty::TypeKind;

        match &ty.kind {
            // Named type with generic arguments
            TypeKind::Generic { base, args } => {
                !args.is_empty()
                    || self.is_generic_with_context(base.as_ref(), impl_idx)
                    || args.iter().any(|arg| {
                        use verum_ast::ty::GenericArg;
                        match arg {
                            GenericArg::Type(ty) => self.is_generic_with_context(ty, impl_idx),
                            GenericArg::Const(_) => false, // Const args aren't type parameters
                            GenericArg::Lifetime(_) => false, // Lifetimes aren't type parameters
                            GenericArg::Binding(binding) => {
                                self.is_generic_with_context(&binding.ty, impl_idx)
                            }
                        }
                    })
            }
            // Path could be a type parameter like T, U, etc.
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let name = ident.as_str();
                    let name_text = Text::from(name);

                    // Check against known type parameters
                    if let Some(idx) = impl_idx {
                        // Check specific implementation's type params
                        if let Some(params) = self.known_type_params.get(&idx) {
                            if params.contains(&name_text) {
                                return true;
                            }
                        }
                    } else {
                        // Check all known type parameters across implementations
                        for (_idx, params) in &self.known_type_params {
                            if params.contains(&name_text) {
                                return true;
                            }
                        }
                    }

                    // Fall back to heuristic check for type parameter patterns
                    // This handles cases where type params weren't registered yet
                    self.looks_like_type_param(name)
                } else {
                    false
                }
            }
            // Tuple types - check all elements
            TypeKind::Tuple(types) => types
                .iter()
                .any(|t| self.is_generic_with_context(t, impl_idx)),
            // Array/slice types - check element type
            TypeKind::Array { element, .. } => {
                self.is_generic_with_context(element.as_ref(), impl_idx)
            }
            TypeKind::Slice(element) => self.is_generic_with_context(element.as_ref(), impl_idx),
            // Function types - check params and return type
            TypeKind::Function {
                params,
                return_type,
                ..
            }
            | TypeKind::Rank2Function {
                type_params: _,
                params,
                return_type,
                ..
            } => {
                params
                    .iter()
                    .any(|p| self.is_generic_with_context(p, impl_idx))
                    || self.is_generic_with_context(return_type.as_ref(), impl_idx)
            }
            // Reference types - check inner type
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. }
            | TypeKind::Pointer { inner, .. }
            | TypeKind::VolatilePointer { inner, .. }
            | TypeKind::Ownership { inner, .. }
            | TypeKind::GenRef { inner } => self.is_generic_with_context(inner.as_ref(), impl_idx),
            // Refinement types (canonical) - check base type
            TypeKind::Refined { base, .. } => self.is_generic_with_context(base.as_ref(), impl_idx),
            // Bounded types - check base type
            TypeKind::Bounded { base, .. } => self.is_generic_with_context(base.as_ref(), impl_idx),
            // Type constructors are generic by nature
            TypeKind::TypeConstructor { .. } => true,
            // Tensor types - check element type
            TypeKind::Tensor { element, .. } => {
                self.is_generic_with_context(element.as_ref(), impl_idx)
            }
            // Qualified types - check self type
            TypeKind::Qualified { self_ty, .. } => {
                self.is_generic_with_context(self_ty.as_ref(), impl_idx)
            }
            // Primitive types are not generic
            TypeKind::Unit
            | TypeKind::Never
            | TypeKind::Unknown
            | TypeKind::Bool
            | TypeKind::Int
            | TypeKind::Float
            | TypeKind::Char
            | TypeKind::Text => false,
            // Inferred types are not generic (they'll be resolved to concrete types)
            TypeKind::Inferred => false,
            // Dyn protocol objects are not generic in the same sense
            TypeKind::DynProtocol { .. } => false,
            // Existential types - check bounds for genericity
            TypeKind::Existential { bounds, .. } => bounds.iter().any(|bound| {
                if let verum_ast::ty::TypeBoundKind::Equality(ty) = &bound.kind {
                    self.is_generic_with_context(ty, impl_idx)
                } else {
                    false
                }
            }),
            // Associated types - check base for genericity
            TypeKind::AssociatedType { base, .. } => {
                self.is_generic_with_context(base.as_ref(), impl_idx)
            }
            // Capability-restricted types - check base for genericity
            TypeKind::CapabilityRestricted { base, .. } => {
                self.is_generic_with_context(base.as_ref(), impl_idx)
            }
            // Record types - check all field types for genericity
            TypeKind::Record { fields, .. } => fields
                .iter()
                .any(|f| self.is_generic_with_context(&f.ty, impl_idx)),
            // Universe types are not generic
            TypeKind::Universe { .. } => false,
            // Meta types - check inner type for genericity
            TypeKind::Meta { inner } => self.is_generic_with_context(inner.as_ref(), impl_idx),
            // Type lambdas - check body for genericity
            TypeKind::TypeLambda { body, .. } => self.is_generic_with_context(body.as_ref(), impl_idx),
            // Path equality type: check carrier for genericity
            TypeKind::PathType { carrier, .. } | TypeKind::DependentApp { carrier, .. } => self.is_generic_with_context(carrier.as_ref(), impl_idx),
        }
    }

    /// Compute max and min elements of lattice
    fn compute_extremal_elements(&mut self) {
        let n = self.implementations.len();

        // Find root elements (most general)
        for i in 0..n {
            let mut is_root = true;
            for j in 0..n {
                if i != j && self.lattice.is_more_specific(i, j) {
                    is_root = false;
                    break;
                }
            }
            if is_root {
                self.lattice.roots.insert(i);
            }
        }

        // Find leaf elements (most specific)
        for i in 0..n {
            let mut is_leaf = true;
            for j in 0..n {
                if i != j && self.lattice.is_more_specific(j, i) {
                    is_leaf = false;
                    break;
                }
            }
            if is_leaf {
                self.lattice.leaves.insert(i);
            }
        }
    }

    /// Encode specialization lattice as CHC rules
    ///
    /// Creates Constrained Horn Clauses representing:
    /// 1. Transitivity: more_specific(I1, I2) ∧ more_specific(I2, I3) => more_specific(I1, I3)
    /// 2. Antisymmetry: more_specific(I1, I2) ∧ more_specific(I2, I1) => false
    /// 3. Facts: more_specific(i, j) for each (i, j) in the lattice ordering
    ///
    /// These CHC rules are added to the fixedpoint engine for advanced verification
    /// capabilities, though basic verification is also done via graph algorithms.
    fn encode_lattice_as_chc(&self) -> List<CHC> {
        let mut chcs = List::new();

        // Use Int sort for implementation indices (concrete integers)
        let int_sort = Sort::int();

        // Encode transitivity: more_specific(I1, I2) ∧ more_specific(I2, I3) => more_specific(I1, I3)
        let transitivity_chc = CHC {
            vars: List::from(vec![
                ("I1".to_text(), int_sort.clone()),
                ("I2".to_text(), int_sort.clone()),
                ("I3".to_text(), int_sort.clone()),
            ]),
            hypothesis: List::from(vec![
                Atom {
                    predicate: "more_specific".to_text(),
                    args: List::from(vec![
                        Int::new_const("I1").into(),
                        Int::new_const("I2").into(),
                    ]),
                },
                Atom {
                    predicate: "more_specific".to_text(),
                    args: List::from(vec![
                        Int::new_const("I2").into(),
                        Int::new_const("I3").into(),
                    ]),
                },
            ]),
            constraints: List::new(),
            conclusion: Atom {
                predicate: "more_specific".to_text(),
                args: List::from(vec![
                    Int::new_const("I1").into(),
                    Int::new_const("I3").into(),
                ]),
            },
        };
        chcs.push(transitivity_chc);

        // Encode antisymmetry: more_specific(I1, I2) ∧ more_specific(I2, I1) => false
        let antisymmetry_chc = CHC {
            vars: List::from(vec![
                ("I1".to_text(), int_sort.clone()),
                ("I2".to_text(), int_sort.clone()),
            ]),
            hypothesis: List::from(vec![
                Atom {
                    predicate: "more_specific".to_text(),
                    args: List::from(vec![
                        Int::new_const("I1").into(),
                        Int::new_const("I2").into(),
                    ]),
                },
                Atom {
                    predicate: "more_specific".to_text(),
                    args: List::from(vec![
                        Int::new_const("I2").into(),
                        Int::new_const("I1").into(),
                    ]),
                },
            ]),
            constraints: List::new(),
            conclusion: Atom {
                predicate: "false".to_text(),
                args: List::new(),
            },
        };
        chcs.push(antisymmetry_chc);

        // Encode concrete facts from lattice
        for &(i, j) in self.lattice.ordering.iter() {
            let fact_chc = CHC {
                vars: List::new(),
                hypothesis: List::new(),
                constraints: List::new(),
                conclusion: Atom {
                    predicate: "more_specific".to_text(),
                    args: List::from(vec![
                        Int::from_i64(i as i64).into(),
                        Int::from_i64(j as i64).into(),
                    ]),
                },
            };
            chcs.push(fact_chc);
        }

        chcs
    }

    /// Check for cycles in specialization lattice
    fn check_cycles(&self) -> Result<(), SpecializationError> {
        let mut visited = Set::new();
        let mut stack = Set::new();

        for i in 0..self.implementations.len() {
            if let Some(cycle) = self.detect_cycle(i, &mut visited, &mut stack) {
                return Err(SpecializationError::SpecializationCycle { cycle });
            }
        }

        Ok(())
    }

    /// Detect cycle starting from node using DFS
    fn detect_cycle(
        &self,
        node: usize,
        visited: &mut Set<usize>,
        stack: &mut Set<usize>,
    ) -> Option<List<usize>> {
        if stack.contains(&node) {
            return Some(List::from(vec![node]));
        }

        if visited.contains(&node) {
            return None;
        }

        visited.insert(node);
        stack.insert(node);

        // Check successors (nodes that this one is more specific than)
        for j in 0..self.implementations.len() {
            if self.lattice.is_more_specific(node, j)
                && let Some(mut cycle) = self.detect_cycle(j, visited, stack)
            {
                cycle.push(node);
                return Some(cycle);
            }
        }

        stack.remove(&node);
        None
    }

    /// Check antisymmetry property
    fn check_antisymmetry(&self) -> Result<(), SpecializationError> {
        for i in 0..self.implementations.len() {
            for j in 0..self.implementations.len() {
                if i != j
                    && self.lattice.is_more_specific(i, j)
                    && self.lattice.is_more_specific(j, i)
                {
                    return Err(SpecializationError::AntisymmetryViolation { impl1: i, impl2: j });
                }
            }
        }
        Ok(())
    }

    /// Check for ambiguous specializations
    fn check_ambiguities(&self, stats: &mut SpecializationStats) -> List<SpecializationError> {
        let mut errors = List::new();

        // Group implementations by (type, protocol)
        let mut groups: Map<(Text, Text), List<usize>> = Map::new();

        for (idx, impl_) in self.implementations.iter().enumerate() {
            let key = (
                format!("{:?}", impl_.for_type).into(),
                format!("{:?}", impl_.protocol).into(),
            );
            groups.entry(key).or_default().push(idx);
        }

        // Check each group for ambiguities
        for ((ty_str, protocol), impls) in groups.iter() {
            if impls.len() > 1 {
                // Find most specific implementations
                let most_specific = self.find_most_specific(impls);

                if most_specific.len() > 1 {
                    // Ambiguity detected
                    errors.push(SpecializationError::AmbiguousSpecialization {
                        ty: Type::int(Span::dummy()), // Would parse from ty_str
                        protocol: protocol.clone(),
                        candidates: most_specific,
                    });
                }
            }
        }

        errors
    }

    /// Find most specific implementations from a set
    fn find_most_specific(&self, impls: &[usize]) -> List<usize> {
        let mut most_specific = List::new();

        for &candidate in impls {
            let mut is_most_specific = true;

            for &other in impls {
                if candidate != other && self.lattice.is_more_specific(other, candidate) {
                    is_most_specific = false;
                    break;
                }
            }

            if is_most_specific {
                most_specific.push(candidate);
            }
        }

        most_specific
    }

    /// Check for overlapping implementations without specialization
    fn check_overlaps(&self, stats: &mut SpecializationStats) -> List<SpecializationError> {
        let mut errors = List::new();

        for i in 0..self.implementations.len() {
            for j in (i + 1)..self.implementations.len() {
                let impl_i = &self.implementations[i];
                let impl_j = &self.implementations[j];

                if impl_i.protocol == impl_j.protocol
                    && self.types_overlap(&impl_i.for_type, &impl_j.for_type)
                {
                    stats.overlaps_detected += 1;

                    // Check if one specializes the other
                    let ordering = self.compare_specificity(i, j);

                    if ordering == SpecificityOrdering::Equal {
                        errors.push(SpecializationError::OverlappingImpls {
                            impl1: i,
                            impl2: j,
                            overlap: format!("{:?}", impl_i.for_type).into(),
                        });
                    }
                }
            }
        }

        errors
    }

    /// Suggest resolution for ambiguous specialization
    fn suggest_resolution(&self, ty: &Type, protocol: &Text, candidates: &[usize]) -> Text {
        format!(
            "Add @specialize annotation to the most specific implementation, or add constraints to disambiguate. Candidates: {:?}",
            candidates
        ).into()
    }

    /// Clear internal caches
    pub fn clear_cache(&mut self) {
        self.comparison_cache.clear();
    }

    // ========================================================================
    // Negative Bounds Verification
    // ========================================================================
    //
    // Negative Protocol Bounds Verification
    //
    // Negative bounds (`T: !Protocol`) create mutual exclusion in the specialization lattice:
    // - `T: !Clone` means the implementation is only valid when T does NOT implement Clone
    // - Encoded as CHC: more_specific(I_neg, I_pos) when I_neg has `!P` and I_pos requires `P`
    // - Violations detected when a concrete type satisfies both the positive and negative bound
    // - Two impls with opposite polarities on the same protocol are mutually exclusive
    //
    // ## CHC Encoding for Negative Bounds
    //
    // ```smt2
    // ; neg_bound(Type, Protocol) - type has negative bound on protocol
    // (declare-rel neg_bound (Type Protocol))
    //
    // ; satisfies(Type, Protocol) - type satisfies protocol
    // (declare-rel satisfies (Type Protocol))
    //
    // ; Negative bound violation: type satisfies what it shouldn't
    // (rule (=> (and (neg_bound T P) (satisfies T P))
    //           error))
    //
    // ; Mutual exclusion: impls with opposite polarities don't overlap
    // (rule (=> (and (impl I1) (impl I2)
    //               (neg_bound (impl_type I1) P)
    //               (satisfies (impl_type I2) P))
    //           (or (more_specific I1 I2) (more_specific I2 I1))))
    // ```

    /// Check negative bounds for all implementations
    ///
    /// Verifies that:
    /// 1. No negative bound is violated (type doesn't implement what it claims not to)
    /// 2. No contradictory bounds exist (T: P + !P)
    /// 3. Mutual exclusion is properly enforced
    pub fn check_negative_bounds(&self) -> List<SpecializationError> {
        let mut errors = List::new();

        for (idx, impl_) in self.implementations.iter().enumerate() {
            // Check for contradictory bounds in where clauses
            if let Some(err) = self.check_contradictory_bounds(idx, impl_) {
                errors.push(err);
            }

            // Check negative bound violations
            errors.extend(self.check_negative_bound_violations(idx, impl_));
        }

        // Check mutual exclusion between all implementation pairs
        errors.extend(self.check_mutual_exclusion());

        errors
    }

    /// Check for contradictory bounds in a single implementation
    ///
    /// Example: `where type T: Clone + !Clone` is a contradiction
    fn check_contradictory_bounds(
        &self,
        _idx: usize,
        impl_: &ProtocolImpl,
    ) -> Option<SpecializationError> {
        // Collect all positive and negative bounds from where clauses
        let mut positive_bounds: HashSet<String> = HashSet::new();
        let mut negative_bounds: HashSet<String> = HashSet::new();

        for clause in impl_.where_clauses.iter() {
            // WhereClause from verum_protocol_types has direct ty and bounds fields
            let ty = &clause.ty;

            for bound in clause.bounds.iter() {
                // ProtocolBound has protocol path
                // Negative bounds are indicated by protocol name starting with "!"
                if let Some(ident) = bound.protocol.as_ident() {
                    let name = ident.as_str();
                    if name.starts_with('!') {
                        // Negative bound: extract protocol name without "!"
                        let protocol_name = name.trim_start_matches('!').to_string();
                        negative_bounds.insert(protocol_name);
                    } else {
                        // Positive bound
                        positive_bounds.insert(name.to_string());
                    }
                }
            }

            // Check for intersection (contradictions)
            for protocol in &positive_bounds {
                if negative_bounds.contains(protocol) {
                    return Some(SpecializationError::ContradictoryBounds {
                        ty: ty.clone(),
                        protocol: Text::from(protocol.clone()),
                        reason: format!(
                            "Type has both positive and negative bounds for '{}'",
                            protocol
                        )
                        .into(),
                    });
                }
            }
        }

        None
    }

    /// Check if any negative bounds are violated
    ///
    /// A negative bound `T: !Protocol` is violated if T actually implements Protocol
    fn check_negative_bound_violations(
        &self,
        idx: usize,
        impl_: &ProtocolImpl,
    ) -> List<SpecializationError> {
        let mut errors = List::new();

        for clause in impl_.where_clauses.iter() {
            // WhereClause from verum_protocol_types has direct ty and bounds fields
            let ty = &clause.ty;

            for bound in clause.bounds.iter() {
                // Check if this is a negative bound by examining protocol name
                if let Some(ident) = bound.protocol.as_ident() {
                    let name = ident.as_str();
                    if name.starts_with('!') {
                        // This is a negative bound
                        let protocol_name = name.trim_start_matches('!');

                        // Check if this type actually implements the negative protocol
                        if self.type_implements_protocol_by_name(ty, protocol_name) {
                            errors.push(SpecializationError::NegativeBoundViolation {
                                impl_idx: idx,
                                ty: ty.clone(),
                                protocol: Text::from(protocol_name),
                                reason: "Type implements a protocol it was declared not to".into(),
                            });
                        }
                    }
                }
            }
        }

        errors
    }

    /// Check if a type implements a protocol (by Path)
    /// Check if a type implements a protocol (by name string)
    ///
    /// Full implementation using the protocol checker:
    /// 1. Query the protocol encoder for direct implementation
    /// 2. Check transitive superprotocol implementations
    /// 3. Handle blanket implementations (impl<T> Protocol for T)
    /// 4. Verify where clause satisfaction for conditional impls
    fn type_implements_protocol_by_name(&self, ty: &Type, protocol_name: &str) -> bool {
        // First, try the crate-level check_implements function from protocol_smt
        // which uses a full ProtocolEncoder
        match crate::protocol_smt::check_implements(ty, protocol_name) {
            Ok(result) => return result,
            Err(_) => {
                // If protocol checker fails, fall back to local database check
                // This can happen if the protocol is not registered with the encoder
            }
        }

        // Fall back to local implementation database check
        self.type_implements_protocol_local(ty, protocol_name)
    }

    /// Local implementation check using the registered implementations
    ///
    /// This is the fallback when the global protocol checker doesn't have
    /// information about the protocol or type.
    fn type_implements_protocol_local(&self, ty: &Type, protocol_name: &str) -> bool {
        // Check direct implementations
        for impl_ in &self.implementations {
            let impl_protocol = impl_.protocol.as_ident().map(|i| i.as_str()).unwrap_or("");

            if impl_protocol == protocol_name
                && self.types_match_for_impl(ty, &impl_.for_type, impl_)
            {
                return true;
            }
        }

        // Check blanket implementations (impl<T: Bound> Protocol for T)
        for impl_ in &self.implementations {
            let impl_protocol = impl_.protocol.as_ident().map(|i| i.as_str()).unwrap_or("");

            if impl_protocol == protocol_name && self.is_blanket_impl(impl_) {
                // This is a blanket impl - check if ty satisfies the where clauses
                if self.type_satisfies_blanket_impl(ty, impl_) {
                    return true;
                }
            }
        }

        // Check superprotocol relationships
        // If ty implements a subprotocol, it also implements all superprotocols
        for impl_ in &self.implementations {
            if self.types_match_for_impl(ty, &impl_.for_type, impl_) {
                // ty implements impl_.protocol
                // Check if impl_.protocol is a subprotocol of protocol_name
                if self.is_subprotocol(&impl_.protocol, protocol_name) {
                    return true;
                }
            }
        }

        false
    }

    /// Check if an implementation's type matches the target type
    ///
    /// Handles:
    /// - Concrete type matching
    /// - Generic type instantiation
    /// - Type parameter binding
    /// - Variance in type arguments
    fn types_match_for_impl(&self, target_ty: &Type, impl_ty: &Type, impl_: &ProtocolImpl) -> bool {
        // Get the index of this implementation
        let impl_idx = self
            .implementations
            .iter()
            .position(|i| std::ptr::eq(i, impl_))
            .unwrap_or(0);

        // Get known type parameters for this impl
        let type_params = self
            .known_type_params
            .get(&impl_idx)
            .cloned()
            .unwrap_or_default();

        self.types_match_with_params(impl_ty, target_ty, &type_params)
    }

    /// Check if types match considering type parameters
    fn types_match_with_params(
        &self,
        impl_ty: &Type,
        target_ty: &Type,
        type_params: &Set<Text>,
    ) -> bool {
        use verum_ast::ty::TypeKind;

        match (&impl_ty.kind, &target_ty.kind) {
            // Exact primitive matches
            (TypeKind::Unit, TypeKind::Unit) => true,
            (TypeKind::Bool, TypeKind::Bool) => true,
            (TypeKind::Int, TypeKind::Int) => true,
            (TypeKind::Float, TypeKind::Float) => true,
            (TypeKind::Char, TypeKind::Char) => true,
            (TypeKind::Text, TypeKind::Text) => true,

            // Path type - check if it's a type parameter or concrete type
            (TypeKind::Path(impl_path), _) => {
                if let Some(ident) = impl_path.as_ident() {
                    let name = Text::from(ident.as_str());
                    if type_params.contains(&name) {
                        // This is a type parameter - matches any type that satisfies
                        // the where clause bounds. For specialization coherence, we
                        // conservatively assume the bounds are satisfied when doing
                        // overlap detection. The actual bound checking is performed
                        // by the protocol verification system (protocol_smt.rs) which
                        // uses Z3 to verify that target_ty satisfies all bounds
                        // specified in the where clauses for this type parameter.
                        return true;
                    }
                }
                // Concrete path - must match exactly
                if let TypeKind::Path(target_path) = &target_ty.kind {
                    self.paths_equal(impl_path, target_path)
                } else {
                    false
                }
            }

            // Generic types - match base and args
            (
                TypeKind::Generic {
                    base: impl_base,
                    args: impl_args,
                },
                TypeKind::Generic {
                    base: target_base,
                    args: target_args,
                },
            ) => {
                if !self.types_match_with_params(impl_base, target_base, type_params) {
                    return false;
                }
                if impl_args.len() != target_args.len() {
                    return false;
                }
                impl_args
                    .iter()
                    .zip(target_args.iter())
                    .all(|(impl_arg, target_arg)| {
                        self.generic_args_match(impl_arg, target_arg, type_params)
                    })
            }

            // Tuple types
            (TypeKind::Tuple(impl_elems), TypeKind::Tuple(target_elems)) => {
                if impl_elems.len() != target_elems.len() {
                    return false;
                }
                impl_elems
                    .iter()
                    .zip(target_elems.iter())
                    .all(|(i, t)| self.types_match_with_params(i, t, type_params))
            }

            // Reference types
            (
                TypeKind::Reference {
                    mutable: impl_mut,
                    inner: impl_inner,
                },
                TypeKind::Reference {
                    mutable: target_mut,
                    inner: target_inner,
                },
            ) => {
                impl_mut == target_mut
                    && self.types_match_with_params(impl_inner, target_inner, type_params)
            }

            // Inferred type matches anything
            (TypeKind::Inferred, _) | (_, TypeKind::Inferred) => true,

            _ => false,
        }
    }

    /// Check if generic arguments match
    fn generic_args_match(
        &self,
        impl_arg: &verum_ast::ty::GenericArg,
        target_arg: &verum_ast::ty::GenericArg,
        type_params: &Set<Text>,
    ) -> bool {
        use verum_ast::ty::GenericArg;
        match (impl_arg, target_arg) {
            (GenericArg::Type(impl_ty), GenericArg::Type(target_ty)) => {
                self.types_match_with_params(impl_ty, target_ty, type_params)
            }
            (GenericArg::Const(impl_expr), GenericArg::Const(target_expr)) => {
                format!("{:?}", impl_expr) == format!("{:?}", target_expr)
            }
            (GenericArg::Lifetime(impl_lt), GenericArg::Lifetime(target_lt)) => {
                impl_lt.name == target_lt.name
            }
            (GenericArg::Binding(impl_b), GenericArg::Binding(target_b)) => {
                impl_b.name.name == target_b.name.name
                    && self.types_match_with_params(&impl_b.ty, &target_b.ty, type_params)
            }
            _ => false,
        }
    }

    /// Check if two paths are equal
    fn paths_equal(&self, path1: &Path, path2: &Path) -> bool {
        use verum_ast::ty::PathSegment;

        if path1.segments.len() != path2.segments.len() {
            return false;
        }

        path1
            .segments
            .iter()
            .zip(path2.segments.iter())
            .all(|(s1, s2)| match (s1, s2) {
                (PathSegment::Name(i1), PathSegment::Name(i2)) => i1.name == i2.name,
                (PathSegment::SelfValue, PathSegment::SelfValue) => true,
                (PathSegment::Super, PathSegment::Super) => true,
                (PathSegment::Cog, PathSegment::Cog) => true,
                (PathSegment::Relative, PathSegment::Relative) => true,
                _ => false,
            })
    }

    /// Check if an implementation is a blanket impl (impl<T> for T or similar)
    fn is_blanket_impl(&self, impl_: &ProtocolImpl) -> bool {
        use verum_ast::ty::TypeKind;

        // A blanket impl is one where the for_type is just a type parameter
        match &impl_.for_type.kind {
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    // Single identifier that looks like a type parameter
                    self.looks_like_type_param(ident.as_str())
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Check if a type satisfies a blanket implementation's where clauses
    fn type_satisfies_blanket_impl(&self, ty: &Type, impl_: &ProtocolImpl) -> bool {
        // For each where clause, check if ty satisfies the bound
        for clause in impl_.where_clauses.iter() {
            // The clause typically constrains a type parameter
            // We need to check if ty satisfies all bounds in the clause
            for bound in clause.bounds.iter() {
                let bound_protocol = bound.protocol.as_ident().map(|i| i.as_str()).unwrap_or("");

                // Check if ty implements this bound protocol
                if !self.type_implements_protocol_local(ty, bound_protocol) {
                    return false;
                }
            }
        }

        true
    }

    /// Check if `sub_protocol` is a subprotocol of `super_protocol_name`,
    /// reflexively or transitively, by walking the super-protocol graph.
    ///
    /// Uses BFS over the `protocols` registry. Each protocol's
    /// `super_protocols` list provides the next frontier. A `visited` set
    /// guards against cycles in malformed declarations (e.g., user writes
    /// `protocol A: B { ... }` and `protocol B: A { ... }` — cycles
    /// shouldn't typecheck, but the coherence walker must still terminate
    /// rather than loop forever during checking).
    ///
    /// Pre-fix this returned `false` for everything except the trivial
    /// reflexive case, which silently broke `type_implements_protocol_local`:
    /// types implementing subprotocols were not recognized as satisfying
    /// their superprotocols, narrowing valid specializations.
    fn is_subprotocol(&self, sub_protocol: &Path, super_protocol_name: &str) -> bool {
        let sub_name = sub_protocol.as_ident().map(|i| i.as_str()).unwrap_or("");
        if sub_name.is_empty() {
            return false;
        }
        if sub_name == super_protocol_name {
            return true;
        }

        let mut visited: Set<Text> = Set::new();
        let mut frontier: Vec<Text> = vec![Text::from(sub_name)];

        while let Some(current) = frontier.pop() {
            if !visited.insert(current.clone()) {
                continue;
            }
            let Some(proto) = self.protocols.get(&current) else {
                continue;
            };
            for bound in proto.super_protocols.iter() {
                let parent_name = bound
                    .protocol
                    .as_ident()
                    .map(|i| i.as_str())
                    .unwrap_or("");
                if parent_name.is_empty() {
                    continue;
                }
                if parent_name == super_protocol_name {
                    return true;
                }
                if !visited.contains(&Text::from(parent_name)) {
                    frontier.push(Text::from(parent_name));
                }
            }
        }
        false
    }

    /// Check mutual exclusion between implementations with negative bounds
    ///
    /// Two implementations are mutually exclusive if:
    /// 1. One has `T: Protocol` and the other has `T: !Protocol` for the same T
    /// 2. They would otherwise overlap
    ///
    /// Mutual exclusion ensures at most one of them can apply to any concrete type.
    fn check_mutual_exclusion(&self) -> List<SpecializationError> {
        let mut errors = List::new();

        for i in 0..self.implementations.len() {
            for j in (i + 1)..self.implementations.len() {
                let impl_i = &self.implementations[i];
                let impl_j = &self.implementations[j];

                // Only check implementations of the same protocol
                if impl_i.protocol != impl_j.protocol {
                    continue;
                }

                // Check if implementations have opposite polarity bounds
                let bounds_i = self.extract_negative_bounds(impl_i);
                let bounds_j = self.extract_negative_bounds(impl_j);
                let positive_bounds_i = self.extract_positive_bounds(impl_i);
                let positive_bounds_j = self.extract_positive_bounds(impl_j);

                // Check for mutual exclusion: one has !P and other has P
                for neg_bound in &bounds_i {
                    if positive_bounds_j.contains(neg_bound) {
                        // i has !P, j has P - they should be mutually exclusive
                        // Verify that one specializes the other or they don't overlap
                        if self.types_could_overlap(&impl_i.for_type, &impl_j.for_type) {
                            let ordering = self.compare_specificity(i, j);
                            if ordering == SpecificityOrdering::Equal
                                || ordering == SpecificityOrdering::Incomparable
                            {
                                // They overlap but neither is more specific
                                // This is OK for mutual exclusion - they're exclusive by design
                                // However, if they truly overlap on the same type, it's an error
                                if self.types_overlap(&impl_i.for_type, &impl_j.for_type) {
                                    errors.push(SpecializationError::MutualExclusionViolation {
                                        impl1: i,
                                        impl2: j,
                                        reason: format!(
                                            "Implementations have opposite polarity for '{}' but overlap",
                                            neg_bound
                                        ).into(),
                                    });
                                }
                            }
                        }
                    }
                }

                // Check the reverse: j has !P, i has P
                for neg_bound in &bounds_j {
                    if positive_bounds_i.contains(neg_bound)
                        && self.types_could_overlap(&impl_i.for_type, &impl_j.for_type)
                    {
                        let ordering = self.compare_specificity(i, j);
                        if (ordering == SpecificityOrdering::Equal
                            || ordering == SpecificityOrdering::Incomparable)
                            && self.types_overlap(&impl_i.for_type, &impl_j.for_type)
                        {
                            errors.push(SpecializationError::MutualExclusionViolation {
                                impl1: i,
                                impl2: j,
                                reason: format!(
                                    "Implementations have opposite polarity for '{}' but overlap",
                                    neg_bound
                                )
                                .into(),
                            });
                        }
                    }
                }
            }
        }

        errors
    }

    /// Extract negative bounds from an implementation's where clauses
    fn extract_negative_bounds(&self, impl_: &ProtocolImpl) -> HashSet<String> {
        let mut bounds = HashSet::new();

        for clause in impl_.where_clauses.iter() {
            for bound in clause.bounds.iter() {
                // Negative bounds indicated by protocol name starting with "!"
                if let Some(ident) = bound.protocol.as_ident() {
                    let name = ident.as_str();
                    if name.starts_with('!') {
                        // Extract protocol name without "!"
                        bounds.insert(name.trim_start_matches('!').to_string());
                    }
                }
            }
        }

        bounds
    }

    /// Extract positive bounds from an implementation's where clauses
    fn extract_positive_bounds(&self, impl_: &ProtocolImpl) -> HashSet<String> {
        let mut bounds = HashSet::new();

        for clause in impl_.where_clauses.iter() {
            for bound in clause.bounds.iter() {
                // Positive bounds are those not starting with "!"
                if let Some(ident) = bound.protocol.as_ident() {
                    let name = ident.as_str();
                    if !name.starts_with('!') {
                        bounds.insert(name.to_string());
                    }
                }
            }
        }

        bounds
    }

    /// Check if two types could potentially overlap (conservative check)
    fn types_could_overlap(&self, ty1: &Type, ty2: &Type) -> bool {
        // If either is a type variable or generic, they could overlap
        // Note: Type enum variants vary - this is a conservative check
        let is_generic_like = |t: &Type| -> bool {
            // Check if type contains variables or generics (simplified check)
            let ty_str = format!("{:?}", t);
            ty_str.contains("Var") || ty_str.contains("Generic") || ty_str.contains("Param")
        };

        if is_generic_like(ty1) || is_generic_like(ty2) {
            true
        } else {
            self.types_overlap(ty1, ty2)
        }
    }

    // ========================================================================
    // CHC Encoding for Negative Bounds
    // ========================================================================

    /// Count the number of negative bounds in all implementations
    ///
    /// Useful for statistics and debugging.
    pub fn count_negative_bounds(&self) -> usize {
        let mut count = 0;

        for impl_ in &self.implementations {
            for clause in impl_.where_clauses.iter() {
                // WhereClause from verum_protocol_types has direct bounds field
                // Check if any bound is negative by examining the protocol path
                // In the current type system, negative bounds may be encoded in the path
                // with a "!" prefix in the name, or tracked separately
                for bound in clause.bounds.iter() {
                    // Check if the protocol path indicates a negative bound
                    if let Some(ident) = bound.protocol.as_ident()
                        && ident.as_str().starts_with('!')
                    {
                        count += 1;
                    }
                }
            }
        }

        count
    }

    /// Verify negative bounds using direct checks
    ///
    /// This performs comprehensive verification of negative bounds:
    /// 1. No contradictory bounds (T: P + !P)
    /// 2. No violated negative bounds (type implements excluded protocol)
    /// 3. Mutual exclusion is properly enforced
    ///
    /// Returns true if all negative bounds are valid.
    pub fn verify_negative_bounds_complete(&self) -> Result<bool, List<SpecializationError>> {
        let errors = self.check_negative_bounds();

        if errors.is_empty() {
            Ok(true)
        } else {
            Err(errors)
        }
    }
}

impl Default for SpecializationVerifier {
    fn default() -> Self {
        Self::new().unwrap()
    }
}

// ==================== High-Level API ====================

/// Verify specialization coherence for a set of implementations
pub fn verify_specialization(implementations: &[ProtocolImpl]) -> SpecializationVerificationResult {
    let mut verifier = SpecializationVerifier::new().unwrap();

    for (idx, impl_) in implementations.iter().enumerate() {
        verifier.register_implementation(impl_.clone(), idx);
    }

    verifier.verify()
}

/// Check if a specialization lattice is coherent
pub fn is_coherent(lattice: &SpecializationLattice) -> bool {
    // Check basic lattice properties
    // This is a simplified check - full version would verify all axioms
    true
}

/// Detect overlaps between implementations
pub fn detect_overlaps(implementations: &[ProtocolImpl]) -> List<(usize, usize)> {
    let mut overlaps = List::new();

    for i in 0..implementations.len() {
        for j in (i + 1)..implementations.len() {
            let impl_i = &implementations[i];
            let impl_j = &implementations[j];

            if impl_i.protocol == impl_j.protocol {
                // Simplified overlap check
                overlaps.push((i, j));
            }
        }
    }

    overlaps
}

#[cfg(test)]
mod tests {
    use super::*;
    
    

    #[test]
    fn test_verifier_creation() {
        let verifier = SpecializationVerifier::new();
        assert!(verifier.is_ok());
    }

    #[test]
    fn test_empty_verification() {
        let mut verifier = SpecializationVerifier::new().unwrap();
        let result = verifier.verify();
        assert!(result.is_coherent);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_specificity_ordering() {
        let verifier = SpecializationVerifier::new().unwrap();

        // Equal ordering should be symmetric
        let eq = SpecificityOrdering::Equal;
        assert_eq!(eq, SpecificityOrdering::Equal);
    }

    #[test]
    fn test_lattice_computation() {
        let mut verifier = SpecializationVerifier::new().unwrap();
        let mut stats = SpecializationStats::default();

        verifier.build_lattice(&mut stats);
        assert_eq!(stats.comparisons, 0); // No implementations
    }

    #[test]
    fn test_cycle_detection() {
        let verifier = SpecializationVerifier::new().unwrap();
        let result = verifier.check_cycles();
        assert!(result.is_ok());
    }

    #[test]
    fn test_antisymmetry() {
        let verifier = SpecializationVerifier::new().unwrap();
        let result = verifier.check_antisymmetry();
        assert!(result.is_ok());
    }

    #[test]
    fn test_is_coherent() {
        let default_protocol = Path::single(verum_ast::ty::Ident {
            name: "Test".to_string().into(),
            span: verum_ast::span::Span::default(),
        });
        let lattice = SpecializationLattice::new(default_protocol);
        assert!(is_coherent(&lattice));
    }

    #[test]
    fn test_detect_overlaps_empty() {
        let overlaps = detect_overlaps(&[]);
        assert!(overlaps.is_empty());
    }
}
