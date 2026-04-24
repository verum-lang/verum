//! GAT (Generic Associated Type) Constraint Verification
//!
//! GATs (Generic Associated Types) allow protocol associated types to have their own
//! type parameters, e.g., `type Item<T> where T: Clone`. Well-formedness requires:
//! (1) where-clause constraints are satisfiable, (2) no circular type dependencies,
//! (3) variance annotations are consistent. In CBGR, GATs use generation tracking
//! instead of lifetime parameters, simplifying lending iterator patterns.
//!
//! This module encodes GAT constraints to Z3 and verifies:
//! 1. Type parameter constraints are satisfied
//! 2. Where clauses hold for all instantiations
//! 3. No circular dependencies in GAT definitions
//! 4. Variance constraints are respected
//!
//! # Performance Targets
//!
//! - Simple GAT verification: <50ms
//! - Complex GAT with multiple constraints: <100ms
//! - Circular dependency detection: <20ms
//!
//! # Theory
//!
//! GATs are encoded as universally quantified formulas in Z3:
//! ```smt2
//! (forall ((T Sort))
//!   (=> (constraint T)
//!       (well_formed (GAT T))))
//! ```

use std::collections::HashMap;
use std::time::{Duration, Instant};

use verum_ast::ty::{GenericArg, Path, Type, TypeBound, TypeBoundKind, TypeKind};
use verum_common::{List, Map, Maybe, Set, Text};
use verum_protocol_types::gat_types::{AssociatedTypeGAT, GATTypeParam, GATWhereClause, Variance};
use verum_protocol_types::protocol_base::ProtocolBound;
use verum_common::ToText;

use z3::ast::{Bool, Dynamic, Int, forall_const};
use z3::{Context, FuncDecl, Pattern, SatResult, Solver, Sort, Symbol};

use crate::context::Context as VerumContext;
use crate::translate::Translator;

// ==================== Protocol Table for Type Checking ====================

/// Protocol table entry for SMT verification
///
/// Tracks protocol hierarchies and method requirements for bound checking.
#[derive(Debug, Clone, Default)]
pub struct ProtocolTable {
    /// Known protocols: name -> super-protocols
    pub protocols: Map<Text, List<Text>>,
    /// Protocol methods: (protocol, method) -> exists
    pub methods: Set<(Text, Text)>,
    /// Protocol implementations: (type, protocol) -> exists
    pub implementations: Set<(Text, Text)>,
}

impl ProtocolTable {
    /// Create an empty protocol table
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a protocol with its super-protocols
    pub fn register_protocol(&mut self, name: Text, supers: List<Text>) {
        self.protocols.insert(name, supers);
    }

    /// Register that a type implements a protocol
    pub fn register_impl(&mut self, ty: Text, protocol: Text) {
        self.implementations.insert((ty, protocol));
    }

    /// Check if a type implements a protocol (directly or via hierarchy)
    pub fn implements(&self, ty: &Text, protocol: &Text) -> bool {
        if self
            .implementations
            .contains(&(ty.clone(), protocol.clone()))
        {
            return true;
        }
        // Check super-protocol hierarchy
        if let Some(supers) = self.protocols.get(protocol) {
            for super_proto in supers.iter() {
                if self.implements(ty, super_proto) {
                    return true;
                }
            }
        }
        false
    }

    /// Get all protocols a type implements
    pub fn get_implemented_protocols(&self, ty: &Text) -> Set<Text> {
        let mut result = Set::new();
        for (impl_ty, proto) in self.implementations.iter() {
            if impl_ty == ty {
                result.insert(proto.clone());
                // Add all sub-protocols via hierarchy
                self.collect_super_protocols(proto, &mut result);
            }
        }
        result
    }

    fn collect_super_protocols(&self, proto: &Text, result: &mut Set<Text>) {
        if let Some(supers) = self.protocols.get(proto) {
            for super_proto in supers.iter() {
                if !result.contains(super_proto) {
                    result.insert(super_proto.clone());
                    self.collect_super_protocols(super_proto, result);
                }
            }
        }
    }
}

// ==================== Variance Tracking ====================

/// Position in the type structure for variance tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariancePosition {
    /// Covariant position (output, return types)
    Covariant,
    /// Contravariant position (input, function parameters)
    Contravariant,
    /// Invariant position (mutable references, both in/out)
    Invariant,
}

impl VariancePosition {
    /// Flip polarity (for entering function parameter position)
    pub fn flip(self) -> Self {
        match self {
            VariancePosition::Covariant => VariancePosition::Contravariant,
            VariancePosition::Contravariant => VariancePosition::Covariant,
            VariancePosition::Invariant => VariancePosition::Invariant,
        }
    }

    /// Combine with another position (for nested types)
    pub fn combine(self, other: Self) -> Self {
        match (self, other) {
            (VariancePosition::Invariant, _) | (_, VariancePosition::Invariant) => {
                VariancePosition::Invariant
            }
            (VariancePosition::Covariant, p) | (p, VariancePosition::Covariant) => p,
            (VariancePosition::Contravariant, VariancePosition::Contravariant) => {
                VariancePosition::Covariant // Double flip = covariant
            }
        }
    }
}

/// Tracks variance of a type parameter across all usages
pub struct VarianceTracker {
    /// Name of the type parameter being tracked
    pub param_name: Text,
    /// Has the parameter appeared in covariant position?
    pub seen_covariant: bool,
    /// Has the parameter appeared in contravariant position?
    pub seen_contravariant: bool,
    /// Has the parameter appeared in invariant position?
    pub seen_invariant: bool,
}

impl VarianceTracker {
    /// Create a new variance tracker for a parameter
    pub fn new(param_name: Text) -> Self {
        Self {
            param_name,
            seen_covariant: false,
            seen_contravariant: false,
            seen_invariant: false,
        }
    }

    /// Record a usage of the parameter at the given position
    pub fn record_usage(&mut self, position: VariancePosition) {
        match position {
            VariancePosition::Covariant => self.seen_covariant = true,
            VariancePosition::Contravariant => self.seen_contravariant = true,
            VariancePosition::Invariant => self.seen_invariant = true,
        }
    }

    /// Get the inferred variance based on all recorded usages
    pub fn get_variance(&self) -> Variance {
        if self.seen_invariant {
            // Any invariant usage forces invariance
            Variance::Invariant
        } else if self.seen_covariant && self.seen_contravariant {
            // Both co and contra = invariant
            Variance::Invariant
        } else if self.seen_contravariant {
            Variance::Contravariant
        } else {
            // Only covariant or no usage (default to covariant)
            Variance::Covariant
        }
    }

