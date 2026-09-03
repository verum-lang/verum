//! Public dispatch entry points (single-file run / check / parse).
//!
//! Extracted from `pipeline.rs` (#106 Phase 16). Houses the
//! single-source entry points that the CLI tier-selection logic
//! routes to:
//!
//!  * `run` — unified dispatch based on `CompilerOptions`
//!  flags; routes to `run_check_only` (Checked) or
//!  `run_native_compilation` (Built).
//!  * `run_full_compilation` — run all phases (parse → typecheck
//!  → verify → cbgr → interpret).
//!  * `run_check_only` — type-check-only flow for IDEs / CI.
//!  * `run_parse_only` — parse-only flow for VCS parse-pass tests.
//!  * `run_compiled_vbc` — execute a pre-compiled VBC module
//!  directly (script-mode persistent-cache hit path).
//!  * `run_interpreter` — interpreter dispatch with args.

use std::time::Instant;

use anyhow::Result;
use tracing::info;

use verum_common::{List, Text};
use verum_vbc::interpreter::Interpreter as VbcInterpreter;

use crate::options::VerifyMode;

use super::{CompilationPipeline, RunResult};

impl<'s> CompilationPipeline<'s> {
    /// Type-check every module of the cog before this entry point acts
    /// on it (T1119).
    ///
    /// `phase_load_source` collapses a project to ONE module — `main.vr`
    /// — so every driver that starts with it checks that file and lets
    /// the rest of the cog through unchecked. Measured 2026-09-03: the
    /// same `-> Int` returning `Text` was caught in `main.vr` and missed
    /// in a sibling, and `verum run` EXECUTED the program and printed
    /// the `Text`.
    ///
    /// Ten entry points begin that way, so the repair could not be a
    /// guard copied ten times — that is the shape the defect is made of.
    /// The DECISION (does this entry promise a checked program?) stays
    /// with the entry, which is why this is a method it calls rather
    /// than something buried in `phase_load_source`: `run_parse_only`
    /// promises the opposite, and its measured 8 MB / 0.01 s exists
    /// precisely because it materialises nothing.
    ///
    /// The condition is "the input belongs to a cog", NOT "the input is
    /// a directory". `verum build` is handed the directory, but
    /// `verum run` resolves `src/main.vr` itself (commands/run.rs) and
    /// hands over a FILE — an `is_dir()` guard was inert there, on the
    /// path that executes.
    ///
    /// ONLY the codegen path calls this today, and the reason is a
    /// measured regression rather than a preference. Called at the top
    /// of `run_interpreter` (or `run_for_test`) it leaves state the
    /// interpreter cannot use: a CORRECT cog that printed 42 then died
    /// with `[lenient] stage-5 qualified cross-module fn stub never
    /// resolved`. The control that caught it — "a correct cog must
    /// still build AND run" — was written into the verification before
    /// the binary existed, precisely because a narrowing repair can
    /// narrow to the empty set.
    ///
    /// So `run` and the harness still execute a cog whose siblings were
    /// never type-checked. Closing that needs the pre-pass to run
    /// against its OWN session rather than this one, which is the next
    /// step of T1119, not a smaller version of this one.
    pub(super) fn ensure_project_type_checked(&mut self) -> Result<()> {
        if !self.input_belongs_to_a_cog() {
            return Ok(());
        }
        self.check_project()?;
        Ok(())
    }

    /// Is there a `verum.toml` at or above the input?
    pub(super) fn input_belongs_to_a_cog(&self) -> bool {
        let input = &self.session.options().input;
        let mut dir = if input.is_dir() {
            input.clone()
        } else {
            match input.parent() {
                Some(p) => p.to_path_buf(),
                None => return false,
            }
        };
        // Bounded walk: a cog root within eight levels of its sources is
        // every layout this toolchain produces, and an unbounded loop on
        // a path that never terminates is worse than a missed check.
        for _ in 0..8 {
            if dir.join("verum.toml").exists() {
                return true;
            }
            match dir.parent() {
                Some(p) => dir = p.to_path_buf(),
                None => return false,
            }
        }
        false
    }

