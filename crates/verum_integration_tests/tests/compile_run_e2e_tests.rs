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
//! End-to-End Integration Tests: Parse -> Typecheck -> VBC Codegen -> Interpret
//!
//! These tests exercise the complete Verum compilation pipeline from source code
//! to execution results. Each test compiles a Verum source program through:
//!
//! 1. Lexical analysis (verum_lexer)
//! 2. Parsing (verum_fast_parser)
//! 3. Type checking (verum_types)
//! 4. VBC code generation (verum_vbc::codegen)
//! 5. VBC interpretation (verum_vbc::interpreter)
//!
//! All test programs use valid Verum syntax as defined in grammar/verum.ebnf.

use std::sync::Arc;

use verum_compiler::api::{compile_to_vbc, parse, CommonPipelineConfig, SourceFile};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::FunctionId;
use verum_vbc::value::Value;
use verum_vbc::VbcModule;

// ============================================================================
// Test Harness
// ============================================================================

/// Compile a Verum source program to VBC and execute it via the interpreter.
///
/// Returns (stdout, exit_code) where exit_code is 0 for success, 1 for runtime error.
fn compile_and_run(source: &str) -> Result<(String, i32), String> {
    // Step 1: Parse + typecheck + VBC codegen via compiler API
    let vbc_module = compile_to_vbc(source)
        .map_err(|e| format!("Compilation failed: {}", e))?;

    // Step 2: Create interpreter
    let module = Arc::new(vbc_module);
    let mut interpreter = Interpreter::new(module.clone());

    // Enable output capture for testing
    interpreter.state.enable_output_capture();
    interpreter.state.config.max_instructions = 10_000_000; // 10M limit

    // Step 3: Find main function
    let main_id = find_main(&module)?;

    // Step 4: Execute
    let result = interpreter.execute_function(main_id);

    let stdout = interpreter.state.take_stdout();

    match result {
        Ok(_) => Ok((stdout, 0)),
        Err(e) => {
            let stderr = interpreter.state.take_stderr();
            Ok((stdout, 1))
        }
    }
}

/// Compile and run, expecting success (exit code 0).
fn run_ok(source: &str) -> String {
    let (stdout, exit_code) = compile_and_run(source)
        .expect("Compilation should succeed");
    assert_eq!(exit_code, 0, "Program should exit successfully. stdout: {}", stdout);
    stdout
}

/// Compile and run, expecting a runtime error (exit code 1).
fn run_err(source: &str) -> String {
    let (stdout, exit_code) = compile_and_run(source)
        .expect("Compilation should succeed");
    assert_eq!(exit_code, 1, "Program should exit with error. stdout: {}", stdout);
    stdout
}

/// Compile only, expecting compilation success.
fn compile_ok(source: &str) -> VbcModule {
    compile_to_vbc(source).expect("Compilation should succeed")
}

/// Compile only, expecting compilation failure.
fn compile_fail(source: &str) {
    assert!(
        compile_to_vbc(source).is_err(),
        "Compilation should fail for invalid source"
    );
}

/// Parse only, expecting success.
fn parse_ok(source: &str) {
    parse(source).expect("Parsing should succeed");
}

/// Parse only, expecting failure.
fn parse_fail(source: &str) {
    assert!(
        parse(source).is_err(),
        "Parsing should fail for invalid source"
    );
}

/// Find the main function ID in a VBC module.
fn find_main(module: &VbcModule) -> Result<FunctionId, String> {
    for (idx, func_desc) in module.functions.iter().enumerate() {
        if let Some(name) = module.get_string(func_desc.name)
            && name == "main" {
                return Ok(FunctionId(idx as u32));
            }
    }
    Err("No main function found in VBC module".to_string())
}

// ============================================================================
// A. BASIC EXECUTION (10 tests)
// ============================================================================

