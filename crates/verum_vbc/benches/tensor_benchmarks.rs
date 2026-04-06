//! Comprehensive tensor operation benchmarks for VBC interpreter.
//!
//! Benchmarks CPU (scalar, NEON/AVX2) and Metal GPU backends.
//! Covers all tensor operations: creation, binop, unop, reduce, matmul,
//! broadcasting, shape operations, neural network ops, and more.
//!
//! Run with: cargo bench -p verum_vbc --features metal

use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};
use std::hint::black_box;
use verum_vbc::interpreter::tensor::{
    DType, TensorHandle, tensor_binop, tensor_unop, tensor_matmul, tensor_reduce,
    tensor_transpose, tensor_reshape, tensor_softmax, tensor_concat, tensor_slice,
    tensor_stack, tensor_arange, tensor_linspace, tensor_rand, tensor_clone,
    tensor_identity, tensor_squeeze, tensor_argmax, tensor_layer_norm,
    tensor_conv2d, tensor_pool2d, PoolOp,
};
use verum_vbc::interpreter::kernel::{
    dispatch_binop, dispatch_unop, dispatch_reduce, dispatch_matmul,
    broadcast_shapes, broadcast_to, get_capabilities,
};
use verum_vbc::interpreter::kernel::cpu::{
    binop_f32_scalar, unop_f32_scalar,
    reduce_f32_scalar, matmul_f32_scalar, matmul_f32_tiled,
};
#[cfg(target_arch = "aarch64")]
use verum_vbc::interpreter::kernel::cpu::{
    binop_f32_neon, unop_f32_neon, reduce_f32_neon,
};
use verum_vbc::instruction::{TensorBinaryOp, TensorUnaryOp, TensorReduceOp};

// ============================================================================
// Test Sizes Configuration
// ============================================================================

const TINY_SIZES: &[usize] = &[4, 8, 16];
const SMALL_SIZES: &[usize] = &[32, 64, 128, 256];
const MEDIUM_SIZES: &[usize] = &[512, 1024, 2048, 4096, 8192];
const LARGE_SIZES: &[usize] = &[16384, 32768, 65536, 131072, 262144];
const HUGE_SIZES: &[usize] = &[524288, 1048576, 2097152, 4194304];

const MATMUL_SIZES: &[usize] = &[16, 32, 64, 128, 256, 512];
const LARGE_MATMUL_SIZES: &[usize] = &[1024, 2048];

// ============================================================================
// Tensor Creation Benchmarks
// ============================================================================

fn bench_tensor_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("tensor_creation");

    for size in MEDIUM_SIZES.iter().chain(LARGE_SIZES.iter()) {
        group.throughput(Throughput::Elements(*size as u64));

        // zeros
        group.bench_with_input(BenchmarkId::new("zeros_f32", size), size, |b, &size| {
            b.iter(|| black_box(TensorHandle::zeros(&[size], DType::F32)));
        });

        // full
        group.bench_with_input(BenchmarkId::new("full_f32", size), size, |b, &size| {
            b.iter(|| black_box(TensorHandle::full(&[size], DType::F32, 3.14)));
        });

        // rand
        group.bench_with_input(BenchmarkId::new("rand_f32", size), size, |b, &size| {
            b.iter(|| black_box(tensor_rand(&[size], DType::F32)));
        });
    }

    // arange
    group.bench_function("arange_1000", |b| {
        b.iter(|| black_box(tensor_arange(0.0, 1000.0, 1.0, DType::F32)));
    });

    // linspace
    group.bench_function("linspace_1000", |b| {
        b.iter(|| black_box(tensor_linspace(0.0, 100.0, 1000, DType::F32)));
    });

    // identity matrix
    for size in [64, 128, 256, 512, 1024].iter() {
        group.bench_with_input(BenchmarkId::new("identity", size), size, |b, &size| {
            b.iter(|| black_box(tensor_identity(size, DType::F32)));
        });
    }

    group.finish();
}

fn bench_tensor_2d_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("tensor_2d_creation");

    let shapes = [
        (64, 64),
        (128, 128),
        (256, 256),
        (512, 512),
        (1024, 1024),
    ];

    for (rows, cols) in shapes.iter() {
        let numel = rows * cols;
        group.throughput(Throughput::Elements(numel as u64));

        let label = format!("{}x{}", rows, cols);

        group.bench_function(BenchmarkId::new("zeros_2d", &label), |b| {
            b.iter(|| black_box(TensorHandle::zeros(&[*rows, *cols], DType::F32)));
        });

        group.bench_function(BenchmarkId::new("full_2d", &label), |b| {
            b.iter(|| black_box(TensorHandle::full(&[*rows, *cols], DType::F32, 1.0)));
        });
    }

    group.finish();
}

// ============================================================================
// Binary Operation Benchmarks - All Operations
// ============================================================================

fn bench_binop_all_ops(c: &mut Criterion) {
    let ops = [
        ("add", TensorBinaryOp::Add),
        ("sub", TensorBinaryOp::Sub),
        ("mul", TensorBinaryOp::Mul),
        ("div", TensorBinaryOp::Div),
        ("max", TensorBinaryOp::Max),
        ("min", TensorBinaryOp::Min),
        ("pow", TensorBinaryOp::Pow),
        ("mod", TensorBinaryOp::Mod),
    ];

    for (name, op) in ops.iter() {
        let mut group = c.benchmark_group(format!("binop_{}", name));

        for size in MEDIUM_SIZES.iter().chain(LARGE_SIZES.iter()) {
            group.throughput(Throughput::Elements(*size as u64));
            group.bench_with_input(BenchmarkId::new("dispatch", size), size, |b, &size| {
                let a = TensorHandle::full(&[size], DType::F32, 2.0).unwrap();
                let b_tensor = TensorHandle::full(&[size], DType::F32, 3.0).unwrap();
                b.iter(|| {
                    black_box(dispatch_binop(&a, &b_tensor, *op))
                });
            });
        }
        group.finish();
    }
}

#[cfg(target_arch = "aarch64")]
fn bench_binop_scalar_vs_simd(c: &mut Criterion) {
    let mut group = c.benchmark_group("binop_scalar_vs_neon");

    for size in MEDIUM_SIZES.iter().chain(LARGE_SIZES.iter()) {
        group.throughput(Throughput::Elements(*size as u64));

        // Scalar version
        group.bench_with_input(BenchmarkId::new("scalar", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 2.0).unwrap();
            let b_tensor = TensorHandle::full(&[size], DType::F32, 3.0).unwrap();
            b.iter(|| {
                black_box(binop_f32_scalar(&a, &b_tensor, TensorBinaryOp::Add))
            });
        });

        // NEON version
        group.bench_with_input(BenchmarkId::new("neon", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 2.0).unwrap();
            let b_tensor = TensorHandle::full(&[size], DType::F32, 3.0).unwrap();
            b.iter(|| {
                black_box(binop_f32_neon(&a, &b_tensor, TensorBinaryOp::Add))
            });
        });
    }
    group.finish();
}

#[cfg(not(target_arch = "aarch64"))]
fn bench_binop_scalar_vs_simd(c: &mut Criterion) {
    let mut group = c.benchmark_group("binop_scalar");

    for size in MEDIUM_SIZES.iter().chain(LARGE_SIZES.iter()) {
        group.throughput(Throughput::Elements(*size as u64));

        group.bench_with_input(BenchmarkId::new("scalar", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 2.0).unwrap();
            let b_tensor = TensorHandle::full(&[size], DType::F32, 3.0).unwrap();
            b.iter(|| {
                black_box(binop_f32_scalar(&a, &b_tensor, TensorBinaryOp::Add))
            });
        });
    }
    group.finish();
}

