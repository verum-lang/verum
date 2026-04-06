//! LLVM Lowering Regression Benchmark
//!
//! Criterion-based benchmarks for detecting performance regressions in the
//! VBC-to-LLVM lowering path and CBGR escape analysis overhead.
//!
//! Benchmarks:
//! - LLVM lowering speed (lower a VBC module with 20 functions)
//! - CBGR check elimination (measure escape analysis overhead)

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use verum_codegen::llvm::vbc_lowering::{LoweringConfig, VbcToLlvmLowering};
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::VbcModule;
use verum_fast_parser::Parser;

// ============================================================================
// Program Generators
// ============================================================================

/// Generate a Verum program with N functions for VBC module construction.
fn generate_n_functions(n: usize) -> String {
    let mut src = String::with_capacity(n * 100);
    for i in 0..n {
        src.push_str(&format!(
            "fn func{i}(x: Int, y: Int) -> Int {{\n\
             \x20   let a = x + {i};\n\
             \x20   let b = y * 2;\n\
             \x20   let c = a + b;\n\
             \x20   c\n\
             }}\n\n"
        ));
    }
    src.push_str("fn main() {\n    let r = func0(1, 2);\n    r\n}\n");
    src
}

/// Generate a program with reference-heavy code to stress CBGR analysis.
fn generate_reference_heavy_program(n: usize) -> String {
    let mut src = String::with_capacity(n * 200);

    // Type definitions
    for i in 0..n {
        src.push_str(&format!(
            "type Data{i} is {{ value: Int, tag: Int }};\n\n"
        ));
    }

    // Functions that create and use references (CBGR-relevant)
    for i in 0..n {
        src.push_str(&format!(
            "fn process_data{i}(d: Data{i}) -> Int {{\n\
             \x20   let v = d.value;\n\
             \x20   let t = d.tag;\n\
             \x20   v + t + {i}\n\
             }}\n\n"
        ));
    }

    // Functions with control flow that complicates escape analysis
    for i in 0..n {
        src.push_str(&format!(
            "fn branch{i}(x: Int) -> Int {{\n\
             \x20   if x > {i} {{\n\
             \x20       x * 2\n\
             \x20   }} else {{\n\
             \x20       x + {val}\n\
             \x20   }}\n\
             }}\n\n",
            val = i * 3
        ));
    }

    src.push_str("fn main() {\n    0\n}\n");
    src
}

/// Generate a program with deeply nested control flow.
fn generate_nested_control_flow(depth: usize, funcs: usize) -> String {
    let mut src = String::with_capacity(funcs * depth * 50);
    for f in 0..funcs {
        src.push_str(&format!("fn nested{f}(x: Int) -> Int {{\n"));
        for d in 0..depth {
            src.push_str(&format!(
                "{indent}if x > {threshold} {{\n",
                indent = "    ".repeat(d + 1),
                threshold = d * 10 + f
            ));
        }
        src.push_str(&format!(
            "{indent}x\n",
            indent = "    ".repeat(depth + 1)
        ));
        for d in (0..depth).rev() {
            src.push_str(&format!(
                "{indent}}} else {{ {val} }}\n",
                indent = "    ".repeat(d + 1),
                val = d + f
            ));
        }
        src.push_str("}\n\n");
    }
    src.push_str("fn main() { 0 }\n");
    src
}

/// Compile source to a VbcModule for use in lowering benchmarks.
fn source_to_vbc_module(source: &str) -> VbcModule {
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("parse should succeed");
    let config = CodegenConfig::new("bench_module");
    let mut codegen = VbcCodegen::with_config(config);
    codegen.compile_module(&module).expect("VBC codegen should succeed")
}

// ============================================================================
// LLVM Lowering Speed Benchmark
// ============================================================================

