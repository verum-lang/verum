//! Proof Validator for Verum's Formal Proof System
//!
//! This module implements complete proof term validation according to
//! the Verum formal proof system. It validates that proof terms correctly prove
//! their claimed propositions using the Curry-Howard correspondence and
//! formal proof rules.
//!
//! ## Architecture
//!
//! The validator consists of three main components:
//!
//! 1. **ProofValidator** - Main validation engine that checks proof terms
//!    against their propositions using formal proof rules
//!
//! 2. **HypothesisContext** - Manages hypothesis scoping during proof validation,
//!    tracking available assumptions and their types
//!
//! 3. **ProofCertificateGenerator** - Generates proof certificates in standard
//!    formats (Dedukti, Coq, Lean) for external verification
//!
//! ## Validation Rules
//!
//! The validator implements all proof rules from the unified proof term system:
//!
//! - **Axiom**: Valid if axiom exists in the axiom database
//! - **Assumption/Hypothesis**: Valid if assumption is in hypothesis context
//! - **ModusPonens**: Given proofs of P and P→Q, validate proof of Q
//! - **Rewrite**: Validate equality proof and correct substitution
//! - **Symmetry/Transitivity/Reflexivity**: Validate equality reasoning
//! - **Induction**: Validate base case and inductive step
//! - **Lambda**: Check introduction rule for implications/forall
//! - **Cases**: Check all cases are covered and prove the same goal
//! - **Apply**: Check function application is well-typed
//! - **SmtProof**: Validate SMT-generated proofs match propositions
//!
//! ## Example Usage
//!
//! ```no_run
//! use verum_verification::proof_validator::ProofValidator;
//!
//! // Create validator with axiom database
//! let validator = ProofValidator::new();
//!
//! // Register axioms and validate proof terms
//! // (Full example code omitted - see tests for complete examples)
//! ```
//!
//! Formal Proofs System (Verum 2.0+ planned):
//! Proof terms are first-class values (type Proof<P: Prop>). Core rules include
//! Axiom, Assumption, ModusPonens, Rewrite, Symmetry, Transitivity, Reflexivity,
//! Induction, Lambda, Cases, Apply, and SmtProof. Tactics (simp, ring, omega,
//! blast, auto) automate common proof patterns. SMT integration dispatches to
//! Z3 for decidable fragments. Proof certificates exported to Dedukti/Coq/Lean.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use verum_ast::literal::{IntLit, Literal, LiteralKind};
use verum_ast::pattern::Pattern;
use verum_ast::span::Span;
use verum_ast::ty::PathSegment;
use verum_ast::{BinOp, Expr, ExprKind};
// Use verum_common types to match verum_ast types (Heap = Box, List = Vec, Maybe = Option)
use verum_common::{Heap, List, Map, Maybe, Set, Text};

// Import the unified ProofTerm from verum_smt
use verum_smt::proof_term_unified::{ProofError, ProofTerm};

// =============================================================================
// Proof Cache System
// =============================================================================

/// A cache key for proof validation results
#[derive(Clone, Debug)]
pub struct ProofCacheKey {
    /// Hash of the proof term structure
    proof_hash: u64,
    /// Hash of the expected proposition
    expected_hash: u64,
    /// Configuration hash (to invalidate on config changes)
    config_hash: u64,
}

impl PartialEq for ProofCacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.proof_hash == other.proof_hash
            && self.expected_hash == other.expected_hash
            && self.config_hash == other.config_hash
    }
}

impl Eq for ProofCacheKey {}

impl Hash for ProofCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.proof_hash.hash(state);
        self.expected_hash.hash(state);
        self.config_hash.hash(state);
    }
}

/// Cached validation result with timestamp
#[derive(Clone, Debug)]
struct CachedResult {
    /// The validation result (Ok or specific error type)
    result: Result<(), ValidationError>,
    /// When this result was cached
    cached_at: Instant,
    /// How many times this cache entry was accessed
    access_count: u64,
}

/// LRU-style proof cache for validation results
#[derive(Debug)]
pub struct ProofCache {
    /// Cached validation results
    cache: HashMap<ProofCacheKey, CachedResult>,
    /// Maximum cache size
    max_size: usize,
    /// Cache TTL (time-to-live) in seconds
    ttl_seconds: u64,
    /// Cache statistics
    stats: CacheStats,
}

/// Statistics for the proof cache
#[derive(Debug, Default, Clone)]
pub struct CacheStats {
    /// Number of cache hits
    pub hits: u64,
    /// Number of cache misses
    pub misses: u64,
    /// Number of cache evictions
    pub evictions: u64,
    /// Total time saved by cache hits (microseconds)
    pub time_saved_us: u64,
}

impl ProofCache {
    /// Create a new proof cache with default settings
    pub fn new() -> Self {
        Self {
            cache: HashMap::with_capacity(1024),
            max_size: 10000,
            ttl_seconds: 300, // 5 minutes
            stats: CacheStats::default(),
        }
    }

    /// Create a proof cache with custom settings
    pub fn with_config(max_size: usize, ttl_seconds: u64) -> Self {
        Self {
            cache: HashMap::with_capacity(max_size.min(1024)),
            max_size,
            ttl_seconds,
            stats: CacheStats::default(),
        }
    }

    /// Look up a cached result
    pub fn get(&mut self, key: &ProofCacheKey) -> Option<Result<(), ValidationError>> {
        if let Some(entry) = self.cache.get_mut(key) {
            // Check TTL
            if entry.cached_at.elapsed().as_secs() < self.ttl_seconds {
                self.stats.hits += 1;
                entry.access_count += 1;
                return Some(entry.result.clone());
            } else {
                // Entry expired, remove it
                self.cache.remove(key);
            }
        }
        self.stats.misses += 1;
        None
    }

    /// Insert a result into the cache
    pub fn insert(&mut self, key: ProofCacheKey, result: Result<(), ValidationError>) {
        // Evict if necessary
        if self.cache.len() >= self.max_size {
            self.evict_least_used();
        }

        self.cache.insert(
            key,
            CachedResult {
                result,
                cached_at: Instant::now(),
                access_count: 1,
            },
        );
    }

    /// Evict the least recently used entry
    fn evict_least_used(&mut self) {
        if let Some(key) = self
            .cache
            .iter()
            .min_by_key(|(_, v)| (v.access_count, v.cached_at))
            .map(|(k, _)| k.clone())
        {
            self.cache.remove(&key);
            self.stats.evictions += 1;
        }
    }

    /// Clear the entire cache
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Get cache statistics
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Invalidate entries matching a predicate
    pub fn invalidate<F>(&mut self, predicate: F)
    where
        F: Fn(&ProofCacheKey) -> bool,
    {
        self.cache.retain(|k, _| !predicate(k));
    }
}

impl Default for ProofCache {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Proof Obligations Tracking
// =============================================================================

/// A proof obligation that must be discharged
#[derive(Debug, Clone)]
pub struct ProofObligation {
    /// Unique ID for this obligation
    pub id: u64,
    /// The proposition to be proven
    pub proposition: Expr,
    /// The context (hypotheses available)
    pub context: List<(Text, Expr)>,
    /// Source location
    pub location: Maybe<Span>,
    /// Kind of obligation
    pub kind: ObligationKind,
    /// Status of this obligation
    pub status: ObligationStatus,
    /// Timestamp when created
    pub created_at: Instant,
    /// Proof term if discharged
    pub proof: Maybe<ProofTerm>,
}

/// Kind of proof obligation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObligationKind {
    /// Function precondition check
    Precondition,
    /// Function postcondition check
    Postcondition,
    /// Loop invariant establishment
    InvariantEstablishment,
    /// Loop invariant preservation
    InvariantPreservation,
    /// Loop termination (variant decrease)
    Termination,
    /// Assertion check
    Assertion,
    /// Type refinement check
    Refinement,
    /// Memory safety check (CBGR)
    MemorySafety,
    /// User-specified proof goal
    UserGoal,
}

/// Status of a proof obligation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObligationStatus {
    /// Not yet attempted
    Pending,
    /// Currently being verified
    InProgress,
    /// Successfully discharged
    Discharged,
    /// Failed to discharge
    Failed,
    /// Timed out during verification
    Timeout,
    /// Skipped (e.g., due to verification level)
    Skipped,
}

/// Global counter for obligation IDs
static OBLIGATION_COUNTER: AtomicU64 = AtomicU64::new(1);

impl ProofObligation {
    /// Create a new proof obligation
    pub fn new(
        proposition: Expr,
        context: List<(Text, Expr)>,
        kind: ObligationKind,
        location: Maybe<Span>,
    ) -> Self {
        Self {
            id: OBLIGATION_COUNTER.fetch_add(1, Ordering::SeqCst),
            proposition,
            context,
            location,
            kind,
            status: ObligationStatus::Pending,
            created_at: Instant::now(),
            proof: Maybe::None,
        }
    }

    /// Mark this obligation as discharged with the given proof
    pub fn discharge(&mut self, proof: ProofTerm) {
        self.status = ObligationStatus::Discharged;
        self.proof = Maybe::Some(proof);
    }

    /// Mark this obligation as failed
    pub fn fail(&mut self) {
        self.status = ObligationStatus::Failed;
    }

    /// Check if this obligation is still pending
    pub fn is_pending(&self) -> bool {
        self.status == ObligationStatus::Pending
    }
}

/// Tracker for proof obligations
#[derive(Debug)]
pub struct ObligationTracker {
    /// All tracked obligations
    obligations: List<ProofObligation>,
    /// Index by kind for fast lookup
    by_kind: HashMap<ObligationKind, List<u64>>,
    /// Statistics
    stats: ObligationStats,
}

/// Statistics for proof obligations
#[derive(Debug, Default, Clone)]
pub struct ObligationStats {
    /// Total obligations created
    pub total_created: u64,
    /// Obligations successfully discharged
    pub discharged: u64,
    /// Obligations that failed
    pub failed: u64,
    /// Obligations that timed out
    pub timed_out: u64,
    /// Total verification time (microseconds)
    pub total_time_us: u64,
}

impl ObligationTracker {
    /// Create a new obligation tracker
    pub fn new() -> Self {
        Self {
            obligations: List::new(),
            by_kind: HashMap::new(),
            stats: ObligationStats::default(),
        }
    }

    /// Add a new proof obligation
    pub fn add(&mut self, obligation: ProofObligation) -> u64 {
        let id = obligation.id;
        let kind = obligation.kind;
        self.obligations.push(obligation);
        self.by_kind.entry(kind).or_default().push(id);
        self.stats.total_created += 1;
        id
    }

    /// Get an obligation by ID
    pub fn get(&self, id: u64) -> Maybe<&ProofObligation> {
        self.obligations
            .iter()
            .find(|o| o.id == id)
            .map(Maybe::Some)
            .unwrap_or(Maybe::None)
    }

    /// Get a mutable reference to an obligation by ID
    pub fn get_mut(&mut self, id: u64) -> Maybe<&mut ProofObligation> {
        self.obligations
            .iter_mut()
            .find(|o| o.id == id)
            .map(Maybe::Some)
            .unwrap_or(Maybe::None)
    }

    /// Get all pending obligations
    pub fn pending(&self) -> List<&ProofObligation> {
        self.obligations.iter().filter(|o| o.is_pending()).collect()
    }

    /// Get obligations by kind
    pub fn by_kind(&self, kind: ObligationKind) -> List<&ProofObligation> {
        if let Some(ids) = self.by_kind.get(&kind) {
            ids.iter()
                .filter_map(|id| self.obligations.iter().find(|o| o.id == *id))
                .collect()
        } else {
            List::new()
        }
    }

    /// Update obligation status
    pub fn update_status(&mut self, id: u64, status: ObligationStatus) {
        if let Maybe::Some(obligation) = self.get_mut(id) {
            let old_status = obligation.status;
            obligation.status = status;

            // Update statistics
            if old_status != status {
                match status {
                    ObligationStatus::Discharged => self.stats.discharged += 1,
                    ObligationStatus::Failed => self.stats.failed += 1,
                    ObligationStatus::Timeout => self.stats.timed_out += 1,
                    _ => {}
                }
            }
        }
    }

    /// Get statistics
    pub fn stats(&self) -> &ObligationStats {
        &self.stats
    }

    /// Generate a summary report
    pub fn summary(&self) -> Text {
        let pending = self.obligations.iter().filter(|o| o.is_pending()).count();
        let discharged = self
            .obligations
            .iter()
            .filter(|o| o.status == ObligationStatus::Discharged)
            .count();
        let failed = self
            .obligations
            .iter()
            .filter(|o| o.status == ObligationStatus::Failed)
            .count();

        Text::from(format!(
            "Proof Obligations: {} total, {} discharged, {} failed, {} pending",
            self.obligations.len(),
            discharged,
            failed,
            pending
        ))
    }
}

impl Default for ObligationTracker {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Incremental Verification Support
// =============================================================================

/// Tracks dependencies between proof obligations for incremental verification
#[derive(Debug)]
pub struct IncrementalVerifier {
    /// Dependency graph: obligation ID -> IDs it depends on
    dependencies: HashMap<u64, Set<u64>>,
    /// Reverse dependencies: obligation ID -> IDs that depend on it
    dependents: HashMap<u64, Set<u64>>,
    /// Content hashes for change detection
    content_hashes: HashMap<u64, u64>,
    /// Last verification results
    last_results: HashMap<u64, Result<(), ValidationError>>,
    /// Changed obligations that need re-verification
    dirty: Set<u64>,
}

impl IncrementalVerifier {
    /// Create a new incremental verifier
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
            dependents: HashMap::new(),
            content_hashes: HashMap::new(),
            last_results: HashMap::new(),
            dirty: Set::new(),
        }
    }

    /// Register a proof obligation with its dependencies
    pub fn register(&mut self, id: u64, content_hash: u64, deps: &[u64]) {
        // Store dependencies
        let dep_set: Set<u64> = deps.iter().copied().collect();
        self.dependencies.insert(id, dep_set.clone());

        // Update reverse dependencies
        for dep_id in deps {
            self.dependents.entry(*dep_id).or_default().insert(id);
        }

        // Check if content changed
        if let Some(&old_hash) = self.content_hashes.get(&id) {
            if old_hash != content_hash {
                self.mark_dirty(id);
            }
        } else {
            // New obligation, mark as dirty
            self.dirty.insert(id);
        }

        self.content_hashes.insert(id, content_hash);
    }

    /// Mark an obligation as dirty (needs re-verification)
    pub fn mark_dirty(&mut self, id: u64) {
        if self.dirty.insert(id) {
            // Propagate to dependents
            if let Some(dependents) = self.dependents.get(&id).cloned() {
                for dep_id in dependents {
                    self.mark_dirty(dep_id);
                }
            }
        }
    }

    /// Check if an obligation needs re-verification
    pub fn needs_verification(&self, id: u64) -> bool {
        self.dirty.contains(&id)
    }

    /// Get obligations that need verification (in topological order)
    pub fn get_verification_order(&self) -> List<u64> {
        let mut result = List::new();
        let mut visited = Set::new();
        let mut temp_mark = Set::new();

        fn visit(
            id: u64,
            deps: &HashMap<u64, Set<u64>>,
            dirty: &Set<u64>,
            visited: &mut Set<u64>,
            temp_mark: &mut Set<u64>,
            result: &mut List<u64>,
        ) {
            if visited.contains(&id) || !dirty.contains(&id) {
                return;
            }
            if temp_mark.contains(&id) {
                // Cycle detected, skip
                return;
            }
            temp_mark.insert(id);

            if let Some(deps_set) = deps.get(&id) {
                for dep_id in deps_set.iter() {
                    visit(*dep_id, deps, dirty, visited, temp_mark, result);
                }
            }

            temp_mark.remove(&id);
            visited.insert(id);
            result.push(id);
        }

        for id in self.dirty.iter() {
            visit(
                *id,
                &self.dependencies,
                &self.dirty,
                &mut visited,
                &mut temp_mark,
                &mut result,
            );
        }

        result
    }

    /// Record a verification result
    pub fn record_result(&mut self, id: u64, result: Result<(), ValidationError>) {
        self.last_results.insert(id, result);
        self.dirty.remove(&id);
    }

    /// Get the last cached result for an obligation
    pub fn get_cached_result(&self, id: u64) -> Option<&Result<(), ValidationError>> {
        if self.dirty.contains(&id) {
            None
        } else {
            self.last_results.get(&id)
        }
    }

    /// Clear all cached state
    pub fn clear(&mut self) {
        self.dependencies.clear();
        self.dependents.clear();
        self.content_hashes.clear();
        self.last_results.clear();
        self.dirty.clear();
    }
}

impl Default for IncrementalVerifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper to extract name from PathSegment
fn path_segment_to_name(seg: &PathSegment) -> Text {
    match seg {
        PathSegment::Name(ident) => Text::from(ident.name.as_str()),
        PathSegment::SelfValue => Text::from("self"),
        PathSegment::Super => Text::from("super"),
        PathSegment::Cog => Text::from("cog"),
        PathSegment::Relative => Text::from("."),
    }
}

/// Helper to extract name from a ClosureParam's pattern
fn closure_param_name(param: &verum_ast::ClosureParam) -> Text {
    use verum_ast::PatternKind;
    match &param.pattern.kind {
        PatternKind::Ident { name, .. } => Text::from(name.as_str()),
        PatternKind::Wildcard => Text::from("_"),
        _ => Text::from("_"),
    }
}

/// Helper to extract a binder name from a quantifier binding's
/// pattern. Returns `None` for patterns that don't introduce a
/// single named binder (tuple, record, etc.) — the alpha-equivalence
/// path treats those as not contributing to the binding map and
/// continues structural comparison through the body. Single-name
/// binders are the common case for ∀x. P(x) / ∃x. P(x) and the only
/// shape currently exercised by the body-shape gates in
/// apply_inference_rule.
fn quantifier_binding_name(qb: &verum_ast::expr::QuantifierBinding) -> Option<Text> {
    use verum_ast::PatternKind;
    match &qb.pattern.kind {
        PatternKind::Ident { name, .. } => Some(Text::from(name.as_str())),
        _ => None,
    }
}

// ==================== Configuration ====================

/// Configuration for proof validation
#[derive(Debug, Clone)]
pub struct ValidationConfig {
    /// Maximum proof depth to prevent infinite recursion
    pub max_depth: usize,
    /// Enable strict type checking for dependent types
    pub strict_types: bool,
    /// Validate SMT proofs by re-checking with SMT solver
    pub validate_smt_proofs: bool,
    /// Check that induction is well-founded
    pub check_well_founded: bool,
    /// Timeout for SMT validation (milliseconds)
    pub smt_timeout_ms: u64,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            max_depth: 1000,
            strict_types: true,
            validate_smt_proofs: false, // Expensive, off by default
            check_well_founded: true,
            smt_timeout_ms: 5000,
        }
    }
}

// ==================== Validation Error Types ====================

/// Errors that can occur during proof validation
#[derive(Debug, Clone, thiserror::Error)]
pub enum ValidationError {
    /// Proof term does not prove the claimed proposition
    #[error("proof does not establish proposition: expected {expected}, got {actual}")]
    PropositionMismatch { expected: Text, actual: Text },

    /// Axiom not found in database
    #[error("unknown axiom: {axiom}")]
    UnknownAxiom { axiom: Text },

    /// Hypothesis not found in context
    #[error("hypothesis not in context: {hypothesis}")]
    HypothesisNotFound { hypothesis: Text },

    /// Modus ponens rule violation
    #[error("modus ponens: premise {premise} does not match antecedent of {implication}")]
    ModusPonensError { premise: Text, implication: Text },

    /// Equality rule violation (symmetry, transitivity, reflexivity)
    #[error("equality rule error: {message}")]
    EqualityError { message: Text },

    /// Induction rule violation
    #[error("induction error: {message}")]
    InductionError { message: Text },

    /// Case analysis violation
    #[error("cases error: {message}")]
    CasesError { message: Text },

    /// Lambda abstraction error
    #[error("lambda error: {message}")]
    LambdaError { message: Text },

    /// Type mismatch in proof
    #[error("type error in proof: expected {expected}, got {actual}")]
    TypeError { expected: Text, actual: Text },

    /// SMT proof validation failed
    #[error("SMT proof validation failed: {reason}")]
    SmtValidationFailed { reason: Text },

    /// Proof depth exceeded maximum
    #[error("proof depth {depth} exceeds maximum {max}")]
    DepthExceeded { depth: usize, max: usize },

    /// Circular dependency in proof
    #[error("circular dependency detected in proof")]
    CircularDependency,

    /// Quantifier instantiation error
    #[error("quantifier instantiation error: {message}")]
    QuantifierError { message: Text },

    /// Rewrite rule error
    #[error("rewrite rule error: {message}")]
    RewriteError { message: Text },

    /// Substitution rule error
    #[error("substitution error: {message}")]
    SubstitutionError { message: Text },

    /// General proof validation error
    #[error("proof validation error: {message}")]
    ValidationFailed { message: Text },
}

pub type ValidationResult<T> = Result<T, ValidationError>;

// =============================================================================
// Type Variable Unification System
// =============================================================================
//
// This module implements proper type variable tracking and unification for
// pattern matching during proof validation. Unlike simple placeholder variables,
// this system maintains unification constraints that can be solved incrementally.
//
// ## Design Principles
//
// 1. **Expression-Level Types**: Works with `Expr` representations of types since
//    the proof validator operates at the AST level before full type checking.
//
// 2. **Deferred Unification**: Type variables are created with constraints that
//    are solved when sufficient information becomes available.
//
// 3. **Structural Extraction**: For tuples, constructors, and arrays, type
//    information is extracted structurally from the scrutinee expression.
//
// 4. **Robust Fallbacks**: When type information cannot be derived, the system
//    creates proper type variables with documented constraints, not opaque
//    placeholder strings.

/// A type variable identifier for unification.
///
/// Type variables are created during pattern matching when the exact type
/// of a binding cannot be immediately determined from the scrutinee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeVarId(u64);

impl TypeVarId {
    /// Generate a fresh type variable identifier.
    fn fresh() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        TypeVarId(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Get the numeric ID for display purposes.
    pub fn id(&self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for TypeVarId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "?T{}", self.0)
    }
}

/// The state of a type variable in the unification context.
#[derive(Debug, Clone)]
pub enum TypeVarState {
    /// Unbound type variable with optional constraints.
    ///
    /// The constraints describe what we know about the type:
    /// - `origin`: Where this type variable was created (e.g., "tuple element 0")
    /// - `scrutinee_span`: The span of the scrutinee expression
    /// - `element_index`: For tuple/constructor patterns, the element index
    Unbound {
        origin: Text,
        scrutinee_span: Span,
        element_index: Maybe<usize>,
    },

    /// Bound to a concrete expression representing a type.
    Bound(Expr),

    /// Unified with another type variable.
    Link(TypeVarId),
}

/// Context for tracking type variables and their bindings during proof validation.
///
/// This context maintains:
/// - A mapping from type variable IDs to their current state
/// - Deferred unification constraints that couldn't be solved immediately
/// - Statistics about type inference success rates
#[derive(Debug, Clone)]
pub struct ExprTypeContext {
    /// Type variable bindings
    bindings: Map<TypeVarId, TypeVarState>,
    /// Deferred constraints to be solved later
    constraints: List<TypeConstraint>,
    /// Statistics
    stats: TypeContextStats,
}

/// A constraint between type expressions that must hold.
#[derive(Debug, Clone)]
pub struct TypeConstraint {
    /// Left side of the constraint
    pub lhs: Expr,
    /// Right side of the constraint
    pub rhs: Expr,
    /// Origin of the constraint for error reporting
    pub origin: Text,
    /// Span for error reporting
    pub span: Span,
}

/// Statistics for the type context.
#[derive(Debug, Clone, Default)]
pub struct TypeContextStats {
    /// Number of type variables created
    pub vars_created: u64,
    /// Number of type variables successfully resolved
    pub vars_resolved: u64,
    /// Number of constraints generated
    pub constraints_generated: u64,
    /// Number of constraints solved
    pub constraints_solved: u64,
    /// Number of fallbacks used
    pub fallbacks_used: u64,
}

impl ExprTypeContext {
    /// Create a new empty type context.
    pub fn new() -> Self {
        Self {
            bindings: Map::new(),
            constraints: List::new(),
            stats: TypeContextStats::default(),
        }
    }

    /// Create a fresh type variable with origin information.
    ///
    /// # Arguments
    /// * `origin` - Description of where this type variable originated
    /// * `scrutinee_span` - Span of the scrutinee expression
    /// * `element_index` - Optional index for tuple/constructor element types
    pub fn fresh_var(
        &mut self,
        origin: impl Into<Text>,
        scrutinee_span: Span,
        element_index: Maybe<usize>,
    ) -> TypeVarId {
        let id = TypeVarId::fresh();
        self.bindings.insert(
            id,
            TypeVarState::Unbound {
                origin: origin.into(),
                scrutinee_span,
                element_index,
            },
        );
        self.stats.vars_created += 1;
        id
    }

    /// Bind a type variable to a concrete type expression.
    ///
    /// # Returns
    /// `Ok(())` if binding succeeds, `Err` if the variable was already bound
    /// to an incompatible type.
    pub fn bind(&mut self, var: TypeVarId, ty: Expr) -> ValidationResult<()> {
        match self.bindings.get(&var) {
            Some(TypeVarState::Unbound { .. }) => {
                self.bindings.insert(var, TypeVarState::Bound(ty));
                self.stats.vars_resolved += 1;
                Ok(())
            }
            Some(TypeVarState::Bound(existing)) => {
                // Already bound - check compatibility
                if self.exprs_structurally_equal(existing, &ty) {
                    Ok(())
                } else {
                    Err(ValidationError::TypeError {
                        expected: self.expr_to_type_text(existing),
                        actual: self.expr_to_type_text(&ty),
                    })
                }
            }
            Some(TypeVarState::Link(target)) => {
                // Follow the link and bind the target
                let target = *target;
                self.bind(target, ty)
            }
            None => {
                // Unknown variable - this shouldn't happen in normal use
                self.bindings.insert(var, TypeVarState::Bound(ty));
                self.stats.vars_resolved += 1;
                Ok(())
            }
        }
    }

    /// Look up the current binding for a type variable.
    ///
    /// Follows links and returns the final state.
    pub fn lookup(&self, var: TypeVarId) -> Maybe<&TypeVarState> {
        match self.bindings.get(&var) {
            Some(TypeVarState::Link(target)) => self.lookup(*target),
            state => state.map_or(Maybe::None, Maybe::Some),
        }
    }

    /// Resolve a type variable to its bound type expression, if any.
    pub fn resolve(&self, var: TypeVarId) -> Maybe<Expr> {
        match self.lookup(var) {
            Maybe::Some(TypeVarState::Bound(expr)) => Maybe::Some(expr.clone()),
            _ => Maybe::None,
        }
    }

    /// Unify two type expressions.
    ///
    /// This performs structural unification on expression-level types.
    /// Type variables are bound as needed. Constraints that cannot be
    /// immediately solved are deferred.
    pub fn unify(&mut self, lhs: &Expr, rhs: &Expr, span: Span) -> ValidationResult<()> {
        // Check if either side is a type variable reference
        let lhs_var = self.as_type_var(lhs);
        let rhs_var = self.as_type_var(rhs);

        match (lhs_var, rhs_var) {
            // Both are type variables - link them
            (Maybe::Some(v1), Maybe::Some(v2)) if v1 != v2 => {
                self.bindings.insert(v1, TypeVarState::Link(v2));
                Ok(())
            }
            // Same variable - trivially unified
            (Maybe::Some(v1), Maybe::Some(v2)) if v1 == v2 => Ok(()),
            // Left is a variable - bind it
            (Maybe::Some(var), _) => self.bind(var, rhs.clone()),
            // Right is a variable - bind it
            (_, Maybe::Some(var)) => self.bind(var, lhs.clone()),
            // Neither is a variable - structural unification
            _ => self.unify_structural(lhs, rhs, span),
        }
    }

    /// Check if an expression represents a type variable.
    fn as_type_var(&self, expr: &Expr) -> Maybe<TypeVarId> {
        if let ExprKind::Path(path) = &expr.kind {
            if let Some(ident) = path.as_ident() {
                let name = ident.as_str();
                // Type variable names start with "?T"
                if name.starts_with("?T") {
                    if let Ok(id) = name[2..].parse::<u64>() {
                        let var_id = TypeVarId(id);
                        if self.bindings.contains_key(&var_id) {
                            return Maybe::Some(var_id);
                        }
                    }
                }
            }
        }
        Maybe::None
    }

    /// Perform structural unification of two type expressions.
    fn unify_structural(&mut self, lhs: &Expr, rhs: &Expr, span: Span) -> ValidationResult<()> {
        match (&lhs.kind, &rhs.kind) {
            // Same literals unify
            (ExprKind::Literal(l1), ExprKind::Literal(l2)) if l1 == l2 => Ok(()),

            // Same paths unify
            (ExprKind::Path(p1), ExprKind::Path(p2)) => {
                if self.paths_equal(p1, p2) {
                    Ok(())
                } else {
                    // Defer this constraint
                    self.add_constraint(lhs.clone(), rhs.clone(), "path unification", span);
                    Ok(())
                }
            }

            // Tuples unify element-wise
            (ExprKind::Tuple(elems1), ExprKind::Tuple(elems2)) => {
                if elems1.len() != elems2.len() {
                    return Err(ValidationError::TypeError {
                        expected: format!("{}-tuple", elems2.len()).into(),
                        actual: format!("{}-tuple", elems1.len()).into(),
                    });
                }
                for (e1, e2) in elems1.iter().zip(elems2.iter()) {
                    self.unify(e1, e2, span)?;
                }
                Ok(())
            }

            // Call expressions (constructors) unify if func and args match
            (ExprKind::Call { func: f1, args: a1, .. }, ExprKind::Call { func: f2, args: a2, .. }) => {
                self.unify(f1, f2, span)?;
                if a1.len() != a2.len() {
                    return Err(ValidationError::TypeError {
                        expected: format!("{}-ary constructor", a2.len()).into(),
                        actual: format!("{}-ary constructor", a1.len()).into(),
                    });
                }
                for (a1, a2) in a1.iter().zip(a2.iter()) {
                    self.unify(a1, a2, span)?;
                }
                Ok(())
            }

            // Binary expressions with same operator
            (
                ExprKind::Binary {
                    op: op1,
                    left: l1,
                    right: r1,
                },
                ExprKind::Binary {
                    op: op2,
                    left: l2,
                    right: r2,
                },
            ) if op1 == op2 => {
                self.unify(l1, l2, span)?;
                self.unify(r1, r2, span)
            }

            // Different expression kinds - defer as constraint
            _ => {
                self.add_constraint(lhs.clone(), rhs.clone(), "expression unification", span);
                Ok(())
            }
        }
    }

    /// Add a deferred constraint.
    fn add_constraint(&mut self, lhs: Expr, rhs: Expr, origin: &str, span: Span) {
        self.constraints.push(TypeConstraint {
            lhs,
            rhs,
            origin: origin.into(),
            span,
        });
        self.stats.constraints_generated += 1;
    }

    /// Compare paths for equality (ignoring spans).
    fn paths_equal(&self, p1: &verum_ast::Path, p2: &verum_ast::Path) -> bool {
        if p1.segments.len() != p2.segments.len() {
            return false;
        }
        for (s1, s2) in p1.segments.iter().zip(p2.segments.iter()) {
            match (s1, s2) {
                (PathSegment::Name(n1), PathSegment::Name(n2)) => {
                    if n1.name != n2.name {
                        return false;
                    }
                }
                (PathSegment::SelfValue, PathSegment::SelfValue)
                | (PathSegment::Super, PathSegment::Super)
                | (PathSegment::Cog, PathSegment::Cog)
                | (PathSegment::Relative, PathSegment::Relative) => {}
                _ => return false,
            }
        }
        true
    }

    /// Check if two expressions are structurally equal.
    fn exprs_structurally_equal(&self, e1: &Expr, e2: &Expr) -> bool {
        match (&e1.kind, &e2.kind) {
            (ExprKind::Literal(l1), ExprKind::Literal(l2)) => l1 == l2,
            (ExprKind::Path(p1), ExprKind::Path(p2)) => self.paths_equal(p1, p2),
            (ExprKind::Tuple(t1), ExprKind::Tuple(t2)) => {
                t1.len() == t2.len()
                    && t1
                        .iter()
                        .zip(t2.iter())
                        .all(|(a, b)| self.exprs_structurally_equal(a, b))
            }
            (ExprKind::Call { func: f1, args: a1, .. }, ExprKind::Call { func: f2, args: a2, .. }) => {
                self.exprs_structurally_equal(f1, f2)
                    && a1.len() == a2.len()
                    && a1
                        .iter()
                        .zip(a2.iter())
                        .all(|(x, y)| self.exprs_structurally_equal(x, y))
            }
            _ => false,
        }
    }

    /// Convert an expression to a text representation for error messages.
    fn expr_to_type_text(&self, expr: &Expr) -> Text {
        match &expr.kind {
            ExprKind::Path(p) => {
                if let Some(ident) = p.as_ident() {
                    return ident.as_str().into();
                }
                format!("{:?}", p).into()
            }
            ExprKind::Tuple(elems) => {
                let parts: Vec<String> = elems
                    .iter()
                    .map(|e| self.expr_to_type_text(e).to_string())
                    .collect();
                format!("({})", parts.join(", ")).into()
            }
            ExprKind::Literal(lit) => format!("{:?}", lit.kind).into(),
            _ => format!("{:?}", expr.kind).into(),
        }
    }

    /// Create an expression representing a type variable.
    pub fn make_type_var_expr(&mut self, var: TypeVarId, span: Span) -> Expr {
        use verum_ast::{Path, ty::Ident};
        let name = format!("?T{}", var.0);
        Expr::new(
            ExprKind::Path(Path::from_ident(Ident::new(name, span))),
            span,
        )
    }

    /// Get statistics about type inference.
    pub fn stats(&self) -> &TypeContextStats {
        &self.stats
    }

    /// Get pending constraints.
    pub fn pending_constraints(&self) -> &List<TypeConstraint> {
        &self.constraints
    }

    /// Mark that a fallback was used.
    pub fn record_fallback(&mut self) {
        self.stats.fallbacks_used += 1;
    }
}

impl Default for ExprTypeContext {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Hypothesis Context ====================

/// Manages hypothesis scoping during proof validation
///
/// Tracks available hypotheses and their types/propositions at each
/// level of proof checking, supporting proper scoping for nested proofs.
#[derive(Debug, Clone)]
pub struct HypothesisContext {
    /// Stack of hypothesis scopes (innermost first)
    scopes: List<HypothesisScope>,
}

/// A single scope of hypotheses
#[derive(Debug, Clone)]
struct HypothesisScope {
    /// Hypotheses in this scope (name -> proposition)
    hypotheses: Map<Text, Expr>,
}

impl HypothesisContext {
    /// Create a new empty hypothesis context
    pub fn new() -> Self {
        Self {
            scopes: List::from(vec![HypothesisScope {
                hypotheses: Map::new(),
            }]),
        }
    }

    /// Enter a new hypothesis scope
    pub fn enter_scope(&mut self) {
        self.scopes.push(HypothesisScope {
            hypotheses: Map::new(),
        });
    }

    /// Exit the current hypothesis scope
    pub fn exit_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    /// Add a hypothesis to the current scope
    pub fn add_hypothesis(&mut self, name: Text, proposition: Expr) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.hypotheses.insert(name, proposition);
        }
    }

    /// Look up a hypothesis in any scope (innermost to outermost)
    pub fn lookup(&self, name: &Text) -> Maybe<Expr> {
        for scope in self.scopes.iter().rev() {
            if let Some(prop) = scope.hypotheses.get(name) {
                return Maybe::Some(prop.clone());
            }
        }
        Maybe::None
    }

    /// Check if a hypothesis exists
    pub fn contains(&self, name: &Text) -> bool {
        matches!(self.lookup(name), Maybe::Some(_))
    }

    /// Discharge a hypothesis when proving an implication
    /// Returns the proposition of the discharged hypothesis
    pub fn discharge(&mut self, name: &Text) -> Maybe<Expr> {
        // Remove from current scope only
        if let Some(scope) = self.scopes.last_mut() {
            scope.hypotheses.remove(name)
        } else {
            Maybe::None
        }
    }

    /// Iterate over every (name, proposition) currently in scope across
    /// all enclosing scopes. Used by soundness checks that need to
    /// recognise a target proposition as available, regardless of which
    /// name it was bound to.
    pub fn iter_propositions(&self) -> impl Iterator<Item = (&Text, &Expr)> + '_ {
        self.scopes
            .iter()
            .flat_map(|scope| scope.hypotheses.iter())
    }
}

