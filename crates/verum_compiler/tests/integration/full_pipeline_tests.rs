//! Full Pipeline Integration Tests for Verum Compiler
//!
//! Comprehensive end-to-end testing of the entire compilation pipeline.
//! Tests coverage targets: 75% → 95%
//!
//! Test categories:
//! - Full compilation pipeline (lex → parse → resolve → typecheck → verify → codegen)
//! - Error propagation through pipeline stages
//! - Optimization passes verification
//! - Multi-module compilation
//! - Incremental compilation
//! - Cross-phase invariants

use verum_compiler::*;
use verum_lexer::Lexer;
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
use verum_verification::Verifier;
use verum_common::{List, Text};
use tempfile::TempDir;
use std::fs;
use std::path::Path;

// ============================================================================
// Test Utilities
// ============================================================================

/// Helper to create a temporary test environment
fn setup_test_env() -> TempDir {
    TempDir::new().expect("Failed to create temp dir")
}

/// Helper to write source file to temp directory
fn write_source_file(dir: &Path, filename: &str, content: &str) {
    fs::write(dir.join(filename), content).expect("Failed to write source file");
}

/// Helper to compile source code through full pipeline
fn compile_full_pipeline(source: &str) -> Result<CompilationResult, CompilerError> {
    let session = CompilationSession::new();
    session.compile_source(source)
}

/// Helper to run compiled program and capture output
fn run_compiled_program(result: &CompilationResult) -> Result<String, String> {
    // Execute the compiled code and capture output
    result.execute()
}

// ============================================================================
// Phase 1: Lexing Tests
// ============================================================================

#[test]
fn test_phase1_lexing_success() {
    let source = r#"
        fn main() {
            let x = 42;
            println!("Hello, World!");
        }
    "#;

    let session = CompilationSession::new();
    let result = session.run_phase_1_lexing(source);

    assert!(result.is_ok());
    let tokens = result.unwrap();
    assert!(!tokens.is_empty());
    assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::Fn)));
    assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::Ident(_))));
}

#[test]
fn test_phase1_lexing_with_unicode() {
    let source = r#"
        fn greet() {
            let message = "Привет мир";  // Russian
            let emoji = "🚀 🎉";
        }
    "#;

    let session = CompilationSession::new();
    let result = session.run_phase_1_lexing(source);

    assert!(result.is_ok());
    let tokens = result.unwrap();
    assert!(!tokens.is_empty());
}

#[test]
fn test_phase1_lexing_error_recovery() {
    let source = r#"
        fn main() {
            let x = @@@;  // Invalid token sequence
            let y = 42;
        }
    "#;

    let session = CompilationSession::new();
    let result = session.run_phase_1_lexing(source);

    // Should either succeed with error tokens or return structured errors
    match result {
        Ok(tokens) => {
            // Lexer may produce error tokens
            assert!(!tokens.is_empty());
        }
        Err(errors) => {
            // Should have error information
            assert!(!errors.is_empty());
        }
    }
}

// ============================================================================
// Phase 2: Parsing Tests
// ============================================================================

#[test]
fn test_phase2_parsing_complete_module() {
    let source = r#"
        fn add(x: Int, y: Int) -> Int {
            x + y
        }

        fn main() {
            let result = add(40, 2);
            println!("{}", result);
        }
    "#;

    let session = CompilationSession::new();
    let tokens = session.run_phase_1_lexing(source).unwrap();
    let result = session.run_phase_2_parsing(&tokens);

    assert!(result.is_ok());
    let ast = result.unwrap();
    assert_eq!(ast.items.len(), 2); // Two function declarations
}

#[test]
fn test_phase2_parsing_with_types() {
    let source = r#"
        struct Point {
            x: Float,
            y: Float,
        }

        fn distance(p1: Point, p2: Point) -> Float {
            let dx = p2.x - p1.x;
            let dy = p2.y - p1.y;
            sqrt(dx * dx + dy * dy)
        }
    "#;

    let session = CompilationSession::new();
    let tokens = session.run_phase_1_lexing(source).unwrap();
    let result = session.run_phase_2_parsing(&tokens);

    assert!(result.is_ok());
    let ast = result.unwrap();
    assert_eq!(ast.items.len(), 2); // Struct + function
}

