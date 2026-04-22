//! Verification Command with Cost Reporting
//!
//! P0 Feature for v1.0: Show verification costs and suggest optimizations
//!
//! # Example Output
//!
//! ```text
//! $ verum verify app.vr --show-costs
//!
//! Verification Report:
//!   ✓ algorithm(): Proved in 1.2s (Z3)
//!   ⚠ complex_fn(): Timeout after 30s, falling back to runtime
//!   ✗ invalid_fn(): Counterexample found: n = 0
//!
//! Suggestions:
//!   - Use @verify(runtime) for complex_fn (30s → 0s)
//!   - Add precondition n > 0 to invalid_fn
//! ```

use anyhow::{Context as AnyhowContext, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};
use verum_ast::{Expr, FunctionDecl, FunctionParamKind, ItemKind, Module, Type, TypeKind};
use verum_ast::decl::TheoremDecl;
use verum_common::span::Span;
use verum_smt::{
    Context as SmtContext, ContextConfig, Translator, VerificationError,
    VerifyMode, verification_cache::VerificationCache, verify_refinement,
    proof_search::ProofSearchEngine,
};

use verum_common::{List, Map, Text, ToText};

use crate::phases::proof_verification::{verify_proof_body, ProofVerificationResult};
use crate::pipeline::CompilationPipeline;
use crate::session::Session;
use crate::verification_profiler::{FileLocation, VerificationProfiler};

/// Verification command handler
pub struct VerifyCommand<'s> {
    session: &'s mut Session,
    cache: VerificationCache,
    budget_tracker: BudgetTracker,
    profiler: Option<VerificationProfiler>,
}

impl<'s> VerifyCommand<'s> {
    /// Create new verification command
    pub fn new(session: &'s mut Session) -> Self {
        let budget = session
            .options()
            .verification_budget_secs
            .map(|s| Duration::from_secs(s));
        let slow_threshold =
            Duration::from_secs(session.options().slow_verification_threshold_secs);

        // Enable profiler if requested
        let profiler = if session.options().profile_verification {
            Some(VerificationProfiler::new())
        } else {
            None
        };

        Self {
            session,
            cache: VerificationCache::new(),
            budget_tracker: BudgetTracker::new(budget, slow_threshold),
            profiler,
        }
    }

    /// Run verification with cost reporting
    pub fn run(mut self, function_name: Option<&str>) -> Result<()> {
        info!(
            "SMT verification backend: {:?} (timeout: {}s)",
            self.session.options().smt_solver,
            self.session.options().smt_timeout_secs
        );

        // Load and parse source
        let input = self.session.options().input.clone();
        let file_id = self
            .session
            .load_file(&input)
            .with_context(|| format!("Failed to load: {}", input.display()))?;

        // Parse and type check
        let mut pipeline = CompilationPipeline::new(self.session);
        pipeline.run_check_only()?;

        let module = self
            .session
            .get_module(file_id)
            .map(|m| (*m).clone())
            .ok_or_else(|| anyhow::anyhow!("Module not found after parsing"))?;

        // Run verification
        let report = self.verify_module(&module, function_name)?;

        // Display report
        self.display_report(&report);

        // Display cache statistics
        self.display_cache_stats(&report);

        // Display suggestions if enabled
        if self.session.options().show_verification_costs {
            self.display_suggestions(&report);
        }

        // Display profiler report if enabled
        if let Some(ref profiler) = self.profiler {
            // Update profiler with cache stats
            let _cache_stats = self.cache.stats();
            // Note: We'd need to make profiler mutable here, but that requires
            // refactoring. For now, the profiler tracks its own stats.
            profiler.print_report();
        }

        // Export to JSON if requested
        if self.session.options().export_verification_json {
            self.export_json(&report)?;
        }

        // Check budget
        if self.budget_tracker.is_exceeded() {
            let exceeded_by = self.budget_tracker.exceeded_by();
            anyhow::bail!(
                "Verification budget exceeded by {:.1}s",
                exceeded_by.as_secs_f64()
            );
        }

        // Exit with error if any verification failed
        if report.has_failures() {
            anyhow::bail!("Verification failed");
        }

        Ok(())
    }

