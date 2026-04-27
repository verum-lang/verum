//! SIMD (Single Instruction, Multiple Data) code generation.
//!
//! This module provides LLVM IR generation for SIMD vector operations,
//! implementing platform-specific lowering for various CPU architectures.
//!
//! # Overview
//!
//! SIMD operations in Verum use portable vector types (`Vec<T, N>`, `Mask<N>`)
//! that compile to optimal instructions on each target platform:
//!
//! - **x86_64**: SSE4.2, AVX, AVX2, AVX-512
//! - **aarch64**: NEON, SVE
//! - **RISC-V**: V extension
//!
//! # Generated Code Patterns
//!
//! ```llvm
//! ; Vector addition (4xf32)
//! %sum = fadd <4 x float> %a, %b
//!
//! ; Fused multiply-add (with intrinsic)
//! %fma = call <4 x float> @llvm.fma.v4f32(<4 x float> %a, <4 x float> %b, <4 x float> %c)
//!
//! ; Horizontal sum reduction
//! %sum = call float @llvm.vector.reduce.fadd.v4f32(float 0.0, <4 x float> %v)
//! ```
//!
//! # SIMD Architecture
//!
//! Verum provides portable SIMD via `Vec<T: SimdElement, N>` types with `@repr(simd)`.
//! Key features:
//! - Portable vector types compile to optimal instructions per platform:
//!   x86_64 (SSE4.2/AVX/AVX2/AVX-512), aarch64 (NEON/SVE), RISC-V (V extension)
//! - Operations: splat, load (aligned/unaligned), arithmetic (+, *, fma),
//!   horizontal reductions, shuffle/permute, gather/scatter, masked load/store
//! - `Mask<N>` type for conditional SIMD operations (lane-wise comparisons)
//! - `@multiversion` attribute generates multiple implementations for runtime dispatch
//! - `@target_feature(enable = "avx2")` for platform-specific intrinsic access
//! - VBC opcodes 0xC0-0xCF handle SIMD at bytecode level
//! - LLVM lowering maps to `<N x T>` vector types and vector intrinsics

use verum_llvm::values::{BasicValue, BasicValueEnum, IntValue, PointerValue, VectorValue};
use verum_llvm::types::{BasicTypeEnum, VectorType};
use verum_llvm::builder::Builder;
use verum_llvm::context::Context;
use verum_llvm::IntPredicate;

use super::error::{LlvmLoweringError, Result};

// =============================================================================
// SIMD TARGET CONFIGURATION
// =============================================================================

/// Target architecture for SIMD code generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SimdTargetArch {
    /// x86_64 with SSE4.2 baseline
    #[default]
    X86_64,
    /// 32-bit x86
    X86,
    /// ARM 64-bit (AArch64)
    AArch64,
    /// ARM 32-bit
    Arm,
    /// RISC-V 64-bit
    RiscV64,
    /// RISC-V 32-bit
    RiscV32,
    /// WebAssembly 32-bit
    Wasm32,
}

/// Available SIMD feature levels for a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SimdFeatureLevel {
    /// No SIMD support (scalar fallback)
    None,
    /// 128-bit vectors (SSE4.2, NEON baseline)
    V128,
    /// 256-bit vectors (AVX2)
    V256,
    /// 512-bit vectors (AVX-512, SVE)
    V512,
}

impl SimdFeatureLevel {
    /// Get the maximum vector width in bits.
    pub fn max_bits(self) -> usize {
        match self {
            SimdFeatureLevel::None => 0,
            SimdFeatureLevel::V128 => 128,
            SimdFeatureLevel::V256 => 256,
            SimdFeatureLevel::V512 => 512,
        }
    }

    /// Get the maximum vector width in bytes.
    pub fn max_bytes(self) -> usize {
        self.max_bits() / 8
    }
}

// =============================================================================
// SIMD ELEMENT TYPES
// =============================================================================

/// Element types supported in SIMD vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdElementKind {
    /// 8-bit signed integer
    I8,
    /// 16-bit signed integer
    I16,
    /// 32-bit signed integer
    I32,
    /// 64-bit signed integer
    I64,
    /// 8-bit unsigned integer
    U8,
    /// 16-bit unsigned integer
    U16,
    /// 32-bit unsigned integer
    U32,
    /// 64-bit unsigned integer
    U64,
    /// 32-bit floating point
    F32,
    /// 64-bit floating point
    F64,
}