    /// Unified dispatch entry-point: routes to the appropriate
    /// internal `run_*` method based on the session's
    /// `CompilerOptions`. Centralises tier selection in one place
    /// so future tiers (Tier-0 interpret, MLIR JIT, MLIR AOT) extend
    /// the dispatch by adding a new arm rather than touching every
    /// caller. The matched [`RunResult`] tells the caller whether
    /// codegen produced a binary (`Built(path)`) or was skipped
    /// because `check_only=true` (`Checked`).
    pub fn run(&mut self) -> Result<RunResult> {
        if self.session.options().check_only {
            self.run_check_only()?;
            return Ok(RunResult::Checked);
        }
        // T1119 — a cog's SIBLING modules were never type-checked.
        //
        // `run_native_compilation` runs its phases in full: load, parse,
        // type_check, dependency_analysis. The gap is its INPUT.
        // `phase_load_source` on a directory finds `main.vr` inside it and
        // hands back ONE module, so the whole chain honestly checks that
        // one file; the rest of the cog arrives later, when codegen
        // resolves mounts, and never passes the type checker.
        //
        // Measured 2026-09-03 — one defect, only its LOCATION moved:
        //
        //     `-> Int` returning `Text` in src/mistyped.vr   check 3, build 0
        //     the same in src/main.vr                        check 3, build 2
        //
        // and `verum run` on the first PRINTED `x` — a `Text` where the
        // signature says `Int`, executed. That is not a missing
        // diagnostic; it is a soundness hole reachable by the ordinary
        // shape of any real program.
        //
        // Not the same defect as T1101 (a phase absent from a driver's
        // list) or T1118 (a driver stopping at the first failing phase).
        // A third form: the phase is called, is correct, and is handed
        // less than the program.
        //
        // `check_project` already walks every discovered module and type
        // checks it without generating code, so the repair is to ask it
        // first rather than to write a second traversal — a second one
        // would drift from this one exactly the way the drivers already
        // have.
        self.ensure_project_type_checked()?;
        let path = self.run_native_compilation()?;
        Ok(RunResult::Built(path))
    }

    /// Run complete compilation (all phases)
    pub fn run_full_compilation(&mut self) -> Result<()> {
        let start = Instant::now();

        // Phase 1: Lexing
        let file_id = self.phase_load_source()?;

        // Phase 2: Parsing
        let mut module = self.phase_parse(file_id)?;

        // Lazy-stdlib scope (#281): match the AOT and interpreter paths by
        // never materialising stdlib modules outside the user's mount tree.
        // STDLIB-LOAD-COST-1 moved the stdlib load below the parse so the
        // closure is known first — see `run_check_only`.
        // Honours `VERUM_FULL_STDLIB=1`.
        let reachable = if std::env::var("VERUM_FULL_STDLIB").is_ok() {
            None
        } else {
            crate::stdlib_reachability::compute_reachable_stdlib_modules(&module)
        };

        // Load stdlib modules (enables std.* imports)
        self.load_stdlib_modules_scoped(reachable.as_ref())?;

        if std::env::var("VERUM_FULL_STDLIB").is_err() {
            self.clear_non_compilable_stdlib_modules(Some(&module));
        }

        // Phase 3: Type checking
        self.phase_type_check(&module)?;
        // Apply typechecker-resolved call targets to the AST so the
        // VBC fast path in `compile_method_call` picks them up.
        self.apply_resolved_call_targets(&mut module);

        // Phase 3b: Dependency analysis
        self.phase_dependency_analysis(&module)?;

        // Phase 4: Refinement verification
        if self.session.options().verify_mode.use_smt() {
            self.phase_verify(&module)?;
        }

        // Phase 5: CBGR analysis
        self.phase_cbgr_analysis(&module)?;

        self.diagnostic_gate()?;

        // Phase 6: Interpretation
        self.phase_interpret(&module)?;

        let elapsed = start.elapsed();
        info!("Compilation completed in {:.2}s", elapsed.as_secs_f64());

        Ok(())
    }