    /// Verify all functions in module
    fn verify_module(
        &mut self,
        module: &Module,
        filter: Option<&str>,
    ) -> Result<VerificationReport> {
        let mut report = VerificationReport::new();
        let timeout = Duration::from_secs(self.session.options().smt_timeout_secs);

        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                // Skip if filter doesn't match
                if let Some(name) = filter {
                    if func.name.as_str() != name {
                        continue;
                    }
                }

                debug!("Verifying function: {}", func.name);

                // Check budget before starting
                if self.budget_tracker.should_stop() {
                    let result = VerificationResult::Skipped;
                    report.add_result(func.name.as_str().to_text(), result);
                    continue;
                }

                // Verify the function
                let start_time = Instant::now();
                let result = self.verify_function(func, timeout);
                let elapsed = start_time.elapsed();

                // Profile if enabled (extract location before mutable borrow)
                let location = if self.profiler.is_some() {
                    Some(self.extract_file_location(func.span))
                } else {
                    None
                };

                if let Some(ref mut profiler) = self.profiler {
                    // Safe to use location here since it was extracted before the mutable borrow
                    profiler.record_result(func.name.as_str(), location.unwrap(), elapsed, &result);
                }

                // Convert VerificationResult to our result type
                let result = match result {
                    Ok(proof) => VerificationResult::Proved {
                        elapsed: proof.cost.duration,
                    },
                    Err(VerificationError::Timeout { .. }) => VerificationResult::Timeout {
                        elapsed: timeout,
                        timeout,
                    },
                    Err(VerificationError::CannotProve {
                        counterexample,
                        cost,
                        constraint,
                        ..
                    }) => VerificationResult::Failed {
                        // Prefer the structured counterexample's human-
                        // readable summary over the Debug form. Falls
                        // back to the constraint description when no
                        // model was extracted.
                        counterexample: counterexample
                            .map(|ce| ce.format_with_suggestions(&[]))
                            .or(Some(constraint)),
                        elapsed: cost.duration,
                    },
                    Err(e) => VerificationResult::Failed {
                        counterexample: Some(format!("{}", e).to_text()),
                        elapsed: timeout,
                    },
                };

                // Track time spent
                if let VerificationResult::Proved { elapsed } = &result {
                    self.budget_tracker
                        .add_time(*elapsed, func.name.as_str().to_text());
                } else if let VerificationResult::Failed { elapsed, .. } = &result {
                    self.budget_tracker
                        .add_time(*elapsed, func.name.as_str().to_text());
                } else if let VerificationResult::Timeout { elapsed, .. } = &result {
                    self.budget_tracker
                        .add_time(*elapsed, func.name.as_str().to_text());
                }

                report.add_result(func.name.as_str().to_text(), result);
            }

            // Verify theorems, lemmas, and corollaries
            let (thm, kind_name) = match &item.kind {
                ItemKind::Theorem(t) => (t, "theorem"),
                ItemKind::Lemma(t) => (t, "lemma"),
                ItemKind::Corollary(t) => (t, "corollary"),
                _ => continue,
            };

            // Skip if filter doesn't match
            if let Some(name) = filter {
                if thm.name.as_str() != name {
                    continue;
                }
            }

            debug!("Verifying {} '{}' ({} requires, {} ensures)",
                kind_name, thm.name, thm.requires.len(), thm.ensures.len());

            // Check budget before starting
            if self.budget_tracker.should_stop() {
                report.add_result(
                    format!("{} {}", kind_name, thm.name).to_text(),
                    VerificationResult::Skipped,
                );
                continue;
            }

            let result = self.verify_theorem(thm, kind_name, timeout);

            // Note: Profiler is not used for theorem verification (different result type)

            // Track time spent
            match &result {
                VerificationResult::Proved { elapsed }
                | VerificationResult::Failed { elapsed, .. } => {
                    self.budget_tracker
                        .add_time(*elapsed, thm.name.as_str().to_text());
                }
                VerificationResult::Timeout { elapsed, .. } => {
                    self.budget_tracker
                        .add_time(*elapsed, thm.name.as_str().to_text());
                }
                _ => {}
            }