fn bench_binop_datatypes(c: &mut Criterion) {
    let mut group = c.benchmark_group("binop_datatypes");
    let size = 65536;

    group.throughput(Throughput::Elements(size as u64));

    // F32
    group.bench_function("f32", |b| {
        let a = TensorHandle::full(&[size], DType::F32, 2.0).unwrap();
        let b_tensor = TensorHandle::full(&[size], DType::F32, 3.0).unwrap();
        b.iter(|| black_box(dispatch_binop(&a, &b_tensor, TensorBinaryOp::Add)));
    });

    // F64
    group.bench_function("f64", |b| {
        let a = TensorHandle::full(&[size], DType::F64, 2.0).unwrap();
        let b_tensor = TensorHandle::full(&[size], DType::F64, 3.0).unwrap();
        b.iter(|| black_box(dispatch_binop(&a, &b_tensor, TensorBinaryOp::Add)));
    });

    // I32
    group.bench_function("i32", |b| {
        let a = TensorHandle::full(&[size], DType::I32, 2.0).unwrap();
        let b_tensor = TensorHandle::full(&[size], DType::I32, 3.0).unwrap();
        b.iter(|| black_box(dispatch_binop(&a, &b_tensor, TensorBinaryOp::Add)));
    });

    group.finish();
}

fn bench_binop_edge_cases(c: &mut Criterion) {
    let mut group = c.benchmark_group("binop_edge_cases");

    // Very small tensors (below SIMD threshold)
    for size in TINY_SIZES.iter() {
        group.bench_with_input(BenchmarkId::new("tiny", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 2.0).unwrap();
            let b_tensor = TensorHandle::full(&[size], DType::F32, 3.0).unwrap();
            b.iter(|| black_box(dispatch_binop(&a, &b_tensor, TensorBinaryOp::Add)));
        });
    }

    // Non power-of-2 sizes
    let non_pow2_sizes = [63, 127, 255, 1023, 4095, 65537];
    for &size in non_pow2_sizes.iter() {
        group.bench_with_input(BenchmarkId::new("non_pow2", size), &size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 2.0).unwrap();
            let b_tensor = TensorHandle::full(&[size], DType::F32, 3.0).unwrap();
            b.iter(|| black_box(dispatch_binop(&a, &b_tensor, TensorBinaryOp::Add)));
        });
    }

    group.finish();
}

// ============================================================================
// Unary Operation Benchmarks - All Operations
// ============================================================================

fn bench_unop_all_ops(c: &mut Criterion) {
    let ops = [
        ("neg", TensorUnaryOp::Neg),
        ("abs", TensorUnaryOp::Abs),
        ("sqrt", TensorUnaryOp::Sqrt),
        ("rsqrt", TensorUnaryOp::Rsqrt),
        ("exp", TensorUnaryOp::Exp),
        ("log", TensorUnaryOp::Log),
        ("log2", TensorUnaryOp::Log2),
        ("sin", TensorUnaryOp::Sin),
        ("cos", TensorUnaryOp::Cos),
        ("tan", TensorUnaryOp::Tan),
        ("tanh", TensorUnaryOp::Tanh),
        ("sigmoid", TensorUnaryOp::Sigmoid),
        ("relu", TensorUnaryOp::Relu),
        ("gelu", TensorUnaryOp::Gelu),
        ("silu", TensorUnaryOp::Silu),
        ("softplus", TensorUnaryOp::Softplus),
        ("mish", TensorUnaryOp::Mish),
        ("floor", TensorUnaryOp::Floor),
        ("ceil", TensorUnaryOp::Ceil),
        ("round", TensorUnaryOp::Round),
        ("sign", TensorUnaryOp::Sign),
        ("erf", TensorUnaryOp::Erf),
    ];

    for (name, op) in ops.iter() {
        let mut group = c.benchmark_group(format!("unop_{}", name));

        for size in MEDIUM_SIZES.iter() {
            group.throughput(Throughput::Elements(*size as u64));
            group.bench_with_input(BenchmarkId::new("f32", size), size, |b, &size| {
                let a = TensorHandle::full(&[size], DType::F32, 1.5).unwrap();
                b.iter(|| {
                    black_box(dispatch_unop(&a, *op))
                });
            });
        }
        group.finish();
    }
}

#[cfg(target_arch = "aarch64")]
fn bench_unop_scalar_vs_neon(c: &mut Criterion) {
    let ops = [
        ("neg", TensorUnaryOp::Neg),
        ("abs", TensorUnaryOp::Abs),
        ("sqrt", TensorUnaryOp::Sqrt),
        ("relu", TensorUnaryOp::Relu),
        ("floor", TensorUnaryOp::Floor),
    ];

    for (name, op) in ops.iter() {
        let mut group = c.benchmark_group(format!("unop_compare_{}", name));

        for size in LARGE_SIZES.iter() {
            group.throughput(Throughput::Elements(*size as u64));

            // Scalar
            group.bench_with_input(BenchmarkId::new("scalar", size), size, |b, &size| {
                let a = TensorHandle::full(&[size], DType::F32, 4.0).unwrap();
                b.iter(|| black_box(unop_f32_scalar(&a, *op)));
            });

            // NEON
            group.bench_with_input(BenchmarkId::new("neon", size), size, |b, &size| {
                let a = TensorHandle::full(&[size], DType::F32, 4.0).unwrap();
                b.iter(|| black_box(unop_f32_neon(&a, *op)));
            });
        }
        group.finish();
    }
}

#[cfg(not(target_arch = "aarch64"))]
fn bench_unop_scalar_vs_neon(c: &mut Criterion) {
    // No-op on non-ARM
    let _ = c;
}

fn bench_unop_activation_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("activation_functions");
    let size = 65536;
    group.throughput(Throughput::Elements(size as u64));

    let activations = [
        ("relu", TensorUnaryOp::Relu),
        ("sigmoid", TensorUnaryOp::Sigmoid),
        ("tanh", TensorUnaryOp::Tanh),
        ("gelu", TensorUnaryOp::Gelu),
        ("silu", TensorUnaryOp::Silu),
        ("softplus", TensorUnaryOp::Softplus),
        ("mish", TensorUnaryOp::Mish),
    ];

    for (name, op) in activations.iter() {
        group.bench_function(*name, |b| {
            let a = TensorHandle::full(&[size], DType::F32, 0.5).unwrap();
            b.iter(|| black_box(dispatch_unop(&a, *op)));
        });
    }
    group.finish();
}

fn bench_unop_math_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("math_functions");
    let size = 65536;
    group.throughput(Throughput::Elements(size as u64));

    let funcs = [
        ("sqrt", TensorUnaryOp::Sqrt),
        ("rsqrt", TensorUnaryOp::Rsqrt),
        ("exp", TensorUnaryOp::Exp),
        ("log", TensorUnaryOp::Log),
        ("log2", TensorUnaryOp::Log2),
        ("sin", TensorUnaryOp::Sin),
        ("cos", TensorUnaryOp::Cos),
        ("tan", TensorUnaryOp::Tan),
        ("erf", TensorUnaryOp::Erf),
    ];

    for (name, op) in funcs.iter() {
        group.bench_function(*name, |b| {
            let a = TensorHandle::full(&[size], DType::F32, 2.0).unwrap();
            b.iter(|| black_box(dispatch_unop(&a, *op)));
        });
    }
    group.finish();
}

