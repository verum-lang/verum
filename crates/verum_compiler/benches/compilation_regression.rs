//! Compilation Regression Benchmark
//!
//! Criterion-based benchmarks for detecting performance regressions in the
//! Verum compiler pipeline. Each benchmark isolates a specific compilation
//! phase so regressions can be pinpointed to the responsible subsystem.
//!
//! Benchmarks:
//! - Parse speed (1K LOC Verum program)
//! - Type check speed (50 functions)
//! - VBC codegen speed (typed AST to VBC bytecode)
//! - Stdlib loading speed (load ~166 stdlib modules)
//! - End-to-end compile_string pipeline

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use std::path::PathBuf;
use tempfile::TempDir;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};
use verum_fast_parser::Parser;
use verum_lexer::Lexer;
use verum_ast::span::FileId;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

// ============================================================================
// Program Generators
// ============================================================================

/// Generate a ~1K LOC Verum program with diverse syntax constructs.
fn generate_1k_loc_program() -> String {
    let mut src = String::with_capacity(32_000);

    // Type definitions (~50 LOC)
    for i in 0..10 {
        src.push_str(&format!(
            "type Record{i} is {{ x{i}: Int, y{i}: Float, name{i}: Text }};\n"
        ));
    }

    // Sum types (~20 LOC)
    for i in 0..5 {
        src.push_str(&format!(
            "type Choice{i} is OptionA{i}(Int) | OptionB{i}(Text) | Empty{i};\n"
        ));
    }

    // Simple functions (~200 LOC)
    for i in 0..50 {
        src.push_str(&format!(
            "fn compute{i}(x: Int, y: Int) -> Int {{\n    let a = x + {i};\n    let b = y * 2;\n    a + b\n}}\n\n"
        ));
    }

    // Functions with match expressions (~300 LOC)
    for i in 0..30 {
        src.push_str(&format!(
            "fn classify{i}(n: Int) -> Int {{\n    match n {{\n        0 => 0,\n        1 => 1,\n        _ => n * {i}\n    }}\n}}\n\n"
        ));
    }

    // Functions with let-bindings and control flow (~300 LOC)
    for i in 0..25 {
        src.push_str(&format!(
            "fn process{i}(input: Int) -> Int {{\n\
             \x20   let step1 = input + {i};\n\
             \x20   let step2 = step1 * 2;\n\
             \x20   let step3 = if step2 > 100 {{ step2 - 50 }} else {{ step2 + 50 }};\n\
             \x20   let step4 = step3 + {val};\n\
             \x20   step4\n\
             }}\n\n",
            val = i * 3
        ));
    }

    // Recursive functions (~100 LOC)
    for i in 0..10 {
        src.push_str(&format!(
            "fn recurse{i}(n: Int) -> Int {{\n\
             \x20   match n {{\n\
             \x20       0 => {i},\n\
             \x20       _ => n + recurse{i}(n - 1)\n\
             \x20   }}\n\
             }}\n\n"
        ));
    }

    // Main function
    src.push_str("fn main() {\n    let result = compute0(1, 2);\n    result\n}\n");

    src
}

/// Generate a program with exactly N functions for type-checking stress.
fn generate_n_functions(n: usize) -> String {
    let mut src = String::with_capacity(n * 80);
    for i in 0..n {
        src.push_str(&format!(
            "fn func{i}(x: Int, y: Int) -> Int {{\n    let z = x + y + {i};\n    z\n}}\n\n"
        ));
    }
    src.push_str("fn main() {\n    let r = func0(1, 2);\n    r\n}\n");
    src
}

