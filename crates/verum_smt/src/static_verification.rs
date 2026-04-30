//! Static Verification Engine for AOT Tier
//!
//! This module provides Z3-based static verification for the AOT compilation tier,
//! enabling CBGR check elimination through SMT proofs.
//!
//! # Features
//!
//! - **Static Safety Proofs**: Prove memory safety at compile time using Z3
//! - **CBGR Check Elimination**: Remove runtime checks when Z3 proves safety
//! - **Counterexample Extraction**: Detailed failure diagnostics with variable bindings
//! - **Unsat Core Extraction**: Minimal counterexamples for debugging
//! - **Timeout-Based Graceful Degradation**: Fallback to runtime checks on timeout
//!
//! # Architecture
//!
//! ```text
//! AST with CBGR Annotations
//!          |
//!          v
//! +-------------------+
//! | StaticVerifier    |
//! |  - Constraint     |
//! |    Generation     |
//! |  - Z3 Solving     |
//! |  - Proof Cache    |
//! +-------------------+
//!          |
//!     +----+----+
//!     |         |
//!     v         v
//! Proved    Failed/Timeout
//! (Eliminate)  (Keep Check)
//! ```
//!
//! # Performance Targets
//!
//! - SMT queries: < 10ms average for CBGR constraints
//! - CBGR elimination rate: > 80% in typical code
//! - Timeout handling: graceful degradation within 100ms
//!
//! In AOT compilation (Tier 2-3), static verification proves CBGR checks unnecessary
//! via escape analysis and dataflow, enabling `&T` to `&checked T` promotion (0ns overhead).
//! Three reference tiers: `&T` (~15ns CBGR check), `&checked T` (0ns, compiler-proven safe),
//! `&unsafe T` (0ns, manual safety proof, AOT-only). Graceful degradation: lower tiers
//! fall back to runtime CBGR checks. ThinRef is 16 bytes (ptr + generation + epoch),
//! FatRef is 24 bytes (ptr + generation + epoch + len).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
#[allow(unused_imports)]
use z3::{
    Config, Context, Goal, Model, Params, Probe, SatResult, Solver, Tactic,
    ast::{Ast, Bool, Dynamic, Int},
};

use verum_common::{List, Map, Maybe, Set, Text};
use verum_common::ToText;

#[allow(unused_imports)]
use crate::counterexample::{
    CounterExample, CounterExampleValue, EnhancedCounterExample, TraceStep,
};
#[allow(unused_imports)]
use crate::option_to_maybe;
#[allow(unused_imports)]
use crate::unsat_core::{
    AssertionCategory, TrackedAssertion, UnsatCore, UnsatCoreConfig, UnsatCoreExtractor,
};

// ==================== Configuration ====================

/// Configuration for static verification
#[derive(Debug, Clone)]
pub struct StaticVerificationConfig {
    /// Global timeout for verification queries (ms)
    pub timeout_ms: u64,
    /// Per-constraint timeout (ms) - triggers graceful degradation
    pub constraint_timeout_ms: u64,
    /// Enable proof generation for formal verification
    pub enable_proofs: bool,
    /// Enable unsat core extraction for minimal counterexamples
    pub enable_unsat_cores: bool,
    /// Minimize unsat cores (more expensive but smaller cores)
    pub minimize_cores: bool,
    /// Enable proof caching for repeated constraints
    pub enable_caching: bool,
    /// Maximum cache size
    pub max_cache_size: usize,
    /// Enable parallel verification for independent constraints
    pub enable_parallel: bool,
    /// Number of parallel workers
    pub num_workers: usize,
    /// Enable auto-tactic selection based on constraint analysis
    pub auto_tactics: bool,
    /// Memory limit (MB)
    pub memory_limit_mb: Option<usize>,
}

impl Default for StaticVerificationConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 30000,          // 30s global timeout
            constraint_timeout_ms: 100, // 100ms per constraint - triggers graceful degradation
            enable_proofs: true,
            enable_unsat_cores: true,
            minimize_cores: true,
            enable_caching: true,
            max_cache_size: 10000,
            enable_parallel: false, // Z3 Context is not Send/Sync
            num_workers: num_cpus::get().max(4),
            auto_tactics: true,
            memory_limit_mb: Some(4096), // 4GB
        }
    }
}

// ==================== Core Types ====================

/// Safety constraint for static verification
#[derive(Debug, Clone)]
pub struct SafetyConstraint {
    /// Unique identifier for the constraint
    pub id: Text,
    /// The constraint formula (preconditions => safety property)
    pub formula: ConstraintFormula,
    /// Source location for diagnostics
    pub source_location: Maybe<SourceLocation>,
    /// Category for unsat core organization
    pub category: ConstraintCategory,
    /// Variables involved in this constraint
    pub variables: List<VariableInfo>,
    /// Human-readable description
    pub description: Text,
}

