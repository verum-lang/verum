//! VBC interpreter dispatch (Phase 7, Tier 0 execution).
//!
//! Extracted from `pipeline.rs` (#106 Phase 11). Houses the
//! interpreter-side execution path: AST → VBC codegen → VBC
//! interpreter, plus the main-function discovery and exit-code
//! propagation helpers it depends on.
//!
//! Methods:
//!
//!   * `phase_interpret` — primary AST-driven entry; compiles to
//!     VBC, builds an interpreter, finds main, runs.
//!   * `phase_interpret_with_args` — args-bearing variant; passes
//!     argv into `main(args: List<Text>) -> Int`.
//!   * `phase_interpret_for_test` — captured-stdout/stderr variant
//!     for the test harness; threads the same script-mode policy
//!     into the interpreter's PermissionRouter.
//!   * `find_main_function_id` — script vs application entry-point
//!     selection (`__verum_script_main` vs `main`).
//!   * `propagate_main_exit_code` — int-return → `Session::pending_exit_code`.
//!   * `finalize_run_result` — common error-shaping for all three
//!     dispatch entry points.
//!   * `run_for_test` — public test-harness entry that wraps the
//!     full compile + interpret-for-test flow.

use std::path::PathBuf;

use anyhow::{Context as AnyhowContext, Result};
use tracing::{debug, info, warn};

use verum_ast::Module;
use verum_common::{List, Text};
use verum_vbc::interpreter::Interpreter as VbcInterpreter;
use verum_vbc::module::{FunctionId as VbcFunctionId, VbcModule};