// ============================================================================
// Reduction Benchmarks - All Operations
// ============================================================================

fn bench_reduce_all_ops(c: &mut Criterion) {
    let ops = [
        ("sum", TensorReduceOp::Sum),
        ("prod", TensorReduceOp::Prod),
        ("max", TensorReduceOp::Max),
        ("min", TensorReduceOp::Min),
        ("mean", TensorReduceOp::Mean),
        ("var", TensorReduceOp::Var),
        ("std", TensorReduceOp::Std),
        ("norm", TensorReduceOp::Norm),
        ("logsumexp", TensorReduceOp::LogSumExp),
        ("all", TensorReduceOp::All),
        ("any", TensorReduceOp::Any),
    ];

    for (name, op) in ops.iter() {
        let mut group = c.benchmark_group(format!("reduce_{}", name));

        for size in MEDIUM_SIZES.iter().chain(LARGE_SIZES.iter()) {
            group.throughput(Throughput::Elements(*size as u64));
            group.bench_with_input(BenchmarkId::new("f32", size), size, |b, &size| {
                let a = TensorHandle::full(&[size], DType::F32, 1.0).unwrap();
                b.iter(|| {
                    black_box(dispatch_reduce(&a, *op, None))
                });
            });
        }
        group.finish();
    }
}

#[cfg(target_arch = "aarch64")]
fn bench_reduce_scalar_vs_neon(c: &mut Criterion) {
    let mut group = c.benchmark_group("reduce_scalar_vs_neon");

    for size in LARGE_SIZES.iter().chain(HUGE_SIZES.iter().take(2)) {
        group.throughput(Throughput::Elements(*size as u64));

        // Scalar sum
        group.bench_with_input(BenchmarkId::new("sum_scalar", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 1.0).unwrap();
            b.iter(|| black_box(reduce_f32_scalar(&a, TensorReduceOp::Sum, None)));
        });

        // NEON sum
        group.bench_with_input(BenchmarkId::new("sum_neon", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 1.0).unwrap();
            b.iter(|| black_box(reduce_f32_neon(&a, TensorReduceOp::Sum, None)));
        });
    }
    group.finish();
}

#[cfg(not(target_arch = "aarch64"))]
fn bench_reduce_scalar_vs_neon(c: &mut Criterion) {
    let mut group = c.benchmark_group("reduce_scalar");

    for size in LARGE_SIZES.iter() {
        group.throughput(Throughput::Elements(*size as u64));

        group.bench_with_input(BenchmarkId::new("sum_scalar", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 1.0).unwrap();
            b.iter(|| black_box(reduce_f32_scalar(&a, TensorReduceOp::Sum, None)));
        });
    }
    group.finish();
}

fn bench_reduce_axis(c: &mut Criterion) {
    let mut group = c.benchmark_group("reduce_axis");

    // 2D tensor reduction along different axes
    let shapes = [
        (128, 128),
        (256, 256),
        (512, 512),
        (1024, 1024),
    ];

    for (rows, cols) in shapes.iter() {
        let label = format!("{}x{}", rows, cols);

        // Reduce along axis 0 (rows)
        group.bench_function(BenchmarkId::new("axis0", &label), |b| {
            let a = TensorHandle::full(&[*rows, *cols], DType::F32, 1.0).unwrap();
            b.iter(|| black_box(tensor_reduce(&a, Some(0), TensorReduceOp::Sum)));
        });

        // Reduce along axis 1 (cols)
        group.bench_function(BenchmarkId::new("axis1", &label), |b| {
            let a = TensorHandle::full(&[*rows, *cols], DType::F32, 1.0).unwrap();
            b.iter(|| black_box(tensor_reduce(&a, Some(1), TensorReduceOp::Sum)));
        });
    }

    // 3D tensor reduction
    let shape3d = [64, 128, 256];
    let numel3d: usize = shape3d.iter().product();
    group.throughput(Throughput::Elements(numel3d as u64));

    group.bench_function("3d_axis0", |b| {
        let a = TensorHandle::full(&shape3d, DType::F32, 1.0).unwrap();
        b.iter(|| black_box(tensor_reduce(&a, Some(0), TensorReduceOp::Sum)));
    });

    group.bench_function("3d_axis1", |b| {
        let a = TensorHandle::full(&shape3d, DType::F32, 1.0).unwrap();
        b.iter(|| black_box(tensor_reduce(&a, Some(1), TensorReduceOp::Sum)));
    });

    group.bench_function("3d_axis2", |b| {
        let a = TensorHandle::full(&shape3d, DType::F32, 1.0).unwrap();
        b.iter(|| black_box(tensor_reduce(&a, Some(2), TensorReduceOp::Sum)));
    });

    group.finish();
}

// ============================================================================
// Matrix Multiplication Benchmarks
// ============================================================================

fn bench_matmul_square(c: &mut Criterion) {
    let mut group = c.benchmark_group("matmul_square");

    for size in MATMUL_SIZES.iter() {
        let flops = 2 * (*size as u64).pow(3);
        group.throughput(Throughput::Elements(flops));
        group.bench_with_input(BenchmarkId::new("f32", size), size, |b, &size| {
            let a = TensorHandle::full(&[size, size], DType::F32, 0.1).unwrap();
            let b_tensor = TensorHandle::full(&[size, size], DType::F32, 0.1).unwrap();
            b.iter(|| {
                black_box(dispatch_matmul(&a, &b_tensor))
            });
        });
    }
    group.finish();
}

fn bench_matmul_scalar_vs_tiled(c: &mut Criterion) {
    let mut group = c.benchmark_group("matmul_scalar_vs_tiled");

    for size in MATMUL_SIZES.iter() {
        let flops = 2 * (*size as u64).pow(3);
        group.throughput(Throughput::Elements(flops));

        // Scalar
        group.bench_with_input(BenchmarkId::new("scalar", size), size, |b, &size| {
            let a = TensorHandle::full(&[size, size], DType::F32, 0.1).unwrap();
            let b_tensor = TensorHandle::full(&[size, size], DType::F32, 0.1).unwrap();
            b.iter(|| black_box(matmul_f32_scalar(&a, &b_tensor)));
        });

        // Tiled
        group.bench_with_input(BenchmarkId::new("tiled", size), size, |b, &size| {
            let a = TensorHandle::full(&[size, size], DType::F32, 0.1).unwrap();
            let b_tensor = TensorHandle::full(&[size, size], DType::F32, 0.1).unwrap();
            b.iter(|| black_box(matmul_f32_tiled(&a, &b_tensor)));
        });
    }
    group.finish();
}

fn bench_matmul_rectangular(c: &mut Criterion) {
    let mut group = c.benchmark_group("matmul_rectangular");

    let shapes = [
        (64, 32, 128),
        (128, 64, 256),
        (256, 128, 512),
        (512, 256, 1024),
        (1024, 512, 256),
        (32, 1024, 32),   // Thin middle
        (1024, 32, 1024), // Wide output
    ];

    for (m, k, n) in shapes.iter() {
        let flops = 2 * (*m as u64) * (*k as u64) * (*n as u64);
        let label = format!("{}x{}x{}", m, k, n);
        group.throughput(Throughput::Elements(flops));
        group.bench_with_input(BenchmarkId::new("f32", &label), &(*m, *k, *n), |b, &(m, k, n)| {
            let a = TensorHandle::full(&[m, k], DType::F32, 0.1).unwrap();
            let b_tensor = TensorHandle::full(&[k, n], DType::F32, 0.1).unwrap();
            b.iter(|| black_box(dispatch_matmul(&a, &b_tensor)));
        });
    }
    group.finish();
}