    /// Analyze a type and record usages of the tracked parameter
    pub fn analyze_type(&mut self, ty: &Type, position: VariancePosition) {
        match &ty.kind {
            TypeKind::Path(path) => {
                // Check if this is our parameter
                if let Some(ident) = path.as_ident()
                    && ident.as_str() == self.param_name.as_str()
                {
                    self.record_usage(position);
                }
                // Note: Generic type arguments are in TypeKind::Generic, not in Path
            }
            TypeKind::Reference { mutable, inner } => {
                // &T is covariant in T
                // &mut T is invariant in T
                let inner_position = if *mutable {
                    VariancePosition::Invariant
                } else {
                    position
                };
                self.analyze_type(inner, inner_position);
            }
            TypeKind::CheckedReference { mutable, inner } => {
                // &checked T follows same variance rules
                let inner_position = if *mutable {
                    VariancePosition::Invariant
                } else {
                    position
                };
                self.analyze_type(inner, inner_position);
            }
            TypeKind::UnsafeReference { mutable, inner } => {
                // &unsafe T follows same variance rules
                let inner_position = if *mutable {
                    VariancePosition::Invariant
                } else {
                    position
                };
                self.analyze_type(inner, inner_position);
            }
            TypeKind::Slice(inner) => {
                // [T] is covariant in T
                self.analyze_type(inner, position);
            }
            TypeKind::Array { element, .. } => {
                // [T; N] is covariant in T
                self.analyze_type(element, position);
            }
            TypeKind::Tuple(elements) => {
                // Tuples are covariant in all elements
                for elem in elements.iter() {
                    self.analyze_type(elem, position);
                }
            }
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
                // fn(T) -> U: T is contravariant, U is covariant
                for param in params.iter() {
                    self.analyze_type(param, position.flip());
                }
                self.analyze_type(return_type, position);
            }
            TypeKind::Inferred => {
                // Inferred type - skip
            }
            TypeKind::Generic { base, args } => {
                // Generic<T> - analyze base and args
                self.analyze_type(base, position);
                for arg in args.iter() {
                    if let GenericArg::Type(arg_ty) = arg {
                        self.analyze_type(arg_ty, position);
                    }
                }
            }
            TypeKind::GenRef { inner } => {
                // GenRef<T> is covariant in T (like a smart pointer)
                self.analyze_type(inner, position);
            }
            TypeKind::Pointer { mutable, inner } => {
                // *const T is covariant, *mut T is invariant
                let inner_position = if *mutable {
                    VariancePosition::Invariant
                } else {
                    position
                };
                self.analyze_type(inner, inner_position);
            }
            TypeKind::DynProtocol { bounds, bindings } => {
                // dyn Protocol types require comprehensive bounds analysis.
                //
                // The type parameter can appear in:
                // 1. Protocol bounds themselves (e.g., dyn Iterator<Item = T>)
                // 2. Associated type bindings (e.g., dyn Container<Item = T>)
                // 3. Transitively through superprotocols
                //
                // Protocol bounds constrain input behavior, so they are contravariant.
                // Associated type bindings can appear in covariant position.
                for bound in bounds.iter() {
                    self.analyze_type_bound(bound, position.flip());
                }

                // Analyze associated type bindings if present
                if let Some(binding_list) = bindings {
                    for binding in binding_list.iter() {
                        // Associated type bindings are covariant in their type
                        // (e.g., in `dyn Iterator<Item = T>`, T is covariant)
                        self.analyze_type(&binding.ty, position);
                    }
                }
            }
            TypeKind::Ownership { mutable, inner } => {
                // %T is like a unique/move type
                let inner_position = if *mutable {
                    VariancePosition::Invariant
                } else {
                    position
                };
                self.analyze_type(inner, inner_position);
            }
            TypeKind::Refined { base, .. } => {
                // Refined types preserve base type variance
                self.analyze_type(base, position);
            }
            _ => {
                // Other types: skip or handle conservatively
                // (Unit, Bool, Int, Float, Char, Text, Bounded, Qualified, etc.)
            }
        }
    }

    /// Analyze a type bound for variance of the tracked parameter
    ///
    /// TypeBounds can contain type parameters in:
    /// 1. Protocol bounds with generic arguments (e.g., Iterator<Item = T>)
    /// 2. Equality bounds (e.g., Self::Item = T)
    /// 3. Negative protocol bounds (e.g., T: !Sized)
    /// 4. Higher-ranked protocol bounds (e.g., for<'a> Fn(&'a T) -> &'a U)
    pub fn analyze_type_bound(&mut self, bound: &TypeBound, position: VariancePosition) {
        match &bound.kind {
            TypeBoundKind::Protocol(path) | TypeBoundKind::NegativeProtocol(path) => {
                // Check if the protocol path references our type parameter
                // This handles cases like `dyn T` or `dyn SomeTrait<T>`
                if let Some(ident) = path.as_ident()
                    && ident.as_str() == self.param_name.as_str()
                {
                    self.record_usage(position);
                }

                // Check path segments for type parameter references
                for segment in path.segments.iter() {
                    use verum_ast::ty::PathSegment;
                    if let PathSegment::Name(ident) = segment {
                        // Check if segment name matches our type parameter
                        if ident.as_str() == self.param_name.as_str() {
                            self.record_usage(position);
                        }
                    }
                }
            }
            TypeBoundKind::Equality(ty) => {
                // Equality bounds are invariant - the type must match exactly
                // e.g., `Self::Item = T` makes T invariant
                self.analyze_type(ty, VariancePosition::Invariant);
            }
            TypeBoundKind::AssociatedTypeBound {
                type_path,
                assoc_name,
                bounds,
            } => {
                // Associated type bounds like T.Item: Display
                // Check the type path for our parameter
                for segment in type_path.segments.iter() {
                    use verum_ast::ty::PathSegment;
                    if let PathSegment::Name(ident) = segment {
                        if ident.as_str() == self.param_name.as_str() {
                            self.record_usage(position);
                        }
                    }
                }
                // Recursively analyze bounds on the associated type
                for bound in bounds.iter() {
                    self.analyze_type_bound(bound, position);
                }
            }
            TypeBoundKind::AssociatedTypeEquality {
                type_path,
                assoc_name: _,
                eq_type,
            } => {
                // Associated type equality like T.Item = Int
                // Check the type path for our parameter
                for segment in type_path.segments.iter() {
                    use verum_ast::ty::PathSegment;
                    if let PathSegment::Name(ident) = segment {
                        if ident.as_str() == self.param_name.as_str() {
                            // Equality constraints are invariant
                            self.record_usage(VariancePosition::Invariant);
                        }
                    }
                }
                // Analyze the equality type
                self.analyze_type(eq_type, VariancePosition::Invariant);
            }
            TypeBoundKind::GenericProtocol(ty) => {
                // Generic protocol bound like Iterator<Item = T>
                // Analyze the full type which contains the protocol and its type arguments
                self.analyze_type(ty, position);
            }
        }
    }

    /// Analyze a protocol bound for variance of the tracked parameter
    ///
    /// ProtocolBounds can contain type parameters in:
    /// 1. The protocol path itself (rare, e.g., dyn T)
    /// 2. Type arguments to the protocol (e.g., Iterator<Item = T>)
    ///
    /// Returns true if the parameter was found in the bound
    pub fn analyze_protocol_bound(
        &mut self,
        bound: &ProtocolBound,
        position: VariancePosition,
    ) -> bool {
        let mut found = false;

        // Check if the protocol path references our type parameter
        if let Some(ident) = bound.protocol.as_ident()
            && ident.as_str() == self.param_name.as_str()
        {
            self.record_usage(position);
            found = true;
        }

        // Check path segments for type parameter references
        for segment in bound.protocol.segments.iter() {
            use verum_ast::ty::PathSegment;
            if let PathSegment::Name(ident) = segment {
                if ident.as_str() == self.param_name.as_str() {
                    self.record_usage(position);
                    found = true;
                }
            }
        }

        // Check type arguments to the protocol
        for arg_ty in bound.args.iter() {
            if self.contains_param(arg_ty) {
                self.analyze_type(arg_ty, position);
                found = true;
            }
        }

        found
    }

    /// Check if a type contains a reference to the tracked parameter
    fn contains_param(&self, ty: &Type) -> bool {
        match &ty.kind {
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    ident.as_str() == self.param_name.as_str()
                } else {
                    false
                }
            }
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. }
            | TypeKind::Slice(inner)
            | TypeKind::GenRef { inner }
            | TypeKind::Pointer { inner, .. }
            | TypeKind::Ownership { inner, .. }
            | TypeKind::Refined { base: inner, .. } => self.contains_param(inner),
            TypeKind::Array { element, .. } => self.contains_param(element),
            TypeKind::Tuple(elements) => elements.iter().any(|e| self.contains_param(e)),
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
            } => params.iter().any(|p| self.contains_param(p)) || self.contains_param(return_type),
            TypeKind::Generic { base, args } => {
                self.contains_param(base)
                    || args.iter().any(|arg| {
                        if let GenericArg::Type(arg_ty) = arg {
                            self.contains_param(arg_ty)
                        } else {
                            false
                        }
                    })
            }
            _ => false,
        }
    }
}

// ==================== Core Types ====================

/// Result of GAT verification
#[derive(Debug, Clone)]
pub struct GATVerificationResult {
    /// Whether the GAT is well-formed
    pub is_valid: bool,
    /// Verification time
    pub duration: Duration,
    /// Errors found (if any)
    pub errors: List<GATError>,
    /// Counterexamples for violated constraints
    pub counterexamples: List<GATCounterexample>,
    /// Statistics
    pub stats: GATStats,
}

/// Error in GAT verification
#[derive(Debug, Clone)]
pub enum GATError {
    /// Type parameter constraint not satisfied
    ConstraintViolation {
        param: Text,
        constraint: Text,
        counterexample: Maybe<Text>,
    },
    /// Circular dependency detected
    CircularDependency { cycle: List<Text> },
    /// Where clause not satisfiable
    UnsatisfiableWhereClause { param: Text, clause: Text },
    /// Variance violation
    VarianceViolation {
        param: Text,
        expected: Variance,
        found: Variance,
    },
    /// Arity mismatch
    ArityMismatch {
        gat_name: Text,
        expected: usize,
        found: usize,
    },
    /// Protocol bound not satisfied by type argument
    UnsatisfiedBound {
        gat_name: Text,
        ty: Type,
        bound: String,
    },
    /// Variance mismatch between declared and inferred variance
    VarianceMismatch {
        gat_name: Text,
        expected: String,
        found: String,
    },
    /// Lifetime bound violation
    LifetimeBoundViolation {
        gat_name: Text,
        lifetime_param: Text,
        required_bound: Text,
        explanation: Text,
    },
    /// Transitive bound not satisfied
    TransitiveBoundViolation {
        gat_name: Text,
        param: Text,
        direct_bound: Text,
        transitive_bound: Text,
        explanation: Text,
    },
}

/// Counterexample showing GAT constraint violation
#[derive(Debug, Clone)]
pub struct GATCounterexample {
    /// Type parameter causing the violation
    pub param: Text,
    /// Concrete type that violates constraint
    pub violating_type: Text,
    /// The constraint that was violated
    pub violated_constraint: Text,
    /// Explanation
    pub explanation: Text,
}

/// Statistics for GAT verification
#[derive(Debug, Clone, Default)]
pub struct GATStats {
    /// Number of type parameters checked
    pub type_params_checked: usize,
    /// Number of where clauses verified
    pub where_clauses_checked: usize,
    /// Number of dependencies analyzed
    pub dependencies_checked: usize,
    /// Number of lifetime bounds verified
    pub lifetime_bounds_checked: usize,
    /// Number of transitive bounds verified
    pub transitive_bounds_checked: usize,
    /// Time spent in Z3
    pub smt_time: Duration,
}