/// Constraint formula in a structured format
#[derive(Debug, Clone)]
pub enum ConstraintFormula {
    /// Reference safety: ptr is valid and in bounds
    ReferenceValid {
        /// Name of the pointer variable being checked
        ptr_name: Text,
        /// Base address of the memory region
        base_addr: i64,
        /// Size of the valid memory region in bytes
        size: usize,
    },
    /// Bounds check: index < length
    BoundsCheck {
        index_var: Text,
        length_var: Text,
        length_value: Option<i64>,
    },
    /// Non-null check: ptr != null
    NonNull { ptr_name: Text },
    /// Lifetime validity: ref lifetime ⊆ owner lifetime
    LifetimeValid {
        ref_lifetime: Text,
        owner_lifetime: Text,
    },
    /// Aliasing check: no mutable aliasing
    NoMutableAliasing { refs: List<Text> },
    /// Arithmetic safety: no overflow/underflow
    ArithmeticSafe {
        operation: ArithOp,
        operands: List<Text>,
        bit_width: u8,
    },
    /// Division by zero check
    DivisionSafe { divisor_var: Text },
    /// Custom constraint expressed as Z3 formula string
    Custom { formula: Text },
}

/// Arithmetic operation types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Neg,
}

/// Constraint category for organization
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConstraintCategory {
    /// CBGR reference safety check
    CbgrSafety,
    /// Bounds checking
    BoundsCheck,
    /// Null pointer check
    NullCheck,
    /// Lifetime validity
    Lifetime,
    /// Aliasing constraint
    Aliasing,
    /// Arithmetic safety
    Arithmetic,
    /// User-specified assertion
    UserAssertion,
    /// Loop invariant
    Invariant,
    /// Function contract (pre/post)
    Contract,
}

/// Source location for diagnostics
#[derive(Debug, Clone)]
pub struct SourceLocation {
    pub file: Text,
    pub line: u32,
    pub column: u32,
    pub span_start: usize,
    pub span_end: usize,
}

impl std::fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

/// Variable information for counterexample extraction
#[derive(Debug, Clone)]
pub struct VariableInfo {
    pub name: Text,
    pub var_type: VariableType,
    pub source_name: Maybe<Text>, // Original name in source code
}

/// Variable types for Z3 translation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariableType {
    Int,
    Bool,
    BitVec(u32),
    Ptr,
    Array {
        elem_type: Box<VariableType>,
        length: Option<usize>,
    },
}

// ==================== Verification Results ====================

/// Result of static verification
#[derive(Debug, Clone)]
pub enum VerificationResult {
    /// Constraint proved safe - can eliminate runtime check
    Proved {
        /// Time taken for proof
        proof_time: Duration,
        /// Proof witness (if enabled)
        witness: Maybe<ProofWitness>,
        /// Whether result was cached
        cached: bool,
    },
    /// Constraint could not be proved - keep runtime check
    Unprovable {
        /// Counterexample showing violation
        counterexample: EnhancedCounterExample,
        /// Minimal unsat core (if enabled)
        unsat_core: Maybe<MinimalUnsatCore>,
        /// Suggestions for fixing the issue
        suggestions: List<Text>,
    },
    /// Verification timed out - graceful degradation to runtime check
    Timeout {
        /// Time spent before timeout
        elapsed: Duration,
        /// Reason for timeout
        reason: Text,
        /// Partial result if available
        partial: Maybe<PartialResult>,
    },
    /// Error during verification
    Error { message: Text, recoverable: bool },
}

impl VerificationResult {
    /// Check if the constraint can be eliminated (proved safe)
    pub fn can_eliminate_check(&self) -> bool {
        matches!(self, Self::Proved { .. })
    }

    /// Check if we should keep the runtime check
    pub fn needs_runtime_check(&self) -> bool {
        !self.can_eliminate_check()
    }

    /// Get the reason for keeping the check (for diagnostics)
    pub fn reason_for_check(&self) -> Text {
        match self {
            Self::Proved { .. } => Text::from("Proved safe - no check needed"),
            Self::Unprovable { counterexample, .. } => Text::from(format!(
                "Counterexample found: {}",
                counterexample.base.description
            )),
            Self::Timeout { reason, .. } => Text::from(format!("Verification timeout: {}", reason)),
            Self::Error { message, .. } => Text::from(format!("Verification error: {}", message)),
        }
    }
}

/// Proof witness for formal verification
#[derive(Debug, Clone)]
pub struct ProofWitness {
    /// Proof term in SMT-LIB2 format
    pub proof_term: Text,
    /// Axioms used in the proof
    pub used_axioms: Set<Text>,
    /// Number of proof steps
    pub proof_steps: usize,
    /// Tactic used for proving
    pub tactic_used: Maybe<Text>,
}