fn bench_matmul_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("matmul_batch_simulation");

    // Simulate batch matmul by running multiple small matmuls
    let batch_sizes = [1, 4, 8, 16, 32];
    let matrix_size = 64;

    for batch in batch_sizes.iter() {
        let flops = (*batch as u64) * 2 * (matrix_size as u64).pow(3);
        group.throughput(Throughput::Elements(flops));
        group.bench_with_input(BenchmarkId::new("batch", batch), batch, |b, &batch| {
            let matrices: Vec<_> = (0..batch)
                .map(|_| {
                    let a = TensorHandle::full(&[matrix_size, matrix_size], DType::F32, 0.1).unwrap();
                    let b = TensorHandle::full(&[matrix_size, matrix_size], DType::F32, 0.1).unwrap();
                    (a, b)
                })
                .collect();
            b.iter(|| {
                for (a, b_tensor) in matrices.iter() {
                    black_box(dispatch_matmul(a, b_tensor));
                }
            });
        });
    }
    group.finish();
}

// ============================================================================
// Shape Operation Benchmarks
// ============================================================================

fn bench_transpose(c: &mut Criterion) {
    let mut group = c.benchmark_group("transpose");

    let shapes = [
        (64, 64),
        (128, 128),
        (256, 256),
        (512, 512),
        (1024, 1024),
        (128, 512),  // Non-square
        (512, 128),
    ];

    for (rows, cols) in shapes.iter() {
        let numel = rows * cols;
        let label = format!("{}x{}", rows, cols);
        group.throughput(Throughput::Elements(numel as u64));

        group.bench_function(&label, |b| {
            let a = TensorHandle::full(&[*rows, *cols], DType::F32, 1.0).unwrap();
            b.iter(|| black_box(tensor_transpose(&a)));
        });
    }

    group.finish();
}

fn bench_reshape(c: &mut Criterion) {
    let mut group = c.benchmark_group("reshape");

    // Reshape is mostly a metadata operation (view), should be very fast
    let size = 65536;
    group.throughput(Throughput::Elements(size as u64));

    // 1D -> 2D
    group.bench_function("1d_to_2d", |b| {
        let a = TensorHandle::full(&[size], DType::F32, 1.0).unwrap();
        b.iter(|| black_box(tensor_reshape(&a, &[256, 256])));
    });

    // 2D -> 1D
    group.bench_function("2d_to_1d", |b| {
        let a = TensorHandle::full(&[256, 256], DType::F32, 1.0).unwrap();
        b.iter(|| black_box(tensor_reshape(&a, &[size])));
    });

    // 2D -> 3D
    group.bench_function("2d_to_3d", |b| {
        let a = TensorHandle::full(&[256, 256], DType::F32, 1.0).unwrap();
        b.iter(|| black_box(tensor_reshape(&a, &[16, 16, 256])));
    });

    // 3D -> 4D
    group.bench_function("3d_to_4d", |b| {
        let a = TensorHandle::full(&[16, 16, 256], DType::F32, 1.0).unwrap();
        b.iter(|| black_box(tensor_reshape(&a, &[4, 4, 16, 256])));
    });

    group.finish();
}

fn bench_squeeze(c: &mut Criterion) {
    let mut group = c.benchmark_group("squeeze");

    group.bench_function("squeeze_1d", |b| {
        let a = TensorHandle::full(&[1, 256, 1], DType::F32, 1.0).unwrap();
        b.iter(|| black_box(tensor_squeeze(&a, None)));
    });

    group.bench_function("squeeze_specific", |b| {
        let a = TensorHandle::full(&[1, 256, 1], DType::F32, 1.0).unwrap();
        b.iter(|| black_box(tensor_squeeze(&a, Some(0))));
    });

    group.finish();
}

fn bench_slice(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice");

    // 1D slice
    group.bench_function("1d_slice", |b| {
        let a = TensorHandle::full(&[1024], DType::F32, 1.0).unwrap();
        b.iter(|| black_box(tensor_slice(&a, &[(256, 768)])));
    });

    // 2D slice
    let shapes = [
        (256, 256),
        (512, 512),
        (1024, 1024),
    ];

    for (rows, cols) in shapes.iter() {
        let label = format!("{}x{}", rows, cols);
        group.bench_function(BenchmarkId::new("2d", &label), |b| {
            let a = TensorHandle::full(&[*rows, *cols], DType::F32, 1.0).unwrap();
            let half_r = rows / 4;
            let half_c = cols / 4;
            b.iter(|| black_box(tensor_slice(&a, &[(half_r, rows - half_r), (half_c, cols - half_c)])));
        });
    }

    group.finish();
}

fn bench_concat(c: &mut Criterion) {
    let mut group = c.benchmark_group("concat");

    // Concatenate varying number of tensors
    let num_tensors = [2, 4, 8, 16];
    let tensor_size = 4096;

    for &n in num_tensors.iter() {
        let total_elements = n * tensor_size;
        group.throughput(Throughput::Elements(total_elements as u64));

        group.bench_with_input(BenchmarkId::new("1d", n), &n, |b, &n| {
            let tensors: Vec<_> = (0..n)
                .map(|_| TensorHandle::full(&[tensor_size], DType::F32, 1.0).unwrap())
                .collect();
            let refs: Vec<_> = tensors.iter().collect();
            b.iter(|| black_box(tensor_concat(&refs, 0)));
        });
    }

    // 2D concat along different axes
    group.bench_function("2d_axis0", |b| {
        let a = TensorHandle::full(&[128, 256], DType::F32, 1.0).unwrap();
        let bv = TensorHandle::full(&[128, 256], DType::F32, 2.0).unwrap();
        let refs = vec![&a, &bv];
        b.iter(|| black_box(tensor_concat(&refs, 0)));
    });

    group.bench_function("2d_axis1", |b| {
        let a = TensorHandle::full(&[256, 128], DType::F32, 1.0).unwrap();
        let bv = TensorHandle::full(&[256, 128], DType::F32, 2.0).unwrap();
        let refs = vec![&a, &bv];
        b.iter(|| black_box(tensor_concat(&refs, 1)));
    });

    group.finish();
}

fn bench_stack(c: &mut Criterion) {
    let mut group = c.benchmark_group("stack");

    let num_tensors = [2, 4, 8];
    let tensor_shape = [64, 64];

    for &n in num_tensors.iter() {
        let total_elements = n * tensor_shape[0] * tensor_shape[1];
        group.throughput(Throughput::Elements(total_elements as u64));

        group.bench_with_input(BenchmarkId::new("2d", n), &n, |b, &n| {
            let tensors: Vec<_> = (0..n)
                .map(|_| TensorHandle::full(&tensor_shape, DType::F32, 1.0).unwrap())
                .collect();
            let refs: Vec<_> = tensors.iter().collect();
            b.iter(|| black_box(tensor_stack(&refs, 0)));
        });
    }

    group.finish();
}

fn bench_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("clone");

    for size in MEDIUM_SIZES.iter().chain(LARGE_SIZES.iter()) {
        let bytes = (*size * 4) as u64;
        group.throughput(Throughput::Bytes(bytes));

        group.bench_with_input(BenchmarkId::new("f32", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 1.0).unwrap();
            b.iter(|| black_box(tensor_clone(&a)));
        });
    }

    group.finish();
}

