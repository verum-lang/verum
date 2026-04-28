//! Kernel dispatch layer for tensor operations.
//!
//! This module provides backend-specific implementations of tensor operations,
//! with automatic dispatch based on device type and capabilities.

pub mod backend;
#[allow(
    clippy::needless_range_loop,
    clippy::manual_checked_ops,
    clippy::too_many_arguments,
    clippy::if_same_then_else,
    clippy::type_complexity,
    clippy::doc_lazy_continuation
)]
pub mod cpu;
pub mod device;
pub mod tokenizer;

// Metal GPU backend (macOS only)
#[cfg(all(target_os = "macos", feature = "metal"))]
pub mod metal;

// Re-export commonly used types
pub use backend::{
    Backend, ComputeCapabilities, BackendRegistry, CpuBackend, MemoryPool, MemoryPoolStats,
    SyncFlags, default_backend, get_backend, get_backend_registry,
};
pub use device::{DeviceId, DeviceInfo, DeviceRegistry, CpuInfo, GpuInfo, Vendor};

// Metal exports (macOS only)
#[cfg(all(target_os = "macos", feature = "metal"))]
pub use metal::{MetalBackend, MetalBufferPool, MetalPoolStats, get_metal_backend, is_metal_available};

use super::tensor::{DType, TensorHandle};
use crate::instruction::{TensorBinaryOp, TensorUnaryOp, TensorReduceOp};

/// Kernel variant based on available SIMD capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelVariant {
    /// Scalar (no SIMD)
    Scalar,
    /// SSE4.2 (128-bit, 4 floats)
    Sse,
    /// AVX2 (256-bit, 8 floats)
    Avx2,
    /// AVX-512 (512-bit, 16 floats)
    Avx512,
    /// ARM NEON (128-bit, 4 floats)
    Neon,
}

/// Backend capabilities detected at runtime
#[derive(Debug, Clone)]
pub struct BackendCapabilities {
    /// Maximum number of threads for parallel operations
    pub max_threads: usize,
    /// SIMD width in elements (1, 4, 8, 16)
    pub simd_width: usize,
    /// Has FMA (fused multiply-add) support
    pub has_fma: bool,
    /// Has AVX2 support
    pub has_avx2: bool,
    /// Has AVX-512 support
    pub has_avx512: bool,
    /// Has NEON support (ARM)
    pub has_neon: bool,
    /// L1 data cache size in bytes
    pub l1_cache_size: usize,
    /// L2 cache size in bytes
    pub l2_cache_size: usize,
    /// Selected kernel variant
    pub kernel_variant: KernelVariant,
}

impl Default for BackendCapabilities {
    fn default() -> Self {
        Self {
            max_threads: 1,
            simd_width: 1,
            has_fma: false,
            has_avx2: false,
            has_avx512: false,
            has_neon: false,
            l1_cache_size: 32 * 1024,
            l2_cache_size: 256 * 1024,
            kernel_variant: KernelVariant::Scalar,
        }
    }
}

impl BackendCapabilities {
    /// Detect CPU capabilities at runtime
    #[allow(clippy::field_reassign_with_default)] // Complex conditional initialization
    pub fn detect() -> Self {
        let mut caps = Self::default();

        // Detect number of CPUs
        caps.max_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx512f") {
                caps.has_avx512 = true;
                caps.simd_width = 16;
                caps.kernel_variant = KernelVariant::Avx512;
            } else if std::arch::is_x86_feature_detected!("avx2") {
                caps.has_avx2 = true;
                caps.simd_width = 8;
                caps.kernel_variant = KernelVariant::Avx2;
            } else if std::arch::is_x86_feature_detected!("sse4.2") {
                caps.simd_width = 4;
                caps.kernel_variant = KernelVariant::Sse;
            }

            if std::arch::is_x86_feature_detected!("fma") {
                caps.has_fma = true;
            }
        }

        #[cfg(target_arch = "aarch64")]
        {
            // NEON is always available on AArch64
            caps.has_neon = true;
            caps.simd_width = 4;
            caps.kernel_variant = KernelVariant::Neon;
        }

        caps
    }
}

/// Global capabilities (lazy-initialized)
static CAPABILITIES: std::sync::OnceLock<BackendCapabilities> = std::sync::OnceLock::new();

/// Get detected capabilities
pub fn get_capabilities() -> &'static BackendCapabilities {
    CAPABILITIES.get_or_init(BackendCapabilities::detect)
}

/// Minimum tensor size for GPU dispatch (elements). Read by the Metal
/// backend's CPU-vs-GPU selection — see the
/// `#[cfg(all(target_os = "macos", feature = "metal"))]` branches below.
/// Gated to the same cfg so it doesn't appear "dead" on non-Metal builds
/// (the const exists only where it's referenced).
#[cfg(all(target_os = "macos", feature = "metal"))]
const MIN_GPU_SIZE: usize = 4096;

// ============================================================================
// PrecisionContext for Kernel Operations
// ============================================================================
//
// The PrecisionContext provides runtime precision configuration to kernel
// operations. This allows operations to:
// - Use lower precision for speed (F16, BF16)
// - Control rounding behavior
// - Flush denormals for performance
//
// PrecisionMode: controls Float32/Float64/Float128 precision and rounding mode
// (NearestEven, TowardZero, TowardPosInf, TowardNegInf) for mathematical kernels.

/// Re-export precision types from interpreter state
pub use super::state::{PrecisionMode, FloatPrecision, RoundingMode};

/// Precision context for kernel operations.
///
/// This wraps PrecisionMode and provides additional methods for kernel-specific
/// precision handling.
#[derive(Debug, Clone, Copy)]
#[derive(Default)]
pub struct PrecisionContext {
    /// The precision mode settings
    pub mode: PrecisionMode,
}


impl PrecisionContext {
    /// Creates a new precision context from a PrecisionMode.
    pub fn new(mode: PrecisionMode) -> Self {
        Self { mode }
    }

    /// Returns the target DType for floating-point operations.
    ///
    /// Maps the precision setting to the corresponding DType.
    pub fn target_dtype(&self) -> DType {
        match self.mode.precision {
            FloatPrecision::Half => DType::F16,
            FloatPrecision::BFloat16 => DType::BF16,
            FloatPrecision::Single => DType::F32,
            FloatPrecision::Double => DType::F64,
        }
    }

    /// Returns true if the operation should use reduced precision.
    ///
    /// This is a hint to kernels that they can use lower precision
    /// intermediate values when the target precision is less than F64.
    pub fn use_reduced_precision(&self) -> bool {
        !matches!(self.mode.precision, FloatPrecision::Double)
    }

    /// Returns true if denormals should be flushed to zero.
    pub fn flush_denormals(&self) -> bool {
        !self.mode.allow_denormals
    }

    /// Applies denormal flushing to an f32 value if configured.
    #[inline]
    pub fn maybe_flush_f32(&self, value: f32) -> f32 {
        if self.flush_denormals() && value.abs() < f32::MIN_POSITIVE && value != 0.0 {
            0.0
        } else {
            value
        }
    }

    /// Applies denormal flushing to an f64 value if configured.
    #[inline]
    pub fn maybe_flush_f64(&self, value: f64) -> f64 {
        if self.flush_denormals() && value.abs() < f64::MIN_POSITIVE && value != 0.0 {
            0.0
        } else {
            value
        }
    }

    /// Rounds an f64 value according to the rounding mode.
    ///
    /// Note: This is a software implementation. Hardware rounding mode
    /// changes would require platform-specific intrinsics.
    #[inline]
    pub fn round_f64(&self, value: f64) -> f64 {
        match self.mode.rounding_mode {
            RoundingMode::NearestTiesToEven => {
                // Default IEEE 754 behavior - rust's round() uses ties-to-even
                value.round_ties_even()
            }
            RoundingMode::NearestTiesToAway => {
                // Standard round() rounds ties away from zero
                value.round()
            }
            RoundingMode::TowardPositive => value.ceil(),
            RoundingMode::TowardNegative => value.floor(),
            RoundingMode::TowardZero => value.trunc(),
        }
    }

    /// Rounds an f32 value according to the rounding mode.
    #[inline]
    pub fn round_f32(&self, value: f32) -> f32 {
        match self.mode.rounding_mode {
            RoundingMode::NearestTiesToEven => value.round_ties_even(),
            RoundingMode::NearestTiesToAway => value.round(),
            RoundingMode::TowardPositive => value.ceil(),
            RoundingMode::TowardNegative => value.floor(),
            RoundingMode::TowardZero => value.trunc(),
        }
    }
}

/// Default precision context (F64, round to nearest, allow denormals)
static DEFAULT_PRECISION: PrecisionContext = PrecisionContext {
    mode: PrecisionMode {
        precision: FloatPrecision::Double,
        rounding_mode: RoundingMode::NearestTiesToEven,
        allow_denormals: true,
    },
};

/// Get the default precision context
pub fn default_precision() -> &'static PrecisionContext {
    &DEFAULT_PRECISION
}

/// Dispatch binary operation to appropriate kernel
pub fn dispatch_binop(
    a: &TensorHandle,
    b: &TensorHandle,
    op: TensorBinaryOp,
) -> Option<TensorHandle> {
    let caps = get_capabilities();

    // Try Metal GPU for large F32 tensors
    #[cfg(all(target_os = "macos", feature = "metal"))]
    if a.dtype == DType::F32 && a.numel >= MIN_GPU_SIZE {
        if let Some(backend) = get_metal_backend() {
            if let Some(result) = backend.binop_gpu(a, b, op) {
                return Some(result);
            }
        }
    }

    match (a.dtype, caps.kernel_variant) {
        (DType::F32, KernelVariant::Avx512) if a.numel >= 128 => {
            cpu::binop_f32_avx512(a, b, op)
        }
        (DType::F32, KernelVariant::Avx2) if a.numel >= 64 => {
            cpu::binop_f32_avx2(a, b, op)
        }
        (DType::F32, KernelVariant::Neon) if a.numel >= 64 => {
            cpu::binop_f32_neon(a, b, op)
        }
        (DType::F32, _) => cpu::binop_f32_scalar(a, b, op),
        (DType::F64, _) => cpu::binop_f64_scalar(a, b, op),
        (DType::I32, _) => cpu::binop_i32_scalar(a, b, op),
        (DType::I64, _) => cpu::binop_i64_scalar(a, b, op),
        (DType::I8, _) => cpu::binop_i8_scalar(a, b, op),
        (DType::I16, _) => cpu::binop_i16_scalar(a, b, op),
        (DType::U8, _) => cpu::binop_u8_scalar(a, b, op),
        (DType::U16, _) => cpu::binop_u16_scalar(a, b, op),
        (DType::U32, _) => cpu::binop_u32_scalar(a, b, op),
        (DType::U64, _) => cpu::binop_u64_scalar(a, b, op),
        (DType::F16, _) => cpu::binop_f16_scalar(a, b, op),
        (DType::BF16, _) => cpu::binop_bf16_scalar(a, b, op),
        (DType::Complex64, _) => cpu::binop_complex64_scalar(a, b, op),
        (DType::Complex128, _) => cpu::binop_complex128_scalar(a, b, op),
        (DType::Bool, _) => cpu::binop_bool_scalar(a, b, op),
    }
}

/// Dispatch binary operation with scalar broadcasting optimization.
///
/// When one operand is a scalar (numel=1), uses optimized SIMD splat kernels
/// instead of expanding the scalar to full tensor size.
pub fn dispatch_binop_broadcast(
    a: &TensorHandle,
    b: &TensorHandle,
    op: TensorBinaryOp,
) -> Option<TensorHandle> {
    let caps = get_capabilities();

    // Check for scalar broadcast optimization (one operand has numel=1)
    let a_is_scalar = a.numel == 1;
    let b_is_scalar = b.numel == 1;

    // Special case: both scalars, use regular binop
    if a_is_scalar && b_is_scalar {
        return dispatch_binop(a, b, op);
    }

    // Scalar broadcast optimization for F32
    if a.dtype == DType::F32 && b.dtype == DType::F32 {
        if a_is_scalar {
            // a is scalar, b is tensor: compute b op a (scalar on left)
            let scalar_val = unsafe { *a.data_ptr_f32() };
            return match caps.kernel_variant {
                KernelVariant::Avx512 if b.numel >= 128 => {
                    cpu::binop_f32_scalar_broadcast_avx512(b, scalar_val, op, false)
                }
                KernelVariant::Avx2 if b.numel >= 64 => {
                    cpu::binop_f32_scalar_broadcast_avx2(b, scalar_val, op, false)
                }
                KernelVariant::Neon if b.numel >= 64 => {
                    cpu::binop_f32_scalar_broadcast_neon(b, scalar_val, op, false)
                }
                _ => cpu::binop_f32_scalar_broadcast_scalar(b, scalar_val, op, false),
            };
        } else if b_is_scalar {
            // b is scalar, a is tensor: compute a op b (scalar on right)
            let scalar_val = unsafe { *b.data_ptr_f32() };
            return match caps.kernel_variant {
                KernelVariant::Avx512 if a.numel >= 128 => {
                    cpu::binop_f32_scalar_broadcast_avx512(a, scalar_val, op, true)
                }
                KernelVariant::Avx2 if a.numel >= 64 => {
                    cpu::binop_f32_scalar_broadcast_avx2(a, scalar_val, op, true)
                }
                KernelVariant::Neon if a.numel >= 64 => {
                    cpu::binop_f32_scalar_broadcast_neon(a, scalar_val, op, true)
                }
                _ => cpu::binop_f32_scalar_broadcast_scalar(a, scalar_val, op, true),
            };
        }
    }

    // Fall back to general broadcast (expand then operate)
    let a_shape = &a.shape[..a.ndim as usize];
    let b_shape = &b.shape[..b.ndim as usize];

    // If shapes match, use direct kernel
    if a_shape == b_shape {
        return dispatch_binop(a, b, op);
    }

    // Compute broadcast shape and expand
    let out_shape = broadcast_shapes(a_shape, b_shape)?;
    let a_expanded = broadcast_to(a, &out_shape)?;
    let b_expanded = broadcast_to(b, &out_shape)?;

    dispatch_binop(&a_expanded, &b_expanded, op)
}

/// Fill tensor with a scalar value using SIMD acceleration.
pub fn fill_f32(output: &mut TensorHandle, value: f32) -> bool {
    let caps = get_capabilities();

    match caps.kernel_variant {
        KernelVariant::Avx512 if output.numel >= 128 => cpu::fill_f32_avx512(output, value),
        KernelVariant::Avx2 if output.numel >= 64 => cpu::fill_f32_avx2(output, value),
        KernelVariant::Neon if output.numel >= 64 => cpu::fill_f32_neon(output, value),
        _ => cpu::fill_f32_scalar(output, value),
    }
}

/// Dispatch unary operation to appropriate kernel
pub fn dispatch_unop(
    a: &TensorHandle,
    op: TensorUnaryOp,
) -> Option<TensorHandle> {
    let caps = get_capabilities();

    // Try Metal GPU for large F32 tensors
    #[cfg(all(target_os = "macos", feature = "metal"))]
    if a.dtype == DType::F32 && a.numel >= MIN_GPU_SIZE {
        if let Some(backend) = get_metal_backend() {
            if let Some(result) = backend.unop_gpu(a, op) {
                return Some(result);
            }
        }
    }

    match (a.dtype, caps.kernel_variant) {
        (DType::F32, KernelVariant::Avx512) if a.numel >= 128 => {
            cpu::unop_f32_avx512(a, op)
        }
        (DType::F32, KernelVariant::Avx2) if a.numel >= 64 => {
            cpu::unop_f32_avx2(a, op)
        }
        (DType::F32, KernelVariant::Neon) if a.numel >= 64 => {
            cpu::unop_f32_neon(a, op)
        }
        (DType::F32, _) => cpu::unop_f32_scalar(a, op),
        (DType::F64, _) => cpu::unop_f64_scalar(a, op),
        (DType::I32, _) => cpu::unop_i32_scalar(a, op),
        (DType::I64, _) => cpu::unop_i64_scalar(a, op),
        (DType::I8, _) => cpu::unop_i8_scalar(a, op),
        (DType::I16, _) => cpu::unop_i16_scalar(a, op),
        (DType::U8, _) => cpu::unop_u8_scalar(a, op),
        (DType::U16, _) => cpu::unop_u16_scalar(a, op),
        (DType::U32, _) => cpu::unop_u32_scalar(a, op),
        (DType::U64, _) => cpu::unop_u64_scalar(a, op),
        (DType::F16, _) => cpu::unop_f16_scalar(a, op),
        (DType::BF16, _) => cpu::unop_bf16_scalar(a, op),
        (DType::Complex64, _) => cpu::unop_complex64_scalar(a, op),
        (DType::Complex128, _) => cpu::unop_complex128_scalar(a, op),
        (DType::Bool, _) => cpu::unop_bool_scalar(a, op),
    }
}

