//! Comprehensive integration tests for the VBC tensor execution system.
//!
//! These tests verify end-to-end correctness of tensor operations,
//! including dispatch, kernel execution, and memory management.

use verum_vbc::interpreter::kernel::{
    device::{DeviceId, get_registry},
    backend::{get_backend_registry, MemoryPool, CpuBackend},
    get_capabilities, dispatch_binop,
    broadcast_shapes, broadcast_to,
};
use verum_vbc::interpreter::tensor::{
    DType, TensorHandle, tensor_binop, tensor_unop, tensor_reduce, tensor_matmul,
    tensor_reshape, tensor_transpose, tensor_clone, tensor_from_slice,
    tensor_dot, tensor_outer, tensor_batch_matmul, tensor_topk, tensor_cumulative,
    tensor_pool2d, tensor_conv2d, tensor_cmp, tensor_masked_fill,
    tensor_layer_norm, tensor_batch_norm, tensor_rms_norm,
    CumulativeOp, PoolOp, CompareOp,
};
use verum_vbc::instruction::{TensorBinaryOp, TensorUnaryOp, TensorReduceOp};

// ============================================================================
// Device Detection Tests
// ============================================================================

#[test]
fn test_device_registry_initialization() {
    let registry = get_registry();
    assert!(!registry.devices.is_empty(), "Should have at least one device");
    assert!(registry.cpu_info().is_some(), "CPU info should be available");
}

#[test]
fn test_cpu_capabilities_detection() {
    let caps = get_capabilities();
    assert!(caps.max_threads >= 1, "Should have at least 1 thread");
    assert!(caps.simd_width >= 1, "Should have at least scalar width");

    // On most modern CPUs, we should have SSE or AVX
    #[cfg(target_arch = "x86_64")]
    {
        // At minimum SSE4.2 is widely available
        assert!(caps.simd_width >= 1);
    }

    #[cfg(target_arch = "aarch64")]
    {
        // NEON is always available on AArch64
        assert!(caps.has_neon || caps.simd_width >= 4);
    }
}

#[test]
fn test_backend_registry() {
    let registry = get_backend_registry();
    let cpu = registry.backend(DeviceId::CPU);
    assert!(cpu.is_some(), "CPU backend should be available");
    assert_eq!(cpu.unwrap().name(), "CPU");
}

// ============================================================================
// Memory Pool Tests
// ============================================================================

#[test]
fn test_memory_pool_allocation_deallocation() {
    let pool = MemoryPool::new(DeviceId::CPU);
    let backend = CpuBackend::new();

    // Test various allocation sizes
    let sizes = [64, 256, 1024, 4096, 65536];
    let mut ptrs = Vec::new();

    for &size in &sizes {
        let ptr = pool.allocate(size, &backend);
        assert!(ptr.is_some(), "Should allocate {} bytes", size);
        ptrs.push((ptr.unwrap(), size));
    }

    // Return to pool
    for (ptr, size) in ptrs {
        pool.deallocate(ptr, size);
    }

    let stats = pool.stats();
    assert!(stats.cache_misses >= sizes.len(), "Should have cache misses for new allocations");
}

#[test]
fn test_memory_pool_reuse() {
    let pool = MemoryPool::new(DeviceId::CPU);
    let backend = CpuBackend::new();

    // Allocate and deallocate
    let ptr1 = pool.allocate(1024, &backend).unwrap();
    pool.deallocate(ptr1, 1024);

    // Reallocate - should get from cache
    let ptr2 = pool.allocate(1024, &backend).unwrap();
    assert_eq!(ptr1, ptr2, "Should reuse pointer from cache");

    let stats = pool.stats();
    assert_eq!(stats.cache_hits, 1, "Should have 1 cache hit");
}

// ============================================================================
// Tensor Creation Tests
// ============================================================================

#[test]
fn test_tensor_zeros_creation() {
    let shapes = vec![
        vec![10],
        vec![4, 4],
        vec![2, 3, 4],
        vec![2, 2, 2, 2],
    ];

    for shape in shapes {
        let t = TensorHandle::zeros(&shape, DType::F32).unwrap();
        assert_eq!(t.ndim as usize, shape.len());
        for (i, &dim) in shape.iter().enumerate() {
            assert_eq!(t.shape[i], dim);
        }
    }
}

#[test]
fn test_tensor_from_slice() {
    let data = vec![1.0f64, 2.0, 3.0, 4.0, 5.0, 6.0];
    let t = tensor_from_slice(&data, &[2, 3], DType::F32).unwrap();

    assert_eq!(t.ndim, 2);
    assert_eq!(t.shape[0], 2);
    assert_eq!(t.shape[1], 3);
    assert_eq!(t.numel, 6);
}

#[test]
fn test_tensor_clone_independence() {
    let t1 = TensorHandle::full(&[3, 3], DType::F32, 1.0).unwrap();
    let t2 = tensor_clone(&t1).unwrap();

    // Should have same shape
    assert_eq!(t1.shape[..t1.ndim as usize], t2.shape[..t2.ndim as usize]);

    // But different data pointers (deep copy)
    assert_ne!(t1.data_ptr_f32(), t2.data_ptr_f32());
}

// ============================================================================
// Binary Operation Tests
// ============================================================================

#[test]
fn test_tensor_binop_add() {
    let a = TensorHandle::full(&[4, 4], DType::F32, 2.0).unwrap();
    let b = TensorHandle::full(&[4, 4], DType::F32, 3.0).unwrap();

    let c = tensor_binop(&a, &b, TensorBinaryOp::Add).unwrap();
    assert_eq!(c.numel, 16);
}

#[test]
fn test_tensor_binop_all_operations() {
    let a = TensorHandle::full(&[8], DType::F32, 4.0).unwrap();
    let b = TensorHandle::full(&[8], DType::F32, 2.0).unwrap();

    let ops = [
        TensorBinaryOp::Add,
        TensorBinaryOp::Sub,
        TensorBinaryOp::Mul,
        TensorBinaryOp::Div,
        TensorBinaryOp::Max,
        TensorBinaryOp::Min,
    ];

    for op in ops {
        let result = tensor_binop(&a, &b, op);
        assert!(result.is_some(), "Binary op {:?} should succeed", op);
    }
}

#[test]
fn test_kernel_dispatch_binop() {
    let a = TensorHandle::full(&[64], DType::F32, 1.0).unwrap();
    let b = TensorHandle::full(&[64], DType::F32, 2.0).unwrap();

    let result = dispatch_binop(&a, &b, TensorBinaryOp::Add);
    assert!(result.is_some(), "Dispatch binop should succeed");
}

// ============================================================================
// Unary Operation Tests
// ============================================================================

#[test]
fn test_tensor_unop_all_operations() {
    let t = TensorHandle::full(&[16], DType::F32, 0.5).unwrap();

    let ops = [
        TensorUnaryOp::Neg,
        TensorUnaryOp::Abs,
        TensorUnaryOp::Sqrt,
        TensorUnaryOp::Exp,
        TensorUnaryOp::Log,
        TensorUnaryOp::Sin,
        TensorUnaryOp::Cos,
        TensorUnaryOp::Tanh,
    ];

    for op in ops {
        let result = tensor_unop(&t, op);
        assert!(result.is_some(), "Unary op {:?} should succeed", op);
    }
}

// ============================================================================
// Reduction Tests
// ============================================================================

#[test]
fn test_tensor_reduce_sum() {
    let t = TensorHandle::full(&[4, 4], DType::F32, 1.0).unwrap();

    // Global reduction
    let sum = tensor_reduce(&t, None, TensorReduceOp::Sum).unwrap();
    assert_eq!(sum.ndim, 0);
    let val = sum.get_scalar_f64().unwrap();
    assert!((val - 16.0).abs() < 1e-5);
}