#[test]
fn test_phase2_parsing_refinement_types() {
    let source = r#"
        type PositiveInt = { x: Int | x > 0 };

        fn factorial(n: PositiveInt) -> PositiveInt {
            if n == 1 {
                1
            } else {
                n * factorial(n - 1)
            }
        }
    "#;

    let session = CompilationSession::new();
    let tokens = session.run_phase_1_lexing(source).unwrap();
    let result = session.run_phase_2_parsing(&tokens);

    assert!(result.is_ok());
}

// ============================================================================
// Phase 3: Name Resolution Tests
// ============================================================================

#[test]
fn test_phase3_resolution_simple() {
    let source = r#"
        fn helper() -> Int { 42 }

        fn main() {
            let x = helper();
        }
    "#;

    let session = CompilationSession::new();
    let tokens = session.run_phase_1_lexing(source).unwrap();
    let ast = session.run_phase_2_parsing(&tokens).unwrap();
    let result = session.run_phase_3_resolution(&ast);

    assert!(result.is_ok());
}

#[test]
fn test_phase3_resolution_multi_module() {
    let temp_dir = setup_test_env();

    write_source_file(temp_dir.path(), "lib.vr", r#"
        pub fn add(x: Int, y: Int) -> Int {
            x + y
        }
    "#);

    write_source_file(temp_dir.path(), "main.vr", r#"
        import lib;

        fn main() {
            let result = lib::add(40, 2);
        }
    "#);

    let session = CompilationSession::with_source_dir(temp_dir.path());
    let result = session.compile_module("main.vr");

    assert!(result.is_ok());
}

#[test]
fn test_phase3_resolution_undefined_symbol() {
    let source = r#"
        fn main() {
            let x = undefined_function();
        }
    "#;

    let session = CompilationSession::new();
    let tokens = session.run_phase_1_lexing(source).unwrap();
    let ast = session.run_phase_2_parsing(&tokens).unwrap();
    let result = session.run_phase_3_resolution(&ast);

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.contains("undefined") || e.contains("not found")));
}

// ============================================================================
// Phase 4: Type Checking Tests
// ============================================================================

#[test]
fn test_phase4_typecheck_success() {
    let source = r#"
        fn add(x: Int, y: Int) -> Int {
            x + y
        }

        fn main() {
            let result: Int = add(40, 2);
        }
    "#;

    let session = CompilationSession::new();
    let tokens = session.run_phase_1_lexing(source).unwrap();
    let ast = session.run_phase_2_parsing(&tokens).unwrap();
    let resolved = session.run_phase_3_resolution(&ast).unwrap();
    let result = session.run_phase_4_type_checking(&resolved);

    assert!(result.is_ok());
}

#[test]
fn test_phase4_typecheck_type_mismatch() {
    let source = r#"
        fn add(x: Int, y: Int) -> Int {
            x + y
        }

        fn main() {
            let result: Text = add(40, 2);  // Type error
        }
    "#;

    let session = CompilationSession::new();
    let tokens = session.run_phase_1_lexing(source).unwrap();
    let ast = session.run_phase_2_parsing(&tokens).unwrap();
    let resolved = session.run_phase_3_resolution(&ast).unwrap();
    let result = session.run_phase_4_type_checking(&resolved);

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.contains("type") && (e.contains("mismatch") || e.contains("expected"))));
}