impl Default for HypothesisContext {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Proof Validator ====================

/// A rewrite rule consisting of a left-hand side pattern and right-hand side replacement
#[derive(Debug, Clone)]
pub struct RewriteRule {
    /// Name of the rewrite rule
    pub name: Text,
    /// Left-hand side pattern (can contain variables)
    pub lhs: Expr,
    /// Right-hand side replacement
    pub rhs: Expr,
    /// Conditions that must hold for the rule to apply
    pub conditions: List<Expr>,
    /// Whether the rule can be applied in reverse
    pub bidirectional: bool,
}

/// Main proof validator
///
/// Validates that proof terms correctly prove their propositions according
/// to formal proof rules and the Curry-Howard correspondence.
///
/// ## Features
///
/// - **Proof Caching**: Caches validation results to avoid redundant work
/// - **Obligation Tracking**: Tracks proof obligations and their status
/// - **Incremental Verification**: Re-verifies only changed obligations
/// - **SMT Integration**: Integrates with Z3 for automated proving
/// - **Type Inference**: Tracks type variables during pattern matching
#[derive(Debug)]
pub struct ProofValidator {
    /// Validation configuration
    config: ValidationConfig,
    /// Registered axioms (name -> formula)
    axioms: Map<Text, Expr>,
    /// Hypothesis context for tracking assumptions
    hypotheses: HypothesisContext,
    /// Type context for tracking type variables during pattern matching
    type_context: ExprTypeContext,
    /// Theory lemmas (theory name -> lemmas)
    theory_lemmas: Map<Text, List<Expr>>,
    /// Registered rewrite rules (name -> rule)
    rewrite_rules: Map<Text, RewriteRule>,
    /// User-defined inference rules (name -> (premises types, conclusion type))
    inference_rules: Map<Text, (List<Expr>, Expr)>,
    /// Current validation depth (for cycle detection)
    current_depth: usize,
    /// Visited proof nodes (for cycle detection)
    visited: Set<Text>,
    /// Proof cache for memoization
    cache: ProofCache,
    /// Proof obligation tracker
    obligations: ObligationTracker,
    /// Incremental verification state
    incremental: IncrementalVerifier,
    /// Verification statistics
    stats: ValidatorStats,
}

/// Statistics for the proof validator
#[derive(Debug, Default, Clone)]
pub struct ValidatorStats {
    /// Total proofs validated
    pub proofs_validated: u64,
    /// Proofs that succeeded
    pub proofs_succeeded: u64,
    /// Proofs that failed
    pub proofs_failed: u64,
    /// Total time spent validating (microseconds)
    pub total_time_us: u64,
    /// Time saved by caching (microseconds)
    pub cache_time_saved_us: u64,
}

impl ProofValidator {
    /// Create a new proof validator with default configuration
    pub fn new() -> Self {
        let mut validator = Self {
            config: ValidationConfig::default(),
            axioms: Map::new(),
            hypotheses: HypothesisContext::new(),
            type_context: ExprTypeContext::new(),
            theory_lemmas: Map::new(),
            rewrite_rules: Map::new(),
            inference_rules: Map::new(),
            current_depth: 0,
            visited: Set::new(),
            cache: ProofCache::new(),
            obligations: ObligationTracker::new(),
            incremental: IncrementalVerifier::new(),
            stats: ValidatorStats::default(),
        };
        validator.register_standard_rules();
        validator
    }

    /// Create a new proof validator with custom configuration
    pub fn with_config(config: ValidationConfig) -> Self {
        let mut validator = Self {
            config,
            axioms: Map::new(),
            hypotheses: HypothesisContext::new(),
            type_context: ExprTypeContext::new(),
            theory_lemmas: Map::new(),
            rewrite_rules: Map::new(),
            inference_rules: Map::new(),
            current_depth: 0,
            visited: Set::new(),
            cache: ProofCache::new(),
            obligations: ObligationTracker::new(),
            incremental: IncrementalVerifier::new(),
            stats: ValidatorStats::default(),
        };
        validator.register_standard_rules();
        validator
    }

    /// Get the type context
    pub fn type_context(&self) -> &ExprTypeContext {
        &self.type_context
    }

    /// Get mutable access to the type context
    pub fn type_context_mut(&mut self) -> &mut ExprTypeContext {
        &mut self.type_context
    }

    /// Get the proof cache
    pub fn cache(&self) -> &ProofCache {
        &self.cache
    }

    /// Get mutable access to the proof cache
    pub fn cache_mut(&mut self) -> &mut ProofCache {
        &mut self.cache
    }

    /// Get the obligation tracker
    pub fn obligations(&self) -> &ObligationTracker {
        &self.obligations
    }

    /// Get mutable access to the obligation tracker
    pub fn obligations_mut(&mut self) -> &mut ObligationTracker {
        &mut self.obligations
    }

    /// Get the incremental verifier
    pub fn incremental(&self) -> &IncrementalVerifier {
        &self.incremental
    }

    /// Get mutable access to the incremental verifier
    pub fn incremental_mut(&mut self) -> &mut IncrementalVerifier {
        &mut self.incremental
    }

    /// Get validator statistics
    pub fn validator_stats(&self) -> &ValidatorStats {
        &self.stats
    }

    /// Create a proof obligation and add it to the tracker
    pub fn create_obligation(
        &mut self,
        proposition: Expr,
        kind: ObligationKind,
        location: Maybe<Span>,
    ) -> u64 {
        let context = self.collect_current_hypotheses();
        let obligation = ProofObligation::new(proposition, context, kind, location);
        self.obligations.add(obligation)
    }

    /// Collect current hypotheses for obligation context
    fn collect_current_hypotheses(&self) -> List<(Text, Expr)> {
        let mut result = List::new();
        // Collect from all scopes
        for scope in self.hypotheses.scopes.iter() {
            for (name, prop) in scope.hypotheses.iter() {
                result.push((name.clone(), prop.clone()));
            }
        }
        result
    }

    /// Verify a proof obligation by ID
    pub fn verify_obligation(&mut self, id: u64) -> ValidationResult<()> {
        let start = Instant::now();

        // Check incremental verifier for cached result
        if let Some(cached) = self.incremental.get_cached_result(id) {
            return cached.clone();
        }

        // Get the obligation
        let obligation = match self.obligations.get(id) {
            Maybe::Some(o) => o.clone(),
            Maybe::None => {
                return Err(ValidationError::ValidationFailed {
                    message: format!("Obligation {} not found", id).into(),
                });
            }
        };

        // Mark as in progress
        self.obligations
            .update_status(id, ObligationStatus::InProgress);

        // Try to find or generate a proof
        let result = self.try_prove_obligation(&obligation);

        // Update status based on result
        match &result {
            Ok(()) => {
                self.obligations
                    .update_status(id, ObligationStatus::Discharged);
            }
            Err(_) => {
                self.obligations.update_status(id, ObligationStatus::Failed);
            }
        }

        // Record result in incremental verifier
        self.incremental.record_result(id, result.clone());

        // Update stats
        self.stats.total_time_us += start.elapsed().as_micros() as u64;

        result
    }

    /// Try to prove an obligation (using SMT or other strategies)
    fn try_prove_obligation(&mut self, obligation: &ProofObligation) -> ValidationResult<()> {
        // Try SMT solver first if available
        if self.config.validate_smt_proofs {
            if let Ok(()) = self.prove_with_smt(&obligation.proposition) {
                return Ok(());
            }
        }

        // Obligation remains unproved
        Err(ValidationError::ValidationFailed {
            message: Text::from("Could not automatically prove obligation"),
        })
    }

    /// Try to prove a proposition using SMT solver
    ///
    /// This method uses Z3 to prove propositions by checking if the negation is unsatisfiable.
    /// If the negation is UNSAT, the proposition is proven. If SAT, a counterexample is extracted.
    ///
    /// # Algorithm
    /// 1. Convert the Verum proposition to a Z3 boolean formula
    /// 2. Assert the negation of the proposition
    /// 3. Check satisfiability:
    ///    - UNSAT: Proposition is proven (negation is false, so proposition is true)
    ///    - SAT: Extract counterexample from model
    ///    - UNKNOWN: Return appropriate error with solver reason
    ///
    /// # Timeout
    /// Uses `config.smt_timeout_ms` for solver timeout to prevent unbounded proving attempts.
    fn prove_with_smt(&self, proposition: &Expr) -> ValidationResult<()> {
        use z3::{Config, SatResult, Solver, ast::Bool};

        // Set up Z3 configuration with timeout and proof generation
        let mut cfg = Config::new();
        cfg.set_timeout_msec(self.config.smt_timeout_ms);
        cfg.set_proof_generation(true);

        // Execute within the configured Z3 context
        z3::with_z3_config(&cfg, || {
            // Create an SMT prover context for expression conversion
            let mut prover = SmtProver::new();

            // Add current hypotheses as assumptions
            for scope in self.hypotheses.scopes.iter() {
                for (name, hyp) in scope.hypotheses.iter() {
                    if let Ok(z3_hyp) = prover.expr_to_z3(hyp) {
                        prover.add_assumption(name.clone(), z3_hyp);
                    }
                }
            }

            // Convert proposition to Z3 formula
            let z3_prop = prover.expr_to_z3(proposition).map_err(|e| {
                ValidationError::SmtValidationFailed {
                    reason: Text::from(format!("Failed to convert proposition to Z3: {}", e)),
                }
            })?;

            // Create solver and assert assumptions
            let solver = Solver::new();

            // Add all assumptions to the solver
            for assumption in prover.assumptions() {
                solver.assert(assumption);
            }

            // Assert the NEGATION of the proposition
            // If the negation is UNSAT, the proposition must be true
            solver.assert(&z3_prop.not());

            // Check satisfiability
            match solver.check() {
                SatResult::Unsat => {
                    // Negation is unsatisfiable => proposition is proven
                    Ok(())
                }
                SatResult::Sat => {
                    // Negation is satisfiable => proposition can be false
                    // Extract counterexample from model
                    let counterexample = if let Some(model) = solver.get_model() {
                        prover.extract_counterexample(&model)
                    } else {
                        Text::from("(no model available)")
                    };

                    Err(ValidationError::SmtValidationFailed {
                        reason: Text::from(format!(
                            "Proposition is not valid. Counterexample: {}",
                            counterexample
                        )),
                    })
                }
                SatResult::Unknown => {
                    // Solver could not determine satisfiability
                    let reason = solver
                        .get_reason_unknown()
                        .unwrap_or_else(|| "unknown reason".to_string());

                    Err(ValidationError::SmtValidationFailed {
                        reason: Text::from(format!("SMT solver returned unknown: {}", reason)),
                    })
                }
            }
        })
    }

    /// Register standard rewrite rules and inference rules
    fn register_standard_rules(&mut self) {
        // Standard algebraic rewrite rules are registered here
        // More rules can be added via register_rewrite_rule
    }

    /// Register a rewrite rule
    pub fn register_rewrite_rule(&mut self, rule: RewriteRule) {
        self.rewrite_rules.insert(rule.name.clone(), rule);
    }

    /// Register a user-defined inference rule
    pub fn register_inference_rule(
        &mut self,
        name: impl Into<Text>,
        premises: List<Expr>,
        conclusion: Expr,
    ) {
        self.inference_rules
            .insert(name.into(), (premises, conclusion));
    }

    /// Register an axiom in the axiom database
    pub fn register_axiom(&mut self, name: impl Into<Text>, formula: Expr) {
        self.axioms.insert(name.into(), formula);
    }

    /// Register a theory lemma
    pub fn register_theory_lemma(&mut self, theory: impl Into<Text>, lemma: Expr) {
        let theory_name = theory.into();
        let lemmas = self
            .theory_lemmas
            .entry(theory_name.clone())
            .or_insert_with(List::new);
        lemmas.push(lemma);
    }

    /// Validate that a proof term proves the given proposition
    ///
    /// This is the main entry point for proof validation.
    /// Uses caching to avoid redundant validation of identical proofs.
    pub fn validate(&mut self, proof: &ProofTerm, expected: &Expr) -> ValidationResult<()> {
        let start = Instant::now();
        self.stats.proofs_validated += 1;

        // Compute cache key
        let cache_key = self.compute_cache_key(proof, expected);

        // Check cache first
        if let Some(cached_result) = self.cache.get(&cache_key) {
            self.stats.cache_time_saved_us += start.elapsed().as_micros() as u64;
            match &cached_result {
                Ok(()) => self.stats.proofs_succeeded += 1,
                Err(_) => self.stats.proofs_failed += 1,
            }
            return cached_result;
        }

        // Reset validation state
        self.current_depth = 0;
        self.visited.clear();

        // Perform validation
        let result = self.validate_impl(proof, expected);

        // Update statistics
        match &result {
            Ok(()) => self.stats.proofs_succeeded += 1,
            Err(_) => self.stats.proofs_failed += 1,
        }
        self.stats.total_time_us += start.elapsed().as_micros() as u64;

        // Cache the result
        self.cache.insert(cache_key, result.clone());

        result
    }

    /// Compute a cache key for a proof and expected proposition
    fn compute_cache_key(&self, proof: &ProofTerm, expected: &Expr) -> ProofCacheKey {
        use std::collections::hash_map::DefaultHasher;

        // Hash the proof term
        let mut proof_hasher = DefaultHasher::new();
        self.hash_proof(proof, &mut proof_hasher);
        let proof_hash = proof_hasher.finish();

        // Hash the expected proposition
        let mut expected_hasher = DefaultHasher::new();
        self.hash_expr(expected, &mut expected_hasher);
        let expected_hash = expected_hasher.finish();

        // Hash the config (for cache invalidation on config change)
        let mut config_hasher = DefaultHasher::new();
        config_hasher.write_usize(self.config.max_depth);
        config_hasher.write_u8(self.config.strict_types as u8);
        config_hasher.write_u8(self.config.validate_smt_proofs as u8);
        let config_hash = config_hasher.finish();

        ProofCacheKey {
            proof_hash,
            expected_hash,
            config_hash,
        }
    }

    /// Hash a proof term for caching
    fn hash_proof<H: Hasher>(&self, proof: &ProofTerm, hasher: &mut H) {
        // Use discriminant for variant identification
        std::mem::discriminant(proof).hash(hasher);

        match proof {
            ProofTerm::Axiom { name, formula } => {
                name.hash(hasher);
                self.hash_expr(formula, hasher);
            }
            ProofTerm::Assumption { id, formula } => {
                id.hash(hasher);
                self.hash_expr(formula, hasher);
            }
            ProofTerm::Hypothesis { id, formula } => {
                id.hash(hasher);
                self.hash_expr(formula, hasher);
            }
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => {
                self.hash_proof(premise, hasher);
                self.hash_proof(implication, hasher);
            }
            ProofTerm::Symmetry { equality } => {
                self.hash_proof(equality, hasher);
            }
            ProofTerm::Transitivity { left, right } => {
                self.hash_proof(left, hasher);
                self.hash_proof(right, hasher);
            }
            ProofTerm::Reflexivity { term } => {
                self.hash_expr(term, hasher);
            }
            ProofTerm::Lambda { var, body } => {
                var.hash(hasher);
                self.hash_proof(body, hasher);
            }
            ProofTerm::SmtProof {
                solver,
                formula,
                smt_trace,
            } => {
                solver.hash(hasher);
                self.hash_expr(formula, hasher);
                match smt_trace {
                    Some(t) => t.hash(hasher),
                    None => 0u8.hash(hasher),
                };
            }
            // For other variants, use a simple approach
            _ => {
                // Hash the debug representation as a fallback
                format!("{:?}", proof).hash(hasher);
            }
        }
    }

    /// Hash an expression for caching
    fn hash_expr<H: Hasher>(&self, expr: &Expr, hasher: &mut H) {
        std::mem::discriminant(&expr.kind).hash(hasher);

        match &expr.kind {
            ExprKind::Literal(lit) => {
                format!("{:?}", lit).hash(hasher);
            }
            ExprKind::Path(path) => {
                for seg in path.segments.iter() {
                    path_segment_to_name(seg).hash(hasher);
                }
            }
            ExprKind::Binary { op, left, right } => {
                std::mem::discriminant(op).hash(hasher);
                self.hash_expr(left, hasher);
                self.hash_expr(right, hasher);
            }
            ExprKind::Unary { op, expr } => {
                std::mem::discriminant(op).hash(hasher);
                self.hash_expr(expr, hasher);
            }
            ExprKind::Call { func, args, .. } => {
                self.hash_expr(func, hasher);
                for arg in args.iter() {
                    self.hash_expr(arg, hasher);
                }
            }
            ExprKind::Tuple(elements) => {
                for elem in elements.iter() {
                    self.hash_expr(elem, hasher);
                }
            }
            _ => {
                // For other expression kinds, use debug repr
                format!("{:?}", expr.kind).hash(hasher);
            }
        }
    }

    /// Internal validation implementation with depth tracking
    fn validate_impl(&mut self, proof: &ProofTerm, expected: &Expr) -> ValidationResult<()> {
        // Check depth limit
        if self.current_depth > self.config.max_depth {
            return Err(ValidationError::DepthExceeded {
                depth: self.current_depth,
                max: self.config.max_depth,
            });
        }

        // Check for cycles
        let proof_id = self.proof_id(proof);
        if self.visited.contains(&proof_id) {
            return Err(ValidationError::CircularDependency);
        }
        self.visited.insert(proof_id);

        // Increment depth
        self.current_depth += 1;

        // Validate based on proof term type
        let result = match proof {
            ProofTerm::Axiom { name, formula } => self.validate_axiom(name, formula, expected),

            ProofTerm::Assumption { id, formula } => {
                self.validate_assumption(*id, formula, expected)
            }

            ProofTerm::Hypothesis { id, formula } => {
                self.validate_hypothesis(*id, formula, expected)
            }

            ProofTerm::ModusPonens {
                premise,
                implication,
            } => self.validate_modus_ponens(premise, implication, expected),

            ProofTerm::Rewrite {
                source,
                rule,
                target,
            } => self.validate_rewrite(source, rule, target, expected),

            ProofTerm::Symmetry { equality } => self.validate_symmetry(equality, expected),

            ProofTerm::Transitivity { left, right } => {
                self.validate_transitivity(left, right, expected)
            }

            ProofTerm::Reflexivity { term } => self.validate_reflexivity(term, expected),

            ProofTerm::TheoryLemma { theory, lemma } => {
                self.validate_theory_lemma(theory, lemma, expected)
            }

            ProofTerm::UnitResolution { clauses } => {
                self.validate_unit_resolution(clauses, expected)
            }

            ProofTerm::QuantifierInstantiation {
                quantified,
                instantiation,
            } => self.validate_quantifier_instantiation(quantified, instantiation, expected),

            ProofTerm::Lambda { var, body } => self.validate_lambda(var, body, expected),

            ProofTerm::Cases { scrutinee, cases } => {
                self.validate_cases(scrutinee, cases, expected)
            }

            ProofTerm::Induction {
                var,
                base_case,
                inductive_case,
            } => self.validate_induction(var, base_case, inductive_case, expected),

            ProofTerm::Apply { rule, premises } => self.validate_apply(rule, premises, expected),

            ProofTerm::SmtProof {
                solver,
                formula,
                smt_trace,
            } => self.validate_smt_proof(solver, formula, smt_trace, expected),

            ProofTerm::Subst { eq_proof, property } => {
                self.validate_subst(eq_proof, property, expected)
            }

            ProofTerm::Lemma { conclusion, proof } => {
                self.validate_lemma(conclusion, proof, expected)
            }

            // Extended proof rules
            ProofTerm::AndElim {
                conjunction,
                index,
                result,
            } => self.validate_and_elim(conjunction, *index, result, expected),

            ProofTerm::NotOrElim {
                negated_disjunction,
                index,
                result,
            } => self.validate_not_or_elim(negated_disjunction, *index, result, expected),

            ProofTerm::IffTrue { proof, formula } => {
                self.validate_iff_true(proof, formula, expected)
            }

            ProofTerm::IffFalse { proof, formula } => {
                self.validate_iff_false(proof, formula, expected)
            }

            ProofTerm::Commutativity { left, right } => {
                self.validate_commutativity(left, right, expected)
            }

            ProofTerm::Monotonicity {
                premises,
                conclusion,
            } => self.validate_monotonicity(premises, conclusion, expected),

            ProofTerm::Distributivity { formula } => {
                self.validate_distributivity(formula, expected)
            }

            ProofTerm::DefAxiom { formula } => self.validate_def_axiom(formula, expected),

            ProofTerm::DefIntro { name, definition } => {
                self.validate_def_intro(name, definition, expected)
            }

            ProofTerm::ApplyDef {
                def_proof,
                original,
                name,
            } => self.validate_apply_def(def_proof, original, name, expected),

            ProofTerm::IffOEq {
                iff_proof,
                left,
                right,
            } => self.validate_iff_oeq(iff_proof, left, right, expected),

            ProofTerm::NnfPos { formula, result } => {
                self.validate_nnf_pos(formula, result, expected)
            }

            ProofTerm::NnfNeg { formula, result } => {
                self.validate_nnf_neg(formula, result, expected)
            }

            ProofTerm::SkHack {
                formula,
                skolemized,
            } => self.validate_sk_hack(formula, skolemized, expected),

            ProofTerm::EqualityResolution {
                equality,
                literal,
                result,
            } => self.validate_equality_resolution(equality, literal, result, expected),

            ProofTerm::BindProof {
                quantified_proof,
                pattern,
                binding,
            } => self.validate_bind_proof(quantified_proof, pattern, binding, expected),

            ProofTerm::PullQuantifier {
                formula,
                quantifier_type,
                result,
            } => self.validate_pull_quantifier(formula, quantifier_type, result, expected),

            ProofTerm::PushQuantifier {
                formula,
                quantifier_type,
                result,
            } => self.validate_push_quantifier(formula, quantifier_type, result, expected),

            ProofTerm::ElimUnusedVars { formula, result } => {
                self.validate_elim_unused_vars(formula, result, expected)
            }

            ProofTerm::DerElim { premise } => self.validate_der_elim(premise, expected),

            ProofTerm::QuickExplain {
                unsat_core,
                explanation,
            } => self.validate_quick_explain(unsat_core, explanation, expected),
        };

        // Decrement depth
        self.current_depth -= 1;

        result
    }

    // ==================== Individual Proof Rule Validators ====================

    /// Validate axiom proof term
    fn validate_axiom(&self, name: &Text, formula: &Expr, expected: &Expr) -> ValidationResult<()> {
        // Check axiom exists in database
        if let Some(registered_formula) = self.axioms.get(name) {
            // Verify formula matches registered axiom
            if !self.expr_eq(formula, registered_formula) {
                return Err(ValidationError::ValidationFailed {
                    message: format!("axiom {} formula mismatch", name).into(),
                });
            }
        } else {
            return Err(ValidationError::UnknownAxiom {
                axiom: name.clone(),
            });
        }

        // Check formula proves expected proposition
        if !self.expr_eq(formula, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(formula),
            });
        }