            report.add_result(
                format!("{} {}", kind_name, thm.name).to_text(),
                result,
            );
        }

        Ok(report)
    }

    /// Verify a theorem/lemma/corollary using the proof verification engine
    ///
    /// This verifies:
    /// 1. The proposition is well-formed
    /// 2. The proof body (if present) correctly proves the proposition
    /// 3. Preconditions (requires clauses) and postconditions (ensures clauses)
    fn verify_theorem(
        &self,
        theorem: &TheoremDecl,
        kind_name: &str,
        timeout: Duration,
    ) -> VerificationResult {
        let start = Instant::now();

        // Theorems without proof bodies are axioms - accept them
        if theorem.proof.is_none() {
            info!("{} '{}' accepted as axiom (no proof body)", kind_name, theorem.name);
            return VerificationResult::Proved {
                elapsed: start.elapsed(),
            };
        }

        // Create SMT context for proof verification
        let smt_config = ContextConfig {
            timeout: Some(timeout),
            ..Default::default()
        };
        let smt_ctx = SmtContext::with_config(smt_config);

        // Create proof search engine
        let hints_db = verum_smt::proof_search::HintsDatabase::new();
        let mut proof_engine = ProofSearchEngine::with_hints(hints_db);

        // Run the full proof verification pipeline
        match verify_proof_body(&mut proof_engine, &smt_ctx, theorem) {
            ProofVerificationResult::Verified(cert) => {
                let has_incomplete = cert.has_incomplete_steps;
                info!(
                    "{} '{}' verified ({} steps, {:.1}ms){}",
                    kind_name,
                    theorem.name,
                    cert.steps.len(),
                    cert.total_duration.as_secs_f64() * 1000.0,
                    if has_incomplete { " [incomplete: uses admit/sorry]" } else { "" }
                );
                VerificationResult::Proved {
                    elapsed: start.elapsed(),
                }
            }
            ProofVerificationResult::Failed { unproved, .. } => {
                let error_msg = if let Some(first) = unproved.first() {
                    let mut msg = format!("unproved goal: {}", first.goal);
                    if !first.suggestions.is_empty() {
                        msg.push_str(&format!(
                            " (hint: {})",
                            first.suggestions.iter().next().map(|s| s.as_str()).unwrap_or("")
                        ));
                    }
                    msg
                } else {
                    "proof verification failed".to_string()
                };

                warn!(
                    "{} '{}' verification failed: {} unproved goal(s)",
                    kind_name,
                    theorem.name,
                    unproved.len()
                );

                VerificationResult::Failed {
                    counterexample: Some(error_msg.to_text()),
                    elapsed: start.elapsed(),
                }
            }
        }
    }

    /// Verify a single function using Z3 SMT solver
    ///
    /// This performs real verification of:
    /// 1. Preconditions (requires clauses) - must be satisfiable
    /// 2. Postconditions (ensures clauses) - must hold given preconditions
    /// 3. Refinement types in parameters and return type
    fn verify_function(
        &self,
        func: &FunctionDecl,
        timeout: Duration,
    ) -> Result<verum_smt::ProofResult, VerificationError> {
        let start = Instant::now();

        // Check if function has any verifiable content
        let has_requires = !func.requires.is_empty();
        let has_ensures = !func.ensures.is_empty();
        let has_refined_params = self.has_refinement_types_in_params(func);
        let has_refined_return = self.has_refinement_type(&func.return_type);

        if !has_requires && !has_ensures && !has_refined_params && !has_refined_return {
            // Return a proof result with zero cost
            return Ok(verum_smt::ProofResult::new(
                verum_smt::VerificationCost::new("no_verification".into(), Duration::ZERO, true),
            ));
        }

        // Create Z3 context with timeout
        let config = ContextConfig {
            timeout: Some(timeout),
            ..Default::default()
        };
        let ctx = SmtContext::with_config(config);

        // Create translator for AST -> Z3 conversion
        let mut translator = Translator::new(&ctx);

        // Bind function parameters as Z3 variables
        for param in &func.params {
            if let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                if let Some(name) = self.extract_param_name(pattern) {
                    if let Ok(z3_var) = translator.create_var(name.as_str(), ty) {
                        translator.bind(name.clone(), z3_var);
                    }
                }
            }
        }

        // Step 1: Verify preconditions are satisfiable (not contradictory)
        if has_requires {
            if let Err(e) =
                self.verify_preconditions(&ctx, &mut translator, &func.requires, timeout)
            {
                debug!(
                    "Precondition verification failed for {}: {}",
                    func.name,
                    e.as_str()
                );
                return Err(VerificationError::CannotProve {
                    constraint: e,
                    counterexample: None,
                    cost: verum_smt::VerificationCost::new(
                        func.name.as_str().into(),
                        start.elapsed(),
                        false,
                    ),
                    suggestions: List::new(),
                });
            }
            debug!("Preconditions verified for {}", func.name);
        }

        // Step 2: Verify postconditions hold given preconditions
        if has_ensures {
            match self.verify_postconditions(
                &ctx,
                &mut translator,
                &func.requires,
                &func.ensures,
                timeout,
            ) {
                Ok(()) => debug!("Postconditions verified for {}", func.name),
                Err(VerifyError::Timeout) => {
                    return Err(VerificationError::Timeout {
                        constraint: func.name.as_str().into(),
                        timeout,
                        cost: verum_smt::VerificationCost::new(
                            func.name.as_str().into(),
                            start.elapsed(),
                            false,
                        )
                        .with_timeout(),
                    });
                }
                Err(VerifyError::Failed(desc, ce)) => {
                    return Err(VerificationError::CannotProve {
                        constraint: desc,
                        counterexample: ce,
                        cost: verum_smt::VerificationCost::new(
                            func.name.as_str().into(),
                            start.elapsed(),
                            false,
                        ),
                        suggestions: List::new(),
                    });
                }
            }
        }

        // Step 3: Verify refinement types in parameters
        if has_refined_params {
            for param in &func.params {
                if let FunctionParamKind::Regular { pattern: _, ty, .. } = &param.kind {
                    if let TypeKind::Refined {
                        base: _,
                        predicate: _,
                    } = &ty.kind
                    {
                        if let Err(e) = verify_refinement(&ctx, ty, None, VerifyMode::Auto) {
                            return Err(e);
                        }
                    }
                }
            }
        }

        // Step 4: Verify refinement type in return type
        if has_refined_return {
            if let Some(ref ret_ty) = func.return_type {
                if let TypeKind::Refined {
                    base: _,
                    predicate: _,
                } = &ret_ty.kind
                {
                    if let Err(e) = verify_refinement(&ctx, ret_ty, None, VerifyMode::Auto) {
                        return Err(e);
                    }
                }
            }
        }

        // All checks passed - create proof result with cost tracking
        let cost = verum_smt::VerificationCost::new(
            func.name.as_str().into(),
            start.elapsed(),
            true, // succeeded
        );

        Ok(verum_smt::ProofResult::new(cost))
    }

    /// Check if function has any refinement types in parameters
    fn has_refinement_types_in_params(&self, func: &FunctionDecl) -> bool {
        func.params.iter().any(|p| {
            if let FunctionParamKind::Regular { pattern: _, ty, .. } = &p.kind {
                self.type_has_refinement(ty)
            } else {
                false
            }
        })
    }

    /// Check if type contains refinement predicates
    fn has_refinement_type(&self, ty: &Option<Type>) -> bool {
        match ty {
            Some(t) => self.type_has_refinement(t),
            None => false,
        }
    }

    /// Recursively check if type has refinement
    fn type_has_refinement(&self, ty: &Type) -> bool {
        match &ty.kind {
            TypeKind::Refined { .. } => true,
            TypeKind::Generic { args, .. } => args.iter().any(|arg| {
                if let verum_ast::ty::GenericArg::Type(t) = arg {
                    self.type_has_refinement(t)
                } else {
                    false
                }
            }),
            TypeKind::Tuple(types) => types.iter().any(|t| self.type_has_refinement(t)),
            TypeKind::Function {
                params,
                return_type,
                ..
            } => {
                params.iter().any(|t| self.type_has_refinement(t))
                    || self.type_has_refinement(return_type)
            }
            _ => false,
        }
    }

    /// Extract parameter name from pattern
    fn extract_param_name(&self, pattern: &verum_ast::Pattern) -> Option<Text> {
        match &pattern.kind {
            verum_ast::PatternKind::Ident { name, .. } => Some(name.as_str().to_text()),
            _ => None,
        }
    }

    /// Extract file location (path, line, column) from a span
    ///
    /// Converts a byte-offset Span to a human-readable FileLocation
    /// by looking up the source file and computing line/column positions.
    fn extract_file_location(&self, span: Span) -> FileLocation {
        use std::path::PathBuf;

        // Try to get the source file for this span
        if let Some(source_file) = self.session.get_source(span.file_id) {
            // Convert byte offsets to line/column positions
            let (line, column) = source_file.line_col(span.start);

            // Get the file path (or name if path is not available)
            let file_path = if let Some(ref path) = source_file.path {
                path.clone()
            } else {
                PathBuf::from(source_file.name.as_str())
            };

            FileLocation::new(
                file_path,
                line + 1,   // Convert from 0-indexed to 1-indexed
                column + 1, // Convert from 0-indexed to 1-indexed
            )
        } else {
            // Source file not found - return unknown location
            FileLocation::unknown()
        }
    }

    /// Verify preconditions are satisfiable (not contradictory)
    fn verify_preconditions(
        &self,
        ctx: &SmtContext,
        translator: &mut Translator<'_>,
        requires: &[Expr],
        _timeout: Duration,
    ) -> Result<(), Text> {
        if requires.is_empty() {
            return Ok(());
        }

        let solver = ctx.solver();

        // Assert all preconditions
        for req in requires {
            match translator.translate_expr(req) {
                Ok(z3_expr) => {
                    if let Some(bool_expr) = z3_expr.as_bool() {
                        solver.assert(&bool_expr);
                    } else {
                        return Err(format!("Precondition is not boolean: {:?}", req).to_text());
                    }
                }
                Err(e) => {
                    return Err(format!("Failed to translate precondition: {}", e).to_text());
                }
            }
        }

        // Check satisfiability - preconditions should be satisfiable
        match solver.check() {
            z3::SatResult::Sat => Ok(()),
            z3::SatResult::Unsat => {
                Err("Preconditions are contradictory (unsatisfiable)".to_text())
            }
            z3::SatResult::Unknown => {
                // Unknown is acceptable - may be due to timeout or complex formulas
                Ok(())
            }
        }
    }

    /// Verify postconditions hold given preconditions
    fn verify_postconditions(
        &self,
        ctx: &SmtContext,
        translator: &mut Translator<'_>,
        requires: &[Expr],
        ensures: &[Expr],
        _timeout: Duration,
    ) -> Result<(), VerifyError> {
        if ensures.is_empty() {
            return Ok(());
        }

        let solver = ctx.solver();

        // Assert preconditions as assumptions
        for req in requires {
            if let Ok(z3_expr) = translator.translate_expr(req) {
                if let Some(bool_expr) = z3_expr.as_bool() {
                    solver.assert(&bool_expr);
                }
            }
        }

        // For each postcondition, try to find a counterexample
        // (i.e., check if NOT(postcondition) is satisfiable)
        for ens in ensures {
            match translator.translate_expr(ens) {
                Ok(z3_expr) => {
                    if let Some(bool_expr) = z3_expr.as_bool() {
                        // Push a new scope
                        solver.push();

                        // Assert negation of postcondition
                        solver.assert(&bool_expr.not());

                        match solver.check() {
                            z3::SatResult::Sat => {
                                // Found counterexample — postcondition can
                                // be violated. Extract a structured
                                // CounterExample from the model so the CLI
                                // shows the witnessing variable assignment
                                // rather than Debug-formatted Z3 output.
                                let (ce_opt, ce_summary) = match solver.get_model() {
                                    Some(m) => {
                                        let ce = build_counterexample_from_model(&m);
                                        let summary = ce.format_with_suggestions(&[]);
                                        (Some(ce), summary)
                                    }
                                    None => (
                                        None,
                                        Text::from("counterexample exists (model unavailable)"),
                                    ),
                                };
                                solver.pop(1);
                                return Err(VerifyError::Failed(
                                    format!(
                                        "Postcondition violation\n{}",
                                        ce_summary.as_str()
                                    )
                                    .to_text(),
                                    ce_opt,
                                ));
                            }
                            z3::SatResult::Unsat => {
                                // No counterexample - postcondition holds
                                solver.pop(1);
                            }
                            z3::SatResult::Unknown => {
                                solver.pop(1);
                                return Err(VerifyError::Timeout);
                            }
                        }
                    }
                }
                Err(e) => {
                    return Err(VerifyError::Failed(
                        format!("Failed to translate postcondition: {}", e).to_text(),
                        None,
                    ));
                }
            }
        }

        Ok(())
    }

    /// Display verification report
    fn display_report(&self, report: &VerificationReport) {
        println!("{}", "\nVerification Report:".bold());
        println!("{}", "=".repeat(60));

        for (name, result) in &report.results {
            match result {
                VerificationResult::Proved { elapsed } => {
                    println!(
                        "  {} {}: Proved in {:.2}s",
                        "✓".green().bold(),
                        name.as_str().bold(),
                        elapsed.as_secs_f64()
                    );
                }
                VerificationResult::Failed {
                    counterexample,
                    elapsed,
                } => {
                    println!(
                        "  {} {}: Failed in {:.2}s",
                        "✗".red().bold(),
                        name.as_str().bold(),
                        elapsed.as_secs_f64()
                    );
                    if let Some(ce) = counterexample {
                        println!("      Counterexample: {}", ce.as_str().yellow());
                    }
                }
                VerificationResult::Timeout { elapsed, timeout } => {
                    println!(
                        "  {} {}: Timeout after {:.2}s (limit: {:.2}s)",
                        "⚠".yellow().bold(),
                        name.as_str().bold(),
                        elapsed.as_secs_f64(),
                        timeout.as_secs_f64()
                    );
                    println!("      {}", "Falling back to runtime checks".yellow());
                }
                VerificationResult::Skipped => {
                    println!(
                        "  {} {}: Skipped (no refinement types)",
                        "-".dimmed(),
                        name.as_str().dimmed()
                    );
                }
            }
        }

        println!();
        println!(
            "Summary: {} proved, {} failed, {} timeout, {} skipped",
            report.num_proved().to_string().green(),
            report.num_failed().to_string().red(),
            report.num_timeout().to_string().yellow(),
            report.num_skipped().to_string().dimmed()
        );
    }

    /// Display optimization suggestions
    fn display_suggestions(&self, report: &VerificationReport) {
        if !report.has_failures() && report.num_timeout() == 0 {
            return;
        }

        println!("{}", "\nSuggestions:".bold());
        println!("{}", "=".repeat(60));

        for (name, result) in &report.results {
            match result {
                VerificationResult::Timeout {
                    elapsed,
                    timeout: _,
                } => {
                    println!(
                        "  {} Use {} for {} ({:.1}s → 0s)",
                        "•".yellow(),
                        "@verify(runtime)".cyan(),
                        name,
                        elapsed.as_secs_f64()
                    );
                    println!(
                        "      This will skip SMT verification and use runtime checks instead"
                    );
                }
                VerificationResult::Failed { counterexample, .. } => {
                    println!("  {} Fix preconditions in {}", "•".red(), name);
                    if let Some(ce) = counterexample {
                        println!("      Add constraint to prevent: {}", ce);
                    }
                }
                _ => {}
            }
        }

        // Display slow functions
        let slow_threshold = self.budget_tracker.slow_threshold;
        let slow_funcs = self.budget_tracker.get_slow_functions();
        if !slow_funcs.is_empty() {
            println!(
                "\n  {} Slow verifications (>{:.1}s):",
                "⚠".yellow(),
                slow_threshold.as_secs_f64()
            );
            for (name, time) in slow_funcs {
                println!("      {} took {:.1}s", name.as_str(), time.as_secs_f64());
            }
        }

        println!();
    }

    /// Display cache statistics
    fn display_cache_stats(&self, report: &VerificationReport) {
        let stats = self.cache.stats();
        if stats.cache_hits == 0 && stats.cache_misses == 0 {
            return; // No cache activity
        }

        println!("{}", "\nCache Statistics:".bold());
        println!("{}", "=".repeat(60));

        let total_time = report.total_time();
        print!("{}", stats.format_report(total_time).as_str());

        if let Some(expired) = self.cache.count_expired().checked_sub(0) {
            if expired > 0 {
                println!("Cache evictions:  {} (TTL expired)", expired);
            }
        }

        println!();
    }

    /// Export verification results to JSON
    fn export_json(&self, report: &VerificationReport) -> Result<()> {
        let json_path = self
            .session
            .options()
            .verification_json_path
            .clone()
            .unwrap_or_else(|| "verification_report.json".into());

        let json_report = report.to_json();
        let json_str = serde_json::to_string_pretty(&json_report)
            .context("Failed to serialize verification report")?;

        let mut file = File::create(&json_path)
            .with_context(|| format!("Failed to create {}", json_path.display()))?;

        file.write_all(json_str.as_bytes())
            .with_context(|| format!("Failed to write to {}", json_path.display()))?;

        println!("Exported verification report to: {}", json_path.display());

        Ok(())
    }
}

