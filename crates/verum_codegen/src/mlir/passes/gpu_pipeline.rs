//! GPU pass pipeline configuration and execution.
//!
//! Manages the sequence of MLIR passes for GPU code generation.
//! This pipeline is separate from the CPU pass pipeline because GPU
//! compilation requires fundamentally different pass stages:
//!
//! ```text
//! Phase 1: Early Optimizations
//!   ├── Canonicalization
//!   └── CSE
//!
//! Phase 2: Tensor-to-Linalg Conversion
//!   └── tensor → linalg (named ops)
//!
//! Phase 3: Linalg Optimizations
//!   ├── Element-wise fusion
//!   ├── Fold unit extent dims
//!   └── Inline scalar operands
//!
//! Phase 4: Linalg → Parallel Loops
//!   └── linalg → scf.parallel
//!
//! Phase 5: GPU Mapping
//!   ├── parallel loops → gpu.launch
//!   ├── GPU kernel outlining
//!   └── GPU launch sink index computations
//!
//! Phase 6: GPU Optimizations
//!   ├── GPU decompose memrefs
//!   ├── GPU eliminate barriers
//!   └── GPU async region (optional)
//!
//! Phase 7: Target-Specific Lowering
//!   ├── Attach target (NVVM / ROCDL / SPIRV)
//!   └── GPU ops → target ops (nvvm / rocdl / spirv)
//!
//! Phase 8: Host Code Lowering
//!   ├── SCF → CF
//!   ├── Host code → LLVM
//!   └── GPU → LLVM (host-side runtime calls)
//!
//! Phase 9: GPU Binary Generation
//!   └── gpu.module → binary (PTX / HSACO / SPIR-V)
//! ```

use crate::mlir::error::{MlirError, Result};
use crate::mlir::vbc_lowering::GpuTarget;

use verum_mlir::{
    Context,
    ir::Module,
    ir::operation::OperationLike,
    pass::PassManager,
};
use verum_common::Text;
use std::time::Instant;

/// Configuration for the GPU pass pipeline.
#[derive(Debug, Clone)]
pub struct GpuPassConfig {
    /// GPU target platform.
    pub target: GpuTarget,

    /// Optimization level (0-3).
    pub optimization_level: u8,

    /// Enable async GPU operations.
    pub enable_async: bool,

    /// Enable tensor core utilization (NVIDIA).
    pub enable_tensor_cores: bool,

    /// Enable verbose logging.
    pub verbose: bool,

    /// Enable verification after each pass phase.
    pub verify_after_each_phase: bool,
}

impl Default for GpuPassConfig {
    fn default() -> Self {
        Self {
            target: GpuTarget::Cuda,
            optimization_level: 2,
            enable_async: false,
            enable_tensor_cores: true,
            verbose: false,
            verify_after_each_phase: true,
        }
    }
}

impl GpuPassConfig {
    /// Create config for CUDA target.
    pub fn cuda() -> Self {
        Self::default()
    }

    /// Create config for ROCm target.
    pub fn rocm() -> Self {
        Self {
            target: GpuTarget::Rocm,
            enable_tensor_cores: false,
            ..Default::default()
        }
    }

    /// Create config for Vulkan target.
    pub fn vulkan() -> Self {
        Self {
            target: GpuTarget::Vulkan,
            enable_tensor_cores: false,
            enable_async: false,
            ..Default::default()
        }
    }

    /// Create config for Metal target.
    pub fn metal() -> Self {
        Self {
            target: GpuTarget::Metal,
            enable_tensor_cores: false,
            enable_async: false,
            ..Default::default()
        }
    }

    /// Set optimization level.
    pub fn with_optimization_level(mut self, level: u8) -> Self {
        self.optimization_level = level.min(3);
        self
    }

    /// Set verbose logging.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }
}