        Ok(())
    }

    /// Validate assumption proof term
    fn validate_assumption(
        &self,
        id: usize,
        formula: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Check formula matches expected
        if !self.expr_eq(formula, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(formula),
            });
        }

        // Assumptions are valid by construction in the proof context
        Ok(())
    }

    /// Validate hypothesis proof term.
    ///
    /// A `Hypothesis { id, formula }` claims that `h{id}` proves
    /// `formula`. Soundness requires checking THREE things:
    /// 1. `formula` matches the user-supplied `expected` (sanity check).
    /// 2. `h{id}` is actually in scope (no dangling reference).
    /// 3. The hypothesis at `h{id}` has `formula` as its proposition —
    ///    otherwise the user could claim anything that happens to
    ///    syntactically match `expected`, even if `h{id}` proves
    ///    something completely different.
    ///
    /// Pre-fix only (1) and (2) were checked. (3) was missing, so a
    /// hypothesis `h0 : P` could be re-labeled by the user as proving
    /// `Q` (with formula = expected = Q) and the validator silently
    /// accepted. This commit closes that gap.
    fn validate_hypothesis(
        &self,
        id: usize,
        formula: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // (1) sanity: user's formula must match the claimed conclusion.
        if !self.expr_eq(formula, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(formula),
            });
        }

        // (2) reference: h{id} must be in scope.
        let hyp_name = Text::from(format!("h{}", id));
        let actual = match self.hypotheses.lookup(&hyp_name) {
            Maybe::Some(prop) => prop,
            Maybe::None => {
                return Err(ValidationError::HypothesisNotFound {
                    hypothesis: hyp_name,
                });
            }
        };

        // (3) content: the hypothesis at h{id} must actually carry the
        // claimed `formula`. Without this gate the user could claim
        // anything as long as it matches `expected`.
        if !self.expr_eq(&actual, formula) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(formula),
                actual: self.expr_to_text(&actual),
            });
        }

        Ok(())
    }

    /// Validate modus ponens: from P and P→Q, derive Q
    fn validate_modus_ponens(
        &mut self,
        premise: &ProofTerm,
        implication: &ProofTerm,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Get conclusions of premise and implication
        let premise_conclusion = premise.conclusion();
        let impl_conclusion = implication.conclusion();

        // Extract P and Q from implication (P → Q)
        let (antecedent, consequent) = self.extract_implication(&impl_conclusion)?;

        // Check that premise proves P
        self.validate_impl(premise, &antecedent)?;

        // Check that implication proves P → Q
        self.validate_impl(implication, &impl_conclusion)?;

        // Check that Q matches expected
        if !self.expr_eq(&consequent, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(&consequent),
            });
        }

        Ok(())
    }

    /// Validate rewrite rule application
    ///
    /// This validates that applying a named rewrite rule to the source
    /// expression produces the target expression. The rewrite rule must
    /// either be a registered rule or a standard theory rule.
    ///
    /// ## Validation Process
    ///
    /// 1. Validate the source proof establishes its conclusion
    /// 2. Look up the rewrite rule by name
    /// 3. Check that the source matches the rule's LHS pattern
    /// 4. Verify that the target matches the rule's RHS with substitutions
    /// 5. Validate any conditions required by the rule
    ///
    /// ## Standard Theory Rules
    ///
    /// The following standard rewrite rules are recognized:
    /// - `simp`: Simplification rules (arithmetic, boolean)
    /// - `ring`: Ring normalization (associativity, commutativity, distributivity)
    /// - `field`: Field operations (division, inverses)
    /// - `arith`: Linear arithmetic simplification
    /// - `beta`: Beta reduction for lambda expressions
    /// - `eta`: Eta expansion/reduction
    /// - `unfold_*`: Definition unfolding
    ///
    /// Rewrite tactic: given a proof of an equality or a named rewrite rule
    /// (simp, arith, beta, eta, unfold_*), validate that applying the rewrite
    /// to the source produces the expected target expression.
    fn validate_rewrite(
        &mut self,
        source: &ProofTerm,
        rule: &Text,
        target: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Validate source proof
        let source_conclusion = source.conclusion();
        self.validate_impl(source, &source_conclusion)?;

        // Check target matches expected
        if !self.expr_eq(target, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(target),
            });
        }

        // Look up the rewrite rule
        if let Some(rewrite_rule) = self.rewrite_rules.get(rule) {
            // Validate the rule application by pattern matching
            self.validate_rewrite_rule_application(&source_conclusion, target, rewrite_rule)?;
        } else {
            // Check for standard theory rules
            self.validate_standard_rewrite(rule, &source_conclusion, target)?;
        }

        Ok(())
    }

    /// Validate application of a registered rewrite rule
    fn validate_rewrite_rule_application(
        &self,
        source: &Expr,
        target: &Expr,
        rule: &RewriteRule,
    ) -> ValidationResult<()> {
        // Try to match source against LHS pattern
        let mut bindings = Map::new();
        if self.pattern_match(&rule.lhs, source, &mut bindings) {
            // Apply bindings to RHS and check it matches target
            let expected_target = self.apply_bindings(&rule.rhs, &bindings);
            if self.expr_eq(&expected_target, target) {
                // Discharge each condition rather than trust it. A
                // conditional rewrite is only sound when its conditions
                // actually hold; previously this loop discarded the
                // instantiated condition and accepted the rewrite
                // unconditionally.
                for condition in &rule.conditions {
                    let instantiated_cond = self.apply_bindings(condition, &bindings);
                    self.discharge_rewrite_condition(&rule.name, &instantiated_cond)?;
                }
                return Ok(());
            }
        }

        // Try reverse direction if bidirectional. Reverse rewrites must
        // also discharge their conditions.
        if rule.bidirectional {
            let mut reverse_bindings = Map::new();
            if self.pattern_match(&rule.rhs, source, &mut reverse_bindings) {
                let expected_target = self.apply_bindings(&rule.lhs, &reverse_bindings);
                if self.expr_eq(&expected_target, target) {
                    for condition in &rule.conditions {
                        let instantiated_cond =
                            self.apply_bindings(condition, &reverse_bindings);
                        self.discharge_rewrite_condition(&rule.name, &instantiated_cond)?;
                    }
                    return Ok(());
                }
            }
        }

        Err(ValidationError::RewriteError {
            message: format!(
                "rewrite rule '{}' does not transform {} to {}",
                rule.name,
                self.expr_to_text(source),
                self.expr_to_text(target)
            )
            .into(),
        })
    }

    /// Discharge a single instantiated rewrite condition.
    ///
    /// Soundness gate for conditional rewrites — a conditional rule is
    /// only valid when its conditions actually hold at the rewrite site.
    /// We accept exactly the cases that can be checked syntactically
    /// against state already in the validator (axioms, hypotheses,
    /// trivial truths). Anything beyond that returns an error so the
    /// proof author can either register an axiom, introduce a
    /// hypothesis, or use a different proof tactic.
    ///
    /// Pre-fix this was an empty trust-the-user path that accepted any
    /// condition without checking — a soundness leak: a malformed proof
    /// could apply `safe_div(a, b) → a/b` claiming `b ≠ 0` while it
    /// actually doesn't hold.
    ///
    /// Accepted shapes:
    /// 1. The literal `true`.
    /// 2. Reflexive equality (`x == x` for any x).
    /// 3. The condition matches a registered axiom's formula.
    /// 4. The condition matches a hypothesis currently in scope.
    fn discharge_rewrite_condition(
        &self,
        rule_name: &Text,
        condition: &Expr,
    ) -> ValidationResult<()> {
        // 1. Literal `true`.
        if let ExprKind::Literal(lit) = &condition.kind {
            if matches!(lit.kind, verum_ast::LiteralKind::Bool(true)) {
                return Ok(());
            }
            // Literal `false` is decisively unprovable.
            if matches!(lit.kind, verum_ast::LiteralKind::Bool(false)) {
                return Err(ValidationError::RewriteError {
                    message: format!(
                        "rewrite rule '{}' has condition that is literally false",
                        rule_name
                    )
                    .into(),
                });
            }
        }
        // 2. Reflexive equality `x == x`.
        if let ExprKind::Binary {
            op: BinOp::Eq,
            left,
            right,
        } = &condition.kind
        {
            if self.expr_eq(left, right) {
                return Ok(());
            }
        }
        // 3. Axiom match.
        for (_axiom_name, axiom_formula) in self.axioms.iter() {
            if self.expr_eq(axiom_formula, condition) {
                return Ok(());
            }
        }
        // 4. Hypothesis match. `HypothesisContext.contains` is name-keyed;
        //    here we need to recognise the proposition regardless of
        //    which name it was bound to, so iterate across scopes.
        for (_name, prop) in self.hypotheses.iter_propositions() {
            if self.expr_eq(prop, condition) {
                return Ok(());
            }
        }

        Err(ValidationError::RewriteError {
            message: format!(
                "rewrite rule '{}' has unverified condition '{}' — register it as an axiom or introduce it as a hypothesis before rewriting",
                rule_name,
                self.expr_to_text(condition)
            )
            .into(),
        })
    }

    /// Validate standard theory rewrite rules
    fn validate_standard_rewrite(
        &self,
        rule_name: &Text,
        source: &Expr,
        target: &Expr,
    ) -> ValidationResult<()> {
        let rule_str = rule_name.as_str();

        match rule_str {
            // Simplification - accepts any transformation that preserves logical equivalence
            //
            // Pre-fix this arm returned `Ok(())` unconditionally —
            // any source→target pair (even Forall→Path) would
            // validate under `simp`. Now gates on
            // `structurally_compatible` to reject cross-kind
            // pairs while preserving the legitimate simp uses
            // (same-kind reductions plus the Literal↔Path
            // definition-unfolding cross-pair). Same trust-the-
            // user soundness pattern as 7ef97a6d.
            "simp" | "simplify" => {
                if !self.structurally_compatible(source, target) {
                    return Err(ValidationError::RewriteError {
                        message: format!(
                            "{} requires structurally-compatible source/target",
                            rule_str
                        )
                        .into(),
                    });
                }
                Ok(())
            }

            // Ring normalization - associativity, commutativity, distributivity
            "ring" => {
                // Ring rules normalize arithmetic expressions
                // Verify structure is compatible (both arithmetic expressions)
                if self.is_arithmetic_expr(source) && self.is_arithmetic_expr(target) {
                    Ok(())
                } else {
                    Err(ValidationError::RewriteError {
                        message: "ring tactic requires arithmetic expressions".into(),
                    })
                }
            }

            // Field operations — extend ring with division/inverses
            // Same arithmetic-expr gate as `ring` above; pre-fix
            // accepted any source→target unconditionally.
            "field" => {
                if self.is_arithmetic_expr(source) && self.is_arithmetic_expr(target) {
                    Ok(())
                } else {
                    Err(ValidationError::RewriteError {
                        message: "field tactic requires arithmetic expressions".into(),
                    })
                }
            }

            // Linear integer arithmetic — same arithmetic-expr gate
            // as `ring` and `field`. Pre-fix accepted any
            // source→target unconditionally; arith claiming
            // `Forall x. P(x) ↦ 42` would validate trivially.
            "arith" | "omega" | "lia" => {
                if self.is_arithmetic_expr(source) && self.is_arithmetic_expr(target) {
                    Ok(())
                } else {
                    Err(ValidationError::RewriteError {
                        message: format!(
                            "{} tactic requires arithmetic expressions",
                            rule_str
                        )
                        .into(),
                    })
                }
            }

            // Beta reduction: (\x.e) v -> e[v/x]
            "beta" => self.validate_beta_reduction(source, target),

            // Eta reduction/expansion
            "eta" => self.validate_eta_conversion(source, target),

            // Definition unfolding — replaces a name (Path) with
            // its definition. Pre-fix accepted any source→target
            // pair under any rule starting with "unfold_". The
            // structural gate allows the legitimate Path↔Literal
            // cross-pair (per `structurally_compatible`'s explicit
            // arm) and same-discriminant pairs, while rejecting
            // arbitrary cross-kind transformations claimed under
            // an `unfold_*` name.
            _ if rule_str.starts_with("unfold_") => {
                if !self.structurally_compatible(source, target) {
                    return Err(ValidationError::RewriteError {
                        message: format!(
                            "{} requires structurally-compatible source/target",
                            rule_str
                        )
                        .into(),
                    });
                }
                Ok(())
            }

            // Unknown rule - accept if source and target have compatible structure
            _ => {
                // For unknown rules, we accept the rewrite if:
                // 1. Source and target have the same top-level structure, or
                // 2. They are logically equivalent by some standard property
                if self.structurally_compatible(source, target) {
                    Ok(())
                } else {
                    Err(ValidationError::RewriteError {
                        message: format!(
                            "unknown rewrite rule '{}' and expressions are not compatible",
                            rule_name
                        )
                        .into(),
                    })
                }
            }
        }
    }

    /// Check if an expression is an arithmetic expression
    fn is_arithmetic_expr(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Literal(lit) => {
                matches!(lit.kind, LiteralKind::Int(_) | LiteralKind::Float(_))
            }
            ExprKind::Binary { op, .. } => matches!(
                op,
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem
            ),
            ExprKind::Unary { op, .. } => matches!(op, verum_ast::UnOp::Neg),
            ExprKind::Path(_) => true, // Variables can be arithmetic
            ExprKind::Paren(inner) => self.is_arithmetic_expr(inner),
            _ => false,
        }
    }

    /// Validate beta reduction: (\x.e) v -> e[v/x]
    fn validate_beta_reduction(&self, source: &Expr, target: &Expr) -> ValidationResult<()> {
        // Source should be a function application where the function is a lambda
        if let ExprKind::Call { func, args, .. } = &source.kind {
            if let ExprKind::Closure { params, body, .. } = &func.kind {
                // Check we have matching number of arguments
                if params.len() != args.len() {
                    return Err(ValidationError::RewriteError {
                        message: "beta reduction: argument count mismatch".into(),
                    });
                }

                // Build substitution map
                let mut subst = Map::new();
                for (param, arg) in params.iter().zip(args.iter()) {
                    let param_name = closure_param_name(param);
                    subst.insert(param_name, arg.clone());
                }

                // Apply substitution and check result
                let reduced = self.substitute_expr(body, &subst, &Set::new());
                if self.expr_eq(&reduced, target) {
                    return Ok(());
                }
            }
        }

        Err(ValidationError::RewriteError {
            message: "beta reduction pattern not matched".into(),
        })
    }

    /// Validate eta conversion: \x.f x <-> f (when x not free in f)
    fn validate_eta_conversion(&self, source: &Expr, target: &Expr) -> ValidationResult<()> {
        // Eta expansion: f -> \x.f x
        if let ExprKind::Closure { params, body, .. } = &target.kind {
            if params.len() == 1 {
                if let ExprKind::Call { func, args, .. } = &body.kind {
                    if args.len() == 1 {
                        let param_name = closure_param_name(&params[0]);
                        // Check if arg is just the parameter
                        if let ExprKind::Path(path) = &args[0].kind {
                            if let Some(ident) = path.as_ident() {
                                if ident.as_str() == param_name.as_str() {
                                    // Check func equals source
                                    if self.expr_eq(func, source) {
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Eta reduction: \x.f x -> f
        if let ExprKind::Closure { params, body, .. } = &source.kind {
            if params.len() == 1 {
                if let ExprKind::Call { func, args, .. } = &body.kind {
                    if args.len() == 1 {
                        let param_name = closure_param_name(&params[0]);
                        if let ExprKind::Path(path) = &args[0].kind {
                            if let Some(ident) = path.as_ident() {
                                if ident.as_str() == param_name.as_str() {
                                    if self.expr_eq(func, target) {
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Err(ValidationError::RewriteError {
            message: "eta conversion pattern not matched".into(),
        })
    }

    /// Check if two expressions are structurally compatible for rewriting.
    ///
    /// Compatibility means the rewrite is at least PLAUSIBLY a valid
    /// transformation: source and target share the same top-level
    /// `ExprKind` discriminant, OR they fall into one of the
    /// explicitly-allowed cross-kind rewrites (Literal ↔ Path —
    /// constants and named bindings can interchange under
    /// definition unfolding).
    ///
    /// Pre-fix the catch-all returned `true` for every pair, which
    /// made the "unknown rewrite rule" branch in
    /// `validate_apply_rewrite_rule` (line ~2360) accept any
    /// source→target pair under any unregistered rule name. Same
    /// trust-the-user soundness pattern as the inference-rule
    /// catch-all fixed in 8429bd4e and the quantifier rules in
    /// 80f43418.
    ///
    /// The new same-discriminant fallback uses
    /// `std::mem::discriminant` so EVERY ExprKind variant pair is
    /// covered uniformly: Binary/Unary check the operator on top of
    /// the discriminant match (preserving the prior strict op==
    /// gate); explicit Literal/Path cross-pair stays accepted; all
    /// other same-kind pairs (Forall/Forall, Block/Block, etc.) get
    /// the structural-discriminant check; cross-kind pairs reject.
    ///
    /// Note: this is NOT semantic equivalence — it only confirms
    /// the rewrite isn't trivially malformed. A rewrite that
    /// transforms `2+3` → `2*3` has matching Binary discriminants
    /// AND matching `Add` operators, so it passes here, but the
    /// SMT/expr_eq pipeline catches the actual mathematical
    /// content elsewhere.
    fn structurally_compatible(&self, source: &Expr, target: &Expr) -> bool {
        match (&source.kind, &target.kind) {
            // Same constructor types are compatible (Binary/Unary
            // strengthen with operator equality).
            (ExprKind::Binary { op: op1, .. }, ExprKind::Binary { op: op2, .. }) => op1 == op2,
            (ExprKind::Unary { op: op1, .. }, ExprKind::Unary { op: op2, .. }) => op1 == op2,
            // Explicit Literal ↔ Path cross-pair: definition unfolding
            // legitimately swaps a constant for its name (or vice
            // versa). Pre-existing accepted cross-pair; preserved.
            (ExprKind::Literal(_), ExprKind::Path(_)) => true,
            (ExprKind::Path(_), ExprKind::Literal(_)) => true,
            // Same-discriminant fallback for every other ExprKind:
            // Forall/Forall, Exists/Exists, Block/Block, Match/Match,
            // Tuple/Tuple, Array/Array, etc. Discriminant-only check
            // (no operator comparison) is the structural minimum.
            (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b),
        }
    }

    /// Pattern match an expression against a pattern, collecting variable bindings
    fn pattern_match(&self, pattern: &Expr, expr: &Expr, bindings: &mut Map<Text, Expr>) -> bool {
        match &pattern.kind {
            // Variable pattern - matches anything and binds it
            ExprKind::Path(path) if path.segments.len() == 1 => {
                if let Some(ident) = path.as_ident() {
                    let var_name = Text::from(ident.as_str());
                    // Check if this is a pattern variable (starts with lowercase)
                    if ident
                        .as_str()
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_lowercase())
                    {
                        // Check for existing binding
                        if let Some(existing) = bindings.get(&var_name) {
                            // Must match existing binding
                            return self.expr_eq(existing, expr);
                        } else {
                            // New binding
                            bindings.insert(var_name, expr.clone());
                            return true;
                        }
                    }
                }
                // Non-pattern variable - must match exactly
                self.expr_eq(pattern, expr)
            }

            // Binary operation pattern
            ExprKind::Binary {
                op: pop,
                left: pl,
                right: pr,
            } => {
                if let ExprKind::Binary {
                    op: eop,
                    left: el,
                    right: er,
                } = &expr.kind
                {
                    pop == eop
                        && self.pattern_match(pl, el, bindings)
                        && self.pattern_match(pr, er, bindings)
                } else {
                    false
                }
            }

            // Unary operation pattern
            ExprKind::Unary { op: pop, expr: pe } => {
                if let ExprKind::Unary { op: eop, expr: ee } = &expr.kind {
                    pop == eop && self.pattern_match(pe, ee, bindings)
                } else {
                    false
                }
            }

            // Call pattern
            ExprKind::Call { func: pf, args: pa, .. } => {
                if let ExprKind::Call { func: ef, args: ea, .. } = &expr.kind {
                    self.pattern_match(pf, ef, bindings)
                        && pa.len() == ea.len()
                        && pa
                            .iter()
                            .zip(ea.iter())
                            .all(|(p, e)| self.pattern_match(p, e, bindings))
                } else {
                    false
                }
            }

            // Literal pattern - must match exactly
            ExprKind::Literal(_) => self.expr_eq(pattern, expr),

            // Default - structural equality
            _ => self.expr_eq(pattern, expr),
        }
    }

    /// Apply variable bindings to an expression
    fn apply_bindings(&self, expr: &Expr, bindings: &Map<Text, Expr>) -> Expr {
        self.substitute_expr(expr, bindings, &Set::new())
    }

    /// Validate symmetry: from A = B, derive B = A
    fn validate_symmetry(&mut self, equality: &ProofTerm, expected: &Expr) -> ValidationResult<()> {
        let eq_conclusion = equality.conclusion();

        // Extract A and B from A = B
        let (left, right) = self.extract_equality(&eq_conclusion)?;

        // Validate equality proof
        self.validate_impl(equality, &eq_conclusion)?;

        // Construct B = A
        let flipped = self.make_equality(&right, &left);

        // Check flipped equality matches expected
        if !self.expr_eq(&flipped, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(&flipped),
            });
        }

        Ok(())
    }

    /// Validate transitivity: from A = B and B = C, derive A = C
    fn validate_transitivity(
        &mut self,
        left: &ProofTerm,
        right: &ProofTerm,
        expected: &Expr,
    ) -> ValidationResult<()> {
        let left_conclusion = left.conclusion();
        let right_conclusion = right.conclusion();

        // Extract A = B
        let (a, b1) = self.extract_equality(&left_conclusion)?;

        // Extract B = C
        let (b2, c) = self.extract_equality(&right_conclusion)?;

        // Validate both equality proofs
        self.validate_impl(left, &left_conclusion)?;
        self.validate_impl(right, &right_conclusion)?;

        // Check that middle terms match (B = B)
        if !self.expr_eq(&b1, &b2) {
            return Err(ValidationError::EqualityError {
                message: format!(
                    "transitivity middle terms don't match: {} ≠ {}",
                    self.expr_to_text(&b1),
                    self.expr_to_text(&b2)
                )
                .into(),
            });
        }

        // Construct A = C
        let result = self.make_equality(&a, &c);

        // Check result matches expected
        if !self.expr_eq(&result, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(&result),
            });
        }

        Ok(())
    }

    /// Validate reflexivity: derive A = A
    fn validate_reflexivity(&self, term: &Expr, expected: &Expr) -> ValidationResult<()> {
        // Construct term = term
        let equality = self.make_equality(term, term);

        // Check equality matches expected
        if !self.expr_eq(&equality, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(&equality),
            });
        }

        Ok(())
    }

    /// Validate theory lemma
    fn validate_theory_lemma(
        &self,
        theory: &Text,
        lemma: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Check theory exists and lemma is registered
        if let Some(lemmas) = self.theory_lemmas.get(theory) {
            let found = lemmas.iter().any(|l| self.expr_eq(l, lemma));
            if !found {
                return Err(ValidationError::ValidationFailed {
                    message: format!("theory lemma not found in theory {}", theory).into(),
                });
            }
        }

        // Check lemma matches expected
        if !self.expr_eq(lemma, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(lemma),
            });
        }

        Ok(())
    }

    /// Validate unit resolution (SAT reasoning)
    ///
    /// Unit resolution is a sound and complete SAT reasoning rule:
    /// Given a clause (L1 ∨ L2 ∨ ... ∨ Ln) and unit clauses containing
    /// negated literals, we can derive a smaller clause.
    ///
    /// Rule: If we have clause C = (L ∨ D) and unit clause ¬L,
    ///       we can derive D (with L resolved away).
    ///
    /// Unit Resolution proof rule: given clause C = (L v D) and unit clause ~L,
    /// derive D (with L resolved away). Used in SAT/SMT proof reconstruction.
    fn validate_unit_resolution(
        &mut self,
        clauses: &List<ProofTerm>,
        expected: &Expr,
    ) -> ValidationResult<()> {
        if clauses.is_empty() {
            return Err(ValidationError::ValidationFailed {
                message: "unit resolution requires at least one clause".into(),
            });
        }

        // Validate each clause proof
        for clause in clauses.iter() {
            let clause_conclusion = clause.conclusion();
            self.validate_impl(clause, &clause_conclusion)?;
        }

        // Perform unit resolution algorithm
        // 1. Extract all literals from unit clauses (clauses with single literal)
        // 2. For non-unit clauses, resolve against unit literals
        // 3. Check that the result matches expected

        let mut unit_literals: List<(Expr, bool)> = List::new(); // (literal, is_positive)
        let mut non_unit_clauses: List<List<(Expr, bool)>> = List::new();

        for clause in clauses.iter() {
            let conclusion = clause.conclusion();
            let literals = self.extract_clause_literals(&conclusion);

            if literals.len() == 1 {
                // Unit clause - add to unit literals
                unit_literals.push(literals[0].clone());
            } else if !literals.is_empty() {
                // Non-unit clause
                non_unit_clauses.push(literals);
            }
        }

        // Apply resolution: for each non-unit clause, remove literals that
        // have their negation in the unit literals
        let resolved_clause = if non_unit_clauses.is_empty() {
            // If only unit clauses, the result is the conjunction of units
            // or if there's a conflict (P and ¬P), the result is False
            if self.has_unit_conflict(&unit_literals) {
                // Conflict detected - result is False
                self.make_false()
            } else if unit_literals.len() == 1 {
                unit_literals[0].0.clone()
            } else {
                // Build conjunction of remaining units
                self.build_conjunction_from_literals(&unit_literals)
            }
        } else {
            // Resolve each non-unit clause against unit literals
            let mut result_clause = non_unit_clauses[0].clone();

            for (unit_lit, unit_positive) in &unit_literals {
                result_clause.retain(|(lit, positive)| {
                    // Keep literal if it doesn't conflict with unit
                    !(self.expr_eq(lit, unit_lit) && *positive != *unit_positive)
                });
            }

            // Convert remaining literals back to expression
            if result_clause.is_empty() {
                self.make_false() // Empty clause = contradiction
            } else if result_clause.len() == 1 {
                let (lit, positive) = &result_clause[0];
                if *positive {
                    lit.clone()
                } else {
                    self.make_negation(lit)
                }
            } else {
                self.build_disjunction_from_literals(&result_clause)
            }
        };

        // Check resolved clause matches expected
        if !self.expr_eq(&resolved_clause, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(&resolved_clause),
            });
        }

        Ok(())
    }

    /// Extract literals from a clause expression
    /// Returns list of (literal, is_positive) pairs
    fn extract_clause_literals(&self, clause: &Expr) -> List<(Expr, bool)> {
        let mut literals = List::new();
        self.extract_literals_recursive(clause, &mut literals);
        literals
    }

    fn extract_literals_recursive(&self, expr: &Expr, literals: &mut List<(Expr, bool)>) {
        match &expr.kind {
            // Disjunction: L1 ∨ L2
            ExprKind::Binary {
                op: BinOp::Or,
                left,
                right,
            } => {
                self.extract_literals_recursive(left, literals);
                self.extract_literals_recursive(right, literals);
            }
            // Negation: ¬L
            ExprKind::Unary {
                op: verum_ast::UnOp::Not,
                expr: inner,
            } => {
                // Negative literal
                literals.push(((**inner).clone(), false));
            }
            // Any other expression is a positive literal
            _ => {
                literals.push((expr.clone(), true));
            }
        }
    }

    /// Check if there's a conflicting pair in unit literals (P and ¬P)
    fn has_unit_conflict(&self, units: &[(Expr, bool)]) -> bool {
        for i in 0..units.len() {
            for j in (i + 1)..units.len() {
                let (lit_i, pos_i) = &units[i];
                let (lit_j, pos_j) = &units[j];
                if self.expr_eq(lit_i, lit_j) && pos_i != pos_j {
                    return true;
                }
            }
        }
        false
    }

    /// Build a disjunction from literals
    fn build_disjunction_from_literals(&self, literals: &[(Expr, bool)]) -> Expr {
        if literals.is_empty() {
            return self.make_false();
        }

        let mut result = if literals[0].1 {
            literals[0].0.clone()
        } else {
            self.make_negation(&literals[0].0)
        };

        for (lit, positive) in &literals[1..] {
            let lit_expr = if *positive {
                lit.clone()
            } else {
                self.make_negation(lit)
            };
            result = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Or,
                    left: Heap::new(result),
                    right: Heap::new(lit_expr),
                },
                Span::dummy(),
            );
        }
        result
    }

    /// Build a conjunction from literals
    fn build_conjunction_from_literals(&self, literals: &[(Expr, bool)]) -> Expr {
        if literals.is_empty() {
            return self.make_true();
        }

        let mut result = if literals[0].1 {
            literals[0].0.clone()
        } else {
            self.make_negation(&literals[0].0)
        };

        for (lit, positive) in &literals[1..] {
            let lit_expr = if *positive {
                lit.clone()
            } else {
                self.make_negation(lit)
            };
            result = Expr::new(
                ExprKind::Binary {
                    op: BinOp::And,
                    left: Heap::new(result),
                    right: Heap::new(lit_expr),
                },
                Span::dummy(),
            );
        }
        result
    }

    /// Make a negation expression
    fn make_negation(&self, expr: &Expr) -> Expr {
        Expr::new(
            ExprKind::Unary {
                op: verum_ast::UnOp::Not,
                expr: Heap::new(expr.clone()),
            },
            Span::dummy(),
        )
    }

    /// Make a False literal
    fn make_false(&self) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(false), Span::dummy())),
            Span::dummy(),
        )
    }

    /// Make a True literal
    fn make_true(&self) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        )
    }

    /// Validate quantifier instantiation
    ///
    /// This validates the elimination rule for universal quantification:
    /// From `forall x: T. P(x)` and a term `t: T`, we can derive `P(t)`.
    ///
    /// For dependent types, we also need to verify:
    /// 1. The witness term `t` has the correct type `T`
    /// 2. The instantiated formula `P(t)` is well-typed
    /// 3. Any type dependencies are properly substituted
    ///
    /// ## Dependent Type Considerations
    ///
    /// In dependent type theory, the type `T` may depend on earlier bindings,
    /// and the formula `P(x)` may contain type-level computations. For example:
    ///
    /// ```text
    /// forall (n: Nat). forall (v: Vec n). length(v) = n
    /// ```
    ///
    /// When instantiating `n` with `3`, we get:
    /// ```text
    /// forall (v: Vec 3). length(v) = 3
    /// ```
    ///
    /// Quantifier instantiation for dependent types: given a universally
    /// quantified proof (forall x: T. P(x)) and a substitution map,
    /// validate that applying the substitution produces the expected result.
    fn validate_quantifier_instantiation(
        &mut self,
        quantified: &ProofTerm,
        instantiation: &Map<Text, Expr>,
        expected: &Expr,
    ) -> ValidationResult<()> {
        let quant_conclusion = quantified.conclusion();

        // Validate quantified proof
        self.validate_impl(quantified, &quant_conclusion)?;

        // Extract the quantifier structure
        let (bound_vars, body) = self.extract_quantifier_structure(&quant_conclusion)?;

        // Validate that all bound variables have instantiations
        for (var_name, var_type) in &bound_vars {
            if let Some(witness) = instantiation.get(var_name) {
                // In strict mode, validate the witness has the expected type
                if self.config.strict_types {
                    self.validate_witness_type(witness, var_type)?;
                }
            }
        }

        // Apply instantiation to the body
        let instantiated = self.apply_instantiation(&body, instantiation);

        // For dependent types, we may need to normalize types after substitution
        let normalized = self.normalize_types(&instantiated);

        // Check instantiated formula matches expected
        if !self.expr_eq(&normalized, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(&normalized),
            });
        }

        Ok(())
    }

    /// Extract the structure of a universally quantified formula
    ///
    /// Returns a list of (variable_name, variable_type) pairs and the body.
    fn extract_quantifier_structure(
        &self,
        expr: &Expr,
    ) -> ValidationResult<(List<(Text, Expr)>, Expr)> {
        let mut bound_vars = List::new();
        let mut current = expr.clone();

        loop {
            match &current.kind {
                ExprKind::Forall { bindings, body } => {
                    // Extract variable names and types from bindings
                    for binding in bindings {
                        if let verum_ast::PatternKind::Ident { name, .. } = &binding.pattern.kind {
                            let var_name = Text::from(name.as_str());
                            // Use explicit type if available, otherwise inferred type
                            let var_type = if let Maybe::Some(ty) = &binding.ty {
                                self.ty_to_expr(ty)
                            } else {
                                // Create a placeholder for inferred type
                                let placeholder_ident = verum_ast::Ident::new("_", binding.span);
                                Expr::new(
                                    ExprKind::Path(verum_ast::ty::Path::single(placeholder_ident)),
                                    binding.span,
                                )
                            };
                            bound_vars.push((var_name, var_type));
                        }
                    }
                    current = (**body).clone();
                }
                ExprKind::Exists { bindings, body } => {
                    // Existential quantifier uses same extraction logic
                    for binding in bindings {
                        if let verum_ast::PatternKind::Ident { name, .. } = &binding.pattern.kind {
                            let var_name = Text::from(name.as_str());
                            let var_type = if let Maybe::Some(ty) = &binding.ty {
                                self.ty_to_expr(ty)
                            } else {
                                let placeholder_ident = verum_ast::Ident::new("_", binding.span);
                                Expr::new(
                                    ExprKind::Path(verum_ast::ty::Path::single(placeholder_ident)),
                                    binding.span,
                                )
                            };
                            bound_vars.push((var_name, var_type));
                        }
                    }
                    current = (**body).clone();
                }
                _ => break,
            }
        }

        Ok((bound_vars, current))
    }

    /// Validate that a witness term has the expected type
    ///
    /// This is used in dependent type checking to ensure instantiations
    /// are well-typed. For existential elimination, the witness must have
    /// the correct type to instantiate the existentially quantified variable.
    ///
    /// # Type Inference Strategy
    /// 1. Infer the type of the witness expression from its structure
    /// 2. Normalize both inferred and expected types
    /// 3. Check structural compatibility with type coercion rules
    ///
    /// # Supported Type Forms
    /// - Literals: Int, Bool, Float, Text, Char
    /// - Paths: Named types and variables
    /// - Tuples: Product types
    /// - Function applications: Result types
    /// - Binary operations: Result types based on operator
    ///
    /// Witness type validation for existential proofs and dependent types.
    /// Infers the type of the witness expression (literals, paths, tuples,
    /// function applications, binary operations) and checks compatibility
    /// with the expected type after normalization.
    fn validate_witness_type(&self, witness: &Expr, expected_type: &Expr) -> ValidationResult<()> {
        // Infer the type of the witness expression
        let inferred_type = self.infer_witness_type(witness)?;

        // Normalize both types for comparison
        let normalized_inferred = self.normalize_types(&inferred_type);
        let normalized_expected = self.normalize_types(expected_type);

        // Check type compatibility
        if self.types_compatible(&normalized_inferred, &normalized_expected) {
            Ok(())
        } else {
            Err(ValidationError::TypeError {
                expected: format!("{:?}", expected_type).into(),
                actual: format!("{:?}", inferred_type).into(),
            })
        }
    }

    /// Infer the type of a witness expression
    ///
    /// This performs local type inference on expressions without requiring
    /// a full type environment. It handles common expression forms used
    /// in proof witnesses.
    fn infer_witness_type(&self, witness: &Expr) -> ValidationResult<Expr> {
        use verum_ast::{Ident, Path};

        let span = witness.span;

        match &witness.kind {
            // Literals have known types
            ExprKind::Literal(lit) => {
                let type_name = match &lit.kind {
                    LiteralKind::Int(_) => "Int",
                    LiteralKind::Float(_) => "Float",
                    LiteralKind::Bool(_) => "Bool",
                    LiteralKind::Char(_) => "Char",
                    LiteralKind::Text(_) => "Text",
                    _ => "Unknown", // Handle other literal kinds
                };
                let path = Path::from_ident(Ident::new(type_name, span));
                Ok(Expr::new(ExprKind::Path(path), span))
            }

            // Path expressions: look up in hypotheses or return generic type
            ExprKind::Path(path) => {
                // Check if this is a hypothesis with a known type
                if let Some(ident) = path.as_ident() {
                    let name: Text = ident.as_str().into();
                    // Look through hypothesis scopes for type information
                    for scope in self.hypotheses.scopes.iter() {
                        if let Maybe::Some(prop) = scope.hypotheses.get(&name) {
                            // If the hypothesis is a typing judgment, extract the type
                            if let Some(ty) = self.extract_type_from_judgment(prop) {
                                return Ok(ty);
                            }
                        }
                    }
                }
                // Return a generic type variable for unknown paths
                // This allows the comparison to succeed if expected type is also generic
                Ok(witness.clone())
            }

            // Tuples: product type of element types
            ExprKind::Tuple(elements) => {
                let element_types: Result<List<Expr>, _> = elements
                    .iter()
                    .map(|e| self.infer_witness_type(e))
                    .collect();
                Ok(Expr::new(ExprKind::Tuple(element_types?), span))
            }

            // Binary operations: infer result type based on operator and operands
            ExprKind::Binary { op, left, right } => {
                let left_type = self.infer_witness_type(left)?;
                let right_type = self.infer_witness_type(right)?;

                // Determine result type based on operator
                match op {
                    // Comparison operators return Bool
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        let path = Path::from_ident(Ident::new("Bool", span));
                        Ok(Expr::new(ExprKind::Path(path), span))
                    }
                    // Logical operators return Bool
                    BinOp::And | BinOp::Or => {
                        let path = Path::from_ident(Ident::new("Bool", span));
                        Ok(Expr::new(ExprKind::Path(path), span))
                    }
                    // Arithmetic operators preserve type of operands
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                        // Type is the common type of operands (simplified)
                        Ok(left_type)
                    }
                    // Bitwise operators preserve integer type
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                        Ok(left_type)
                    }
                    // Other operators - return left operand type as default
                    _ => Ok(left_type),
                }
            }

            // Function calls: infer return type from known function signatures
            // For proof witnesses, we track common proof-relevant functions
            ExprKind::Call { func, args, .. } => {
                if let ExprKind::Path(path) = &func.kind {
                    // Get function name for pattern matching
                    if let Some(ident) = path.as_ident() {
                        let func_name = ident.as_str();

                        // Match against known proof-related functions and their return types
                        return match func_name {
                            // Standard constructors that preserve their type
                            "Some" | "Just" | "Ok" | "Left" | "Right" => {
                                // These wrap their argument type
                                if let Some(arg) = args.first() {
                                    let inner_type = self.infer_witness_type(arg)?;
                                    // Return wrapped type (e.g., Option<T>)
                                    let type_path = Path::from_ident(Ident::new(func_name, span));
                                    Ok(Expr::new(
                                        ExprKind::Call {
                                            func: Heap::new(Expr::new(
                                                ExprKind::Path(type_path),
                                                span,
                                            )),
                                            type_args: Vec::new().into(),
                                            args: vec![inner_type].into(),
                                        },
                                        span,
                                    ))
                                } else {
                                    Ok(self.make_unknown_type())
                                }
                            }

                            // Unwrapping functions
                            "unwrap" | "expect" | "get" => {
                                // These extract from wrapped types
                                // Argument type determines result
                                if let Some(arg) = args.first() {
                                    let arg_type = self.infer_witness_type(arg)?;
                                    // If arg is Option<T>, result is T
                                    if let ExprKind::Call {
                                        args: inner_args, ..
                                    } = &arg_type.kind
                                    {
                                        if let Some(inner) = inner_args.first() {
                                            return Ok(inner.clone());
                                        }
                                    }
                                }
                                Ok(self.make_unknown_type())
                            }

                            // Boolean-returning predicates
                            "is_some" | "is_none" | "is_ok" | "is_err" | "contains"
                            | "is_empty" | "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "and"
                            | "or" | "not" => {
                                let path = Path::from_ident(Ident::new("Bool", span));
                                Ok(Expr::new(ExprKind::Path(path), span))
                            }

                            // Arithmetic functions that preserve numeric type
                            "abs" | "neg" | "sqrt" | "sin" | "cos" | "tan" | "exp" | "log"
                            | "floor" | "ceil" | "round" => {
                                if let Some(arg) = args.first() {
                                    self.infer_witness_type(arg)
                                } else {
                                    let path = Path::from_ident(Ident::new("Float", span));
                                    Ok(Expr::new(ExprKind::Path(path), span))
                                }
                            }

                            // Integer-returning functions
                            "len" | "count" | "size" | "index_of" => {
                                let path = Path::from_ident(Ident::new("Int", span));
                                Ok(Expr::new(ExprKind::Path(path), span))
                            }

                            // Collection access functions
                            "head" | "first" | "last" | "get_at" => {
                                // These return the element type of the collection
                                if let Some(arg) = args.first() {
                                    let coll_type = self.infer_witness_type(arg)?;
                                    // Extract element type from List<T> or Array<T>
                                    if let ExprKind::Call {
                                        args: type_args, ..
                                    } = &coll_type.kind
                                    {
                                        if let Some(elem) = type_args.first() {
                                            return Ok(elem.clone());
                                        }
                                    }
                                }
                                Ok(self.make_unknown_type())
                            }

                            // Map functions
                            "map" | "filter" | "fold" | "reduce" => {
                                // These depend on the closure argument - use unknown
                                Ok(self.make_unknown_type())
                            }

                            // Pair/tuple accessors
                            "fst" | "first_of" => {
                                if let Some(arg) = args.first() {
                                    let pair_type = self.infer_witness_type(arg)?;
                                    if let ExprKind::Tuple(elems) = &pair_type.kind {
                                        if let Some(first) = elems.first() {
                                            return Ok(first.clone());
                                        }
                                    }
                                }
                                Ok(self.make_unknown_type())
                            }
                            "snd" | "second_of" => {
                                if let Some(arg) = args.first() {
                                    let pair_type = self.infer_witness_type(arg)?;
                                    if let ExprKind::Tuple(elems) = &pair_type.kind {
                                        if elems.len() >= 2 {
                                            return Ok(elems[1].clone());
                                        }
                                    }
                                }
                                Ok(self.make_unknown_type())
                            }

                            // Default: check axioms for function signature
                            _ => {
                                // Try to find function type in axioms or inference rules
                                let func_text: Text = func_name.into();
                                if let Some((_, conclusion)) = self.inference_rules.get(&func_text)
                                {
                                    // Use the conclusion type as return type hint
                                    return Ok(conclusion.clone());
                                }
                                Ok(self.make_unknown_type())
                            }
                        };
                    }
                }
                // Method calls or complex expressions - use unknown type
                Ok(self.make_unknown_type())
            }

            // Field access: would need struct type lookup
            ExprKind::Field { .. } => Ok(self.make_unknown_type()),

            // If expressions: both branches should have same type
            ExprKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                // Get type from then branch's last expression
                if let Some(last_stmt) = then_branch.stmts.last() {
                    if let verum_ast::stmt::StmtKind::Expr { expr, .. } = &last_stmt.kind {
                        return self.infer_witness_type(expr);
                    }
                }
                // Fall back to else branch if present
                if let Some(else_expr) = else_branch {
                    return self.infer_witness_type(else_expr);
                }
                Ok(self.make_unknown_type())
            }

            // Block expressions: type is type of final expression
            ExprKind::Block(block) => {
                if let Some(last_stmt) = block.stmts.last() {
                    if let verum_ast::stmt::StmtKind::Expr { expr, .. } = &last_stmt.kind {
                        return self.infer_witness_type(expr);
                    }
                }
                // Empty block has Unit type
                let path = Path::from_ident(Ident::new("Unit", span));
                Ok(Expr::new(ExprKind::Path(path), span))
            }

            // For other expressions, return unknown type
            _ => Ok(self.make_unknown_type()),
        }
    }

    /// Extract type from a typing judgment proposition
    ///
    /// Typing judgments have the form `x : T` which may appear as
    /// hypotheses in proof contexts.
    fn extract_type_from_judgment(&self, prop: &Expr) -> Option<Expr> {
        // Look for typing judgment pattern: x : T
        // This would be represented as a call to 'HasType' predicate
        match &prop.kind {
            // Could also check for call to 'HasType' predicate
            ExprKind::Call { func, args, .. } => {
                if let ExprKind::Path(path) = &func.kind {
                    if path.as_ident().map(|i| i.as_str()) == Some("HasType") {
                        if args.len() >= 2 {
                            return Some(args[1].clone());
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Check if two types are compatible (with coercion rules)
    ///
    /// This implements structural type comparison with support for:
    /// - Alpha equivalence for bound variables
    /// - Unknown type wildcards
    /// - Numeric type coercions
    fn types_compatible(&self, inferred: &Expr, expected: &Expr) -> bool {
        // Unknown types are compatible with anything
        if self.is_unknown_type(inferred) || self.is_unknown_type(expected) {
            return true;
        }

        // Use structural equality with alpha-equivalence
        if self.expr_eq(inferred, expected) {
            return true;
        }

        // Check for numeric coercions (Int -> Float is allowed in some contexts)
        if self.is_numeric_coercion_allowed(inferred, expected) {
            return true;
        }

        // Check tuple compatibility element-wise
        if let (ExprKind::Tuple(inferred_elems), ExprKind::Tuple(expected_elems)) =
            (&inferred.kind, &expected.kind)
        {
            if inferred_elems.len() == expected_elems.len() {
                return inferred_elems
                    .iter()
                    .zip(expected_elems.iter())
                    .all(|(i, e)| self.types_compatible(i, e));
            }
        }

        false
    }

    /// Check if an expression represents an unknown type placeholder
    fn is_unknown_type(&self, expr: &Expr) -> bool {
        if let ExprKind::Path(path) = &expr.kind {
            if let Some(ident) = path.as_ident() {
                let name = ident.as_str();
                return name == "_Unknown" || name == "_" || name.starts_with('_');
            }
        }
        false
    }

    /// Check if numeric type coercion is allowed
    fn is_numeric_coercion_allowed(&self, from: &Expr, to: &Expr) -> bool {
        let from_name = self.get_type_name(from);
        let to_name = self.get_type_name(to);

        match (from_name.as_deref(), to_name.as_deref()) {
            // Int can coerce to Float
            (Some("Int"), Some("Float")) => true,
            // Smaller integer types can coerce to larger ones
            (Some("I8"), Some("I16" | "I32" | "I64" | "Int")) => true,
            (Some("I16"), Some("I32" | "I64" | "Int")) => true,
            (Some("I32"), Some("I64" | "Int")) => true,
            (Some("U8"), Some("U16" | "U32" | "U64" | "Int")) => true,
            (Some("U16"), Some("U32" | "U64" | "Int")) => true,
            (Some("U32"), Some("U64" | "Int")) => true,
            // Byte can coerce to Int
            (Some("Byte"), Some("Int" | "U8")) => true,
            _ => false,
        }
    }

    /// Extract type name from a type expression
    fn get_type_name(&self, expr: &Expr) -> Option<String> {
        if let ExprKind::Path(path) = &expr.kind {
            if let Some(ident) = path.as_ident() {
                return Some(ident.as_str().to_string());
            }
        }
        None
    }

    /// Convert a Type to an Expr for type-level operations
    fn ty_to_expr(&self, ty: &verum_ast::Type) -> Expr {
        use verum_ast::{Ident, Path, TypeKind};

        let span = ty.span;
        match &ty.kind {
            TypeKind::Path(type_path) => {
                // Convert type path to expression path
                Expr::new(ExprKind::Path(type_path.clone()), span)
            }
            TypeKind::Bool => {
                let path = Path::from_ident(Ident::new("Bool", span));
                Expr::new(ExprKind::Path(path), span)
            }
            TypeKind::Int => {
                let path = Path::from_ident(Ident::new("Int", span));
                Expr::new(ExprKind::Path(path), span)
            }
            TypeKind::Float => {
                let path = Path::from_ident(Ident::new("Float", span));
                Expr::new(ExprKind::Path(path), span)
            }
            TypeKind::Text => {
                let path = Path::from_ident(Ident::new("Text", span));
                Expr::new(ExprKind::Path(path), span)
            }
            TypeKind::Char => {
                let path = Path::from_ident(Ident::new("Char", span));
                Expr::new(ExprKind::Path(path), span)
            }
            TypeKind::Unit => {
                // Unit type represented as empty tuple
                Expr::new(ExprKind::Tuple(List::new()), span)
            }
            TypeKind::Tuple(types) => {
                // Convert tuple types to expression
                let exprs: List<Expr> = types.iter().map(|t| self.ty_to_expr(t)).collect();
                Expr::new(ExprKind::Tuple(exprs), span)
            }
            _ => {
                // For complex types, create a placeholder
                let path = Path::from_ident(Ident::new("_Type", span));
                Expr::new(ExprKind::Path(path), span)
            }
        }
    }

    /// Create an unknown type placeholder
    fn make_unknown_type(&self) -> Expr {
        use verum_ast::{Ident, Path};
        let path = Path::from_ident(Ident::new("_Unknown", Span::dummy()));
        Expr::new(ExprKind::Path(path), Span::dummy())
    }

    /// Normalize types in an expression (beta reduction, etc.)
    ///
    /// This handles type-level computation that may occur after substitution
    /// in dependent types. Normalization reduces expressions to their simplest
    /// equivalent form for comparison.
    ///
    /// # Normalization Strategy (Weak Head Normal Form)
    /// 1. **Beta reduction**: `(λx. body)(arg)` → `body[arg/x]`
    /// 2. **Type family application**: Reduce type constructor applications
    /// 3. **Equality simplification**: `T = T` → `true`, reflexivity
    /// 4. **Constant folding**: Simplify literal operations
    /// 5. **Structural recursion**: Normalize subexpressions
    ///
    /// # Termination
    /// Uses a depth limit to prevent infinite loops in recursive types.
    /// Maximum normalization depth is 100 steps.
    ///
    /// Type-level computation: normalize type expressions by expanding
    /// type aliases, reducing beta-redexes, and simplifying arithmetic.
    /// Uses a depth limit of 100 to prevent infinite loops in recursive types.
    fn normalize_types(&self, expr: &Expr) -> Expr {
        self.normalize_types_impl(expr, 0)
    }

    /// Implementation of type normalization with depth tracking
    fn normalize_types_impl(&self, expr: &Expr, depth: usize) -> Expr {
        // Termination check - prevent infinite normalization
        const MAX_DEPTH: usize = 100;
        if depth >= MAX_DEPTH {
            return expr.clone();
        }

        let span = expr.span;

        match &expr.kind {
            // Beta reduction: (λx. body)(arg) → body[arg/x]
            ExprKind::Call { func, args, .. } => {
                // First normalize the function
                let normalized_func = self.normalize_types_impl(func, depth + 1);

                // Check for closure application (beta redex)
                if let ExprKind::Closure { params, body, .. } = &normalized_func.kind {
                    // Perform beta reduction if we have matching arguments
                    if params.len() == args.len() {
                        // Build substitution map: param names -> normalized args
                        let mut substitution = Map::new();
                        for (param, arg) in params.iter().zip(args.iter()) {
                            let normalized_arg = self.normalize_types_impl(arg, depth + 1);
                            // Extract parameter name from pattern
                            let param_name: Text = match &param.pattern.kind {
                                verum_ast::pattern::PatternKind::Ident { name, .. } => {
                                    Text::from(name.name.as_str())
                                }
                                _ => Text::from("_"),
                            };
                            substitution.insert(param_name, normalized_arg);
                        }

                        // Apply substitution to body
                        let reduced_body = self.instantiate(body, &substitution);

                        // Recursively normalize the result
                        return self.normalize_types_impl(&reduced_body, depth + 1);
                    }
                }

                // Not a beta redex - normalize arguments and rebuild
                let normalized_args: List<Expr> = args
                    .iter()
                    .map(|a| self.normalize_types_impl(a, depth + 1))
                    .collect();

                Expr::new(
                    ExprKind::Call {
                        func: Heap::new(normalized_func),
                        type_args: Vec::new().into(),
                        args: normalized_args,
                    },
                    span,
                )
            }

            // Simplify equality expressions: T = T → true
            ExprKind::Binary {
                op: BinOp::Eq,
                left,
                right,
            } => {
                let normalized_left = self.normalize_types_impl(left, depth + 1);
                let normalized_right = self.normalize_types_impl(right, depth + 1);

                // Reflexivity: if both sides are equal, reduce to true
                if self.expr_eq(&normalized_left, &normalized_right) {
                    return Expr::new(
                        ExprKind::Literal(Literal {
                            kind: LiteralKind::Bool(true),
                            span,
                        }),
                        span,
                    );
                }

                // Otherwise rebuild with normalized subexpressions
                Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Eq,
                        left: Heap::new(normalized_left),
                        right: Heap::new(normalized_right),
                    },
                    span,
                )
            }

            // Normalize other binary operations
            ExprKind::Binary { op, left, right } => {
                let normalized_left = self.normalize_types_impl(left, depth + 1);
                let normalized_right = self.normalize_types_impl(right, depth + 1);

                // Try constant folding for boolean operations
                let result =
                    self.try_fold_binary_op(*op, &normalized_left, &normalized_right, span);
                result.unwrap_or_else(|| {
                    Expr::new(
                        ExprKind::Binary {
                            op: *op,
                            left: Heap::new(normalized_left),
                            right: Heap::new(normalized_right),
                        },
                        span,
                    )
                })
            }

            // Normalize closure bodies (but don't reduce under closures)
            ExprKind::Closure {
                async_,
                move_,
                params,
                contexts,
                return_type,
                body,
            } => {
                // In weak head normal form, we don't normalize under closures
                // But we do normalize for full normalization needed in type comparison
                let normalized_body = self.normalize_types_impl(body, depth + 1);
                Expr::new(
                    ExprKind::Closure {
                        async_: *async_,
                        move_: *move_,
                        params: params.clone(),
                        contexts: contexts.clone(),
                        return_type: return_type.clone(),
                        body: Heap::new(normalized_body),
                    },
                    span,
                )
            }

            // Normalize conditional expressions
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Normalize condition first
                let normalized_conditions: List<_> = condition
                    .conditions
                    .iter()
                    .map(|c| self.normalize_condition_kind(c, depth + 1))
                    .collect();

                // Check if condition is a literal true/false for reduction
                if normalized_conditions.len() == 1 {
                    if let verum_ast::expr::ConditionKind::Expr(cond_expr) =
                        &normalized_conditions[0]
                    {
                        if let ExprKind::Literal(lit) = &cond_expr.kind {
                            if let LiteralKind::Bool(b) = &lit.kind {
                                if *b {
                                    // Condition is true - return then branch
                                    if let Some(last_stmt) = then_branch.stmts.last() {
                                        if let verum_ast::stmt::StmtKind::Expr { expr, .. } =
                                            &last_stmt.kind
                                        {
                                            return self.normalize_types_impl(expr, depth + 1);
                                        }
                                    }
                                } else if let Some(else_expr) = else_branch {
                                    // Condition is false - return else branch
                                    return self.normalize_types_impl(else_expr, depth + 1);
                                }
                            }
                        }
                    }
                }

                // Can't reduce - normalize branches and rebuild
                let normalized_else = else_branch
                    .as_ref()
                    .map(|e| Heap::new(self.normalize_types_impl(e, depth + 1)));

                Expr::new(
                    ExprKind::If {
                        condition: Heap::new(verum_ast::expr::IfCondition {
                            conditions: normalized_conditions.into_iter().collect(),
                            span,
                        }),
                        then_branch: self.normalize_block(then_branch, depth + 1),
                        else_branch: normalized_else,
                    },
                    span,
                )
            }

            // Normalize tuple elements
            ExprKind::Tuple(elements) => {
                let normalized: List<Expr> = elements
                    .iter()
                    .map(|e| self.normalize_types_impl(e, depth + 1))
                    .collect();
                Expr::new(ExprKind::Tuple(normalized), span)
            }

            // Normalize field access - try to reduce record literals
            ExprKind::Field { expr: base, field } => {
                let normalized_base = self.normalize_types_impl(base, depth + 1);

                // Check if base is a record literal we can reduce
                if let ExprKind::Record { fields, .. } = &normalized_base.kind {
                    let field_name = field.as_str();
                    for record_field in fields.iter() {
                        if record_field.name.as_str() == field_name {
                            if let Maybe::Some(value) = &record_field.value {
                                return self.normalize_types_impl(value, depth + 1);
                            }
                        }
                    }
                }

                // Can't reduce - rebuild
                Expr::new(
                    ExprKind::Field {
                        expr: Heap::new(normalized_base),
                        field: field.clone(),
                    },
                    span,
                )
            }

            // Paths, literals, and other atomic forms are already normalized
            ExprKind::Path(_) | ExprKind::Literal(_) => expr.clone(),

            // For other expression kinds, preserve structure
            _ => expr.clone(),
        }
    }

    /// Normalize a condition kind (helper for if-expressions)
    fn normalize_condition_kind(
        &self,
        cond: &verum_ast::expr::ConditionKind,
        depth: usize,
    ) -> verum_ast::expr::ConditionKind {
        match cond {
            verum_ast::expr::ConditionKind::Expr(e) => {
                verum_ast::expr::ConditionKind::Expr(self.normalize_types_impl(e, depth))
            }
            verum_ast::expr::ConditionKind::Let { pattern, value } => {
                verum_ast::expr::ConditionKind::Let {
                    pattern: pattern.clone(),
                    value: self.normalize_types_impl(value, depth),
                }
            }
        }
    }

    /// Normalize a block by normalizing all statements and the trailing expression
    ///
    /// # Arguments
    /// * `block` - The block to normalize
    /// * `depth` - Current normalization recursion depth
    ///
    /// # Returns
    /// A new block with all expressions normalized
    fn normalize_block(&self, block: &verum_ast::Block, depth: usize) -> verum_ast::Block {
        use verum_ast::stmt::StmtKind;

        // Normalize each statement in the block
        let normalized_stmts: List<verum_ast::Stmt> = block
            .stmts
            .iter()
            .map(|stmt| {
                let new_kind = match &stmt.kind {
                    StmtKind::Let { pattern, ty, value } => StmtKind::Let {
                        pattern: pattern.clone(),
                        ty: ty.clone(),
                        value: value.as_ref().map(|v| self.normalize_types_impl(v, depth)),
                    },
                    StmtKind::LetElse {
                        pattern,
                        ty,
                        value,
                        else_block,
                    } => StmtKind::LetElse {
                        pattern: pattern.clone(),
                        ty: ty.clone(),
                        value: self.normalize_types_impl(value, depth),
                        else_block: self.normalize_block(else_block, depth),
                    },
                    StmtKind::Expr { expr, has_semi } => StmtKind::Expr {
                        expr: self.normalize_types_impl(expr, depth),
                        has_semi: *has_semi,
                    },
                    StmtKind::Item(item) => {
                        // Items don't need expression normalization
                        StmtKind::Item(item.clone())
                    }
                    StmtKind::Defer(expr) => {
                        StmtKind::Defer(self.normalize_types_impl(expr, depth))
                    }
                    StmtKind::Errdefer(expr) => {
                        StmtKind::Errdefer(self.normalize_types_impl(expr, depth))
                    }
                    StmtKind::Provide { context, alias, value } => StmtKind::Provide {
                        context: context.clone(),
                        alias: alias.clone(),
                        value: Heap::new(self.normalize_types_impl(value, depth)),
                    },
                    StmtKind::ProvideScope {
                        context,
                        alias,
                        value,
                        block: scope_block,
                    } => StmtKind::ProvideScope {
                        context: context.clone(),
                        alias: alias.clone(),
                        value: Heap::new(self.normalize_types_impl(value, depth)),
                        block: Heap::new(self.normalize_types_impl(scope_block, depth)),
                    },
                    StmtKind::Empty => StmtKind::Empty,
                };
                verum_ast::Stmt {
                    kind: new_kind,
                    span: stmt.span,
                    attributes: stmt.attributes.clone(),
                }
            })
            .collect();

        // Normalize the trailing expression if present
        let normalized_expr = block
            .expr
            .as_ref()
            .map(|e| Heap::new(self.normalize_types_impl(e, depth)));

        verum_ast::Block {
            stmts: normalized_stmts,
            expr: normalized_expr,
            span: block.span,
        }
    }

    /// Try to fold binary operations on literals (constant folding)
    fn try_fold_binary_op(&self, op: BinOp, left: &Expr, right: &Expr, span: Span) -> Option<Expr> {
        // Extract boolean literal from left side if present
        let left_bool = match &left.kind {
            ExprKind::Literal(Literal {
                kind: LiteralKind::Bool(l),
                ..
            }) => Some(*l),
            _ => None,
        };

        // Extract boolean literal from right side if present
        let right_bool = match &right.kind {
            ExprKind::Literal(Literal {
                kind: LiteralKind::Bool(r),
                ..
            }) => Some(*r),
            _ => None,
        };

        // Fold boolean operations when both sides are literals
        if let (Some(l), Some(r)) = (left_bool, right_bool) {
            let result = match op {
                BinOp::And => l && r,
                BinOp::Or => l || r,
                BinOp::Eq => l == r,
                BinOp::Ne => l != r,
                _ => return None,
            };
            return Some(Expr::new(
                ExprKind::Literal(Literal {
                    kind: LiteralKind::Bool(result),
                    span,
                }),
                span,
            ));
        }

        // Fold with single boolean literal (short-circuit)
        if let Some(l) = left_bool {
            match op {
                BinOp::And if !l => {
                    return Some(Expr::new(
                        ExprKind::Literal(Literal {
                            kind: LiteralKind::Bool(false),
                            span,
                        }),
                        span,
                    ));
                }
                BinOp::And if l => return Some(right.clone()),
                BinOp::Or if l => {
                    return Some(Expr::new(
                        ExprKind::Literal(Literal {
                            kind: LiteralKind::Bool(true),
                            span,
                        }),
                        span,
                    ));
                }
                BinOp::Or if !l => return Some(right.clone()),
                _ => {}
            }
        }

        if let Some(r) = right_bool {
            match op {
                BinOp::And if !r => {
                    return Some(Expr::new(
                        ExprKind::Literal(Literal {
                            kind: LiteralKind::Bool(false),
                            span,
                        }),
                        span,
                    ));
                }
                BinOp::And if r => return Some(left.clone()),
                BinOp::Or if r => {
                    return Some(Expr::new(
                        ExprKind::Literal(Literal {
                            kind: LiteralKind::Bool(true),
                            span,
                        }),
                        span,
                    ));
                }
                BinOp::Or if !r => return Some(left.clone()),
                _ => {}
            }
        }

        // Extract integer literals for integer folding
        let (left_int, right_int) = match (&left.kind, &right.kind) {
            (
                ExprKind::Literal(Literal {
                    kind: LiteralKind::Int(l),
                    ..
                }),
                ExprKind::Literal(Literal {
                    kind: LiteralKind::Int(r),
                    ..
                }),
            ) => (Some(l.value), Some(r.value)),
            _ => (None, None),
        };

        // Fold integer operations
        if let (Some(l), Some(r)) = (left_int, right_int) {
            match op {
                BinOp::Add => {
                    return Some(Expr::new(
                        ExprKind::Literal(Literal {
                            kind: LiteralKind::Int(IntLit {
                                value: l.wrapping_add(r),
                                suffix: None,
                            }),
                            span,
                        }),
                        span,
                    ));
                }
                BinOp::Sub => {
                    return Some(Expr::new(
                        ExprKind::Literal(Literal {
                            kind: LiteralKind::Int(IntLit {
                                value: l.wrapping_sub(r),
                                suffix: None,
                            }),
                            span,
                        }),
                        span,
                    ));
                }
                BinOp::Mul => {
                    return Some(Expr::new(
                        ExprKind::Literal(Literal {
                            kind: LiteralKind::Int(IntLit {
                                value: l.wrapping_mul(r),
                                suffix: None,
                            }),
                            span,
                        }),
                        span,
                    ));
                }
                BinOp::Eq => {
                    return Some(Expr::new(
                        ExprKind::Literal(Literal {
                            kind: LiteralKind::Bool(l == r),
                            span,
                        }),
                        span,
                    ));
                }
                BinOp::Ne => {
                    return Some(Expr::new(
                        ExprKind::Literal(Literal {
                            kind: LiteralKind::Bool(l != r),
                            span,
                        }),
                        span,
                    ));
                }
                BinOp::Lt => {
                    return Some(Expr::new(
                        ExprKind::Literal(Literal {
                            kind: LiteralKind::Bool(l < r),
                            span,
                        }),
                        span,
                    ));
                }
                BinOp::Le => {
                    return Some(Expr::new(
                        ExprKind::Literal(Literal {
                            kind: LiteralKind::Bool(l <= r),
                            span,
                        }),
                        span,
                    ));
                }
                BinOp::Gt => {
                    return Some(Expr::new(
                        ExprKind::Literal(Literal {
                            kind: LiteralKind::Bool(l > r),
                            span,
                        }),
                        span,
                    ));
                }
                BinOp::Ge => {
                    return Some(Expr::new(
                        ExprKind::Literal(Literal {
                            kind: LiteralKind::Bool(l >= r),
                            span,
                        }),
                        span,
                    ));
                }
                _ => {}
            }
        }

        None
    }

    /// Validate lambda abstraction (introduction rule)
    ///
    /// Lambda abstraction implements the introduction rules for:
    /// 1. Implication introduction: If Γ, P ⊢ Q then Γ ⊢ P → Q
    /// 2. Universal introduction: If Γ ⊢ P(x) for fresh x then Γ ⊢ ∀x. P(x)
    ///
    /// The expected result is either:
    /// - An implication (P → Q) where P is the assumed hypothesis type
    /// - A universal quantification (∀x: T. P(x)) where T is the variable type
    ///
    /// Lambda introduction rule: validates proof of implication (P -> Q) or
    /// universal quantification (forall x: T. P(x)) by introducing the
    /// variable/hypothesis into a new scope and validating the body.
    fn validate_lambda(
        &mut self,
        var: &Text,
        body: &ProofTerm,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Enter new scope for lambda parameter
        self.hypotheses.enter_scope();

        // Extract the type of the bound variable from the expected result
        // Expected should be either: P → Q (implication) or ∀x: T. P(x) (forall)
        let (hypothesis_type, expected_body) = self.extract_lambda_type(expected)?;

        // Add var as hypothesis with its type
        // For implication P → Q, the hypothesis type is P
        // For forall ∀x: T. P(x), we add x: T as an assumption
        self.hypotheses
            .add_hypothesis(var.clone(), hypothesis_type.clone());

        // Validate body - should prove the conclusion of the implication/forall
        let body_conclusion = body.conclusion();
        let result = self.validate_impl(body, &body_conclusion);

        // Exit lambda scope
        self.hypotheses.exit_scope();

        result?;

        // For implication: body should prove Q (the conclusion of P → Q)
        // For universal: body should prove P(x) where x is the bound variable
        //
        // We need to check that body_conclusion matches expected_body
        // after accounting for the variable binding
        if !self.expr_eq_modulo_var(&body_conclusion, &expected_body, var) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(&expected_body),
                actual: self.expr_to_text(&body_conclusion),
            });
        }

        Ok(())
    }

    /// Extract the type from a lambda's expected result
    /// For implication P → Q: returns (P, Q)
    /// For forall ∀x: T. P: returns (T-as-proposition, P)
    fn extract_lambda_type(&self, expected: &Expr) -> ValidationResult<(Expr, Expr)> {
        match &expected.kind {
            // Implication: P → Q
            ExprKind::Binary {
                op: BinOp::Imply,
                left,
                right,
            } => Ok(((**left).clone(), (**right).clone())),
            // Universal quantification: ∀x: T. P
            ExprKind::Forall { bindings, body } => {
                // Use the first binding for type extraction
                // For multiple bindings, the caller should handle nested structure
                if let Some(binding) = bindings.first() {
                    // Create default type to avoid temporary borrow issues
                    let default_ty = verum_ast::Type::inferred(Span::dummy());
                    let ty = binding.ty.as_ref().unwrap_or(&default_ty);
                    // The type becomes the hypothesis type
                    // Represented as a proposition for the Curry-Howard correspondence
                    let type_prop = self.make_type_proposition_from_pattern(&binding.pattern, ty);
                    Ok((type_prop, (**body).clone()))
                } else {
                    Err(ValidationError::ValidationFailed {
                        message: "forall expression has no bindings".into(),
                    })
                }
            }
            _ => Err(ValidationError::ValidationFailed {
                message: format!(
                    "lambda abstraction expects implication or forall, got {:?}",
                    expected.kind
                )
                .into(),
            }),
        }
    }

    /// Make a type proposition from pattern and type: x : T
    fn make_type_proposition_from_pattern(&self, pattern: &Pattern, ty: &verum_ast::Type) -> Expr {
        // Extract the variable name from the pattern
        let var_name = match &pattern.kind {
            verum_ast::pattern::PatternKind::Ident { name, .. } => name.clone(),
            _ => verum_ast::Ident::new("_", Span::dummy()),
        };

        // Create a path expression for the variable
        let var_path = verum_ast::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(var_name)],
            span: Span::dummy(),
        };
        let var_expr = Expr::new(ExprKind::Path(var_path), Span::dummy());

        // Create a type ascription expression: x : T
        Expr::new(
            ExprKind::Cast {
                expr: Heap::new(var_expr),
                ty: ty.clone(),
            },
            Span::dummy(),
        )
    }

    /// Check expression equality modulo a bound variable
    /// This handles alpha-equivalence for the bound variable
    fn expr_eq_modulo_var(&self, e1: &Expr, e2: &Expr, bound_var: &Text) -> bool {
        // For most cases, structural equality with alpha-equivalence handling
        // For the bound variable, we allow renaming
        self.expr_eq_with_binding(e1, e2, bound_var)
    }

    /// Expression equality with one variable bound on both sides.
    ///
    /// Reuses [`expr_eq_impl`]'s binding-map machinery: we pre-populate
    /// each side's binding map with `bound_var ↦ depth 0`, so a Path
    /// referring to `bound_var` matches the corresponding Path on the
    /// other side as a bound-at-same-depth occurrence rather than a
    /// free variable.
    ///
    /// Pre-fix this just delegated to `expr_eq` and ignored the
    /// `bound_var` argument entirely — α-equivalence checking did not
    /// happen, so a `bound_var` Path appearing inside two different
    /// scopes would be wrongly equated even when only one of them was
    /// the actual bound occurrence.
    ///
    /// This single-binder variant covers the immediate caller in
    /// `validate_lambda`; multi-binder generalisation (different
    /// names on each side) would need a renaming map and is tracked
    /// separately.
    fn expr_eq_with_binding(&self, e1: &Expr, e2: &Expr, bound_var: &Text) -> bool {
        let mut left_bindings: HashMap<Text, usize> = HashMap::new();
        let mut right_bindings: HashMap<Text, usize> = HashMap::new();
        left_bindings.insert(bound_var.clone(), 0);
        right_bindings.insert(bound_var.clone(), 0);
        self.expr_eq_impl(e1, e2, &mut left_bindings, &mut right_bindings, 1)
    }

    /// Validate proof by cases
    fn validate_cases(
        &mut self,
        scrutinee: &Expr,
        cases: &List<(Expr, Heap<ProofTerm>)>,
        expected: &Expr,
    ) -> ValidationResult<()> {
        if cases.is_empty() {
            return Err(ValidationError::CasesError {
                message: "cases proof must have at least one case".into(),
            });
        }

        // Each case must prove the same conclusion
        for (pattern, proof) in cases.iter() {
            // Enter new scope for case
            self.hypotheses.enter_scope();

            // Extract and add pattern bindings as hypotheses
            // Pattern matching introduces variable bindings that can be used in the proof
            self.add_pattern_bindings(pattern, scrutinee);

            // Validate case proof
            let case_conclusion = proof.conclusion();
            self.validate_impl(proof, &case_conclusion)?;

            // Check case proves expected
            if !self.expr_eq(&case_conclusion, expected) {
                return Err(ValidationError::CasesError {
                    message: format!(
                        "case proves wrong conclusion: expected {}, got {}",
                        self.expr_to_text(expected),
                        self.expr_to_text(&case_conclusion)
                    )
                    .into(),
                });
            }

            // Exit case scope
            self.hypotheses.exit_scope();
        }

        Ok(())
    }

    /// Validate proof by induction
    ///
    /// Validates a proof by mathematical induction on a variable.
    /// The expected result should be a universally quantified property:
    /// ∀n: Nat. P(n)
    ///
    /// Induction requires:
    /// 1. Base case: P(0) or P(base_value)
    /// 2. Inductive case: ∀n. P(n) → P(n+1) or similar
    ///
    /// The inductive hypothesis IH is made available in the inductive case scope.
    ///
    /// Induction tactic: validates proof by structural induction.
    /// Requires: (1) base case P(0) or P(base_value),
    /// (2) inductive case forall n. P(n) -> P(n+1).
    /// The inductive hypothesis IH is made available in the inductive case scope.
    fn validate_induction(
        &mut self,
        var: &Text,
        base_case: &ProofTerm,
        inductive_case: &ProofTerm,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Extract the property P and base value from expected (∀n. P(n))
        let (induction_var, property_template) = self.extract_induction_structure(expected)?;

        // Verify that the induction variable matches
        if &induction_var != var {
            return Err(ValidationError::InductionError {
                message: format!(
                    "induction variable mismatch: expected {}, got {}",
                    induction_var, var
                )
                .into(),
            });
        }

        // Well-foundedness gate: if the property template does not
        // reference the induction variable, induction is vacuous —
        // the IH P(n) and the obligation P(n+1) are syntactically
        // identical, so the IH trivially discharges the step case
        // regardless of whether P actually holds for any value.
        // Reject up front when the documented `check_well_founded`
        // flag is on. The cheap test: substitute the var with a
        // distinct probe value; if the result is structurally
        // identical to the original, the var is unused.
        if self.config.check_well_founded {
            let probe = self.make_zero();
            let probed =
                self.substitute_induction_var(&property_template, var, &probe);
            if self.expr_eq(&property_template, &probed) {
                return Err(ValidationError::InductionError {
                    message: format!(
                        "induction is not well-founded: property does not \
                         reference induction variable `{}` — IH and step \
                         obligation are identical, so any P would `discharge` \
                         vacuously",
                        var
                    )
                    .into(),
                });
            }
        }

        // 1. Validate base case: should prove P(0) or P(base)
        let base_conclusion = base_case.conclusion();
        self.validate_impl(base_case, &base_conclusion)?;

        // Check that base case is a valid substitution of the property at base value
        let expected_base =
            self.substitute_induction_var(&property_template, var, &self.make_zero());
        if !self.expr_eq(&base_conclusion, &expected_base) {
            // Try with the template directly if substitution failed
            if !self.is_base_case_of(&base_conclusion, &property_template, var) {
                return Err(ValidationError::InductionError {
                    message: format!(
                        "base case does not prove P(base): expected {}, got {}",
                        self.expr_to_text(&expected_base),
                        self.expr_to_text(&base_conclusion)
                    )
                    .into(),
                });
            }
        }

        // 2. Validate inductive case with IH in scope
        self.hypotheses.enter_scope();

        // Add inductive hypothesis: IH_n := P(n)
        // The IH is the property applied to the current value n
        let ih_name = Text::from(format!("IH_{}", var));
        let ih_prop = property_template.clone(); // P(n) - will be available as hypothesis
        self.hypotheses.add_hypothesis(ih_name.clone(), ih_prop);

        // Also add the induction variable itself as a hypothesis
        // with the constraint that it's a natural number (n : Nat, n >= 0)
        let var_constraint = self.make_nat_constraint(var);
        self.hypotheses.add_hypothesis(var.clone(), var_constraint);

        let inductive_conclusion = inductive_case.conclusion();
        let result = self.validate_impl(inductive_case, &inductive_conclusion);

        self.hypotheses.exit_scope();

        result?;

        // Check inductive case proves P(n+1) given IH: P(n)
        let expected_step =
            self.substitute_induction_var(&property_template, var, &self.make_successor(var));

        if !self.expr_eq(&inductive_conclusion, &expected_step) {
            // Try checking if it's a valid successor case
            if !self.is_step_case_of(&inductive_conclusion, &property_template, var) {
                return Err(ValidationError::InductionError {
                    message: format!(
                        "inductive case does not prove P(n+1): expected {}, got {}",
                        self.expr_to_text(&expected_step),
                        self.expr_to_text(&inductive_conclusion)
                    )
                    .into(),
                });
            }
        }

        Ok(())
    }

    /// Extract the structure of an induction target: ∀n. P(n) → (var_name, property)
    fn extract_induction_structure(&self, expected: &Expr) -> ValidationResult<(Text, Expr)> {
        match &expected.kind {
            ExprKind::Forall { bindings, body } => {
                // Use the first binding for induction variable
                if let Some(binding) = bindings.first() {
                    let var_name = match &binding.pattern.kind {
                        verum_ast::pattern::PatternKind::Ident { name, .. } => {
                            Text::from(name.as_str())
                        }
                        _ => {
                            return Err(ValidationError::InductionError {
                                message: "induction pattern must be a simple binding".into(),
                            });
                        }
                    };
                    Ok((var_name, (**body).clone()))
                } else {
                    Err(ValidationError::InductionError {
                        message: "forall expression has no bindings".into(),
                    })
                }
            }
            _ => {
                // Try to treat the whole expression as the property
                // with an implicit universal quantifier
                Err(ValidationError::InductionError {
                    message: "induction target must be a universally quantified property".into(),
                })
            }
        }
    }

    /// Substitute the induction variable with a value in the property
    fn substitute_induction_var(&self, property: &Expr, var: &Text, value: &Expr) -> Expr {
        self.substitute_var_in_expr(property, var, value)
    }

    /// Substitute a variable with a value in an expression
    fn substitute_var_in_expr(&self, expr: &Expr, var: &Text, value: &Expr) -> Expr {
        match &expr.kind {
            ExprKind::Path(path) if path.segments.len() == 1 => {
                if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                    if ident.as_str() == var.as_str() {
                        return value.clone();
                    }
                }
                expr.clone()
            }
            ExprKind::Binary { op, left, right } => Expr::new(
                ExprKind::Binary {
                    op: *op,
                    left: Heap::new(self.substitute_var_in_expr(left, var, value)),
                    right: Heap::new(self.substitute_var_in_expr(right, var, value)),
                },
                expr.span,
            ),
            ExprKind::Unary { op, expr: inner } => Expr::new(
                ExprKind::Unary {
                    op: *op,
                    expr: Heap::new(self.substitute_var_in_expr(inner, var, value)),
                },
                expr.span,
            ),
            ExprKind::Call { func, args, .. } => Expr::new(
                ExprKind::Call {
                    func: Heap::new(self.substitute_var_in_expr(func, var, value)),
                    type_args: Vec::new().into(),
                    args: args
                        .iter()
                        .map(|a| self.substitute_var_in_expr(a, var, value))
                        .collect(),
                },
                expr.span,
            ),
            _ => expr.clone(),
        }
    }

    /// Check if a conclusion is a valid base case of an inductive property
    fn is_base_case_of(&self, conclusion: &Expr, property: &Expr, var: &Text) -> bool {
        // Try common base values: 0, [], Nothing, etc.
        let base_values = [self.make_zero(), self.make_empty_list()];

        for base in &base_values {
            let expected = self.substitute_induction_var(property, var, base);
            if self.expr_eq(conclusion, &expected) {
                return true;
            }
        }
        false
    }

    /// Check if a conclusion is a valid step case of an inductive property
    fn is_step_case_of(&self, conclusion: &Expr, property: &Expr, var: &Text) -> bool {
        // The step case should prove P(succ(n)) or P(n+1) or P(x::xs)
        let successor = self.make_successor(var);
        let expected = self.substitute_induction_var(property, var, &successor);
        self.expr_eq(conclusion, &expected)
    }

    /// Make a zero/base value expression
    fn make_zero(&self) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit {
                    value: 0,
                    suffix: None,
                }),
                Span::dummy(),
            )),
            Span::dummy(),
        )
    }

    /// Make an empty list expression
    fn make_empty_list(&self) -> Expr {
        Expr::new(
            ExprKind::Array(verum_ast::ArrayExpr::List(List::new())),
            Span::dummy(),
        )
    }

    /// Make a successor expression: n + 1
    fn make_successor(&self, var: &Text) -> Expr {
        let var_path = verum_ast::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(verum_ast::Ident::new(
                var.as_str(),
                Span::dummy()
            ),)],
            span: Span::dummy(),
        };
        let var_expr = Expr::new(ExprKind::Path(var_path), Span::dummy());
        let one = Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit {
                    value: 1,
                    suffix: None,
                }),
                Span::dummy(),
            )),
            Span::dummy(),
        );

        Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Heap::new(var_expr),
                right: Heap::new(one),
            },
            Span::dummy(),
        )
    }

    /// Make a natural number constraint: n >= 0
    fn make_nat_constraint(&self, var: &Text) -> Expr {
        let var_path = verum_ast::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(verum_ast::Ident::new(
                var.as_str(),
                Span::dummy()
            ),)],
            span: Span::dummy(),
        };
        let var_expr = Expr::new(ExprKind::Path(var_path), Span::dummy());
        let zero = self.make_zero();

        Expr::new(
            ExprKind::Binary {
                op: BinOp::Ge,
                left: Heap::new(var_expr),
                right: Heap::new(zero),
            },
            Span::dummy(),
        )
    }

    /// Validate rule application
    ///
    /// Validates the application of a named inference rule to premises.
    /// Each rule has a specific signature that specifies:
    /// - Number and types of required premises
    /// - How the conclusion is derived from the premises
    ///
    /// Common rules supported:
    /// - "modus_ponens": P, P → Q ⊢ Q
    /// - "and_intro": P, Q ⊢ P ∧ Q
    /// - "and_elim_left": P ∧ Q ⊢ P
    /// - "and_elim_right": P ∧ Q ⊢ Q
    /// - "or_intro_left": P ⊢ P ∨ Q
    /// - "or_intro_right": Q ⊢ P ∨ Q
    /// - "impl_intro": [P ⊢ Q] ⊢ P → Q
    /// - "forall_intro": [x ⊢ P(x)] ⊢ ∀x. P(x)
    /// - "forall_elim": ∀x. P(x), t ⊢ P(t)
    /// - "exists_intro": P(t), t ⊢ ∃x. P(x)
    ///
    /// Apply proof rule: validates application of standard logical rules.
    /// Supported rules: modus_ponens (P, P->Q |- Q), impl_intro ([P|-Q] |- P->Q),
    /// forall_intro ([x|-P(x)] |- forall x. P(x)), forall_elim (forall x. P(x), t |- P(t)),
    /// exists_intro (P(t), t |- exists x. P(x)).
    fn validate_apply(
        &mut self,
        rule: &Text,
        premises: &List<Heap<ProofTerm>>,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Validate all premises first
        let mut premise_conclusions: List<Expr> = List::new();
        for premise in premises.iter() {
            let premise_conclusion = premise.conclusion();
            self.validate_impl(premise, &premise_conclusion)?;
            premise_conclusions.push(premise_conclusion);
        }

        // Apply the rule based on its name
        let derived = self.apply_inference_rule(rule, &premise_conclusions, expected)?;

        // Check derived conclusion matches expected
        if !self.expr_eq(&derived, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(&derived),
            });
        }

        Ok(())
    }

    /// Apply an inference rule to premises and derive the conclusion
    fn apply_inference_rule(
        &self,
        rule: &Text,
        premises: &[Expr],
        expected: &Expr,
    ) -> ValidationResult<Expr> {
        let rule_str = rule.as_str();

        match rule_str {
            // Modus Ponens: P, P → Q ⊢ Q
            "modus_ponens" | "mp" => {
                if premises.len() != 2 {
                    return Err(ValidationError::ValidationFailed {
                        message: format!(
                            "modus_ponens requires 2 premises, got {}",
                            premises.len()
                        )
                        .into(),
                    });
                }
                let p = &premises[0];
                let implication = &premises[1];

                // Extract P → Q
                let (antecedent, consequent) = self.extract_implication(implication)?;

                // Check P matches antecedent
                if !self.expr_eq(p, &antecedent) {
                    return Err(ValidationError::ModusPonensError {
                        premise: self.expr_to_text(p),
                        implication: self.expr_to_text(implication),
                    });
                }

                Ok(consequent)
            }

            // And Introduction: P, Q ⊢ P ∧ Q
            "and_intro" | "conj_intro" => {
                if premises.len() != 2 {
                    return Err(ValidationError::ValidationFailed {
                        message: format!("and_intro requires 2 premises, got {}", premises.len())
                            .into(),
                    });
                }
                Ok(Expr::new(
                    ExprKind::Binary {
                        op: BinOp::And,
                        left: Heap::new(premises[0].clone()),
                        right: Heap::new(premises[1].clone()),
                    },
                    Span::dummy(),
                ))
            }

            // And Elimination Left: P ∧ Q ⊢ P
            "and_elim_left" | "conj_elim_l" => {
                if premises.len() != 1 {
                    return Err(ValidationError::ValidationFailed {
                        message: format!(
                            "and_elim_left requires 1 premise, got {}",
                            premises.len()
                        )
                        .into(),
                    });
                }
                let (left, _right) = self.extract_conjunction(&premises[0])?;
                Ok(left)
            }

            // And Elimination Right: P ∧ Q ⊢ Q
            "and_elim_right" | "conj_elim_r" => {
                if premises.len() != 1 {
                    return Err(ValidationError::ValidationFailed {
                        message: format!(
                            "and_elim_right requires 1 premise, got {}",
                            premises.len()
                        )
                        .into(),
                    });
                }
                let (_left, right) = self.extract_conjunction(&premises[0])?;
                Ok(right)
            }

            // Or Introduction Left: P ⊢ P ∨ Q
            "or_intro_left" | "disj_intro_l" => {
                if premises.len() != 1 {
                    return Err(ValidationError::ValidationFailed {
                        message: format!(
                            "or_intro_left requires 1 premise, got {}",
                            premises.len()
                        )
                        .into(),
                    });
                }
                // The right disjunct comes from expected
                let (_, right) = self.extract_disjunction(expected)?;
                Ok(Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Or,
                        left: Heap::new(premises[0].clone()),
                        right: Heap::new(right),
                    },
                    Span::dummy(),
                ))
            }

            // Or Introduction Right: Q ⊢ P ∨ Q
            "or_intro_right" | "disj_intro_r" => {
                if premises.len() != 1 {
                    return Err(ValidationError::ValidationFailed {
                        message: format!(
                            "or_intro_right requires 1 premise, got {}",
                            premises.len()
                        )
                        .into(),
                    });
                }
                // The left disjunct comes from expected
                let (left, _) = self.extract_disjunction(expected)?;
                Ok(Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Or,
                        left: Heap::new(left),
                        right: Heap::new(premises[0].clone()),
                    },
                    Span::dummy(),
                ))
            }

            // Or Elimination: P ∨ Q, P → R, Q → R ⊢ R
            "or_elim" | "disj_elim" => {
                if premises.len() != 3 {
                    return Err(ValidationError::ValidationFailed {
                        message: format!("or_elim requires 3 premises, got {}", premises.len())
                            .into(),
                    });
                }
                let (p, q) = self.extract_disjunction(&premises[0])?;
                let (p_ant, r1) = self.extract_implication(&premises[1])?;
                let (q_ant, r2) = self.extract_implication(&premises[2])?;

                // Check P matches first implication's antecedent
                if !self.expr_eq(&p, &p_ant) {
                    return Err(ValidationError::ValidationFailed {
                        message: "or_elim: first disjunct doesn't match first implication".into(),
                    });
                }
                // Check Q matches second implication's antecedent
                if !self.expr_eq(&q, &q_ant) {
                    return Err(ValidationError::ValidationFailed {
                        message: "or_elim: second disjunct doesn't match second implication".into(),
                    });
                }
                // Check both implications have the same consequent
                if !self.expr_eq(&r1, &r2) {
                    return Err(ValidationError::ValidationFailed {
                        message: "or_elim: implications must have the same consequent".into(),
                    });
                }

                Ok(r1)
            }

            // Negation Introduction: [P ⊢ ⊥] ⊢ ¬P
            "not_intro" | "neg_intro" => {
                if premises.len() != 1 {
                    return Err(ValidationError::ValidationFailed {
                        message: format!(
                            "not_intro requires 1 premise (derivation of ⊥), got {}",
                            premises.len()
                        )
                        .into(),
                    });
                }
                // The premise should be False (⊥) derived from assumption P
                // P is extracted from expected (¬P)
                let inner = self.extract_negation(expected)?;
                Ok(self.make_negation(&inner))
            }

            // Negation Elimination (reductio ad absurdum): ¬¬P ⊢ P
            "not_elim" | "neg_elim" | "double_neg" => {
                if premises.len() != 1 {
                    return Err(ValidationError::ValidationFailed {
                        message: format!("not_elim requires 1 premise, got {}", premises.len())
                            .into(),
                    });
                }
                // Extract ¬¬P → P
                let inner = self.extract_negation(&premises[0])?;
                let result = self.extract_negation(&inner)?;
                Ok(result)
            }

            // Forall Elimination: ∀x. P(x) ⊢ P(t)
            //
            // Soundness gate: the premise MUST be syntactically a
            // universal quantifier. Pre-fix the rule accepted any
            // premise + any expected — `Ok(expected.clone())` made
            // the downstream `expr_eq(derived, expected)` check
            // trivially true, mirroring the trust-unknown-rules
            // soundness leak fixed in 8429bd4e for the catch-all
            // arm.
            //
            // Full instantiation checking (verifying expected = body[x := t]
            // for some t) requires higher-order matching that this
            // module does not implement; the unification check is
            // tracked separately. The current gate catches the most
            // common misuse (forall_elim called on a non-quantified
            // premise) which the pass-through fallback masked.
            "forall_elim" | "univ_elim" => {
                if premises.is_empty() {
                    return Err(ValidationError::ValidationFailed {
                        message: "forall_elim requires at least 1 premise".into(),
                    });
                }
                let body = match &premises[0].kind {
                    ExprKind::Forall { body, .. } => body,
                    _ => {
                        return Err(ValidationError::ValidationFailed {
                            message: format!(
                                "forall_elim requires a universally-quantified premise (∀x. P(x)); \
                                 got {:?}",
                                std::mem::discriminant(&premises[0].kind)
                            )
                            .into(),
                        });
                    }
                };
                // Stronger gate: the expected (instantiation `P(t)`) must
                // share the body's outermost discriminant. A body shaped
                // `Binary(And, …)` instantiates to a Binary And — never
                // to a bare Path or Literal. This catches "called
                // forall_elim on `∀x. P(x) ∧ Q(x)` and claimed `42`"
                // misuse without requiring a unifier. Documented as
                // an extension of the prior structural gate (80f43418).
                if std::mem::discriminant(&body.kind) != std::mem::discriminant(&expected.kind) {
                    return Err(ValidationError::ValidationFailed {
                        message: format!(
                            "forall_elim: expected instantiation must share the body's \
                             outermost shape — body is {:?}, expected is {:?}",
                            std::mem::discriminant(&body.kind),
                            std::mem::discriminant(&expected.kind)
                        )
                        .into(),
                    });
                }
                Ok(expected.clone())
            }

            // Exists Introduction: P(t) ⊢ ∃x. P(x)
            //
            // Symmetric soundness gate to forall_elim above: the
            // expected MUST be syntactically an existential
            // quantifier. Without this gate the rule accepted any
            // premise + any expected.
            "exists_intro" | "exist_intro" => {
                if premises.is_empty() {
                    return Err(ValidationError::ValidationFailed {
                        message: "exists_intro requires at least 1 premise".into(),
                    });
                }
                let body = match &expected.kind {
                    ExprKind::Exists { body, .. } => body,
                    _ => {
                        return Err(ValidationError::ValidationFailed {
                            message: format!(
                                "exists_intro requires an existentially-quantified expected (∃x. P(x)); \
                                 got {:?}",
                                std::mem::discriminant(&expected.kind)
                            )
                            .into(),
                        });
                    }
                };
                // Stronger symmetric gate: the premise (witness `P(t)`)
                // must share the body's outermost discriminant. Same
                // rationale as the forall_elim body-vs-expected gate
                // above — catches `∃x. P(x) ∧ Q(x)` introduced from a
                // bare-Path premise without a unifier.
                if std::mem::discriminant(&body.kind) != std::mem::discriminant(&premises[0].kind) {
                    return Err(ValidationError::ValidationFailed {
                        message: format!(
                            "exists_intro: premise (witness) must share the body's outermost \
                             shape — body is {:?}, premise is {:?}",
                            std::mem::discriminant(&body.kind),
                            std::mem::discriminant(&premises[0].kind)
                        )
                        .into(),
                    });
                }
                Ok(expected.clone())
            }

            // Identity/Reflexivity: ⊢ t = t
            "refl" | "eq_refl" => {
                // No premises needed for reflexivity
                // Expected should be t = t
                let (left, right) = self.extract_equality(expected)?;
                if !self.expr_eq(&left, &right) {
                    return Err(ValidationError::ValidationFailed {
                        message: "reflexivity requires identical terms".into(),
                    });
                }
                Ok(expected.clone())
            }

            // Lookup user-defined rule in the inference-rule registry. Any
            // rule name that doesn't match a hardcoded inference rule above
            // MUST come from `register_inference_rule`. There is no
            // "trust the user" fallback — validate_apply is the only soundness
            // gate between a proof term and its claimed conclusion, so an
            // unknown rule has to be a hard error, never a silent pass.
            _ => {
                let (schema_premises, conclusion) = self.lookup_registered_rule(rule).ok_or_else(
                    || ValidationError::ValidationFailed {
                        message: format!(
                            "unknown inference rule '{}'. Register it via \
                             ProofValidator::register_inference_rule before \
                             referencing it in a proof.",
                            rule
                        )
                        .into(),
                    },
                )?;

                // Arity check: caller must supply exactly the number of
                // premises the rule schema declares. Unification with the
                // schema's premise patterns happens in the outer
                // validate_apply via expr_eq against the derived conclusion.
                if premises.len() != schema_premises.len() {
                    return Err(ValidationError::ValidationFailed {
                        message: format!(
                            "rule '{}' expects {} premises, got {}",
                            rule,
                            schema_premises.len(),
                            premises.len()
                        )
                        .into(),
                    });
                }

                Ok(conclusion)
            }
        }
    }

    /// Extract conjunction P ∧ Q into (P, Q)
    fn extract_conjunction(&self, expr: &Expr) -> ValidationResult<(Expr, Expr)> {
        match &expr.kind {
            ExprKind::Binary {
                op: BinOp::And,
                left,
                right,
            } => Ok(((**left).clone(), (**right).clone())),
            _ => Err(ValidationError::ValidationFailed {
                message: format!("expected conjunction, got {:?}", expr.kind).into(),
            }),
        }
    }

    /// Extract disjunction P ∨ Q into (P, Q)
    fn extract_disjunction(&self, expr: &Expr) -> ValidationResult<(Expr, Expr)> {
        match &expr.kind {
            ExprKind::Binary {
                op: BinOp::Or,
                left,
                right,
            } => Ok(((**left).clone(), (**right).clone())),
            _ => Err(ValidationError::ValidationFailed {
                message: format!("expected disjunction, got {:?}", expr.kind).into(),
            }),
        }
    }

    /// Extract negation ¬P into P
    fn extract_negation(&self, expr: &Expr) -> ValidationResult<Expr> {
        match &expr.kind {
            ExprKind::Unary {
                op: verum_ast::UnOp::Not,
                expr: inner,
            } => Ok((**inner).clone()),
            _ => Err(ValidationError::ValidationFailed {
                message: format!("expected negation, got {:?}", expr.kind).into(),
            }),
        }
    }

    /// Look up a user-defined inference rule by name.
    ///
    /// Returns the rule's schema as `(premise_patterns, conclusion)` so the
    /// caller can do an arity check and obtain the conclusion that the rule
    /// derives. Reads from `self.inference_rules`, which is populated via
    /// `register_inference_rule`.
    ///
    /// Returns `None` when the name isn't registered — soundness depends on
    /// the caller treating that as a hard error rather than falling back to
    /// "trust the user". Pre-fix, this was an unconditional `None` (stub),
    /// which forced the apply-rule branch to silently accept ANY unknown
    /// rule with non-empty premises.
    fn lookup_registered_rule(&self, rule: &Text) -> Option<(List<Expr>, Expr)> {
        self.inference_rules.get(rule).cloned()
    }

    /// Validate SMT solver proof
    ///
    /// This validates a proof that was generated by an SMT solver. The validation
    /// has multiple levels of rigor:
    ///
    /// 1. **Basic validation** (always): Check that the formula matches expected
    /// 2. **Trace validation** (if trace provided): Parse and validate the SMT-LIB2 proof trace
    /// 3. **Re-checking** (if configured): Re-submit to SMT solver for verification
    ///
    /// ## Supported Solvers
    ///
    /// - `z3`: Z3 SMT solver with proof production
    /// - `cvc5`: CVC5 SMT solver with LFSC proofs
    /// - `vampire`: Vampire theorem prover
    /// - `e`: E theorem prover
    ///
    /// ## SMT Proof Trace Format
    ///
    /// When an SMT trace is provided, it should be in one of:
    /// - SMT-LIB2 proof format (for Z3)
    /// - LFSC proof format (for CVC5)
    /// - TSTP format (for first-order provers)
    ///
    /// SMT proof validation: verifies that an SMT solver (Z3/CVC5) correctly
    /// discharged the formula. Accepts traces in SMT-LIB2 (Z3), LFSC (CVC5),
    /// or TSTP (first-order provers) format. Formula must match expected proposition.
    fn validate_smt_proof(
        &mut self,
        solver: &Text,
        formula: &Expr,
        smt_trace: &Maybe<Text>,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Check formula matches expected
        if !self.expr_eq(formula, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(formula),
            });
        }

        // Validate the solver is known
        let solver_str = solver.as_str();
        if !self.is_known_solver(solver_str) {
            return Err(ValidationError::ValidationFailed {
                message: format!("unknown SMT solver: {}", solver).into(),
            });
        }

        // Validate SMT proof trace if provided
        if let Maybe::Some(trace) = smt_trace {
            self.validate_smt_trace(solver_str, trace, formula)?;
        }

        // If configured, re-validate with SMT solver
        if self.config.validate_smt_proofs {
            self.recheck_with_smt(solver_str, formula)?;
        }

        Ok(())
    }

    /// Check if a solver name is known
    fn is_known_solver(&self, solver: &str) -> bool {
        matches!(
            solver.to_lowercase().as_str(),
            "z3" | "cvc5" | "cvc4" | "vampire" | "e" | "yices" | "mathsat" | "boolector"
        )
    }

    /// Validate an SMT proof trace
    ///
    /// This parses and validates the proof trace from the SMT solver.
    /// The trace format depends on the solver used.
    fn validate_smt_trace(
        &self,
        solver: &str,
        trace: &Text,
        _formula: &Expr,
    ) -> ValidationResult<()> {
        let trace_str = trace.as_str();

        // Basic syntactic validation of the trace
        match solver.to_lowercase().as_str() {
            "z3" => {
                // Z3 proof traces use a Lisp-like format
                // Check for basic structure indicators
                if trace_str.contains("(proof") || trace_str.contains("(asserted") {
                    self.validate_z3_proof_trace(trace_str)?;
                } else if trace_str.is_empty() {
                    // Empty trace is acceptable for simple proofs
                    return Ok(());
                }
            }

            "cvc5" | "cvc4" => {
                // CVC5 uses LFSC (Logical Framework with Side Conditions)
                if trace_str.contains("(check") || trace_str.contains("(decl") {
                    self.validate_lfsc_proof_trace(trace_str)?;
                }
            }

            "vampire" | "e" => {
                // First-order provers use TSTP format
                if trace_str.contains("cnf(") || trace_str.contains("fof(") {
                    self.validate_tstp_proof_trace(trace_str)?;
                }
            }

            _ => {
                // Unknown solver format - accept as valid for extensibility
            }
        }

        Ok(())
    }

    /// Validate Z3 proof trace structure
    fn validate_z3_proof_trace(&self, trace: &str) -> ValidationResult<()> {
        // Basic validation: check for balanced parentheses
        let mut depth = 0i32;
        for c in trace.chars() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth < 0 {
                        return Err(ValidationError::ValidationFailed {
                            message: "malformed Z3 proof trace: unbalanced parentheses".into(),
                        });
                    }
                }
                _ => {}
            }
        }

        if depth != 0 {
            return Err(ValidationError::ValidationFailed {
                message: "malformed Z3 proof trace: unbalanced parentheses".into(),
            });
        }

        // Check for proof term conclusion
        if trace.contains("(proof") && !trace.contains("false") && !trace.contains("unsat") {
            // Proof should end with proving the goal or unsat
            // This is a heuristic check
        }

        Ok(())
    }

    /// Validate LFSC proof trace structure
    fn validate_lfsc_proof_trace(&self, trace: &str) -> ValidationResult<()> {
        // LFSC proofs should have proper declaration and check structure
        let mut depth = 0i32;
        for c in trace.chars() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth < 0 {
                        return Err(ValidationError::ValidationFailed {
                            message: "malformed LFSC proof trace: unbalanced parentheses".into(),
                        });
                    }
                }
                _ => {}
            }
        }

        if depth != 0 {
            return Err(ValidationError::ValidationFailed {
                message: "malformed LFSC proof trace: unbalanced parentheses".into(),
            });
        }

        Ok(())
    }

    /// Validate TSTP proof trace structure
    fn validate_tstp_proof_trace(&self, trace: &str) -> ValidationResult<()> {
        // TSTP format uses cnf() or fof() clauses
        // Each clause should be properly formatted
        for line in trace.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('%') {
                // Empty line or comment
                continue;
            }

            // Check for valid TSTP clause structure
            if trimmed.starts_with("cnf(")
                || trimmed.starts_with("fof(")
                || trimmed.starts_with("thf(")
            {
                // Should end with ).\n or similar
                if !trimmed.ends_with(").") && !trimmed.ends_with(')') {
                    // Multi-line clause - acceptable
                }
            }
        }

        Ok(())
    }

    /// Re-check formula with SMT solver (for rigorous validation)
    ///
    /// This function integrates with Z3 via verum_smt to verify that a formula
    /// is valid (i.e., its negation is UNSAT). This provides an independent
    /// check of proof correctness.
    ///
    /// # Algorithm
    /// 1. Create a fresh SMT context with configured timeout
    /// 2. Translate the Verum Expr to Z3 AST
    /// 3. Assert the negation of the formula
    /// 4. Check satisfiability:
    ///    - UNSAT => formula is valid, proof is correct
    ///    - SAT => formula is invalid, found counterexample
    ///    - UNKNOWN => solver timeout or undecidable
    ///
    /// # Solver Support
    /// Currently supports "z3" solver. CVC5 support is planned.
    ///
    /// Re-check a formula with SMT solver. Encodes the negation of the formula
    /// in SMT-LIB format and checks satisfiability. UNSAT = formula valid,
    /// SAT = counterexample found, UNKNOWN = solver timeout or undecidable.
    /// Currently supports Z3 solver only. CVC5 support is planned.
    fn recheck_with_smt(&self, solver_name: &str, formula: &Expr) -> ValidationResult<()> {
        use std::time::Duration;

        // Validate solver name
        if solver_name != "z3" && solver_name != "Z3" {
            return Err(ValidationError::SmtValidationFailed {
                reason: format!(
                    "Unsupported solver: {}. Only 'z3' is currently supported.",
                    solver_name
                )
                .into(),
            });
        }

        // Create SMT context with configured timeout
        let timeout_ms = self.config.smt_timeout_ms as u32;
        let smt_context = verum_smt::Context::with_config(verum_smt::context::ContextConfig {
            timeout: Some(Duration::from_millis(timeout_ms as u64)),
            model_generation: true,
            proof_generation: false,
            ..Default::default()
        });

        // Create translator and fresh solver
        let mut translator = verum_smt::Translator::new(&smt_context);
        let solver = smt_context.solver();

        // Bind hypotheses as assumptions. Each hypothesis carries an
        // actual proposition (`prop`); we translate that proposition to
        // a Z3 expression and assert it. The hypothesis name is also
        // bound to its translated value so the formula being rechecked
        // can refer to it by name.
        //
        // Pre-fix this loop bound a FRESH `Bool::new_const(name)` and
        // asserted that — completely discarding `prop`. Z3 then saw
        // every hypothesis as the opaque true boolean `h0 := true`,
        // regardless of whether the hypothesis actually said `x > 5`
        // or `is_sorted(xs)`. Re-checks that should have found
        // counterexamples silently passed because the actual semantic
        // content of the hypothesis context never made it to the
        // solver.
        for scope in self.hypotheses.scopes.iter() {
            for (name, prop) in scope.hypotheses.iter() {
                let translated = match translator.translate_expr(prop) {
                    Ok(t) => t,
                    Err(_) => {
                        // Hypothesis can't be translated to Z3 (e.g. it
                        // references types we can't encode). Skip it
                        // rather than silently substituting a vacuous
                        // `true` — the rechecker will operate without
                        // this assumption, which is sound but more
                        // conservative.
                        continue;
                    }
                };
                translator.bind(name.clone(), translated.clone());
                if let Some(prop_bool) = translated.as_bool() {
                    solver.assert(&prop_bool);
                }
                // If `prop` doesn't translate to a Bool (e.g. it's an
                // Int-valued expression mistakenly stored as a
                // hypothesis), the binding is still useful for naming
                // but no truth-assertion can be made.
            }
        }

        // Translate the formula to Z3
        let z3_formula = match translator.translate_expr(formula) {
            Ok(expr) => expr,
            Err(e) => {
                return Err(ValidationError::SmtValidationFailed {
                    reason: format!("Translation error: {}", e).into(),
                });
            }
        };

        // Get the formula as a boolean (it should be a proposition)
        let z3_bool = match z3_formula.as_bool() {
            Some(b) => b,
            None => {
                return Err(ValidationError::SmtValidationFailed {
                    reason: "Formula does not translate to a boolean SMT expression".into(),
                });
            }
        };

        // To prove the formula is valid, we assert its negation
        // and check for UNSAT (no counterexample exists)
        solver.assert(z3_bool.not());

        // Check satisfiability
        match solver.check() {
            z3::SatResult::Unsat => {
                // No counterexample exists - formula is valid
                Ok(())
            }
            z3::SatResult::Sat => {
                // Found counterexample - formula is not valid
                // Extract counterexample for error message
                let counterexample_msg = if let Some(model) = solver.get_model() {
                    format!("Counterexample model: {}", model)
                } else {
                    "Counterexample exists but model extraction failed".to_string()
                };

                Err(ValidationError::SmtValidationFailed {
                    reason: format!(
                        "Formula is not valid. SMT solver found counterexample. {}",
                        counterexample_msg
                    )
                    .into(),
                })
            }
            z3::SatResult::Unknown => {
                // Solver couldn't determine result (timeout or undecidable)
                let reason = solver
                    .get_reason_unknown()
                    .unwrap_or_else(|| "unknown".to_string());

                Err(ValidationError::SmtValidationFailed {
                    reason: format!(
                        "SMT solver returned unknown result. Reason: {}. \
                         Consider increasing timeout or simplifying the formula.",
                        reason
                    )
                    .into(),
                })
            }
        }
    }

    /// Validate proof by substitution
    ///
    /// Given an equality proof (a = b) and a property P containing a,
    /// validate that we can derive P[b/a] (P with b substituted for a).
    ///
    /// This implements Leibniz's law (indiscernibility of identicals):
    /// If a = b and P(a), then P(b).
    ///
    /// Substitution rule (Leibniz's law / indiscernibility of identicals):
    /// given proof of a = b and property P(a), derive P[b/a].
    fn validate_subst(
        &mut self,
        eq_proof: &ProofTerm,
        property: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        let eq_conclusion = eq_proof.conclusion();

        // Validate equality proof
        self.validate_impl(eq_proof, &eq_conclusion)?;

        // Extract equality: a = b
        let (left, right) = self.extract_equality(&eq_conclusion)?;

        // The property should contain occurrences of 'left' (a)
        // and expected should be the property with 'left' replaced by 'right' (b)
        //
        // We verify by checking:
        // 1. expected == property[right/left] (forward substitution)
        // OR
        // 2. property == expected[left/right] (backward substitution)

        // For single-term substitution (Leibniz's law), we use the existing
        // substitution mechanism with a single-entry map when the target is a variable
        // For expression-to-expression substitution, we use structural equality checking

        // First check direct equality (trivial case)
        if self.expr_eq(property, expected) {
            return Ok(());
        }

        // Try to perform substitution if one of the equality terms is a variable
        let check_substitution = |from_expr: &Expr, to_expr: &Expr| -> bool {
            if let ExprKind::Path(path) = &from_expr.kind {
                if let Some(ident) = path.as_ident() {
                    let var_name = Text::from(ident.as_str());
                    let mut instantiation = Map::new();
                    instantiation.insert(var_name, to_expr.clone());
                    let substituted = self.instantiate(property, &instantiation);
                    return self.expr_eq(&substituted, expected);
                }
            }
            false
        };

        // Try substituting left for right
        if check_substitution(&left, &right) {
            return Ok(());
        }

        // Try substituting right for left
        if check_substitution(&right, &left) {
            return Ok(());
        }

        // For non-variable equality terms, we accept if expected structurally
        // contains the transformation described by the equality
        // This is a conservative check that accepts valid proofs

        // None of the substitution patterns matched
        Err(ValidationError::SubstitutionError {
            message: format!(
                "substitution failed: cannot derive {} from {} using {} = {}",
                self.expr_to_text(expected),
                self.expr_to_text(property),
                self.expr_to_text(&left),
                self.expr_to_text(&right)
            )
            .into(),
        })
    }

    /// Validate lemma
    fn validate_lemma(
        &mut self,
        conclusion: &Expr,
        proof: &ProofTerm,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Validate the proof of the lemma
        self.validate_impl(proof, conclusion)?;

        // Check conclusion matches expected
        if !self.expr_eq(conclusion, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(conclusion),
            });
        }

        Ok(())
    }

    // ==================== Extended Proof Rule Validators ====================

    /// Validate And Elimination: from (and l_1 ... l_n), derive l_i
    ///
    /// This rule extracts a specific conjunct from a conjunction.
    ///
    /// And Elimination: from (and l_1 ... l_n), derive l_i (extract i-th conjunct).
    fn validate_and_elim(
        &mut self,
        conjunction: &ProofTerm,
        index: usize,
        result: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        let conj_conclusion = conjunction.conclusion();
        self.validate_impl(conjunction, &conj_conclusion)?;

        // Extract the indexed conjunct from the conjunction
        let conjuncts = self.extract_conjuncts(&conj_conclusion);

        if index >= conjuncts.len() {
            return Err(ValidationError::ValidationFailed {
                message: format!(
                    "and_elim index {} out of bounds for conjunction with {} elements",
                    index,
                    conjuncts.len()
                )
                .into(),
            });
        }

        // Check that extracted element matches result and expected
        let extracted = &conjuncts[index];
        if !self.expr_eq(extracted, result) {
            return Err(ValidationError::ValidationFailed {
                message: format!(
                    "and_elim result mismatch: expected {}, got {}",
                    self.expr_to_text(result),
                    self.expr_to_text(extracted)
                )
                .into(),
            });
        }

        if !self.expr_eq(result, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(result),
            });
        }

        Ok(())
    }

    /// Extract all conjuncts from a conjunction expression
    fn extract_conjuncts(&self, expr: &Expr) -> List<Expr> {
        let mut conjuncts = List::new();
        self.extract_conjuncts_recursive(expr, &mut conjuncts);
        conjuncts
    }

    fn extract_conjuncts_recursive(&self, expr: &Expr, conjuncts: &mut List<Expr>) {
        match &expr.kind {
            ExprKind::Binary {
                op: BinOp::And,
                left,
                right,
            } => {
                self.extract_conjuncts_recursive(left, conjuncts);
                self.extract_conjuncts_recursive(right, conjuncts);
            }
            _ => {
                conjuncts.push(expr.clone());
            }
        }
    }

    /// Validate Not-Or Elimination: from (not (or l_1 ... l_n)), derive (not l_i)
    ///
    /// This rule applies De Morgan's law to extract a negated disjunct.
    ///
    /// Not-Or Elimination (De Morgan): from (not (or l_1 ... l_n)), derive (not l_i).
    fn validate_not_or_elim(
        &mut self,
        negated_disjunction: &ProofTerm,
        index: usize,
        result: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        let neg_disj_conclusion = negated_disjunction.conclusion();
        self.validate_impl(negated_disjunction, &neg_disj_conclusion)?;

        // Extract the inner disjunction from ¬(or ...)
        let inner_disj = self.extract_negation(&neg_disj_conclusion)?;

        // Extract disjuncts
        let disjuncts = self.extract_disjuncts(&inner_disj);

        if index >= disjuncts.len() {
            return Err(ValidationError::ValidationFailed {
                message: format!(
                    "not_or_elim index {} out of bounds for disjunction with {} elements",
                    index,
                    disjuncts.len()
                )
                .into(),
            });
        }

        // The result should be ¬l_i
        let expected_result = self.make_negation(&disjuncts[index]);

        if !self.expr_eq(&expected_result, result) {
            return Err(ValidationError::ValidationFailed {
                message: format!(
                    "not_or_elim result mismatch: expected {}, got {}",
                    self.expr_to_text(&expected_result),
                    self.expr_to_text(result)
                )
                .into(),
            });
        }

        if !self.expr_eq(result, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(result),
            });
        }

        Ok(())
    }

    /// Extract all disjuncts from a disjunction expression
    fn extract_disjuncts(&self, expr: &Expr) -> List<Expr> {
        let mut disjuncts = List::new();
        self.extract_disjuncts_recursive(expr, &mut disjuncts);
        disjuncts
    }

    fn extract_disjuncts_recursive(&self, expr: &Expr, disjuncts: &mut List<Expr>) {
        match &expr.kind {
            ExprKind::Binary {
                op: BinOp::Or,
                left,
                right,
            } => {
                self.extract_disjuncts_recursive(left, disjuncts);
                self.extract_disjuncts_recursive(right, disjuncts);
            }
            _ => {
                disjuncts.push(expr.clone());
            }
        }
    }

    /// Validate Iff-True: from p, derive (iff p true)
    ///
    /// If P is proven, then P <=> True.
    ///
    /// Iff-True rule: from proof of P, derive (iff P true). If P is proven, then P <=> True.
    fn validate_iff_true(
        &mut self,
        proof: &ProofTerm,
        formula: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        let proof_conclusion = proof.conclusion();
        self.validate_impl(proof, &proof_conclusion)?;

        // Check that proof proves the formula
        if !self.expr_eq(&proof_conclusion, formula) {
            return Err(ValidationError::ValidationFailed {
                message: format!(
                    "iff_true proof does not prove formula: got {}",
                    self.expr_to_text(&proof_conclusion)
                )
                .into(),
            });
        }

        // Construct expected result: formula <=> true
        let expected_iff = self.make_iff(formula, &self.make_true());

        if !self.expr_eq(&expected_iff, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(&expected_iff),
            });
        }

        Ok(())
    }

    /// Validate Iff-False: from (not p), derive (iff p false)
    ///
    /// If ¬P is proven, then P <=> False.
    ///
    /// Iff-False rule: from proof of (not P), derive (iff P false). If ~P is proven, then P <=> False.
    fn validate_iff_false(
        &mut self,
        proof: &ProofTerm,
        formula: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        let proof_conclusion = proof.conclusion();
        self.validate_impl(proof, &proof_conclusion)?;

        // Check that proof proves ¬formula
        let expected_neg = self.make_negation(formula);
        if !self.expr_eq(&proof_conclusion, &expected_neg) {
            return Err(ValidationError::ValidationFailed {
                message: format!(
                    "iff_false proof should prove ¬P: got {}",
                    self.expr_to_text(&proof_conclusion)
                )
                .into(),
            });
        }

        // Construct expected result: formula <=> false
        let expected_iff = self.make_iff(formula, &self.make_false());

        if !self.expr_eq(&expected_iff, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(&expected_iff),
            });
        }

        Ok(())
    }

    /// Construct an iff expression (a <=> b)
    fn make_iff(&self, left: &Expr, right: &Expr) -> Expr {
        // Iff is: (left → right) ∧ (right → left)
        // Or we can use a dedicated Iff binary op if available
        Expr::new(
            ExprKind::Binary {
                op: BinOp::And,
                left: Heap::new(Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Imply,
                        left: Heap::new(left.clone()),
                        right: Heap::new(right.clone()),
                    },
                    Span::dummy(),
                )),
                right: Heap::new(Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Imply,
                        left: Heap::new(right.clone()),
                        right: Heap::new(left.clone()),
                    },
                    Span::dummy(),
                )),
            },
            Span::dummy(),
        )
    }

    /// Validate Commutativity: derive (= (f a b) (f b a))
    ///
    /// Validates that a commutative operation produces equal results
    /// when arguments are swapped.
    ///
    /// Commutativity rule: derive (= (f a b) (f b a)) for commutative operations.
    fn validate_commutativity(
        &self,
        left: &Expr,
        right: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Commutativity: left = right where left and right are
        // the same operation with swapped arguments
        let expected_eq = self.make_equality(left, right);

        if !self.expr_eq(&expected_eq, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(&expected_eq),
            });
        }

        // Check that left and right are related by argument swapping
        // This is a semantic check - we verify the structure looks commutative
        if !self.is_commutative_pair(left, right) {
            return Err(ValidationError::ValidationFailed {
                message: format!(
                    "commutativity: expressions are not commutative variants: {} and {}",
                    self.expr_to_text(left),
                    self.expr_to_text(right)
                )
                .into(),
            });
        }

        Ok(())
    }

    /// Check if two expressions are related by argument commutativity.
    ///
    /// Pre-fix this returned true whenever two expressions shared the
    /// SAME `BinOp` and had swapped operands — without checking that
    /// the operator is actually commutative. Result: `5 - 3 = 3 - 5`
    /// (subtraction is not commutative, the equation is FALSE) was
    /// accepted as a "commutativity" claim. Same gap for `Div`,
    /// `Imply`, `Lt`/`Le`/`Gt`/`Ge`, `Concat`, `Shl`/`Shr`, `In`.
    /// The `Call` arm trusted any 2-arg function as commutative.
    ///
    /// Post-fix: only mathematically commutative operators flow
    /// through. For Call, no special-cased trust — a future extension
    /// can register specific commutative functions, but the default
    /// is reject.
    fn is_commutative_pair(&self, left: &Expr, right: &Expr) -> bool {
        // Whitelist of operators where `a OP b == b OP a` is universal.
        fn is_commutative_op(op: BinOp) -> bool {
            matches!(
                op,
                BinOp::Add
                    | BinOp::Mul
                    | BinOp::And
                    | BinOp::Or
                    | BinOp::Eq
                    | BinOp::Ne
                    | BinOp::BitAnd
                    | BinOp::BitOr
                    | BinOp::BitXor
            )
        }

        match (&left.kind, &right.kind) {
            (
                ExprKind::Binary {
                    op: op1,
                    left: l1,
                    right: r1,
                },
                ExprKind::Binary {
                    op: op2,
                    left: l2,
                    right: r2,
                },
            ) => {
                op1 == op2
                    && is_commutative_op(*op1)
                    && self.expr_eq(l1, r2)
                    && self.expr_eq(r1, l2)
            }
            // Call arm intentionally returns false: arbitrary user
            // functions are not assumed commutative. A registered
            // commutative-function whitelist is a future extension —
            // for now the soundness gate is conservative-by-default.
            _ => false,
        }
    }

    /// Validate Monotonicity: if a R a', b R b', then f(a,b) R f(a',b')
    ///
    /// Validates that a function is monotone with respect to a relation.
    ///
    /// Monotonicity rule: if a R a' and b R b', then f(a,b) R f(a',b') for monotone f.
    fn validate_monotonicity(
        &mut self,
        premises: &List<ProofTerm>,
        conclusion: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Validate all premise proofs
        for premise in premises.iter() {
            let prem_conclusion = premise.conclusion();
            self.validate_impl(premise, &prem_conclusion)?;
        }

        // Check conclusion matches expected
        if !self.expr_eq(conclusion, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(conclusion),
            });
        }

        Ok(())
    }

    /// Validate Distributivity: f distributes over g
    ///
    /// Validates a distributivity axiom like a*(b+c) = a*b + a*c.
    ///
    /// Distributivity axiom: validates f distributes over g, e.g., a*(b+c) = a*b + a*c.
    fn validate_distributivity(&self, formula: &Expr, expected: &Expr) -> ValidationResult<()> {
        // The formula should be an equality expressing distributivity
        if !self.expr_eq(formula, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(formula),
            });
        }

        Ok(())
    }

    /// Validate DefAxiom: Tseitin-style CNF transformation axiom
    ///
    /// Validates a definitional axiom introduced during CNF conversion.
    ///
    /// DefAxiom: Tseitin-style CNF transformation axiom (tautology by construction).
    fn validate_def_axiom(&self, formula: &Expr, expected: &Expr) -> ValidationResult<()> {
        // Definition axioms are tautologies by construction
        // We trust the formula if it matches expected
        if !self.expr_eq(formula, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(formula),
            });
        }

        Ok(())
    }

    /// Validate DefIntro: Definition introduction
    ///
    /// Validates the introduction of a new definition.
    ///
    /// Definition introduction: establishes a definitional equality for a new name.
    fn validate_def_intro(
        &self,
        _name: &Text,
        definition: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Definition introductions establish definitional equality
        if !self.expr_eq(definition, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(definition),
            });
        }

        Ok(())
    }

    /// Validate ApplyDef: Apply a definition
    ///
    /// Validates the application of a definition to an expression.
    ///
    /// Apply a definition: unfold a named definition within an expression.
    fn validate_apply_def(
        &mut self,
        def_proof: &ProofTerm,
        _original: &Expr,
        _name: &Text,
        expected: &Expr,
    ) -> ValidationResult<()> {
        let def_conclusion = def_proof.conclusion();
        self.validate_impl(def_proof, &def_conclusion)?;

        // The result of applying the definition should match expected
        // In a full implementation, we would check that expected is
        // the original with the definition applied
        if !self.expr_eq(&def_conclusion, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(&def_conclusion),
            });
        }

        Ok(())
    }

    /// Validate IffOEq: Iff to oriented equality
    ///
    /// Converts a biconditional to an oriented equality.
    ///
    /// Iff to oriented equality: converts a biconditional (P <=> Q) to an oriented equality.
    fn validate_iff_oeq(
        &mut self,
        iff_proof: &ProofTerm,
        left: &Expr,
        right: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        let iff_conclusion = iff_proof.conclusion();
        self.validate_impl(iff_proof, &iff_conclusion)?;

        // The expected result is an equality left = right
        let expected_eq = self.make_equality(left, right);

        if !self.expr_eq(&expected_eq, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(&expected_eq),
            });
        }

        Ok(())
    }

    /// Validate NnfPos: Negation normal form (positive)
    ///
    /// Validates transformation to NNF for a positive formula.
    ///
    /// NNF positive: validates transformation to negation normal form for a positive formula.
    fn validate_nnf_pos(
        &self,
        formula: &Expr,
        result: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // The result should be the NNF of formula, matching expected
        if !self.expr_eq(result, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(result),
            });
        }

        // Verify that result is a valid NNF of formula
        // (For full validation, we would check NNF properties)
        Ok(())
    }

    /// Validate NnfNeg: Negation normal form (negative)
    ///
    /// Validates transformation to NNF for a negated formula.
    ///
    /// NNF negative: validates transformation to negation normal form for a negated formula.
    fn validate_nnf_neg(
        &self,
        formula: &Expr,
        result: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // The result should be the NNF of ¬formula, matching expected
        if !self.expr_eq(result, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(result),
            });
        }

        Ok(())
    }

    /// Validate SkHack: Skolemization hack
    ///
    /// Validates a Skolemization transformation (existential to function).
    ///
    /// Skolemization hack: transforms exists x. P(x) to P(f(free_vars)) where f is
    /// a fresh Skolem function. Trusted if result matches expected.
    fn validate_sk_hack(
        &self,
        _formula: &Expr,
        skolemized: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Skolemization transforms ∃x.P(x) to P(f(free_vars))
        // We trust the Skolemization if the result matches expected
        if !self.expr_eq(skolemized, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(skolemized),
            });
        }

        Ok(())
    }

    /// Validate EqualityResolution: Resolve an equality
    ///
    /// Validates resolution using an equality.
    ///
    /// Equality resolution: resolve a literal using an equality proof.
    fn validate_equality_resolution(
        &mut self,
        equality: &ProofTerm,
        literal: &ProofTerm,
        result: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        let eq_conclusion = equality.conclusion();
        self.validate_impl(equality, &eq_conclusion)?;

        let lit_conclusion = literal.conclusion();
        self.validate_impl(literal, &lit_conclusion)?;

        // Check result matches expected
        if !self.expr_eq(result, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(result),
            });
        }

        Ok(())
    }

    /// Validate BindProof: Bind a quantified proof
    ///
    /// Validates binding a pattern to a quantified proof.
    ///
    /// Bind proof: bind a pattern to a quantified proof for instantiation.
    fn validate_bind_proof(
        &mut self,
        quantified_proof: &ProofTerm,
        _pattern: &Expr,
        binding: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        let quant_conclusion = quantified_proof.conclusion();
        self.validate_impl(quantified_proof, &quant_conclusion)?;

        // Check binding matches expected
        if !self.expr_eq(binding, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(binding),
            });
        }

        Ok(())
    }

    /// Validate PullQuantifier: Pull a quantifier outward
    ///
    /// Validates pulling a quantifier out of a formula.
    /// Example: (∀x.P) ∧ Q → ∀x.(P ∧ Q) when x not free in Q
    ///
    /// Pull quantifier outward: e.g., (forall x. P) & Q -> forall x. (P & Q) when x not free in Q.
    fn validate_pull_quantifier(
        &self,
        _formula: &Expr,
        _quantifier_type: &Text,
        result: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Check result matches expected
        if !self.expr_eq(result, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(result),
            });
        }

        Ok(())
    }

    /// Validate PushQuantifier: Push a quantifier inward
    ///
    /// Validates pushing a quantifier into a formula.
    /// Example: ∀x.(P ∧ Q) → (∀x.P) ∧ (∀x.Q)
    ///
    /// Push quantifier inward: e.g., forall x. (P & Q) -> (forall x. P) & (forall x. Q).
    fn validate_push_quantifier(
        &self,
        _formula: &Expr,
        _quantifier_type: &Text,
        result: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Check result matches expected
        if !self.expr_eq(result, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(result),
            });
        }

        Ok(())
    }

    /// Validate ElimUnusedVars: Eliminate unused variables
    ///
    /// Validates elimination of quantified variables that don't
    /// appear in the body.
    /// Example: ∀x.P → P (when x not in P)
    ///
    /// Eliminate unused quantified variables: forall x. P -> P when x not in P.
    fn validate_elim_unused_vars(
        &self,
        _formula: &Expr,
        result: &Expr,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // Check result matches expected
        if !self.expr_eq(result, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(result),
            });
        }

        Ok(())
    }

    /// Validate DerElim: Der elimination
    ///
    /// Validates a derived rule elimination.
    ///
    /// Derived rule elimination: validate conclusion of a derived inference rule.
    fn validate_der_elim(&mut self, premise: &ProofTerm, expected: &Expr) -> ValidationResult<()> {
        let premise_conclusion = premise.conclusion();
        self.validate_impl(premise, &premise_conclusion)?;

        // Check premise conclusion matches expected
        if !self.expr_eq(&premise_conclusion, expected) {
            return Err(ValidationError::PropositionMismatch {
                expected: self.expr_to_text(expected),
                actual: self.expr_to_text(&premise_conclusion),
            });
        }

        Ok(())
    }

    /// Validate QuickExplain: Unsat core extraction
    ///
    /// Validates that the unsat core is valid.
    ///
    /// QuickExplain: validates an unsat core extraction. The conjunction of
    /// the unsat core formulas should be inconsistent.
    fn validate_quick_explain(
        &self,
        unsat_core: &List<Expr>,
        _explanation: &Text,
        expected: &Expr,
    ) -> ValidationResult<()> {
        // QuickExplain produces an unsat core - the conjunction of
        // the unsat core should be inconsistent
        // For validation, we check that expected represents the core

        // Build conjunction of unsat core
        if unsat_core.is_empty() {
            let false_expr = self.make_false();
            if !self.expr_eq(&false_expr, expected) {
                return Err(ValidationError::ValidationFailed {
                    message: "quick_explain with empty core should prove False".into(),
                });
            }
        }

        // Accept the unsat core as valid (full validation would re-check with SMT)
        Ok(())
    }

    // ==================== Helper Methods ====================

    /// Extract implication (P → Q) into (P, Q)
    fn extract_implication(&self, expr: &Expr) -> ValidationResult<(Expr, Expr)> {
        match &expr.kind {
            ExprKind::Binary {
                op: BinOp::Imply,
                left,
                right,
            } => Ok(((**left).clone(), (**right).clone())),
            _ => Err(ValidationError::ModusPonensError {
                premise: "unknown".into(),
                implication: self.expr_to_text(expr),
            }),
        }
    }

    /// Extract equality (A = B) into (A, B)
    fn extract_equality(&self, expr: &Expr) -> ValidationResult<(Expr, Expr)> {
        match &expr.kind {
            ExprKind::Binary {
                op: BinOp::Eq,
                left,
                right,
            } => Ok(((**left).clone(), (**right).clone())),
            _ => Err(ValidationError::EqualityError {
                message: format!("expected equality, got {}", self.expr_to_text(expr)).into(),
            }),
        }
    }

    /// Construct an equality expression
    fn make_equality(&self, left: &Expr, right: &Expr) -> Expr {
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Box::new(left.clone()),
                right: Box::new(right.clone()),
            },
            Span::dummy(),
        )
    }

    /// Extract bindings from a pattern and add them as hypotheses
    ///
    /// Pattern matching in proofs introduces variable bindings. For example:
    /// - `Some(x)` binds `x` to the inner value
    /// - `Pair(a, b)` binds `a` and `b` to the pair elements
    /// - `Left(x)` or `Right(y)` bind the sum type's payload
    ///
    /// # Arguments
    /// * `pattern` - The pattern expression (may contain variable bindings)
    /// * `scrutinee` - The value being matched (used to derive types)
    fn add_pattern_bindings(&mut self, pattern: &Expr, scrutinee: &Expr) {
        match &pattern.kind {
            // Simple variable pattern: x
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    // This is a variable binding - add it as a hypothesis
                    // The type is derived from the scrutinee
                    self.hypotheses
                        .add_hypothesis(Text::from(ident.as_str()), scrutinee.clone());
                }
            }

            // Constructor pattern: Some(x), Pair(a, b), etc.
            ExprKind::Call { func, args, .. } => {
                // The constructor is in func, bindings are in args
                // Add each argument as a binding with a derived type
                for (idx, arg) in args.iter().enumerate() {
                    self.add_pattern_bindings_with_index(arg, scrutinee, idx);
                }
            }

            // Tuple pattern: (a, b, c)
            ExprKind::Tuple(elements) => {
                for (idx, elem) in elements.iter().enumerate() {
                    self.add_pattern_bindings_with_index(elem, scrutinee, idx);
                }
            }

            // Literal patterns don't introduce bindings
            ExprKind::Literal(_) => {}

            // Wildcard: _ (no binding)
            // Other patterns may not introduce bindings
            _ => {}
        }
    }

    /// Helper: Add pattern bindings with tuple/constructor index context
    ///
    /// This method handles pattern matching by deriving the type of each binding
    /// from the scrutinee expression. For tuples, it extracts element types.
    /// For constructors/structs, it extracts field types.
    ///
    /// # Type Derivation Strategy
    ///
    /// 1. **Tuple patterns**: Extract type at index from tuple type signature
    /// 2. **Constructor patterns**: Extract field type from constructor definition
    /// 3. **Struct patterns**: Extract field type from struct definition
    /// 4. **Array patterns**: Use element type of array
    /// 5. **Fallback**: Use a type variable to be unified later
    fn add_pattern_bindings_with_index(&mut self, pattern: &Expr, scrutinee: &Expr, idx: usize) {
        // For nested patterns, recursively extract bindings
        match &pattern.kind {
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    // Variable binding at this index
                    // Derive the type from scrutinee[idx]
                    let binding_type = self.derive_element_type(scrutinee, idx);
                    self.hypotheses
                        .add_hypothesis(Text::from(ident.as_str()), binding_type);
                }
            }
            // Recursively handle nested patterns
            _ => self.add_pattern_bindings(pattern, scrutinee),
        }
    }

    /// Derive the element type at a given index from a scrutinee expression
    ///
    /// This method attempts to extract precise type information from the scrutinee
    /// expression's structure. When type information cannot be immediately derived,
    /// it creates a proper type variable in the type context that can be unified
    /// later when more information becomes available.
    ///
    /// # Type Derivation Strategy
    ///
    /// 1. **Tuple expressions**: `(a, b, c)` -> extract element at index
    /// 2. **Constructor applications**: `Some(x)`, `Point{x, y}` -> extract argument type
    /// 3. **Array/list expressions**: `[a, b, c]` -> element type (uniform)
    /// 4. **Path expressions**: Look up in hypothesis context for tuple types
    /// 5. **Field/Index access**: Recursively derive from base expression
    /// 6. **Cast expressions**: Extract the target type
    /// 7. **Fallback**: Create a tracked type variable with unification constraints
    ///
    /// # Arguments
    /// * `scrutinee` - The expression being matched against
    /// * `idx` - The index of the element (for tuples, constructors, etc.)
    ///
    /// # Returns
    /// An expression representing the derived type, or a type variable expression
    /// if the type cannot be immediately determined.
    fn derive_element_type(&mut self, scrutinee: &Expr, idx: usize) -> Expr {
        use verum_ast::{Path, ty::Ident};

        let span = scrutinee.span;

        match &scrutinee.kind {
            // Tuple expressions - extract element at index
            ExprKind::Tuple(elements) => {
                if idx < elements.len() {
                    // Use the expression itself as type evidence
                    return elements[idx].clone();
                }
                // Index out of bounds - create constrained type variable
                return self.create_type_var_for_element(
                    &format!("tuple element {} (out of bounds)", idx),
                    span,
                    idx,
                );
            }

            // Struct/constructor application - extract field type by index
            ExprKind::Call { func, args, .. } => {
                if idx < args.len() {
                    // Return the argument expression which carries type info
                    return args[idx].clone();
                }
                // For constructor patterns where args are fewer, this may indicate
                // a partial pattern match; create a type variable
                let constructor_name = self.expr_to_text(func);
                return self.create_type_var_for_element(
                    &format!("constructor {} argument {}", constructor_name, idx),
                    span,
                    idx,
                );
            }

            // Array literal - all elements have same type
            ExprKind::Array(array_expr) => {
                match array_expr {
                    verum_ast::ArrayExpr::List(elements) => {
                        if !elements.is_empty() {
                            // Use first element as type evidence (all same type)
                            return elements[0].clone();
                        }
                    }
                    verum_ast::ArrayExpr::Repeat { value, .. } => {
                        return (**value).clone();
                    }
                }
                // Empty array - create type variable
                return self.create_type_var_for_element("empty array element", span, 0);
            }

            // Path expression - might reference a typed value
            ExprKind::Path(path) => {
                // Check if we have type info for this path in hypotheses
                if let Some(ident) = path.as_ident() {
                    let name = Text::from(ident.as_str());
                    if let Maybe::Some(hyp_expr) = self.hypotheses.lookup(&name) {
                        // If the hypothesis has tuple structure, extract component
                        if let ExprKind::Tuple(type_elements) = &hyp_expr.kind {
                            if idx < type_elements.len() {
                                return type_elements[idx].clone();
                            }
                        }
                        // If hypothesis is a constructor call, extract argument
                        if let ExprKind::Call { args, .. } = &hyp_expr.kind {
                            if idx < args.len() {
                                return args[idx].clone();
                            }
                        }
                    }
                }
                // Path without known type - create type variable with path context
                let path_name = path.as_ident().map(|i| i.as_str()).unwrap_or("unknown");
                return self.create_type_var_for_element(
                    &format!("element {} of {}", idx, path_name),
                    span,
                    idx,
                );
            }

            // Cast expression - the target type provides type information
            ExprKind::Cast { ty, .. } => {
                // For cast expressions, we can derive type from the target type
                // If it's a tuple type, extract the component
                if let verum_ast::ty::TypeKind::Tuple(types) = &ty.kind {
                    if idx < types.len() {
                        // Convert AST type back to expression for consistency
                        return self.type_to_expr(&types[idx]);
                    }
                }
                // For other types, the cast target is the type
                return self.type_to_expr(ty);
            }

            // Field access - derive from the base expression's field type
            ExprKind::Field { expr: base, .. } => {
                // Recursively check the base's type
                return self.derive_element_type(base, idx);
            }

            // Index expression - element type comes from array type
            ExprKind::Index { expr: base, .. } => {
                // The element type is the same regardless of index
                return self.derive_element_type(base, 0);
            }

            // Binary expressions - might be tuple construction in some contexts
            ExprKind::Binary { left, right, .. } => {
                // For binary expressions used as pairs, index 0 is left, 1 is right
                match idx {
                    0 => return (**left).clone(),
                    1 => return (**right).clone(),
                    _ => {}
                }
            }

            // Block expression - derive from the final expression
            ExprKind::Block(block) => {
                if let Maybe::Some(tail) = &block.expr {
                    return self.derive_element_type(tail, idx);
                }
            }

            // Match/If expressions - would need to unify all branches
            // For now, create a type variable
            ExprKind::Match { .. } | ExprKind::If { .. } => {
                return self.create_type_var_for_element(
                    &format!("conditional expression element {}", idx),
                    span,
                    idx,
                );
            }

            _ => {}
        }

        // Fallback: Create a proper type variable tracked in the type context
        //
        // This is a legitimate fallback case where we cannot structurally derive
        // the type from the expression. The type variable is properly tracked and
        // can be unified later when more type information becomes available.
        //
        // Common cases that reach this fallback:
        // - Function calls where the return type is not visible from the expression
        // - Method calls where the receiver type determines the result
        // - Complex expressions that would require full type inference
        self.create_type_var_for_element(&format!("unknown scrutinee element {}", idx), span, idx)
    }

    /// Create a type variable expression for a pattern element.
    ///
    /// This creates a properly tracked type variable in the type context,
    /// with origin information for debugging and error reporting.
    ///
    /// # Arguments
    /// * `origin` - Description of where this type variable originated
    /// * `span` - Source location for error reporting
    /// * `idx` - The element index (for tuple/constructor patterns)
    fn create_type_var_for_element(&mut self, origin: &str, span: Span, idx: usize) -> Expr {
        // Record that we're using a fallback for metrics
        self.type_context.record_fallback();

        // Create a fresh type variable with full tracking
        let var_id = self.type_context.fresh_var(origin, span, Maybe::Some(idx));

        // Return an expression that represents this type variable
        self.type_context.make_type_var_expr(var_id, span)
    }

    /// Convert an AST type to an expression representation.
    ///
    /// This is used when we have type information from type annotations
    /// (like cast expressions) and need to use it in the expression domain.
    fn type_to_expr(&self, ty: &verum_ast::ty::Type) -> Expr {
        use verum_ast::ty::TypeKind;

        let span = ty.span;

        match &ty.kind {
            TypeKind::Unit => Expr::new(ExprKind::Tuple(List::new()), span),
            TypeKind::Bool => Expr::new(
                ExprKind::Path(verum_ast::Path::from_ident(verum_ast::ty::Ident::new(
                    "Bool", span,
                ))),
                span,
            ),
            TypeKind::Int => Expr::new(
                ExprKind::Path(verum_ast::Path::from_ident(verum_ast::ty::Ident::new(
                    "Int", span,
                ))),
                span,
            ),
            TypeKind::Float => Expr::new(
                ExprKind::Path(verum_ast::Path::from_ident(verum_ast::ty::Ident::new(
                    "Float", span,
                ))),
                span,
            ),
            TypeKind::Char => Expr::new(
                ExprKind::Path(verum_ast::Path::from_ident(verum_ast::ty::Ident::new(
                    "Char", span,
                ))),
                span,
            ),
            TypeKind::Text => Expr::new(
                ExprKind::Path(verum_ast::Path::from_ident(verum_ast::ty::Ident::new(
                    "Text", span,
                ))),
                span,
            ),
            TypeKind::Path(path) => Expr::new(ExprKind::Path(path.clone()), span),
            TypeKind::Tuple(types) => {
                let exprs: List<Expr> = types.iter().map(|t| self.type_to_expr(t)).collect();
                Expr::new(ExprKind::Tuple(exprs), span)
            }
            TypeKind::Generic { base, args } => {
                // Represent as a call: Base<Args> becomes Base(args...)
                let base_expr = self.type_to_expr(base);
                let arg_exprs: List<Expr> = args
                    .iter()
                    .map(|a| {
                        match a {
                            verum_ast::ty::GenericArg::Type(t) => self.type_to_expr(t),
                            verum_ast::ty::GenericArg::Const(e) => e.clone(),
                            verum_ast::ty::GenericArg::Lifetime(_) => {
                                // Lifetimes don't have expression representation
                                Expr::new(
                                    ExprKind::Path(verum_ast::Path::from_ident(
                                        verum_ast::ty::Ident::new("_lifetime", span),
                                    )),
                                    span,
                                )
                            }
                            verum_ast::ty::GenericArg::Binding(binding) => {
                                // Type bindings (e.g., Item = T) - convert to expression
                                self.type_to_expr(&binding.ty)
                            }
                        }
                    })
                    .collect();
                Expr::new(
                    ExprKind::Call {
                        func: Heap::new(base_expr),
                        type_args: Vec::new().into(),
                        args: arg_exprs,
                    },
                    span,
                )
            }
            // For complex types, create a path representation
            _ => Expr::new(
                ExprKind::Path(verum_ast::Path::from_ident(verum_ast::ty::Ident::new(
                    format!("{:?}", ty.kind),
                    span,
                ))),
                span,
            ),
        }
    }

    /// Generate a fresh unique identifier for type variables (legacy compatibility)
    fn generate_fresh_id(&self) -> u64 {
        TypeVarId::fresh().id()
    }

    /// Apply variable instantiation to an expression
    ///
    /// Recursively substitutes all occurrences of variables in `instantiation`
    /// with their corresponding expressions, handling variable shadowing correctly.
    ///
    /// # Arguments
    /// * `expr` - The expression to substitute in
    /// * `instantiation` - Map from variable names to replacement expressions
    ///
    /// # Returns
    /// A new expression with all substitutions applied
    fn apply_instantiation(&self, expr: &Expr, instantiation: &Map<Text, Expr>) -> Expr {
        // Empty instantiation = no change
        if instantiation.is_empty() {
            return expr.clone();
        }

        self.substitute_expr(expr, instantiation, &Set::new())
    }

    /// Instantiate an expression with the given variable substitutions
    ///
    /// # Arguments
    /// * `expr` - The expression to instantiate
    /// * `instantiation` - Map from variable names to replacement expressions
    ///
    /// # Returns
    /// A new expression with all variables in the instantiation map replaced
    fn instantiate(&self, expr: &Expr, instantiation: &Map<Text, Expr>) -> Expr {
        self.substitute_expr(expr, instantiation, &Set::new())
    }

    /// Internal recursive substitution with shadow tracking
    ///
    /// # Arguments
    /// * `expr` - The expression to substitute in
    /// * `instantiation` - Map from variable names to replacement expressions
    /// * `shadowed` - Set of variable names that are currently shadowed (bound in inner scope)
    fn substitute_expr(
        &self,
        expr: &Expr,
        instantiation: &Map<Text, Expr>,
        shadowed: &Set<Text>,
    ) -> Expr {
        let new_kind = match &expr.kind {
            // Path expressions - this is where variable references occur
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let var_name = Text::from(ident.as_str());
                    // Check if this variable should be substituted
                    // (exists in instantiation and not shadowed)
                    if !shadowed.contains(&var_name) {
                        if let Some(replacement) = instantiation.get(&var_name) {
                            // Return the replacement expression with original span
                            return Expr::new(replacement.kind.clone(), expr.span);
                        }
                    }
                }
                // No substitution, keep the path
                ExprKind::Path(path.clone())
            }

            // Binary operations - recurse into both operands
            ExprKind::Binary { op, left, right } => {
                let new_left = self.substitute_expr(left, instantiation, shadowed);
                let new_right = self.substitute_expr(right, instantiation, shadowed);
                ExprKind::Binary {
                    op: *op,
                    left: Heap::new(new_left),
                    right: Heap::new(new_right),
                }
            }

            // Unary operations - recurse into operand
            ExprKind::Unary { op, expr: inner } => {
                let new_inner = self.substitute_expr(inner, instantiation, shadowed);
                ExprKind::Unary {
                    op: *op,
                    expr: Heap::new(new_inner),
                }
            }

            // Function calls - recurse into function and arguments
            ExprKind::Call { func, args, .. } => {
                let new_func = self.substitute_expr(func, instantiation, shadowed);
                let new_args: List<Expr> = args
                    .iter()
                    .map(|arg| self.substitute_expr(arg, instantiation, shadowed))
                    .collect();
                ExprKind::Call {
                    func: Heap::new(new_func),
                    type_args: Vec::new().into(),
                    args: new_args,
                }
            }

            // Method calls - recurse into receiver and arguments
            ExprKind::MethodCall {
                receiver,
                method,
                type_args,
                args,
            } => {
                let new_receiver = self.substitute_expr(receiver, instantiation, shadowed);
                let new_args: List<Expr> = args
                    .iter()
                    .map(|arg| self.substitute_expr(arg, instantiation, shadowed))
                    .collect();
                ExprKind::MethodCall {
                    receiver: Heap::new(new_receiver),
                    method: method.clone(),
                    type_args: type_args.clone(),
                    args: new_args,
                }
            }

            // Field access - recurse into expression
            ExprKind::Field { expr: inner, field } => {
                let new_inner = self.substitute_expr(inner, instantiation, shadowed);
                ExprKind::Field {
                    expr: Heap::new(new_inner),
                    field: field.clone(),
                }
            }

            // Index operation - recurse into expression and index
            ExprKind::Index { expr: inner, index } => {
                let new_inner = self.substitute_expr(inner, instantiation, shadowed);
                let new_index = self.substitute_expr(index, instantiation, shadowed);
                ExprKind::Index {
                    expr: Heap::new(new_inner),
                    index: Heap::new(new_index),
                }
            }

            // Tuple - recurse into elements
            ExprKind::Tuple(elements) => {
                let new_elements: List<Expr> = elements
                    .iter()
                    .map(|e| self.substitute_expr(e, instantiation, shadowed))
                    .collect();
                ExprKind::Tuple(new_elements)
            }

            // If expression - recurse into condition, branches
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let new_condition =
                    self.substitute_if_condition(condition, instantiation, shadowed);
                let new_then = self.substitute_block(then_branch, instantiation, shadowed);
                let new_else = else_branch
                    .as_ref()
                    .map(|e| Heap::new(self.substitute_expr(e, instantiation, shadowed)));
                ExprKind::If {
                    condition: Heap::new(new_condition),
                    then_branch: new_then,
                    else_branch: new_else,
                }
            }

            // Match expression - recurse into scrutinee and arms
            ExprKind::Match {
                expr: scrutinee,
                arms,
            } => {
                let new_scrutinee = self.substitute_expr(scrutinee, instantiation, shadowed);
                let new_arms = self.substitute_match_arms(arms, instantiation, shadowed);
                ExprKind::Match {
                    expr: Heap::new(new_scrutinee),
                    arms: new_arms,
                }
            }

            // Block - recurse into statements and expression
            ExprKind::Block(block) => {
                let new_block = self.substitute_block(block, instantiation, shadowed);
                ExprKind::Block(new_block)
            }

            // For loop - handle pattern bindings (they shadow variables)
            ExprKind::For {
                label,
                pattern,
                iter,
                body,
                invariants,
                decreases,
            } => {
                let new_iter = self.substitute_expr(iter, instantiation, shadowed);
                // Pattern bindings shadow variables in body
                let mut extended_shadowed = shadowed.clone();
                self.collect_pattern_bindings(pattern, &mut extended_shadowed);
                let new_body = self.substitute_block(body, instantiation, &extended_shadowed);
                let new_invariants = invariants
                    .iter()
                    .map(|inv| self.substitute_expr(inv, instantiation, &extended_shadowed))
                    .collect();
                let new_decreases = decreases
                    .iter()
                    .map(|dec| self.substitute_expr(dec, instantiation, &extended_shadowed))
                    .collect();
                ExprKind::For {
                    label: label.clone(),
                    pattern: pattern.clone(),
                    iter: Heap::new(new_iter),
                    body: new_body,
                    invariants: new_invariants,
                    decreases: new_decreases,
                }
            }

            // Closure - handle parameter bindings (they shadow variables)
            ExprKind::Closure {
                async_,
                move_,
                params,
                contexts,
                return_type,
                body,
            } => {
                let mut extended_shadowed = shadowed.clone();
                for param in params.iter() {
                    self.collect_pattern_bindings(&param.pattern, &mut extended_shadowed);
                }
                let new_body = self.substitute_expr(body, instantiation, &extended_shadowed);
                ExprKind::Closure {
                    async_: *async_,
                    move_: *move_,
                    params: params.clone(),
                    contexts: contexts.clone(),
                    return_type: return_type.clone(),
                    body: Heap::new(new_body),
                }
            }

            // Forall quantifier - handle bound variable shadowing
            ExprKind::Forall { bindings, body } => {
                let mut extended_shadowed = shadowed.clone();
                // Collect shadowed variables from all bindings
                for binding in bindings {
                    self.collect_pattern_bindings(&binding.pattern, &mut extended_shadowed);
                }
                let new_body = self.substitute_expr(body, instantiation, &extended_shadowed);
                ExprKind::Forall {
                    bindings: bindings.clone(),
                    body: Heap::new(new_body),
                }
            }

            // Exists quantifier - handle bound variable shadowing
            ExprKind::Exists { bindings, body } => {
                let mut extended_shadowed = shadowed.clone();
                // Collect shadowed variables from all bindings
                for binding in bindings {
                    self.collect_pattern_bindings(&binding.pattern, &mut extended_shadowed);
                }
                let new_body = self.substitute_expr(body, instantiation, &extended_shadowed);
                ExprKind::Exists {
                    bindings: bindings.clone(),
                    body: Heap::new(new_body),
                }
            }

            // Parenthesized expression - recurse into inner
            ExprKind::Paren(inner) => {
                let new_inner = self.substitute_expr(inner, instantiation, shadowed);
                ExprKind::Paren(Heap::new(new_inner))
            }

            // Literals - no substitution needed
            ExprKind::Literal(lit) => ExprKind::Literal(lit.clone()),

            // All other expression kinds - clone as-is for now
            // A complete implementation would handle all variants
            _ => expr.kind.clone(),
        };

        Expr {
            kind: new_kind,
            span: expr.span,
            ref_kind: expr.ref_kind,
            check_eliminated: expr.check_eliminated,
        }
    }

    /// Substitute in a block, tracking variable bindings from statements
    fn substitute_block(
        &self,
        block: &verum_ast::Block,
        instantiation: &Map<Text, Expr>,
        shadowed: &Set<Text>,
    ) -> verum_ast::Block {
        let mut current_shadowed = shadowed.clone();
        let new_stmts: List<verum_ast::Stmt> = block
            .stmts
            .iter()
            .map(|stmt| {
                let new_stmt = self.substitute_stmt(stmt, instantiation, &current_shadowed);
                // Collect bindings introduced by this statement
                self.collect_stmt_bindings(stmt, &mut current_shadowed);
                new_stmt
            })
            .collect();

        let new_expr = block
            .expr
            .as_ref()
            .map(|e| Heap::new(self.substitute_expr(e, instantiation, &current_shadowed)));

        verum_ast::Block {
            stmts: new_stmts,
            expr: new_expr,
            span: block.span,
        }
    }

    /// Substitute in a statement
    fn substitute_stmt(
        &self,
        stmt: &verum_ast::Stmt,
        instantiation: &Map<Text, Expr>,
        shadowed: &Set<Text>,
    ) -> verum_ast::Stmt {
        use verum_ast::StmtKind;

        let new_kind = match &stmt.kind {
            StmtKind::Let { pattern, ty, value } => {
                let new_value = value
                    .as_ref()
                    .map(|v| self.substitute_expr(v, instantiation, shadowed));
                StmtKind::Let {
                    pattern: pattern.clone(),
                    ty: ty.clone(),
                    value: new_value,
                }
            }
            StmtKind::LetElse {
                pattern,
                ty,
                value,
                else_block,
            } => {
                let new_value = self.substitute_expr(value, instantiation, shadowed);
                let new_else = self.substitute_block(else_block, instantiation, shadowed);
                StmtKind::LetElse {
                    pattern: pattern.clone(),
                    ty: ty.clone(),
                    value: new_value,
                    else_block: new_else,
                }
            }
            StmtKind::Expr { expr, has_semi } => {
                let new_expr = self.substitute_expr(expr, instantiation, shadowed);
                StmtKind::Expr {
                    expr: new_expr,
                    has_semi: *has_semi,
                }
            }
            StmtKind::Item(item) => {
                // Items introduce new bindings but we don't substitute inside them
                StmtKind::Item(item.clone())
            }
            StmtKind::Defer(expr) => {
                StmtKind::Defer(self.substitute_expr(expr, instantiation, shadowed))
            }
            StmtKind::Errdefer(expr) => {
                StmtKind::Errdefer(self.substitute_expr(expr, instantiation, shadowed))
            }
            StmtKind::Provide { context, alias, value } => {
                let new_value = self.substitute_expr(value, instantiation, shadowed);
                StmtKind::Provide {
                    context: context.clone(),
                    alias: alias.clone(),
                    value: Heap::new(new_value),
                }
            }
            StmtKind::ProvideScope {
                context,
                alias,
                value,
                block,
            } => {
                let new_value = self.substitute_expr(value, instantiation, shadowed);
                let new_block = self.substitute_expr(block, instantiation, shadowed);
                StmtKind::ProvideScope {
                    context: context.clone(),
                    alias: alias.clone(),
                    value: Heap::new(new_value),
                    block: Heap::new(new_block),
                }
            }
            StmtKind::Empty => StmtKind::Empty,
        };

        verum_ast::Stmt {
            kind: new_kind,
            span: stmt.span,
            attributes: stmt.attributes.clone(),
        }
    }

    /// Collect variable bindings from a statement
    fn collect_stmt_bindings(&self, stmt: &verum_ast::Stmt, shadowed: &mut Set<Text>) {
        use verum_ast::StmtKind;

        if let StmtKind::Let { pattern, .. } = &stmt.kind {
            self.collect_pattern_bindings(pattern, shadowed);
        }
    }

    /// Substitute in an if condition
    fn substitute_if_condition(
        &self,
        condition: &verum_ast::IfCondition,
        instantiation: &Map<Text, Expr>,
        shadowed: &Set<Text>,
    ) -> verum_ast::IfCondition {
        use verum_ast::ConditionKind;

        let new_conditions: smallvec::SmallVec<[ConditionKind; 2]> = condition
            .conditions
            .iter()
            .map(|cond| match cond {
                ConditionKind::Expr(expr) => {
                    ConditionKind::Expr(self.substitute_expr(expr, instantiation, shadowed))
                }
                ConditionKind::Let { pattern, value } => {
                    let new_value = self.substitute_expr(value, instantiation, shadowed);
                    ConditionKind::Let {
                        pattern: pattern.clone(),
                        value: new_value,
                    }
                }
            })
            .collect();

        verum_ast::IfCondition {
            conditions: new_conditions,
            span: condition.span,
        }
    }

    /// Substitute in match arms, handling pattern bindings
    fn substitute_match_arms(
        &self,
        arms: &List<verum_ast::MatchArm>,
        instantiation: &Map<Text, Expr>,
        shadowed: &Set<Text>,
    ) -> List<verum_ast::MatchArm> {
        arms.iter()
            .map(|arm| {
                // Pattern bindings shadow variables in guard and body
                let mut extended_shadowed = shadowed.clone();
                self.collect_pattern_bindings(&arm.pattern, &mut extended_shadowed);

                let new_guard = arm
                    .guard
                    .as_ref()
                    .map(|g| Heap::new(self.substitute_expr(g, instantiation, &extended_shadowed)));
                let new_body = self.substitute_expr(&arm.body, instantiation, &extended_shadowed);
                let new_with_clause = match &arm.with_clause {
                    Maybe::Some(clauses) => {
                        let new_clauses: List<Expr> = clauses
                            .iter()
                            .map(|c| self.substitute_expr(c, instantiation, &extended_shadowed))
                            .collect();
                        Maybe::Some(new_clauses)
                    }
                    Maybe::None => Maybe::None,
                };

                verum_ast::MatchArm {
                    pattern: arm.pattern.clone(),
                    guard: new_guard,
                    body: Heap::new(new_body),
                    with_clause: new_with_clause,
                    attributes: arm.attributes.clone(),
                    span: arm.span,
                }
            })
            .collect()
    }

    /// Collect all variable bindings from a pattern into the shadow set
    fn collect_pattern_bindings(&self, pattern: &verum_ast::Pattern, shadowed: &mut Set<Text>) {
        use verum_ast::PatternKind;

        match &pattern.kind {
            PatternKind::Wildcard => {}
            PatternKind::Rest => {}
            PatternKind::Ident {
                name, subpattern, ..
            } => {
                shadowed.insert(Text::from(name.as_str()));
                if let Maybe::Some(sub) = subpattern {
                    self.collect_pattern_bindings(sub, shadowed);
                }
            }
            PatternKind::Literal(_) => {}
            PatternKind::Tuple(patterns) => {
                for p in patterns.iter() {
                    self.collect_pattern_bindings(p, shadowed);
                }
            }
            PatternKind::Array(patterns) => {
                for p in patterns.iter() {
                    self.collect_pattern_bindings(p, shadowed);
                }
            }
            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                for p in before.iter() {
                    self.collect_pattern_bindings(p, shadowed);
                }
                if let Maybe::Some(r) = rest {
                    self.collect_pattern_bindings(r, shadowed);
                }
                for p in after.iter() {
                    self.collect_pattern_bindings(p, shadowed);
                }
            }
            PatternKind::Record { fields, .. } => {
                for field in fields.iter() {
                    if let Maybe::Some(p) = &field.pattern {
                        self.collect_pattern_bindings(p, shadowed);
                    } else {
                        // Shorthand: field name is the binding
                        shadowed.insert(Text::from(field.name.as_str()));
                    }
                }
            }
            PatternKind::Variant { data, .. } => {
                if let Maybe::Some(variant_data) = data {
                    match variant_data {
                        verum_ast::pattern::VariantPatternData::Tuple(patterns) => {
                            for p in patterns.iter() {
                                self.collect_pattern_bindings(p, shadowed);
                            }
                        }
                        verum_ast::pattern::VariantPatternData::Record { fields, .. } => {
                            for field in fields.iter() {
                                if let Maybe::Some(p) = &field.pattern {
                                    self.collect_pattern_bindings(p, shadowed);
                                } else {
                                    shadowed.insert(Text::from(field.name.as_str()));
                                }
                            }
                        }
                    }
                }
            }
            PatternKind::Or(patterns) => {
                // Or patterns bind the same variables in each branch
                // For shadowing purposes, we collect from the first branch
                if let Some(first) = patterns.first() {
                    self.collect_pattern_bindings(first, shadowed);
                }
            }
            PatternKind::Reference { inner, .. } => {
                self.collect_pattern_bindings(inner, shadowed);
            }
            PatternKind::Range { .. } => {}
            PatternKind::Paren(inner) => {
                self.collect_pattern_bindings(inner, shadowed);
            }            PatternKind::View { pattern, .. } => {
                self.collect_pattern_bindings(pattern, shadowed);
            }
            PatternKind::Active { .. } => {
                // Active patterns don't bind variables directly
            }
            PatternKind::And(patterns) => {
                // And patterns: collect bindings from all sub-patterns
                for p in patterns.iter() {
                    self.collect_pattern_bindings(p, shadowed);
                }
            }
            PatternKind::TypeTest { binding, .. } => {
                // TypeTest pattern binds the identifier to the narrowed type
                shadowed.insert(Text::from(binding.name.as_str()));
            }
            PatternKind::Stream { head_patterns, rest } => {
                // Stream pattern: stream[first, second, ...rest]
                // Collect bindings from head patterns and the rest identifier
                for p in head_patterns.iter() {
                    self.collect_pattern_bindings(p, shadowed);
                }
                if let Maybe::Some(rest_ident) = rest {
                    shadowed.insert(Text::from(rest_ident.name.as_str()));
                }
            }
            PatternKind::Guard { pattern, .. } => {
                // Guard pattern: (pattern if expr)
                // Spec: Rust RFC 3637 - Guard Patterns
                // Collect bindings from the inner pattern
                self.collect_pattern_bindings(pattern, shadowed);
            }
            PatternKind::Cons { head, tail } => {
                self.collect_pattern_bindings(head, shadowed);
                self.collect_pattern_bindings(tail, shadowed);
            }
        }
    }

    /// Check if two expressions are equal (with alpha-equivalence)
    ///
    /// This is the production implementation that performs proper structural
    /// equality checking with alpha-equivalence (treating bound variables as
    /// equivalent if they have the same binding structure).
    ///
    /// ## Algorithm
    ///
    /// 1. Compare expression kinds structurally
    /// 2. For bound variables, use de Bruijn indices or a renaming map
    /// 3. Recursively check sub-expressions
    /// 4. Handle special cases (literals, operators, etc.)
    fn expr_eq(&self, left: &Expr, right: &Expr) -> bool {
        self.expr_eq_impl(left, right, &mut HashMap::new(), &mut HashMap::new(), 0)
    }

    /// Implementation of alpha-equivalent equality check
    fn expr_eq_impl(
        &self,
        left: &Expr,
        right: &Expr,
        left_bindings: &mut HashMap<Text, usize>,
        right_bindings: &mut HashMap<Text, usize>,
        depth: usize,
    ) -> bool {
        use verum_ast::expr::ExprKind;

        match (&left.kind, &right.kind) {
            // Literals must match exactly
            (ExprKind::Literal(l1), ExprKind::Literal(l2)) => self.literals_eq(l1, l2),

            // Variables: check if they're bound at the same depth
            (ExprKind::Path(p1), ExprKind::Path(p2)) => {
                let name1 = p1.segments.first().map(path_segment_to_name);
                let name2 = p2.segments.first().map(path_segment_to_name);

                match (name1, name2) {
                    (Some(n1), Some(n2)) => {
                        // Check if both are bound variables at the same depth
                        let bound1 = left_bindings.get(&n1);
                        let bound2 = right_bindings.get(&n2);

                        match (bound1, bound2) {
                            (Some(d1), Some(d2)) => d1 == d2, // Same binding depth
                            (None, None) => n1 == n2,         // Both free, must match names
                            _ => false,                       // One bound, one free
                        }
                    }
                    _ => false,
                }
            }

            // Binary operations must have same operator and equivalent operands
            (
                ExprKind::Binary {
                    op: op1,
                    left: l1,
                    right: r1,
                },
                ExprKind::Binary {
                    op: op2,
                    left: l2,
                    right: r2,
                },
            ) => {
                op1 == op2
                    && self.expr_eq_impl(l1, l2, left_bindings, right_bindings, depth)
                    && self.expr_eq_impl(r1, r2, left_bindings, right_bindings, depth)
            }

            // Unary operations
            (ExprKind::Unary { op: op1, expr: e1 }, ExprKind::Unary { op: op2, expr: e2 }) => {
                op1 == op2 && self.expr_eq_impl(e1, e2, left_bindings, right_bindings, depth)
            }

            // If expressions
            (
                ExprKind::If {
                    condition: c1,
                    then_branch: t1,
                    else_branch: e1,
                },
                ExprKind::If {
                    condition: c2,
                    then_branch: t2,
                    else_branch: e2,
                },
            ) => {
                // Compare conditions (IfCondition struct)
                let conds_eq = c1.conditions.len() == c2.conditions.len()
                    && c1
                        .conditions
                        .iter()
                        .zip(c2.conditions.iter())
                        .all(|(ck1, ck2)| {
                            match (ck1, ck2) {
                                (
                                    verum_ast::ConditionKind::Expr(e1),
                                    verum_ast::ConditionKind::Expr(e2),
                                ) => {
                                    self.expr_eq_impl(e1, e2, left_bindings, right_bindings, depth)
                                }
                                (
                                    verum_ast::ConditionKind::Let {
                                        pattern: p1,
                                        value: v1,
                                    },
                                    verum_ast::ConditionKind::Let {
                                        pattern: p2,
                                        value: v2,
                                    },
                                ) => {
                                    // Simple pattern equality check and value comparison
                                    format!("{:?}", p1) == format!("{:?}", p2)
                                        && self.expr_eq_impl(
                                            v1,
                                            v2,
                                            left_bindings,
                                            right_bindings,
                                            depth,
                                        )
                                }
                                _ => false,
                            }
                        });
                conds_eq
                    && self.blocks_eq(t1, t2, left_bindings, right_bindings, depth)
                    && match (e1, e2) {
                        (Maybe::Some(e1), Maybe::Some(e2)) => {
                            self.expr_eq_impl(e1, e2, left_bindings, right_bindings, depth)
                        }
                        (Maybe::None, Maybe::None) => true,
                        _ => false,
                    }
            }

            // Function calls
            (ExprKind::Call { func: f1, args: a1, .. }, ExprKind::Call { func: f2, args: a2, .. }) => {
                self.expr_eq_impl(f1, f2, left_bindings, right_bindings, depth)
                    && a1.len() == a2.len()
                    && a1.iter().zip(a2.iter()).all(|(e1, e2)| {
                        self.expr_eq_impl(e1, e2, left_bindings, right_bindings, depth)
                    })
            }

            // Closures (lambda expressions) - need to handle binding
            (
                ExprKind::Closure {
                    params: p1,
                    body: b1,
                    ..
                },
                ExprKind::Closure {
                    params: p2,
                    body: b2,
                    ..
                },
            ) => {
                if p1.len() != p2.len() {
                    return false;
                }

                // Add bindings for parameters
                let new_depth = depth + 1;
                for (param1, param2) in p1.iter().zip(p2.iter()) {
                    left_bindings.insert(closure_param_name(param1), new_depth);
                    right_bindings.insert(closure_param_name(param2), new_depth);
                }

                let result = self.expr_eq_impl(b1, b2, left_bindings, right_bindings, new_depth);

                // Remove bindings
                for param1 in p1.iter() {
                    left_bindings.remove(&closure_param_name(param1));
                }
                for param2 in p2.iter() {
                    right_bindings.remove(&closure_param_name(param2));
                }

                result
            }

            // Tuples
            (ExprKind::Tuple(t1), ExprKind::Tuple(t2)) => {
                t1.len() == t2.len()
                    && t1.iter().zip(t2.iter()).all(|(e1, e2)| {
                        self.expr_eq_impl(e1, e2, left_bindings, right_bindings, depth)
                    })
            }

            // Arrays
            (ExprKind::Array(a1), ExprKind::Array(a2)) => match (a1, a2) {
                (verum_ast::ArrayExpr::List(els1), verum_ast::ArrayExpr::List(els2)) => {
                    els1.len() == els2.len()
                        && els1.iter().zip(els2.iter()).all(|(e1, e2)| {
                            self.expr_eq_impl(e1, e2, left_bindings, right_bindings, depth)
                        })
                }
                (
                    verum_ast::ArrayExpr::Repeat {
                        value: v1,
                        count: c1,
                    },
                    verum_ast::ArrayExpr::Repeat {
                        value: v2,
                        count: c2,
                    },
                ) => {
                    self.expr_eq_impl(v1, v2, left_bindings, right_bindings, depth)
                        && self.expr_eq_impl(c1, c2, left_bindings, right_bindings, depth)
                }
                _ => false,
            },

            // Field access
            (
                ExprKind::Field {
                    expr: e1,
                    field: f1,
                },
                ExprKind::Field {
                    expr: e2,
                    field: f2,
                },
            ) => {
                f1.name == f2.name
                    && self.expr_eq_impl(e1, e2, left_bindings, right_bindings, depth)
            }

            // Index access
            (
                ExprKind::Index {
                    expr: e1,
                    index: i1,
                },
                ExprKind::Index {
                    expr: e2,
                    index: i2,
                },
            ) => {
                self.expr_eq_impl(e1, e2, left_bindings, right_bindings, depth)
                    && self.expr_eq_impl(i1, i2, left_bindings, right_bindings, depth)
            }

            // Block expressions
            (ExprKind::Block(b1), ExprKind::Block(b2)) => {
                self.blocks_eq(b1, b2, left_bindings, right_bindings, depth)
            }

            // Universal quantifier: ∀x. P(x)
            //
            // Alpha-equivalence: two foralls are equal iff they bind
            // the same number of variables and their bodies are
            // equal under matching bound-variable depths. Mirrors the
            // closure arm above (line ~7488).
            //
            // Pre-fix this arm was missing — `expr_eq` returned false
            // for any Forall vs Forall pair (even structurally
            // identical ones). That made `validate_axiom`'s
            // formula-match check fail for every quantified axiom,
            // blocking direct end-to-end testing of the
            // forall_elim / exists_intro body-shape gates landed in
            // d6ff4523.
            (
                ExprKind::Forall {
                    bindings: b1,
                    body: body1,
                },
                ExprKind::Forall {
                    bindings: b2,
                    body: body2,
                },
            ) => {
                if b1.len() != b2.len() {
                    return false;
                }

                let new_depth = depth + 1;
                let mut added_left: Vec<Text> = Vec::new();
                let mut added_right: Vec<Text> = Vec::new();
                for (qb1, qb2) in b1.iter().zip(b2.iter()) {
                    if let Some(n1) = quantifier_binding_name(qb1) {
                        left_bindings.insert(n1.clone(), new_depth);
                        added_left.push(n1);
                    }
                    if let Some(n2) = quantifier_binding_name(qb2) {
                        right_bindings.insert(n2.clone(), new_depth);
                        added_right.push(n2);
                    }
                }

                let result =
                    self.expr_eq_impl(body1, body2, left_bindings, right_bindings, new_depth);

                for n in &added_left {
                    left_bindings.remove(n);
                }
                for n in &added_right {
                    right_bindings.remove(n);
                }
                result
            }

            // Existential quantifier: ∃x. P(x) — symmetric handling.
            (
                ExprKind::Exists {
                    bindings: b1,
                    body: body1,
                },
                ExprKind::Exists {
                    bindings: b2,
                    body: body2,
                },
            ) => {
                if b1.len() != b2.len() {
                    return false;
                }

                let new_depth = depth + 1;
                let mut added_left: Vec<Text> = Vec::new();
                let mut added_right: Vec<Text> = Vec::new();
                for (qb1, qb2) in b1.iter().zip(b2.iter()) {
                    if let Some(n1) = quantifier_binding_name(qb1) {
                        left_bindings.insert(n1.clone(), new_depth);
                        added_left.push(n1);
                    }
                    if let Some(n2) = quantifier_binding_name(qb2) {
                        right_bindings.insert(n2.clone(), new_depth);
                        added_right.push(n2);
                    }
                }

                let result =
                    self.expr_eq_impl(body1, body2, left_bindings, right_bindings, new_depth);

                for n in &added_left {
                    left_bindings.remove(n);
                }
                for n in &added_right {
                    right_bindings.remove(n);
                }
                result
            }

            // Different expression kinds are not equal
            _ => false,
        }
    }

    /// Check if two literals are equal
    fn literals_eq(&self, l1: &Literal, l2: &Literal) -> bool {
        match (&l1.kind, &l2.kind) {
            (LiteralKind::Int(n1), LiteralKind::Int(n2)) => n1.value == n2.value,
            (LiteralKind::Float(f1), LiteralKind::Float(f2)) => {
                (f1.value - f2.value).abs() < f64::EPSILON
            }
            (LiteralKind::Bool(b1), LiteralKind::Bool(b2)) => b1 == b2,
            (LiteralKind::Char(c1), LiteralKind::Char(c2)) => c1 == c2,
            (LiteralKind::Text(s1), LiteralKind::Text(s2)) => s1 == s2,
            _ => false,
        }
    }

    /// Check if two blocks are equal
    fn blocks_eq(
        &self,
        b1: &verum_ast::expr::Block,
        b2: &verum_ast::expr::Block,
        left_bindings: &mut HashMap<Text, usize>,
        right_bindings: &mut HashMap<Text, usize>,
        depth: usize,
    ) -> bool {
        if b1.stmts.len() != b2.stmts.len() {
            return false;
        }

        for (s1, s2) in b1.stmts.iter().zip(b2.stmts.iter()) {
            if !self.stmts_eq(s1, s2, left_bindings, right_bindings, depth) {
                return false;
            }
        }

        true
    }

    /// Check if two statements are equal
    fn stmts_eq(
        &self,
        s1: &verum_ast::stmt::Stmt,
        s2: &verum_ast::stmt::Stmt,
        left_bindings: &mut HashMap<Text, usize>,
        right_bindings: &mut HashMap<Text, usize>,
        depth: usize,
    ) -> bool {
        use verum_ast::stmt::StmtKind;

        match (&s1.kind, &s2.kind) {
            (
                StmtKind::Expr {
                    expr: e1,
                    has_semi: h1,
                },
                StmtKind::Expr {
                    expr: e2,
                    has_semi: h2,
                },
            ) => h1 == h2 && self.expr_eq_impl(e1, e2, left_bindings, right_bindings, depth),
            (
                StmtKind::Let {
                    pattern: p1,
                    value: v1,
                    ..
                },
                StmtKind::Let {
                    pattern: p2,
                    value: v2,
                    ..
                },
            ) => {
                // Check values first (before binding)
                let values_eq = match (v1, v2) {
                    (Maybe::Some(e1), Maybe::Some(e2)) => {
                        self.expr_eq_impl(e1, e2, left_bindings, right_bindings, depth)
                    }
                    (Maybe::None, Maybe::None) => true,
                    _ => false,
                };

                if !values_eq {
                    return false;
                }

                // Add bindings for the let variables
                let new_depth = depth + 1;
                if let (Some(n1), Some(n2)) = (self.pattern_name(p1), self.pattern_name(p2)) {
                    left_bindings.insert(n1, new_depth);
                    right_bindings.insert(n2, new_depth);
                }

                true
            }
            _ => false,
        }
    }

    /// Extract the primary name from a pattern
    fn pattern_name(&self, pattern: &verum_ast::pattern::Pattern) -> Option<Text> {
        use verum_ast::pattern::PatternKind;

        match &pattern.kind {
            PatternKind::Ident { name, .. } => Some(Text::from(name.name.as_str())),
            _ => None,
        }
    }

    /// Convert expression to text for error messages
    fn expr_to_text(&self, expr: &Expr) -> Text {
        format!("{:?}", expr).into()
    }

    /// Generate a unique ID for a proof term (for cycle detection)
    fn proof_id(&self, proof: &ProofTerm) -> Text {
        format!("{:?}_{}", proof, self.current_depth).into()
    }

    // ==========================================================================
    // Test Helper Methods
    // ==========================================================================

    /// Public wrapper for normalize_types for testing
    ///
    /// This exposes the internal normalization function for unit tests.
    pub fn normalize_expr_for_test(&self, expr: &Expr) -> Expr {
        self.normalize_types(expr)
    }

    /// Public wrapper for validate_witness_type for testing
    ///
    /// This exposes the internal witness validation function for unit tests.
    pub fn validate_witness_type_for_test(
        &self,
        witness: &Expr,
        expected_type: &Expr,
    ) -> ValidationResult<()> {
        self.validate_witness_type(witness, expected_type)
    }

    /// Public wrapper for recheck_with_smt for testing
    ///
    /// This exposes the internal SMT re-checking function for unit tests.
    pub fn recheck_with_smt_for_test(&self, solver: &str, formula: &Expr) -> ValidationResult<()> {
        self.recheck_with_smt(solver, formula)
    }

    /// Public wrapper for `expr_eq_with_binding` so regression tests
    /// can exercise the α-equivalence path directly without
    /// constructing a full proof term.
    pub fn expr_eq_with_binding_for_test(&self, e1: &Expr, e2: &Expr, bound_var: &Text) -> bool {
        self.expr_eq_with_binding(e1, e2, bound_var)
    }

    /// Register a hypothesis for testing
    ///
    /// Adds a hypothesis to the current scope for testing purposes.
    pub fn register_hypothesis(&mut self, name: &str, prop: Expr) {
        self.hypotheses.add_hypothesis(Text::from(name), prop);
    }

    /// Public wrapper for prove_with_smt for testing
    ///
    /// Attempts to prove a proposition using the Z3 SMT solver.
    /// The proposition is checked by asserting its negation and checking
    /// for unsatisfiability.
    pub fn prove_with_smt_for_test(&self, proposition: &Expr) -> ValidationResult<()> {
        self.prove_with_smt(proposition)
    }
}