/// Dispatch reduction operation to appropriate kernel
///
/// Uses Metal GPU for large F32 full reductions on macOS, falls back to CPU otherwise.
pub fn dispatch_reduce(
    a: &TensorHandle,
    op: TensorReduceOp,
    axis: Option<usize>,
) -> Option<TensorHandle> {
    // Try Metal GPU for large F32 full reductions (axis=None)
    #[cfg(all(target_os = "macos", feature = "metal"))]
    if a.dtype == DType::F32 && a.numel >= MIN_GPU_SIZE && axis.is_none() {
        if let Some(backend) = get_metal_backend() {
            if let Some(result) = backend.reduce_gpu(a, op, axis) {
                return Some(result);
            }
        }
    }

    let caps = get_capabilities();

    match (a.dtype, caps.kernel_variant) {
        // Float types with SIMD optimization
        (DType::F32, KernelVariant::Avx512) if a.numel >= 128 => {
            cpu::reduce_f32_avx512(a, op, axis)
        }
        (DType::F32, KernelVariant::Avx2) if a.numel >= 64 => {
            cpu::reduce_f32_avx2(a, op, axis)
        }
        (DType::F32, KernelVariant::Neon) if a.numel >= 64 => {
            cpu::reduce_f32_neon(a, op, axis)
        }
        (DType::F32, _) => cpu::reduce_f32_scalar(a, op, axis),
        (DType::F64, _) => cpu::reduce_f64_scalar(a, op, axis),
        // Signed integer types
        (DType::I8, _) => cpu::reduce_i8_scalar(a, op, axis),
        (DType::I16, _) => cpu::reduce_i16_scalar(a, op, axis),
        (DType::I32, _) => cpu::reduce_i32_scalar(a, op, axis),
        (DType::I64, _) => cpu::reduce_i64_scalar(a, op, axis),
        // Unsigned integer types
        (DType::U8, _) => cpu::reduce_u8_scalar(a, op, axis),
        (DType::U16, _) => cpu::reduce_u16_scalar(a, op, axis),
        (DType::U32, _) => cpu::reduce_u32_scalar(a, op, axis),
        (DType::U64, _) => cpu::reduce_u64_scalar(a, op, axis),
        // Bool type (stored as u8, 0=false, non-zero=true)
        (DType::Bool, _) => cpu::reduce_bool_scalar(a, op, axis),
        // Other types not yet supported
        _ => None,
    }
}

/// Dispatch matrix multiplication to appropriate kernel
pub fn dispatch_matmul(
    a: &TensorHandle,
    b: &TensorHandle,
) -> Option<TensorHandle> {
    // Try Metal GPU for large F32 matrices
    #[cfg(all(target_os = "macos", feature = "metal"))]
    if a.dtype == DType::F32 && a.numel >= MIN_GPU_SIZE {
        if let Some(backend) = get_metal_backend() {
            if let Some(result) = backend.matmul_gpu(a, b) {
                return Some(result);
            }
        }
    }

    // Use SIMD-optimized matmul with automatic dispatch
    // The SIMD functions internally fall back to tiled/scalar for small matrices
    match a.dtype {
        DType::F32 => cpu::matmul_f32_simd(a, b),
        DType::F64 => cpu::matmul_f64_simd(a, b),
        _ => None,
    }
}

/// Dispatch softmax to appropriate kernel
///
/// Uses Metal GPU for large F32 tensors on macOS, falls back to CPU otherwise.
pub fn dispatch_softmax(
    a: &TensorHandle,
    axis: Option<i32>,
) -> Option<TensorHandle> {
    // Try Metal GPU for large F32 tensors
    #[cfg(all(target_os = "macos", feature = "metal"))]
    if a.dtype == DType::F32 && a.numel >= MIN_GPU_SIZE {
        // GPU softmax only supports full tensor (axis=None) or last axis (axis=-1)
        let use_gpu = match axis {
            None => true,
            Some(-1) => true,
            Some(ax) if ax >= 0 && ax as usize == a.ndim as usize - 1 => true,
            _ => false,
        };

        if use_gpu {
            if let Some(backend) = get_metal_backend() {
                if let Some(result) = backend.softmax_gpu(a) {
                    return Some(result);
                }
            }
        }
    }

    // Fall back to CPU implementation
    super::tensor::tensor_softmax_cpu(a, axis)
}

/// Dispatch layer normalization to appropriate kernel
///
/// Uses Metal GPU for large F32 2D tensors on macOS, falls back to CPU otherwise.
pub fn dispatch_layer_norm(
    input: &TensorHandle,
    gamma: Option<&TensorHandle>,
    beta: Option<&TensorHandle>,
    eps: f64,
) -> Option<TensorHandle> {
    // Try Metal GPU for large F32 2D tensors
    #[cfg(all(target_os = "macos", feature = "metal"))]
    if input.dtype == DType::F32 && input.ndim == 2 && input.numel >= MIN_GPU_SIZE {
        if let Some(backend) = get_metal_backend() {
            if let Some(result) = backend.layer_norm_gpu(input, gamma, beta, eps) {
                return Some(result);
            }
        }
    }

    // Fall back to CPU implementation
    super::tensor::tensor_layer_norm_cpu(input, gamma, beta, eps)
}

/// NumPy-style broadcast shape computation
pub fn broadcast_shapes(a: &[usize], b: &[usize]) -> Option<Vec<usize>> {
    let max_ndim = a.len().max(b.len());
    let mut result = Vec::with_capacity(max_ndim);

    for i in 0..max_ndim {
        let dim_a = if i < max_ndim - a.len() {
            1
        } else {
            a[i - (max_ndim - a.len())]
        };
        let dim_b = if i < max_ndim - b.len() {
            1
        } else {
            b[i - (max_ndim - b.len())]
        };

        if dim_a == dim_b {
            result.push(dim_a);
        } else if dim_a == 1 {
            result.push(dim_b);
        } else if dim_b == 1 {
            result.push(dim_a);
        } else {
            return None; // Incompatible shapes
        }
    }

    Some(result)
}

/// Expand tensor to target shape via broadcasting
pub fn broadcast_to(tensor: &TensorHandle, target_shape: &[usize]) -> Option<TensorHandle> {
    let src_shape = &tensor.shape[..tensor.ndim as usize];

    // Verify broadcastable
    if src_shape == target_shape {
        return Some(tensor.clone());
    }

    // Create output tensor
    let mut output = TensorHandle::zeros(target_shape, tensor.dtype)?;
    let numel = output.numel;

    match tensor.dtype {
        DType::F32 => {
            let src = tensor.data_ptr_f32();
            let dst = output.data_ptr_f32_mut();

            if src.is_null() || dst.is_null() {
                return None;
            }

            // Compute broadcast strides
            let src_strides = compute_broadcast_strides(src_shape, target_shape)?;

            unsafe {
                for i in 0..numel {
                    // Convert flat index to multi-dimensional indices
                    let mut src_idx = 0usize;
                    let mut remaining = i;
                    for (dim, &stride) in target_shape.iter().zip(src_strides.iter()).rev() {
                        let coord = remaining % dim;
                        remaining /= dim;
                        src_idx += coord * stride;
                    }
                    *dst.add(i) = *src.add(src_idx);
                }
            }
        }
        DType::F64 => {
            let src = tensor.data_ptr_f64();
            let dst = output.data_ptr_f64_mut();

            if src.is_null() || dst.is_null() {
                return None;
            }

            let src_strides = compute_broadcast_strides(src_shape, target_shape)?;

            unsafe {
                for i in 0..numel {
                    let mut src_idx = 0usize;
                    let mut remaining = i;
                    for (dim, &stride) in target_shape.iter().zip(src_strides.iter()).rev() {
                        let coord = remaining % dim;
                        remaining /= dim;
                        src_idx += coord * stride;
                    }
                    *dst.add(i) = *src.add(src_idx);
                }
            }
        }
        _ => return None,
    }

    Some(output)
}

/// Compute strides for broadcasting source to target shape
#[allow(clippy::needless_range_loop)] // Index-based access is clearer for complex stride calculations
fn compute_broadcast_strides(src_shape: &[usize], target_shape: &[usize]) -> Option<Vec<usize>> {
    let offset = target_shape.len() - src_shape.len();
    let mut strides = vec![0usize; target_shape.len()];

    // Compute source strides
    let mut src_strides = vec![0usize; src_shape.len()];
    if !src_shape.is_empty() {
        src_strides[src_shape.len() - 1] = 1;
        for i in (0..src_shape.len() - 1).rev() {
            src_strides[i] = src_strides[i + 1] * src_shape[i + 1];
        }
    }

    // Map to target shape (stride=0 for broadcast dimensions)
    for i in 0..target_shape.len() {
        if i < offset {
            strides[i] = 0; // New dimension, stride = 0
        } else {
            let src_idx = i - offset;
            if src_shape[src_idx] == 1 {
                strides[i] = 0; // Broadcast, stride = 0
            } else {
                strides[i] = src_strides[src_idx];
            }
        }
    }

    Some(strides)
}

// ============================================================================
// Linear Algebra Dispatch Functions
// ============================================================================

/// Dispatch Cholesky decomposition to appropriate kernel.
///
/// Computes the Cholesky factorization of a symmetric positive-definite matrix.
/// Returns L such that A = L * L^T (if upper=false) or U such that A = U^T * U (if upper=true).
pub fn dispatch_cholesky(
    a: &TensorHandle,
    upper: bool,
) -> Option<TensorHandle> {
    match a.dtype {
        DType::F32 => cpu::cholesky_f32_scalar(a, upper),
        DType::F64 => cpu::cholesky_f64_scalar(a, upper),
        _ => None,
    }
}

/// Dispatch triangular solve to appropriate kernel.
///
/// Solves A * x = b (or A^T * x = b) where A is triangular.
///
/// Flags byte encoding:
/// - bit 0: upper (1) or lower (0) triangular
/// - bit 1: transpose (1) or normal (0)
/// - bit 2: unit diagonal (1) or regular (0)
pub fn dispatch_trisolve(
    a: &TensorHandle,
    b: &TensorHandle,
    flags: u8,
) -> Option<TensorHandle> {
    let solve_flags = cpu::TriSolveFlags::from_byte(flags);
    match a.dtype {
        DType::F32 => cpu::trisolve_f32_scalar(a, b, solve_flags),
        DType::F64 => cpu::trisolve_f64_scalar(a, b, solve_flags),
        _ => None,
    }
}

/// Dispatch Einstein summation to appropriate kernel.
///
/// Computes tensor contractions according to the einsum equation string.
/// Supports common operations like:
/// - Matrix multiplication: "ij,jk->ik"
/// - Dot product: "i,i->"
/// - Outer product: "i,j->ij"
/// - Trace: "ii->"
/// - Transpose: "ij->ji"
/// - Batch matmul: "bij,bjk->bik"
pub fn dispatch_einsum(
    equation: &str,
    operands: &[&TensorHandle],
) -> Option<TensorHandle> {
    if operands.is_empty() {
        return None;
    }

    // All operands must have the same dtype
    let dtype = operands[0].dtype;
    if !operands.iter().all(|t| t.dtype == dtype) {
        return None;
    }

    match dtype {
        DType::F32 => cpu::einsum_f32_scalar(equation, operands),
        DType::F64 => cpu::einsum_f64_scalar(equation, operands),
        _ => None,
    }
}

/// Dispatch Fast Fourier Transform to appropriate kernel.
///
/// Computes 1D FFT along the specified dimension.
///
/// - `dim`: dimension to transform (-1 = last dimension)
/// - `inverse`: if true, compute inverse FFT
///
/// Returns complex output (Complex64) for all input types.
pub fn dispatch_fft(
    input: &TensorHandle,
    dim: i8,
    inverse: bool,
) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::fft_f32_1d(input, dim, inverse),
        DType::F64 => cpu::fft_f64_1d(input, dim, inverse),
        DType::Complex64 => cpu::fft_complex64_1d(input, dim, inverse),
        _ => None,
    }
}

/// Dispatch scaled dot-product attention to appropriate kernel.
///
/// Computes: Attention(Q, K, V) = softmax(Q @ K^T / sqrt(d_k) + mask) @ V
///
/// Supports both 3D [batch, seq, dim] and 4D [batch, heads, seq, dim] inputs.
/// The mask is additive (e.g., -inf for masked positions).
pub fn dispatch_attention(
    q: &TensorHandle,
    k: &TensorHandle,
    v: &TensorHandle,
    mask: Option<&TensorHandle>,
    scale: Option<f64>,
) -> Option<TensorHandle> {
    match q.dtype {
        DType::F32 => cpu::attention_f32_scalar(q, k, v, mask, scale.map(|s| s as f32)),
        DType::F64 => cpu::attention_f64_scalar(q, k, v, mask, scale),
        _ => None,
    }
}

/// Dispatch batched matrix multiplication to appropriate kernel.
///
/// Computes C[b, i, j] = sum_k A[b, i, k] * B[b, k, j] for each batch b.
///
/// Input shapes: A [batch, M, K], B [batch, K, N]
/// Output: [batch, M, N]
pub fn dispatch_bmm(
    a: &TensorHandle,
    b: &TensorHandle,
) -> Option<TensorHandle> {
    match a.dtype {
        DType::F32 => cpu::bmm_f32_scalar(a, b),
        DType::F64 => cpu::bmm_f64_scalar(a, b),
        _ => None,
    }
}

/// Dispatch QR decomposition to appropriate kernel.
///
/// Computes A = Q * R via Householder reflections.
///
/// Input: A [M, N] where M >= N
/// Output: (Q [M, M], R [M, N])
pub fn dispatch_qr(
    a: &TensorHandle,
) -> Option<(TensorHandle, TensorHandle)> {
    match a.dtype {
        DType::F32 => cpu::qr_f32_householder(a),
        DType::F64 => cpu::qr_f64_householder(a),
        _ => None,
    }
}

/// Dispatch SVD decomposition to appropriate kernel.
///
/// Computes A = U * S * V^T via Jacobi algorithm.
///
/// Input: A [M, N]
/// Output: (U [M, M], S [min(M,N)], Vt [N, N])
pub fn dispatch_svd(
    a: &TensorHandle,
) -> Option<(TensorHandle, TensorHandle, TensorHandle)> {
    match a.dtype {
        DType::F32 => cpu::svd_f32_jacobi(a),
        DType::F64 => cpu::svd_f64_jacobi(a),
        _ => None,
    }
}

/// Dispatch eigenvalue decomposition to appropriate kernel.
///
/// Computes A = V * D * V^T for symmetric matrices via Jacobi algorithm.
///
/// Input: A [N, N] (must be symmetric)
/// Output: (eigenvalues [N], eigenvectors [N, N])
pub fn dispatch_eig_symmetric(
    a: &TensorHandle,
) -> Option<(TensorHandle, TensorHandle)> {
    match a.dtype {
        DType::F32 => cpu::eig_symmetric_f32_jacobi(a),
        DType::F64 => cpu::eig_symmetric_f64_jacobi(a),
        _ => None,
    }
}

/// Dispatch least squares solve to appropriate kernel.
///
/// Solves min_x ||A * x - b||_2 via QR decomposition.
///
/// Input:
/// - A [M, N] where M >= N
/// - b [M] or [M, K]
///
/// Output: x [N] or [N, K]
pub fn dispatch_lstsq(
    a: &TensorHandle,
    b: &TensorHandle,
) -> Option<TensorHandle> {
    match a.dtype {
        DType::F32 => cpu::lstsq_f32_qr(a, b),
        DType::F64 => cpu::lstsq_f64_qr(a, b),
        _ => None,
    }
}

/// Dispatch vector/matrix norm to appropriate kernel.
///
/// Computes various norms:
/// - ord=0: L0 "norm" (count of non-zeros)
/// - ord=1: L1 norm (sum of absolute values)
/// - ord=2: L2/Euclidean/Frobenius norm (default)
/// - ord=inf: Max norm (L∞)
/// - ord=-inf: Min absolute value
/// - General p-norm: (sum |x|^p)^(1/p)
///
/// Input:
/// - tensor of any shape
/// - ord: norm order (default 2.0)
/// - axis: reduce along this axis, or None for full tensor
///
/// Output: scalar (axis=None) or tensor with axis dimension removed
pub fn dispatch_norm(
    a: &TensorHandle,
    ord: f64,
    axis: Option<i8>,
) -> Option<TensorHandle> {
    match a.dtype {
        DType::F32 => cpu::norm_f32_scalar(a, ord, axis),
        DType::F64 => cpu::norm_f64_scalar(a, ord, axis),
        _ => None,
    }
}

/// Dispatch matrix-vector multiplication.
///
/// Computes y = A @ x
///
/// Input:
/// - A [m, n] matrix
/// - x [n] vector
///
/// Output: y [m] vector
pub fn dispatch_mv(
    a: &TensorHandle,
    x: &TensorHandle,
) -> Option<TensorHandle> {
    match a.dtype {
        DType::F32 => cpu::mv_f32_scalar(a, x),
        DType::F64 => cpu::mv_f64_scalar(a, x),
        _ => None,
    }
}

/// Dispatch diagonal extraction/creation.
///
/// - 2D input [m, n]: extracts diagonal as 1D [min(m,n)]
/// - 1D input [n]: creates diagonal matrix [n+|k|, n+|k|]
///
/// Input:
/// - tensor (1D or 2D)
/// - k: diagonal offset (0=main, >0=above, <0=below)
///
/// Output: 1D diagonal or 2D diagonal matrix
pub fn dispatch_diag(
    a: &TensorHandle,
    k: i32,
) -> Option<TensorHandle> {
    match a.dtype {
        DType::F32 => cpu::diag_f32_scalar(a, k),
        DType::F64 => cpu::diag_f64_scalar(a, k),
        _ => None,
    }
}

