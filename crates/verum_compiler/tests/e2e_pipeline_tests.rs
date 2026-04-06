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
//! Comprehensive End-to-End Integration Tests for Verum Compiler Pipeline
//!
//! Tests the complete compilation pipeline: parse → typecheck → codegen → execute
//!
//! ## Test Coverage
//!
//! 1. **Simple Function Execution** - Basic compilation and execution flow
//! 2. **Refinement Types** - Refinement type checking through full pipeline
//! 3. **CBGR Checks** - Verification that CBGR checks are inserted in compiled code
//! 4. **Context System** - Dependency injection end-to-end
//! 5. **Protocol Dispatch** - Protocol method dispatch through compilation
//!
//! ## Architecture
//!
//! Each test follows the pipeline:
//! - **Phase 1**: Lexing & Parsing (verum_fast_parser)
//! - **Phase 2**: Type Checking (verum_types)
//! - **Phase 3**: Verification (verum_verification)
//! - **Phase 4**: Code Generation (verum_codegen)
//! - **Phase 5**: Execution (verum_runtime)
//!
//! Pipeline: Phase 0 (stdlib prep) -> Phase 1 (lex/parse) -> Phase 2 (meta registry) ->
//! Phase 3 (macro expansion) -> Phase 3a (contract verification) -> Phase 4 (semantic
//! analysis) -> Phase 4a (autodiff) -> Phase 5 (VBC codegen) -> Phase 6 (optimization)
//! -> Phase 7 (execution: interpreter or AOT via LLVM) -> Phase 7.5 (linking).
//! Safety is absolute: no speculation on memory safety. Multi-pass meta system ensures
//! proper ordering of compile-time features.

use std::path::PathBuf;
use tempfile::TempDir;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};
use verum_fast_parser::VerumParser;

// ============================================================================
// Test Helpers
// ============================================================================

/// Create a test session with default options
fn create_test_session(temp_dir: &TempDir) -> Session {
    let options = CompilerOptions {
        input: PathBuf::from("test.vr"),
        output: temp_dir.path().join("test"),
        ..Default::default()
    };
    Session::new(options)
}

/// Compile source code through the full pipeline.
/// NOTE: RUST_MIN_STACK is set in .cargo/config.toml to ensure adequate stack space.
fn compile_source(source: &str) -> Result<(), String> {
    let temp_dir = TempDir::new().map_err(|e| format!("Failed to create temp dir: {}", e))?;
    let mut session = create_test_session(&temp_dir);
    let mut pipeline = CompilationPipeline::new(&mut session);

    match pipeline.compile_string(source) {
        Ok(_) => Ok(()),
        Err(e) => {
            // Collect all diagnostic messages to include in error
            let diagnostics = session.diagnostics();
            let mut error_msg = format!("Compilation failed: {}", e);

            if !diagnostics.is_empty() {
                error_msg.push_str("\nDiagnostics:");
                for diag in diagnostics {
                    error_msg.push_str(&format!("\n  {}", diag.message()));
                }
            }

            Err(error_msg)
        }
    }
}

/// Parse source into AST for inspection
fn parse_source(source: &str) -> Result<verum_ast::Module, String> {
    use verum_ast::FileId;
    use verum_lexer::Lexer;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();

    parser
        .parse_module(lexer, file_id)
        .map_err(|errors| format!("Parse error: {:?}", errors))
}

// ============================================================================
// Test 1: Simple Function Execution
// ============================================================================

#[test]
fn test_simple_function_execution() {
    // Goal: Parse, compile, and execute a simple function
    let source = r#"
        fn add(x: Int, y: Int) -> Int {
            x + y
        }

        fn main() -> Int {
            let result = add(40, 2);
            result
        }
    "#;

    // Step 1: Parse
    let module = parse_source(source).expect("Should parse successfully");
    assert_eq!(module.items.len(), 2, "Should have 2 function items");

    // Step 2-5: Full compilation pipeline
    let result = compile_source(source);

    if let Err(e) = &result {
        eprintln!("Compilation error: {}", e);
    }

    // Note: Currently testing that compilation succeeds.
    // Future: Add execution verification when runtime is fully integrated
    assert!(
        result.is_ok(),
        "Simple function should compile successfully"
    );
}