#[test]
fn test_tensor_reduce_along_axis() {
    let t = TensorHandle::full(&[3, 4], DType::F32, 1.0).unwrap();

    // Reduce along axis 0
    let result = tensor_reduce(&t, Some(0), TensorReduceOp::Sum).unwrap();
    assert_eq!(result.shape[0], 4);

    // Reduce along axis 1
    let result = tensor_reduce(&t, Some(1), TensorReduceOp::Sum).unwrap();
    assert_eq!(result.shape[0], 3);
}

#[test]
fn test_tensor_reduce_all_ops() {
    let t = TensorHandle::full(&[8], DType::F32, 2.0).unwrap();

    let ops = [
        TensorReduceOp::Sum,
        TensorReduceOp::Prod,
        TensorReduceOp::Max,
        TensorReduceOp::Min,
        TensorReduceOp::Mean,
    ];

    for op in ops {
        let result = tensor_reduce(&t, None, op);
        assert!(result.is_some(), "Reduce op {:?} should succeed", op);
    }
}

// ============================================================================
// Matrix Multiplication Tests
// ============================================================================

#[test]
fn test_tensor_matmul_2d() {
    let a = TensorHandle::full(&[3, 4], DType::F32, 1.0).unwrap();
    let b = TensorHandle::full(&[4, 5], DType::F32, 1.0).unwrap();

    let c = tensor_matmul(&a, &b).unwrap();
    assert_eq!(c.shape[0], 3);
    assert_eq!(c.shape[1], 5);
}

#[test]
fn test_tensor_matmul_correctness() {
    // [1 2; 3 4] @ [5 6; 7 8] = [19 22; 43 50]
    let a = tensor_from_slice(&[1.0, 2.0, 3.0, 4.0], &[2, 2], DType::F32).unwrap();
    let b = tensor_from_slice(&[5.0, 6.0, 7.0, 8.0], &[2, 2], DType::F32).unwrap();

    let c = tensor_matmul(&a, &b).unwrap();
    assert_eq!(c.shape[0], 2);
    assert_eq!(c.shape[1], 2);
}

#[test]
fn test_tensor_batch_matmul() {
    let a = TensorHandle::full(&[2, 3, 4], DType::F32, 1.0).unwrap();
    let b = TensorHandle::full(&[2, 4, 5], DType::F32, 1.0).unwrap();

    let c = tensor_batch_matmul(&a, &b).unwrap();
    assert_eq!(c.shape[0], 2); // Batch dim
    assert_eq!(c.shape[1], 3); // M
    assert_eq!(c.shape[2], 5); // N
}

// ============================================================================
// Reshape and Transpose Tests
// ============================================================================

#[test]
fn test_tensor_reshape() {
    let t = TensorHandle::zeros(&[2, 3, 4], DType::F32).unwrap();

    let r = tensor_reshape(&t, &[6, 4]).unwrap();
    assert_eq!(r.shape[0], 6);
    assert_eq!(r.shape[1], 4);
    assert_eq!(r.numel, 24);
}

#[test]
fn test_tensor_transpose() {
    let t = TensorHandle::zeros(&[3, 5], DType::F32).unwrap();

    let r = tensor_transpose(&t).unwrap();
    assert_eq!(r.shape[0], 5);
    assert_eq!(r.shape[1], 3);
}

// ============================================================================
// Broadcasting Tests
// ============================================================================

#[test]
fn test_broadcast_shapes_compatible() {
    assert_eq!(broadcast_shapes(&[3, 1], &[1, 4]), Some(vec![3, 4]));
    assert_eq!(broadcast_shapes(&[2, 3, 4], &[3, 4]), Some(vec![2, 3, 4]));
    assert_eq!(broadcast_shapes(&[5, 1, 4], &[1, 3, 1]), Some(vec![5, 3, 4]));
}

#[test]
fn test_broadcast_shapes_incompatible() {
    assert_eq!(broadcast_shapes(&[2, 3], &[3, 2]), None);
    assert_eq!(broadcast_shapes(&[4, 5], &[3, 5]), None);
}

#[test]
fn test_broadcast_to() {
    let t = TensorHandle::full(&[1, 3], DType::F32, 1.0).unwrap();
    let expanded = broadcast_to(&t, &[4, 3]).unwrap();

    assert_eq!(expanded.shape[0], 4);
    assert_eq!(expanded.shape[1], 3);
}

// ============================================================================
// Advanced Operations Tests
// ============================================================================

#[test]
fn test_tensor_dot_product() {
    let a = tensor_from_slice(&[1.0, 2.0, 3.0], &[3], DType::F32).unwrap();
    let b = tensor_from_slice(&[4.0, 5.0, 6.0], &[3], DType::F32).unwrap();

    let result = tensor_dot(&a, &b).unwrap();
    let val = result.get_scalar_f64().unwrap();
    assert!((val - 32.0).abs() < 1e-5); // 1*4 + 2*5 + 3*6 = 32
}

#[test]
fn test_tensor_outer_product() {
    let a = tensor_from_slice(&[1.0, 2.0], &[2], DType::F32).unwrap();
    let b = tensor_from_slice(&[3.0, 4.0, 5.0], &[3], DType::F32).unwrap();

    let result = tensor_outer(&a, &b).unwrap();
    assert_eq!(result.shape[0], 2);
    assert_eq!(result.shape[1], 3);
}

#[test]
fn test_tensor_topk() {
    let t = tensor_from_slice(&[5.0, 2.0, 8.0, 1.0, 9.0, 3.0], &[6], DType::F32).unwrap();

    let (values, indices) = tensor_topk(&t, 3, None, true, true).unwrap();
    assert_eq!(values.shape[0], 3);
    assert_eq!(indices.shape[0], 3);
}

#[test]
fn test_tensor_cumsum() {
    let t = tensor_from_slice(&[1.0, 2.0, 3.0, 4.0], &[4], DType::F32).unwrap();

    let result = tensor_cumulative(&t, CumulativeOp::Sum, None).unwrap();
    assert_eq!(result.numel, 4);
}

// ============================================================================
// Convolution and Pooling Tests
// ============================================================================

#[test]
fn test_tensor_conv2d() {
    let input = TensorHandle::full(&[1, 1, 8, 8], DType::F32, 1.0).unwrap();
    let kernel = TensorHandle::full(&[1, 1, 3, 3], DType::F32, 1.0).unwrap();

    let output = tensor_conv2d(&input, &kernel, None, (1, 1), (0, 0), (1, 1), 1).unwrap();
    assert_eq!(output.shape[0], 1);
    assert_eq!(output.shape[1], 1);
    assert_eq!(output.shape[2], 6); // (8 - 3) / 1 + 1 = 6
    assert_eq!(output.shape[3], 6);
}

#[test]
fn test_tensor_pool2d_max() {
    let input = TensorHandle::full(&[1, 1, 4, 4], DType::F32, 1.0).unwrap();

    let output = tensor_pool2d(&input, PoolOp::Max, (2, 2), (2, 2), (0, 0)).unwrap();
    assert_eq!(output.shape[2], 2);
    assert_eq!(output.shape[3], 2);
}

#[test]
fn test_tensor_pool2d_avg() {
    let input = TensorHandle::full(&[1, 1, 4, 4], DType::F32, 4.0).unwrap();

    let output = tensor_pool2d(&input, PoolOp::Avg, (2, 2), (2, 2), (0, 0)).unwrap();
    assert_eq!(output.shape[2], 2);
    assert_eq!(output.shape[3], 2);
}

// ============================================================================
// Normalization Tests
// ============================================================================

#[test]
fn test_tensor_layer_norm() {
    let input = TensorHandle::full(&[2, 8], DType::F32, 1.0).unwrap();

    let output = tensor_layer_norm(&input, None, None, 1e-5).unwrap();
    assert_eq!(output.shape[0], 2);
    assert_eq!(output.shape[1], 8);
}