// ============================================================================
// Neural Network Operation Benchmarks
// ============================================================================

fn bench_softmax(c: &mut Criterion) {
    let mut group = c.benchmark_group("softmax");

    for size in MEDIUM_SIZES.iter() {
        group.throughput(Throughput::Elements(*size as u64));

        group.bench_with_input(BenchmarkId::new("1d", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 0.5).unwrap();
            b.iter(|| black_box(tensor_softmax(&a, None)));
        });
    }

    // 2D softmax (common in attention)
    let shapes = [
        (32, 32),
        (64, 64),
        (128, 128),
        (256, 256),
    ];

    for (rows, cols) in shapes.iter() {
        let label = format!("{}x{}", rows, cols);
        let numel = rows * cols;
        group.throughput(Throughput::Elements(numel as u64));

        group.bench_function(BenchmarkId::new("2d", &label), |b| {
            let a = TensorHandle::full(&[*rows, *cols], DType::F32, 0.5).unwrap();
            b.iter(|| black_box(tensor_softmax(&a, Some(-1))));
        });
    }

    group.finish();
}

fn bench_argmax(c: &mut Criterion) {
    let mut group = c.benchmark_group("argmax");

    for size in MEDIUM_SIZES.iter().chain(LARGE_SIZES.iter()) {
        group.throughput(Throughput::Elements(*size as u64));

        group.bench_with_input(BenchmarkId::new("f32", size), size, |b, &size| {
            let a = tensor_rand(&[size], DType::F32).unwrap();
            b.iter(|| black_box(tensor_argmax(&a, None)));
        });
    }

    group.finish();
}

fn bench_layer_norm(c: &mut Criterion) {
    let mut group = c.benchmark_group("layer_norm");

    // Typical transformer dimensions
    let shapes = [
        (32, 512),    // batch=32, hidden=512
        (32, 768),    // batch=32, hidden=768 (BERT-base)
        (32, 1024),   // batch=32, hidden=1024 (BERT-large)
        (16, 2048),   // batch=16, hidden=2048
        (8, 4096),    // batch=8, hidden=4096
    ];

    for (batch, hidden) in shapes.iter() {
        let label = format!("{}x{}", batch, hidden);
        let numel = batch * hidden;
        group.throughput(Throughput::Elements(numel as u64));

        group.bench_function(&label, |b| {
            let input = TensorHandle::full(&[*batch, *hidden], DType::F32, 0.5).unwrap();
            let gamma = TensorHandle::full(&[*hidden], DType::F32, 1.0).unwrap();
            let beta = TensorHandle::full(&[*hidden], DType::F32, 0.0).unwrap();
            b.iter(|| black_box(tensor_layer_norm(&input, Some(&gamma), Some(&beta), 1e-5)));
        });
    }

    group.finish();
}

fn bench_conv2d(c: &mut Criterion) {
    let mut group = c.benchmark_group("conv2d");

    // Input: (batch, in_channels, height, width)
    // Kernel: (out_channels, in_channels, kH, kW)
    let configs = [
        // (batch, in_c, h, w, out_c, kh, kw)
        (1, 3, 32, 32, 16, 3, 3),      // Small image
        (1, 3, 64, 64, 32, 3, 3),      // Medium image
        (1, 32, 32, 32, 64, 3, 3),     // Deeper network
        (1, 64, 16, 16, 128, 3, 3),    // Even deeper
        (8, 3, 32, 32, 16, 3, 3),      // Batched
    ];

    for (batch, in_c, h, w, out_c, kh, kw) in configs.iter() {
        let label = format!("{}x{}x{}x{}_k{}x{}", batch, in_c, h, w, kh, kw);

        group.bench_function(&label, |b| {
            let input = TensorHandle::full(&[*batch, *in_c, *h, *w], DType::F32, 0.5).unwrap();
            let kernel = TensorHandle::full(&[*out_c, *in_c, *kh, *kw], DType::F32, 0.1).unwrap();
            let bias = TensorHandle::full(&[*out_c], DType::F32, 0.0).unwrap();
            b.iter(|| black_box(tensor_conv2d(&input, &kernel, Some(&bias), (1, 1), (1, 1), (1, 1), 1)));
        });
    }

    group.finish();
}

fn bench_pool2d(c: &mut Criterion) {
    let mut group = c.benchmark_group("pool2d");

    let configs = [
        // (batch, channels, height, width, pool_size)
        (1, 32, 32, 32, 2),
        (1, 64, 16, 16, 2),
        (1, 128, 8, 8, 2),
        (8, 32, 32, 32, 2),
    ];

    for (batch, ch, h, w, pool) in configs.iter() {
        let label = format!("{}x{}x{}x{}_p{}", batch, ch, h, w, pool);

        // Max pooling
        group.bench_function(BenchmarkId::new("max", &label), |b| {
            let input = TensorHandle::full(&[*batch, *ch, *h, *w], DType::F32, 0.5).unwrap();
            b.iter(|| black_box(tensor_pool2d(&input, PoolOp::Max, (*pool, *pool), (*pool, *pool), (0, 0))));
        });

        // Average pooling
        group.bench_function(BenchmarkId::new("avg", &label), |b| {
            let input = TensorHandle::full(&[*batch, *ch, *h, *w], DType::F32, 0.5).unwrap();
            b.iter(|| black_box(tensor_pool2d(&input, PoolOp::Avg, (*pool, *pool), (*pool, *pool), (0, 0))));
        });
    }

    group.finish();
}

// ============================================================================
// Broadcasting Benchmarks
// ============================================================================

fn bench_broadcast_shapes(c: &mut Criterion) {
    let mut group = c.benchmark_group("broadcast_shapes");

    let shape_pairs = [
        (vec![1], vec![1000]),
        (vec![1, 1000], vec![1000, 1]),
        (vec![1, 1, 1000], vec![10, 100, 1]),
        (vec![256, 1, 64], vec![1, 128, 64]),
    ];

    for (a, b) in shape_pairs.iter() {
        let label = format!("{:?}_vs_{:?}", a, b);
        group.bench_function(&label, |bench| {
            bench.iter(|| black_box(broadcast_shapes(a, b)));
        });
    }
    group.finish();
}

fn bench_broadcast_to(c: &mut Criterion) {
    let mut group = c.benchmark_group("broadcast_to");

    let cases = [
        (vec![1], vec![1024]),
        (vec![1, 64], vec![128, 64]),
        (vec![1, 1, 256], vec![32, 64, 256]),
    ];

    for (src_shape, dst_shape) in cases.iter() {
        let label = format!("{:?}_to_{:?}", src_shape, dst_shape);
        let numel: usize = dst_shape.iter().product();
        group.throughput(Throughput::Elements(numel as u64));
        group.bench_function(&label, |bench| {
            let tensor = TensorHandle::full(src_shape, DType::F32, 1.0).unwrap();
            bench.iter(|| black_box(broadcast_to(&tensor, dst_shape)));
        });
    }
    group.finish();
}

