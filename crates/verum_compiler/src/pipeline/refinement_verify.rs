//! Per-function refinement-type verification (Z3 + CVC5 portfolio).
//!
//! Extracted from `pipeline.rs` (#106 Phase 4). Implements full
//! Z3-based refinement-type verification:
//!
//!  1. Extracts refinement predicates from parameter / return types.
//!  2. Generates Z3 assertions for each refinement constraint.
//!  3. Uses `verum_smt::RefinementVerifier` to verify constraints.
//!  4. Caches verification results for performance.
//!  5. Reports detailed error messages with counterexamples.
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
        alias_map: &std::collections::HashMap<Text, Vec<verum_ast::expr::Expr>>,
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

        // The parameters' refinement predicates, binder-rewritten to the
        // parameter's own name — these are the HYPOTHESES the return-
        // refinement query runs under (T0680). Without them the
        // identity function over a refined type false-rejected:
        // `fn f(x: Int{> 0}) -> Int{> 0} { x }` asked Z3 to prove
        // `x > 0` from nothing and got `result = 0`. Covers the inline
        // form AND alias-wrapped params (`x: P` where
        // `type P is Int{> 0}`) via the module's refinement alias map.
        // Dropping an untranslatable hypothesis is sound (the query
        // just gets weaker); fabricating one never happens — the
        // rewrite is the same explicit|it|self -> name contract the
        // verify path uses (T0678).
        let param_hypotheses: Vec<verum_ast::expr::Expr> =
            collect_param_refinement_hypotheses(func, alias_map);

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

                    let verification_result =
                        verifier.verify_refinement(ty, None, Some(SmtVerifyMode::Auto));

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
                    &param_hypotheses,
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
    ///  1. Extracts all return values from the function body.
    ///  2. Uses syntactic checking as a fast path for simple cases.
    ///  3. Falls back to Z3 SMT solver for complex cases.
    ///  4. Leverages subsumption checking for type relationships.
    ///  5. Reports detailed error messages with counterexamples.
    #[allow(clippy::too_many_arguments)]
    fn verify_return_refinement_smt(
        &self,
        func: &verum_ast::decl::FunctionDecl,
        return_ty: &verum_ast::Type,
        predicate: &verum_ast::expr::Expr,
        smt_context: &SmtContext,
        _verifier: &SmtRefinementVerifier,
        _subsumption_checker: &SubsumptionChecker,
        param_constraints: &List<(&verum_ast::Type, Text)>,
        param_hypotheses: &[verum_ast::expr::Expr],
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

            // The REFINED BASE TYPE decides the sort of `it` / `result`. It was
            // available here all along (`return_ty` arrived as an ignored
            // `_return_ty`) while the binder was hardcoded to Int — see
            // `verify_return_expr_smt`.
            let refined_base = match &return_ty.kind {
                TypeKind::Refined { base, .. } => Some(base.as_ref()),
                _ => None,
            };

            let z3_result = self.verify_return_expr_smt(
                return_expr,
                predicate,
                refined_base,
                &mut translator,
                smt_context,
                param_hypotheses,
            );

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
        refined_base: Option<&verum_ast::Type>,
        translator: &mut verum_smt::translate::Translator<'_>,
        smt_context: &SmtContext,
        param_hypotheses: &[verum_ast::expr::Expr],
    ) -> SmtCheckResult {
        use z3::SatResult;
        use z3::ast::{Dynamic, Int};

        // The refinement binder takes the sort of the type being refined.
        //
        // This used to be `Int::new_const("it")` unconditionally, with no
        // reference to the refined type at all. Every refinement on a
        // non-integer type therefore mistranslated: for
        // `Float{it >= -1.0}` the binder was a Z3 Int while the literal
        // became a Z3 Real (`precise_floats` defaults false, so
        // `Real::from_rational` is taken), and the binop dispatcher — which
        // tries (int,int), then (real,real), then (bool,bool), and has no
        // arm for a MIXED pair and performs no Int-to-Real coercion — fell
        // through to
        // `TypeMismatch("incompatible types for binary operation >=")`.
        // (There is an IEEE-754 float path too, but only when
        // `precise_floats` is enabled, which it is not by default.)
        //
        // The predicate then "failed to translate", the static proof
        // silently DEGRADED to a runtime check, and the build still
        // succeeded with only a warning — a verification feature reporting
        // success for a proof it never performed. 18 of the 20 refinements
        // in `core/` are that shape.
        //
        // `create_var` already derives the right sort per TypeKind and is
        // exactly what the PARAMETER path above uses; only the return binder
        // was hardcoded.
        //
        // Declared unconditionally so the fallback bindings outlive the match;
        // only used when there is no refined base type to take a sort from.
        let fallback_result = Int::new_const("result");
        let fallback_it = Int::new_const("it");
        let (result_dyn, it_dyn) = match refined_base {
            Some(base) => {
                match (
                    translator.create_var("result", base),
                    translator.create_var("it", base),
                ) {
                    (Ok(r), Ok(i)) => (r, i),
                    // Report rather than silently falling back to Int: an
                    // unsortable binder is exactly the condition that produced
                    // the false green, and a fallback would restore it.
                    _ => {
                        return SmtCheckResult::Unknown {
                            reason: format!(
                                "Cannot give the refinement binder a sort for base type {:?}",
                                base.kind
                            ),
                        };
                    }
                }
            }
            None => (
                Dynamic::from_ast(&fallback_result),
                Dynamic::from_ast(&fallback_it),
            ),
        };

        translator.bind("result".into(), result_dyn.clone());
        translator.bind("it".into(), it_dyn.clone());

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

        // Assert the parameters' refinement predicates as HYPOTHESES
        // on THIS solver (T0680). A refined parameter is an
        // assumption about the argument, established at every call
        // site by refinement subtyping — without it the identity
        // function over a refined type false-rejected
        // (`fn f(x: Int{> 0}) -> Int{> 0} { x }` asked Z3 to prove
        // `x > 0` from no premises and got `result = 0`).
        //
        // These MUST be asserted on the solver the query runs on:
        // `SmtContext::solver()` constructs a FRESH solver per call
        // (context.rs:97), so hypotheses asserted anywhere else are
        // silently discarded. An untranslatable hypothesis is dropped
        // (the query merely gets weaker — sound), never fabricated.
        for hyp in param_hypotheses {
            match translator.translate_expr(hyp) {
                Ok(z3_hyp) => {
                    if let Some(b) = z3_hyp.as_bool() {
                        solver.assert(&b);
                    }
                }
                Err(e) => {
                    debug!(
                        "param refinement hypothesis dropped (untranslatable): {:?}",
                        e
                    );
                }
            }
        }

        // Tie the binder to the returned value AT THE BINDER'S OWN SORT.
        // This was `if let Some(return_int) = z3_return_value.as_int()`, so for
        // any non-integer return the equality was simply never asserted and the
        // binder stayed unconstrained — the second Int assumption on this path.
        // Downcast both sides to a common concrete sort before asserting —
        // mirrors how the translator's own binop dispatcher decides, and
        // avoids asserting an equality across mismatched sorts.
        //
        // Whether the tie actually happened is RECORDED, because when it does
        // not the binder is a free variable and every later verdict is about
        // that free variable rather than about this code. `create_var` only
        // ever produces Int, Bool or Real for a scalar base (anything else
        // returns Err above), so these three arms are exhaustive over the
        // binders that reach here — what they cannot cover is a VALUE of a
        // different sort, e.g. a Real binder against the Int-sorted symbol an
        // unregistered callee gets.
        let binder_tied = if let (Some(r), Some(i), Some(v)) = (
            result_dyn.as_int(),
            it_dyn.as_int(),
            z3_return_value.as_int(),
        ) {
            solver.assert(r.eq(&v));
            solver.assert(i.eq(&v));
            true
        } else if let (Some(r), Some(i), Some(v)) = (
            result_dyn.as_real(),
            it_dyn.as_real(),
            z3_return_value.as_real(),
        ) {
            solver.assert(r.eq(&v));
            solver.assert(i.eq(&v));
            true
        } else if let (Some(r), Some(i), Some(v)) = (
            result_dyn.as_bool(),
            it_dyn.as_bool(),
            z3_return_value.as_bool(),
        ) {
            solver.assert(r.eq(&v));
            solver.assert(i.eq(&v));
            true
        } else {
            false
        };

        // We want to check if the predicate can be FALSE given the return value.
        // If UNSAT: predicate is always true for this return value (verified).
        // If SAT: found a counterexample where predicate is false (violated).
        solver.assert(z3_bool.not());

        match smt_context.check(&solver) {
            SatResult::Unsat => SmtCheckResult::Verified,
            // SOUND, not an approximation: if the equality above could not be
            // asserted, the binder was never connected to the returned value.
            // The solver is then free to pick any value for it, so a Sat is a
            // statement about a free variable and says nothing about this code.
            // Reporting it as a violation is a false red by construction.
            //
            // This is the arm that catches the common case on THIS path: the
            // pipeline registers no callee signatures, so an unregistered
            // callee's symbol is Int-sorted while a `Float` refinement's binder
            // is Real, and no branch above matches. It therefore carries most
            // of the weight that the syntactic rule below would otherwise bear,
            // leaving that rule load-bearing only where the two sorts DO agree.
            SatResult::Sat if !binder_tied => SmtCheckResult::Unknown {
                reason: "not proven: the refinement binder could not be tied to \
                         the returned value at a common sort, so it is \
                         unconstrained and any counterexample describes a free \
                         variable rather than this code"
                    .to_string(),
            },
            // A Sat is only a DISPROOF when the counterexample is built from
            // constrained terms. When the return expression contains a CALL,
            // the callee is an UNINTERPRETED function to the solver: nothing
            // constrains its result, so a model that returns 100 from it always
            // exists and the "counterexample" carries no information about the
            // code. Measured: `fn bounded() -> Float { 0.5 }` with
            // `fn t() -> Float{it >= -1.0 && it <= 1.0} { bounded() }` is Sat,
            // although the callee ALWAYS satisfies the predicate.
            //
            // Reporting those as violations produces FALSE REDS on correct
            // code, which is the same defect as a false green pointed the other
            // way — and a build-blocking diagnostic that fires on correct code
            // gets silenced wholesale, taking the true reds with it.
            //
            // APPROXIMATION, STATED: this is syntactic — any call in the return
            // expression demotes Sat to "not proven". IT MAY THEREFORE MASK A
            // GENUINE VIOLATION THAT ARRIVES THROUGH A CALL. The real remedy is
            // to give callees POSTCONDITIONS so the model is constrained and Sat
            // regains its meaning.
            //
            // Note the division of labour with the arm above: every call whose
            // symbol carries a DIFFERENT sort from the binder is already caught
            // there, soundly. What is left for this rule is the same-sort call,
            // and for that case the solver genuinely cannot tell an invented
            // result from a real violation, so no finer rule is available.
            //
            // Out-of-range literal returns are unaffected: those are rejected
            // earlier by the syntactic checker (`E500: refinement constraint
            // failed`) before SMT runs, which is what keeps this tolerable.
            SatResult::Sat if expr_contains_call(return_expr) => SmtCheckResult::Unknown {
                reason: "not proven: the return value comes from a call, and an \
                         uninterpreted callee admits any result — supply a \
                         postcondition on the callee to make this provable"
                    .to_string(),
            },
            SatResult::Sat => {
                let counterexample = solver.get_model().map(|model| {
                    let mut cex_str = String::new();
                    // Render at whatever sort the binder actually has — an
                    // `as_i64()` here silently produced an EMPTY counterexample
                    // for every non-integer refinement.
                    if let Some(val) = model.eval(&result_dyn, true) {
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
///  * `x + 1` satisfies `result > x` (syntactic: x+1 > x always true for Int).
///  * `5` satisfies `result > 0` (syntactic: 5 > 0 is true).
///  * `-5` violates `result > 0` (syntactic: -5 > 0 is false).
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

/// Does `expr` contain a call anywhere in its subtree?
///
/// Used to decide whether a Z3 `Sat` is a genuine disproof or merely the
/// solver inventing a result for an uninterpreted callee — see the
/// `SatResult::Sat` arm in `verify_return_expr_smt`.
fn expr_contains_call(expr: &verum_ast::expr::Expr) -> bool {
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::visitor::{Visitor, walk_expr};

    struct CallFinder {
        found: bool,
    }

    impl Visitor for CallFinder {
        fn visit_expr(&mut self, expr: &Expr) {
            if matches!(
                expr.kind,
                ExprKind::Call { .. } | ExprKind::MethodCall { .. }
            ) {
                self.found = true;
            }
            walk_expr(self, expr);
        }
    }

    let mut finder = CallFinder { found: false };
    finder.visit_expr(expr);
    finder.found
}

/// Pins the REACH of the call-shape approximation in `verify_return_expr_smt`,
/// in both directions — because it is wrong in both directions.
///
/// Too narrow and correct code is accused (a `Sat` over an uninterpreted callee
/// becomes a build-blocking error). Too wide and genuine refinement violations
/// are demoted from errors to `not proven` warnings. There is no count to
/// ratchet here: how many `core/` refinements land in the unproven bucket is a
/// measure of how few callees carry postconditions, not of how many defects
/// exist — freezing it would pin ignorance. What CAN be pinned is the rule, so
/// that is what these do.
#[cfg(test)]
mod refinement_binder_sort_pins {
    use super::expr_contains_call;
    use verum_ast::expr::{BinOp, Expr, ExprKind};
    use verum_ast::{Ident, Literal, Span};
    use verum_common::{Heap, List};

    fn lit_float(v: f64) -> Expr {
        Expr::literal(Literal::float(v, Span::dummy()))
    }

    fn var(name: &str) -> Expr {
        Expr::ident(Ident::new(name, Span::dummy()))
    }

    fn call_of(name: &str) -> Expr {
        Expr::new(
            ExprKind::Call {
                func: Heap::new(var(name)),
                type_args: List::default(),
                args: List::default(),
            },
            Span::dummy(),
        )
    }

    fn binary(op: BinOp, left: Expr, right: Expr) -> Expr {
        Expr::new(
            ExprKind::Binary {
                op,
                left: Heap::new(left),
                right: Heap::new(right),
            },
            Span::dummy(),
        )
    }

    /// THE DEFECT AND THE FIX, SIDE BY SIDE — the pin for the binder sort.
    ///
    /// `it >= 0.0` over a `Float` refinement translates cleanly when the binder
    /// carries the refined type's sort, and FAILS TO TRANSLATE under the
    /// hardcoded `Int` binder this change removed. Both halves run in one body,
    /// so neither can rot: restoring the Int binder breaks the first assertion,
    /// and "fixing" the mismatch by making the binop dispatcher coerce Int to
    /// Real breaks the second — at which point this pin would no longer be
    /// witnessing a difference and says so.
    ///
    /// Why it mattered: an untranslatable predicate is not an error. It
    /// silently DEGRADED the static proof to a runtime check while the build
    /// went on succeeding with a warning — a verification feature reporting
    /// success for a proof it never performed.
    #[test]
    fn float_refinement_binder_needs_the_refined_types_sort() {
        use verum_ast::ty::{Type, TypeKind};
        use verum_smt::translate::Translator;

        let ctx = verum_smt::Context::new();
        let float_ty = Type::new(TypeKind::Float, Span::dummy());
        let predicate = binary(BinOp::Ge, var("it"), lit_float(0.0));

        // AFTER: the binder takes the refined base's sort, so it is a Z3 Real —
        // the same sort the literal gets, because `precise_floats` defaults off
        // and float literals become `Real::from_rational`.
        let mut fixed = Translator::new(&ctx);
        let it = fixed
            .create_var("it", &float_ty)
            .expect("Float must have a Z3 sort");
        assert!(
            it.as_real().is_some(),
            "the binder for a Float refinement must be Real-sorted; the Int \
             binder is what made every non-integer refinement untranslatable"
        );
        fixed.bind("it".into(), it);
        assert!(
            fixed.translate_expr(&predicate).is_ok(),
            "`it >= 0.0` must translate once the binder carries the refined \
             type's sort"
        );

        // BEFORE: the hardcoded Int binder. The dispatcher has arms for
        // (int,int), (real,real) and (bool,bool) and none for a mixed pair, so
        // this is the `TypeMismatch` that was silently degrading the proof.
        let mut broken = Translator::new(&ctx);
        broken.bind(
            "it".into(),
            z3::ast::Dynamic::from_ast(&z3::ast::Int::new_const("it")),
        );
        assert!(
            broken.translate_expr(&predicate).is_err(),
            "an Int binder against a Real literal must still FAIL to translate \
             — if this ever succeeds, the two halves of this pin no longer \
             differ and it has stopped witnessing the defect"
        );
    }

    /// The premise of the three-arm equality downcast and of the `binder_tied`
    /// guard: `create_var` produces exactly Int, Bool or Real for a scalar
    /// base, and REFUSES anything else rather than inventing a sort.
    ///
    /// Both halves are load-bearing. If a fourth scalar sort ever appears, the
    /// three arms stop being exhaustive over binders and that binder silently
    /// stops being tied to its return value. If the refusal ever became a
    /// fallback — say to Int — the original defect returns wholesale, since an
    /// Int binder against a Real value is exactly what was mistranslating.
    #[test]
    fn create_var_covers_the_three_downcast_sorts_and_refuses_the_rest() {
        use verum_ast::ty::{Path, Type, TypeKind};
        use verum_smt::translate::Translator;

        let ctx = verum_smt::Context::new();
        let tr = Translator::new(&ctx);
        let of = |kind: TypeKind| Type::new(kind, Span::dummy());

        let int = tr.create_var("b", &of(TypeKind::Int)).expect("Int sorts");
        let boolean = tr.create_var("b", &of(TypeKind::Bool)).expect("Bool sorts");
        let float = tr
            .create_var("b", &of(TypeKind::Float))
            .expect("Float sorts");
        assert!(
            int.as_int().is_some() && boolean.as_bool().is_some() && float.as_real().is_some(),
            "Int/Bool/Float must map to the three sorts the equality downcast \
             handles; a fourth would silently stop being tied to its return value"
        );

        // A named type the translator has no sort for. The binder path must
        // report this, not substitute Int.
        let named = of(TypeKind::Path(Path::single(Ident::new(
            "Text",
            Span::dummy(),
        ))));
        assert!(
            tr.create_var("b", &named).is_err(),
            "an unsortable base must be REFUSED, not given a fallback sort — a \
             fallback to Int is precisely the defect this change removed"
        );
    }

    // ---- must NOT be call-shaped: these keep genuine violations as errors ----

    /// A literal return is fully constrained, so a `Sat` over it is a REAL
    /// counterexample and must stay a hard error.
    #[test]
    fn literal_return_is_not_call_shaped() {
        assert!(
            !expr_contains_call(&lit_float(0.5)),
            "a literal return must NOT be treated as call-shaped, or genuine \
             refinement violations are demoted to `not proven` warnings"
        );
    }

    /// Arithmetic over literals is equally constrained — `2.0 * 3.0` violating
    /// `it <= 1.0` is exactly the violation this checker exists to catch.
    #[test]
    fn arithmetic_over_literals_is_not_call_shaped() {
        let expr = binary(BinOp::Mul, lit_float(2.0), lit_float(3.0));
        assert!(
            !expr_contains_call(&expr),
            "arithmetic over literals must NOT be treated as call-shaped: the \
             solver has every term, so a Sat is a genuine counterexample"
        );
    }

    /// The load-bearing negative. Returning a bare parameter is the one shape
    /// where the value IS unconstrained and a `Sat` IS still a disproof — the
    /// caller may pass anything, so the refinement genuinely does not hold.
    /// Demoting this to a warning would hide real violations behind the same
    /// "unconstrained term" intuition that justifies the call rule.
    #[test]
    fn bare_parameter_return_is_not_call_shaped() {
        assert!(
            !expr_contains_call(&var("x")),
            "returning a parameter must NOT be treated as call-shaped: it is \
             unconstrained, but a Sat over it is a GENUINE violation because \
             the caller chooses the value"
        );
    }

    // ---- must BE call-shaped: these prevent false reds on correct code ----

    /// A direct call — the shape measured as Sat-but-correct
    /// (`fn t() -> Float{-1.0 <= it <= 1.0} { bounded() }`).
    #[test]
    fn call_return_is_call_shaped() {
        assert!(
            expr_contains_call(&call_of("bounded")),
            "a call-shaped return must be detected, or a vacuous Sat over an \
             uninterpreted callee is reported as a violation of correct code"
        );
    }

    /// The other arm of the discriminator. A method call is just as
    /// uninterpreted to the solver as a free function call.
    #[test]
    fn method_call_return_is_call_shaped() {
        let expr = Expr::new(
            ExprKind::MethodCall {
                receiver: Heap::new(var("x")),
                method: Ident::new("bounded", Span::dummy()),
                type_args: List::default(),
                args: List::default(),
            },
            Span::dummy(),
        );
        assert!(
            expr_contains_call(&expr),
            "a method call must be detected too, or `x.bounded()` gets the \
             false red that `bounded()` is spared"
        );
    }

    /// Pins the WALK rather than the root node. `expr_contains_call` must
    /// descend; replacing it with a top-level `matches!` would still pass every
    /// test above and silently restore the false red for `f() + 1.0`.
    #[test]
    fn call_nested_in_arithmetic_is_call_shaped() {
        let expr = binary(BinOp::Add, call_of("bounded"), lit_float(1.0));
        assert!(
            expr_contains_call(&expr),
            "a call nested inside an expression must be detected: the callee is \
             no less uninterpreted for having a literal added to it"
        );
    }
}

/// The refinement predicates carried by a function's parameters,
/// binder-rewritten to each parameter's own name (T0680). Inline
/// `x: Int{ it > 0 }` forms rewrite their binder (explicit binding,
/// or the implicit `it` / legacy `self`); alias-wrapped `x: P` forms
/// take the module alias map's flattened predicates (stored in the
/// `self` spelling, `it` also covered). These are the hypotheses a
/// return-refinement query must run under — a refined parameter IS
/// an assumption about the argument, established at every call site.
pub(super) fn collect_param_refinement_hypotheses(
    func: &verum_ast::decl::FunctionDecl,
    alias_map: &std::collections::HashMap<Text, Vec<verum_ast::expr::Expr>>,
) -> Vec<verum_ast::expr::Expr> {
    use crate::phases::proof_verification::substitute_ident;
    use verum_ast::ty::TypeKind;
    let mut out = Vec::new();
    for param in &func.params {
        let verum_ast::decl::FunctionParamKind::Regular { pattern, ty, .. } = &param.kind else {
            continue;
        };
        let Some(param_name) = extract_pattern_name(pattern) else {
            continue;
        };
        match &ty.kind {
            TypeKind::Refined { base: _, predicate } => {
                let target = verum_ast::ty::Ident::new(param_name.as_str(), predicate.expr.span);
                let substitutions: Vec<(Text, verum_ast::ty::Ident)> = match &predicate.binding {
                    verum_common::Maybe::Some(binder) => {
                        vec![(binder.name.clone(), target)]
                    }
                    verum_common::Maybe::None => vec![
                        (Text::from("it"), target.clone()),
                        (Text::from("self"), target),
                    ],
                };
                out.push(substitute_ident(&predicate.expr, &substitutions));
            }
            TypeKind::Path(path) => {
                if let Some(id) = path.as_ident()
                    && let Some(preds) = alias_map.get(&id.name)
                {
                    for pred in preds {
                        let target = verum_ast::ty::Ident::new(param_name.as_str(), pred.span);
                        out.push(substitute_ident(
                            pred,
                            &[
                                (Text::from("it"), target.clone()),
                                (Text::from("self"), target.clone()),
                            ],
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    out
}