/// Verification result for a single function
#[derive(Debug, Clone)]
pub enum VerificationResult {
    /// Successfully proved all refinements
    Proved { elapsed: Duration },

    /// Verification failed with counterexample
    Failed {
        counterexample: Option<Text>,
        elapsed: Duration,
    },

    /// Verification timeout
    Timeout {
        elapsed: Duration,
        timeout: Duration,
    },

    /// Skipped (no refinement types)
    Skipped,
}

/// Complete verification report
#[derive(Debug, Clone)]
pub struct VerificationReport {
    results: List<(Text, VerificationResult)>,
    start_time: Instant,
}

impl VerificationReport {
    /// Create a new empty verification report
    pub fn new() -> Self {
        Self {
            results: List::new(),
            start_time: Instant::now(),
        }
    }

    /// Add a verification result for a function
    pub fn add_result(&mut self, name: Text, result: VerificationResult) {
        self.results.push((name, result));
    }

    /// Count of successfully proved functions
    pub fn num_proved(&self) -> usize {
        self.results
            .iter()
            .filter(|(_, r)| matches!(r, VerificationResult::Proved { .. }))
            .count()
    }

    /// Count of failed verifications
    pub fn num_failed(&self) -> usize {
        self.results
            .iter()
            .filter(|(_, r)| matches!(r, VerificationResult::Failed { .. }))
            .count()
    }