/// Statistics from the GPU pass pipeline run.
#[derive(Debug, Clone, Default)]
pub struct GpuPipelineStats {
    /// Time for early optimizations (microseconds).
    pub early_opts_time_us: u64,
    /// Time for tensor-to-linalg conversion (microseconds).
    pub tensor_to_linalg_time_us: u64,
    /// Time for linalg optimizations (microseconds).
    pub linalg_opts_time_us: u64,
    /// Time for GPU mapping (microseconds).
    pub gpu_mapping_time_us: u64,
    /// Time for GPU optimizations (microseconds).
    pub gpu_opts_time_us: u64,
    /// Time for target-specific lowering (microseconds).
    pub target_lowering_time_us: u64,
    /// Time for host code lowering (microseconds).
    pub host_lowering_time_us: u64,
    /// Time for GPU binary generation (microseconds).
    pub gpu_binary_time_us: u64,
    /// Total pipeline time (microseconds).
    pub total_time_us: u64,
    /// Number of pass phases completed.
    pub phases_completed: usize,
}

impl GpuPipelineStats {
    /// Get summary string.
    pub fn summary(&self) -> String {
        format!(
            "GPU Pipeline: {}/{} phases, {:.2}ms total \
             (early: {:.2}ms, t2l: {:.2}ms, linalg: {:.2}ms, \
              mapping: {:.2}ms, opts: {:.2}ms, target: {:.2}ms, \
              host: {:.2}ms, binary: {:.2}ms)",
            self.phases_completed,
            9, // total phases
            self.total_time_us as f64 / 1000.0,
            self.early_opts_time_us as f64 / 1000.0,
            self.tensor_to_linalg_time_us as f64 / 1000.0,
            self.linalg_opts_time_us as f64 / 1000.0,
            self.gpu_mapping_time_us as f64 / 1000.0,
            self.gpu_opts_time_us as f64 / 1000.0,
            self.target_lowering_time_us as f64 / 1000.0,
            self.host_lowering_time_us as f64 / 1000.0,
            self.gpu_binary_time_us as f64 / 1000.0,
        )
    }
}

/// Result of running the GPU pass pipeline.
#[derive(Debug, Clone, Default)]
pub struct GpuPipelineResult {
    /// Whether the pipeline completed successfully.
    pub success: bool,
    /// Pass phases that completed.
    pub completed_phases: Vec<Text>,
    /// Statistics.
    pub stats: GpuPipelineStats,
}

/// GPU pass pipeline for Verum MLIR.
///
/// Orchestrates the full GPU compilation pass sequence:
/// VBC MLIR → linalg → scf.parallel → gpu.launch → target binary.
///
/// This pipeline uses MLIR's built-in GPU passes from LLVM 21.x
/// to perform kernel outlining, target attachment, and binary generation.
pub struct GpuPassPipeline<'c> {
    /// MLIR context.
    context: &'c Context,
    /// Configuration.
    config: GpuPassConfig,
}

impl<'c> GpuPassPipeline<'c> {
    /// Create a new GPU pass pipeline.
    pub fn new(context: &'c Context, config: GpuPassConfig) -> Self {
        Self { context, config }
    }