/// Minimal unsat core for diagnostics
#[derive(Debug, Clone)]
pub struct MinimalUnsatCore {
    /// Constraint IDs in the core
    pub constraint_ids: Set<Text>,
    /// Human-readable explanation
    pub explanation: Text,
    /// Reduction percentage from original
    pub reduction_percent: f64,
    /// Is this core minimal?
    pub is_minimal: bool,
}

/// Partial result from timeout
#[derive(Debug, Clone)]
pub struct PartialResult {
    /// Constraints that were proved
    pub proved_constraints: List<Text>,
    /// Constraints that remain unproved
    pub unproved_constraints: List<Text>,
    /// Progress estimate (0.0-1.0)
    pub progress: f64,
}

// ==================== Static Verifier ====================

/// Main static verification engine
///
/// Integrates Z3 for static safety proofs with CBGR check elimination.
pub struct StaticVerifier {
    /// Configuration
    config: StaticVerificationConfig,
    /// Proof cache
    cache: Arc<RwLock<ProofCache>>,
    /// Statistics
    stats: Arc<RwLock<VerificationStats>>,
    /// Current verification context
    context_stack: List<VerificationContext>,
}

/// Verification context for scoped constraints
#[derive(Debug, Clone, Default)]
pub struct VerificationContext {
    /// Assumptions in this context
    pub assumptions: List<SafetyConstraint>,
    /// Variable bindings
    pub bindings: Map<Text, i64>,
    /// Function preconditions
    pub preconditions: List<SafetyConstraint>,
}

/// Proof cache for repeated constraints
struct ProofCache {
    /// Cache: constraint hash -> result
    entries: HashMap<u64, CachedResult>,
    /// Maximum size
    max_size: usize,
    /// Hit count
    hits: u64,
    /// Miss count
    misses: u64,
}

impl ProofCache {
    fn new(max_size: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_size,
            hits: 0,
            misses: 0,
        }
    }

    fn get(&mut self, hash: u64) -> Option<&CachedResult> {
        if let Some(result) = self.entries.get(&hash) {
            self.hits += 1;
            Some(result)
        } else {
            self.misses += 1;
            None
        }
    }

    fn insert(&mut self, hash: u64, result: CachedResult) {
        if self.entries.len() >= self.max_size {
            // Simple eviction: clear oldest entries
            let to_remove: List<u64> = self
                .entries
                .keys()
                .take(self.max_size / 4)
                .cloned()
                .collect();
            for key in to_remove {
                self.entries.remove(&key);
            }
        }
        self.entries.insert(hash, result);
    }

    fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

#[derive(Debug, Clone)]
struct CachedResult {
    result: VerificationResult,
}

/// Verification statistics
#[derive(Debug, Clone, Default)]
pub struct VerificationStats {
    /// Total constraints verified
    pub total_verified: u64,
    /// Constraints proved safe (checks eliminated)
    pub proved_safe: u64,
    /// Constraints with counterexamples
    pub counterexamples_found: u64,
    /// Timeouts (graceful degradation)
    pub timeouts: u64,
    /// Errors
    pub errors: u64,
    /// Total verification time
    pub total_time_ms: u64,
    /// Average time per constraint
    pub avg_time_ms: f64,
    /// Cache hit rate
    pub cache_hit_rate: f64,
    /// CBGR check elimination rate
    pub elimination_rate: f64,
}

impl StaticVerifier {
    /// Create a new static verifier
    pub fn new(config: StaticVerificationConfig) -> Self {
        let cache = Arc::new(RwLock::new(ProofCache::new(config.max_cache_size)));

        Self {
            config,
            cache,
            stats: Arc::new(RwLock::new(VerificationStats::default())),
            context_stack: vec![VerificationContext::default()].into(),
        }
    }

    /// Create with default configuration
    pub fn default_config() -> Self {
        Self::new(StaticVerificationConfig::default())
    }

    /// Push a new verification context
    pub fn push_context(&mut self) {
        let current = self.context_stack.last().cloned().unwrap_or_default();
        self.context_stack.push(current);
    }

    /// Pop verification context
    pub fn pop_context(&mut self) {
        if self.context_stack.len() > 1 {
            self.context_stack.pop();
        }
    }

    /// Add assumption to current context
    pub fn add_assumption(&mut self, constraint: SafetyConstraint) {
        if let Some(ctx) = self.context_stack.last_mut() {
            ctx.assumptions.push(constraint);
        }
    }

