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
//!  x86_64 (SSE4.2/AVX/AVX2/AVX-512), aarch64 (NEON/SVE), RISC-V (V extension)
//! - Operations: splat, load (aligned/unaligned), arithmetic (+, *, fma),
//!  horizontal reductions, shuffle/permute, gather/scatter, masked load/store
//! - `Mask<N>` type for conditional SIMD operations (lane-wise comparisons)
//! - `@multiversion` attribute generates multiple implementations for runtime dispatch
//! - `@target_feature(enable = "avx2")` for platform-specific intrinsic access
//! - VBC opcodes 0xC0-0xCF handle SIMD at bytecode level
//! - LLVM lowering maps to `<N x T>` vector types and vector intrinsics

use verum_llvm::IntPredicate;
use verum_llvm::builder::Builder;
use verum_llvm::context::Context;
use verum_llvm::types::{BasicTypeEnum, VectorType};
use verum_llvm::values::{BasicValue, BasicValueEnum, IntValue, PointerValue, VectorValue};

use super::error::{BuildExt, LlvmLoweringError, Result};

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

/// Per-variant projection for [`SimdFeatureLevel`].
///
/// `max_bits` is the maximum SIMD vector width — also serves as the
/// rank for the derived `PartialOrd`/`Ord` (None=0 < V128 < V256 <
/// V512). `name` is the canonical short identifier (`"none"`, `"v128"`,
/// `"v256"`, `"v512"`).
#[derive(Debug, Clone, Copy)]
pub struct SimdFeatureLevelMeta {
    pub name: &'static str,
    pub max_bits: usize,
}

impl SimdFeatureLevel {
    pub const ALL: &'static [Self] =
        &[Self::None, Self::V128, Self::V256, Self::V512];

    pub const fn meta(self) -> SimdFeatureLevelMeta {
        match self {
            Self::None => SimdFeatureLevelMeta {
                name: "none",
                max_bits: 0,
            },
            Self::V128 => SimdFeatureLevelMeta {
                name: "v128",
                max_bits: 128,
            },
            Self::V256 => SimdFeatureLevelMeta {
                name: "v256",
                max_bits: 256,
            },
            Self::V512 => SimdFeatureLevelMeta {
                name: "v512",
                max_bits: 512,
            },
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        for v in Self::ALL {
            if v.meta().name == s {
                return Some(*v);
            }
        }
        None
    }

    #[inline]
    pub const fn as_str(self) -> &'static str {
        self.meta().name
    }

    /// Get the maximum vector width in bits.
    #[inline]
    pub const fn max_bits(self) -> usize {
        self.meta().max_bits
    }