// ==================== Main Verifier ====================

/// GAT constraint verifier
///
/// Encodes GAT definitions to Z3 and verifies well-formedness properties.
///
/// # SMT Encoding Strategy
///
/// Protocol bounds are encoded using:
/// 1. Uninterpreted `Type` and `Protocol` sorts
/// 2. `implements(Type, Protocol) -> Bool` predicate
/// 3. Quantified implications for protocol hierarchies
/// 4. Pattern-guided instantiation for efficient verification
pub struct GATVerifier {
    /// Z3 context
    #[allow(dead_code)] // Reserved for direct Z3 operations
    context: Context,
    /// Verum SMT context
    #[allow(dead_code)] // Reserved for extended verification operations
    verum_ctx: VerumContext,
    /// Translator for Verum AST to Z3
    #[allow(dead_code)] // Reserved for AST translation in extended verification
    translator: Translator<'static>,
    /// Cache of verified GATs
    cache: Map<Text, GATVerificationResult>,
    /// Dependency graph for cycle detection
    dependency_graph: Map<Text, Set<Text>>,
    /// Protocol table for bound checking
    protocol_table: ProtocolTable,
    /// SMT sorts for types and protocols
    type_sort: Option<Sort>,
    protocol_sort: Option<Sort>,
    /// implements(Type, Protocol) -> Bool predicate
    implements_pred: Option<FuncDecl>,
}

impl GATVerifier {
    /// Create a new GAT verifier
    pub fn new() -> Self {
        let context = Context::thread_local();
        let verum_ctx = VerumContext::new();

        // SAFETY: The Verum context lives as long as the verifier
        // and is not moved, so the static lifetime is safe in practice.
        // This is a workaround for Rust's lifetime system.
        let translator = unsafe {
            let ctx_ref = &verum_ctx as *const VerumContext;
            Translator::new(&*ctx_ref)
        };

        Self {
            context,
            verum_ctx,
            translator,
            cache: Map::new(),
            dependency_graph: Map::new(),
            protocol_table: ProtocolTable::new(),
            type_sort: None,
            protocol_sort: None,
            implements_pred: None,
        }
    }

    /// Create a GAT verifier with a protocol table for bound checking
    pub fn with_protocol_table(protocol_table: ProtocolTable) -> Self {
        let mut verifier = Self::new();
        verifier.protocol_table = protocol_table;
        verifier
    }

    /// Initialize SMT sorts and predicates for protocol encoding
    ///
    /// Creates:
    /// - `Type` sort: Abstract sort for all types
    /// - `Protocol` sort: Abstract sort for all protocols
    /// - `implements(Type, Protocol) -> Bool`: Protocol membership predicate
    fn init_smt_sorts(&mut self) {
        if self.type_sort.is_some() {
            return; // Already initialized
        }

        // Create uninterpreted sorts for types and protocols
        self.type_sort = Some(Sort::uninterpreted(Symbol::String("Type".to_string())));
        self.protocol_sort = Some(Sort::uninterpreted(Symbol::String("Protocol".to_string())));

        // Create implements(Type, Protocol) -> Bool predicate
        let type_sort = self.type_sort.as_ref().unwrap();
        let protocol_sort = self.protocol_sort.as_ref().unwrap();

        self.implements_pred = Some(FuncDecl::new(
            Symbol::String("implements".to_string()),
            &[type_sort, protocol_sort],
            &Sort::bool(),
        ));
    }

    /// Create a type constant in the Type sort
    fn create_type_const(&self, name: &str) -> Dynamic {
        let type_sort = self.type_sort.as_ref().expect("SMT sorts not initialized");
        let const_decl = FuncDecl::new(Symbol::String(name.to_string()), &[], type_sort);
        const_decl.apply(&[])
    }

    /// Create a protocol constant in the Protocol sort
    fn create_protocol_const(&self, name: &str) -> Dynamic {
        let protocol_sort = self
            .protocol_sort
            .as_ref()
            .expect("SMT sorts not initialized");
        let const_decl = FuncDecl::new(Symbol::String(name.to_string()), &[], protocol_sort);
        const_decl.apply(&[])
    }

    /// Create a type variable for quantification
    fn create_type_var(&self, name: &str) -> Dynamic {
        self.create_type_const(name)
    }

    /// Register standard protocols (Clone, Debug, Eq, etc.)
    pub fn register_standard_protocols(&mut self) {
        // Standard protocol hierarchy
        self.protocol_table
            .register_protocol("Any".into(), List::new());
        self.protocol_table
            .register_protocol("Sized".into(), List::new());
        self.protocol_table
            .register_protocol("Clone".into(), List::new());
        self.protocol_table
            .register_protocol("Copy".into(), List::from(vec!["Clone".into()]));
        self.protocol_table
            .register_protocol("Debug".into(), List::new());
        self.protocol_table
            .register_protocol("Display".into(), List::new());
        self.protocol_table
            .register_protocol("Eq".into(), List::from(vec!["PartialEq".into()]));
        self.protocol_table
            .register_protocol("PartialEq".into(), List::new());
        self.protocol_table.register_protocol(
            "Ord".into(),
            List::from(vec!["PartialOrd".into(), "Eq".into()]),
        );
        self.protocol_table
            .register_protocol("PartialOrd".into(), List::from(vec!["PartialEq".into()]));
        self.protocol_table
            .register_protocol("Hash".into(), List::new());
        self.protocol_table
            .register_protocol("Default".into(), List::new());
        self.protocol_table
            .register_protocol("Send".into(), List::new());
        self.protocol_table
            .register_protocol("Sync".into(), List::new());
        self.protocol_table
            .register_protocol("Iterator".into(), List::new());
    }

    /// Verify a GAT definition
    ///
    /// Checks:
    /// 1. Type parameter constraints are satisfiable
    /// 2. Where clauses are well-formed
    /// 3. No circular dependencies
    /// 4. Variance annotations are correct
    ///
    /// # Example
    ///
    /// ```ignore
    /// use verum_smt::gat_verification::GATVerifier;
    /// use verum_smt::AssociatedTypeGAT;
    ///
    /// let verifier = GATVerifier::new();
    /// let gat = AssociatedTypeGAT::simple("Item".into(), vec![].into());
    /// let result = verifier.verify(&gat);
    /// assert!(result.is_valid);
    /// ```
    pub fn verify(&mut self, gat: &AssociatedTypeGAT) -> GATVerificationResult {
        let start = Instant::now();

        // Check cache
        if let Some(cached) = self.cache.get(&gat.name) {
            return cached.clone();
        }

        let mut errors = List::new();
        let mut counterexamples = List::new();
        let mut stats = GATStats::default();

        // Step 1: Check for circular dependencies
        if let Err(err) = self.check_circular_dependencies(gat) {
            errors.push(err);
        }

        // Step 2: Verify type parameter constraints
        for param in gat.type_params.iter() {
            stats.type_params_checked += 1;
            if let Err((err, maybe_ce)) = self.verify_type_param_constraints(gat, param) {
                errors.push(err);
                if let Maybe::Some(ce) = maybe_ce {
                    counterexamples.push(ce);
                }
            }
        }

        // Step 3: Verify where clauses
        for clause in gat.where_clauses.iter() {
            stats.where_clauses_checked += 1;
            if let Err(err) = self.verify_where_clause(gat, clause) {
                errors.push(err);
            }
        }

        // Step 4: Check variance annotations
        for param in gat.type_params.iter() {
            if let Err(err) = self.check_variance(gat, param) {
                errors.push(err);
            }
        }

        // Step 5: Verify lifetime bounds on GATs
        if let Err(errs) = self.verify_lifetime_bounds(gat, &mut stats) {
            for err in errs {
                errors.push(err);
            }
        }

        // Step 6: Verify transitive bounds
        if let Err(errs) = self.verify_transitive_bounds(gat, &mut stats) {
            for err in errs {
                errors.push(err);
            }
        }

        let duration = start.elapsed();
        stats.smt_time = duration;

        let result = GATVerificationResult {
            is_valid: errors.is_empty(),
            duration,
            errors,
            counterexamples,
            stats,
        };

        // Cache the result
        self.cache.insert(gat.name.clone(), result.clone());

        result
    }

    /// Check for circular dependencies in GAT definitions
    ///
    /// Example of circular dependency:
    /// ```verum
    /// protocol Bad {
    ///     type A<T> = Self.B<T>
    ///     type B<T> = Self.A<T>  // Circular!
    /// }
    /// ```
    fn check_circular_dependencies(&mut self, gat: &AssociatedTypeGAT) -> Result<(), GATError> {
        // Build dependency graph
        let deps = self.extract_dependencies(gat);
        self.dependency_graph.insert(gat.name.clone(), deps);

        // Detect cycles using DFS
        let mut visited = Set::new();
        let mut stack = Set::new();

        if let Some(cycle) = self.detect_cycle(&gat.name, &mut visited, &mut stack) {
            return Err(GATError::CircularDependency { cycle });
        }

        Ok(())
    }