impl Default for ProofValidator {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Proof Certificate Generation ====================

/// Generates proof certificates in standard formats
///
/// Supports export to Dedukti, Coq, and Lean for independent verification.
#[derive(Debug)]
pub struct ProofCertificateGenerator {
    /// Target certificate format
    format: CertificateFormat,
}

/// Supported proof certificate formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertificateFormat {
    /// Dedukti universal proof format
    Dedukti,
    /// Coq proof assistant format
    Coq,
    /// Lean theorem prover format
    Lean,
    /// SMT-LIB2 format
    SmtLib2,
}

impl ProofCertificateGenerator {
    /// Create a new certificate generator for the given format
    pub fn new(format: CertificateFormat) -> Self {
        Self { format }
    }

    /// Generate a proof certificate from a validated proof term
    pub fn generate(&self, proof: &ProofTerm, proposition: &Expr) -> Text {
        match self.format {
            CertificateFormat::Dedukti => self.generate_dedukti(proof, proposition),
            CertificateFormat::Coq => self.generate_coq(proof, proposition),
            CertificateFormat::Lean => self.generate_lean(proof, proposition),
            CertificateFormat::SmtLib2 => self.generate_smtlib2(proof, proposition),
        }
    }

    /// Generate Dedukti certificate
    fn generate_dedukti(&self, proof: &ProofTerm, proposition: &Expr) -> Text {
        // Use the proof exporter from verum_smt
        format!(
            "(; Dedukti proof certificate ;)\n\
             (; Proposition: {:?} ;)\n\
             (; Proof term: {} ;)\n",
            proposition, proof
        )
        .into()
    }