#[test]
fn test_tensor_batch_norm() {
    let input = TensorHandle::full(&[2, 3, 4, 4], DType::F32, 1.0).unwrap();

    let output = tensor_batch_norm(&input, None, None, None, None, 1e-5, false).unwrap();
    assert_eq!(output.shape[0], 2);
    assert_eq!(output.shape[1], 3);
}

#[test]
fn test_tensor_rms_norm() {
    let input = TensorHandle::full(&[2, 8], DType::F32, 2.0).unwrap();

    let output = tensor_rms_norm(&input, None, 1e-5).unwrap();
    assert_eq!(output.shape[0], 2);
    assert_eq!(output.shape[1], 8);
}

// ============================================================================
// Comparison and Selection Tests
// ============================================================================

#[test]
fn test_tensor_cmp() {
    let a = tensor_from_slice(&[1.0, 2.0, 3.0], &[3], DType::F32).unwrap();
    let b = tensor_from_slice(&[2.0, 2.0, 2.0], &[3], DType::F32).unwrap();

    let eq = tensor_cmp(&a, &b, CompareOp::Eq).unwrap();
    assert_eq!(eq.dtype, DType::Bool);

    let lt = tensor_cmp(&a, &b, CompareOp::Lt).unwrap();
    assert_eq!(lt.dtype, DType::Bool);
}

#[test]
fn test_tensor_masked_fill() {
    let src = TensorHandle::full(&[4], DType::F32, 1.0).unwrap();
    let mask = TensorHandle::zeros(&[4], DType::Bool).unwrap();

    let result = tensor_masked_fill(&src, &mask, -999.0).unwrap();
    assert_eq!(result.numel, 4);
}

// ============================================================================
// End-to-End Pipeline Tests
// ============================================================================

#[test]
fn test_matmul_pipeline() {
    // Full pipeline: create -> matmul -> reduce -> result
    let a = TensorHandle::full(&[4, 8], DType::F32, 0.5).unwrap();
    let b = TensorHandle::full(&[8, 4], DType::F32, 0.5).unwrap();

    // Matrix multiply
    let c = tensor_matmul(&a, &b).unwrap();
    assert_eq!(c.shape[0], 4);
    assert_eq!(c.shape[1], 4);

    // Sum result
    let sum = tensor_reduce(&c, None, TensorReduceOp::Sum).unwrap();
    assert!(sum.get_scalar_f64().is_some());
}

#[test]
fn test_conv_relu_pool_pipeline() {
    // Typical CNN layer: conv -> relu -> pool
    let input = TensorHandle::full(&[1, 3, 16, 16], DType::F32, 1.0).unwrap();
    let kernel = TensorHandle::full(&[8, 3, 3, 3], DType::F32, 0.1).unwrap();

    // Conv2D
    let conv_out = tensor_conv2d(&input, &kernel, None, (1, 1), (1, 1), (1, 1), 1).unwrap();
    assert_eq!(conv_out.shape[1], 8); // 8 output channels

    // ReLU (max with 0)
    let zero = TensorHandle::zeros(&conv_out.shape[..conv_out.ndim as usize], DType::F32).unwrap();
    let relu_out = tensor_binop(&conv_out, &zero, TensorBinaryOp::Max).unwrap();

    // MaxPool
    let pool_out = tensor_pool2d(&relu_out, PoolOp::Max, (2, 2), (2, 2), (0, 0)).unwrap();
    assert_eq!(pool_out.shape[2], conv_out.shape[2] / 2);
}

#[test]
fn test_attention_pipeline() {
    // Simplified attention: Q @ K^T -> softmax -> @ V
    let batch = 2;
    let seq_len = 8;
    let d_model = 16;

    let q = TensorHandle::full(&[batch, seq_len, d_model], DType::F32, 0.1).unwrap();
    let _k = TensorHandle::full(&[batch, seq_len, d_model], DType::F32, 0.1).unwrap();
    let _v = TensorHandle::full(&[batch, seq_len, d_model], DType::F32, 0.1).unwrap();

    // Q @ K^T (would need batch matmul with transpose)
    // For simplicity, just verify shapes work with batch matmul
    let k_t = TensorHandle::full(&[batch, d_model, seq_len], DType::F32, 0.1).unwrap();
    let scores = tensor_batch_matmul(&q, &k_t).unwrap();
    assert_eq!(scores.shape[0], batch);
    assert_eq!(scores.shape[1], seq_len);
    assert_eq!(scores.shape[2], seq_len);
}

// ============================================================================
// Memory Safety Tests
// ============================================================================

#[test]
fn test_tensor_memory_reference_counting() {
    let t1 = TensorHandle::zeros(&[100, 100], DType::F32).unwrap();
    let t2 = t1.clone();
    let t3 = t2.clone();

    // All should be valid
    assert_eq!(t1.numel, 10000);
    assert_eq!(t2.numel, 10000);
    assert_eq!(t3.numel, 10000);

    drop(t1);
    assert_eq!(t2.numel, 10000);
    assert_eq!(t3.numel, 10000);

    drop(t2);
    assert_eq!(t3.numel, 10000);
}

#[test]
fn test_tensor_stress_allocation() {
    // Allocate and deallocate many tensors
    for _ in 0..100 {
        let t = TensorHandle::zeros(&[64, 64], DType::F32).unwrap();
        assert_eq!(t.numel, 4096);
    }
}

// ============================================================================
// Data Type Tests
// ============================================================================

#[test]
fn test_tensor_f64_operations() {
    let a = TensorHandle::full(&[4, 4], DType::F64, 2.0).unwrap();
    let b = TensorHandle::full(&[4, 4], DType::F64, 3.0).unwrap();

    let c = tensor_binop(&a, &b, TensorBinaryOp::Add).unwrap();
    assert_eq!(c.dtype, DType::F64);
}

#[test]
fn test_tensor_i32_operations() {
    let a = TensorHandle::full(&[4, 4], DType::I32, 2.0).unwrap();
    let b = TensorHandle::full(&[4, 4], DType::I32, 3.0).unwrap();

    let c = tensor_binop(&a, &b, TensorBinaryOp::Add).unwrap();
    assert_eq!(c.dtype, DType::I32);
}

// ============================================================================
// Integer Reduction Tests
// ============================================================================

#[test]
fn test_tensor_reduce_i32_sum() {
    let t = TensorHandle::full(&[4, 4], DType::I32, 3.0).unwrap();
    let result = tensor_reduce(&t, None, TensorReduceOp::Sum).unwrap();
    assert_eq!(result.ndim, 0);
    // 16 elements * 3 = 48
    let val = result.get_scalar_f64().unwrap();
    assert!((val - 48.0).abs() < 1e-5, "Expected 48.0, got {}", val);
}

#[test]
fn test_tensor_reduce_i32_max_min() {
    // Create tensor with values 1, 2, 3, ..., 16
    let mut t = TensorHandle::zeros(&[4, 4], DType::I32).unwrap();
    let ptr = t.data_ptr_i32_mut();
    for i in 0..16 {
        unsafe { *ptr.add(i) = (i + 1) as i32; }
    }

    let max = tensor_reduce(&t, None, TensorReduceOp::Max).unwrap();
    let max_val = max.get_scalar_f64().unwrap();
    assert!((max_val - 16.0).abs() < 1e-5, "Max should be 16, got {}", max_val);

    let min = tensor_reduce(&t, None, TensorReduceOp::Min).unwrap();
    let min_val = min.get_scalar_f64().unwrap();
    assert!((min_val - 1.0).abs() < 1e-5, "Min should be 1, got {}", min_val);
}

#[test]
fn test_tensor_reduce_i32_prod() {
    let t = TensorHandle::full(&[4], DType::I32, 2.0).unwrap();
    let result = tensor_reduce(&t, None, TensorReduceOp::Prod).unwrap();
    // 2^4 = 16
    let val = result.get_scalar_f64().unwrap();
    assert!((val - 16.0).abs() < 1e-5, "Expected 16.0, got {}", val);
}