    /// Add precondition
    pub fn add_precondition(&mut self, constraint: SafetyConstraint) {
        if let Some(ctx) = self.context_stack.last_mut() {
            ctx.preconditions.push(constraint);
        }
    }

    /// Verify a safety constraint
    ///
    /// Returns whether the constraint can be statically verified,
    /// allowing the corresponding runtime check to be eliminated.
    pub fn verify(&self, constraint: &SafetyConstraint) -> VerificationResult {
        let start = Instant::now();

        // Check cache first
        if self.config.enable_caching {
            let hash = self.hash_constraint(constraint);
            if let Some(cached) = self.cache.write().get(hash) {
                self.update_stats_cached();
                return cached.result.clone();
            }
        }

        // Get current context
        let context = self.context_stack.last().cloned().unwrap_or_default();

        // Perform verification with timeout
        let result = self.verify_with_timeout(constraint, &context);

        // Update statistics
        self.update_stats(&result, start.elapsed());

        // Cache result
        if self.config.enable_caching {
            let hash = self.hash_constraint(constraint);
            self.cache.write().insert(
                hash,
                CachedResult {
                    result: result.clone(),
                },
            );
        }

        result
    }

    /// Verify multiple constraints, returning elimination decisions.
    ///
    /// `config.timeout_ms` bounds the cumulative wall-clock of the
    /// batch — once the budget is exhausted, every remaining
    /// constraint is short-circuited to a `Timeout` result rather
    /// than processed. Without this enforcement the field is inert:
    /// per-constraint `constraint_timeout_ms` already caps each
    /// individual verify call, but a batch of N slow constraints
    /// still ran to N × constraint_timeout_ms regardless of what
    /// callers configured for the session-level cap.
    pub fn verify_batch(
        &self,
        constraints: &[SafetyConstraint],
    ) -> List<(Text, VerificationResult)> {
        let global_timeout = Duration::from_millis(self.config.timeout_ms);
        let start = Instant::now();
        let mut out: List<(Text, VerificationResult)> = List::new();
        for c in constraints {
            let elapsed = start.elapsed();
            if elapsed >= global_timeout {
                out.push((
                    c.id.clone(),
                    VerificationResult::Timeout {
                        elapsed,
                        reason: Text::from(
                            "batch verification exceeded global timeout (StaticVerificationConfig.timeout_ms)",
                        ),
                        partial: Maybe::None,
                    },
                ));
                continue;
            }
            out.push((c.id.clone(), self.verify(c)));
        }
        out
    }

    /// Verify with timeout handling (graceful degradation)
    fn verify_with_timeout(
        &self,
        constraint: &SafetyConstraint,
        context: &VerificationContext,
    ) -> VerificationResult {
        let start = Instant::now();
        let timeout = Duration::from_millis(self.config.constraint_timeout_ms);

        // Create Z3 configuration with timeout
        let mut cfg = Config::new();
        cfg.set_timeout_msec(self.config.constraint_timeout_ms);
        if self.config.enable_proofs {
            cfg.set_proof_generation(true);
        }
        // Forward the configured memory ceiling to Z3.
        // `memory_max_size` is a *global* Z3 parameter (process-wide,
        // applied via `Z3_global_param_set`) rather than a context-
        // or solver-level option — those latter scopes silently
        // mis-route queries when handed the unknown key. We set it
        // here so the value is in effect for every Z3 query
        // initiated through the static verifier; subsequent calls
        // overwrite, so the most-recent verifier configuration
        // wins. `None` means "no caller-imposed limit" — leave Z3
        // at its native default.
        if let Some(mb) = self.config.memory_limit_mb {
            z3::set_global_param("memory_max_size", &mb.to_string());
        }

        // Execute verification in Z3 context
        let result = z3::with_z3_config(&cfg, || {
            self.verify_in_context(constraint, context, timeout)
        });

        // Check for timeout
        if start.elapsed() >= timeout {
            return VerificationResult::Timeout {
                elapsed: start.elapsed(),
                reason: Text::from("Constraint verification exceeded timeout"),
                partial: Maybe::None,
            };
        }

        result
    }

