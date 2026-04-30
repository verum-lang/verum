//! MLIR JIT/AOT compilation (GPU code path).
//!
//! Extracted from `pipeline.rs` (#106 Phase 15). Houses the
//! AST → Verum MLIR dialect → LLVM dialect → JIT/AOT path used
//! exclusively for GPU targets (`@device(GPU)` annotation or
//! tensor-op threshold detection).  CPU code never touches MLIR;
//! it uses LLVM IR directly via `pipeline/native_codegen.rs`.
//!
//! Methods:
//!
//!   * `run_mlir_jit` — JIT-compile + execute the module via
//!     MLIR ExecutionEngine; entry point for `verum run --mlir`.
//!   * `execute_mlir_jit` — internal helper that drives the
//!     ExecutionEngine after lowering.
//!   * `run_mlir_aot` — AOT-compile to a GPU binary (PTX,
//!     HSACO, SPIR-V, Metal); entry point for `verum build --gpu`.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context as AnyhowContext, Result};
use tracing::{debug, info, warn};

use verum_common::{List, Text};

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {

    // ==================== MLIR COMPILATION ====================

    /// Run MLIR-based JIT compilation (experimental)
    ///
    /// This compilation path uses:
    /// 1. AST → Verum MLIR dialect lowering
    /// 2. CBGR elimination and context monomorphization passes
    /// 3. Progressive lowering to LLVM dialect
    /// 4. JIT compilation via ExecutionEngine
    ///
    /// Benefits over direct LLVM:
    /// - Domain-specific optimizations via custom MLIR passes
    /// - Better debugging via MLIR's verifier
    /// - Reusable transformations for different backends
    ///
    /// MLIR is used for GPU targets only; CPU code uses LLVM IR directly.
    pub fn run_mlir_jit(&mut self, args: List<Text>) -> Result<()> {
        use verum_codegen::mlir::MlirContext;

        let start = Instant::now();
        info!("Starting MLIR JIT compilation");

        // Phase 1: Load source
        let file_id = self.phase_load_source()?;

        // Phase 2: Parse
        let module = self.phase_parse(file_id)?;

        // Phase 3: Type check
        self.phase_type_check(&module)?;

        // Phase 4: Convert AST to VBC bytecode
        info!("  Converting AST to VBC bytecode");
        let codegen_config = CodegenConfig {
            module_name: "mlir_jit".to_string(),
            debug_info: self.session.options().verbose > 0,
            optimization_level: self.session.options().optimization_level,
            ..Default::default()
        };
        let mut vbc_codegen = VbcCodegen::with_config(codegen_config);
        let vbc_module = vbc_codegen.compile_module(&module)
            .with_context(|| "Failed to compile AST to VBC")?;

        info!("  VBC bytecode generated: {} functions", vbc_module.functions.len());

        // Phase 5: Create MLIR context and GPU lowering
        // Note: MLIR JIT is specifically for GPU tensor operations
        info!("  Lowering VBC to MLIR for GPU");
        use verum_codegen::mlir::{VbcToMlirGpuLowering, GpuLoweringConfig, GpuTarget};

        let mlir_ctx = MlirContext::new()
            .with_context(|| "Failed to create MLIR context")?;

        // Select GPU target: [codegen].gpu_backend overrides auto-detect.
        let gpu_backend = self
            .session
            .language_features()
            .codegen
            .gpu_backend
            .as_str();
        let gpu_target = match gpu_backend {
            "metal" => GpuTarget::Metal,
            "cuda" => GpuTarget::Cuda,
            "rocm" => GpuTarget::Cuda, // ROCm uses HIP → CUDA path
            "vulkan" => GpuTarget::Cuda, // Vulkan→SPIR-V → CUDA fallback
            _ => {
                // "auto" or unknown → platform-based detection
                if cfg!(target_os = "macos") {
                    GpuTarget::Metal
                } else {
                    GpuTarget::Cuda
                }
            }
        };
        let gpu_config = GpuLoweringConfig {
            target: gpu_target,
            opt_level: self.session.options().optimization_level,
            enable_tensor_cores: !cfg!(target_os = "macos"), // Not for Metal
            max_shared_memory: if cfg!(target_os = "macos") { 32 * 1024 } else { 48 * 1024 },
            default_block_size: [256, 1, 1],
            enable_async_copy: true,
            debug_info: self.session.options().verbose > 0,
        };

        let mut gpu_lowering = VbcToMlirGpuLowering::new(mlir_ctx.context(), gpu_config);
        let mlir_module = gpu_lowering.lower_module(&vbc_module)
            .with_context(|| "Failed to lower VBC to MLIR")?;

        // Phase 6: Execute via JIT
        info!("  JIT compiling and executing");

        // Print MLIR if verbose
        if self.session.options().verbose > 0 {
            let mlir_str = format!("{}", mlir_module.as_operation());
            info!("Generated MLIR:\n{}", mlir_str);
        }

        // Try to create JIT engine and execute
        match self.execute_mlir_jit(&mlir_module) {
            Ok(exit_code) => {
                let elapsed = start.elapsed();
                info!(
                    "MLIR JIT execution completed in {:.2}s with exit code {}",
                    elapsed.as_secs_f64(),
                    exit_code
                );
                Ok(())
            }
            Err(e) => {
                // JIT execution failed - fall back to interpreter
                warn!("MLIR JIT execution failed: {} - falling back to interpreter", e);
                self.mode = CompilationMode::Interpret;
                self.run_interpreter(args)
            }
        }
    }

    /// Execute an MLIR module using the JIT engine.
    fn execute_mlir_jit(&self, module: &verum_codegen::verum_mlir::ir::Module<'_>) -> Result<i64> {
        use verum_codegen::mlir::jit::{JitEngine, JitConfig};

        // Create JIT configuration
        let jit_config = JitConfig::new()
            .with_optimization_level(self.session.options().optimization_level as usize)
            .with_verbose(self.session.options().verbose > 0);

        // Create JIT engine
        let engine = JitEngine::new(module, jit_config)
            .with_context(|| "Failed to create JIT engine")?;

        // Register stdlib symbols
        engine.register_stdlib()
            .with_context(|| "Failed to register stdlib symbols")?;

        // Look up and call main function
        if engine.lookup("main").is_some() {
            info!("  Executing main function via JIT");
            // SAFETY: main has known signature () -> i64
            let result = engine.call_i64("main", &[])?;
            Ok(result)
        } else if engine.lookup("_start").is_some() {
            info!("  Executing _start function via JIT");
            // SAFETY: _start has known signature () -> ()
            unsafe {
                engine.call_void("_start")?;
            }
            Ok(0)
        } else {
            // No entry point found
            warn!("No main or _start function found in MLIR module");
            Ok(0)
        }
    }

    /// Run MLIR-based AOT compilation (experimental)
    ///
    /// Similar to run_mlir_jit but produces an executable instead of running directly.
    /// Uses VBC → MLIR path for GPU tensor operations.
    pub fn run_mlir_aot(&mut self) -> Result<PathBuf> {
        use verum_codegen::mlir::{
            MlirContext, MlirConfig, MlirCodegen,
            VbcToMlirGpuLowering, GpuLoweringConfig, GpuTarget,
        };

        let start = Instant::now();
        info!("Starting MLIR AOT compilation (GPU path)");

        // Phase 1: Load source
        let file_id = self.phase_load_source()?;

        // Phase 2: Parse
        let module = self.phase_parse(file_id)?;

        // Phase 2.9: Safety gate — explicit for parity with CPU AOT.
        self.phase_safety_gate(&module)?;

        // Phase 3: Type check
        self.phase_type_check(&module)?;

        // Phase 3.5: Dependency analysis (target-profile enforcement).
        self.phase_dependency_analysis(&module)?;

        // Phase 4: Refinement verification (if enabled).
        // GPU kernels can carry refinement types and contracts; they
        // deserve the same SMT pass as the CPU AOT path.
        if self.session.options().verify_mode.use_smt() {
            self.phase_verify(&module)?;
        }

        // Phase 4c-4e: Context, send/sync, FFI validation —
        // identical to the CPU AOT path. GPU code that uses `using
        // [...]`, crosses thread boundaries, or declares FFI gets
        // the same checks as CPU code.
        self.phase_context_validation(&module);
        self.phase_send_sync_validation(&module);
        self.phase_ffi_validation(&module)?;

        // Phase 5: CBGR analysis (tier promotion decisions).
        self.phase_cbgr_analysis(&module)?;

        // Phase 6: Multi-module VBC codegen (resolves stdlib imports)
        info!("  Converting AST to VBC bytecode (multi-module)");
        let vbc_module = self.compile_ast_to_vbc(&module)?;
        info!("  VBC bytecode generated: {} functions", vbc_module.functions.len());

        // Phase 5: Monomorphization (specialize generics)
        let vbc_module = {
            use crate::phases::vbc_mono::VbcMonomorphizationPhase;
            let mono = VbcMonomorphizationPhase::new();
            let mono = if !self.session.language_features().codegen.monomorphization_cache {
                mono.without_cache()
            } else { mono };
            let mut mono = mono;
            match mono.monomorphize(&vbc_module) {
                Ok(specialized) => {
                    info!("  Monomorphization complete: {} functions", specialized.functions.len());
                    std::sync::Arc::new(specialized)
                }
                Err(diags) => {
                    warn!("  Monomorphization had {} diagnostics, using unspecialized module", diags.len());
                    vbc_module
                }
            }
        };

        // Phase 6: Create MLIR context and GPU lowering
        let mlir_ctx = MlirContext::new()
            .with_context(|| "Failed to create MLIR context")?;

        // Auto-select GPU target based on platform
        let gpu_target = if cfg!(target_os = "macos") {
            GpuTarget::Metal  // Apple Silicon (M1/M2/M3)
        } else {
            GpuTarget::Cuda   // Default to NVIDIA on Linux/Windows
        };

        let gpu_config = GpuLoweringConfig {
            target: gpu_target,
            opt_level: self.session.options().optimization_level,
            enable_tensor_cores: !cfg!(target_os = "macos"),
            max_shared_memory: if cfg!(target_os = "macos") { 32 * 1024 } else { 48 * 1024 },
            default_block_size: [256, 1, 1],
            enable_async_copy: true,
            debug_info: self.session.options().verbose > 0,
        };

        info!("  Lowering VBC to MLIR for GPU (target: {:?})", gpu_target);
        let mut gpu_lowering = VbcToMlirGpuLowering::new(mlir_ctx.context(), gpu_config);
        let _mlir_module = gpu_lowering.lower_module(&vbc_module)
            .with_context(|| "Failed to lower VBC to MLIR")?;

        info!("  GPU lowering stats: {} tensor ops, {} kernel launches",
            gpu_lowering.stats().tensor_ops, gpu_lowering.stats().kernel_launches);

        // Phase 7: Run GPU pass pipeline (tensor→linalg→scf→gpu→target)
        let mlir_config = MlirConfig::new("gpu_module")
            .with_optimization_level(self.session.options().optimization_level)
            .with_debug_info(self.session.options().verbose > 0);

        let mut codegen = MlirCodegen::new(&mlir_ctx, mlir_config)
            .map_err(|e| anyhow::anyhow!("MLIR codegen init failed: {:?}", e))?;

        codegen.lower_vbc_module(&vbc_module, gpu_target)
            .map_err(|e| anyhow::anyhow!("MLIR VBC lowering failed: {:?}", e))?;

        let gpu_result = codegen.optimize_gpu(gpu_target)
            .map_err(|e| anyhow::anyhow!("GPU pass pipeline failed: {:?}", e))?;

        info!("  GPU pass pipeline completed: {} phases run", gpu_result.completed_phases.len());

        // Phase 8: Print MLIR for debugging
        if self.session.options().verbose > 0 {
            if let Ok(mlir_str) = codegen.get_mlir_string() {
                info!("Generated MLIR:\n{}", mlir_str);
            }
        }

        let elapsed = start.elapsed();
        info!(
            "MLIR AOT compilation completed in {:.2}s",
            elapsed.as_secs_f64()
        );

        // Phase 9: GPU binary emission — translate MLIR→LLVM IR + extract kernels
        use verum_codegen::mlir::gpu_binary::GpuBinaryEmitter;

        let emitter = GpuBinaryEmitter::new(gpu_target, self.session.options().verbose > 0);
        let mlir_module = codegen.module()
            .map_err(|e| anyhow::anyhow!("Failed to get MLIR module: {:?}", e))?;

        match emitter.emit(mlir_module) {
            Ok(gpu_output) => {
                info!(
                    "GPU binary emission complete: {} kernel module(s), {} bytes, host IR {} bytes",
                    gpu_output.kernel_binaries.len(),
                    gpu_output.total_binary_size,
                    gpu_output.host_llvm_ir.len(),
                );

                // Phase 10: Compile host LLVM IR + link with GPU binaries
                //
                // Write the host LLVM IR to a temp file, then pass it to
                // the native compilation pipeline which handles LLVM IR → object → link.
                let build_dir = std::env::temp_dir().join("verum_gpu_build");
                std::fs::create_dir_all(&build_dir)
                    .with_context(|| format!("Failed to create build dir: {}", build_dir.display()))?;

                let host_ir_path = build_dir.join("gpu_host.ll");
                std::fs::write(&host_ir_path, &gpu_output.host_llvm_ir)
                    .with_context(|| "Failed to write host LLVM IR")?;

                // Write kernel binaries for runtime loading
                for (i, kb) in gpu_output.kernel_binaries.iter().enumerate() {
                    let kernel_path = build_dir.join(format!("gpu_kernel_{}.bin", i));
                    std::fs::write(&kernel_path, &kb.data)
                        .with_context(|| format!("Failed to write kernel binary {}", i))?;
                    info!("  Kernel module '{}': {} bytes → {}",
                        kb.module_name, kb.data.len(), kernel_path.display());
                }

                // Fall through to native compilation for host code.
                // The GPU kernels are either:
                // - Embedded in the LLVM IR as global constants (from MLIR binary pass)
                // - Written as separate files for runtime loading
                // - Using built-in shader library (Metal METAL_SHADER_SOURCE)
                info!("Compiling host code via LLVM native path...");
                self.run_native_compilation()
            }
            Err(e) => {
                // GPU binary emission failed — fall back gracefully to CPU.
                // This is non-fatal: the program will still run correctly,
                // just without GPU acceleration.
                warn!(
                    "GPU binary emission failed: {:?}. \
                     Falling back to CPU-only compilation. \
                     GPU tensor ops will execute on CPU via VBC interpreter.",
                    e
                );
                self.run_native_compilation()
            }
        }
    }
}