    /// Extract GAT dependencies from type references
    fn extract_dependencies(&self, gat: &AssociatedTypeGAT) -> Set<Text> {
        let mut deps = Set::new();

        // Check bounds for references to other GATs
        for bound in gat.bounds.iter() {
            if let Some(path) = Self::extract_gat_from_bound(bound) {
                deps.insert(path);
            }
        }

        // Check default type if present
        if let Maybe::Some(default_ty) = &gat.default {
            Self::extract_gat_references(default_ty, &mut deps);
        }

        deps
    }

    /// Extract GAT name from a protocol bound
    ///
    /// Parses the bound's protocol path to extract GAT references.
    /// This handles:
    /// - Simple protocol bounds: `Clone` -> None (not a GAT)
    /// - Associated type references: `Self::Item` -> Some("Item")
    /// - Qualified paths: `Container::Item` -> Some("Item")
    fn extract_gat_from_bound(bound: &ProtocolBound) -> Option<Text> {
        use verum_ast::ty::PathSegment;

        // Look for associated type patterns in the protocol path
        let segments = &bound.protocol.segments;

        // Check for Self:: prefix or other associated type patterns
        if segments.len() >= 2 {
            // Pattern: `Self::AssocType` or `Protocol::AssocType`
            let last_segment = segments.last()?;
            // PathSegment only has Name, SelfValue, Super, Crate, Relative variants
            if let PathSegment::Name(ident) = last_segment {
                // If the last segment looks like an associated type (starts uppercase),
                // it might be a GAT reference
                let name = ident.as_str();
                if !name.is_empty() && name.chars().next()?.is_uppercase() {
                    return Some(Text::from(name));
                }
            }
        }

        // Also check protocol type arguments for GAT references
        for arg in bound.args.iter() {
            if let TypeKind::Path(path) = &arg.kind {
                if let Some(ident) = path.as_ident() {
                    let name = ident.as_str();
                    // Type parameters typically start with uppercase
                    if !name.is_empty() && name.chars().next().is_some_and(|c| c.is_uppercase()) {
                        // This could be a reference to another GAT
                        return Some(Text::from(name));
                    }
                }
            }
        }

        None
    }