#[test]
fn test_phase4_typecheck_generics() {
    let source = r#"
        fn identity<T>(x: T) -> T {
            x
        }

        fn main() {
            let int_result: Int = identity(42);
            let str_result: Text = identity("hello");
        }
    "#;

    let session = CompilationSession::new();
    let tokens = session.run_phase_1_lexing(source).unwrap();
    let ast = session.run_phase_2_parsing(&tokens).unwrap();
    let resolved = session.run_phase_3_resolution(&ast).unwrap();
    let result = session.run_phase_4_type_checking(&resolved);

    assert!(result.is_ok());
}

// ============================================================================
// Phase 5: Refinement Verification Tests
// ============================================================================

#[test]
fn test_phase5_verification_refinement_satisfied() {
    let source = r#"
        type Positive = { x: Int | x > 0 };

        fn increment(n: Positive) -> Positive {
            n + 1
        }
    "#;

    let session = CompilationSession::new();
    let result = compile_full_pipeline(source);

    assert!(result.is_ok());
}

#[test]
fn test_phase5_verification_refinement_violated() {
    let source = r#"
        type Positive = { x: Int | x > 0 };

        fn decrement(n: Positive) -> Positive {
            n - 1  // May violate refinement
        }
    "#;

    let session = CompilationSession::new();
    let result = compile_full_pipeline(source);

    // Should either fail verification or produce warning
    match result {
        Err(e) => {
            assert!(e.to_string().contains("refinement") || e.to_string().contains("constraint"));
        }
        Ok(res) => {
            // May succeed with warnings
            assert!(res.has_warnings());
        }
    }
}

#[test]
fn test_phase5_verification_array_bounds() {
    let source = r#"
        fn safe_index(arr: List<Int>, idx: { i: Int | i >= 0 && i < arr.len() }) -> Int {
            arr[idx]
        }
    "#;

    let session = CompilationSession::new();
    let result = compile_full_pipeline(source);

    assert!(result.is_ok());
}

// ============================================================================
// Phase 6: Optimization Tests
// ============================================================================

#[test]
fn test_phase6_optimization_constant_folding() {
    let source = r#"
        fn main() {
            let x = 2 + 3 * 4;  // Should be folded to 14
        }
    "#;

    let session = CompilationSession::with_optimization_level(2);
    let result = compile_full_pipeline(source);

    assert!(result.is_ok());
    let optimized = result.unwrap();

    // Check that constant was folded
    let ir = optimized.get_ir();
    assert!(ir.contains("14") || !ir.contains("mul"));
}

#[test]
fn test_phase6_optimization_dead_code_elimination() {
    let source = r#"
        fn main() {
            let x = 42;
            let y = 100;  // Dead code - never used
            println!("{}", x);
        }
    "#;

    let session = CompilationSession::with_optimization_level(2);
    let result = compile_full_pipeline(source);

    assert!(result.is_ok());
    let optimized = result.unwrap();

    // Dead code should be eliminated
    let ir = optimized.get_ir();
    // Exact check depends on IR format
    assert!(!ir.contains("unused") || optimized.stats().dead_code_eliminated > 0);
}

#[test]
fn test_phase6_optimization_inlining() {
    let source = r#"
        fn add_one(x: Int) -> Int {
            x + 1
        }

        fn main() {
            let result = add_one(41);
        }
    "#;

    let session = CompilationSession::with_optimization_level(3);
    let result = compile_full_pipeline(source);

    assert!(result.is_ok());
    let optimized = result.unwrap();

    // Small function should be inlined
    assert!(optimized.stats().functions_inlined > 0);
}

// ============================================================================
// Phase 7: Code Generation Tests
// ============================================================================

#[test]
fn test_phase7_codegen_simple_function() {
    let source = r#"
        fn add(x: Int, y: Int) -> Int {
            x + y
        }
    "#;

    let session = CompilationSession::new();
    let result = compile_full_pipeline(source);

    assert!(result.is_ok());
    let compiled = result.unwrap();

    // Should generate valid LLVM IR
    let ir = compiled.get_llvm_ir();
    assert!(ir.contains("define") || ir.contains("add"));
}

