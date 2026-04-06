//! Core refinement type verification using Z3.
//!
//! This module provides the main verification logic for checking if values
//! satisfy refinement predicates.

use crate::context::Context;
use crate::cost::CostMeasurement;
pub use crate::cost::VerificationCost;
use crate::counterexample::{CounterExample, CounterExampleExtractor, generate_suggestions};
use crate::translate::{TranslationError, Translator};
use crate::verification_cache::VerificationCache;
use std::sync::OnceLock;
use std::time::Duration;
use verum_ast::ty::GenericArg;
use verum_ast::{Expr, ExprKind, Type, TypeKind};
use verum_common::{List, Text};
use verum_common::ToText;

// Global verification cache (lazy-initialized, thread-safe)
static VERIFICATION_CACHE: OnceLock<VerificationCache> = OnceLock::new();

/// Get the global verification cache.
fn get_verification_cache() -> &'static VerificationCache {
    VERIFICATION_CACHE.get_or_init(VerificationCache::new)
}

/// Get verification cache statistics.
///
/// Returns metrics about cache performance including hit rate, size, and timing.
/// Useful for monitoring and optimizing verification performance.
///
/// # Example
/// ```ignore
/// let stats = get_cache_stats();
/// println!("Cache hit rate: {:.1}%", stats.hit_rate() * 100.0);
/// println!("Current size: {}/{}", stats.current_size, stats.max_size);
/// ```
pub fn get_cache_stats() -> crate::verification_cache::CacheStats {
    get_verification_cache().stats()
}

/// Clear the verification cache.
///
/// This removes all cached verification results. Useful for testing or when
/// starting a new compilation session.
pub fn clear_cache() {
    get_verification_cache().clear();
}

/// Verification mode controlling how checks are performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerifyMode {
    /// Skip SMT verification, use runtime checks only
    Runtime,

    /// Full SMT verification (may be slow)
    Proof,

    /// Automatic mode - uses heuristics to decide
    #[default]
    Auto,
}

/// Result of a successful proof.
#[derive(Debug, Clone)]
pub struct ProofResult {
    /// Time taken for verification
    pub cost: VerificationCost,

    /// Whether the proof was cached
    pub cached: bool,

    /// SMT-LIB2 output (if requested)
    pub smt_lib: Option<Text>,

    /// Extracted proof witness (if proof generation is enabled)
    pub proof_witness: Option<crate::z3_backend::ProofWitness>,

    /// Raw proof object (if available)
    pub raw_proof: Option<Text>,

    /// Proof validation result (if validated)
    pub validation: Option<crate::proof_extraction::ProofValidation>,

    /// Structured proof term extracted from Z3 (for downstream analysis)
    pub structured_proof: Option<crate::proof_extraction::ProofTerm>,
}

impl ProofResult {
    /// Create a new proof result.
    pub fn new(cost: VerificationCost) -> Self {
        Self {
            cost,
            cached: false,
            smt_lib: None,
            proof_witness: None,
            raw_proof: None,
            validation: None,
            structured_proof: None,
        }
    }

    /// Mark as cached.
    pub fn with_cached(mut self) -> Self {
        self.cached = true;
        self
    }

    /// Add SMT-LIB2 output.
    pub fn with_smt_lib(mut self, smt_lib: Text) -> Self {
        self.smt_lib = Some(smt_lib);
        self
    }

    /// Add proof witness.
    pub fn with_proof_witness(mut self, witness: crate::z3_backend::ProofWitness) -> Self {
        self.proof_witness = Some(witness);
        self
    }

    /// Add raw proof.
    pub fn with_raw_proof(mut self, proof: Text) -> Self {
        self.raw_proof = Some(proof);
        self
    }

    /// Add proof validation result.
    pub fn with_validation(mut self, validation: crate::proof_extraction::ProofValidation) -> Self {
        self.validation = Some(validation);
        self
    }

    /// Add structured proof term.
    ///
    /// This stores the fully parsed proof term from Z3 for downstream analysis,
    /// proof minimization, and export to other proof formats.
    pub fn with_structured_proof(mut self, proof: crate::proof_extraction::ProofTerm) -> Self {
        self.structured_proof = Some(proof);
        self
    }

    /// Check if this result has a proof
    pub fn has_proof(&self) -> bool {
        self.proof_witness.is_some() || self.raw_proof.is_some() || self.structured_proof.is_some()
    }