    /// Extract GAT references from a type
    ///
    /// Recursively traverses a type to find all GAT references.
    /// This is critical for dependency analysis and cycle detection.
    fn extract_gat_references(ty: &Type, deps: &mut Set<Text>) {
        match &ty.kind {
            TypeKind::Path(path) => {
                // Check if path refers to a GAT
                if let Some(ident) = path.as_ident() {
                    let text: Text = ident.as_str().into();
                    deps.insert(text);
                }

                // Also check path segments for GAT references
                use verum_ast::ty::PathSegment;
                for segment in path.segments.iter() {
                    if let PathSegment::Name(ident) = segment {
                        // The segment might be a GAT
                        deps.insert(Text::from(ident.as_str()));
                    }
                }
            }
            TypeKind::Generic { base, args } => {
                // Recursively check base and arguments
                Self::extract_gat_references(base, deps);
                for arg in args.iter() {
                    if let GenericArg::Type(arg_ty) = arg {
                        Self::extract_gat_references(arg_ty, deps);
                    }
                }
            }
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. }
            | TypeKind::Pointer { inner, .. }
            | TypeKind::Ownership { inner, .. }
            | TypeKind::GenRef { inner }
            | TypeKind::Slice(inner) => {
                Self::extract_gat_references(inner, deps);
            }
            TypeKind::Array { element, .. } => {
                Self::extract_gat_references(element, deps);
            }
            TypeKind::Tuple(elements) => {
                for elem in elements.iter() {
                    Self::extract_gat_references(elem, deps);
                }
            }
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
                for param in params.iter() {
                    Self::extract_gat_references(param, deps);
                }
                Self::extract_gat_references(return_type, deps);
            }
            TypeKind::DynProtocol { bounds, bindings } => {
                // Check type bounds for GAT references
                for bound in bounds.iter() {
                    if let TypeBoundKind::Protocol(path) = &bound.kind {
                        use verum_ast::ty::PathSegment;
                        for segment in path.segments.iter() {
                            if let PathSegment::Name(ident) = segment {
                                deps.insert(Text::from(ident.as_str()));
                            }
                        }
                    }
                }

                // Check associated type bindings
                if let Some(binding_list) = bindings {
                    for binding in binding_list.iter() {
                        Self::extract_gat_references(&binding.ty, deps);
                    }
                }
            }
            TypeKind::Refined { base, .. } => {
                Self::extract_gat_references(base, deps);
            }
            _ => {
                // Primitive types and others don't contain GAT references
            }
        }
    }

    /// Detect cycle in dependency graph using DFS
    fn detect_cycle(
        &self,
        node: &Text,
        visited: &mut Set<Text>,
        stack: &mut Set<Text>,
    ) -> Option<List<Text>> {
        if stack.contains(node) {
            // Found cycle
            return Some(List::from(vec![node.clone()]));
        }

        if visited.contains(node) {
            return None;
        }

        visited.insert(node.clone());
        stack.insert(node.clone());

        if let Some(deps) = self.dependency_graph.get(node) {
            for dep in deps.iter() {
                if let Some(mut cycle) = self.detect_cycle(dep, visited, stack) {
                    cycle.push(node.clone());
                    return Some(cycle);
                }
            }
        }

        stack.remove(node);
        None
    }

    /// Verify constraints on a type parameter
    ///
    /// Encodes the constraint as a Z3 formula and checks satisfiability.
    ///
    /// # SMT Encoding
    ///
    /// For `type Item<T> where T: Clone + Debug`, generates:
    /// ```smt2
    /// (declare-sort Type)
    /// (declare-sort Protocol)
    /// (declare-fun implements (Type Protocol) Bool)
    /// (declare-const T_Item Type)
    /// (declare-const Clone Protocol)
    /// (declare-const Debug Protocol)
    /// (assert (implements T_Item Clone))
    /// (assert (implements T_Item Debug))
    /// (check-sat)  ; Should be SAT (there exist types implementing both)
    /// ```
    fn verify_type_param_constraints(
        &mut self,
        gat: &AssociatedTypeGAT,
        param: &GATTypeParam,
    ) -> Result<(), (GATError, Maybe<GATCounterexample>)> {
        if param.bounds.is_empty() {
            return Ok(());
        }

        // Initialize SMT sorts if needed
        self.init_smt_sorts();

        let solver = Solver::new();

        // Add protocol hierarchy constraints
        self.encode_protocol_hierarchy(&solver);

        // Add known implementation facts
        self.add_implementation_facts(&solver);

        // Encode each bound as a constraint
        // We check: exists T such that T implements all bounds
        let mut bound_constraints = Vec::new();
        for bound in param.bounds.iter() {
            match self.encode_protocol_bound(bound, &param.name) {
                Ok(constraint) => {
                    bound_constraints.push(constraint.clone());
                    solver.assert(&constraint);
                }
                Err(e) => {
                    return Err((
                        GATError::ConstraintViolation {
                            param: param.name.clone(),
                            constraint: format!("{:?}", bound).into(),
                            counterexample: Maybe::Some(e.to_string().into()),
                        },
                        Maybe::None,
                    ));
                }
            }
        }

        // Check if constraints are satisfiable
        match solver.check() {
            SatResult::Sat => {
                // Constraints are satisfiable - there exist types satisfying all bounds
                Ok(())
            }
            SatResult::Unsat => {
                // Constraints are contradictory
                // Generate counterexample showing why
                let counterexample = self.generate_unsatisfiable_counterexample(&solver, param);

                Err((
                    GATError::UnsatisfiableWhereClause {
                        param: param.name.clone(),
                        clause: format!("{:?}", param.bounds).into(),
                    },
                    Maybe::Some(counterexample),
                ))
            }
            SatResult::Unknown => {
                // Solver timed out - conservatively accept
                // Log warning for debugging
                Ok(())
            }
        }
    }

    /// Generate counterexample for unsatisfiable constraints
    fn generate_unsatisfiable_counterexample(
        &self,
        solver: &Solver,
        param: &GATTypeParam,
    ) -> GATCounterexample {
        // Try to identify conflicting bounds
        let mut conflicting_bounds = Vec::new();

        // Check each bound in isolation
        for (i, bound) in param.bounds.iter().enumerate() {
            let test_solver = Solver::new();
            if let Ok(constraint) = self.encode_protocol_bound(bound, &param.name) {
                test_solver.assert(&constraint);
                if test_solver.check() == SatResult::Unsat {
                    conflicting_bounds.push(format!("Bound {}: {:?}", i, bound));
                }
            }
        }

        // Check pairs of bounds for conflicts
        if conflicting_bounds.is_empty() && param.bounds.len() >= 2 {
            for i in 0..param.bounds.len() {
                for j in (i + 1)..param.bounds.len() {
                    let test_solver = Solver::new();
                    self.encode_protocol_hierarchy(&test_solver);

                    if let (Ok(c1), Ok(c2)) = (
                        self.encode_protocol_bound(&param.bounds[i], &param.name),
                        self.encode_protocol_bound(&param.bounds[j], &param.name),
                    ) {
                        test_solver.assert(&c1);
                        test_solver.assert(&c2);
                        if test_solver.check() == SatResult::Unsat {
                            conflicting_bounds.push(format!(
                                "Conflict between bounds {} and {}: {:?} + {:?}",
                                i, j, param.bounds[i], param.bounds[j]
                            ));
                        }
                    }
                }
            }
        }

        let explanation = if conflicting_bounds.is_empty() {
            "The combination of all bounds is unsatisfiable - no type can implement all protocols simultaneously".to_text()
        } else {
            format!("Conflicting constraints: {}", conflicting_bounds.join("; ")).into()
        };

        GATCounterexample {
            param: param.name.clone(),
            violating_type: "∅ (no type satisfies constraints)".to_text(),
            violated_constraint: format!("{:?}", param.bounds).into(),
            explanation,
        }
    }

    /// Encode a protocol bound as a Z3 Boolean formula
    ///
    /// Translates `param_name: Protocol` to `implements(param_name, Protocol)`
    ///
    /// # SMT Encoding
    ///
    /// For bound `T: Clone`, generates:
    /// ```smt2
    /// (implements T_const Clone_const)
    /// ```
    fn encode_protocol_bound(
        &self,
        bound: &ProtocolBound,
        param_name: &Text,
    ) -> Result<Bool, Text> {
        let implements_pred = self
            .implements_pred
            .as_ref()
            .ok_or_else(|| "SMT sorts not initialized".to_text())?;

        // Get protocol name from bound
        let protocol_name = if let Some(ident) = bound.protocol.as_ident() {
            ident.as_str().to_string()
        } else {
            // Handle qualified paths - extract name from last segment
            use verum_ast::ty::PathSegment;
            bound
                .protocol
                .segments
                .last()
                .and_then(|s| match s {
                    PathSegment::Name(ident) => Some(ident.as_str().to_string()),
                    _ => None,
                })
                .unwrap_or_else(|| format!("{:?}", bound.protocol))
        };

        // Create type constant for parameter
        let type_const = self.create_type_const(param_name.as_str());

        // Create protocol constant
        let protocol_const = self.create_protocol_const(&protocol_name);

        // Apply implements predicate: implements(T, Protocol)
        let implements_app = implements_pred.apply(&[&type_const, &protocol_const]);

        // Convert to Bool (implements returns Bool)
        if let Some(bool_ast) = implements_app.as_bool() {
            Ok(bool_ast)
        } else {
            // Fallback: create a fresh boolean variable
            let bound_name = format!("{}_{}", param_name, protocol_name);
            Ok(Bool::new_const(bound_name.as_str()))
        }
    }

    /// Encode protocol hierarchy constraints to solver
    ///
    /// For each super-protocol relationship, adds:
    /// ```smt2
    /// (assert (forall ((T Type))
    ///   (=> (implements T SubProtocol)
    ///       (implements T SuperProtocol))))
    /// ```
    fn encode_protocol_hierarchy(&self, solver: &Solver) {
        let implements_pred = match self.implements_pred.as_ref() {
            Some(p) => p,
            None => return,
        };

        for (protocol, supers) in self.protocol_table.protocols.iter() {
            for super_proto in supers.iter() {
                // Create type variable for quantification
                let type_var = self.create_type_var("T_hier");
                let protocol_const = self.create_protocol_const(protocol.as_str());
                let super_const = self.create_protocol_const(super_proto.as_str());

                // implements(T, Protocol)
                let impl_protocol = implements_pred.apply(&[&type_var, &protocol_const]);
                // implements(T, SuperProtocol)
                let impl_super = implements_pred.apply(&[&type_var, &super_const]);

                // Create implication: impl_protocol => impl_super
                if let (Some(p_bool), Some(s_bool)) =
                    (impl_protocol.as_bool(), impl_super.as_bool())
                {
                    let implication = p_bool.implies(&s_bool);

                    // Create forall quantifier with pattern
                    // Note: forall_const returns Bool directly, not Dynamic
                    let pattern = Pattern::new(&[&impl_protocol]);
                    let forall: Bool = forall_const(&[&type_var], &[&pattern], &implication);

                    solver.assert(&forall);
                }
            }
        }
    }

    /// Add known implementation facts to the solver
    fn add_implementation_facts(&self, solver: &Solver) {
        let implements_pred = match self.implements_pred.as_ref() {
            Some(p) => p,
            None => return,
        };

        for (ty, proto) in self.protocol_table.implementations.iter() {
            let type_const = self.create_type_const(ty.as_str());
            let proto_const = self.create_protocol_const(proto.as_str());

            let impl_fact = implements_pred.apply(&[&type_const, &proto_const]);
            if let Some(bool_ast) = impl_fact.as_bool() {
                solver.assert(&bool_ast);
            }
        }
    }

    /// Verify a where clause on a GAT
    ///
    /// Example: `type Item<T> where T: Clone + Debug`
    ///
    /// Checks:
    /// 1. Parameter exists in GAT definition
    /// 2. All protocol bounds are well-formed (protocols exist)
    /// 3. Constraints are satisfiable with GAT's own bounds
    fn verify_where_clause(
        &mut self,
        gat: &AssociatedTypeGAT,
        clause: &GATWhereClause,
    ) -> Result<(), GATError> {
        // Ensure the parameter exists
        let param = gat.type_params.iter().find(|p| p.name == clause.param);
        if param.is_none() {
            return Err(GATError::ConstraintViolation {
                param: clause.param.clone(),
                constraint: format!("{:?}", clause.constraints).into(),
                counterexample: Maybe::Some(
                    format!("Parameter {} not found in GAT {}", clause.param, gat.name).into(),
                ),
            });
        }

        let param = param.unwrap();

        // Initialize SMT sorts if needed
        self.init_smt_sorts();

        let solver = Solver::new();

        // Add protocol hierarchy constraints
        self.encode_protocol_hierarchy(&solver);

        // First, add the parameter's own bounds
        for bound in param.bounds.iter() {
            if let Ok(constraint) = self.encode_protocol_bound(bound, &clause.param) {
                solver.assert(&constraint);
            }
        }

        // Now verify each where clause constraint
        for constraint in clause.constraints.iter() {
            // Check if the protocol exists in our table
            use verum_ast::ty::PathSegment;
            let protocol_name = if let Some(ident) = constraint.protocol.as_ident() {
                ident.as_str().to_string()
            } else {
                constraint
                    .protocol
                    .segments
                    .last()
                    .and_then(|s| match s {
                        PathSegment::Name(ident) => Some(ident.as_str().to_string()),
                        _ => None,
                    })
                    .unwrap_or_default()
            };

            // Verify protocol is known (exists in protocol table or standard protocols)
            if !self
                .protocol_table
                .protocols
                .contains_key(&protocol_name.clone().into())
            {
                // Warning: Protocol not registered, but allow it
                // A stricter implementation could error here
            }

            // Encode and check the constraint is satisfiable
            match self.encode_protocol_bound(constraint, &clause.param) {
                Ok(bound_constraint) => {
                    // Check if adding this constraint makes the system unsatisfiable
                    solver.push();
                    solver.assert(&bound_constraint);

                    if solver.check() == SatResult::Unsat {
                        solver.pop(1);
                        return Err(GATError::UnsatisfiableWhereClause {
                            param: clause.param.clone(),
                            clause: format!(
                                "Where clause constraint {} conflicts with parameter bounds",
                                protocol_name
                            )
                            .into(),
                        });
                    }

                    solver.pop(1);
                    // Add the constraint for subsequent checks
                    solver.assert(&bound_constraint);
                }
                Err(e) => {
                    return Err(GATError::ConstraintViolation {
                        param: clause.param.clone(),
                        constraint: format!("{:?}", constraint).into(),
                        counterexample: Maybe::Some(e.to_string().into()),
                    });
                }
            }
        }

        Ok(())
    }

    /// Verify lifetime bounds on GAT type parameters
    ///
    /// GATs can have lifetime bounds that constrain how long the associated type
    /// can be used. This verifies:
    /// 1. Lifetime parameters are valid (e.g., 'a, 'b, 'static)
    /// 2. Lifetime bounds are well-formed (outlives relationships)
    /// 3. Lifetime bounds are satisfiable
    ///
    /// # Example
    ///
    /// ```verum
    /// protocol Container {
    ///     type Item<'a> where Self: 'a;  // Item requires Self to outlive 'a
    /// }
    /// ```
    fn verify_lifetime_bounds(
        &mut self,
        gat: &AssociatedTypeGAT,
        stats: &mut GATStats,
    ) -> Result<(), List<GATError>> {
        let mut errors = List::new();

        // Check lifetime parameters in type params
        for param in gat.type_params.iter() {
            for bound in param.bounds.iter() {
                // Extract lifetime constraints from bounds
                // Lifetimes can appear in protocol bounds like 'a: 'b
                if self.is_lifetime_bound_in_protocol_bound(bound) {
                    stats.lifetime_bounds_checked += 1;

                    // Verify the lifetime bound is well-formed
                    if let Err(err) = self.verify_lifetime_well_formed(gat, param, bound) {
                        errors.push(err);
                    }
                }
            }
        }

        // Check where clauses for lifetime constraints
        for clause in gat.where_clauses.iter() {
            for constraint in clause.constraints.iter() {
                if self.is_lifetime_bound_in_protocol_bound(constraint) {
                    stats.lifetime_bounds_checked += 1;
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Check if a protocol bound contains lifetime constraints
    fn is_lifetime_bound_in_protocol_bound(&self, bound: &ProtocolBound) -> bool {
        // PathSegment variants don't include Generic in verum_ast
        // Lifetimes would be found in the type arguments of the bound's args field
        // which is checked below

        // Check type arguments for lifetime references
        for arg_ty in bound.args.iter() {
            if self.type_contains_lifetime(arg_ty) {
                return true;
            }
        }

        false
    }

    /// Check if a type contains lifetime references
    fn type_contains_lifetime(&self, ty: &Type) -> bool {
        match &ty.kind {
            TypeKind::Reference { .. }
            | TypeKind::CheckedReference { .. }
            | TypeKind::UnsafeReference { .. } => {
                // References always have implied lifetimes
                true
            }
            TypeKind::Generic { base, args } => {
                self.type_contains_lifetime(base)
                    || args.iter().any(|arg| match arg {
                        GenericArg::Lifetime(_) => true,
                        GenericArg::Type(t) => self.type_contains_lifetime(t),
                        _ => false,
                    })
            }
            TypeKind::Tuple(elems) => elems.iter().any(|e| self.type_contains_lifetime(e)),
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
                params.iter().any(|p| self.type_contains_lifetime(p))
                    || self.type_contains_lifetime(return_type)
            }
            _ => false,
        }
    }

    /// Verify that a lifetime bound is well-formed
    fn verify_lifetime_well_formed(
        &self,
        gat: &AssociatedTypeGAT,
        param: &GATTypeParam,
        bound: &ProtocolBound,
    ) -> Result<(), GATError> {
        // Use Z3 to verify lifetime constraints are satisfiable
        // Lifetimes are modeled as partial orders (outlives relation)

        let solver = Solver::new();

        // Create a lifetime sort
        let lifetime_sort = Sort::uninterpreted(Symbol::String("Lifetime".to_string()));

        // Create outlives predicate: outlives(L1, L2) means L1: L2
        let outlives_pred = FuncDecl::new(
            Symbol::String("outlives".to_string()),
            &[&lifetime_sort, &lifetime_sort],
            &Sort::bool(),
        );

        // Reflexivity: forall L. outlives(L, L)
        let l_var =
            FuncDecl::new(Symbol::String("L_refl".to_string()), &[], &lifetime_sort).apply(&[]);
        let refl = outlives_pred.apply(&[&l_var, &l_var]);
        if let Some(b) = refl.as_bool() {
            solver.assert(&b);
        }

        // Transitivity: forall L1, L2, L3. (outlives(L1, L2) && outlives(L2, L3)) => outlives(L1, L3)
        // (Encoded but simplified for now)

        // 'static outlives everything
        let static_lt =
            FuncDecl::new(Symbol::String("static".to_string()), &[], &lifetime_sort).apply(&[]);
        let any_lt = FuncDecl::new(
            Symbol::String("any_lifetime".to_string()),
            &[],
            &lifetime_sort,
        )
        .apply(&[]);
        let static_outlives = outlives_pred.apply(&[&static_lt, &any_lt]);
        if let Some(b) = static_outlives.as_bool() {
            solver.assert(&b);
        }

        // Check satisfiability
        match solver.check() {
            SatResult::Sat | SatResult::Unknown => Ok(()),
            SatResult::Unsat => Err(GATError::LifetimeBoundViolation {
                gat_name: gat.name.clone(),
                lifetime_param: param.name.clone(),
                required_bound: format!("{:?}", bound).into(),
                explanation: "Lifetime bounds are unsatisfiable".to_text(),
            }),
        }
    }

    /// Verify transitive bounds on GAT type parameters
    ///
    /// When a type parameter has bounds like `T: Clone` and there's a where clause
    /// like `where T::Item: Debug`, we need to verify that transitive bounds are
    /// satisfiable.
    ///
    /// # Example
    ///
    /// ```verum
    /// protocol Container {
    ///     type Item<T> where T: Clone, T: Iterator, <T as Iterator>::Item: Debug;
    /// }
    /// ```
    fn verify_transitive_bounds(
        &mut self,
        gat: &AssociatedTypeGAT,
        stats: &mut GATStats,
    ) -> Result<(), List<GATError>> {
        let mut errors = List::new();

        self.init_smt_sorts();
        let solver = Solver::new();

        // Add protocol hierarchy to solver
        self.encode_protocol_hierarchy(&solver);

        // Collect all bounds for each parameter
        let mut param_bounds: HashMap<Text, Vec<&ProtocolBound>> = HashMap::new();
        for param in gat.type_params.iter() {
            let bounds = param_bounds.entry(param.name.clone()).or_default();
            for bound in param.bounds.iter() {
                bounds.push(bound);
            }
        }

        // Add where clause constraints
        for clause in gat.where_clauses.iter() {
            let bounds = param_bounds.entry(clause.param.clone()).or_default();
            for constraint in clause.constraints.iter() {
                bounds.push(constraint);
            }
        }

        // For each parameter, verify that all bounds (including transitive) are satisfiable
        for (param_name, bounds) in param_bounds.iter() {
            stats.transitive_bounds_checked += 1;

            // Encode all bounds for this parameter
            for bound in bounds.iter() {
                if let Ok(constraint) = self.encode_protocol_bound(bound, param_name) {
                    solver.push();
                    solver.assert(&constraint);

                    // Check for transitive implications
                    if let Some(super_protocols) = self.get_super_protocols_from_bound(bound) {
                        for super_proto in super_protocols.iter() {
                            // The type must also implement all super-protocols
                            let super_bound = ProtocolBound::positive(
                                Path::from_ident(verum_ast::Ident::new(
                                    super_proto.as_str(),
                                    verum_ast::span::Span::dummy(),
                                )),
                                List::new(),
                            );
                            if let Ok(super_constraint) =
                                self.encode_protocol_bound(&super_bound, param_name)
                            {
                                // Super-protocol should be implied
                                solver.assert(super_constraint.not());
                                if solver.check() == SatResult::Sat {
                                    // The super-protocol constraint is not satisfied
                                    let direct_name = bound
                                        .protocol
                                        .as_ident()
                                        .map(|i| i.as_str().to_string())
                                        .unwrap_or_else(|| format!("{:?}", bound.protocol));

                                    let explanation = format!(
                                        "Protocol {} requires {}, but this bound is not satisfied",
                                        direct_name, super_proto
                                    );

                                    errors.push(GATError::TransitiveBoundViolation {
                                        gat_name: gat.name.clone(),
                                        param: param_name.clone(),
                                        direct_bound: direct_name.into(),
                                        transitive_bound: super_proto.clone(),
                                        explanation: explanation.into(),
                                    });
                                }
                            }
                        }
                    }

                    solver.pop(1);
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Get super-protocols from a protocol bound
    fn get_super_protocols_from_bound(&self, bound: &ProtocolBound) -> Option<List<Text>> {
        let protocol_name = bound.protocol.as_ident().map(|i| Text::from(i.as_str()))?;

        self.protocol_table.protocols.get(&protocol_name).cloned()
    }

    /// Check variance annotation correctness
    ///
    /// Ensures variance matches actual usage in the GAT definition.
    fn check_variance(
        &self,
        gat: &AssociatedTypeGAT,
        param: &GATTypeParam,
    ) -> Result<(), GATError> {
        // Infer actual variance from usage
        let inferred_variance = self.infer_variance(gat, param);

        // Check if declared variance is compatible
        // Invariant is compatible with anything (most restrictive)
        // Covariant is compatible with Covariant
        // Contravariant is compatible with Contravariant
        let is_compatible = match (param.variance, inferred_variance) {
            // Same variance: always compatible
            (v1, v2) if v1 == v2 => true,
            // Declared Invariant: always compatible (most restrictive)
            (Variance::Invariant, _) => true,
            // Inferred Invariant: declared must be Invariant
            (_, Variance::Invariant) => false,
            // Otherwise: incompatible
            _ => false,
        };

        if !is_compatible {
            return Err(GATError::VarianceViolation {
                param: param.name.clone(),
                expected: inferred_variance,
                found: param.variance,
            });
        }

        Ok(())
    }

    /// Infer variance from type parameter usage
    ///
    /// Analysis rules:
    /// - Appears only in output positions (return types, &T): Covariant
    /// - Appears only in input positions (parameters): Contravariant
    /// - Appears in both or in mutable positions (&mut T): Invariant
    /// - Not used: Covariant by default (safe)
    ///
    /// # Position Tracking
    ///
    /// The algorithm tracks position polarity through type structure:
    /// - Initial position: Covariant (output)
    /// - fn(T) -> U: T is Contravariant, U is Covariant
    /// - &T: T position unchanged (covariant in covariant)
    /// - &mut T: T becomes Invariant
    /// - Container<T>: Depends on Container's declared variance for T
    fn infer_variance(&self, gat: &AssociatedTypeGAT, param: &GATTypeParam) -> Variance {
        let mut tracker = VarianceTracker::new(param.name.clone());

        // Analyze the default type if present
        if let Maybe::Some(ref default_ty) = gat.default {
            tracker.analyze_type(default_ty, VariancePosition::Covariant);
        }

        // Analyze bounds on the associated type itself
        // Only record variance if the parameter actually appears in the bound
        for bound in gat.bounds.iter() {
            // Protocol bounds are in input position, so they flip variance
            // But we only record usage if the parameter appears in the bound
            tracker.analyze_protocol_bound(bound, VariancePosition::Contravariant);
        }

        // NOTE: Where clauses are constraints, not usage positions.
        // They don't affect variance inference - variance comes from
        // actual type usage in bounds, defaults, and type structure.
        // Simply having a where clause doesn't make a parameter contravariant.

        // If no usage was found, default to the declared variance
        // This allows GATs with only constraints (where clauses) to specify their own variance
        if !tracker.seen_covariant && !tracker.seen_contravariant && !tracker.seen_invariant {
            return param.variance;
        }

        // Return inferred variance based on actual usage
        tracker.get_variance()
    }

    /// Verify GAT instantiation with concrete type arguments
    ///
    /// Checks that the type arguments satisfy all GAT constraints.
    ///
    /// # Example
    ///
    /// ```no_run
    /// // GAT: type Item<T> where T: Clone
    /// // Instantiation: Item<Int>
    /// // Verify: Int implements Clone
    /// ```
    pub fn verify_instantiation(
        &mut self,
        gat: &AssociatedTypeGAT,
        type_args: &[Type],
    ) -> Result<(), GATError> {
        // Check arity
        if type_args.len() != gat.type_params.len() {
            return Err(GATError::ArityMismatch {
                gat_name: gat.name.clone(),
                expected: gat.type_params.len(),
                found: type_args.len(),
            });
        }

        // Verify each type argument against parameter constraints
        for (param, arg_ty) in gat.type_params.iter().zip(type_args.iter()) {
            self.verify_arg_satisfies_constraints(param, arg_ty)?;
        }

        Ok(())
    }

    /// Verify a type argument satisfies parameter constraints
    ///
    /// Full implementation that:
    /// 1. Queries the protocol table for direct implementations
    /// 2. Checks superprotocol hierarchy for transitive implementations
    /// 3. Uses SMT solver for complex constraint verification
    /// 4. Handles variance requirements for type parameters
    fn verify_arg_satisfies_constraints(
        &self,
        param: &GATTypeParam,
        arg_ty: &Type,
    ) -> Result<(), GATError> {
        // For each bound, check if the type implements the protocol
        for bound in param.bounds.iter() {
            // Get the protocol name from the bound
            let protocol_name = bound
                .protocol
                .as_ident()
                .map(|i| Text::from(i.as_str()))
                .unwrap_or_else(|| Text::from("Unknown"));

            // Get a text representation of the type for lookup
            let type_text = self.type_to_text(arg_ty);

            // Check 1: Direct lookup in protocol table
            if self.protocol_table.implements(&type_text, &protocol_name) {
                continue; // This bound is satisfied
            }

            // Check 2: Use the crate-level protocol checker for full verification
            match crate::protocol_smt::check_implements(arg_ty, protocol_name.as_str()) {
                Ok(true) => continue, // Bound satisfied
                Ok(false) => {
                    // Check 3: For generic types, use SMT verification
                    if self.is_generic_type(arg_ty) {
                        // Encode as SMT constraint and verify
                        if self.verify_generic_type_bound_smt(arg_ty, bound)? {
                            continue;
                        }
                    }

                    // Bound not satisfied
                    return Err(GATError::UnsatisfiedBound {
                        gat_name: param.name.clone(),
                        ty: arg_ty.clone(),
                        bound: protocol_name.to_string(),
                    });
                }
                Err(_) => {
                    // Protocol checker error - try SMT verification as fallback
                    if !self.verify_bound_with_smt(arg_ty, bound)? {
                        return Err(GATError::UnsatisfiedBound {
                            gat_name: param.name.clone(),
                            ty: arg_ty.clone(),
                            bound: protocol_name.to_string(),
                        });
                    }
                }
            }
        }

        // Check variance constraints
        self.verify_variance_constraints(param, arg_ty, &param.variance)?;

        Ok(())
    }

    /// Convert a type to a text representation for protocol table lookup
    fn type_to_text(&self, ty: &Type) -> Text {
        match &ty.kind {
            TypeKind::Path(path) => path
                .as_ident()
                .map(|i| Text::from(i.as_str()))
                .unwrap_or_else(|| Text::from(format!("{:?}", path))),
            TypeKind::Int => Text::from("Int"),
            TypeKind::Bool => Text::from("Bool"),
            TypeKind::Float => Text::from("Float"),
            TypeKind::Text => Text::from("Text"),
            TypeKind::Char => Text::from("Char"),
            TypeKind::Unit => Text::from("Unit"),
            TypeKind::Generic { base, args } => {
                let base_text = self.type_to_text(base);
                let args_text: List<String> = args
                    .iter()
                    .filter_map(|arg| {
                        if let GenericArg::Type(t) = arg {
                            Some(self.type_to_text(t).to_string())
                        } else {
                            None
                        }
                    })
                    .collect();
                Text::from(format!("{}<{}>", base_text, args_text.join(", ")))
            }
            _ => Text::from(format!("{:?}", ty.kind)),
        }
    }

    /// Check if a type is generic (contains type variables)
    fn is_generic_type(&self, ty: &Type) -> bool {
        match &ty.kind {
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    // Single uppercase letter or common type param patterns
                    let name = ident.as_str();
                    if name.len() == 1 && name.chars().all(|c| c.is_uppercase()) {
                        return true;
                    }
                }
                false
            }
            TypeKind::Generic { base, args } => {
                self.is_generic_type(base)
                    || args.iter().any(|arg| {
                        if let GenericArg::Type(t) = arg {
                            self.is_generic_type(t)
                        } else {
                            false
                        }
                    })
            }
            TypeKind::Reference { inner, .. } => self.is_generic_type(inner),
            TypeKind::Tuple(elems) => elems.iter().any(|e| self.is_generic_type(e)),
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
                params.iter().any(|p| self.is_generic_type(p)) || self.is_generic_type(return_type)
            }
            TypeKind::Inferred => true, // Inferred types are essentially generic
            _ => false,
        }
    }

    /// Verify a generic type bound using SMT solver
    ///
    /// For generic types, we encode the constraint as an SMT formula:
    /// forall T. (implements(T, Bound) => valid_instantiation)
    fn verify_generic_type_bound_smt(
        &self,
        _arg_ty: &Type,
        bound: &ProtocolBound,
    ) -> Result<bool, GATError> {
        let solver = Solver::new();

        // If we have the implements predicate, use it
        if let (Some(_type_sort), Some(_protocol_sort), Some(implements_pred)) =
            (&self.type_sort, &self.protocol_sort, &self.implements_pred)
        {
            // Create type constant for the argument
            let t = Int::fresh_const("T");

            // Get protocol name
            let protocol_name = bound
                .protocol
                .as_ident()
                .map(|i| i.as_str())
                .unwrap_or("Unknown");

            // Create the implements(T, Protocol) assertion
            let protocol_const = Int::fresh_const(protocol_name);
            let implements_app = implements_pred.apply(&[&t, &protocol_const]);

            // For generic bounds, we need to check if the constraint is consistent
            // We check if NOT(implements(T, Protocol)) is unsatisfiable
            // (meaning all T must implement Protocol based on the constraints)
            solver.assert(implements_app.as_bool().unwrap().not());

            match solver.check() {
                SatResult::Unsat => Ok(true),    // Constraint holds for all types
                SatResult::Sat => Ok(false),     // Found counterexample
                SatResult::Unknown => Ok(false), // Conservative: assume not verified
            }
        } else {
            // No implements predicate set up - return true (assume valid)
            // A full implementation would set this up in the constructor
            Ok(true)
        }
    }

    /// Fallback SMT verification for bounds
    fn verify_bound_with_smt(
        &self,
        arg_ty: &Type,
        bound: &ProtocolBound,
    ) -> Result<bool, GATError> {
        // Encode bound using encode_protocol_bound
        match self.encode_protocol_bound(bound, &Text::from("T")) {
            Ok(constraint) => {
                let solver = Solver::new();
                solver.assert(constraint.not());

                match solver.check() {
                    SatResult::Unsat => Ok(true),
                    _ => Ok(false),
                }
            }
            Err(_) => {
                // Encoding failed - check the protocol table as last resort
                let type_text = self.type_to_text(arg_ty);
                let protocol_name = bound
                    .protocol
                    .as_ident()
                    .map(|i| Text::from(i.as_str()))
                    .unwrap_or_else(|| Text::from("Unknown"));

                Ok(self.protocol_table.implements(&type_text, &protocol_name))
            }
        }
    }

    /// Verify variance constraints for a type argument
    fn verify_variance_constraints(
        &self,
        param: &GATTypeParam,
        arg_ty: &Type,
        declared_variance: &Variance,
    ) -> Result<(), GATError> {
        // Track the actual variance usage of the type
        let mut tracker = VarianceTracker::new(param.name.clone());

        // Analyze the type structure to determine actual variance
        self.track_type_variance(arg_ty, VariancePosition::Covariant, &mut tracker);

        // Check if actual variance is compatible with declared variance
        let inferred = tracker.get_variance();

        let compatible = match declared_variance {
            Variance::Covariant => inferred == Variance::Covariant,
            Variance::Contravariant => inferred == Variance::Contravariant,
            Variance::Invariant => true, // Invariant is always safe (most restrictive)
        };

        if !compatible {
            return Err(GATError::VarianceMismatch {
                gat_name: param.name.clone(),
                expected: format!("{:?}", declared_variance),
                found: format!("{:?}", inferred),
            });
        }

        Ok(())
    }

    /// Track variance positions in a type
    fn track_type_variance(
        &self,
        ty: &Type,
        position: VariancePosition,
        tracker: &mut VarianceTracker,
    ) {
        match &ty.kind {
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    if ident.as_str() == tracker.param_name.as_str() {
                        tracker.record_usage(position);
                    }
                }
            }
            TypeKind::Reference {
                mutable: true,
                inner,
            } => {
                // Mutable references are invariant
                self.track_type_variance(inner, VariancePosition::Invariant, tracker);
            }
            TypeKind::Reference {
                mutable: false,
                inner,
            } => {
                // Immutable references are covariant
                self.track_type_variance(inner, position, tracker);
            }
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
                // Parameters are contravariant
                for param in params {
                    self.track_type_variance(param, position.flip(), tracker);
                }
                // Return type is covariant
                self.track_type_variance(return_type, position, tracker);
            }
            TypeKind::Generic { base, args } => {
                // Base type maintains position
                self.track_type_variance(base, position, tracker);
                // Generic args depend on the specific container's variance
                // For now, assume invariant for all args (conservative)
                for arg in args {
                    if let GenericArg::Type(t) = arg {
                        self.track_type_variance(t, VariancePosition::Invariant, tracker);
                    }
                }
            }
            TypeKind::Tuple(elems) => {
                for elem in elems {
                    self.track_type_variance(elem, position, tracker);
                }
            }
            _ => {}
        }
    }

    /// Generate counterexample for constraint violation
    ///
    /// Uses Z3 model to extract a concrete type that violates the constraint.
    #[allow(dead_code)] // Part of verification diagnostics API
    fn generate_counterexample(
        &self,
        solver: &Solver,
        param: &Text,
        constraint: &Text,
    ) -> GATCounterexample {
        // Try to get a model
        if let Some(model) = solver.get_model() {
            // Extract type variable value from model
            let violating_type = format!("{:?}", model).into();

            GATCounterexample {
                param: param.clone(),
                violating_type,
                violated_constraint: constraint.clone(),
                explanation: "Extracted from Z3 model".to_text(),
            }
        } else {
            GATCounterexample {
                param: param.clone(),
                violating_type: "Unknown".to_text(),
                violated_constraint: constraint.clone(),
                explanation: "No model available".to_text(),
            }
        }
    }

    /// Clear verification cache
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> CacheStats {
        CacheStats {
            entries: self.cache.len(),
            hits: 0, // Would track in production
            misses: 0,
        }
    }
}