impl SimdElementKind {
    /// Get the size in bits.
    pub fn bit_width(self) -> usize {
        match self {
            SimdElementKind::I8 | SimdElementKind::U8 => 8,
            SimdElementKind::I16 | SimdElementKind::U16 => 16,
            SimdElementKind::I32 | SimdElementKind::U32 | SimdElementKind::F32 => 32,
            SimdElementKind::I64 | SimdElementKind::U64 | SimdElementKind::F64 => 64,
        }
    }

    /// Check if this is a floating point type.
    pub fn is_float(self) -> bool {
        matches!(self, SimdElementKind::F32 | SimdElementKind::F64)
    }

    /// Check if this is a signed integer.
    pub fn is_signed(self) -> bool {
        matches!(
            self,
            SimdElementKind::I8
                | SimdElementKind::I16
                | SimdElementKind::I32
                | SimdElementKind::I64
        )
    }
}

// =============================================================================
// SIMD OPERATIONS
// =============================================================================

/// SIMD binary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdBinaryOp {
    /// Lane-wise addition
    Add,
    /// Lane-wise subtraction
    Sub,
    /// Lane-wise multiplication
    Mul,
    /// Lane-wise division
    Div,
    /// Lane-wise minimum
    Min,
    /// Lane-wise maximum
    Max,
    /// Bitwise AND
    And,
    /// Bitwise OR
    Or,
    /// Bitwise XOR
    Xor,
    /// Saturating addition (integers only)
    SaturatingAdd,
    /// Saturating subtraction (integers only)
    SaturatingSub,
}

/// SIMD unary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdUnaryOp {
    /// Absolute value
    Abs,
    /// Negation
    Neg,
    /// Square root (floats only)
    Sqrt,
    /// Reciprocal (1/x, floats only)
    Reciprocal,
    /// Reciprocal square root (1/sqrt(x), floats only)
    Rsqrt,
    /// Ceiling (floats only)
    Ceil,
    /// Floor (floats only)
    Floor,
    /// Round to nearest (floats only)
    Round,
    /// Truncate toward zero (floats only)
    Trunc,
    /// Bitwise NOT
    Not,
    /// Leading zeros count
    Clz,
    /// Trailing zeros count
    Ctz,
    /// Population count
    Popcount,
}

/// SIMD comparison operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdCompareOp {
    /// Equal
    Eq,
    /// Not equal
    Ne,
    /// Less than
    Lt,
    /// Less than or equal
    Le,
    /// Greater than
    Gt,
    /// Greater than or equal
    Ge,
}

/// SIMD reduction operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdReduceOp {
    /// Sum all lanes
    Add,
    /// Multiply all lanes
    Mul,
    /// Minimum across lanes
    Min,
    /// Maximum across lanes
    Max,
    /// Bitwise AND across lanes
    And,
    /// Bitwise OR across lanes
    Or,
    /// Bitwise XOR across lanes
    Xor,
}

// =============================================================================
// STATISTICS
// =============================================================================

/// Statistics for SIMD code generation.
#[derive(Debug, Clone, Default)]
pub struct SimdStats {
    /// Number of vector operations lowered
    pub ops_lowered: usize,
    /// Number of splat operations
    pub splats: usize,
    /// Number of binary operations
    pub binary_ops: usize,
    /// Number of unary operations
    pub unary_ops: usize,
    /// Number of reduction operations
    pub reductions: usize,
    /// Number of shuffle operations
    pub shuffles: usize,
    /// Number of gather/scatter operations
    pub gathers: usize,
    /// Number of masked operations
    pub masked_ops: usize,
    /// Number of FMA operations
    pub fma_ops: usize,
    /// Number of scalar fallbacks (no SIMD available)
    pub scalar_fallbacks: usize,
}

// =============================================================================
// SIMD LOWERING
// =============================================================================