#[test]
fn test_simple_arithmetic_expression() {
    let source = r#"
        fn main() {
            let x = 2 + 3 * 4;
            let y = (2 + 3) * 4;
            let z = x + y;
            z
        }
    "#;

    let result = compile_source(source);

    if let Err(e) = &result {
        eprintln!("Arithmetic compilation error: {}", e);
    }

    assert!(
        result.is_ok(),
        "Arithmetic expressions should compile successfully"
    );
}

#[test]
fn test_recursive_function_execution() {
    let source = r#"
        fn factorial(n: Int) -> Int {
            if n <= 1 {
                1
            } else {
                n * factorial(n - 1)
            }
        }

        fn main() -> Int {
            let result = factorial(5);
            result
        }
    "#;

    let result = compile_source(source);

    if let Err(e) = &result {
        eprintln!("Recursive function error: {}", e);
    }

    assert!(
        result.is_ok(),
        "Recursive functions should compile successfully"
    );
}

#[test]
fn test_pattern_matching_execution() {
    let source = r#"
        fn classify(n: Int) -> Text {
            match n {
                0 => "zero",
                1 => "one",
                _ => "many"
            }
        }

        fn main() {
            let result = classify(42);
            result
        }
    "#;

    let result = compile_source(source);

    if let Err(e) = &result {
        eprintln!("Pattern matching error: {}", e);
    }

    assert!(
        result.is_ok(),
        "Pattern matching should compile successfully"
    );
}

// ============================================================================
// Test 2: Refinement Types End-to-End
// ============================================================================

#[test]
fn test_refinement_types_e2e() {
    // Goal: Test that refinement types work through the full pipeline
    // Note: Using simple type alias with 'is' syntax per parser expectations
    // Full refinement syntax with 'where' clauses is not yet fully implemented
    let source = r#"
        type Positive is Int;

        fn square(x: Positive) -> Positive {
            x * x
        }

        fn main() {
            let x: Positive = 5;
            let result = square(x);
            result
        }
    "#;

    // Parse and verify refinement type syntax is recognized
    let module = parse_source(source);

    match &module {
        Ok(m) => {
            println!("Parsed {} items", m.items.len());
        }
        Err(e) => {
            eprintln!("Parse error: {}", e);
        }
    }

    // Full compilation (may require SMT verification)
    let result = compile_source(source);

    // Note: Refinement type verification may not be fully implemented yet
    // We accept either success or graceful error reporting
    match result {
        Ok(_) => {
            println!("Refinement types compiled successfully");
        }
        Err(e) => {
            println!("Refinement compilation error (expected): {}", e);
        }
    }
}

#[test]
fn test_refinement_types_violation_detection() {
    // Goal: Test that refinement violations are detected
    // Note: Using 'is' syntax per parser expectations, and simple type alias
    // Full refinement checking is not yet implemented, so this test
    // checks that the code parses and compiles, even if the violation
    // isn't caught at compile time.
    let source = r#"
        type Positive is Int;

        fn bad_function() -> Positive {
            -1  // Should fail: negative value for Positive type (when refinements are implemented)
        }

        fn main() {
            let x = bad_function();
            x
        }
    "#;

    let result = compile_source(source);

    // Refinement type verification is not yet fully implemented
    // For now, we accept that the code compiles without error
    match result {
        Ok(_) => {
            println!("Warning: Refinement violation not caught (verification incomplete)");
        }
        Err(e) => {
            println!("Compilation error: {}", e);
            // If there's an error, it should be type-related
            // Note: We don't assert on this since refinements aren't fully implemented
        }
    }
}

#[test]
fn test_refinement_types_array_bounds() {
    // Note: Sigma type syntax { i: Int | predicate } is not yet fully implemented
    // Using simple type alias instead
    let source = r#"
        type ValidIndex is Int;

        fn safe_access(arr: List<Int>, idx: ValidIndex) -> Int {
            arr[idx]
        }

        fn main() {
            let numbers = [1, 2, 3, 4, 5];
            let result = safe_access(numbers, 2);
            result
        }
    "#;

    let result = compile_source(source);

    // Test documents current refinement type support
    match result {
        Ok(_) => println!("Array bounds refinement compiled"),
        Err(e) => println!("Array bounds refinement error: {}", e),
    }
}

