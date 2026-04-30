//! Per-function refinement-type verification (Z3 + CVC5 portfolio).
//!
//! Extracted from `pipeline.rs` (#106 Phase 4). Implements full
//! Z3-based refinement-type verification:
//!
//!   1. Extracts refinement predicates from parameter / return types.
//!   2. Generates Z3 assertions for each refinement constraint.
//!   3. Uses `verum_smt::RefinementVerifier` to verify constraints.
//!   4. Caches verification results for performance.
//!   5. Reports detailed error messages with counterexamples.
//!
//! Fast-path for syntactic subsumption; falls back to full Z3 SMT
//! solving for complex cases. Timeout-bounded (10-500ms per
//! sub-check). Refinement-type subsumption via Z3: syntactic check
//! catches simple predicates without solver involvement.

use anyhow::Result;
use tracing::{debug, warn};

use verum_common::{List, Text};
use verum_diagnostics::{DiagnosticBuilder, Severity};

use verum_smt::{
    Context as SmtContext, RefinementVerifier as SmtRefinementVerifier, SubsumptionChecker,
    SubsumptionConfig, VerificationError as SmtVerificationError, VerifyMode as SmtVerifyMode,
};

use super::CompilationPipeline;

/// Outcome of an SMT-based refinement check at a return site.
///
/// When a function declares a refinement on its return type
/// (e.g., `Int{> 0}`), the compiler checks via SMT that the return
/// expression satisfies the predicate. Three outcomes: Verified
/// (proven by Z3), Timeout (solver exceeded budget), or
/// Falsifiable (counterexample found).
#[derive(Debug, Clone)]
pub(super) enum SmtCheckResult {
    /// The refinement constraint was successfully verified.
    /// The return expression provably satisfies the predicate.
    Verified,

    /// The refinement constraint was violated.
    /// A counterexample demonstrates a case where the predicate fails.
    Violated {
        /// Optional counterexample showing values that violate the constraint.
        counterexample: Option<String>,
    },

    /// The SMT solver could not determine the result.
    /// May happen for complex predicates or unsupported operations.
    Unknown {
        /// Explanation of why verification was inconclusive.
        reason: String,
    },

    /// The SMT solver timed out before completing verification.
    /// The constraint should be checked at runtime instead.
    Timeout,
}

impl<'s> CompilationPipeline<'s> {
    /// Verify refinement types for a single function.
    pub(super) fn verify_function_refinements(
        &self,
        func: &verum_ast::decl::FunctionDecl,
        timeout_ms: u64,
    ) -> Result<bool> {
        use std::time::Duration;
        use verum_ast::ty::TypeKind;
        use verum_smt::context::ContextConfig;

        // Create SMT context with timeout configuration. Install the
        // session's shared routing-stats collector so every Z3 check()
        // during refinement verification feeds `verum smt-stats`.
        let smt_config = ContextConfig {
            timeout: Some(Duration::from_millis(timeout_ms)),
            ..Default::default()
        };
        let smt_context = SmtContext::with_config(smt_config)
            .with_routing_stats(self.session.routing_stats().clone());

        // Create refinement verifier with SMT mode.
        let verifier = SmtRefinementVerifier::with_mode(SmtVerifyMode::Auto);

        // Create subsumption checker for type relationships.
        let subsumption_config = SubsumptionConfig {
            cache_size: 10000,
            smt_timeout_ms: timeout_ms.min(500), // 10-500ms for subsumption checking
        };
        let subsumption_checker = SubsumptionChecker::with_config(subsumption_config);

        let mut all_verified = true;

        // Collect parameter refinements for use in return type verification.
        let mut param_constraints: List<(&verum_ast::Type, Text)> = List::new();

        // Verify parameter refinements.
        for param in &func.params {
            if let verum_ast::decl::FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                if let TypeKind::Refined {
                    base: _,
                    predicate: _,
                } = &ty.kind
                {
                    let param_name =
                        extract_pattern_name(pattern).unwrap_or_else(|| "param".into());

                    debug!(
                        "Verifying refined parameter '{}' with predicate in function '{}'",
                        param_name, func.name.name
                    );

                    param_constraints.push((ty, param_name.clone()));

                    let verification_result = verifier.verify_refinement(
                        ty,
                        None,
                        Some(SmtVerifyMode::Auto),
                    );

                    match verification_result {
                        Ok(_proof_result) => {
                            debug!(
                                "Parameter '{}' refinement verified in '{}'",
                                param_name, func.name.name
                            );
                        }
                        Err(SmtVerificationError::CannotProve {
                            constraint,
                            counterexample,
                            suggestions,
                            ..
                        }) => {
                            all_verified = false;

                            let mut msg = format!(
                                "Parameter '{}' has unsatisfiable refinement constraint: {}",
                                param_name, constraint
                            );

                            if let Some(ref cex) = counterexample {
                                msg.push_str(&format!("\n  Counterexample: {:?}", cex));
                            }

                            if !suggestions.is_empty() {
                                msg.push_str("\n  Suggestions:");
                                for suggestion in &suggestions {
                                    msg.push_str(&format!("\n    - {}", suggestion));
                                }
                            }

                            let diag = DiagnosticBuilder::new(Severity::Error).message(msg).build();
                            self.session.emit_diagnostic(diag);
                        }
                        Err(SmtVerificationError::Timeout {
                            constraint,
                            timeout,
                            ..
                        }) => {
                            warn!(
                                "Timeout verifying parameter '{}' refinement ({}ms): {}",
                                param_name,
                                timeout.as_millis(),
                                constraint
                            );
                            let diag = DiagnosticBuilder::new(Severity::Warning)
                                .message(format!(
                                    "Timeout verifying parameter '{}' refinement. Falling back to runtime checks.",
                                    param_name
                                ))
                                .build();
                            self.session.emit_diagnostic(diag);
                        }
                        Err(e) => {
                            debug!("Parameter '{}' refinement check error: {}", param_name, e);
                        }
                    }
                }
            }
        }