    /// Get the maximum vector width in bytes.
    #[inline]
    pub const fn max_bytes(self) -> usize {
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

/// Per-variant projection for [`SimdElementKind`].
///
/// Three parallel `matches!()` (legacy `bit_width` / `is_float` /
/// `is_signed`) collapse to per-variant fields. `name` is the
/// Verum-side type spelling (`"i8"`, `"u32"`, `"f64"`, …) — same
/// form returned by `as_str` and accepted by `from_str`. The
/// integer-vs-float partition is exhaustive: every variant is
/// signed, unsigned, or float; cross-cutting pin enforces this.
#[derive(Debug, Clone, Copy)]
pub struct SimdElementKindMeta {
    pub name: &'static str,
    pub bit_width: usize,
    pub is_float: bool,
    pub is_signed: bool,
}

impl SimdElementKind {
    pub const ALL: &'static [Self] = &[
        Self::I8,
        Self::I16,
        Self::I32,
        Self::I64,
        Self::U8,
        Self::U16,
        Self::U32,
        Self::U64,
        Self::F32,
        Self::F64,
    ];

    pub const fn meta(self) -> SimdElementKindMeta {
        match self {
            Self::I8 => SimdElementKindMeta {
                name: "i8",
                bit_width: 8,
                is_float: false,
                is_signed: true,
            },
            Self::I16 => SimdElementKindMeta {
                name: "i16",
                bit_width: 16,
                is_float: false,
                is_signed: true,
            },
            Self::I32 => SimdElementKindMeta {
                name: "i32",
                bit_width: 32,
                is_float: false,
                is_signed: true,
            },
            Self::I64 => SimdElementKindMeta {
                name: "i64",
                bit_width: 64,
                is_float: false,
                is_signed: true,
            },
            Self::U8 => SimdElementKindMeta {
                name: "u8",
                bit_width: 8,
                is_float: false,
                is_signed: false,
            },
            Self::U16 => SimdElementKindMeta {
                name: "u16",
                bit_width: 16,
                is_float: false,
                is_signed: false,
            },
            Self::U32 => SimdElementKindMeta {
                name: "u32",
                bit_width: 32,
                is_float: false,
                is_signed: false,
            },
            Self::U64 => SimdElementKindMeta {
                name: "u64",
                bit_width: 64,
                is_float: false,
                is_signed: false,
            },
            Self::F32 => SimdElementKindMeta {
                name: "f32",
                bit_width: 32,
                is_float: true,
                is_signed: false,
            },
            Self::F64 => SimdElementKindMeta {
                name: "f64",
                bit_width: 64,
                is_float: true,
                is_signed: false,
            },
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        for v in Self::ALL {
            if v.meta().name == s {
                return Some(*v);
            }
        }
        None
    }

    /// Verum-side type spelling (`"i8"`, `"u32"`, `"f64"`, …).
    #[inline]
    pub const fn as_str(self) -> &'static str {
        self.meta().name
    }

    /// Get the size in bits.
    #[inline]
    pub const fn bit_width(self) -> usize {
        self.meta().bit_width
    }

    /// Check if this is a floating point type.
    #[inline]
    pub const fn is_float(self) -> bool {
        self.meta().is_float
    }

    /// Check if this is a signed integer.
    #[inline]
    pub const fn is_signed(self) -> bool {
        self.meta().is_signed
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
            SimdElementKind::I8 | SimdElementKind::U8 => self.context.i8_type().into(),
            SimdElementKind::I16 | SimdElementKind::U16 => self.context.i16_type().into(),
            SimdElementKind::I32 | SimdElementKind::U32 => self.context.i32_type().into(),
            SimdElementKind::I64 | SimdElementKind::U64 => self.context.i64_type().into(),
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
                ));
            }
        };

        // Insert scalar at index 0
        let poison = vec_type.get_poison();
        let zero = self.context.i32_type().const_zero();
        let v0 = self
            .builder
            .build_insert_element(poison, scalar, zero, &format!("{}_ins", name))
            .or_llvm_err()?;

        // Shuffle to broadcast
        let mask = vec_type.const_zero();
        let splat = self
            .builder
            .build_shuffle_vector(v0, poison, mask, name)
            .or_llvm_err()?;

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
                .or_llvm_err()?,
            (SimdBinaryOp::Sub, true) => self
                .builder
                .build_float_sub(lhs, rhs, name)
                .or_llvm_err()?,
            (SimdBinaryOp::Mul, true) => self
                .builder
                .build_float_mul(lhs, rhs, name)
                .or_llvm_err()?,
            (SimdBinaryOp::Div, true) => self
                .builder
                .build_float_div(lhs, rhs, name)
                .or_llvm_err()?,

            // Integer operations
            (SimdBinaryOp::Add, false) => self
                .builder
                .build_int_add(lhs, rhs, name)
                .or_llvm_err()?,
            (SimdBinaryOp::Sub, false) => self
                .builder
                .build_int_sub(lhs, rhs, name)
                .or_llvm_err()?,
            (SimdBinaryOp::Mul, false) => self
                .builder
                .build_int_mul(lhs, rhs, name)
                .or_llvm_err()?,
            (SimdBinaryOp::Div, false) if element.is_signed() => self
                .builder
                .build_int_signed_div(lhs, rhs, name)
                .or_llvm_err()?,
            (SimdBinaryOp::Div, false) => self
                .builder
                .build_int_unsigned_div(lhs, rhs, name)
                .or_llvm_err()?,

            // Bitwise operations
            (SimdBinaryOp::And, _) => self
                .builder
                .build_and(lhs, rhs, name)
                .or_llvm_err()?,
            (SimdBinaryOp::Or, _) => self
                .builder
                .build_or(lhs, rhs, name)
                .or_llvm_err()?,
            (SimdBinaryOp::Xor, _) => self
                .builder
                .build_xor(lhs, rhs, name)
                .or_llvm_err()?,

            // Min/max use select
            (SimdBinaryOp::Min, false) => {
                self.build_int_minmax(lhs, rhs, true, element.is_signed(), name)?
            }
            (SimdBinaryOp::Max, false) => {
                self.build_int_minmax(lhs, rhs, false, element.is_signed(), name)?
            }
            (SimdBinaryOp::Min, true) => self.build_float_minmax(lhs, rhs, true, name)?,
            (SimdBinaryOp::Max, true) => self.build_float_minmax(lhs, rhs, false, name)?,

            // Saturating arithmetic via overflow detection + select.
            // add_sat(a, b) = if (a + b) overflows ? (a > 0 ? INT_MAX : INT_MIN) : a + b
            // sub_sat(a, b) = if (a - b) overflows ? (a > 0 ? INT_MAX : INT_MIN) : a - b
            (SimdBinaryOp::SaturatingAdd, false) => {
                let sum = self
                    .builder
                    .build_int_add(lhs, rhs, &format!("{}_sum", name))
                    .or_llvm_err()?;
                // Signed overflow: (lhs > 0 && rhs > 0 && sum < 0) || (lhs < 0 && rhs < 0 && sum > 0)
                let zero = lhs.get_type().const_zero();
                let lhs_pos = self
                    .builder
                    .build_int_compare(IntPredicate::SGT, lhs, zero, &format!("{}_lp", name))
                    .or_llvm_err()?;
                let rhs_pos = self
                    .builder
                    .build_int_compare(IntPredicate::SGT, rhs, zero, &format!("{}_rp", name))
                    .or_llvm_err()?;
                let sum_neg = self
                    .builder
                    .build_int_compare(IntPredicate::SLT, sum, zero, &format!("{}_sn", name))
                    .or_llvm_err()?;
                let lhs_neg = self
                    .builder
                    .build_int_compare(IntPredicate::SLT, lhs, zero, &format!("{}_ln", name))
                    .or_llvm_err()?;
                let rhs_neg = self
                    .builder
                    .build_int_compare(IntPredicate::SLT, rhs, zero, &format!("{}_rn", name))
                    .or_llvm_err()?;
                let sum_pos = self
                    .builder
                    .build_int_compare(IntPredicate::SGT, sum, zero, &format!("{}_sp", name))
                    .or_llvm_err()?;
                let pos_overflow = self
                    .builder
                    .build_and(lhs_pos, rhs_pos, &format!("{}_pp", name))
                    .or_llvm_err()?;
                let pos_overflow = self
                    .builder
                    .build_and(pos_overflow, sum_neg, &format!("{}_po", name))
                    .or_llvm_err()?;
                let neg_overflow = self
                    .builder
                    .build_and(lhs_neg, rhs_neg, &format!("{}_nn", name))
                    .or_llvm_err()?;
                let neg_overflow = self
                    .builder
                    .build_and(neg_overflow, sum_pos, &format!("{}_no", name))
                    .or_llvm_err()?;
                let overflow = self
                    .builder
                    .build_or(pos_overflow, neg_overflow, &format!("{}_ov", name))
                    .or_llvm_err()?;
                // Clamp: if positive overflow → INT_MAX (all 1s >> 1), if negative overflow → INT_MIN
                let elem_type = lhs.get_type().get_element_type().into_int_type();
                let max_scalar = elem_type.const_all_ones();
                let one_scalar = elem_type.const_int(1, false);
                let max_scalar = self
                    .builder
                    .build_right_shift(max_scalar, one_scalar, false, "smax_scalar")
                    .or_llvm_err()?;
                let min_scalar = self
                    .builder
                    .build_not(max_scalar, "smin_scalar")
                    .or_llvm_err()?;
                let lanes = lhs.get_type().get_size();
                let int_max = VectorType::const_vector(&vec![max_scalar; lanes as usize]);
                let int_min = VectorType::const_vector(&vec![min_scalar; lanes as usize]);
                let int_max_bv: BasicValueEnum = int_max.into();
                let int_min_bv: BasicValueEnum = int_min.into();
                let sum_bv: BasicValueEnum = sum.into();
                let clamp_val = self
                    .builder
                    .build_select(lhs_pos, int_max_bv, int_min_bv, &format!("{}_clamp", name))
                    .or_llvm_err()?;
                self.builder
                    .build_select(overflow, clamp_val, sum_bv, name)
                    .or_llvm_err()?
                    .into_vector_value()
            }
            (SimdBinaryOp::SaturatingSub, false) => {
                let diff = self
                    .builder
                    .build_int_sub(lhs, rhs, &format!("{}_diff", name))
                    .or_llvm_err()?;
                let zero = lhs.get_type().const_zero();
                // Signed overflow on sub: (lhs > 0 && rhs < 0 && diff < 0) || (lhs < 0 && rhs > 0 && diff > 0)
                let lhs_pos = self
                    .builder
                    .build_int_compare(IntPredicate::SGT, lhs, zero, &format!("{}_lp", name))
                    .or_llvm_err()?;
                let rhs_neg = self
                    .builder
                    .build_int_compare(IntPredicate::SLT, rhs, zero, &format!("{}_rn", name))
                    .or_llvm_err()?;
                let diff_neg = self
                    .builder
                    .build_int_compare(IntPredicate::SLT, diff, zero, &format!("{}_dn", name))
                    .or_llvm_err()?;
                let lhs_neg = self
                    .builder
                    .build_int_compare(IntPredicate::SLT, lhs, zero, &format!("{}_ln", name))
                    .or_llvm_err()?;
                let rhs_pos = self
                    .builder
                    .build_int_compare(IntPredicate::SGT, rhs, zero, &format!("{}_rp", name))
                    .or_llvm_err()?;
                let diff_pos = self
                    .builder
                    .build_int_compare(IntPredicate::SGT, diff, zero, &format!("{}_dp", name))
                    .or_llvm_err()?;
                let pos_overflow = self
                    .builder
                    .build_and(lhs_pos, rhs_neg, &format!("{}_pp", name))
                    .or_llvm_err()?;
                let pos_overflow = self
                    .builder
                    .build_and(pos_overflow, diff_neg, &format!("{}_po", name))
                    .or_llvm_err()?;
                let neg_overflow = self
                    .builder
                    .build_and(lhs_neg, rhs_pos, &format!("{}_nn", name))
                    .or_llvm_err()?;
                let neg_overflow = self
                    .builder
                    .build_and(neg_overflow, diff_pos, &format!("{}_no", name))
                    .or_llvm_err()?;
                let overflow = self
                    .builder
                    .build_or(pos_overflow, neg_overflow, &format!("{}_ov", name))
                    .or_llvm_err()?;
                let elem_type = lhs.get_type().get_element_type().into_int_type();
                let max_scalar = elem_type.const_all_ones();
                let one_scalar = elem_type.const_int(1, false);
                let max_scalar = self
                    .builder
                    .build_right_shift(max_scalar, one_scalar, false, "smax_scalar")
                    .or_llvm_err()?;
                let min_scalar = self
                    .builder
                    .build_not(max_scalar, "smin_scalar")
                    .or_llvm_err()?;
                let lanes = lhs.get_type().get_size();
                let int_max = VectorType::const_vector(&vec![max_scalar; lanes as usize]);
                let int_min = VectorType::const_vector(&vec![min_scalar; lanes as usize]);
                let int_max_bv: BasicValueEnum = int_max.into();
                let int_min_bv: BasicValueEnum = int_min.into();
                let diff_bv: BasicValueEnum = diff.into();
                let clamp_val = self
                    .builder
                    .build_select(lhs_pos, int_max_bv, int_min_bv, &format!("{}_clamp", name))
                    .or_llvm_err()?;
                self.builder
                    .build_select(overflow, clamp_val, diff_bv, name)
                    .or_llvm_err()?
                    .into_vector_value()
            }
            (SimdBinaryOp::SaturatingAdd | SimdBinaryOp::SaturatingSub, true) => {
                return Err(LlvmLoweringError::InvalidType(
                    "Saturating arithmetic not supported for floats".into(),
                ));
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
            .or_llvm_err()?;

        let result = self
            .builder
            .build_select(cmp, lhs, rhs, name)
            .or_llvm_err()?
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
            .or_llvm_err()?;

        let result = self
            .builder
            .build_select(cmp, lhs, rhs, name)
            .or_llvm_err()?
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
            .or_llvm_err()?;

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
            .or_llvm_err()?;

        // Set alignment
        load.as_instruction_value()
            .unwrap()
            .set_alignment(alignment)
            .or_llvm_err()?;

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
            .or_llvm_err()?;

        store
            .set_alignment(alignment)
            .or_llvm_err()?;

        Ok(())
    }
}