fn bench_binop_with_broadcast(c: &mut Criterion) {
    let mut group = c.benchmark_group("binop_broadcast");

    // Vector + scalar broadcast
    group.bench_function("vec_scalar_1024", |b| {
        let vec = TensorHandle::full(&[1024], DType::F32, 2.0).unwrap();
        let scalar = TensorHandle::full(&[1], DType::F32, 3.0).unwrap();
        b.iter(|| black_box(tensor_binop(&vec, &scalar, TensorBinaryOp::Add)));
    });

    // Matrix + vector broadcast
    group.bench_function("mat_vec_128x128", |b| {
        let mat = TensorHandle::full(&[128, 128], DType::F32, 2.0).unwrap();
        let vec = TensorHandle::full(&[128], DType::F32, 3.0).unwrap();
        b.iter(|| black_box(tensor_binop(&mat, &vec, TensorBinaryOp::Add)));
    });

    // 3D + 2D broadcast
    group.bench_function("3d_2d_32x64x128", |b| {
        let tensor3d = TensorHandle::full(&[32, 64, 128], DType::F32, 2.0).unwrap();
        let tensor2d = TensorHandle::full(&[64, 128], DType::F32, 3.0).unwrap();
        b.iter(|| black_box(tensor_binop(&tensor3d, &tensor2d, TensorBinaryOp::Mul)));
    });

    // Scalar + large tensor (common pattern)
    group.bench_function("scalar_large_65536", |b| {
        let scalar = TensorHandle::full(&[1], DType::F32, 2.0).unwrap();
        let large = TensorHandle::full(&[65536], DType::F32, 3.0).unwrap();
        b.iter(|| black_box(tensor_binop(&scalar, &large, TensorBinaryOp::Mul)));
    });

    group.finish();
}

// ============================================================================
// Memory Throughput Benchmarks
// ============================================================================

fn bench_memory_bandwidth(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_bandwidth");

    // Simple copy operation (measures memory bandwidth)
    for size in HUGE_SIZES.iter() {
        let bytes = (*size * 4) as u64;  // F32 = 4 bytes
        group.throughput(Throughput::Bytes(bytes));
        group.bench_with_input(BenchmarkId::new("copy_via_add0", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 1.0).unwrap();
            let zero = TensorHandle::full(&[size], DType::F32, 0.0).unwrap();
            b.iter(|| black_box(dispatch_binop(&a, &zero, TensorBinaryOp::Add)));
        });
    }
    group.finish();
}

// ============================================================================
// Metal GPU Benchmarks (macOS only)
// ============================================================================

#[cfg(all(target_os = "macos", feature = "metal"))]
fn bench_metal_binop_all(c: &mut Criterion) {
    use verum_vbc::interpreter::kernel::metal::{MetalBackend, is_metal_available};

    if !is_metal_available() {
        return;
    }

    let backend = match MetalBackend::new() {
        Some(b) => b,
        None => return,
    };

    let ops = [
        ("add", TensorBinaryOp::Add),
        ("sub", TensorBinaryOp::Sub),
        ("mul", TensorBinaryOp::Mul),
        ("div", TensorBinaryOp::Div),
        ("max", TensorBinaryOp::Max),
        ("min", TensorBinaryOp::Min),
    ];

    for (name, op) in ops.iter() {
        let mut group = c.benchmark_group(format!("metal_binop_{}", name));

        for size in [4096, 16384, 65536, 262144, 1048576, 4194304].iter() {
            group.throughput(Throughput::Elements(*size as u64));
            group.bench_with_input(BenchmarkId::new("f32_gpu", size), size, |b, &size| {
                let a = TensorHandle::full(&[size], DType::F32, 2.0).unwrap();
                let b_tensor = TensorHandle::full(&[size], DType::F32, 3.0).unwrap();
                b.iter(|| black_box(backend.binop_gpu(&a, &b_tensor, *op)));
            });
        }
        group.finish();
    }
}

#[cfg(all(target_os = "macos", feature = "metal"))]
fn bench_metal_unop_all(c: &mut Criterion) {
    use verum_vbc::interpreter::kernel::metal::{MetalBackend, is_metal_available};

    if !is_metal_available() {
        return;
    }

    let backend = match MetalBackend::new() {
        Some(b) => b,
        None => return,
    };

    let ops = [
        ("neg", TensorUnaryOp::Neg),
        ("abs", TensorUnaryOp::Abs),
        ("sqrt", TensorUnaryOp::Sqrt),
        ("exp", TensorUnaryOp::Exp),
        ("log", TensorUnaryOp::Log),
        ("sin", TensorUnaryOp::Sin),
        ("cos", TensorUnaryOp::Cos),
        ("tanh", TensorUnaryOp::Tanh),
        ("relu", TensorUnaryOp::Relu),
        ("sigmoid", TensorUnaryOp::Sigmoid),
        ("gelu", TensorUnaryOp::Gelu),
        ("silu", TensorUnaryOp::Silu),
    ];

    for (name, op) in ops.iter() {
        let mut group = c.benchmark_group(format!("metal_unop_{}", name));

        for size in [4096, 16384, 65536, 262144, 1048576].iter() {
            group.throughput(Throughput::Elements(*size as u64));
            group.bench_with_input(BenchmarkId::new("f32_gpu", size), size, |b, &size| {
                let a = TensorHandle::full(&[size], DType::F32, 2.0).unwrap();
                b.iter(|| black_box(backend.unop_gpu(&a, *op)));
            });
        }
        group.finish();
    }
}

#[cfg(all(target_os = "macos", feature = "metal"))]
fn bench_metal_matmul(c: &mut Criterion) {
    use verum_vbc::interpreter::kernel::metal::{MetalBackend, is_metal_available};

    if !is_metal_available() {
        return;
    }

    let backend = match MetalBackend::new() {
        Some(b) => b,
        None => return,
    };

    let mut group = c.benchmark_group("metal_matmul");

    for size in [64, 128, 256, 512, 1024, 2048].iter() {
        let flops = 2 * (*size as u64).pow(3);
        group.throughput(Throughput::Elements(flops));
        group.bench_with_input(BenchmarkId::new("f32_gpu", size), size, |b, &size| {
            let a = TensorHandle::full(&[size, size], DType::F32, 0.1).unwrap();
            let b_tensor = TensorHandle::full(&[size, size], DType::F32, 0.1).unwrap();
            b.iter(|| black_box(backend.matmul_gpu(&a, &b_tensor)));
        });
    }
    group.finish();
}

#[cfg(all(target_os = "macos", feature = "metal"))]
fn bench_metal_vs_cpu(c: &mut Criterion) {
    use verum_vbc::interpreter::kernel::metal::{MetalBackend, is_metal_available};

    if !is_metal_available() {
        return;
    }

    let backend = match MetalBackend::new() {
        Some(b) => b,
        None => return,
    };

    let mut group = c.benchmark_group("gpu_vs_cpu");

    // Compare GPU vs CPU for various operations
    for size in [4096, 16384, 65536, 262144, 1048576].iter() {
        group.throughput(Throughput::Elements(*size as u64));

        // CPU binop
        group.bench_with_input(BenchmarkId::new("cpu_add", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 2.0).unwrap();
            let b_tensor = TensorHandle::full(&[size], DType::F32, 3.0).unwrap();
            b.iter(|| black_box(dispatch_binop(&a, &b_tensor, TensorBinaryOp::Add)));
        });

        // GPU binop
        group.bench_with_input(BenchmarkId::new("gpu_add", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 2.0).unwrap();
            let b_tensor = TensorHandle::full(&[size], DType::F32, 3.0).unwrap();
            b.iter(|| black_box(backend.binop_gpu(&a, &b_tensor, TensorBinaryOp::Add)));
        });
    }
    group.finish();
}

