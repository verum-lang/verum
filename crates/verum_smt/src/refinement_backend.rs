//! Z3 SMT Backend Implementation for Refinement Type Checking
//!
//! Refinement types with gradual verification: types can carry predicates (Int{> 0}) verified at compile-time or runtime depending on verification level — .1 - Refinement Types with SMT
//!
//! This module provides a bridge implementation that delegates to verum_smt's
//! SubsumptionChecker for all SMT operations. It implements the
//! `verum_types::refinement::SmtBackend` trait.
//!
//! Historical note: this module used to live in `verum_types::smt_backend`.
//! It was moved into `verum_smt` to break the circular dependency between
//! `verum_smt` and `verum_types`. The public type was renamed from
//! `Z3Backend` to `RefinementZ3Backend` to avoid colliding with the
//! pre-existing `verum_smt::solver::Z3Backend` type.
//!
//! ## Architecture
//!
//! ```text
//! RefinementChecker (verum_types)
//!   ↓ (uses SmtBackend trait from verum_types::refinement)
//! RefinementZ3Backend (verum_smt - this file, delegation)
//!   ↓ (delegates to)
//! SubsumptionChecker (verum_smt - full Z3 implementation)
//!   ↓ (uses)
//! Z3 Solver (via z3-rs with proper Context management)
//! ```

use std::sync::Arc;
use std::time::Instant;

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::{Literal, LiteralKind};
use verum_ast::span::Span;
use verum_common::{Map, Text};

use verum_types::refinement::{
    RefinementError, SmtBackend, SmtResult, VerificationResult,
};

// Import SubsumptionChecker + shared stats struct from this crate
use crate::{CheckMode, SolverStats, SubsumptionChecker, SubsumptionResult};

/// Z3-based SMT backend for refinement verification
///
/// This implementation delegates all SMT operations to verum_smt::SubsumptionChecker,
/// which provides:
/// - Proper Z3 Context management (thread-local)
/// - Query caching with configurable TTL
/// - Syntactic pre-checking for common cases
/// - Timeout handling
/// - Performance statistics
pub struct RefinementZ3Backend {
    /// The underlying SubsumptionChecker from verum_smt
    checker: SubsumptionChecker,
    /// Statistics tracking — reuses crate-wide `SolverStats`.
    stats: Arc<parking_lot::RwLock<SolverStats>>,
}

impl RefinementZ3Backend {
    /// Create a new Z3 backend with default configuration
    pub fn new() -> Self {
        Self {
            checker: SubsumptionChecker::new(),
            stats: Arc::new(parking_lot::RwLock::new(SolverStats::new())),
        }
    }

    /// Create with custom timeout in milliseconds
    pub fn with_timeout(timeout_ms: u64) -> Self {
        Self {
            checker: SubsumptionChecker::with_config(crate::SubsumptionConfig {
                smt_timeout_ms: timeout_ms,
                ..Default::default()
            }),
            stats: Arc::new(parking_lot::RwLock::new(SolverStats::new())),
        }
    }

    /// Create with custom SubsumptionChecker
    pub fn with_checker(checker: SubsumptionChecker) -> Self {
        Self {
            checker,
            stats: Arc::new(parking_lot::RwLock::new(SolverStats::new())),
        }
    }

    /// Get backend statistics
    pub fn stats(&self) -> SolverStats {
        self.stats.read().clone()
    }

    /// Get SubsumptionChecker statistics
    pub fn checker_stats(&self) -> crate::SubsumptionStats {
        self.checker.stats()
    }

    /// Clear the subsumption cache
    pub fn clear_cache(&self) {
        self.checker.clear_cache();
    }
}

impl Default for RefinementZ3Backend {
    fn default() -> Self {
        Self::new()
    }
}

impl SmtBackend for RefinementZ3Backend {
    /// Check satisfiability of an expression
    fn check(&mut self, expr: &Expr) -> Result<SmtResult, RefinementError> {
        let start = Instant::now();

        // Check if expr => false is valid: VALID => expr is UNSAT; INVALID => expr is SAT.
        let false_expr = make_bool_literal(false, expr.span);
        let result = self.checker.check(expr, &false_expr, CheckMode::SmtAllowed);

        let elapsed_ms = start.elapsed().as_millis() as u64;

        let smt_result = match result {
            SubsumptionResult::Syntactic(true) => SmtResult::Unsat,
            SubsumptionResult::Syntactic(false) => SmtResult::Sat,
            SubsumptionResult::Smt { valid: true, .. } => SmtResult::Unsat,
            SubsumptionResult::Smt { valid: false, .. } => SmtResult::Sat,
            SubsumptionResult::Unknown { .. } => SmtResult::Unknown,
        };

        let mut stats = self.stats.write();
        match smt_result {
            SmtResult::Sat => stats.record_sat(elapsed_ms),
            SmtResult::Unsat => stats.record_unsat(elapsed_ms),
            SmtResult::Unknown => stats.record_unknown(elapsed_ms),
        }

        Ok(smt_result)
    }

