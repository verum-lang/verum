//! Phase orchestration cluster (safety gate → typecheck → verify
//! → context/send_sync/cbgr/ffi).
//!

//! Extracted from `pipeline.rs` (#106 Phase 23). Houses eight
//! phase orchestrator wrappers that drive the per-module
//! compilation lifecycle. Each wrapper either delegates to a
//! dedicated phase crate (`crate::phases::*`) or composes the
//! sub-pass set itself.
//!

//! Surface:
//!

//!  * `phase_safety_gate` — Phase 2.9 `[safety]` feature gates
//!  (unsafe blocks, `@ffi`, extern fn).
//!  * `phase_stdlib_lints` — pure-AST stdlib hazard linter
//!  (W05xx warning class).
//!  * `phase_type_check` — Phase 3 type checking
//!  (the largest single method at ~792 LOC; orchestrates
//!  every sub-pass from name registration through method
//!  dispatch).
//!  * `phase_dependency_analysis` — Phase 3.5 target-profile
//!  enforcement (no_std / no_alloc / embedded / cbgr_static_only).
//!  * `phase_verify` — Phase 4 refinement + theorem
//!  verification (Z3 + CVC5 portfolio).
//!  * `phase_context_validation` — Phase 4c context-system
//!  validation (negative constraints, provision checks).
//!  * `phase_send_sync_validation` — Phase 4d Send/Sync
//!  compile-time enforcement.
//!  * `phase_ffi_validation` — Phase 5b FFI boundary
//!  validation.

use std::time::Instant;

use anyhow::Result;
use colored::Colorize;
use tracing::{debug, info, warn};

use verum_ast::Module;
use verum_common::List;
use verum_diagnostics::{DiagnosticBuilder, Severity};
use verum_smt::{Context as SmtContext, CostTracker};
use verum_types::TypeChecker;

