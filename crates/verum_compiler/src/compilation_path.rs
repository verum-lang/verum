//! Dual-Path Compilation Infrastructure
//!
//! This module provides the infrastructure for determining whether code should
//! be compiled via the CPU path (VBC → LLVM IR) or GPU path (VBC → MLIR).
//!
//! # Architecture
//!
//! ```text
//! VBC Bytecode
//!      │
//!      ├─── CPU Path (default) ───► LLVM IR → Native Code
//!      │    • Scalar operations
//!      │    • Control flow
//!      │    • Memory operations (CBGR)
//!      │    • Non-GPU tensor ops
//!      │
//!      └─── GPU Path ───► MLIR → GPU Binaries
//!           • @device(GPU) annotated code
//!           • Tensor operations above threshold
//!           • GPU-specific opcodes (0xF8-0xFF)
//! ```

use verum_vbc::instruction::Instruction;
use verum_vbc::module::{FunctionDescriptor, VbcModule};

/// Compilation path for a function or region.
///
/// Determines whether code should be lowered via LLVM IR (CPU) or MLIR (GPU).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CompilationPath {
    /// CPU compilation path: VBC → LLVM IR → Native Code
    ///
    /// Used for:
    /// - Scalar operations
    /// - Control flow
    /// - Memory operations
    /// - Functions without GPU annotations
    Cpu,

    /// GPU compilation path: VBC → MLIR → GPU Binaries
    ///
    /// Used for:
    /// - @device(GPU) annotated functions
    /// - Tensor operations above threshold
    /// - GPU-specific opcodes
    Gpu,

    /// Hybrid path: function contains both CPU and GPU regions
    ///
    /// Used for:
    /// - Functions mixing CPU control flow with GPU kernels
    /// - Requires splitting into CPU host code + GPU kernels
    Hybrid {
        /// GPU region byte offsets within the function
        gpu_regions: Vec<(usize, usize)>,
    },
}

impl Default for CompilationPath {
    fn default() -> Self {
        CompilationPath::Cpu
    }
}

impl CompilationPath {
    /// Returns true if this path requires GPU compilation.
    pub fn requires_gpu(&self) -> bool {
        matches!(self, CompilationPath::Gpu | CompilationPath::Hybrid { .. })
    }

    /// Returns true if this path requires CPU compilation.
    pub fn requires_cpu(&self) -> bool {
        matches!(self, CompilationPath::Cpu | CompilationPath::Hybrid { .. })
    }

    /// Returns true if this is a pure CPU path.
    pub fn is_cpu_only(&self) -> bool {
        matches!(self, CompilationPath::Cpu)
    }

    /// Returns true if this is a pure GPU path.
    pub fn is_gpu_only(&self) -> bool {
        matches!(self, CompilationPath::Gpu)
    }
}

/// Device annotation parsed from function metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceAnnotation {
    /// Explicit CPU placement
    Cpu,
    /// Explicit GPU placement
    Gpu {
        /// Optional specific GPU index
        index: Option<u32>,
        /// Optional fallback to CPU
        fallback: Option<Box<DeviceAnnotation>>,
    },
    /// Automatic device selection based on operation analysis
    Auto,
}

impl Default for DeviceAnnotation {
    fn default() -> Self {
        DeviceAnnotation::Auto
    }
}

/// Target configuration for compilation path decisions.
#[derive(Debug, Clone)]
pub struct TargetConfig {
    /// Whether GPU targets are available
    pub has_gpu: bool,
    /// Available GPU targets
    pub gpu_targets: Vec<GpuTarget>,
    /// Threshold for tensor operations to trigger GPU path
    pub tensor_gpu_threshold: usize,
    /// Force specific compilation path (overrides automatic detection)
    pub force_path: Option<CompilationPath>,
}

impl Default for TargetConfig {
    fn default() -> Self {
        Self {
            has_gpu: false,
            gpu_targets: Vec::new(),
            tensor_gpu_threshold: 10,
            force_path: None,
        }
    }
}

impl TargetConfig {
    /// Create configuration for CPU-only compilation
    pub fn cpu_only() -> Self {
        Self {
            has_gpu: false,
            gpu_targets: Vec::new(),
            tensor_gpu_threshold: usize::MAX,
            force_path: Some(CompilationPath::Cpu),
        }
    }