/// SIMD code generation context.
///
/// Provides LLVM IR generation for SIMD vector operations with
/// platform-specific intrinsic selection.
pub struct SimdLowering<'ctx> {
    /// LLVM context
    context: &'ctx Context,
    /// LLVM IR builder
    builder: &'ctx Builder<'ctx>,
    /// Target architecture
    target_arch: SimdTargetArch,
    /// Available feature level
    feature_level: SimdFeatureLevel,
    /// Statistics
    stats: SimdStats,
}

impl<'ctx> SimdLowering<'ctx> {
    /// Create a new SIMD lowering context.
    pub fn new(
        context: &'ctx Context,
        builder: &'ctx Builder<'ctx>,
        target_arch: SimdTargetArch,
        feature_level: SimdFeatureLevel,
    ) -> Self {
        Self {
            context,
            builder,
            target_arch,
            feature_level,
            stats: SimdStats::default(),
        }
    }

    /// Get the accumulated statistics.
    pub fn stats(&self) -> &SimdStats {
        &self.stats
    }

    /// Get the target architecture.
    pub fn target_arch(&self) -> SimdTargetArch {
        self.target_arch
    }

    /// Get the feature level.
    pub fn feature_level(&self) -> SimdFeatureLevel {
        self.feature_level
    }

    // =========================================================================
    // TYPE HELPERS
    // =========================================================================