    /// Post-compile diagnostic gate: error diagnostics become the
    /// command verdict (abort), and accumulated warnings render once.
    /// Phases emit diagnostics without returning Err (E0319 proof-
    /// verification failures, W0319 admitted-proof warnings, W05xx
    /// lints); every execution path must pass this gate between
    /// "compile phases done" and "execute / write artifact" —
    /// otherwise emitted errors let the program run anyway (the
    /// silent-acceptance class in pipeline form, T0105).
    pub(super) fn diagnostic_gate(&self) -> Result<()> {
        self.session.abort_if_errors()?;
        if self.session.warning_count() > 0 {
            let _ = self.session.display_diagnostics();
        }
        Ok(())
    }

    /// Run type checking only (no execution)
    ///
    /// Note: For complex type checking scenarios, ensure RUST_MIN_STACK is set
    /// appropriately (e.g., 16MB) in the build/test environment.
    /// Test hook: run ONLY the project-module load, so a test can read
    /// the module paths the loader derives without compiling anything.
    ///
    /// The derivation is what three separate defects were about — the
    /// root taken from the directory name, `src` becoming a segment, and
    /// a method accepted as an entry point — and each was invisible in
    /// `core/`, where directory and cog name coincide and there is no
    /// `src`. A test that only ran a project would report "it works"
    /// without pinning the fact that made it stop working.
    #[doc(hidden)]
    pub fn load_project_modules_for_testing(&mut self) -> Result<()> {
        self.load_project_modules()
    }