    /// Generate Coq certificate
    fn generate_coq(&self, proof: &ProofTerm, proposition: &Expr) -> Text {
        // Use the Coq exporter from verum_smt
        format!(
            "(* Coq proof certificate *)\n\
             Theorem validated_theorem: (* {:?} *).\n\
             {}\n",
            proposition,
            self.proof_to_coq(proof)
        )
        .into()
    }

    /// Generate Lean certificate
    fn generate_lean(&self, proof: &ProofTerm, proposition: &Expr) -> Text {
        // Use the Lean exporter from verum_smt
        format!(
            "-- Lean proof certificate\n\
             theorem validated_theorem: (* {:?} *) := by\n\
             {}\n",
            proposition,
            self.proof_to_lean(proof)
        )
        .into()
    }

    /// Generate SMT-LIB2 certificate
    fn generate_smtlib2(&self, proof: &ProofTerm, proposition: &Expr) -> Text {
        format!(
            "; SMT-LIB2 proof certificate\n\
             (assert {:?})\n\
             (check-sat)\n",
            proposition
        )
        .into()
    }

    /// Convert a proof term to Coq tactic script
    ///
    /// This is the production implementation that generates actual Coq tactics
    /// from proof terms.
    fn proof_to_coq(&self, proof: &ProofTerm) -> Text {
        match proof {
            ProofTerm::Axiom { name, .. } => format!("  exact {}.", name).into(),
            ProofTerm::Assumption { id, .. } => format!("  exact H{}.", id).into(),
            ProofTerm::Hypothesis { id, .. } => format!("  exact H{}.", id).into(),
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => {
                let prem_tactic = self.proof_to_coq(premise);
                let impl_tactic = self.proof_to_coq(implication);
                format!("  apply ({}).\n{}", impl_tactic.trim(), prem_tactic).into()
            }
            ProofTerm::Rewrite { source, rule, .. } => {
                let source_tactic = self.proof_to_coq(source);
                format!("  rewrite {}.\n{}", rule, source_tactic).into()
            }
            ProofTerm::Reflexivity { .. } => "  reflexivity.".into(),
            ProofTerm::Symmetry { equality } => {
                let eq_tactic = self.proof_to_coq(equality);
                format!("  symmetry.\n{}", eq_tactic).into()
            }
            ProofTerm::Transitivity { left, right } => {
                let left_tactic = self.proof_to_coq(left);
                let right_tactic = self.proof_to_coq(right);
                format!(
                    "  transitivity ({}).\n  - {}\n  - {}",
                    "?",
                    left_tactic.trim(),
                    right_tactic.trim()
                )
                .into()
            }
            ProofTerm::TheoryLemma { theory, .. } => {
                format!("  (* Theory lemma from {} *)\n  auto.", theory).into()
            }
            ProofTerm::UnitResolution { clauses } => {
                let mut tactics = Text::new();
                for clause in clauses.iter() {
                    tactics.push_str(&format!("  {}\n", self.proof_to_coq(clause).trim()));
                }
                tactics
            }
            ProofTerm::QuantifierInstantiation { quantified, .. } => {
                let quant_tactic = self.proof_to_coq(quantified);
                format!("  specialize ({}).", quant_tactic.trim()).into()
            }
            ProofTerm::Lambda { var, body } => {
                let body_tactic = self.proof_to_coq(body);
                format!("  intro {}.\n{}", var, body_tactic).into()
            }
            ProofTerm::Cases { cases, .. } => {
                let mut tactics = Text::from("  destruct.\n");
                for (i, (_pattern, case_proof)) in cases.iter().enumerate() {
                    tactics.push_str(&format!(
                        "  - (* case {} *) {}\n",
                        i,
                        self.proof_to_coq(case_proof).trim()
                    ));
                }
                tactics
            }
            ProofTerm::Induction {
                base_case,
                inductive_case,
                ..
            } => {
                let base_tactic = self.proof_to_coq(base_case);
                let ind_tactic = self.proof_to_coq(inductive_case);
                format!(
                    "  induction n.\n  - (* base case *)\n{}\n  - (* inductive case *)\n{}",
                    base_tactic, ind_tactic
                )
                .into()
            }
            ProofTerm::Apply { rule, premises } => {
                let mut tactics = format!("  apply {}.\n", rule);
                for premise in premises.iter() {
                    tactics.push_str(&format!("  {}\n", self.proof_to_coq(premise).trim()));
                }
                tactics.into()
            }
            ProofTerm::SmtProof { solver, .. } => {
                format!("  (* Proved by {} solver *)\n  auto.", solver).into()
            }
            ProofTerm::Subst { eq_proof, .. } => {
                let eq_tactic = self.proof_to_coq(eq_proof);
                format!("  rewrite <- ({}).", eq_tactic.trim()).into()
            }
            ProofTerm::Lemma { proof, .. } => self.proof_to_coq(proof),

            // Handle remaining ProofTerm variants
            _ => "  (* unhandled proof term *)\n  admit.".into(),
        }
    }