    /// Get model (satisfying assignment) for SAT result
    fn get_model(&mut self) -> Result<Map<Text, Text>, RefinementError> {
        Ok(Map::new())
    }

    /// Verify that a refinement predicate holds for a given value
    fn verify_refinement(
        &mut self,
        predicate: &Expr,
        value: &Expr,
        assumptions: &[Expr],
    ) -> Result<VerificationResult, RefinementError> {
        let start = Instant::now();

        // Build VC: check whether (assumptions ∧ value) ⇒ predicate.
        let mut combined = value.clone();
        for assumption in assumptions {
            combined = make_binary_and(combined, assumption.clone());
        }
        let result = self
            .checker
            .check(&combined, predicate, CheckMode::SmtAllowed);

        let elapsed_ms = start.elapsed().as_millis() as u64;

        let verification_result = match result {
            SubsumptionResult::Syntactic(true) | SubsumptionResult::Smt { valid: true, .. } => {
                VerificationResult::Valid
            }
            SubsumptionResult::Syntactic(false) | SubsumptionResult::Smt { valid: false, .. } => {
                VerificationResult::Invalid {
                    counterexample: Option::None,
                }
            }
            SubsumptionResult::Unknown { reason } => VerificationResult::Unknown {
                reason: reason.into(),
            },
        };

        let mut stats = self.stats.write();
        match &verification_result {
            VerificationResult::Valid => stats.record_unsat(elapsed_ms),
            VerificationResult::Invalid { .. } => stats.record_sat(elapsed_ms),
            VerificationResult::Unknown { .. } => stats.record_unknown(elapsed_ms),
        }

        Ok(verification_result)
    }

    /// Forward the per-query timeout to the underlying
    /// `SubsumptionChecker`, which forwards it to Z3 via the
    /// `timeout` solver parameter on every `check_smt`. This is
    /// the production wiring for `RefinementConfig.timeout_ms`:
    /// without it the checker's timeout was frozen at
    /// construction and any caller-side knob was ignored.
    fn set_timeout_ms(&mut self, ms: u64) {
        self.checker.set_smt_timeout_ms(ms);
    }
}

/// Helper to check refinement subsumption using SMT
///
/// This is the key function for subtyping: T{φ1} <: T{φ2} iff φ1 => φ2
///
/// Delegates to verum_smt::SubsumptionChecker which handles all Z3 complexity.
///
/// # Algorithm
///
/// To check φ1 ⇒ φ2, SubsumptionChecker:
/// 1. First tries syntactic subsumption (cheap, no SMT)
/// 2. If inconclusive, uses Z3 to check ¬(φ1 ⇒ φ2) is UNSAT
/// 3. Caches results for performance
pub fn check_subsumption_smt(
    phi1: &Expr,
    phi2: &Expr,
    _timeout_ms: u64,
) -> Result<bool, RefinementError> {
    // Use SubsumptionChecker for the actual work
    let checker = SubsumptionChecker::new();
    let result = checker.check(phi1, phi2, CheckMode::SmtAllowed);

    match result {
        SubsumptionResult::Syntactic(valid) => Ok(valid),
        SubsumptionResult::Smt { valid, .. } => Ok(valid),
        SubsumptionResult::Unknown { .. } => Ok(false), // Conservative: return false when unknown
    }
}

// ==================== Helper Functions ====================

/// Create a boolean literal expression
fn make_bool_literal(value: bool, span: Span) -> Expr {
    Expr {
        kind: ExprKind::Literal(Literal {
            kind: LiteralKind::Bool(value),
            span,
        }),
        span,
        ref_kind: None,
        check_eliminated: false,
    }
}

/// Create a binary AND expression
fn make_binary_and(left: Expr, right: Expr) -> Expr {
    // Use the left span's file_id for the combined span
    let span = Span::new(left.span.start, right.span.end, left.span.file_id);
    Expr {
        kind: ExprKind::Binary {
            op: BinOp::And,
            left: Box::new(left),
            right: Box::new(right),
        },
        span,
        ref_kind: None,
        check_eliminated: false,
    }
}

// Stats exercised via the shared `context::SolverStats` tests — no
// duplicate coverage here.
