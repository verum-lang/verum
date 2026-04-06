//! VBC → MLIR Lowering for GPU Path
//!
//! Converts VBC (Verum Bytecode) instructions to MLIR operations with proper
//! operand wiring. The GPU path handles tensor operations, GPU kernel launches,
//! and scalar pass-through for hybrid CPU+GPU functions.
//!
//! # Architecture
//!
//! ```text
//! VBC Module (GPU functions)
//!     │
//!     ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                    VBC → MLIR LOWERING (this module)                │
//! │                                                                     │
//! │  VBC tensors → verum.tensor / linalg / arith dialect               │
//! │  VBC GPU ops → gpu dialect                                         │
//! │  VBC scalar  → arith dialect (hybrid CPU+GPU pass-through)         │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;

use verum_mlir::{
    Context, Error as MlirError,
    ir::{
        Block, Location, Module, Region, Type, Value,
        attribute::{
            DenseI64ArrayAttribute, FloatAttribute, IntegerAttribute,
            StringAttribute, TypeAttribute,
        },
        operation::{Operation, OperationBuilder, OperationLike},
        r#type::{FunctionType, IntegerType, MemRefType, RankedTensorType},
        BlockLike, RegionLike,
    },
    dialect::{arith, func},
};

use verum_vbc::{
    instruction::{
        BinaryFloatOp, BinaryIntOp, Instruction, Reg,
        TensorBinaryOp, TensorDType, TensorReduceOp, TensorUnaryOp,
    },
    module::VbcModule,
};

/// Sentinel for dynamic dimensions in `RankedTensorType` (`u64::MAX` = -1 in two's complement).
const DYNAMIC: u64 = u64::MAX;

/// Result type for VBC → MLIR lowering operations.
pub type Result<T> = std::result::Result<T, VbcMlirError>;

// =============================================================================
// Error Types
// =============================================================================

/// Errors that can occur during VBC → MLIR lowering.
#[derive(Debug, Clone)]
pub enum VbcMlirError {
    /// Unsupported VBC instruction for GPU lowering.
    UnsupportedInstruction { opcode: u8, name: String },
    /// Type conversion failed.
    TypeConversionFailed { vbc_type: String, reason: String },
    /// MLIR operation build failed.
    MlirBuildFailed { op: String, reason: String },
    /// Function not found.
    FunctionNotFound { id: u32 },
    /// Register not found in value map.
    RegisterNotFound { reg: u16 },
    /// GPU target not available.
    GpuTargetNotAvailable { target: String },
    /// Invalid tensor shape.
    InvalidTensorShape { reason: String },
    /// Internal MLIR error.
    InternalMlirError(String),
}

impl std::fmt::Display for VbcMlirError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedInstruction { opcode, name } =>
                write!(f, "Unsupported GPU instruction: {} (0x{:02X})", name, opcode),
            Self::TypeConversionFailed { vbc_type, reason } =>
                write!(f, "Type conversion failed for {}: {}", vbc_type, reason),
            Self::MlirBuildFailed { op, reason } =>
                write!(f, "MLIR operation {} build failed: {}", op, reason),
            Self::FunctionNotFound { id } =>
                write!(f, "Function not found: {}", id),
            Self::RegisterNotFound { reg } =>
                write!(f, "Register r{} not found in value map", reg),
            Self::GpuTargetNotAvailable { target } =>
                write!(f, "GPU target not available: {}", target),
            Self::InvalidTensorShape { reason } =>
                write!(f, "Invalid tensor shape: {}", reason),
            Self::InternalMlirError(msg) =>
                write!(f, "Internal MLIR error: {}", msg),
        }
    }
}

impl std::error::Error for VbcMlirError {}

impl From<MlirError> for VbcMlirError {
    fn from(e: MlirError) -> Self {
        VbcMlirError::InternalMlirError(format!("{:?}", e))
    }
}

// =============================================================================
// GPU Target
// =============================================================================

/// GPU target for code generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuTarget {
    /// NVIDIA CUDA (PTX)
    Cuda,
    /// AMD ROCm (HSACO)
    Rocm,
    /// Vulkan (SPIR-V)
    Vulkan,
    /// Apple Metal
    Metal,
}

