//! VBC → LLVM IR lowering + native-binary generation.
//!
//! Extracted from `pipeline.rs` (#106 Phase 20). Houses the
//! AOT-only LLVM lowering surface that translates VBC bytecode
//! to LLVM IR (CPU code path) and drives the native-binary
//! generation steps that follow:
//!
//!   * `lower_vbc_to_llvm` — core CPU compilation step;
//!     translates VBC bytecode instructions to LLVM IR via
//!     `VbcToLlvmLowering`, applying tier-aware CBGR
//!     optimisations.
//!   * `execute_llvm_jit` — JIT-execute the LLVM module via
//!     ExecutionEngine; called from the VBC JIT path.
//!   * `generate_native_from_llvm` — emit native object file
//!     from LLVM IR (post-optimisation), invoke the linker.
//!   * `analyze_compilation_paths` — pre-codegen analysis that
//!     classifies each function as CPU / GPU / Both for routing
//!     to the appropriate code-generator.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tracing::{debug, info};

use verum_ast::Module;
use verum_codegen::llvm::{
    LoweringConfig as LlvmLoweringConfig, LoweringStats as LlvmLoweringStats, VbcToLlvmLowering,
};
use verum_vbc::module::VbcModule;

use crate::compilation_path::{
    CompilationPath, TargetConfig as PathTargetConfig, analyze_function,
    determine_compilation_path,
};

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    /// Lower a VBC module to LLVM IR.
    ///
    /// This is the core of the CPU compilation path. It translates VBC bytecode
    /// instructions to LLVM IR, applying tier-aware CBGR optimizations.
    pub(super) fn lower_vbc_to_llvm<'ctx>(
        &self,
        llvm_context: &'ctx verum_codegen::llvm::verum_llvm::context::Context,
        vbc_module: &std::sync::Arc<verum_vbc::module::VbcModule>,
    ) -> Result<(verum_codegen::llvm::verum_llvm::module::Module<'ctx>, LlvmLoweringStats)> {
        let input_path = &self.session.options().input;
        let module_name = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("main");

        // Configure lowering based on session options
        let opt_level = self.session.options().optimization_level;
        let config = if opt_level >= 2 {
            LlvmLoweringConfig::release(module_name)
        } else if opt_level == 0 {
            LlvmLoweringConfig::debug(module_name)
        } else {
            LlvmLoweringConfig::new(module_name).with_opt_level(opt_level)
        };

        // Wire debug info and coverage flags
        let config = config
            .with_debug_info(self.session.options().debug_info)
            .with_coverage(self.session.options().coverage);

        // Set target triple for the host
        let config = config.with_target(verum_codegen::llvm::verum_llvm::targets::TargetMachine::get_default_triple().as_str().to_string_lossy());

        // Wire the AOT permission policy into lowering. `None` is the
        // trusted-application default — `PermissionAssert` is elided.
        // `Some` makes the lowerer bake the policy into every
        // permission gate at compile time, sealing the resolved
        // grants in the binary so `--aot` runs of script-shaped
        // sources enforce identically to the interpreter.
        let config = config.with_permission_policy(self.session.aot_permission_policy());

        info!("  Lowering VBC to LLVM IR (opt level: {})", opt_level);

        // Run CBGR escape analysis on decoded VBC functions
        let escape_result = {
            use verum_vbc::cbgr_analysis::VbcEscapeAnalyzer;
            let analyzer = VbcEscapeAnalyzer::new();
            let functions: Vec<verum_vbc::VbcFunction> = vbc_module.functions.iter()
                .filter_map(|f| {
                    f.instructions.as_ref().map(|instrs| {
                        verum_vbc::VbcFunction::new(f.clone(), instrs.clone())
                    })
                })
                .collect();
            let result = analyzer.analyze(&functions);
            info!("  CBGR escape analysis: {} refs analyzed, {} promoted to tier1 ({:.1}%)",
                result.stats.total_refs,
                result.stats.promoted_to_tier1,
                result.stats.promotion_rate());
            result
        };

        let mut lowering = VbcToLlvmLowering::new(llvm_context, config);
        lowering.set_escape_analysis(escape_result);
        lowering.lower_module(vbc_module)
            .map_err(|e| anyhow::anyhow!("VBC → LLVM lowering failed: {}", e))?;

        let stats = lowering.stats().clone();
        let llvm_module = lowering.into_module();

        // Optionally dump IR for debugging
        if self.session.options().verbose > 1 {
            debug!("Generated LLVM IR:\n{}", llvm_module.print_to_string().to_string_lossy());
        }

        Ok((llvm_module, stats))
    }

    /// Execute an LLVM module using the JIT engine.
    pub(super) fn execute_llvm_jit(
        &self,
        llvm_module: &verum_codegen::llvm::verum_llvm::module::Module<'_>,
        _ast_module: &Module, // Reserved for future: extract metadata for runtime
    ) -> Result<i64> {
        info!("  Creating LLVM JIT execution engine");

        // Create JIT execution engine
        let execution_engine = llvm_module
            .create_jit_execution_engine(verum_codegen::llvm::verum_llvm::OptimizationLevel::Default)
            .map_err(|e| anyhow::anyhow!("Failed to create JIT engine: {}", e))?;

        // Look up main function
        // SAFETY: get_function requires unsafe because it can return arbitrary function pointers.
        // We're looking for known entry points that we've compiled with expected signatures.
        if let Ok(main_fn) = unsafe {
            execution_engine.get_function::<unsafe extern "C" fn() -> i64>("main")
        } {
            info!("  Executing main function via LLVM JIT");
            // SAFETY: We've compiled main with the expected signature
            let result = unsafe { main_fn.call() };
            Ok(result)
        } else {
            // Try _start as fallback
            if let Ok(start_fn) = unsafe {
                execution_engine.get_function::<unsafe extern "C" fn()>("_start")
            } {
                info!("  Executing _start function via LLVM JIT");
                // SAFETY: We've compiled _start with the expected signature
                unsafe { start_fn.call() };
                Ok(0)
            } else {
                Err(anyhow::anyhow!("No main or _start function found"))
            }
        }
    }

    /// Generate a native executable from an LLVM module.
    pub(super) fn generate_native_from_llvm(
        &self,
        llvm_module: &verum_codegen::llvm::verum_llvm::module::Module<'_>,
    ) -> Result<PathBuf> {
        use verum_codegen::llvm::verum_llvm::targets::{
            InitializationConfig, Target, TargetMachine, RelocMode, CodeModel, FileType,
        };

        // Initialize LLVM targets ONCE per process.
        {
            static INIT: std::sync::Once = std::sync::Once::new();
            INIT.call_once(|| {
                let _ = Target::initialize_native(&InitializationConfig::default());
            });
        }

        // Get input path and determine output paths
        let input_path = &self.session.options().input;
        let project_root = self.get_project_root(input_path);

        let profile = if self.session.options().optimization_level >= 2 {
            "release"
        } else {
            "debug"
        };

        let target_dir = project_root.join("target");
        let profile_dir = target_dir.join(profile);
        let build_dir = target_dir.join("build");

        std::fs::create_dir_all(&profile_dir)?;
        std::fs::create_dir_all(&build_dir)?;

        let module_name = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("main");

        let output_path = if self.session.options().output.to_str().unwrap_or("").is_empty() {
            profile_dir.join(if cfg!(windows) {
                format!("{}.exe", module_name)
            } else {
                module_name.to_string()
            })
        } else {
            self.session.options().output.clone()
        };

        // Create target machine
        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple)
            .map_err(|e| anyhow::anyhow!("Failed to get target: {}", e))?;

        let opt_level = match self.session.options().optimization_level {
            0 => verum_codegen::llvm::verum_llvm::OptimizationLevel::None,
            1 => verum_codegen::llvm::verum_llvm::OptimizationLevel::Less,
            2 => verum_codegen::llvm::verum_llvm::OptimizationLevel::Default,
            _ => verum_codegen::llvm::verum_llvm::OptimizationLevel::Aggressive,
        };

        let target_machine = target
            .create_target_machine(
                &triple,
                "generic",
                "",
                opt_level,
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or_else(|| anyhow::anyhow!("Failed to create target machine"))?;

        // Write object file
        let obj_path = build_dir.join(format!("{}.o", module_name));
        info!("  Writing object file to {}", obj_path.display());

        target_machine
            .write_to_file(llvm_module, FileType::Object, &obj_path)
            .map_err(|e| anyhow::anyhow!("Failed to write object file: {}", e))?;

        // Generate runtime stubs
        let runtime_stubs_path = self.generate_runtime_stubs(&build_dir)?;
        let runtime_obj = self.compile_c_file(&runtime_stubs_path, &build_dir)?;

        // Link into executable
        info!("  Linking executable to {}", output_path.display());
        self.link_executable(&[obj_path, runtime_obj], &output_path)?;

        Ok(output_path)
    }

    /// Analyze VBC module to determine compilation paths for each function.
    ///
    /// This phase analyzes the VBC bytecode to determine whether functions
    /// should be compiled via the CPU path (LLVM IR) or GPU path (MLIR).
    ///
    /// # Arguments
    ///
    /// * `vbc_module` - The VBC module to analyze
    /// * `target_config` - Target configuration (GPU availability, thresholds, etc.)
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if all functions can be compiled, or an error if GPU
    /// compilation is required but unavailable.
    pub(super) fn analyze_compilation_paths(
        &self,
        vbc_module: &std::sync::Arc<verum_vbc::module::VbcModule>,
        target_config: &PathTargetConfig,
    ) -> Result<()> {
        use tracing::{debug, warn};

        let mut cpu_count = 0usize;
        let mut gpu_count = 0usize;
        let mut hybrid_count = 0usize;
        let mut total_tensor_ops = 0usize;
        let mut total_gpu_ops = 0usize;

        for func_desc in &vbc_module.functions {
            let func_name = vbc_module
                .strings
                .get(func_desc.name)
                .unwrap_or("<unknown>");

            // Analyze the function
            let analysis = match analyze_function(func_desc, vbc_module) {
                Ok(a) => a,
                Err(e) => {
                    debug!(
                        "  Function '{}': analysis skipped ({})",
                        func_name,
                        e
                    );
                    // Skip functions that can't be analyzed (e.g., no bytecode)
                    cpu_count += 1;
                    continue;
                }
            };

            // Determine compilation path
            let path = determine_compilation_path(&analysis, target_config);

            // Track statistics
            total_tensor_ops += analysis.tensor_op_count;
            total_gpu_ops += analysis.gpu_op_count;

            match &path {
                CompilationPath::Cpu => {
                    cpu_count += 1;
                    debug!(
                        "  Function '{}': CPU path ({} instructions, {} tensor ops)",
                        func_name, analysis.instruction_count, analysis.tensor_op_count
                    );
                }
                CompilationPath::Gpu => {
                    gpu_count += 1;
                    debug!(
                        "  Function '{}': GPU path ({} GPU ops, {} tensor ops)",
                        func_name, analysis.gpu_op_count, analysis.tensor_op_count
                    );

                    // Currently, GPU path requires MLIR which isn't wired for VBC yet
                    if !target_config.has_gpu {
                        warn!(
                            "Function '{}' requires GPU but no GPU target available, falling back to CPU",
                            func_name
                        );
                    }
                }
                CompilationPath::Hybrid { gpu_regions } => {
                    hybrid_count += 1;
                    debug!(
                        "  Function '{}': Hybrid path ({} CPU + {} GPU regions)",
                        func_name,
                        analysis.instruction_count - analysis.gpu_op_count,
                        gpu_regions.len()
                    );
                }
            }
        }

        info!(
            "Compilation path analysis: {} CPU, {} GPU, {} hybrid functions ({} tensor ops, {} GPU ops total)",
            cpu_count, gpu_count, hybrid_count, total_tensor_ops, total_gpu_ops
        );

        // For now, we only support CPU path - error on GPU-only functions
        if gpu_count > 0 && !target_config.has_gpu {
            warn!(
                "{} functions require GPU compilation but will use CPU fallback",
                gpu_count
            );
        }

        Ok(())
    }
}