/// Generate a program with type definitions and impl blocks.
fn generate_typed_program(type_count: usize, methods_per_type: usize) -> String {
    let mut src = String::with_capacity(type_count * methods_per_type * 120);

    for t in 0..type_count {
        src.push_str(&format!(
            "type Widget{t} is {{ value: Int, factor: Int }};\n\n"
        ));
        src.push_str(&format!("implement Widget{t} {{\n"));
        for m in 0..methods_per_type {
            src.push_str(&format!(
                "    fn method{m}(&self) -> Int {{ self.value + self.factor * {m} }}\n"
            ));
        }
        src.push_str("}\n\n");
    }

    src.push_str("fn main() {\n    0\n}\n");
    src
}

// ============================================================================
// Parse Speed Benchmark
// ============================================================================

fn bench_parse_1k_loc(c: &mut Criterion) {
    let mut group = c.benchmark_group("regression/parse");

    let source = generate_1k_loc_program();
    let loc = source.lines().count();

    group.throughput(Throughput::Elements(loc as u64));
    group.bench_function(BenchmarkId::new("1k_loc", loc), |b| {
        b.iter(|| {
            let mut parser = Parser::new(&source);
            let module = parser.parse_module();
            black_box(module)
        });
    });

    // Also benchmark lexing alone to separate lex vs parse cost
    group.bench_function(BenchmarkId::new("lex_only_1k_loc", loc), |b| {
        b.iter(|| {
            let lexer = Lexer::new(&source, FileId::new(0));
            let tokens: Vec<_> = lexer.collect();
            black_box(tokens)
        });
    });

    // Scaling: parse at different sizes
    for size in [100, 500, 1000, 2000] {
        let src = generate_n_functions(size);
        let lines = src.lines().count();
        group.throughput(Throughput::Elements(lines as u64));
        group.bench_with_input(
            BenchmarkId::new("scale_funcs", size),
            &src,
            |b, source| {
                b.iter(|| {
                    let mut parser = Parser::new(source);
                    black_box(parser.parse_module())
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Type Check Speed Benchmark
// ============================================================================

fn bench_type_check_50_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("regression/type_check");
    group.sample_size(20);

    // 50-function program: parse once, then benchmark type checking via pipeline
    let source_50 = generate_n_functions(50);
    let loc_50 = source_50.lines().count();

    group.throughput(Throughput::Elements(loc_50 as u64));
    group.bench_function(BenchmarkId::new("50_functions", loc_50), |b| {
        b.iter(|| {
            let temp_dir = TempDir::new().expect("temp dir");
            let mut session = Session::new(CompilerOptions {
                input: PathBuf::from("bench_tc.vr"),
                output: temp_dir.path().join("bench_tc"),
                ..Default::default()
            });
            let mut pipeline = CompilationPipeline::new(&mut session);
            // compile_string runs parse + type_check + cbgr_analysis
            black_box(pipeline.compile_string(&source_50))
        });
    });

    // Program with type definitions and methods
    let typed_src = generate_typed_program(10, 5);
    let typed_loc = typed_src.lines().count();

    group.throughput(Throughput::Elements(typed_loc as u64));
    group.bench_function(BenchmarkId::new("10_types_5_methods", typed_loc), |b| {
        b.iter(|| {
            let temp_dir = TempDir::new().expect("temp dir");
            let mut session = Session::new(CompilerOptions {
                input: PathBuf::from("bench_typed.vr"),
                output: temp_dir.path().join("bench_typed"),
                ..Default::default()
            });
            let mut pipeline = CompilationPipeline::new(&mut session);
            black_box(pipeline.compile_string(&typed_src))
        });
    });

    // Scaling: type check at increasing sizes
    for n in [10, 50, 100, 200] {
        let src = generate_n_functions(n);
        let lines = src.lines().count();
        group.throughput(Throughput::Elements(lines as u64));
        group.bench_with_input(
            BenchmarkId::new("scale_funcs", n),
            &src,
            |b, source| {
                b.iter(|| {
                    let temp_dir = TempDir::new().expect("temp dir");
                    let mut session = Session::new(CompilerOptions {
                        input: PathBuf::from("bench_scale.vr"),
                        output: temp_dir.path().join("bench_scale"),
                        ..Default::default()
                    });
                    let mut pipeline = CompilationPipeline::new(&mut session);
                    black_box(pipeline.compile_string(source))
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// VBC Codegen Speed Benchmark
// ============================================================================

fn bench_vbc_codegen(c: &mut Criterion) {
    let mut group = c.benchmark_group("regression/vbc_codegen");
    group.sample_size(20);

    // Parse the program once, then benchmark VBC codegen repeatedly
    let source = generate_n_functions(50);

    // Pre-parse the AST
    let mut parser = Parser::new(&source);
    let module = parser.parse_module().expect("parse should succeed");

    let loc = source.lines().count();
    group.throughput(Throughput::Elements(loc as u64));

    group.bench_function(BenchmarkId::new("50_functions", loc), |b| {
        b.iter(|| {
            let config = CodegenConfig::new("bench_module");
            let mut codegen = VbcCodegen::with_config(config);
            let vbc_module = codegen.compile_module(&module);
            black_box(vbc_module)
        });
    });

    // VBC codegen with type definitions
    let typed_source = generate_typed_program(10, 5);
    let mut typed_parser = Parser::new(&typed_source);
    let typed_module = typed_parser.parse_module().expect("parse should succeed");
    let typed_loc = typed_source.lines().count();

    group.throughput(Throughput::Elements(typed_loc as u64));
    group.bench_function(BenchmarkId::new("10_types_5_methods", typed_loc), |b| {
        b.iter(|| {
            let config = CodegenConfig::new("bench_typed_module");
            let mut codegen = VbcCodegen::with_config(config);
            let vbc_module = codegen.compile_module(&typed_module);
            black_box(vbc_module)
        });
    });

    // Scaling: VBC codegen at increasing program sizes
    let sizes = [20, 50, 100, 200];
    for &n in &sizes {
        let src = generate_n_functions(n);
        let mut p = Parser::new(&src);
        let m = p.parse_module().expect("parse should succeed");
        let lines = src.lines().count();

        group.throughput(Throughput::Elements(lines as u64));
        group.bench_with_input(BenchmarkId::new("scale_funcs", n), &m, |b, module| {
            b.iter(|| {
                let config = CodegenConfig::new("bench_scale");
                let mut codegen = VbcCodegen::with_config(config);
                black_box(codegen.compile_module(module))
            });
        });
    }

    group.finish();
}

// ============================================================================
// Stdlib Loading Speed Benchmark
// ============================================================================

fn bench_stdlib_loading(c: &mut Criterion) {
    let mut group = c.benchmark_group("regression/stdlib_loading");
    group.sample_size(10); // Stdlib loading involves I/O

    // Benchmark pipeline creation which triggers stdlib loading
    group.bench_function("pipeline_creation_with_stdlib", |b| {
        b.iter(|| {
            let temp_dir = TempDir::new().expect("temp dir");
            let mut session = Session::new(CompilerOptions {
                input: PathBuf::from("bench_stdlib.vr"),
                output: temp_dir.path().join("bench_stdlib"),
                ..Default::default()
            });
            // Pipeline creation loads stdlib modules
            let pipeline = CompilationPipeline::new(&mut session);
            black_box(&pipeline);
        });
    });

    // Benchmark minimal compile (dominated by stdlib loading on first call)
    let minimal = "fn main() { 0 }";
    group.bench_function("minimal_compile_with_stdlib", |b| {
        b.iter(|| {
            let temp_dir = TempDir::new().expect("temp dir");
            let mut session = Session::new(CompilerOptions {
                input: PathBuf::from("bench_min.vr"),
                output: temp_dir.path().join("bench_min"),
                ..Default::default()
            });
            let mut pipeline = CompilationPipeline::new(&mut session);
            black_box(pipeline.compile_string(minimal))
        });
    });

    group.finish();
}

// ============================================================================
// End-to-End Pipeline Benchmark
// ============================================================================

fn bench_end_to_end(c: &mut Criterion) {
    let mut group = c.benchmark_group("regression/end_to_end");
    group.sample_size(10);

    // Small program: baseline latency
    let small = "fn main() {\n    let x = 42;\n    x\n}\n";
    group.bench_function("small_program", |b| {
        b.iter(|| {
            let temp_dir = TempDir::new().expect("temp dir");
            let mut session = Session::new(CompilerOptions {
                input: PathBuf::from("bench_small.vr"),
                output: temp_dir.path().join("bench_small"),
                ..Default::default()
            });
            let mut pipeline = CompilationPipeline::new(&mut session);
            black_box(pipeline.compile_string(small))
        });
    });

    // Medium program: 1K LOC
    let medium = generate_1k_loc_program();
    let medium_loc = medium.lines().count();
    group.throughput(Throughput::Elements(medium_loc as u64));

    group.bench_function(BenchmarkId::new("1k_loc_full_pipeline", medium_loc), |b| {
        b.iter(|| {
            let temp_dir = TempDir::new().expect("temp dir");
            let mut session = Session::new(CompilerOptions {
                input: PathBuf::from("bench_1k.vr"),
                output: temp_dir.path().join("bench_1k"),
                ..Default::default()
            });
            let mut pipeline = CompilationPipeline::new(&mut session);
            black_box(pipeline.compile_string(&medium))
        });
    });

    // Realistic program with mixed constructs
    let realistic = r#"
type Point is { x: Int, y: Int };

implement Point {
    fn distance_sq(&self, other: &Point) -> Int {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx * dx + dy * dy
    }
}

fn factorial(n: Int) -> Int {
    match n {
        0 => 1,
        _ => n * factorial(n - 1)
    }
}

fn fibonacci(n: Int) -> Int {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2)
    }
}

fn sum_range(start: Int, end: Int) -> Int {
    let mut total = 0;
    let mut i = start;
    while i < end {
        total = total + i;
        i = i + 1;
    }
    total
}

fn main() {
    let f10 = factorial(10);
    let fib10 = fibonacci(10);
    let s = sum_range(0, 100);
    f10 + fib10 + s
}
"#;

    group.bench_function("realistic_program", |b| {
        b.iter(|| {
            let temp_dir = TempDir::new().expect("temp dir");
            let mut session = Session::new(CompilerOptions {
                input: PathBuf::from("bench_real.vr"),
                output: temp_dir.path().join("bench_real"),
                ..Default::default()
            });
            let mut pipeline = CompilationPipeline::new(&mut session);
            black_box(pipeline.compile_string(realistic))
        });
    });

    // Throughput scaling
    for size in [100, 500, 1000, 2000] {
        let src = generate_n_functions(size);
        let lines = src.lines().count();
        group.throughput(Throughput::Elements(lines as u64));

        group.bench_with_input(
            BenchmarkId::new("throughput_funcs", size),
            &src,
            |b, source| {
                b.iter(|| {
                    let temp_dir = TempDir::new().expect("temp dir");
                    let mut session = Session::new(CompilerOptions {
                        input: PathBuf::from("bench_tp.vr"),
                        output: temp_dir.path().join("bench_tp"),
                        ..Default::default()
                    });
                    let mut pipeline = CompilationPipeline::new(&mut session);
                    black_box(pipeline.compile_string(source))
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark Groups
// ============================================================================

criterion_group!(
    parse_regression,
    bench_parse_1k_loc
);

criterion_group!(
    type_check_regression,
    bench_type_check_50_functions
);

criterion_group!(
    vbc_codegen_regression,
    bench_vbc_codegen
);

criterion_group!(
    stdlib_regression,
    bench_stdlib_loading
);

criterion_group!(
    end_to_end_regression,
    bench_end_to_end
);

criterion_main!(
    parse_regression,
    type_check_regression,
    vbc_codegen_regression,
    stdlib_regression,
    end_to_end_regression
);
