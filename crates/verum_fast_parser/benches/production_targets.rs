//! Production Target Benchmarks for verum_fast_parser
//!
//! Target: Compilation > 50K LOC/sec (parsing is part of the pipeline)
//!
//! This benchmark generates realistic Verum source at various scales
//! and measures parse throughput in LOC/sec.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use std::time::Duration;
use verum_ast::span::FileId;
use verum_fast_parser::FastParser;
use verum_lexer::Lexer;

// ============================================================================
// Realistic Code Generators
// ============================================================================

/// Generate realistic Verum module with types, functions, impls.
/// Each "unit" is ~10 LOC, so 1000 units = ~10K LOC.
fn generate_realistic_module(units: usize) -> String {
    let mut out = String::with_capacity(units * 200);

    for i in 0..units {
        match i % 5 {
            0 => {
                // Type definition + impl
                out.push_str(&format!(
                    r#"
type Widget{i} is {{
    id: Int,
    name: Text,
    value: Float,
    active: Bool,
}};

implement Widget{i} {{
    fn new(id: Int, name: Text) -> Widget{i} {{
        Widget{i} {{ id, name, value: 0.0, active: true }}
    }}
}}
"#
                ));
            }
            1 => {
                // Function with match
                out.push_str(&format!(
                    r#"
fn process_{i}(input: Maybe<Int>) -> Int {{
    match input {{
        Some(x) if x > 0 => x * 2,
        Some(x) => x + 1,
        None => 0,
    }}
}}
"#
                ));
            }
            2 => {
                // Async function with context
                out.push_str(&format!(
                    r#"
async fn fetch_{i}(url: Text) -> Result<Text, Error> using [Network] {{
    let response = http_get(url).await?;
    let body = response.text().await?;
    Ok(body)
}}
"#
                ));
            }
            3 => {
                // Variant type + pattern matching
                out.push_str(&format!(
                    r#"
type Shape{i} is Circle(Float) | Rect(Float, Float) | Triangle(Float, Float, Float);

fn area_{i}(shape: Shape{i}) -> Float {{
    match shape {{
        Circle(r) => 3.14159 * r * r,
        Rect(w, h) => w * h,
        Triangle(a, b, c) => {{
            let s = (a + b + c) / 2.0;
            sqrt(s * (s - a) * (s - b) * (s - c))
        }},
    }}
}}
"#
                ));
            }
            _ => {
                // Protocol + impl
                out.push_str(&format!(
                    r#"
type Printable{i} is protocol {{
    fn to_text(&self) -> Text;
    fn debug_text(&self) -> Text {{ f"Debug({{self.to_text()}})" }}
}};

implement Printable{i} for Int {{
    fn to_text(&self) -> Text {{ f"{{self}}" }}
}}
"#
                ));
            }
        }
    }

    out
}

/// Count lines of code (non-empty, non-comment).
fn count_loc(source: &str) -> usize {
    source
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("//")
        })
        .count()
}

// ============================================================================
// Benchmarks
// ============================================================================

fn bench_parse_throughput_loc(c: &mut Criterion) {
    let mut group = c.benchmark_group("production_target_parse_throughput");
    group.warm_up_time(Duration::from_secs(2));
    group.measurement_time(Duration::from_secs(5));

    let parser = FastParser::new();

    // Test at different scales to verify >50K LOC/sec
    for units in [100, 500, 1000, 2000] {
        let source = generate_realistic_module(units);
        let loc = count_loc(&source);

        group.throughput(Throughput::Elements(loc as u64));
        group.bench_with_input(
            BenchmarkId::new("parse", format!("{loc}_loc")),
            &source,
            |b, source| {
                let file_id = FileId::new(0);
                b.iter(|| {
                    let lexer = Lexer::new(black_box(source), file_id);
                    let result = parser.parse_module(lexer, file_id);
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

fn bench_lex_throughput_loc(c: &mut Criterion) {
    let mut group = c.benchmark_group("production_target_lex_throughput");
    group.warm_up_time(Duration::from_secs(2));
    group.measurement_time(Duration::from_secs(5));

    // ~10K LOC module
    let source = generate_realistic_module(1000);
    let loc = count_loc(&source);

    group.throughput(Throughput::Elements(loc as u64));
    group.bench_function("lex_10k_loc", |b| {
        let file_id = FileId::new(0);
        b.iter(|| {
            let lexer = Lexer::new(black_box(&source), file_id);
            let tokens: Vec<_> = lexer.collect();
            black_box(tokens)
        });
    });

    group.finish();
}

fn validate_50k_loc_sec(c: &mut Criterion) {
    let mut group = c.benchmark_group("production_validation_parse");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));

    let parser = FastParser::new();

    // Generate ~10K LOC and verify it parses fast enough
    // At 50K LOC/sec target, 10K LOC should take < 200ms
    let source = generate_realistic_module(1000);
    let loc = count_loc(&source);

    group.bench_function(
        format!("{loc}_loc_target_under_200ms"),
        |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                let file_id = FileId::new(0);
                for _ in 0..iters {
                    let start = std::time::Instant::now();
                    let lexer = Lexer::new(black_box(&source), file_id);
                    let result = parser.parse_module(lexer, file_id);
                    let _ = black_box(result);
                    total += start.elapsed();
                }
                total
            });
        },
    );

    group.finish();
}

criterion_group!(
    benches,
    bench_parse_throughput_loc,
    bench_lex_throughput_loc,
    validate_50k_loc_sec,
);
criterion_main!(benches);