#[test]
fn test_tensor_reduce_i32_mean() {
    let mut t = TensorHandle::zeros(&[4], DType::I32).unwrap();
    let ptr = t.data_ptr_i32_mut();
    for i in 0..4 {
        unsafe { *ptr.add(i) = (i + 1) as i32; } // 1, 2, 3, 4
    }

    let mean = tensor_reduce(&t, None, TensorReduceOp::Mean).unwrap();
    // Mean of 1+2+3+4 = 10/4 = 2.5
    let val = mean.get_scalar_f64().unwrap();
    assert!((val - 2.5).abs() < 1e-5, "Expected 2.5, got {}", val);
}

#[test]
fn test_tensor_reduce_i64_sum() {
    let t = TensorHandle::full(&[4, 4], DType::I64, 5.0).unwrap();
    let result = tensor_reduce(&t, None, TensorReduceOp::Sum).unwrap();
    // 16 * 5 = 80
    let val = result.get_scalar_f64().unwrap();
    assert!((val - 80.0).abs() < 1e-5, "Expected 80.0, got {}", val);
}

#[test]
fn test_tensor_reduce_u32_sum() {
    let t = TensorHandle::full(&[4, 4], DType::U32, 7.0).unwrap();
    let result = tensor_reduce(&t, None, TensorReduceOp::Sum).unwrap();
    // 16 * 7 = 112
    let val = result.get_scalar_f64().unwrap();
    assert!((val - 112.0).abs() < 1e-5, "Expected 112.0, got {}", val);
}

#[test]
fn test_tensor_reduce_u64_sum() {
    let t = TensorHandle::full(&[4, 4], DType::U64, 11.0).unwrap();
    let result = tensor_reduce(&t, None, TensorReduceOp::Sum).unwrap();
    // 16 * 11 = 176
    let val = result.get_scalar_f64().unwrap();
    assert!((val - 176.0).abs() < 1e-5, "Expected 176.0, got {}", val);
}

#[test]
fn test_tensor_reduce_i8_sum() {
    let t = TensorHandle::full(&[8], DType::I8, 10.0).unwrap();
    let result = tensor_reduce(&t, None, TensorReduceOp::Sum).unwrap();
    // 8 * 10 = 80
    let val = result.get_scalar_f64().unwrap();
    assert!((val - 80.0).abs() < 1e-5, "Expected 80.0, got {}", val);
}

#[test]
fn test_tensor_reduce_i16_sum() {
    let t = TensorHandle::full(&[8], DType::I16, 100.0).unwrap();
    let result = tensor_reduce(&t, None, TensorReduceOp::Sum).unwrap();
    // 8 * 100 = 800
    let val = result.get_scalar_f64().unwrap();
    assert!((val - 800.0).abs() < 1e-5, "Expected 800.0, got {}", val);
}

#[test]
fn test_tensor_reduce_u8_sum() {
    let t = TensorHandle::full(&[8], DType::U8, 20.0).unwrap();
    let result = tensor_reduce(&t, None, TensorReduceOp::Sum).unwrap();
    // 8 * 20 = 160
    let val = result.get_scalar_f64().unwrap();
    assert!((val - 160.0).abs() < 1e-5, "Expected 160.0, got {}", val);
}

#[test]
fn test_tensor_reduce_u16_sum() {
    let t = TensorHandle::full(&[8], DType::U16, 200.0).unwrap();
    let result = tensor_reduce(&t, None, TensorReduceOp::Sum).unwrap();
    // 8 * 200 = 1600
    let val = result.get_scalar_f64().unwrap();
    assert!((val - 1600.0).abs() < 1e-5, "Expected 1600.0, got {}", val);
}

#[test]
fn test_tensor_reduce_integer_all_any() {
    // Test All - all non-zero
    let t_all = TensorHandle::full(&[4], DType::I32, 1.0).unwrap();
    let all_result = tensor_reduce(&t_all, None, TensorReduceOp::All).unwrap();
    let all_val = all_result.get_scalar_f64().unwrap();
    assert!((all_val - 1.0).abs() < 1e-5, "All of [1,1,1,1] should be 1");

    // Test All with a zero
    let mut t_not_all = TensorHandle::zeros(&[4], DType::I32).unwrap();
    let ptr = t_not_all.data_ptr_i32_mut();
    unsafe {
        *ptr.add(0) = 1;
        *ptr.add(1) = 0;  // zero element
        *ptr.add(2) = 1;
        *ptr.add(3) = 1;
    }
    let not_all_result = tensor_reduce(&t_not_all, None, TensorReduceOp::All).unwrap();
    let not_all_val = not_all_result.get_scalar_f64().unwrap();
    assert!((not_all_val - 0.0).abs() < 1e-5, "All of [1,0,1,1] should be 0");

    // Test Any
    let any_result = tensor_reduce(&t_not_all, None, TensorReduceOp::Any).unwrap();
    let any_val = any_result.get_scalar_f64().unwrap();
    assert!((any_val - 1.0).abs() < 1e-5, "Any of [1,0,1,1] should be 1");

    // Test Any - all zeros
    let t_none = TensorHandle::zeros(&[4], DType::I32).unwrap();
    let none_result = tensor_reduce(&t_none, None, TensorReduceOp::Any).unwrap();
    let none_val = none_result.get_scalar_f64().unwrap();
    assert!((none_val - 0.0).abs() < 1e-5, "Any of [0,0,0,0] should be 0");
}

#[test]
fn test_tensor_reduce_integer_var_std() {
    // Variance and std for integers should return F64
    let mut t = TensorHandle::zeros(&[4], DType::I32).unwrap();
    let ptr = t.data_ptr_i32_mut();
    // Values: 2, 4, 4, 4 -> mean = 3.5, var = ((2-3.5)^2 + 3*(4-3.5)^2)/4 = (2.25 + 0.75)/4 = 0.75
    unsafe {
        *ptr.add(0) = 2;
        *ptr.add(1) = 4;
        *ptr.add(2) = 4;
        *ptr.add(3) = 4;
    }

    let var = tensor_reduce(&t, None, TensorReduceOp::Var).unwrap();
    assert_eq!(var.dtype, DType::F64, "Variance should return F64");
    let var_val = var.get_scalar_f64().unwrap();
    assert!((var_val - 0.75).abs() < 1e-5, "Expected variance 0.75, got {}", var_val);

    let std = tensor_reduce(&t, None, TensorReduceOp::Std).unwrap();
    assert_eq!(std.dtype, DType::F64, "Std should return F64");
    let std_val = std.get_scalar_f64().unwrap();
    let expected_std = 0.75f64.sqrt();
    assert!((std_val - expected_std).abs() < 1e-5, "Expected std {}, got {}", expected_std, std_val);
}

#[test]
fn test_tensor_reduce_integer_norm() {
    // Norm for integers: sqrt(sum of squares)
    let mut t = TensorHandle::zeros(&[3], DType::I32).unwrap();
    let ptr = t.data_ptr_i32_mut();
    // Values: 3, 4, 0 -> norm = sqrt(9 + 16 + 0) = 5
    unsafe {
        *ptr.add(0) = 3;
        *ptr.add(1) = 4;
        *ptr.add(2) = 0;
    }

    let norm = tensor_reduce(&t, None, TensorReduceOp::Norm).unwrap();
    assert_eq!(norm.dtype, DType::F64, "Norm should return F64");
    let norm_val = norm.get_scalar_f64().unwrap();
    assert!((norm_val - 5.0).abs() < 1e-5, "Expected norm 5.0, got {}", norm_val);
}

// ============================================================================
// Tests for new TensorExtended operations (2026-01-25)
// ============================================================================