#[cfg(test)]
mod meta_consolidation_pins {
    use super::*;

    #[test]
    fn meta_pin_simd_feature_level_round_trip_and_max_bits_dense() {
        assert_eq!(SimdFeatureLevel::ALL.len(), 4);
        for v in SimdFeatureLevel::ALL {
            let s = v.as_str();
            assert_eq!(
                SimdFeatureLevel::from_str(s),
                Some(*v),
                "SimdFeatureLevel::{:?}: '{}' round-trip",
                v,
                s
            );
        }
        // max_bits is dense in the SIMD-width family: 0/128/256/512.
        let pairs: &[(SimdFeatureLevel, usize)] = &[
            (SimdFeatureLevel::None, 0),
            (SimdFeatureLevel::V128, 128),
            (SimdFeatureLevel::V256, 256),
            (SimdFeatureLevel::V512, 512),
        ];
        for (v, expected) in pairs {
            assert_eq!(v.max_bits(), *expected);
            assert_eq!(v.max_bytes(), expected / 8);
        }
        // PartialOrd derived ordering matches max_bits ordering
        // (declaration order). Pin for the legacy `<` invariant.
        for w in SimdFeatureLevel::ALL.windows(2) {
            assert!(
                w[0] < w[1],
                "PartialOrd drift: {:?} < {:?}",
                w[0],
                w[1]
            );
            assert!(w[0].max_bits() < w[1].max_bits());
        }
    }

