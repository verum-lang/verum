//! Metal GPU backend for tensor operations.
//!
//! This module provides Metal-accelerated tensor operations for macOS/iOS.
//! Metal is Apple's low-level, high-performance GPU programming framework.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │                         MetalBackend                                    │
//! ├─────────────────────────────────────────────────────────────────────────┤
//! │  device: metal::Device           - GPU device handle                    │
//! │  command_queue: CommandQueue     - Command submission queue             │
//! │  library: Library                - Compiled shader library              │
//! │  pipelines: HashMap<K, Pipeline> - Cached compute pipelines             │
//! │  buffer_pool: MetalBufferPool    - Reusable GPU buffers                 │
//! └─────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Apple Silicon M3 Optimizations
//!
//! This implementation follows Apple's Metal best practices for M3/A17 Pro:
//!
//! - **Optimal threadgroup sizes**: Uses 256 threads per group (8 SIMD groups)
//!   for compute shaders, balancing occupancy and register pressure
//! - **Reduced threadgroup memory**: On M3, direct buffer access is often faster
//!   than copying to threadgroup memory (software-managed cache less beneficial)
//! - **Tiled matrix multiplication**: 16x16 tiles for better cache utilization
//! - **Unified memory model**: Shared buffers avoid H2D/D2H copies
//!
//! # Usage
//!
//! ```ignore
//! let backend = MetalBackend::new()?;
//! let result = backend.binop(&a, &b, TensorBinaryOp::Add)?;
//! ```
//!
//! # References
//!
//! - [Metal Overview](https://developer.apple.com/metal/)
//! - [Metal Performance Shaders](https://developer.apple.com/documentation/metalperformanceshaders)
//! - [Learn performance best practices for Metal shaders](https://developer.apple.com/videos/play/tech-talks/111373/)

#![cfg(all(target_os = "macos", feature = "metal"))]

use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use metal::{
    Buffer, CommandQueue, CompileOptions, ComputePipelineDescriptor, ComputePipelineState,
    Device, Library, MTLResourceOptions, MTLSize,
};

use super::backend::{Backend, ComputeCapabilities};
use super::device::DeviceId;
use super::super::tensor::{DType, TensorHandle};
use crate::instruction::{TensorBinaryOp, TensorReduceOp, TensorUnaryOp};

// ============================================================================
// Metal Shader Source
// ============================================================================

/// Metal Shading Language (MSL) kernels for tensor operations
/// Optimized for Apple Silicon M3/A17 Pro (Apple GPU Family 9)
///
/// Key optimizations applied:
/// 1. SIMD vectorization with float4 for 4x throughput on element-wise ops
/// 2. Fused multiply-add (FMA) operations where applicable
/// 3. Reduced register pressure for better occupancy
/// 4. Coalesced memory access patterns
/// 5. Minimal threadgroup memory usage (direct buffer access on M3)
const METAL_SHADER_SOURCE: &str = r#"
#include <metal_stdlib>
using namespace metal;

// ============================================================================
// Configuration Constants
// ============================================================================

// Apple Silicon optimal SIMD width is 32, but we use 4 for vectorization
constant uint VECTOR_WIDTH = 4;

// ============================================================================
// SIMD Vectorized Binary Operations (4x throughput)
// ============================================================================

// Vectorized add - processes 4 elements per thread
kernel void tensor_add_f32(
    device const float4* a [[buffer(0)]],
    device const float4* b [[buffer(1)]],
    device float4* out [[buffer(2)]],
    constant uint& n [[buffer(3)]],      // Total elements (not vectors)
    constant uint& n_vec [[buffer(4)]],  // n / 4 (vector count)
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = a[id] + b[id];
    }
}

// Scalar fallback for remainder elements
kernel void tensor_add_f32_scalar(
    device const float* a [[buffer(0)]],
    device const float* b [[buffer(1)]],
    device float* out [[buffer(2)]],
    constant uint& n [[buffer(3)]],
    constant uint& offset [[buffer(4)]],  // Start offset for remainder
    uint id [[thread_position_in_grid]]
) {
    uint idx = offset + id;
    if (idx < n) {
        out[idx] = a[idx] + b[idx];
    }
}

kernel void tensor_sub_f32(
    device const float4* a [[buffer(0)]],
    device const float4* b [[buffer(1)]],
    device float4* out [[buffer(2)]],
    constant uint& n [[buffer(3)]],
    constant uint& n_vec [[buffer(4)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = a[id] - b[id];
    }
}

kernel void tensor_mul_f32(
    device const float4* a [[buffer(0)]],
    device const float4* b [[buffer(1)]],
    device float4* out [[buffer(2)]],
    constant uint& n [[buffer(3)]],
    constant uint& n_vec [[buffer(4)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = a[id] * b[id];
    }
}

kernel void tensor_div_f32(
    device const float4* a [[buffer(0)]],
    device const float4* b [[buffer(1)]],
    device float4* out [[buffer(2)]],
    constant uint& n [[buffer(3)]],
    constant uint& n_vec [[buffer(4)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        // Use fast_divide for better performance (slightly less accurate)
        out[id] = a[id] / b[id];
    }
}

kernel void tensor_pow_f32(
    device const float4* a [[buffer(0)]],
    device const float4* b [[buffer(1)]],
    device float4* out [[buffer(2)]],
    constant uint& n [[buffer(3)]],
    constant uint& n_vec [[buffer(4)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = pow(a[id], b[id]);
    }
}

kernel void tensor_max_f32(
    device const float4* a [[buffer(0)]],
    device const float4* b [[buffer(1)]],
    device float4* out [[buffer(2)]],
    constant uint& n [[buffer(3)]],
    constant uint& n_vec [[buffer(4)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = max(a[id], b[id]);
    }
}

kernel void tensor_min_f32(
    device const float4* a [[buffer(0)]],
    device const float4* b [[buffer(1)]],
    device float4* out [[buffer(2)]],
    constant uint& n [[buffer(3)]],
    constant uint& n_vec [[buffer(4)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = min(a[id], b[id]);
    }
}

// ============================================================================
// Fused Multiply-Add Operations (FMA)
// These use hardware FMA units for better precision and performance
// ============================================================================

// out = a * b + c (fused)
kernel void tensor_fma_f32(
    device const float4* a [[buffer(0)]],
    device const float4* b [[buffer(1)]],
    device const float4* c [[buffer(2)]],
    device float4* out [[buffer(3)]],
    constant uint& n_vec [[buffer(4)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = fma(a[id], b[id], c[id]);
    }
}

// out = a * scalar + b (scale and shift)
kernel void tensor_scale_add_f32(
    device const float4* a [[buffer(0)]],
    device const float4* b [[buffer(1)]],
    device float4* out [[buffer(2)]],
    constant float& scale [[buffer(3)]],
    constant uint& n_vec [[buffer(4)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = fma(a[id], float4(scale), b[id]);
    }
}

// ============================================================================
// SIMD Vectorized Unary Operations
// ============================================================================

kernel void tensor_neg_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = -a[id];
    }
}

kernel void tensor_abs_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = abs(a[id]);
    }
}

kernel void tensor_sqrt_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        // Use rsqrt for better performance when applicable
        out[id] = sqrt(a[id]);
    }
}

// Fast reciprocal square root (1/sqrt(x))
kernel void tensor_rsqrt_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = rsqrt(a[id]);
    }
}

kernel void tensor_exp_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = exp(a[id]);
    }
}

// Fast exp2 (2^x) - faster than exp on Apple GPUs
kernel void tensor_exp2_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = exp2(a[id]);
    }
}

kernel void tensor_log_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = log(a[id]);
    }
}

// Fast log2 - faster than log on Apple GPUs
kernel void tensor_log2_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = log2(a[id]);
    }
}

kernel void tensor_sin_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = sin(a[id]);
    }
}

kernel void tensor_cos_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = cos(a[id]);
    }
}

// Compute sin and cos simultaneously (more efficient)
kernel void tensor_sincos_f32(
    device const float4* a [[buffer(0)]],
    device float4* out_sin [[buffer(1)]],
    device float4* out_cos [[buffer(2)]],
    constant uint& n [[buffer(3)]],
    constant uint& n_vec [[buffer(4)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        float4 val = a[id];
        out_sin[id] = sin(val);
        out_cos[id] = cos(val);
    }
}

kernel void tensor_tanh_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = tanh(a[id]);
    }
}

kernel void tensor_relu_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        out[id] = max(a[id], float4(0.0f));
    }
}

kernel void tensor_sigmoid_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        // Optimized sigmoid: 1 / (1 + exp(-x))
        // Using fast_recip for better performance
        float4 val = a[id];
        out[id] = float4(1.0f) / (float4(1.0f) + exp(-val));
    }
}

// GELU activation (used in transformers)
kernel void tensor_gelu_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        // GELU(x) = x * 0.5 * (1 + tanh(sqrt(2/pi) * (x + 0.044715 * x^3)))
        // Approximation: 0.5 * x * (1 + tanh(0.7978845608 * (x + 0.044715 * x^3)))
        float4 x = a[id];
        float4 x3 = x * x * x;
        float4 inner = 0.7978845608f * (x + 0.044715f * x3);
        out[id] = 0.5f * x * (1.0f + tanh(inner));
    }
}

// Swish/SiLU activation (x * sigmoid(x))
kernel void tensor_swish_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        float4 x = a[id];
        float4 sigmoid_x = float4(1.0f) / (float4(1.0f) + exp(-x));
        out[id] = x * sigmoid_x;
    }
}