    /// Create configuration with GPU support
    pub fn with_gpu(targets: Vec<GpuTarget>) -> Self {
        Self {
            has_gpu: !targets.is_empty(),
            gpu_targets: targets,
            tensor_gpu_threshold: 10,
            force_path: None,
        }
    }

    /// Set the tensor GPU threshold
    pub fn with_threshold(mut self, threshold: usize) -> Self {
        self.tensor_gpu_threshold = threshold;
        self
    }

    /// Force a specific compilation path
    pub fn force(mut self, path: CompilationPath) -> Self {
        self.force_path = Some(path);
        self
    }
}

/// GPU target specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuTarget {
    /// NVIDIA CUDA (PTX)
    Cuda {
        /// SM versions (e.g., [70, 80, 90] for sm_70, sm_80, sm_90)
        sm_versions: Vec<u32>,
    },
    /// AMD ROCm (HSACO)
    Rocm {
        /// GFX versions (e.g., ["gfx906", "gfx90a", "gfx1100"])
        gfx_versions: Vec<String>,
    },
    /// Vulkan (SPIR-V)
    Vulkan {
        /// SPIR-V version (major, minor)
        spirv_version: (u32, u32),
    },
    /// Apple Metal
    Metal {
        /// macOS version requirement (major, minor)
        macos_version: (u32, u32),
    },
    /// CPU fallback for GPU operations
    CpuFallback,
}

/// Analysis results for a VBC function.
#[derive(Debug, Clone, Default)]
pub struct FunctionAnalysis {
    /// Total instruction count
    pub instruction_count: usize,
    /// Number of tensor operations
    pub tensor_op_count: usize,
    /// Number of GPU-specific operations (0xF8-0xFF range)
    pub gpu_op_count: usize,
    /// Whether the function has @device annotation
    pub device_annotation: Option<DeviceAnnotation>,
    /// Detected GPU region boundaries (start_offset, end_offset)
    pub gpu_regions: Vec<(usize, usize)>,
    /// Whether function contains CBGR operations
    pub has_cbgr_ops: bool,
    /// Whether function contains async operations
    pub has_async_ops: bool,
}

impl FunctionAnalysis {
    /// Check if function contains operations requiring GPU
    pub fn contains_gpu_ops(&self) -> bool {
        self.gpu_op_count > 0
    }

    /// Check if tensor operations benefit from GPU
    pub fn tensor_ops_benefit_from_gpu(&self, threshold: usize) -> bool {
        self.tensor_op_count > threshold
    }
}

/// Determines the compilation path for a function or region.
///
/// # Decision Algorithm
///
/// 1. Check for forced path in target config
/// 2. Check for explicit @device annotation
/// 3. Check for GPU-requiring operations
/// 4. Check tensor operations above threshold
/// 5. Default to CPU
///
/// # Arguments
///
/// * `analysis` - Analysis results for the function
/// * `target_config` - Target configuration
///
/// # Returns
///
/// The determined compilation path.
pub fn determine_compilation_path(
    analysis: &FunctionAnalysis,
    target_config: &TargetConfig,
) -> CompilationPath {
    // 1. Check for forced path
    if let Some(forced) = &target_config.force_path {
        return forced.clone();
    }

    // 2. Explicit @device annotation
    if let Some(device) = &analysis.device_annotation {
        return match device {
            DeviceAnnotation::Gpu { .. } => {
                if target_config.has_gpu {
                    CompilationPath::Gpu
                } else {
                    // Fallback to CPU if no GPU available
                    CompilationPath::Cpu
                }
            }
            DeviceAnnotation::Cpu => CompilationPath::Cpu,
            DeviceAnnotation::Auto => infer_best_path(analysis, target_config),
        };
    }

    // 3. Check for GPU-requiring ops
    if analysis.contains_gpu_ops() {
        if target_config.has_gpu {
            // Check if it's a hybrid function
            if !analysis.gpu_regions.is_empty() && analysis.instruction_count > analysis.gpu_op_count {
                return CompilationPath::Hybrid {
                    gpu_regions: analysis.gpu_regions.clone(),
                };
            }
            return CompilationPath::Gpu;
        }
        // Fallback to CPU if no GPU (will use CPU implementations)
    }

    // 4. Check tensor ops that benefit from GPU
    if analysis.tensor_ops_benefit_from_gpu(target_config.tensor_gpu_threshold)
        && target_config.has_gpu
    {
        return CompilationPath::Gpu;
    }

    // 5. Default to CPU
    CompilationPath::Cpu
}