fn bench_llvm_lowering(c: &mut Criterion) {
    let mut group = c.benchmark_group("regression/llvm_lowering");
    group.sample_size(10);

    // 20-function module: primary regression target
    let src_20 = generate_n_functions(20);
    let vbc_20 = source_to_vbc_module(&src_20);
    let func_count = vbc_20.functions.len();

    group.throughput(Throughput::Elements(func_count as u64));
    group.bench_function(
        BenchmarkId::new("20_functions", func_count),
        |b| {
            b.iter(|| {
                let llvm_ctx = verum_llvm::context::Context::create();
                let config = LoweringConfig::new("bench_20");
                let mut lowering = VbcToLlvmLowering::new(&llvm_ctx, config);
                let result = lowering.lower_module(&vbc_20);
                black_box(result)
            });
        },
    );

    // Scaling: lowering at different sizes
    for &n in &[5, 10, 20, 50] {
        let src = generate_n_functions(n);
        let vbc = source_to_vbc_module(&src);
        let fcount = vbc.functions.len();

        group.throughput(Throughput::Elements(fcount as u64));
        group.bench_with_input(
            BenchmarkId::new("scale_funcs", n),
            &vbc,
            |b, vbc_module| {
                b.iter(|| {
                    let llvm_ctx = verum_llvm::context::Context::create();
                    let config = LoweringConfig::new("bench_scale");
                    let mut lowering = VbcToLlvmLowering::new(&llvm_ctx, config);
                    black_box(lowering.lower_module(vbc_module))
                });
            },
        );
    }

    // Nested control flow stress test
    let nested_src = generate_nested_control_flow(5, 20);
    let nested_vbc = source_to_vbc_module(&nested_src);

    group.bench_function("nested_control_flow_20_funcs", |b| {
        b.iter(|| {
            let llvm_ctx = verum_llvm::context::Context::create();
            let config = LoweringConfig::new("bench_nested");
            let mut lowering = VbcToLlvmLowering::new(&llvm_ctx, config);
            black_box(lowering.lower_module(&nested_vbc))
        });
    });

    group.finish();
}

// ============================================================================
// CBGR Check Elimination Benchmark
// ============================================================================

fn bench_cbgr_escape_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("regression/cbgr_escape_analysis");
    group.sample_size(10);

    // Reference-heavy program: CBGR escape analysis stress
    let ref_src = generate_reference_heavy_program(20);
    let ref_loc = ref_src.lines().count();

    group.throughput(Throughput::Elements(ref_loc as u64));
    group.bench_function(
        BenchmarkId::new("20_types_ref_heavy", ref_loc),
        |b| {
            b.iter(|| {
                // Parse + VBC codegen (includes CBGR tier annotations)
                let mut parser = Parser::new(&ref_src);
                let module = parser.parse_module().expect("parse should succeed");
                let config = CodegenConfig::new("cbgr_bench");
                let mut codegen = VbcCodegen::with_config(config);
                let vbc = codegen.compile_module(&module);
                black_box(vbc)
            });
        },
    );

    // Scaling: escape analysis at different program sizes
    for &n in &[5, 10, 20, 40] {
        let src = generate_reference_heavy_program(n);
        let lines = src.lines().count();
        group.throughput(Throughput::Elements(lines as u64));

        group.bench_with_input(
            BenchmarkId::new("scale_types", n),
            &src,
            |b, source| {
                b.iter(|| {
                    let mut parser = Parser::new(source);
                    let module = parser.parse_module().expect("parse");
                    let config = CodegenConfig::new("cbgr_scale");
                    let mut codegen = VbcCodegen::with_config(config);
                    black_box(codegen.compile_module(&module))
                });
            },
        );
    }

    // Measure VBC module construction overhead (baseline without LLVM)
    let baseline_src = generate_n_functions(20);
    let baseline_loc = baseline_src.lines().count();

    // Pre-parse for VBC-only benchmark
    let mut bp = Parser::new(&baseline_src);
    let baseline_module = bp.parse_module().expect("parse");

    group.throughput(Throughput::Elements(baseline_loc as u64));
    group.bench_function("vbc_only_baseline_20_funcs", |b| {
        b.iter(|| {
            let config = CodegenConfig::new("baseline");
            let mut codegen = VbcCodegen::with_config(config);
            black_box(codegen.compile_module(&baseline_module))
        });
    });

    group.finish();
}

// ============================================================================
// Benchmark Groups
// ============================================================================

criterion_group!(
    lowering_regression,
    bench_llvm_lowering
);

criterion_group!(
    cbgr_regression,
    bench_cbgr_escape_analysis
);

criterion_main!(
    lowering_regression,
    cbgr_regression
);