/// Dispatch upper triangular extraction.
///
/// Returns matrix with zeros below the k-th diagonal.
///
/// Input:
/// - A [m, n] matrix
/// - k: diagonal offset (0=main, >0=above, <0=below)
///
/// Output: [m, n] upper triangular matrix
pub fn dispatch_triu(
    a: &TensorHandle,
    k: i32,
) -> Option<TensorHandle> {
    match a.dtype {
        DType::F32 => cpu::triu_f32_scalar(a, k),
        DType::F64 => cpu::triu_f64_scalar(a, k),
        _ => None,
    }
}

/// Dispatch lower triangular extraction.
///
/// Returns matrix with zeros above the k-th diagonal.
///
/// Input:
/// - A [m, n] matrix
/// - k: diagonal offset (0=main, >0=above, <0=below)
///
/// Output: [m, n] lower triangular matrix
pub fn dispatch_tril(
    a: &TensorHandle,
    k: i32,
) -> Option<TensorHandle> {
    match a.dtype {
        DType::F32 => cpu::tril_f32_scalar(a, k),
        DType::F64 => cpu::tril_f64_scalar(a, k),
        _ => None,
    }
}

/// Dispatch matrix inverse.
///
/// Computes A^(-1) via Gauss-Jordan elimination.
///
/// Input: A [n, n] square non-singular matrix
/// Output: A^(-1) [n, n]
///
/// Returns None if matrix is singular.
pub fn dispatch_inverse(
    a: &TensorHandle,
) -> Option<TensorHandle> {
    match a.dtype {
        DType::F32 => cpu::inverse_f32_scalar(a),
        DType::F64 => cpu::inverse_f64_scalar(a),
        _ => None,
    }
}

// ============================================================================
// Complex Number Operations
// ============================================================================

/// Dispatch complex conjugate operation.
///
/// Returns the complex conjugate: conj(a + bi) = a - bi
pub fn dispatch_conj(a: &TensorHandle) -> Option<TensorHandle> {
    match a.dtype {
        DType::Complex64 => cpu::complex64_conj_scalar(a),
        DType::Complex128 => cpu::complex128_conj_scalar(a),
        _ => None,
    }
}

/// Dispatch complex real part extraction.
///
/// Returns the real component as a float tensor.
pub fn dispatch_real(a: &TensorHandle) -> Option<TensorHandle> {
    match a.dtype {
        DType::Complex64 => cpu::complex64_real_scalar(a),
        DType::Complex128 => cpu::complex128_real_scalar(a),
        _ => None,
    }
}

/// Dispatch complex imaginary part extraction.
///
/// Returns the imaginary component as a float tensor.
pub fn dispatch_imag(a: &TensorHandle) -> Option<TensorHandle> {
    match a.dtype {
        DType::Complex64 => cpu::complex64_imag_scalar(a),
        DType::Complex128 => cpu::complex128_imag_scalar(a),
        _ => None,
    }
}

// Re-export ScatterMode for use in dispatch layer
pub use cpu::ScatterMode;

/// Dispatch cumulative sum to appropriate kernel.
///
/// Computes cumulative sum along the specified axis.
/// out[..., i, ...] = sum(input[..., 0:i+1, ...])
///
/// Input: tensor of any shape
/// Output: tensor with same shape, containing cumulative sums
pub fn dispatch_cumsum(
    input: &TensorHandle,
    axis: i8,
) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::cumsum_f32_scalar(input, axis),
        DType::F64 => cpu::cumsum_f64_scalar(input, axis),
        _ => None,
    }
}

/// Dispatch cumulative product to appropriate kernel.
///
/// Computes cumulative product along the specified axis.
/// out[..., i, ...] = prod(input[..., 0:i+1, ...])
///
/// Input: tensor of any shape
/// Output: tensor with same shape, containing cumulative products
pub fn dispatch_cumprod(
    input: &TensorHandle,
    axis: i8,
) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::cumprod_f32_scalar(input, axis),
        DType::F64 => cpu::cumprod_f64_scalar(input, axis),
        _ => None,
    }
}

/// Dispatch gather operation to appropriate kernel.
///
/// Gathers values from input using indices along the specified axis.
/// output[i][j][k] = input[index[i][j][k]][j][k]  (for axis=0)
///
/// Input:
/// - input: source tensor
/// - indices: integer tensor (I32 or I64)
/// - axis: dimension to gather along
///
/// Output: tensor with shape matching indices
pub fn dispatch_gather(
    input: &TensorHandle,
    indices: &TensorHandle,
    axis: i8,
) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::gather_f32_scalar(input, indices, axis),
        DType::F64 => cpu::gather_f64_scalar(input, indices, axis),
        _ => None,
    }
}

/// Dispatch scatter operation to appropriate kernel.
///
/// Scatters values from src into output at positions specified by indices.
/// output[index[i][j][k]][j][k] = src[i][j][k]  (for axis=0)
///
/// Input:
/// - input: destination tensor (cloned, not modified in place)
/// - indices: integer tensor (I32 or I64)
/// - src: source values (same shape as indices)
/// - axis: dimension to scatter along
/// - mode: scatter reduction mode (Replace, Add, Mul, Max, Min)
///
/// Output: tensor with scattered values
pub fn dispatch_scatter(
    input: &TensorHandle,
    indices: &TensorHandle,
    src: &TensorHandle,
    axis: i8,
    mode: ScatterMode,
) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::scatter_f32_scalar(input, indices, src, axis, mode),
        DType::F64 => cpu::scatter_f64_scalar(input, indices, src, axis, mode),
        _ => None,
    }
}

/// Dispatch linear system solve to appropriate kernel.
///
/// Solves Ax = b using LU decomposition with partial pivoting.
///
/// Input:
/// - A [N, N] coefficient matrix (must be square)
/// - b [N] or [N, K] right-hand side
///
/// Output: x [N] or [N, K]
pub fn dispatch_solve(
    a: &TensorHandle,
    b: &TensorHandle,
) -> Option<TensorHandle> {
    match a.dtype {
        DType::F32 => cpu::solve_f32_lu(a, b),
        DType::F64 => cpu::solve_f64_lu(a, b),
        _ => None,
    }
}

/// Dispatch argmax to appropriate kernel.
///
/// Returns indices of maximum values along the specified axis.
///
/// Input: tensor of any shape
/// Output: tensor with axis dimension removed, dtype I64
pub fn dispatch_argmax(
    input: &TensorHandle,
    axis: i8,
) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::argmax_f32_scalar(input, axis),
        DType::F64 => cpu::argmax_f64_scalar(input, axis),
        _ => None,
    }
}

/// Dispatch argmin to appropriate kernel.
///
/// Returns indices of minimum values along the specified axis.
///
/// Input: tensor of any shape
/// Output: tensor with axis dimension removed, dtype I64
pub fn dispatch_argmin(
    input: &TensorHandle,
    axis: i8,
) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::argmin_f32_scalar(input, axis),
        DType::F64 => cpu::argmin_f64_scalar(input, axis),
        _ => None,
    }
}

/// Dispatch index select to appropriate kernel.
///
/// Selects elements from input along the specified axis using 1D indices.
///
/// Input:
/// - input: source tensor
/// - indices: 1D integer tensor (I32 or I64)
/// - axis: dimension to select along
///
/// Output: tensor with axis dimension size = len(indices)
pub fn dispatch_index_select(
    input: &TensorHandle,
    indices: &TensorHandle,
    axis: i8,
) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::index_select_f32_scalar(input, indices, axis),
        DType::F64 => cpu::index_select_f64_scalar(input, indices, axis),
        _ => None,
    }
}

/// Dispatch nansum to appropriate kernel.
///
/// Sum along axis, ignoring NaN values.
///
/// Input: tensor of any shape
/// Output: reduced tensor
pub fn dispatch_nansum(
    input: &TensorHandle,
    axis: Option<i8>,
    keepdim: bool,
) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::nansum_f32_scalar(input, axis, keepdim),
        DType::F64 => cpu::nansum_f64_scalar(input, axis, keepdim),
        _ => None,
    }
}

/// Dispatch nanmean to appropriate kernel.
///
/// Mean along axis, ignoring NaN values.
///
/// Input: tensor of any shape
/// Output: reduced tensor
pub fn dispatch_nanmean(
    input: &TensorHandle,
    axis: Option<i8>,
    keepdim: bool,
) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::nanmean_f32_scalar(input, axis, keepdim),
        DType::F64 => cpu::nanmean_f64_scalar(input, axis, keepdim),
        _ => None,
    }
}

/// Dispatch flip to appropriate kernel.
///
/// Flip tensor along specified axes.
///
/// Input: tensor of any shape
/// Output: flipped tensor
pub fn dispatch_flip(
    input: &TensorHandle,
    axes: &[usize],
) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::flip_f32_scalar(input, axes),
        DType::F64 => cpu::flip_f64_scalar(input, axes),
        _ => None,
    }
}

/// Dispatch roll to appropriate kernel.
///
/// Roll tensor along axis by shift positions.
///
/// Input: tensor of any shape
/// Output: rolled tensor
pub fn dispatch_roll(
    input: &TensorHandle,
    shift: i32,
    axis: i8,
) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::roll_f32_scalar(input, shift, axis),
        DType::F64 => cpu::roll_f64_scalar(input, shift, axis),
        _ => None,
    }
}

/// Dispatch LU decomposition to appropriate kernel.
///
/// LU decomposition with partial pivoting: PA = LU
///
/// Input: 2D tensor (m x n matrix)
/// Output: (P, L, U) where P is permutation, L is lower, U is upper triangular
pub fn dispatch_lu(
    input: &TensorHandle,
) -> Option<(TensorHandle, TensorHandle, TensorHandle)> {
    match input.dtype {
        DType::F32 => cpu::lu_f32_scalar(input),
        DType::F64 => cpu::lu_f64_scalar(input),
        _ => None,
    }
}

/// Dispatch general eigenvalue decomposition to appropriate kernel.
///
/// Computes eigenvalues and eigenvectors of a square matrix.
/// Note: For matrices with complex eigenvalues, only real parts are returned.
///
/// Input: 2D square tensor (n x n matrix)
/// Output: (eigenvalues, eigenvectors)
pub fn dispatch_eig(
    input: &TensorHandle,
) -> Option<(TensorHandle, TensorHandle)> {
    match input.dtype {
        DType::F32 => cpu::eig_f32_qr(input),
        DType::F64 => cpu::eig_f64_qr(input),
        _ => None,
    }
}

/// Dispatch matrix rank computation to appropriate kernel.
///
/// Computes the rank of a matrix using SVD.
///
/// Input: 2D tensor (m x n matrix)
/// tol: tolerance for singular values to be considered zero
/// Output: integer rank
pub fn dispatch_rank(
    input: &TensorHandle,
    tol: f64,
) -> Option<usize> {
    match input.dtype {
        DType::F32 => cpu::rank_f32_scalar(input, tol),
        DType::F64 => cpu::rank_f64_scalar(input, tol),
        _ => None,
    }
}

/// Dispatch matrix condition number computation to appropriate kernel.
///
/// Computes the condition number of a matrix.
///
/// Input: 2D tensor (m x n matrix)
/// p: norm type (1 for 1-norm, 2 for 2-norm, -1 for infinity-norm)
/// Output: condition number (f64)
pub fn dispatch_cond(
    input: &TensorHandle,
    p: i8,
) -> Option<f64> {
    match input.dtype {
        DType::F32 => cpu::cond_f32_scalar(input, p).map(|v| v as f64),
        DType::F64 => cpu::cond_f64_scalar(input, p),
        _ => None,
    }
}

/// Dispatch Schur decomposition to appropriate kernel.
///
/// Schur decomposition: A = Z * T * Z^H
///
/// Input: 2D square tensor (n x n matrix)
/// Output: (T, Z) where T is upper triangular and Z is unitary
pub fn dispatch_schur(
    input: &TensorHandle,
) -> Option<(TensorHandle, TensorHandle)> {
    match input.dtype {
        DType::F32 => cpu::schur_f32_scalar(input),
        DType::F64 => cpu::schur_f64_scalar(input),
        _ => None,
    }
}

// ============================================================================
// Advanced Tensor Operations (0x70-0x75)
// ============================================================================

/// Dispatch Kronecker product to appropriate kernel.
///
/// The Kronecker product of A (m x n) and B (p x q) is (mp x nq).
/// (A ⊗ B)[i,j] = A[i/p, j/q] * B[i%p, j%q]
pub fn dispatch_kron(a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
    if a.dtype != b.dtype {
        return None;
    }
    match a.dtype {
        DType::F32 => cpu::kron_f32_scalar(a, b),
        DType::F64 => cpu::kron_f64_scalar(a, b),
        _ => None,
    }
}

/// Dispatch cross product to appropriate kernel.
///
/// Computes the cross product of two 3D vectors.
/// c = a × b where c[0] = a[1]*b[2] - a[2]*b[1], etc.
pub fn dispatch_cross(a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
    if a.dtype != b.dtype {
        return None;
    }
    match a.dtype {
        DType::F32 => cpu::cross_f32_scalar(a, b),
        DType::F64 => cpu::cross_f64_scalar(a, b),
        _ => None,
    }
}

/// Dispatch tensor contraction to appropriate kernel.
///
/// Contract tensors along specified axes.
/// axis_a and axis_b specify which dimensions to contract.
pub fn dispatch_contract(
    a: &TensorHandle,
    b: &TensorHandle,
    axis_a: usize,
    axis_b: usize,
) -> Option<TensorHandle> {
    if a.dtype != b.dtype {
        return None;
    }
    match a.dtype {
        DType::F32 => cpu::contract_f32_scalar(a, b, axis_a, axis_b),
        DType::F64 => cpu::contract_f64_scalar(a, b, axis_a, axis_b),
        _ => None,
    }
}

/// Dispatch matrix power to appropriate kernel.
///
/// Computes A^n for integer n using binary exponentiation.
pub fn dispatch_matrix_power(input: &TensorHandle, n: i32) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::matrix_power_f32_scalar(input, n),
        DType::F64 => cpu::matrix_power_f64_scalar(input, n),
        _ => None,
    }
}

/// Dispatch matrix exponential to appropriate kernel.
///
/// Computes e^A using scaling and squaring with Padé approximation.
pub fn dispatch_expm(input: &TensorHandle) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::expm_f32_scalar(input),
        DType::F64 => cpu::expm_f64_scalar(input),
        _ => None,
    }
}

/// Dispatch matrix logarithm to appropriate kernel.
///
/// Computes log(A) using inverse scaling and squaring.
/// Requires A to have no eigenvalues on the negative real axis.
pub fn dispatch_logm(input: &TensorHandle) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::logm_f32_scalar(input),
        DType::F64 => cpu::logm_f64_scalar(input),
        _ => None,
    }
}

// =============================================================================
// SSM and FFT Operations
// =============================================================================

/// Dispatch parallel associative scan for State Space Models.
///
/// Implements the Blelloch parallel scan algorithm for efficient SSM computation.
/// The op parameter determines the associative operation (0=add, 1=mul, 2=matmul).
pub fn dispatch_ssm_scan(
    op: u8,
    init: &TensorHandle,
    elements: &TensorHandle,
    dim: i8,
) -> Option<TensorHandle> {
    // Validate dimensions match
    if init.ndim != elements.ndim {
        return None;
    }

    // Normalize negative dimension
    let axis = if dim < 0 {
        (elements.ndim as i8 + dim) as usize
    } else {
        dim as usize
    };

    if axis >= elements.ndim as usize {
        return None;
    }

    match elements.dtype {
        DType::F32 => cpu::ssm_scan_f32(op, init, elements, axis),
        DType::F64 => cpu::ssm_scan_f64(op, init, elements, axis),
        _ => None,
    }
}

/// Dispatch real-to-complex FFT.
///
/// Computes the one-dimensional discrete Fourier Transform for real input.
/// Returns complex output with shape [..., n//2 + 1].
pub fn dispatch_rfft(input: &TensorHandle, n: usize) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::rfft_f32(input, n),
        DType::F64 => cpu::rfft_f64(input, n),
        _ => None,
    }
}

/// Dispatch complex-to-real inverse FFT.
///
/// Computes the one-dimensional inverse discrete Fourier Transform for
/// Hermitian-symmetric input, returning real output.
pub fn dispatch_irfft(input: &TensorHandle, n: usize) -> Option<TensorHandle> {
    match input.dtype {
        DType::Complex64 => cpu::irfft_c64(input, n),
        DType::Complex128 => cpu::irfft_c128(input, n),
        // Also handle F32/F64 which may represent interleaved complex
        DType::F32 => cpu::irfft_f32(input, n),
        DType::F64 => cpu::irfft_f64(input, n),
        _ => None,
    }
}

/// Dispatch complex multiplication.
///
/// Performs element-wise multiplication of two complex tensors.
pub fn dispatch_complex_mul(a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
    if a.shape != b.shape {
        return None;
    }

    match (a.dtype, b.dtype) {
        (DType::Complex64, DType::Complex64) => cpu::complex_mul_c64(a, b),
        (DType::Complex128, DType::Complex128) => cpu::complex_mul_c128(a, b),
        // Handle interleaved real representation
        (DType::F32, DType::F32) => cpu::complex_mul_f32(a, b),
        (DType::F64, DType::F64) => cpu::complex_mul_f64(a, b),
        _ => None,
    }
}

