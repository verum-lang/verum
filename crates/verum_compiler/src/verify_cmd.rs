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
//!  ✓ algorithm(): Proved in 1.2s (Z3)
//!  ⚠ complex_fn(): Timeout after 30s, falling back to runtime
//!  ✗ invalid_fn(): Counterexample found: n = 0
//!
//! Suggestions:
//!  - Use @verify(runtime) for complex_fn (30s → 0s)
//!  - Add precondition n > 0 to invalid_fn
//! ```

use anyhow::{Context as AnyhowContext, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};
use verum_ast::decl::TheoremDecl;
use verum_ast::{Expr, FunctionDecl, FunctionParamKind, ItemKind, Module, Type, TypeKind};
use verum_common::span::Span;
use verum_smt::{
    Context as SmtContext, ContextConfig, Translator, VerificationError,
    proof_search::ProofSearchEngine,
    verification_cache::{
        CacheConfig, DistributedCacheConfig as VerifyDistributedCacheConfig,
        TrustLevel as VerifyTrustLevel, VerificationCache,
    },
};

use verum_common::{List, Map, Text, ToText};

use crate::phases::proof_verification::ProofVerificationResult;
use crate::pipeline::CompilationPipeline;
use crate::session::Session;
use crate::verification_profiler::{FileLocation, VerificationProfiler};

/// Parse a `distributed_cache_trust` string into the underlying
/// `TrustLevel`. Recognised values: `"all"`, `"signatures"` (default
/// when `None`), `"signatures_and_expiry"` (case- and whitespace-
/// tolerant). Unknown / mistyped values fall back to `Signatures`
/// with a warning so a typo can't silently downgrade trust to `All`.
fn parse_trust_level(raw: Option<&str>) -> VerifyTrustLevel {
    match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        None | Some("") | Some("signatures") => VerifyTrustLevel::Signatures,
        Some("all") => VerifyTrustLevel::All,
        Some("signatures_and_expiry") => VerifyTrustLevel::SignaturesAndExpiry,
        Some(other) => {
            warn!(
                "unknown distributed_cache_trust value '{}' — defaulting to \
                 'signatures' (the safe baseline). Accepted values: \
                 'all', 'signatures', 'signatures_and_expiry'.",
                other
            );
            VerifyTrustLevel::Signatures
        }
    }
}

/// Verification command handler
pub struct VerifyCommand<'s> {
    session: &'s mut Session,
    cache: VerificationCache,
    budget_tracker: BudgetTracker,
    profiler: Option<VerificationProfiler>,
    /// Scratch-pad for per-obligation timings accumulated during a
    /// single `verify_function` call. Drained by `verify_module`
    /// after each call into `VerificationReport::
    /// add_obligation_timings`. Interior mutability so
    /// `verify_preconditions` / `verify_postconditions` (which take
    /// `&self`) can push into it without threading an extra `&mut`
    /// argument through every helper.
    obligation_scratch: std::cell::RefCell<Vec<(Text, Duration)>>,
}