#[cfg(all(target_os = "macos", feature = "metal"))]
fn bench_metal_vectorization(c: &mut Criterion) {
    use verum_vbc::interpreter::kernel::metal::{MetalBackend, is_metal_available};

    if !is_metal_available() {
        return;
    }

    let backend = match MetalBackend::new() {
        Some(b) => b,
        None => return,
    };

    let mut group = c.benchmark_group("metal_vectorization_throughput");

    // Large tensors to measure peak GPU throughput
    for size in [1048576, 4194304, 16777216].iter() {
        let bytes = (*size * 4) as u64;  // F32 = 4 bytes
        group.throughput(Throughput::Bytes(bytes * 3));  // Read 2, write 1
        group.bench_with_input(BenchmarkId::new("add_throughput", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, std::f64::consts::PI).unwrap();
            let b_tensor = TensorHandle::full(&[size], DType::F32, std::f64::consts::E).unwrap();
            b.iter(|| black_box(backend.binop_gpu(&a, &b_tensor, TensorBinaryOp::Add)));
        });
    }
    group.finish();
}

#[cfg(all(target_os = "macos", feature = "metal"))]
fn bench_metal_softmax(c: &mut Criterion) {
    use verum_vbc::interpreter::kernel::metal::{MetalBackend, is_metal_available};

    if !is_metal_available() {
        return;
    }

    let backend = match MetalBackend::new() {
        Some(b) => b,
        None => return,
    };

    let mut group = c.benchmark_group("metal_softmax");

    // 1D softmax
    for size in [256, 1024, 4096, 16384, 65536].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::new("1d", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 0.5).unwrap();
            b.iter(|| black_box(backend.softmax_gpu(&a)));
        });
    }

    // 2D batch softmax (typical attention pattern)
    let batch_configs = [
        (8, 64),
        (16, 128),
        (32, 256),
        (64, 512),
        (128, 1024),
    ];

    for (batch, dim) in batch_configs.iter() {
        let label = format!("{}x{}", batch, dim);
        let numel = batch * dim;
        group.throughput(Throughput::Elements(numel as u64));
        group.bench_function(BenchmarkId::new("2d_batch", &label), |b| {
            let a = TensorHandle::full(&[*batch, *dim], DType::F32, 0.5).unwrap();
            b.iter(|| black_box(backend.softmax_gpu(&a)));
        });
    }

    group.finish();
}

#[cfg(all(target_os = "macos", feature = "metal"))]
fn bench_metal_layer_norm(c: &mut Criterion) {
    use verum_vbc::interpreter::kernel::metal::{MetalBackend, is_metal_available};

    if !is_metal_available() {
        return;
    }

    let backend = match MetalBackend::new() {
        Some(b) => b,
        None => return,
    };

    let mut group = c.benchmark_group("metal_layer_norm");

    // Typical transformer dimensions
    let configs = [
        (8, 256),
        (16, 512),
        (32, 768),    // BERT-base hidden size
        (32, 1024),   // BERT-large hidden size
        (64, 512),
        (128, 256),
    ];

    for (batch, hidden) in configs.iter() {
        let label = format!("{}x{}", batch, hidden);
        let numel = batch * hidden;
        group.throughput(Throughput::Elements(numel as u64));

        // Without gamma/beta
        group.bench_function(BenchmarkId::new("no_affine", &label), |b| {
            let input = TensorHandle::full(&[*batch, *hidden], DType::F32, 0.5).unwrap();
            b.iter(|| black_box(backend.layer_norm_gpu(&input, None, None, 1e-5)));
        });

        // With gamma/beta
        group.bench_function(BenchmarkId::new("with_affine", &label), |b| {
            let input = TensorHandle::full(&[*batch, *hidden], DType::F32, 0.5).unwrap();
            let gamma = TensorHandle::full(&[*hidden], DType::F32, 1.0).unwrap();
            let beta = TensorHandle::full(&[*hidden], DType::F32, 0.0).unwrap();
            b.iter(|| black_box(backend.layer_norm_gpu(&input, Some(&gamma), Some(&beta), 1e-5)));
        });
    }

    // Large hidden sizes (triggers multi-pass)
    let large_configs = [
        (8, 2048),
        (16, 4096),
    ];

    for (batch, hidden) in large_configs.iter() {
        let label = format!("{}x{}_large", batch, hidden);
        let numel = batch * hidden;
        group.throughput(Throughput::Elements(numel as u64));
        group.bench_function(&label, |b| {
            let input = TensorHandle::full(&[*batch, *hidden], DType::F32, 0.5).unwrap();
            b.iter(|| black_box(backend.layer_norm_gpu(&input, None, None, 1e-5)));
        });
    }

    group.finish();
}

#[cfg(all(target_os = "macos", feature = "metal"))]
fn bench_metal_softmax_vs_cpu(c: &mut Criterion) {
    use verum_vbc::interpreter::kernel::metal::{MetalBackend, is_metal_available};

    if !is_metal_available() {
        return;
    }

    let backend = match MetalBackend::new() {
        Some(b) => b,
        None => return,
    };

    let mut group = c.benchmark_group("softmax_gpu_vs_cpu");

    for size in [1024, 4096, 16384, 65536].iter() {
        group.throughput(Throughput::Elements(*size as u64));

        // CPU softmax
        group.bench_with_input(BenchmarkId::new("cpu", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 0.5).unwrap();
            b.iter(|| black_box(tensor_softmax(&a, None)));
        });

        // GPU softmax
        group.bench_with_input(BenchmarkId::new("gpu", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 0.5).unwrap();
            b.iter(|| black_box(backend.softmax_gpu(&a)));
        });
    }

    group.finish();
}

#[cfg(all(target_os = "macos", feature = "metal"))]
fn bench_metal_layer_norm_vs_cpu(c: &mut Criterion) {
    use verum_vbc::interpreter::kernel::metal::{MetalBackend, is_metal_available};

    if !is_metal_available() {
        return;
    }

    let backend = match MetalBackend::new() {
        Some(b) => b,
        None => return,
    };

    let mut group = c.benchmark_group("layer_norm_gpu_vs_cpu");

    let configs = [
        (16, 512),
        (32, 768),
        (64, 1024),
    ];

    for (batch, hidden) in configs.iter() {
        let label = format!("{}x{}", batch, hidden);
        let numel = batch * hidden;
        group.throughput(Throughput::Elements(numel as u64));

        // CPU layer norm
        group.bench_function(BenchmarkId::new("cpu", &label), |b| {
            let input = TensorHandle::full(&[*batch, *hidden], DType::F32, 0.5).unwrap();
            let gamma = TensorHandle::full(&[*hidden], DType::F32, 1.0).unwrap();
            let beta = TensorHandle::full(&[*hidden], DType::F32, 0.0).unwrap();
            b.iter(|| black_box(tensor_layer_norm(&input, Some(&gamma), Some(&beta), 1e-5)));
        });

        // GPU layer norm
        group.bench_function(BenchmarkId::new("gpu", &label), |b| {
            let input = TensorHandle::full(&[*batch, *hidden], DType::F32, 0.5).unwrap();
            let gamma = TensorHandle::full(&[*hidden], DType::F32, 1.0).unwrap();
            let beta = TensorHandle::full(&[*hidden], DType::F32, 0.0).unwrap();
            b.iter(|| black_box(backend.layer_norm_gpu(&input, Some(&gamma), Some(&beta), 1e-5)));
        });
    }

    group.finish();
}