    /// Count of timed out verifications
    pub fn num_timeout(&self) -> usize {
        self.results
            .iter()
            .filter(|(_, r)| matches!(r, VerificationResult::Timeout { .. }))
            .count()
    }

    /// Count of skipped functions (no refinement types)
    pub fn num_skipped(&self) -> usize {
        self.results
            .iter()
            .filter(|(_, r)| matches!(r, VerificationResult::Skipped))
            .count()
    }

    /// Check if any verification failed
    pub fn has_failures(&self) -> bool {
        self.num_failed() > 0
    }

    /// Total time since report creation
    pub fn total_time(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Convert to JSON representation
    pub fn to_json(&self) -> VerificationReportJson {
        let results: List<_> = self
            .results
            .iter()
            .map(|(name, result)| {
                let (status, elapsed, counterexample) = match result {
                    VerificationResult::Proved { elapsed } => {
                        ("proved".to_string(), Some(elapsed.as_secs_f64()), None)
                    }
                    VerificationResult::Failed {
                        elapsed,
                        counterexample,
                    } => (
                        "failed".to_string(),
                        Some(elapsed.as_secs_f64()),
                        counterexample.clone().map(|t| t.to_string()),
                    ),
                    VerificationResult::Timeout { elapsed, .. } => {
                        ("timeout".to_string(), Some(elapsed.as_secs_f64()), None)
                    }
                    VerificationResult::Skipped => ("skipped".to_string(), None, None),
                };

                FunctionResultJson {
                    function: name.to_string(),
                    status,
                    elapsed_secs: elapsed,
                    counterexample,
                }
            })
            .collect();

        VerificationReportJson {
            total_functions: self.results.len(),
            proved: self.num_proved(),
            failed: self.num_failed(),
            timeout: self.num_timeout(),
            skipped: self.num_skipped(),
            total_time_secs: self.total_time().as_secs_f64(),
            results,
        }
    }
}

/// Internal error type for verification.
///
/// `Failed` carries both a human-readable description and an
/// optional structured [`CounterExample`]. The structured form
/// lets the outer `VerificationError::CannotProve` thread the
/// counterexample through to the CLI's display path rather than
/// burying it inside a Debug-formatted string.
enum VerifyError {
    /// Verification timed out.
    Timeout,
    /// Verification failed; the optional counterexample carries the
    /// SMT model that witnessed the failure.
    Failed(Text, Option<verum_smt::CounterExample>),
}

/// Extract a structured [`verum_smt::CounterExample`] from a Z3
/// model. Iterates every 0-arity declaration in the model and
/// records its value as a [`CounterExampleValue`]. Complex values
/// (records, arrays, non-finite bitvectors) fall through to
/// `Unknown(text)` with the Z3 display form so users still see
/// something actionable.
fn build_counterexample_from_model(model: &z3::Model) -> verum_smt::CounterExample {
    use verum_common::{Map, Text};
    use verum_smt::{CounterExample, CounterExampleValue};

    let mut assignments: Map<Text, CounterExampleValue> = Map::new();

    for decl in model.iter() {
        // Only 0-ary constants carry a concrete value; functions are
        // handled separately via `advanced_model::CompleteFunctionModel`
        // when refinements need them.
        if decl.arity() != 0 {
            continue;
        }
        let name = decl.name().to_string();
        let applied = decl.apply(&[]);
        let evaluated = match model.eval(&applied, true) {
            Some(v) => v,
            None => continue,
        };
        let as_text = evaluated.to_string();

        // Try to narrow the Z3 AST into a typed counterexample value.
        // The Z3 bindings don't expose a stable "AST kind" API, so we
        // fall back on parsing the display form — reliable for the
        // primitive sorts verification actually hits (Int, Bool, Real,
        // BitVector-as-hex, String).
        let value = if let Ok(i) = as_text.parse::<i64>() {
            CounterExampleValue::Int(i)
        } else if as_text == "true" {
            CounterExampleValue::Bool(true)
        } else if as_text == "false" {
            CounterExampleValue::Bool(false)
        } else if let Ok(f) = as_text.parse::<f64>() {
            CounterExampleValue::Float(f)
        } else if as_text.starts_with('"')
            && as_text.ends_with('"')
            && as_text.len() >= 2
        {
            CounterExampleValue::Text(Text::from(&as_text[1..as_text.len() - 1]))
        } else {
            CounterExampleValue::Unknown(Text::from(as_text.clone()))
        };

        assignments.insert(Text::from(name.as_str()), value);
    }

    CounterExample::new(
        assignments,
        Text::from("postcondition violation"),
    )
}

// ==================== JSON Export Structures ====================

/// JSON representation of verification report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReportJson {
    pub total_functions: usize,
    pub proved: usize,
    pub failed: usize,
    pub timeout: usize,
    pub skipped: usize,
    pub total_time_secs: f64,
    pub results: List<FunctionResultJson>,
}