    /// Check if this result has a structured proof
    pub fn has_structured_proof(&self) -> bool {
        self.structured_proof.is_some()
    }

    /// Check if the proof was validated
    pub fn is_validated(&self) -> bool {
        self.validation.as_ref().map(|v| v.is_ok()).unwrap_or(false)
    }
}

/// Result of a verification operation.
pub type VerificationResult = Result<ProofResult, VerificationError>;

/// Errors that can occur during verification.
#[derive(Debug, Clone, thiserror::Error)]
pub enum VerificationError {
    /// Constraint cannot be proven
    #[error("cannot prove constraint: {constraint}")]
    CannotProve {
        /// The constraint that could not be proven
        constraint: Text,
        /// A counterexample demonstrating the constraint violation
        counterexample: Option<CounterExample>,
        /// Cost metrics for this verification attempt
        cost: VerificationCost,
        /// Suggestions for fixing the issue
        suggestions: List<Text>,
    },

    /// Verification timed out
    #[error("verification timeout after {timeout:?}")]
    Timeout {
        /// The constraint being verified
        constraint: Text,
        /// The timeout duration that was exceeded
        timeout: Duration,
        /// Cost metrics up to the timeout
        cost: VerificationCost,
    },

    /// Translation error
    #[error("translation error: {0}")]
    Translation(#[from] TranslationError),

    /// SMT solver error
    #[error("SMT solver error: {0}")]
    SolverError(Text),

    /// Unknown result from solver
    #[error("unknown verification result for: {0}")]
    Unknown(Text),
}

impl VerificationError {
    /// Get the cost if available.
    pub fn cost(&self) -> Option<&VerificationCost> {
        match self {
            VerificationError::CannotProve { cost, .. } => Some(cost),
            VerificationError::Timeout { cost, .. } => Some(cost),
            _ => None,
        }
    }

