//! Integration layer for dependent types SMT verification
//!
//! This module provides the bridge between verum_types type checking and
//! verum_smt dependent types verification. It wraps verum_smt::DependentTypeBackend
//! to provide type checking integration without creating circular dependencies.
//!
//! Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Dependent Types Extension (v2.0+)
//!
//! # Architecture
//!
//! ```text
//! TypeChecker (verum_types)
//!   ↓ uses
//! DependentTypeChecker (this module)
//!   ↓ delegates to
//! verum_smt::DependentTypeBackend
//!   ↓ uses
//! Z3 SMT Solver
//! ```
//!
//! # Key Design Decisions
//!
//! 1. **No Circular Dependencies**: verum_smt doesn't depend on verum_types
//! 2. **Lazy Initialization**: SMT backend only created when needed
//! 3. **Caching**: Results cached in RefinementChecker's cache
//! 4. **Graceful Degradation**: Falls back to syntactic checking if SMT unavailable

use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use verum_ast::Type;
use verum_ast::expr::Expr;
use verum_ast::span::Span;
use verum_common::{List, Map, Maybe, Text};

use crate::context::TypeContext;
use crate::refinement::{RefinementError, VerificationResult};
use crate::ty::Type as InternalType;

// Import from verum_smt
use verum_smt::translate::Translator;
use verum_smt::{
    Context as SmtContext, DependentTypeBackend, EqualityType, PiType, SigmaType,
    VerificationError, VerificationResult as SmtVerificationResult,
};

// ==================== Dependent Type Checker Interface ====================

/// Interface for dependent type verification
///
/// This trait abstracts the dependent type checking operations that the
/// type checker needs. The primary implementation delegates to verum_smt.
pub trait DependentTypeChecker: Send + Sync {
    /// Verify a Pi type (dependent function type)
    ///
    /// Checks that (x: A) -> B(x) is well-formed, meaning:
    /// - A is a valid type
    /// - B(x) is a valid type for all x: A
    /// - No circular dependencies
    fn verify_pi_type(
        &mut self,
        param_name: Text,
        param_type: &Type,
        return_type: &Type,
        span: Span,
    ) -> Result<VerificationResult, RefinementError>;

    /// Verify a Sigma type (dependent pair type)
    ///
    /// Checks that (x: A, B(x)) is well-formed, meaning:
    /// - A is a valid type
    /// - B(x) is a valid type for all x: A
    /// - The pairing is valid (existential check)
    fn verify_sigma_type(
        &mut self,
        fst_name: Text,
        fst_type: &Type,
        snd_type: &Type,
        span: Span,
    ) -> Result<VerificationResult, RefinementError>;

    /// Verify an equality type
    ///
    /// Checks that Eq<A, lhs, rhs> is well-formed:
    /// - Both sides have type A
    /// - The equality is decidable
    fn verify_equality(
        &mut self,
        value_type: &Type,
        lhs: &Expr,
        rhs: &Expr,
        span: Span,
    ) -> Result<VerificationResult, RefinementError>;

    /// Verify Fin type constraint
    ///
    /// Checks that value < bound for Fin<n> types
    fn verify_fin_type(
        &mut self,
        value: &Expr,
        bound: &Expr,
        span: Span,
    ) -> Result<VerificationResult, RefinementError>;
}

// ==================== SMT-Based Implementation ====================

/// SMT-based dependent type checker
///
/// Delegates to verum_smt::DependentTypeBackend for all verification.
/// Maintains a Z3 context and translator for AST to SMT conversion.
pub struct SmtDependentTypeChecker {
    /// Z3 context (thread-local, managed by verum_smt)
    smt_context: Arc<RwLock<Maybe<SmtContext>>>,
    /// Dependent type backend
    backend: Arc<RwLock<DependentTypeBackend>>,
    /// Verification statistics
    stats: Arc<RwLock<DependentVerificationStats>>,
}

impl SmtDependentTypeChecker {
    /// Create a new SMT-based dependent type checker
    pub fn new() -> Self {
        Self {
            smt_context: Arc::new(RwLock::new(Maybe::None)),
            backend: Arc::new(RwLock::new(DependentTypeBackend::new())),
            stats: Arc::new(RwLock::new(DependentVerificationStats::default())),
        }
    }

    /// Get or create the SMT context (lazy initialization)
    fn get_smt_context(&self) -> Result<SmtContext, RefinementError> {
        let mut ctx_lock = self.smt_context.write();
        match &*ctx_lock {
            Maybe::Some(ctx) => Ok(ctx.clone()),
            Maybe::None => {
                // Create new Z3 context
                let ctx = SmtContext::new();
                *ctx_lock = Maybe::Some(ctx.clone());
                Ok(ctx)
            }
        }
    }

