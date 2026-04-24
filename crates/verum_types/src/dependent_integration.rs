//! Integration layer for dependent types SMT verification
//!
//! This module defines the *interface* between verum_types type checking and
//! an SMT-based dependent type verifier. The concrete SMT-backed
//! implementation (`SmtDependentTypeChecker`) lives in the `verum_smt`
//! crate (`verum_smt::dependent_backend`) to avoid a circular dependency.
//!
//! Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Dependent Types Extension (v2.0+)
//!
//! # Architecture
//!
//! ```text
//! TypeChecker (verum_types)
//!   ↓ uses trait
//! DependentTypeChecker trait (this module)
//!   ↓ implemented by
//! SmtDependentTypeChecker (verum_smt::dependent_backend)
//!   ↓ delegates to
//! verum_smt::DependentTypeBackend
//!   ↓ uses
//! Z3 SMT Solver
//! ```

use verum_ast::Type;
use verum_ast::expr::Expr;
use verum_ast::span::Span;
use verum_common::{Maybe, Text};

use crate::refinement::{RefinementError, VerificationResult};

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

// ==================== Integration with RefinementChecker ====================

/// Extension methods for RefinementChecker to support dependent types
impl crate::refinement::RefinementChecker {
    /// Verify a dependent type constraint
    ///
    /// This is the primary integration point. Called by the type checker when
    /// it encounters dependent types that need verification. Returns
    /// `VerificationResult::Unknown` when no dependent type checker has been
    /// injected.
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

    /// Inject an SMT-backed dependent type checker.
    ///
    /// Call this with e.g. `verum_smt::dependent_backend::SmtDependentTypeChecker::new()`
    /// (boxed) to enable SMT verification. When unset, `verify_dependent_type`
    /// returns `Unknown`.
    pub fn with_dependent_checker(mut self, checker: Box<dyn DependentTypeChecker>) -> Self {
        self.dependent_checker = Maybe::Some(checker);
        self
    }

    /// Install a previously-constructed dependent type checker.
    pub fn set_dependent_checker(&mut self, checker: Box<dyn DependentTypeChecker>) {
        self.dependent_checker = Maybe::Some(checker);
    }

    /// Enable dependent type checking (legacy no-arg API).
    ///
    /// This used to construct an `SmtDependentTypeChecker` automatically.
    /// To preserve the circular-dependency-free `verum_types` crate, the
    /// SMT-backed implementation now lives in `verum_smt`. Downstream
    /// callers that want the full SMT integration should call
    /// `set_dependent_checker` with a `verum_smt::dependent_backend::SmtDependentTypeChecker`
    /// instance. Calling this method without injecting a checker is a
    /// no-op and leaves dependent type checking disabled (returns
    /// `Unknown`).
    pub fn enable_dependent_types(&mut self) {
        // Legacy signature preserved. Without an injected checker this is a
        // no-op — callers in verum_compiler set a checker explicitly via
        // `set_dependent_checker` right after.
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

/// Statistics for dependent type verification (canonical shape reused by
/// every `DependentTypeChecker` implementation, including
/// `verum_smt::dependent_backend::SmtDependentTypeChecker`).
#[derive(Debug, Clone, Default)]
pub struct DependentVerificationStats {
    pub total_checks: usize,
    pub valid_count: usize,
    pub invalid_count: usize,
    pub unknown_count: usize,
    pub total_time_ms: u64,
}

impl DependentVerificationStats {
    pub fn average_time_ms(&self) -> f64 {
        if self.total_checks == 0 {
            0.0
        } else {
            self.total_time_ms as f64 / self.total_checks as f64
        }
    }

    pub fn success_rate(&self) -> f64 {
        if self.total_checks == 0 {
            0.0
        } else {
            (self.valid_count + self.invalid_count) as f64 / self.total_checks as f64
        }
    }

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