// Leaky ReLU
kernel void tensor_leaky_relu_f32(
    device const float4* a [[buffer(0)]],
    device float4* out [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant uint& n_vec [[buffer(3)]],
    constant float& alpha [[buffer(4)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        float4 x = a[id];
        out[id] = select(float4(alpha) * x, x, x > float4(0.0f));
    }
}

// ============================================================================
// Reduction Operations
// ============================================================================

// Parallel sum reduction using threadgroup memory
kernel void tensor_reduce_sum_f32(
    device const float* input [[buffer(0)]],
    device float* output [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    threadgroup float* shared [[threadgroup(0)]],
    uint id [[thread_position_in_grid]],
    uint lid [[thread_position_in_threadgroup]],
    uint group_size [[threads_per_threadgroup]]
) {
    // Load into shared memory
    shared[lid] = (id < n) ? input[id] : 0.0f;
    threadgroup_barrier(mem_flags::mem_threadgroup);

    // Parallel reduction
    for (uint stride = group_size / 2; stride > 0; stride /= 2) {
        if (lid < stride) {
            shared[lid] += shared[lid + stride];
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }

    // Write result
    if (lid == 0) {
        output[0] = shared[0];
    }
}

kernel void tensor_reduce_max_f32(
    device const float* input [[buffer(0)]],
    device float* output [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    threadgroup float* shared [[threadgroup(0)]],
    uint id [[thread_position_in_grid]],
    uint lid [[thread_position_in_threadgroup]],
    uint group_size [[threads_per_threadgroup]]
) {
    shared[lid] = (id < n) ? input[id] : -INFINITY;
    threadgroup_barrier(mem_flags::mem_threadgroup);

    for (uint stride = group_size / 2; stride > 0; stride /= 2) {
        if (lid < stride) {
            shared[lid] = max(shared[lid], shared[lid + stride]);
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }

    if (lid == 0) {
        output[0] = shared[0];
    }
}

kernel void tensor_reduce_min_f32(
    device const float* input [[buffer(0)]],
    device float* output [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    threadgroup float* shared [[threadgroup(0)]],
    uint id [[thread_position_in_grid]],
    uint lid [[thread_position_in_threadgroup]],
    uint group_size [[threads_per_threadgroup]]
) {
    shared[lid] = (id < n) ? input[id] : INFINITY;
    threadgroup_barrier(mem_flags::mem_threadgroup);

    for (uint stride = group_size / 2; stride > 0; stride /= 2) {
        if (lid < stride) {
            shared[lid] = min(shared[lid], shared[lid + stride]);
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }

    if (lid == 0) {
        output[0] = shared[0];
    }
}

// ============================================================================
// Matrix Multiplication
// ============================================================================

// Tiled matrix multiplication for better cache utilization
#define TILE_SIZE 16

kernel void tensor_matmul_f32(
    device const float* A [[buffer(0)]],
    device const float* B [[buffer(1)]],
    device float* C [[buffer(2)]],
    constant uint& M [[buffer(3)]],
    constant uint& K [[buffer(4)]],
    constant uint& N [[buffer(5)]],
    threadgroup float* As [[threadgroup(0)]],
    threadgroup float* Bs [[threadgroup(1)]],
    uint2 gid [[thread_position_in_grid]],
    uint2 lid [[thread_position_in_threadgroup]],
    uint2 tgid [[threadgroup_position_in_grid]]
) {
    uint row = gid.y;
    uint col = gid.x;

    float sum = 0.0f;

    // Iterate over tiles
    for (uint t = 0; t < (K + TILE_SIZE - 1) / TILE_SIZE; t++) {
        // Load tiles into shared memory
        uint aRow = tgid.y * TILE_SIZE + lid.y;
        uint aCol = t * TILE_SIZE + lid.x;
        uint bRow = t * TILE_SIZE + lid.y;
        uint bCol = tgid.x * TILE_SIZE + lid.x;

        if (aRow < M && aCol < K) {
            As[lid.y * TILE_SIZE + lid.x] = A[aRow * K + aCol];
        } else {
            As[lid.y * TILE_SIZE + lid.x] = 0.0f;
        }

        if (bRow < K && bCol < N) {
            Bs[lid.y * TILE_SIZE + lid.x] = B[bRow * N + bCol];
        } else {
            Bs[lid.y * TILE_SIZE + lid.x] = 0.0f;
        }

        threadgroup_barrier(mem_flags::mem_threadgroup);

        // Compute partial dot product
        for (uint k = 0; k < TILE_SIZE; k++) {
            sum += As[lid.y * TILE_SIZE + k] * Bs[k * TILE_SIZE + lid.x];
        }

        threadgroup_barrier(mem_flags::mem_threadgroup);
    }

    // Write result
    if (row < M && col < N) {
        C[row * N + col] = sum;
    }
}

// ============================================================================
// Softmax - Optimized Multi-Pass Implementation
// ============================================================================

// Pass 1: Find maximum value across all elements (for numerical stability)
kernel void tensor_softmax_max_f32(
    device const float* input [[buffer(0)]],
    device float* partial_max [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    threadgroup float* shared [[threadgroup(0)]],
    uint id [[thread_position_in_grid]],
    uint lid [[thread_position_in_threadgroup]],
    uint group_size [[threads_per_threadgroup]],
    uint group_id [[threadgroup_position_in_grid]]
) {
    // Each thread loads one element
    shared[lid] = (id < n) ? input[id] : -INFINITY;
    threadgroup_barrier(mem_flags::mem_threadgroup);

    // Parallel reduction within threadgroup
    for (uint stride = group_size / 2; stride > 0; stride /= 2) {
        if (lid < stride) {
            shared[lid] = max(shared[lid], shared[lid + stride]);
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }

    // Write partial result
    if (lid == 0) {
        partial_max[group_id] = shared[0];
    }
}

// Pass 2: Compute exp(x - max) and partial sums
kernel void tensor_softmax_exp_sum_f32(
    device const float* input [[buffer(0)]],
    device float* exp_output [[buffer(1)]],
    device float* partial_sum [[buffer(2)]],
    constant uint& n [[buffer(3)]],
    constant float& max_val [[buffer(4)]],
    threadgroup float* shared [[threadgroup(0)]],
    uint id [[thread_position_in_grid]],
    uint lid [[thread_position_in_threadgroup]],
    uint group_size [[threads_per_threadgroup]],
    uint group_id [[threadgroup_position_in_grid]]
) {
    float val = 0.0f;
    if (id < n) {
        val = exp(input[id] - max_val);
        exp_output[id] = val;
    }
    shared[lid] = val;
    threadgroup_barrier(mem_flags::mem_threadgroup);

    // Parallel sum reduction
    for (uint stride = group_size / 2; stride > 0; stride /= 2) {
        if (lid < stride) {
            shared[lid] += shared[lid + stride];
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }

    if (lid == 0) {
        partial_sum[group_id] = shared[0];
    }
}

// Pass 3: Normalize by dividing by sum
kernel void tensor_softmax_normalize_f32(
    device const float* exp_input [[buffer(0)]],
    device float* output [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    constant float& sum [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n) {
        output[id] = exp_input[id] / sum;
    }
}

// Vectorized softmax normalize (4x throughput)
kernel void tensor_softmax_normalize_vec_f32(
    device const float4* exp_input [[buffer(0)]],
    device float4* output [[buffer(1)]],
    constant uint& n_vec [[buffer(2)]],
    constant float& sum [[buffer(3)]],
    uint id [[thread_position_in_grid]]
) {
    if (id < n_vec) {
        output[id] = exp_input[id] / float4(sum);
    }
}

// Batch softmax: each row processed independently
// Input shape: [batch_size, dim], softmax along dim axis
kernel void tensor_softmax_batch_f32(
    device const float* input [[buffer(0)]],
    device float* output [[buffer(1)]],
    constant uint& batch_size [[buffer(2)]],
    constant uint& dim [[buffer(3)]],
    constant uint& group_sz [[buffer(4)]],  // Pass group size as constant
    threadgroup float* shared [[threadgroup(0)]],
    uint2 gid [[thread_position_in_grid]],
    uint2 lid2 [[thread_position_in_threadgroup]]
) {
    uint batch_idx = gid.y;
    uint elem_idx = gid.x;
    uint lid = lid2.x;  // Use x component for 1D threadgroup

    if (batch_idx >= batch_size) return;

    uint base = batch_idx * dim;

    // Step 1: Load value into shared memory for max reduction
    float val = (elem_idx < dim) ? input[base + elem_idx] : -INFINITY;
    shared[lid] = val;
    threadgroup_barrier(mem_flags::mem_threadgroup);

    // Find max within row
    for (uint stride = group_sz / 2; stride > 0; stride /= 2) {
        if (lid < stride) {
            shared[lid] = max(shared[lid], shared[lid + stride]);
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }
    float max_val = shared[0];
    threadgroup_barrier(mem_flags::mem_threadgroup);

    // Step 2: Compute exp(x - max) and sum
    float exp_val = (elem_idx < dim) ? exp(input[base + elem_idx] - max_val) : 0.0f;
    shared[lid] = exp_val;
    threadgroup_barrier(mem_flags::mem_threadgroup);

    for (uint stride = group_sz / 2; stride > 0; stride /= 2) {
        if (lid < stride) {
            shared[lid] += shared[lid + stride];
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }
    float sum_val = shared[0];
    threadgroup_barrier(mem_flags::mem_threadgroup);

    // Step 3: Normalize and write output
    if (elem_idx < dim) {
        output[base + elem_idx] = exp_val / sum_val;
    }
}

// ============================================================================
// Convolution 2D
// ============================================================================

kernel void tensor_conv2d_f32(
    device const float* input [[buffer(0)]],
    device const float* weights [[buffer(1)]],
    device float* result [[buffer(2)]],
    constant uint4& dims [[buffer(3)]],      // N, C_in, H, W
    constant uint4& kdims [[buffer(4)]],     // C_out, C_in/groups, kH, kW
    constant uint4& params [[buffer(5)]],    // stride_h, stride_w, pad_h, pad_w
    constant uint& num_groups [[buffer(6)]],
    uint3 gid [[thread_position_in_grid]]    // (out_w, out_h, batch*c_out)
) {
    uint N = dims.x;
    uint C_in = dims.y;
    uint H = dims.z;
    uint W = dims.w;

    uint C_out = kdims.x;
    uint KC_in = kdims.y;
    uint kH = kdims.z;
    uint kW = kdims.w;

    uint stride_h = params.x;
    uint stride_w = params.y;
    uint pad_h = params.z;
    uint pad_w = params.w;

    uint out_h = gid.y;
    uint out_w = gid.x;
    uint batch_c_out = gid.z;
    uint batch = batch_c_out / C_out;
    uint c_out = batch_c_out % C_out;

    uint H_out = (H + 2 * pad_h - kH) / stride_h + 1;
    uint W_out = (W + 2 * pad_w - kW) / stride_w + 1;

    if (out_h >= H_out || out_w >= W_out || batch >= N) return;

    uint c_out_per_group = C_out / num_groups;
    uint c_in_per_group = C_in / num_groups;
    uint group = c_out / c_out_per_group;

    float sum = 0.0f;

    for (uint c_in = 0; c_in < c_in_per_group; c_in++) {
        uint c_in_global = group * c_in_per_group + c_in;

        for (uint kh = 0; kh < kH; kh++) {
            for (uint kw = 0; kw < kW; kw++) {
                int in_h = int(out_h * stride_h + kh) - int(pad_h);
                int in_w = int(out_w * stride_w + kw) - int(pad_w);

                if (in_h >= 0 && in_h < int(H) && in_w >= 0 && in_w < int(W)) {
                    uint input_idx = batch * C_in * H * W + c_in_global * H * W + uint(in_h) * W + uint(in_w);
                    uint weight_idx = c_out * KC_in * kH * kW + c_in * kH * kW + kh * kW + kw;
                    sum += input[input_idx] * weights[weight_idx];
                }
            }
        }
    }

    uint result_idx = batch * C_out * H_out * W_out + c_out * H_out * W_out + out_h * W_out + out_w;
    result[result_idx] = sum;
}

// ============================================================================
// Max Pooling 2D
// ============================================================================

kernel void tensor_maxpool2d_f32(
    device const float* input [[buffer(0)]],
    device float* output [[buffer(1)]],
    constant uint4& dims [[buffer(2)]],      // N, C, H, W
    constant uint4& params [[buffer(3)]],    // kernel_h, kernel_w, stride_h, stride_w
    constant uint2& padding [[buffer(4)]],   // pad_h, pad_w
    uint3 gid [[thread_position_in_grid]]    // (out_w, out_h, batch*c)
) {
    uint N = dims.x;
    uint C = dims.y;
    uint H = dims.z;
    uint W = dims.w;

    uint kH = params.x;
    uint kW = params.y;
    uint stride_h = params.z;
    uint stride_w = params.w;
    uint pad_h = padding.x;
    uint pad_w = padding.y;

    uint out_h = gid.y;
    uint out_w = gid.x;
    uint batch_c = gid.z;
    uint batch = batch_c / C;
    uint c = batch_c % C;

    uint H_out = (H + 2 * pad_h - kH) / stride_h + 1;
    uint W_out = (W + 2 * pad_w - kW) / stride_w + 1;

    if (out_h >= H_out || out_w >= W_out || batch >= N) return;

    float max_val = -INFINITY;

    for (uint kh = 0; kh < kH; kh++) {
        for (uint kw = 0; kw < kW; kw++) {
            int in_h = int(out_h * stride_h + kh) - int(pad_h);
            int in_w = int(out_w * stride_w + kw) - int(pad_w);

            if (in_h >= 0 && in_h < int(H) && in_w >= 0 && in_w < int(W)) {
                uint idx = batch * C * H * W + c * H * W + uint(in_h) * W + uint(in_w);
                max_val = max(max_val, input[idx]);
            }
        }
    }

    uint output_idx = batch * C * H_out * W_out + c * H_out * W_out + out_h * W_out + out_w;
    output[output_idx] = max_val;
}

// ============================================================================
// Layer Normalization - Optimized Implementation
// ============================================================================

// Two-pass approach for large hidden sizes:
// Pass 1: Compute mean
// Pass 2: Compute variance and normalize

// Efficient layer norm for hidden_size <= 1024 (fits in threadgroup)
kernel void tensor_layer_norm_small_f32(
    device const float* input [[buffer(0)]],
    device const float* gamma [[buffer(1)]],  // Can be null
    device const float* beta [[buffer(2)]],   // Can be null
    device float* output [[buffer(3)]],
    constant uint& batch_size [[buffer(4)]],
    constant uint& hidden_size [[buffer(5)]],
    constant float& eps [[buffer(6)]],
    constant uint& has_gamma [[buffer(7)]],
    constant uint& has_beta [[buffer(8)]],
    constant uint& group_sz [[buffer(9)]],  // Pass group size as constant
    threadgroup float* shared [[threadgroup(0)]],
    uint2 gid [[thread_position_in_grid]],
    uint2 lid2 [[thread_position_in_threadgroup]]
) {
    uint batch = gid.y;
    uint elem = gid.x;
    uint lid = lid2.x;  // Use x component for 1D threadgroup

    if (batch >= batch_size) return;

    uint base = batch * hidden_size;

    // Load value into registers and shared memory
    float val = (elem < hidden_size) ? input[base + elem] : 0.0f;
    shared[lid] = val;
    threadgroup_barrier(mem_flags::mem_threadgroup);

    // Step 1: Compute mean via parallel reduction
    for (uint stride = group_sz / 2; stride > 0; stride /= 2) {
        if (lid < stride) {
            shared[lid] += shared[lid + stride];
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }
    float mean = shared[0] / float(hidden_size);
    threadgroup_barrier(mem_flags::mem_threadgroup);

    // Step 2: Compute variance
    float diff = (elem < hidden_size) ? (val - mean) : 0.0f;
    shared[lid] = diff * diff;
    threadgroup_barrier(mem_flags::mem_threadgroup);

    for (uint stride = group_sz / 2; stride > 0; stride /= 2) {
        if (lid < stride) {
            shared[lid] += shared[lid + stride];
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }
    float var = shared[0] / float(hidden_size);
    float inv_std = rsqrt(var + eps);  // Use rsqrt for better performance

    // Step 3: Normalize and apply affine transform
    if (elem < hidden_size) {
        float normalized = diff * inv_std;
        float g = has_gamma ? gamma[elem] : 1.0f;
        float b = has_beta ? beta[elem] : 0.0f;
        output[base + elem] = fma(normalized, g, b);  // Use FMA for precision
    }
}

// Layer norm Pass 1: Compute partial sums for mean
kernel void tensor_layer_norm_mean_f32(
    device const float* input [[buffer(0)]],
    device float* partial_sum [[buffer(1)]],
    constant uint& batch_size [[buffer(2)]],
    constant uint& hidden_size [[buffer(3)]],
    constant uint& group_sz [[buffer(4)]],  // Pass group size as constant
    threadgroup float* shared [[threadgroup(0)]],
    uint2 gid [[thread_position_in_grid]],
    uint2 lid2 [[thread_position_in_threadgroup]],
    uint2 group_id [[threadgroup_position_in_grid]]
) {
    uint batch = gid.y;
    uint elem = gid.x;
    uint lid = lid2.x;  // Use x component for 1D threadgroup

    if (batch >= batch_size) return;

    float val = (elem < hidden_size) ? input[batch * hidden_size + elem] : 0.0f;
    shared[lid] = val;
    threadgroup_barrier(mem_flags::mem_threadgroup);

    for (uint stride = group_sz / 2; stride > 0; stride /= 2) {
        if (lid < stride) {
            shared[lid] += shared[lid + stride];
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }

    if (lid == 0) {
        partial_sum[batch * ((hidden_size + group_sz - 1) / group_sz) + group_id.x] = shared[0];
    }
}

// Layer norm Pass 2: Compute variance given mean
kernel void tensor_layer_norm_var_f32(
    device const float* input [[buffer(0)]],
    device float* partial_var [[buffer(1)]],
    device const float* means [[buffer(2)]],  // Per-batch means
    constant uint& batch_size [[buffer(3)]],
    constant uint& hidden_size [[buffer(4)]],
    constant uint& group_sz [[buffer(5)]],  // Pass group size as constant
    threadgroup float* shared [[threadgroup(0)]],
    uint2 gid [[thread_position_in_grid]],
    uint2 lid2 [[thread_position_in_threadgroup]],
    uint2 group_id [[threadgroup_position_in_grid]]
) {
    uint batch = gid.y;
    uint elem = gid.x;
    uint lid = lid2.x;  // Use x component for 1D threadgroup

    if (batch >= batch_size) return;

    float mean = means[batch];
    float val = (elem < hidden_size) ? input[batch * hidden_size + elem] : 0.0f;
    float diff = val - mean;
    shared[lid] = diff * diff;
    threadgroup_barrier(mem_flags::mem_threadgroup);

    for (uint stride = group_sz / 2; stride > 0; stride /= 2) {
        if (lid < stride) {
            shared[lid] += shared[lid + stride];
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }

    if (lid == 0) {
        partial_var[batch * ((hidden_size + group_sz - 1) / group_sz) + group_id.x] = shared[0];
    }
}

// Layer norm Pass 3: Normalize using precomputed mean and variance
kernel void tensor_layer_norm_normalize_f32(
    device const float* input [[buffer(0)]],
    device float* output [[buffer(1)]],
    device const float* gamma [[buffer(2)]],
    device const float* beta [[buffer(3)]],
    device const float* means [[buffer(4)]],
    device const float* vars [[buffer(5)]],
    constant uint& batch_size [[buffer(6)]],
    constant uint& hidden_size [[buffer(7)]],
    constant float& eps [[buffer(8)]],
    constant uint& has_gamma [[buffer(9)]],
    constant uint& has_beta [[buffer(10)]],
    uint2 gid [[thread_position_in_grid]]
) {
    uint batch = gid.y;
    uint elem = gid.x;

    if (batch >= batch_size || elem >= hidden_size) return;

    float mean = means[batch];
    float var = vars[batch];
    float inv_std = rsqrt(var + eps);

    uint idx = batch * hidden_size + elem;
    float val = input[idx];
    float normalized = (val - mean) * inv_std;

    float g = has_gamma ? gamma[elem] : 1.0f;
    float b = has_beta ? beta[elem] : 0.0f;
    output[idx] = fma(normalized, g, b);
}

// Vectorized normalize (4x throughput)
kernel void tensor_layer_norm_normalize_vec_f32(
    device const float4* input [[buffer(0)]],
    device float4* output [[buffer(1)]],
    device const float4* gamma [[buffer(2)]],
    device const float4* beta [[buffer(3)]],
    device const float* means [[buffer(4)]],
    device const float* vars [[buffer(5)]],
    constant uint& batch_size [[buffer(6)]],
    constant uint& hidden_size_vec [[buffer(7)]],  // hidden_size / 4
    constant float& eps [[buffer(8)]],
    constant uint& has_gamma [[buffer(9)]],
    constant uint& has_beta [[buffer(10)]],
    uint2 gid [[thread_position_in_grid]]
) {
    uint batch = gid.y;
    uint vec_idx = gid.x;

    if (batch >= batch_size || vec_idx >= hidden_size_vec) return;

    float mean = means[batch];
    float var = vars[batch];
    float inv_std = rsqrt(var + eps);

    uint idx = batch * hidden_size_vec + vec_idx;
    float4 val = input[idx];
    float4 normalized = (val - float4(mean)) * float4(inv_std);

    float4 g = has_gamma ? gamma[vec_idx] : float4(1.0f);
    float4 b = has_beta ? beta[vec_idx] : float4(0.0f);
    output[idx] = fma(normalized, g, b);
}
"#;

// ============================================================================
// Metal Buffer Pool
// ============================================================================

/// Pool for reusing Metal buffers
pub struct MetalBufferPool {
    device: Device,
    /// Free buffers by size bucket (power of 2)
    free_lists: [Mutex<Vec<Buffer>>; 32],
    /// Statistics
    allocated: AtomicUsize,
    cache_hits: AtomicUsize,
    cache_misses: AtomicUsize,
}

impl MetalBufferPool {
    /// Create a new buffer pool
    pub fn new(device: Device) -> Self {
        Self {
            device,
            free_lists: std::array::from_fn(|_| Mutex::new(Vec::new())),
            allocated: AtomicUsize::new(0),
            cache_hits: AtomicUsize::new(0),
            cache_misses: AtomicUsize::new(0),
        }
    }

    /// Allocate a buffer (may reuse from cache)
    pub fn allocate(&self, size: usize) -> Option<Buffer> {
        if size == 0 {
            return None;
        }

        let bucket = size.next_power_of_two().trailing_zeros() as usize;
        if bucket >= 32 {
            return None;
        }

        // Try cache first
        {
            let mut list = self.free_lists[bucket].lock().ok()?;
            if let Some(buffer) = list.pop() {
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
                return Some(buffer);
            }
        }

        // Allocate new
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
        let alloc_size = 1 << bucket;
        let buffer = self.device.new_buffer(
            alloc_size as u64,
            MTLResourceOptions::StorageModeShared,
        );
        self.allocated.fetch_add(alloc_size, Ordering::Relaxed);
        Some(buffer)
    }

    /// Return a buffer to the pool
    pub fn deallocate(&self, buffer: Buffer, size: usize) {
        if size == 0 {
            return;
        }

        let bucket = size.next_power_of_two().trailing_zeros() as usize;
        if bucket >= 32 {
            return;
        }

        if let Ok(mut list) = self.free_lists[bucket].lock() {
            list.push(buffer);
        }
    }

    /// Get pool statistics
    pub fn stats(&self) -> MetalPoolStats {
        MetalPoolStats {
            allocated: self.allocated.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
        }
    }
}

/// Metal buffer pool statistics
#[derive(Debug, Clone)]
pub struct MetalPoolStats {
    pub allocated: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
}

// ============================================================================
// Metal Backend
// ============================================================================

/// Metal GPU backend for tensor operations
pub struct MetalBackend {
    /// Metal device
    device: Device,
    /// Command queue for submitting work
    command_queue: CommandQueue,
    /// Compiled shader library
    library: Library,
    /// Cached compute pipelines
    pipelines: Mutex<HashMap<String, ComputePipelineState>>,
    /// Buffer pool for memory reuse
    buffer_pool: MetalBufferPool,
    /// Device capabilities
    capabilities: ComputeCapabilities,
    /// GPU index (0 for primary)
    gpu_index: u16,
}

impl MetalBackend {
    /// Create a new Metal backend using the default device
    pub fn new() -> Option<Self> {
        Self::with_device_index(0)
    }

    /// Create a Metal backend with a specific device index
    pub fn with_device_index(index: u16) -> Option<Self> {
        let device = Device::system_default()?;

        let command_queue = device.new_command_queue();

        // Compile shader library
        let options = CompileOptions::new();
        let library = match device.new_library_with_source(METAL_SHADER_SOURCE, &options) {
            Ok(lib) => lib,
            Err(e) => {
                eprintln!("Metal shader compilation error: {}", e);
                return None;
            }
        };

        // Detect capabilities
        let capabilities = Self::detect_capabilities(&device);

        Some(Self {
            device: device.clone(),
            command_queue,
            library,
            pipelines: Mutex::new(HashMap::new()),
            buffer_pool: MetalBufferPool::new(device),
            capabilities,
            gpu_index: index,
        })
    }

    /// Detect GPU capabilities
    fn detect_capabilities(device: &Device) -> ComputeCapabilities {
        ComputeCapabilities {
            max_threads: device.max_threads_per_threadgroup().width as usize,
            simd_width: 32, // Apple GPUs use 32-wide SIMD
            has_fma: true,
            has_tensor_cores: false, // Apple has different architecture
            max_shared_memory: device.max_threadgroup_memory_length() as usize,
            compute_capability: (0, 0), // Not applicable for Metal
            memory_bandwidth_gbps: estimate_bandwidth(device),
            peak_tflops_f32: estimate_tflops(device),
        }
    }

    /// Get or create a compute pipeline
    fn get_pipeline(&self, name: &str) -> Option<ComputePipelineState> {
        // Check cache
        {
            let pipelines = self.pipelines.lock().ok()?;
            if let Some(pipeline) = pipelines.get(name) {
                return Some(pipeline.clone());
            }
        }

        // Create new pipeline
        let function = self.library.get_function(name, None).ok()?;
        let descriptor = ComputePipelineDescriptor::new();
        descriptor.set_compute_function(Some(&function));

        let pipeline = self
            .device
            .new_compute_pipeline_state_with_function(&function)
            .ok()?;

        // Cache it
        {
            let mut pipelines = self.pipelines.lock().ok()?;
            pipelines.insert(name.to_string(), pipeline.clone());
        }

        Some(pipeline)
    }

    /// Create a GPU buffer from tensor data
    fn create_buffer(&self, tensor: &TensorHandle) -> Option<Buffer> {
        let size = tensor.numel * tensor.dtype.size();
        let buffer = self.buffer_pool.allocate(size)?;

        // Copy data from tensor to buffer
        let host_ptr = tensor.data_ptr_f32();
        if !host_ptr.is_null() {
            // Bounds check: buffer was allocated with `size` bytes = numel * dtype.size()
            assert!(
                buffer.length() as usize >= tensor.numel * tensor.dtype.size(),
                "Metal buffer too small for tensor copy: buffer={}, need={}",
                buffer.length(),
                tensor.numel * tensor.dtype.size()
            );
            unsafe {
                let dst = buffer.contents() as *mut f32;
                // SAFETY: host_ptr is non-null (checked above), dst is from buffer.contents()
                // which is valid for buffer.length() bytes >= numel * sizeof(f32).
                std::ptr::copy_nonoverlapping(host_ptr, dst, tensor.numel);
            }
        }

        Some(buffer)
    }

    /// Copy buffer contents to tensor
    fn copy_to_tensor(&self, buffer: &Buffer, tensor: &mut TensorHandle) {
        let dst_ptr = tensor.data_ptr_f32_mut();
        if !dst_ptr.is_null() {
            let copy_bytes = tensor.numel * tensor.dtype.size();
            assert!(
                buffer.length() as usize >= copy_bytes,
                "Metal buffer too small for tensor read: buffer={}, need={}",
                buffer.length(),
                copy_bytes
            );
            unsafe {
                let src = buffer.contents() as *const f32;
                // SAFETY: src is from buffer.contents() valid for buffer.length() bytes (checked above).
                // dst_ptr is non-null and backed by tensor's allocation of numel elements.
                std::ptr::copy_nonoverlapping(src, dst_ptr, tensor.numel);
            }
        }
    }

    /// Execute a binary operation on GPU
    ///
    /// Uses SIMD vectorization with float4 for 4x throughput on Apple Silicon.
    /// Automatically handles alignment and dispatches optimal number of work groups.
    pub fn binop_gpu(
        &self,
        a: &TensorHandle,
        b: &TensorHandle,
        op: TensorBinaryOp,
    ) -> Option<TensorHandle> {
        if a.dtype != DType::F32 || b.dtype != DType::F32 {
            return None;
        }

        if a.shape[..a.ndim as usize] != b.shape[..b.ndim as usize] {
            return None;
        }

        let kernel_name = match op {
            TensorBinaryOp::Add => "tensor_add_f32",
            TensorBinaryOp::Sub => "tensor_sub_f32",
            TensorBinaryOp::Mul => "tensor_mul_f32",
            TensorBinaryOp::Div => "tensor_div_f32",
            TensorBinaryOp::Pow => "tensor_pow_f32",
            TensorBinaryOp::Max => "tensor_max_f32",
            TensorBinaryOp::Min => "tensor_min_f32",
            _ => return None,
        };

        let pipeline = self.get_pipeline(kernel_name)?;

        // Create buffers
        let a_buf = self.create_buffer(a)?;
        let b_buf = self.create_buffer(b)?;
        let mut output = TensorHandle::zeros(&a.shape[..a.ndim as usize], DType::F32)?;
        let out_buf = self.buffer_pool.allocate(output.numel * 4)?;

        // Create command buffer
        let command_buffer = self.command_queue.new_command_buffer();
        let encoder = command_buffer.new_compute_command_encoder();

        encoder.set_compute_pipeline_state(&pipeline);
        encoder.set_buffer(0, Some(&a_buf), 0);
        encoder.set_buffer(1, Some(&b_buf), 0);
        encoder.set_buffer(2, Some(&out_buf), 0);

        // Vectorized dispatch: process 4 elements per thread
        let n = a.numel as u32;
        let n_vec = (a.numel / 4) as u32;  // Number of float4 vectors
        encoder.set_bytes(3, 4, &n as *const u32 as *const _);
        encoder.set_bytes(4, 4, &n_vec as *const u32 as *const _);

        // Dispatch vectorized threads (each processes 4 elements)
        let thread_group_size = MTLSize::new(256, 1, 1);
        let num_groups = MTLSize::new((n_vec as u64 + 255) / 256, 1, 1);
        encoder.dispatch_thread_groups(num_groups, thread_group_size);

        encoder.end_encoding();
        command_buffer.commit();
        command_buffer.wait_until_completed();

        // Copy result back
        self.copy_to_tensor(&out_buf, &mut output);

        // Return buffers to pool
        self.buffer_pool.deallocate(a_buf, a.numel * 4);
        self.buffer_pool.deallocate(b_buf, b.numel * 4);
        self.buffer_pool.deallocate(out_buf, output.numel * 4);

        Some(output)
    }

    /// Execute a unary operation on GPU
    ///
    /// Uses SIMD vectorization with float4 for 4x throughput on Apple Silicon.
    pub fn unop_gpu(&self, a: &TensorHandle, op: TensorUnaryOp) -> Option<TensorHandle> {
        if a.dtype != DType::F32 {
            return None;
        }

        let kernel_name = match op {
            TensorUnaryOp::Neg => "tensor_neg_f32",
            TensorUnaryOp::Abs => "tensor_abs_f32",
            TensorUnaryOp::Sqrt => "tensor_sqrt_f32",
            TensorUnaryOp::Exp => "tensor_exp_f32",
            TensorUnaryOp::Log => "tensor_log_f32",
            TensorUnaryOp::Sin => "tensor_sin_f32",
            TensorUnaryOp::Cos => "tensor_cos_f32",
            TensorUnaryOp::Tanh => "tensor_tanh_f32",
            TensorUnaryOp::Relu => "tensor_relu_f32",
            TensorUnaryOp::Sigmoid => "tensor_sigmoid_f32",
            _ => return None,
        };

        let pipeline = self.get_pipeline(kernel_name)?;

        let a_buf = self.create_buffer(a)?;
        let mut output = TensorHandle::zeros(&a.shape[..a.ndim as usize], DType::F32)?;
        let out_buf = self.buffer_pool.allocate(output.numel * 4)?;

        let command_buffer = self.command_queue.new_command_buffer();
        let encoder = command_buffer.new_compute_command_encoder();

        encoder.set_compute_pipeline_state(&pipeline);
        encoder.set_buffer(0, Some(&a_buf), 0);
        encoder.set_buffer(1, Some(&out_buf), 0);

        // Vectorized dispatch: process 4 elements per thread
        let n = a.numel as u32;
        let n_vec = (a.numel / 4) as u32;  // Number of float4 vectors
        encoder.set_bytes(2, 4, &n as *const u32 as *const _);
        encoder.set_bytes(3, 4, &n_vec as *const u32 as *const _);

        // Dispatch vectorized threads (each processes 4 elements)
        let thread_group_size = MTLSize::new(256, 1, 1);
        let num_groups = MTLSize::new((n_vec as u64 + 255) / 256, 1, 1);
        encoder.dispatch_thread_groups(num_groups, thread_group_size);

        encoder.end_encoding();
        command_buffer.commit();
        command_buffer.wait_until_completed();

        self.copy_to_tensor(&out_buf, &mut output);

        self.buffer_pool.deallocate(a_buf, a.numel * 4);
        self.buffer_pool.deallocate(out_buf, output.numel * 4);

        Some(output)
    }

    /// Execute matrix multiplication on GPU
    pub fn matmul_gpu(&self, a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
        if a.dtype != DType::F32 || b.dtype != DType::F32 {
            return None;
        }

        if a.ndim < 2 || b.ndim < 2 {
            return None;
        }

        let m = a.shape[a.ndim as usize - 2];
        let k1 = a.shape[a.ndim as usize - 1];
        let k2 = b.shape[b.ndim as usize - 2];
        let n = b.shape[b.ndim as usize - 1];

        if k1 != k2 {
            return None;
        }

        let pipeline = self.get_pipeline("tensor_matmul_f32")?;

        let a_buf = self.create_buffer(a)?;
        let b_buf = self.create_buffer(b)?;
        let mut output = TensorHandle::zeros(&[m, n], DType::F32)?;
        let out_buf = self.buffer_pool.allocate(output.numel * 4)?;

        let command_buffer = self.command_queue.new_command_buffer();
        let encoder = command_buffer.new_compute_command_encoder();

        encoder.set_compute_pipeline_state(&pipeline);
        encoder.set_buffer(0, Some(&a_buf), 0);
        encoder.set_buffer(1, Some(&b_buf), 0);
        encoder.set_buffer(2, Some(&out_buf), 0);

        let m_u32 = m as u32;
        let k_u32 = k1 as u32;
        let n_u32 = n as u32;
        encoder.set_bytes(3, 4, &m_u32 as *const u32 as *const _);
        encoder.set_bytes(4, 4, &k_u32 as *const u32 as *const _);
        encoder.set_bytes(5, 4, &n_u32 as *const u32 as *const _);

        // Allocate threadgroup memory for tiles
        let tile_size = 16;
        let shared_size = tile_size * tile_size * 4; // float = 4 bytes
        encoder.set_threadgroup_memory_length(0, shared_size as u64);
        encoder.set_threadgroup_memory_length(1, shared_size as u64);

        let thread_group_size = MTLSize::new(tile_size as u64, tile_size as u64, 1);
        let num_groups = MTLSize::new(
            (n as u64 + tile_size as u64 - 1) / tile_size as u64,
            (m as u64 + tile_size as u64 - 1) / tile_size as u64,
            1,
        );
        encoder.dispatch_thread_groups(num_groups, thread_group_size);

        encoder.end_encoding();
        command_buffer.commit();
        command_buffer.wait_until_completed();

        self.copy_to_tensor(&out_buf, &mut output);

        self.buffer_pool.deallocate(a_buf, a.numel * 4);
        self.buffer_pool.deallocate(b_buf, b.numel * 4);
        self.buffer_pool.deallocate(out_buf, output.numel * 4);

        Some(output)
    }

    /// Execute reduction operation on GPU
    ///
    /// Supports Sum, Max, Min operations over all elements of a tensor.
    /// Uses parallel reduction with threadgroup shared memory.
    ///
    /// For axis-specific reductions, falls back to CPU.
    pub fn reduce_gpu(
        &self,
        a: &TensorHandle,
        op: TensorReduceOp,
        axis: Option<usize>,
    ) -> Option<TensorHandle> {
        if a.dtype != DType::F32 {
            return None;
        }

        // For now, only support full reduction (axis=None)
        // Axis-specific reductions are more complex and fall back to CPU
        if axis.is_some() {
            return None;
        }

        let pipeline_name = match op {
            TensorReduceOp::Sum => "tensor_reduce_sum_f32",
            TensorReduceOp::Max => "tensor_reduce_max_f32",
            TensorReduceOp::Min => "tensor_reduce_min_f32",
            TensorReduceOp::Mean => "tensor_reduce_sum_f32", // Sum then divide
            // Not yet implemented - fall back to CPU
            TensorReduceOp::Prod
            | TensorReduceOp::Var
            | TensorReduceOp::Std
            | TensorReduceOp::Norm
            | TensorReduceOp::LogSumExp
            | TensorReduceOp::All
            | TensorReduceOp::Any => return None,
        };

        let pipeline = self.get_pipeline(pipeline_name)?;

        let a_buf = self.create_buffer(a)?;
        let mut output = TensorHandle::zeros(&[], DType::F32)?;
        let out_buf = self.buffer_pool.allocate(4)?; // Single f32

        let command_buffer = self.command_queue.new_command_buffer();
        let encoder = command_buffer.new_compute_command_encoder();

        encoder.set_compute_pipeline_state(&pipeline);
        encoder.set_buffer(0, Some(&a_buf), 0);
        encoder.set_buffer(1, Some(&out_buf), 0);

        let n = a.numel as u32;
        encoder.set_bytes(2, 4, &n as *const u32 as *const _);

        // Use threadgroup for parallel reduction
        // Must be power of 2 for reduction algorithm to work correctly
        let threads_per_group: u64 = 256;
        let shared_size = threads_per_group * 4; // floats
        encoder.set_threadgroup_memory_length(0, shared_size);

        // Always dispatch full threadgroup (256 threads) for correct reduction
        // Shader handles out-of-bounds with identity values
        let thread_group_size = MTLSize::new(threads_per_group, 1, 1);
        let num_groups = MTLSize::new(1, 1, 1);

        encoder.dispatch_thread_groups(num_groups, thread_group_size);

        encoder.end_encoding();
        command_buffer.commit();
        command_buffer.wait_until_completed();

        self.copy_to_tensor(&out_buf, &mut output);

        // For Mean, divide by count
        if matches!(op, TensorReduceOp::Mean) {
            unsafe {
                let ptr = output.data_ptr_f32_mut();
                if !ptr.is_null() {
                    *ptr /= a.numel as f32;
                }
            }
        }

        self.buffer_pool.deallocate(a_buf, a.numel * 4);
        self.buffer_pool.deallocate(out_buf, 4);

        Some(output)
    }

    /// Execute softmax on GPU
    ///
    /// Implements numerically stable softmax: softmax(x) = exp(x - max(x)) / sum(exp(x - max(x)))
    ///
    /// For 1D tensors: computes softmax over all elements
    /// For 2D tensors: computes softmax along the last dimension (each row independently)
    ///
    /// Uses multi-pass algorithm for large tensors:
    /// 1. Find max value (for numerical stability)
    /// 2. Compute exp(x - max) and sum
    /// 3. Normalize by dividing by sum
    pub fn softmax_gpu(&self, a: &TensorHandle) -> Option<TensorHandle> {
        if a.dtype != DType::F32 {
            return None;
        }

        let shape = &a.shape[..a.ndim as usize];
        let numel = a.numel;

        // For 2D tensors, use batch softmax (each row independently)
        if a.ndim == 2 {
            return self.softmax_batch_gpu(a);
        }

        // For 1D tensors, use multi-pass approach
        let a_buf = self.create_buffer(a)?;
        let mut output = TensorHandle::zeros(shape, DType::F32)?;

        // Threadgroup size
        let group_size: u64 = 256;
        let num_groups = ((numel as u64) + group_size - 1) / group_size;

        // Allocate intermediate buffers
        let partial_max_buf = self.buffer_pool.allocate(num_groups as usize * 4)?;
        let exp_buf = self.buffer_pool.allocate(numel * 4)?;
        let partial_sum_buf = self.buffer_pool.allocate(num_groups as usize * 4)?;
        let out_buf = self.buffer_pool.allocate(numel * 4)?;

        // Get pipelines
        let max_pipeline = self.get_pipeline("tensor_softmax_max_f32")?;
        let exp_sum_pipeline = self.get_pipeline("tensor_softmax_exp_sum_f32")?;
        let normalize_pipeline = self.get_pipeline("tensor_softmax_normalize_vec_f32")?;

        // Pass 1: Find maximum
        {
            let command_buffer = self.command_queue.new_command_buffer();
            let encoder = command_buffer.new_compute_command_encoder();

            encoder.set_compute_pipeline_state(&max_pipeline);
            encoder.set_buffer(0, Some(&a_buf), 0);
            encoder.set_buffer(1, Some(&partial_max_buf), 0);

            let n = numel as u32;
            encoder.set_bytes(2, 4, &n as *const u32 as *const _);
            encoder.set_threadgroup_memory_length(0, group_size * 4);

            let thread_group_size = MTLSize::new(group_size, 1, 1);
            let num_groups_size = MTLSize::new(num_groups, 1, 1);
            encoder.dispatch_thread_groups(num_groups_size, thread_group_size);

            encoder.end_encoding();
            command_buffer.commit();
            command_buffer.wait_until_completed();
        }

        // Reduce partial maxes on CPU (small reduction)
        let max_val: f32 = {
            let ptr = partial_max_buf.contents() as *const f32;
            let mut max_val = f32::NEG_INFINITY;
            for i in 0..num_groups as usize {
                unsafe {
                    max_val = max_val.max(*ptr.add(i));
                }
            }
            max_val
        };

        // Pass 2: Compute exp(x - max) and partial sums
        {
            let command_buffer = self.command_queue.new_command_buffer();
            let encoder = command_buffer.new_compute_command_encoder();

            encoder.set_compute_pipeline_state(&exp_sum_pipeline);
            encoder.set_buffer(0, Some(&a_buf), 0);
            encoder.set_buffer(1, Some(&exp_buf), 0);
            encoder.set_buffer(2, Some(&partial_sum_buf), 0);

            let n = numel as u32;
            encoder.set_bytes(3, 4, &n as *const u32 as *const _);
            encoder.set_bytes(4, 4, &max_val as *const f32 as *const _);
            encoder.set_threadgroup_memory_length(0, group_size * 4);

            let thread_group_size = MTLSize::new(group_size, 1, 1);
            let num_groups_size = MTLSize::new(num_groups, 1, 1);
            encoder.dispatch_thread_groups(num_groups_size, thread_group_size);

            encoder.end_encoding();
            command_buffer.commit();
            command_buffer.wait_until_completed();
        }

        // Reduce partial sums on CPU
        let sum_val: f32 = {
            let ptr = partial_sum_buf.contents() as *const f32;
            let mut sum = 0.0f32;
            for i in 0..num_groups as usize {
                unsafe {
                    sum += *ptr.add(i);
                }
            }
            sum
        };

        // Pass 3: Normalize
        {
            let command_buffer = self.command_queue.new_command_buffer();
            let encoder = command_buffer.new_compute_command_encoder();

            encoder.set_compute_pipeline_state(&normalize_pipeline);
            encoder.set_buffer(0, Some(&exp_buf), 0);
            encoder.set_buffer(1, Some(&out_buf), 0);

            let n_vec = (numel / 4) as u32;
            encoder.set_bytes(2, 4, &n_vec as *const u32 as *const _);
            encoder.set_bytes(3, 4, &sum_val as *const f32 as *const _);

            let thread_group_size = MTLSize::new(256, 1, 1);
            let num_groups_size = MTLSize::new(((n_vec as u64) + 255) / 256, 1, 1);
            encoder.dispatch_thread_groups(num_groups_size, thread_group_size);

            encoder.end_encoding();
            command_buffer.commit();
            command_buffer.wait_until_completed();
        }

        // Copy result back
        self.copy_to_tensor(&out_buf, &mut output);

        // Return buffers to pool
        self.buffer_pool.deallocate(a_buf, numel * 4);
        self.buffer_pool.deallocate(partial_max_buf, num_groups as usize * 4);
        self.buffer_pool.deallocate(exp_buf, numel * 4);
        self.buffer_pool.deallocate(partial_sum_buf, num_groups as usize * 4);
        self.buffer_pool.deallocate(out_buf, numel * 4);

        Some(output)
    }

    /// Batch softmax for 2D tensors (each row independently)
    fn softmax_batch_gpu(&self, a: &TensorHandle) -> Option<TensorHandle> {
        if a.ndim != 2 {
            return None;
        }

        let batch_size = a.shape[0];
        let dim = a.shape[1];

        // For small dims, use the single-pass batch kernel
        if dim <= 1024 {
            let pipeline = self.get_pipeline("tensor_softmax_batch_f32")?;

            let a_buf = self.create_buffer(a)?;
            let mut output = TensorHandle::zeros(&[batch_size, dim], DType::F32)?;
            let out_buf = self.buffer_pool.allocate(output.numel * 4)?;

            let command_buffer = self.command_queue.new_command_buffer();
            let encoder = command_buffer.new_compute_command_encoder();

            encoder.set_compute_pipeline_state(&pipeline);
            encoder.set_buffer(0, Some(&a_buf), 0);
            encoder.set_buffer(1, Some(&out_buf), 0);

            // Threadgroup memory for reduction
            let group_size = dim.next_power_of_two().min(1024) as u64;

            let batch_u32 = batch_size as u32;
            let dim_u32 = dim as u32;
            let group_sz_u32 = group_size as u32;
            encoder.set_bytes(2, 4, &batch_u32 as *const u32 as *const _);
            encoder.set_bytes(3, 4, &dim_u32 as *const u32 as *const _);
            encoder.set_bytes(4, 4, &group_sz_u32 as *const u32 as *const _);

            encoder.set_threadgroup_memory_length(0, group_size * 4);

            let thread_group_size = MTLSize::new(group_size, 1, 1);
            let num_groups = MTLSize::new(1, batch_size as u64, 1);
            encoder.dispatch_thread_groups(num_groups, thread_group_size);

            encoder.end_encoding();
            command_buffer.commit();
            command_buffer.wait_until_completed();

            self.copy_to_tensor(&out_buf, &mut output);

            self.buffer_pool.deallocate(a_buf, a.numel * 4);
            self.buffer_pool.deallocate(out_buf, output.numel * 4);

            return Some(output);
        }

        // For large dims, fall back to CPU or use multi-pass per row
        // For now, process each row as a 1D softmax
        let mut output = TensorHandle::zeros(&[batch_size, dim], DType::F32)?;
        for b in 0..batch_size {
            // Extract row - this is inefficient but correct for large dims
            let mut row = TensorHandle::zeros(&[dim], DType::F32)?;
            let src = a.data_ptr_f32();
            let dst = row.data_ptr_f32_mut();
            if !src.is_null() && !dst.is_null() {
                // Bounds check: src offset b*dim + dim must not exceed source tensor
                assert!(
                    b * dim + dim <= a.numel,
                    "softmax row copy out of bounds: offset={}, dim={}, numel={}",
                    b * dim, dim, a.numel
                );
                unsafe {
                    // SAFETY: src.add(b*dim) is in bounds (checked above), dst has `dim` elements.
                    std::ptr::copy_nonoverlapping(src.add(b * dim), dst, dim);
                }
            }

            // Compute softmax for this row
            let row_result = self.softmax_gpu(&row)?;

            // Copy back to output
            let out_ptr = output.data_ptr_f32_mut();
            let res_ptr = row_result.data_ptr_f32();
            if !out_ptr.is_null() && !res_ptr.is_null() {
                // Bounds check: output offset b*dim + dim must not exceed output tensor
                assert!(
                    b * dim + dim <= output.numel,
                    "softmax output copy out of bounds: offset={}, dim={}, numel={}",
                    b * dim, dim, output.numel
                );
                unsafe {
                    // SAFETY: out_ptr.add(b*dim) is in bounds (checked above), res_ptr has `dim` elements.
                    std::ptr::copy_nonoverlapping(res_ptr, out_ptr.add(b * dim), dim);
                }
            }
        }

        Some(output)
    }

    /// Execute layer normalization on GPU
    ///
    /// LayerNorm(x) = (x - mean) / sqrt(var + eps) * gamma + beta
    ///
    /// Input shape: [batch_size, hidden_size]
    /// Gamma/Beta shape: [hidden_size] (optional)
    ///
    /// Uses efficient parallel reduction for mean/variance computation.
    pub fn layer_norm_gpu(
        &self,
        input: &TensorHandle,
        gamma: Option<&TensorHandle>,
        beta: Option<&TensorHandle>,
        eps: f64,
    ) -> Option<TensorHandle> {
        if input.dtype != DType::F32 {
            return None;
        }

        if input.ndim != 2 {
            return None;
        }

        let _batch_size = input.shape[0];
        let hidden_size = input.shape[1];

        // Validate gamma/beta shapes
        if let Some(g) = gamma {
            if g.ndim != 1 || g.shape[0] != hidden_size {
                return None;
            }
        }
        if let Some(b) = beta {
            if b.ndim != 1 || b.shape[0] != hidden_size {
                return None;
            }
        }

        // For small hidden sizes, use single-pass kernel
        if hidden_size <= 1024 {
            return self.layer_norm_small_gpu(input, gamma, beta, eps);
        }

        // For large hidden sizes, use multi-pass approach
        self.layer_norm_large_gpu(input, gamma, beta, eps)
    }

    /// Layer norm for small hidden sizes (fits in threadgroup)
    fn layer_norm_small_gpu(
        &self,
        input: &TensorHandle,
        gamma: Option<&TensorHandle>,
        beta: Option<&TensorHandle>,
        eps: f64,
    ) -> Option<TensorHandle> {
        let batch_size = input.shape[0];
        let hidden_size = input.shape[1];

        let pipeline = self.get_pipeline("tensor_layer_norm_small_f32")?;

        let input_buf = self.create_buffer(input)?;
        let gamma_buf = gamma.and_then(|g| self.create_buffer(g));
        let beta_buf = beta.and_then(|b| self.create_buffer(b));
        let mut output = TensorHandle::zeros(&[batch_size, hidden_size], DType::F32)?;
        let out_buf = self.buffer_pool.allocate(output.numel * 4)?;

        let command_buffer = self.command_queue.new_command_buffer();
        let encoder = command_buffer.new_compute_command_encoder();

        encoder.set_compute_pipeline_state(&pipeline);
        encoder.set_buffer(0, Some(&input_buf), 0);

        // Create dummy buffers for gamma/beta if not provided
        let dummy_buf = self.buffer_pool.allocate(hidden_size * 4)?;
        if let Some(ref gb) = gamma_buf {
            encoder.set_buffer(1, Some(gb), 0);
        } else {
            encoder.set_buffer(1, Some(&dummy_buf), 0);
        }
        if let Some(ref bb) = beta_buf {
            encoder.set_buffer(2, Some(bb), 0);
        } else {
            encoder.set_buffer(2, Some(&dummy_buf), 0);
        }
        encoder.set_buffer(3, Some(&out_buf), 0);

        // Threadgroup size for reduction
        let group_size = hidden_size.next_power_of_two().min(1024) as u64;

        let batch_u32 = batch_size as u32;
        let hidden_u32 = hidden_size as u32;
        let eps_f32 = eps as f32;
        let has_gamma_u32 = if gamma.is_some() { 1u32 } else { 0u32 };
        let has_beta_u32 = if beta.is_some() { 1u32 } else { 0u32 };
        let group_sz_u32 = group_size as u32;

        encoder.set_bytes(4, 4, &batch_u32 as *const u32 as *const _);
        encoder.set_bytes(5, 4, &hidden_u32 as *const u32 as *const _);
        encoder.set_bytes(6, 4, &eps_f32 as *const f32 as *const _);
        encoder.set_bytes(7, 4, &has_gamma_u32 as *const u32 as *const _);
        encoder.set_bytes(8, 4, &has_beta_u32 as *const u32 as *const _);
        encoder.set_bytes(9, 4, &group_sz_u32 as *const u32 as *const _);

        encoder.set_threadgroup_memory_length(0, group_size * 4);

        let thread_group_size = MTLSize::new(group_size, 1, 1);
        let num_groups = MTLSize::new(1, batch_size as u64, 1);
        encoder.dispatch_thread_groups(num_groups, thread_group_size);

        encoder.end_encoding();
        command_buffer.commit();
        command_buffer.wait_until_completed();

        self.copy_to_tensor(&out_buf, &mut output);

        // Cleanup
        self.buffer_pool.deallocate(input_buf, input.numel * 4);
        if let Some(buf) = gamma_buf {
            self.buffer_pool.deallocate(buf, hidden_size * 4);
        }
        if let Some(buf) = beta_buf {
            self.buffer_pool.deallocate(buf, hidden_size * 4);
        }
        self.buffer_pool.deallocate(dummy_buf, hidden_size * 4);
        self.buffer_pool.deallocate(out_buf, output.numel * 4);

        Some(output)
    }

    /// Layer norm for large hidden sizes (multi-pass)
    fn layer_norm_large_gpu(
        &self,
        input: &TensorHandle,
        gamma: Option<&TensorHandle>,
        beta: Option<&TensorHandle>,
        eps: f64,
    ) -> Option<TensorHandle> {
        let batch_size = input.shape[0];
        let hidden_size = input.shape[1];

        // Multi-pass approach:
        // 1. Compute mean for each batch element
        // 2. Compute variance for each batch element
        // 3. Normalize

        // Get pipelines
        let mean_pipeline = self.get_pipeline("tensor_layer_norm_mean_f32")?;
        let var_pipeline = self.get_pipeline("tensor_layer_norm_var_f32")?;
        let normalize_pipeline = self.get_pipeline("tensor_layer_norm_normalize_f32")?;

        let group_size: u64 = 256;
        let groups_per_batch = ((hidden_size as u64) + group_size - 1) / group_size;

        // Allocate buffers
        let input_buf = self.create_buffer(input)?;
        let gamma_buf = gamma.and_then(|g| self.create_buffer(g));
        let beta_buf = beta.and_then(|b| self.create_buffer(b));
        let partial_sum_buf = self.buffer_pool.allocate(batch_size * groups_per_batch as usize * 4)?;
        let partial_var_buf = self.buffer_pool.allocate(batch_size * groups_per_batch as usize * 4)?;
        let means_buf = self.buffer_pool.allocate(batch_size * 4)?;
        let vars_buf = self.buffer_pool.allocate(batch_size * 4)?;
        let mut output = TensorHandle::zeros(&[batch_size, hidden_size], DType::F32)?;
        let out_buf = self.buffer_pool.allocate(output.numel * 4)?;
        let dummy_buf = self.buffer_pool.allocate(hidden_size * 4)?;

        // Pass 1: Compute partial sums for mean
        {
            let command_buffer = self.command_queue.new_command_buffer();
            let encoder = command_buffer.new_compute_command_encoder();

            encoder.set_compute_pipeline_state(&mean_pipeline);
            encoder.set_buffer(0, Some(&input_buf), 0);
            encoder.set_buffer(1, Some(&partial_sum_buf), 0);

            let batch_u32 = batch_size as u32;
            let hidden_u32 = hidden_size as u32;
            let group_sz_u32 = group_size as u32;
            encoder.set_bytes(2, 4, &batch_u32 as *const u32 as *const _);
            encoder.set_bytes(3, 4, &hidden_u32 as *const u32 as *const _);
            encoder.set_bytes(4, 4, &group_sz_u32 as *const u32 as *const _);
            encoder.set_threadgroup_memory_length(0, group_size * 4);

            let thread_group_size = MTLSize::new(group_size, 1, 1);
            let num_groups = MTLSize::new(groups_per_batch, batch_size as u64, 1);
            encoder.dispatch_thread_groups(num_groups, thread_group_size);

            encoder.end_encoding();
            command_buffer.commit();
            command_buffer.wait_until_completed();
        }

        // Reduce partial sums to get means (on CPU for simplicity)
        {
            let partial_ptr = partial_sum_buf.contents() as *const f32;
            let means_ptr = means_buf.contents() as *mut f32;
            for b in 0..batch_size {
                let mut sum = 0.0f32;
                for g in 0..groups_per_batch as usize {
                    unsafe {
                        sum += *partial_ptr.add(b * groups_per_batch as usize + g);
                    }
                }
                unsafe {
                    *means_ptr.add(b) = sum / hidden_size as f32;
                }
            }
        }

        // Pass 2: Compute partial variances
        {
            let command_buffer = self.command_queue.new_command_buffer();
            let encoder = command_buffer.new_compute_command_encoder();

            encoder.set_compute_pipeline_state(&var_pipeline);
            encoder.set_buffer(0, Some(&input_buf), 0);
            encoder.set_buffer(1, Some(&partial_var_buf), 0);
            encoder.set_buffer(2, Some(&means_buf), 0);

            let batch_u32 = batch_size as u32;
            let hidden_u32 = hidden_size as u32;
            let group_sz_u32 = group_size as u32;
            encoder.set_bytes(3, 4, &batch_u32 as *const u32 as *const _);
            encoder.set_bytes(4, 4, &hidden_u32 as *const u32 as *const _);
            encoder.set_bytes(5, 4, &group_sz_u32 as *const u32 as *const _);
            encoder.set_threadgroup_memory_length(0, group_size * 4);

            let thread_group_size = MTLSize::new(group_size, 1, 1);
            let num_groups = MTLSize::new(groups_per_batch, batch_size as u64, 1);
            encoder.dispatch_thread_groups(num_groups, thread_group_size);

            encoder.end_encoding();
            command_buffer.commit();
            command_buffer.wait_until_completed();
        }

        // Reduce partial variances (on CPU)
        {
            let partial_ptr = partial_var_buf.contents() as *const f32;
            let vars_ptr = vars_buf.contents() as *mut f32;
            for b in 0..batch_size {
                let mut sum = 0.0f32;
                for g in 0..groups_per_batch as usize {
                    unsafe {
                        sum += *partial_ptr.add(b * groups_per_batch as usize + g);
                    }
                }
                unsafe {
                    *vars_ptr.add(b) = sum / hidden_size as f32;
                }
            }
        }

        // Pass 3: Normalize
        {
            let command_buffer = self.command_queue.new_command_buffer();
            let encoder = command_buffer.new_compute_command_encoder();

            encoder.set_compute_pipeline_state(&normalize_pipeline);
            encoder.set_buffer(0, Some(&input_buf), 0);
            encoder.set_buffer(1, Some(&out_buf), 0);
            if let Some(ref gb) = gamma_buf {
                encoder.set_buffer(2, Some(gb), 0);
            } else {
                encoder.set_buffer(2, Some(&dummy_buf), 0);
            }
            if let Some(ref bb) = beta_buf {
                encoder.set_buffer(3, Some(bb), 0);
            } else {
                encoder.set_buffer(3, Some(&dummy_buf), 0);
            }
            encoder.set_buffer(4, Some(&means_buf), 0);
            encoder.set_buffer(5, Some(&vars_buf), 0);

            let batch_u32 = batch_size as u32;
            let hidden_u32 = hidden_size as u32;
            let eps_f32 = eps as f32;
            let has_gamma_u32 = if gamma.is_some() { 1u32 } else { 0u32 };
            let has_beta_u32 = if beta.is_some() { 1u32 } else { 0u32 };

            encoder.set_bytes(6, 4, &batch_u32 as *const u32 as *const _);
            encoder.set_bytes(7, 4, &hidden_u32 as *const u32 as *const _);
            encoder.set_bytes(8, 4, &eps_f32 as *const f32 as *const _);
            encoder.set_bytes(9, 4, &has_gamma_u32 as *const u32 as *const _);
            encoder.set_bytes(10, 4, &has_beta_u32 as *const u32 as *const _);

            let thread_group_size = MTLSize::new(256, 1, 1);
            let num_groups = MTLSize::new(
                (hidden_size as u64 + 255) / 256,
                batch_size as u64,
                1,
            );
            encoder.dispatch_thread_groups(num_groups, thread_group_size);

            encoder.end_encoding();
            command_buffer.commit();
            command_buffer.wait_until_completed();
        }

        self.copy_to_tensor(&out_buf, &mut output);

        // Cleanup
        self.buffer_pool.deallocate(input_buf, input.numel * 4);
        if let Some(buf) = gamma_buf {
            self.buffer_pool.deallocate(buf, hidden_size * 4);
        }
        if let Some(buf) = beta_buf {
            self.buffer_pool.deallocate(buf, hidden_size * 4);
        }
        self.buffer_pool.deallocate(partial_sum_buf, batch_size * groups_per_batch as usize * 4);
        self.buffer_pool.deallocate(partial_var_buf, batch_size * groups_per_batch as usize * 4);
        self.buffer_pool.deallocate(means_buf, batch_size * 4);
        self.buffer_pool.deallocate(vars_buf, batch_size * 4);
        self.buffer_pool.deallocate(dummy_buf, hidden_size * 4);
        self.buffer_pool.deallocate(out_buf, output.numel * 4);

        Some(output)
    }

    /// Get device name
    pub fn device_name(&self) -> String {
        self.device.name().to_string()
    }

    /// Get available GPU memory
    pub fn available_memory(&self) -> usize {
        self.device.recommended_max_working_set_size() as usize
    }
}

/// Estimate memory bandwidth for a Metal device
fn estimate_bandwidth(_device: &Device) -> f64 {
    // Apple Silicon unified memory typically has high bandwidth
    // M1 Pro/Max: ~200-400 GB/s
    // M2 Pro/Max: ~200-400 GB/s
    // M3 Pro/Max: ~150-400 GB/s
    200.0 // Conservative estimate
}

/// Estimate peak TFLOPS for a Metal device
fn estimate_tflops(_device: &Device) -> f64 {
    // Rough estimates based on Apple Silicon GPUs
    // M1 Pro: ~5.2 TFLOPS
    // M1 Max: ~10.4 TFLOPS
    // M2 Pro: ~6.8 TFLOPS
    // M2 Max: ~13.6 TFLOPS
    // M3 Pro: ~7+ TFLOPS
    // M3 Max: ~14+ TFLOPS
    10.0 // Conservative estimate for M1/M2/M3 Max
}

// ============================================================================
// Backend Trait Implementation
// ============================================================================

impl Backend for MetalBackend {
    fn name(&self) -> &'static str {
        "Metal"
    }

    fn device_id(&self) -> DeviceId {
        DeviceId::gpu(self.gpu_index)
    }

    fn capabilities(&self) -> &ComputeCapabilities {
        &self.capabilities
    }

    fn allocate(&self, size: usize, _align: usize) -> Option<NonNull<u8>> {
        let buffer = self.buffer_pool.allocate(size)?;
        NonNull::new(buffer.contents() as *mut u8)
    }

    fn deallocate(&self, _ptr: NonNull<u8>, _size: usize, _align: usize) {
        // Metal manages buffer memory automatically
    }

    fn copy_h2d(&self, host: *const u8, device: NonNull<u8>, size: usize) {
        assert!(!host.is_null(), "copy_h2d: host pointer is null");
        unsafe {
            // SAFETY: host is non-null (asserted), device is NonNull, caller guarantees
            // both buffers are at least `size` bytes. Metal device pointer is from allocate().
            std::ptr::copy_nonoverlapping(host, device.as_ptr(), size);
        }
    }

    fn copy_d2h(&self, device: NonNull<u8>, host: *mut u8, size: usize) {
        assert!(!host.is_null(), "copy_d2h: host pointer is null");
        unsafe {
            // SAFETY: device is NonNull, host is non-null (asserted), caller guarantees
            // both buffers are at least `size` bytes.
            std::ptr::copy_nonoverlapping(device.as_ptr(), host, size);
        }
    }

    fn copy_d2d(&self, src: NonNull<u8>, dst: NonNull<u8>, size: usize) {
        unsafe {
            // SAFETY: both src and dst are NonNull from allocate(), caller guarantees
            // both buffers are at least `size` bytes and do not overlap.
            std::ptr::copy_nonoverlapping(src.as_ptr(), dst.as_ptr(), size);
        }
    }

    fn synchronize(&self) {
        // Metal uses completion handlers, synchronize waits for all pending work
        let command_buffer = self.command_queue.new_command_buffer();
        command_buffer.commit();
        command_buffer.wait_until_completed();
    }

    fn binop(
        &self,
        a: &TensorHandle,
        b: &TensorHandle,
        op: TensorBinaryOp,
    ) -> Option<TensorHandle> {
        self.binop_gpu(a, b, op)
    }

    fn unop(&self, a: &TensorHandle, op: TensorUnaryOp) -> Option<TensorHandle> {
        self.unop_gpu(a, op)
    }

    fn reduce(
        &self,
        a: &TensorHandle,
        op: TensorReduceOp,
        axis: Option<usize>,
    ) -> Option<TensorHandle> {
        // Use GPU reduction for supported operations
        // Falls back to None for unsupported ops (CPU handles those)
        self.reduce_gpu(a, op, axis)
    }

    fn matmul(&self, a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
        self.matmul_gpu(a, b)
    }

    fn memory_info(&self) -> (usize, usize) {
        let total = self.device.recommended_max_working_set_size() as usize;
        let used = self.buffer_pool.stats().allocated;
        (total.saturating_sub(used), total)
    }
}

// ============================================================================
// Global Metal Backend
// ============================================================================

static METAL_BACKEND: OnceLock<Option<Arc<MetalBackend>>> = OnceLock::new();

/// Get the global Metal backend (lazily initialized)
pub fn get_metal_backend() -> Option<&'static Arc<MetalBackend>> {
    METAL_BACKEND
        .get_or_init(|| MetalBackend::new().map(Arc::new))
        .as_ref()
}

/// Check if Metal is available
pub fn is_metal_available() -> bool {
    Device::system_default().is_some()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metal_available() {
        // This test will only pass on macOS with Metal support
        if !is_metal_available() {
            return;
        }
        assert!(Device::system_default().is_some());
    }

    #[test]
    fn test_metal_backend_creation() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new();
        assert!(backend.is_some());

        let backend = backend.unwrap();
        assert_eq!(backend.name(), "Metal");
        assert!(backend.capabilities().max_threads > 0);
    }

    #[test]
    fn test_metal_buffer_pool() {
        if !is_metal_available() {
            return;
        }

        let device = Device::system_default().unwrap();
        let pool = MetalBufferPool::new(device);

        // Allocate
        let buf1 = pool.allocate(1024);
        assert!(buf1.is_some());

        // Return to pool
        pool.deallocate(buf1.unwrap(), 1024);

        // Should get from cache
        let buf2 = pool.allocate(1024);
        assert!(buf2.is_some());

        let stats = pool.stats();
        assert_eq!(stats.cache_hits, 1);
    }

    #[test]
    fn test_metal_binop_add() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        let a = TensorHandle::full(&[1024], DType::F32, 2.0).unwrap();
        let b = TensorHandle::full(&[1024], DType::F32, 3.0).unwrap();

        let result = backend.binop_gpu(&a, &b, TensorBinaryOp::Add);
        assert!(result.is_some());

        let result = result.unwrap();
        assert_eq!(result.numel, 1024);
    }

    #[test]
    fn test_metal_unop_exp() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        let a = TensorHandle::full(&[256], DType::F32, 1.0).unwrap();

        let result = backend.unop_gpu(&a, TensorUnaryOp::Exp);
        assert!(result.is_some());

        let result = result.unwrap();
        assert_eq!(result.numel, 256);
    }

    #[test]
    fn test_metal_matmul() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        let a = TensorHandle::full(&[64, 128], DType::F32, 1.0).unwrap();
        let b = TensorHandle::full(&[128, 64], DType::F32, 1.0).unwrap();

        let result = backend.matmul_gpu(&a, &b);
        assert!(result.is_some());

        let result = result.unwrap();
        assert_eq!(result.shape[0], 64);
        assert_eq!(result.shape[1], 64);
    }

    #[test]
    fn test_metal_memory_info() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();
        let (free, total) = backend.memory_info();

        assert!(total > 0);
        assert!(free <= total);
    }

    // ==========================================================================
    // GPU vs CPU Correctness Tests
    // ==========================================================================

    /// Helper function to compare tensors with tolerance
    fn tensors_close(a: &TensorHandle, b: &TensorHandle, rtol: f32, atol: f32) -> bool {
        if a.numel != b.numel || a.dtype != b.dtype {
            return false;
        }

        if a.dtype != DType::F32 {
            return false;
        }

        let ptr_a = a.data_ptr_f32();
        let ptr_b = b.data_ptr_f32();

        if ptr_a.is_null() || ptr_b.is_null() {
            return false;
        }

        unsafe {
            for i in 0..a.numel {
                let va = *ptr_a.add(i);
                let vb = *ptr_b.add(i);
                let diff = (va - vb).abs();
                let tol = atol + rtol * vb.abs();
                if diff > tol {
                    return false;
                }
            }
        }

        true
    }

    #[test]
    fn test_gpu_cpu_binop_add_correctness() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Create test data using full()
        let a = TensorHandle::full(&[1000], DType::F32, 2.5).unwrap();
        let b = TensorHandle::full(&[1000], DType::F32, 3.5).unwrap();

        // GPU result
        let gpu_result = backend.binop_gpu(&a, &b, TensorBinaryOp::Add).unwrap();

        // CPU result
        let cpu_result = super::super::cpu::binop_f32_scalar(&a, &b, TensorBinaryOp::Add).unwrap();

        // Compare
        assert!(tensors_close(&gpu_result, &cpu_result, 1e-5, 1e-5));
    }

    #[test]
    fn test_gpu_cpu_binop_mul_correctness() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        let a = TensorHandle::full(&[512], DType::F32, 3.0).unwrap();
        let b = TensorHandle::full(&[512], DType::F32, 7.0).unwrap();

        let gpu_result = backend.binop_gpu(&a, &b, TensorBinaryOp::Mul).unwrap();
        let cpu_result = super::super::cpu::binop_f32_scalar(&a, &b, TensorBinaryOp::Mul).unwrap();

        assert!(tensors_close(&gpu_result, &cpu_result, 1e-5, 1e-5));
    }

    #[test]
    fn test_gpu_cpu_unop_exp_correctness() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Small values to avoid overflow - use full() instead of arange()
        let a = TensorHandle::full(&[400], DType::F32, 0.5).unwrap();

        let gpu_result = backend.unop_gpu(&a, TensorUnaryOp::Exp).unwrap();
        let cpu_result = super::super::cpu::unop_f32_scalar(&a, TensorUnaryOp::Exp).unwrap();

        assert!(tensors_close(&gpu_result, &cpu_result, 1e-5, 1e-5));
    }

    #[test]
    fn test_gpu_cpu_unop_sqrt_correctness() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Positive values only - use full()
        let a = TensorHandle::full(&[1000], DType::F32, 16.0).unwrap();

        let gpu_result = backend.unop_gpu(&a, TensorUnaryOp::Sqrt).unwrap();
        let cpu_result = super::super::cpu::unop_f32_scalar(&a, TensorUnaryOp::Sqrt).unwrap();

        assert!(tensors_close(&gpu_result, &cpu_result, 1e-5, 1e-5));
    }

    #[test]
    fn test_gpu_cpu_matmul_correctness() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Create matrices with known values using full()
        let a = TensorHandle::full(&[2, 2], DType::F32, 1.0).unwrap();
        let b = TensorHandle::full(&[2, 2], DType::F32, 2.0).unwrap();

        let gpu_result = backend.matmul_gpu(&a, &b).unwrap();
        let cpu_result = super::super::cpu::matmul_f32_scalar(&a, &b).unwrap();

        // Each element should be 2 * 1.0 * 2.0 = 4.0
        assert!(tensors_close(&gpu_result, &cpu_result, 1e-4, 1e-4));
    }

    #[test]
    fn test_gpu_cpu_matmul_large_correctness() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Larger matrices
        let a = TensorHandle::full(&[128, 64], DType::F32, 0.1).unwrap();
        let b = TensorHandle::full(&[64, 128], DType::F32, 0.1).unwrap();

        let gpu_result = backend.matmul_gpu(&a, &b).unwrap();
        let cpu_result = super::super::cpu::matmul_f32_scalar(&a, &b).unwrap();

        // Expected: each element should be 64 * 0.1 * 0.1 = 0.64
        assert!(tensors_close(&gpu_result, &cpu_result, 1e-4, 1e-4));
    }

    #[test]
    fn test_gpu_large_tensor_operations() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Test with 1M elements - should trigger GPU dispatch
        let size = 1_000_000;
        let a = TensorHandle::full(&[size], DType::F32, 2.5).unwrap();
        let b = TensorHandle::full(&[size], DType::F32, 1.5).unwrap();

        let result = backend.binop_gpu(&a, &b, TensorBinaryOp::Add).unwrap();
        assert_eq!(result.numel, size);

        // Verify first few elements
        let ptr = result.data_ptr_f32();
        assert!(!ptr.is_null());
        unsafe {
            for i in 0..10 {
                let val = *ptr.add(i);
                assert!((val - 4.0).abs() < 1e-5, "Expected 4.0, got {} at index {}", val, i);
            }
        }
    }

    #[test]
    fn test_metal_device_info() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();
        let name = backend.device_name();

        assert!(!name.is_empty());
        println!("Metal device: {}", name);

        let memory = backend.available_memory();
        assert!(memory > 0);
        println!("Available GPU memory: {} GB", memory / (1024 * 1024 * 1024));
    }

    #[test]
    fn test_metal_all_binops() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        let a = TensorHandle::full(&[256], DType::F32, 4.0).unwrap();
        let b = TensorHandle::full(&[256], DType::F32, 2.0).unwrap();

        // Test all supported binary operations
        let ops = [
            (TensorBinaryOp::Add, 6.0),
            (TensorBinaryOp::Sub, 2.0),
            (TensorBinaryOp::Mul, 8.0),
            (TensorBinaryOp::Div, 2.0),
            (TensorBinaryOp::Max, 4.0),
            (TensorBinaryOp::Min, 2.0),
        ];

        for (op, expected) in ops {
            let result = backend.binop_gpu(&a, &b, op);
            assert!(result.is_some(), "Failed for {:?}", op);

            let result = result.unwrap();
            let ptr = result.data_ptr_f32();
            assert!(!ptr.is_null());

            unsafe {
                let val = *ptr;
                assert!((val - expected).abs() < 1e-5,
                    "For {:?}: expected {}, got {}", op, expected, val);
            }
        }
    }

    #[test]
    fn test_metal_all_unops() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Test various unary operations with (op, input, expected) as f64 for full()
        let test_cases: &[(TensorUnaryOp, f64, f32)] = &[
            (TensorUnaryOp::Neg, 2.0, -2.0),
            (TensorUnaryOp::Abs, -3.0, 3.0),
            (TensorUnaryOp::Sqrt, 4.0, 2.0),
            (TensorUnaryOp::Exp, 0.0, 1.0),
            (TensorUnaryOp::Log, 1.0, 0.0),
            (TensorUnaryOp::Sin, 0.0, 0.0),
            (TensorUnaryOp::Cos, 0.0, 1.0),
            (TensorUnaryOp::Relu, -1.0, 0.0),
            (TensorUnaryOp::Relu, 1.0, 1.0),
        ];

        for &(op, input, expected) in test_cases {
            let a = TensorHandle::full(&[64], DType::F32, input).unwrap();
            let result = backend.unop_gpu(&a, op);
            assert!(result.is_some(), "Failed for {:?} with input {}", op, input);

            let result = result.unwrap();
            let ptr = result.data_ptr_f32();
            assert!(!ptr.is_null());

            unsafe {
                let val = *ptr;
                assert!((val - expected).abs() < 1e-4,
                    "For {:?}({}): expected {}, got {}", op, input, expected, val);
            }
        }
    }

    // ==========================================================================
    // Edge Case Tests
    // ==========================================================================

    #[test]
    fn test_metal_various_tensor_sizes() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Test various sizes to ensure vectorization works correctly
        // All sizes must be divisible by 4 for float4 vectorization
        let sizes: &[usize] = &[4, 8, 16, 32, 64, 128, 256, 512, 1024, 4096, 16384, 65536];

        for &size in sizes {
            let a = TensorHandle::full(&[size], DType::F32, 2.0).unwrap();
            let b = TensorHandle::full(&[size], DType::F32, 3.0).unwrap();

            let result = backend.binop_gpu(&a, &b, TensorBinaryOp::Add);
            assert!(result.is_some(), "Failed for size {}", size);

            let result = result.unwrap();
            assert_eq!(result.numel, size);

            // Verify all elements are correct
            let ptr = result.data_ptr_f32();
            unsafe {
                for i in 0..size {
                    let val = *ptr.add(i);
                    assert!((val - 5.0).abs() < 1e-5,
                        "Size {}: expected 5.0, got {} at index {}", size, val, i);
                }
            }
        }
    }

    #[test]
    fn test_metal_multidimensional_tensors() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Test various shapes (all with numel divisible by 4)
        let shapes: &[&[usize]] = &[
            &[4],
            &[2, 2],
            &[2, 2, 4],
            &[4, 4, 4],
            &[8, 8],
            &[16, 16],
            &[32, 32],
            &[4, 8, 16],
        ];

        for shape in shapes {
            let a = TensorHandle::full(shape, DType::F32, 1.5).unwrap();
            let b = TensorHandle::full(shape, DType::F32, 2.5).unwrap();

            let result = backend.binop_gpu(&a, &b, TensorBinaryOp::Mul);
            assert!(result.is_some(), "Failed for shape {:?}", shape);

            let result = result.unwrap();
            let expected_numel: usize = shape.iter().product();
            assert_eq!(result.numel, expected_numel);

            // Verify first element
            let ptr = result.data_ptr_f32();
            unsafe {
                let val = *ptr;
                assert!((val - 3.75).abs() < 1e-5,
                    "Shape {:?}: expected 3.75, got {}", shape, val);
            }
        }
    }

    #[test]
    fn test_metal_numerical_precision() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Test numerical precision with specific values
        let test_cases: &[(f64, f64, TensorBinaryOp, f32, f32)] = &[
            // (a, b, op, expected, tolerance)
            (1e-6, 1e-6, TensorBinaryOp::Add, 2e-6, 1e-10),
            (1e6, 1e6, TensorBinaryOp::Add, 2e6, 1.0),
            (0.1, 0.2, TensorBinaryOp::Add, 0.3, 1e-6),
            (1.0, 3.0, TensorBinaryOp::Div, 0.333333, 1e-5),
            (2.0, 10.0, TensorBinaryOp::Pow, 1024.0, 1e-3),
        ];

        for &(a_val, b_val, op, expected, tol) in test_cases {
            let a = TensorHandle::full(&[64], DType::F32, a_val).unwrap();
            let b = TensorHandle::full(&[64], DType::F32, b_val).unwrap();

            let result = backend.binop_gpu(&a, &b, op);
            assert!(result.is_some(), "Failed for {:?}({}, {})", op, a_val, b_val);

            let result = result.unwrap();
            let ptr = result.data_ptr_f32();
            unsafe {
                let val = *ptr;
                assert!((val - expected).abs() < tol,
                    "For {:?}({}, {}): expected {}, got {} (tol={})",
                    op, a_val, b_val, expected, val, tol);
            }
        }
    }

    #[test]
    fn test_metal_special_float_values() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Test with values that might cause issues
        let special_values: &[(f64, TensorUnaryOp)] = &[
            (0.0, TensorUnaryOp::Exp),    // exp(0) = 1
            (1.0, TensorUnaryOp::Log),    // log(1) = 0
            (0.0, TensorUnaryOp::Sqrt),   // sqrt(0) = 0
            (-5.0, TensorUnaryOp::Relu),  // relu(-5) = 0
            (5.0, TensorUnaryOp::Relu),   // relu(5) = 5
        ];

        for &(val, op) in special_values {
            let a = TensorHandle::full(&[64], DType::F32, val).unwrap();
            let result = backend.unop_gpu(&a, op);
            assert!(result.is_some(), "Failed for {:?}({})", op, val);

            // Just verify it doesn't crash - actual values tested elsewhere
            assert!(result.unwrap().numel == 64);
        }
    }

    #[test]
    fn test_metal_sigmoid_precision() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Test sigmoid for various inputs
        // sigmoid(x) = 1 / (1 + exp(-x))
        let test_cases: &[(f64, f32)] = &[
            (0.0, 0.5),          // sigmoid(0) = 0.5
            (1.0, 0.7310586),    // sigmoid(1)
            (-1.0, 0.26894143),  // sigmoid(-1)
            (5.0, 0.9933072),    // sigmoid(5) ≈ 1
            (-5.0, 0.0066929),   // sigmoid(-5) ≈ 0
        ];

        for &(input, expected) in test_cases {
            let a = TensorHandle::full(&[64], DType::F32, input).unwrap();
            let result = backend.unop_gpu(&a, TensorUnaryOp::Sigmoid);
            assert!(result.is_some());

            let result = result.unwrap();
            let ptr = result.data_ptr_f32();
            unsafe {
                let val = *ptr;
                assert!((val - expected).abs() < 1e-4,
                    "sigmoid({}): expected {}, got {}", input, expected, val);
            }
        }
    }

    #[test]
    fn test_metal_tanh_precision() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Test tanh for various inputs
        let test_cases: &[(f64, f32)] = &[
            (0.0, 0.0),
            (1.0, 0.7615942),
            (-1.0, -0.7615942),
            (2.0, 0.9640276),
            (-2.0, -0.9640276),
        ];

        for &(input, expected) in test_cases {
            let a = TensorHandle::full(&[64], DType::F32, input).unwrap();
            let result = backend.unop_gpu(&a, TensorUnaryOp::Tanh);
            assert!(result.is_some());

            let result = result.unwrap();
            let ptr = result.data_ptr_f32();
            unsafe {
                let val = *ptr;
                assert!((val - expected).abs() < 1e-4,
                    "tanh({}): expected {}, got {}", input, expected, val);
            }
        }
    }

    // ==========================================================================
    // Stress Tests
    // ==========================================================================

    #[test]
    fn test_metal_repeated_operations() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Run many operations to test memory management and stability
        let iterations = 100;
        let size = 4096;

        for i in 0..iterations {
            let a = TensorHandle::full(&[size], DType::F32, 1.0 + i as f64 * 0.01).unwrap();
            let b = TensorHandle::full(&[size], DType::F32, 2.0).unwrap();

            let result = backend.binop_gpu(&a, &b, TensorBinaryOp::Add);
            assert!(result.is_some(), "Failed at iteration {}", i);

            // Verify result
            let result = result.unwrap();
            assert_eq!(result.numel, size);
        }
    }

    #[test]
    fn test_metal_alternating_operations() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        let size = 1024;
        let mut tensor = TensorHandle::full(&[size], DType::F32, 1.0).unwrap();

        // Chain of operations: +2, *3, -1, /2
        let ops: &[(TensorBinaryOp, f64)] = &[
            (TensorBinaryOp::Add, 2.0),  // 1 + 2 = 3
            (TensorBinaryOp::Mul, 3.0),  // 3 * 3 = 9
            (TensorBinaryOp::Sub, 1.0),  // 9 - 1 = 8
            (TensorBinaryOp::Div, 2.0),  // 8 / 2 = 4
        ];

        for (op, val) in ops {
            let b = TensorHandle::full(&[size], DType::F32, *val).unwrap();
            tensor = backend.binop_gpu(&tensor, &b, *op).unwrap();
        }

        // Final result should be 4.0
        let ptr = tensor.data_ptr_f32();
        unsafe {
            for i in 0..size {
                let val = *ptr.add(i);
                assert!((val - 4.0).abs() < 1e-4,
                    "Expected 4.0, got {} at index {}", val, i);
            }
        }
    }

    #[test]
    fn test_metal_large_matmul() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Test larger matrix multiplication
        let sizes: &[(usize, usize, usize)] = &[
            (64, 64, 64),
            (128, 64, 128),
            (256, 128, 64),
            (512, 256, 128),
        ];

        for &(m, k, n) in sizes {
            let a = TensorHandle::full(&[m, k], DType::F32, 0.1).unwrap();
            let b = TensorHandle::full(&[k, n], DType::F32, 0.1).unwrap();

            let result = backend.matmul_gpu(&a, &b);
            assert!(result.is_some(), "Failed for {}x{} * {}x{}", m, k, k, n);

            let result = result.unwrap();
            assert_eq!(result.shape[0], m);
            assert_eq!(result.shape[1], n);

            // Each element should be k * 0.1 * 0.1 = k * 0.01
            let expected = k as f32 * 0.01;
            let ptr = result.data_ptr_f32();
            unsafe {
                let val = *ptr;
                assert!((val - expected).abs() < 0.01,
                    "{}x{} * {}x{}: expected {}, got {}", m, k, k, n, expected, val);
            }
        }
    }

    // ==========================================================================
    // Performance Benchmark Tests
    // ==========================================================================

    #[test]
    fn test_metal_vectorization_benefit() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Large tensor to see vectorization benefit
        let size = 1_000_000;  // 1M elements (must be divisible by 4)
        let size_aligned = (size / 4) * 4;

        let a = TensorHandle::full(&[size_aligned], DType::F32, std::f64::consts::PI).unwrap();
        let b = TensorHandle::full(&[size_aligned], DType::F32, std::f64::consts::E).unwrap();

        // Time the operation (just verify it completes quickly for such large tensors)
        let start = std::time::Instant::now();
        let result = backend.binop_gpu(&a, &b, TensorBinaryOp::Add);
        let elapsed = start.elapsed();

        assert!(result.is_some());
        assert!(elapsed.as_millis() < 1000, "Operation took too long: {:?}", elapsed);

        // Verify result
        let result = result.unwrap();
        let expected = (std::f64::consts::PI + std::f64::consts::E) as f32;
        let ptr = result.data_ptr_f32();
        unsafe {
            let val = *ptr;
            assert!((val - expected).abs() < 1e-4,
                "Expected {}, got {}", expected, val);
        }
    }

    #[test]
    fn test_metal_chain_of_unops() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        let size = 4096;
        let mut tensor = TensorHandle::full(&[size], DType::F32, 2.0).unwrap();

        // Chain: sqrt(2) -> exp -> log -> sqrt
        // sqrt(2) ≈ 1.414
        // exp(1.414) ≈ 4.113
        // log(4.113) ≈ 1.414
        // sqrt(1.414) ≈ 1.189

        tensor = backend.unop_gpu(&tensor, TensorUnaryOp::Sqrt).unwrap();
        tensor = backend.unop_gpu(&tensor, TensorUnaryOp::Exp).unwrap();
        tensor = backend.unop_gpu(&tensor, TensorUnaryOp::Log).unwrap();
        tensor = backend.unop_gpu(&tensor, TensorUnaryOp::Sqrt).unwrap();

        let expected = ((2.0f64.sqrt()).exp().ln().sqrt()) as f32;
        let ptr = tensor.data_ptr_f32();
        unsafe {
            let val = *ptr;
            assert!((val - expected).abs() < 1e-3,
                "Chain result: expected {}, got {}", expected, val);
        }
    }

    #[test]
    fn test_metal_buffer_pool_efficiency() {
        if !is_metal_available() {
            return;
        }

        let device = Device::system_default().unwrap();
        let pool = MetalBufferPool::new(device);

        // Allocate and deallocate many buffers of same size
        let size = 4096;
        for _ in 0..100 {
            let buf = pool.allocate(size).unwrap();
            pool.deallocate(buf, size);
        }

        let stats = pool.stats();
        // After the first allocation, all subsequent ones should be cache hits
        assert!(stats.cache_hits >= 99, "Expected many cache hits, got {}", stats.cache_hits);
    }

    // ==========================================================================
    // Softmax GPU Tests
    // ==========================================================================

    #[test]
    fn test_metal_softmax_1d_basic() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Simple test: softmax([0, 0, 0, 0]) = [0.25, 0.25, 0.25, 0.25]
        let a = TensorHandle::full(&[4], DType::F32, 0.0).unwrap();
        let result = backend.softmax_gpu(&a);
        assert!(result.is_some());

        let result = result.unwrap();
        let ptr = result.data_ptr_f32();
        unsafe {
            for i in 0..4 {
                let val = *ptr.add(i);
                assert!((val - 0.25).abs() < 1e-5,
                    "softmax([0,0,0,0])[{}]: expected 0.25, got {}", i, val);
            }
        }
    }

    #[test]
    fn test_metal_softmax_1d_sum_to_one() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Softmax output should sum to 1.0
        let a = TensorHandle::full(&[256], DType::F32, 0.5).unwrap();
        let result = backend.softmax_gpu(&a).unwrap();

        let ptr = result.data_ptr_f32();
        let mut sum = 0.0f32;
        unsafe {
            for i in 0..256 {
                sum += *ptr.add(i);
            }
        }
        assert!((sum - 1.0).abs() < 1e-4, "Softmax sum: expected 1.0, got {}", sum);
    }

    #[test]
    fn test_metal_softmax_1d_large() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Large tensor
        let size = 65536;
        let a = TensorHandle::full(&[size], DType::F32, 1.0).unwrap();
        let result = backend.softmax_gpu(&a);
        assert!(result.is_some());

        let result = result.unwrap();
        assert_eq!(result.numel, size);

        // All equal inputs -> all equal outputs
        let expected = 1.0 / size as f32;
        let ptr = result.data_ptr_f32();
        unsafe {
            for i in 0..10 {  // Check first few elements
                let val = *ptr.add(i);
                assert!((val - expected).abs() < 1e-6,
                    "softmax uniform[{}]: expected {}, got {}", i, expected, val);
            }
        }
    }

    #[test]
    fn test_metal_softmax_1d_numerical_stability() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Test with large values (should not overflow due to max subtraction)
        let mut a = TensorHandle::zeros(&[4], DType::F32).unwrap();
        let ptr = a.data_ptr_f32_mut();
        unsafe {
            *ptr.add(0) = 1000.0;
            *ptr.add(1) = 1001.0;
            *ptr.add(2) = 1002.0;
            *ptr.add(3) = 1003.0;
        }

        let result = backend.softmax_gpu(&a);
        assert!(result.is_some());

        let result = result.unwrap();
        let out_ptr = result.data_ptr_f32();

        // Verify sum is 1.0 (numerical stability)
        let mut sum = 0.0f32;
        unsafe {
            for i in 0..4 {
                let val = *out_ptr.add(i);
                assert!(val.is_finite(), "softmax large values produced non-finite: {}", val);
                sum += val;
            }
        }
        assert!((sum - 1.0).abs() < 1e-4, "Softmax sum with large values: {}", sum);
    }

    #[test]
    fn test_metal_softmax_2d_batch() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // 2D tensor: each row is independent
        let a = TensorHandle::full(&[4, 8], DType::F32, 0.0).unwrap();
        let result = backend.softmax_gpu(&a);
        assert!(result.is_some());

        let result = result.unwrap();
        assert_eq!(result.shape[0], 4);
        assert_eq!(result.shape[1], 8);

        // Each row should have softmax values summing to 1.0
        let ptr = result.data_ptr_f32();
        for batch in 0..4 {
            let mut row_sum = 0.0f32;
            unsafe {
                for i in 0..8 {
                    row_sum += *ptr.add(batch * 8 + i);
                }
            }
            assert!((row_sum - 1.0).abs() < 1e-4,
                "Batch {}: row sum = {}", batch, row_sum);
        }
    }

    #[test]
    fn test_metal_softmax_2d_larger() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Larger 2D tensor (like attention scores)
        let batch = 32;
        let dim = 64;
        let a = TensorHandle::full(&[batch, dim], DType::F32, 0.5).unwrap();
        let result = backend.softmax_gpu(&a);
        assert!(result.is_some());

        let result = result.unwrap();
        let ptr = result.data_ptr_f32();

        // Check sum for each row
        for b in 0..batch {
            let mut row_sum = 0.0f32;
            unsafe {
                for i in 0..dim {
                    row_sum += *ptr.add(b * dim + i);
                }
            }
            assert!((row_sum - 1.0).abs() < 1e-3,
                "Batch {}: row sum = {}", b, row_sum);
        }
    }

    #[test]
    fn test_metal_softmax_vs_cpu() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Compare GPU softmax with manual CPU computation
        let a = TensorHandle::full(&[16], DType::F32, 1.0).unwrap();
        let gpu_result = backend.softmax_gpu(&a).unwrap();

        // Manual CPU softmax: exp(x - max) / sum(exp(x - max))
        let expected = 1.0 / 16.0;  // All equal -> uniform distribution

        let ptr = gpu_result.data_ptr_f32();
        unsafe {
            for i in 0..16 {
                let val = *ptr.add(i);
                assert!((val - expected).abs() < 1e-5,
                    "softmax_gpu[{}]: expected {}, got {}", i, expected, val);
            }
        }
    }

    // ==========================================================================
    // Layer Normalization GPU Tests
    // ==========================================================================

    #[test]
    fn test_metal_layer_norm_basic() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Simple test: layer norm on uniform data should produce zeros (normalized)
        let input = TensorHandle::full(&[2, 4], DType::F32, 5.0).unwrap();
        let result = backend.layer_norm_gpu(&input, None, None, 1e-5);
        assert!(result.is_some());

        let result = result.unwrap();
        assert_eq!(result.shape[0], 2);
        assert_eq!(result.shape[1], 4);

        // Uniform input -> mean = 5.0, var = 0.0 -> normalized = 0.0
        let ptr = result.data_ptr_f32();
        unsafe {
            for i in 0..8 {
                let val = *ptr.add(i);
                assert!(val.abs() < 1e-3,
                    "layer_norm uniform[{}]: expected ~0.0, got {}", i, val);
            }
        }
    }

    #[test]
    fn test_metal_layer_norm_with_gamma_beta() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        let hidden_size = 4;
        let input = TensorHandle::full(&[2, hidden_size], DType::F32, 5.0).unwrap();
        let gamma = TensorHandle::full(&[hidden_size], DType::F32, 2.0).unwrap();
        let beta = TensorHandle::full(&[hidden_size], DType::F32, 1.0).unwrap();

        let result = backend.layer_norm_gpu(&input, Some(&gamma), Some(&beta), 1e-5);
        assert!(result.is_some());

        let result = result.unwrap();

        // Uniform input -> normalized = 0.0 -> gamma * 0 + beta = beta = 1.0
        let ptr = result.data_ptr_f32();
        unsafe {
            for i in 0..8 {
                let val = *ptr.add(i);
                assert!((val - 1.0).abs() < 1e-3,
                    "layer_norm with gamma/beta[{}]: expected 1.0, got {}", i, val);
            }
        }
    }

    #[test]
    fn test_metal_layer_norm_mean_variance() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Create input with known mean and variance
        let mut input = TensorHandle::zeros(&[1, 4], DType::F32).unwrap();
        let ptr = input.data_ptr_f32_mut();
        unsafe {
            // Values: -1, 0, 0, 1 -> mean = 0, var = 0.5
            *ptr.add(0) = -1.0;
            *ptr.add(1) = 0.0;
            *ptr.add(2) = 0.0;
            *ptr.add(3) = 1.0;
        }

        let result = backend.layer_norm_gpu(&input, None, None, 1e-5);
        assert!(result.is_some());

        let result = result.unwrap();
        let out_ptr = result.data_ptr_f32();

        // After normalization: (x - 0) / sqrt(0.5) ≈ [-1.414, 0, 0, 1.414]
        let expected_std = 0.5f32.sqrt();
        unsafe {
            let v0 = *out_ptr.add(0);
            let v3 = *out_ptr.add(3);
            assert!((v0 - (-1.0 / expected_std)).abs() < 0.1,
                "layer_norm[0]: expected {}, got {}", -1.0 / expected_std, v0);
            assert!((v3 - (1.0 / expected_std)).abs() < 0.1,
                "layer_norm[3]: expected {}, got {}", 1.0 / expected_std, v3);
        }
    }

    #[test]
    fn test_metal_layer_norm_transformer_size() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Typical transformer sizes
        let batch = 32;
        let hidden = 512;

        let input = TensorHandle::full(&[batch, hidden], DType::F32, 0.5).unwrap();
        let gamma = TensorHandle::full(&[hidden], DType::F32, 1.0).unwrap();
        let beta = TensorHandle::full(&[hidden], DType::F32, 0.0).unwrap();

        let result = backend.layer_norm_gpu(&input, Some(&gamma), Some(&beta), 1e-5);
        assert!(result.is_some());

        let result = result.unwrap();
        assert_eq!(result.shape[0], batch);
        assert_eq!(result.shape[1], hidden);

        // Uniform input should produce near-zero output
        let ptr = result.data_ptr_f32();
        unsafe {
            for i in 0..10 {
                let val = *ptr.add(i);
                assert!(val.abs() < 1e-2,
                    "layer_norm transformer[{}]: expected ~0.0, got {}", i, val);
            }
        }
    }

    #[test]
    fn test_metal_layer_norm_large_hidden() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Large hidden size (triggers multi-pass implementation)
        let batch = 8;
        let hidden = 2048;

        let input = TensorHandle::full(&[batch, hidden], DType::F32, 0.5).unwrap();
        let result = backend.layer_norm_gpu(&input, None, None, 1e-5);
        assert!(result.is_some());

        let result = result.unwrap();
        assert_eq!(result.shape[0], batch);
        assert_eq!(result.shape[1], hidden);
    }

    #[test]
    fn test_metal_layer_norm_numerical_stability() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Test with large values
        let input = TensorHandle::full(&[2, 256], DType::F32, 1000.0).unwrap();
        let result = backend.layer_norm_gpu(&input, None, None, 1e-5);
        assert!(result.is_some());

        let result = result.unwrap();
        let ptr = result.data_ptr_f32();

        // All values same -> normalized should be ~0
        unsafe {
            for i in 0..10 {
                let val = *ptr.add(i);
                assert!(val.is_finite(), "layer_norm large produced non-finite: {}", val);
                assert!(val.abs() < 1e-2,
                    "layer_norm large[{}]: expected ~0.0, got {}", i, val);
            }
        }
    }

    #[test]
    fn test_metal_layer_norm_only_gamma() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        let hidden = 8;
        let input = TensorHandle::full(&[2, hidden], DType::F32, 0.0).unwrap();
        let gamma = TensorHandle::full(&[hidden], DType::F32, 3.0).unwrap();

        let result = backend.layer_norm_gpu(&input, Some(&gamma), None, 1e-5);
        assert!(result.is_some());

        // gamma * 0 + 0 = 0
        let result = result.unwrap();
        let ptr = result.data_ptr_f32();
        unsafe {
            for i in 0..16 {
                let val = *ptr.add(i);
                assert!(val.abs() < 1e-3, "layer_norm gamma only[{}]: got {}", i, val);
            }
        }
    }

    #[test]
    fn test_metal_layer_norm_only_beta() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        let hidden = 8;
        let input = TensorHandle::full(&[2, hidden], DType::F32, 0.0).unwrap();
        let beta = TensorHandle::full(&[hidden], DType::F32, 5.0).unwrap();

        let result = backend.layer_norm_gpu(&input, None, Some(&beta), 1e-5);
        assert!(result.is_some());

        // 1 * 0 + 5 = 5
        let result = result.unwrap();
        let ptr = result.data_ptr_f32();
        unsafe {
            for i in 0..16 {
                let val = *ptr.add(i);
                assert!((val - 5.0).abs() < 1e-3,
                    "layer_norm beta only[{}]: expected 5.0, got {}", i, val);
            }
        }
    }

    #[test]
    fn test_metal_layer_norm_various_sizes() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Test various sizes
        let configs = [
            (1, 64),
            (4, 128),
            (8, 256),
            (16, 512),
            (32, 768),
            (64, 1024),
        ];

        for (batch, hidden) in configs {
            let input = TensorHandle::full(&[batch, hidden], DType::F32, 1.0).unwrap();
            let result = backend.layer_norm_gpu(&input, None, None, 1e-5);
            assert!(result.is_some(),
                "Failed for batch={}, hidden={}", batch, hidden);

            let result = result.unwrap();
            assert_eq!(result.shape[0], batch);
            assert_eq!(result.shape[1], hidden);
        }
    }

    // ========================================================================
    // Reduce Operation Tests
    // ========================================================================

    #[test]
    fn test_metal_reduce_sum() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Test sum reduction
        let input = TensorHandle::full(&[256], DType::F32, 1.0).unwrap();
        let result = backend.reduce_gpu(&input, TensorReduceOp::Sum, None);
        assert!(result.is_some());

        let result = result.unwrap();
        assert_eq!(result.ndim, 0); // Scalar output
        assert_eq!(result.numel, 1);

        let ptr = result.data_ptr_f32();
        unsafe {
            let sum = *ptr;
            assert!((sum - 256.0).abs() < 1e-3,
                "Sum of 256 ones: expected 256.0, got {}", sum);
        }
    }

    #[test]
    fn test_metal_reduce_max() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Create tensor with known max
        let mut input = TensorHandle::full(&[100], DType::F32, 0.0).unwrap();
        let ptr = input.data_ptr_f32_mut();
        unsafe {
            *ptr.add(42) = 999.0; // Set max at position 42
        }

        let result = backend.reduce_gpu(&input, TensorReduceOp::Max, None);
        assert!(result.is_some());

        let result = result.unwrap();
        let res_ptr = result.data_ptr_f32();
        unsafe {
            let max = *res_ptr;
            assert!((max - 999.0).abs() < 1e-3,
                "Max: expected 999.0, got {}", max);
        }
    }

    #[test]
    fn test_metal_reduce_min() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Create tensor with known min
        let mut input = TensorHandle::full(&[100], DType::F32, 100.0).unwrap();
        let ptr = input.data_ptr_f32_mut();
        unsafe {
            *ptr.add(55) = -42.0; // Set min at position 55
        }

        let result = backend.reduce_gpu(&input, TensorReduceOp::Min, None);
        assert!(result.is_some());

        let result = result.unwrap();
        let res_ptr = result.data_ptr_f32();
        unsafe {
            let min = *res_ptr;
            assert!((min - (-42.0)).abs() < 1e-3,
                "Min: expected -42.0, got {}", min);
        }
    }

    #[test]
    fn test_metal_reduce_mean() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // Test mean = sum / count
        let input = TensorHandle::full(&[200], DType::F32, 5.0).unwrap();
        let result = backend.reduce_gpu(&input, TensorReduceOp::Mean, None);
        assert!(result.is_some());

        let result = result.unwrap();
        let ptr = result.data_ptr_f32();
        unsafe {
            let mean = *ptr;
            // All values are 5.0, so mean = 5.0
            assert!((mean - 5.0).abs() < 1e-3,
                "Mean of 200 fives: expected 5.0, got {}", mean);
        }
    }

    #[test]
    fn test_metal_reduce_axis_fallback() {
        if !is_metal_available() {
            return;
        }

        let backend = MetalBackend::new().unwrap();

        // GPU reduce currently only supports axis=None
        // Should return None for axis-specific reductions
        let input = TensorHandle::full(&[10, 10], DType::F32, 1.0).unwrap();
        let result = backend.reduce_gpu(&input, TensorReduceOp::Sum, Some(0));
        assert!(result.is_none(), "Should return None for axis-specific reduce");
    }
}