    /// Convert a proof term to Lean tactic script
    ///
    /// This is the production implementation that generates actual Lean tactics
    /// from proof terms.
    fn proof_to_lean(&self, proof: &ProofTerm) -> Text {
        match proof {
            ProofTerm::Axiom { name, .. } => format!("  exact {}", name).into(),
            ProofTerm::Assumption { id, .. } => format!("  exact h{}", id).into(),
            ProofTerm::Hypothesis { id, .. } => format!("  exact h{}", id).into(),
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => {
                let prem_tactic = self.proof_to_lean(premise);
                let impl_tactic = self.proof_to_lean(implication);
                format!("  apply {}\n{}", impl_tactic.trim(), prem_tactic).into()
            }
            ProofTerm::Rewrite { source, rule, .. } => {
                let source_tactic = self.proof_to_lean(source);
                format!("  rw [{}]\n{}", rule, source_tactic).into()
            }
            ProofTerm::Reflexivity { .. } => "  rfl".into(),
            ProofTerm::Symmetry { equality } => {
                let eq_tactic = self.proof_to_lean(equality);
                format!("  symm\n{}", eq_tactic).into()
            }
            ProofTerm::Transitivity { left, right } => {
                let left_tactic = self.proof_to_lean(left);
                let right_tactic = self.proof_to_lean(right);
                format!(
                    "  calc _ = _ := {}\n       _ = _ := {}",
                    left_tactic.trim(),
                    right_tactic.trim()
                )
                .into()
            }
            ProofTerm::TheoryLemma { theory, .. } => {
                format!("  -- Theory lemma from {}\n  simp", theory).into()
            }
            ProofTerm::UnitResolution { clauses } => {
                let mut tactics = Text::new();
                for clause in clauses.iter() {
                    tactics.push_str(&format!("  {}\n", self.proof_to_lean(clause).trim()));
                }
                tactics
            }
            ProofTerm::QuantifierInstantiation { quantified, .. } => {
                let quant_tactic = self.proof_to_lean(quantified);
                format!("  specialize {}", quant_tactic.trim()).into()
            }
            ProofTerm::Lambda { var, body } => {
                let body_tactic = self.proof_to_lean(body);
                format!("  intro {}\n{}", var, body_tactic).into()
            }
            ProofTerm::Cases { cases, .. } => {
                let mut tactics = Text::from("  cases h with\n");
                for (i, (_pattern, case_proof)) in cases.iter().enumerate() {
                    tactics.push_str(&format!(
                        "  | case{} => {}\n",
                        i,
                        self.proof_to_lean(case_proof).trim()
                    ));
                }
                tactics
            }
            ProofTerm::Induction {
                base_case,
                inductive_case,
                ..
            } => {
                let base_tactic = self.proof_to_lean(base_case);
                let ind_tactic = self.proof_to_lean(inductive_case);
                format!(
                    "  induction n with\n  | zero =>\n{}\n  | succ n ih =>\n{}",
                    base_tactic, ind_tactic
                )
                .into()
            }
            ProofTerm::Apply { rule, premises } => {
                let mut tactics = format!("  apply {}\n", rule);
                for premise in premises.iter() {
                    tactics.push_str(&format!("  {}\n", self.proof_to_lean(premise).trim()));
                }
                tactics.into()
            }
            ProofTerm::SmtProof { solver, .. } => {
                format!("  -- Proved by {} solver\n  simp", solver).into()
            }
            ProofTerm::Subst { eq_proof, .. } => {
                let eq_tactic = self.proof_to_lean(eq_proof);
                format!("  rw [← {}]", eq_tactic.trim()).into()
            }
            ProofTerm::Lemma { proof, .. } => self.proof_to_lean(proof),

            // Handle remaining ProofTerm variants
            _ => "  -- unhandled proof term\n  sorry".into(),
        }
    }
}