impl GpuTarget {
    /// Returns the MLIR target triple.
    pub fn target_triple(&self) -> &'static str {
        match self {
            GpuTarget::Cuda => "nvptx64-nvidia-cuda",
            GpuTarget::Rocm => "amdgcn-amd-amdhsa",
            GpuTarget::Vulkan => "spirv64-unknown-vulkan",
            GpuTarget::Metal => "air64-apple-macos",
        }
    }

    /// Returns the primary dialect name.
    pub fn dialect(&self) -> &'static str {
        match self {
            GpuTarget::Cuda => "nvvm",
            GpuTarget::Rocm => "rocdl",
            GpuTarget::Vulkan => "spirv",
            GpuTarget::Metal => "metal",
        }
    }

    /// Returns the GPU memory space identifier for memrefs.
    pub fn memory_space(&self) -> u64 {
        match self {
            GpuTarget::Cuda => 1,
            GpuTarget::Rocm => 1,
            GpuTarget::Vulkan => 0,
            GpuTarget::Metal => 0,
        }
    }
}

// =============================================================================
// Configuration & Statistics
// =============================================================================

/// Configuration for VBC → MLIR lowering.
#[derive(Debug, Clone)]
pub struct GpuLoweringConfig {
    /// Target GPU platform.
    pub target: GpuTarget,
    /// Optimization level (0-3).
    pub opt_level: u8,
    /// Enable tensor core operations (NVIDIA).
    pub enable_tensor_cores: bool,
    /// Maximum shared memory per block (bytes).
    pub max_shared_memory: usize,
    /// Default block size for kernel launches.
    pub default_block_size: [u32; 3],
    /// Enable async memory operations.
    pub enable_async_copy: bool,
    /// Enable debug info in generated code.
    pub debug_info: bool,
}

impl Default for GpuLoweringConfig {
    fn default() -> Self {
        Self {
            target: GpuTarget::Cuda,
            opt_level: 2,
            enable_tensor_cores: true,
            max_shared_memory: 48 * 1024,
            default_block_size: [256, 1, 1],
            enable_async_copy: true,
            debug_info: false,
        }
    }
}

impl GpuLoweringConfig {
    pub fn cuda() -> Self { Self::default() }
    pub fn rocm() -> Self {
        Self { target: GpuTarget::Rocm, max_shared_memory: 64 * 1024, ..Default::default() }
    }
    pub fn vulkan() -> Self {
        Self { target: GpuTarget::Vulkan, enable_tensor_cores: false, max_shared_memory: 32 * 1024, ..Default::default() }
    }
}

/// Statistics collected during GPU lowering.
#[derive(Debug, Clone, Default)]
pub struct GpuLoweringStats {
    pub instructions_processed: usize,
    pub mlir_ops_generated: usize,
    pub tensor_ops: usize,
    pub kernel_launches: usize,
    pub memory_transfers: usize,
    pub functions_lowered: usize,
    pub shared_memory_allocs: usize,
    pub tensor_core_ops: usize,
}

impl GpuLoweringStats {
    pub fn tensor_op_ratio(&self) -> f64 {
        if self.instructions_processed == 0 { 0.0 }
        else { self.tensor_ops as f64 / self.instructions_processed as f64 }
    }
}

// =============================================================================
// Main Lowering
// =============================================================================

/// Main VBC → MLIR lowering for the GPU path.
pub struct VbcToMlirGpuLowering<'ctx> {
    context: &'ctx Context,
    config: GpuLoweringConfig,
    stats: GpuLoweringStats,
    value_map: HashMap<u16, Value<'ctx, 'ctx>>,
}

impl<'ctx> VbcToMlirGpuLowering<'ctx> {
    pub fn new(context: &'ctx Context, config: GpuLoweringConfig) -> Self {
        Self { context, config, stats: GpuLoweringStats::default(), value_map: HashMap::new() }
    }

    pub fn config(&self) -> &GpuLoweringConfig { &self.config }
    pub fn stats(&self) -> &GpuLoweringStats { &self.stats }

    // =========================================================================
    // Type helpers
    // =========================================================================