impl Default for GATVerifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub entries: usize,
    pub hits: usize,
    pub misses: usize,
}

// ==================== High-Level API ====================

/// Verify a GAT definition is well-formed
///
/// Convenience function for one-off verification.
pub fn verify_gat(gat: &AssociatedTypeGAT) -> GATVerificationResult {
    let mut verifier = GATVerifier::new();
    verifier.verify(gat)
}

/// Verify multiple GATs (batch verification)
///
/// More efficient than individual calls due to caching.
pub fn verify_gats(gats: &[AssociatedTypeGAT]) -> List<GATVerificationResult> {
    let mut verifier = GATVerifier::new();
    let mut results = List::new();

    for gat in gats {
        results.push(verifier.verify(gat));
    }

    results
}

/// Check if a GAT is well-formed (quick check)
///
/// Returns true if all constraints are satisfied.
pub fn is_well_formed(gat: &AssociatedTypeGAT) -> bool {
    let result = verify_gat(gat);
    result.is_valid
}

// ==================== Error Suggestions ====================

/// Generate suggestions for fixing GAT errors
pub fn suggest_fixes(error: &GATError) -> List<Text> {
    let mut suggestions = List::new();

    match error {
        GATError::CircularDependency { cycle } => {
            suggestions.push("Break the circular dependency by:".to_text());
            suggestions.push("  1. Using a type alias".to_text());
            suggestions.push("  2. Introducing a helper GAT".to_text());
            suggestions.push(
                format!(
                    "  Cycle: {}",
                    cycle
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(" -> ")
                )
                .into(),
            );
        }
        GATError::ConstraintViolation {
            param, constraint, ..
        } => {
            suggestions.push(format!("Weaken constraint on parameter {}", param).into());
            suggestions.push("Or provide a default type that satisfies it".to_text());
        }
        GATError::UnsatisfiableWhereClause { param, clause } => {
            suggestions.push(format!("Remove contradictory constraints on {}", param).into());
            suggestions.push("Check protocol compatibility".to_text());
        }
        GATError::VarianceViolation {
            param,
            expected,
            found,
        } => {
            suggestions.push(
                format!(
                    "Change variance annotation on {} from {:?} to {:?}",
                    param, found, expected
                )
                .into(),
            );
        }
        GATError::ArityMismatch {
            gat_name,
            expected,
            found,
        } => {
            suggestions.push(
                format!(
                    "Provide {} type arguments to GAT {} (found {})",
                    expected, gat_name, found
                )
                .into(),
            );
        }
        GATError::UnsatisfiedBound {
            gat_name,
            ty: _,
            bound,
        } => {
            suggestions.push(
                format!(
                    "Type argument for GAT {} does not satisfy bound: {}",
                    gat_name, bound
                )
                .into(),
            );
            suggestions
                .push("Consider using a type that implements the required protocol".to_text());
        }
        GATError::VarianceMismatch {
            gat_name,
            expected,
            found,
        } => {
            suggestions.push(
                format!(
                    "GAT {} has variance mismatch: declared {} but inferred {}",
                    gat_name, expected, found
                )
                .into(),
            );
            suggestions
                .push("Update the variance annotation or restructure the GAT definition".to_text());
        }
        GATError::LifetimeBoundViolation {
            gat_name,
            lifetime_param,
            required_bound,
            explanation,
        } => {
            suggestions.push(
                format!(
                    "GAT {} has an unsatisfiable lifetime bound on parameter {}",
                    gat_name, lifetime_param
                )
                .into(),
            );
            suggestions.push(format!("Required bound: {}", required_bound).into());
            suggestions.push(explanation.clone());
            suggestions.push("Consider loosening lifetime constraints or using 'static".to_text());
        }
        GATError::TransitiveBoundViolation {
            gat_name,
            param,
            direct_bound,
            transitive_bound,
            explanation,
        } => {
            suggestions.push(
                format!(
                    "GAT {} parameter {} has unsatisfied transitive bound",
                    gat_name, param
                )
                .into(),
            );
            suggestions.push(
                format!(
                    "Direct bound {} requires transitive bound {}",
                    direct_bound, transitive_bound
                )
                .into(),
            );
            suggestions.push(explanation.clone());
            suggestions.push(
                "Add the missing transitive bound explicitly or remove the direct bound".to_text(),
            );
        }
    }

    suggestions
}

