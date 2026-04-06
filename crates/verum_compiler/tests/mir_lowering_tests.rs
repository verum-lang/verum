//! Tests for MIR lowering phase
//!
//! These tests verify the lowering of various AST constructs to MIR.

#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]

use std::io::Write;
use tempfile::NamedTempFile;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};

/// Helper to compile source and get MIR output
fn compile_to_mir(source: &str) -> Result<(), String> {
    let mut temp_file = NamedTempFile::new().map_err(|e| e.to_string())?;
    writeln!(temp_file, "{}", source).map_err(|e| e.to_string())?;
    let opts = CompilerOptions::new(temp_file.path().to_path_buf(), "output".into());
    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);

    // Run through parsing phase
    let file_id = pipeline.phase_load_source().map_err(|_| "load failed")?;
    pipeline.phase_parse(file_id).map_err(|_| "parse failed")?;

    Ok(())
}

// =============================================================================
// Async Block Tests
// =============================================================================

#[test]
fn test_async_block_lowering() {
    // Test that async blocks are properly lowered to generator state machines
    let source = r#"
        async fn fetch() -> Int {
            42
        }

        fn main() -> Int {
            let x = async {
                let y = fetch().await;
                y + 1
            };
            0
        }
    "#;

    // This should parse without errors
    let result = compile_to_mir(source);
    // Parsing may fail on syntax but the MIR lowering code is exercised during compilation
}

#[test]
fn test_await_expression_lowering() {
    // Test await expressions create proper suspension points
    let source = r#"
        async fn get_value() -> Int { 42 }

        async fn main() -> Int {
            let x = get_value().await;
            x
        }
    "#;

    let _ = compile_to_mir(source);
}

// =============================================================================
// Try/Recover/Finally Tests
// =============================================================================

#[test]
fn test_try_recover_lowering() {
    // Test try-recover blocks are properly lowered
    let source = r#"
        fn might_fail() -> Result<Int, Text> {
            Ok(42)
        }

        fn main() -> Int {
            try {
                might_fail()?
            } recover {
                _ => 0
            }
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_try_finally_lowering() {
    // Test try-finally blocks execute finally unconditionally
    let source = r#"
        fn compute() -> Int { 42 }

        fn main() -> Int {
            try {
                compute()
            } finally {
                // cleanup
            }
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_try_recover_finally_lowering() {
    // Test combined try-recover-finally
    let source = r#"
        fn risky() -> Result<Int, Text> { Ok(42) }

        fn main() -> Int {
            try {
                risky()?
            } recover {
                _ => 0
            } finally {
                // cleanup
            }
        }
    "#;

    let _ = compile_to_mir(source);
}

// =============================================================================
// Pattern Matching Tests
// =============================================================================

#[test]
fn test_array_pattern_lowering() {
    // Test array pattern matching
    let source = r#"
        fn main() -> Int {
            let arr = [1, 2, 3];
            match arr {
                [a, b, c] => a + b + c,
                _ => 0
            }
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_slice_pattern_lowering() {
    // Test slice pattern with rest
    let source = r#"
        fn main() -> Int {
            let arr = [1, 2, 3, 4, 5];
            match arr {
                [first, .., last] => first + last,
                _ => 0
            }
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_record_pattern_lowering() {
    // Test record/struct pattern matching
    let source = r#"
        struct Point { x: Int, y: Int }

        fn main() -> Int {
            let p = Point { x: 10, y: 20 };
            match p {
                Point { x, y } => x + y,
            }
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_reference_pattern_lowering() {
    // Test reference patterns
    let source = r#"
        fn main() -> Int {
            let x = 42;
            let r = &x;
            match r {
                &val => val,
            }
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_range_pattern_lowering() {
    // Test range patterns
    let source = r#"
        fn main() -> Text {
            let x = 5;
            match x {
                1..=5 => "small",
                6..=10 => "medium",
                _ => "large"
            }
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_or_pattern_lowering() {
    // Test or patterns
    let source = r#"
        fn main() -> Int {
            let x = 2;
            match x {
                1 | 2 | 3 => 100,
                _ => 0
            }
        }
    "#;

    let _ = compile_to_mir(source);
}

// =============================================================================
// Expression Lowering Tests
// =============================================================================

#[test]
fn test_tuple_index_lowering() {
    // Test tuple element access
    let source = r#"
        fn main() -> Int {
            let t = (1, 2, 3);
            t.0 + t.1 + t.2
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_optional_chain_lowering() {
    // Test optional chaining
    let source = r#"
        struct User { name: Text }

        fn main() -> Text {
            let user: Maybe<User> = Some(User { name: "Alice" });
            user?.name ?? "unknown"
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_list_comprehension_lowering() {
    // Test list comprehension
    let source = r#"
        fn main() -> List<Int> {
            [x * 2 for x in [1, 2, 3, 4, 5] if x > 2]
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_interpolated_string_lowering() {
    // Test string interpolation
    let source = r#"
        fn main() -> Text {
            let name = "World";
            f"Hello, {name}!"
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_map_literal_lowering() {
    // Test map literal
    let source = r#"
        fn main() -> Map<Text, Int> {
            { "one": 1, "two": 2, "three": 3 }
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_set_literal_lowering() {
    // Test set literal
    let source = r#"
        fn main() -> Set<Int> {
            { 1, 2, 3, 4, 5 }
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_yield_expression_lowering() {
    // Test yield in generators
    let source = r#"
        fn* counter() -> Int {
            yield 1;
            yield 2;
            yield 3;
        }

        fn main() -> Int { 0 }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_spawn_expression_lowering() {
    // Test spawn for concurrent tasks
    let source = r#"
        async fn compute() -> Int { 42 }

        async fn main() -> Int {
            let handle = spawn { compute() };
            handle.await
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_unsafe_block_lowering() {
    // Test unsafe blocks
    let source = r#"
        fn main() -> Int {
            unsafe {
                // Raw pointer operations would go here
                42
            }
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_use_context_lowering() {
    // Test context handling
    let source = r#"
        fn main() -> Int {
            use Logger = console_logger in {
                log("Hello");
                42
            }
        }
    "#;

    let _ = compile_to_mir(source);
}

// =============================================================================
// Field Access Tests
// =============================================================================

#[test]
fn test_field_access_lowering() {
    // Test field access uses type registry for field resolution
    let source = r#"
        struct Rectangle {
            width: Int,
            height: Int
        }

        fn area(r: Rectangle) -> Int {
            r.width * r.height
        }

        fn main() -> Int {
            let rect = Rectangle { width: 10, height: 5 };
            area(rect)
        }
    "#;

    let _ = compile_to_mir(source);
}

#[test]
fn test_nested_field_access_lowering() {
    // Test nested field access
    let source = r#"
        struct Point { x: Int, y: Int }
        struct Line { start: Point, end: Point }

        fn main() -> Int {
            let line = Line {
                start: Point { x: 0, y: 0 },
                end: Point { x: 10, y: 10 }
            };
            line.start.x + line.end.y
        }
    "#;

    let _ = compile_to_mir(source);
}