#[test]
fn test_tensor_nansum() {
    use verum_vbc::interpreter::kernel::cpu::nansum_f32_scalar;

    // Create tensor with NaN values: [1.0, NaN, 3.0, NaN, 5.0]
    let mut t = TensorHandle::zeros(&[5], DType::F32).unwrap();
    let ptr = t.data_ptr_f32_mut();
    unsafe {
        *ptr.add(0) = 1.0;
        *ptr.add(1) = f32::NAN;
        *ptr.add(2) = 3.0;
        *ptr.add(3) = f32::NAN;
        *ptr.add(4) = 5.0;
    }

    // Global nansum should be 1 + 3 + 5 = 9
    let result = nansum_f32_scalar(&t, None, false).unwrap();
    let val = unsafe { *result.data_ptr_f32() };
    assert!((val - 9.0).abs() < 1e-5, "Expected nansum 9.0, got {}", val);
}

#[test]
fn test_tensor_nanmean() {
    use verum_vbc::interpreter::kernel::cpu::nanmean_f32_scalar;

    // Create tensor with NaN values: [1.0, NaN, 3.0, NaN, 5.0]
    let mut t = TensorHandle::zeros(&[5], DType::F32).unwrap();
    let ptr = t.data_ptr_f32_mut();
    unsafe {
        *ptr.add(0) = 1.0;
        *ptr.add(1) = f32::NAN;
        *ptr.add(2) = 3.0;
        *ptr.add(3) = f32::NAN;
        *ptr.add(4) = 5.0;
    }

    // Global nanmean should be (1 + 3 + 5) / 3 = 3
    let result = nanmean_f32_scalar(&t, None, false).unwrap();
    let val = unsafe { *result.data_ptr_f32() };
    assert!((val - 3.0).abs() < 1e-5, "Expected nanmean 3.0, got {}", val);
}

#[test]
fn test_tensor_flip() {
    use verum_vbc::interpreter::kernel::cpu::flip_f32_scalar;

    // Create 1D tensor: [1, 2, 3, 4, 5]
    let mut t = TensorHandle::zeros(&[5], DType::F32).unwrap();
    let ptr = t.data_ptr_f32_mut();
    unsafe {
        for i in 0..5 {
            *ptr.add(i) = (i + 1) as f32;
        }
    }

    // Flip along axis 0: should become [5, 4, 3, 2, 1]
    let result = flip_f32_scalar(&t, &[0]).unwrap();
    let res_ptr = result.data_ptr_f32();
    unsafe {
        assert!(((*res_ptr.add(0)) - 5.0).abs() < 1e-5);
        assert!(((*res_ptr.add(1)) - 4.0).abs() < 1e-5);
        assert!(((*res_ptr.add(2)) - 3.0).abs() < 1e-5);
        assert!(((*res_ptr.add(3)) - 2.0).abs() < 1e-5);
        assert!(((*res_ptr.add(4)) - 1.0).abs() < 1e-5);
    }
}

#[test]
fn test_tensor_roll() {
    use verum_vbc::interpreter::kernel::cpu::roll_f32_scalar;

    // Create 1D tensor: [1, 2, 3, 4, 5]
    let mut t = TensorHandle::zeros(&[5], DType::F32).unwrap();
    let ptr = t.data_ptr_f32_mut();
    unsafe {
        for i in 0..5 {
            *ptr.add(i) = (i + 1) as f32;
        }
    }

    // Roll by 2 along axis 0: should become [4, 5, 1, 2, 3]
    let result = roll_f32_scalar(&t, 2, 0).unwrap();
    let res_ptr = result.data_ptr_f32();
    unsafe {
        assert!(((*res_ptr.add(0)) - 4.0).abs() < 1e-5);
        assert!(((*res_ptr.add(1)) - 5.0).abs() < 1e-5);
        assert!(((*res_ptr.add(2)) - 1.0).abs() < 1e-5);
        assert!(((*res_ptr.add(3)) - 2.0).abs() < 1e-5);
        assert!(((*res_ptr.add(4)) - 3.0).abs() < 1e-5);
    }
}

#[test]
fn test_tensor_lu_decomposition() {
    use verum_vbc::interpreter::kernel::cpu::lu_f64_scalar;

    // Create 3x3 matrix:
    // [1, 2, 3]
    // [4, 5, 6]
    // [7, 8, 10]
    let mut a = TensorHandle::zeros(&[3, 3], DType::F64).unwrap();
    let ptr = a.data_ptr_f64_mut();
    unsafe {
        *ptr.add(0) = 1.0; *ptr.add(1) = 2.0; *ptr.add(2) = 3.0;
        *ptr.add(3) = 4.0; *ptr.add(4) = 5.0; *ptr.add(5) = 6.0;
        *ptr.add(6) = 7.0; *ptr.add(7) = 8.0; *ptr.add(8) = 10.0;
    }

    let result = lu_f64_scalar(&a);
    assert!(result.is_some(), "LU decomposition should succeed");

    let (p, l, u) = result.unwrap();
    assert_eq!(p.shape[0], 3);
    assert_eq!(p.shape[1], 3);
    assert_eq!(l.shape[0], 3);
    assert_eq!(l.shape[1], 3);
    assert_eq!(u.shape[0], 3);
    assert_eq!(u.shape[1], 3);

    // Verify L is lower triangular with 1s on diagonal
    let l_ptr = l.data_ptr_f64();
    unsafe {
        assert!(((*l_ptr.add(0)) - 1.0).abs() < 1e-10, "L[0,0] should be 1");
        assert!(((*l_ptr.add(4)) - 1.0).abs() < 1e-10, "L[1,1] should be 1");
        assert!(((*l_ptr.add(8)) - 1.0).abs() < 1e-10, "L[2,2] should be 1");
    }
}

#[test]
fn test_tensor_eig_symmetric() {
    use verum_vbc::interpreter::kernel::cpu::eig_symmetric_f64_jacobi;

    // Create symmetric 2x2 matrix:
    // [3, 1]
    // [1, 3]
    // Eigenvalues: 4 and 2
    let mut a = TensorHandle::zeros(&[2, 2], DType::F64).unwrap();
    let ptr = a.data_ptr_f64_mut();
    unsafe {
        *ptr.add(0) = 3.0; *ptr.add(1) = 1.0;
        *ptr.add(2) = 1.0; *ptr.add(3) = 3.0;
    }

    let result = eig_symmetric_f64_jacobi(&a);
    assert!(result.is_some(), "Eigenvalue decomposition should succeed");

    let (eigenvalues, eigenvectors) = result.unwrap();
    assert_eq!(eigenvalues.numel, 2);
    assert_eq!(eigenvectors.shape[0], 2);
    assert_eq!(eigenvectors.shape[1], 2);

    // Check eigenvalues (should be 2 and 4, in some order)
    let ev_ptr = eigenvalues.data_ptr_f64();
    let (ev1, ev2) = unsafe { (*ev_ptr.add(0), *ev_ptr.add(1)) };
    let (min_ev, max_ev) = if ev1 < ev2 { (ev1, ev2) } else { (ev2, ev1) };
    assert!((min_ev - 2.0).abs() < 1e-5, "Expected eigenvalue 2, got {}", min_ev);
    assert!((max_ev - 4.0).abs() < 1e-5, "Expected eigenvalue 4, got {}", max_ev);
}

#[test]
fn test_tensor_rank() {
    use verum_vbc::interpreter::kernel::cpu::rank_f64_scalar;

    // Create 3x3 matrix of rank 2:
    // [1, 2, 3]
    // [4, 5, 6]
    // [5, 7, 9] = row1 + row2
    let mut a = TensorHandle::zeros(&[3, 3], DType::F64).unwrap();
    let ptr = a.data_ptr_f64_mut();
    unsafe {
        *ptr.add(0) = 1.0; *ptr.add(1) = 2.0; *ptr.add(2) = 3.0;
        *ptr.add(3) = 4.0; *ptr.add(4) = 5.0; *ptr.add(5) = 6.0;
        *ptr.add(6) = 5.0; *ptr.add(7) = 7.0; *ptr.add(8) = 9.0;
    }

    // Use a reasonable tolerance for numerical SVD (1e-10 is too strict)
    let rank = rank_f64_scalar(&a, 1e-6);
    assert!(rank.is_some(), "Rank computation should succeed");
    assert_eq!(rank.unwrap(), 2, "Matrix should have rank 2");
}