    fn i64_type(&self) -> Type<'ctx> { IntegerType::new(self.context, 64).into() }
    fn f64_type(&self) -> Type<'ctx> { Type::float64(self.context) }
    fn index_type(&self) -> Type<'ctx> { Type::index(self.context) }

    fn dynamic_2d_tensor(&self, elem: Type<'ctx>) -> Type<'ctx> {
        RankedTensorType::new(&[DYNAMIC, DYNAMIC], elem, None).into()
    }

    fn dynamic_nd_tensor(&self, ndims: usize, elem: Type<'ctx>) -> Type<'ctx> {
        let shape = vec![DYNAMIC; ndims];
        RankedTensorType::new(&shape, elem, None).into()
    }

    fn gpu_memref_2d(&self, elem: Type<'ctx>) -> Type<'ctx> {
        let ms = self.config.target.memory_space();
        let ms_attr = IntegerAttribute::new(self.i64_type(), ms as i64).into();
        MemRefType::new(elem, &[-1, -1], None, Some(ms_attr)).into()
    }

    fn tensor_dtype_to_mlir(&self, dtype: TensorDType) -> Type<'ctx> {
        match dtype {
            TensorDType::F64 => Type::float64(self.context),
            TensorDType::F32 => Type::float32(self.context),
            TensorDType::F16 => Type::float16(self.context),
            TensorDType::BF16 => Type::bfloat16(self.context),
            TensorDType::I64 | TensorDType::U64 => IntegerType::new(self.context, 64).into(),
            TensorDType::I32 | TensorDType::U32 => IntegerType::new(self.context, 32).into(),
            TensorDType::I16 | TensorDType::U16 => IntegerType::new(self.context, 16).into(),
            TensorDType::I8  | TensorDType::U8  => IntegerType::new(self.context, 8).into(),
            TensorDType::Bool => IntegerType::new(self.context, 1).into(),
            TensorDType::Complex64  => Type::float32(self.context),
            TensorDType::Complex128 => Type::float64(self.context),
        }
    }

    // =========================================================================
    // Value map helpers
    // =========================================================================

    fn get_value(&self, block: &Block<'ctx>, reg: Reg, location: Location<'ctx>) -> Value<'ctx, 'ctx> {
        if let Some(v) = self.value_map.get(&reg.0) {
            *v
        } else {
            let op = block.append_operation(arith::constant(
                self.context, IntegerAttribute::new(self.i64_type(), 0).into(), location,
            ));
            op.result(0).unwrap().into()
        }
    }

    fn get_index_value(&self, block: &Block<'ctx>, reg: Reg, location: Location<'ctx>) -> Value<'ctx, 'ctx> {
        let val = self.get_value(block, reg, location);
        let cast_op = block.append_operation(
            OperationBuilder::new("arith.index_cast", location)
                .add_operands(&[val])
                .add_results(&[self.index_type()])
                .build()
                .expect("arith.index_cast build"),
        );
        cast_op.result(0).unwrap().into()
    }

    fn set_value(&mut self, dst: Reg, val: Value<'ctx, 'ctx>) {
        self.value_map.insert(dst.0, val);
    }

    // =========================================================================
    // Operation builder helpers
    // =========================================================================

    fn build_and_store(
        &mut self, block: &Block<'ctx>, dst: Reg, op_name: &str,
        operands: &[Value<'ctx, 'ctx>], result_types: &[Type<'ctx>], location: Location<'ctx>,
    ) -> Result<()> {
        let op = OperationBuilder::new(op_name, location)
            .add_operands(operands).add_results(result_types).build()
            .map_err(|e| VbcMlirError::MlirBuildFailed { op: op_name.into(), reason: format!("{:?}", e) })?;
        let result = op.result(0).map_err(|_| VbcMlirError::MlirBuildFailed {
            op: op_name.into(), reason: "no result".into(),
        })?;
        block.append_operation(op);
        self.set_value(dst, result.into());
        Ok(())
    }

    fn build_void(
        &self, block: &Block<'ctx>, op_name: &str,
        operands: &[Value<'ctx, 'ctx>], location: Location<'ctx>,
    ) -> Result<()> {
        let op = OperationBuilder::new(op_name, location)
            .add_operands(operands).build()
            .map_err(|e| VbcMlirError::MlirBuildFailed { op: op_name.into(), reason: format!("{:?}", e) })?;
        block.append_operation(op);
        Ok(())
    }

    // =========================================================================
    // Module & Function Lowering
    // =========================================================================

    pub fn lower_module(&mut self, vbc_module: &VbcModule) -> Result<Module<'ctx>> {
        let location = Location::unknown(self.context);
        let module = Module::new(location);
        for (func_id, func_desc) in vbc_module.functions.iter().enumerate() {
            let func_op = self.lower_function(func_id as u32, func_desc, vbc_module)?;
            module.body().append_operation(func_op);
            self.stats.functions_lowered += 1;
        }
        Ok(module)
    }

    fn lower_function(
        &mut self, func_id: u32,
        func_desc: &verum_vbc::module::FunctionDescriptor, module: &VbcModule,
    ) -> Result<Operation<'ctx>> {
        let location = Location::unknown(self.context);
        let name = module.strings.get(func_desc.name).unwrap_or("unknown");
        let bytecode_offset = func_desc.bytecode_offset as usize;
        let bytecode_len = func_desc.bytecode_length as usize;
        if bytecode_offset + bytecode_len > module.bytecode.len() {
            return Err(VbcMlirError::FunctionNotFound { id: func_id });
        }
        let bytecode = &module.bytecode[bytecode_offset..bytecode_offset + bytecode_len];
        self.value_map.clear();

        let i64_type = self.i64_type();
        let func_type = FunctionType::new(self.context, &[], &[i64_type]);
        let entry_block = Block::new(&[]);

        let mut offset = 0;
        while offset < bytecode.len() {
            let start_offset = offset;
            let instr = verum_vbc::bytecode::decode_instruction(bytecode, &mut offset)
                .map_err(|e| VbcMlirError::UnsupportedInstruction {
                    opcode: bytecode.get(start_offset).copied().unwrap_or(0),
                    name: format!("decode error: {:?}", e),
                })?;
            self.lower_instruction(&entry_block, &instr, location)?;
            self.stats.instructions_processed += 1;
        }

        if entry_block.terminator().is_none() {
            let zero = entry_block.append_operation(arith::constant(
                self.context, IntegerAttribute::new(i64_type, 0).into(), location,
            ));
            entry_block.append_operation(func::r#return(&[zero.result(0).unwrap().into()], location));
        }

        let region = Region::new();
        region.append_block(entry_block);
        let func_op = func::func(
            self.context, StringAttribute::new(self.context, name),
            TypeAttribute::new(func_type.into()), region, &[], location,
        );
        Ok(func_op)
    }

    // =========================================================================
    // Instruction Dispatch
    // =========================================================================

    fn lower_instruction(&mut self, block: &Block<'ctx>, instr: &Instruction, location: Location<'ctx>) -> Result<()> {
        match instr {
            // Tensor Creation
            Instruction::TensorNew { dst, dtype, dims } => {
                let elem_type = self.tensor_dtype_to_mlir(*dtype);
                let result_type = self.dynamic_nd_tensor(dims.len(), elem_type);
                let dim_vals: Vec<Value> = dims.iter().map(|r| self.get_index_value(block, *r, location)).collect();
                self.build_and_store(block, *dst, "tensor.empty", &dim_vals, &[result_type], location)?;
                self.stats.tensor_ops += 1;
            }

            // Tensor Element-wise
            Instruction::TensorBinop { op, dst, a, b } => {
                let lhs = self.get_value(block, *a, location);
                let rhs = self.get_value(block, *b, location);
                let op_name = match op {
                    TensorBinaryOp::Add => "arith.addf", TensorBinaryOp::Sub => "arith.subf",
                    TensorBinaryOp::Mul => "arith.mulf", TensorBinaryOp::Div => "arith.divf",
                    TensorBinaryOp::Pow => "math.powf",  TensorBinaryOp::Mod => "arith.remf",
                    TensorBinaryOp::Min => "arith.minimumf", TensorBinaryOp::Max => "arith.maximumf",
                };
                self.build_and_store(block, *dst, op_name, &[lhs, rhs], &[self.f64_type()], location)?;
                self.stats.tensor_ops += 1;
            }

            Instruction::TensorUnop { op, dst, src } => {
                let operand = self.get_value(block, *src, location);
                let op_name = match op {
                    TensorUnaryOp::Neg => "arith.negf", TensorUnaryOp::Abs => "math.absf",
                    TensorUnaryOp::Sqrt => "math.sqrt", TensorUnaryOp::Exp => "math.exp",
                    TensorUnaryOp::Log => "math.log",   TensorUnaryOp::Sin => "math.sin",
                    TensorUnaryOp::Cos => "math.cos",   TensorUnaryOp::Tan => "math.tan",
                    TensorUnaryOp::Tanh => "math.tanh", TensorUnaryOp::Floor => "math.floor",
                    TensorUnaryOp::Ceil => "math.ceil",  TensorUnaryOp::Round => "math.roundeven",
                    TensorUnaryOp::Rsqrt => "math.rsqrt", TensorUnaryOp::Erf => "math.erf",
                    TensorUnaryOp::Log2 => "math.log2",  TensorUnaryOp::Sign => "math.copysign",
                    TensorUnaryOp::Sigmoid => "verum.tensor.sigmoid",
                    TensorUnaryOp::Relu => "verum.tensor.relu",
                    TensorUnaryOp::Gelu => "verum.tensor.gelu",
                    TensorUnaryOp::Silu => "verum.tensor.silu",
                    TensorUnaryOp::Softplus => "verum.tensor.softplus",
                    TensorUnaryOp::Mish => "verum.tensor.mish",
                };
                self.build_and_store(block, *dst, op_name, &[operand], &[self.f64_type()], location)?;
                self.stats.tensor_ops += 1;
            }

            // Tensor Linear Algebra
            Instruction::TensorMatmul { dst, a, b } => {
                let lhs = self.get_value(block, *a, location);
                let rhs = self.get_value(block, *b, location);
                let elem = self.f64_type();
                let out_type = self.dynamic_2d_tensor(elem);
                // tensor.empty → linalg.fill → linalg.matmul
                let empty_op = block.append_operation(
                    OperationBuilder::new("tensor.empty", location).add_results(&[out_type]).build()
                        .map_err(|e| VbcMlirError::MlirBuildFailed { op: "tensor.empty".into(), reason: format!("{:?}", e) })?,
                );
                let empty_tensor: Value = empty_op.result(0).unwrap().into();
                let zero = block.append_operation(arith::constant(
                    self.context, FloatAttribute::new(self.context, elem, 0.0).into(), location,
                ));
                let zero_val: Value = zero.result(0).unwrap().into();
                let fill_op = block.append_operation(
                    OperationBuilder::new("linalg.fill", location)
                        .add_operands(&[zero_val, empty_tensor]).add_results(&[out_type]).build()
                        .map_err(|e| VbcMlirError::MlirBuildFailed { op: "linalg.fill".into(), reason: format!("{:?}", e) })?,
                );
                let filled: Value = fill_op.result(0).unwrap().into();
                self.build_and_store(block, *dst, "linalg.matmul", &[lhs, rhs, filled], &[out_type], location)?;
                self.stats.tensor_ops += 1;
                if self.config.enable_tensor_cores { self.stats.tensor_core_ops += 1; }
            }

            // Tensor Reduction
            Instruction::TensorReduce { op, dst, src, axes, keepdim } => {
                let input = self.get_value(block, *src, location);
                let op_name = match op {
                    TensorReduceOp::Sum | TensorReduceOp::Prod | TensorReduceOp::Max | TensorReduceOp::Min => "linalg.reduce",
                    TensorReduceOp::Mean => "verum.tensor.reduce_mean",
                    TensorReduceOp::Var => "verum.tensor.reduce_var",
                    TensorReduceOp::Std => "verum.tensor.reduce_std",
                    TensorReduceOp::Norm => "verum.tensor.reduce_norm",
                    TensorReduceOp::LogSumExp => "verum.tensor.reduce_logsumexp",
                    TensorReduceOp::All => "verum.tensor.reduce_all",
                    TensorReduceOp::Any => "verum.tensor.reduce_any",
                };
                let axes_i64: Vec<i64> = axes.iter().map(|a| *a as i64).collect();
                let mlir_op = OperationBuilder::new(op_name, location)
                    .add_operands(&[input]).add_results(&[self.f64_type()])
                    .add_attributes(&[(
                        verum_mlir::ir::Identifier::new(self.context, "dimensions"),
                        DenseI64ArrayAttribute::new(self.context, &axes_i64).into(),
                    )]).build()
                    .map_err(|e| VbcMlirError::MlirBuildFailed { op: op_name.into(), reason: format!("{:?}", e) })?;
                let result = mlir_op.result(0).map_err(|_| VbcMlirError::MlirBuildFailed {
                    op: op_name.into(), reason: "no result".into(),
                })?;
                block.append_operation(mlir_op);
                self.set_value(*dst, result.into());
                self.stats.tensor_ops += 1;
            }

            // GPU Operations
            Instruction::GpuLaunch { kernel_id, grid, block: blk, shared_mem, stream, args } => {
                let mut operands = Vec::with_capacity(6 + args.len());
                for r in grid { operands.push(self.get_index_value(block, *r, location)); }
                for r in blk  { operands.push(self.get_index_value(block, *r, location)); }
                for r in args { operands.push(self.get_value(block, *r, location)); }
                let kernel_name = format!("kernel_{}", kernel_id);
                let mlir_op = OperationBuilder::new("gpu.launch_func", location)
                    .add_operands(&operands)
                    .add_attributes(&[(
                        verum_mlir::ir::Identifier::new(self.context, "kernel"),
                        StringAttribute::new(self.context, &kernel_name).into(),
                    )]).build()
                    .map_err(|e| VbcMlirError::MlirBuildFailed { op: "gpu.launch_func".into(), reason: format!("{:?}", e) })?;
                block.append_operation(mlir_op);
                self.stats.kernel_launches += 1;
            }

            Instruction::GpuSync { stream } => {
                let s = self.get_value(block, *stream, location);
                self.build_void(block, "gpu.wait", &[s], location)?;
            }

            Instruction::GpuMemcpy { dst, src, direction } => {
                let d = self.get_value(block, *dst, location);
                let s = self.get_value(block, *src, location);
                self.build_void(block, "gpu.memcpy", &[d, s], location)?;
                self.stats.memory_transfers += 1;
            }

            Instruction::GpuAlloc { dst, size, device } => {
                let size_val = self.get_index_value(block, *size, location);
                let memref_type = self.gpu_memref_2d(self.f64_type());
                self.build_and_store(block, *dst, "gpu.alloc", &[size_val], &[memref_type], location)?;
            }

            // GPU Streams/Events — no-ops or simple wiring
            Instruction::GpuStreamCreate { dst } =>
                { self.build_and_store(block, *dst, "gpu.create_stream", &[], &[self.i64_type()], location)?; }
            Instruction::GpuStreamDestroy { .. } | Instruction::GpuEventDestroy { .. }
            | Instruction::GpuEventRecord { .. } | Instruction::GpuPrefetch { .. } => {}
            Instruction::GpuStreamWaitEvent { stream, event } => {
                let s = self.get_value(block, *stream, location);
                let e = self.get_value(block, *event, location);
                self.build_void(block, "gpu.wait", &[s, e], location)?;
            }
            Instruction::GpuEventCreate { dst } =>
                { self.build_and_store(block, *dst, "gpu.create_event", &[], &[self.i64_type()], location)?; }
            Instruction::GpuEventSynchronize { event } => {
                let e = self.get_value(block, *event, location);
                self.build_void(block, "gpu.wait", &[e], location)?;
            }
            Instruction::GpuMemcpyAsync { dst, src, .. } => {
                let d = self.get_value(block, *dst, location);
                let s = self.get_value(block, *src, location);
                self.build_void(block, "gpu.memcpy", &[d, s], location)?;
                self.stats.memory_transfers += 1;
            }
            Instruction::GpuFree { ptr } => {
                let p = self.get_value(block, *ptr, location);
                self.build_void(block, "gpu.dealloc", &[p], location)?;
            }

            // Flash Attention
            Instruction::TensorFlashAttention { dst, q, k, v, mask, scale, causal } => {
                let mut operands = vec![
                    self.get_value(block, *q, location),
                    self.get_value(block, *k, location),
                    self.get_value(block, *v, location),
                    self.get_value(block, *scale, location),
                ];
                if let Some(m) = mask { operands.push(self.get_value(block, *m, location)); }
                let result_type = self.dynamic_2d_tensor(self.f64_type());
                self.build_and_store(block, *dst, "verum.tensor.flash_attention", &operands, &[result_type], location)?;
                self.stats.tensor_ops += 1;
                if self.config.enable_tensor_cores { self.stats.tensor_core_ops += 1; }
            }

            // Scalar pass-through (hybrid CPU+GPU)
            Instruction::LoadSmallI { dst, value } => {
                let op = block.append_operation(arith::constant(
                    self.context, IntegerAttribute::new(self.i64_type(), *value as i64).into(), location,
                ));
                self.set_value(*dst, op.result(0).unwrap().into());
            }
            Instruction::LoadI { dst, value } => {
                let op = block.append_operation(arith::constant(
                    self.context, IntegerAttribute::new(self.i64_type(), *value).into(), location,
                ));
                self.set_value(*dst, op.result(0).unwrap().into());
            }
            Instruction::LoadF { dst, value } => {
                let op = block.append_operation(arith::constant(
                    self.context, FloatAttribute::new(self.context, self.f64_type(), *value).into(), location,
                ));
                self.set_value(*dst, op.result(0).unwrap().into());
            }
            Instruction::Mov { dst, src } => {
                let val = self.get_value(block, *src, location);
                self.set_value(*dst, val);
            }
            Instruction::BinaryI { op, dst, a, b } => {
                let lhs = self.get_value(block, *a, location);
                let rhs = self.get_value(block, *b, location);
                let op_name = match op {
                    BinaryIntOp::Add => "arith.addi", BinaryIntOp::Sub => "arith.subi",
                    BinaryIntOp::Mul => "arith.muli", BinaryIntOp::Div => "arith.divsi",
                    BinaryIntOp::Mod => "arith.remsi", BinaryIntOp::Pow => "math.ipowi",
                };
                self.build_and_store(block, *dst, op_name, &[lhs, rhs], &[self.i64_type()], location)?;
            }
            Instruction::BinaryF { op, dst, a, b } => {
                let lhs = self.get_value(block, *a, location);
                let rhs = self.get_value(block, *b, location);
                let op_name = match op {
                    BinaryFloatOp::Add => "arith.addf", BinaryFloatOp::Sub => "arith.subf",
                    BinaryFloatOp::Mul => "arith.mulf", BinaryFloatOp::Div => "arith.divf",
                    BinaryFloatOp::Pow => "math.powf",  BinaryFloatOp::Mod => "arith.remf",
                };
                self.build_and_store(block, *dst, op_name, &[lhs, rhs], &[self.f64_type()], location)?;
            }
            Instruction::Ret { value } => {
                let val = self.get_value(block, *value, location);
                block.append_operation(func::r#return(&[val], location));
            }
            Instruction::RetV => {
                block.append_operation(func::r#return(&[], location));
            }

            // Skip all other instructions (CPU-only path)
            _ => {}
        }
        self.stats.mlir_ops_generated += 1;
        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_lowering_config_default() {
        let config = GpuLoweringConfig::default();
        assert_eq!(config.target, GpuTarget::Cuda);
        assert!(config.enable_tensor_cores);
    }

    #[test]
    fn test_gpu_target_dialect() {
        assert_eq!(GpuTarget::Cuda.dialect(), "nvvm");
        assert_eq!(GpuTarget::Rocm.dialect(), "rocdl");
        assert_eq!(GpuTarget::Vulkan.dialect(), "spirv");
    }

    #[test]
    fn test_gpu_target_memory_space() {
        assert_eq!(GpuTarget::Cuda.memory_space(), 1);
        assert_eq!(GpuTarget::Vulkan.memory_space(), 0);
    }

    #[test]
    fn test_gpu_lowering_stats() {
        let mut stats = GpuLoweringStats::default();
        stats.instructions_processed = 100;
        stats.tensor_ops = 60;
        assert!((stats.tensor_op_ratio() - 0.6).abs() < 0.001);
    }
}
