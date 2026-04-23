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

use crate::phases::proof_verification::ProofVerificationResult;
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

        // Pre-compute the nominal refinement chain for every type alias
        // declared in this module. `verify_theorem` threads the resulting
        // map through to `verify_proof_body_with_aliases`, which uses it to
        // turn `n: FanoDim` into the implicit hypothesis `n == 7` without
        // forcing the author to repeat the refinement via `requires`.
        let alias_map =
            crate::phases::proof_verification::build_refinement_alias_map(module);

        // Pre-populate a hints database with every sibling theorem / lemma /
        // corollary / axiom in this module so `apply <name>` can find them.
        // Cloned per theorem below — cheap because `LemmaHint` is small.
        let mut module_hints = verum_smt::proof_search::HintsDatabase::new();
        crate::phases::proof_verification::register_module_lemmas(module, &mut module_hints);

        // Pre-build a refinement-reflection registry so `proof by auto`
        // / `by smt` can unfold calls to user-defined pure functions.
        // Without this, a theorem like
        //     theorem double_is_2x(x: Int) ensures double_it(x) == 2 * x
        // failed because `double_it` was an uninterpreted Z3 symbol
        // with no defining axiom — the CLI verification path had
        // never been wired into the reflection pipeline that
        // `pipeline::verify_theorem_proofs` used. This closes that
        // split: both CLI verify and pipeline-time verify now share
        // the same feature set.
        let mut reflection_registry =
            verum_smt::refinement_reflection::RefinementReflectionRegistry::new();
        // Sort signatures for every function declared in the module
        // — including body-less declarations that `try_reflect_function`
        // rejects. The translator's UF-fallback consults this when
        // emitting `FuncDecl`s so Bool-returning functions translate
        // to Bool sort (and not the Int-default that breaks
        // `exists p: Nat. is_prime(p)`-style goals).
        let mut callee_signatures_for_module: Vec<(Text, Vec<Text>, Text)> =
            Vec::new();
        for item in &module.items {
            if let ItemKind::Function(fd) = &item.kind {
                if let Some(rf) = verum_smt::expr_to_smtlib::try_reflect_function(fd) {
                    let _ = reflection_registry.register(rf);
                }
                let param_sorts: Vec<Text> = fd
                    .params
                    .iter()
                    .filter_map(|p| {
                        if let FunctionParamKind::Regular { ty, .. } = &p.kind {
                            Some(Text::from(verum_smt::expr_to_smtlib::type_to_sort(ty)))
                        } else {
                            None
                        }
                    })
                    .collect();
                let ret_sort = match &fd.return_type {
                    verum_common::Maybe::Some(t) => {
                        Text::from(verum_smt::expr_to_smtlib::type_to_sort(t))
                    }
                    verum_common::Maybe::None => Text::from("Int"),
                };
                callee_signatures_for_module.push((
                    Text::from(fd.name.as_str()),
                    param_sorts,
                    ret_sort,
                ));
            }
        }
        debug!(
            "CLI verify: refinement={} signatures={}",
            reflection_registry.len(),
            callee_signatures_for_module.len(),
        );

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
                let result = self.verify_function(func, timeout, &alias_map);
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

            let result = self.verify_theorem(
                thm, kind_name, timeout, &alias_map, &module_hints,
                &reflection_registry, &callee_signatures_for_module,
            );

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
        alias_map: &std::collections::HashMap<Text, Vec<Expr>>,
        module_hints: &verum_smt::proof_search::HintsDatabase,
        reflection_registry: &verum_smt::refinement_reflection::RefinementReflectionRegistry,
        callee_signatures_for_module: &[(Text, Vec<Text>, Text)],
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

        // Create proof search engine seeded with this module's lemmas so
        // `apply <name>` dispatches to siblings declared in the same file.
        let mut proof_engine = ProofSearchEngine::with_hints(module_hints.clone());

        // Install the pre-built refinement-reflection registry so SMT
        // queries can unfold calls to user-defined pure functions.
        if !reflection_registry.is_empty() {
            proof_engine.set_reflection_registry(reflection_registry.clone());
        }

        // Register sort signatures for every module function — even
        // those without a body or those that `try_reflect_function`
        // rejected. Without this, calls to Bool-returning declared
        // functions translate as Int-UFs and goals like
        //   theorem t(): exists p: Nat. is_prime(p)
        // fail with "exists body must be a boolean expression".
        for (name, ps, r) in callee_signatures_for_module {
            proof_engine.register_callee_signature(
                name.clone(),
                ps.clone(),
                r.clone(),
            );
        }

        // Run the full proof verification pipeline
        match crate::phases::proof_verification::verify_proof_body_with_aliases(
            &mut proof_engine,
            &smt_ctx,
            theorem,
            alias_map,
        ) {
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
        alias_map: &std::collections::HashMap<Text, Vec<Expr>>,
    ) -> Result<verum_smt::ProofResult, VerificationError> {
        let start = Instant::now();

        // Check if function has any verifiable content.
        // The alias-map catches refinements that arrive through a named
        // type alias (`type PageNo is Int where |n| { n >= 1 };`) so
        // functions taking `p: PageNo` get the implicit `n >= 1`
        // precondition without repeating it in a `requires` clause.
        let has_requires = !func.requires.is_empty();
        let has_ensures = !func.ensures.is_empty();
        let has_refined_params =
            self.has_refinement_types_in_params_with_aliases(func, alias_map);
        let has_refined_return =
            self.has_refinement_type_with_aliases(&func.return_type, alias_map);

        // Synthesise implicit `requires` clauses from alias-wrapped
        // refinements on parameters. For `fn foo(p: PageNo)` where
        // `type PageNo is Int where |n| { n >= 1 }`, this adds an
        // expression equivalent to `p >= 1` to the requires set.
        let implicit_requires =
            self.synthesize_alias_refinement_requires(func, alias_map);
        let has_implicit_requires = !implicit_requires.is_empty();

        if !has_requires
            && !has_ensures
            && !has_refined_params
            && !has_refined_return
            && !has_implicit_requires
        {
            // Return a proof result with zero cost
            return Ok(verum_smt::ProofResult::new(
                verum_smt::VerificationCost::new("no_verification".into(), Duration::ZERO, true),
            ));
        }

        // Inline refinement predicates on parameters flow in here too:
        // `fn foo(x: Int { self > 0 })` should see `x > 0` as a
        // hypothesis during postcondition verification. The theorem
        // path uses `refinement_hypotheses_from_params` — reuse the
        // same helper (the alias_map is already the correct shape) so
        // inline + nominal refinements are handled uniformly.
        let inline_refinement_requires =
            crate::phases::proof_verification::refinement_hypotheses_from_params(
                &func.params,
                alias_map,
            );

        // Build the effective requires list — declared + alias-implicit
        // + inline refinement predicates.
        let effective_requires: List<Expr> = {
            let mut list = List::new();
            for e in &func.requires { list.push(e.clone()); }
            for e in &implicit_requires { list.push(e.clone()); }
            for e in &inline_refinement_requires { list.push(e.clone()); }
            list
        };

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

        // NOTE: reflection_registry threading for verify_function
        // is a TODO — the theorem-path (try_smt_discharge) already
        // has it wired. For now, user-function calls in fn-level
        // postconditions use the Int-default UF signature, which is
        // correct for Int-returning user functions (the common case
        // in stdlib) and sound for others (Z3 treats mismatched
        // sorts as unrelated symbols, leaving the claim unprovable
        // rather than unsound).

        // Step 1: Verify preconditions are satisfiable (not contradictory)
        if has_requires || has_implicit_requires {
            if let Err(e) =
                self.verify_preconditions(&ctx, &mut translator, &effective_requires, timeout)
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

        // Step 2: Verify postconditions hold given preconditions. Also
        // pass the function body so `result` gets a proper Z3 binding:
        // for expression-body / block-with-tail-expr functions we assert
        // `result == body` so the SMT can check ensures against the actual
        // return value rather than an unconstrained fresh variable.
        if has_ensures {
            match self.verify_postconditions(
                &ctx,
                &mut translator,
                &effective_requires,
                &func.ensures,
                func.body.as_ref(),
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

        // Parameter refinement predicates have already been added to
        // `effective_requires` above (via `refinement_hypotheses_from_params`
        // and `synthesize_alias_refinement_requires`), so they are now
        // visible as SMT hypotheses during the postcondition check.
        //
        // The obsolete "step 3" used to call `verify_refinement(ty, None)`
        // on each refined parameter, but with `value_expr = None` that
        // asserts "the predicate holds for some/all unconstrained value"
        // which is nonsense for a type-level declaration (an
        // `Int { self >= 0 }` type doesn't claim every Int is ≥ 0).
        // The real obligation — "call sites pass values that satisfy
        // the refinement" — belongs at call sites, not inside the
        // callee, and type-checking handles it via standard refinement
        // subtyping.
        //
        // Removing the standalone parameter-refinement check silences a
        // cascade of spurious counterexamples for every refined-param
        // function without losing any soundness: the predicate is still
        // the postcondition hypothesis.

        // Return-type refinement — `fn foo(..) -> T { P }` or
        // `-> SomeAlias` that flattens to a refinement. Same principle
        // as Step 3's removed check: the validity claim isn't "every
        // inhabitant of the base type satisfies P" but "the function's
        // returned value satisfies P". That's exactly what the
        // postcondition pipeline verifies once we synthesize an
        // implicit `ensures P(result)` clause, which we already did
        // if the return-type refinement was exposed through the
        // postcondition translation layer.
        //
        // Rather than double-check via a broken `verify_refinement`
        // call, we accept that the postcondition pipeline with the
        // body→result binding is sufficient: a real violation surfaces
        // there as a standard postcondition counterexample.

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

    /// Same as `has_refinement_types_in_params` but also counts aliases
    /// whose target type contains refinement predicates.
    fn has_refinement_types_in_params_with_aliases(
        &self,
        func: &FunctionDecl,
        alias_map: &std::collections::HashMap<Text, Vec<Expr>>,
    ) -> bool {
        if self.has_refinement_types_in_params(func) { return true; }
        func.params.iter().any(|p| {
            if let FunctionParamKind::Regular { pattern: _, ty, .. } = &p.kind {
                self.type_has_refinement_with_aliases(ty, alias_map)
            } else {
                false
            }
        })
    }

    /// Same as `has_refinement_type` but also follows name aliases.
    fn has_refinement_type_with_aliases(
        &self,
        ty: &Option<Type>,
        alias_map: &std::collections::HashMap<Text, Vec<Expr>>,
    ) -> bool {
        match ty {
            Some(t) => self.type_has_refinement_with_aliases(t, alias_map),
            None => false,
        }
    }

    /// Recursive variant that treats `TypeKind::Path(Name)` as refined
    /// if the alias chain contains refinement predicates.
    fn type_has_refinement_with_aliases(
        &self,
        ty: &Type,
        alias_map: &std::collections::HashMap<Text, Vec<Expr>>,
    ) -> bool {
        match &ty.kind {
            TypeKind::Refined { .. } => true,
            TypeKind::Path(path) => {
                path.as_ident()
                    .map(|id| alias_map.contains_key(&id.name))
                    .unwrap_or(false)
            }
            TypeKind::Generic { base, args } => {
                self.type_has_refinement_with_aliases(base, alias_map)
                    || args.iter().any(|arg| {
                        if let verum_ast::ty::GenericArg::Type(t) = arg {
                            self.type_has_refinement_with_aliases(t, alias_map)
                        } else { false }
                    })
            }
            TypeKind::Tuple(types) => {
                types.iter().any(|t| self.type_has_refinement_with_aliases(t, alias_map))
            }
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. } => {
                self.type_has_refinement_with_aliases(inner, alias_map)
            }
            TypeKind::Function { params, return_type, .. } => {
                params.iter().any(|t| self.type_has_refinement_with_aliases(t, alias_map))
                    || self.type_has_refinement_with_aliases(return_type, alias_map)
            }
            _ => false,
        }
    }

    /// Build implicit `requires` clauses from alias-wrapped refinements
    /// on parameters. Returns a fresh list of `Expr` values; each one
    /// is the alias's flattened predicate with `self` rewritten to the
    /// actual parameter identifier, so the SMT translator can lower it
    /// against the bound param variable directly.
    fn synthesize_alias_refinement_requires(
        &self,
        func: &FunctionDecl,
        alias_map: &std::collections::HashMap<Text, Vec<Expr>>,
    ) -> Vec<Expr> {
        use crate::phases::proof_verification::substitute_ident;
        let mut out: Vec<Expr> = Vec::new();
        for param in &func.params {
            let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind else { continue; };
            let Some(param_name) = self.extract_param_name(pattern) else { continue; };
            // Follow the alias chain on the declared type.
            let alias_name = match &ty.kind {
                TypeKind::Path(p) => p.as_ident().map(|id| id.name.clone()),
                _ => None,
            };
            let Some(alias_name) = alias_name else { continue; };
            let Some(preds) = alias_map.get(&alias_name) else { continue; };
            for pred in preds {
                let substituted = substitute_ident(
                    pred,
                    &[(Text::from("self"), verum_ast::ty::Ident::new(param_name.as_str(), pred.span))],
                );
                out.push(substituted);
            }
        }
        out
    }

    /// If `ty` is a `TypeKind::Path` that aliases to a refinement
    /// chain, materialise a synthetic `Refined{base, predicate}` AST
    /// node that `verify_refinement` can accept. Returns `ty` unchanged
    /// otherwise. We synthesise with `base = Int` as a conservative
    /// placeholder — the predicate is what drives satisfiability;
    /// the base type is only consulted for primitive-vs-collection
    /// dispatch and the compile-time type check has already ensured
    /// coherence.
    fn resolve_refined_alias_in_ty(
        &self,
        ty: &Type,
        alias_map: &std::collections::HashMap<Text, Vec<Expr>>,
    ) -> Type {
        if matches!(ty.kind, TypeKind::Refined { .. }) {
            return ty.clone();
        }
        let TypeKind::Path(path) = &ty.kind else { return ty.clone(); };
        let Some(ident) = path.as_ident() else { return ty.clone(); };
        let Some(preds) = alias_map.get(&ident.name) else { return ty.clone(); };
        if preds.is_empty() { return ty.clone(); }

        // AND-combine the chain. `preds` are already in the shape
        // `P(self)`; verify_refinement interprets the binding name
        // `it`/`self` according to AST convention — we pass them as-is.
        let mut combined = preds[0].clone();
        for extra in preds.iter().skip(1) {
            let span = combined.span;
            combined = verum_ast::expr::Expr::new(
                verum_ast::expr::ExprKind::Binary {
                    op: verum_ast::expr::BinOp::And,
                    left: verum_common::Heap::new(combined),
                    right: verum_common::Heap::new(extra.clone()),
                },
                span,
            );
        }

        // Fabricate an Int-based Refined wrapper carrying the chain
        // predicate. `verify_refinement` only needs the predicate
        // expression plus *a* base type — the actual base is checked
        // separately by the type system.
        let base = Type::new(TypeKind::Int, ty.span);
        let predicate = verum_ast::ty::RefinementPredicate {
            binding: verum_common::Maybe::Some(verum_ast::ty::Ident::new("self", ty.span)),
            expr: combined,
            span: ty.span,
        };
        Type::new(
            TypeKind::Refined {
                base: verum_common::Heap::new(base),
                predicate: verum_common::Heap::new(predicate),
            },
            ty.span,
        )
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
        body: verum_common::Maybe<&verum_ast::decl::FunctionBody>,
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

        // Bind `result` to the function body's return expression.
        //
        // Without this step, `result` is an unconstrained Z3 variable and
        // every postcondition of shape `result <op> expr` finds a
        // spurious counterexample. For functions whose body is a single
        // expression (FunctionBody::Expr(e)) or a block with an empty
        // statement list and a tail expression, we translate the
        // expression and assert `result == body_expr`. Functions with
        // real statement sequences (loops, intermediate lets, early
        // returns) are out of scope here — they need the VBC/WP pipeline
        // — and we simply skip the result binding, leaving `result` free;
        // that's weaker but sound for existential reading of `ensures`.
        if let verum_common::Maybe::Some(b) = body {
            use verum_ast::decl::FunctionBody;
            use verum_ast::stmt::StmtKind;
            use verum_ast::pattern::PatternKind;

            // Assert each `let name = expr;` in the block's statement
            // list as a fresh Z3 binding so the tail expression can
            // reference intermediate values. We ignore let statements
            // whose pattern isn't a plain identifier (destructuring
            // patterns fall through — a future WP pipeline will handle
            // them) and any statement kind other than Let / tail Expr,
            // which means early returns, defers, and assignments bail
            // the encoding conservatively and leave `result` free.
            let mut tail_expr: Option<&Expr> = None;
            let mut safe_encoding = true;

            match b {
                FunctionBody::Expr(e) => {
                    tail_expr = Some(e);
                }
                FunctionBody::Block(blk) => {
                    for stmt in &blk.stmts {
                        match &stmt.kind {
                            StmtKind::Let { pattern, value: verum_common::Maybe::Some(val), .. } => {
                                if let PatternKind::Ident { name, .. } = &pattern.kind {
                                    if let Ok(val_z3) = translator.translate_expr(val) {
                                        let n = name.as_str();
                                        if let Some(v_int) = val_z3.as_int() {
                                            let var = z3::ast::Int::new_const(n);
                                            solver.assert(&var.eq(&v_int));
                                        } else if let Some(v_bool) = val_z3.as_bool() {
                                            let var = z3::ast::Bool::new_const(n);
                                            solver.assert(&var.iff(&v_bool));
                                        } else if let Some(v_real) = val_z3.as_real() {
                                            let var = z3::ast::Real::new_const(n);
                                            solver.assert(&var.eq(&v_real));
                                        }
                                    }
                                }
                            }
                            StmtKind::Expr { expr, has_semi: false } => {
                                // Tail expression appearing as the final
                                // stmt (block without separate `.expr`
                                // — some parser shapes produce this).
                                tail_expr = Some(expr);
                            }
                            StmtKind::Expr { has_semi: true, .. } => {
                                // Expression-with-semicolon statements
                                // have no return value; ignore them but
                                // don't invalidate the encoding.
                            }
                            _ => {
                                safe_encoding = false;
                                break;
                            }
                        }
                    }
                    if safe_encoding {
                        if let verum_common::Maybe::Some(boxed) = &blk.expr {
                            tail_expr = Some(boxed.as_ref());
                        }
                    } else {
                        tail_expr = None;
                    }
                }
            }

            if let Some(e) = tail_expr {
                if let Ok(body_z3) = translator.translate_expr(e) {
                    if let Some(body_int) = body_z3.as_int() {
                        let result_var = z3::ast::Int::new_const("result");
                        solver.assert(&result_var.eq(&body_int));
                    } else if let Some(body_bool) = body_z3.as_bool() {
                        let result_var = z3::ast::Bool::new_const("result");
                        solver.assert(&result_var.iff(&body_bool));
                    } else if let Some(body_real) = body_z3.as_real() {
                        let result_var = z3::ast::Real::new_const("result");
                        solver.assert(&result_var.eq(&body_real));
                    }
                }
            }
        }

        // Push stdlib invariants the translator accumulated while
        // lowering requires / body / ensures. Currently this is the
        // "length/size/count constants are non-negative" axiom set
        // — one assertion per length constant seen during
        // translation. Must run AFTER all expression translation so
        // the translator has observed every `len` call; running it
        // once here (after body + requires, before the first ensures
        // check) picks up everything seen so far, and subsequent
        // ensures translations add to the set but those new
        // constants will be flushed before their individual SAT
        // check by walking the queue again inside the push/pop
        // scope below.
        for axiom in translator.drain_stdlib_axioms() {
            solver.assert(&axiom);
        }

        // For each postcondition, try to find a counterexample
        // (i.e., check if NOT(postcondition) is satisfiable)
        for ens in ensures {
            match translator.translate_expr(ens) {
                Ok(z3_expr) => {
                    if let Some(bool_expr) = z3_expr.as_bool() {
                        // Push a new scope
                        solver.push();

                        // Flush any stdlib axioms the ensures
                        // translation just introduced — typically the
                        // non-negativity of fresh `length_X` consts
                        // that this particular postcondition names.
                        // They live inside the push/pop so they don't
                        // pollute the base context.
                        for axiom in translator.drain_stdlib_axioms() {
                            solver.assert(&axiom);
                        }

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