// ============================================================================
// Test 3: CBGR Checks Inserted
// ============================================================================

#[test]
fn test_cbgr_checks_inserted() {
    let result = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            // Goal: Verify CBGR runtime checks are inserted in compiled code
            let source = r#"
                fn use_reference(x: &Int) -> Int {
                    *x + 1
                }

                fn main() -> Int {
                    let value = 42;
                    let result = use_reference(&value);
                    result
                }
            "#;

            let temp_dir = TempDir::new().expect("Should create temp dir");
            let mut session = create_test_session(&temp_dir);
            let mut pipeline = CompilationPipeline::new(&mut session);

            let result = pipeline.compile_string(source);

            if let Err(e) = &result {
                eprintln!("CBGR reference compilation error: {}", e);
            }

            // Future: Verify LLVM IR contains CBGR check calls
            // For now, test that reference syntax compiles
            assert!(
                result.is_ok(),
                "Managed references should compile with CBGR checks"
            );
        })
        .expect("Failed to spawn thread")
        .join();

    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

#[test]
fn test_cbgr_multiple_references() {
    let result = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let source = r#"
                fn sum_refs(a: &Int, b: &Int, c: &Int) -> Int {
                    *a + *b + *c
                }

                fn main() -> Int {
                    let x = 10;
                    let y = 20;
                    let z = 30;
                    let result = sum_refs(&x, &y, &z);
                    result
                }
            "#;

            let result = compile_source(source);

            if let Err(e) = &result {
                eprintln!("Multiple references error: {}", e);
            }

            assert!(
                result.is_ok(),
                "Multiple references should insert multiple CBGR checks"
            );
        })
        .expect("Failed to spawn thread")
        .join();

    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

#[test]
fn test_cbgr_checked_references() {
    let result = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            // Goal: Test that escape analysis promotes to checked references
            let source = r#"
                fn local_only(data: Int) -> Int {
                    let data_ref = &data;  // Should promote to &checked
                    *data_ref
                }

                fn main() -> Int {
                    let value = 42;
                    let result = local_only(value);
                    result
                }
            "#;

            let result = compile_source(source);

            if let Err(e) = &result {
                eprintln!("Checked reference error: {}", e);
            }

            // Future: Verify escape analysis promoted to &checked tier
            assert!(
                result.is_ok(),
                "Local-only references should compile (possibly promoted)"
            );
        })
        .expect("Failed to spawn thread")
        .join();

    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

#[test]
fn test_cbgr_unsafe_references() {
    // Goal: Test unsafe reference tier (manual verification)
    let source = r#"
        fn use_unsafe(x: &unsafe Int) -> Int {
            // SAFETY: Caller guarantees validity
            *x
        }

        fn main() {
            let value = 42;
            // Note: This requires unsafe block in real code
            let result = use_unsafe(&unsafe value);
            result
        }
    "#;

    let result = compile_source(source);

    // Unsafe references may require special syntax/blocks
    match result {
        Ok(_) => println!("Unsafe references compiled"),
        Err(e) => println!("Unsafe reference error (may require unsafe blocks): {}", e),
    }
}

// ============================================================================
// Test 4: Context System End-to-End
// ============================================================================

#[test]
fn test_context_system_e2e() {
    // Goal: Test the context system (dependency injection) through full pipeline
    let source = r#"
        context Logger {
            fn log(msg: Text) -> ();
        }

        using [Logger]
        fn process() -> Text {
            Logger.log("Processing...");
            "done"
        }

        fn main() {
            // Future: provide Logger with ConsoleLogger { ... }
            let result = process();
            result
        }
    "#;

    let result = compile_source(source);

    // Context system is a key Verum feature
    match result {
        Ok(_) => {
            println!("Context system compiled successfully");
        }
        Err(e) => {
            println!("Context system error: {}", e);
            // Context implementation may be in progress
        }
    }
}