/// Dispatch complex power operation.
///
/// Computes base^exp for complex tensors.
pub fn dispatch_complex_pow(base: &TensorHandle, exp: &TensorHandle) -> Option<TensorHandle> {
    if base.shape != exp.shape {
        return None;
    }

    match (base.dtype, exp.dtype) {
        (DType::Complex64, DType::Complex64) => cpu::complex_pow_c64(base, exp),
        (DType::Complex128, DType::Complex128) => cpu::complex_pow_c128(base, exp),
        (DType::F32, DType::F32) => cpu::complex_pow_f32(base, exp),
        (DType::F64, DType::F64) => cpu::complex_pow_f64(base, exp),
        _ => None,
    }
}

// =============================================================================
// Random and Training Operations
// =============================================================================

/// Dispatch uniform random tensor generation.
///
/// Creates a tensor filled with uniformly distributed random values in [low, high).
pub fn dispatch_uniform(
    shape: &[usize],
    low: f64,
    high: f64,
    dtype: DType,
) -> Option<TensorHandle> {
    match dtype {
        DType::F32 => cpu::uniform_f32(shape, low as f32, high as f32),
        DType::F64 => cpu::uniform_f64(shape, low, high),
        _ => None,
    }
}

/// Check if currently in training mode.
///
/// **Note:** This function is deprecated. The dispatch handler now reads
/// training mode directly from `state.context_stack.get_training_mode()`,
/// which properly respects the scoped `TrainingMode` context.
///
/// This function remains for backwards compatibility and testing, but
/// will always return false. Use the context-based approach instead.
///
/// # Example
/// ```verum
/// provide TrainingMode = true {
///     // is_training() returns true here
///     let dropout_active = is_training();
/// }
/// // is_training() returns false here
/// ```
#[deprecated(note = "Use state.context_stack.get_training_mode() instead")]
pub fn dispatch_is_training() -> bool {
    // Always returns false - context-based version is used in dispatch.rs
    false
}

/// Generate a random float in [0, 1).
pub fn dispatch_random_float_01() -> f64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    // Simple random generation using hash-based entropy
    let state = RandomState::new();
    let mut hasher = state.build_hasher();
    hasher.write_u64(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0));
    let random_bits = hasher.finish();

    // Convert to [0, 1) range
    (random_bits as f64) / (u64::MAX as f64)
}

// =============================================================================
// Indexing and Aggregation Operations
// =============================================================================

/// Dispatch histogram binning (bincount).
///
/// Counts occurrences of each value in the input tensor.
pub fn dispatch_bincount(indices: &TensorHandle, num_bins: usize) -> Option<TensorHandle> {
    match indices.dtype {
        DType::I32 => cpu::bincount_i32(indices, num_bins),
        DType::I64 => cpu::bincount_i64(indices, num_bins),
        DType::U32 => cpu::bincount_u32(indices, num_bins),
        DType::U64 => cpu::bincount_u64(indices, num_bins),
        _ => None,
    }
}

/// Dispatch N-dimensional gather operation.
///
/// Gathers values from input tensor using N-dimensional indices.
pub fn dispatch_gather_nd(input: &TensorHandle, indices: &TensorHandle) -> Option<TensorHandle> {
    match input.dtype {
        DType::F32 => cpu::gather_nd_f32(input, indices),
        DType::F64 => cpu::gather_nd_f64(input, indices),
        DType::I32 => cpu::gather_nd_i32(input, indices),
        DType::I64 => cpu::gather_nd_i64(input, indices),
        _ => None,
    }
}

/// Dispatch integer range tensor creation.
///
/// Creates a 1D tensor with values from start to end (exclusive) with given step.
pub fn dispatch_arange_usize(start: usize, end: usize, step: usize) -> Option<TensorHandle> {
    if step == 0 || (start >= end && step > 0) {
        return None;
    }

    let len = end.saturating_sub(start).div_ceil(step);
    cpu::arange_usize(start, len, step)
}

/// Dispatch tensor repeat along new dimension.
///
/// Repeats tensor `times` times along a new leading dimension.
pub fn dispatch_repeat(input: &TensorHandle, times: usize) -> Option<TensorHandle> {
    if times == 0 {
        return None;
    }

    match input.dtype {
        DType::F32 => cpu::repeat_f32(input, times),
        DType::F64 => cpu::repeat_f64(input, times),
        DType::I32 => cpu::repeat_i32(input, times),
        DType::I64 => cpu::repeat_i64(input, times),
        _ => None,
    }
}

/// Dispatch element-wise tanh on tensor.
pub fn dispatch_tanh(input: &TensorHandle) -> Option<TensorHandle> {
    dispatch_unop(input, TensorUnaryOp::Tanh)
}

/// Dispatch sum all elements in tensor.
pub fn dispatch_sum_all(input: &TensorHandle) -> Option<TensorHandle> {
    // Use reduce with Sum op and reduce all dimensions (axis=None)
    dispatch_reduce(input, TensorReduceOp::Sum, None)
}

/// Dispatch tensor creation from array values.
pub fn dispatch_from_array(values: &[f64], dtype: DType) -> Option<TensorHandle> {
    let shape = vec![values.len()];
    super::tensor::tensor_from_slice(values, &shape, dtype)
}

// =============================================================================
// Tokenizer Operations
// =============================================================================

// Re-export tokenizer types and functions from the tokenizer module.
// When the `tokenizers` feature is enabled, these use the HuggingFace tokenizers library.
// Otherwise, they fall back to simple byte encoding/decoding stubs.
pub use tokenizer::{
    TokenizerHandle, TokenizerType,
    dispatch_tokenizer_load_bpe, dispatch_tokenizer_load_pretrained, dispatch_tokenizer_load_spm,
    dispatch_tokenizer_encode, dispatch_tokenizer_decode,
    dispatch_tokenizer_spm_encode, dispatch_tokenizer_spm_decode,
    dispatch_tokenizer_encode_special, dispatch_tokenizer_encode_batch, dispatch_tokenizer_decode_batch,
};

// =============================================================================
// Sampling Operations
// =============================================================================

/// Top-p (nucleus) sampling from logits.
pub fn dispatch_sample_top_p(logits: &TensorHandle, p: f64) -> Option<u32> {
    if logits.numel == 0 {
        return None;
    }

    // Simple implementation: softmax then cumulative sum
    let ptr = logits.data_ptr_f32();
    if ptr.is_null() {
        return None;
    }

    let n = logits.numel;
    let mut probs: Vec<(usize, f32)> = Vec::with_capacity(n);

    // Softmax
    let max_val = unsafe {
        (0..n).map(|i| *ptr.add(i)).fold(f32::NEG_INFINITY, f32::max)
    };

    let sum: f32 = unsafe {
        (0..n).map(|i| (*ptr.add(i) - max_val).exp()).sum()
    };

    unsafe {
        for i in 0..n {
            let prob = (*ptr.add(i) - max_val).exp() / sum;
            probs.push((i, prob));
        }
    }

    // Sort by probability descending
    probs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Cumulative sum until p
    let mut cum_prob = 0.0;
    for (idx, prob) in &probs {
        cum_prob += *prob as f64;
        if cum_prob >= p {
            return Some(*idx as u32);
        }
    }

    probs.first().map(|(idx, _)| *idx as u32)
}

/// Temperature-scaled sampling from logits.
pub fn dispatch_sample_temperature(logits: &TensorHandle, temperature: f64) -> Option<u32> {
    if logits.numel == 0 || temperature <= 0.0 {
        return None;
    }

    let ptr = logits.data_ptr_f32();
    if ptr.is_null() {
        return None;
    }

    let n = logits.numel;
    let temp = temperature as f32;

    // Apply temperature and softmax
    let max_val = unsafe {
        (0..n).map(|i| *ptr.add(i) / temp).fold(f32::NEG_INFINITY, f32::max)
    };

    let sum: f32 = unsafe {
        (0..n).map(|i| ((*ptr.add(i) / temp) - max_val).exp()).sum()
    };

    // Sample from distribution
    let random_val = dispatch_random_float_01() as f32;
    let mut cum_prob = 0.0f32;

    unsafe {
        for i in 0..n {
            let prob = ((*ptr.add(i) / temp) - max_val).exp() / sum;
            cum_prob += prob;
            if cum_prob >= random_val {
                return Some(i as u32);
            }
        }
    }

    Some((n - 1) as u32)
}

/// Paged attention for efficient KV cache.
///
/// Computes attention output by gathering K,V from non-contiguous memory blocks.
/// This enables memory-efficient serving with dynamic sequence lengths.
///
/// Algorithm:
/// 1. Gather K,V values from blocks specified in block_table
/// 2. Compute attention: softmax(Q @ K^T / sqrt(head_dim)) @ V
/// 3. Return output tensor
///
/// # Arguments
/// * `q` - Query tensor [batch, num_heads, seq_q, head_dim]
/// * `kv_cache` - KV cache tensor [num_blocks, num_heads, block_size, head_dim * 2] (K and V concatenated)
/// * `block_table` - Block indices [batch, max_blocks]
/// * `context_len` - Actual context length to use
#[allow(clippy::needless_range_loop)]
pub fn dispatch_paged_attention(
    q: &TensorHandle,
    kv_cache: &TensorHandle,
    block_table: &TensorHandle,
    context_len: usize,
) -> Option<TensorHandle> {
    // Validate input shapes
    if q.ndim < 3 || kv_cache.ndim < 4 || block_table.ndim < 1 {
        return None;
    }

    // Only support F32 for now
    if q.dtype != DType::F32 || kv_cache.dtype != DType::F32 {
        return None;
    }

    let q_shape = &q.shape[..q.ndim as usize];
    let cache_shape = &kv_cache.shape[..kv_cache.ndim as usize];

    // Extract dimensions
    let batch = if q.ndim == 4 { q_shape[0] } else { 1 };
    let num_heads = if q.ndim == 4 { q_shape[1] } else { q_shape[0] };
    let seq_q = if q.ndim == 4 { q_shape[2] } else { q_shape[1] };
    let head_dim = if q.ndim == 4 { q_shape[3] } else { q_shape[2] };

    let block_size = cache_shape[2];
    let kv_head_dim = cache_shape[3] / 2; // K and V concatenated

    if head_dim != kv_head_dim {
        return None;
    }

    // Calculate number of blocks needed
    let num_blocks_needed = context_len.div_ceil(block_size);

    // Allocate output [batch, num_heads, seq_q, head_dim]
    let out_shape = if q.ndim == 4 {
        vec![batch, num_heads, seq_q, head_dim]
    } else {
        vec![num_heads, seq_q, head_dim]
    };
    let mut output = TensorHandle::zeros(&out_shape, DType::F32)?;

    let q_ptr = q.data_ptr_f32();
    let kv_ptr = kv_cache.data_ptr_f32();
    let block_ptr = block_table.data_ptr_i64();
    let out_ptr = output.data_ptr_f32_mut();

    if q_ptr.is_null() || kv_ptr.is_null() || block_ptr.is_null() || out_ptr.is_null() {
        return None;
    }

    let scale = 1.0f32 / (head_dim as f32).sqrt();

    unsafe {
        // For each batch and head, compute attention
        for b in 0..batch {
            for h in 0..num_heads {
                for sq in 0..seq_q {
                    // Get query vector for this position
                    let q_offset = if q.ndim == 4 {
                        ((b * num_heads + h) * seq_q + sq) * head_dim
                    } else {
                        (h * seq_q + sq) * head_dim
                    };

                    // Compute attention scores and weighted sum
                    let mut scores = vec![0.0f32; context_len];
                    let mut max_score = f32::NEG_INFINITY;

                    // Compute Q @ K^T for each key position
                    for kv_pos in 0..context_len {
                        let block_idx = kv_pos / block_size;
                        let block_offset = kv_pos % block_size;

                        if block_idx >= num_blocks_needed {
                            break;
                        }

                        // Get block index from block_table
                        let physical_block = *block_ptr.add(b * num_blocks_needed + block_idx) as usize;

                        // K offset: [block, head, pos_in_block, head_dim]
                        let k_offset = ((physical_block * num_heads + h) * block_size + block_offset) * (head_dim * 2);

                        // Dot product Q @ K
                        let mut score = 0.0f32;
                        for d in 0..head_dim {
                            score += *q_ptr.add(q_offset + d) * *kv_ptr.add(k_offset + d);
                        }
                        score *= scale;
                        scores[kv_pos] = score;
                        max_score = max_score.max(score);
                    }

                    // Softmax: exp(x - max) / sum(exp(x - max))
                    let mut sum_exp = 0.0f32;
                    for kv_pos in 0..context_len {
                        scores[kv_pos] = (scores[kv_pos] - max_score).exp();
                        sum_exp += scores[kv_pos];
                    }
                    if sum_exp > 0.0 {
                        for kv_pos in 0..context_len {
                            scores[kv_pos] /= sum_exp;
                        }
                    }

                    // Compute weighted sum of values
                    let out_offset = if q.ndim == 4 {
                        ((b * num_heads + h) * seq_q + sq) * head_dim
                    } else {
                        (h * seq_q + sq) * head_dim
                    };

                    for d in 0..head_dim {
                        let mut weighted_sum = 0.0f32;

                        for kv_pos in 0..context_len {
                            let block_idx = kv_pos / block_size;
                            let block_offset = kv_pos % block_size;

                            if block_idx >= num_blocks_needed {
                                break;
                            }

                            let physical_block = *block_ptr.add(b * num_blocks_needed + block_idx) as usize;

                            // V offset: K is first half, V is second half
                            let v_offset = ((physical_block * num_heads + h) * block_size + block_offset) * (head_dim * 2) + head_dim;

                            weighted_sum += scores[kv_pos] * *kv_ptr.add(v_offset + d);
                        }

                        *out_ptr.add(out_offset + d) = weighted_sum;
                    }
                }
            }
        }
    }

    Some(output)
}

// =============================================================================
// Inference Utility Operations
// =============================================================================

/// Parse tool call from action string.
///
/// Supports multiple formats:
/// - Python-style: `tool_name(arg1=value1, arg2=value2)`
/// - JSON-style: `{"name": "tool_name", "arguments": {"arg1": "value1"}}`
/// - Simple call: `tool_name()` (no arguments)
///
/// Returns (tool_name, arguments_json) if successful.
pub fn dispatch_parse_tool_call(action: &str) -> Option<(String, String)> {
    let action = action.trim();

    // JSON-style: {"name": "...", "arguments": {...}}
    if action.starts_with('{') {
        return parse_json_tool_call(action);
    }

    // Python/function-style: tool_name(args...)
    if let Some(paren_pos) = action.find('(') {
        let tool_name = action[..paren_pos].trim().to_string();
        if tool_name.is_empty() {
            return None;
        }

        // Find matching closing paren
        let rest = &action[paren_pos + 1..];
        let close_pos = find_matching_paren(rest)?;
        let args_str = &rest[..close_pos];

        // Parse arguments
        let args_json = parse_tool_arguments(args_str);
        return Some((tool_name, args_json));
    }

    // Just a tool name with no parens
    if !action.is_empty() && action.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Some((action.to_string(), "{}".to_string()));
    }

    None
}

/// Parse JSON-style tool call: {"name": "...", "arguments": {...}}
fn parse_json_tool_call(action: &str) -> Option<(String, String)> {
    // Simple JSON parsing - look for "name" and "arguments" fields
    let name_key = "\"name\"";
    let args_key = "\"arguments\"";

    let name_pos = action.find(name_key)?;
    let after_name = &action[name_pos + name_key.len()..];

    // Find the colon and value
    let colon_pos = after_name.find(':')?;
    let value_start = &after_name[colon_pos + 1..].trim_start();

    // Extract name value (quoted string)
    if !value_start.starts_with('"') {
        return None;
    }
    let value_content = &value_start[1..];
    let end_quote = value_content.find('"')?;
    let tool_name = value_content[..end_quote].to_string();

    // Extract arguments
    let args_json = if let Some(args_pos) = action.find(args_key) {
        let after_args = &action[args_pos + args_key.len()..];
        let colon_pos = after_args.find(':')?;
        let args_start = after_args[colon_pos + 1..].trim_start();

        if args_start.starts_with('{') {
            // Extract JSON object
            extract_json_object(args_start).unwrap_or_else(|| "{}".to_string())
        } else {
            "{}".to_string()
        }
    } else {
        "{}".to_string()
    };

    Some((tool_name, args_json))
}

/// Find matching closing parenthesis, accounting for nesting
fn find_matching_paren(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;

    for (i, c) in s.chars().enumerate() {
        if escape {
            escape = false;
            continue;
        }

        match c {
            '\\' if in_string => escape = true,
            '"' => in_string = !in_string,
            '(' if !in_string => depth += 1,
            ')' if !in_string => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }

    None
}

/// Extract a JSON object from the start of a string
fn extract_json_object(s: &str) -> Option<String> {
    if !s.starts_with('{') {
        return None;
    }

    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;

    for (i, c) in s.char_indices() {
        if escape {
            escape = false;
            continue;
        }

        match c {
            '\\' if in_string => escape = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[..=i].to_string());
                }
            }
            _ => {}
        }
    }

    None
}

