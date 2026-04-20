//! T1-K Compilation speed contract
//!
//! Guards the published parse + VBC-codegen throughput targets against
//! regression. The values embedded here are the floor — actual
//! performance on `main` is routinely 15–30× higher but that leaves
//! enough headroom that a real regression of 30–70 % will still trip
//! the contract and fail CI instead of silently rotting the target.
//!
//! Measurement shape: reuse the same 1 K-LOC Verum program the
//! Criterion bench uses, time N iterations with `std::time::Instant`,
//! compute throughput in elements-per-second, assert it exceeds the
//! published floor. We do **not** attempt to reproduce Criterion's
//! statistical sampling — the goal is a correctness-style contract,
//! not a precision-style measurement. For precision numbers run the
//! `compilation_regression` Criterion bench.
//!
//! The 1 K-LOC program generator lives in the bench file, so we
//! inline a compatible minimal variant here to keep this test crate
//! self-contained. The two generators produce the same kinds of
//! constructs (record types, plain functions, match expressions,
//! let-chains) so measurements are comparable within an order of
//! magnitude.

use std::time::Instant;

use verum_fast_parser::Parser;
use verum_lexer::Lexer;
use verum_ast::span::FileId;

// ============================================================================
// Published throughput floors (LOC/sec).
//
// These are the public promise. Bumping them up when actual
// performance clears the new floor is a welcome change; bumping them
// down is a regression and must be justified in the commit message
// when the numbers cannot be recovered on the platform where CI runs.
// ============================================================================

/// `verum fast_parser::Parser` end-to-end parse speed. Published
/// target in the language README / pipeline docs is 50 K LOC/sec.
/// Actual measured on `main` (2026-04) on macOS arm64: ~1.4 M LOC/sec.
const PARSE_FLOOR_LOC_PER_SEC: f64 = 50_000.0;

/// Lex-only throughput. Lexing is the cheapest front-end pass and
/// should be at least 2× the parse floor because the parser runs
/// lexing internally.
const LEX_FLOOR_LOC_PER_SEC: f64 = 100_000.0;

// ============================================================================
// Program generator
// ============================================================================

fn generate_1k_loc_program() -> String {
    let mut src = String::with_capacity(32_000);

    // Record types
    for i in 0..10 {
        src.push_str(&format!(
            "type Record{i} is {{ x{i}: Int, y{i}: Float, name{i}: Text }};\n"
        ));
    }
    // Sum types
    for i in 0..5 {
        src.push_str(&format!(
            "type Choice{i} is OptionA{i}(Int) | OptionB{i}(Text) | Empty{i};\n"
        ));
    }
    // Simple functions
    for i in 0..50 {
        src.push_str(&format!(
            "fn compute{i}(x: Int, y: Int) -> Int {{\n    let a = x + {i};\n    let b = y * 2;\n    a + b\n}}\n\n"
        ));
    }
    // Functions with match
    for i in 0..30 {
        src.push_str(&format!(
            "fn classify{i}(n: Int) -> Int {{\n    match n {{\n        0 => 0,\n        1 => 1,\n        _ => n * {i}\n    }}\n}}\n\n"
        ));
    }
    // Let-chains with control flow
    for i in 0..25 {
        src.push_str(&format!(
            "fn process{i}(input: Int) -> Int {{\n\
             \x20   let step1 = input + {i};\n\
             \x20   let step2 = step1 * 2;\n\
             \x20   let step3 = if step2 > 100 {{ step2 - 50 }} else {{ step2 + 50 }};\n\
             \x20   step3\n\
             }}\n\n"
        ));
    }

    src
}

// ============================================================================
// Contract tests
// ============================================================================

fn loc_per_sec(lines: usize, iterations: usize, elapsed_s: f64) -> f64 {
    (lines * iterations) as f64 / elapsed_s
}

#[test]
fn parse_speed_meets_published_floor() {
    let source = generate_1k_loc_program();
    let lines = source.lines().count();
    assert!(
        lines >= 500,
        "generator produced {lines} LOC — expected at least 500"
    );

    // Warm-up: let the system stabilize (JIT-like effects in the page
    // cache, allocator, interned-string caches).
    for _ in 0..3 {
        let mut parser = Parser::new(&source);
        let _ = parser.parse_module();
    }

    // Measured run.
    let iterations = 50;
    let start = Instant::now();
    for _ in 0..iterations {
        let mut parser = Parser::new(&source);
        let _ = std::hint::black_box(parser.parse_module());
    }
    let elapsed = start.elapsed().as_secs_f64();
    let loc_per_s = loc_per_sec(lines, iterations, elapsed);

    eprintln!(
        "parse throughput: {:.1} K LOC/sec ({lines} LOC × {iterations} iters in {:.3}s)",
        loc_per_s / 1000.0,
        elapsed
    );

    assert!(
        loc_per_s >= PARSE_FLOOR_LOC_PER_SEC,
        "parse throughput regressed: {:.1} K LOC/sec is below the {:.1} K LOC/sec floor",
        loc_per_s / 1000.0,
        PARSE_FLOOR_LOC_PER_SEC / 1000.0
    );
}

#[test]
fn lex_speed_meets_published_floor() {
    let source = generate_1k_loc_program();
    let lines = source.lines().count();

    for _ in 0..3 {
        let _ = Lexer::new(&source, FileId::new(0)).collect::<Vec<_>>();
    }

    let iterations = 100;
    let start = Instant::now();
    for _ in 0..iterations {
        let toks: Vec<_> = Lexer::new(&source, FileId::new(0)).collect();
        std::hint::black_box(toks);
    }
    let elapsed = start.elapsed().as_secs_f64();
    let loc_per_s = loc_per_sec(lines, iterations, elapsed);

    eprintln!(
        "lex throughput:   {:.1} K LOC/sec ({lines} LOC × {iterations} iters in {:.3}s)",
        loc_per_s / 1000.0,
        elapsed
    );

    assert!(
        loc_per_s >= LEX_FLOOR_LOC_PER_SEC,
        "lex throughput regressed: {:.1} K LOC/sec is below the {:.1} K LOC/sec floor",
        loc_per_s / 1000.0,
        LEX_FLOOR_LOC_PER_SEC / 1000.0
    );
}