use crate::api::TestExecutionResult;

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    /// VBC-first architecture: AST → VBC Codegen → VBC Interpreter
    fn phase_interpret(&mut self, module: &Module) -> Result<()> {
        let _bc = verum_error::breadcrumb::enter(
            "compiler.phase.interpret",
            self.session.options().input.display().to_string(),
        );
        debug!("Interpreting module via VBC-first architecture");

        // Context system validation
        self.phase_context_validation(module);

        // Send/Sync compile-time enforcement
        self.phase_send_sync_validation(module);

        // FFI boundary validation
        self.phase_ffi_validation(module)?;

        // Step 1: Compile AST to VBC
        let vbc_module = self.compile_ast_to_vbc(module)?;

        // Capture for the script-mode persistent cache. See the matching
        // capture in `phase_interpret_with_args` for the full rationale.
        self.session.record_compiled_vbc(vbc_module.clone());

        // Emit VBC bytecode dump if requested
        if self.session.options().emit_vbc {
            let dump = verum_vbc::disassemble::disassemble_module(&vbc_module);
            let vbc_path = self.session.options().input.with_extension("vbc.txt");
            if let Err(e) = std::fs::write(&vbc_path, &dump) {
                warn!("Failed to write VBC dump: {}", e);
            } else {
                info!("Wrote VBC dump: {} ({} bytes)", vbc_path.display(), dump.len());
            }
        }

        // Step 2: Create VBC interpreter with runtime config from [runtime]
        let mut interpreter = VbcInterpreter::new(vbc_module);
        {
            let rt = &self.session.language_features().runtime;
            interpreter.state.config.async_scheduler = rt.async_scheduler.as_str().to_string();
            interpreter.state.config.async_worker_threads = rt.async_worker_threads;
            interpreter.state.config.futures_enabled = rt.futures;
            interpreter.state.config.nurseries_enabled = rt.nurseries;
            interpreter.state.config.task_stack_size = rt.task_stack_size;
            interpreter.state.config.heap_policy = rt.heap_policy.as_str().to_string();
        }
        // Script-mode permission policy (see `phase_interpret_with_args`
        // for the full rationale).
        if let Some(policy) = self.session.take_script_permission_policy() {
            interpreter.state.permission_router.set_policy(policy.0);
        }

        // Step 3: Find and execute main function
        let main_func_id = self.find_main_function_id(&interpreter.state.module)?;

        // Run module.global_ctors first so @thread_local static initializers
        // populate their TLS slots before `main` reads them. This mirrors the
        // AOT path (LLVM @llvm.global_ctors runs before main via the C
        // runtime); without it, the CBGR allocator's LOCAL_HEAP/CURRENT_HEAP
        // bootstrap reads Value::default() from an uninitialized TLS slot and
        // crashes on first allocation.
        if let Err(e) = interpreter.run_global_ctors() {
            return Err(anyhow::anyhow!("VBC global_ctors error: {:?}", e));
        }

        info!("Executing main function via VBC interpreter (function ID: {})", main_func_id.0);
        let result = interpreter.execute_function(main_func_id);
        self.finalize_run_result(result)
    }

    /// Tier-parity exit-code propagation.
    ///
    /// When the entry point returns an `Int`, surface it to the OS as the
    /// process exit status — matching what AOT compilation produces (where
    /// `main`'s return value lands directly in `_exit`). Without this, the
    /// interpreter would run `fn main() -> Int { 1 }` to completion but
    /// the process would exit 0, silently masking failures.
    ///
    /// Behaviour:
    /// - `Int` value → record exit code = `value as i32`.
    /// - `Bool` → record 0 for true, 1 for false (Unix convention).
    /// - `Unit` / `Nil` / anything else → leave exit code as `None`,
    ///   which the CLI maps to `0`.
    ///
    /// **Why record instead of `std::process::exit`?** The pipeline runs
    /// inside a CLI driver that needs to perform post-execution work —
    /// persisting the script-mode VBC cache, flushing telemetry, printing
    /// `--timings` — *before* the OS terminates the process. Calling
    /// `process::exit` from inside the interpreter would short-circuit
    /// the cache-store step and force every script to re-pay the full
    /// compile cost on its next invocation. The CLI takes the recorded
    /// code from `Session::take_exit_code()` after housekeeping and
    /// translates to `process::exit` there.
    ///
    /// Called from BOTH `phase_interpret` (no-args entry) and
    /// `phase_interpret_with_args` (args-aware entry) so behaviour is
    /// uniform across `verum run file.vr` and `verum run file.vr a b`.
    /// Script wrappers (`__verum_script_main`) pass through transparently:
    /// the parser lifts an unsemicoloned tail expression into the
    /// wrapper's return slot, so a script ending in `42` records 42 here.
    fn propagate_main_exit_code(&self, value: &verum_vbc::Value) {
        if value.is_int() {
            let code = value.as_i64() as i32;
            self.session.record_exit_code(code);
            return;
        }
        if value.is_bool() {
            self.session.record_exit_code(if value.as_bool() { 0 } else { 1 });
        }
        // Unit / Nil / Float / Object / Pointer / String — no exit-code
        // semantics. CLI defaults to 0.
    }

    /// Map an interpreter execution result into a pipeline result
    /// while honouring the cooperative `ProcessExit` control-flow
    /// signal raised by `exit(n)` calls. The interpreter returns
    /// `Err(InterpreterError::ProcessExit(n))` so the driver can run
    /// post-execution housekeeping (script-cache store, timing
    /// flush, future telemetry) *before* the OS terminates. Any
    /// other `Err` is a real runtime failure; `Ok` carries the
    /// script's terminal value which feeds `propagate_main_exit_code`.
    fn finalize_run_result(
        &self,
        result: verum_vbc::interpreter::InterpreterResult<verum_vbc::Value>,
    ) -> Result<()> {
        use verum_vbc::interpreter::InterpreterError;
        match result {
            Ok(value) => {
                self.propagate_main_exit_code(&value);
                Ok(())
            }
            Err(InterpreterError::ProcessExit(code)) => {
                self.session.record_exit_code(code);
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("VBC execution error: {}", e)),
        }
    }

    /// Phase 5b: Interpretation with arguments
    ///
    /// VBC-first architecture: AST → VBC Codegen → VBC Interpreter with args
    fn phase_interpret_with_args(&mut self, module: &Module, args: List<Text>) -> Result<()> {
        debug!("Interpreting module with {} args via VBC-first architecture", args.len());

        if args.is_empty() {
            return self.phase_interpret(module);
        }

        // Two-path parity: run the same validation phases the no-args
        // path does, so `verum run file.vr arg1` applies the same
        // semantics as `verum run file.vr`. Previously these phases
        // were silently skipped when args were present.
        self.phase_context_validation(module);
        self.phase_send_sync_validation(module);
        self.phase_ffi_validation(module)?;

        // Step 1: Compile AST to VBC
        let vbc_module = self.compile_ast_to_vbc(module)?;

        // Capture for the script-mode persistent cache. The CLI runner
        // pulls this back via `Session::take_compiled_vbc()` after a
        // successful run and serialises it into the on-disk cache so
        // the next invocation of an unchanged script can skip parse +
        // typecheck + verify + codegen entirely.
        self.session.record_compiled_vbc(vbc_module.clone());

        // Step 2: Create VBC interpreter
        let mut interpreter = VbcInterpreter::new(vbc_module);
        // Script-mode permission policy (see `run_compiled_vbc` for
        // the full rationale).
        if let Some(policy) = self.session.take_script_permission_policy() {
            interpreter.state.permission_router.set_policy(policy.0);
        }

        // Step 2b: Skip global constructors (FFI initializers corrupt state on macOS).

        // Step 3: Find main function
        let main_func_id = self.find_main_function_id(&interpreter.state.module)?;

        // Step 4: Check if main() accepts parameters
        let main_param_count = interpreter.state.module.get_function(main_func_id)
            .map(|f| f.params.len())
            .unwrap_or(0);

        if main_param_count == 0 {
            // main() takes no args — execute normally
            info!("Executing main function via VBC interpreter (no args accepted)");
            let result = interpreter.execute_function(main_func_id);
            return self.finalize_run_result(result);
        }

        // Step 5: Allocate args as List<Text> on interpreter heap and call main(args)
        let rust_args: Vec<String> = args.iter().map(|t| t.to_string()).collect();
        let args_value = interpreter.alloc_string_list(&rust_args)
            .map_err(|e| anyhow::anyhow!("Failed to allocate args: {:?}", e))?;

        info!("Executing main function with {} args via VBC interpreter", rust_args.len());
        let result = interpreter.call(main_func_id, &[args_value]);
        self.finalize_run_result(result)
    }


    /// Find the program entry function in the VBC module and return
    /// its function ID.
    ///
    /// Strict mode separation (matches the AST-level
    /// `EntryDetectionPhase::detect_entry_point`):
    ///
    ///   • **Application** entry = `main` (in a non-script module).
    ///     Prefer it when present.
    ///   • **Script** entry = `__verum_script_main` (the synthesised
    ///     wrapper from script-tagged modules).
    ///
    /// The two are not interchangeable. A `fn main` declared *inside*
    /// a script module is a regular callable function, not the
    /// program entry — the AST-level pass already filtered such
    /// `main`s out, so by the time we reach the VBC the only `main`
    /// in the function table came from an application module. We
    /// preserve the precedence (`main` first, then wrapper) only as
    /// a defence-in-depth: if both names somehow appear in the VBC,
    /// the application entry still wins.
    fn find_main_function_id(&self, vbc_module: &VbcModule) -> Result<VbcFunctionId> {
        // First pass: script entry `__verum_script_main`. Its presence
        // is sufficient evidence that the source was a script, and the
        // strict-role contract says a script's entry is the wrapper —
        // never any user-declared `fn main` that may also be in the
        // function table (it's a regular callable function in script
        // mode, not the program entry).
        for (idx, func_desc) in vbc_module.functions.iter().enumerate() {
            if let Some(name) = vbc_module.get_string(func_desc.name) {
                if name == "__verum_script_main" {
                    return Ok(VbcFunctionId(idx as u32));
                }
            }
        }
        // Second pass: application entry `main`. Reached only when no
        // script wrapper exists, i.e. the source is an application
        // (no shebang, has `fn main()`).
        for (idx, func_desc) in vbc_module.functions.iter().enumerate() {
            if let Some(name) = vbc_module.get_string(func_desc.name) {
                if name == "main" {
                    return Ok(VbcFunctionId(idx as u32));
                }
            }
        }
        Err(anyhow::anyhow!("No main function found in VBC module"))
    }

    // ==================== Test Execution ====================

    /// Run test execution with output capture.
    ///
    /// This executes the program via the VBC interpreter with stdout/stderr captured.
    /// Used by vtest for running `run` and `run-panic` tests.
    ///
    /// Returns:
    /// - `Ok(TestExecutionResult)` on successful execution (even if panic)
    /// - `Err` only for compilation errors
    pub fn run_for_test(&mut self) -> Result<TestExecutionResult> {
        let start = Instant::now();

        // Load stdlib modules first (enables std.* imports)
        self.load_stdlib_modules()?;
        debug!("Phase 0 (stdlib): {:.2}s", start.elapsed().as_secs_f64());

        // Phase 1: Load source
        let file_id = self.phase_load_source()?;

        // Phase 2: Parse
        let mut module = self.phase_parse(file_id)?;
        debug!("Phase 2 (parse): {:.2}s", start.elapsed().as_secs_f64());

        // Get module path for registration and expansion
        let module_path = Text::from(self.session.options().input.display().to_string());

        // Register meta functions (enables macro expansion)
        self.register_meta_declarations(&module_path, &module)?;

        // Expand macros (evaluates @macro() invocations)
        self.expand_module(&module_path, &mut module)?;

        // Phase 3+: unified validation so test execution applies
        // the same language-mechanism checks as `verum build` /
        // `verum run`. Previously `run_for_test` skipped safety,
        // context, send/sync, and FFI validation.
        self.validate_module(&module, false)?;
        debug!("Phase 3+ (validate_module): {:.2}s", start.elapsed().as_secs_f64());

        // Phase 5: Compile to VBC and execute with capture
        debug!("Phase 5 starting (interpret_for_test): {:.2}s", start.elapsed().as_secs_f64());
        let result = self.phase_interpret_for_test(&module)?;

        let elapsed = start.elapsed();
        debug!("Test execution completed in {:.2}s", elapsed.as_secs_f64());

        Ok(TestExecutionResult {
            stdout: result.0,
            stderr: result.1,
            exit_code: result.2,
            duration: elapsed,
        })
    }

    /// Phase 5b: Interpret with output capture for test execution.
    ///
    /// Supports two modes:
    /// 1. **main() mode**: If the module has a `main` function, execute it (traditional).
    /// 2. **@test mode**: If no `main` exists, discover all `@test`-annotated functions
    ///    and run them sequentially as a test suite.
    fn phase_interpret_for_test(&mut self, module: &Module) -> Result<(String, String, i32)> {
        debug!("Interpreting module for test via VBC-first architecture");

        // Step 0: Reset global VBC value side-tables for test isolation.
        //
        // `Value` uses process-global `Mutex<Vec<_>>` side tables to hold
        // boxed integers and CBGR ThinRef/FatRef payloads. In batch test
        // runs these tables accumulate entries across tests and retain
        // indices referenced by stale `Value`s from prior interpreters,
        // causing state carryover. Clear them here so each test starts
        // from a pristine side-table state. Safe because the previous
        // interpreter (and therefore every `Value` it held) has been
        // dropped by the time we get here.
        verum_vbc::reset_global_value_tables();

        // Step 1: Compile AST to VBC
        let vbc_module = self.compile_ast_to_vbc(module)?;

        // Step 2: Create VBC interpreter with output capture enabled
        let mut interpreter = VbcInterpreter::new(vbc_module);
        interpreter.state.enable_output_capture();
        interpreter.state.config.count_instructions = true;
        interpreter.state.config.max_instructions = 1_000_000_000; // 1B instruction limit for tests (minimax/DP algorithms)
        // Wire cancel_flag from compiler options to interpreter for cooperative abort
        interpreter.state.config.cancel_flag = self.session.options().cancel_flag.clone();

        // Step 2b: Skip global constructors in test mode.
        // Global ctors are primarily FFI library initializers (e.g., kernel32.dll)
        // that fail on macOS and corrupt interpreter state. VBC interpreter tests
        // don't need FFI initialization.

        // Step 3: Try main() first, fall back to @test function discovery
        if let Ok(main_func_id) = self.find_main_function_id(&interpreter.state.module) {
            // Traditional mode: execute main()
            debug!("Executing main function via VBC interpreter (function ID: {})", main_func_id.0);
            let result = interpreter.execute_function(main_func_id);

            let stdout = interpreter.state.take_stdout();
            let stderr = interpreter.state.take_stderr();

            let exit_code = match result {
                // Tier-parity: if `main` returns an Int, that IS the exit
                // code (same contract as the AOT main→C-exit mapping).
                // Without this, differential tests would see
                // interpreter=0 / AOT=1 for any `fn main() -> Int { 1 }`.
                Ok(value) => {
                    if value.is_int() {
                        value.as_i64() as i32
                    } else {
                        0
                    }
                }
                Err(ref e) => {
                    let error_msg = format!("{}", e);
                    if stderr.is_empty() {
                        return Ok((stdout, error_msg, 1));
                    } else {
                        return Ok((stdout, format!("{}\n{}", stderr, error_msg), 1));
                    }
                }
            };

            return Ok((stdout, stderr, exit_code));
        }

        // @test mode: discover and run all @test-annotated functions.
        // Only user code functions have is_test=true (stdlib @test propagation is disabled).
        let test_functions: Vec<(VbcFunctionId, String)> = interpreter.state.module.functions
            .iter()
            .enumerate()
            .filter(|(_, desc)| desc.is_test)
            .map(|(idx, desc)| {
                let name = interpreter.state.module.get_string(desc.name)
                    .unwrap_or("unknown")
                    .to_string();
                (VbcFunctionId(idx as u32), name)
            })
            .collect();

        if test_functions.is_empty() {
            return Err(anyhow::anyhow!("No main function or @test functions found in VBC module"));
        }

        let total = test_functions.len();
        debug!("Running {} @test functions", total);

        let mut passed = 0usize;
        let mut failed = 0usize;
        let mut failures: Vec<(String, String)> = Vec::new();

        for (func_id, test_name) in &test_functions {
            let result = interpreter.execute_function(*func_id);

            match result {
                Ok(_) => {
                    passed += 1;
                    interpreter.state.writeln_stdout(&format!("  PASS: {}", test_name));
                }
                Err(e) => {
                    failed += 1;
                    let err_msg = format!("{}", e);
                    interpreter.state.writeln_stdout(&format!("  FAIL: {} — {}", test_name, err_msg));
                    failures.push((test_name.clone(), err_msg));
                }
            }

            // Unwind any frames left behind by a panic/error so that the next
            // test runs from a clean call-stack state. Normal returns pop their
            // own frames via `do_return`, but a panic aborts mid-execution.
            while !interpreter.state.call_stack.is_empty() {
                if let Ok(frame) = interpreter.state.call_stack.pop_frame() {
                    interpreter.state.registers.pop_frame(frame.reg_base);
                } else {
                    break;
                }
            }
            // Also clear context entries that were provided inside the test
            // but never ended (e.g. panicked before CtxEnd).
            interpreter.state.context_stack.clear();
            // Clear any pending exception so the next test starts clean.
            interpreter.state.current_exception = None;
            interpreter.state.exception_handlers.clear();
        }

        // Summary
        interpreter.state.writeln_stdout(&format!(
            "\n{}/{} tests passed, {} failed",
            passed, total, failed
        ));

        let stdout = interpreter.state.take_stdout();
        let stderr = interpreter.state.take_stderr();
        let exit_code = if failed > 0 { 1 } else { 0 };

        Ok((stdout, stderr, exit_code))
    }

    /// Phase 7 (Tier 1): Compile to native executable (AOT mode)
    ///
    /// This compiles the source to a standalone native executable that can be run
    /// independently. Uses the VBC → LLVM IR path (NOT MLIR, which is GPU-only).
    ///
    /// Pipeline: Source → AST → TypedAST → VBC → LLVM IR → Object → Executable
    ///
    /// See the Phase 7 architecture comment above `phase_interpret()` for details.
    pub fn run_native_compilation(&mut self) -> Result<PathBuf> {
        let start = Instant::now();
        let _bc_native = verum_error::breadcrumb::enter(
            "compiler.run_native_compilation",
            self.session
                .options()
                .input
                .display()
                .to_string(),
        );

        // Phase 0: Load stdlib modules (populates self.modules for type checking)
        let _bc_stdlib = verum_error::breadcrumb::enter("compiler.phase.stdlib_loading", "");
        let t0 = Instant::now();
        self.load_stdlib_modules()?;
        let stdlib_time = t0.elapsed();
        self.session.record_phase_metrics("Stdlib Loading", stdlib_time, 0);
        drop(_bc_stdlib);

        // Phase 0.5: Load sibling project modules (enables cross-file mount imports)
        let _bc_proj = verum_error::breadcrumb::enter("compiler.phase.project_modules", "");
        let t0 = Instant::now();
        self.load_project_modules()?;
        self.load_external_cog_modules()?;
        self.session.record_phase_metrics("Project Modules", t0.elapsed(), 0);
        drop(_bc_proj);

        // Phase 1: Load source
        let _bc_load = verum_error::breadcrumb::enter("compiler.phase.load_source", "");
        let file_id = self.phase_load_source()?;
        drop(_bc_load);

        // Phase 2: Parse (phase_parse records its own timing)
        let _bc_parse = verum_error::breadcrumb::enter("compiler.phase.parse", "");
        let module = self.phase_parse(file_id)?;
        drop(_bc_parse);

        // Phase 2.5: Scan for @device(gpu) annotations to auto-enable GPU compilation.
        // Gated on [codegen].mlir_gpu: when false, GPU annotations are
        // silently ignored (the code compiles as CPU-only). This lets
        // projects disable GPU compilation without removing @device(gpu)
        // annotations from source.
        let gpu_enabled = self.session.language_features().gpu_enabled();
        if gpu_enabled && !self.session.options().is_no_gpu() {
            let gpu_detected = Self::detect_gpu_kernels(&module);
            if gpu_detected {
                info!("Detected @device(gpu) annotations — GPU compilation path will be enabled");
                self.session.options_mut().has_gpu_kernels = true;
            }
        }

        // Phase 2.9: Safety gate (unsafe, @ffi) — always runs,
        // matching the interpreter path. No-op fast path when both
        // `[safety].unsafe_allowed` and `[safety].ffi` are true.
        self.phase_safety_gate(&module)?;

        // Phase 3: Type check (uses self.modules for stdlib type/method registration)
        let _bc_tc = verum_error::breadcrumb::enter("compiler.phase.type_check", "");
        let t0 = Instant::now();
        self.phase_type_check(&module)?;
        self.session.record_phase_metrics("Type Checking", t0.elapsed(), 0);
        drop(_bc_tc);

        // ════════════════════════════════════════════════════════════
        // POST-TYPECHECK PARALLEL FAN-OUT (#104, parallel-monad pipeline)
        //
        // The six post-typecheck gates are pure read-only walks over
        // the typed AST that share NO mutable data:
        //
        //   * dependency_analysis — target-profile enforcement
        //   * verify              — refinement + theorem SMT (Z3+CVC5)
        //   * context_validation  — DI / negative-constraint checks
        //   * send_sync_validation — Send/Sync compile-time enforcement
        //   * cbgr_analysis       — tier promotion (~15ns→0ns refs)
        //   * ffi_validation      — boundary checks
        //
        // Each gate's only sink is `Session::*` writers, which are all
        // `&self` with internal locking (lock-free SegQueue for
        // diagnostics post-#105; RwLock-protected metrics/cache).
        // Their compile-time `&self` signatures (post-#100/#104 audit)
        // make this fan-out structurally safe — the borrow checker
        // proves there are no aliased mutable references to compiler
        // state across worker threads.
        //
        // Wall-clock model for production builds:
        //
        //   Sequential (pre-#104):    Σ phase_i  (~6 fully serialised)
        //   Parallel    (post-#104): max phase_i (slowest dominates)
        //
        // For SMT-heavy modules, `phase_verify` dominates by 5-10× over
        // every other gate, so the overall win is "every other gate is
        // free" — the slowest call sets the floor.
        //
        // Opt-out: `VERUM_NO_PARALLEL_POST_TYPECHECK=1` falls back to
        // the sequential chain for diagnostic-ordering reproducibility.
        // ════════════════════════════════════════════════════════════
        let parallel_post_typecheck =
            std::env::var("VERUM_NO_PARALLEL_POST_TYPECHECK").is_err();

        // `clear_non_compilable_stdlib_modules` is a `&mut self` write
        // to `self.modules` — must run BEFORE the parallel fan-out so
        // none of the parallel readers observe a torn HashMap. Cheap
        // (just removes entries from a HashMap), so the serialisation
        // is essentially free.
        self.clear_non_compilable_stdlib_modules(Some(&module));

        let smt_enabled = self.session.options().verify_mode.use_smt();

        if parallel_post_typecheck {
            // rayon::scope provides structured parallelism: every
            // spawned task must complete before the scope exits, so
            // the surrounding `&self` borrow stays live for the whole
            // fan-out. No dynamic dispatch, no Arc cloning, no
            // owned-data marshalling — workers borrow `module` and
            // `self` directly.
            let dep_result = std::sync::Mutex::new(Ok(()));
            let verify_result = std::sync::Mutex::new(Ok(()));
            let cbgr_result = std::sync::Mutex::new(Ok(()));
            let ffi_result = std::sync::Mutex::new(Ok(()));

            let dep_metrics = std::sync::Mutex::new(std::time::Duration::ZERO);
            let verify_metrics = std::sync::Mutex::new(std::time::Duration::ZERO);
            let cbgr_metrics = std::sync::Mutex::new(std::time::Duration::ZERO);

            rayon::scope(|s| {
                s.spawn(|_| {
                    let _bc = verum_error::breadcrumb::enter(
                        "compiler.phase.dependency_analysis",
                        "parallel",
                    );
                    let t0 = Instant::now();
                    let r = self.phase_dependency_analysis(&module);
                    *dep_metrics.lock().unwrap() = t0.elapsed();
                    *dep_result.lock().unwrap() = r;
                });

                if smt_enabled {
                    s.spawn(|_| {
                        let _bc = verum_error::breadcrumb::enter(
                            "compiler.phase.verify",
                            "parallel",
                        );
                        let t0 = Instant::now();
                        let r = self.phase_verify(&module);
                        *verify_metrics.lock().unwrap() = t0.elapsed();
                        *verify_result.lock().unwrap() = r;
                    });
                }

                s.spawn(|_| {
                    self.phase_context_validation(&module);
                });

                s.spawn(|_| {
                    self.phase_send_sync_validation(&module);
                });

                s.spawn(|_| {
                    let _bc = verum_error::breadcrumb::enter(
                        "compiler.phase.cbgr_analysis",
                        "parallel",
                    );
                    let t0 = Instant::now();
                    let r = self.phase_cbgr_analysis(&module);
                    *cbgr_metrics.lock().unwrap() = t0.elapsed();
                    *cbgr_result.lock().unwrap() = r;
                });

                s.spawn(|_| {
                    let _bc = verum_error::breadcrumb::enter(
                        "compiler.phase.ffi_validation",
                        "parallel",
                    );
                    *ffi_result.lock().unwrap() = self.phase_ffi_validation(&module);
                });
            });

            // Surface the first error in deterministic order (matches
            // the sequential chain's bail order so error-pinning tests
            // remain stable).
            dep_result.into_inner().unwrap()?;
            if smt_enabled {
                verify_result.into_inner().unwrap()?;
            }
            cbgr_result.into_inner().unwrap()?;
            ffi_result.into_inner().unwrap()?;

            // Persist phase metrics now that the workers have settled.
            self.session.record_phase_metrics(
                "Dependency Analysis",
                dep_metrics.into_inner().unwrap(),
                0,
            );
            if smt_enabled {
                self.session.record_phase_metrics(
                    "Verification",
                    verify_metrics.into_inner().unwrap(),
                    0,
                );
            }
            self.session.record_phase_metrics(
                "CBGR Analysis",
                cbgr_metrics.into_inner().unwrap(),
                0,
            );
        } else {
            // Sequential fallback — bit-identical to the pre-#104 path.
            let t0 = Instant::now();
            self.phase_dependency_analysis(&module)?;
            self.session.record_phase_metrics("Dependency Analysis", t0.elapsed(), 0);

            if smt_enabled {
                let t0 = Instant::now();
                self.phase_verify(&module)?;
                self.session.record_phase_metrics("Verification", t0.elapsed(), 0);
            }

            self.phase_context_validation(&module);
            self.phase_send_sync_validation(&module);

            let t0 = Instant::now();
            self.phase_cbgr_analysis(&module)?;
            self.session.record_phase_metrics("CBGR Analysis", t0.elapsed(), 0);

            self.phase_ffi_validation(&module)?;
        }

        // Phase 5c: rayon fence before LLVM codegen.
        //
        // LLVM registers backend passes lazily via function-local
        // statics guarded by __cxa_guard_acquire (Itanium C++ ABI).
        // While the main thread was inside that guard, rayon workers
        // parked after stdlib parsing would race the same guard's
        // wake-path, corrupting its semaphore state on arm64 macOS —
        // observable as a ~70% SIGSEGV in phase_generate_native in
        // release builds.
        //
        // `rayon::broadcast(|_| ())` dispatches a no-op to every
        // worker and waits for completion, which is a true fence: all
        // workers run, exit their wake path, and re-park *before* we
        // touch LLVM's cxa-guards. Combined with the eager
        // `Target::initialize_native` in `verum_cli::main` (which
        // pre-populates the IR-pass half of the same registry), this
        // eliminates the race.
        //
        // Diagnosed via `verum diagnose` crash reports showing 14/14
        // stacks at `__os_semaphore_wait` → `callDefaultCtor<*Pass>`.
        {
            let _bc_barrier =
                verum_error::breadcrumb::enter("compiler.phase.rayon_fence", "broadcast");
            let _ = rayon::broadcast(|_| ());
        }

        // Phase 6: Generate native code (CPU path) — the hot spot where
        // the documented Z3/LLVM teardown race manifests. Mark the
        // breadcrumb with the input file so the crash report points the
        // reader straight at the translation unit.
        let _bc_codegen = verum_error::breadcrumb::enter(
            "compiler.phase.generate_native",
            self.session
                .options()
                .input
                .display()
                .to_string(),
        );
        let t0 = Instant::now();
        let output_path = self.phase_generate_native(&module)?;
        self.session.record_phase_metrics("Code Generation", t0.elapsed(), 0);
        drop(_bc_codegen);

        // Phase 6b: GPU compilation (MLIR path) — auto-triggered by @device(gpu) detection
        // Runs alongside CPU compilation to produce GPU kernel binaries.
        if self.session.options().has_gpu_kernels {
            info!("Auto-detected GPU kernels — running MLIR GPU compilation");
            let t0 = Instant::now();
            match self.run_mlir_aot() {
                Ok(gpu_binary) => {
                    info!("GPU compilation produced: {}", gpu_binary.display());
                    self.session.record_phase_metrics("GPU Compilation", t0.elapsed(), 0);
                }
                Err(e) => {
                    // GPU compilation failure is non-fatal — CPU binary is still valid
                    warn!("GPU compilation failed (CPU binary still valid): {}", e);
                }
            }
        }

        // Save incremental compilation cache for next build.
        self.save_incremental_cache();

        let elapsed = start.elapsed();
        info!(
            "Native compilation completed in {:.2}s",
            elapsed.as_secs_f64()
        );

        Ok(output_path)
    }

    /// Phase 6: Generate native executable
}