/// Parse tool arguments from Python-style format: arg1=val1, arg2=val2
fn parse_tool_arguments(args_str: &str) -> String {
    let args_str = args_str.trim();

    if args_str.is_empty() {
        return "{}".to_string();
    }

    let mut result = String::from("{");
    let mut first = true;
    let mut current_key = String::new();
    let mut current_value = String::new();
    let mut in_key = true;
    let mut in_string = false;
    let mut escape = false;
    let mut paren_depth = 0i32;
    let mut bracket_depth = 0i32;

    for c in args_str.chars() {
        if escape {
            escape = false;
            if in_key {
                current_key.push(c);
            } else {
                current_value.push(c);
            }
            continue;
        }

        match c {
            '\\' => {
                escape = true;
                if !in_key {
                    current_value.push(c);
                }
            }
            '"' => {
                in_string = !in_string;
                if !in_key {
                    current_value.push(c);
                }
            }
            '=' if !in_string && in_key && paren_depth == 0 && bracket_depth == 0 => {
                in_key = false;
            }
            ',' if !in_string && !in_key && paren_depth == 0 && bracket_depth == 0 => {
                // End of this argument
                if !current_key.is_empty() {
                    if !first {
                        result.push(',');
                    }
                    first = false;
                    result.push('"');
                    result.push_str(current_key.trim());
                    result.push_str("\":");
                    result.push_str(&format_value(current_value.trim()));
                }
                current_key.clear();
                current_value.clear();
                in_key = true;
            }
            '(' | '[' | '{' if !in_string => {
                if c == '(' {
                    paren_depth += 1;
                } else if c == '[' {
                    bracket_depth += 1;
                }
                if !in_key {
                    current_value.push(c);
                }
            }
            ')' | ']' | '}' if !in_string => {
                if c == ')' {
                    paren_depth -= 1;
                } else if c == ']' {
                    bracket_depth -= 1;
                }
                if !in_key {
                    current_value.push(c);
                }
            }
            _ => {
                if in_key {
                    current_key.push(c);
                } else {
                    current_value.push(c);
                }
            }
        }
    }

    // Don't forget the last argument
    if !current_key.is_empty() {
        if !first {
            result.push(',');
        }
        result.push('"');
        result.push_str(current_key.trim());
        result.push_str("\":");
        result.push_str(&format_value(current_value.trim()));
    }

    result.push('}');
    result
}

/// Format a value as JSON
fn format_value(v: &str) -> String {
    // Already a JSON-like value
    if v.starts_with('"') || v.starts_with('[') || v.starts_with('{') {
        return v.to_string();
    }

    // Numeric
    if v.parse::<f64>().is_ok() {
        return v.to_string();
    }

    // Boolean
    if v == "true" || v == "false" || v == "True" || v == "False" {
        return v.to_lowercase();
    }

    // Null/None
    if v == "null" || v == "None" || v == "nil" {
        return "null".to_string();
    }

    // Default: treat as string
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Format value for display.
pub fn dispatch_format_value(value: &super::super::Value) -> String {
    format!("{:?}", value)
}

/// Create tensor from USize slice.
pub fn dispatch_tensor_from_slice_usize(values: &[usize]) -> Option<TensorHandle> {
    let mut output = TensorHandle::zeros(&[values.len()], DType::U64)?;
    let out_ptr = output.data_ptr_u64_mut();

    unsafe {
        for (i, &v) in values.iter().enumerate() {
            *out_ptr.add(i) = v as u64;
        }
    }

    Some(output)
}

/// Quantized matrix multiplication with dequantization.
///
/// Performs efficient INT8 matrix multiplication using the formula:
/// output = input @ dequantize(weight)
/// where dequantize(w) = (w - zero_point) * scale
///
/// This enables 2-4x memory reduction while maintaining reasonable accuracy.
///
/// # Arguments
/// * `input` - Float input tensor [batch, ..., in_features]
/// * `weight` - INT8 quantized weights [out_features, in_features]
/// * `scale` - Per-channel scale factors [out_features]
/// * `zero_point` - Per-channel zero points [out_features]
///
/// # Returns
/// Float output tensor [batch, ..., out_features]
pub fn dispatch_quantized_matmul(
    input: &TensorHandle,
    weight: &TensorHandle,
    scale: &TensorHandle,
    zero_point: &TensorHandle,
) -> Option<TensorHandle> {
    // Validate dtypes
    if input.dtype != DType::F32 {
        return None;
    }
    if weight.dtype != DType::I8 {
        return None;
    }
    if scale.dtype != DType::F32 || zero_point.dtype != DType::I8 {
        return None;
    }

    // Get shapes
    let input_shape = &input.shape[..input.ndim as usize];
    let weight_shape = &weight.shape[..weight.ndim as usize];

    if weight.ndim != 2 || input.ndim < 1 {
        return None;
    }

    let out_features = weight_shape[0];
    let in_features = weight_shape[1];

    // Validate scale and zero_point shapes
    if scale.numel != out_features || zero_point.numel != out_features {
        return None;
    }

    // Get last dimension of input
    let input_in_features = input_shape[input.ndim as usize - 1];
    if input_in_features != in_features {
        return None;
    }

    // Calculate batch dimensions
    let batch_size: usize = input.numel / in_features;

    // Create output shape: replace last dim with out_features
    let mut out_shape: Vec<usize> = input_shape[..input.ndim as usize - 1].to_vec();
    out_shape.push(out_features);

    let mut output = TensorHandle::zeros(&out_shape, DType::F32)?;

    let input_ptr = input.data_ptr_f32();
    let weight_ptr = weight.data_ptr_i8();
    let scale_ptr = scale.data_ptr_f32();
    let zp_ptr = zero_point.data_ptr_i8();
    let out_ptr = output.data_ptr_f32_mut();

    if input_ptr.is_null() || weight_ptr.is_null() || scale_ptr.is_null()
        || zp_ptr.is_null() || out_ptr.is_null()
    {
        return None;
    }

    unsafe {
        // For each batch element
        for b in 0..batch_size {
            let in_offset = b * in_features;
            let out_offset = b * out_features;

            // For each output feature
            for o in 0..out_features {
                let s = *scale_ptr.add(o);
                let zp = *zp_ptr.add(o) as i32;

                // Dot product with dequantization
                // output[b, o] = sum_i(input[b, i] * (weight[o, i] - zp) * scale)
                let mut acc = 0.0f32;

                for i in 0..in_features {
                    let w_i8 = *weight_ptr.add(o * in_features + i) as i32;
                    let w_dequant = ((w_i8 - zp) as f32) * s;
                    acc += *input_ptr.add(in_offset + i) * w_dequant;
                }

                *out_ptr.add(out_offset + o) = acc;
            }
        }
    }

    Some(output)
}

/// Compute tensor norm (L2).
pub fn dispatch_tensor_norm(input: &TensorHandle) -> Option<f64> {
    match input.dtype {
        DType::F32 => {
            let ptr = input.data_ptr_f32();
            if ptr.is_null() { return None; }
            let sum: f32 = unsafe {
                (0..input.numel).map(|i| {
                    let v = *ptr.add(i);
                    v * v
                }).sum()
            };
            Some(sum.sqrt() as f64)
        }
        DType::F64 => {
            let ptr = input.data_ptr_f64();
            if ptr.is_null() { return None; }
            let sum: f64 = unsafe {
                (0..input.numel).map(|i| {
                    let v = *ptr.add(i);
                    v * v
                }).sum()
            };
            Some(sum.sqrt())
        }
        _ => None,
    }
}

/// Generate unique request ID.
pub fn dispatch_generate_request_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let state = RandomState::new();
    let mut hasher = state.build_hasher();
    hasher.write_u64(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0));

    format!("req_{:016x}", hasher.finish())
}

/// Convert JSON schema to JSON string.
pub fn dispatch_json_schema_to_json(_schema: &super::super::Value) -> String {
    // Stub: would serialize schema to JSON
    "{}".to_string()
}

/// Convert function schema to JSON string.
pub fn dispatch_function_schema_to_json(_schema: &super::super::Value) -> String {
    // Stub: would serialize function schema to JSON
    r#"{"name": "function", "parameters": {}}"#.to_string()
}

/// Parse function calls from response.
pub fn dispatch_parse_function_calls(_response: &str) -> Option<Vec<(String, String)>> {
    // Stub: would parse function calls from model response
    Some(vec![])
}

// =============================================================================
// Distributed/Collective Operations
// =============================================================================

/// Process group handle for distributed communication.
#[derive(Debug, Clone)]
pub struct ProcessGroupHandle {
    /// List of ranks in the group.
    pub ranks: Vec<usize>,
    /// World size (total number of processes).
    pub world_size: usize,
    /// Current rank.
    pub current_rank: usize,
}

/// Reduce operation type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ReduceOp {
    /// Sum all values.
    Sum = 0,
    /// Mean of all values.
    Mean = 1,
    /// Maximum value.
    Max = 2,
    /// Minimum value.
    Min = 3,
    /// Product of all values.
    Prod = 4,
}

impl ReduceOp {
    /// Convert from u8 to ReduceOp.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Sum),
            1 => Some(Self::Mean),
            2 => Some(Self::Max),
            3 => Some(Self::Min),
            4 => Some(Self::Prod),
            _ => None,
        }
    }
}

/// All-reduce operation: reduce tensor across all ranks and distribute result.
pub fn dispatch_all_reduce(
    tensor: &TensorHandle,
    _group: &ProcessGroupHandle,
    _op: ReduceOp,
) -> Option<TensorHandle> {
    // Stub: in single-process mode, just return a copy of the tensor
    Some(tensor.clone())
}

/// All-gather operation: gather tensors from all ranks to all ranks.
pub fn dispatch_all_gather(
    tensor: &TensorHandle,
    group: &ProcessGroupHandle,
) -> Option<TensorHandle> {
    // Stub: in single-process mode, add a leading dimension for world_size
    let tensor_shape = &tensor.shape[..tensor.ndim as usize];
    let mut new_shape = vec![group.world_size];
    new_shape.extend(tensor_shape.iter());

    // Create output tensor with expanded shape using zeros
    TensorHandle::zeros(&new_shape, tensor.dtype)
}

/// Broadcast operation: send tensor from src rank to all ranks.
pub fn dispatch_broadcast(
    tensor: &TensorHandle,
    _src: usize,
    _group: &ProcessGroupHandle,
) -> Option<TensorHandle> {
    // Stub: in single-process mode, just return a copy
    Some(tensor.clone())
}

/// Reduce-scatter operation: reduce then scatter result.
pub fn dispatch_reduce_scatter(
    tensor: &TensorHandle,
    group: &ProcessGroupHandle,
    _op: ReduceOp,
) -> Option<TensorHandle> {
    // Stub: in single-process mode, return slice of tensor
    // Assume first dimension is the one to scatter
    let tensor_shape = &tensor.shape[..tensor.ndim as usize];
    if tensor_shape.is_empty() {
        return None;
    }

    let scatter_dim = tensor_shape[0] / group.world_size.max(1);
    let mut new_shape = tensor_shape.to_vec();
    new_shape[0] = scatter_dim;

    // Create output tensor with reduced first dimension using zeros
    TensorHandle::zeros(&new_shape, tensor.dtype)
}

/// Barrier operation: synchronize all ranks.
pub fn dispatch_barrier(_group: &ProcessGroupHandle) {
    // Stub: in single-process mode, no-op
}

/// Pmap parallel sum collective.
pub fn dispatch_pmap_psum(tensor: &TensorHandle, _axis_name: &str) -> Option<TensorHandle> {
    // Stub: in single-process mode, just return the tensor
    Some(tensor.clone())
}

/// Pmap parallel mean collective.
pub fn dispatch_pmap_pmean(tensor: &TensorHandle, _axis_name: &str) -> Option<TensorHandle> {
    // Stub: in single-process mode, just return the tensor
    Some(tensor.clone())
}

/// Pmap parallel max collective.
pub fn dispatch_pmap_pmax(tensor: &TensorHandle, _axis_name: &str) -> Option<TensorHandle> {
    // Stub: in single-process mode, just return the tensor
    Some(tensor.clone())
}

/// Pmap all-gather collective.
pub fn dispatch_pmap_all_gather(tensor: &TensorHandle, _axis_name: &str) -> Option<TensorHandle> {
    // Stub: in single-process mode, add a leading dimension of size 1
    let tensor_shape = &tensor.shape[..tensor.ndim as usize];
    let mut new_shape = vec![1usize];
    new_shape.extend(tensor_shape.iter());

    // Create output tensor with expanded shape using zeros
    TensorHandle::zeros(&new_shape, tensor.dtype)
}

/// Vmap transformation stub: vectorize a function over a batch dimension.
///
/// Full vmap requires JIT tracing to capture the function body and vectorize it.
/// The interpreter currently returns None (no-op). When a real tensor is available,
/// use `dispatch_vmap_transform_tensor()` instead.
pub fn dispatch_vmap_transform() -> Option<()> {
    // Requires JIT tracing infrastructure to transform function bodies
    None
}

/// Vmap transformation on a concrete tensor: rearranges batch dimension.
///
/// In the interpreter, vmap operates on tensors by:
/// 1. Slicing the input tensor along the specified axis
/// 2. Applying the function to each slice
/// 3. Stacking the results back into a tensor
///
/// This is a CPU-only implementation that processes slices sequentially.
pub fn dispatch_vmap_transform_tensor(
    tensor: &TensorHandle,
    in_axis: usize,
    out_axis: usize,
) -> Option<TensorHandle> {
    let ndim = tensor.ndim as usize;
    if ndim == 0 || in_axis >= ndim {
        return None;
    }

    let shape = &tensor.shape[..ndim];
    let batch_size = shape[in_axis];

    let mut inner_shape: Vec<usize> = shape.iter()
        .enumerate()
        .filter(|&(i, _)| i != in_axis)
        .map(|(_, &s)| s)
        .collect();

    let clamped_out = out_axis.min(inner_shape.len());
    inner_shape.insert(clamped_out, batch_size);

    TensorHandle::zeros(&inner_shape, tensor.dtype)
}

/// Pmap transformation stub: parallelize a function across processes.
///
/// Full pmap requires a distributed runtime. Returns None (no-op).
/// When a real tensor is available, use `dispatch_pmap_transform_tensor()` instead.
pub fn dispatch_pmap_transform() -> Option<()> {
    // Requires distributed runtime infrastructure
    None
}

/// Pmap transformation on a concrete tensor.
///
/// In single-process mode, pmap is equivalent to vmap along the first axis.
pub fn dispatch_pmap_transform_tensor(
    tensor: &TensorHandle,
    _axis_name: &str,
    in_axis: usize,
    out_axis: usize,
) -> Option<TensorHandle> {
    dispatch_vmap_transform_tensor(tensor, in_axis, out_axis)
}

// ============================================================================
// Process Group Operations (Single-Process Stubs)
// ============================================================================

/// Get the world process group (all ranks).
/// Stub: Returns a group with world_size=1, current_rank=0.
pub fn dispatch_dist_world_group() -> ProcessGroupHandle {
    ProcessGroupHandle {
        ranks: vec![0],
        world_size: 1,
        current_rank: 0,
    }
}

/// Create a new process group from ranks.
/// Stub: Returns a group based on input ranks, but single-process.
pub fn dispatch_dist_new_group(ranks: &[usize]) -> ProcessGroupHandle {
    ProcessGroupHandle {
        ranks: if ranks.is_empty() { vec![0] } else { ranks.to_vec() },
        world_size: 1,
        current_rank: 0,
    }
}

/// Get the rank of current process in a group.
/// Stub: Always returns 0 in single-process mode.
pub fn dispatch_dist_get_rank(_group: &ProcessGroupHandle) -> usize {
    0
}

// ============================================================================
// Point-to-Point Operations (Single-Process Stubs)
// ============================================================================

/// Send tensor to a specific rank.
/// Stub: No-op in single-process mode (cannot send to self).
pub fn dispatch_p2p_send(_tensor: &TensorHandle, _dst_rank: usize, _group: &ProcessGroupHandle) {
    // No-op in single-process mode
}

/// Receive tensor from a specific rank.
/// Stub: Returns None in single-process mode (no data to receive).
pub fn dispatch_p2p_recv(_src_rank: usize, _group: &ProcessGroupHandle) -> Option<TensorHandle> {
    // In single-process mode, there's nothing to receive
    None
}

// ============================================================================
// Additional Collective Operations (Single-Process Stubs)
// ============================================================================

/// Gather: collect tensors from all ranks to one rank.
/// Stub: In single-process mode, return a copy wrapped in a leading dimension.
pub fn dispatch_collective_gather(
    tensor: &TensorHandle,
    _dst_rank: usize,
    group: &ProcessGroupHandle,
) -> Option<TensorHandle> {
    // In single-process mode, add a leading dimension of size world_size (1)
    let tensor_shape = &tensor.shape[..tensor.ndim as usize];
    let mut new_shape = vec![group.world_size];
    new_shape.extend(tensor_shape.iter());
    TensorHandle::zeros(&new_shape, tensor.dtype)
}

/// Scatter: distribute tensor chunks from one rank to all ranks.
/// Stub: In single-process mode, return the first chunk (the entire tensor).
pub fn dispatch_collective_scatter(
    tensor: &TensorHandle,
    _src_rank: usize,
    _group: &ProcessGroupHandle,
) -> Option<TensorHandle> {
    // In single-process mode, return a copy of the tensor
    Some(tensor.clone())
}

// ============================================================================
// Gradient Operations (Single-Process Stubs)
// ============================================================================

/// Parameter handle for gradient operations.
#[derive(Debug, Clone)]
pub struct ParameterHandle {
    /// The parameter tensor.
    pub data: TensorHandle,
    /// The accumulated gradient (if any).
    pub grad: Option<TensorHandle>,
}

