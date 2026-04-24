//! SMT-based implementation of the `DependentTypeChecker` trait.
//!
//! This module lives in `verum_smt` (rather than `verum_types`) so the
//! `verum_smt` → `verum_types` dependency edge remains acyclic. The trait
//! definition and constraint types stay in
//! `verum_types::dependent_integration`; only the concrete Smt-based
//! implementation was migrated here.
//!
//! ```text
//! TypeChecker (verum_types)
//!   ↓ uses
//! DependentTypeChecker trait (verum_types::dependent_integration)
//!   ↓ implemented by
//! SmtDependentTypeChecker (verum_smt - this file)
//!   ↓ delegates to
//! verum_smt::DependentTypeBackend + Translator
//!   ↓ uses
//! Z3 SMT Solver
//! ```

use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use verum_ast::Type;
use verum_ast::expr::Expr;
use verum_ast::span::Span;
use verum_common::{Maybe, Text};

use verum_types::dependent_integration::DependentTypeChecker;
use verum_types::refinement::{RefinementError, VerificationResult};

// Import from this crate (verum_smt).
use crate::translate::Translator;
use crate::{
    Context as SmtContext, DependentTypeBackend, EqualityType, PiType, SigmaType, VerificationError,
};

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
                let ctx = SmtContext::new();
                *ctx_lock = Maybe::Some(ctx.clone());
                Ok(ctx)
            }
        }
    }

    /// Convert SMT verification result to refinement verification result
    fn convert_result(
        &self,
        smt_result: Result<crate::verify::ProofResult, VerificationError>,
    ) -> Result<VerificationResult, RefinementError> {
        match smt_result {
            Ok(_proof) => Ok(VerificationResult::Valid),
            Err(VerificationError::CannotProve {
                constraint: _,
                counterexample,
                ..
            }) => {
                // Convert counterexample if available
                let ce = if let Some(ce_smt) = counterexample {
                    // Extract first variable assignment as a simple counterexample
                    let (var_name, value) =
                        if let Some((k, v)) = ce_smt.assignments.iter().next() {
                            (k.clone(), format!("{}", v).into())
                        } else {
                            ("value".into(), "unknown".into())
                        };

                    Maybe::Some(verum_types::refinement::CounterExample {
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
        _span: Span,
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
        _span: Span,
    ) -> Result<VerificationResult, RefinementError> {
        let start = Instant::now();

        let ctx = self.get_smt_context()?;
        let translator = Translator::new(&ctx);

        let sigma = SigmaType::new(fst_name, fst_type.clone(), snd_type.clone());

        let backend = self.backend.read();
        let result = self.convert_result(backend.verify_sigma_type(&sigma, &translator))?;

        let elapsed = start.elapsed().as_millis() as u64;
        self.update_stats(&result, elapsed);

        Ok(result)
    }

    fn verify_equality(
        &mut self,
        value_type: &Type,
        lhs: &Expr,
        rhs: &Expr,
        _span: Span,
    ) -> Result<VerificationResult, RefinementError> {
        let start = Instant::now();

        let ctx = self.get_smt_context()?;
        let translator = Translator::new(&ctx);

        let eq = EqualityType::new(value_type.clone(), lhs.clone(), rhs.clone());

        let backend = self.backend.read();
        let result = self.convert_result(backend.verify_equality(&eq, &translator))?;

        let elapsed = start.elapsed().as_millis() as u64;
        self.update_stats(&result, elapsed);

        Ok(result)
    }

    fn verify_fin_type(
        &mut self,
        value: &Expr,
        bound: &Expr,
        _span: Span,
    ) -> Result<VerificationResult, RefinementError> {
        let start = Instant::now();

        let ctx = self.get_smt_context()?;
        let translator = Translator::new(&ctx);

        let mut backend = self.backend.write();
        let result = self.convert_result(backend.verify_fin_type(value, bound, &translator))?;

        let elapsed = start.elapsed().as_millis() as u64;
        self.update_stats(&result, elapsed);

        Ok(result)
    }
}

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