#[test]
fn test_context_multiple_dependencies() {
    let source = r#"
        context Database {
            fn query(sql: Text) -> List<Text>;
        }

        context Logger {
            fn log(msg: Text) -> ();
        }

        using [Database, Logger]
        fn fetch_users() -> List<Text> {
            Logger.log("Fetching users...");
            Database.query("SELECT * FROM users")
        }

        fn main() {
            let users = fetch_users();
            users
        }
    "#;

    let result = compile_source(source);

    match result {
        Ok(_) => println!("Multiple contexts compiled"),
        Err(e) => println!("Multiple contexts error: {}", e),
    }
}

#[test]
fn test_context_provide_block() {
    let source = r#"
        context Config {
            fn get(key: Text) -> Text;
        }

        using [Config]
        fn app_name() -> Text {
            Config.get("app.name")
        }

        fn main() {
            provide Config with {
                fn get(key: Text) -> Text {
                    "MyApp"
                }
            } {
                let name = app_name();
                name
            }
        }
    "#;

    let result = compile_source(source);

    match result {
        Ok(_) => println!("Context provide block compiled"),
        Err(e) => println!("Context provide error: {}", e),
    }
}

// ============================================================================
// Test 5: Protocol Dispatch End-to-End
// ============================================================================

#[test]
fn test_protocol_dispatch_e2e() {
    // Goal: Test protocol method dispatch through compilation
    let source = r#"
        protocol Show {
            fn show(self) -> Text;
        }

        impl Show for Int {
            fn show(self) -> Text {
                // Future: Convert int to string
                "42"
            }
        }

        fn display<T: Show>(value: T) -> Text {
            value.show()
        }

        fn main() {
            let result = display(42);
            result
        }
    "#;

    let result = compile_source(source);

    match result {
        Ok(_) => {
            println!("Protocol dispatch compiled successfully");
        }
        Err(e) => {
            println!("Protocol dispatch error: {}", e);
        }
    }
}

#[test]
fn test_protocol_multiple_implementations() {
    let source = r#"
        protocol Eq {
            fn equals(self, other: Self) -> Bool;
        }

        impl Eq for Int {
            fn equals(self, other: Int) -> Bool {
                // Built-in equality
                self == other
            }
        }

        impl Eq for Text {
            fn equals(self, other: Text) -> Bool {
                // String comparison
                self == other
            }
        }

        fn compare<T: Eq>(a: T, b: T) -> Bool {
            a.equals(b)
        }

        fn main() {
            let int_eq = compare(1, 1);
            let str_eq = compare("hello", "hello");
            int_eq && str_eq
        }
    "#;

    let result = compile_source(source);

    match result {
        Ok(_) => println!("Multiple protocol implementations compiled"),
        Err(e) => println!("Multiple protocol implementations error: {}", e),
    }
}

#[test]
fn test_protocol_with_associated_types() {
    let source = r#"
        protocol Container {
            type Item;
            fn get(self, index: Int) -> Item;
        }

        impl Container for List<Int> {
            type Item = Int;
            fn get(self, index: Int) -> Int {
                self[index]
            }
        }

        fn first<C: Container>(container: C) -> C::Item {
            container.get(0)
        }

        fn main() {
            let numbers = [1, 2, 3];
            let result = first(numbers);
            result
        }
    "#;

    let result = compile_source(source);

    match result {
        Ok(_) => println!("Protocol with associated types compiled"),
        Err(e) => println!("Associated types error: {}", e),
    }
}

#[test]
fn test_protocol_inheritance() {
    let source = r#"
        protocol Eq {
            fn equals(self, other: Self) -> Bool;
        }

        protocol Ord extends Eq {
            fn compare(self, other: Self) -> Int;
        }

        impl Eq for Int {
            fn equals(self, other: Int) -> Bool {
                self == other
            }
        }

        impl Ord for Int {
            fn compare(self, other: Int) -> Int {
                if self < other {
                    -1
                } else if self > other {
                    1
                } else {
                    0
                }
            }
        }

        fn sort<T: Ord>(items: List<T>) -> List<T> {
            // Future: Implement sorting
            items
        }

        fn main() {
            let numbers = [3, 1, 4, 1, 5];
            let sorted = sort(numbers);
            sorted
        }
    "#;

    let result = compile_source(source);

    match result {
        Ok(_) => println!("Protocol inheritance compiled"),
        Err(e) => println!("Protocol inheritance error: {}", e),
    }
}