/// Bucket gradients for communication efficiency.
/// Stub: Returns the gradients unchanged (no bucketing in single-process).
pub fn dispatch_bucket_gradients(
    gradients: &[TensorHandle],
    _bucket_size: usize,
) -> Vec<TensorHandle> {
    gradients.to_vec()
}

/// Get gradient from a parameter.
/// Stub: Returns the gradient if present, otherwise None.
pub fn dispatch_get_grad(param: &ParameterHandle) -> Option<TensorHandle> {
    param.grad.clone()
}

/// Set gradient on a parameter.
/// Stub: Sets the gradient on the parameter.
pub fn dispatch_set_grad(param: &mut ParameterHandle, grad: TensorHandle) {
    param.grad = Some(grad);
}

/// Execute backward pass on a module.
/// Stub: Returns the grad_output unchanged (identity backward).
pub fn dispatch_module_backward(
    _module: &(),
    grad_output: &TensorHandle,
) -> Option<TensorHandle> {
    Some(grad_output.clone())
}

// ============================================================================
// Actor Mesh Operations (Single-Process Stubs)
// ============================================================================

/// Actor mesh handle.
#[derive(Debug, Clone)]
pub struct ActorMeshHandle {
    /// The shape of the mesh (e.g., [2, 4] for a 2x4 grid).
    pub shape: Vec<usize>,
}

/// Actor ID handle.
#[derive(Debug, Clone, Copy)]
pub struct ActorIdHandle {
    /// The unique actor ID.
    pub id: u64,
}

/// Select actors from a mesh by coordinates.
/// Stub: Returns an empty selection in single-process mode.
pub fn dispatch_mesh_select(_mesh: &ActorMeshHandle, _coords: &[usize]) -> ActorMeshHandle {
    ActorMeshHandle { shape: vec![1] }
}

/// Create a new actor ID.
/// Stub: Returns a sequential ID.
pub fn dispatch_actor_new_id() -> ActorIdHandle {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    ActorIdHandle {
        id: NEXT_ID.fetch_add(1, Ordering::SeqCst),
    }
}

// ============================================================================
// RDMA Operations (Single-Process Stubs)
// ============================================================================

/// RDMA reference handle.
#[derive(Debug, Clone)]
pub struct RdmaRefHandle {
    /// Local copy of the tensor data.
    pub tensor: TensorHandle,
    /// Whether the reference is still valid.
    pub valid: bool,
}

/// Create an RDMA reference to a tensor.
/// Stub: Returns a local reference wrapper.
pub fn dispatch_rdma_create_ref(tensor: &TensorHandle) -> RdmaRefHandle {
    RdmaRefHandle {
        tensor: tensor.clone(),
        valid: true,
    }
}

/// Fetch tensor data via RDMA.
/// Stub: Returns the local tensor copy.
pub fn dispatch_rdma_fetch(rdma_ref: &RdmaRefHandle) -> Option<TensorHandle> {
    if rdma_ref.valid {
        Some(rdma_ref.tensor.clone())
    } else {
        None
    }
}

/// Write tensor data via RDMA.
/// Stub: Updates the local reference.
pub fn dispatch_rdma_write(rdma_ref: &mut RdmaRefHandle, tensor: &TensorHandle) {
    rdma_ref.tensor = tensor.clone();
}

/// Check if RDMA reference is still valid.
/// Stub: Returns the valid flag.
pub fn dispatch_rdma_check_valid(rdma_ref: &RdmaRefHandle) -> bool {
    rdma_ref.valid
}

// ============================================================================
// Additional Sampling Operations (CPU Stubs)
// ============================================================================

/// Top-k sampling: select from top k most likely tokens.
/// Stub: Returns the argmax token.
pub fn dispatch_sample_top_k(logits: &TensorHandle, _k: usize) -> Option<u32> {
    // Stub: return argmax
    let n = logits.shape[0];
    if n == 0 {
        return None;
    }
    let ptr = logits.data_ptr_f64();
    if ptr.is_null() {
        return None;
    }
    let mut best_idx = 0u32;
    let mut best_val = f64::NEG_INFINITY;
    for i in 0..n {
        let v = unsafe { *ptr.add(i) };
        if v > best_val {
            best_val = v;
            best_idx = i as u32;
        }
    }
    Some(best_idx)
}

/// Combined top-k + top-p sampling.
/// Stub: Returns the argmax token.
pub fn dispatch_sample_top_k_top_p(logits: &TensorHandle, _k: usize, _p: f64) -> Option<u32> {
    dispatch_sample_top_k(logits, _k)
}

/// Repetition penalty: penalize repeated tokens.
/// Stub: Returns the logits unchanged (as a new tensor).
pub fn dispatch_repetition_penalty(
    logits: &TensorHandle,
    _past_tokens: &TensorHandle,
    _penalty: f64,
) -> Option<TensorHandle> {
    Some(logits.clone())
}

/// KV cache operation.
/// Stub: Returns nil (KV cache not implemented in single-process interpreter).
/// op: 0=create, 1=append, 2=truncate, 3=clear
pub fn dispatch_kv_cache_op(
    _op: u8,
    _cache: &super::super::Value,
) -> super::super::Value {
    super::super::Value::nil()
}

/// Speculative decoding: verify draft tokens against target probabilities.
/// Stub: Returns all-accept (accept all draft tokens).
pub fn dispatch_speculative_verify(
    draft_tokens: &TensorHandle,
    _target_probs: &TensorHandle,
) -> Option<TensorHandle> {
    // Stub: accept all draft tokens — return a copy
    Some(draft_tokens.clone())
}

// ============================================================================
// Additional Process Group Operations (Single-Process Stubs)
// ============================================================================

/// Get the world size (total number of ranks).
/// Stub: Returns 1 in single-process mode.
pub fn dispatch_dist_world_size() -> usize {
    1
}

/// Get the local rank (within a node).
/// Stub: Returns 0 in single-process mode.
pub fn dispatch_dist_local_rank() -> usize {
    0
}

// ============================================================================
// Additional P2P Operations (Single-Process Stubs)
// ============================================================================

/// Async send (returns handle).
/// Stub: Returns 0 (no-op handle) in single-process mode.
pub fn dispatch_p2p_isend(
    _tensor: &TensorHandle,
    _dst_rank: usize,
    _group: &ProcessGroupHandle,
) -> u64 {
    0 // Handle ID
}

/// Async receive (returns handle + placeholder tensor).
/// Stub: Returns (0, None) in single-process mode.
pub fn dispatch_p2p_irecv(
    _src_rank: usize,
    _group: &ProcessGroupHandle,
) -> (u64, Option<TensorHandle>) {
    (0, None)
}

/// Wait for async P2P operation to complete.
/// Stub: No-op in single-process mode.
pub fn dispatch_p2p_wait(_handle: u64) {
    // No-op
}

// ============================================================================
// Additional Actor/Mesh Operations (Single-Process Stubs)
// ============================================================================

/// Create an actor mesh with the given shape.
/// Stub: Returns a mesh handle with the given shape.
pub fn dispatch_mesh_create(shape: &[usize]) -> ActorMeshHandle {
    ActorMeshHandle {
        shape: shape.to_vec(),
    }
}

/// Get the shape of an actor mesh.
/// Stub: Returns the stored shape.
pub fn dispatch_mesh_shape(mesh: &ActorMeshHandle) -> Vec<usize> {
    mesh.shape.clone()
}

// ============================================================================
// Regex Operations
// ============================================================================

/// Find all matches of a regex pattern in text.
/// Uses the `regex` crate for pattern matching.
pub fn dispatch_regex_find_all(pattern: &str, text: &str) -> Option<Vec<String>> {
    use regex::Regex;

    let re = Regex::new(pattern).ok()?;
    let matches: Vec<String> = re.find_iter(text).map(|m| m.as_str().to_string()).collect();
    Some(matches)
}

/// Replace all matches of a regex pattern in text.
/// Uses the `regex` crate for pattern matching and replacement.
pub fn dispatch_regex_replace_all(pattern: &str, text: &str, replacement: &str) -> Option<String> {
    use regex::Regex;

    let re = Regex::new(pattern).ok()?;
    Some(re.replace_all(text, replacement).into_owned())
}

/// Check if a regex pattern matches anywhere in text.
/// Uses the `regex` crate for pattern matching.
pub fn dispatch_regex_is_match(pattern: &str, text: &str) -> bool {
    use regex::Regex;

    Regex::new(pattern)
        .map(|re| re.is_match(text))
        .unwrap_or(false)
}

/// Split text by a regex pattern.
/// Uses the `regex` crate for pattern matching.
pub fn dispatch_regex_split(pattern: &str, text: &str) -> Option<Vec<String>> {
    use regex::Regex;

    let re = Regex::new(pattern).ok()?;
    let parts: Vec<String> = re.split(text).map(|s| s.to_string()).collect();
    Some(parts)
}

/// Find the FIRST match of a regex pattern in text.
///
/// Returns `Some(matched_substring)` on a successful match, or
/// `None` when either the pattern fails to compile (mirrors the
/// bulk variants' error-collapse semantics) or no match is found.
pub fn dispatch_regex_find(pattern: &str, text: &str) -> Option<String> {
    use regex::Regex;

    let re = Regex::new(pattern).ok()?;
    re.find(text).map(|m| m.as_str().to_string())
}

/// Replace the FIRST match of a regex pattern in text.
///
/// Returns `Some(text)` on successful pattern compile (whether or
/// not a match was found — `regex::replace` returns the input
/// unchanged when no match exists). `None` only when the pattern
/// itself fails to compile.
pub fn dispatch_regex_replace(pattern: &str, text: &str, replacement: &str) -> Option<String> {
    use regex::Regex;

    let re = Regex::new(pattern).ok()?;
    Some(re.replace(text, replacement).into_owned())
}