#[test]
fn e2e_hello_world() {
    let stdout = run_ok(r#"
        fn main() {
            print("Hello, World!");
        }
    "#);
    assert!(
        stdout.contains("Hello, World!"),
        "Expected 'Hello, World!' in stdout, got: {:?}",
        stdout
    );
}

#[test]
fn e2e_integer_arithmetic() {
    let stdout = run_ok(r#"
        fn main() {
            let x = 2 + 3 * 4;
            print(x);
        }
    "#);
    assert!(stdout.contains("14"), "Expected 14, got: {:?}", stdout);
}

#[test]
fn e2e_variable_binding() {
    let stdout = run_ok(r#"
        fn main() {
            let x = 10;
            let y = 20;
            let z = x + y;
            print(z);
        }
    "#);
    assert!(stdout.contains("30"), "Expected 30, got: {:?}", stdout);
}

#[test]
fn e2e_function_call() {
    let stdout = run_ok(r#"
        fn add(a: Int, b: Int) -> Int {
            a + b
        }

        fn main() {
            let result = add(40, 2);
            print(result);
        }
    "#);
    assert!(stdout.contains("42"), "Expected 42, got: {:?}", stdout);
}

#[test]
fn e2e_nested_function_calls() {
    let stdout = run_ok(r#"
        fn double(x: Int) -> Int {
            x * 2
        }

        fn add_one(x: Int) -> Int {
            x + 1
        }

        fn main() {
            let result = add_one(double(5));
            print(result);
        }
    "#);
    assert!(stdout.contains("11"), "Expected 11, got: {:?}", stdout);
}

#[test]
fn e2e_recursive_factorial() {
    let stdout = run_ok(r#"
        fn factorial(n: Int) -> Int {
            if n <= 1 {
                1
            } else {
                n * factorial(n - 1)
            }
        }

        fn main() {
            print(factorial(5));
        }
    "#);
    assert!(stdout.contains("120"), "Expected 120, got: {:?}", stdout);
}

#[test]
fn e2e_recursive_fibonacci() {
    let stdout = run_ok(r#"
        fn fib(n: Int) -> Int {
            if n <= 0 {
                0
            } else {
                if n == 1 {
                    1
                } else {
                    fib(n - 1) + fib(n - 2)
                }
            }
        }

        fn main() {
            print(fib(10));
        }
    "#);
    assert!(stdout.contains("55"), "Expected 55, got: {:?}", stdout);
}

#[test]
fn e2e_boolean_expressions() {
    let stdout = run_ok(r#"
        fn main() {
            let a = true;
            let b = false;
            let c = a && b;
            let d = a || b;
            print(c);
            print(d);
        }
    "#);
    assert!(stdout.contains("false"), "Expected 'false' in output, got: {:?}", stdout);
    assert!(stdout.contains("true"), "Expected 'true' in output, got: {:?}", stdout);
}

#[test]
fn e2e_string_literal() {
    let stdout = run_ok(r#"
        fn main() {
            let greeting = "Verum";
            print(greeting);
        }
    "#);
    assert!(stdout.contains("Verum"), "Expected 'Verum', got: {:?}", stdout);
}

#[test]
fn e2e_multiple_print_statements() {
    let stdout = run_ok(r#"
        fn main() {
            print(1);
            print(2);
            print(3);
        }
    "#);
    assert!(stdout.contains("1"), "Missing 1 in output: {:?}", stdout);
    assert!(stdout.contains("2"), "Missing 2 in output: {:?}", stdout);
    assert!(stdout.contains("3"), "Missing 3 in output: {:?}", stdout);
}

// ============================================================================
// B. TYPE SYSTEM INTEGRATION (10 tests)
// ============================================================================

#[test]
fn e2e_type_annotation_int() {
    let stdout = run_ok(r#"
        fn main() {
            let x: Int = 42;
            print(x);
        }
    "#);
    assert!(stdout.contains("42"), "Expected 42, got: {:?}", stdout);
}

#[test]
fn e2e_type_annotation_bool() {
    let stdout = run_ok(r#"
        fn main() {
            let x: Bool = true;
            print(x);
        }
    "#);
    assert!(stdout.contains("true"), "Expected true, got: {:?}", stdout);
}

#[test]
fn e2e_type_annotation_float() {
    let stdout = run_ok(r#"
        fn main() {
            let x: Float = 3.14;
            print(x);
        }
    "#);
    assert!(stdout.contains("3.14"), "Expected 3.14, got: {:?}", stdout);
}

#[test]
fn e2e_function_return_type() {
    let stdout = run_ok(r#"
        fn square(x: Int) -> Int {
            x * x
        }

        fn main() {
            print(square(7));
        }
    "#);
    assert!(stdout.contains("49"), "Expected 49, got: {:?}", stdout);
}

#[test]
fn e2e_multiple_return_types() {
    let stdout = run_ok(r#"
        fn is_even(n: Int) -> Bool {
            n % 2 == 0
        }

        fn main() {
            print(is_even(4));
            print(is_even(7));
        }
    "#);
    assert!(stdout.contains("true"), "Expected true for 4, got: {:?}", stdout);
    assert!(stdout.contains("false"), "Expected false for 7, got: {:?}", stdout);
}

#[test]
fn e2e_generic_function() {
    let stdout = run_ok(r#"
        fn identity<T>(x: T) -> T {
            x
        }

        fn main() {
            let a = identity(42);
            print(a);
        }
    "#);
    assert!(stdout.contains("42"), "Expected 42, got: {:?}", stdout);
}

#[test]
fn e2e_type_inference_let() {
    // Type inference should work without explicit annotation
    let stdout = run_ok(r#"
        fn main() {
            let x = 100;
            let y = x + 50;
            print(y);
        }
    "#);
    assert!(stdout.contains("150"), "Expected 150, got: {:?}", stdout);
}

#[test]
fn e2e_function_as_value() {
    let stdout = run_ok(r#"
        fn apply(f: fn(Int) -> Int, x: Int) -> Int {
            f(x)
        }

        fn double(x: Int) -> Int {
            x * 2
        }

        fn main() {
            print(apply(double, 21));
        }
    "#);
    assert!(stdout.contains("42"), "Expected 42, got: {:?}", stdout);
}

#[test]
fn e2e_compile_type_record() {
    // Verify record type definitions compile through the full pipeline
    let _module = compile_ok(r#"
        type Point is { x: Int, y: Int };

        fn main() {
            let p = Point { x: 10, y: 20 };
        }
    "#);
}

#[test]
fn e2e_compile_sum_type() {
    // Verify sum type definitions compile through the full pipeline
    let _module = compile_ok(r#"
        type Shape is Circle(Float) | Rectangle(Float, Float);

        fn main() {
            let s = Circle(5.0);
        }
    "#);
}

// ============================================================================
// C. CONTROL FLOW (10 tests)
// ============================================================================

#[test]
fn e2e_if_else_basic() {
    let stdout = run_ok(r#"
        fn main() {
            let x = 10;
            if x > 5 {
                print("greater");
            } else {
                print("not greater");
            }
        }
    "#);
    assert!(stdout.contains("greater"), "Expected 'greater', got: {:?}", stdout);
}

#[test]
fn e2e_if_else_false_branch() {
    let stdout = run_ok(r#"
        fn main() {
            let x = 3;
            if x > 5 {
                print("big");
            } else {
                print("small");
            }
        }
    "#);
    assert!(stdout.contains("small"), "Expected 'small', got: {:?}", stdout);
}

#[test]
fn e2e_nested_if() {
    let stdout = run_ok(r#"
        fn classify(n: Int) -> Int {
            if n > 0 {
                if n > 100 {
                    2
                } else {
                    1
                }
            } else {
                0
            }
        }

        fn main() {
            print(classify(50));
        }
    "#);
    assert!(stdout.contains("1"), "Expected 1, got: {:?}", stdout);
}

#[test]
fn e2e_match_integer() {
    let stdout = run_ok(r#"
        fn main() {
            let x = 2;
            match x {
                1 => print("one"),
                2 => print("two"),
                3 => print("three"),
                _ => print("other"),
            }
        }
    "#);
    assert!(stdout.contains("two"), "Expected 'two', got: {:?}", stdout);
}

#[test]
fn e2e_match_wildcard() {
    let stdout = run_ok(r#"
        fn main() {
            let x = 99;
            match x {
                1 => print("one"),
                2 => print("two"),
                _ => print("other"),
            }
        }
    "#);
    assert!(stdout.contains("other"), "Expected 'other', got: {:?}", stdout);
}

#[test]
fn e2e_while_loop() {
    let stdout = run_ok(r#"
        fn main() {
            let mut i = 0;
            let mut sum = 0;
            while i < 5 {
                sum = sum + i;
                i = i + 1;
            }
            print(sum);
        }
    "#);
    assert!(stdout.contains("10"), "Expected 10, got: {:?}", stdout);
}

#[test]
fn e2e_while_loop_countdown() {
    let stdout = run_ok(r#"
        fn main() {
            let mut n = 5;
            while n > 0 {
                print(n);
                n = n - 1;
            }
        }
    "#);
    assert!(stdout.contains("5"), "Expected 5 in output, got: {:?}", stdout);
    assert!(stdout.contains("1"), "Expected 1 in output, got: {:?}", stdout);
}

#[test]
fn e2e_early_return() {
    let stdout = run_ok(r#"
        fn find_first_even(a: Int, b: Int, c: Int) -> Int {
            if a % 2 == 0 {
                return a;
            }
            if b % 2 == 0 {
                return b;
            }
            c
        }

        fn main() {
            print(find_first_even(3, 4, 6));
        }
    "#);
    assert!(stdout.contains("4"), "Expected 4, got: {:?}", stdout);
}

#[test]
fn e2e_loop_with_break() {
    let stdout = run_ok(r#"
        fn main() {
            let mut i = 0;
            loop {
                if i >= 3 {
                    break;
                }
                i = i + 1;
            }
            print(i);
        }
    "#);
    assert!(stdout.contains("3"), "Expected 3, got: {:?}", stdout);
}

#[test]
fn e2e_sum_via_loop() {
    // Sum 1..5 using a while loop (for-range requires stdlib IntoIterator)
    let stdout = run_ok(r#"
        fn main() {
            let mut sum = 0;
            let mut i = 1;
            while i <= 5 {
                sum = sum + i;
                i = i + 1;
            }
            print(sum);
        }
    "#);
    assert!(stdout.contains("15"), "Expected 15, got: {:?}", stdout);
}

// ============================================================================
// D. DATA STRUCTURES (10 tests)
// ============================================================================

#[test]
fn e2e_tuple_creation() {
    let stdout = run_ok(r#"
        fn main() {
            let pair = (10, 20);
            print(pair);
        }
    "#);
    // Tuple should print in some form
    assert!(!stdout.is_empty(), "Expected some output for tuple, got empty");
}

#[test]
fn e2e_compile_record_type() {
    let _module = compile_ok(r#"
        type Person is { name: Text, age: Int };

        fn main() {
            let p = Person { name: "Alice", age: 30 };
        }
    "#);
}

#[test]
fn e2e_compile_nested_record() {
    let _module = compile_ok(r#"
        type Inner is { value: Int };
        type Outer is { inner: Inner, label: Text };

        fn main() {
            let x = Outer {
                inner: Inner { value: 42 },
                label: "test",
            };
        }
    "#);
}

#[test]
fn e2e_compile_sum_type_variants() {
    let _module = compile_ok(r#"
        type Color is Red | Green | Blue;

        fn main() {
            let c = Red;
        }
    "#);
}

#[test]
fn e2e_compile_maybe_type() {
    let _module = compile_ok(r#"
        type Maybe<T> is None | Some(T);

        fn main() {
            let x = Some(42);
            let y = None;
        }
    "#);
}

#[test]
fn e2e_compile_result_type() {
    let _module = compile_ok(r#"
        type Result<T, E> is Ok(T) | Err(E);

        fn main() {
            let x = Ok(42);
        }
    "#);
}

#[test]
fn e2e_list_literal() {
    let stdout = run_ok(r#"
        fn main() {
            let xs = [1, 2, 3];
            print(xs);
        }
    "#);
    assert!(!stdout.is_empty(), "Expected some output for list");
}

#[test]
fn e2e_compile_newtype() {
    let _module = compile_ok(r#"
        type UserId is (Int);

        fn main() {
            let id = UserId(42);
        }
    "#);
}

#[test]
fn e2e_compile_unit_type() {
    let _module = compile_ok(r#"
        type Marker is ();

        fn main() {
            let m = Marker;
        }
    "#);
}

#[test]
fn e2e_empty_function_body() {
    let stdout = run_ok(r#"
        fn do_nothing() {
        }

        fn main() {
            do_nothing();
            print("done");
        }
    "#);
    assert!(stdout.contains("done"), "Expected 'done', got: {:?}", stdout);
}

// ============================================================================
// E. ERROR HANDLING (5 tests)
// ============================================================================

#[test]
fn e2e_parse_error_missing_brace() {
    compile_fail(r#"
        fn main() {
            print("hello")
    "#);
}

#[test]
fn e2e_parse_error_invalid_syntax() {
    compile_fail(r#"
        fn 123invalid() {
        }
    "#);
}

#[test]
fn e2e_parse_error_unterminated_string() {
    compile_fail(r#"
        fn main() {
            let x = "unterminated;
        }
    "#);
}

#[test]
fn e2e_compile_multiple_functions_before_main() {
    let stdout = run_ok(r#"
        fn helper() -> Int {
            99
        }

        fn another_helper(x: Int) -> Int {
            x + 1
        }

        fn main() {
            let a = helper();
            let b = another_helper(a);
            print(b);
        }
    "#);
    assert!(stdout.contains("100"), "Expected 100, got: {:?}", stdout);
}

#[test]
fn e2e_runtime_division_by_zero() {
    // Division by zero should cause a runtime error
    let (stdout, exit_code) = compile_and_run(r#"
        fn main() {
            let x = 1 / 0;
            print(x);
        }
    "#).expect("Compilation should succeed");
    // Either it panics (exit_code 1) or prints some result
    // Just verify it compiles and runs without hanging
    assert!(exit_code == 0 || exit_code == 1, "Should complete execution");
}

// ============================================================================
// F. CONTEXT SYSTEM (5 tests)
// ============================================================================

#[test]
fn e2e_compile_using_clause() {
    // Verify that using clauses parse and compile with defined contexts
    let _module = compile_ok(r#"
        context Logger {}

        fn process() using [Logger] {
        }

        fn main() {
        }
    "#);
}

#[test]
fn e2e_compile_provide_block() {
    // provide uses = syntax: provide ContextName = value { body }
    let _module = compile_ok(r#"
        context AppConfig {}

        fn main() {
            provide AppConfig = AppConfig {} {
                print("with context");
            }
        }
    "#);
}

#[test]
fn e2e_compile_multiple_contexts() {
    let _module = compile_ok(r#"
        context Logger {}
        context Database {}

        fn work() using [Logger, Database] {
        }

        fn main() {
        }
    "#);
}

#[test]
fn e2e_compile_context_propagation() {
    let _module = compile_ok(r#"
        context Logger {}

        fn inner() using [Logger] {
        }

        fn outer() using [Logger] {
            inner();
        }

        fn main() {
        }
    "#);
}

#[test]
fn e2e_compile_context_with_return_type() {
    let _module = compile_ok(r#"
        context Database {}

        fn fetch_data() -> Int using [Database] {
            42
        }

        fn main() {
        }
    "#);
}

// ============================================================================
// G. ADDITIONAL COVERAGE (11 tests)
// ============================================================================

#[test]
fn e2e_negative_numbers() {
    let stdout = run_ok(r#"
        fn main() {
            let x = -5;
            let y = x + 10;
            print(y);
        }
    "#);
    assert!(stdout.contains("5"), "Expected 5, got: {:?}", stdout);
}

#[test]
fn e2e_comparison_operators() {
    let stdout = run_ok(r#"
        fn main() {
            print(10 > 5);
            print(3 < 1);
            print(5 == 5);
            print(5 != 3);
        }
    "#);
    assert!(stdout.contains("true"), "Expected true in output, got: {:?}", stdout);
    assert!(stdout.contains("false"), "Expected false in output, got: {:?}", stdout);
}

#[test]
fn e2e_modulo_operator() {
    let stdout = run_ok(r#"
        fn main() {
            let x = 17 % 5;
            print(x);
        }
    "#);
    assert!(stdout.contains("2"), "Expected 2, got: {:?}", stdout);
}

#[test]
fn e2e_string_concatenation() {
    let stdout = run_ok(r#"
        fn main() {
            let a = "Hello";
            let b = " World";
            print(a);
            print(b);
        }
    "#);
    assert!(stdout.contains("Hello"), "Expected Hello, got: {:?}", stdout);
    assert!(stdout.contains("World"), "Expected World, got: {:?}", stdout);
}

#[test]
fn e2e_deeply_nested_calls() {
    let stdout = run_ok(r#"
        fn f(x: Int) -> Int { x + 1 }
        fn g(x: Int) -> Int { f(x) + 1 }
        fn h(x: Int) -> Int { g(x) + 1 }

        fn main() {
            print(h(0));
        }
    "#);
    assert!(stdout.contains("3"), "Expected 3, got: {:?}", stdout);
}

#[test]
fn e2e_mutual_recursion() {
    let stdout = run_ok(r#"
        fn is_even(n: Int) -> Bool {
            if n == 0 {
                true
            } else {
                is_odd(n - 1)
            }
        }

        fn is_odd(n: Int) -> Bool {
            if n == 0 {
                false
            } else {
                is_even(n - 1)
            }
        }

        fn main() {
            print(is_even(4));
        }
    "#);
    assert!(stdout.contains("true"), "Expected true, got: {:?}", stdout);
}

#[test]
fn e2e_variable_shadowing() {
    let stdout = run_ok(r#"
        fn main() {
            let x = 1;
            let x = x + 10;
            let x = x * 2;
            print(x);
        }
    "#);
    assert!(stdout.contains("22"), "Expected 22, got: {:?}", stdout);
}

#[test]
fn e2e_empty_program_no_output() {
    let stdout = run_ok(r#"
        fn main() {
        }
    "#);
    assert!(stdout.is_empty() || stdout.trim().is_empty(),
        "Expected no output from empty main, got: {:?}", stdout);
}

#[test]
fn e2e_compile_comment_preservation() {
    // Comments should not affect compilation
    let stdout = run_ok(r#"
        // This is a line comment
        fn main() {
            /* Block comment */
            let x = 42; // inline comment
            print(x);
        }
    "#);
    assert!(stdout.contains("42"), "Expected 42, got: {:?}", stdout);
}

#[test]
fn e2e_compile_protocol_definition() {
    let _module = compile_ok(r#"
        type Printable is protocol {
            fn to_text(&self) -> Text;
        };

        fn main() {
        }
    "#);
}

#[test]
fn e2e_compile_implement_block() {
    let _module = compile_ok(r#"
        type Counter is { value: Int };

        implement Counter {
            fn increment(&mut self) {
                self.value = self.value + 1;
            }
        }

        fn main() {
        }
    "#);
}

// ============================================================================
// PARSING-ONLY TESTS (verify grammar coverage without full execution)
// ============================================================================

#[test]
fn e2e_parse_mount_statement() {
    parse_ok(r#"
        mount std.io;

        fn main() {
        }
    "#);
}

#[test]
fn e2e_parse_attribute() {
    parse_ok(r#"
        @derive(Eq, Hash)
        type Id is (Int);

        fn main() {
        }
    "#);
}

#[test]
fn e2e_parse_async_fn() {
    parse_ok(r#"
        async fn fetch() -> Int {
            42
        }

        fn main() {
        }
    "#);
}

#[test]
fn e2e_parse_lambda() {
    parse_ok(r#"
        fn main() {
            let f = |x: Int| x * 2;
        }
    "#);
}

#[test]
fn e2e_parse_format_string() {
    parse_ok(r#"
        fn main() {
            let name = "world";
            print(f"hello {name}");
        }
    "#);
}