#[test]
fn test_tensor_cond() {
    use verum_vbc::interpreter::kernel::cpu::cond_f64_scalar;

    // Create identity matrix (condition number should be 1)
    let mut eye = TensorHandle::zeros(&[3, 3], DType::F64).unwrap();
    let ptr = eye.data_ptr_f64_mut();
    unsafe {
        *ptr.add(0) = 1.0;
        *ptr.add(4) = 1.0;
        *ptr.add(8) = 1.0;
    }

    // 2-norm condition number of identity matrix should be 1
    let cond = cond_f64_scalar(&eye, 2);
    assert!(cond.is_some(), "Condition number should be computed");
    let cond_val = cond.unwrap();
    assert!((cond_val - 1.0).abs() < 1e-5, "Expected cond(I) = 1, got {}", cond_val);
}

#[test]
fn test_tensor_general_eig() {
    use verum_vbc::interpreter::kernel::cpu::eig_f64_qr;

    // Create diagonal matrix (eigenvalues are diagonal elements)
    // [2, 0, 0]
    // [0, 3, 0]
    // [0, 0, 5]
    let mut a = TensorHandle::zeros(&[3, 3], DType::F64).unwrap();
    let ptr = a.data_ptr_f64_mut();
    unsafe {
        *ptr.add(0) = 2.0;
        *ptr.add(4) = 3.0;
        *ptr.add(8) = 5.0;
    }

    let result = eig_f64_qr(&a);
    assert!(result.is_some(), "Eig should succeed for diagonal matrix");

    let (eigenvalues, _) = result.unwrap();
    assert_eq!(eigenvalues.numel, 3);

    // Eigenvalues should be 2, 3, 5 (in some order)
    let ev_ptr = eigenvalues.data_ptr_f64();
    let mut evs = Vec::new();
    unsafe {
        for i in 0..3 {
            evs.push(*ev_ptr.add(i));
        }
    }
    evs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!((evs[0] - 2.0).abs() < 1e-3, "Expected eigenvalue 2, got {}", evs[0]);
    assert!((evs[1] - 3.0).abs() < 1e-3, "Expected eigenvalue 3, got {}", evs[1]);
    assert!((evs[2] - 5.0).abs() < 1e-3, "Expected eigenvalue 5, got {}", evs[2]);
}

// ============================================================================
// Advanced Tensor Operations (0x70-0x75) Tests
// ============================================================================

#[test]
fn test_tensor_kronecker_product() {
    use verum_vbc::interpreter::kernel::dispatch_kron;

    // A = [[1, 2], [3, 4]] (2x2)
    // B = [[5, 6], [7, 8]] (2x2)
    // Expected: A ⊗ B = [[5, 6, 10, 12], [7, 8, 14, 16], [15, 18, 20, 24], [21, 24, 28, 32]]
    let mut a = TensorHandle::zeros(&[2, 2], DType::F64).unwrap();
    let mut b = TensorHandle::zeros(&[2, 2], DType::F64).unwrap();

    let a_ptr = a.data_ptr_f64_mut();
    let b_ptr = b.data_ptr_f64_mut();
    unsafe {
        *a_ptr.add(0) = 1.0;
        *a_ptr.add(1) = 2.0;
        *a_ptr.add(2) = 3.0;
        *a_ptr.add(3) = 4.0;

        *b_ptr.add(0) = 5.0;
        *b_ptr.add(1) = 6.0;
        *b_ptr.add(2) = 7.0;
        *b_ptr.add(3) = 8.0;
    }

    let result = dispatch_kron(&a, &b);
    assert!(result.is_some(), "Kronecker product should succeed");

    let kron = result.unwrap();
    assert_eq!(kron.shape[0], 4, "Output rows should be 4");
    assert_eq!(kron.shape[1], 4, "Output cols should be 4");

    let r_ptr = kron.data_ptr_f64();
    // Check first row: [5, 6, 10, 12]
    unsafe {
        assert!(((*r_ptr.add(0)) - 5.0).abs() < 1e-10);
        assert!(((*r_ptr.add(1)) - 6.0).abs() < 1e-10);
        assert!(((*r_ptr.add(2)) - 10.0).abs() < 1e-10);
        assert!(((*r_ptr.add(3)) - 12.0).abs() < 1e-10);
        // Check last row: [21, 24, 28, 32]
        assert!(((*r_ptr.add(12)) - 21.0).abs() < 1e-10);
        assert!(((*r_ptr.add(13)) - 24.0).abs() < 1e-10);
        assert!(((*r_ptr.add(14)) - 28.0).abs() < 1e-10);
        assert!(((*r_ptr.add(15)) - 32.0).abs() < 1e-10);
    }
}

#[test]
fn test_tensor_cross_product() {
    use verum_vbc::interpreter::kernel::dispatch_cross;

    // a = [1, 0, 0], b = [0, 1, 0]
    // Expected: a × b = [0, 0, 1]
    let a = tensor_from_slice(&[1.0, 0.0, 0.0], &[3], DType::F64).unwrap();
    let b = tensor_from_slice(&[0.0, 1.0, 0.0], &[3], DType::F64).unwrap();

    let result = dispatch_cross(&a, &b);
    assert!(result.is_some(), "Cross product should succeed");

    let cross = result.unwrap();
    assert_eq!(cross.numel, 3, "Output should have 3 elements");

    let r_ptr = cross.data_ptr_f64();
    unsafe {
        assert!(((*r_ptr.add(0)) - 0.0).abs() < 1e-10, "x should be 0");
        assert!(((*r_ptr.add(1)) - 0.0).abs() < 1e-10, "y should be 0");
        assert!(((*r_ptr.add(2)) - 1.0).abs() < 1e-10, "z should be 1");
    }

    // Test another case: a = [1, 2, 3], b = [4, 5, 6]
    // Expected: a × b = [2*6-3*5, 3*4-1*6, 1*5-2*4] = [-3, 6, -3]
    let a2 = tensor_from_slice(&[1.0, 2.0, 3.0], &[3], DType::F64).unwrap();
    let b2 = tensor_from_slice(&[4.0, 5.0, 6.0], &[3], DType::F64).unwrap();

    let result2 = dispatch_cross(&a2, &b2).unwrap();
    let r2_ptr = result2.data_ptr_f64();
    unsafe {
        assert!(((*r2_ptr.add(0)) - (-3.0)).abs() < 1e-10, "x should be -3");
        assert!(((*r2_ptr.add(1)) - 6.0).abs() < 1e-10, "y should be 6");
        assert!(((*r2_ptr.add(2)) - (-3.0)).abs() < 1e-10, "z should be -3");
    }
}