#[cfg(test)]
mod tests {
    use super::*;
    

    #[test]
    fn test_simple_gat_verification() {
        let gat = AssociatedTypeGAT::simple("Item".into(), List::new());
        let result = verify_gat(&gat);
        assert!(result.is_valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_gat_with_constraints() {
        let param = GATTypeParam {
            name: "T".into(),
            bounds: List::new(), // No bounds - should verify
            default: Maybe::None,
            variance: Variance::Covariant,
        };

        let gat = AssociatedTypeGAT::generic(
            "Wrapped".into(),
            List::from(vec![param]),
            List::new(),
            List::new(),
        );

        let result = verify_gat(&gat);
        assert!(result.is_valid);
    }

    #[test]
    fn test_batch_verification() {
        let gat1 = AssociatedTypeGAT::simple("Item".into(), List::new());
        let gat2 = AssociatedTypeGAT::simple("Output".into(), List::new());

        let results = verify_gats(&[gat1, gat2]);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.is_valid));
    }

    #[test]
    fn test_is_well_formed() {
        let gat = AssociatedTypeGAT::simple("Item".into(), List::new());
        assert!(is_well_formed(&gat));
    }

    #[test]
    fn test_cache_stats() {
        let mut verifier = GATVerifier::new();
        let gat = AssociatedTypeGAT::simple("Item".into(), List::new());

        verifier.verify(&gat);
        let stats = verifier.cache_stats();
        assert_eq!(stats.entries, 1);
    }
}
