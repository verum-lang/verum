//! MLIR lowering hints for VBC → MLIR optimization.
//!
//! Provides hints for the VBC → MLIR lowering pass to enable:
//! - Kernel fusion opportunities
//! - Target-specific optimizations (Tensor Cores, Matrix Cores)
//! - Region marking for MLIR vs interpreter execution

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::device::DeviceType;
use super::shape::InstructionId;

/// Region identifier for MLIR lowering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RegionId(pub u32);

/// Fusion group for kernel fusion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FusionGroup {
    /// Group identifier.
    pub id: u32,
    /// Instructions in this fusion group.
    pub instructions: Vec<InstructionId>,
    /// Fusion pattern name (e.g., "matmul_bias", "attention_fused").
    pub pattern: Option<String>,
    /// Estimated speedup from fusion (1.0 = no speedup).
    pub estimated_speedup: f32,
}

impl FusionGroup {
    /// Creates a new fusion group.
    pub fn new(id: u32, instructions: Vec<InstructionId>) -> Self {
        Self {
            id,
            instructions,
            pattern: None,
            estimated_speedup: 1.0,
        }
    }

    /// Sets the fusion pattern.
    pub fn with_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.pattern = Some(pattern.into());
        self
    }

    /// Sets the estimated speedup.
    pub fn with_speedup(mut self, speedup: f32) -> Self {
        self.estimated_speedup = speedup;
        self
    }
}

/// Target-specific optimization strategy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TargetOptStrategy {
    /// NVIDIA Tensor Cores (sm_70+).
    NvidiaTensorCore {
        /// WMMA shape (M, N, K).
        wmma_shape: (u32, u32, u32),
        /// Use TF32 for FP32 inputs.
        use_tf32: bool,
    },
    /// AMD Matrix Cores (gfx90a+).
    AmdMatrixCore {
        /// MFMA shape (M, N, K).
        mfma_shape: (u32, u32, u32),
    },
    /// Apple Metal with MPSGraph.
    MetalMPS {
        /// Use MPSGraph for automatic optimization.
        use_mps_graph: bool,
    },
    /// Generic SIMD vectorization.
    GenericSimd {
        /// Vector width in elements.
        vector_width: u32,
    },
    /// Use cuBLAS/rocBLAS for BLAS operations.
    VendorBlas {
        /// Library name.
        library: String,
    },
    /// Use cuDNN/MIOpen for convolutions.
    VendorDnn {
        /// Library name.
        library: String,
    },
}

impl TargetOptStrategy {
    /// Creates an NVIDIA Tensor Core strategy for FP16.
    pub fn nvidia_tensor_core_fp16() -> Self {
        Self::NvidiaTensorCore {
            wmma_shape: (16, 16, 16),
            use_tf32: false,
        }
    }

    /// Creates an NVIDIA Tensor Core strategy for TF32.
    pub fn nvidia_tensor_core_tf32() -> Self {
        Self::NvidiaTensorCore {
            wmma_shape: (16, 16, 8),
            use_tf32: true,
        }
    }

    /// Creates an AMD Matrix Core strategy.
    pub fn amd_matrix_core() -> Self {
        Self::AmdMatrixCore {
            mfma_shape: (32, 32, 8),
        }
    }

    /// Creates a Metal MPSGraph strategy.
    pub fn metal_mps() -> Self {
        Self::MetalMPS {
            use_mps_graph: true,
        }
    }

    /// Creates a generic SIMD strategy.
    pub fn generic(vector_width: u32) -> Self {
        Self::GenericSimd { vector_width }
    }
}

/// Target-specific optimization hints.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TargetOptHints {
    /// Strategy per device type.
    pub strategies: HashMap<DeviceType, TargetOptStrategy>,
    /// Prefer fused operations when possible.
    pub prefer_fusion: bool,
    /// Tile sizes for tiled operations.
    pub tile_sizes: Option<Vec<u32>>,
    /// Thread block sizes for GPU.
    pub block_sizes: Option<(u32, u32, u32)>,
    /// Use async copies (CUDA cp.async).
    pub use_async_copy: bool,
    /// Use shared memory for data reuse.
    pub use_shared_memory: bool,
}

impl TargetOptHints {
    /// Creates empty target optimization hints.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the strategy for a device type.
    pub fn set_strategy(&mut self, device: DeviceType, strategy: TargetOptStrategy) {
        self.strategies.insert(device, strategy);
    }

    /// Gets the strategy for a device type.
    pub fn get_strategy(&self, device: DeviceType) -> Option<&TargetOptStrategy> {
        self.strategies.get(&device)
    }