    /// Create an LLVM vector type for integers.
    pub fn int_vector_type(&self, bits: u32, lanes: u32) -> VectorType<'ctx> {
        let int_type = self.context.custom_width_int_type(bits);
        int_type.vec_type(lanes)
    }

    /// Create an LLVM vector type for f32.
    pub fn f32_vector_type(&self, lanes: u32) -> VectorType<'ctx> {
        self.context.f32_type().vec_type(lanes)
    }

    /// Create an LLVM vector type for f64.
    pub fn f64_vector_type(&self, lanes: u32) -> VectorType<'ctx> {
        self.context.f64_type().vec_type(lanes)
    }

    /// Get the LLVM type for a SIMD element.
    fn element_type(&self, element: SimdElementKind) -> BasicTypeEnum<'ctx> {
        match element {
            SimdElementKind::I8 | SimdElementKind::U8 => {
                self.context.i8_type().into()
            }
            SimdElementKind::I16 | SimdElementKind::U16 => {
                self.context.i16_type().into()
            }
            SimdElementKind::I32 | SimdElementKind::U32 => {
                self.context.i32_type().into()
            }
            SimdElementKind::I64 | SimdElementKind::U64 => {
                self.context.i64_type().into()
            }
            SimdElementKind::F32 => self.context.f32_type().into(),
            SimdElementKind::F64 => self.context.f64_type().into(),
        }
    }

    // =========================================================================
    // VECTOR CONSTRUCTION
    // =========================================================================

    /// Splat (broadcast) a scalar to all lanes.
    ///
    /// ```llvm
    /// %v = insertelement <4 x float> poison, float %scalar, i32 0
    /// %splat = shufflevector <4 x float> %v, <4 x float> poison, <4 x i32> zeroinitializer
    /// ```
    pub fn build_splat(
        &mut self,
        scalar: BasicValueEnum<'ctx>,
        lanes: u32,
        name: &str,
    ) -> Result<VectorValue<'ctx>> {
        self.stats.splats += 1;
        self.stats.ops_lowered += 1;

        let vec_type = match scalar {
            BasicValueEnum::IntValue(v) => v.get_type().vec_type(lanes),
            BasicValueEnum::FloatValue(v) => v.get_type().vec_type(lanes),
            _ => {
                return Err(LlvmLoweringError::InvalidType(
                    "Splat requires int or float scalar".into(),
                ))
            }
        };

        // Insert scalar at index 0
        let poison = vec_type.get_poison();
        let zero = self.context.i32_type().const_zero();
        let v0 = self
            .builder
            .build_insert_element(poison, scalar, zero, &format!("{}_ins", name))
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;

        // Shuffle to broadcast
        let mask = vec_type.const_zero();
        let splat = self
            .builder
            .build_shuffle_vector(v0, poison, mask, name)
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;

        Ok(splat)
    }

    // =========================================================================
    // BINARY OPERATIONS
    // =========================================================================

    /// Build a binary SIMD operation.
    pub fn build_binary_op(
        &mut self,
        op: SimdBinaryOp,
        lhs: VectorValue<'ctx>,
        rhs: VectorValue<'ctx>,
        element: SimdElementKind,
        name: &str,
    ) -> Result<VectorValue<'ctx>> {
        self.stats.binary_ops += 1;
        self.stats.ops_lowered += 1;

        let result = match (op, element.is_float()) {
            // Floating point operations
            (SimdBinaryOp::Add, true) => self
                .builder
                .build_float_add(lhs, rhs, name)
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?,
            (SimdBinaryOp::Sub, true) => self
                .builder
                .build_float_sub(lhs, rhs, name)
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?,
            (SimdBinaryOp::Mul, true) => self
                .builder
                .build_float_mul(lhs, rhs, name)
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?,
            (SimdBinaryOp::Div, true) => self
                .builder
                .build_float_div(lhs, rhs, name)
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?,

            // Integer operations
            (SimdBinaryOp::Add, false) => self
                .builder
                .build_int_add(lhs, rhs, name)
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?,
            (SimdBinaryOp::Sub, false) => self
                .builder
                .build_int_sub(lhs, rhs, name)
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?,
            (SimdBinaryOp::Mul, false) => self
                .builder
                .build_int_mul(lhs, rhs, name)
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?,
            (SimdBinaryOp::Div, false) if element.is_signed() => self
                .builder
                .build_int_signed_div(lhs, rhs, name)
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?,
            (SimdBinaryOp::Div, false) => self
                .builder
                .build_int_unsigned_div(lhs, rhs, name)
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?,

            // Bitwise operations
            (SimdBinaryOp::And, _) => self
                .builder
                .build_and(lhs, rhs, name)
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?,
            (SimdBinaryOp::Or, _) => self
                .builder
                .build_or(lhs, rhs, name)
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?,
            (SimdBinaryOp::Xor, _) => self
                .builder
                .build_xor(lhs, rhs, name)
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?,

            // Min/max use select
            (SimdBinaryOp::Min, false) => {
                self.build_int_minmax(lhs, rhs, true, element.is_signed(), name)?
            }
            (SimdBinaryOp::Max, false) => {
                self.build_int_minmax(lhs, rhs, false, element.is_signed(), name)?
            }
            (SimdBinaryOp::Min, true) => {
                self.build_float_minmax(lhs, rhs, true, name)?
            }
            (SimdBinaryOp::Max, true) => {
                self.build_float_minmax(lhs, rhs, false, name)?
            }

            // Saturating arithmetic via overflow detection + select.
            // add_sat(a, b) = if (a + b) overflows ? (a > 0 ? INT_MAX : INT_MIN) : a + b
            // sub_sat(a, b) = if (a - b) overflows ? (a > 0 ? INT_MAX : INT_MIN) : a - b
            (SimdBinaryOp::SaturatingAdd, false) => {
                let sum = self.builder
                    .build_int_add(lhs, rhs, &format!("{}_sum", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                // Signed overflow: (lhs > 0 && rhs > 0 && sum < 0) || (lhs < 0 && rhs < 0 && sum > 0)
                let zero = lhs.get_type().const_zero();
                let lhs_pos = self.builder
                    .build_int_compare(IntPredicate::SGT, lhs, zero, &format!("{}_lp", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let rhs_pos = self.builder
                    .build_int_compare(IntPredicate::SGT, rhs, zero, &format!("{}_rp", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let sum_neg = self.builder
                    .build_int_compare(IntPredicate::SLT, sum, zero, &format!("{}_sn", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let lhs_neg = self.builder
                    .build_int_compare(IntPredicate::SLT, lhs, zero, &format!("{}_ln", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let rhs_neg = self.builder
                    .build_int_compare(IntPredicate::SLT, rhs, zero, &format!("{}_rn", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let sum_pos = self.builder
                    .build_int_compare(IntPredicate::SGT, sum, zero, &format!("{}_sp", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let pos_overflow = self.builder
                    .build_and(lhs_pos, rhs_pos, &format!("{}_pp", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let pos_overflow = self.builder
                    .build_and(pos_overflow, sum_neg, &format!("{}_po", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let neg_overflow = self.builder
                    .build_and(lhs_neg, rhs_neg, &format!("{}_nn", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let neg_overflow = self.builder
                    .build_and(neg_overflow, sum_pos, &format!("{}_no", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let overflow = self.builder
                    .build_or(pos_overflow, neg_overflow, &format!("{}_ov", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                // Clamp: if positive overflow → INT_MAX (all 1s >> 1), if negative overflow → INT_MIN
                let elem_type = lhs.get_type().get_element_type().into_int_type();
                let max_scalar = elem_type.const_all_ones();
                let one_scalar = elem_type.const_int(1, false);
                let max_scalar = self.builder
                    .build_right_shift(max_scalar, one_scalar, false, "smax_scalar")
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let min_scalar = self.builder
                    .build_not(max_scalar, "smin_scalar")
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let lanes = lhs.get_type().get_size();
                let int_max = VectorType::const_vector(&vec![max_scalar; lanes as usize]);
                let int_min = VectorType::const_vector(&vec![min_scalar; lanes as usize]);
                let int_max_bv: BasicValueEnum = int_max.into();
                let int_min_bv: BasicValueEnum = int_min.into();
                let sum_bv: BasicValueEnum = sum.into();
                let clamp_val = self.builder
                    .build_select(lhs_pos, int_max_bv, int_min_bv, &format!("{}_clamp", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                self.builder
                    .build_select(overflow, clamp_val, sum_bv, name)
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?
                    .into_vector_value()
            }
            (SimdBinaryOp::SaturatingSub, false) => {
                let diff = self.builder
                    .build_int_sub(lhs, rhs, &format!("{}_diff", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let zero = lhs.get_type().const_zero();
                // Signed overflow on sub: (lhs > 0 && rhs < 0 && diff < 0) || (lhs < 0 && rhs > 0 && diff > 0)
                let lhs_pos = self.builder
                    .build_int_compare(IntPredicate::SGT, lhs, zero, &format!("{}_lp", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let rhs_neg = self.builder
                    .build_int_compare(IntPredicate::SLT, rhs, zero, &format!("{}_rn", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let diff_neg = self.builder
                    .build_int_compare(IntPredicate::SLT, diff, zero, &format!("{}_dn", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let lhs_neg = self.builder
                    .build_int_compare(IntPredicate::SLT, lhs, zero, &format!("{}_ln", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let rhs_pos = self.builder
                    .build_int_compare(IntPredicate::SGT, rhs, zero, &format!("{}_rp", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let diff_pos = self.builder
                    .build_int_compare(IntPredicate::SGT, diff, zero, &format!("{}_dp", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let pos_overflow = self.builder
                    .build_and(lhs_pos, rhs_neg, &format!("{}_pp", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let pos_overflow = self.builder
                    .build_and(pos_overflow, diff_neg, &format!("{}_po", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let neg_overflow = self.builder
                    .build_and(lhs_neg, rhs_pos, &format!("{}_nn", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let neg_overflow = self.builder
                    .build_and(neg_overflow, diff_pos, &format!("{}_no", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let overflow = self.builder
                    .build_or(pos_overflow, neg_overflow, &format!("{}_ov", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let elem_type = lhs.get_type().get_element_type().into_int_type();
                let max_scalar = elem_type.const_all_ones();
                let one_scalar = elem_type.const_int(1, false);
                let max_scalar = self.builder
                    .build_right_shift(max_scalar, one_scalar, false, "smax_scalar")
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let min_scalar = self.builder
                    .build_not(max_scalar, "smin_scalar")
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                let lanes = lhs.get_type().get_size();
                let int_max = VectorType::const_vector(&vec![max_scalar; lanes as usize]);
                let int_min = VectorType::const_vector(&vec![min_scalar; lanes as usize]);
                let int_max_bv: BasicValueEnum = int_max.into();
                let int_min_bv: BasicValueEnum = int_min.into();
                let diff_bv: BasicValueEnum = diff.into();
                let clamp_val = self.builder
                    .build_select(lhs_pos, int_max_bv, int_min_bv, &format!("{}_clamp", name))
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;
                self.builder
                    .build_select(overflow, clamp_val, diff_bv, name)
                    .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?
                    .into_vector_value()
            }
            (SimdBinaryOp::SaturatingAdd | SimdBinaryOp::SaturatingSub, true) => {
                return Err(LlvmLoweringError::InvalidType(
                    "Saturating arithmetic not supported for floats".into(),
                ))
            }
        };

        Ok(result)
    }

    /// Build floating point min/max using fcmp + select.
    fn build_float_minmax(
        &self,
        lhs: VectorValue<'ctx>,
        rhs: VectorValue<'ctx>,
        is_min: bool,
        name: &str,
    ) -> Result<VectorValue<'ctx>> {
        use verum_llvm::FloatPredicate;

        let predicate = if is_min {
            FloatPredicate::OLT
        } else {
            FloatPredicate::OGT
        };

        let cmp = self
            .builder
            .build_float_compare(predicate, lhs, rhs, &format!("{}_cmp", name))
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;

        let result = self
            .builder
            .build_select(cmp, lhs, rhs, name)
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?
            .into_vector_value();

        Ok(result)
    }

    /// Build integer min/max using icmp + select.
    fn build_int_minmax(
        &self,
        lhs: VectorValue<'ctx>,
        rhs: VectorValue<'ctx>,
        is_min: bool,
        is_signed: bool,
        name: &str,
    ) -> Result<VectorValue<'ctx>> {
        let predicate = match (is_min, is_signed) {
            (true, true) => IntPredicate::SLT,
            (true, false) => IntPredicate::ULT,
            (false, true) => IntPredicate::SGT,
            (false, false) => IntPredicate::UGT,
        };

        let cmp = self
            .builder
            .build_int_compare(predicate, lhs, rhs, &format!("{}_cmp", name))
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;

        let result = self
            .builder
            .build_select(cmp, lhs, rhs, name)
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?
            .into_vector_value();

        Ok(result)
    }

    // =========================================================================
    // SHUFFLE OPERATIONS
    // =========================================================================

    /// Build a shuffle operation with compile-time mask.
    ///
    /// ```llvm
    /// %result = shufflevector <4 x float> %a, <4 x float> %b, <4 x i32> <i32 0, i32 5, i32 2, i32 7>
    /// ```
    pub fn build_shuffle(
        &mut self,
        a: VectorValue<'ctx>,
        b: VectorValue<'ctx>,
        mask: &[u32],
        name: &str,
    ) -> Result<VectorValue<'ctx>> {
        self.stats.shuffles += 1;
        self.stats.ops_lowered += 1;

        let i32_type = self.context.i32_type();
        let mask_values: Vec<IntValue<'ctx>> = mask
            .iter()
            .map(|&i| i32_type.const_int(i as u64, false))
            .collect();
        let mask_vec = VectorType::const_vector(&mask_values);

        let result = self
            .builder
            .build_shuffle_vector(a, b, mask_vec, name)
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;

        Ok(result)
    }

    // =========================================================================
    // LOAD/STORE OPERATIONS
    // =========================================================================

    /// Build an aligned vector load.
    pub fn build_aligned_load(
        &mut self,
        ptr: PointerValue<'ctx>,
        vec_type: VectorType<'ctx>,
        alignment: u32,
        name: &str,
    ) -> Result<VectorValue<'ctx>> {
        self.stats.ops_lowered += 1;

        let load = self
            .builder
            .build_load(vec_type, ptr, name)
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;

        // Set alignment
        load.as_instruction_value()
            .unwrap()
            .set_alignment(alignment)
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;

        Ok(load.into_vector_value())
    }

    /// Build an aligned vector store.
    pub fn build_aligned_store(
        &mut self,
        value: VectorValue<'ctx>,
        ptr: PointerValue<'ctx>,
        alignment: u32,
    ) -> Result<()> {
        self.stats.ops_lowered += 1;

        let store = self
            .builder
            .build_store(ptr, value)
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;

        store
            .set_alignment(alignment)
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;

        Ok(())
    }
}