#[test]
fn test_phase7_codegen_cbgr_references() {
    let source = r#"
        fn use_reference(x: &Int) -> Int {
            *x
        }

        fn main() {
            let value = 42;
            let result = use_reference(&value);
        }
    "#;

    let session = CompilationSession::new();
    let result = compile_full_pipeline(source);

    assert!(result.is_ok());
    let compiled = result.unwrap();

    // Should generate CBGR checks
    let ir = compiled.get_llvm_ir();
    assert!(ir.contains("cbgr") || compiled.stats().cbgr_checks_inserted > 0);
}

#[test]
fn test_phase7_codegen_async_functions() {
    let source = r#"
        async fn fetch_data() -> Text {
            await http::get("https://example.com")
        }

        async fn main() {
            let data = await fetch_data();
        }
    "#;

    let session = CompilationSession::new();
    let result = compile_full_pipeline(source);

    assert!(result.is_ok());
}

// ============================================================================
// Full E2E Tests
// ============================================================================

#[test]
fn test_e2e_hello_world() {
    let source = r#"
        fn main() {
            println!("Hello, World!");
        }
    "#;

    let result = compile_full_pipeline(source);
    assert!(result.is_ok());

    let compiled = result.unwrap();
    let output = run_compiled_program(&compiled).unwrap();

    assert_eq!(output.trim(), "Hello, World!");
}

#[test]
fn test_e2e_fibonacci() {
    let source = r#"
        fn fibonacci(n: Int) -> Int {
            if n <= 1 {
                n
            } else {
                fibonacci(n - 1) + fibonacci(n - 2)
            }
        }

        fn main() {
            let result = fibonacci(10);
            println!("{}", result);
        }
    "#;

    let result = compile_full_pipeline(source);
    assert!(result.is_ok());

    let compiled = result.unwrap();
    let output = run_compiled_program(&compiled).unwrap();

    assert_eq!(output.trim(), "55");
}

#[test]
fn test_e2e_with_context_system() {
    let source = r#"
        context Logger {
            fn log(msg: Text);
        }

        using [Logger]
        fn process() {
            Logger::log("Processing...");
        }

        fn main() {
            provide Logger with ConsoleLogger {
                process();
            }
        }
    "#;

    let result = compile_full_pipeline(source);
    assert!(result.is_ok());
}

#[test]
fn test_e2e_with_refinement_types() {
    let source = r#"
        type NonEmpty<T> = { list: List<T> | list.len() > 0 };

        fn first<T>(list: NonEmpty<T>) -> T {
            list[0]
        }

        fn main() {
            let numbers = [1, 2, 3];
            let result = first(numbers);
            println!("{}", result);
        }
    "#;

    let result = compile_full_pipeline(source);
    assert!(result.is_ok());

    let compiled = result.unwrap();
    let output = run_compiled_program(&compiled).unwrap();

    assert_eq!(output.trim(), "1");
}

// ============================================================================
// Error Propagation Tests
// ============================================================================

#[test]
fn test_error_propagation_lex_to_parse() {
    let source = r#"fn @@@() {}"#;

    let result = compile_full_pipeline(source);
    assert!(result.is_err());

    let error = result.unwrap_err();
    assert!(error.phase() == CompilationPhase::Lexing ||
            error.phase() == CompilationPhase::Parsing);
}

#[test]
fn test_error_propagation_parse_to_resolve() {
    let source = r#"
        fn main() {
            let x = undefined_func();
        }
    "#;

    let result = compile_full_pipeline(source);
    assert!(result.is_err());

    let error = result.unwrap_err();
    assert!(error.phase() == CompilationPhase::Resolution);
}

#[test]
fn test_error_propagation_typecheck_to_verify() {
    let source = r#"
        type Positive = { x: Int | x > 0 };

        fn bad_function() -> Positive {
            -1  // Type error: negative value for Positive
        }
    "#;

    let result = compile_full_pipeline(source);
    assert!(result.is_err());

    let error = result.unwrap_err();
    assert!(error.phase() == CompilationPhase::TypeChecking ||
            error.phase() == CompilationPhase::Verification);
}