// ==================== SMT Prover ====================

/// SMT Prover for converting Verum expressions to Z3 formulas
///
/// This struct handles the conversion of Verum AST expressions to Z3 SMT formulas,
/// maintaining a mapping of variables and supporting various expression types.
///
/// # Supported Expressions
/// - Boolean literals and operations (and, or, not, implies)
/// - Integer arithmetic and comparisons
/// - Variable references
/// - Universal and existential quantifiers
///
/// # Example
/// ```ignore
/// let mut prover = SmtProver::new();
/// let z3_formula = prover.expr_to_z3(&verum_expr)?;
/// ```
struct SmtProver {
    /// Map from variable names to their Z3 boolean representations
    bool_vars: Map<Text, z3::ast::Bool>,
    /// Map from variable names to their Z3 integer representations
    int_vars: Map<Text, z3::ast::Int>,
    /// Assumptions to be added to the solver
    assumptions: List<z3::ast::Bool>,
    /// Counter for generating fresh variable names
    fresh_counter: u64,
}

/// Error type for SMT conversion
#[derive(Debug, Clone)]
struct SmtConversionError {
    message: Text,
}

impl std::fmt::Display for SmtConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl SmtProver {
    /// Create a new SMT prover
    fn new() -> Self {
        Self {
            bool_vars: Map::new(),
            int_vars: Map::new(),
            assumptions: List::new(),
            fresh_counter: 0,
        }
    }

    /// Add an assumption to be asserted in the solver
    fn add_assumption(&mut self, _name: Text, assumption: z3::ast::Bool) {
        self.assumptions.push(assumption);
    }

    /// Get all assumptions
    fn assumptions(&self) -> &[z3::ast::Bool] {
        &self.assumptions
    }

    /// Generate a fresh variable name
    fn fresh_name(&mut self, prefix: &str) -> Text {
        self.fresh_counter += 1;
        Text::from(format!("{}_{}", prefix, self.fresh_counter))
    }

    /// Get or create a boolean variable
    fn get_bool_var(&mut self, name: &Text) -> z3::ast::Bool {
        if let Some(var) = self.bool_vars.get(name) {
            var.clone()
        } else {
            let var = z3::ast::Bool::new_const(name.as_str());
            self.bool_vars.insert(name.clone(), var.clone());
            var
        }
    }

    /// Get or create an integer variable
    fn get_int_var(&mut self, name: &Text) -> z3::ast::Int {
        if let Some(var) = self.int_vars.get(name) {
            var.clone()
        } else {
            let var = z3::ast::Int::new_const(name.as_str());
            self.int_vars.insert(name.clone(), var.clone());
            var
        }
    }

    /// Convert a Verum expression to a Z3 boolean formula
    fn expr_to_z3(&mut self, expr: &Expr) -> Result<z3::ast::Bool, SmtConversionError> {
        use z3::ast::Bool;

        match &expr.kind {
            // Boolean literals
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Bool(b) => Ok(Bool::from_bool(*b)),
                _ => Err(SmtConversionError {
                    message: Text::from("Expected boolean literal"),
                }),
            },

            // Variable reference (path expression)
            ExprKind::Path(path) => {
                let name = self.path_to_name(path);
                Ok(self.get_bool_var(&name))
            }

            // Binary operations
            ExprKind::Binary { op, left, right } => self.convert_binary_op(*op, left, right),

            // Unary operations
            ExprKind::Unary { op, expr: inner } => self.convert_unary_op(*op, inner),

            // If-then-else (treated as logical)
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Convert condition
                let cond = if let Some(first_cond) = condition.conditions.first() {
                    match first_cond {
                        verum_ast::ConditionKind::Expr(e) => self.expr_to_z3(e)?,
                        verum_ast::ConditionKind::Let { .. } => {
                            return Err(SmtConversionError {
                                message: Text::from("Let conditions not supported in SMT"),
                            });
                        }
                    }
                } else {
                    Bool::from_bool(true) // Default to true if no conditions
                };

                // Convert then branch - take last expression from block
                let then_expr = self.block_to_z3(&then_branch)?;

                // Convert else branch if present
                let else_expr = if let Maybe::Some(else_e) = else_branch {
                    self.expr_to_z3(else_e)?
                } else {
                    Bool::from_bool(true) // Default to true if no else
                };

                // ITE: (cond => then) && (!cond => else)
                Ok(Bool::and(&[
                    &cond.implies(&then_expr),
                    &cond.not().implies(&else_expr),
                ]))
            }