    /// Run the complete GPU pass pipeline on a module.
    ///
    /// Executes all phases in order, verifying the module after each
    /// phase if verification is enabled. Returns statistics about
    /// each phase's timing.
    pub fn run(&self, module: &mut Module<'c>) -> Result<GpuPipelineResult> {
        let total_start = Instant::now();
        let mut result = GpuPipelineResult::default();
        let mut stats = GpuPipelineStats::default();

        // Phase 1: Early optimizations (canonicalize, CSE)
        self.run_phase(module, &mut stats, &mut result, "early-opts", |pm| {
            use verum_mlir::pass::transform;

            pm.add_pass(transform::create_canonicalizer());
            pm.add_pass(transform::create_cse());
        }, |s, elapsed| s.early_opts_time_us = elapsed)?;

        // Phase 2: Tensor → Linalg conversion
        self.run_phase(module, &mut stats, &mut result, "tensor-to-linalg", |pm| {
            use verum_mlir::pass::conversion;

            pm.add_pass(conversion::create_tensor_to_linalg());
        }, |s, elapsed| s.tensor_to_linalg_time_us = elapsed)?;

        // Phase 3: Linalg optimizations
        if self.config.optimization_level >= 1 {
            self.run_phase(module, &mut stats, &mut result, "linalg-opts", |pm| {
                use verum_mlir::pass::linalg;

                pm.add_pass(linalg::create_linalg_elementwise_op_fusion_pass());
                pm.add_pass(linalg::create_linalg_fold_unit_extent_dims_pass());
                pm.add_pass(linalg::create_linalg_inline_scalar_operands_pass());
            }, |s, elapsed| s.linalg_opts_time_us = elapsed)?;
        }

        // Phase 4: Linalg → Parallel Loops
        self.run_phase(module, &mut stats, &mut result, "linalg-to-parallel", |pm| {
            use verum_mlir::pass::linalg;

            pm.add_pass(linalg::create_convert_linalg_to_parallel_loops_pass());
        }, |s, elapsed| {
            // Counted in gpu_mapping_time
            s.gpu_mapping_time_us += elapsed;
        })?;

        // Phase 5: GPU mapping (parallel loops → gpu.launch + kernel outlining)
        self.run_phase(module, &mut stats, &mut result, "gpu-mapping", |pm| {
            use verum_mlir::pass::gpu;

            pm.add_pass(gpu::create_gpu_map_parallel_loops_pass());
            pm.add_pass(gpu::create_gpu_kernel_outlining_pass());
            pm.add_pass(gpu::create_gpu_launch_sink_index_computations_pass());
        }, |s, elapsed| s.gpu_mapping_time_us += elapsed)?;

        // Phase 6: GPU optimizations
        if self.config.optimization_level >= 1 {
            self.run_phase(module, &mut stats, &mut result, "gpu-opts", |pm| {
                use verum_mlir::pass::gpu;

                pm.add_pass(gpu::create_gpu_decompose_memrefs_pass());
                pm.add_pass(gpu::create_gpu_eliminate_barriers());

                if self.config.enable_async {
                    pm.add_pass(gpu::create_gpu_async_region_pass());
                }
            }, |s, elapsed| s.gpu_opts_time_us = elapsed)?;
        }

        // Phase 7: Target-specific lowering (attach target + convert ops)
        self.run_phase(module, &mut stats, &mut result, "target-lowering", |pm| {
            use verum_mlir::pass::{gpu, conversion};

            match self.config.target {
                GpuTarget::Cuda => {
                    pm.add_pass(gpu::create_gpu_nvvm_attach_target());
                    pm.add_pass(conversion::create_gpu_ops_to_nvvm_ops());
                }
                GpuTarget::Rocm => {
                    pm.add_pass(gpu::create_gpu_rocdl_attach_target());
                    pm.add_pass(conversion::create_gpu_ops_to_rocdl_ops());
                }
                GpuTarget::Vulkan => {
                    pm.add_pass(gpu::create_gpu_spirv_attach_target());
                    pm.add_pass(conversion::create_gpu_to_spirv());
                }
                GpuTarget::Metal => {
                    // Metal uses SPIRV as intermediate for now
                    pm.add_pass(gpu::create_gpu_spirv_attach_target());
                    pm.add_pass(conversion::create_gpu_to_spirv());
                }
            }
        }, |s, elapsed| s.target_lowering_time_us = elapsed)?;

        // Phase 8: Host code lowering (host-side GPU runtime calls → LLVM)
        self.run_phase(module, &mut stats, &mut result, "host-lowering", |pm| {
            use verum_mlir::pass::conversion;

            pm.add_pass(conversion::create_scf_to_control_flow());
            pm.add_pass(conversion::create_gpu_to_llvm());
            pm.add_pass(conversion::create_lower_host_code_to_llvm());
            pm.add_pass(conversion::create_to_llvm());
            pm.add_pass(conversion::create_reconcile_unrealized_casts());
        }, |s, elapsed| s.host_lowering_time_us = elapsed)?;

        // Phase 9: GPU module → binary (PTX/HSACO/SPIR-V)
        self.run_phase(module, &mut stats, &mut result, "gpu-binary", |pm| {
            use verum_mlir::pass::gpu;

            pm.add_pass(gpu::create_gpu_module_to_binary_pass());
        }, |s, elapsed| s.gpu_binary_time_us = elapsed)?;

        // Final verification
        if !module.as_operation().verify() {
            return Err(MlirError::verification(
                "Module verification failed after GPU pass pipeline"
            ));
        }

        stats.total_time_us = total_start.elapsed().as_micros() as u64;

        if self.config.verbose {
            tracing::info!("{}", stats.summary());
        }

        result.success = true;
        result.stats = stats;
        Ok(result)
    }