    /// Enables all GPU optimizations.
    pub fn with_gpu_optimizations(mut self) -> Self {
        self.prefer_fusion = true;
        self.use_async_copy = true;
        self.use_shared_memory = true;
        self
    }

    /// Sets tile sizes.
    pub fn with_tile_sizes(mut self, sizes: Vec<u32>) -> Self {
        self.tile_sizes = Some(sizes);
        self
    }

    /// Sets block sizes for GPU.
    pub fn with_block_sizes(mut self, x: u32, y: u32, z: u32) -> Self {
        self.block_sizes = Some((x, y, z));
        self
    }
}

/// MLIR lowering hints for a VBC module.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MlirHints {
    /// Regions to lower to MLIR (vs interpret).
    pub mlir_regions: Vec<RegionId>,
    /// Fusion groups (instructions that should be fused).
    pub fusion_groups: Vec<FusionGroup>,
    /// Target-specific optimizations.
    pub target_opts: TargetOptHints,
    /// Instructions to lower to MLIR.
    pub lowerable_instructions: Vec<InstructionId>,
    /// Instructions that must stay in interpreter.
    pub interpreter_only: Vec<InstructionId>,
    /// Memory layout preferences.
    pub memory_layout: MemoryLayoutHints,
}

/// Memory layout preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryLayoutHints {
    /// Prefer row-major (C) layout.
    pub prefer_row_major: bool,
    /// Alignment requirements in bytes.
    pub alignment: Option<usize>,
    /// Use contiguous memory for tensors.
    pub prefer_contiguous: bool,
}

impl MlirHints {
    /// Creates empty MLIR hints.
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks a region for MLIR lowering.
    pub fn add_mlir_region(&mut self, region: RegionId) {
        if !self.mlir_regions.contains(&region) {
            self.mlir_regions.push(region);
        }
    }

    /// Adds a fusion group.
    pub fn add_fusion_group(&mut self, group: FusionGroup) {
        self.fusion_groups.push(group);
    }

    /// Marks an instruction as lowerable to MLIR.
    pub fn mark_lowerable(&mut self, instr: InstructionId) {
        if !self.lowerable_instructions.contains(&instr) {
            self.lowerable_instructions.push(instr);
        }
    }

    /// Marks an instruction as interpreter-only.
    pub fn mark_interpreter_only(&mut self, instr: InstructionId) {
        if !self.interpreter_only.contains(&instr) {
            self.interpreter_only.push(instr);
        }
    }

    /// Returns true if an instruction should be lowered to MLIR.
    pub fn should_lower(&self, instr: InstructionId) -> bool {
        !self.interpreter_only.contains(&instr)
            && (self.lowerable_instructions.is_empty()
                || self.lowerable_instructions.contains(&instr))
    }

    /// Finds the fusion group containing an instruction.
    pub fn find_fusion_group(&self, instr: InstructionId) -> Option<&FusionGroup> {
        self.fusion_groups
            .iter()
            .find(|g| g.instructions.contains(&instr))
    }

    /// Returns true if this MLIR hints metadata is empty.
    pub fn is_empty(&self) -> bool {
        self.mlir_regions.is_empty()
            && self.fusion_groups.is_empty()
            && self.lowerable_instructions.is_empty()
            && self.interpreter_only.is_empty()
            && self.target_opts.strategies.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fusion_group() {
        let group = FusionGroup::new(
            0,
            vec![InstructionId(0), InstructionId(1), InstructionId(2)],
        )
        .with_pattern("matmul_bias_relu")
        .with_speedup(1.5);

        assert_eq!(group.pattern, Some("matmul_bias_relu".to_string()));
        assert_eq!(group.estimated_speedup, 1.5);
    }

    #[test]
    fn test_target_opt_strategy() {
        let strategy = TargetOptStrategy::nvidia_tensor_core_fp16();
        if let TargetOptStrategy::NvidiaTensorCore { wmma_shape, .. } = strategy {
            assert_eq!(wmma_shape, (16, 16, 16));
        } else {
            panic!("Wrong strategy type");
        }
    }

    #[test]
    fn test_mlir_hints() {
        let mut hints = MlirHints::new();
        hints.add_fusion_group(FusionGroup::new(
            0,
            vec![InstructionId(0), InstructionId(1)],
        ));
        hints.mark_interpreter_only(InstructionId(5));

        assert!(hints.should_lower(InstructionId(0)));
        assert!(hints.should_lower(InstructionId(1)));
        assert!(!hints.should_lower(InstructionId(5)));

        let group = hints.find_fusion_group(InstructionId(0));
        assert!(group.is_some());
    }
}