    /// Test hook: every module the pipeline holds, with its AST.
    ///
    /// `check_project` reported zero errors for a file that
    /// `verum check <that file>` refused, so the question "what does the
    /// project path actually hold" needed an answer that is read rather
    /// than inferred.
    #[doc(hidden)]
    pub fn loaded_modules_for_testing(
        &self,
    ) -> Vec<(String, std::sync::Arc<verum_ast::Module>)> {
        self.modules
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), std::sync::Arc::clone(v)))
            .collect()
    }

    /// Test hook: the module paths the project load registered.
    #[doc(hidden)]
    pub fn loaded_project_module_paths_for_testing(&self) -> Vec<String> {
        self.project_modules
            .keys()
            .map(|k| k.as_str().to_string())
            .collect()
    }

    pub fn run_check_only(&mut self) -> Result<()> {
        let start = Instant::now();

        // STDLIB-LOAD-COST-1 — the user file is read and parsed FIRST.
        //
        // It used to be the other way round, and that ordering decided the
        // whole cost profile of the command: `verum check` on a file with a
        // syntax error peaked at 721 MB and 0.22 s, because the entire
        // stdlib registry was materialised before the parser ever looked at
        // the source.  The same command with `--parse-only` — which skips
        // the stdlib — peaks at 8 MB and 0.01 s.  Nothing between
        // `phase_load_source` and `phase_parse` consults the stdlib: the
        // parser reads source, applies `@cfg`, and injects the implicit
        // prelude mount, all from the file itself.
        //
        // Parsing first also makes the mount closure KNOWN before the load,
        // which is what lets the load be scoped to it instead of pruned
        // after it.
        let file_id = self.phase_load_source()?;
        let mut module = self.phase_parse(file_id)?;

        // #109 lazy stdlib monomorphization — compute the transitive
        // closure of stdlib modules referenced by the user file's mount
        // tree (plus the implicit prelude). Stdlib modules outside the
        // closure are never materialised; the lazy resolver still picks
        // them up if a downstream phase actually walks into them.
        //
        // Two opt-out gates honour callers that need every stdlib
        // module in the registry up front (e.g. `verum audit
        // --framework-axioms`, full-corpus tooling):
        //   * `VERUM_FULL_STDLIB=1` env var (debug + CI)
        //   * `[build].full-stdlib` manifest key (per-project)
        let full_stdlib = std::env::var("VERUM_FULL_STDLIB").is_ok();
        let reachable = if full_stdlib {
            None
        } else {
            crate::stdlib_reachability::compute_reachable_stdlib_modules(&module)
        };

        // Load the stdlib modules the closure names (enables std.* imports).
        // This populates `self.modules` but does NOT yet register them in
        // the session's ModuleRegistry — registration happens below.
        self.load_stdlib_modules_scoped(reachable.as_ref())?;

        // Load sibling project modules (enables cross-file mount imports).
        // These are user-side files — always registered regardless of the
        // stdlib reachability filter.
        self.load_project_modules()?;
        // Load externally-registered cogs (script-mode `dependencies`,
        // verum-add deps, etc.) using the same module-registration
        // machinery so cross-cog `mount foo.bar` resolves transparently.
        self.load_external_cog_modules()?;

        // Register stdlib + project + cog modules for cross-file
        // type/context/import resolution. Without this, `mount
        // core.sys.darwin.libsystem.{...}` and `using [ComputeDevice]`
        // fail because the type checker doesn't know about symbols
        // from sibling modules. This is the CORRECT architectural fix
        // — not lenient bypasses.
        self.register_modules_for_cross_file_resolution_filtered(reachable.as_ref())?;

        // Get module path for registration and expansion.  Prefer the
        // file's `module <dotted.path>;` declaration (the canonical
        // namespace name) over the filesystem path — the latter
        // produces strings like `verify/kernel_v0/soundness.vr`
        // which break super-/self-resolution because the path
        // resolver expects dotted module-namespace segments.
        //
        // Fundamental fix for #192 (kernel_v0 self-check unblock).
        let module_path = self
            .extract_top_level_module_decl_path(&module)
            .unwrap_or_else(|| {
                Text::from(self.session.options().input.display().to_string())
            });

        // Register meta functions (enables meta-fail tests)
        self.register_meta_declarations(&module_path, &module)?;

        // Expand macros (evaluates @macro() invocations, triggers hygiene checks)
        self.expand_module(&module_path, &mut module)?;

        // Check if file has meta functions for special handling
        let has_meta_functions = module.items.iter().any(|item| {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                func.is_meta
            } else {
                false
            }
        });

        // CHECK-SUBSET-BUILD, third and fourth entries (T1101). The
        // invariant stated below for `phase_verify` — "check ⊆ build must
        // hold for verdicts" — was violated twice more by this same
        // function, and by phases nobody had listed:
        //
        //   `clear_non_compilable_stdlib_modules` — `run_interpreter`
        //     drops the stdlib modules that cannot compile before it
        //     judges anything. Without it `check` walks into modules the
        //     shipped path never sees, so the two commands do not even
        //     read the same library.
        //   `phase_safety_gate` — the user's `[safety]` configuration
        //     (unsafe, @ffi, …) was enforced by `build` and `run` and
        //     ignored by `check`. Inert on the default permissive
        //     configuration, which is why it stayed invisible, and a
        //     silent hole under a restrictive one.
        //
        // Both are taken in the order `run_interpreter` takes them, and
        // both run BEFORE type checking there, so the same diagnostic
        // arrives at the same point in both commands rather than at two
        // different ones. `scripts/ci/check_entry_points_agree_on_their_phases.py`
        // holds the mapping and names these two rows.
        if std::env::var("VERUM_FULL_STDLIB").is_err() {
            self.clear_non_compilable_stdlib_modules(Some(&module));
        }
        self.phase_safety_gate(&module)?;

        if has_meta_functions {
            // For files with meta functions, run BOTH meta evaluation and type checking.
            // Meta evaluation runs first to produce M-code errors (needed for meta-fail tests).
            // Type checking also runs to produce E-code errors (needed for tests expecting E400, etc.).
            // Both phases emit diagnostics to the session, so all errors are collected
            // in format_diagnostics() regardless of which phase returned first.
            let meta_result = self.phase_meta_evaluation(&module, &module_path);
            let type_check_result = self.phase_type_check(&module);

            // Return error from whichever phase failed (meta errors take priority)
            if let Err(e) = meta_result {
                // Also report type check errors if any
                if let Err(_tc_err) = type_check_result {
                    // Both failed - diagnostics from both are in the session
                }
                return Err(e);
            }
            if let Err(e) = type_check_result {
                return Err(e);
            }
        } else {
            // For files without meta functions, original ordering: type check then meta eval
            let type_check_result = self.phase_type_check(&module);
            if type_check_result.is_ok() {
                self.phase_meta_evaluation(&module, &module_path)?;
            } else {
                return type_check_result;
            }
        }

        // SMT verification phase — refinement obligations, contracts,
        // and theorem/lemma/corollary proofs (E0319 / W0319).
        // `verum check`'s purpose is "validate without executing", and
        // proof obligations ARE validation: before T0105 this phase
        // simply never ran in check-only mode, so a false theorem
        // passed `verum check` while the same file failed `verum
        // build`. check ⊆ build must hold for verdicts. Callers that
        // verify separately afterwards (`verum verify`, the
        // `verum test` preflight) opt out via `VerifyMode::Runtime`.
        if self.session.options().verify_mode.use_smt() {
            let r = self.phase_verify(&module);
            self.session.collect_phase_error("verify", r)?;
        }

        // Context validation — Phase 4c, the same one `build` runs.
        // The invariant stated above ("check ⊆ build must hold for
        // verdicts") was violated here a second time, by a different
        // phase: measured 2026-09-03, a function declaring
        // `using [!Zlog]` and calling one that requires `Zlog` produced
        // a diagnostic under `verum build` and NOTHING under
        // `verum check <file>`. `check_project` got this in T1095; the
        // single-file path is a third entry point and got it here.
        self.phase_context_validation(&module);

        // Dependency analysis (validates against target constraints)
        self.phase_dependency_analysis(&module)?;

        // CBGR analysis — the fourth entry of the same invariant (T1101).
        // `run_interpreter` and `run_for_test` both run it; `verum check`
        // skipped it outright, so a borrow the shipped path refuses passed
        // `check` clean. It goes last here for the same reason it goes
        // last there: it reads the types the checker has just settled.
        self.phase_cbgr_analysis(&module)?;

        let elapsed = start.elapsed();
        info!("Type checking completed in {:.2}s", elapsed.as_secs_f64());

        self.diagnostic_gate()
    }

    /// Run parse only (no type checking, for VCS parse-pass tests)
    pub fn run_parse_only(&mut self) -> Result<()> {
        let start = Instant::now();

        let file_id = self.phase_load_source()?;
        let _module = self.phase_parse(file_id)?;

        let elapsed = start.elapsed();
        info!("Parsing completed in {:.2}s", elapsed.as_secs_f64());

        Ok(())
    }

    /// Run interpreter mode
    /// Execute a pre-compiled VBC module against the given args.
    ///
    /// Used by the script-mode persistent cache: on a cache hit the
    /// runner deserialises the stored VBC bytes into a `VbcModule` and
    /// calls this method, skipping every front-end phase (parse,
    /// typecheck, verify, codegen) for a sub-millisecond cold start
    /// of unchanged scripts.
    ///
    /// Behaviour matches `phase_interpret_with_args` post-compile —
    /// builds a `VbcInterpreter`, resolves the entry function (`main`
    /// with `__verum_script_main` fallback), executes with or without
    /// the args list, and routes the terminal value through
    /// `propagate_main_exit_code` for tier-parity with AOT.
    pub fn run_compiled_vbc(
        &mut self,
        vbc_module: std::sync::Arc<verum_vbc::module::VbcModule>,
        args: List<Text>,
    ) -> Result<()> {
        // Re-record the captured VBC so a subsequent
        // `take_compiled_vbc()` still surfaces something — useful
        // when the runner wants to refresh metadata even on cache hits.
        self.session.record_compiled_vbc(vbc_module.clone());

        // Fingerprint the module as it arrives from the cache, so it can
        // be compared against the one printed at compile time (T0737).
        super::vbc_codegen::trace_module_fingerprint(&vbc_module, "from-cache");

        // Make the embedded scripting engine able to compile scripts at
        // runtime (core.script / script_engine_eval). Idempotent.
        crate::api::ensure_scripting_compiler_installed();

        let mut interpreter = VbcInterpreter::new(vbc_module);
        // PARITY WITH `phase_interpret` (T0737).
        //
        // This function runs a module that came back from the script
        // cache. Everything below was missing here, so a script behaved
        // one way on its first run (compile → `phase_interpret`) and
        // another way on every run after it (cache hit → this function),
        // with no source change in between. Measured on
        // vcs/specs/L1-core/atomic_bool_rmw.vr: cache miss printed
        // `atomic-bool ok`, cache hit died on an assertion.
        //
        // Setting up an interpreter is therefore not "extra" work the
        // cache path can skip — it is part of what running a module MEANS,
        // and the two entry points have to agree on it.
        {
            let rt = &self.session.language_features().runtime;
            interpreter.state.config.async_scheduler = rt.async_scheduler.as_str().to_string();
            interpreter.state.config.async_worker_threads = rt.async_worker_threads;
            interpreter.state.config.futures_enabled = rt.futures;
            interpreter.state.config.nurseries_enabled = rt.nurseries;
            interpreter.state.config.task_stack_size = rt.task_stack_size;
            interpreter.state.config.heap_policy = rt.heap_policy.as_str().to_string();
        }
        // Production has no wall-clock budget: the 30s deadline and the
        // instruction cap are test-runner safety nets. Without these two
        // lines a server or REPL loop ran fine on its first invocation and
        // was killed on its second.
        interpreter.state.config.timeout_ms = 0;
        interpreter.state.config.max_instructions = 0;
        // Transfer the script-mode permission policy (if the CLI
        // installed one) into the interpreter's PermissionRouter
        // before the first instruction dispatches. The router's
        // one-entry cache + warm path keeps repeated checks at
        // ≤2ns; the policy itself is consulted only on cache miss.
        if let Some(policy) = self.session.take_script_permission_policy() {
            interpreter.state.permission_router.set_policy(policy.0);
        }
        // `@thread_local` static initializers populate their TLS slots
        // here. `phase_interpret` documents why: without it the CBGR
        // allocator's LOCAL_HEAP/CURRENT_HEAP bootstrap reads
        // `Value::default()` from an uninitialised slot. A cached module
        // carries the same ctors and needs the same run.
        if let Err(e) = interpreter.run_global_ctors() {
            return Err(anyhow::anyhow!("VBC global_ctors error: {:?}", e));
        }
        let main_func_id = self.find_main_function_id(&interpreter.state.module)?;
        let main_param_count = interpreter
            .state
            .module
            .get_function(main_func_id)
            .map(|f| f.params.len())
            .unwrap_or(0);

        if main_param_count == 0 || args.is_empty() {
            info!("Executing cached VBC (no-args path)");
            let result = interpreter.execute_function(main_func_id);
            return self.finalize_run_result(result);
        }

        let rust_args: Vec<String> = args.iter().map(|t| t.to_string()).collect();
        let args_value = interpreter
            .alloc_string_list(&rust_args)
            .map_err(|e| anyhow::anyhow!("Failed to allocate args: {:?}", e))?;
        info!("Executing cached VBC with {} args", rust_args.len());
        let result = interpreter.call(main_func_id, &[args_value]);
        self.finalize_run_result(result)
    }

    pub fn run_interpreter(&mut self, args: List<Text>) -> Result<()> {
        let trace = std::env::var("VERUM_TRACE_PHASES").is_ok();
        let t_total = std::time::Instant::now();

        // T1119, the interpreter half. `phase_load_source` below hands
        // back ONE module for a directory — `main.vr` — so a sibling's
        // body never reaches the type checker on this path either.
        // Measured after the `run()` half was repaired:
        //
        //     defect in src/helper.vr   check 3   build 2   run 0
        //
        // `build` had been taught; `run` had not, and `run` is the path
        // that EXECUTES. The same pre-pass, for the same reason: ask the
        // traversal that already walks every module rather than write a
        // third one.
        // NOT here — see `ensure_project_type_checked`. Calling it before
        // this path's own pipeline leaves state the interpreter cannot use:
        // a correct cog then dies with `[lenient] stage-5 qualified
        // cross-module fn stub never resolved`. Measured: v13 prints 42,
        // v14 panics on the same source.

        // STDLIB-LOAD-COST-1 — source first, stdlib second.  See
        // `run_check_only` for the measurement; the ordering is what makes
        // the mount closure known before the load rather than after it.
        let t = std::time::Instant::now();
        let file_id = self.phase_load_source()?;
        if trace { eprintln!("[run_interpreter] phase_load_source: {:.2}ms", t.elapsed().as_secs_f64() * 1000.0); }
        let t = std::time::Instant::now();
        let mut module = self.phase_parse(file_id)?;
        if trace { eprintln!("[run_interpreter] phase_parse: {:.2}ms", t.elapsed().as_secs_f64() * 1000.0); }

        // Lazy-stdlib scope (#281, parity with run_native_compilation):
        // never materialise stdlib modules outside the user's mount tree.
        // Without this, `verum run script.vr` walks every loaded stdlib AST
        // through type-check + codegen + monomorphization and each cold
        // script run pays the full ~83 K-function cost — even for a
        // `--help` print.  This used to be a prune AFTER the load; the
        // closure is now computed before it, so the modules are not built
        // in the first place.  `clear_non_compilable_stdlib_modules` still
        // runs: it also drops non-stdlib entries the closure has no say
        // over, and it is idempotent when the scope already excluded them.
        // Honours the same `VERUM_FULL_STDLIB=1` opt-out used elsewhere
        // (full-corpus tooling: `verum audit --framework-axioms` etc.).
        let reachable = if std::env::var("VERUM_FULL_STDLIB").is_ok() {
            None
        } else {
            crate::stdlib_reachability::compute_reachable_stdlib_modules(&module)
        };

        // Load stdlib modules (enables std.* imports)
        let t = std::time::Instant::now();
        self.load_stdlib_modules_scoped(reachable.as_ref())?;
        if trace {
            eprintln!("[run_interpreter] load_stdlib_modules: {:.2}ms", t.elapsed().as_secs_f64() * 1000.0);
        }

        // Load sibling project modules (enables cross-file mount imports)
        let t = std::time::Instant::now();
        self.load_project_modules()?;
        if trace { eprintln!("[run_interpreter] load_project_modules: {:.2}ms", t.elapsed().as_secs_f64() * 1000.0); }
        let t = std::time::Instant::now();
        // Load externally-registered cogs (script-mode `dependencies`,
        // verum-add deps, etc.) using the same module-registration
        // machinery so cross-cog `mount foo.bar` resolves transparently.
        self.load_external_cog_modules()?;
        if trace { eprintln!("[run_interpreter] load_external_cog_modules: {:.2}ms", t.elapsed().as_secs_f64() * 1000.0); }

        // NO MACRO EXPANSION HERE, and the reason is a measurement rather
        // than an omission (T0732, 2026-09-03).
        //
        // `run_check_only` registers meta declarations and expands macros;
        // `run_for_test` does too; this path does not, which reads like the
        // drift this task is about. It is not. Measured on one file, two
        // binaries, with a per-run nonce so the content-keyed VBC cache
        // could not answer:
        //
        //     meta fn six() -> Int { 1 + 2 + 3 }
        //     fn main() { print(@six()); }
        //
        //     verum check   warns E0410, no error
        //     verum run     prints `nil`, warns E0410
        //     the harness   prints `nil`, FAILED
        //
        // All three paths AGREE, before and after adding the two calls
        // here — the addition was inert. `@meta_fn()` evaluating to `nil`
        // and the parser's E0410 firing for a `meta` the same file
        // declares are real defects, but they belong to the meta system,
        // not to harness parity, and adding an unmeasured phase to the
        // shipped path to chase them would be its own kind of drift.
        if std::env::var("VERUM_FULL_STDLIB").is_err() {
            self.clear_non_compilable_stdlib_modules(Some(&module));
        }

        // Safety-feature gates (unsafe, @ffi, etc.) ALWAYS run —
        // independent of verify_mode. Without this, `--verify runtime`
        // silently bypassed the user's `[safety]` configuration.
        let t = std::time::Instant::now();
        self.phase_safety_gate(&module)?;
        if trace { eprintln!("[run_interpreter] phase_safety_gate: {:.2}ms", t.elapsed().as_secs_f64() * 1000.0); }

        // Type check unless in runtime-only mode
        // Runtime mode skips static analysis for faster iteration
        if self.session.options().verify_mode != VerifyMode::Runtime {
            let t = std::time::Instant::now();
            self.phase_type_check(&module)?;
            // #91/#95 — apply the typechecker's resolution side-table
            // to the AST so the VBC compile_method_call fast path
            // can pick up `Expr::resolved_call_target` directly.
            self.apply_resolved_call_targets(&mut module);
            if trace { eprintln!("[run_interpreter] phase_type_check: {:.2}ms", t.elapsed().as_secs_f64() * 1000.0); }

            let t = std::time::Instant::now();
            // Dependency analysis (validates against target constraints)
            self.phase_dependency_analysis(&module)?;
            if trace { eprintln!("[run_interpreter] phase_dependency_analysis: {:.2}ms", t.elapsed().as_secs_f64() * 1000.0); }

            // Verify refinements if enabled
            if self.session.options().verify_mode.use_smt() {
                let t = std::time::Instant::now();
                self.phase_verify(&module)?;
                if trace { eprintln!("[run_interpreter] phase_verify: {:.2}ms", t.elapsed().as_secs_f64() * 1000.0); }
            }

            let t = std::time::Instant::now();
            // CBGR analysis
            self.phase_cbgr_analysis(&module)?;
            if trace { eprintln!("[run_interpreter] phase_cbgr_analysis: {:.2}ms", t.elapsed().as_secs_f64() * 1000.0); }
        }

        self.diagnostic_gate()?;

        // Interpret and execute the module
        info!("Executing program...");
        let t = std::time::Instant::now();
        self.phase_interpret_with_args(&module, args)?;
        if trace {
            eprintln!("[run_interpreter] phase_interpret_with_args: {:.2}ms", t.elapsed().as_secs_f64() * 1000.0);
            eprintln!("[run_interpreter] TOTAL: {:.2}ms", t_total.elapsed().as_secs_f64() * 1000.0);
        }

        Ok(())
    }

    /// Extract the canonical dotted module path from a parsed
    /// module's top-level `module <dotted.path>;` declaration.
    ///
    /// Returns `Some("core.verify.kernel_v0.soundness")` when the
    /// file's first item is `module core.verify.kernel_v0.soundness;`,
    /// `None` when no top-level module declaration exists (e.g.,
    /// inline-modules-only files or scripts).
    ///
    /// Used by `run_check_only` to anchor the type-checker's
    /// `current_module_path` correctly in single-file mode — the
    /// filesystem path is unsuitable because the relative-path
    /// resolver (`super.X`) splits on `.` and expects dotted
    /// namespace segments, not slashes/`.vr`.
    fn extract_top_level_module_decl_path(
        &self,
        module: &verum_ast::Module,
    ) -> Option<Text> {
        for item in &module.items {
            if let verum_ast::ItemKind::Module(decl) = &item.kind {
                // Inline modules (`module foo { ... }` with body) are
                // NOT the top-level module declaration; skip those.
                if matches!(&decl.items, verum_common::Maybe::Some(_)) {
                    continue;
                }
                // The parser stores the full dotted name (`core.verify.kernel_v0.soundness`)
                // as a single Text in `decl.name.name`.
                //
                // Strip a trailing `.mod` segment so `module
                // core.verify.kernel_v0.mod;` (the convention for
                // mod.vr files) registers under the canonical
                // namespace path `core.verify.kernel_v0`.  Without
                // this strip, `super.X` from the mod.vr file would
                // pop to `core.verify.kernel_v0` (correct) instead
                // of `core.verify` (the author's intent: sibling
                // access within the directory).
                let name = decl.name.name.as_str();
                if !name.is_empty() {
                    let canonical = name
                        .strip_suffix(".mod")
                        .unwrap_or(name);
                    return Some(Text::from(canonical));
                }
            }
        }
        None
    }
}