    /// Run a single phase of the pipeline.
    ///
    /// Creates a fresh PassManager, configures it with the provided closure,
    /// runs it, and records timing. If the phase fails, returns an error
    /// with the phase name for diagnostics.
    fn run_phase(
        &self,
        module: &mut Module<'c>,
        stats: &mut GpuPipelineStats,
        result: &mut GpuPipelineResult,
        phase_name: &str,
        configure: impl FnOnce(&PassManager<'c>),
        record_time: impl FnOnce(&mut GpuPipelineStats, u64),
    ) -> Result<()> {
        if self.config.verbose {
            tracing::info!("GPU pipeline phase: {}", phase_name);
        }

        let pm = PassManager::new(self.context);
        pm.enable_verifier(self.config.verify_after_each_phase);

        configure(&pm);

        let start = Instant::now();
        pm.run(module)
            .map_err(|_| MlirError::PassPipelineError {
                message: Text::from(format!("GPU pass phase '{}' failed", phase_name)),
            })?;
        let elapsed = start.elapsed().as_micros() as u64;

        record_time(stats, elapsed);
        stats.phases_completed += 1;
        result.completed_phases.push(Text::from(phase_name));

        Ok(())
    }

    /// Get the configuration.
    pub fn config(&self) -> &GpuPassConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_pass_config_default() {
        let config = GpuPassConfig::default();
        assert_eq!(config.target, GpuTarget::Cuda);
        assert_eq!(config.optimization_level, 2);
        assert!(config.enable_tensor_cores);
        assert!(!config.enable_async);
    }

    #[test]
    fn test_gpu_pass_config_cuda() {
        let config = GpuPassConfig::cuda();
        assert_eq!(config.target, GpuTarget::Cuda);
    }

    #[test]
    fn test_gpu_pass_config_rocm() {
        let config = GpuPassConfig::rocm();
        assert_eq!(config.target, GpuTarget::Rocm);
        assert!(!config.enable_tensor_cores);
    }

    #[test]
    fn test_gpu_pass_config_vulkan() {
        let config = GpuPassConfig::vulkan();
        assert_eq!(config.target, GpuTarget::Vulkan);
        assert!(!config.enable_async);
    }

    #[test]
    fn test_gpu_pass_config_metal() {
        let config = GpuPassConfig::metal();
        assert_eq!(config.target, GpuTarget::Metal);
    }

    #[test]
    fn test_gpu_pass_config_builder() {
        let config = GpuPassConfig::cuda()
            .with_optimization_level(3)
            .with_verbose(true);
        assert_eq!(config.optimization_level, 3);
        assert!(config.verbose);
    }

    #[test]
    fn test_gpu_pipeline_stats_summary() {
        let stats = GpuPipelineStats {
            early_opts_time_us: 100,
            tensor_to_linalg_time_us: 200,
            linalg_opts_time_us: 150,
            gpu_mapping_time_us: 300,
            gpu_opts_time_us: 50,
            target_lowering_time_us: 400,
            host_lowering_time_us: 100,
            gpu_binary_time_us: 500,
            total_time_us: 1800,
            phases_completed: 9,
        };
        let summary = stats.summary();
        assert!(summary.contains("GPU Pipeline"));
        assert!(summary.contains("9/9 phases"));
    }

    #[test]
    fn test_gpu_pipeline_result_default() {
        let result = GpuPipelineResult::default();
        assert!(!result.success);
        assert!(result.completed_phases.is_empty());
    }
}