use crate::phases::type_error_to_diagnostic;

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    /// Phase 2.9: Safety feature gates (unsafe blocks, unsafe fn,
    /// `@ffi` / extern fn, per `[safety]` in verum.toml).
    ///

    /// **Runs independently of verify_mode** so `--verify runtime`
    /// cannot silently bypass the gate. Invoked by BOTH the
    /// interpreter and AOT paths before type-checking. Emits a
    /// diagnostic for each rejected construct and returns Err if any
    /// were rejected.
    pub(super) fn phase_safety_gate(&self, module: &Module) -> Result<()> {
        let features = self.session.language_features();
        // Fast path: when every relevant [safety] flag is permissive,
        // skip the walker entirely — zero cost on the default
        // configuration.
        if features.unsafe_allowed()
            && features.safety.ffi
            && !features.safety.capability_required
            && !features.safety.forbid_stdlib_extern
        {
            return Ok(());
        }
        let policy = crate::phases::safety_gate::SafetyPolicy::from_features(&features.safety);
        let diags = crate::phases::safety_gate::check_safety(std::slice::from_ref(module), policy);
        if !diags.is_empty() {
            let n = diags.len();
            for d in diags.iter() {
                self.session.emit_diagnostic(d.clone());
            }
            return Err(anyhow::anyhow!(
                "safety gate rejected {} construct(s); see diagnostics",
                n
            ));
        }
        Ok(())
    }

    /// Stdlib-hazard lint pass.
    ///

    /// Pure AST walk; runs before type checking so the user sees
    /// W05xx warnings even when the module has type errors that
    /// would stop the pipeline at `phase_type_check`. Findings
    /// are emitted as warning-level diagnostics and never fail
    /// the build (the lint's `default_level()` is `Warn`); CI
    /// teams that want stricter enforcement set `-Dmap_get_hazard`
    /// in their lint configuration.
    pub(super) fn phase_stdlib_lints(&self, module: &Module) {
        use crate::lint::{StdlibLintFinding, walk_module_for_stdlib_hazards};
        let findings: Vec<StdlibLintFinding> = walk_module_for_stdlib_hazards(module);
        for finding in findings {
            let summary_text = finding.lint.summary();
            let diag = verum_diagnostics::DiagnosticBuilder::warning()
                .code(finding.lint.warning_code())
                .message(format!("{} (`{}`)", summary_text, finding.lint.name()))
                .span(crate::phases::ast_span_to_diagnostic_span(
                    finding.span,
                    None,
                ))
                .help(
                    "Prefer `get_optional(key)` for presence semantics or \
                     `get_or(key, default)` for an explicit fallback.",
                )
                .build();
            self.session.emit_diagnostic(diag);
        }
    }

    /// Phase 3: Type checking.
    ///
    /// Resolution side-table from inference is drained into
    /// `self.resolved_call_targets` for the caller to apply via
    /// `apply_resolved_call_targets(&mut module)` once it has
    /// mutable access to the AST (#91/#95 fast path — codegen reads
    /// `Expr::resolved_call_target` to skip the legacy 7-step
    /// name-resolution cascade).  Splitting into "type-check (&Mod)"
    /// + "apply (&mut Mod)" keeps the immutable-AST contract every
    /// other phase relies on while still routing the typechecker's
    /// resolutions to the AST without &mut plumbing through eight
    /// upstream phases.
    pub(super) fn phase_type_check(&mut self, module: &Module) -> Result<()> {
        debug!("Type checking module");

        // Safety-gate pre-pass. Still invoked here (in addition to the
        // callsites in run_interpreter / run_native_compilation) so any
        // pipeline entry point that jumps directly to type_check keeps
        // the gate active. Idempotent — running twice costs one extra
        // walk on the fast path (no violations).
        self.phase_safety_gate(module)?;

        // Stdlib-hazard lint pass — AST-only, emits W05xx
        // warnings. Runs before type checking so users see the
        // hazards even when the module has unrelated type errors.
        self.phase_stdlib_lints(module);

        let start = Instant::now();

        // Mode selection:
        // - NormalBuild (stdlib_metadata = Some): Use pre-compiled stdlib types
        // - StdlibBootstrap (stdlib_metadata = None): Use builtins only
        let mut checker = match self.stdlib_metadata.get() {
            Some(metadata) => {
                debug!(
                    "Phase 3: Using stdlib metadata for type checking ({} types)",
                    metadata.types.len()
                );
                TypeChecker::new_with_core(std::sync::Arc::clone(metadata))
            }
            None => {
                // Compiling stdlib itself - use minimal context
                TypeChecker::with_minimal_context()
            }
        };

        // Register built-in types and functions
        // NOTE: In NormalBuild mode, these may already be loaded from stdlib metadata,
        // but register_builtins() is idempotent and ensures core intrinsics are available.
        checker.register_builtins();

        // T2-extended-perf: lazy stdlib type registration.  The
        // `new_with_core` constructor stores `core_metadata` but
        // doesn't pre-register anything.  Scan the user module for
        // every named-type reference and pull each from metadata —
        // O(types_used_by_user) instead of O(stdlib_total ≈ 1000+).
        // For a hello.vr touching ~5 stdlib symbols this drops the
        // typecheck from 3.8s to ~50ms.
        if self.stdlib_metadata.is_some() {
            checker.register_stdlib_types_for_module(module);

            // **Audit-driven fundamental fix** — seed blanket
            // protocol impls from `core/base/protocols.vr` so
            // primitive method dispatch on `partial_cmp` / `ne` /
            // `lt` / `le` / `gt` / `ge` / etc. resolves through the
            // `implement<T: Ord> PartialOrd for T` blanket.  The
            // archive-driven stdlib loader builds a synthetic empty
            // AST, so `register_module_blanket_impls`'s walker has
            // no impl items to register.  Mirrors the codegen-side
            // `seed_protocol_registry_from_embedded_stdlib` at
            // `pipeline/vbc_codegen.rs:830`.
            seed_typechecker_blanket_impls(&mut checker);
        }

        // Apply `[protocols].coherence` from manifest. Closes the
        // inert-defense pattern at session.rs:587 — pre-fix the CLI
        // build path bypassed the field entirely (it only flowed
        // through `run_common_pipeline` from api.rs). Now both
        // entry points consume the manifest value and gate
        // `register_impl`'s orphan-rule + overlap checks.
        {
            let coherence = &self.session.language_features().protocols.coherence;
            checker.set_protocol_coherence_mode(
                verum_types::protocol::CoherenceMode::from_manifest_str(coherence.as_str()),
            );
        }

        // Apply `[protocols].higher_kinded_protocols` from manifest.
        // Closes the inert-defense pattern at session.rs:590.
        // Default false: a protocol declaring an HKT generic
        // parameter (e.g. `protocol Functor<F<_>>`) is rejected at
        // registration time. Manifest validation enforces that
        // this flag can be true only when [types].higher_kinded
        // is also true.
        checker.set_higher_kinded_protocols_enabled(
            self.session
                .language_features()
                .protocols
                .higher_kinded_protocols,
        );

        // Apply `[protocols].generic_associated_types` from manifest.
        // Closes #265. Default false: a protocol declaring an
        // associated type with non-empty type_params (`type Item<T>`)
        // is rejected at registration time. Manifest validation
        // enforces that this flag can be true only when
        // [protocols].associated_types is also true.
        checker.set_generic_associated_types_enabled(
            self.session
                .language_features()
                .protocols
                .generic_associated_types,
        );

        // Post-cycle-break (2026-04-24): `RefinementChecker` no longer
        // auto-constructs a Z3 backend. Install the concrete bridge from
        // `verum_smt` so refinement subsumption keeps working.
        checker.set_smt_backend(Box::new(
            verum_smt::refinement_backend::RefinementZ3Backend::new(),
        ));

        // Enable lenient contexts if the module has meta functions with using clauses.
        // Meta contexts (MetaTypes, MetaRuntime, etc.) are handled at the meta evaluation
        // level, not at the runtime context level. Without lenient mode, @const blocks
        // that call meta functions with contexts would fail type checking.
        let has_meta_contexts = module.items.iter().any(|item| {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                func.is_meta && !func.contexts.is_empty()
            } else {
                false
            }
        });
        if has_meta_contexts {
            checker.set_lenient_contexts(true);
        }

        // ==============================================================
        // Dependent type pipeline activation (Phase A.5 — surgical)
        // ==============================================================
        //

        // Historical note: before this hook was added, the entire SMT-
        // based dependent type verification path in `verum_types` and
        // `verum_smt/src/dependent.rs` was dormant code — fully
        // implemented (~3700 LoC in verum_smt alone plus the full
        // `UniverseContext` solver at `verum_types/src/context.rs:681`)
        // but never switched on from the main compiler pipeline. It only
        // ran in isolated unit tests that called `enable_dependent_types`
        // explicitly. Production builds therefore silently bypassed it.
        //

        // This activation is deliberately **opportunistic**:
        //

        //  - If the module contains ANY declaration that uses dependent-
        //  type machinery (theorem / lemma / axiom / corollary /
        //  tactic items, or refinement predicates on function
        //  parameters / return types), we enable the checker's
        //  dependent type subsystem so Pi, Sigma, Eq and universe
        //  constraints are verified through SMT.
        //

        //  - Modules that do NOT use any of the above pay zero cost —
        //  the subsystem remains disabled and compilation is
        //  bit-identical to the pre-activation behaviour.
        //

        // This preserves backward compatibility with every existing
        // `.vr` source file while making dependent types available to
        // theorem-bearing modules without requiring any user flag.
        //

        // Detection criteria (keep in sync with proof_erasure in
        // `crates/verum_vbc/src/codegen/mod.rs:3232-3250`):
        //

        //  - `ItemKind::Theorem | Lemma | Corollary | Axiom | Tactic`
        //  always require dependent type checking (proof goals are
        //  type-level propositions).
        //

        // A future refinement may also scan for explicit Pi / Sigma /
        // refinement types in function signatures and enable the
        // subsystem only when needed; for now, "any proof item present"
        // is the conservative, opt-in trigger.
        //

        // Related: `crates/verum_types/src/infer.rs:3135`
        // (`enable_dependent_types`), `:3118` (`verify_dependent_type`),
        // `:4097` (`verify_dependent_type_constraint`), call sites at
        // `:19754, :20229, :20263, :20328`.
        let has_proof_items = module.items.iter().any(|item| {
            matches!(
                &item.kind,
                verum_ast::ItemKind::Theorem(_)
                    | verum_ast::ItemKind::Lemma(_)
                    | verum_ast::ItemKind::Corollary(_)
                    | verum_ast::ItemKind::Axiom(_)
                    | verum_ast::ItemKind::Tactic(_)
            )
        });
        if has_proof_items {
            debug!("Phase 3: Enabling dependent type checking (proof items detected)");
            // Post-cycle-break (2026-04-24): `enable_dependent_types()`
            // no longer auto-constructs a verifier. Inject the SMT-backed
            // implementation directly from `verum_smt`.
            checker.enable_dependent_types();
            checker.set_dependent_checker(Box::new(
                verum_smt::dependent_backend::SmtDependentTypeChecker::new(),
            ));
        }

        // Configure type checker with module registry for cross-file resolution
        let registry = self.session.module_registry();
        checker.set_module_registry(registry.clone());

        // Configure lazy resolver for on-demand module loading
        // This enables imports to trigger module loading if not already loaded
        checker.set_lazy_resolver(self.lazy_resolver.clone());

        // Register cross-file contexts (protocols and contexts from other modules)
        // This enables `using [Database, Auth]` to work when these are defined elsewhere
        for context_name in &self.collected_contexts {
            checker.register_protocol_as_context(context_name.clone());
        }

        // Pre-register stdlib context declarations from CoreMetadata.
        // This enables `using [ComputeDevice]` etc. to resolve even
        // in single-file compilation where the declaring module
        // (gpu.vr) isn't explicitly loaded.
        //
        // **Cold-start fast-path**: when the metadata sidecar embeds
        // full `ContextDecl` AST nodes (newer precompile output),
        // register them via `register_stdlib_context_full` so method
        // signatures land at typecheck-ready depth — equivalent to
        // the legacy fallback's full-parse loop, but without re-
        // parsing 568 stdlib `.vr` files at runtime.  Sidecars built
        // by older precompile runs only have the names list; the
        // legacy fallback below still fires in that case.
        if let Some(metadata) = self.stdlib_metadata.get() {
            for (ctx_name, ctx_decl) in metadata.context_decl_nodes.iter() {
                if !self.collected_contexts.contains(ctx_name) {
                    checker.register_protocol_as_context(ctx_name.clone());
                }
                checker.register_stdlib_context_full(
                    ctx_name.clone(),
                    ctx_decl.clone(),
                );
            }
            // Fall back to name-only registration for any context
            // that's listed in `context_declarations` but didn't make
            // it into `context_decl_nodes` (e.g. a parse error during
            // precompile dropped the AST while the line-scan still
            // caught the name).
            for ctx_name in &metadata.context_declarations {
                if metadata.context_decl_nodes.contains_key(ctx_name) {
                    continue;
                }
                if !self.collected_contexts.contains(ctx_name) {
                    checker.register_protocol_as_context(ctx_name.clone());
                }
            }
        }

        // #102 — embedded source-archive fallback for context
        // declarations REMOVED.  Pre-fix this branch ran when
        // `stdlib_metadata.context_declarations` was empty (the
        // "old metadata cache format" case) and reparsed every
        // `core/**/*.vr` looking for `public context …` patterns.
        // That path was the last consumer of `embedded_stdlib`'s
        // gzipped .vr sources for production typecheck — keeping it
        // forced the binary to embed the full source archive.
        //
        // Post-fix: `CoreMetadata.context_declarations` /
        // `context_decl_nodes` are populated unconditionally during
        // precompile (`scan_context_declarations` in
        // `precompile.rs::write_core_metadata_alongside_archive`)
        // and the schema-version cache key (#97) invalidates any
        // stale precompile that pre-dates that field.  An empty
        // contexts list now indicates a real bug — re-precompile
        // (`verum stdlib precompile`) is the correct remediation,
        // not silent reparse-from-source.

        // Compute the current module path for resolving relative imports (self, super)
        // For single-file checking, we need to find the project's src root by looking
        // upward from the file location. The src root is typically:
        // - A directory named "src" that contains .vr files
        // - Or a directory containing Verum.toml
        // - Or the parent of "core/" for stdlib files
        let project_root = self.session.options().input.clone();
        let src_root = if project_root.is_dir() {
            let src_dir = project_root.join("src");
            if src_dir.exists() && src_dir.is_dir() {
                src_dir
            } else {
                project_root.clone()
            }
        } else {
            // Single file: walk up the directory tree to find src root
            // The src root is typically a directory named "src" or a directory
            // that contains Verum.toml (project root's src subdirectory)
            let mut current = project_root.parent().map(|p| p.to_path_buf());
            let mut found_src = None;

            while let Some(dir) = current {
                // Check if this directory is named "src"
                if dir.file_name().map(|n| n == "src").unwrap_or(false) {
                    found_src = Some(dir.clone());
                    break;
                }
                // Check if parent has a Verum.toml (meaning this is inside a project)
                if let Some(parent) = dir.parent() {
                    if parent.join("Verum.toml").exists() || parent.join("verum.toml").exists() {
                        // Special case: if the parent directory is named "core", this
                        // is a stdlib file. Use core/'s parent as the src_root so that
                        // file_path_to_module_path produces paths like "core.async".
                        if parent.file_name().map(|n| n == "core").unwrap_or(false) {
                            if let Some(grandparent) = parent.parent() {
                                found_src = Some(grandparent.to_path_buf());
                                break;
                            }
                        }
                        // This directory (or "src" subdirectory) is the source root
                        let src_dir = parent.join("src");
                        if src_dir.exists() && src_dir.is_dir() {
                            found_src = Some(src_dir);
                        } else {
                            found_src = Some(dir.clone());
                        }
                        break;
                    }
                }
                current = dir.parent().map(|p| p.to_path_buf());
            }

            found_src.unwrap_or_else(|| {
                // Fallback: use the file's parent directory
                project_root
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or(project_root.clone())
            })
        };

        // Try to compute module path from file if available
        let current_module_path_str = if let Some(item) = module.items.first() {
            if let Some(source_file) = self.session.get_source(item.span.file_id) {
                if let Some(ref file_path) = source_file.path {
                    let module_path = self.file_path_to_module_path(file_path, &src_root);
                    debug!(
                        "phase_type_check: file_path={:?}, src_root={:?}, module_path={}",
                        file_path, src_root, module_path
                    );
                    module_path.to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Load imported modules into the registry for single-file checking.
        // This ensures that imported types, functions, and contexts are available
        // for type checking. Without this, imports like `import super.contexts.{Database}`
        // would fail because the imported module wouldn't be in the registry.
        debug!(
            "Single-file type check: src_root={:?}, current_module_path={}",
            src_root, current_module_path_str
        );
        let mut loaded_paths = std::collections::HashSet::new();
        if let Err(e) = self.load_imported_modules(
            module,
            &current_module_path_str,
            &src_root,
            &mut loaded_paths,
        ) {
            warn!("Error loading imported modules: {}", e);
        }
        debug!(
            "Loaded {} imported modules, collected {} contexts",
            loaded_paths.len(),
            self.collected_contexts.len()
        );

        // Re-register cross-file contexts after loading imported modules
        // (new contexts may have been discovered)
        for context_name in &self.collected_contexts {
            checker.register_protocol_as_context(context_name.clone());
        }

        // Multi-pass type checking:
        // Pass -1: Pre-register all inline modules
        // This enables cross-module imports even when modules are declared after
        // the modules that import from them.
        // Pre-register inline modules for order-independent cross-module imports.
        for item in &module.items {
            if let verum_ast::ItemKind::Module(module_decl) = &item.kind {
                checker.pre_register_module_public(module_decl, "cog");
            }
        }

        // Pass 0: Process imports to register imported types and functions
        // This enables cross-file type resolution for imported types
        for item in &module.items {
            if let verum_ast::ItemKind::Mount(import) = &item.kind {
                if let Err(type_error) =
                    checker.process_import(import, &current_module_path_str, &registry.read())
                {
                    let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        // ═══════════════════════════════════════════════════════════════════
        // PRE-PASS: Register stdlib module declarations into the type checker.
        // This processes all parsed stdlib modules through the same multi-pass
        // registration that user modules get, making stdlib types, protocols,
        // and implement block methods (List.push, Maybe.unwrap, etc.) available
        // for type checking user code.
        //

        // This is stdlib-agnostic: the compiler knows nothing about which types
        // or methods exist — it simply processes whatever .vr files were loaded.
        // ═══════════════════════════════════════════════════════════════════
        // Collect all stdlib modules (clone Arc handles to avoid borrow conflict).
        //

        // CRITICAL: Sort by key to ensure deterministic iteration order. self.modules is a
        // HashMap, so its natural iteration order is non-deterministic. When two modules
        // expose functions with the same short name (e.g. `core.base.memory::drop<T>` vs
        // `core.base.iterator.Transducer::drop<A>` (nested module)), non-deterministic
        // ordering lets different signatures "win" the top-level binding on different
        // runs, causing flaky L2 test failures.
        //

        // Shallower (fewer-dot) module keys are prioritized so top-level stdlib functions
        // beat nested-module helpers when short names collide.
        let mut stdlib_entries: Vec<_> = self
            .modules
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        stdlib_entries.sort_by(|(a, _), (b, _)| {
            let depth_a = a.as_str().matches('.').count();
            let depth_b = b.as_str().matches('.').count();
            depth_a
                .cmp(&depth_b)
                .then_with(|| a.as_str().cmp(b.as_str()))
        });

        // Preserve the user-file module path so we can restore it after
        // stdlib registration transiently rebinds it per-module.
        let saved_module_path = checker.current_module_path().clone();

        if !stdlib_entries.is_empty() {
            if self.stdlib_metadata.is_none() {
                debug!(
                    "Registering {} stdlib modules into type checker (bootstrap mode)",
                    stdlib_entries.len()
                );

                // Pass S0a: Register all stdlib type names (module-scoped).
                for (module_path, stdlib_mod) in &stdlib_entries {
                    checker.set_current_module_path(module_path.clone());
                    checker.register_all_type_names(&stdlib_mod.items);
                }

                // Pass S0b: Resolve all stdlib type definitions
                let mut resolution_stack = List::new();
                for (module_path, stdlib_mod) in &stdlib_entries {
                    checker.set_current_module_path(module_path.clone());
                    for item in &stdlib_mod.items {
                        if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                            if let Err(e) =
                                checker.resolve_type_definition(type_decl, &mut resolution_stack)
                            {
                                debug!("Stdlib type resolution error: {:?}", e);
                            }
                        }
                    }
                }

                // Pass S1: Register stdlib function signatures
                for (module_path, stdlib_mod) in &stdlib_entries {
                    checker.set_current_module_path(module_path.clone());
                    for item in &stdlib_mod.items {
                        if let verum_ast::ItemKind::Function(func) = &item.kind {
                            if !checker.is_function_preregistered(func.name.name.as_str()) {
                                let _ = checker.register_function_signature(func);
                            }
                        }
                    }
                }

                // Pass S2: Register stdlib protocols
                for (module_path, stdlib_mod) in &stdlib_entries {
                    checker.set_current_module_path(module_path.clone());
                    for item in &stdlib_mod.items {
                        if let verum_ast::ItemKind::Protocol(protocol_decl) = &item.kind {
                            let _ = checker.register_protocol(protocol_decl);
                        }
                    }
                }
            }

            // Pass S3: ALWAYS register stdlib impl blocks (module-scoped).
            debug!(
                "Registering stdlib impl blocks ({} modules)",
                stdlib_entries.len()
            );
            for (module_path, stdlib_mod) in &stdlib_entries {
                checker.set_current_module_path(module_path.clone());
                for item in &stdlib_mod.items {
                    if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                        let _ = checker.register_impl_block(impl_decl);
                    }
                }
            }

            // Collect any unresolved placeholder names from stdlib so we can
            // exclude them from user-module placeholder verification
            let stdlib_placeholder_errors = checker.verify_no_placeholders();
            for err in &stdlib_placeholder_errors {
                debug!("Stdlib placeholder (expected): {:?}", err);
            }

            debug!("Stdlib registration complete");
        }

        // Restore the user-file module path so subsequent passes run in
        // the right resolution scope.
        checker.set_current_module_path(saved_module_path);

        // Signal transition to user code phase: variant short-name protection is relaxed
        // so user-defined types can shadow stdlib unit variants (e.g., Status.Pending).
        checker.set_user_code_phase();

        // Collect pre-existing placeholder names (from stdlib) to exclude from user verification
        let pre_existing_placeholders: std::collections::HashSet<String> = checker
            .verify_no_placeholders()
            .iter()
            .filter_map(|e| {
                if let verum_types::TypeError::UnresolvedPlaceholder { name, .. } = e {
                    Some(name.to_string())
                } else {
                    None
                }
            })
            .collect();

        // Two-pass type resolution for order-independent type definitions:
        //

        // This allows types to reference each other regardless of definition order:
        //  type SearchRequest is { sort_by: SortOrder }; // SortOrder used before defined
        //  type SortOrder is Relevance | Downloads;
        //

        // Pass 1a: Register all type names as placeholders
        // This makes all type names available for forward references
        checker.register_all_type_names(&module.items);

        // Pass 1b: Resolve all type definitions now that all names are known
        // This replaces placeholders with actual type definitions
        let mut resolution_stack = List::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                if let Err(e) = checker.resolve_type_definition(type_decl, &mut resolution_stack) {
                    let diag = type_error_to_diagnostic(&e, Some(self.session));
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        // Verify no placeholder types remain (indicates unresolved forward references)
        // Skip placeholders that were already unresolved from stdlib pre-registration
        for error in checker.verify_no_placeholders() {
            let is_stdlib_placeholder =
                if let verum_types::TypeError::UnresolvedPlaceholder { name, .. } = &error {
                    pre_existing_placeholders.contains(name.as_str())
                } else {
                    false
                };
            if !is_stdlib_placeholder {
                let diag = type_error_to_diagnostic(&error, Some(self.session));
                self.session.emit_diagnostic(diag);
            }
        }

        // Pass 1.9: Pre-register context declarations for forward references.
        // Without this, `using [ComputeDevice]` fails if ComputeDevice is
        // declared later in the same file. We register with full AST (not
        // just names) so method resolution works.
        for item in &module.items {
            if let verum_ast::ItemKind::Context(ctx_decl) = &item.kind {
                checker.register_stdlib_context_full(
                    verum_common::Text::from(ctx_decl.name.name.as_str()),
                    ctx_decl.clone(),
                );
            }
        }

        // Pass 2: Register protocol declarations
        for item in &module.items {
            if let verum_ast::ItemKind::Protocol(protocol_decl) = &item.kind {
                if let Err(e) = checker.register_protocol(protocol_decl) {
                    let diag = type_error_to_diagnostic(&e, Some(self.session));
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        // Pass 3: Register protocol implementations
        for item in &module.items {
            if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                if let Err(e) = checker.register_impl_block(impl_decl) {
                    let diag = type_error_to_diagnostic(&e, Some(self.session));
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        // Pass 3.5: Coherence checking (orphan rule, overlap detection)
        // Protocol coherence: at most one `implement Protocol for Type` per concrete
        // type in the entire dependency graph (orphan rule + overlap prevention).
        self.check_protocol_coherence(module)?;

        // Pass 3.6: Profile boundary enforcement
        // Profile boundaries: Application modules cannot depend on Systems-profile code.
        self.check_profile_boundaries(module)?;

        // Pass 4: Register function signatures (enables forward references)
        // This allows functions to call other functions defined later in the file:
        //  fn main() -> Int { fib(10) } // fib is defined below
        //  fn fib(n: Int) -> Int { ... }
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                if let Err(e) = checker.register_function_signature(func) {
                    let diag = type_error_to_diagnostic(&e, Some(self.session));
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        // Pass 4.5: Register extern function signatures (FFI)
        for item in &module.items {
            if let verum_ast::ItemKind::ExternBlock(extern_block) = &item.kind {
                for func in &extern_block.functions {
                    if let Err(e) = checker.register_function_signature(func) {
                        let diag = type_error_to_diagnostic(&e, Some(self.session));
                        self.session.emit_diagnostic(diag);
                    }
                }
            }
        }

        // Pass 4.6: Pre-register const declarations (enables forward references)
        // Constants defined after functions should still be visible in function bodies.
        for item in &module.items {
            if let verum_ast::ItemKind::Const(const_decl) = &item.kind {
                checker.pre_register_const(const_decl);
            }
        }

        // Enable lenient context validation for files with @test annotations.
        // VCS test files use `using [Database]`, `using [Logger]`, etc. but don't
        // define these contexts — they expect a test harness to provide them at runtime.
        // In lenient mode, undefined contexts are silently accepted.
        //

        // Detection: Check source for `// @test:` comment header (VCS convention)
        // or `@test` AST attribute on any item.
        let has_test_annotation = {
            let has_ast_attr = module
                .items
                .iter()
                .any(|item| item.attributes.iter().any(|attr| attr.is_named("test")));
            let has_comment_annotation = module
                .items
                .first()
                .and_then(|item| self.session.get_source(item.span.file_id))
                .map(|sf| {
                    // Check if the first few lines contain `// @test:` header
                    sf.source.as_str().lines().take(10).any(|line| {
                        let trimmed = line.trim();
                        trimmed.starts_with("// @test:") || trimmed.starts_with("// @test ")
                    })
                })
                .unwrap_or(false);
            has_ast_attr || has_comment_annotation
        };
        if has_test_annotation {
            checker.context_resolver_mut().set_lenient_contexts(true);
        }

        // Enable lenient contexts for stdlib files (core/**/*.vr).
        // Stdlib modules reference contexts from sibling modules that
        // may not be fully loaded in single-file check mode. Lenient
        // mode defers method-level validation to the call site where
        // the context is `provide`d with a concrete implementation.
        let is_stdlib_file = self
            .session
            .options()
            .input
            .to_str()
            .map(|p| {
                p.contains("/core/")
                    || p.contains("\\core\\")
                    || p.starts_with("core/")
                    || p.starts_with("core\\")
            })
            .unwrap_or(false);
        if is_stdlib_file {
            checker.context_resolver_mut().set_lenient_contexts(true);
            checker.set_lenient_context_checking(true);
        }

        // For stdlib files, enable lenient mode that persists through
        // all scope changes. This flag is never reset by any operation
        // inside the type checker.
        if is_stdlib_file {
            checker.set_lenient_context_checking(true);
            checker.stdlib_single_file_mode = true;
        }

        // Pass 5: Type check all items (function bodies, impl blocks, etc.)
        for item in module.items.iter() {
            if let Err(type_error) = checker.check_item(item) {
                let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                self.session.emit_diagnostic(diag);
            }
        }

        // Forward type checker diagnostics (exhaustiveness errors, warnings, etc.)
        for diag in checker.diagnostics() {
            self.session.emit_diagnostic(diag.clone());
        }
        checker.clear_diagnostics();

        // Drain deferred verification goals from the type-checker.
        // These are Type::Eq failures that the cubical bridge couldn't
        // resolve and universe constraints the local solver left
        // undecided. Store them on the pipeline for the
        // DependentVerifier phase (Phase 4.4) to consume.
        let deferred_goals = std::mem::take(&mut checker.deferred_verification_goals);
        if !deferred_goals.is_empty() {
            tracing::debug!(
                "Type checker deferred {} verification goal(s) for orchestrator",
                deferred_goals.len()
            );
        }
        self.deferred_verification_goals = deferred_goals;

        let elapsed = start.elapsed();
        let metrics = checker.metrics();

        info!(
            "Type checking completed in {:.2}ms ({} synth, {} check, {} unify)",
            elapsed.as_millis(),
            metrics.synth_count,
            metrics.check_count,
            metrics.unify_count
        );

        if self.session.options().verbose >= 2 {
            debug!("{}", metrics.report());
        }

        // Export type metadata for separate compilation (.vtyp file)
        if self.session.options().emit_types {
            let inherent = checker.get_inherent_methods();
            let methods_guard = inherent.read();
            let mut exporter = verum_types::type_exporter::TypeExporter::new(&methods_guard);
            let module_path = self
                .session
                .options()
                .input
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "module".to_string());
            let exports = exporter.export_module(module, &module_path);
            let data = verum_types::type_exporter::serialize_module_exports(exports);
            if !data.is_empty() {
                let vtyp_path = self.session.options().input.with_extension("vtyp");
                if let Err(e) = std::fs::write(&vtyp_path, &data) {
                    debug!("Failed to write type metadata: {}", e);
                } else {
                    info!(
                        "Exported type metadata: {} ({} bytes)",
                        vtyp_path.display(),
                        data.len()
                    );
                }
            }
            drop(methods_guard);
        }

        // Drain coherence violations downgraded to warnings under
        // `[protocols].coherence = "lenient"`. Strict / unchecked
        // modes leave this empty. Surfacing as Warning-severity
        // diagnostics keeps the user informed without blocking the
        // build — closes the inert-defense pattern around the
        // manifest field by making lenient mode observable.
        for warning in checker.drain_protocol_coherence_warnings() {
            let diag = DiagnosticBuilder::new(Severity::Warning)
                .message(format!("[protocols].coherence=lenient: {}", warning))
                .build();
            self.session.emit_diagnostic(diag);
        }

        // #91/#95 — drain the typechecker-resolved call-target
        // side-table into the pipeline so the caller (which owns
        // `&mut Module`) can stamp the AST via
        // `apply_resolved_call_targets`.  Codegen's
        // `compile_method_call` fast path then picks up
        // `Expr::resolved_call_target` and skips the legacy 7-step
        // cascade in `try_resolve_static_method`.  Empty when no
        // resolution sites have been instrumented yet — codegen
        // falls through to the cascade which is the existing
        // happy-path behaviour.
        self.resolved_call_targets = checker.take_resolved_call_targets();

        // Store the type registry for later use by codegen
        // This enables closure parameter type inference without explicit annotations
        self.type_registry = Some(checker.take_type_registry());

        // Abort if errors
        self.session.abort_if_errors()?;

        Ok(())
    }

    /// #91/#95 — apply the typechecker's resolved-call-target
    /// side-table (drained from `TypeChecker` at the end of
    /// `phase_type_check` into `self.resolved_call_targets`) to the
    /// supplied AST module.  Stamps `Expr::resolved_call_target` for
    /// every `MethodCall` whose span is in the table.  Idempotent;
    /// no-op when the side-table is empty.
    ///
    /// Call this AFTER `phase_type_check` but BEFORE the codegen
    /// phase so the codegen's `compile_method_call` fast path picks
    /// up the resolutions.
    pub(super) fn apply_resolved_call_targets(&self, module: &mut Module) {
        verum_types::apply_resolved_call_targets(module, &self.resolved_call_targets);
    }

    /// Phase 3b: Dependency analysis for embedded constraints
    ///

    /// This phase validates that items are compatible with the target profile's
    /// constraints (no_alloc, no_std, embedded, cbgr_static_only, no_gpu).
    ///

    /// It runs after type checking to ensure all types are resolved before
    /// analyzing their dependency requirements.
    ///

    /// Validates items against target profile constraints (no_alloc, no_std, etc.).
    pub(super) fn phase_dependency_analysis(&self, module: &Module) -> Result<()> {
        use crate::phases::dependency_analysis::DependencyAnalyzer;

        // Get target profile from compiler options
        let profile = self.session.options().to_target_profile();

        // Skip analysis if no constraints are active
        if !profile.no_alloc
            && !profile.no_std
            && !profile.embedded
            && !profile.cbgr_static_only
            && !profile.no_gpu
        {
            debug!("Dependency analysis skipped: no target constraints active");
            return Ok(());
        }

        debug!(
            "Running dependency analysis (profile: {}, no_alloc={}, no_std={}, embedded={})",
            profile.name, profile.no_alloc, profile.no_std, profile.embedded
        );

        let start = Instant::now();
        let mut analyzer = DependencyAnalyzer::new(profile);

        // Analyze all items in the module
        let errors = analyzer.analyze_module(module);

        let elapsed = start.elapsed();

        if !errors.is_empty() {
            // Convert TargetErrors to diagnostics
            let span_converter = |span: verum_ast::Span| -> verum_diagnostics::Span {
                self.session.convert_span(span)
            };

            for error in &errors {
                let diag = error.to_diagnostic(span_converter);
                self.session.emit_diagnostic(diag);
            }

            info!(
                "Dependency analysis completed in {:.2}ms: {} error(s)",
                elapsed.as_millis(),
                errors.len()
            );

            // Record phase metrics before aborting
            self.session
                .record_phase_metrics("Dependency Analysis", elapsed, module.items.len());

            // Abort compilation on target constraint violations
            self.session.abort_if_errors()?;
        } else {
            info!(
                "Dependency analysis completed in {:.2}ms: all items compatible with target",
                elapsed.as_millis()
            );
        }

        // Record phase metrics
        self.session
            .record_phase_metrics("Dependency Analysis", elapsed, module.items.len());

        Ok(())
    }

    /// Phase 4: Refinement verification
    pub(super) fn phase_verify(&self, module: &Module) -> Result<()> {
        let _bc = verum_error::breadcrumb::enter("compiler.phase.verify", "");
        debug!("Running refinement verification");

        let start = Instant::now();
        let _smt_ctx = SmtContext::new();

        // Framework-hygiene preamble (#190): R1+R2+R3 discipline
        // for @framework / @enact annotations. Runs once per module
        // before the per-function refinement loop. R1/R2 produce
        // Warnings (recorded but non-blocking); R3 produces an
        // Error and short-circuits the verify phase.
        {
            let mut hygiene_pass = verum_verification::HygieneRecheckPass::new();
            let mut ctx = verum_verification::VerificationContext::new();
            use verum_verification::VerificationPass;
            let _ = hygiene_pass.run(module, &mut ctx);
            for d in hygiene_pass.diagnostics() {
                let builder = match d.severity {
                    verum_verification::HygieneSeverity::Error => {
                        verum_diagnostics::DiagnosticBuilder::error()
                    }
                    verum_verification::HygieneSeverity::Warning => {
                        verum_diagnostics::DiagnosticBuilder::warning()
                    }
                    verum_verification::HygieneSeverity::Info => {
                        verum_diagnostics::DiagnosticBuilder::warning()
                    }
                };
                let diag = builder
                    .message(format!(
                        "framework-hygiene {}: {}",
                        d.rule,
                        d.message.as_str()
                    ))
                    .build();
                self.session.emit_diagnostic(diag);
            }
            if hygiene_pass.error_count() > 0 {
                debug!(
                    "Framework-hygiene errored ({} R3 violations); \
                     skipping refinement verification",
                    hygiene_pass.error_count()
                );
                return Ok(());
            }
        }

        // Extract functions that need verification (those with refinement types)
        let functions_to_verify: List<_> = module
            .items
            .iter()
            .filter_map(|item| {
                if let verum_ast::ItemKind::Function(func) = &item.kind {
                    // Check if function has refinement types in params or return type
                    let has_refinements = func.params.iter().any(|p| {
                        if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &p.kind {
                            self.has_refinement_type(ty)
                        } else {
                            false
                        }
                    }) || func
                        .return_type
                        .as_ref()
                        .map(|t| self.has_refinement_type(t))
                        .unwrap_or(false);

                    if has_refinements { Some(func) } else { None }
                } else {
                    None
                }
            })
            .collect();

        let num_to_verify = functions_to_verify.len();

        if num_to_verify == 0 {
            debug!("No functions with refinement types found");
            return Ok(());
        }

        info!(
            "Verifying {} function(s) with refinement types",
            num_to_verify
        );

        // ───────────────────────────────────────────────────────────
        // Parallel per-function verification (#100, Z3 + CVC5).
        //

        // Both backends are constructed per call inside
        // `verify_function_refinements` (`SmtContext::with_config(…)`
        // + `SmtRefinementVerifier::with_mode(…)`), so each rayon
        // worker gets its own thread-confined Z3 / CVC5 context. The
        // shared session-level `routing_stats` is `Arc<RoutingStats>`
        // with internally-atomic counters, safe to fan out.
        //

        // Counters and `CostTracker` are accumulated under a single
        // Mutex held only across the per-record append (microsecond-
        // scale) — ~zero contention versus the millisecond/second
        // SMT calls each worker spends.
        //

        // Opt-out: `VERUM_NO_PARALLEL_VERIFY=1` falls back to the
        // sequential loop. Useful for debugging non-deterministic
        // diagnostic ordering or pinning down a parallel-only
        // regression. Closure-cache and routing-stats are unaffected
        // — both are designed for concurrent access.
        // ───────────────────────────────────────────────────────────
        let timeout_ms = self.session.options().smt_timeout_secs * 1000;
        let timeout_secs = self.session.options().smt_timeout_secs;
        let parallel_verify = std::env::var("VERUM_NO_PARALLEL_VERIFY").is_err();

        let work: Vec<&verum_ast::decl::FunctionDecl> =
            functions_to_verify.iter().copied().collect();

        let aggregate = std::sync::Mutex::new((
            0usize, // num_verified
            0usize, // num_failed
            0usize, // num_timeout
            CostTracker::new(),
        ));

        let verify_one = |func: &verum_ast::decl::FunctionDecl| {
            debug!("Verifying function: {}", func.name.name);

            let verify_start = Instant::now();

            // K-rule preamble (#187): walk the function's refinement
            // types and run the kernel rules (currently
            // K-Refine-omega) BEFORE invoking SMT. K-rule failures
            // are hard formation errors per the trusted-base contract;
            // short-circuiting saves the SMT round and surfaces a
            // sharper diagnostic.
            let kernel_outcomes = verum_verification::KernelRecheck::recheck_function(func);
            let mut kernel_failure: Option<(verum_common::Text, String)> = None;
            for (label, outcome) in kernel_outcomes.iter() {
                if let Err(err) = outcome {
                    kernel_failure = Some((label.clone(), format!("{}", err)));
                    break;
                }
            }
            if let Some((label, msg)) = kernel_failure {
                let diag = verum_diagnostics::DiagnosticBuilder::error()
                    .message(format!(
                        "kernel-recheck failed for '{}': {} — {}",
                        func.name.name.as_str(),
                        label.as_str(),
                        msg,
                    ))
                    .build();
                self.session.emit_diagnostic(diag);
                let mut g = aggregate.lock().unwrap();
                g.1 += 1;
                return;
            }

            // Perform actual SMT-based refinement verification.
            let verification_result = self.verify_function_refinements(func, timeout_ms);
            let verify_elapsed = verify_start.elapsed();
            let func_name_text: verum_common::Text = func.name.as_str().to_string().into();

            match verification_result {
                Ok(true) => {
                    debug!(
                        "Verified function '{}' in {:.2}ms",
                        func.name.name,
                        verify_elapsed.as_millis()
                    );
                    let mut g = aggregate.lock().unwrap();
                    g.0 += 1;
                    g.3.record(verum_smt::VerificationCost::new(
                        func_name_text,
                        verify_elapsed,
                        true,
                    ));
                }
                Ok(false) => {
                    let mut g = aggregate.lock().unwrap();
                    g.1 += 1;
                    g.3.record(verum_smt::VerificationCost::new(
                        func_name_text,
                        verify_elapsed,
                        false,
                    ));
                }
                Err(e) => {
                    if verify_elapsed.as_secs() > timeout_secs {
                        warn!("Verification timeout for function: {}", func.name.name);
                        let diag = DiagnosticBuilder::new(Severity::Warning)
                            .message(format!(
                                "Verification timeout for function '{}' ({}s > {}s). Falling back to runtime checks.",
                                func.name.name,
                                verify_elapsed.as_secs(),
                                timeout_secs
                            ))
                            .build();
                        self.session.emit_diagnostic(diag);
                        let mut g = aggregate.lock().unwrap();
                        g.2 += 1;
                        g.3.record(verum_smt::VerificationCost::new(
                            func_name_text,
                            verify_elapsed,
                            false,
                        ));
                    } else {
                        warn!(
                            "Verification error for function '{}': {}",
                            func.name.name, e
                        );
                        let diag = DiagnosticBuilder::new(Severity::Error)
                            .message(format!(
                                "Verification error for function '{}': {}",
                                func.name.name, e
                            ))
                            .build();
                        self.session.emit_diagnostic(diag);
                        let mut g = aggregate.lock().unwrap();
                        g.1 += 1;
                        g.3.record(verum_smt::VerificationCost::new(
                            func_name_text,
                            verify_elapsed,
                            false,
                        ));
                    }
                }
            }
        };

        if parallel_verify && work.len() > 1 {
            use rayon::prelude::*;
            debug!(
                "  Verifying {} functions in parallel (rayon, Z3+CVC5)",
                work.len()
            );
            work.par_iter().copied().for_each(verify_one);
        } else {
            for func in &work {
                verify_one(func);
            }
        }

        let (num_verified, num_failed, num_timeout, cost_tracker) = aggregate.into_inner().unwrap();

        // Phase 4b: Verify theorem/lemma/axiom proofs via SMT
        self.verify_theorem_proofs(module)?;

        // Phase 4c: model-theoretic discharge of protocol axioms at
        // `implement` sites. For every `implement P for T { … }` block in
        // this module, collect P's axioms, Self-substitute into T's
        // concrete items, and discharge via explicit proof clauses or
        // auto_prove. Failures are emitted as warnings by default;
        // callers can elevate to errors via session options once their
        // stdlib surface is fully covered by explicit proofs or
        // SMT-closable obligations.
        self.verify_impl_axioms_for_module(module)?;

        let elapsed = start.elapsed();

        info!(
            "Verification complete: {} verified, {} timeout, {} failed in {:.2}s",
            num_verified,
            num_timeout,
            num_failed,
            elapsed.as_secs_f64()
        );

        if self.session.options().show_verification_costs {
            let report = cost_tracker.report();
            println!("{}", "\nVerification Cost Report:".bold());
            println!("{}", report);
        }

        // Bounds elimination analysis (verum_verification integration)
        // This identifies array accesses that can be proven safe at compile time
        if self.session.options().enable_bounds_elimination {
            self.run_bounds_elimination_analysis(module)?;
        }

        // Auto-route proof certificates when manifest enables them (#285).
        // For each verified theorem/lemma/corollary, emit a stub
        // certificate file in the manifest-selected format. The
        // stub carries the theorem statement with `Admitted.` /
        // `sorry` placeholder body — full proof-term reconstruction
        // is tracked as #285-Followup.
        if self.session.options().emit_proof_certificate {
            self.emit_theorem_certificates(module)?;
        }

        Ok(())
    }

    /// Emit proof-certificate files for every verified theorem-like
    /// item in the module (#285). Format and output directory come
    /// from `CompilerOptions.proof_certificate_format` /
    /// `proof_certificate_path`; defaults are Lean format and
    /// `target/audit-reports/proof-certificates/` directory.
    fn emit_theorem_certificates(&self, module: &Module) -> Result<()> {
        use verum_smt::certificates::{CertificateFormat, CertificateGenerator, Theorem};

        let opts = self.session.options();
        let format_str = opts
            .proof_certificate_format
            .as_ref()
            .map(|t| t.as_str().to_ascii_lowercase())
            .unwrap_or_else(|| "lean".to_string());
        let format = match format_str.as_str() {
            "coq" => CertificateFormat::Coq,
            "lean" => CertificateFormat::Lean,
            "dedukti" => CertificateFormat::Dedukti,
            "metamath" => CertificateFormat::Metamath,
            "opentheory" => CertificateFormat::OpenTheory,
            "json" => CertificateFormat::Json,
            other => {
                warn!(
                    "[proof-certificate] unknown format {:?} — falling back to lean",
                    other
                );
                CertificateFormat::Lean
            }
        };

        let output_dir = match &opts.proof_certificate_path {
            Some(p) => p.clone(),
            None => {
                let project_root = opts
                    .input
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                project_root
                    .join("target")
                    .join("audit-reports")
                    .join("proof-certificates")
            }
        };
        if let Err(e) = std::fs::create_dir_all(&output_dir) {
            warn!(
                "[proof-certificate] could not create {}: {} — \
                 skipping certificate emission",
                output_dir.display(),
                e
            );
            return Ok(());
        }

        let generator = CertificateGenerator::new(format);
        let mut emitted = 0u32;

        for item in &module.items {
            let thm = match &item.kind {
                verum_ast::ItemKind::Theorem(t) => t,
                verum_ast::ItemKind::Lemma(t) => t,
                verum_ast::ItemKind::Corollary(t) => t,
                _ => continue,
            };
            let name: verum_common::Text = thm.name.name.clone();
            let statement: verum_common::Text = format!("{:?}", thm.proposition).into();
            let theorem = Theorem::new(name.clone(), statement);
            match generator.generate_stub(theorem) {
                Ok(cert) => {
                    let mut filename = name.as_str().to_string();
                    filename.push_str(format.extension());
                    let path = output_dir.join(filename);
                    if let Err(e) = std::fs::write(&path, cert.content.as_str()) {
                        warn!(
                            "[proof-certificate] failed to write {}: {}",
                            path.display(),
                            e
                        );
                    } else {
                        emitted += 1;
                        debug!(
                            "[proof-certificate] wrote {} ({})",
                            path.display(),
                            format.name()
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "[proof-certificate] generator rejected theorem {:?}: {}",
                        name.as_str(),
                        e
                    );
                }
            }
        }

        if emitted > 0 {
            info!(
                "[proof-certificate] wrote {} {} certificate(s) to {}",
                emitted,
                format.name(),
                output_dir.display()
            );
        }
        Ok(())
    }

    // verify_impl_axioms_for_module + find_protocol_decl extracted to
    // crate::pipeline::impl_axioms (#106 Phase 3 — pipeline.rs split).
    //

    // verify_theorem_proofs extracted to crate::pipeline::theorem_proofs
    // (#106 Phase 2 — pipeline.rs split).
    //

    // run_bounds_elimination_analysis + analyze_function_bounds_checks
    // + count_index_accesses + count_index_in_expr extracted to
    // crate::pipeline::bounds_stats (#106 Phase 1 — pipeline.rs split).
    //

    // verify_function_refinements + verify_return_refinement_smt +
    // verify_return_expr_smt + extract_return_values +
    // syntactic_check_refinement + extract_pattern_name +
    // has_refinement_type + SmtCheckResult extracted to
    // crate::pipeline::refinement_verify (#106 Phase 4 — pipeline.rs split).

    // check_profile_boundaries + extract_module_profile +
    // extract_profile_name + import_to_module_path +
    // get_module_profile_from_registry + type_to_text extracted to
    // crate::pipeline::profile_boundaries (#106 Phase 7).
    //

    // check_protocol_coherence + register_module_coherence_items
    // extracted to crate::pipeline::coherence (#106 Phase 6).
    //

    // extract_cfg_predicates / cfg_expr_to_predicate / expr_to_ident_string /
    // expr_to_string_literal extracted (and were previously duplicated) to
    // crate::cfg_eval — single source of truth (#106 Phase 5).

    // ==================== Phase 7: VBC Execution (Two-Tier Model v2.1) ====================
    //

    // This section handles the final execution step for VBC modules:
    // - Tier 0 (Interpreter): Direct VBC interpretation via `phase_interpret()`
    // - Tier 1 (AOT): VBC → LLVM IR → Native via `run_native_compilation()`
    //  VBC → MLIR → GPU binaries via `run_mlir_aot()` (for @device(GPU))
    //

    // Architecture:
    // ```
    // Monomorphized VBC Module
    //  │
    //  ├─── Tier 0 ──► verum_vbc::interpreter::Interpreter
    //  │ • ~1ms startup, ~20ns/call
    //  │ • Full CBGR safety checks
    //  │ • Used for: dev, REPL, debugging
    //  │
    //  └─── Tier 1 ──► CPU: VbcToLlvmLowering → LLVM IR → Native
    //  GPU: VbcToMlirGpuLowering → MLIR → PTX/HSACO
    //  • ~1s startup, ~1ns/call
    //  • Proven-safe checks eliminated (0ns)
    //  • Used for: production builds
    // ```
    //

    // Performance Characteristics:
    // | Tier | Startup | Runtime | Check Elimination |
    // |-------------|---------|------------|-------------------|
    // | Interpreter | ~1ms | ~20ns/call | None (full CBGR) |
    // | AOT (CPU) | ~1s | ~1ns/call | 50-90% typical |
    //

    // Two-tier execution: Interpreter (fast startup, full CBGR) and AOT (LLVM, 50-90% check elimination).

    /// Phase 4b: FFI Boundary Validation
    /// Phase 4c: Context System Validation
    ///

    /// Validates context usage: undeclared contexts, unprovided contexts,
    /// negative context violations (direct + transitive), and conflicts.
    /// Runs as warnings for now (errors would break existing code that
    /// doesn't yet declare all contexts).
    pub(super) fn phase_context_validation(&self, module: &Module) {
        use crate::phases::context_validation::ContextValidationPhase;

        // Gate on the unified language-feature flag.
        // `[context] enabled = false` disables the whole DI/context
        // system, so there's nothing to validate.
        if !self.session.language_features().context_system_on() {
            return;
        }

        let t0 = Instant::now();
        // [context].unresolved_policy: "error" (default) → emit
        // errors; "warn" → downgrade to warnings; "allow" → suppress.
        let policy = self
            .session
            .language_features()
            .context
            .unresolved_policy
            .as_str()
            .to_string();

        let phase = ContextValidationPhase::new();
        match phase.validate_module_public(module) {
            Ok(warnings) => {
                let elapsed = t0.elapsed();
                if !warnings.is_empty() && policy != "allow" {
                    info!(
                        "Context validation: {} warnings ({:.2}ms)",
                        warnings.len(),
                        elapsed.as_secs_f64() * 1000.0
                    );
                    for w in warnings.iter() {
                        warn!("Context: {}", w.message());
                    }
                }
            }
            Err(errors) => {
                match policy.as_str() {
                    "allow" => {
                        // Silently suppress all context errors.
                    }
                    "warn" => {
                        for e in errors.iter() {
                            warn!("Context (downgraded to warning): {}", e.message());
                        }
                    }
                    _ => {
                        // "error" (default) — report as-is.
                        for e in errors.iter() {
                            warn!("Context: {}", e.message());
                        }
                    }
                }
            }
        }
    }

    /// Phase 4d: Send/Sync compile-time enforcement
    ///

    /// Validates that types crossing thread boundaries (spawn, Channel.send, Shared)
    /// satisfy Send/Sync bounds. Emits warnings (not errors) for now.
    pub(super) fn phase_send_sync_validation(&self, module: &Module) {
        use crate::phases::send_sync_validation::SendSyncValidationPhase;

        let phase = SendSyncValidationPhase::new();
        let warnings = phase.validate_module(module);
        if !warnings.is_empty() {
            for w in warnings.iter() {
                warn!("Send/Sync: {}", w.message());
            }
        }
    }

    /// Phase 4b: FFI Boundary Validation
    ///

    /// `extern {}` blocks: warn-only (stdlib compatibility).
    /// `ffi {}` blocks: strict errors (user-written contracts must be correct).

    pub(super) fn phase_ffi_validation(&self, module: &Module) -> Result<()> {
        use crate::phases::ffi_boundary::validate_module_ffi;
        let t0 = Instant::now();
        let result = validate_module_ffi(module, false);
        let elapsed = t0.elapsed();
        if result.functions_validated > 0 {
            info!(
                "FFI validation: {} extern blocks, {} ffi boundaries, {} functions ({:.2}ms, {} diagnostics)",
                result.extern_blocks_validated,
                result.ffi_boundaries_validated,
                result.functions_validated,
                elapsed.as_secs_f64() * 1000.0,
                result.diagnostics.len()
            );
        }
        // Warnings (from extern blocks)
        for diag in result
            .diagnostics
            .iter()
            .filter(|d| d.severity() != Severity::Error)
        {
            warn!("FFI: {}", diag.message());
        }
        // Errors (from ffi blocks) — fail compilation
        let errors: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.severity() == Severity::Error)
            .collect();
        if !errors.is_empty() {
            let mut msg = format!("{} FFI type safety error(s):\n", errors.len());
            for err in &errors {
                msg.push_str(&format!("  - {}\n", err.message()));
            }
            return Err(anyhow::anyhow!("{}", msg));
        }
        Ok(())
    }
}

/// Seed user-side typechecker with blanket protocol impls from the
/// embedded stdlib `core/base/protocols.vr` source.  The archive-
/// driven path builds a synthetic empty AST, so the typechecker's
/// `register_module_blanket_impls` AST walker sees no impls.  Mirrors
/// codegen-side `seed_protocol_registry_from_embedded_stdlib`.
///
/// Idempotent — `register_module_blanket_impls` short-circuits via
/// `blanket_impls_registered_modules`.
fn seed_typechecker_blanket_impls(checker: &mut TypeChecker) {
    use std::sync::OnceLock;
    use verum_ast::Module as AstModule;

    const SEED_PATHS: &[&str] = &["base/protocols.vr"];

    static SEED_MODULES: OnceLock<Vec<AstModule>> = OnceLock::new();
    let modules = SEED_MODULES.get_or_init(|| {
        let stdlib = match crate::embedded_stdlib::get_embedded_stdlib() {
            Some(s) if s.file_count() > 0 => s,
            _ => return Vec::new(),
        };
        SEED_PATHS
            .iter()
            .filter_map(|rel| {
                let src = stdlib.get_file(rel)?;
                let mut parser = verum_fast_parser::Parser::new(src);
                match parser.parse_module() {
                    Ok(m) => Some(m),
                    Err(e) => {
                        tracing::warn!(
                            target: "phases_orchestration",
                            "blanket-impl seed failed to parse `{}`: {:?}",
                            rel, e,
                        );
                        None
                    }
                }
            })
            .collect()
    });

    for ast_module in modules {
        // Register protocol type definitions FIRST so their default
        // methods (e.g. `PartialEq.ne(&self, other) -> Bool { !self.eq(other) }`)
        // are in the protocol_checker.method_registry before any
        // dispatch site queries it.  Without this, `v1.ne(&v2)` on a
        // user PartialEq impl fails MethodNotFound despite the impl.
        checker.register_module_protocols(ast_module, "core.base.protocols");
        checker.register_module_blanket_impls(ast_module, "core.base.protocols");
    }
}