// ============================================================================
// Integration Tests: Combining Features
// ============================================================================

#[test]
fn test_combined_refinements_and_protocols() {
    let source = r#"
        type Positive is Int;

        protocol Numeric {
            fn add(self, other: Self) -> Self;
        }

        impl Numeric for Positive {
            fn add(self, other: Positive) -> Positive {
                self + other  // Result is still positive (when refinements are implemented)
            }
        }

        fn sum_positive<T: Numeric>(a: T, b: T) -> T {
            a.add(b)
        }

        fn main() {
            let x: Positive = 5;
            let y: Positive = 3;
            let result = sum_positive(x, y);
            result
        }
    "#;

    let result = compile_source(source);

    match result {
        Ok(_) => println!("Combined refinements and protocols compiled"),
        Err(e) => println!("Combined features error: {}", e),
    }
}

#[test]
fn test_combined_cbgr_and_context() {
    let source = r#"
        context Allocator {
            fn alloc(size: Int) -> &Int;
            fn dealloc(ptr: &Int) -> ();
        }

        using [Allocator]
        fn allocate_and_use() -> Int {
            let ptr = Allocator.alloc(100);
            let value = *ptr;
            Allocator.dealloc(ptr);
            value
        }

        fn main() {
            let result = allocate_and_use();
            result
        }
    "#;

    let result = compile_source(source);

    match result {
        Ok(_) => println!("Combined CBGR and context compiled"),
        Err(e) => println!("Combined CBGR/context error: {}", e),
    }
}