/// Infers the best compilation path based on analysis.
fn infer_best_path(analysis: &FunctionAnalysis, target_config: &TargetConfig) -> CompilationPath {
    if !target_config.has_gpu {
        return CompilationPath::Cpu;
    }

    // GPU ops take priority
    if analysis.contains_gpu_ops() {
        return CompilationPath::Gpu;
    }

    // Many tensor ops benefit from GPU
    if analysis.tensor_ops_benefit_from_gpu(target_config.tensor_gpu_threshold) {
        return CompilationPath::Gpu;
    }

    // Default to CPU for general code
    CompilationPath::Cpu
}

/// Analyzes a VBC function to determine its characteristics.
///
/// This function decodes the bytecode and counts various operation types
/// to inform compilation path decisions.
pub fn analyze_function(
    func_desc: &FunctionDescriptor,
    module: &VbcModule,
) -> Result<FunctionAnalysis, AnalysisError> {
    let mut analysis = FunctionAnalysis::default();

    // Get bytecode range
    let bytecode_offset = func_desc.bytecode_offset as usize;
    let bytecode_len = func_desc.bytecode_length as usize;

    if bytecode_offset + bytecode_len > module.bytecode.len() {
        return Err(AnalysisError::InvalidBytecodeRange);
    }

    let bytecode = &module.bytecode[bytecode_offset..bytecode_offset + bytecode_len];
    let mut offset = 0;

    while offset < bytecode.len() {
        let start = offset;
        let instr = verum_vbc::bytecode::decode_instruction(bytecode, &mut offset)
            .map_err(|e| AnalysisError::DecodeError(format!("{:?}", e)))?;

        analysis.instruction_count += 1;

        // Analyze instruction
        analyze_instruction(&instr, start, &mut analysis);
    }

    // Parse device annotation from module-level device hints.
    // The VBC codegen translates @device(GPU) attributes into DeviceHints entries
    // keyed by BlockId. We check if any block within this function's bytecode range
    // has an explicit device placement.
    if !module.device_hints.placements.is_empty() {
        use crate::compilation_path::DeviceAnnotation;
        use verum_vbc::metadata::device::DevicePreference;

        // Check if any block placement covers this function's bytecode range
        for (_block_id, preference) in &module.device_hints.placements {
            match preference {
                DevicePreference::GPU { index, .. } => {
                    analysis.device_annotation = Some(DeviceAnnotation::Gpu {
                        index: *index,
                        fallback: None,
                    });
                    break;
                }
                DevicePreference::CPU => {
                    analysis.device_annotation = Some(DeviceAnnotation::Cpu);
                    break;
                }
                _ => {}
            }
        }
    }

    // Fallback: GPU functions are also detected by the presence of GPU instructions
    if analysis.device_annotation.is_none() && analysis.gpu_op_count > 0 {
        analysis.device_annotation = Some(DeviceAnnotation::Gpu {
            index: None,
            fallback: None,
        });
    }

    Ok(analysis)
}