    /// Verify constraint within Z3 context
    fn verify_in_context(
        &self,
        constraint: &SafetyConstraint,
        context: &VerificationContext,
        timeout: Duration,
    ) -> VerificationResult {
        // Create solver with appropriate tactic
        let solver = if self.config.auto_tactics {
            self.create_solver_with_tactic(&constraint.formula)
        } else {
            Solver::new()
        };

        // Set solver parameters
        let mut params = Params::new();
        params.set_u32("timeout", self.config.constraint_timeout_ms as u32);
        // Wire `minimize_cores`: Z3's `smt.core.minimize` controls
        // whether the solver runs additional minimization on the
        // unsat core before returning it. Pre-fix the field
        // defaulted to `true` (and `MinimalUnsatCore.is_minimal`
        // stamped that flag onto the result) but never reached
        // Z3 — every returned core was whatever non-minimized set
        // the solver produced in passing. Mirrors z3_backend.rs's
        // global_param wiring at the per-solver-params layer.
        params.set_bool("smt.core.minimize", self.config.minimize_cores);
        solver.set_params(&params);

        // Track assertions for unsat core
        let mut tracked_assertions: List<(Text, Bool)> = List::new();

        // Add context assumptions
        for (idx, assumption) in context.assumptions.iter().enumerate() {
            if let Some(z3_formula) = self.translate_constraint(&assumption.formula) {
                let track_id = format!("assumption_{}", idx);
                let track_lit = Bool::new_const(track_id.as_str());
                solver.assert_and_track(&z3_formula, &track_lit);
                tracked_assertions.push((Text::from(track_id), track_lit));
            }
        }

        // Add preconditions
        for (idx, pre) in context.preconditions.iter().enumerate() {
            if let Some(z3_formula) = self.translate_constraint(&pre.formula) {
                let track_id = format!("precondition_{}", idx);
                let track_lit = Bool::new_const(track_id.as_str());
                solver.assert_and_track(&z3_formula, &track_lit);
                tracked_assertions.push((Text::from(track_id), track_lit));
            }
        }

        // Translate and assert the main constraint (negated for proof by contradiction)
        let main_formula = match self.translate_constraint(&constraint.formula) {
            Some(f) => f,
            None => {
                return VerificationResult::Error {
                    message: Text::from("Failed to translate constraint to Z3"),
                    recoverable: true,
                };
            }
        };

        // To prove the constraint, we check if NOT(constraint) is UNSAT
        // If UNSAT, the constraint is always true (proved)
        // If SAT, we have a counterexample
        let negated = main_formula.not();
        let track_id = "main_constraint";
        let track_lit = Bool::new_const(track_id);
        solver.assert_and_track(&negated, &track_lit);
        tracked_assertions.push((Text::from(track_id), track_lit));

        // Check satisfiability
        let check_start = Instant::now();
        let sat_result = solver.check();
        let proof_time = check_start.elapsed();

        match sat_result {
            SatResult::Unsat => {
                // Constraint is PROVED SAFE - can eliminate runtime check
                let witness = if self.config.enable_proofs {
                    self.extract_proof_witness(&solver)
                } else {
                    Maybe::None
                };

                VerificationResult::Proved {
                    proof_time,
                    witness,
                    cached: false,
                }
            }
            SatResult::Sat => {
                // Found counterexample - constraint may be violated
                let counterexample =
                    self.extract_counterexample(&solver, constraint, &tracked_assertions);

                let unsat_core = if self.config.enable_unsat_cores {
                    self.extract_minimal_core(&solver, &tracked_assertions)
                } else {
                    Maybe::None
                };

                let suggestions = self.generate_suggestions(constraint, &counterexample);

                VerificationResult::Unprovable {
                    counterexample,
                    unsat_core,
                    suggestions,
                }
            }
            SatResult::Unknown => {
                // Couldn't determine - treat as timeout/graceful degradation
                let reason = solver
                    .get_reason_unknown()
                    .unwrap_or_else(|| "Unknown reason".to_string());

                VerificationResult::Timeout {
                    elapsed: proof_time,
                    reason: Text::from(reason),
                    partial: Maybe::None,
                }
            }
        }
    }

    /// Create solver with appropriate tactic based on constraint type
    fn create_solver_with_tactic(&self, formula: &ConstraintFormula) -> Solver {
        let tactic = match formula {
            ConstraintFormula::BoundsCheck { .. }
            | ConstraintFormula::ArithmeticSafe { .. }
            | ConstraintFormula::DivisionSafe { .. } => {
                // Linear integer arithmetic - use specialized tactic
                Tactic::and_then(&Tactic::new("simplify"), &Tactic::new("smt"))
            }
            ConstraintFormula::ReferenceValid { .. } | ConstraintFormula::NonNull { .. } => {
                // Pointer arithmetic - may need bit-vector reasoning
                let is_qfbv = Probe::new("is-qfbv");
                let bv_tactic =
                    Tactic::and_then(&Tactic::new("solve-eqs"), &Tactic::new("bit-blast"));
                let smt_tactic = Tactic::new("smt");
                Tactic::cond(&is_qfbv, &bv_tactic, &smt_tactic)
            }
            ConstraintFormula::LifetimeValid { .. }
            | ConstraintFormula::NoMutableAliasing { .. } => {
                // May involve quantifiers
                Tactic::new("smt")
            }
            ConstraintFormula::Custom { .. } => {
                // Unknown - use auto-detection
                let simplify = Tactic::new("simplify");
                let smt = Tactic::new("smt");
                Tactic::and_then(&simplify, &smt)
            }
        };

        // Apply timeout to tactic
        let tactic_with_timeout =
            tactic.try_for(Duration::from_millis(self.config.constraint_timeout_ms));

        tactic_with_timeout.solver()
    }