/// The SMT sort a declared type carries, and `Int` for everything else.
///
/// ONE rule, consulted by a function's SIGNATURE and by a theorem's
/// PARAMETERS. Two copies of it is how a signature and its own argument
/// came to disagree: one side spelled `Verum!AuthorityRole` and the
/// other kept the `Int` default, so the application was refused and the
/// claim could not be stated at all (T0902, T0904).
///
/// "Everything else" is the load-bearing half. `type_to_sort` answers
/// `Verum!Generic` for `List<Authority>` — a SHAPE TAG rather than a
/// type name, and one every generic shares, so two unrelated generics
/// would become the same opaque sort. `Int` is the honest answer for a
/// type nothing here knows the shape of, and it is what the goal side
/// gives such a value, so the two agree.
///
/// A DECLARED SHAPE is a record layout or a variant's constructor list
/// — the two things this command actually learns about a named type.
fn declared_sort_of(
    ty: &verum_ast::ty::Type,
    record_layouts: &std::collections::HashMap<String, std::collections::HashMap<String, (String, Option<String>)>>,
    variant_names: &[(Text, Vec<Text>)],
    positional_layouts: &std::collections::HashMap<String, Vec<(String, Option<String>)>>,
) -> (Text, Option<String>) {
    let (sort_name, type_name) = verum_smt::expr_to_smtlib::type_to_sort_and_name(ty);
    let opaque = sort_name.starts_with("Verum!");
    // A NEWTYPE is a declared type like any other. Counting only
    // records and variants as "known" meant a parameter typed
    // `Narrow` — `type Narrow is (Hash<32>)` — was declared to the
    // solver as a plain `Int`, so a reflected definition built over
    // `Verum!Narrow` could not apply to it even when it arrived.
    // Measured with `VERUM_DUMP_SMT_DIR`: the goal read
    // `(declare-fun a () Int)` for a newtype parameter (T0965).
    let known = type_name.as_deref().is_some_and(|n| {
        record_layouts.contains_key(n)
            || variant_names.iter().any(|(t, _)| t.as_str() == n)
            || positional_layouts.contains_key(n)
    });
    if !opaque || known {
        (Text::from(sort_name.as_str()), type_name)
    } else {
        (Text::from("Int"), type_name)
    }
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

        // Honour `CompilerOptions.distributed_cache_url` and
        // `distributed_cache_trust`: when a URL is configured, build a
        // `VerificationCache` that routes lookups through the
        // configured backend with the requested trust policy. Pre-fix
        // both fields landed on `CompilerOptions` (set by
        // `cli::commands::verify::verify_file_proof`) but no production
        // path consulted them — `VerifyCommand` always called
        // `VerificationCache::new()`, so configuring a distributed
        // cache in `verum.toml` had zero observable effect on the
        // verify command. The trust default mirrors the documented
        // safe baseline (`"signatures"`); unknown / mistyped values
        // fall back to `"signatures"` with a warning so a typo never
        // silently downgrades to `All`.
        let cache = Self::build_cache(session);

        Self {
            session,
            cache,
            budget_tracker: BudgetTracker::new(budget, slow_threshold),
            profiler,
            obligation_scratch: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// Build the `VerificationCache` honouring the session-level
    /// distributed-cache options. Extracted so a unit test can pin
    /// the URL→backend + trust→`TrustLevel` translation independently
    /// of the rest of `VerifyCommand` construction (which requires a
    /// full `Session`).
    fn build_cache(session: &Session) -> VerificationCache {
        let opts = session.options();
        let url = match opts.distributed_cache_url.as_deref() {
            Some(u) if !u.is_empty() => u,
            _ => return VerificationCache::new(),
        };
        let trust = parse_trust_level(opts.distributed_cache_trust.as_deref());
        let dc_config = VerifyDistributedCacheConfig {
            s3_url: Text::from(url),
            cache_dir: Text::from(".verum/verify-cache"),
            trust,
            verify_signatures: trust != VerifyTrustLevel::All,
        };
        VerificationCache::with_config(CacheConfig::default().with_distributed_cache(dc_config))
    }

    /// Record a discharged obligation's elapsed time. Called by the
    /// per-obligation verifiers; drained by `verify_module` into
    /// the report after each function's verification completes.
    fn record_obligation(&self, label: &str, elapsed: Duration) {
        self.obligation_scratch
            .borrow_mut()
            .push((Text::from(label.to_string()), elapsed));
    }

    /// Drain the scratch-pad and return the collected timings for
    /// the just-completed `verify_function` call.
    fn drain_obligation_timings(&self) -> Vec<(Text, Duration)> {
        std::mem::take(&mut *self.obligation_scratch.borrow_mut())
    }

    /// Run verification with cost reporting
    /// Load, check and verify — and answer the REPORT rather than
    /// printing it.
    ///
    /// `run` is this plus the rendering. Split because a verdict is
    /// worth asserting on: a caller that wants to know whether a
    /// theorem could be STATED had no way to ask without parsing
    /// human-facing output, and "unstateable" and "unproved" print
    /// almost the same.
    pub fn run_to_report(
        &mut self,
        function_name: Option<&str>,
    ) -> Result<VerificationReport> {
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

        // Parse and type check. The check-only pre-pass must NOT run
        // the pipeline's own SMT verification phase — this command IS
        // the verifier and re-runs every obligation below with richer
        // reporting; leaving the default mode would verify each
        // theorem twice and emit duplicate diagnostics.
        self.session.options_mut().verify_mode = crate::options::VerifyMode::Runtime;
        let mut pipeline = CompilationPipeline::new(self.session);
        pipeline.run_check_only()?;

        let module = self
            .session
            .get_module(file_id)
            .map(|m| (*m).clone())
            .ok_or_else(|| anyhow::anyhow!("Module not found after parsing"))?;

        self.verify_module(&module, function_name)
    }

    pub fn run(mut self, function_name: Option<&str>) -> Result<()> {
        let report = self.run_to_report(function_name)?;

        // --lsp-mode short-circuit: emit one LSP-formatted JSON
        // diagnostic per line on stdout, then skip the human
        // report. Callers pipe `verum verify --lsp-mode` through
        // a JSON-RPC adapter that forwards each line as a
        // `textDocument/publishDiagnostics` notification.
        if std::env::var("VERUM_LSP_MODE").is_ok() {
            self.emit_lsp_diagnostics(&report);
            return Ok(());
        }

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

        // Per-obligation breakdown — enabled by `--profile-obligation`.
        // Renders a "Slowest obligations" table from the obligation
        // timings collected by the verifier. Gated on the CLI flag so
        // the default profile report stays concise.
        if self.session.options().profile_obligation {
            self.display_obligation_breakdown(&report);
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
        let alias_map = crate::phases::proof_verification::build_refinement_alias_map(module);

        // Pre-populate a hints database with every sibling theorem / lemma /
        // corollary / axiom in this module so `apply <name>` can find them.
        // Cloned per theorem below — cheap because `LemmaHint` is small.
        let mut module_hints = verum_smt::proof_search::HintsDatabase::new();
        crate::phases::proof_verification::register_module_lemmas(module, &mut module_hints);

        // Pre-build a refinement-reflection registry so `proof by auto`
        // / `by smt` can unfold calls to user-defined pure functions.
        // Without this, a theorem like
        //  theorem double_is_2x(x: Int) ensures double_it(x) == 2 * x
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
        let mut callee_signatures_for_module: Vec<(Text, Vec<Text>, Text)> = Vec::new();
        // Module type facts for member-bearing bodies (T0843) — the
        // same env the pipeline-side reflection scan builds.
        //
        // The file's OWN declarations first, so a name it declares
        // itself wins; then every other module the check loaded, for
        // the types this one MOUNTS. Without the second half a theorem
        // parameter typed by a mounted record or variant had no
        // declared shape, kept the `Int` default while the predicate it
        // was passed to was declared over that type's own sort, and the
        // application was refused — the claim could not be stated at
        // all, which reads from the outside exactly like a claim that
        // failed to prove (T0904).
        let mut reflection_env =
            verum_smt::expr_to_smtlib::ReflectionTypeEnv::from_module(&module);
        // The MODULE REGISTRY, not the session's parsed-file cache: the
        // cache holds only the file the command was pointed at, while
        // the registry is what cross-file resolution was built against
        // and therefore holds every module the check actually loaded.
        // Measured — the cache reported ONE module for a project whose
        // mount had just resolved.
        let sibling_modules: Vec<Module> = {
            let registry = self.session.module_registry();
            let reg = registry.read();
            reg.all_modules()
                .map(|(_id, info)| info.ast.clone())
                .collect()
        };
        for other in &sibling_modules {
            reflection_env.absorb_types_from(other);
        }

        // Variant-type registry — (name, ctor-names) for every
        // `type T is A | B | C;`, from this module and every sibling,
        // this module's own first so a local name wins. Built HERE
        // rather than beside its other consumers because the signature
        // rule below needs it: what counts as a DECLARED SHAPE has to
        // be one predicate, or a signature and a theorem parameter
        // disagree about the same type.
        let mut variant_registry: Vec<(Text, Vec<Text>)> = Vec::new();
        let collect_variants = |m: &Module, out: &mut Vec<(Text, Vec<Text>)>| {
            for item in &m.items {
                if let ItemKind::Type(td) = &item.kind
                    && let verum_ast::decl::TypeDeclBody::Variant(vs) = &td.body
                {
                    let name = Text::from(td.name.name.as_str());
                    if out.iter().any(|(t, _)| *t == name) {
                        continue;
                    }
                    let ctors: Vec<Text> = vs
                        .iter()
                        .map(|v| Text::from(v.name.name.as_str()))
                        .collect();
                    out.push((name, ctors));
                }
            }
        };
        collect_variants(&module, &mut variant_registry);
        for other in &sibling_modules {
            collect_variants(other, &mut variant_registry);
        }

        let sort_of_declared = |ty: &verum_ast::ty::Type| -> Text {
            declared_sort_of(
                ty,
                &reflection_env.record_fields,
                &variant_registry,
                &reflection_env.positional,
            )
            .0
        };

        for item in &module.items {
            if let ItemKind::Function(fd) = &item.kind {
                if let Some(rf) = verum_smt::expr_to_smtlib::try_reflect_function_with_env(
                    fd,
                    &reflection_env,
                ) {
                    let _ = reflection_registry.register(rf);
                }

                // A SIGNATURE SPELLS THE SAME SORTS A THEOREM
                // PARAMETER GETS — `sort_of_declared`, above.
                //
                // This was briefly conditional on the reflector having
                // accepted the function, because giving an unreflected
                // one distinct sorts regressed an `axiom`-style
                // declaration over a non-primitive parameter and a
                // `cases` proof (T0901). Both regressions were caused by
                // the OTHER half of that change — parameters getting
                // sorts for type names nothing knew the shape of — and
                // once that was narrowed to declared shapes, the
                // condition made no difference: measured over the sort
                // ladders and all L3 proof specs, every number
                // identical. One rule beats two.
                let sort_of = &sort_of_declared;
                let param_sorts: Vec<Text> = fd
                    .params
                    .iter()
                    .filter_map(|p| {
                        if let FunctionParamKind::Regular { ty, .. } = &p.kind {
                            Some(sort_of(ty))
                        } else {
                            None
                        }
                    })
                    .collect();
                let ret_sort = match &fd.return_type {
                    // The same rule as the parameters. `Bool` is not
                    // opaque, so a Bool-returning predicate keeps its
                    // sort and the goal stays a proposition — which is
                    // the property that matters here and the one an
                    // Int default would destroy.
                    verum_common::Maybe::Some(t) => sort_of_declared(t),
                    verum_common::Maybe::None => Text::from("Int"),
                };
                callee_signatures_for_module.push((
                    Text::from(fd.name.as_str()),
                    param_sorts,
                    ret_sort,
                ));
            }
        }
        // Refinement-reflection soundness gate (T0489): a reflected
        // function whose body names a symbol the SMT-LIB block never
        // declares — a callee that is itself body-less or multi-statement,
        // hence not reflected — makes Z3's `from_string` reject the WHOLE
        // block, silently disabling every reflection axiom in the module.
        // `to_smtlib_block` now closes the registry under its call graph
        // and omits such entries; surface each omission loudly so the skip
        // is never silent.
        for skip in reflection_registry.open_entry_drops() {
            tracing::warn!(
                "refinement reflection: skipping `{}` — its body references `{}`, \
                 which is neither a parameter nor another reflected function; \
                 reflecting it would invalidate the module's entire SMT block. \
                 Other reflections are unaffected.",
                skip.name.as_str(),
                skip.missing_symbol.as_str(),
            );
        }
        // Variant disjointness axioms: for every `type T is A | B
        // | C;`, emit `T.A != T.B`, `T.A != T.C`, `T.B != T.C`.
        // These are asserted on the solver so claims like
        //  theorem variants_distinct(): T.A != T.B
        // close automatically.
        let variant_axioms = crate::phases::proof_verification::variant_disjointness_axioms(module);

        // Variant-type registry — (name, ctor-names) pairs for
        // every `type T is A | B | C;`. Threaded to each theorem
        // so the hypothesis layer can emit exhaustiveness claims.
        // The reflection registry needs the same set: a `path_K.A`
        // constant inside a reflected body is declared with `K`'s sort,
        // and the goal side gives the same constant the same sort
        // through `ProofSearchEngine::register_variant_type`. Two
        // spellings of one constant is what silenced every axiom in a
        // module that reflected a `match` (T0902).
        for (tname, _) in &variant_registry {
            reflection_registry.register_variant_type_name(tname.clone());
        }
        debug!(
            "CLI verify: siblings={} record_shapes={} variants={} \
             refinement={} signatures={} variant_axioms={}",
            sibling_modules.len(),
            reflection_env.record_fields.len(),
            variant_registry.len(),
            reflection_registry.len(),
            callee_signatures_for_module.len(),
            variant_axioms.len(),
        );

        // Contract table for BODY-OBLIGATION generation (Step 3 of
        // verify_function): every sibling's requires/ensures, so a
        // call site inside any body yields a precondition obligation
        // with the actual arguments substituted in. Before this table
        // existed, `verum verify` generated ZERO obligations from
        // function bodies — `caller() { divide(10, 0) }` against
        // `divide requires b != 0` reported Proved (T0657).
        let contract_table = verum_verification::VCGenerator::build_module_contract_table(module);

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
                let result = self.verify_function(
                    func,
                    timeout,
                    &alias_map,
                    &reflection_registry,
                    &callee_signatures_for_module,
                    &contract_table,
                    &variant_registry,
                );
                let elapsed = start_time.elapsed();

                // Drain per-obligation timings accumulated during
                // verify_function and associate them with this
                // function's entry in the report. Empty drain
                // (function had no requires/ensures instrumented)
                // is a no-op — the report's fallback view keeps
                // working.
                let obligation_timings = self.drain_obligation_timings();
                if !obligation_timings.is_empty() {
                    let mut timings_list: List<(Text, Duration)> = List::new();
                    for t in obligation_timings {
                        timings_list.push(t);
                    }
                    report.add_obligation_timings(func.name.as_str().to_text(), timings_list);
                }

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
                        // Solver/translation errors surface
                        // immediately — reporting the timeout
                        // constant here read as a 120s hang.
                        elapsed: start_time.elapsed(),
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

            debug!(
                "Verifying {} '{}' ({} requires, {} ensures)",
                kind_name,
                thm.name,
                thm.requires.len(),
                thm.ensures.len()
            );

            // Check budget before starting
            if self.budget_tracker.should_stop() {
                report.add_result(
                    format!("{} {}", kind_name, thm.name).to_text(),
                    VerificationResult::Skipped,
                );
                continue;
            }

            let result = self.verify_theorem(
                thm,
                kind_name,
                timeout,
                &alias_map,
                &module_hints,
                &reflection_registry,
                &callee_signatures_for_module,
                &reflection_env.record_fields,
                &variant_axioms,
                &variant_registry,
                &reflection_env.positional,
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

            report.add_result(format!("{} {}", kind_name, thm.name).to_text(), result);
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
        record_layouts: &std::collections::HashMap<
            String,
            std::collections::HashMap<String, (String, Option<String>)>,
        >,
        variant_axioms: &[Expr],
        variant_registry: &[(Text, Vec<Text>)],
        positional_layouts: &std::collections::HashMap<String, Vec<(String, Option<String>)>>,
    ) -> VerificationResult {
        let start = Instant::now();

        // Theorems without proof bodies are axioms - accept them
        if theorem.proof.is_none() {
            info!(
                "{} '{}' accepted as axiom (no proof body)",
                kind_name, theorem.name
            );
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

        // Honour the user-configured SMT timeout on the proof search
        // engine. `ProofSearchEngine::with_hints` defaults to 5s
        // (proof_search.rs:2349); the SmtContext above (line 548) gets
        // the configured timeout via `ContextConfig.timeout`, but the
        // proof-search engine's own internal timeout (consulted at
        // depth-bounded search entry points) silently used 5s
        // regardless. Mirror the same fix as the pipeline.rs site
        // (commit 8a80b1ea) so verify_cmd's per-theorem proof-search
        // also honours `--smt-timeout`.
        proof_engine.set_timeout(timeout);

        // Install the pre-built refinement-reflection registry so SMT
        // queries can unfold calls to user-defined pure functions.
        if !reflection_registry.is_empty() {
            proof_engine.set_reflection_registry(reflection_registry.clone());
        }

        // Register sort signatures for every module function — even
        // those without a body or those that `try_reflect_function`
        // rejected. Without this, calls to Bool-returning declared
        // functions translate as Int-UFs and goals like
        //  theorem t(): exists p: Nat. is_prime(p)
        // fail with "exists body must be a boolean expression".
        for (name, ps, r) in callee_signatures_for_module {
            proof_engine.register_callee_signature(name.clone(), ps.clone(), r.clone());
        }

        // Record layouts, from the same module scan the reflection
        // env uses. A goal that projects a record field needs the
        // field's declared sort; without it every field translated as
        // an Int constant, so a `Bool` field could not be stated as a
        // proposition at all, and the goal side named the field
        // differently from the reflection side so the two never met.
        // The theorem's own parameters. The SORT is registered for a
        // type whose SHAPE is known — one with a record layout — and the
        // TYPE NAME wherever there is one, for member access to resolve
        // a layout against.
        //
        // Narrow on purpose. Giving every unrecognised type name its own
        // sort made `theorem add_zero_right(n: Nat): n + 0 == n`
        // ill-typed, and a bare type parameter `T` broke every
        // comparison the same way; both were measured (T0901).
        //
        // A DECLARED SHAPE is a record layout or a variant's constructor
        // list — the two things this module actually knows about a named
        // type. Only records counted at first, so a theorem parameter
        // typed as a variant kept the `Int` default while the predicate
        // it was passed to was declared over the variant's own sort, and
        // the application was refused as an "unsupported argument sort".
        // The registry's `verify_accepts_exactly_agreeing_evidence`,
        // whose fifth parameter is an `AuthorityRole`, could not be
        // stated at all for that reason.
        for p in &theorem.params {
            if let verum_ast::decl::FunctionParamKind::Regular { pattern, ty, .. } = &p.kind
                && let verum_ast::pattern::PatternKind::Ident { name, .. } = &pattern.kind
            {
                // Registered unconditionally: `declared_sort_of`
                // already answers `Int` for a type whose shape is
                // unknown, and Int is what the translator would have
                // defaulted to anyway — so writing it down changes
                // nothing except that the sort now has ONE source.
                let (sort_name, type_name) =
                    declared_sort_of(ty, record_layouts, variant_registry, positional_layouts);
                proof_engine.register_value_sort(name.name.clone(), sort_name);
                if let Some(tn) = type_name {
                    proof_engine
                        .register_value_type(name.name.clone(), Text::from(tn.as_str()));
                }
            }
        }

        for (type_name, fields) in record_layouts.iter() {
            proof_engine.register_record_type(Text::from(type_name.as_str()), fields.clone());
        }
        for (type_name, positions) in positional_layouts.iter() {
            proof_engine.register_positional_type(
                Text::from(type_name.as_str()),
                positions.iter().map(|(s, _)| s.clone()).collect(),
            );
        }

        // Register variant-disjointness axioms (computed once per
        // module, passed in via `variant_axioms`). Each pair of
        // variants `T.A` / `T.B` gets an axiom `T.A != T.B` so
        //  theorem variants_distinct(): T.A != T.B
        // closes by SMT.
        for ax in variant_axioms {
            proof_engine.register_axiom(ax.clone());
        }

        // Register the variant-type registry so the hypothesis-
        // elaboration pass can emit exhaustiveness claims for
        // parameters typed as variants. Data is passed in as a
        // pre-computed (type-name, ctors) list.
        for (tname, ctors) in variant_registry {
            proof_engine.register_variant_type(tname.clone(), ctors.clone());
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
                    if has_incomplete {
                        " [incomplete: uses admit/sorry]"
                    } else {
                        ""
                    }
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
                            first
                                .suggestions
                                .iter()
                                .next()
                                .map(|s| s.as_str())
                                .unwrap_or("")
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
        reflection_registry: &verum_smt::refinement_reflection::RefinementReflectionRegistry,
        callee_signatures_for_module: &[(Text, Vec<Text>, Text)],
        contract_table: &verum_verification::SymbolTable,
        variant_registry: &[(Text, Vec<Text>)],
    ) -> Result<verum_smt::ProofResult, VerificationError> {
        let start = Instant::now();

        // Check if function has any verifiable content.
        // The alias-map catches refinements that arrive through a named
        // type alias (`type PageNo is Int where |n| { n >= 1 };`) so
        // functions taking `p: PageNo` get the implicit `n >= 1`
        // precondition without repeating it in a `requires` clause.
        let has_requires = !func.requires.is_empty();
        let has_ensures = !func.ensures.is_empty();
        let has_refined_params = self.has_refinement_types_in_params_with_aliases(func, alias_map);
        let has_refined_return =
            self.has_refinement_type_with_aliases(&func.return_type, alias_map);

        // Synthesise implicit `requires` clauses from alias-wrapped
        // refinements on parameters. For `fn foo(p: PageNo)` where
        // `type PageNo is Int where |n| { n >= 1 }`, this adds an
        // expression equivalent to `p >= 1` to the requires set.
        let implicit_requires = self.synthesize_alias_refinement_requires(func, alias_map);
        let has_implicit_requires = !implicit_requires.is_empty();

        // Certified strategy: run the proof kernel's K-rule recheck
        // over the function's refinement surface BEFORE any SMT.
        // A kernel formation error is a hard failure no SMT proof
        // can recover (same short-circuit contract as
        // `SmtVerificationPass`). (T0671)
        let strategy = self.session.options().verify_strategy;
        if strategy.kernel_recheck() {
            for (label, outcome) in
                verum_verification::KernelRecheck::recheck_function(func).iter()
            {
                if let Err(err) = outcome {
                    return Err(VerificationError::CannotProve {
                        constraint: Text::from(format!(
                            "kernel recheck rejected {}: {}",
                            label.as_str(),
                            err
                        )),
                        counterexample: None,
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

        // Body obligations (Step 3): wp-woven call-site
        // preconditions, division guards, bounds checks, and the
        // loop-invariant family. Generated for EVERY function —
        // a caller with no contracts of its own still has to honour
        // its callees' preconditions; the pre-fix short-circuit
        // below returned "no_verification"-Proved without ever
        // looking at the body, which is how `divide(10, 0)` against
        // `requires b != 0` verified (T0657). Generation is pure
        // formula construction (no solver), so doing it before the
        // short-circuit costs nothing measurable.
        //
        // Under `--mode=thorough|certified` termination obligations
        // are MANDATORY for every loop (T0671); per-function
        // `@verify(thorough|certified)` attributes force the same
        // inside the generator regardless of the session strategy.
        let mut body_vcgen = verum_verification::VCGenerator::new()
            .with_mandatory_termination(strategy.mandatory_termination())
            .with_symbol_table(contract_table.clone());
        let body_vcs = body_vcgen.generate_body_obligation_vcs(func);
        let has_body_obligations = !body_vcs.is_empty();

        if !has_requires
            && !has_ensures
            && !has_refined_params
            && !has_refined_return
            && !has_implicit_requires
            && !has_body_obligations
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
            for e in &func.requires {
                list.push(e.clone());
            }
            for e in &implicit_requires {
                list.push(e.clone());
            }
            for e in &inline_refinement_requires {
                list.push(e.clone());
            }
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

        // THE SAME VARIANTS THE OTHER TWO REGISTRIES GET.
        //
        // `variant_registry` is already pushed into the reflection
        // registry and into `ProofSearchEngine`. This translator — the
        // one that binds `result` to the function body — was the third
        // consumer and nobody told it, so a variant CONSTANT fell to the
        // Int default while a variant-typed PARAMETER correctly carried
        // its opaque sort. The arm test a `match` builds is an equality
        // between exactly those two:
        //
        //     incompatible types for binary operation ==
        //         (left is Verum!K, right is Int)
        //
        // The equality could not be built, so `translate_match` failed,
        // so the body→result binding was skipped, so `result` was a free
        // constant and every refined return over a `match` reported a
        // counterexample that named no cause (T0914). Three registries
        // for one fact; two of them were fed.
        for (tname, ctors) in variant_registry {
            let ctor_names: Vec<String> =
                ctors.iter().map(|c| c.as_str().to_string()).collect();
            translator.register_variant_type(tname.as_str(), ctor_names);
        }

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

        // Reflection registry + module-level callee signatures wired
        // identically to the theorem-path (verify_theorem above).
        //

        // Theorem-path (`ProofSearchEngine::set_reflection_registry`
        // + `register_callee_signature`) routes these into the same
        // translator API. Function-path calls the translator API
        // directly — no engine wrapper is needed because function
        // verification is a single SMT query, not a structured proof.
        //

        // For every `@logic` function in the module, the translator
        // now knows both the sort signature AND the SMT-LIB defining
        // block, so calls like `safe_div(a, b)` unfold their body
        // during solve — instead of becoming an opaque Int-returning
        // UF symbol that defeats any non-trivial postcondition.
        //

        // Body-less or unreflected functions still get their sort
        // signature registered, so Bool-returning declared-only
        // functions (e.g. `fn is_prime(n: Int) -> Bool;`) translate
        // to Bool-sorted UFs instead of the Int default, which was
        // making `exists n: Nat. is_prime(n)`-style ensures clauses
        // fail with "exists body must be a boolean expression."
        //

        // Pinned by:
        //  vcs/specs/L1-core/verification_phase/reflection_function_level.vr
        for rf in reflection_registry.iter() {
            let param_sorts: Vec<String> = rf
                .parameter_sorts
                .iter()
                .map(|s| s.as_str().to_string())
                .collect();
            translator.register_callee_signature(
                rf.name.as_str(),
                param_sorts,
                rf.return_sort.as_str().to_string(),
            );
        }
        for (name, ps, r) in callee_signatures_for_module {
            let param_sorts: Vec<String> = ps.iter().map(|s| s.as_str().to_string()).collect();
            translator.register_callee_signature(
                name.as_str(),
                param_sorts,
                r.as_str().to_string(),
            );
        }

        // Step 1: Verify preconditions are satisfiable (not contradictory)
        if has_requires || has_implicit_requires {
            let pre_start = Instant::now();
            let pre_result =
                self.verify_preconditions(&ctx, &mut translator, &effective_requires, timeout);
            self.record_obligation("precondition", pre_start.elapsed());
            if let Err(e) = pre_result {
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

        // Return-type refinement → implicit `ensures` (T0678). The
        // validity claim of `fn foo(n: Int) -> Int{P} { body }` is
        // "the RETURNED VALUE satisfies P" — exactly a postcondition
        // with the predicate's binder rewritten to `result`. A
        // previous revision left this as a comment asserting the
        // synthesis had happened elsewhere; nothing synthesized it,
        // so a refined-return-only function skipped Step 1 (no
        // requires), skipped Step 2 (no ensures), and returned
        // "Proved in 0.00s" unconditionally. Constant predicates were
        // saved upstream by the type checker's static evaluation
        // (E500), which is why exactly the parameter-referencing
        // predicates — the ones only SMT can decide — escaped as
        // false "Proved". The check pipeline rejected the same
        // programs, inverting the trust order between `check` and
        // `verify`.
        let implicit_return_ensures =
            self.synthesize_return_refinement_ensures(&func.return_type, alias_map);

        // Step 2: Verify postconditions hold given preconditions. Also
        // pass the function body so `result` gets a proper Z3 binding:
        // for expression-body / block-with-tail-expr functions we assert
        // `result == body` so the SMT can check ensures against the actual
        // return value rather than an unconstrained fresh variable.
        //
        // Declared `ensures` and the return-refinement's implicit
        // ensures run through the SAME pipeline — the body→result
        // binding is what makes the obligation non-vacuous.
        let effective_ensures: Vec<Expr> = func
            .ensures
            .iter()
            .cloned()
            .chain(implicit_return_ensures.iter().cloned())
            .collect();
        if !effective_ensures.is_empty() {
            // Per-ensures timings are recorded inside
            // verify_postconditions (one row per clause),
            // so no aggregate "postcondition" record here —
            // that would double-count when a function has
            // multiple ensures clauses.
            // Bind `result` in the TRANSLATOR with the sort of the
            // declared return type (Refined unwraps to its base) so
            // the ensures-side `result` and the solver-side
            // body-equality constant agree. Without this, `result`
            // fell to the translator's Int default and every
            // Float-returning refined function died in translation
            // ("incompatible types for binary operation").
            if let Some(ret_ty) = &func.return_type {
                let base_ty: &Type = match &ret_ty.kind {
                    TypeKind::Refined { base, .. } => base,
                    _ => ret_ty,
                };
                if let Ok(z3_result) = translator.create_var("result", base_ty) {
                    translator.bind(Text::from("result"), z3_result);
                }
            }

            let post_result = self.verify_postconditions(
                &ctx,
                &mut translator,
                &effective_requires,
                &effective_ensures,
                func.body.as_ref(),
                timeout,
                reflection_registry,
            );
            match post_result {
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

        // Step 3: discharge the body obligations generated above —
        // `P => wp(body, true)` with every call-site precondition,
        // division guard and bounds check woven in as a labeled
        // conjunct, plus the loop-invariant side family. One solver
        // query per VC; a Sat model is re-evaluated per label so the
        // failure names the exact obligation, not just "the body".
        if has_body_obligations {
            let step3_start = Instant::now();
            let step3 =
                self.verify_body_obligations(&body_vcgen, &body_vcs, func, timeout, start);
            self.record_obligation("body obligations", step3_start.elapsed());
            step3?;
            debug!("Body obligations verified for {}", func.name);
        }

        // All checks passed - create proof result with cost tracking
        let cost = verum_smt::VerificationCost::new(
            func.name.as_str().into(),
            start.elapsed(),
            true, // succeeded
        );

        Ok(verum_smt::ProofResult::new(cost))
    }

    /// Step 3 worker: discharge wp-generated body-obligation VCs
    /// through the Hoare Z3 backend, translating a failure into the
    /// same `CannotProve`/`Timeout` surface Steps 1-2 use.
    fn verify_body_obligations(
        &self,
        body_vcgen: &verum_verification::VCGenerator,
        body_vcs: &verum_common::List<verum_verification::VerificationCondition>,
        func: &FunctionDecl,
        timeout: Duration,
        fn_start: Instant,
    ) -> Result<(), VerificationError> {
        // A fresh context per function keeps solver state disjoint
        // from the Steps 1-2 context (different translator, different
        // variable universe).
        let config = ContextConfig {
            timeout: Some(timeout),
            ..Default::default()
        };
        let body_ctx = SmtContext::with_config(config);
        let verifier = verum_verification::HoareZ3Verifier::new(&body_ctx)
            .with_timeout(timeout.as_millis().min(u32::MAX as u128) as u32);

        for vc in body_vcs.iter() {
            // Trivial formulas were already skipped at generation;
            // simplify() here catches the loop-family VCs that are
            // pushed unsimplified.
            let formula = vc.formula.simplify();
            if formula == verum_verification::Formula::True {
                continue;
            }

            let outcome = match verifier.verify_labeled_formula(&formula) {
                Ok(o) => o,
                Err(verum_verification::WPError::Unknown { .. }) => {
                    return Err(VerificationError::Timeout {
                        constraint: Text::from(format!(
                            "{}: {}",
                            func.name.as_str(),
                            vc.description.as_str()
                        )),
                        timeout,
                        cost: verum_smt::VerificationCost::new(
                            func.name.as_str().into(),
                            fn_start.elapsed(),
                            false,
                        )
                        .with_timeout(),
                    });
                }
                Err(e) => {
                    return Err(VerificationError::SolverError(Text::from(format!(
                        "body obligation '{}': {}",
                        vc.description.as_str(),
                        e
                    ))));
                }
            };

            if outcome.valid {
                continue;
            }

            // Blame: name the exact violated obligations when the
            // model pins them; fall back to the VC description.
            let mut blamed: Vec<Text> = Vec::new();
            for label in &outcome.failed_labels {
                if let Some(meta) = body_vcgen.obligation_meta(*label) {
                    blamed.push(Text::from(format!(
                        "{} ({})",
                        meta.message.as_str(),
                        meta.kind.description()
                    )));
                }
            }
            let constraint = if blamed.is_empty() {
                Text::from(vc.description.as_str())
            } else {
                Text::from(blamed.join("; "))
            };

            // Model assignment → structured counterexample, minus
            // the translator's internal fresh terms.
            let mut assignments: verum_common::Map<Text, verum_smt::CounterExampleValue> =
                verum_common::Map::new();
            for (name, value) in outcome.counterexample.iter() {
                if name.as_str().starts_with("__vc_") {
                    continue;
                }
                let as_str = value.as_str();
                let typed = if let Ok(i) = as_str.parse::<i64>() {
                    verum_smt::CounterExampleValue::Int(i)
                } else if as_str == "true" {
                    verum_smt::CounterExampleValue::Bool(true)
                } else if as_str == "false" {
                    verum_smt::CounterExampleValue::Bool(false)
                } else if let Ok(f) = as_str.parse::<f64>() {
                    verum_smt::CounterExampleValue::Float(f)
                } else {
                    verum_smt::CounterExampleValue::Unknown(value.clone())
                };
                assignments.insert(name.clone(), typed);
            }
            // The constraint text already names the violated
            // obligation ("call to 'divide' requires b != 0"); the
            // display path renders it after "Violates:", so no
            // extra prefix here.
            let counterexample =
                verum_smt::CounterExample::new(assignments, constraint.clone());

            return Err(VerificationError::CannotProve {
                constraint,
                counterexample: Some(counterexample),
                cost: verum_smt::VerificationCost::new(
                    func.name.as_str().into(),
                    fn_start.elapsed(),
                    false,
                ),
                suggestions: List::new(),
            });
        }

        Ok(())
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
        if self.has_refinement_types_in_params(func) {
            return true;
        }
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
            TypeKind::Path(path) => path
                .as_ident()
                .map(|id| alias_map.contains_key(&id.name))
                .unwrap_or(false),
            TypeKind::Generic { base, args } => {
                self.type_has_refinement_with_aliases(base, alias_map)
                    || args.iter().any(|arg| {
                        if let verum_ast::ty::GenericArg::Type(t) = arg {
                            self.type_has_refinement_with_aliases(t, alias_map)
                        } else {
                            false
                        }
                    })
            }
            TypeKind::Tuple(types) => types
                .iter()
                .any(|t| self.type_has_refinement_with_aliases(t, alias_map)),
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. } => {
                self.type_has_refinement_with_aliases(inner, alias_map)
            }
            TypeKind::Function {
                params,
                return_type,
                ..
            } => {
                params
                    .iter()
                    .any(|t| self.type_has_refinement_with_aliases(t, alias_map))
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
            let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind else {
                continue;
            };
            let Some(param_name) = self.extract_param_name(pattern) else {
                continue;
            };
            // Follow the alias chain on the declared type.
            let alias_name = match &ty.kind {
                TypeKind::Path(p) => p.as_ident().map(|id| id.name.clone()),
                _ => None,
            };
            let Some(alias_name) = alias_name else {
                continue;
            };
            let Some(preds) = alias_map.get(&alias_name) else {
                continue;
            };
            for pred in preds {
                let substituted = substitute_ident(
                    pred,
                    &[(
                        Text::from("self"),
                        verum_ast::ty::Ident::new(param_name.as_str(), pred.span),
                    )],
                );
                out.push(substituted);
            }
        }
        out
    }

    /// Synthesize the implicit `ensures` clauses carried by a refined
    /// return type (T0678). `fn f(n: Int) -> Int{== n + 1000}` means
    /// `ensures result == n + 1000`; the predicate's binder — the
    /// implicit `it` (the parser's desugar target for leading-operator
    /// forms), the legacy `self` spelling, or an explicit `|x|`
    /// binding — is rewritten to `result` so the postcondition
    /// pipeline's body→result binding makes the obligation
    /// non-vacuous. Alias-wrapped returns (`-> PageNo` where the alias
    /// chain carries predicates) contribute their flattened
    /// predicates the same way.
    fn synthesize_return_refinement_ensures(
        &self,
        return_type: &Option<Type>,
        alias_map: &std::collections::HashMap<Text, Vec<Expr>>,
    ) -> Vec<Expr> {
        synthesize_return_refinement_ensures_impl(return_type, alias_map)
    }

}

/// Pure core of [`VerifyCommand::synthesize_return_refinement_ensures`]
/// (T0678) — a free function so the binder-rewrite contract is unit-
/// testable without constructing a compilation session.
pub(crate) fn synthesize_return_refinement_ensures_impl(
    return_type: &Option<Type>,
    alias_map: &std::collections::HashMap<Text, Vec<Expr>>,
) -> Vec<Expr> {
    use crate::phases::proof_verification::substitute_ident;
    let Some(ret_ty) = return_type else {
        return Vec::new();
    };
        let mut out: Vec<Expr> = Vec::new();
        match &ret_ty.kind {
            TypeKind::Refined { base: _, predicate } => {
                let result_ident = |span| verum_ast::ty::Ident::new("result", span);
                let substitutions: Vec<(Text, verum_ast::ty::Ident)> = match &predicate.binding {
                    Some(binder) => vec![(
                        binder.name.clone(),
                        result_ident(predicate.expr.span),
                    )],
                    None => vec![
                        (Text::from("it"), result_ident(predicate.expr.span)),
                        (Text::from("self"), result_ident(predicate.expr.span)),
                    ],
                };
                out.push(substitute_ident(&predicate.expr, &substitutions));
            }
            TypeKind::Path(path) => {
                if let Some(id) = path.as_ident()
                    && let Some(preds) = alias_map.get(&id.name)
                {
                    for pred in preds {
                        let substitutions = vec![
                            (
                                Text::from("it"),
                                verum_ast::ty::Ident::new("result", pred.span),
                            ),
                            (
                                Text::from("self"),
                                verum_ast::ty::Ident::new("result", pred.span),
                            ),
                        ];
                        out.push(substitute_ident(pred, &substitutions));
                    }
                }
            }
            _ => {}
        }
        out
}

impl<'s> VerifyCommand<'s> {
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
        reflection_registry: &verum_smt::refinement_reflection::RefinementReflectionRegistry,
    ) -> Result<(), VerifyError> {
        if ensures.is_empty() {
            return Ok(());
        }

        let solver = ctx.solver();

        // Inject the refinement-reflection SMT-LIB block BEFORE any
        // assertions. The block defines every `@logic` function in
        // the module as `declare-fun` + `forall` axiom pair. With it
        // in scope, Z3 can unfold a call like `prime(n)` into its
        // boolean body during proof search rather than leaving it
        // opaque. Mirrors the theorem-path wiring at
        // `proof_search.rs::apply_decision_procedure` line ~4529.
        if !reflection_registry.is_empty() {
            let block = reflection_registry.to_smtlib_block();
            solver.from_string(block.as_str().to_string());
        }

        // Assert preconditions as assumptions
        for req in requires {
            if let Ok(z3_expr) = translator.translate_expr(req) {
                if let Some(bool_expr) = z3_expr.as_bool() {
                    solver.assert(&bool_expr);
                }
            }
        }

        if std::env::var("VERUM_TRACE_RESULT_BIND").is_ok() {
            eprintln!(
                "[result-bind] enter: body_present={} ensures={}",
                matches!(body, verum_common::Maybe::Some(_)),
                ensures.len(),
            );
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
            use verum_ast::pattern::PatternKind;
            use verum_ast::stmt::{Stmt, StmtKind};

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
            let folded_body: Option<Expr>;
            let mut has_semi_stmt = false;

            match b {
                FunctionBody::Expr(e) => {
                    tail_expr = Some(e);
                }
                FunctionBody::Block(blk) => {
                    let last_index = blk.stmts.len().saturating_sub(1);
                    for (i, stmt) in blk.stmts.iter().enumerate() {
                        // The only statement whose value the block
                        // RETURNS is a final one without a semicolon,
                        // and only when the block states no `expr` of
                        // its own. Everything else runs for effect,
                        // semicolon or not — `while c { … }` in the
                        // middle of a block carries no semicolon and
                        // changes everything after it.
                        //
                        // Measured: without this distinction a body
                        // whose loop the fold could not decide fell
                        // back to the bare tail, the solver still held
                        // `acc == 0` from the `let`, and
                        // `ensures result == 0` PROVED for a function
                        // that never returns. That is the T0954 hole
                        // reopening through the no-semicolon door, and
                        // the negative spec written for the loop work
                        // is what caught it.
                        let is_the_tail = i == last_index
                            && matches!(blk.expr, verum_common::Maybe::None)
                            && matches!(&stmt.kind, StmtKind::Expr { has_semi: false, .. });
                        match &stmt.kind {
                            StmtKind::Let {
                                pattern,
                                value: verum_common::Maybe::Some(val),
                                ..
                            } => {
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
                            StmtKind::Expr { .. } => {
                                // No VALUE is not the same as no EFFECT.
                                // `acc = 1;`, `xs.push(v);` and a
                                // `while` in the middle of a block all
                                // change what the tail evaluates to.
                                // Skipping them is only sound when the
                                // fold below models them; record that
                                // one is present and let that decide.
                                //
                                // The one exception is the block's own
                                // tail, which is a value rather than an
                                // effect and is read below.
                                if !is_the_tail {
                                    has_semi_stmt = true;
                                }
                            }
                            _ => {
                                safe_encoding = false;
                                break;
                            }
                        }
                    }
                    if safe_encoding {
                        // A block states its tail in one of two shapes: its
                        // own `expr`, or a final statement without a
                        // semicolon. They mean the same thing, so they are
                        // reduced to one pair here — the statements that
                        // run, and the expression whose value is returned —
                        // and everything below sees a single shape. Keeping
                        // two paths is how one of them acquires a fix the
                        // other does not.
                        let (body_stmts, body_tail) = match &blk.expr {
                            verum_common::Maybe::Some(boxed) => {
                                (blk.stmts.as_slice(), Some(boxed.as_ref()))
                            }
                            verum_common::Maybe::None => match blk.stmts.split_last() {
                                Some((
                                    Stmt {
                                        kind:
                                            StmtKind::Expr {
                                                expr,
                                                has_semi: false,
                                            },
                                        ..
                                    },
                                    rest,
                                )) => (rest, Some(expr)),
                                _ => (blk.stmts.as_slice(), None),
                            },
                        };
                        if let Some(boxed) = body_tail {
                            // FOLD the block, do not read its tail alone.
                            //
                            // The walk above asserts each `let` into the
                            // solver, which is right until a statement
                            // CHANGES one: `let mut acc = 0; acc = 1; acc`
                            // told the solver `acc == 0`, said nothing about
                            // the assignment, and then bound `result` to the
                            // bare tail `acc` — so `ensures result == 1`
                            // came back with the counterexample
                            // `acc = 0, result = 0`, the function's own body
                            // arguing against its own contract. Every
                            // imperative body was unverifiable for this
                            // reason, and the registry showcase attributed
                            // it to loops, which are only the loudest case.
                            //
                            // `fold_block_to_expr` is the same answer
                            // reflection uses, so the two cannot drift.
                            // It returns `None` for anything the
                            // substitution model does not represent, and
                            // then the bare tail is used exactly as before.
                            folded_body =
                                verum_smt::expr_to_smtlib::fold_block_to_expr(body_stmts, boxed);
                            tail_expr = match &folded_body {
                                Some(f) => Some(f),
                                // The fold declined. Reading the bare tail
                                // is only correct when nothing between the
                                // `let`s and the tail changed anything —
                                // otherwise the solver holds `acc == 0`
                                // from a `let`, hears nothing of `acc = 1;`
                                // and PROVES `result == 0` for a function
                                // that returns 1. A false postcondition is
                                // worse than an unproved one, so when the
                                // body has a statement the fold could not
                                // model, `result` is left free and the
                                // obligation simply does not discharge.
                                //
                                // A block of nothing but `let`s and a tail
                                // still encodes as before: the fold can
                                // decline it for length alone, and there
                                // is no effect to have missed.
                                None if has_semi_stmt => None,
                                None => Some(boxed),
                            };
                        }
                    } else {
                        tail_expr = None;
                    }
                }
            }

            if let Some(e) = tail_expr {
                // THE REASON, RESTORED. This binding is what makes a
                // postcondition non-vacuous, and when the body cannot be
                // translated the failure is silent: `result` stays a free
                // constant and every refined return reports a
                // counterexample that names no cause. A function whose
                // body is `match s { K.A => 0, K.B => 1 }` failed
                // `Int{it >= 0}` with `result = (- 1)` for exactly this
                // reason, and nothing said so (T0914).
                let translated = translator.translate_expr(e);
                if let Err(why) = &translated {
                    // ALWAYS AUDIBLE UNDER THE TRACE FLAG. This was a
                    // `tracing::debug!` and printed nothing, because the
                    // verify command installs no subscriber — so the one
                    // reason that explains the whole failure was written
                    // to a channel nobody reads.
                    if std::env::var("VERUM_TRACE_RESULT_BIND").is_ok() {
                        eprintln!(
                            "[result-bind] SKIPPED: body did not translate: {:?}",
                            why
                        );
                    }
                    tracing::debug!(
                        "result binding skipped: body did not translate: {:?}",
                        why
                    );
                }
                if let Ok(body_z3) = translated {
                    if std::env::var("VERUM_TRACE_RESULT_BIND").is_ok() {
                        eprintln!(
                            "[result-bind] int={} bool={} real={}",
                            body_z3.as_int().is_some(),
                            body_z3.as_bool().is_some(),
                            body_z3.as_real().is_some(),
                        );
                        eprintln!("[result-bind] body ast={}", body_z3);
                    }
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
        for (ens_idx, ens) in ensures.iter().enumerate() {
            let ens_start = Instant::now();
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
                                    format!("Postcondition violation\n{}", ce_summary.as_str())
                                        .to_text(),
                                    ce_opt,
                                ));
                            }
                            z3::SatResult::Unsat => {
                                // No counterexample - postcondition holds
                                solver.pop(1);
                            }
                            z3::SatResult::Unknown => {
                                // Z3 can't decide within its budget —
                                // delegate to CVC5, which has a
                                // complementary decision portfolio
                                // (stronger on quantifiers, strings,
                                // nonlinear arithmetic). Same
                                // convention: `¬ensures` unsat means
                                // the ensures holds. If CVC5 also
                                // returns Unknown we surface the
                                // timeout as before.
                                let cvc5_result = self.cvc5_discharge_negated(ens, requires);
                                solver.pop(1);
                                match cvc5_result {
                                    Cvc5Outcome::Valid => {
                                        // ensures holds per CVC5
                                    }
                                    Cvc5Outcome::Invalid => {
                                        return Err(VerifyError::Failed(
                                            Text::from(format!(
                                                "Postcondition violation (CVC5 found \
                                                 counterexample for negated ensures)"
                                            )),
                                            None,
                                        ));
                                    }
                                    Cvc5Outcome::Unknown => {
                                        return Err(VerifyError::Timeout);
                                    }
                                }
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
            // Per-ensures timing — each clause lands as its own
            // obligation row in the `--profile-obligation`
            // breakdown. Label is `ensures[i]` so multi-clause
            // functions surface which specific clause dominates.
            self.record_obligation(&format!("ensures[{}]", ens_idx), ens_start.elapsed());
        }

        Ok(())
    }

    /// Outcome of a CVC5 fallback check for a function ensures.
    /// Same trichotomy Z3 uses: Valid (ensures holds), Invalid
    /// (counterexample), Unknown (solver can't decide).
    fn cvc5_discharge_negated(&self, ensures: &Expr, requires: &[Expr]) -> Cvc5Outcome {
        use verum_smt::cvc5_backend::{Cvc5Backend, Cvc5Config, SatResult};
        let mut cvc5 = match Cvc5Backend::new(Cvc5Config::default()) {
            Ok(b) => b,
            Err(_) => return Cvc5Outcome::Unknown,
        };
        // Assert preconditions, then the negated postcondition.
        for req in requires {
            if cvc5.assert_formula_from_expr(req).is_err() {
                return Cvc5Outcome::Unknown;
            }
        }
        let neg = Expr::new(
            verum_ast::ExprKind::Unary {
                op: verum_ast::UnOp::Not,
                expr: verum_common::Heap::new(ensures.clone()),
            },
            ensures.span,
        );
        if cvc5.assert_formula_from_expr(&neg).is_err() {
            return Cvc5Outcome::Unknown;
        }
        match cvc5.check_sat() {
            Ok(SatResult::Unsat) => Cvc5Outcome::Valid,
            Ok(SatResult::Sat) => Cvc5Outcome::Invalid,
            _ => Cvc5Outcome::Unknown,
        }
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
                        // The formatted counterexample is a multi-line
                        // block that already opens with its own
                        // "Counterexample:" header — indent it as-is
                        // rather than prefixing a second header.
                        for line in ce.as_str().lines() {
                            if line.is_empty() {
                                println!();
                            } else {
                                println!("      {}", line.yellow());
                            }
                        }
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

    /// Render the per-obligation breakdown ("Slowest obligations").
    ///
    /// Sorts every verified function by its elapsed time descending and
    /// prints the top 10 as a ranked table. At current instrumentation
    /// obligation-level timing is not yet collected separately from
    /// function-level (each `VerificationResult::Proved { elapsed }`
    /// aggregates every obligation discharged inside that function), so
    /// this rendering is the function-granularity view — it will
    /// transparently upgrade to true per-obligation rows once the
    /// verifier instruments individual obligation discharges. That
    /// upgrade is a pure collection change; the display surface here
    /// does not need to move.
    fn display_obligation_breakdown(&self, report: &VerificationReport) {
        println!("{}", "\nSlowest obligations:".bold());
        println!("{}", "=".repeat(60));

        // When obligation-level instrumentation is available,
        // render true per-obligation rows (function.obligation,
        // one row per discharge). Otherwise fall back to the
        // function-granular view (one row per function).
        let rows: Vec<(Text, Duration)> = if report.total_obligation_count() > 0 {
            self.collect_per_obligation_rows(report)
        } else {
            self.collect_per_function_rows(report)
        };

        if rows.is_empty() {
            println!("  (no obligations discharged in this run)");
            println!();
            return;
        }

        let mut sorted = rows;
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        let take = sorted.len().min(10);

        println!(
            "  {:<40} {:>12} {:>10}",
            "obligation", "time (ms)", "share %"
        );
        println!("  {}", "-".repeat(64));

        let total_ms: f64 = sorted.iter().map(|(_, d)| d.as_secs_f64() * 1000.0).sum();

        for (name, elapsed) in sorted.iter().take(take) {
            let ms = elapsed.as_secs_f64() * 1000.0;
            let share = if total_ms > 0.0 {
                100.0 * ms / total_ms
            } else {
                0.0
            };
            println!("  {:<40} {:>12.1} {:>9.1}%", name.as_str(), ms, share);
        }

        if sorted.len() > take {
            println!(
                "  (… {} more obligations omitted; pass --export to dump full list)",
                sorted.len() - take
            );
        }

        println!();
    }

    /// Collect rows from instrumented per-obligation timings.
    /// Row label is `function.obligation` so the renderer can
    /// tell which function each obligation came from.
    fn collect_per_obligation_rows(&self, report: &VerificationReport) -> Vec<(Text, Duration)> {
        let mut rows = Vec::new();
        for (fn_name, timings) in &report.obligation_timings {
            for (label, elapsed) in timings {
                let composite = Text::from(format!("{}.{}", fn_name.as_str(), label.as_str()));
                rows.push((composite, *elapsed));
            }
        }
        rows
    }

    /// Collect rows from function-granular aggregate timings.
    /// Used when obligation-level instrumentation is
    /// unavailable. One row per function.
    fn collect_per_function_rows(&self, report: &VerificationReport) -> Vec<(Text, Duration)> {
        let mut rows = Vec::new();
        for (name, result) in &report.results {
            let elapsed = match result {
                VerificationResult::Proved { elapsed } => Some(*elapsed),
                VerificationResult::Failed { elapsed, .. } => Some(*elapsed),
                VerificationResult::Timeout { elapsed, .. } => Some(*elapsed),
                VerificationResult::Skipped => None,
            };
            if let Some(e) = elapsed {
                rows.push((name.clone(), e));
            }
        }
        rows
    }

    /// Emit newline-delimited JSON LSP-format diagnostics for every
    /// failed / timed-out verification result. Gated on
    /// `VERUM_LSP_MODE=1`. Each line is a single
    /// `textDocument/publishDiagnostics` payload that an external
    /// JSON-RPC adapter can forward directly to the IDE.
    ///
    /// Schema per line:
    ///
    ///  { "function": "<name>",
    ///  "severity": "error" | "warning",
    ///  "message": "<one-line summary>",
    ///  "elapsed_ms": <number>,
    ///  "counterexample": "<string>" | null }
    ///
    /// We deliberately emit a Verum-flavored JSON object rather than a
    /// verbatim LSP `Diagnostic` — the LSP shape needs `range`
    /// (start/end line+column), which the verifier doesn't track at
    /// function granularity. The adapter responsible for piping
    /// through JSON-RPC attaches the range from the LSP document's
    /// symbol index using the `function` field as the lookup key.
    fn emit_lsp_diagnostics(&self, report: &VerificationReport) {
        for (name, result) in &report.results {
            let (severity, message, elapsed_ms, counterexample): (
                &str,
                String,
                f64,
                Option<String>,
            ) = match result {
                VerificationResult::Proved { elapsed } => (
                    "info",
                    "proved".to_string(),
                    elapsed.as_secs_f64() * 1000.0,
                    None,
                ),
                VerificationResult::Failed {
                    elapsed,
                    counterexample,
                } => (
                    "error",
                    "verification failed".to_string(),
                    elapsed.as_secs_f64() * 1000.0,
                    counterexample.as_ref().map(|t| t.as_str().to_string()),
                ),
                VerificationResult::Timeout { elapsed, timeout } => (
                    "warning",
                    format!(
                        "verification timed out after {:.1}s (budget: {:.1}s)",
                        elapsed.as_secs_f64(),
                        timeout.as_secs_f64()
                    ),
                    elapsed.as_secs_f64() * 1000.0,
                    None,
                ),
                VerificationResult::Skipped => continue,
            };

            // Hand-roll the JSON to avoid dragging the full
            // serde_json dependency graph into the LSP emission
            // path. Fields are simple strings + numbers; escape
            // quotes and backslashes.
            let ce_json: String = match counterexample {
                Some(ref ce) => format!("\"{}\"", Self::json_escape(ce.as_str())),
                None => "null".to_string(),
            };
            println!(
                "{{\"function\":\"{}\",\"severity\":\"{}\",\"message\":\"{}\",\"elapsed_ms\":{:.1},\"counterexample\":{}}}",
                Self::json_escape(name.as_str()),
                severity,
                Self::json_escape(&message),
                elapsed_ms,
                ce_json
            );
        }
    }

    /// JSON-escape a string. Handles the minimum set of characters
    /// the JSON spec requires: `"`, `\`, newline, tab. Extended
    /// Unicode escapes would be overkill for our diagnostic payload.
    fn json_escape(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for ch in s.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => out.push(c),
            }
        }
        out
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
    /// Optional per-obligation timings keyed by function name.
    ///
    /// Populated when the verifier has obligation-level
    /// instrumentation available (currently: none of the
    /// in-tree verify paths — this is the slot a future
    /// instrumentation patch writes into). Empty map means
    /// "only function-granular timing is available" — the
    /// `--profile-obligation` renderer falls back to
    /// aggregate-per-function rows in that case.
    ///
    /// The key is the function name (matching `results`'s
    /// first tuple element); the value is
    /// `[(obligation_label, elapsed), …]` in order of
    /// discharge.
    obligation_timings: List<(Text, List<(Text, Duration)>)>,
}

impl VerificationReport {
    /// Create a new empty verification report
    pub fn new() -> Self {
        Self {
            obligation_timings: List::new(),
            results: List::new(),
            start_time: Instant::now(),
        }
    }

    /// Add a verification result for a function
    /// The names of every obligation that did not verify — whatever
    /// the reason. A caller asserting that a claim is STATEABLE reads
    /// this: it must not contain the claim's name.
    pub fn failed_names(&self) -> Vec<&str> {
        self.results
            .iter()
            .filter(|(_, r)| !matches!(r, VerificationResult::Proved { .. }))
            .map(|(n, _)| n.as_str())
            .collect()
    }

    pub fn add_result(&mut self, name: Text, result: VerificationResult) {
        self.results.push((name, result));
    }

    /// Record per-obligation timings for a function.
    ///
    /// Called by instrumentation-aware verifiers after
    /// discharging each obligation. Labels are caller-chosen
    /// (typical: `"pre"`, `"post"`, `"refinement(x)"`,
    /// `"loop_inv(i)"`, `"termination"`) — the renderer
    /// displays them verbatim.
    ///
    /// Multiple calls for the same function name append to
    /// the existing timing list, preserving discharge order.
    pub fn add_obligation_timings(&mut self, function: Text, timings: List<(Text, Duration)>) {
        // Look up existing entry for this function; append or
        // create. List<(K, V)> is the stdlib pattern for
        // order-preserving maps — we match it here.
        for (name, existing) in self.obligation_timings.iter_mut() {
            if name.as_str() == function.as_str() {
                for t in timings {
                    existing.push(t);
                }
                return;
            }
        }
        self.obligation_timings.push((function, timings));
    }

    /// Return the per-obligation timings for `function`, or
    /// empty if none were recorded.
    pub fn obligation_timings_for(&self, function: &str) -> &[(Text, Duration)] {
        for (name, timings) in &self.obligation_timings {
            if name.as_str() == function {
                return timings.as_slice();
            }
        }
        &[]
    }

    /// Total number of recorded obligations across every
    /// function in the report. Zero when no instrumentation
    /// is available — the caller should fall back to the
    /// function-granular view.
    pub fn total_obligation_count(&self) -> usize {
        self.obligation_timings.iter().map(|(_, t)| t.len()).sum()
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
/// Outcome of a CVC5 fallback discharge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cvc5Outcome {
    /// Negation of the claim is Unsat — the claim is valid.
    Valid,
    /// Negation is Sat — a counterexample exists.
    Invalid,
    /// Solver returned Unknown or initialisation failed.
    Unknown,
}

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
        } else if as_text.starts_with('"') && as_text.ends_with('"') && as_text.len() >= 2 {
            CounterExampleValue::Text(Text::from(&as_text[1..as_text.len() - 1]))
        } else {
            CounterExampleValue::Unknown(Text::from(as_text.clone()))
        };

        assignments.insert(Text::from(name.as_str()), value);
    }

    CounterExample::new(assignments, Text::from("postcondition violation"))
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

#[cfg(test)]
mod trust_level_tests {
    use super::*;


    #[test]
    fn return_refinement_synthesis_rewrites_binder_to_result() {
        // T0678 — the soundness hole was a refined-return-only
        // function generating NO postcondition obligation at all.
        // Pin the synthesis contract: an inline `Int{ it == n + 1000 }`
        // return type yields exactly one implicit ensures whose binder
        // occurrences are rewritten to `result` (so the postcondition
        // pipeline's body->result binding applies), with the parameter
        // reference `n` left intact.
        use verum_ast::expr::{Expr, ExprKind};
        use verum_ast::ty::{Ident, Path, RefinementPredicate, Type, TypeKind};
        use verum_common::span::Span;

        let sp = Span::default();
        let ident_expr = |name: &str| {
            Expr::new(
                ExprKind::Path(Path::single(Ident::new(verum_common::Text::from(name), sp))),
                sp,
            )
        };
        // it == n  (the arithmetic tail is irrelevant to the rewrite)
        let pred_expr = Expr::new(
            ExprKind::Binary {
                op: verum_ast::expr::BinOp::Eq,
                left: verum_common::Heap::new(ident_expr("it")),
                right: verum_common::Heap::new(ident_expr("n")),
            },
            sp,
        );
        let ret_ty = Type::new(
            TypeKind::Refined {
                base: verum_common::Heap::new(Type::new(
                    TypeKind::Path(Path::single(Ident::new(verum_common::Text::from("Int"), sp))),
                    sp,
                )),
                predicate: verum_common::Heap::new(RefinementPredicate {
                    expr: pred_expr,
                    binding: verum_common::Maybe::None,
                    span: sp,
                }),
            },
            sp,
        );
        let out = synthesize_return_refinement_ensures_impl(
            &Some(ret_ty),
            &std::collections::HashMap::new(),
        );
        assert_eq!(out.len(), 1, "one implicit ensures per refined return");
        let rendered = format!("{:?}", out[0]);
        assert!(rendered.contains("result"), "binder rewritten to result: {rendered}");
        assert!(!rendered.contains("\"it\""), "no bare `it` remains: {rendered}");
        assert!(rendered.contains("\"n\""), "parameter reference survives: {rendered}");
        // An unrefined return synthesizes nothing.
        assert!(synthesize_return_refinement_ensures_impl(&None, &std::collections::HashMap::new()).is_empty());
    }

    #[test]
    fn parse_trust_level_default_is_signatures() {
        // Pin: missing / empty trust strings default to the documented
        // safe baseline. Anything else would mean a `verum.toml` that
        // declares only `distributed_cache = "..."` (no trust line)
        // silently downgrades to a different trust stance.
        assert_eq!(parse_trust_level(None), VerifyTrustLevel::Signatures);
        assert_eq!(parse_trust_level(Some("")), VerifyTrustLevel::Signatures);
        assert_eq!(parse_trust_level(Some("   ")), VerifyTrustLevel::Signatures);
    }

    #[test]
    fn parse_trust_level_recognises_documented_values() {
        // Pin: the three documented trust stances ("all",
        // "signatures", "signatures_and_expiry") parse to the
        // matching `TrustLevel` variant. Case-insensitive; surrounding
        // whitespace tolerated. Locks the contract surface so a typo
        // in the verum.toml schema docs can't drift the parser.
        assert_eq!(parse_trust_level(Some("all")), VerifyTrustLevel::All);
        assert_eq!(parse_trust_level(Some("ALL")), VerifyTrustLevel::All);
        assert_eq!(
            parse_trust_level(Some("signatures")),
            VerifyTrustLevel::Signatures
        );
        assert_eq!(
            parse_trust_level(Some(" Signatures ")),
            VerifyTrustLevel::Signatures
        );
        assert_eq!(
            parse_trust_level(Some("signatures_and_expiry")),
            VerifyTrustLevel::SignaturesAndExpiry
        );
        assert_eq!(
            parse_trust_level(Some("Signatures_And_Expiry")),
            VerifyTrustLevel::SignaturesAndExpiry
        );
    }

    #[test]
    fn parse_trust_level_unknown_falls_back_to_signatures() {
        // Pin: load-bearing safety contract — an unknown value MUST
        // never silently downgrade trust to `All`. Defaults to the
        // safe baseline and emits a warning (not asserted here; the
        // warning travels via `tracing` and is part of the user-
        // visible story, not the wire contract).
        assert_eq!(
            parse_trust_level(Some("trust_everything")),
            VerifyTrustLevel::Signatures
        );
        assert_eq!(
            parse_trust_level(Some("none")),
            VerifyTrustLevel::Signatures
        );
        assert_eq!(parse_trust_level(Some("0")), VerifyTrustLevel::Signatures);
    }
}