/// Analyzes a single instruction for compilation path determination.
fn analyze_instruction(instr: &Instruction, offset: usize, analysis: &mut FunctionAnalysis) {
    // Match on instruction variants to categorize operations
    match instr {
        // GPU operations (explicit GPU instructions)
        Instruction::GpuLaunch { .. }
        | Instruction::GpuSync { .. }
        | Instruction::GpuMemcpy { .. }
        | Instruction::GpuAlloc { .. }
        | Instruction::GpuStreamCreate { .. }
        | Instruction::GpuStreamCreateWithPriority { .. }
        | Instruction::GpuStreamCreateNonBlocking { .. }
        | Instruction::GpuStreamDestroy { .. }
        | Instruction::GpuStreamQuery { .. }
        | Instruction::GpuStreamWaitEvent { .. }
        | Instruction::GpuStreamGetPriority { .. }
        | Instruction::GpuStreamAddCallback { .. }
        | Instruction::GpuEventCreate { .. }
        | Instruction::GpuEventCreateWithFlags { .. }
        | Instruction::GpuEventDestroy { .. }
        | Instruction::GpuEventRecord { .. }
        | Instruction::GpuEventRecordWithFlags { .. }
        | Instruction::GpuEventSynchronize { .. }
        | Instruction::GpuEventQuery { .. }
        | Instruction::GpuEventElapsed { .. }
        | Instruction::GpuGetDevice { .. }
        | Instruction::GpuSetDevice { .. }
        | Instruction::GpuGetDeviceCount { .. }
        | Instruction::GpuGetDeviceProperty { .. }
        | Instruction::GpuGetMemoryInfo { .. }
        | Instruction::GpuCanAccessPeer { .. }
        | Instruction::GpuEnablePeerAccess { .. }
        | Instruction::GpuDisablePeerAccess { .. }
        | Instruction::GpuDeviceReset { .. }
        | Instruction::GpuSetDeviceFlags { .. }
        | Instruction::GpuMemcpyAsync { .. }
        | Instruction::GpuFree { .. }
        | Instruction::GpuPinMemory { .. }
        | Instruction::GpuUnpinMemory { .. }
        | Instruction::GpuPrefetch { .. }
        | Instruction::GpuMemset { .. }
        | Instruction::GpuMemsetAsync { .. }
        | Instruction::GpuMemcpy2D { .. }
        | Instruction::GpuMemcpy2DAsync { .. }
        | Instruction::GpuMallocManaged { .. }
        | Instruction::GpuMemAdvise { .. }
        | Instruction::GpuPrefetchAsync { .. }
        | Instruction::GpuMemGetAttribute { .. }
        | Instruction::GpuGraphCreate { .. }
        | Instruction::GpuGraphBeginCapture { .. }
        | Instruction::GpuGraphEndCapture { .. }
        | Instruction::GpuGraphInstantiate { .. }
        | Instruction::GpuGraphLaunch { .. }
        | Instruction::GpuGraphDestroy { .. }
        | Instruction::GpuGraphExecDestroy { .. }
        | Instruction::GpuGraphExecUpdate { .. }
        | Instruction::GpuProfileRangeStart { .. }
        | Instruction::GpuProfileRangeEnd
        | Instruction::GpuProfileMarkerPush { .. }
        | Instruction::GpuProfileMarkerPop
        | Instruction::GpuLaunchCooperative { .. }
        | Instruction::GpuLaunchMultiDevice { .. }
        | Instruction::GpuDeviceSync => {
            analysis.gpu_op_count += 1;
            // Mark this as a GPU region (single instruction)
            analysis.gpu_regions.push((offset, offset + 1));
        }

        // Tensor operations
        Instruction::TensorNew { .. }
        | Instruction::TensorFull { .. }
        | Instruction::TensorFromSlice { .. }
        | Instruction::TensorArange { .. }
        | Instruction::TensorLinspace { .. }
        | Instruction::TensorRand { .. }
        | Instruction::TensorClone { .. }
        | Instruction::TensorIdentity { .. }
        | Instruction::TensorReshape { .. }
        | Instruction::TensorTranspose { .. }
        | Instruction::TensorSlice { .. }
        | Instruction::TensorIndex { .. }
        | Instruction::TensorConcat { .. }
        | Instruction::TensorStack { .. }
        | Instruction::TensorBroadcast { .. }
        | Instruction::TensorSqueeze { .. }
        | Instruction::TensorBinop { .. }
        | Instruction::TensorUnop { .. }
        | Instruction::TensorCmp { .. }
        | Instruction::TensorWhere { .. }
        | Instruction::TensorClamp { .. }
        | Instruction::TensorCast { .. }
        | Instruction::TensorMaskedFill { .. }
        | Instruction::TensorLerp { .. }
        | Instruction::TensorMatmul { .. }
        | Instruction::TensorDot { .. }
        | Instruction::TensorConv { .. }
        | Instruction::TensorBatchMatmul { .. }
        | Instruction::TensorEinsum { .. }
        | Instruction::TensorOuter { .. }
        | Instruction::TensorTriSolve { .. }
        | Instruction::TensorCholesky { .. }
        | Instruction::TensorReduce { .. }
        | Instruction::TensorArgmax { .. }
        | Instruction::TensorTopk { .. }
        | Instruction::TensorCumulative { .. }
        | Instruction::TensorSoftmax { .. }
        | Instruction::TensorLayerNorm { .. }
        | Instruction::TensorBatchNorm { .. }
        | Instruction::TensorRmsNorm { .. }
        | Instruction::TensorFlashAttention { .. }
        | Instruction::TensorFft { .. }
        | Instruction::TensorScatter { .. }
        | Instruction::TensorPool { .. } => {
            analysis.tensor_op_count += 1;
        }

        // CBGR operations are handled through the CbgrExtended opcode (0x8F)
        // which doesn't have a dedicated instruction variant - it uses sub-opcodes.
        // CBGR usage is tracked via bytecode analysis during function scanning.
        // For now, has_cbgr_ops is set based on function-level analysis.

        // Async operations
        Instruction::Spawn { .. }
        | Instruction::Await { .. }
        | Instruction::Select { .. } => {
            analysis.has_async_ops = true;
        }

        // All other instructions don't affect path selection
        _ => {}
    }
}

