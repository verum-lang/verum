//! Tier-specific pipeline constructors + VBC JIT/AOT entries.
//!
//! Extracted from `pipeline.rs` (#106 Phase 19). Houses the
//! sibling-mode constructor variants and their corresponding
//! tier-execution entry points:
//!
//!   * `new_mlir_jit` / `new_mlir_aot` — MLIR JIT / AOT
//!     constructors (GPU code path).
//!   * `new_vbc_jit` / `new_vbc_aot` — VBC JIT / AOT
//!     constructors (CPU code path).
//!   * `run_vbc_jit` — JIT-execute the VBC module via the
//!     interpreter and return the i64 exit code.
//!   * `run_vbc_aot` — AOT-compile the VBC module to a native
//!     binary (sibling of `run_native_compilation` for the
//!     explicit-tier path).

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use tracing::info;

use crate::compilation_path::TargetConfig as PathTargetConfig;
use crate::session::Session;

use super::{CompilationMode, CompilationPipeline};

impl<'s> CompilationPipeline<'s> {

    /// Create a pipeline for MLIR JIT mode
    pub fn new_mlir_jit(session: &'s mut Session) -> Self {
        let mut pipeline = Self::new(session);
        pipeline.mode = CompilationMode::MlirJit;
        pipeline
    }

    /// Create a pipeline for MLIR AOT mode
    pub fn new_mlir_aot(session: &'s mut Session) -> Self {
        let mut pipeline = Self::new(session);
        pipeline.mode = CompilationMode::MlirAot;
        pipeline
    }

    // ==================== VBC → LLVM COMPILATION ====================
    //
    // These methods implement the CPU compilation path using the new
    // VBC → LLVM IR lowering infrastructure.
    //
    // Architecture:
    //   AST → VBC (verum_vbc) → LLVM IR (verum_llvm) → Native Code
    //
    // This path is used for:
    // - Tier 1/2 JIT: Hot path optimization
    // - Tier 3 AOT: Ahead-of-time compilation to native executables

    /// Create a pipeline for VBC → LLVM JIT mode.
    ///
    /// This mode compiles Verum source through VBC to LLVM IR, then executes
    /// immediately using LLVM's JIT engine. This is the preferred path for:
    /// - Development/debugging with fast iteration
    /// - Hot path optimization (Tier 1/2)
    pub fn new_vbc_jit(session: &'s mut Session) -> Self {
        let mut pipeline = Self::new(session);
        pipeline.mode = CompilationMode::Jit;
        pipeline
    }

    /// Create a pipeline for VBC → LLVM AOT mode.
    ///
    /// This mode compiles Verum source through VBC to LLVM IR, then generates
    /// a native executable. This is the preferred path for:
    /// - Production builds (Tier 3)
    /// - Distribution as standalone executables
    pub fn new_vbc_aot(session: &'s mut Session) -> Self {
        let mut pipeline = Self::new(session);
        pipeline.mode = CompilationMode::Aot;
        pipeline
    }

    /// Run VBC → LLVM JIT compilation and execution.
    ///
    /// This is the main entry point for the CPU JIT compilation path:
    /// 1. Parse source to AST
    /// 2. Type check
    /// 3. CBGR analysis (determines tier for each reference)
    /// 4. Compile AST to VBC
    /// 5. Lower VBC to LLVM IR
    /// 6. Execute via LLVM JIT
    ///
    /// # Returns
    ///
    /// Returns the exit code from the main function, or an error if compilation fails.
    pub fn run_vbc_jit(&mut self) -> Result<i64> {
        let start = Instant::now();
        info!("Starting VBC → LLVM JIT compilation");

        // Phase 1: Load source
        let file_id = self.phase_load_source()?;

        // Phase 2: Parse
        let module = self.phase_parse(file_id)?;

        // Phase 3: Type check
        self.phase_type_check(&module)?;

        // Phase 4: Refinement verification (if enabled)
        if self.session.options().verify_mode.use_smt() {
            self.phase_verify(&module)?;
        }

        // Phase 5: CBGR analysis
        self.phase_cbgr_analysis(&module)?;

        // Phase 6: Compile AST to VBC
        let vbc_module = self.compile_ast_to_vbc(&module)?;

        // Phase 6.5: Compilation path analysis
        let target_config = PathTargetConfig::cpu_only(); // CPU-only for now
        self.analyze_compilation_paths(&vbc_module, &target_config)?;

        // Phase 7: Lower VBC to LLVM IR (CPU path)
        let llvm_context = verum_codegen::llvm::verum_llvm::context::Context::create();
        let (llvm_module, stats) = self.lower_vbc_to_llvm(&llvm_context, &vbc_module)?;

        info!(
            "VBC → LLVM lowering complete: {} functions, {} instructions, {:.1}% CBGR elimination",
            stats.functions_lowered,
            stats.instructions_lowered,
            stats.elimination_rate() * 100.0
        );

        // Phase 8: Execute via JIT
        let result = self.execute_llvm_jit(&llvm_module, &module)?;

        let elapsed = start.elapsed();
        info!(
            "VBC JIT execution completed in {:.2}s with exit code {}",
            elapsed.as_secs_f64(),
            result
        );

        Ok(result)
    }

    /// Run VBC → LLVM AOT compilation.
    ///
    /// This is the main entry point for the CPU AOT compilation path:
    /// 1. Parse source to AST
    /// 2. Type check
    /// 3. CBGR analysis
    /// 4. Compile AST to VBC
    /// 5. Lower VBC to LLVM IR
    /// 6. Optimize LLVM IR
    /// 7. Generate object file
    /// 8. Link into executable
    ///
    /// # Returns
    ///
    /// Returns the path to the generated executable.
    pub fn run_vbc_aot(&mut self) -> Result<PathBuf> {
        let start = Instant::now();
        info!("Starting VBC → LLVM AOT compilation");

        // Phase 1: Load source
        let file_id = self.phase_load_source()?;

        // Phase 2: Parse
        let module = self.phase_parse(file_id)?;

        // Phase 3: Type check
        self.phase_type_check(&module)?;

        // Phase 4: Refinement verification (if enabled)
        if self.session.options().verify_mode.use_smt() {
            self.phase_verify(&module)?;
        }

        // Phase 5: CBGR analysis
        self.phase_cbgr_analysis(&module)?;

        // Phase 6: Compile AST to VBC
        let vbc_module = self.compile_ast_to_vbc(&module)?;

        // Phase 6.5: Compilation path analysis
        let target_config = PathTargetConfig::cpu_only(); // CPU-only for now
        self.analyze_compilation_paths(&vbc_module, &target_config)?;

        // Phase 7: Lower VBC to LLVM IR (CPU path)
        let llvm_context = verum_codegen::llvm::verum_llvm::context::Context::create();
        let (llvm_module, stats) = self.lower_vbc_to_llvm(&llvm_context, &vbc_module)?;

        info!(
            "VBC → LLVM lowering complete: {} functions, {} instructions, {:.1}% CBGR elimination",
            stats.functions_lowered,
            stats.instructions_lowered,
            stats.elimination_rate() * 100.0
        );

        // Phase 8: Generate native executable
        let output_path = self.generate_native_from_llvm(&llvm_module)?;

        let elapsed = start.elapsed();
        info!(
            "VBC AOT compilation completed in {:.2}s: {}",
            elapsed.as_secs_f64(),
            output_path.display()
        );

        Ok(output_path)
    }
}