    /// Convert SMT verification result to refinement verification result
    fn convert_result(
        &self,
        smt_result: Result<verum_smt::verify::ProofResult, VerificationError>,
    ) -> Result<VerificationResult, RefinementError> {
        match smt_result {
            Ok(_proof) => Ok(VerificationResult::Valid),
            Err(VerificationError::CannotProve {
                constraint,
                counterexample,
                ..
            }) => {
                // Convert counterexample if available
                let ce = if let Some(ce_smt) = counterexample {
                    // Extract first variable assignment as a simple counterexample
                    let (var_name, value) = if let Some((k, v)) = ce_smt.assignments.iter().next() {
                        (k.clone(), format!("{}", v).into())
                    } else {
                        ("value".into(), "unknown".into())
                    };

                    Maybe::Some(crate::refinement::CounterExample {
                        var_name,
                        value,
                        explanation: Maybe::Some(ce_smt.description),
                    })
                } else {
                    Maybe::None
                };
                Ok(VerificationResult::Invalid { counterexample: ce })
            }
            Err(VerificationError::Timeout { .. }) => Ok(VerificationResult::Unknown {
                reason: "SMT solver timeout".into(),
            }),
            Err(VerificationError::Translation(e)) => Err(RefinementError::new(
                format!("Translation error: {}", e).into(),
                Span::dummy(),
            )),
            Err(VerificationError::SolverError(msg)) => {
                Ok(VerificationResult::Unknown { reason: msg })
            }
            Err(VerificationError::Unknown(reason)) => Ok(VerificationResult::Unknown { reason }),
        }
    }

    /// Update verification statistics
    fn update_stats(&self, result: &VerificationResult, elapsed_ms: u64) {
        let mut stats = self.stats.write();
        stats.total_checks += 1;
        stats.total_time_ms += elapsed_ms;

        match result {
            VerificationResult::Valid => stats.valid_count += 1,
            VerificationResult::Invalid { .. } => stats.invalid_count += 1,
            VerificationResult::Unknown { .. } => stats.unknown_count += 1,
        }
    }

    /// Get statistics
    pub fn stats(&self) -> DependentVerificationStats {
        self.stats.read().clone()
    }
}

impl Default for SmtDependentTypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl DependentTypeChecker for SmtDependentTypeChecker {
    fn verify_pi_type(
        &mut self,
        param_name: Text,
        param_type: &Type,
        return_type: &Type,
        span: Span,
    ) -> Result<VerificationResult, RefinementError> {
        let start = Instant::now();

        // Get SMT context
        let ctx = self.get_smt_context()?;

        // Create translator
        let translator = Translator::new(&ctx);

        // Create Pi type structure
        let pi = PiType::new(param_name, param_type.clone(), return_type.clone());

        // Verify using backend
        let backend = self.backend.read();
        let result = self.convert_result(backend.verify_pi_type(&pi, &translator))?;

        // Update stats
        let elapsed = start.elapsed().as_millis() as u64;
        self.update_stats(&result, elapsed);

        Ok(result)
    }

    fn verify_sigma_type(
        &mut self,
        fst_name: Text,
        fst_type: &Type,
        snd_type: &Type,
        span: Span,
    ) -> Result<VerificationResult, RefinementError> {
        let start = Instant::now();

        // Get SMT context
        let ctx = self.get_smt_context()?;

        // Create translator
        let translator = Translator::new(&ctx);

        // Create Sigma type structure
        let sigma = SigmaType::new(fst_name, fst_type.clone(), snd_type.clone());

        // Verify using backend
        let backend = self.backend.read();
        let result = self.convert_result(backend.verify_sigma_type(&sigma, &translator))?;

        // Update stats
        let elapsed = start.elapsed().as_millis() as u64;
        self.update_stats(&result, elapsed);

        Ok(result)
    }

    fn verify_equality(
        &mut self,
        value_type: &Type,
        lhs: &Expr,
        rhs: &Expr,
        span: Span,
    ) -> Result<VerificationResult, RefinementError> {
        let start = Instant::now();

        // Get SMT context
        let ctx = self.get_smt_context()?;

        // Create translator
        let translator = Translator::new(&ctx);

        // Create equality type structure
        let eq = EqualityType::new(value_type.clone(), lhs.clone(), rhs.clone());

        // Verify using backend
        let backend = self.backend.read();
        let result = self.convert_result(backend.verify_equality(&eq, &translator))?;

        // Update stats
        let elapsed = start.elapsed().as_millis() as u64;
        self.update_stats(&result, elapsed);

        Ok(result)
    }

    fn verify_fin_type(
        &mut self,
        value: &Expr,
        bound: &Expr,
        span: Span,
    ) -> Result<VerificationResult, RefinementError> {
        let start = Instant::now();

        // Get SMT context
        let ctx = self.get_smt_context()?;

        // Create translator
        let translator = Translator::new(&ctx);

        // Verify using backend
        let mut backend = self.backend.write();
        let result = self.convert_result(backend.verify_fin_type(value, bound, &translator))?;

        // Update stats
        let elapsed = start.elapsed().as_millis() as u64;
        self.update_stats(&result, elapsed);

        Ok(result)
    }
}