#[cfg(all(target_os = "macos", feature = "metal"))]
fn bench_metal_reduce(c: &mut Criterion) {
    use verum_vbc::interpreter::kernel::metal::{MetalBackend, is_metal_available};
    use verum_vbc::instruction::TensorReduceOp as MetalReduceOp;

    if !is_metal_available() {
        return;
    }

    let backend = match MetalBackend::new() {
        Some(b) => b,
        None => return,
    };

    let mut group = c.benchmark_group("metal_reduce");

    // Test reduce operations on various sizes
    for size in [256, 1024, 4096, 16384, 65536].iter() {
        group.throughput(Throughput::Elements(*size as u64));

        // Sum reduction
        group.bench_with_input(BenchmarkId::new("sum", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 1.0).unwrap();
            b.iter(|| black_box(backend.reduce_gpu(&a, MetalReduceOp::Sum, None)));
        });

        // Max reduction
        group.bench_with_input(BenchmarkId::new("max", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 1.0).unwrap();
            b.iter(|| black_box(backend.reduce_gpu(&a, MetalReduceOp::Max, None)));
        });

        // Mean reduction
        group.bench_with_input(BenchmarkId::new("mean", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 1.0).unwrap();
            b.iter(|| black_box(backend.reduce_gpu(&a, MetalReduceOp::Mean, None)));
        });
    }

    group.finish();
}

#[cfg(all(target_os = "macos", feature = "metal"))]
fn bench_metal_reduce_vs_cpu(c: &mut Criterion) {
    use verum_vbc::interpreter::kernel::metal::{MetalBackend, is_metal_available};
    use verum_vbc::instruction::TensorReduceOp as MetalReduceOp;

    if !is_metal_available() {
        return;
    }

    let backend = match MetalBackend::new() {
        Some(b) => b,
        None => return,
    };

    let mut group = c.benchmark_group("reduce_gpu_vs_cpu");

    for size in [4096, 16384, 65536].iter() {
        group.throughput(Throughput::Elements(*size as u64));

        // CPU sum
        group.bench_with_input(BenchmarkId::new("cpu_sum", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 1.0).unwrap();
            b.iter(|| black_box(tensor_reduce(&a, None, TensorReduceOp::Sum)));
        });

        // GPU sum
        group.bench_with_input(BenchmarkId::new("gpu_sum", size), size, |b, &size| {
            let a = TensorHandle::full(&[size], DType::F32, 1.0).unwrap();
            b.iter(|| black_box(backend.reduce_gpu(&a, MetalReduceOp::Sum, None)));
        });
    }

    group.finish();
}

// ============================================================================
// Capability Detection Benchmark
// ============================================================================

fn bench_capabilities_detection(c: &mut Criterion) {
    let mut group = c.benchmark_group("capabilities");

    group.bench_function("detect", |b| {
        b.iter(|| black_box(get_capabilities()));
    });

    group.finish();
}

// ============================================================================
// End-to-End Operation Chain Benchmarks
// ============================================================================

fn bench_operation_chains(c: &mut Criterion) {
    let mut group = c.benchmark_group("operation_chains");

    // Typical ML inference chain: matmul -> add bias -> relu
    group.bench_function("mlp_layer_256", |b| {
        let input = TensorHandle::full(&[32, 256], DType::F32, 0.5).unwrap();
        let weights = TensorHandle::full(&[256, 256], DType::F32, 0.1).unwrap();
        let bias = TensorHandle::full(&[256], DType::F32, 0.0).unwrap();
        b.iter(|| {
            let mm = tensor_matmul(&input, &weights).unwrap();
            let added = tensor_binop(&mm, &bias, TensorBinaryOp::Add).unwrap();
            black_box(tensor_unop(&added, TensorUnaryOp::Relu))
        });
    });

    // Attention pattern: matmul -> softmax
    group.bench_function("attention_64x64", |b| {
        let q = TensorHandle::full(&[64, 64], DType::F32, 0.5).unwrap();
        let k = TensorHandle::full(&[64, 64], DType::F32, 0.5).unwrap();
        b.iter(|| {
            let scores = tensor_matmul(&q, &k).unwrap();
            black_box(tensor_softmax(&scores, Some(-1)))
        });
    });

    // Normalization chain: mean -> sub -> var -> div
    group.bench_function("normalize_65536", |b| {
        let x = tensor_rand(&[65536], DType::F32).unwrap();
        b.iter(|| {
            let sum = tensor_reduce(&x, None, TensorReduceOp::Sum).unwrap();
            let mean = sum.get_scalar_f64().unwrap() / 65536.0;
            let mean_t = TensorHandle::full(&[1], DType::F32, mean).unwrap();
            let centered = tensor_binop(&x, &mean_t, TensorBinaryOp::Sub).unwrap();
            let sq = tensor_binop(&centered, &centered, TensorBinaryOp::Mul).unwrap();
            let var_sum = tensor_reduce(&sq, None, TensorReduceOp::Sum).unwrap();
            black_box(var_sum)
        });
    });

    group.finish();
}

// ============================================================================
// Criterion Groups
// ============================================================================

criterion_group!(
    creation_benches,
    bench_tensor_creation,
    bench_tensor_2d_creation,
);

criterion_group!(
    binop_benches,
    bench_binop_all_ops,
    bench_binop_scalar_vs_simd,
    bench_binop_datatypes,
    bench_binop_edge_cases,
);

criterion_group!(
    unop_benches,
    bench_unop_all_ops,
    bench_unop_scalar_vs_neon,
    bench_unop_activation_functions,
    bench_unop_math_functions,
);

criterion_group!(
    reduce_benches,
    bench_reduce_all_ops,
    bench_reduce_scalar_vs_neon,
    bench_reduce_axis,
);

criterion_group!(
    matmul_benches,
    bench_matmul_square,
    bench_matmul_scalar_vs_tiled,
    bench_matmul_rectangular,
    bench_matmul_batch,
);

criterion_group!(
    shape_benches,
    bench_transpose,
    bench_reshape,
    bench_squeeze,
    bench_slice,
    bench_concat,
    bench_stack,
    bench_clone,
);

criterion_group!(
    nn_benches,
    bench_softmax,
    bench_argmax,
    bench_layer_norm,
    bench_conv2d,
    bench_pool2d,
);

criterion_group!(
    broadcast_benches,
    bench_broadcast_shapes,
    bench_broadcast_to,
    bench_binop_with_broadcast,
);

criterion_group!(
    memory_benches,
    bench_memory_bandwidth,
);

criterion_group!(
    misc_benches,
    bench_capabilities_detection,
    bench_operation_chains,
);

#[cfg(all(target_os = "macos", feature = "metal"))]
criterion_group!(
    metal_benches,
    bench_metal_binop_all,
    bench_metal_unop_all,
    bench_metal_matmul,
    bench_metal_vs_cpu,
    bench_metal_vectorization,
    bench_metal_softmax,
    bench_metal_layer_norm,
    bench_metal_softmax_vs_cpu,
    bench_metal_layer_norm_vs_cpu,
    bench_metal_reduce,
    bench_metal_reduce_vs_cpu,
);

#[cfg(all(target_os = "macos", feature = "metal"))]
criterion_main!(
    creation_benches,
    binop_benches,
    unop_benches,
    reduce_benches,
    matmul_benches,
    shape_benches,
    nn_benches,
    broadcast_benches,
    memory_benches,
    misc_benches,
    metal_benches
);

#[cfg(not(all(target_os = "macos", feature = "metal")))]
criterion_main!(
    creation_benches,
    binop_benches,
    unop_benches,
    reduce_benches,
    matmul_benches,
    shape_benches,
    nn_benches,
    broadcast_benches,
    memory_benches,
    misc_benches
);
