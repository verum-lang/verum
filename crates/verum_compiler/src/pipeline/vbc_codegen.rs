//! AST → VBC codegen orchestration + import resolution.
//!

//! Extracted from `pipeline.rs` (#106 Phase 10). Houses the core
//! "build the runtime VBC module from a typed AST" pipeline step,
//! plus the module-graph helpers it depends on:
//!

//!  * `compile_ast_to_vbc` — primary orchestrator: runs
//!  dependent-type verification, proof erasure, CBGR tier
//!  analysis, VBC codegen + monomorphisation, retains
//!  stdlib-imported modules for cross-module symbol lookups.
//!  * `collect_imported_stdlib_modules` — transitive `mount`
//!  closure walker that pulls every stdlib module reachable
//!  from the user module's import graph (the foundation of
//!  lazy stdlib loading).
//!  * `resolve_super_path` — `super.*` / `.X` mount-path
//!  resolution helper.
//!  * `clear_non_compilable_stdlib_modules` — selective module
//!  retention after type checking; drops prelude /
//!  protocol-definition modules that would introduce
//!  unresolvable cross-module method references at codegen,
//!  while keeping modules whose function bodies need to be
//!  compiled to VBC (collections, sync, text, io, mem, etc.).

use std::sync::Arc;

use anyhow::Result;
use tracing::{debug, info, warn};