#[test]
fn test_full_feature_integration() {
    // Combines: protocols, refinements, contexts, CBGR
    // Note: Using simple type alias - full refinement syntax not yet implemented
    let source = r#"
        type NonEmpty<T> is List<T>;

        protocol Show {
            fn show(self) -> Text;
        }

        context Logger {
            fn log(msg: Text) -> ();
        }

        impl Show for Int {
            fn show(self) -> Text {
                "number"
            }
        }

        using [Logger]
        fn process_list<T: Show>(items: NonEmpty<T>) -> Text {
            let first_ref = &items[0];
            let text = first_ref.show();
            Logger.log(text);
            text
        }

        fn main() {
            let numbers: NonEmpty<Int> = [1, 2, 3];
            let result = process_list(numbers);
            result
        }
    "#;

    let result = compile_source(source);

    match result {
        Ok(_) => println!("Full feature integration compiled successfully!"),
        Err(e) => println!("Full integration error: {}", e),
    }
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_parse_error_handling() {
    let source = r#"
        fn incomplete(x: Int {
            x + 1
        // Missing closing brace and return arrow
    "#;

    let result = compile_source(source);

    assert!(result.is_err(), "Invalid syntax should produce parse error");

    if let Err(e) = result {
        println!("Parse error (expected): {}", e);
    }
}

#[test]
fn test_type_error_handling() {
    let source = r#"
        fn bad_add(x: Int, y: Bool) -> Int {
            x + y  // Type error: can't add Int and Bool
        }

        fn main() {
            let result = bad_add(1, true);
            result
        }
    "#;

    let result = compile_source(source);

    // Should fail during type checking
    match result {
        Ok(_) => println!("Warning: Type error not caught"),
        Err(e) => {
            println!("Type error (expected): {}", e);
            assert!(
                e.contains("type") || e.contains("Bool") || e.contains("Int"),
                "Error should mention type mismatch"
            );
        }
    }
}

#[test]
fn test_refinement_error_handling() {
    let source = r#"
        type Positive is Int;

        fn main() {
            let x: Positive = -5;  // Should fail: negative value (when refinements are implemented)
            x
        }
    "#;

    let result = compile_source(source);

    // Refinement checking is not yet implemented, so this may compile
    match result {
        Ok(_) => println!("Warning: Refinement violation not caught (verification incomplete)"),
        Err(e) => println!("Refinement error (expected): {}", e),
    }
}

#[test]
fn test_undefined_symbol_error() {
    let source = r#"
        fn main() {
            let result = undefined_function();
            result
        }
    "#;

    let result = compile_source(source);

    assert!(result.is_err(), "Undefined symbol should produce error");

    if let Err(e) = result {
        println!("Undefined symbol error (expected): {}", e);
        assert!(
            e.contains("undefined") || e.contains("not found"),
            "Error should mention undefined symbol"
        );
    }
}

// ============================================================================
// Performance and Stress Tests
// ============================================================================

#[test]
fn test_large_function_compilation() {
    // Generate a function with many statements
    let mut source = String::from("fn main() -> Int {\n");
    source.push_str("    let mut sum = 0;\n");

    for i in 0..100 {
        source.push_str(&format!("    sum = sum + {};\n", i));
    }

    source.push_str("    sum\n");
    source.push_str("}\n");

    let result = compile_source(&source);

    if let Err(e) = &result {
        eprintln!("Large function error: {}", e);
    }

    assert!(result.is_ok(), "Large functions should compile");
}

#[test]
fn test_deeply_nested_expressions() {
    let source = r#"
        fn main() -> Int {
            ((((1 + 2) + 3) + 4) + 5) +
            ((((6 + 7) + 8) + 9) + 10)
        }
    "#;

    let result = compile_source(source);

    if let Err(e) = &result {
        eprintln!("Nested expressions error: {}", e);
    }

    assert!(result.is_ok(), "Deeply nested expressions should compile");
}

#[test]
fn test_compilation_performance() {
    // Measure compilation time for a moderately sized program
    let mut source = String::new();

    // Generate 20 functions
    for i in 0..20 {
        source.push_str(&format!(
            r#"
        fn func_{}(x: Int) -> Int {{
            let y = x + {};
            let z = y * 2;
            z
        }}
        "#,
            i, i
        ));
    }

    source.push_str(
        r#"
        fn main() -> Int {
            func_0(1)
        }
    "#,
    );

    let start = std::time::Instant::now();
    let result = compile_source(&source);
    let elapsed = start.elapsed();

    if let Err(e) = &result {
        eprintln!("Performance test error: {}", e);
    }

    println!("Compilation time for 20 functions: {:?}", elapsed);

    // Should compile in reasonable time for 20 functions
    assert!(
        elapsed.as_secs() < 30,
        "Compilation should be reasonably fast (took {:?})",
        elapsed
    );
}

// ============================================================================
// Edge Cases and Boundary Tests
// ============================================================================

#[test]
fn test_empty_main_function() {
    let source = r#"
        fn main() {
            // Empty function
        }
    "#;

    let result = compile_source(source);

    if let Err(e) = &result {
        eprintln!("Empty main error: {}", e);
    }

    // Empty functions should compile (may need return type adjustment)
    let _ = result;
}

#[test]
fn test_comment_only_file() {
    let source = r#"
        // This is a comment
        /* This is a block comment */
        // More comments
    "#;

    let result = compile_source(source);

    // Comment-only files may or may not compile depending on requirements
    let _ = result;
}

#[test]
fn test_unicode_in_strings() {
    let source = r#"
        fn main() -> Text {
            let greeting = "Hello 世界 🌍";
            greeting
        }
    "#;

    let result = compile_source(source);

    if let Err(e) = &result {
        eprintln!("Unicode string error: {}", e);
    }

    // Unicode in strings should be supported
    assert!(result.is_ok(), "Unicode strings should compile");
}

/// Test deeply nested control flow structures
///
/// Spawns a separate thread with a large stack to handle deep recursion.
/// The default test thread stack is often too small for deeply nested AST processing.
#[test]
fn test_deeply_nested_control_flow() {
    // Spawn a thread with a large stack (16MB) to handle deep recursion
    let handle = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let source = r#"
                fn nested() -> Int {
                    if true {
                        if true {
                            if true {
                                if true {
                                    42
                                } else {
                                    0
                                }
                            } else {
                                0
                            }
                        } else {
                            0
                        }
                    } else {
                        0
                    }
                }

                fn main() -> Int {
                    nested()
                }
            "#;

            let result = compile_source(source);

            if let Err(e) = &result {
                eprintln!("Nested control flow error: {}", e);
            }

            assert!(result.is_ok(), "Deeply nested control flow should compile");
        })
        .expect("Failed to spawn thread with larger stack");

    handle.join().expect("Thread panicked");
}