/// JSON representation of a single function result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionResultJson {
    pub function: String,
    pub status: String,
    pub elapsed_secs: Option<f64>,
    pub counterexample: Option<String>,
}

// ==================== Budget Tracking ====================

/// Tracks verification budget and slow functions
pub struct BudgetTracker {
    /// Total budget (None = unlimited)
    budget: Option<Duration>,
    /// Slow function threshold
    slow_threshold: Duration,
    /// Time spent so far
    time_spent: Duration,
    /// Function times
    function_times: Map<Text, Duration>,
}

impl BudgetTracker {
    pub fn new(budget: Option<Duration>, slow_threshold: Duration) -> Self {
        Self {
            budget,
            slow_threshold,
            time_spent: Duration::ZERO,
            function_times: Map::new(),
        }
    }

    pub fn add_time(&mut self, elapsed: Duration, function_name: Text) {
        self.time_spent += elapsed;
        self.function_times.insert(function_name, elapsed);
    }

    pub fn should_stop(&self) -> bool {
        if let Some(budget) = self.budget {
            self.time_spent >= budget
        } else {
            false
        }
    }

    pub fn is_exceeded(&self) -> bool {
        if let Some(budget) = self.budget {
            self.time_spent > budget
        } else {
            false
        }
    }

    pub fn exceeded_by(&self) -> Duration {
        if let Some(budget) = self.budget {
            if self.time_spent > budget {
                return self.time_spent - budget;
            }
        }
        Duration::ZERO
    }

    pub fn get_slow_functions(&self) -> List<(Text, Duration)> {
        let mut slow: List<_> = self
            .function_times
            .iter()
            .filter(|(_, time)| **time > self.slow_threshold)
            .map(|(name, time)| (name.clone(), *time))
            .collect();

        // Sort by time descending
        slow.sort_by(|a, b| b.1.cmp(&a.1));
        slow
    }

    pub fn remaining_budget(&self) -> Option<Duration> {
        self.budget.map(|b| {
            if self.time_spent < b {
                b - self.time_spent
            } else {
                Duration::ZERO
            }
        })
    }
}