    /// Get suggestions if available.
    pub fn suggestions(&self) -> &[Text] {
        match self {
            VerificationError::CannotProve { suggestions, .. } => suggestions,
            _ => &[],
        }
    }
}

/// Verify a refinement type constraint.
///
/// # Arguments
/// * `context` - The Z3 context
/// * `ty` - The type being verified (should be a refinement type)
/// * `value_expr` - Optional expression representing the value to check
/// * `mode` - Verification mode
///
/// # Returns
/// * `Ok(ProofResult)` if the constraint is proven
/// * `Err(VerificationError)` if the constraint cannot be proven or verification fails
pub fn verify_refinement(
    context: &Context,
    ty: &Type,
    _value_expr: Option<&Expr>,
    mode: VerifyMode,
) -> VerificationResult {
    // Extract refinement predicate
    let (base_type, predicate) = match &ty.kind {
        TypeKind::Refined { base, predicate } => (&**base, &predicate.expr),
        _ => {
            return Err(VerificationError::SolverError(
                "not a refinement type".to_text(),
            ));
        }
    };

    // Check if we should skip SMT based on mode
    if mode == VerifyMode::Runtime {
        // Return success immediately - runtime checks will handle this
        let cost = VerificationCost::new("runtime".into(), Duration::ZERO, true);
        return Ok(ProofResult::new(cost));
    }

    // PERFORMANCE: Try cache first (10-100x speedup for incremental builds)
    let cache = get_verification_cache();

    cache.get_or_verify(predicate, base_type, || {
        verify_refinement_uncached(context, base_type, predicate, mode)
    })
}

/// Uncached verification (internal helper).
///
/// This function performs the actual SMT verification without caching.
/// It is called by verify_refinement when a cache miss occurs.
fn verify_refinement_uncached(
    context: &Context,
    base_type: &Type,
    predicate: &Expr,
    _mode: VerifyMode,
) -> VerificationResult {
    // Start cost measurement
    let measurement = CostMeasurement::start("refinement_check");

    // Create translator
    let translator = Translator::new(context);

    // Create a variable for the value being checked (use 'it' as convention)
    let var_name = "it";

    // Create Z3 variable for the base type
    let z3_var = translator.create_var(var_name, base_type)?;

    // Bind the variable
    let mut translator = translator;
    translator.bind(var_name.to_text(), z3_var.clone());

    // Translate the predicate
    let z3_predicate = translator.translate_expr(predicate)?;

    // Convert to boolean
    let z3_bool = z3_predicate
        .as_bool()
        .ok_or_else(|| VerificationError::SolverError("predicate is not boolean".to_text()))?;

    // Create solver and check if there exists a value that violates the constraint
    let solver = context.solver();

    // We want to find if there's a value where the predicate is FALSE
    // (i.e., a counterexample)
    solver.assert(z3_bool.not());

    // Check satisfiability
    let check_result = solver.check();

    match check_result {
        z3::SatResult::Unsat => {
            // No counterexample exists - the constraint always holds!
            let cost = measurement.finish(true);
            Ok(ProofResult::new(cost))
        }

        z3::SatResult::Sat => {
            // Found a counterexample - constraint can be violated
            let model = solver
                .get_model()
                .ok_or_else(|| VerificationError::SolverError("no model available".to_text()))?;

            // Extract counterexample
            let extractor = CounterExampleExtractor::new(&model);
            let counterexample =
                extractor.extract(&[var_name.to_text()], &format!("{:?}", predicate));

            // Generate suggestions
            let suggestions = generate_suggestions(&counterexample, &format!("{:?}", predicate));

            let cost = measurement.finish(false);

            Err(VerificationError::CannotProve {
                constraint: format!("{:?}", predicate).into(),
                counterexample: Some(counterexample),
                cost,
                suggestions,
            })
        }

        z3::SatResult::Unknown => {
            // Solver couldn't determine result (timeout or too complex)
            let cost = measurement.finish(false);

            // Check if this was a timeout
            if let Some(timeout) = context.config().timeout
                && cost.duration >= timeout
            {
                return Err(VerificationError::Timeout {
                    constraint: format!("{:?}", predicate).into(),
                    timeout,
                    cost: cost.with_timeout(),
                });
            }

            Err(VerificationError::Unknown(
                format!("{:?}", predicate).into(),
            ))
        }
    }
}

/// Verify multiple refinement constraints in batch.
pub fn verify_batch(
    context: &Context,
    constraints: &[(Type, Option<Expr>)],
    mode: VerifyMode,
) -> List<VerificationResult> {
    constraints
        .iter()
        .map(|(ty, expr)| verify_refinement(context, ty, expr.as_ref(), mode))
        .collect()
}

/// Estimate the complexity of a verification problem (0-100).
pub fn estimate_complexity(ty: &Type) -> u32 {
    match &ty.kind {
        TypeKind::Refined { base, predicate } => {
            let base_complexity = estimate_complexity(base);
            let predicate_complexity = estimate_expr_complexity(&predicate.expr);

            // Combine complexities (weighted average)
            (base_complexity + predicate_complexity * 3) / 4
        }

        TypeKind::Generic { base, args } => {
            let base_complexity = estimate_complexity(base);
            let args_complexity = args
                .iter()
                .filter_map(|arg| match arg {
                    GenericArg::Type(t) => Some(estimate_complexity(t)),
                    _ => None,
                })
                .max()
                .unwrap_or(0);

            base_complexity.max(args_complexity) + 5
        }

        _ => 10, // Base complexity for simple types
    }
}

/// Estimate the complexity of an expression.
///
/// Public wrapper for testing. This function estimates how complex an expression
/// is for SMT verification purposes (0-100 scale).
pub fn estimate_expr_complexity(expr: &Expr) -> u32 {
    estimate_expr_complexity_impl(expr)
}

/// Internal implementation of expression complexity estimation.
fn estimate_expr_complexity_impl(expr: &Expr) -> u32 {
    match &expr.kind {
        ExprKind::Literal(_) | ExprKind::Path(_) => 1,

        ExprKind::Binary { left, right, .. } => {
            1 + estimate_expr_complexity_impl(left) + estimate_expr_complexity_impl(right)
        }

        ExprKind::Unary { expr, .. } => 1 + estimate_expr_complexity_impl(expr),

        ExprKind::Call { func, args, .. } => {
            let func_complexity = estimate_expr_complexity_impl(func);
            let args_complexity: u32 = args.iter().map(estimate_expr_complexity_impl).sum();
            5 + func_complexity + args_complexity
        }

        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            // Calculate condition complexity by analyzing each condition kind
            let cond_complexity: u32 = condition
                .conditions
                .iter()
                .map(|cond_kind| match cond_kind {
                    verum_ast::expr::ConditionKind::Expr(expr) => {
                        estimate_expr_complexity_impl(expr)
                    }
                    verum_ast::expr::ConditionKind::Let { pattern: _, value } => {
                        // Let-binding conditions add pattern matching complexity
                        5 + estimate_expr_complexity_impl(value)
                    }
                })
                .sum();

            // Calculate then-branch complexity from statements
            let then_complexity: u32 = then_branch
                .stmts
                .iter()
                .map(|stmt| match &stmt.kind {
                    verum_ast::stmt::StmtKind::Expr { expr, .. } => {
                        estimate_expr_complexity_impl(expr)
                    }
                    verum_ast::stmt::StmtKind::Let { value, .. } => value
                        .as_ref()
                        .map(estimate_expr_complexity_impl)
                        .unwrap_or(1),
                    _ => 5,
                })
                .sum();

            let else_complexity = else_branch
                .as_ref()
                .map(|e| estimate_expr_complexity_impl(e))
                .unwrap_or(0);

            // Base complexity for branching + sum of components
            10 + cond_complexity + then_complexity + else_complexity
        }

        ExprKind::Attenuate { context, .. } => {
            // Attenuate expressions restrict capability, recursively process the context
            estimate_expr_complexity_impl(context)
        }

        _ => 20, // Complex expressions
    }
}

