//! Theorem / lemma / corollary proof verification orchestration.
//!
//! Extracted from `pipeline.rs` (#106 Phase 2) so the proof-verification
//! orchestration is independently reviewable. The single public entry
//! `verify_theorem_proofs` walks every `theorem` / `lemma` / `corollary`
//! in the module and dispatches to the appropriate proof-verification
//! strategy:
//!
//!   * Tactic proofs   → ProofSearchEngine (automated tactic application).
//!   * Term proofs     → Z3 formula translation + satisfiability check.
//!   * Structured     → weakest-precondition calculus.
//!   * Method proofs   → induction / cases via WP engine.
//!
//! The closure-cache fast path (#79 / #88) keys each verdict on a
//! blake3 hash of the theorem signature + proof body + sorted+deduped
//! `@framework(...)` citations. Cache hits skip the SMT round entirely.

use std::time::{Duration, Instant};

use anyhow::Result;
use tracing::{debug, info, warn};

use verum_ast::Module;
use verum_smt::Context as SmtContext;

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    /// Process all `theorem`, `lemma`, and `corollary` declarations in
    /// the module.
    pub(super) fn verify_theorem_proofs(&self, module: &Module) -> Result<()> {
        use crate::phases::proof_verification::{
            build_refinement_alias_map, register_module_lemmas,
            verify_proof_body_with_aliases, ProofVerificationResult,
        };
        use verum_smt::proof_search::{HintsDatabase, ProofSearchEngine};

        // Flatten every nominal refinement alias in this module so
        // downstream `verify_proof_body_with_aliases` can materialise
        // hypotheses for parameters typed as aliases (e.g.
        // `n: FanoDim` → `n == 7`).
        let alias_map = build_refinement_alias_map(module);

        let mut theorem_count = 0u32;
        let mut verified_count = 0u32;
        let mut failed_count = 0u32;
        let mut axiom_count = 0u32;
        let mut cache_hits = 0u32;
        let mut cache_misses = 0u32;

        // Optional closure-cache wiring (#79 / #88). When enabled,
        // each theorem proof's verdict is keyed on a blake3 closure
        // hash of its signature + proof body + sorted+deduped
        // citations (+ kernel version). Cache hits skip the SMT /
        // kernel re-check entirely.
        let closure_cache: Option<verum_verification::closure_cache::FilesystemCacheStore> =
            if self.session.options().closure_cache_enabled {
                let root = match &self.session.options().closure_cache_root {
                    Some(p) => p.clone(),
                    None => {
                        let parent = self
                            .session
                            .options()
                            .input
                            .parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| std::path::PathBuf::from("."));
                        parent
                            .join("target")
                            .join(".verum_cache")
                            .join("closure-hashes")
                    }
                };
                match verum_verification::closure_cache::FilesystemCacheStore::new(&root) {
                    Ok(s) => {
                        debug!(
                            "Closure cache enabled at {} (kernel={})",
                            root.display(),
                            verum_verification::closure_cache::KERNEL_VERSION
                        );
                        Some(s)
                    }
                    Err(e) => {
                        warn!(
                            "Closure cache requested but could not open at {}: {} \
                             — proceeding without cache",
                            root.display(),
                            e
                        );
                        None
                    }
                }
            } else {
                None
            };

        let timeout_ms = self.session.options().smt_timeout_secs * 1000;
        let timeout = Duration::from_millis(timeout_ms);

        // Seed the hints DB with stdlib core lemmas *and* every
        // sibling theorem / axiom / lemma in this module, so
        // `apply <name>` can dispatch to local declarations in the
        // same file — the idiom used by the UHM bridge / corollary
        // structure.
        let mut hints_db = HintsDatabase::with_core();
        register_module_lemmas(module, &mut hints_db);
        let mut proof_engine = ProofSearchEngine::with_hints(hints_db);

        // Refinement reflection: scan the module for pure,
        // single-expression functions and translate their bodies to
        // SMT-LIB via the Expr→SMT-LIB translator. Successfully
        // translated definitions are registered as axioms in the
        // proof engine so `proof by auto` can unfold user function
        // calls through Z3.
        //
        // Conservative: functions that can't be translated (multi-
        // statement bodies, unsupported operators, closures, etc.)
        // are silently skipped — no incorrect axiom is ever emitted.
        {
            use verum_smt::expr_to_smtlib::try_reflect_function;
            use verum_smt::refinement_reflection::RefinementReflectionRegistry;

            let mut registry = RefinementReflectionRegistry::new();
            for item in &module.items {
                if let verum_ast::ItemKind::Function(func_decl) = &item.kind {
                    if let Some(rf) = try_reflect_function(func_decl) {
                        let _ = registry.register(rf);
                    }
                }
            }
            if !registry.is_empty() {
                tracing::debug!(
                    "Refinement reflection: {} function(s) reflected as SMT axioms",
                    registry.len()
                );
                proof_engine.set_reflection_registry(registry);
            }
        }

        let smt_config = verum_smt::context::ContextConfig {
            timeout: Some(timeout),
            ..Default::default()
        };
        let smt_ctx = SmtContext::with_config(smt_config);

        for item in &module.items {
            let (thm, kind_name) = match &item.kind {
                verum_ast::ItemKind::Theorem(t) => (t, "theorem"),
                verum_ast::ItemKind::Lemma(t) => (t, "lemma"),
                verum_ast::ItemKind::Corollary(t) => (t, "corollary"),
                _ => continue,
            };
            theorem_count += 1;

            if thm.proof.is_none() {
                axiom_count += 1;
                debug!(
                    "{} '{}' accepted as axiom (no proof body)",
                    kind_name, thm.name.name
                );
                continue;
            }

            debug!(
                "Verifying {} '{}' ({} requires, {} ensures)",
                kind_name,
                thm.name.name,
                thm.requires.len(),
                thm.ensures.len()
            );

            // Closure-cache fast path (#79 / #88). Compute fingerprint
            // and probe the cache before invoking the SMT engine. On
            // hit we serve the cached verdict; on miss we run the
            // engine and persist the new verdict. No-op when
            // closure_cache is None.
            let cache_outcome = closure_cache.as_ref().map(|store| {
                use verum_verification::closure_cache::{
                    cached_check, CachedVerdict, ClosureFingerprint,
                };
                // Signature payload: name + requires + ensures +
                // proposition rendering. Stable across runs so long
                // as the AST `Debug` projection is stable.
                let signature_payload = format!(
                    "{}|requires={:?}|ensures={:?}|prop={:?}",
                    thm.name.name.as_str(),
                    thm.requires,
                    thm.ensures,
                    thm.proposition,
                );
                // Body payload: proof body rendering.
                let body_payload = format!("{:?}", thm.proof);
                // Citations: every @framework("name", "citation") on
                // this declaration. Sorted+deduped inside `compute`.
                let mut citations: Vec<String> = Vec::new();
                for a in &thm.attributes {
                    if a.is_named("framework") {
                        citations.push(format!("{:?}", a));
                    }
                }
                let cite_refs: Vec<&str> = citations.iter().map(String::as_str).collect();
                let fp = ClosureFingerprint::compute(
                    verum_verification::closure_cache::KERNEL_VERSION,
                    signature_payload.as_bytes(),
                    body_payload.as_bytes(),
                    &cite_refs,
                );

                cached_check(store, thm.name.name.as_str(), &fp, || {
                    // The verify closure: runs the actual SMT engine
                    // and projects its verdict onto CachedVerdict.
                    let verify_start = Instant::now();
                    match verify_proof_body_with_aliases(
                        &mut proof_engine,
                        &smt_ctx,
                        thm,
                        &alias_map,
                    ) {
                        ProofVerificationResult::Verified(cert) => {
                            let elapsed = verify_start.elapsed().as_millis() as u64;
                            verified_count += 1;
                            info!(
                                "✓ {} '{}' verified ({} steps, {:.1}ms)",
                                kind_name,
                                thm.name.name,
                                cert.steps.len(),
                                cert.total_duration.as_secs_f64() * 1000.0
                            );
                            CachedVerdict::Ok { elapsed_ms: elapsed }
                        }
                        ProofVerificationResult::Failed { unproved, .. } => {
                            failed_count += 1;
                            warn!(
                                "✗ {} '{}' verification failed ({} unproved goal(s))",
                                kind_name,
                                thm.name.name,
                                unproved.len()
                            );
                            for goal in &unproved {
                                debug!("  unproved: {:?}", goal.goal);
                                for s in &goal.suggestions {
                                    debug!("    hint: {}", s);
                                }
                            }
                            CachedVerdict::Failed {
                                reason: verum_common::Text::from(format!(
                                    "{} unproved goal(s)",
                                    unproved.len()
                                )),
                            }
                        }
                    }
                })
            });

            match cache_outcome {
                Some(outcome) => {
                    use verum_verification::closure_cache::CachedCheckOutcome;
                    if let CachedCheckOutcome::Hit { cached, .. } = &outcome {
                        cache_hits += 1;
                        verified_count += 1; // Cached Ok counts as verified.
                        debug!(
                            "✓ {} '{}' cached (hit, recorded_at={})",
                            kind_name, thm.name.name, cached.recorded_at,
                        );
                    } else {
                        cache_misses += 1;
                    }
                }
                None => {
                    // No cache configured — run the engine directly.
                    match verify_proof_body_with_aliases(
                        &mut proof_engine,
                        &smt_ctx,
                        thm,
                        &alias_map,
                    ) {
                        ProofVerificationResult::Verified(cert) => {
                            verified_count += 1;
                            info!(
                                "✓ {} '{}' verified ({} steps, {:.1}ms)",
                                kind_name,
                                thm.name.name,
                                cert.steps.len(),
                                cert.total_duration.as_secs_f64() * 1000.0
                            );
                        }
                        ProofVerificationResult::Failed { unproved, .. } => {
                            failed_count += 1;
                            warn!(
                                "✗ {} '{}' verification failed ({} unproved goal(s))",
                                kind_name,
                                thm.name.name,
                                unproved.len()
                            );
                            for goal in &unproved {
                                debug!("  unproved: {:?}", goal.goal);
                                for s in &goal.suggestions {
                                    debug!("    hint: {}", s);
                                }
                            }
                        }
                    }
                }
            }
        }

        if theorem_count > 0 {
            let stats = proof_engine.stats();
            info!(
                "Theorem verification: {}/{} verified, {} failed, {} axioms \
                 (search: {} attempts, {} hits)",
                verified_count,
                theorem_count - axiom_count,
                failed_count,
                axiom_count,
                stats.total_attempts,
                stats.successes
            );
            if closure_cache.is_some() {
                let total_decisions = cache_hits + cache_misses;
                let ratio = if total_decisions == 0 {
                    0.0
                } else {
                    cache_hits as f64 / total_decisions as f64
                };
                info!(
                    "Closure cache: {} hit(s), {} miss(es), {:.1}% hit-ratio",
                    cache_hits,
                    cache_misses,
                    ratio * 100.0,
                );
            }
        }

        Ok(())
    }
}