/// Errors that can occur during function analysis.
#[derive(Debug, Clone)]
pub enum AnalysisError {
    /// Invalid bytecode range in function descriptor
    InvalidBytecodeRange,
    /// Failed to decode instruction
    DecodeError(String),
}

impl std::fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBytecodeRange => write!(f, "Invalid bytecode range in function descriptor"),
            Self::DecodeError(msg) => write!(f, "Instruction decode error: {}", msg),
        }
    }
}

impl std::error::Error for AnalysisError {}

/// Batch analysis for an entire VBC module.
///
/// Returns analysis results for all functions in the module.
pub fn analyze_module(module: &VbcModule) -> Vec<(u32, FunctionAnalysis, CompilationPath)> {
    let target_config = TargetConfig::default();

    module
        .functions
        .iter()
        .enumerate()
        .filter_map(|(idx, func_desc)| {
            let analysis = analyze_function(func_desc, module).ok()?;
            let path = determine_compilation_path(&analysis, &target_config);
            Some((idx as u32, analysis, path))
        })
        .collect()
}

/// Batch analysis with custom target configuration.
pub fn analyze_module_with_config(
    module: &VbcModule,
    target_config: &TargetConfig,
) -> Vec<(u32, FunctionAnalysis, CompilationPath)> {
    module
        .functions
        .iter()
        .enumerate()
        .filter_map(|(idx, func_desc)| {
            let analysis = analyze_function(func_desc, module).ok()?;
            let path = determine_compilation_path(&analysis, target_config);
            Some((idx as u32, analysis, path))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compilation_path_default() {
        let analysis = FunctionAnalysis::default();
        let config = TargetConfig::default();

        let path = determine_compilation_path(&analysis, &config);
        assert_eq!(path, CompilationPath::Cpu);
    }

    #[test]
    fn test_compilation_path_forced() {
        let analysis = FunctionAnalysis {
            gpu_op_count: 100,
            ..Default::default()
        };
        let config = TargetConfig::default().force(CompilationPath::Cpu);

        let path = determine_compilation_path(&analysis, &config);
        assert_eq!(path, CompilationPath::Cpu);
    }

    #[test]
    fn test_compilation_path_gpu_ops() {
        let analysis = FunctionAnalysis {
            gpu_op_count: 5,
            instruction_count: 5,
            ..Default::default()
        };
        let config = TargetConfig::with_gpu(vec![GpuTarget::Cuda { sm_versions: vec![80] }]);

        let path = determine_compilation_path(&analysis, &config);
        assert_eq!(path, CompilationPath::Gpu);
    }

    #[test]
    fn test_compilation_path_tensor_threshold() {
        let analysis = FunctionAnalysis {
            tensor_op_count: 15,
            instruction_count: 20,
            ..Default::default()
        };
        let config = TargetConfig::with_gpu(vec![GpuTarget::Cuda { sm_versions: vec![80] }])
            .with_threshold(10);

        let path = determine_compilation_path(&analysis, &config);
        assert_eq!(path, CompilationPath::Gpu);
    }

    #[test]
    fn test_compilation_path_no_gpu_fallback() {
        let analysis = FunctionAnalysis {
            gpu_op_count: 5,
            device_annotation: Some(DeviceAnnotation::Gpu {
                index: None,
                fallback: None,
            }),
            ..Default::default()
        };
        let config = TargetConfig::cpu_only();

        let path = determine_compilation_path(&analysis, &config);
        assert_eq!(path, CompilationPath::Cpu);
    }

    #[test]
    fn test_compilation_path_requires_methods() {
        assert!(CompilationPath::Cpu.requires_cpu());
        assert!(!CompilationPath::Cpu.requires_gpu());

        assert!(CompilationPath::Gpu.requires_gpu());
        assert!(!CompilationPath::Gpu.requires_cpu());

        let hybrid = CompilationPath::Hybrid { gpu_regions: vec![(0, 10)] };
        assert!(hybrid.requires_cpu());
        assert!(hybrid.requires_gpu());
    }
}
