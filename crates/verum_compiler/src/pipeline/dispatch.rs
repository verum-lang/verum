//! Public dispatch entry points (single-file run / check / parse).
//!
//! Extracted from `pipeline.rs` (#106 Phase 16). Houses the
//! single-source entry points that the CLI tier-selection logic
//! routes to:
//!
//!   * `run` — unified dispatch based on `CompilerOptions`
//!     flags; routes to `run_check_only` (Checked) or
//!     `run_native_compilation` (Built).
//!   * `run_full_compilation` — run all phases (parse → typecheck
//!     → verify → cbgr → interpret).
//!   * `run_check_only` — type-check-only flow for IDEs / CI.
//!   * `run_parse_only` — parse-only flow for VCS parse-pass tests.
//!   * `run_compiled_vbc` — execute a pre-compiled VBC module
//!     directly (script-mode persistent-cache hit path).
//!   * `run_interpreter` — interpreter dispatch with args.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tracing::{debug, info};

use verum_common::{List, Text};
use verum_vbc::interpreter::Interpreter as VbcInterpreter;

use crate::options::VerifyMode;

use super::{CompilationPipeline, RunResult};

impl<'s> CompilationPipeline<'s> {
    /// Unified dispatch entry-point: routes to the appropriate
    /// internal `run_*` method based on the session's
    /// `CompilerOptions`.  Centralises tier selection in one place
    /// so future tiers (Tier-0 interpret, MLIR JIT, MLIR AOT) extend
    /// the dispatch by adding a new arm rather than touching every
    /// caller.  The matched [`RunResult`] tells the caller whether
    /// codegen produced a binary (`Built(path)`) or was skipped
    /// because `check_only=true` (`Checked`).
    pub fn run(&mut self) -> Result<RunResult> {
        if self.session.options().check_only {
            self.run_check_only()?;
            return Ok(RunResult::Checked);
        }
        let path = self.run_native_compilation()?;
        Ok(RunResult::Built(path))
    }

    /// Run complete compilation (all phases)
    pub fn run_full_compilation(&mut self) -> Result<()> {
        let start = Instant::now();

        // Load stdlib modules first (enables std.* imports)
        self.load_stdlib_modules()?;

        // Phase 1: Lexing
        let file_id = self.phase_load_source()?;

        // Phase 2: Parsing
        let module = self.phase_parse(file_id)?;

        // Phase 3: Type checking
        self.phase_type_check(&module)?;

        // Phase 3b: Dependency analysis
        self.phase_dependency_analysis(&module)?;

        // Phase 4: Refinement verification
        if self.session.options().verify_mode.use_smt() {
            self.phase_verify(&module)?;
        }

        // Phase 5: CBGR analysis
        self.phase_cbgr_analysis(&module)?;

        // Phase 6: Interpretation
        self.phase_interpret(&module)?;

        let elapsed = start.elapsed();
        info!("Compilation completed in {:.2}s", elapsed.as_secs_f64());

        Ok(())
    }

    /// Run type checking only (no execution)
    ///
    /// Note: For complex type checking scenarios, ensure RUST_MIN_STACK is set
    /// appropriately (e.g., 16MB) in the build/test environment.
    pub fn run_check_only(&mut self) -> Result<()> {
        let start = Instant::now();

        // Load stdlib modules first (enables std.* imports)
        self.load_stdlib_modules()?;

        // Register stdlib modules for cross-file type/context/import
        // resolution. Without this, `mount core.sys.darwin.libsystem.{...}`
        // and `using [ComputeDevice]` fail because the type checker
        // doesn't know about symbols from sibling modules.
        // This is the CORRECT architectural fix — not lenient bypasses.
        self.register_modules_for_cross_file_resolution()?;

        // Load sibling project modules (enables cross-file mount imports)
        self.load_project_modules()?;
        // Load externally-registered cogs (script-mode `dependencies`,
        // verum-add deps, etc.) using the same module-registration
        // machinery so cross-cog `mount foo.bar` resolves transparently.
        self.load_external_cog_modules()?;

        let file_id = self.phase_load_source()?;
        let mut module = self.phase_parse(file_id)?;

        // Get module path for registration and expansion
        let module_path = Text::from(self.session.options().input.display().to_string());

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

        // Dependency analysis (validates against target constraints)
        self.phase_dependency_analysis(&module)?;

        let elapsed = start.elapsed();
        info!("Type checking completed in {:.2}s", elapsed.as_secs_f64());

        Ok(())
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

        let mut interpreter = VbcInterpreter::new(vbc_module);
        // Transfer the script-mode permission policy (if the CLI
        // installed one) into the interpreter's PermissionRouter
        // before the first instruction dispatches. The router's
        // one-entry cache + warm path keeps repeated checks at
        // ≤2ns; the policy itself is consulted only on cache miss.
        if let Some(policy) = self.session.take_script_permission_policy() {
            interpreter.state.permission_router.set_policy(policy.0);
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
        info!(
            "Executing cached VBC with {} args",
            rust_args.len()
        );
        let result = interpreter.call(main_func_id, &[args_value]);
        self.finalize_run_result(result)
    }

    pub fn run_interpreter(&mut self, args: List<Text>) -> Result<()> {
        // Load stdlib modules first (enables std.* imports)
        self.load_stdlib_modules()?;

        // Load sibling project modules (enables cross-file mount imports)
        self.load_project_modules()?;
        // Load externally-registered cogs (script-mode `dependencies`,
        // verum-add deps, etc.) using the same module-registration
        // machinery so cross-cog `mount foo.bar` resolves transparently.
        self.load_external_cog_modules()?;

        let file_id = self.phase_load_source()?;
        let module = self.phase_parse(file_id)?;

        // Safety-feature gates (unsafe, @ffi, etc.) ALWAYS run —
        // independent of verify_mode. Without this, `--verify runtime`
        // silently bypassed the user's `[safety]` configuration.
        self.phase_safety_gate(&module)?;

        // Type check unless in runtime-only mode
        // Runtime mode skips static analysis for faster iteration
        if self.session.options().verify_mode != VerifyMode::Runtime {
            self.phase_type_check(&module)?;

            // Dependency analysis (validates against target constraints)
            self.phase_dependency_analysis(&module)?;

            // Verify refinements if enabled
            if self.session.options().verify_mode.use_smt() {
                self.phase_verify(&module)?;
            }

            // CBGR analysis
            self.phase_cbgr_analysis(&module)?;
        }

        // Interpret and execute the module
        info!("Executing program...");
        self.phase_interpret_with_args(&module, args)?;

        Ok(())
    }

}