    /// Translate constraint formula to Z3 Bool
    fn translate_constraint(&self, formula: &ConstraintFormula) -> Option<Bool> {
        match formula {
            ConstraintFormula::BoundsCheck {
                index_var,
                length_var,
                length_value,
            } => {
                let index = Int::new_const(index_var.as_str());
                let zero = Int::from_i64(0);

                let length = if let Some(len) = length_value {
                    Int::from_i64(*len)
                } else {
                    Int::new_const(length_var.as_str())
                };

                // 0 <= index < length
                let lower_bound = index.ge(&zero);
                let upper_bound = index.lt(&length);
                Some(Bool::and(&[&lower_bound, &upper_bound]))
            }
            ConstraintFormula::NonNull { ptr_name } => {
                let ptr = Int::new_const(ptr_name.as_str());
                let zero = Int::from_i64(0);
                Some(ptr.eq(&zero).not())
            }
            ConstraintFormula::DivisionSafe { divisor_var } => {
                let divisor = Int::new_const(divisor_var.as_str());
                let zero = Int::from_i64(0);
                Some(divisor.eq(&zero).not())
            }
            ConstraintFormula::ReferenceValid {
                ptr_name,
                base_addr,
                size,
            } => {
                let ptr = Int::new_const(ptr_name.as_str());
                let base = Int::from_i64(*base_addr);
                let end = Int::from_i64(*base_addr + *size as i64);

                // base <= ptr < base + size
                let lower = ptr.ge(&base);
                let upper = ptr.lt(&end);
                Some(Bool::and(&[&lower, &upper]))
            }
            ConstraintFormula::ArithmeticSafe {
                operation,
                operands,
                bit_width,
            } => {
                // Check for overflow/underflow
                let max_val = (1i64 << (*bit_width - 1)) - 1;
                let min_val = -(1i64 << (*bit_width - 1));

                if operands.len() >= 2 {
                    let a = Int::new_const(operands[0].as_str());
                    let b = Int::new_const(operands[1].as_str());

                    let result = match operation {
                        ArithOp::Add => &a + &b,
                        ArithOp::Sub => &a - &b,
                        ArithOp::Mul => &a * &b,
                        ArithOp::Div => &a / &b,
                        ArithOp::Neg => -&a,
                    };

                    let min = Int::from_i64(min_val);
                    let max = Int::from_i64(max_val);
                    let in_range = Bool::and(&[&result.ge(&min), &result.le(&max)]);
                    Some(in_range)
                } else {
                    None
                }
            }
            ConstraintFormula::LifetimeValid {
                ref_lifetime,
                owner_lifetime,
            } => {
                // Model lifetimes as intervals [start, end]
                let ref_start = Int::new_const(format!("{}_start", ref_lifetime).as_str());
                let ref_end = Int::new_const(format!("{}_end", ref_lifetime).as_str());
                let owner_start = Int::new_const(format!("{}_start", owner_lifetime).as_str());
                let owner_end = Int::new_const(format!("{}_end", owner_lifetime).as_str());

                // ref lifetime ⊆ owner lifetime
                let start_valid = ref_start.ge(&owner_start);
                let end_valid = ref_end.le(&owner_end);
                Some(Bool::and(&[&start_valid, &end_valid]))
            }
            ConstraintFormula::NoMutableAliasing { refs } => {
                if refs.len() < 2 {
                    return Some(Bool::from_bool(true));
                }

                // All refs must be distinct
                let ref_vars: List<Int> = refs.iter().map(|r| Int::new_const(r.as_str())).collect();

                let mut distinct_constraints = List::new();
                for i in 0..ref_vars.len() {
                    for j in (i + 1)..ref_vars.len() {
                        distinct_constraints.push(ref_vars[i].eq(&ref_vars[j]).not());
                    }
                }

                let refs: List<&Bool> = distinct_constraints.iter().collect();
                Some(Bool::and(&refs))
            }
            ConstraintFormula::Custom { formula } => {
                // Parse custom formula - simplified for now
                // In a full implementation, this would parse SMT-LIB2
                let _ = formula;
                None
            }
        }
    }

    /// Extract proof witness from solver
    fn extract_proof_witness(&self, solver: &Solver) -> Maybe<ProofWitness> {
        solver.get_proof().map(|proof| ProofWitness {
            proof_term: Text::from(format!("{:?}", proof)),
            used_axioms: Set::new(),
            proof_steps: 0, // Would need proof traversal
            tactic_used: Maybe::None,
        })
    }