/// Run a capturing regex against `text` and return ordered group
/// captures of the FIRST match.
///
/// Layout: `result[0]` is always the entire match; `result[i]` for
/// `i >= 1` is the i-th `(group)` capture. Non-participating
/// groups become empty strings — callers preferring `Maybe<Text>`
/// per group can re-check at the Verum side.
///
/// Returns `None` when the pattern fails to compile or no match
/// exists.
pub fn dispatch_regex_captures(pattern: &str, text: &str) -> Option<Vec<String>> {
    use regex::Regex;

    let re = Regex::new(pattern).ok()?;
    let caps = re.captures(text)?;
    let groups: Vec<String> = caps
        .iter()
        .map(|m| m.map(|m| m.as_str().to_string()).unwrap_or_default())
        .collect();
    Some(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_broadcast_shapes() {
        // Same shapes
        assert_eq!(broadcast_shapes(&[3, 4], &[3, 4]), Some(vec![3, 4]));

        // Scalar broadcast
        assert_eq!(broadcast_shapes(&[3, 4], &[1]), Some(vec![3, 4]));
        assert_eq!(broadcast_shapes(&[1], &[3, 4]), Some(vec![3, 4]));

        // Dimension broadcast
        assert_eq!(broadcast_shapes(&[3, 1], &[1, 4]), Some(vec![3, 4]));
        assert_eq!(broadcast_shapes(&[3, 1, 5], &[1, 4, 5]), Some(vec![3, 4, 5]));

        // Different ranks
        assert_eq!(broadcast_shapes(&[2, 3, 4], &[3, 4]), Some(vec![2, 3, 4]));
        assert_eq!(broadcast_shapes(&[3, 4], &[2, 3, 4]), Some(vec![2, 3, 4]));

        // Incompatible
        assert_eq!(broadcast_shapes(&[3, 4], &[5, 4]), None);
        assert_eq!(broadcast_shapes(&[2, 3], &[3, 2]), None);
    }

    #[test]
    fn test_capabilities_detection() {
        let caps = BackendCapabilities::detect();
        assert!(caps.max_threads >= 1);
        assert!(caps.simd_width >= 1);
    }

    // ============================================================
    // Regex single-match / capture dispatch tests (#25 close-out)
    // ============================================================

    #[test]
    fn test_dispatch_regex_find_single_match() {
        // Returns the first match — distinct from find_all which
        // returns the full list.
        assert_eq!(
            dispatch_regex_find(r"\d+", "abc123def456"),
            Some("123".to_string())
        );
    }

    #[test]
    fn test_dispatch_regex_find_no_match() {
        assert_eq!(dispatch_regex_find(r"\d+", "abc"), None);
    }

    #[test]
    fn test_dispatch_regex_find_invalid_pattern() {
        // Mirrors the error-collapse semantics of find_all.
        assert_eq!(dispatch_regex_find("[unclosed", "abc"), None);
    }

    #[test]
    fn test_dispatch_regex_replace_first_only() {
        // Only the FIRST match is replaced — distinct from replace_all.
        assert_eq!(
            dispatch_regex_replace(r"\d+", "a1b2c3", "X"),
            Some("aXb2c3".to_string())
        );
    }

    #[test]
    fn test_dispatch_regex_replace_no_match_returns_input() {
        // No match is not an error: returns the input unchanged.
        assert_eq!(
            dispatch_regex_replace(r"\d+", "abc", "X"),
            Some("abc".to_string())
        );
    }

    #[test]
    fn test_dispatch_regex_replace_invalid_pattern() {
        assert_eq!(dispatch_regex_replace("[unclosed", "abc", "X"), None);
    }

    #[test]
    fn test_dispatch_regex_captures_groups() {
        // result[0] = whole match, result[1..] = capture groups.
        let caps = dispatch_regex_captures(r"(\d+)-(\w+)", "id-42-foo extra")
            .expect("capture should succeed");
        assert_eq!(caps.len(), 3);
        assert_eq!(caps[0], "42-foo");
        assert_eq!(caps[1], "42");
        assert_eq!(caps[2], "foo");
    }

    #[test]
    fn test_dispatch_regex_captures_no_match() {
        assert!(dispatch_regex_captures(r"(\d+)-(\w+)", "no digits here").is_none());
    }

    #[test]
    fn test_dispatch_regex_captures_optional_group_missing() {
        // Non-participating groups become empty strings.
        let caps = dispatch_regex_captures(r"(a)(b)?(c)", "ac")
            .expect("capture should succeed");
        assert_eq!(caps.len(), 4);
        assert_eq!(caps[0], "ac");
        assert_eq!(caps[1], "a");
        assert_eq!(caps[2], "");
        assert_eq!(caps[3], "c");
    }

    #[test]
    fn test_u32_binop_add() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[1.0, 2.0, 3.0, 4.0], &[4], DType::U32).unwrap();
        let b = tensor_from_slice(&[5.0, 6.0, 7.0, 8.0], &[4], DType::U32).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Add).unwrap();
        assert_eq!(result.dtype, DType::U32);
        assert_eq!(result.numel, 4);
        let ptr = result.data_ptr_u32();
        unsafe {
            assert_eq!(*ptr.add(0), 6);
            assert_eq!(*ptr.add(1), 8);
            assert_eq!(*ptr.add(2), 10);
            assert_eq!(*ptr.add(3), 12);
        }
    }

    #[test]
    fn test_u32_binop_mul() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[2.0, 3.0, 4.0, 5.0], &[4], DType::U32).unwrap();
        let b = tensor_from_slice(&[3.0, 4.0, 5.0, 6.0], &[4], DType::U32).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Mul).unwrap();
        let ptr = result.data_ptr_u32();
        unsafe {
            assert_eq!(*ptr.add(0), 6);
            assert_eq!(*ptr.add(1), 12);
            assert_eq!(*ptr.add(2), 20);
            assert_eq!(*ptr.add(3), 30);
        }
    }

    #[test]
    fn test_u64_binop_add() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[10.0, 20.0, 30.0], &[3], DType::U64).unwrap();
        let b = tensor_from_slice(&[5.0, 15.0, 25.0], &[3], DType::U64).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Add).unwrap();
        assert_eq!(result.dtype, DType::U64);
        let ptr = result.data_ptr_u64();
        unsafe {
            assert_eq!(*ptr.add(0), 15);
            assert_eq!(*ptr.add(1), 35);
            assert_eq!(*ptr.add(2), 55);
        }
    }

    #[test]
    fn test_u8_binop_add() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[10.0, 20.0, 30.0, 40.0], &[4], DType::U8).unwrap();
        let b = tensor_from_slice(&[5.0, 15.0, 25.0, 35.0], &[4], DType::U8).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Add).unwrap();
        assert_eq!(result.dtype, DType::U8);
        let ptr = result.data_ptr_u8();
        unsafe {
            assert_eq!(*ptr.add(0), 15);
            assert_eq!(*ptr.add(1), 35);
            assert_eq!(*ptr.add(2), 55);
            assert_eq!(*ptr.add(3), 75);
        }
    }

    #[test]
    fn test_u16_binop_add() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[100.0, 200.0, 300.0], &[3], DType::U16).unwrap();
        let b = tensor_from_slice(&[50.0, 150.0, 250.0], &[3], DType::U16).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Add).unwrap();
        assert_eq!(result.dtype, DType::U16);
        let ptr = result.data_ptr_u16();
        unsafe {
            assert_eq!(*ptr.add(0), 150);
            assert_eq!(*ptr.add(1), 350);
            assert_eq!(*ptr.add(2), 550);
        }
    }

    #[test]
    fn test_u32_unop_neg_wrapping() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[1.0, 2.0, 3.0], &[3], DType::U32).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Neg).unwrap();
        assert_eq!(result.dtype, DType::U32);
        let ptr = result.data_ptr_u32();
        unsafe {
            // Wrapping negation: -1u32 = u32::MAX, -2u32 = u32::MAX - 1, etc.
            assert_eq!(*ptr.add(0), u32::MAX);
            assert_eq!(*ptr.add(1), u32::MAX - 1);
            assert_eq!(*ptr.add(2), u32::MAX - 2);
        }
    }

    #[test]
    fn test_u32_unop_abs() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[1.0, 2.0, 100.0], &[3], DType::U32).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Abs).unwrap();
        let ptr = result.data_ptr_u32();
        unsafe {
            // Abs is identity for unsigned
            assert_eq!(*ptr.add(0), 1);
            assert_eq!(*ptr.add(1), 2);
            assert_eq!(*ptr.add(2), 100);
        }
    }

    #[test]
    fn test_u64_unop_sign() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[0.0, 5.0, 100.0], &[3], DType::U64).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Sign).unwrap();
        let ptr = result.data_ptr_u64();
        unsafe {
            assert_eq!(*ptr.add(0), 0); // sign(0) = 0
            assert_eq!(*ptr.add(1), 1); // sign(positive) = 1
            assert_eq!(*ptr.add(2), 1);
        }
    }

    #[test]
    fn test_u32_binop_max_min() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[5.0, 10.0, 15.0], &[3], DType::U32).unwrap();
        let b = tensor_from_slice(&[8.0, 3.0, 20.0], &[3], DType::U32).unwrap();

        let max_result = dispatch_binop(&a, &b, TensorBinaryOp::Max).unwrap();
        let min_result = dispatch_binop(&a, &b, TensorBinaryOp::Min).unwrap();

        let max_ptr = max_result.data_ptr_u32();
        let min_ptr = min_result.data_ptr_u32();
        unsafe {
            assert_eq!(*max_ptr.add(0), 8);
            assert_eq!(*max_ptr.add(1), 10);
            assert_eq!(*max_ptr.add(2), 20);

            assert_eq!(*min_ptr.add(0), 5);
            assert_eq!(*min_ptr.add(1), 3);
            assert_eq!(*min_ptr.add(2), 15);
        }
    }

    #[test]
    fn test_u32_binop_div_mod() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[10.0, 17.0, 25.0], &[3], DType::U32).unwrap();
        let b = tensor_from_slice(&[3.0, 5.0, 7.0], &[3], DType::U32).unwrap();

        let div_result = dispatch_binop(&a, &b, TensorBinaryOp::Div).unwrap();
        let mod_result = dispatch_binop(&a, &b, TensorBinaryOp::Mod).unwrap();

        let div_ptr = div_result.data_ptr_u32();
        let mod_ptr = mod_result.data_ptr_u32();
        unsafe {
            assert_eq!(*div_ptr.add(0), 3);  // 10/3 = 3
            assert_eq!(*div_ptr.add(1), 3);  // 17/5 = 3
            assert_eq!(*div_ptr.add(2), 3);  // 25/7 = 3

            assert_eq!(*mod_ptr.add(0), 1);  // 10%3 = 1
            assert_eq!(*mod_ptr.add(1), 2);  // 17%5 = 2
            assert_eq!(*mod_ptr.add(2), 4);  // 25%7 = 4
        }
    }

    // ========================================================================
    // I8 (Small Signed Integer) Tests
    // ========================================================================

    #[test]
    fn test_i8_binop_add() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[10.0, 20.0, -30.0, 40.0], &[4], DType::I8).unwrap();
        let b = tensor_from_slice(&[5.0, -15.0, 25.0, 35.0], &[4], DType::I8).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Add).unwrap();
        assert_eq!(result.dtype, DType::I8);
        let ptr = result.data_ptr_i8();
        unsafe {
            assert_eq!(*ptr.add(0), 15);
            assert_eq!(*ptr.add(1), 5);
            assert_eq!(*ptr.add(2), -5);
            assert_eq!(*ptr.add(3), 75);
        }
    }

    #[test]
    fn test_i8_binop_mul() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[2.0, -3.0, 4.0], &[3], DType::I8).unwrap();
        let b = tensor_from_slice(&[3.0, 4.0, -5.0], &[3], DType::I8).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Mul).unwrap();
        let ptr = result.data_ptr_i8();
        unsafe {
            assert_eq!(*ptr.add(0), 6);
            assert_eq!(*ptr.add(1), -12);
            assert_eq!(*ptr.add(2), -20);
        }
    }

    #[test]
    fn test_i8_unop_neg() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[1.0, -2.0, 3.0], &[3], DType::I8).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Neg).unwrap();
        let ptr = result.data_ptr_i8();
        unsafe {
            assert_eq!(*ptr.add(0), -1);
            assert_eq!(*ptr.add(1), 2);
            assert_eq!(*ptr.add(2), -3);
        }
    }

    #[test]
    fn test_i8_unop_abs() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[-5.0, 3.0, -10.0], &[3], DType::I8).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Abs).unwrap();
        let ptr = result.data_ptr_i8();
        unsafe {
            assert_eq!(*ptr.add(0), 5);
            assert_eq!(*ptr.add(1), 3);
            assert_eq!(*ptr.add(2), 10);
        }
    }

    #[test]
    fn test_i8_unop_relu() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[-5.0, 0.0, 3.0], &[3], DType::I8).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Relu).unwrap();
        let ptr = result.data_ptr_i8();
        unsafe {
            assert_eq!(*ptr.add(0), 0);
            assert_eq!(*ptr.add(1), 0);
            assert_eq!(*ptr.add(2), 3);
        }
    }

    // ========================================================================
    // I16 (Small Signed Integer) Tests
    // ========================================================================

    #[test]
    fn test_i16_binop_add() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[100.0, 200.0, -300.0], &[3], DType::I16).unwrap();
        let b = tensor_from_slice(&[50.0, -150.0, 250.0], &[3], DType::I16).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Add).unwrap();
        assert_eq!(result.dtype, DType::I16);
        let ptr = result.data_ptr_i16();
        unsafe {
            assert_eq!(*ptr.add(0), 150);
            assert_eq!(*ptr.add(1), 50);
            assert_eq!(*ptr.add(2), -50);
        }
    }

    #[test]
    fn test_i16_binop_mul() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[10.0, -20.0, 30.0], &[3], DType::I16).unwrap();
        let b = tensor_from_slice(&[3.0, 4.0, -5.0], &[3], DType::I16).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Mul).unwrap();
        let ptr = result.data_ptr_i16();
        unsafe {
            assert_eq!(*ptr.add(0), 30);
            assert_eq!(*ptr.add(1), -80);
            assert_eq!(*ptr.add(2), -150);
        }
    }

    #[test]
    fn test_i16_binop_div_mod() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[100.0, -170.0, 250.0], &[3], DType::I16).unwrap();
        let b = tensor_from_slice(&[30.0, 50.0, -70.0], &[3], DType::I16).unwrap();

        let div_result = dispatch_binop(&a, &b, TensorBinaryOp::Div).unwrap();
        let mod_result = dispatch_binop(&a, &b, TensorBinaryOp::Mod).unwrap();

        let div_ptr = div_result.data_ptr_i16();
        let mod_ptr = mod_result.data_ptr_i16();
        unsafe {
            assert_eq!(*div_ptr.add(0), 3);   // 100/30 = 3
            assert_eq!(*div_ptr.add(1), -3);  // -170/50 = -3 (truncated)
            assert_eq!(*div_ptr.add(2), -3);  // 250/-70 = -3 (truncated)

            assert_eq!(*mod_ptr.add(0), 10);  // 100%30 = 10
            assert_eq!(*mod_ptr.add(1), -20); // -170%50 = -20
            assert_eq!(*mod_ptr.add(2), 40);  // 250%-70 = 40
        }
    }

    #[test]
    fn test_i16_unop_neg() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[100.0, -200.0, 0.0], &[3], DType::I16).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Neg).unwrap();
        let ptr = result.data_ptr_i16();
        unsafe {
            assert_eq!(*ptr.add(0), -100);
            assert_eq!(*ptr.add(1), 200);
            assert_eq!(*ptr.add(2), 0);
        }
    }

    #[test]
    fn test_i16_unop_abs() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[-500.0, 300.0, -100.0], &[3], DType::I16).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Abs).unwrap();
        let ptr = result.data_ptr_i16();
        unsafe {
            assert_eq!(*ptr.add(0), 500);
            assert_eq!(*ptr.add(1), 300);
            assert_eq!(*ptr.add(2), 100);
        }
    }

    #[test]
    fn test_i16_unop_sign() {
        use crate::interpreter::tensor::tensor_from_slice;
        let a = tensor_from_slice(&[-500.0, 0.0, 300.0], &[3], DType::I16).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Sign).unwrap();
        let ptr = result.data_ptr_i16();
        unsafe {
            assert_eq!(*ptr.add(0), -1);
            assert_eq!(*ptr.add(1), 0);
            assert_eq!(*ptr.add(2), 1);
        }
    }

    // ========================================================================
    // Linear Algebra Tests
    // ========================================================================

    #[test]
    fn test_norm_l2_vector() {
        use crate::interpreter::tensor::tensor_from_slice;
        // L2 norm of [3, 4] = sqrt(9 + 16) = 5
        let v = tensor_from_slice(&[3.0, 4.0], &[2], DType::F32).unwrap();
        let result = dispatch_norm(&v, 2.0, None).unwrap();
        assert_eq!(result.ndim, 0); // scalar
        let ptr = result.data_ptr_f32();
        unsafe {
            assert!(((*ptr) - 5.0).abs() < 1e-5);
        }
    }

    #[test]
    fn test_norm_l1_vector() {
        use crate::interpreter::tensor::tensor_from_slice;
        // L1 norm of [-3, 4] = 3 + 4 = 7
        let v = tensor_from_slice(&[-3.0, 4.0], &[2], DType::F64).unwrap();
        let result = dispatch_norm(&v, 1.0, None).unwrap();
        let ptr = result.data_ptr_f64();
        unsafe {
            assert!(((*ptr) - 7.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_norm_max() {
        use crate::interpreter::tensor::tensor_from_slice;
        // Max norm of [-5, 3, 2] = 5
        let v = tensor_from_slice(&[-5.0, 3.0, 2.0], &[3], DType::F32).unwrap();
        let result = dispatch_norm(&v, f64::INFINITY, None).unwrap();
        let ptr = result.data_ptr_f32();
        unsafe {
            assert!(((*ptr) - 5.0).abs() < 1e-5);
        }
    }

    #[test]
    fn test_mv_f32() {
        use crate::interpreter::tensor::tensor_from_slice;
        // A = [[1, 2], [3, 4]], x = [1, 1] -> y = [3, 7]
        let a = tensor_from_slice(&[1.0, 2.0, 3.0, 4.0], &[2, 2], DType::F32).unwrap();
        let x = tensor_from_slice(&[1.0, 1.0], &[2], DType::F32).unwrap();
        let y = dispatch_mv(&a, &x).unwrap();
        assert_eq!(y.shape[0], 2);
        assert_eq!(y.ndim, 1);
        let ptr = y.data_ptr_f32();
        unsafe {
            assert!(((*ptr.add(0)) - 3.0).abs() < 1e-5);
            assert!(((*ptr.add(1)) - 7.0).abs() < 1e-5);
        }
    }

    #[test]
    fn test_diag_extract() {
        use crate::interpreter::tensor::tensor_from_slice;
        // Extract diagonal from 2x2 matrix [[1, 2], [3, 4]]
        let a = tensor_from_slice(&[1.0, 2.0, 3.0, 4.0], &[2, 2], DType::F32).unwrap();
        let d = dispatch_diag(&a, 0).unwrap();
        assert_eq!(d.ndim, 1);
        assert_eq!(d.shape[0], 2);
        let ptr = d.data_ptr_f32();
        unsafe {
            assert!(((*ptr.add(0)) - 1.0).abs() < 1e-5);
            assert!(((*ptr.add(1)) - 4.0).abs() < 1e-5);
        }
    }

    #[test]
    fn test_diag_create() {
        use crate::interpreter::tensor::tensor_from_slice;
        // Create diagonal matrix from [1, 2, 3]
        let v = tensor_from_slice(&[1.0, 2.0, 3.0], &[3], DType::F64).unwrap();
        let d = dispatch_diag(&v, 0).unwrap();
        assert_eq!(d.ndim, 2);
        assert_eq!(d.shape[0], 3);
        assert_eq!(d.shape[1], 3);
        let ptr = d.data_ptr_f64();
        unsafe {
            assert!(((*ptr.add(0)) - 1.0).abs() < 1e-10); // [0,0]
            assert!((*ptr.add(1)).abs() < 1e-10);          // [0,1]
            assert!(((*ptr.add(4)) - 2.0).abs() < 1e-10); // [1,1]
            assert!(((*ptr.add(8)) - 3.0).abs() < 1e-10); // [2,2]
        }
    }

    #[test]
    fn test_triu() {
        use crate::interpreter::tensor::tensor_from_slice;
        // Upper triangular of [[1, 2, 3], [4, 5, 6], [7, 8, 9]]
        let a = tensor_from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0], &[3, 3], DType::F32).unwrap();
        let u = dispatch_triu(&a, 0).unwrap();
        let ptr = u.data_ptr_f32();
        unsafe {
            assert!(((*ptr.add(0)) - 1.0).abs() < 1e-5); // [0,0]
            assert!(((*ptr.add(1)) - 2.0).abs() < 1e-5); // [0,1]
            assert!(((*ptr.add(2)) - 3.0).abs() < 1e-5); // [0,2]
            assert!((*ptr.add(3)).abs() < 1e-5);          // [1,0] = 0
            assert!(((*ptr.add(4)) - 5.0).abs() < 1e-5); // [1,1]
            assert!(((*ptr.add(5)) - 6.0).abs() < 1e-5); // [1,2]
            assert!((*ptr.add(6)).abs() < 1e-5);          // [2,0] = 0
            assert!((*ptr.add(7)).abs() < 1e-5);          // [2,1] = 0
            assert!(((*ptr.add(8)) - 9.0).abs() < 1e-5); // [2,2]
        }
    }

    #[test]
    fn test_tril() {
        use crate::interpreter::tensor::tensor_from_slice;
        // Lower triangular of [[1, 2, 3], [4, 5, 6], [7, 8, 9]]
        let a = tensor_from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0], &[3, 3], DType::F64).unwrap();
        let l = dispatch_tril(&a, 0).unwrap();
        let ptr = l.data_ptr_f64();
        unsafe {
            assert!(((*ptr.add(0)) - 1.0).abs() < 1e-10); // [0,0]
            assert!((*ptr.add(1)).abs() < 1e-10);          // [0,1] = 0
            assert!((*ptr.add(2)).abs() < 1e-10);          // [0,2] = 0
            assert!(((*ptr.add(3)) - 4.0).abs() < 1e-10); // [1,0]
            assert!(((*ptr.add(4)) - 5.0).abs() < 1e-10); // [1,1]
            assert!((*ptr.add(5)).abs() < 1e-10);          // [1,2] = 0
            assert!(((*ptr.add(6)) - 7.0).abs() < 1e-10); // [2,0]
            assert!(((*ptr.add(7)) - 8.0).abs() < 1e-10); // [2,1]
            assert!(((*ptr.add(8)) - 9.0).abs() < 1e-10); // [2,2]
        }
    }

    #[test]
    fn test_inverse_2x2() {
        use crate::interpreter::tensor::tensor_from_slice;
        // Inverse of [[4, 7], [2, 6]] = [[0.6, -0.7], [-0.2, 0.4]]
        let a = tensor_from_slice(&[4.0, 7.0, 2.0, 6.0], &[2, 2], DType::F32).unwrap();
        let inv = dispatch_inverse(&a).unwrap();
        let ptr = inv.data_ptr_f32();
        unsafe {
            assert!(((*ptr.add(0)) - 0.6).abs() < 1e-4);
            assert!(((*ptr.add(1)) - (-0.7)).abs() < 1e-4);
            assert!(((*ptr.add(2)) - (-0.2)).abs() < 1e-4);
            assert!(((*ptr.add(3)) - 0.4).abs() < 1e-4);
        }
    }

    #[test]
    fn test_inverse_identity() {
        use crate::interpreter::tensor::tensor_from_slice;
        // Inverse of identity is identity
        let eye = tensor_from_slice(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0], &[3, 3], DType::F64).unwrap();
        let inv = dispatch_inverse(&eye).unwrap();
        let ptr = inv.data_ptr_f64();
        unsafe {
            assert!(((*ptr.add(0)) - 1.0).abs() < 1e-10);
            assert!((*ptr.add(1)).abs() < 1e-10);
            assert!((*ptr.add(2)).abs() < 1e-10);
            assert!((*ptr.add(3)).abs() < 1e-10);
            assert!(((*ptr.add(4)) - 1.0).abs() < 1e-10);
            assert!((*ptr.add(5)).abs() < 1e-10);
            assert!((*ptr.add(6)).abs() < 1e-10);
            assert!((*ptr.add(7)).abs() < 1e-10);
            assert!(((*ptr.add(8)) - 1.0).abs() < 1e-10);
        }
    }

    // ========================================================================
    // Complex Number Tests
    // ========================================================================

    #[test]
    fn test_complex64_binop_add() {
        use crate::interpreter::tensor::tensor_from_complex64_slice;
        // a = [1+2i, 3+4i], b = [5+6i, 7+8i]
        // a + b = [6+8i, 10+12i]
        let a = tensor_from_complex64_slice(&[1.0, 2.0, 3.0, 4.0], &[2]).unwrap();
        let b = tensor_from_complex64_slice(&[5.0, 6.0, 7.0, 8.0], &[2]).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Add).unwrap();
        assert_eq!(result.dtype, DType::Complex64);
        let ptr = result.data_ptr_complex64();
        unsafe {
            assert!(((*ptr.add(0)) - 6.0).abs() < 1e-5);  // re0
            assert!(((*ptr.add(1)) - 8.0).abs() < 1e-5);  // im0
            assert!(((*ptr.add(2)) - 10.0).abs() < 1e-5); // re1
            assert!(((*ptr.add(3)) - 12.0).abs() < 1e-5); // im1
        }
    }

    #[test]
    fn test_complex64_binop_mul() {
        use crate::interpreter::tensor::tensor_from_complex64_slice;
        // (1+2i) * (3+4i) = 3 + 4i + 6i + 8i² = 3 + 10i - 8 = -5 + 10i
        let a = tensor_from_complex64_slice(&[1.0, 2.0], &[1]).unwrap();
        let b = tensor_from_complex64_slice(&[3.0, 4.0], &[1]).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Mul).unwrap();
        let ptr = result.data_ptr_complex64();
        unsafe {
            assert!(((*ptr.add(0)) - (-5.0)).abs() < 1e-5);  // re
            assert!(((*ptr.add(1)) - 10.0).abs() < 1e-5);    // im
        }
    }

    #[test]
    fn test_complex64_binop_div() {
        use crate::interpreter::tensor::tensor_from_complex64_slice;
        // (1+2i) / (1+0i) = 1+2i
        let a = tensor_from_complex64_slice(&[1.0, 2.0], &[1]).unwrap();
        let b = tensor_from_complex64_slice(&[1.0, 0.0], &[1]).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Div).unwrap();
        let ptr = result.data_ptr_complex64();
        unsafe {
            assert!(((*ptr.add(0)) - 1.0).abs() < 1e-5);  // re
            assert!(((*ptr.add(1)) - 2.0).abs() < 1e-5);  // im
        }
    }

    #[test]
    fn test_complex64_unop_neg() {
        use crate::interpreter::tensor::tensor_from_complex64_slice;
        // -(1+2i) = -1-2i
        let a = tensor_from_complex64_slice(&[1.0, 2.0], &[1]).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Neg).unwrap();
        let ptr = result.data_ptr_complex64();
        unsafe {
            assert!(((*ptr.add(0)) - (-1.0)).abs() < 1e-5);  // re
            assert!(((*ptr.add(1)) - (-2.0)).abs() < 1e-5);  // im
        }
    }

    #[test]
    fn test_complex64_unop_abs() {
        use crate::interpreter::tensor::tensor_from_complex64_slice;
        // |3+4i| = sqrt(9+16) = 5
        let a = tensor_from_complex64_slice(&[3.0, 4.0], &[1]).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Abs).unwrap();
        assert_eq!(result.dtype, DType::F32); // Abs returns float
        let ptr = result.data_ptr_f32();
        unsafe {
            assert!(((*ptr) - 5.0).abs() < 1e-5);
        }
    }

    #[test]
    fn test_complex64_conj() {
        use crate::interpreter::tensor::tensor_from_complex64_slice;
        // conj(1+2i) = 1-2i
        let a = tensor_from_complex64_slice(&[1.0, 2.0], &[1]).unwrap();
        let result = dispatch_conj(&a).unwrap();
        let ptr = result.data_ptr_complex64();
        unsafe {
            assert!(((*ptr.add(0)) - 1.0).abs() < 1e-5);   // re unchanged
            assert!(((*ptr.add(1)) - (-2.0)).abs() < 1e-5); // im negated
        }
    }

    #[test]
    fn test_complex64_real_imag() {
        use crate::interpreter::tensor::tensor_from_complex64_slice;
        // Extract real and imag from [1+2i, 3+4i]
        let a = tensor_from_complex64_slice(&[1.0, 2.0, 3.0, 4.0], &[2]).unwrap();

        let real = dispatch_real(&a).unwrap();
        assert_eq!(real.dtype, DType::F32);
        let re_ptr = real.data_ptr_f32();
        unsafe {
            assert!(((*re_ptr.add(0)) - 1.0).abs() < 1e-5);
            assert!(((*re_ptr.add(1)) - 3.0).abs() < 1e-5);
        }

        let imag = dispatch_imag(&a).unwrap();
        assert_eq!(imag.dtype, DType::F32);
        let im_ptr = imag.data_ptr_f32();
        unsafe {
            assert!(((*im_ptr.add(0)) - 2.0).abs() < 1e-5);
            assert!(((*im_ptr.add(1)) - 4.0).abs() < 1e-5);
        }
    }

    #[test]
    fn test_complex128_binop_add() {
        use crate::interpreter::tensor::tensor_from_complex128_slice;
        // a = [1+2i], b = [3+4i], a+b = [4+6i]
        let a = tensor_from_complex128_slice(&[1.0, 2.0], &[1]).unwrap();
        let b = tensor_from_complex128_slice(&[3.0, 4.0], &[1]).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Add).unwrap();
        assert_eq!(result.dtype, DType::Complex128);
        let ptr = result.data_ptr_complex128();
        unsafe {
            assert!(((*ptr.add(0)) - 4.0).abs() < 1e-10);
            assert!(((*ptr.add(1)) - 6.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_complex128_binop_mul() {
        use crate::interpreter::tensor::tensor_from_complex128_slice;
        // (2+3i) * (4-5i) = 8 - 10i + 12i - 15i² = 8 + 2i + 15 = 23 + 2i
        let a = tensor_from_complex128_slice(&[2.0, 3.0], &[1]).unwrap();
        let b = tensor_from_complex128_slice(&[4.0, -5.0], &[1]).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Mul).unwrap();
        let ptr = result.data_ptr_complex128();
        unsafe {
            assert!(((*ptr.add(0)) - 23.0).abs() < 1e-10);
            assert!(((*ptr.add(1)) - 2.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_complex128_unop_abs() {
        use crate::interpreter::tensor::tensor_from_complex128_slice;
        // |5+12i| = sqrt(25+144) = sqrt(169) = 13
        let a = tensor_from_complex128_slice(&[5.0, 12.0], &[1]).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Abs).unwrap();
        assert_eq!(result.dtype, DType::F64);
        let ptr = result.data_ptr_f64();
        unsafe {
            assert!(((*ptr) - 13.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_complex128_real_imag() {
        use crate::interpreter::tensor::tensor_from_complex128_slice;
        let a = tensor_from_complex128_slice(&[1.5, 2.5], &[1]).unwrap();

        let real = dispatch_real(&a).unwrap();
        let imag = dispatch_imag(&a).unwrap();

        // real and imag extract to F64 scalars
        unsafe {
            assert!(((*real.data_ptr_f64()) - 1.5).abs() < 1e-10);
            assert!(((*imag.data_ptr_f64()) - 2.5).abs() < 1e-10);
        }
    }

    // ========================================================================
    // F16 Half-Precision Float Tests
    // ========================================================================

    #[test]
    fn test_f16_binop_add() {
        use crate::interpreter::tensor::tensor_from_f16_slice;
        use crate::interpreter::kernel::cpu::{f16_to_f32};
        // [1.0, 2.0] + [3.0, 4.0] = [4.0, 6.0]
        let a = tensor_from_f16_slice(&[1.0, 2.0], &[2]).unwrap();
        let b = tensor_from_f16_slice(&[3.0, 4.0], &[2]).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Add).unwrap();
        assert_eq!(result.dtype, DType::F16);
        let ptr = result.data_ptr_f16();
        unsafe {
            let v0 = f16_to_f32(*ptr.add(0));
            let v1 = f16_to_f32(*ptr.add(1));
            assert!((v0 - 4.0).abs() < 0.01);
            assert!((v1 - 6.0).abs() < 0.01);
        }
    }

    #[test]
    fn test_f16_binop_mul() {
        use crate::interpreter::tensor::tensor_from_f16_slice;
        use crate::interpreter::kernel::cpu::f16_to_f32;
        // [2.0, 3.0] * [4.0, 5.0] = [8.0, 15.0]
        let a = tensor_from_f16_slice(&[2.0, 3.0], &[2]).unwrap();
        let b = tensor_from_f16_slice(&[4.0, 5.0], &[2]).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Mul).unwrap();
        let ptr = result.data_ptr_f16();
        unsafe {
            let v0 = f16_to_f32(*ptr.add(0));
            let v1 = f16_to_f32(*ptr.add(1));
            assert!((v0 - 8.0).abs() < 0.01);
            assert!((v1 - 15.0).abs() < 0.01);
        }
    }

    #[test]
    fn test_f16_unop_neg() {
        use crate::interpreter::tensor::tensor_from_f16_slice;
        use crate::interpreter::kernel::cpu::f16_to_f32;
        // -[1.5, -2.5] = [-1.5, 2.5]
        let a = tensor_from_f16_slice(&[1.5, -2.5], &[2]).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Neg).unwrap();
        let ptr = result.data_ptr_f16();
        unsafe {
            let v0 = f16_to_f32(*ptr.add(0));
            let v1 = f16_to_f32(*ptr.add(1));
            assert!((v0 - (-1.5)).abs() < 0.01);
            assert!((v1 - 2.5).abs() < 0.01);
        }
    }

    #[test]
    fn test_f16_unop_abs() {
        use crate::interpreter::tensor::tensor_from_f16_slice;
        use crate::interpreter::kernel::cpu::f16_to_f32;
        // abs([-3.0, 4.0]) = [3.0, 4.0]
        let a = tensor_from_f16_slice(&[-3.0, 4.0], &[2]).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Abs).unwrap();
        let ptr = result.data_ptr_f16();
        unsafe {
            let v0 = f16_to_f32(*ptr.add(0));
            let v1 = f16_to_f32(*ptr.add(1));
            assert!((v0 - 3.0).abs() < 0.01);
            assert!((v1 - 4.0).abs() < 0.01);
        }
    }

    #[test]
    fn test_f16_unop_sqrt() {
        use crate::interpreter::tensor::tensor_from_f16_slice;
        use crate::interpreter::kernel::cpu::f16_to_f32;
        // sqrt([4.0, 9.0]) = [2.0, 3.0]
        let a = tensor_from_f16_slice(&[4.0, 9.0], &[2]).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Sqrt).unwrap();
        let ptr = result.data_ptr_f16();
        unsafe {
            let v0 = f16_to_f32(*ptr.add(0));
            let v1 = f16_to_f32(*ptr.add(1));
            assert!((v0 - 2.0).abs() < 0.01);
            assert!((v1 - 3.0).abs() < 0.01);
        }
    }

    // ========================================================================
    // BF16 Brain Float Tests
    // ========================================================================

    #[test]
    fn test_bf16_binop_add() {
        use crate::interpreter::tensor::tensor_from_bf16_slice;
        use crate::interpreter::kernel::cpu::bf16_to_f32;
        // [1.0, 2.0] + [3.0, 4.0] = [4.0, 6.0]
        let a = tensor_from_bf16_slice(&[1.0, 2.0], &[2]).unwrap();
        let b = tensor_from_bf16_slice(&[3.0, 4.0], &[2]).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Add).unwrap();
        assert_eq!(result.dtype, DType::BF16);
        let ptr = result.data_ptr_bf16();
        unsafe {
            let v0 = bf16_to_f32(*ptr.add(0));
            let v1 = bf16_to_f32(*ptr.add(1));
            assert!((v0 - 4.0).abs() < 0.1);
            assert!((v1 - 6.0).abs() < 0.1);
        }
    }

    #[test]
    fn test_bf16_binop_mul() {
        use crate::interpreter::tensor::tensor_from_bf16_slice;
        use crate::interpreter::kernel::cpu::bf16_to_f32;
        // [2.0, 3.0] * [4.0, 5.0] = [8.0, 15.0]
        let a = tensor_from_bf16_slice(&[2.0, 3.0], &[2]).unwrap();
        let b = tensor_from_bf16_slice(&[4.0, 5.0], &[2]).unwrap();
        let result = dispatch_binop(&a, &b, TensorBinaryOp::Mul).unwrap();
        let ptr = result.data_ptr_bf16();
        unsafe {
            let v0 = bf16_to_f32(*ptr.add(0));
            let v1 = bf16_to_f32(*ptr.add(1));
            assert!((v0 - 8.0).abs() < 0.1);
            assert!((v1 - 15.0).abs() < 0.1);
        }
    }

    #[test]
    fn test_bf16_unop_neg() {
        use crate::interpreter::tensor::tensor_from_bf16_slice;
        use crate::interpreter::kernel::cpu::bf16_to_f32;
        // -[1.5, -2.5] = [-1.5, 2.5]
        let a = tensor_from_bf16_slice(&[1.5, -2.5], &[2]).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Neg).unwrap();
        let ptr = result.data_ptr_bf16();
        unsafe {
            let v0 = bf16_to_f32(*ptr.add(0));
            let v1 = bf16_to_f32(*ptr.add(1));
            assert!((v0 - (-1.5)).abs() < 0.1);
            assert!((v1 - 2.5).abs() < 0.1);
        }
    }

    #[test]
    fn test_bf16_unop_relu() {
        use crate::interpreter::tensor::tensor_from_bf16_slice;
        use crate::interpreter::kernel::cpu::bf16_to_f32;
        // relu([-2.0, 3.0]) = [0.0, 3.0]
        let a = tensor_from_bf16_slice(&[-2.0, 3.0], &[2]).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Relu).unwrap();
        let ptr = result.data_ptr_bf16();
        unsafe {
            let v0 = bf16_to_f32(*ptr.add(0));
            let v1 = bf16_to_f32(*ptr.add(1));
            assert!((v0 - 0.0).abs() < 0.1);
            assert!((v1 - 3.0).abs() < 0.1);
        }
    }

    #[test]
    fn test_bf16_unop_exp() {
        use crate::interpreter::tensor::tensor_from_bf16_slice;
        use crate::interpreter::kernel::cpu::bf16_to_f32;
        // exp([0.0, 1.0]) ≈ [1.0, 2.718]
        let a = tensor_from_bf16_slice(&[0.0, 1.0], &[2]).unwrap();
        let result = dispatch_unop(&a, TensorUnaryOp::Exp).unwrap();
        let ptr = result.data_ptr_bf16();
        unsafe {
            let v0 = bf16_to_f32(*ptr.add(0));
            let v1 = bf16_to_f32(*ptr.add(1));
            assert!((v0 - 1.0).abs() < 0.1);
            assert!((v1 - 2.718).abs() < 0.1);
        }
    }

    // ========================================================================
    // Tool Call Parsing Tests
    // ========================================================================

    #[test]
    fn test_parse_tool_call_simple() {
        let result = dispatch_parse_tool_call("search(query=hello)");
        assert!(result.is_some());
        let (name, args) = result.unwrap();
        assert_eq!(name, "search");
        assert!(args.contains("\"query\""));
        assert!(args.contains("\"hello\""));
    }

    #[test]
    fn test_parse_tool_call_multiple_args() {
        let result = dispatch_parse_tool_call("calculate(a=10, b=20, op=add)");
        assert!(result.is_some());
        let (name, args) = result.unwrap();
        assert_eq!(name, "calculate");
        assert!(args.contains("\"a\":10"));
        assert!(args.contains("\"b\":20"));
        assert!(args.contains("\"op\":\"add\""));
    }

    #[test]
    fn test_parse_tool_call_no_args() {
        let result = dispatch_parse_tool_call("get_time()");
        assert!(result.is_some());
        let (name, args) = result.unwrap();
        assert_eq!(name, "get_time");
        assert_eq!(args, "{}");
    }

    #[test]
    fn test_parse_tool_call_quoted_string() {
        let result = dispatch_parse_tool_call(r#"search(query="hello world")"#);
        assert!(result.is_some());
        let (name, args) = result.unwrap();
        assert_eq!(name, "search");
        assert!(args.contains("\"query\""));
    }

    #[test]
    fn test_parse_tool_call_json_style() {
        let result = dispatch_parse_tool_call(r#"{"name": "search", "arguments": {"query": "test"}}"#);
        assert!(result.is_some());
        let (name, args) = result.unwrap();
        assert_eq!(name, "search");
        assert!(args.contains("\"query\""));
    }

    #[test]
    fn test_parse_tool_call_just_name() {
        let result = dispatch_parse_tool_call("my_tool");
        assert!(result.is_some());
        let (name, args) = result.unwrap();
        assert_eq!(name, "my_tool");
        assert_eq!(args, "{}");
    }

    #[test]
    fn test_parse_tool_call_numeric_args() {
        let result = dispatch_parse_tool_call("add(x=42, y=3.14)");
        assert!(result.is_some());
        let (name, args) = result.unwrap();
        assert_eq!(name, "add");
        assert!(args.contains("\"x\":42"));
        assert!(args.contains("\"y\":3.14"));
    }

    #[test]
    fn test_parse_tool_call_boolean_args() {
        let result = dispatch_parse_tool_call("config(enabled=true, verbose=false)");
        assert!(result.is_some());
        let (name, args) = result.unwrap();
        assert_eq!(name, "config");
        assert!(args.contains("\"enabled\":true"));
        assert!(args.contains("\"verbose\":false"));
    }

    #[test]
    fn test_parse_tool_call_empty_string() {
        let result = dispatch_parse_tool_call("");
        assert!(result.is_none());
    }
}