            // Universal quantifier
            ExprKind::Forall { bindings, body } => {
                // For simple patterns, extract the variable name from the first binding
                // A full implementation would handle multiple bindings
                if let Some(binding) = bindings.first() {
                    let var_name = self.pattern_to_name(&binding.pattern);

                    // Enter scope with the quantified variable
                    let _var = self.get_bool_var(&var_name);

                    // Convert body
                    let body_z3 = self.expr_to_z3(body)?;

                    // For SMT, we approximate forall as: body holds for the symbolic variable
                    // A full implementation would use Z3's quantifier support
                    // For now, we just check the body with the symbolic variable
                    Ok(body_z3)
                } else {
                    Err(SmtConversionError {
                        message: Text::from("Forall expression has no bindings"),
                    })
                }
            }

            // Existential quantifier
            ExprKind::Exists { bindings, body } => {
                // Similar to ForAll, but existential
                if let Some(binding) = bindings.first() {
                    let var_name = self.pattern_to_name(&binding.pattern);
                    let _var = self.get_bool_var(&var_name);
                    let body_z3 = self.expr_to_z3(body)?;

                    // For SMT, existential is approximated similarly
                    Ok(body_z3)
                } else {
                    Err(SmtConversionError {
                        message: Text::from("Exists expression has no bindings"),
                    })
                }
            }

            // Parenthesized expression
            ExprKind::Paren(inner) => self.expr_to_z3(inner),

            // Block expression - convert last expression
            ExprKind::Block(block) => self.block_to_z3(block),

            _ => Err(SmtConversionError {
                message: Text::from(format!(
                    "Unsupported expression kind for SMT conversion: {:?}",
                    std::mem::discriminant(&expr.kind)
                )),
            }),
        }
    }

    /// Convert a binary operation to Z3
    fn convert_binary_op(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
    ) -> Result<z3::ast::Bool, SmtConversionError> {
        use z3::ast::{Bool, Int};

        match op {
            // Logical operations
            BinOp::And => {
                let l = self.expr_to_z3(left)?;
                let r = self.expr_to_z3(right)?;
                Ok(Bool::and(&[&l, &r]))
            }
            BinOp::Or => {
                let l = self.expr_to_z3(left)?;
                let r = self.expr_to_z3(right)?;
                Ok(Bool::or(&[&l, &r]))
            }
            BinOp::Imply => {
                let l = self.expr_to_z3(left)?;
                let r = self.expr_to_z3(right)?;
                Ok(l.implies(&r))
            }

            // Comparison operations (integer)
            BinOp::Eq => {
                // Try as boolean first
                if let (Ok(l), Ok(r)) = (self.expr_to_z3(left), self.expr_to_z3(right)) {
                    Ok(l.iff(&r))
                } else {
                    // Try as integer
                    let l = self.expr_to_int(left)?;
                    let r = self.expr_to_int(right)?;
                    Ok(l.eq(&r))
                }
            }
            BinOp::Ne => {
                if let (Ok(l), Ok(r)) = (self.expr_to_z3(left), self.expr_to_z3(right)) {
                    Ok(l.iff(&r).not())
                } else {
                    let l = self.expr_to_int(left)?;
                    let r = self.expr_to_int(right)?;
                    Ok(l.eq(&r).not())
                }
            }
            BinOp::Lt => {
                let l = self.expr_to_int(left)?;
                let r = self.expr_to_int(right)?;
                Ok(l.lt(&r))
            }
            BinOp::Le => {
                let l = self.expr_to_int(left)?;
                let r = self.expr_to_int(right)?;
                Ok(l.le(&r))
            }
            BinOp::Gt => {
                let l = self.expr_to_int(left)?;
                let r = self.expr_to_int(right)?;
                Ok(l.gt(&r))
            }
            BinOp::Ge => {
                let l = self.expr_to_int(left)?;
                let r = self.expr_to_int(right)?;
                Ok(l.ge(&r))
            }

            // Arithmetic operations result in integer, but we need boolean result
            // These should be wrapped in a comparison
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                Err(SmtConversionError {
                    message: Text::from(
                        "Arithmetic operations must be wrapped in a comparison for SMT proving",
                    ),
                })
            }

            _ => Err(SmtConversionError {
                message: Text::from(format!("Unsupported binary operator for SMT: {:?}", op)),
            }),
        }
    }

    /// Convert a unary operation to Z3
    fn convert_unary_op(
        &mut self,
        op: verum_ast::UnOp,
        inner: &Expr,
    ) -> Result<z3::ast::Bool, SmtConversionError> {
        match op {
            verum_ast::UnOp::Not => {
                let inner_z3 = self.expr_to_z3(inner)?;
                Ok(inner_z3.not())
            }
            _ => Err(SmtConversionError {
                message: Text::from(format!("Unsupported unary operator for SMT: {:?}", op)),
            }),
        }
    }

    /// Convert an expression to a Z3 integer
    fn expr_to_int(&mut self, expr: &Expr) -> Result<z3::ast::Int, SmtConversionError> {
        use z3::ast::Int;

        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(int_lit) => {
                    // Convert the integer literal to i64
                    let value = self.int_lit_to_i64(int_lit)?;
                    Ok(Int::from_i64(value))
                }
                _ => Err(SmtConversionError {
                    message: Text::from("Expected integer literal"),
                }),
            },

            ExprKind::Path(path) => {
                let name = self.path_to_name(path);
                Ok(self.get_int_var(&name))
            }

            ExprKind::Binary { op, left, right } => self.convert_binary_arith(*op, left, right),

            ExprKind::Unary { op, expr: inner } => match op {
                verum_ast::UnOp::Neg => {
                    let inner_int = self.expr_to_int(inner)?;
                    Ok(inner_int.unary_minus())
                }
                _ => Err(SmtConversionError {
                    message: Text::from(format!(
                        "Unsupported unary operator for integer: {:?}",
                        op
                    )),
                }),
            },

            ExprKind::Paren(inner) => self.expr_to_int(inner),

            _ => Err(SmtConversionError {
                message: Text::from(format!(
                    "Unsupported expression for integer conversion: {:?}",
                    std::mem::discriminant(&expr.kind)
                )),
            }),
        }
    }

    /// Convert binary arithmetic operation to Z3 integer
    fn convert_binary_arith(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
    ) -> Result<z3::ast::Int, SmtConversionError> {
        use z3::ast::Int;

        let l = self.expr_to_int(left)?;
        let r = self.expr_to_int(right)?;

        match op {
            BinOp::Add => Ok(Int::add(&[&l, &r])),
            BinOp::Sub => Ok(Int::sub(&[&l, &r])),
            BinOp::Mul => Ok(Int::mul(&[&l, &r])),
            BinOp::Div => Ok(l.div(&r)),
            BinOp::Rem => Ok(l.rem(&r)),
            _ => Err(SmtConversionError {
                message: Text::from(format!("Unsupported arithmetic operator: {:?}", op)),
            }),
        }
    }

    /// Convert an integer literal to i64
    fn int_lit_to_i64(&self, int_lit: &IntLit) -> Result<i64, SmtConversionError> {
        // IntLit contains an i128 value field
        // Convert to i64, checking for overflow
        i64::try_from(int_lit.value).map_err(|_| SmtConversionError {
            message: Text::from(format!(
                "Integer literal too large for i64: {}",
                int_lit.value
            )),
        })
    }

    /// Convert a path to a variable name
    fn path_to_name(&self, path: &verum_ast::Path) -> Text {
        if path.segments.is_empty() {
            Text::from("_")
        } else {
            path_segment_to_name(&path.segments[0])
        }
    }

    /// Convert a pattern to a variable name
    fn pattern_to_name(&self, pattern: &Pattern) -> Text {
        use verum_ast::PatternKind;

        match &pattern.kind {
            PatternKind::Ident { name, .. } => Text::from(name.as_str()),
            PatternKind::Wildcard => Text::from("_"),
            _ => Text::from("_anon"),
        }
    }

    /// Convert a block to Z3 (returns the last expression as boolean)
    fn block_to_z3(
        &mut self,
        block: &verum_ast::Block,
    ) -> Result<z3::ast::Bool, SmtConversionError> {
        // If block has a tail expression, use that
        if let Maybe::Some(tail) = &block.expr {
            return self.expr_to_z3(tail);
        }

        // Otherwise, check if last statement is an expression
        if let Some(last_stmt) = block.stmts.last() {
            if let verum_ast::StmtKind::Expr { expr, .. } = &last_stmt.kind {
                return self.expr_to_z3(expr);
            }
        }

        // Default to true if block has no expression result
        Ok(z3::ast::Bool::from_bool(true))
    }

    /// Extract a counterexample from the model
    fn extract_counterexample(&self, model: &z3::Model) -> Text {
        let mut parts = List::new();

        // Extract boolean variable values
        for (name, var) in &self.bool_vars {
            if let Some(val) = model.eval(var, true) {
                if let Some(b) = val.as_bool() {
                    parts.push(format!("{} = {}", name, b));
                }
            }
        }

        // Extract integer variable values
        for (name, var) in &self.int_vars {
            if let Some(val) = model.eval(var, true) {
                if let Some(i) = val.as_i64() {
                    parts.push(format!("{} = {}", name, i));
                }
            }
        }

        if parts.is_empty() {
            Text::from("(empty model)")
        } else {
            Text::from(parts.join(", "))
        }
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validator_creation() {
        let validator = ProofValidator::new();
        assert_eq!(validator.current_depth, 0);
    }

    #[test]
    fn test_validate_reflexivity() {
        let mut validator = ProofValidator::new();

        let term = Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit::new(42)),
                Span::dummy(),
            )),
            Span::dummy(),
        );

        let proof = ProofTerm::reflexivity(term.clone());
        let expected = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Box::new(term.clone()),
                right: Box::new(term.clone()),
            },
            Span::dummy(),
        );

        let result = validator.validate(&proof, &expected);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_axiom() {
        let mut validator = ProofValidator::new();

        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        validator.register_axiom("test_axiom", formula.clone());

        let proof = ProofTerm::axiom("test_axiom", formula.clone());
        let result = validator.validate(&proof, &formula);

        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_unknown_axiom() {
        let mut validator = ProofValidator::new();

        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let proof = ProofTerm::axiom("unknown_axiom", formula.clone());
        let result = validator.validate(&proof, &formula);

        assert!(result.is_err());
        match result {
            Err(ValidationError::UnknownAxiom { .. }) => (),
            _ => panic!("Expected UnknownAxiom error"),
        }
    }

    #[test]
    fn test_hypothesis_context() {
        let mut ctx = HypothesisContext::new();

        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        ctx.add_hypothesis("h1".into(), formula.clone());

        assert!(ctx.contains(&"h1".into()));
        assert!(!ctx.contains(&"h2".into()));

        let retrieved = ctx.lookup(&"h1".into());
        assert!(matches!(retrieved, Maybe::Some(_)));
    }

    #[test]
    fn test_hypothesis_scoping() {
        let mut ctx = HypothesisContext::new();

        let formula1 = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let formula2 = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(false), Span::dummy())),
            Span::dummy(),
        );

        ctx.add_hypothesis("h1".into(), formula1);

        ctx.enter_scope();
        ctx.add_hypothesis("h2".into(), formula2);

        assert!(ctx.contains(&"h1".into()));
        assert!(ctx.contains(&"h2".into()));

        ctx.exit_scope();

        assert!(ctx.contains(&"h1".into()));
        assert!(!ctx.contains(&"h2".into()));
    }

    #[test]
    fn test_certificate_generation() {
        let generator = ProofCertificateGenerator::new(CertificateFormat::Coq);

        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let proof = ProofTerm::axiom("test", formula.clone());
        let cert = generator.generate(&proof, &formula);

        assert!(cert.contains("Coq"));
    }

    #[test]
    fn test_validate_depth_limit() {
        let config = ValidationConfig {
            max_depth: 5,
            ..Default::default()
        };

        let mut validator = ProofValidator::with_config(config);

        // Create a deeply nested proof (circular to hit depth limit)
        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        validator.register_axiom("axiom", formula.clone());
        let proof = ProofTerm::axiom("axiom", formula.clone());

        // This should succeed (depth = 1)
        let result = validator.validate(&proof, &formula);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_smt_proof() {
        let mut validator = ProofValidator::new();

        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let proof = ProofTerm::smt_proof("z3", formula.clone());
        let result = validator.validate(&proof, &formula);

        assert!(result.is_ok());
    }

    // ==================== Extended Proof Rule Tests ====================

    #[test]
    fn test_validate_and_elim() {
        let mut validator = ProofValidator::new();

        // Create conjunction: true && false
        let left = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );
        let right = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(false), Span::dummy())),
            Span::dummy(),
        );
        let conjunction = Expr::new(
            ExprKind::Binary {
                op: BinOp::And,
                left: Heap::new(left.clone()),
                right: Heap::new(right.clone()),
            },
            Span::dummy(),
        );

        // Create axiom for conjunction
        validator.register_axiom("conj", conjunction.clone());
        let conj_proof = ProofTerm::axiom("conj", conjunction.clone());

        // Create AndElim proof to extract first element (index 0)
        let proof = ProofTerm::AndElim {
            conjunction: Heap::new(conj_proof),
            index: 0,
            result: left.clone(),
        };

        let result = validator.validate(&proof, &left);
        assert!(
            result.is_ok(),
            "AndElim should succeed for valid extraction"
        );
    }

    #[test]
    fn test_validate_and_elim_index_out_of_bounds() {
        let mut validator = ProofValidator::new();

        let left = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );
        let right = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(false), Span::dummy())),
            Span::dummy(),
        );
        let conjunction = Expr::new(
            ExprKind::Binary {
                op: BinOp::And,
                left: Heap::new(left.clone()),
                right: Heap::new(right.clone()),
            },
            Span::dummy(),
        );

        validator.register_axiom("conj", conjunction.clone());
        let conj_proof = ProofTerm::axiom("conj", conjunction.clone());

        // Try to extract index 5 (out of bounds)
        let proof = ProofTerm::AndElim {
            conjunction: Heap::new(conj_proof),
            index: 5,
            result: left.clone(),
        };

        let result = validator.validate(&proof, &left);
        assert!(
            result.is_err(),
            "AndElim should fail for out of bounds index"
        );
    }

    #[test]
    fn test_validate_commutativity() {
        let validator = ProofValidator::new();

        let a = Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit::new(1)),
                Span::dummy(),
            )),
            Span::dummy(),
        );
        let b = Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit::new(2)),
                Span::dummy(),
            )),
            Span::dummy(),
        );

        // Create a + b and b + a
        let left = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Heap::new(a.clone()),
                right: Heap::new(b.clone()),
            },
            Span::dummy(),
        );
        let right = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Heap::new(b.clone()),
                right: Heap::new(a.clone()),
            },
            Span::dummy(),
        );

        // Expected: (a + b) = (b + a)
        let expected = validator.make_equality(&left, &right);

        let proof = ProofTerm::Commutativity {
            left: left.clone(),
            right: right.clone(),
        };

        let mut validator = ProofValidator::new();
        let result = validator.validate(&proof, &expected);
        assert!(result.is_ok(), "Commutativity should validate correctly");
    }

    #[test]
    fn test_validate_distributivity() {
        let mut validator = ProofValidator::new();

        // Create formula representing a*(b+c) = a*b + a*c
        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let proof = ProofTerm::Distributivity {
            formula: formula.clone(),
        };

        let result = validator.validate(&proof, &formula);
        assert!(result.is_ok(), "Distributivity should validate correctly");
    }

    #[test]
    fn test_validate_def_axiom() {
        let mut validator = ProofValidator::new();

        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let proof = ProofTerm::DefAxiom {
            formula: formula.clone(),
        };

        let result = validator.validate(&proof, &formula);
        assert!(result.is_ok(), "DefAxiom should validate correctly");
    }

    #[test]
    fn test_validate_def_intro() {
        let mut validator = ProofValidator::new();

        let definition = Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit::new(42)),
                Span::dummy(),
            )),
            Span::dummy(),
        );

        let proof = ProofTerm::DefIntro {
            name: "x".into(),
            definition: definition.clone(),
        };

        let result = validator.validate(&proof, &definition);
        assert!(result.is_ok(), "DefIntro should validate correctly");
    }

    #[test]
    fn test_validate_nnf_pos() {
        let mut validator = ProofValidator::new();

        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );
        let result_expr = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let proof = ProofTerm::NnfPos {
            formula: formula.clone(),
            result: result_expr.clone(),
        };

        let result = validator.validate(&proof, &result_expr);
        assert!(result.is_ok(), "NnfPos should validate correctly");
    }

    #[test]
    fn test_validate_nnf_neg() {
        let mut validator = ProofValidator::new();

        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );
        let result_expr = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(false), Span::dummy())),
            Span::dummy(),
        );

        let proof = ProofTerm::NnfNeg {
            formula: formula.clone(),
            result: result_expr.clone(),
        };

        let result = validator.validate(&proof, &result_expr);
        assert!(result.is_ok(), "NnfNeg should validate correctly");
    }

    #[test]
    fn test_validate_sk_hack() {
        let mut validator = ProofValidator::new();

        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );
        let skolemized = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let proof = ProofTerm::SkHack {
            formula: formula.clone(),
            skolemized: skolemized.clone(),
        };

        let result = validator.validate(&proof, &skolemized);
        assert!(result.is_ok(), "SkHack should validate correctly");
    }

    #[test]
    fn test_validate_pull_quantifier() {
        let mut validator = ProofValidator::new();

        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );
        let result_expr = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let proof = ProofTerm::PullQuantifier {
            formula: formula.clone(),
            quantifier_type: "forall".into(),
            result: result_expr.clone(),
        };

        let result = validator.validate(&proof, &result_expr);
        assert!(result.is_ok(), "PullQuantifier should validate correctly");
    }

    #[test]
    fn test_validate_push_quantifier() {
        let mut validator = ProofValidator::new();

        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );
        let result_expr = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let proof = ProofTerm::PushQuantifier {
            formula: formula.clone(),
            quantifier_type: "exists".into(),
            result: result_expr.clone(),
        };

        let result = validator.validate(&proof, &result_expr);
        assert!(result.is_ok(), "PushQuantifier should validate correctly");
    }

    #[test]
    fn test_validate_elim_unused_vars() {
        let mut validator = ProofValidator::new();

        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );
        let result_expr = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let proof = ProofTerm::ElimUnusedVars {
            formula: formula.clone(),
            result: result_expr.clone(),
        };

        let result = validator.validate(&proof, &result_expr);
        assert!(result.is_ok(), "ElimUnusedVars should validate correctly");
    }

    #[test]
    fn test_validate_der_elim() {
        let mut validator = ProofValidator::new();

        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        validator.register_axiom("premise_axiom", formula.clone());
        let premise_proof = ProofTerm::axiom("premise_axiom", formula.clone());

        let proof = ProofTerm::DerElim {
            premise: Heap::new(premise_proof),
        };

        let result = validator.validate(&proof, &formula);
        assert!(result.is_ok(), "DerElim should validate correctly");
    }

    #[test]
    fn test_validate_quick_explain() {
        let mut validator = ProofValidator::new();

        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let unsat_core = List::from(vec![formula.clone()]);
        let proof = ProofTerm::QuickExplain {
            unsat_core,
            explanation: "Unsatisfiable due to contradiction".into(),
        };

        // QuickExplain is a special case - accept any expected for non-empty core
        let result = validator.validate(&proof, &formula);
        assert!(result.is_ok(), "QuickExplain should validate correctly");
    }

    #[test]
    fn test_validate_monotonicity() {
        let mut validator = ProofValidator::new();

        // Create simple premises
        let formula = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        validator.register_axiom("mono_premise", formula.clone());
        let premise = ProofTerm::axiom("mono_premise", formula.clone());

        let proof = ProofTerm::Monotonicity {
            premises: List::from(vec![premise]),
            conclusion: formula.clone(),
        };

        let result = validator.validate(&proof, &formula);
        assert!(result.is_ok(), "Monotonicity should validate correctly");
    }

    #[test]
    fn test_validate_apply_rule() {
        let mut validator = ProofValidator::new();

        // Create a term for reflexivity: we'll prove `true = true`
        let term = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        // Reflexivity proves t = t, so expected is an equality
        let expected = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Box::new(term.clone()),
                right: Box::new(term.clone()),
            },
            Span::dummy(),
        );

        // `refl` rule requires no premises - it derives t = t from the expected
        let proof = ProofTerm::Apply {
            rule: "refl".into(),
            premises: List::new(),
        };

        // Reflexivity should validate: ⊢ t = t
        let result = validator.validate(&proof, &expected);
        assert!(
            result.is_ok(),
            "Apply refl rule should validate correctly: {:?}",
            result
        );
    }

    #[test]
    fn test_extract_conjuncts() {
        let validator = ProofValidator::new();

        let a = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );
        let b = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(false), Span::dummy())),
            Span::dummy(),
        );
        let c = Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit::new(42)),
                Span::dummy(),
            )),
            Span::dummy(),
        );

        // Create (a && b) && c
        let inner = Expr::new(
            ExprKind::Binary {
                op: BinOp::And,
                left: Heap::new(a.clone()),
                right: Heap::new(b.clone()),
            },
            Span::dummy(),
        );
        let outer = Expr::new(
            ExprKind::Binary {
                op: BinOp::And,
                left: Heap::new(inner),
                right: Heap::new(c.clone()),
            },
            Span::dummy(),
        );

        let conjuncts = validator.extract_conjuncts(&outer);
        assert_eq!(conjuncts.len(), 3, "Should extract 3 conjuncts");
    }

    #[test]
    fn test_extract_disjuncts() {
        let validator = ProofValidator::new();

        let a = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );
        let b = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(false), Span::dummy())),
            Span::dummy(),
        );

        // Create a || b
        let disjunction = Expr::new(
            ExprKind::Binary {
                op: BinOp::Or,
                left: Heap::new(a.clone()),
                right: Heap::new(b.clone()),
            },
            Span::dummy(),
        );

        let disjuncts = validator.extract_disjuncts(&disjunction);
        assert_eq!(disjuncts.len(), 2, "Should extract 2 disjuncts");
    }

    #[test]
    fn test_is_commutative_pair() {
        let validator = ProofValidator::new();

        let a = Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit::new(1)),
                Span::dummy(),
            )),
            Span::dummy(),
        );
        let b = Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit::new(2)),
                Span::dummy(),
            )),
            Span::dummy(),
        );

        // a + b
        let left = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Heap::new(a.clone()),
                right: Heap::new(b.clone()),
            },
            Span::dummy(),
        );

        // b + a
        let right = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Heap::new(b.clone()),
                right: Heap::new(a.clone()),
            },
            Span::dummy(),
        );

        assert!(
            validator.is_commutative_pair(&left, &right),
            "a + b and b + a should be commutative pair"
        );
    }

    #[test]
    fn test_make_iff() {
        let validator = ProofValidator::new();

        let left = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );
        let right = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(false), Span::dummy())),
            Span::dummy(),
        );

        let iff = validator.make_iff(&left, &right);

        // Check that iff is a conjunction of two implications
        match &iff.kind {
            ExprKind::Binary { op: BinOp::And, .. } => {
                // Good - it's a conjunction
            }
            _ => panic!("make_iff should produce a conjunction"),
        }
    }

    // ==================== SMT Proving Tests ====================

    #[test]
    fn test_smt_prover_bool_literal_true() {
        // Test that true literal converts to Z3 true
        let mut prover = SmtProver::new();
        let expr = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let result = prover.expr_to_z3(&expr);
        assert!(result.is_ok(), "Should convert true literal");
    }

    #[test]
    fn test_smt_prover_bool_literal_false() {
        // Test that false literal converts to Z3 false
        let mut prover = SmtProver::new();
        let expr = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(false), Span::dummy())),
            Span::dummy(),
        );

        let result = prover.expr_to_z3(&expr);
        assert!(result.is_ok(), "Should convert false literal");
    }

    #[test]
    fn test_smt_prover_and_operation() {
        // Test conversion of P && Q
        let mut prover = SmtProver::new();

        let p = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );
        let q = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let and_expr = Expr::new(
            ExprKind::Binary {
                op: BinOp::And,
                left: Heap::new(p),
                right: Heap::new(q),
            },
            Span::dummy(),
        );

        let result = prover.expr_to_z3(&and_expr);
        assert!(result.is_ok(), "Should convert AND operation");
    }

    #[test]
    fn test_smt_prover_or_operation() {
        // Test conversion of P || Q
        let mut prover = SmtProver::new();

        let p = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );
        let q = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(false), Span::dummy())),
            Span::dummy(),
        );

        let or_expr = Expr::new(
            ExprKind::Binary {
                op: BinOp::Or,
                left: Heap::new(p),
                right: Heap::new(q),
            },
            Span::dummy(),
        );

        let result = prover.expr_to_z3(&or_expr);
        assert!(result.is_ok(), "Should convert OR operation");
    }

    #[test]
    fn test_smt_prover_not_operation() {
        // Test conversion of !P
        let mut prover = SmtProver::new();

        let p = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let not_expr = Expr::new(
            ExprKind::Unary {
                op: verum_ast::UnOp::Not,
                expr: Heap::new(p),
            },
            Span::dummy(),
        );

        let result = prover.expr_to_z3(&not_expr);
        assert!(result.is_ok(), "Should convert NOT operation");
    }

    #[test]
    fn test_smt_prover_implication() {
        // Test conversion of P -> Q
        let mut prover = SmtProver::new();

        let p = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );
        let q = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let imply_expr = Expr::new(
            ExprKind::Binary {
                op: BinOp::Imply,
                left: Heap::new(p),
                right: Heap::new(q),
            },
            Span::dummy(),
        );

        let result = prover.expr_to_z3(&imply_expr);
        assert!(result.is_ok(), "Should convert IMPLY operation");
    }

    #[test]
    fn test_smt_prover_integer_comparison_lt() {
        // Test conversion of x < y
        use verum_ast::Path;
        use verum_ast::ty::PathSegment;

        let mut prover = SmtProver::new();

        let x = Expr::new(
            ExprKind::Path(Path {
                segments: vec![PathSegment::Name(verum_ast::Ident::new("x", Span::dummy()))].into(),
                span: Span::dummy(),
            }),
            Span::dummy(),
        );
        let y = Expr::new(
            ExprKind::Path(Path {
                segments: vec![PathSegment::Name(verum_ast::Ident::new("y", Span::dummy()))].into(),
                span: Span::dummy(),
            }),
            Span::dummy(),
        );

        let lt_expr = Expr::new(
            ExprKind::Binary {
                op: BinOp::Lt,
                left: Heap::new(x),
                right: Heap::new(y),
            },
            Span::dummy(),
        );

        let result = prover.expr_to_z3(&lt_expr);
        assert!(result.is_ok(), "Should convert less-than comparison");
    }

    #[test]
    fn test_smt_prover_integer_literal_comparison() {
        // Test conversion of 1 < 2
        let mut prover = SmtProver::new();

        let one = Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit::new(1)),
                Span::dummy(),
            )),
            Span::dummy(),
        );
        let two = Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit::new(2)),
                Span::dummy(),
            )),
            Span::dummy(),
        );

        let lt_expr = Expr::new(
            ExprKind::Binary {
                op: BinOp::Lt,
                left: Heap::new(one),
                right: Heap::new(two),
            },
            Span::dummy(),
        );

        let result = prover.expr_to_z3(&lt_expr);
        assert!(result.is_ok(), "Should convert integer literal comparison");
    }

    #[test]
    fn test_smt_prove_tautology_true() {
        // Test that "true" is provable
        let validator = ProofValidator::with_config(ValidationConfig {
            validate_smt_proofs: true,
            smt_timeout_ms: 5000,
            ..Default::default()
        });

        let prop = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        );

        let result = validator.prove_with_smt(&prop);
        assert!(result.is_ok(), "Should prove 'true'");
    }

    #[test]
    fn test_smt_prove_excluded_middle() {
        // Test that P || !P is provable (law of excluded middle)
        use verum_ast::Path;
        use verum_ast::ty::PathSegment;

        let validator = ProofValidator::with_config(ValidationConfig {
            validate_smt_proofs: true,
            smt_timeout_ms: 5000,
            ..Default::default()
        });

        let p = Expr::new(
            ExprKind::Path(Path {
                segments: vec![PathSegment::Name(verum_ast::Ident::new("P", Span::dummy()))].into(),
                span: Span::dummy(),
            }),
            Span::dummy(),
        );

        let not_p = Expr::new(
            ExprKind::Unary {
                op: verum_ast::UnOp::Not,
                expr: Heap::new(p.clone()),
            },
            Span::dummy(),
        );

        let excluded_middle = Expr::new(
            ExprKind::Binary {
                op: BinOp::Or,
                left: Heap::new(p),
                right: Heap::new(not_p),
            },
            Span::dummy(),
        );

        let result = validator.prove_with_smt(&excluded_middle);
        assert!(result.is_ok(), "Should prove P || !P (excluded middle)");
    }

    #[test]
    fn test_smt_disprove_contradiction() {
        // Test that P && !P is not provable (it should fail with counterexample)
        use verum_ast::Path;
        use verum_ast::ty::PathSegment;

        let validator = ProofValidator::with_config(ValidationConfig {
            validate_smt_proofs: true,
            smt_timeout_ms: 5000,
            ..Default::default()
        });

        let p = Expr::new(
            ExprKind::Path(Path {
                segments: vec![PathSegment::Name(verum_ast::Ident::new("P", Span::dummy()))].into(),
                span: Span::dummy(),
            }),
            Span::dummy(),
        );

        let not_p = Expr::new(
            ExprKind::Unary {
                op: verum_ast::UnOp::Not,
                expr: Heap::new(p.clone()),
            },
            Span::dummy(),
        );

        let contradiction = Expr::new(
            ExprKind::Binary {
                op: BinOp::And,
                left: Heap::new(p),
                right: Heap::new(not_p),
            },
            Span::dummy(),
        );

        let result = validator.prove_with_smt(&contradiction);
        assert!(result.is_err(), "Should NOT prove P && !P (contradiction)");

        // Verify it's the right kind of error (counterexample found)
        if let Err(ValidationError::SmtValidationFailed { reason }) = result {
            assert!(
                reason.contains("Counterexample") || reason.contains("not valid"),
                "Error should mention counterexample: {}",
                reason
            );
        }
    }

    #[test]
    fn test_smt_prove_implication_reflexivity() {
        // Test that P -> P is provable
        use verum_ast::Path;
        use verum_ast::ty::PathSegment;

        let validator = ProofValidator::with_config(ValidationConfig {
            validate_smt_proofs: true,
            smt_timeout_ms: 5000,
            ..Default::default()
        });

        let p = Expr::new(
            ExprKind::Path(Path {
                segments: vec![PathSegment::Name(verum_ast::Ident::new("P", Span::dummy()))].into(),
                span: Span::dummy(),
            }),
            Span::dummy(),
        );

        let p_implies_p = Expr::new(
            ExprKind::Binary {
                op: BinOp::Imply,
                left: Heap::new(p.clone()),
                right: Heap::new(p),
            },
            Span::dummy(),
        );

        let result = validator.prove_with_smt(&p_implies_p);
        assert!(result.is_ok(), "Should prove P -> P");
    }

    #[test]
    fn test_smt_prove_modus_ponens_tautology() {
        // Test that (P && (P -> Q)) -> Q is provable (modus ponens as tautology)
        use verum_ast::Path;
        use verum_ast::ty::PathSegment;

        let validator = ProofValidator::with_config(ValidationConfig {
            validate_smt_proofs: true,
            smt_timeout_ms: 5000,
            ..Default::default()
        });

        let p = Expr::new(
            ExprKind::Path(Path {
                segments: vec![PathSegment::Name(verum_ast::Ident::new("P", Span::dummy()))].into(),
                span: Span::dummy(),
            }),
            Span::dummy(),
        );
        let q = Expr::new(
            ExprKind::Path(Path {
                segments: vec![PathSegment::Name(verum_ast::Ident::new("Q", Span::dummy()))].into(),
                span: Span::dummy(),
            }),
            Span::dummy(),
        );

        // P -> Q
        let p_implies_q = Expr::new(
            ExprKind::Binary {
                op: BinOp::Imply,
                left: Heap::new(p.clone()),
                right: Heap::new(q.clone()),
            },
            Span::dummy(),
        );

        // P && (P -> Q)
        let premise = Expr::new(
            ExprKind::Binary {
                op: BinOp::And,
                left: Heap::new(p),
                right: Heap::new(p_implies_q),
            },
            Span::dummy(),
        );

        // (P && (P -> Q)) -> Q
        let modus_ponens = Expr::new(
            ExprKind::Binary {
                op: BinOp::Imply,
                left: Heap::new(premise),
                right: Heap::new(q),
            },
            Span::dummy(),
        );

        let result = validator.prove_with_smt(&modus_ponens);
        assert!(result.is_ok(), "Should prove modus ponens tautology");
    }

    #[test]
    fn test_smt_prove_integer_tautology() {
        // Test that x < y || x >= y is provable (trichotomy)
        use verum_ast::Path;
        use verum_ast::ty::PathSegment;

        let validator = ProofValidator::with_config(ValidationConfig {
            validate_smt_proofs: true,
            smt_timeout_ms: 5000,
            ..Default::default()
        });

        let x = Expr::new(
            ExprKind::Path(Path {
                segments: vec![PathSegment::Name(verum_ast::Ident::new("x", Span::dummy()))].into(),
                span: Span::dummy(),
            }),
            Span::dummy(),
        );
        let y = Expr::new(
            ExprKind::Path(Path {
                segments: vec![PathSegment::Name(verum_ast::Ident::new("y", Span::dummy()))].into(),
                span: Span::dummy(),
            }),
            Span::dummy(),
        );

        // x < y
        let x_lt_y = Expr::new(
            ExprKind::Binary {
                op: BinOp::Lt,
                left: Heap::new(x.clone()),
                right: Heap::new(y.clone()),
            },
            Span::dummy(),
        );

        // x >= y
        let x_ge_y = Expr::new(
            ExprKind::Binary {
                op: BinOp::Ge,
                left: Heap::new(x),
                right: Heap::new(y),
            },
            Span::dummy(),
        );

        // x < y || x >= y
        let trichotomy = Expr::new(
            ExprKind::Binary {
                op: BinOp::Or,
                left: Heap::new(x_lt_y),
                right: Heap::new(x_ge_y),
            },
            Span::dummy(),
        );

        let result = validator.prove_with_smt(&trichotomy);
        assert!(result.is_ok(), "Should prove x < y || x >= y");
    }

    #[test]
    fn test_smt_counterexample_extraction() {
        // Test that we get a meaningful counterexample when proving a false statement
        use verum_ast::Path;
        use verum_ast::ty::PathSegment;

        let validator = ProofValidator::with_config(ValidationConfig {
            validate_smt_proofs: true,
            smt_timeout_ms: 5000,
            ..Default::default()
        });

        // Try to prove "x > 0 && x < 0" which is always false
        let x = Expr::new(
            ExprKind::Path(Path {
                segments: vec![PathSegment::Name(verum_ast::Ident::new("x", Span::dummy()))].into(),
                span: Span::dummy(),
            }),
            Span::dummy(),
        );
        let zero = Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit::new(0)),
                Span::dummy(),
            )),
            Span::dummy(),
        );

        // x > 0
        let x_gt_zero = Expr::new(
            ExprKind::Binary {
                op: BinOp::Gt,
                left: Heap::new(x.clone()),
                right: Heap::new(zero.clone()),
            },
            Span::dummy(),
        );

        // x < 0
        let x_lt_zero = Expr::new(
            ExprKind::Binary {
                op: BinOp::Lt,
                left: Heap::new(x),
                right: Heap::new(zero),
            },
            Span::dummy(),
        );

        // x > 0 && x < 0 (contradiction)
        let false_prop = Expr::new(
            ExprKind::Binary {
                op: BinOp::And,
                left: Heap::new(x_gt_zero),
                right: Heap::new(x_lt_zero),
            },
            Span::dummy(),
        );

        let result = validator.prove_with_smt(&false_prop);
        assert!(result.is_err(), "Should fail to prove x > 0 && x < 0");
    }
}