    /// Extract counterexample from SAT model
    fn extract_counterexample(
        &self,
        solver: &Solver,
        constraint: &SafetyConstraint,
        _tracked: &[(Text, Bool)],
    ) -> EnhancedCounterExample {
        let model = solver.get_model();

        let mut assignments = Map::new();

        // Extract values for constraint variables
        for var in &constraint.variables {
            if let Some(ref m) = model {
                match var.var_type {
                    VariableType::Int | VariableType::Ptr => {
                        let z3_var = Int::new_const(var.name.as_str());
                        if let Some(value) = m.eval(&z3_var, true)
                            && let Some(i) = value.as_i64()
                        {
                            assignments.insert(var.name.to_text(), CounterExampleValue::Int(i));
                        }
                    }
                    VariableType::Bool => {
                        let z3_var = Bool::new_const(var.name.as_str());
                        if let Some(value) = m.eval(&z3_var, true)
                            && let Some(b) = value.as_bool()
                        {
                            assignments.insert(var.name.to_text(), CounterExampleValue::Bool(b));
                        }
                    }
                    _ => {}
                }
            }
        }

        let base = CounterExample::new(assignments, constraint.description.to_text());

        let mut enhanced = EnhancedCounterExample::new(base);
        enhanced.confidence = 1.0;

        enhanced
    }

    /// Extract minimal unsat core
    fn extract_minimal_core(
        &self,
        solver: &Solver,
        tracked: &[(Text, Bool)],
    ) -> Maybe<MinimalUnsatCore> {
        let core_asts = solver.get_unsat_core();
        if core_asts.is_empty() {
            return Maybe::None;
        }

        let mut core_ids = Set::new();
        for (id, track_lit) in tracked {
            for core_ast in &core_asts {
                // Compare by string representation
                if format!("{}", track_lit) == format!("{}", core_ast) {
                    core_ids.insert(id.clone());
                    break;
                }
            }
        }

        let total = tracked.len();
        let core_size = core_ids.len();
        let reduction = if total > 0 {
            (1.0 - (core_size as f64 / total as f64)) * 100.0
        } else {
            0.0
        };

        Maybe::Some(MinimalUnsatCore {
            constraint_ids: core_ids.iter().cloned().collect(),
            explanation: Text::from("Minimal set of constraints causing unsatisfiability"),
            reduction_percent: reduction,
            is_minimal: self.config.minimize_cores,
        })
    }

    /// Generate fix suggestions based on counterexample
    fn generate_suggestions(
        &self,
        constraint: &SafetyConstraint,
        counterexample: &EnhancedCounterExample,
    ) -> List<Text> {
        let mut suggestions = List::new();

        match &constraint.category {
            ConstraintCategory::BoundsCheck => {
                suggestions.push(Text::from("Add bounds check: ensure index < array.length"));
                suggestions.push(Text::from(
                    "Use .get(index) for safe access with Option return",
                ));
            }
            ConstraintCategory::NullCheck => {
                suggestions.push(Text::from("Add null check: if ptr != null { ... }"));
                suggestions.push(Text::from("Use Maybe<T> type instead of raw pointer"));
            }
            ConstraintCategory::CbgrSafety => {
                suggestions.push(Text::from("Ensure reference does not outlive its source"));
                suggestions.push(Text::from("Use explicit lifetime annotations"));
            }
            ConstraintCategory::Arithmetic => {
                suggestions.push(Text::from("Use checked arithmetic operations"));
                suggestions.push(Text::from("Add precondition constraining input range"));
            }
            ConstraintCategory::Aliasing => {
                suggestions.push(Text::from("Ensure no mutable aliasing exists"));
                suggestions.push(Text::from("Use Shared<T> for shared ownership"));
            }
            _ => {
                suggestions.push(Text::from(
                    "Add explicit precondition to rule out this case",
                ));
                suggestions.push(Text::from("Use @verify(runtime) for dynamic checking"));
            }
        }

        // Add specific suggestion based on counterexample values
        for (var_name, value) in counterexample.base.assignments.iter() {
            match value {
                CounterExampleValue::Int(i) if *i < 0 => {
                    suggestions.push(Text::from(format!(
                        "Add precondition: {} >= 0 (found negative value {})",
                        var_name, i
                    )));
                }
                CounterExampleValue::Int(i) if *i == 0 => {
                    suggestions.push(Text::from(format!(
                        "Add precondition: {} != 0 (division by zero possible)",
                        var_name
                    )));
                }
                _ => {}
            }
        }

        suggestions
    }