// ============================================================================
// Multi-Module Compilation Tests
// ============================================================================

#[test]
fn test_multi_module_compilation() {
    let temp_dir = setup_test_env();

    write_source_file(temp_dir.path(), "math.vr", r#"
        pub fn add(x: Int, y: Int) -> Int {
            x + y
        }

        pub fn multiply(x: Int, y: Int) -> Int {
            x * y
        }
    "#);

    write_source_file(temp_dir.path(), "utils.vr", r#"
        import math;

        pub fn square(x: Int) -> Int {
            math::multiply(x, x)
        }
    "#);

    write_source_file(temp_dir.path(), "main.vr", r#"
        import math;
        import utils;

        fn main() {
            let sum = math::add(10, 20);
            let sq = utils::square(5);
            println!("{} {}", sum, sq);
        }
    "#);

    let session = CompilationSession::with_source_dir(temp_dir.path());
    let result = session.compile_all_modules();

    assert!(result.is_ok());
    let compiled = result.unwrap();

    let output = run_compiled_program(&compiled).unwrap();
    assert_eq!(output.trim(), "30 25");
}

// ============================================================================
// Incremental Compilation Tests
// ============================================================================

#[test]
fn test_incremental_compilation_no_changes() {
    let source = r#"
        fn main() {
            println!("Hello!");
        }
    "#;

    let session = CompilationSession::new();

    // First compilation
    let result1 = session.compile_source(source);
    assert!(result1.is_ok());

    // Second compilation (no changes)
    let result2 = session.compile_source(source);
    assert!(result2.is_ok());

    // Should use cached results
    assert!(result2.unwrap().from_cache());
}

#[test]
fn test_incremental_compilation_with_changes() {
    let temp_dir = setup_test_env();

    write_source_file(temp_dir.path(), "lib.vr", r#"
        pub fn version() -> Int { 1 }
    "#);

    write_source_file(temp_dir.path(), "main.vr", r#"
        import lib;

        fn main() {
            println!("{}", lib::version());
        }
    "#);

    let session = CompilationSession::with_source_dir(temp_dir.path());

    // First compilation
    let result1 = session.compile_all_modules();
    assert!(result1.is_ok());

    // Modify lib.vr
    write_source_file(temp_dir.path(), "lib.vr", r#"
        pub fn version() -> Int { 2 }
    "#);

    // Second compilation
    let result2 = session.compile_all_modules();
    assert!(result2.is_ok());

    // Should recompile affected modules
    assert!(!result2.unwrap().from_cache());
}

// ============================================================================
// Performance Tests
// ============================================================================

#[test]
fn test_compile_large_module() {
    // Generate a large module with 1000 functions
    let mut source = String::new();
    for i in 0..1000 {
        source.push_str(&format!(r#"
            fn func_{}() -> Int {{
                {}
            }}
        "#, i, i));
    }

    let start = std::time::Instant::now();
    let result = compile_full_pipeline(&source);
    let duration = start.elapsed();

    assert!(result.is_ok());
    // Should compile in reasonable time (< 10 seconds for 1000 functions)
    assert!(duration.as_secs() < 10);
}

#[test]
fn test_parallel_compilation() {
    let temp_dir = setup_test_env();

    // Create 100 independent modules
    for i in 0..100 {
        write_source_file(temp_dir.path(), &format!("module_{}.vr", i), &format!(r#"
            pub fn func_{}() -> Int {{
                {}
            }}
        "#, i, i));
    }

    let session = CompilationSession::with_source_dir(temp_dir.path())
        .with_parallel_compilation(true);

    let start = std::time::Instant::now();
    let result = session.compile_all_modules();
    let duration = start.elapsed();

    assert!(result.is_ok());

    // Parallel compilation should be faster than sequential
    // (This is a heuristic test - actual speedup depends on hardware)
    println!("Parallel compilation took: {:?}", duration);
}