/// Decide verification mode using heuristics.
pub fn auto_mode(ty: &Type) -> VerifyMode {
    let complexity = estimate_complexity(ty);

    if complexity <= 30 {
        // Simple constraint - use SMT
        VerifyMode::Proof
    } else if complexity >= 70 {
        // Very complex - use runtime
        VerifyMode::Runtime
    } else {
        // Medium complexity - use proof with short timeout
        VerifyMode::Proof
    }
}

/// Incremental verification context for efficient multi-query solving
pub struct IncrementalVerifier<'ctx> {
    context: &'ctx Context,
    assertion_stack: List<Text>,
}

impl<'ctx> IncrementalVerifier<'ctx> {
    /// Create a new incremental verifier
    pub fn new(context: &'ctx Context) -> Self {
        Self {
            context,
            assertion_stack: List::new(),
        }
    }

    /// Push a verification scope
    pub fn push(&mut self) {
        let solver = self.context.solver();
        solver.push();
        self.assertion_stack.push("scope".to_text());
    }

    /// Pop a verification scope
    pub fn pop(&mut self) {
        let solver = self.context.solver();
        solver.pop(1);
        if !self.assertion_stack.is_empty() {
            self.assertion_stack.pop();
        }
    }

    /// Verify a refinement type incrementally
    pub fn verify_incremental(
        &mut self,
        ty: &Type,
        value_expr: Option<&Expr>,
        mode: VerifyMode,
    ) -> VerificationResult {
        verify_refinement(self.context, ty, value_expr, mode)
    }

    /// Get current scope depth
    pub fn scope_depth(&self) -> usize {
        self.assertion_stack.len()
    }
}

/// Parallel verification for independent constraints
///
/// Verifies multiple constraints in parallel (conceptually - actual parallelism
/// would require thread-safe Z3 contexts). Returns results in the same order
/// as input constraints.
pub fn verify_parallel(
    context: &Context,
    constraints: &[(Type, Option<Expr>)],
    mode: VerifyMode,
) -> List<VerificationResult> {
    // Note: Z3 Context is not Send/Sync, so true parallelism requires
    // careful design. For now, this is a sequential implementation with
    // the parallel interface for future enhancement.

    constraints
        .iter()
        .map(|(ty, expr)| verify_refinement(context, ty, expr.as_ref(), mode))
        .collect()
}

/// Batch verification with shared context
///
/// Optimized for verifying multiple related constraints that share
/// common subexpressions or variables. Uses incremental solving.
pub fn verify_batch_incremental(
    context: &Context,
    constraints: &[(Type, Option<Expr>)],
    mode: VerifyMode,
) -> List<VerificationResult> {
    let mut verifier = IncrementalVerifier::new(context);
    let mut results = List::new();

    for (ty, expr) in constraints {
        verifier.push();
        let result = verifier.verify_incremental(ty, expr.as_ref(), mode);
        results.push(result);
        verifier.pop();
    }

    results
}