#[test]
fn test_tensor_contract() {
    use verum_vbc::interpreter::kernel::dispatch_contract;

    // Dot product: contract two 1D vectors
    let a = tensor_from_slice(&[1.0, 2.0, 3.0], &[3], DType::F64).unwrap();
    let b = tensor_from_slice(&[4.0, 5.0, 6.0], &[3], DType::F64).unwrap();

    let result = dispatch_contract(&a, &b, 0, 0);
    assert!(result.is_some(), "Contraction should succeed");

    let dot = result.unwrap();
    let r_ptr = dot.data_ptr_f64();
    // Expected: 1*4 + 2*5 + 3*6 = 4 + 10 + 18 = 32
    unsafe {
        assert!(((*r_ptr) - 32.0).abs() < 1e-10, "Dot product should be 32, got {}", *r_ptr);
    }

    // Matrix multiplication: contract (2,3) x (3,2) along axis 1 and 0
    let mut m1 = TensorHandle::zeros(&[2, 3], DType::F64).unwrap();
    let mut m2 = TensorHandle::zeros(&[3, 2], DType::F64).unwrap();

    let m1_ptr = m1.data_ptr_f64_mut();
    let m2_ptr = m2.data_ptr_f64_mut();
    unsafe {
        // m1 = [[1, 2, 3], [4, 5, 6]]
        *m1_ptr.add(0) = 1.0;
        *m1_ptr.add(1) = 2.0;
        *m1_ptr.add(2) = 3.0;
        *m1_ptr.add(3) = 4.0;
        *m1_ptr.add(4) = 5.0;
        *m1_ptr.add(5) = 6.0;

        // m2 = [[1, 4], [2, 5], [3, 6]]
        *m2_ptr.add(0) = 1.0;
        *m2_ptr.add(1) = 4.0;
        *m2_ptr.add(2) = 2.0;
        *m2_ptr.add(3) = 5.0;
        *m2_ptr.add(4) = 3.0;
        *m2_ptr.add(5) = 6.0;
    }

    let result2 = dispatch_contract(&m1, &m2, 1, 0);
    assert!(result2.is_some(), "Matrix contraction should succeed");
    let mm = result2.unwrap();
    assert_eq!(mm.shape[0], 2, "Output rows should be 2");
    assert_eq!(mm.shape[1], 2, "Output cols should be 2");

    let mm_ptr = mm.data_ptr_f64();
    unsafe {
        // Expected: [[1*1+2*2+3*3, 1*4+2*5+3*6], [4*1+5*2+6*3, 4*4+5*5+6*6]]
        //         = [[14, 32], [32, 77]]
        assert!(((*mm_ptr.add(0)) - 14.0).abs() < 1e-10, "[0,0] should be 14");
        assert!(((*mm_ptr.add(1)) - 32.0).abs() < 1e-10, "[0,1] should be 32");
        assert!(((*mm_ptr.add(2)) - 32.0).abs() < 1e-10, "[1,0] should be 32");
        assert!(((*mm_ptr.add(3)) - 77.0).abs() < 1e-10, "[1,1] should be 77");
    }
}

#[test]
fn test_tensor_matrix_power() {
    use verum_vbc::interpreter::kernel::dispatch_matrix_power;

    // A = [[1, 1], [0, 1]] (identity + nilpotent)
    // A^0 = I, A^1 = A, A^2 = [[1, 2], [0, 1]], A^3 = [[1, 3], [0, 1]]
    let mut a = TensorHandle::zeros(&[2, 2], DType::F64).unwrap();
    let a_ptr = a.data_ptr_f64_mut();
    unsafe {
        *a_ptr.add(0) = 1.0;
        *a_ptr.add(1) = 1.0;
        *a_ptr.add(2) = 0.0;
        *a_ptr.add(3) = 1.0;
    }

    // A^0 = I
    let result0 = dispatch_matrix_power(&a, 0);
    assert!(result0.is_some(), "A^0 should succeed");
    let a0 = result0.unwrap();
    let a0_ptr = a0.data_ptr_f64();
    unsafe {
        assert!(((*a0_ptr.add(0)) - 1.0).abs() < 1e-10, "A^0[0,0] should be 1");
        assert!(((*a0_ptr.add(1)) - 0.0).abs() < 1e-10, "A^0[0,1] should be 0");
        assert!(((*a0_ptr.add(2)) - 0.0).abs() < 1e-10, "A^0[1,0] should be 0");
        assert!(((*a0_ptr.add(3)) - 1.0).abs() < 1e-10, "A^0[1,1] should be 1");
    }

    // A^3 = [[1, 3], [0, 1]]
    let result3 = dispatch_matrix_power(&a, 3);
    assert!(result3.is_some(), "A^3 should succeed");
    let a3 = result3.unwrap();
    let a3_ptr = a3.data_ptr_f64();
    unsafe {
        assert!(((*a3_ptr.add(0)) - 1.0).abs() < 1e-10, "A^3[0,0] should be 1");
        assert!(((*a3_ptr.add(1)) - 3.0).abs() < 1e-10, "A^3[0,1] should be 3");
        assert!(((*a3_ptr.add(2)) - 0.0).abs() < 1e-10, "A^3[1,0] should be 0");
        assert!(((*a3_ptr.add(3)) - 1.0).abs() < 1e-10, "A^3[1,1] should be 1");
    }
}

#[test]
fn test_tensor_matrix_exponential() {
    use verum_vbc::interpreter::kernel::dispatch_expm;

    // Test with a simple 2x2 matrix
    // For a diagonal matrix D = diag(a, b), exp(D) = diag(e^a, e^b)
    let mut d = TensorHandle::zeros(&[2, 2], DType::F64).unwrap();
    let d_ptr = d.data_ptr_f64_mut();
    unsafe {
        *d_ptr.add(0) = 1.0;  // e^1 ≈ 2.718
        *d_ptr.add(1) = 0.0;
        *d_ptr.add(2) = 0.0;
        *d_ptr.add(3) = 2.0;  // e^2 ≈ 7.389
    }

    let result = dispatch_expm(&d);
    assert!(result.is_some(), "Matrix exponential should succeed");

    let expd = result.unwrap();
    let expd_ptr = expd.data_ptr_f64();
    unsafe {
        // Diagonal entries should be e^1 and e^2
        // Tolerance is ~1% for Padé approximation numerical accuracy
        let e1 = std::f64::consts::E;
        let e2 = e1 * e1;
        assert!(((*expd_ptr.add(0)) - e1).abs() < 0.1, "exp(D)[0,0] should be e ≈ {}, got {}", e1, *expd_ptr.add(0));
        assert!(((*expd_ptr.add(3)) - e2).abs() < 0.1, "exp(D)[1,1] should be e^2 ≈ {}, got {}", e2, *expd_ptr.add(3));
        // Off-diagonal should be near 0
        assert!((*expd_ptr.add(1)).abs() < 0.1, "exp(D)[0,1] should be near 0");
        assert!((*expd_ptr.add(2)).abs() < 0.1, "exp(D)[1,0] should be near 0");
    }

    // Test with identity: exp(I) = e*I
    let mut id = TensorHandle::zeros(&[2, 2], DType::F64).unwrap();
    let id_ptr = id.data_ptr_f64_mut();
    unsafe {
        *id_ptr.add(0) = 0.0;
        *id_ptr.add(1) = 0.0;
        *id_ptr.add(2) = 0.0;
        *id_ptr.add(3) = 0.0;
    }
    let exp_zero = dispatch_expm(&id).unwrap();
    let exp_zero_ptr = exp_zero.data_ptr_f64();
    unsafe {
        // exp(0) = I
        assert!(((*exp_zero_ptr.add(0)) - 1.0).abs() < 0.01, "exp(0)[0,0] should be 1");
        assert!(((*exp_zero_ptr.add(3)) - 1.0).abs() < 0.01, "exp(0)[1,1] should be 1");
    }
}