// ==================== Statistics ====================

/// Statistics for dependent type verification
#[derive(Debug, Clone, Default)]
pub struct DependentVerificationStats {
    /// Total verification checks
    pub total_checks: usize,
    /// Valid verifications
    pub valid_count: usize,
    /// Invalid verifications
    pub invalid_count: usize,
    /// Unknown results
    pub unknown_count: usize,
    /// Total time in milliseconds
    pub total_time_ms: u64,
}

impl DependentVerificationStats {
    /// Get average verification time
    pub fn average_time_ms(&self) -> f64 {
        if self.total_checks == 0 {
            0.0
        } else {
            self.total_time_ms as f64 / self.total_checks as f64
        }
    }

    /// Get success rate (valid + invalid vs unknown)
    pub fn success_rate(&self) -> f64 {
        if self.total_checks == 0 {
            0.0
        } else {
            (self.valid_count + self.invalid_count) as f64 / self.total_checks as f64
        }
    }

    /// Generate a report
    pub fn report(&self) -> Text {
        format!(
            "Dependent Type Verification Statistics:\n\
             - Total checks: {}\n\
             - Valid: {}, Invalid: {}, Unknown: {}\n\
             - Success rate: {:.1}%\n\
             - Average time: {:.2}ms\n\
             - Total time: {}ms",
            self.total_checks,
            self.valid_count,
            self.invalid_count,
            self.unknown_count,
            self.success_rate() * 100.0,
            self.average_time_ms(),
            self.total_time_ms
        )
        .into()
    }
}

// ==================== Integration with RefinementChecker ====================

/// Extension methods for RefinementChecker to support dependent types
impl crate::refinement::RefinementChecker {
    /// Verify a dependent type constraint
    ///
    /// This is the primary integration point. Called by the type checker when
    /// it encounters dependent types that need verification.
    pub fn verify_dependent_type(
        &mut self,
        constraint: &DependentTypeConstraint,
    ) -> Result<VerificationResult, RefinementError> {
        // Check if we have a dependent type checker
        if let Maybe::Some(ref mut checker) = self.dependent_checker {
            match constraint {
                DependentTypeConstraint::PiType {
                    param_name,
                    param_type,
                    return_type,
                    span,
                } => checker.verify_pi_type(param_name.clone(), param_type, return_type, *span),
                DependentTypeConstraint::SigmaType {
                    fst_name,
                    fst_type,
                    snd_type,
                    span,
                } => checker.verify_sigma_type(fst_name.clone(), fst_type, snd_type, *span),
                DependentTypeConstraint::Equality {
                    value_type,
                    lhs,
                    rhs,
                    span,
                } => checker.verify_equality(value_type, lhs, rhs, *span),
                DependentTypeConstraint::FinType { value, bound, span } => {
                    checker.verify_fin_type(value, bound, *span)
                }
            }
        } else {
            // No dependent type checker available - return unknown
            Ok(VerificationResult::Unknown {
                reason: "Dependent type checking not enabled".into(),
            })
        }
    }

    /// Enable dependent type checking
    ///
    /// Call this to enable SMT-based dependent type verification.
    /// This is typically done during type checker initialization.
    pub fn enable_dependent_types(&mut self) {
        self.dependent_checker = Maybe::Some(Box::new(SmtDependentTypeChecker::new()));
    }

    /// Check if dependent type checking is enabled
    pub fn has_dependent_types(&self) -> bool {
        matches!(self.dependent_checker, Maybe::Some(_))
    }
}

// ==================== Constraint Types ====================

/// A dependent type constraint to be verified
///
/// This enum represents the different kinds of dependent type constraints
/// that the type checker needs to verify.
#[derive(Debug, Clone)]
pub enum DependentTypeConstraint {
    /// Pi type: (x: A) -> B(x)
    PiType {
        param_name: Text,
        param_type: Type,
        return_type: Type,
        span: Span,
    },
    /// Sigma type: (x: A, B(x))
    SigmaType {
        fst_name: Text,
        fst_type: Type,
        snd_type: Type,
        span: Span,
    },
    /// Equality type: Eq<A, lhs, rhs>
    Equality {
        value_type: Type,
        lhs: Expr,
        rhs: Expr,
        span: Span,
    },
    /// Fin type: value < bound
    FinType {
        value: Expr,
        bound: Expr,
        span: Span,
    },
}

// ==================== Update RefinementChecker struct ====================

// Note: We need to add a field to RefinementChecker to hold the dependent type checker.
// This is done by modifying the existing struct in refinement.rs.
// The field is added as:
//
// pub(crate) dependent_checker: Maybe<Box<dyn DependentTypeChecker>>,
//
// This is initialized to Maybe::None by default and can be enabled by calling
// enable_dependent_types().