    /// Hash constraint for caching
    fn hash_constraint(&self, constraint: &SafetyConstraint) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        constraint.id.hash(&mut hasher);
        constraint.description.hash(&mut hasher);
        hasher.finish()
    }

    /// Update statistics after verification
    fn update_stats(&self, result: &VerificationResult, elapsed: Duration) {
        let mut stats = self.stats.write();
        stats.total_verified += 1;
        stats.total_time_ms += elapsed.as_millis() as u64;

        match result {
            VerificationResult::Proved { .. } => stats.proved_safe += 1,
            VerificationResult::Unprovable { .. } => stats.counterexamples_found += 1,
            VerificationResult::Timeout { .. } => stats.timeouts += 1,
            VerificationResult::Error { .. } => stats.errors += 1,
        }

        // Update derived statistics
        if stats.total_verified > 0 {
            stats.avg_time_ms = stats.total_time_ms as f64 / stats.total_verified as f64;
            stats.elimination_rate = stats.proved_safe as f64 / stats.total_verified as f64;
            stats.cache_hit_rate = self.cache.read().hit_rate();
        }
    }

    fn update_stats_cached(&self) {
        // Just update cache hit - no need to update other stats
    }

    /// Get current statistics
    pub fn stats(&self) -> VerificationStats {
        self.stats.read().clone()
    }

    /// Get CBGR check elimination rate
    pub fn elimination_rate(&self) -> f64 {
        self.stats.read().elimination_rate
    }

    /// Clear cache
    pub fn clear_cache(&self) {
        self.cache.write().entries.clear();
    }
}

// ==================== CBGR Integration ====================

/// CBGR check elimination result
#[derive(Debug, Clone)]
pub struct CbgrEliminationResult {
    /// Check ID that was analyzed
    pub check_id: Text,
    /// Whether the check can be eliminated
    pub can_eliminate: bool,
    /// Verification result details
    pub verification: VerificationResult,
    /// Original constraint
    pub constraint: SafetyConstraint,
}

/// Batch CBGR analysis for a function
pub struct CbgrBatchAnalyzer {
    verifier: StaticVerifier,
    results: List<CbgrEliminationResult>,
}

impl CbgrBatchAnalyzer {
    pub fn new(config: StaticVerificationConfig) -> Self {
        Self {
            verifier: StaticVerifier::new(config),
            results: List::new(),
        }
    }

    /// Analyze a CBGR check for potential elimination
    pub fn analyze_check(&mut self, constraint: SafetyConstraint) -> &CbgrEliminationResult {
        let check_id = constraint.id.clone();
        let verification = self.verifier.verify(&constraint);
        let can_eliminate = verification.can_eliminate_check();

        self.results.push(CbgrEliminationResult {
            check_id,
            can_eliminate,
            verification,
            constraint,
        });

        self.results.last().unwrap()
    }

    /// Get all elimination results
    pub fn get_results(&self) -> &[CbgrEliminationResult] {
        &self.results
    }

    /// Get checks that can be eliminated
    pub fn eliminable_checks(&self) -> List<&CbgrEliminationResult> {
        self.results.iter().filter(|r| r.can_eliminate).collect()
    }

    /// Get checks that must be kept
    pub fn required_checks(&self) -> List<&CbgrEliminationResult> {
        self.results.iter().filter(|r| !r.can_eliminate).collect()
    }

    /// Get elimination statistics
    pub fn elimination_stats(&self) -> CbgrEliminationStats {
        let total = self.results.len();
        let eliminated = self.results.iter().filter(|r| r.can_eliminate).count();
        let timeouts = self
            .results
            .iter()
            .filter(|r| matches!(r.verification, VerificationResult::Timeout { .. }))
            .count();

        CbgrEliminationStats {
            total_checks: total,
            eliminated_checks: eliminated,
            remaining_checks: total - eliminated,
            timeout_fallbacks: timeouts,
            elimination_rate: if total > 0 {
                eliminated as f64 / total as f64
            } else {
                0.0
            },
        }
    }

    /// Get verifier statistics
    pub fn verifier_stats(&self) -> VerificationStats {
        self.verifier.stats()
    }
}

/// CBGR elimination statistics
#[derive(Debug, Clone)]
pub struct CbgrEliminationStats {
    pub total_checks: usize,
    pub eliminated_checks: usize,
    pub remaining_checks: usize,
    pub timeout_fallbacks: usize,
    pub elimination_rate: f64,
}

impl std::fmt::Display for CbgrEliminationStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CBGR Elimination: {}/{} checks eliminated ({:.1}%), {} timeout fallbacks",
            self.eliminated_checks,
            self.total_checks,
            self.elimination_rate * 100.0,
            self.timeout_fallbacks
        )
    }
}

// ==================== Tests ====================