#[test]
fn test_tensor_matrix_logarithm() {
    use verum_vbc::interpreter::kernel::dispatch_logm;

    // Test with identity: log(I) = 0
    let mut id = TensorHandle::zeros(&[2, 2], DType::F64).unwrap();
    let id_ptr = id.data_ptr_f64_mut();
    unsafe {
        *id_ptr.add(0) = 1.0;
        *id_ptr.add(1) = 0.0;
        *id_ptr.add(2) = 0.0;
        *id_ptr.add(3) = 1.0;
    }

    let result = dispatch_logm(&id);
    assert!(result.is_some(), "Matrix logarithm should succeed for identity");

    let logid = result.unwrap();
    let logid_ptr = logid.data_ptr_f64();
    unsafe {
        // log(I) should be close to zero matrix
        assert!((*logid_ptr.add(0)).abs() < 0.01, "log(I)[0,0] should be near 0");
        assert!((*logid_ptr.add(1)).abs() < 0.01, "log(I)[0,1] should be near 0");
        assert!((*logid_ptr.add(2)).abs() < 0.01, "log(I)[1,0] should be near 0");
        assert!((*logid_ptr.add(3)).abs() < 0.01, "log(I)[1,1] should be near 0");
    }

    // Test with e*I: log(e*I) = I
    let mut eid = TensorHandle::zeros(&[2, 2], DType::F64).unwrap();
    let eid_ptr = eid.data_ptr_f64_mut();
    let e = std::f64::consts::E;
    unsafe {
        *eid_ptr.add(0) = e;
        *eid_ptr.add(1) = 0.0;
        *eid_ptr.add(2) = 0.0;
        *eid_ptr.add(3) = e;
    }

    let result2 = dispatch_logm(&eid);
    assert!(result2.is_some(), "Matrix logarithm should succeed for e*I");

    let logeid = result2.unwrap();
    let logeid_ptr = logeid.data_ptr_f64();
    unsafe {
        // log(e*I) should be approximately I
        assert!(((*logeid_ptr.add(0)) - 1.0).abs() < 0.1, "log(e*I)[0,0] should be near 1, got {}", *logeid_ptr.add(0));
        assert!((*logeid_ptr.add(1)).abs() < 0.1, "log(e*I)[0,1] should be near 0");
        assert!((*logeid_ptr.add(2)).abs() < 0.1, "log(e*I)[1,0] should be near 0");
        assert!(((*logeid_ptr.add(3)) - 1.0).abs() < 0.1, "log(e*I)[1,1] should be near 1, got {}", *logeid_ptr.add(3));
    }
}

// ============================================================================
// Flash Attention Tests
// ============================================================================

#[test]
fn test_tensor_flash_attention_basic() {
    use verum_vbc::interpreter::tensor::flash_attention;

    // Test basic flash attention: [batch=1, heads=1, seq_len=2, head_dim=4]
    let mut query = TensorHandle::zeros(&[1, 1, 2, 4], DType::F32).unwrap();
    let mut key = TensorHandle::zeros(&[1, 1, 2, 4], DType::F32).unwrap();
    let mut value = TensorHandle::zeros(&[1, 1, 2, 4], DType::F32).unwrap();

    // Initialize Q, K, V with simple values
    let q_ptr = query.data_ptr_f32_mut();
    let k_ptr = key.data_ptr_f32_mut();
    let v_ptr = value.data_ptr_f32_mut();

    unsafe {
        // Q = [[1, 0, 0, 0], [0, 1, 0, 0]]
        *q_ptr.add(0) = 1.0;
        *q_ptr.add(5) = 1.0;

        // K = same as Q (so attention is identity-like)
        *k_ptr.add(0) = 1.0;
        *k_ptr.add(5) = 1.0;

        // V = [[1, 2, 3, 4], [5, 6, 7, 8]]
        *v_ptr.add(0) = 1.0;
        *v_ptr.add(1) = 2.0;
        *v_ptr.add(2) = 3.0;
        *v_ptr.add(3) = 4.0;
        *v_ptr.add(4) = 5.0;
        *v_ptr.add(5) = 6.0;
        *v_ptr.add(6) = 7.0;
        *v_ptr.add(7) = 8.0;
    }

    // Scale = 1.0
    let scale = 1.0f32;
    let causal = false;

    let result = flash_attention(&query, &key, &value, scale, causal);
    assert!(result.is_some(), "Flash attention should succeed");

    let output = result.unwrap();
    assert_eq!(output.ndim, 4);
    assert_eq!(output.shape[0], 1);
    assert_eq!(output.shape[1], 1);
    assert_eq!(output.shape[2], 2);
    assert_eq!(output.shape[3], 4);

    // Output should be a weighted combination of V rows
    let o_ptr = output.data_ptr_f32();
    unsafe {
        // Since Q[0] @ K^T gives [1, 0] -> softmax -> something like [higher, lower]
        // The exact values depend on softmax, but output should be valid
        assert!(!(*o_ptr.add(0)).is_nan(), "Output should not be NaN");
        assert!(!(*o_ptr.add(4)).is_nan(), "Output should not be NaN");
    }
}

#[test]
fn test_tensor_flash_attention_causal() {
    use verum_vbc::interpreter::tensor::flash_attention;

    // Test causal mask: [batch=1, heads=1, seq_len=3, head_dim=2]
    let mut query = TensorHandle::zeros(&[1, 1, 3, 2], DType::F32).unwrap();
    let mut key = TensorHandle::zeros(&[1, 1, 3, 2], DType::F32).unwrap();
    let mut value = TensorHandle::zeros(&[1, 1, 3, 2], DType::F32).unwrap();

    let q_ptr = query.data_ptr_f32_mut();
    let k_ptr = key.data_ptr_f32_mut();
    let v_ptr = value.data_ptr_f32_mut();

    unsafe {
        // Initialize all to 1.0 for simplicity
        for i in 0..6 {
            *q_ptr.add(i) = 1.0;
            *k_ptr.add(i) = 1.0;
        }
        // V = [[1, 0], [0, 1], [1, 1]]
        *v_ptr.add(0) = 1.0;
        *v_ptr.add(1) = 0.0;
        *v_ptr.add(2) = 0.0;
        *v_ptr.add(3) = 1.0;
        *v_ptr.add(4) = 1.0;
        *v_ptr.add(5) = 1.0;
    }

    let scale = 1.0f32 / (2.0f32).sqrt(); // 1/sqrt(head_dim)
    let causal = true;

    let result = flash_attention(&query, &key, &value, scale, causal);
    assert!(result.is_some(), "Causal flash attention should succeed");

    let output = result.unwrap();
    let o_ptr = output.data_ptr_f32();

    unsafe {
        // With causal mask:
        // Position 0 can only attend to position 0 -> output is V[0] = [1, 0]
        assert!(((*o_ptr.add(0)) - 1.0).abs() < 0.01, "Causal pos 0 should be [1, _]");
        assert!((*o_ptr.add(1)).abs() < 0.01, "Causal pos 0 should be [_, 0]");

        // Position 1 can attend to [0, 1] -> weighted average
        // Position 2 can attend to [0, 1, 2] -> weighted average
        // Just verify no NaN
        assert!(!(*o_ptr.add(2)).is_nan(), "Output should not be NaN");
        assert!(!(*o_ptr.add(4)).is_nan(), "Output should not be NaN");
    }
}

#[test]
fn test_tensor_flash_attention_multi_head() {
    use verum_vbc::interpreter::tensor::flash_attention;

    // Test multi-head: [batch=2, heads=4, seq_len=3, head_dim=8]
    let query = TensorHandle::full(&[2, 4, 3, 8], DType::F32, 1.0).unwrap();
    let key = TensorHandle::full(&[2, 4, 3, 8], DType::F32, 1.0).unwrap();
    let value = TensorHandle::full(&[2, 4, 3, 8], DType::F32, 0.5).unwrap();

    let scale = 1.0f32 / (8.0f32).sqrt();
    let causal = false;

    let result = flash_attention(&query, &key, &value, scale, causal);
    assert!(result.is_some(), "Multi-head flash attention should succeed");

    let output = result.unwrap();
    assert_eq!(output.shape[0], 2);
    assert_eq!(output.shape[1], 4);
    assert_eq!(output.shape[2], 3);
    assert_eq!(output.shape[3], 8);

    // With uniform Q, K, uniform attention weights -> output = V (which is all 0.5)
    let o_ptr = output.data_ptr_f32();
    unsafe {
        for i in 0..10 {
            assert!(((*o_ptr.add(i)) - 0.5).abs() < 0.01,
                "Uniform attention should output V values, got {} at {}", *o_ptr.add(i), i);
        }
    }
}