    #[test]
    fn meta_pin_simd_element_kind_round_trip_partition_and_widths() {
        assert_eq!(SimdElementKind::ALL.len(), 10);
        let mut seen = Vec::new();
        for v in SimdElementKind::ALL {
            let s = v.as_str();
            assert_eq!(
                SimdElementKind::from_str(s),
                Some(*v),
                "SimdElementKind::{:?}: '{}' round-trip",
                v,
                s
            );
            assert!(!seen.contains(&s), "duplicate name '{}'", s);
            seen.push(s);
        }
        // Three-way partition is exhaustive: every element is exactly
        // one of {signed int, unsigned int, float}. Cross-pin: no
        // variant is both signed and float; the unsigned bucket is
        // the implicit complement.
        let signed_count =
            SimdElementKind::ALL.iter().filter(|v| v.is_signed()).count();
        let float_count =
            SimdElementKind::ALL.iter().filter(|v| v.is_float()).count();
        let unsigned_count = SimdElementKind::ALL
            .iter()
            .filter(|v| !v.is_signed() && !v.is_float())
            .count();
        assert_eq!(signed_count, 4, "I8/I16/I32/I64");
        assert_eq!(float_count, 2, "F32/F64");
        assert_eq!(unsigned_count, 4, "U8/U16/U32/U64");
        assert_eq!(signed_count + float_count + unsigned_count, 10);
        for v in SimdElementKind::ALL {
            assert!(
                !(v.is_signed() && v.is_float()),
                "SimdElementKind::{:?}: signed ⊕ float must be disjoint",
                v
            );
        }
        // Bit-width consistency: each variant's width matches the
        // suffix on its name (`u8`→8, `f64`→64, …).
        for v in SimdElementKind::ALL {
            let name = v.as_str();
            // Strip prefix (i/u/f), parse remaining digits.
            let digits = &name[1..];
            let expected: usize = digits.parse().expect("name suffix must be digits");
            assert_eq!(
                v.bit_width(),
                expected,
                "SimdElementKind::{:?}: name suffix '{}' vs bit_width {}",
                v,
                digits,
                v.bit_width()
            );
        }
        // Widths are restricted to {8, 16, 32, 64}.
        for v in SimdElementKind::ALL {
            assert!(matches!(v.bit_width(), 8 | 16 | 32 | 64));
        }
    }
}