use verum_ast::Module;
use verum_common::{Map, Text};
use verum_vbc::codegen::{CodegenConfig, TierContext, VbcCodegen};
use verum_vbc::module::VbcModule;

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    /// Compile AST module to VBC module.
    pub(super) fn compile_ast_to_vbc(&self, module: &Module) -> Result<Arc<VbcModule>> {
        // Phase 4.4: Dependent-type verification at module boundary.
        // The `DependentVerifier` orchestrator dispatches accumulated
        // goals (cubical equality, universe constraints, sheaf descent,
        // epistemic invariants) and checks instance coherence across
        // all `implement P for T` blocks in the module. This runs
        // *before* proof erasure so that theorems, axioms, and proof
        // bodies are still available for verification.
        //

        // The orchestrator is fire-and-report: it does not block
        // compilation on verification failure — diagnostics are
        // emitted, and the pipeline continues. This matches the
        // gradual-verification philosophy: `@verify(formal)` goals
        // that fail are reported but do not prevent `@verify(runtime)`
        // code from compiling.
        {
            use verum_verification::dependent_verification::DependentVerifier;
            let mut verifier = DependentVerifier::new();

            // Instance coherence: scan implement blocks for
            // duplicate protocol implementations on the same type.
            for item in module.items.iter() {
                if let verum_ast::decl::ItemKind::Impl(impl_block) = &item.kind {
                    if let verum_ast::decl::ImplKind::Protocol {
                        protocol, for_type, ..
                    } = &impl_block.kind
                    {
                        let protocol_name = protocol
                            .segments
                            .iter()
                            .map(|s| match s {
                                verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                                _ => "_",
                            })
                            .collect::<Vec<_>>()
                            .join(".");
                        let type_name = format!("{:?}", for_type.kind);
                        let location = format!("module:{}", item.span.start);

                        verifier.instance_registry_mut().register(
                            verum_types::instance_search::InstanceCandidate::new(
                                protocol_name,
                                type_name,
                            )
                            .at(location),
                        );
                    }
                }
            }

            // Feed deferred verification goals from the type-checker
            // into the orchestrator. These are Type::Eq failures that
            // the cubical bridge couldn't resolve and universe
            // constraints the local solver left undecided.
            for goal in self.deferred_verification_goals.iter() {
                match goal {
                    verum_types::infer::DeferredVerificationGoal::CubicalEquality {
                        lhs,
                        rhs,
                        ..
                    } => {
                        use verum_types::cubical_bridge::eq_to_cubical;
                        verifier.add_goal(
                            verum_verification::dependent_verification::DependentGoalKind::CubicalEquality {
                                lhs: eq_to_cubical(lhs),
                                rhs: eq_to_cubical(rhs),
                            },
                        );
                    }
                    verum_types::infer::DeferredVerificationGoal::UniverseConstraints {
                        constraints,
                    } => {
                        verifier.add_goal(
                            verum_verification::dependent_verification::DependentGoalKind::UniverseConstraints(
                                constraints.clone(),
                            ),
                        );
                    }
                }
            }

            let report = verifier.verify_all();
            if !report.is_all_good() {
                tracing::warn!(
                    "Dependent verification: {} verified, {} refuted, {} undetermined, coherence {}",
                    report.verified_count(),
                    report.refuted_count(),
                    report.undetermined_count(),
                    if report.coherence.is_coherent() {
                        "clean"
                    } else {
                        "violated"
                    },
                );
            }
        }

        // Phase 4.5: Proof erasure — strip all proof-level items (theorem,
        // lemma, corollary, axiom, tactic) before VBC codegen. This formally
        // enforces the VBC-first architecture invariant that runtime carries
        // zero proof-term overhead. The VBC codegen itself has a defensive
        // skip for the same item kinds, but doing it upstream keeps the
        // module in a canonical runtime-only form.
        //

        // Gated on [codegen].proof_erasure. When the flag is disabled,
        // proof terms survive into VBC and become runtime values — used
        // by research scenarios that inspect the proof witness at
        // runtime. Default: true (production path).
        let proof_erasure_on = self.session.language_features().codegen.proof_erasure;
        let erased_module = if proof_erasure_on {
            let (m, erasure_stats) =
                crate::phases::proof_erasure::erase_proofs_from_module(module.clone());
            if erasure_stats.total_erased() > 0 {
                tracing::debug!(
                    "Proof erasure: {} proof items stripped before VBC codegen",
                    erasure_stats.total_erased()
                );
            }
            m
        } else {
            tracing::debug!(
                "Proof erasure SKIPPED ([codegen] proof_erasure = false); \
                 proof terms will survive into VBC"
            );
            module.clone()
        };
        let module = &erased_module;

        // Get profile from session and configure VBC codegen accordingly
        let profile = self.session.options().profile;
        let config = CodegenConfig {
            module_name: "main".to_string(),
            debug_info: self.session.options().debug_info,
            optimization_level: 0,
            // Default-off (matches CodegenConfig::default()) — the
            // structural validator currently rejects ~8000 dangling
            // `TypeId(515)` (= Maybe) references emitted by the
            // stdlib's pre-existing well-known-type emit gap (see
            // `codegen/mod.rs:480` comment block). With `validate:
            // true` here, every `verum run` aborts at codegen-finalize
            // before reaching the interpreter — silently breaking
            // hello-world and downstream weft probes. CI / release
            // paths still flip the gate on through
            // `CodegenConfig::with_validation()`. Keeping it off in
            // the user-facing pipeline is consistent with the existing
            // "Default lenient" architectural intent below for
            // `strict_codegen`.
            validate: false,
            source_map: false,
            target_config: verum_ast::cfg::TargetConfig::host(),
            // V-LLSI profile configuration
            is_interpretable: profile.is_vbc_interpretable(),
            is_systems_profile: profile == crate::profile_system::Profile::Systems,
            is_embedded: self.session.options().is_embedded(),
            // #110 strict-codegen plumbing: read from
            // `LintConfig.strict_codegen`. Default remains lenient
            // (the field defaults to false) so existing pipelines
            // are unaffected. CI / release / `--strict-codegen` CLI
            // flag flip it on — bug-class skips
            // (UndefinedFunction, WrongArgumentCount, TypeMismatch,
            // …) become hard errors. `Irreducible` skips
            // (UnsupportedExpr, NotImplemented, …) remain debug
            // traces regardless — those are interpreter limitations,
            // not codebase defects.
            strict_codegen: self.session.options().lint_config.strict_codegen,
        };

        let mut codegen = VbcCodegen::with_config(config);

        // #122 — bridge stdlib-prep registry into the user-side codegen.
        //
        // Pre-fix the user-side `VbcCodegen` started with an empty
        // `ctx.functions` HashMap. Stdlib functions registered in the
        // earlier `Phase0CoreCompiler::compile_core` pass (which uses
        // its own `VbcCodegen` instance, exports to
        // `pipeline.global_function_registry`) were invisible to user
        // code's compile_call lookups — every user call to a stdlib
        // intrinsic (`write_stderr`, `random_bytes`, `exit_process`,
        // etc.) surfaced as `[lenient] SKIP fn caller (bug-class):
        // undefined function: write_stderr`.
        //
        // The fix is to feed the global function registry the same
        // way stdlib_bootstrap.rs:919 does for cross-stdlib-module
        // compilation. After this call every symbol that any earlier
        // codegen instance registered (including `@intrinsic` extern
        // declarations and `mount X.Y as Z` aliases) is visible to
        // the user-side compile_item / compile_call path.
        if !self.global_function_registry.is_empty() {
            codegen.import_functions(&self.global_function_registry);
        }
        if !self.global_protocol_registry.is_empty() {
            codegen.import_protocols(&self.global_protocol_registry);
        }

        // Run CBGR tier analysis: escape analysis → tier determination
        // → RefChecked/RefUnsafe emission. Promotes non-escaping refs
        // from Tier 0 (~15ns) to Tier 1 (0ns).
        //

        // #118 — correctness fix + parallelisation.
        //

        // Pre-fix the merge loop iterated `0..func_tc.decision_count()`
        // and constructed `ExprId(i)` for `i in 0..N`, but
        // `from_analysis_result` populates decisions with span-encoded
        // `ExprId(start<<32|end)` keys. The `get_tier(ExprId(i))`
        // lookup always missed and the merge silently inserted only
        // `default_tier` (Tier0). CBGR tier promotion was therefore
        // NEVER applied to user code — every reference got Tier 0
        // CBGR overhead at runtime, defeating the language's headline
        // memory-safety/perf trade-off. Now we use the canonical
        // `iter_decisions()` API so real ExprId-keyed promotions reach
        // codegen. Per-function analyses also fan out via rayon —
        // each `TierAnalyzer` is independent, results merge into the
        // module-level `TierContext` under a single Mutex held only
        // for the per-function append (microsecond-scale).
        let tier_start = std::time::Instant::now();
        let tier_context = {
            use crate::phases::cfg_constructor::CfgConstructor;
            use verum_cbgr::tier_analysis::TierAnalyzer;
            let module_cfg = CfgConstructor::from_module(module);
            let parallel_tier = std::env::var("VERUM_NO_PARALLEL_TIER").is_err();

            let aggregate = std::sync::Mutex::new(TierContext::new());
            let analyse_one = |func_cfg: &crate::phases::cfg_constructor::FunctionCfg| {
                let analyzer = TierAnalyzer::with_config(
                    func_cfg.cfg.clone(),
                    verum_cbgr::tier_analysis::TierAnalysisConfig::minimal(),
                );
                let analysis_result = analyzer.analyze();
                let func_tc = TierContext::from_analysis_result(&analysis_result);
                let mut g = aggregate.lock().unwrap();
                g.merge_from(&func_tc);
            };

            if parallel_tier && module_cfg.functions.len() > 1 {
                use rayon::prelude::*;
                let cfgs: Vec<_> = module_cfg.functions.values().collect();
                cfgs.par_iter().for_each(|func_cfg| analyse_one(func_cfg));
            } else {
                for (_func_id, func_cfg) in module_cfg.functions.iter() {
                    analyse_one(func_cfg);
                }
            }

            let mut tc = aggregate.into_inner().unwrap();
            tc.enabled = true;
            tc
        };
        codegen.set_tier_context(tier_context);
        debug!(
            "tier analysis in compile_ast_to_vbc took {:.2}s",
            tier_start.elapsed().as_secs_f64()
        );

        // Single-path archive-driven epic (T2): the embedded
        // precompiled stdlib `VbcArchive` is the ONLY source of stdlib
        // types and function info.  Source-driven codegen of stdlib
        // is removed entirely — there are no alternative paths.
        //
        // Production builds embed the archive via `build.rs`.  If
        // it's absent (only happens during compiler bootstrap before
        // T3/T4 land), error out loudly: building user code without
        // a stdlib archive is a configuration error, not a fallback
        // we should silently paper over.
        let archive = crate::embedded_stdlib_vbc::get_runtime_archive().ok_or_else(|| {
            anyhow::anyhow!(
                "no precompiled stdlib archive embedded in this verum binary — \
                 single-path archive-driven codegen requires `target/precompiled-stdlib/runtime.vbca`. \
                 Run `verum stdlib precompile` and rebuild verum to embed the archive."
            )
        })?;
        if std::env::var("VERUM_TRACE_CODEGEN_PATH").is_ok() {
            eprintln!(
                "[compile_ast_to_vbc] archive-driven: {} modules, {} KB",
                archive.module_count(),
                crate::embedded_stdlib_vbc::embedded_size_bytes() / 1024,
            );
        }

        // No source-driven imported_modules collection — stdlib
        // types/functions come from the archive via T1 below.  The
        // user module is the ONLY AST that goes through codegen.

        // Archive-driven single-path: pre-populate codegen ctx from
        // embedded archive (T1+T2), then compile ONLY the user
        // module.  Stdlib types/functions come from the archive —
        // no source-driven walk of stdlib `.vr` files anywhere.
        codegen.initialize();
        // Built-in core variants (Maybe.Some / Result.Ok /
        // Ordering.Lt etc.) — compiler intrinsics with hardcoded
        // tags, not part of the archive.  Run before archive
        // population so any archive-side variant ctor with the same
        // simple name yields to the built-in via first-wins.
        codegen.register_builtin_variants();
        codegen.register_stdlib_constants();
        codegen.register_stdlib_intrinsics();
        codegen.register_runtime_io_functions();

        // T1 archive → ctx, lazy mount-driven registration.  Walks the
        // user `Module`'s `mount` declarations + transitively-required
        // names, registers ONLY those FunctionInfo entries from the
        // archive.  For a hello.vr that mounts ~5 stdlib symbols, this
        // touches ~5 of the 7484 archive entries — typically <1ms.
        //
        // The full table is still built lazily on demand via
        // `apply_lazy`'s fallback path (codegen's
        // `find_function_by_suffix` redirect chain triggers
        // re-registration on miss).  Cost amortises across
        // compilations within the same process for REPL / watch /
        // test-runner workflows.
        static CTX_CACHE: crate::archive_ctx_loader::ArchiveCtxCache =
            crate::archive_ctx_loader::ArchiveCtxCache::new();
        let t_pop = std::time::Instant::now();
        // Split borrows so apply_lazy can REMAP each archive-local
        // FunctionId to a globally-unique slot.  Without this, two
        // archive modules with overlapping local ids (canonical
        // example: `core.text.Text.trim_end_matches` and
        // `core.shell.script.args` both at id=0 within their
        // respective module-local function tables) collapse onto a
        // single `ctx.functions` slot — `emit_missing_stub_descriptors`
        // then emits exactly one stub for id=0 with whichever name
        // wins the longest-dotted tiebreak, and Call(0) at runtime
        // dispatches every call site through that one name's
        // intercept (or, when no intercept matches, returns Unit).
        // Combined archive load: function table + type table in a
        // single walk, so each archive module is decoded exactly once
        // (apply_lazy + import_types_for_module previously decoded the
        // same module twice — measurable cold-start regression on
        // scripts with deep mount trees).  Function/id remap and
        // type-side first-wins discipline are layered inside.
        let (fn_modules, type_modules) = CTX_CACHE.apply_lazy_with_types(
            archive,
            &mut codegen,
            module,
        );
        tracing::debug!(
            target: "compile_ast_to_vbc",
            "archive lazy apply (combined): {} fn-modules + {} type-modules in {:.2}ms",
            fn_modules,
            type_modules,
            t_pop.elapsed().as_secs_f64() * 1000.0,
        );
        tracing::debug!(
            target: "compile_ast_to_vbc",
            "archive lazy pre-population in {:.2}ms",
            t_pop.elapsed().as_secs_f64() * 1000.0,
        );
        if std::env::var("VERUM_TRACE_CODEGEN_PATH").is_ok() {
            eprintln!(
                "[compile_ast_to_vbc] T1 lazy apply: {:.2}ms",
                t_pop.elapsed().as_secs_f64() * 1000.0
            );
        }

        // User module: protocols + declarations + bodies.  Stdlib
        // walking is gone entirely.
        let trace_path = std::env::var("VERUM_TRACE_CODEGEN_PATH").is_ok();
        let t_user = std::time::Instant::now();
        codegen.collect_protocol_definitions(module);
        codegen
            .collect_non_protocol_declarations(module)
            .map_err(|e| {
                anyhow::anyhow!("VBC codegen error (user declarations): {}", e)
            })?;
        codegen.mark_user_defined_types(module);
        codegen.resolve_pending_imports();
        codegen
            .compile_pending_default_methods()
            .map_err(|e| {
                anyhow::anyhow!("VBC codegen error (default methods): {}", e)
            })?;
        codegen.set_propagate_test_attr(true);
        codegen
            .compile_module_items(module)
            .map_err(|e| anyhow::anyhow!("VBC codegen error (user bodies): {}", e))?;
        if trace_path {
            eprintln!(
                "[compile_ast_to_vbc] user codegen: {:.2}ms",
                t_user.elapsed().as_secs_f64() * 1000.0
            );
        }
        let t_fin = std::time::Instant::now();
        let mut vbc_module = codegen
            .finalize_module()
            .map_err(|e| anyhow::anyhow!("VBC codegen error (finalize): {}", e))?;
        if trace_path {
            eprintln!(
                "[compile_ast_to_vbc] finalize_module: {:.2}ms",
                t_fin.elapsed().as_secs_f64() * 1000.0
            );
        }

        // Set source directory for FFI library path resolution
        // Use the parent directory of the input file, or current directory if none
        let input_path = &self.session.options().input;
        let source_dir = if input_path.is_file() {
            input_path
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
        } else {
            Some(input_path.to_string_lossy().into_owned())
        };
        vbc_module.source_dir = source_dir;

        // Phase 6d: optional linker-merge with the embedded
        // precompiled stdlib archive. Gated on
        // `VERUM_LINKER_MERGE=1` for opt-in testing — production
        // default-on switch follows once we've validated end-to-end
        // dispatch on a script that exercises the merge boundary.
        // The linker round-trips deterministically and is verified
        // by `crates/verum_compiler::embedded_stdlib_vbc::tests::
        // linker_round_trip_through_embedded_archive`.
        //
        // Phase 7 (#precompile-stdlib epic): cross-compile path.
        // When `--target X` is explicitly set, the embedded archive
        // already carries per-target variants (cfg_keys +
        // function_variants), so route through the linker for archive-
        // wide variant pick instead of the source-driven path. This
        // is the "no per-target filesystem cache" property — same
        // embedded archive serves every triple via cfg_key matching.
        // Auto-trigger when a target triple is set; opt-in
        // VERUM_LINKER_MERGE=1 still works for host-target testing.
        let cross_compile = self.session.options().target_triple.is_some();
        let linker_merge = std::env::var("VERUM_LINKER_MERGE").is_ok() || cross_compile;
        if linker_merge {
            if let Some(archive) = crate::embedded_stdlib_vbc::get_runtime_archive() {
                let target_triple = self
                    .session
                    .options()
                    .target_triple
                    .as_ref()
                    .map(|t| t.as_str().to_string())
                    .unwrap_or_else(|| std::env::consts::ARCH.to_string()
                        + "-"
                        + std::env::consts::OS);
                let mut linker = verum_vbc::linker::VbcLinker::new(&target_triple);
                if let Err(e) = linker.add_archive(archive) {
                    tracing::warn!(
                        target: "compile_ast_to_vbc",
                        "VERUM_LINKER_MERGE: archive merge failed ({}); falling back to source-driven codegen",
                        e
                    );
                } else if let Err(e) = linker.add_user_module(vbc_module.clone()) {
                    tracing::warn!(
                        target: "compile_ast_to_vbc",
                        "VERUM_LINKER_MERGE: user module merge failed ({}); falling back to source-driven codegen",
                        e
                    );
                } else {
                    let merged = linker.finalize();
                    tracing::info!(
                        target: "compile_ast_to_vbc",
                        "VERUM_LINKER_MERGE: merged {} stdlib modules + user module — {} functions, {} types",
                        archive.module_count(),
                        merged.functions.len(),
                        merged.types.len(),
                    );
                    return Ok(std::sync::Arc::new(merged));
                }
            }
        }

        Ok(std::sync::Arc::new(vbc_module))
    }


    /// Retain stdlib modules that contain compilable function bodies.
    ///

    /// After type-checking, we clear modules whose ASTs are no longer needed.
    /// Modules with function implementations (function bodies with statements)
    /// are retained so their bodies can be compiled to VBC → LLVM.
    /// Modules containing only type/protocol declarations are cleared — their
    /// type information was already extracted during type-checking.
    ///

    /// `user_module`, when provided, is scanned for `mount` statements and any
    /// stdlib modules matching the mount target (plus their submodules) are
    /// retained. Without this, user code that mounts a stdlib module outside
    /// the `ALWAYS_INCLUDE` allowlist (e.g. `mount core.term.layout.Rect`)
    /// would lose the impl-block ASTs before VBC codegen runs, the impl
    /// methods would never reach `compile_module_items_lenient`, and the
    /// LLVM backend would emit unresolved `Call`s that const-fold to bogus
    /// pointers — the bug tracked by `vcs/specs/L0-critical/vbc/aot_stdlib_return/`.
    pub(super) fn clear_non_compilable_stdlib_modules(&mut self, user_module: Option<&Module>) {
        // Only retain stdlib modules whose functions are actually compiled to
        // native code via VBC → LLVM. Most stdlib modules only provide type
        // definitions used during type checking and should be dropped to avoid
        // compiling thousands of unreachable functions.
        //

        // Modules in ALWAYS_INCLUDE have compiled implementations that the AOT
        // pipeline dispatches to (Strategy 1/2 in instruction.rs).
        const ALWAYS_INCLUDE: &[&str] = &[
            // Platform sys modules — retained so that mount aliases
            // (e.g., futex_wait as sys_futex_wait) can resolve across modules.
            // libsystem/syscall MUST be retained for pthread FFI declarations
            // that thread/mutex/condvar modules import via `mount super.libsystem.{...}`.
            "core.sys.darwin.libsystem",
            "core.sys.darwin.thread",
            "core.sys.linux.syscall",
            "core.sys.linux.thread",
            // T0.6.1 — per-platform TLS providers: thread_entry's
            // create_thread_tls call resolves through these (one
            // platform module wins the conditional cfg). Pre-T0.6.1
            // they were missing from the AOT-retention list and the
            // top-level `thread_entry` function bug-class lenient-
            // SKIPped, leaving the runtime entry without thread-local
            // storage init.
            "core.sys.darwin.tls",
            "core.sys.linux.tls",
            // Collections
            "core.collections.list",
            "core.collections.map",
            "core.collections.set",
            "core.collections.deque",
            "core.collections.heap",
            "core.collections.btree",
            "core.collections.slice",
            // Text
            "core.text.text",
            "core.text.char",
            "core.text.format",
            // Base types
            "core.base.maybe",
            "core.base.result",
            "core.base.ordering",
            // T0.6.1 — typed-OOM allocation primitives. core.base.memory
            // hosts try_alloc / try_alloc_zeroed / try_realloc that
            // List / Map / Text / Deque / Heap call from
            // try_with_capacity / try_grow / try_resize. Without retention
            // here, the AOT cull dropped memory's AST before VBC codegen
            // and every fallible-allocation API ended up bug-class
            // lenient-SKIPped (#200 follow-up; companion to the
            // type-checking ALWAYS_INCLUDE entry around line 11061).
            "core.base.memory",
            // Time
            "core.time.duration",
            // Sync
            "core.sync.mutex",
            "core.sync.condvar",
            "core.sync.rwlock",
            "core.sync.semaphore",
            "core.sync.barrier",
            "core.sync.once",
            "core.sync.atomic",
            // Async
            "core.async.channel",
            "core.async.generator",
            // spawn_with and parallel excluded from AOT retention:
            // Their free functions (execute_with_retry, parallel_map) have
            // common names that collide with user code. LLVMAddFunction returns
            // the existing function for same-name same-arity functions, causing
            // body corruption. User code that redefines these functions works
            // correctly without the stdlib versions.
            // "core.async.spawn_with",
            // "core.async.parallel",
            // Runtime context bridge for AOT
            "core.runtime.ctx_bridge",
            // Memory / CBGR allocator — required for user code that
            // constructs Shared<T>, Weak<T>, or other CBGR-tracked
            // reference types. Without these, the stdlib Shared::new
            // call site resolves at type-check time but has no body at
            // codegen, producing "undefined function: get_heap" errors.
            // See KNOWN_ISSUES.md "Shared<T> / CBGR-allocator Bootstrap".
            "core.mem.allocator",
            "core.mem.heap",
            // Capability-audit substrate (#202). MUST be retained
            // alongside `core.mem.header` because every CBGR writer
            // entry point (try_revoke / attenuate_capabilities /
            // increment_ref_count / decrement_ref_count /
            // increment_generation) emits a `record_*` call into the
            // audit ring. Without these in the retained set, the
            // codegen skips the writer methods (bug-class) and CBGR
            // primitives have no working bodies.
            "core.mem.cap_audit_ring",
            "core.mem.cap_audit",
            "core.mem.header",
            "core.mem.thin_ref",
            "core.mem.fat_ref",
            "core.mem.epoch",
            "core.mem.size_class",
            "core.mem.capability",
            "core.mem.raw_ops",
            // T0.6.1 — segment-classification constants used by
            // LocalHeap.alloc_huge / LocalHeap.free for the
            // SEGMENT_HUGE branch. Without retention, both methods
            // bug-class lenient-SKIP because SEGMENT_HUGE resolves
            // as undefined at codegen.
            "core.mem.segment",
            // I/O — excluded from AOT retention: core.io.fs read()/write()
            // FFI declarations conflict with LLVM builtins (wrong arg count).
            // Included in the type-checking ALWAYS_INCLUDE list (list 1) above.
            // #122 — see list 1 for the full rationale; this list governs
            // which modules survive the post-typecheck cull, and panic.vr
            // / runtime.os transitively must be retained for the codegen
            // to register `write_stderr` / `exit_process` in
            // ctx.functions for the user-side compile pass.
            "core.base.panic",
            "core.intrinsics.runtime.os",
            // #122 — `core.sys.common` hosts `random_bytes` /
            // `read` / `write` / `pread` / `pwrite` / locking / sync
            // primitives used as aliased mounts across the stdlib
            // (Ed25519/X25519/ULID/nanoid/backoff CSPRNG seeding,
            // io.fs writers, sync.condvar). Pre-fix the post-
            // typecheck retention culled this module → its function
            // bodies never compiled → `register_import_aliases` for
            // any consumer's `mount core.sys.common.random_bytes
            // as sys_random_bytes` couldn't resolve the qualified
            // target → alias never registered → bug-class lenient
            // SKIP for every fill_random / OS-syscall site.
            "core.sys.common",
        ];

        // Collect user-mounted module paths so their ASTs survive the cull.
        // A user `mount core.term.layout.Rect` should keep `core.term.layout`,
        // `core.term.layout.rect` (the module the type lives in), and every
        // other submodule under the mounted prefix that participates in type
        // resolution. We do not filter by AST shape here — the per-item
        // lenient compilation already skips functions whose bodies fail to
        // compile, so a few extra type-only modules cost nothing.
        let mut user_mount_prefixes: Vec<String> = Vec::new();
        if let Some(module) = user_module {
            for item in &module.items {
                if let verum_ast::ItemKind::Mount(mount_decl) = &item.kind {
                    let parent = self.extract_import_module_path(&mount_decl.tree.kind);
                    if !parent.is_empty() {
                        user_mount_prefixes.push(parent);
                    }
                    // Also remember the *full* mounted path (incl. last
                    // segment). `extract_import_module_path` strips the last
                    // segment because it usually names an item rather than a
                    // module, but when the mount targets a module directly
                    // (e.g. `mount core.sys.darwin.libsystem`) the full path
                    // is the one we need.
                    use verum_ast::MountTreeKind;
                    if let MountTreeKind::Path(path) = &mount_decl.tree.kind {
                        // Same extraction policy as the other sites in
                        // this file — preserve Super/Relative as
                        // "super" so downstream consumers see the
                        // structural prefix rather than a silently-
                        // truncated path. See the user-mount loop at
                        // ~line 11158 and the closure walker at
                        // ~line 11336 for the full bug-class context.
                        let full = path
                            .segments
                            .iter()
                            .filter_map(|seg| match seg {
                                verum_ast::ty::PathSegment::Name(ident) => {
                                    Some(ident.name.as_str().to_string())
                                }
                                verum_ast::ty::PathSegment::Super
                                | verum_ast::ty::PathSegment::Relative => Some("super".to_string()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join(".");
                        if !full.is_empty() {
                            user_mount_prefixes.push(full.clone());
                            // `std.*` aliasing: stdlib modules live under
                            // `core.*` in self.modules, so translate here.
                            if let Some(rest) = full.strip_prefix("std.") {
                                user_mount_prefixes.push(format!("core.{rest}"));
                            }
                            if !full.starts_with("core.") && !full.starts_with("std.") {
                                user_mount_prefixes.push(format!("core.{full}"));
                            }
                        }
                    }
                }
            }
        }

        let retains_user_path = |p: &str| -> bool {
            user_mount_prefixes.iter().any(|prefix| {
                // Exact match, ancestor (parent module of item), or descendant
                // (submodule under a mounted prefix).
                p == prefix
                    || p.starts_with(&format!("{prefix}."))
                    || prefix.starts_with(&format!("{p}."))
            })
        };

        // #117 — augment the user-mount-prefix retention with the
        // *transitive-mount* reachability set computed by the stdlib
        // dep graph. Without this, a stdlib module M2 that's mounted
        // *indirectly* (via M1's `mount …M2` body, where the user
        // only writes `mount …M1` themselves) gets pruned even though
        // M1's compiled body references M2's symbols. The downstream
        // symptom is `[lenient] SKIP method.X (bug-class): undefined
        // function: <symbol>` for every M2 symbol M1 calls.
        //
        // The original failure shape that motivated this: user mounts
        // `core.collections.{Map}` → `core.collections.bloom` is
        // re-exported by `collections/mod.vr` and pulled into the
        // compile set → bloom.vr's body mounts
        // `core.security.util.rng.{fill_secure}` → rng was NOT in
        // user_mount_prefixes (no user wrote `core.security`) so
        // `clear_non_compilable_stdlib_modules` dropped its AST →
        // BloomFilter.try_new lenient-SKIPs on every audit.
        //
        // The dep-graph reachability set already follows every
        // transitive `mount` edge (#109's foundation). Use it as a
        // SECOND retention oracle, unioned with the user-prefix one
        // so we never prune a module the user transitively needs.
        let reachable_stdlib: Option<std::collections::HashSet<String>> =
            user_module.and_then(crate::stdlib_reachability::compute_reachable_stdlib_modules);

        let total_before = self.modules.len();
        let retained: Map<Text, Arc<Module>> = self
            .modules
            .drain()
            .filter(|(path, _module)| {
                let p = path.as_str();
                ALWAYS_INCLUDE.contains(&p)
                    || retains_user_path(p)
                    || reachable_stdlib
                        .as_ref()
                        .is_some_and(|set| set.contains(p))
            })
            .collect();

        let retained_count = retained.len();
        self.modules = retained;
        debug!(
            "Retained {}/{} stdlib modules for AOT compilation ({} user-mount paths)",
            retained_count,
            total_before,
            user_mount_prefixes.len()
        );
    }
}