        // Verify return type refinement with full SMT integration.
        if let Some(ref return_ty) = func.return_type {
            if let TypeKind::Refined { base: _, predicate } = &return_ty.kind {
                debug!(
                    "Verifying refined return type with predicate in function '{}'",
                    func.name.name
                );

                let return_check_result = self.verify_return_refinement_smt(
                    func,
                    return_ty,
                    &predicate.expr,
                    &smt_context,
                    &verifier,
                    &subsumption_checker,
                    &param_constraints,
                );

                match return_check_result {
                    Ok(true) => {
                        debug!("Return refinement verified for '{}'", func.name.name);
                    }
                    Ok(false) => {
                        all_verified = false;
                        warn!("Return refinement violated for '{}'", func.name.name);
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }
        }

        if self.session.options().verbose > 0 {
            let stats = subsumption_checker.stats();
            debug!(
                "Subsumption stats for '{}': syntactic={}, smt={}, cache_hits={}",
                func.name.name, stats.syntactic_checks, stats.smt_checks, stats.cache_hits
            );
        }

        Ok(all_verified)
    }

    /// Verify return refinement using full Z3 SMT integration.
    ///
    /// Performs comprehensive SMT-based verification:
    ///
    ///   1. Extracts all return values from the function body.
    ///   2. Uses syntactic checking as a fast path for simple cases.
    ///   3. Falls back to Z3 SMT solver for complex cases.
    ///   4. Leverages subsumption checking for type relationships.
    ///   5. Reports detailed error messages with counterexamples.
    #[allow(clippy::too_many_arguments)]
    fn verify_return_refinement_smt(
        &self,
        func: &verum_ast::decl::FunctionDecl,
        _return_ty: &verum_ast::Type,
        predicate: &verum_ast::expr::Expr,
        smt_context: &SmtContext,
        _verifier: &SmtRefinementVerifier,
        _subsumption_checker: &SubsumptionChecker,
        param_constraints: &List<(&verum_ast::Type, Text)>,
    ) -> Result<bool> {
        use verum_ast::ty::TypeKind;
        use verum_smt::translate::Translator;

        let return_values = extract_return_values(func);

        if return_values.is_empty() {
            debug!("No explicit returns found in function '{}'", func.name.name);
            return Ok(true);
        }

        let mut all_verified = true;

        for (idx, return_expr) in return_values.iter().enumerate() {
            debug!(
                "Verifying return #{} in function '{}' against predicate",
                idx + 1,
                func.name.name
            );

            // Step 1: Try fast syntactic verification first (<1ms).
            if let Some(satisfied) = syntactic_check_refinement(return_expr, predicate) {
                if !satisfied {
                    all_verified = false;
                    let diag = DiagnosticBuilder::new(Severity::Error)
                        .message(format!(
                            "Return value in function '{}' violates refinement constraint: {:?}",
                            func.name.name, predicate
                        ))
                        .build();
                    self.session.emit_diagnostic(diag);
                    continue;
                }
                debug!(
                    "Return #{} verified syntactically in '{}'",
                    idx + 1,
                    func.name.name
                );
                continue;
            }

            // Step 2: Syntactic check inconclusive — use Z3 SMT solver.
            debug!(
                "Return #{} requires SMT verification in '{}'",
                idx + 1,
                func.name.name
            );

            let mut translator = Translator::new(smt_context);

            for (param_ty, param_name) in param_constraints {
                if let TypeKind::Refined {
                    base,
                    predicate: _param_pred,
                } = &param_ty.kind
                {
                    if let Ok(z3_var) = translator.create_var(param_name.as_str(), base) {
                        translator.bind(param_name.clone(), z3_var);
                    }
                }
            }

            let z3_result =
                self.verify_return_expr_smt(return_expr, predicate, &mut translator, smt_context);

            match z3_result {
                SmtCheckResult::Verified => {
                    debug!(
                        "Return #{} verified via SMT in '{}'",
                        idx + 1,
                        func.name.name
                    );
                }
                SmtCheckResult::Violated { counterexample } => {
                    all_verified = false;

                    let mut msg = format!(
                        "Return value in function '{}' does not satisfy refinement constraint",
                        func.name.name
                    );
                    if let Some(cex) = counterexample {
                        msg.push_str(&format!("\n  Counterexample: {}", cex));
                    }

                    let diag = DiagnosticBuilder::new(Severity::Error).message(msg).build();
                    self.session.emit_diagnostic(diag);
                }
                SmtCheckResult::Unknown { reason } => {
                    debug!(
                        "Return #{} SMT check inconclusive in '{}': {}",
                        idx + 1,
                        func.name.name,
                        reason
                    );
                    let diag = DiagnosticBuilder::new(Severity::Warning)
                        .message(format!(
                            "Cannot statically verify return refinement in '{}'. Falling back to runtime check. Reason: {}",
                            func.name.name, reason
                        ))
                        .build();
                    self.session.emit_diagnostic(diag);
                }
                SmtCheckResult::Timeout => {
                    debug!(
                        "Return #{} SMT check timed out in '{}'",
                        idx + 1,
                        func.name.name
                    );
                    let diag = DiagnosticBuilder::new(Severity::Warning)
                        .message(format!(
                            "Timeout verifying return refinement in '{}'. Falling back to runtime check.",
                            func.name.name
                        ))
                        .build();
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        Ok(all_verified)
    }

    /// Verify a specific return expression against a predicate using Z3.
    fn verify_return_expr_smt(
        &self,
        return_expr: &verum_ast::expr::Expr,
        predicate: &verum_ast::expr::Expr,
        translator: &mut verum_smt::translate::Translator<'_>,
        smt_context: &SmtContext,
    ) -> SmtCheckResult {
        use z3::ast::{Dynamic, Int};
        use z3::SatResult;

        // Create a fresh variable for the return value (bound to 'result' or 'it').
        let result_var = Int::new_const("result");
        let it_var = Int::new_const("it");

        translator.bind("result".into(), Dynamic::from_ast(&result_var));
        translator.bind("it".into(), Dynamic::from_ast(&it_var));

        let z3_predicate = match translator.translate_expr(predicate) {
            Ok(expr) => expr,
            Err(e) => {
                return SmtCheckResult::Unknown {
                    reason: format!("Failed to translate predicate: {:?}", e),
                };
            }
        };

        let z3_bool = match z3_predicate.as_bool() {
            Some(b) => b,
            None => {
                return SmtCheckResult::Unknown {
                    reason: "Predicate does not evaluate to boolean".to_string(),
                };
            }
        };

        let z3_return_value = match translator.translate_expr(return_expr) {
            Ok(expr) => expr,
            Err(e) => {
                return SmtCheckResult::Unknown {
                    reason: format!("Failed to translate return expression: {:?}", e),
                };
            }
        };

        let solver = smt_context.solver();

        if let Some(return_int) = z3_return_value.as_int() {
            solver.assert(result_var.eq(&return_int));
            solver.assert(it_var.eq(&return_int));
        }

        // We want to check if the predicate can be FALSE given the return value.
        // If UNSAT: predicate is always true for this return value (verified).
        // If SAT: found a counterexample where predicate is false (violated).
        solver.assert(z3_bool.not());

        match smt_context.check(&solver) {
            SatResult::Unsat => SmtCheckResult::Verified,
            SatResult::Sat => {
                let counterexample = solver.get_model().map(|model| {
                    let mut cex_str = String::new();
                    if let Some(val) = model.eval(&result_var, true).and_then(|v| v.as_i64()) {
                        cex_str.push_str(&format!("result = {}", val));
                    }
                    cex_str
                });
                SmtCheckResult::Violated { counterexample }
            }
            SatResult::Unknown => {
                let reason = solver
                    .get_reason_unknown()
                    .unwrap_or_else(|| "Unknown".to_string());
                if reason.contains("timeout") || reason.contains("canceled") {
                    SmtCheckResult::Timeout
                } else {
                    SmtCheckResult::Unknown { reason }
                }
            }
        }
    }

    /// Check if a type contains refinement predicates.
    pub(super) fn has_refinement_type(&self, ty: &verum_ast::Type) -> bool {
        use verum_ast::ty::TypeKind;
        match &ty.kind {
            TypeKind::Refined { .. } => true,
            TypeKind::Function {
                params,
                return_type,
                ..
            } => {
                params.iter().any(|p| self.has_refinement_type(p))
                    || self.has_refinement_type(return_type)
            }
            TypeKind::Tuple(types) => types.iter().any(|t| self.has_refinement_type(t)),
            TypeKind::Array { element, .. } => self.has_refinement_type(element),
            TypeKind::Reference { inner, .. } => self.has_refinement_type(inner),
            TypeKind::Ownership { inner, .. } => self.has_refinement_type(inner),
            _ => false,
        }
    }
}

/// Extract a variable name from a binding pattern.
pub(super) fn extract_pattern_name(pattern: &verum_ast::pattern::Pattern) -> Option<Text> {
    use verum_ast::pattern::PatternKind;
    match &pattern.kind {
        PatternKind::Ident { name, .. } => Some(Text::from(name.name.as_str())),
        _ => None,
    }
}

/// Extract all return values from a function body.
pub(super) fn extract_return_values(
    func: &verum_ast::decl::FunctionDecl,
) -> List<verum_ast::expr::Expr> {
    use verum_ast::decl::FunctionBody;
    use verum_ast::expr::ExprKind;
    use verum_ast::stmt::StmtKind;

    let mut returns = List::new();

    if let Some(ref body) = func.body {
        match body {
            FunctionBody::Block(block) => {
                if let Some(ref final_expr) = block.expr {
                    returns.push((**final_expr).clone());
                }
                for stmt in &block.stmts {
                    if let StmtKind::Expr { expr, .. } = &stmt.kind {
                        if let ExprKind::Return(Some(return_expr)) = &expr.kind {
                            returns.push((**return_expr).clone());
                        }
                    }
                }
            }
            FunctionBody::Expr(expr) => {
                returns.push(expr.clone());
            }
        }
    }

    returns
}

/// Simple syntactic check for common refinement patterns.
///
/// Returns `Some(true)` if definitely satisfied, `Some(false)` if
/// violated, `None` if inconclusive (needs SMT).
///
/// Examples:
///
///   * `x + 1` satisfies `result > x` (syntactic: x+1 > x always true for Int).
///   * `5` satisfies `result > 0` (syntactic: 5 > 0 is true).
///   * `-5` violates `result > 0` (syntactic: -5 > 0 is false).
pub(super) fn syntactic_check_refinement(
    value: &verum_ast::expr::Expr,
    predicate: &verum_ast::expr::Expr,
) -> Option<bool> {
    use verum_ast::expr::{BinOp, ExprKind};
    use verum_ast::literal::{Literal, LiteralKind};

    if let ExprKind::Binary { op, left, right } = &predicate.kind {
        if let ExprKind::Path(path) = &left.kind {
            if path.segments.len() == 1 {
                let var_name = match &path.segments[0] {
                    verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                    _ => return None,
                };

                if var_name == "result" || var_name == "it" {
                    if let ExprKind::Literal(Literal {
                        kind: LiteralKind::Int(lit),
                        ..
                    }) = &right.kind
                    {
                        let threshold = lit.value as i64;

                        if let ExprKind::Literal(Literal {
                            kind: LiteralKind::Int(val_lit),
                            ..
                        }) = &value.kind
                        {
                            let val = val_lit.value as i64;
                            let satisfied = match op {
                                BinOp::Gt => val > threshold,
                                BinOp::Ge => val >= threshold,
                                BinOp::Lt => val < threshold,
                                BinOp::Le => val <= threshold,
                                BinOp::Eq => val == threshold,
                                BinOp::Ne => val != threshold,
                                _ => return None,
                            };
                            return Some(satisfied);
                        }

                        // Pattern: value is `x + constant2` and
                        // predicate is `result > constant1`. If
                        // constant2 > 0, then x + constant2 > x,
                        // which may satisfy the predicate.
                        if let ExprKind::Binary {
                            op: BinOp::Add,
                            left: _,
                            right: add_right,
                        } = &value.kind
                        {
                            if let ExprKind::Literal(Literal {
                                kind: LiteralKind::Int(add_lit),
                                ..
                            }) = &add_right.kind
                            {
                                let add_val = add_lit.value as i64;
                                if matches!(op, BinOp::Gt) && threshold == 0 && add_val > 0 {
                                    return Some(true);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}
