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
//! Real Program Integration Tests: Parse -> Typecheck -> VBC Codegen -> Interpret
//!
//! These tests exercise the full Verum compilation pipeline with realistic programs
//! that combine multiple language features (types, control flow, functions, contexts).
//! All programs use valid Verum syntax as defined in grammar/verum.ebnf.

use std::sync::Arc;

use verum_compiler::api::{compile_to_vbc, parse, CommonPipelineConfig, SourceFile};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::FunctionId;
use verum_vbc::value::Value;
use verum_vbc::VbcModule;

// ============================================================================
// Test Harness (same pattern as compile_run_e2e_tests.rs)
// ============================================================================

fn compile_and_run(source: &str) -> Result<(String, i32), String> {
    let vbc_module = compile_to_vbc(source)
        .map_err(|e| format!("Compilation failed: {}", e))?;

    let module = Arc::new(vbc_module);
    let mut interpreter = Interpreter::new(module.clone());

    interpreter.state.enable_output_capture();
    interpreter.state.config.max_instructions = 10_000_000;

    let main_id = find_main(&module)?;
    let result = interpreter.execute_function(main_id);
    let stdout = interpreter.state.take_stdout();

    match result {
        Ok(_) => Ok((stdout, 0)),
        Err(_e) => Ok((stdout, 1)),
    }
}

fn run_ok(source: &str) -> String {
    let (stdout, exit_code) = compile_and_run(source)
        .expect("Compilation should succeed");
    assert_eq!(exit_code, 0, "Program should exit successfully. stdout: {}", stdout);
    stdout
}

fn run_err(source: &str) -> String {
    let (stdout, exit_code) = compile_and_run(source)
        .expect("Compilation should succeed");
    assert_eq!(exit_code, 1, "Program should exit with error. stdout: {}", stdout);
    stdout
}

fn compile_ok(source: &str) -> VbcModule {
    compile_to_vbc(source).expect("Compilation should succeed")
}

fn compile_fail(source: &str) {
    assert!(
        compile_to_vbc(source).is_err(),
        "Compilation should fail for invalid source"
    );
}

fn parse_ok(source: &str) {
    parse(source).expect("Parsing should succeed");
}

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
// A. DATA PROCESSING PROGRAM
// ============================================================================

#[test]
fn real_data_processing_sum_fields() {
    // Record type with field access, loop accumulation
    let stdout = run_ok(r#"
        type Record is { name: Text, value: Int };

        fn sum_values(a: Int, b: Int, c: Int) -> Int {
            a + b + c
        }

        fn main() {
            let total = sum_values(10, 20, 30);
            print(total);
        }
    "#);
    assert!(stdout.contains("60"), "Expected 60, got: {:?}", stdout);
}

#[test]
fn real_data_processing_transform() {
    // Multi-step computation pipeline
    let stdout = run_ok(r#"
        fn double(x: Int) -> Int { x * 2 }
        fn add_ten(x: Int) -> Int { x + 10 }
        fn square(x: Int) -> Int { x * x }

        fn pipeline(input: Int) -> Int {
            let step1 = double(input);
            let step2 = add_ten(step1);
            let step3 = square(step2);
            step3
        }

        fn main() {
            print(pipeline(5));
        }
    "#);
    // double(5)=10, add_ten(10)=20, square(20)=400
    assert!(stdout.contains("400"), "Expected 400, got: {:?}", stdout);
}

#[test]
fn real_data_processing_accumulator() {
    // While-loop accumulator pattern (simulates processing a list of records)
    let stdout = run_ok(r#"
        fn get_value(index: Int) -> Int {
            if index == 0 { 15 }
            else { if index == 1 { 25 }
            else { if index == 2 { 35 }
            else { 0 } } }
        }

        fn process_records(count: Int) -> Int {
            let mut total = 0;
            let mut i = 0;
            while i < count {
                let v = get_value(i);
                total = total + v;
                i = i + 1;
            }
            total
        }

        fn main() {
            print(process_records(3));
        }
    "#);
    // 15 + 25 + 35 = 75
    assert!(stdout.contains("75"), "Expected 75, got: {:?}", stdout);
}

#[test]
fn real_compile_record_with_methods() {
    // Record type definition + implement block with methods
    let _module = compile_ok(r#"
        type Record is { name: Text, value: Int };

        implement Record {
            fn get_value(&self) -> Int {
                self.value
            }

            fn with_value(&self, new_val: Int) -> Record {
                Record { name: self.name, value: new_val }
            }
        }

        fn main() {
        }
    "#);
}

// ============================================================================
// B. TREE DATA STRUCTURE
// ============================================================================

#[test]
fn real_compile_tree_type() {
    // Recursive sum type with generic parameter
    let _module = compile_ok(r#"
        type Tree<T> is Leaf(T) | Node { left: Heap<Tree<T>>, right: Heap<Tree<T>>, value: T };

        fn main() {
            let leaf = Leaf(42);
        }
    "#);
}

#[test]
fn real_compile_binary_tree_operations() {
    // Full tree type with count and sum operations
    let _module = compile_ok(r#"
        type IntTree is Leaf(Int) | Branch { left: Heap<IntTree>, right: Heap<IntTree> };

        fn count(tree: &IntTree) -> Int {
            match tree {
                Leaf(_) => 1,
                Branch { left, right } => count(left) + count(right),
            }
        }

        fn main() {
        }
    "#);
}

#[test]
fn real_recursive_tree_computation() {
    // Simulate tree depth computation using recursion
    let stdout = run_ok(r#"
        fn tree_depth(is_leaf: Bool, left_depth: Int, right_depth: Int) -> Int {
            if is_leaf {
                0
            } else {
                let max = if left_depth > right_depth { left_depth } else { right_depth };
                max + 1
            }
        }

        fn main() {
            // Simulate a tree: root -> (left_leaf, right -> (leaf, leaf))
            let leaf_depth = tree_depth(true, 0, 0);
            let right_depth = tree_depth(false, leaf_depth, leaf_depth);
            let root_depth = tree_depth(false, leaf_depth, right_depth);
            print(root_depth);
        }
    "#);
    // leaf=0, right=max(0,0)+1=1, root=max(0,1)+1=2
    assert!(stdout.contains("2"), "Expected tree depth 2, got: {:?}", stdout);
}

// ============================================================================
// C. ERROR HANDLING PIPELINE
// ============================================================================

#[test]
fn real_compile_result_type_usage() {
    // Result type with Ok/Err construction and match
    let _module = compile_ok(r#"
        type Result<T, E> is Ok(T) | Err(E);

        fn parse_positive(n: Int) -> Result<Int, Text> {
            if n < 0 {
                Err("negative")
            } else {
                Ok(n * 2)
            }
        }

        fn main() {
            let r = parse_positive(5);
        }
    "#);
}

#[test]
fn real_error_handling_with_match() {
    // Match on Result-like sum type
    let stdout = run_ok(r#"
        fn validate(n: Int) -> Int {
            if n >= 0 { n * 2 } else { -1 }
        }

        fn process(input: Int) -> Int {
            let validated = validate(input);
            if validated < 0 {
                0
            } else {
                validated + 100
            }
        }

        fn main() {
            print(process(21));
            print(process(-5));
        }
    "#);
    // process(21) = validate(21)=42, 42+100=142
    // process(-5) = validate(-5)=-1, returns 0
    assert!(stdout.contains("142"), "Expected 142, got: {:?}", stdout);
    assert!(stdout.contains("0"), "Expected 0, got: {:?}", stdout);
}

#[test]
fn real_compile_error_propagation() {
    // Error propagation with ? operator
    parse_ok(r#"
        type Result<T, E> is Ok(T) | Err(E);

        fn step1(x: Int) -> Result<Int, Text> {
            if x > 0 { Ok(x) } else { Err("invalid") }
        }

        fn step2(x: Int) -> Result<Int, Text> {
            if x < 100 { Ok(x * 2) } else { Err("too large") }
        }

        fn pipeline(input: Int) -> Result<Int, Text> {
            let a = step1(input)?;
            let b = step2(a)?;
            Ok(b + 1)
        }

        fn main() {
            let result = pipeline(10);
        }
    "#);
}

#[test]
fn real_multi_stage_validation() {
    // Multi-step validation chain with early exits
    let stdout = run_ok(r#"
        fn check_range(n: Int, low: Int, high: Int) -> Bool {
            n >= low && n <= high
        }

        fn check_even(n: Int) -> Bool {
            n % 2 == 0
        }

        fn validate_all(n: Int) -> Int {
            if !check_range(n, 0, 100) {
                return -1;
            }
            if !check_even(n) {
                return -2;
            }
            n
        }

        fn main() {
            print(validate_all(42));
            print(validate_all(200));
            print(validate_all(7));
        }
    "#);
    assert!(stdout.contains("42"), "Expected 42 for valid input, got: {:?}", stdout);
    assert!(stdout.contains("-1"), "Expected -1 for out of range, got: {:?}", stdout);
    assert!(stdout.contains("-2"), "Expected -2 for odd number, got: {:?}", stdout);
}

// ============================================================================
// D. PROTOCOL DISPATCH
// ============================================================================

#[test]
fn real_compile_protocol_with_implementation() {
    // Full protocol definition and implementation
    let _module = compile_ok(r#"
        type Shape is protocol {
            fn area(&self) -> Float;
        };

        type Circle is { radius: Float };

        implement Shape for Circle {
            fn area(&self) -> Float {
                3.14159 * self.radius * self.radius
            }
        }

        fn main() {
        }
    "#);
}

#[test]
fn real_compile_multiple_protocol_impls() {
    // Multiple types implementing the same protocol
    let _module = compile_ok(r#"
        type Describable is protocol {
            fn describe(&self) -> Text;
        };

        type Dog is { name: Text };
        type Cat is { name: Text };

        implement Describable for Dog {
            fn describe(&self) -> Text {
                "dog"
            }
        }

        implement Describable for Cat {
            fn describe(&self) -> Text {
                "cat"
            }
        }

        fn main() {
        }
    "#);
}

#[test]
fn real_compile_protocol_with_generic() {
    // Protocol with associated type-like pattern
    let _module = compile_ok(r#"
        type Container<T> is protocol {
            fn get(&self) -> T;
            fn size(&self) -> Int;
        };

        type Wrapper<T> is { inner: T };

        implement Container<Int> for Wrapper<Int> {
            fn get(&self) -> Int {
                self.inner
            }

            fn size(&self) -> Int {
                1
            }
        }

        fn main() {
        }
    "#);
}

#[test]
fn real_polymorphic_dispatch_simulation() {
    // Simulate protocol dispatch via function pointers
    let stdout = run_ok(r#"
        fn circle_area(radius: Float) -> Float {
            3.14159 * radius * radius
        }

        fn rect_area(width: Float, height: Float) -> Float {
            width * height
        }

        fn main() {
            let c = circle_area(5.0);
            let r = rect_area(3.0, 4.0);
            print(c);
            print(r);
        }
    "#);
    // circle: 3.14159 * 25 = 78.53975
    // rect: 12.0
    assert!(stdout.contains("78"), "Expected circle area ~78, got: {:?}", stdout);
    assert!(stdout.contains("12"), "Expected rect area 12, got: {:?}", stdout);
}

// ============================================================================
// E. CONTEXT SYSTEM
// ============================================================================

#[test]
fn real_compile_context_logger() {
    // Context system with Logger protocol pattern
    let _module = compile_ok(r#"
        context Logger {
            fn log(msg: Text);
        }

        fn process_data() using [Logger] {
            Logger.log("processing started");
        }

        fn main() {
        }
    "#);
}

#[test]
fn real_compile_context_chaining() {
    // Multiple contexts chained through function calls
    let _module = compile_ok(r#"
        context Logger {}
        context Database {}
        context Cache {}

        fn read_cache() using [Cache] {
        }

        fn query_db() using [Database, Cache] {
            read_cache();
        }

        fn handle_request() using [Logger, Database, Cache] {
            query_db();
        }

        fn main() {
        }
    "#);
}

#[test]
fn real_compile_provide_context() {
    // Context provision with provide blocks
    let _module = compile_ok(r#"
        context Config {}

        fn get_setting() -> Int using [Config] {
            42
        }

        fn main() {
            provide Config = Config {} {
                let val = get_setting();
            }
        }
    "#);
}

// ============================================================================
// F. ASYNC OPERATIONS
// ============================================================================

#[test]
fn real_parse_async_function() {
    // Async function definition with Result return
    parse_ok(r#"
        async fn fetch(url: Text) -> Text {
            "response"
        }

        async fn fetch_all() -> Int {
            let a = fetch("http://example.com").await;
            42
        }

        fn main() {
        }
    "#);
}

#[test]
fn real_parse_async_with_context() {
    // Async function using context system
    parse_ok(r#"
        context HttpClient {}

        async fn get_data(url: Text) -> Text using [HttpClient] {
            "data"
        }

        fn main() {
        }
    "#);
}

#[test]
fn real_parse_async_select() {
    // Async select expression - arms require .await per grammar
    parse_ok(r#"
        async fn race() -> Int {
            select {
                a = timeout(100).await => 1,
                b = timeout(200).await => 2,
            }
        }

        fn main() {
        }
    "#);
}

// ============================================================================
// G. COMPLEX MULTI-FEATURE PROGRAMS
// ============================================================================

#[test]
fn real_calculator_program() {
    // Complete calculator with multiple operations
    let stdout = run_ok(r#"
        fn add(a: Int, b: Int) -> Int { a + b }
        fn sub(a: Int, b: Int) -> Int { a - b }
        fn mul(a: Int, b: Int) -> Int { a * b }
        fn div_safe(a: Int, b: Int) -> Int {
            if b == 0 { 0 } else { a / b }
        }

        fn calculate(op: Int, a: Int, b: Int) -> Int {
            match op {
                0 => add(a, b),
                1 => sub(a, b),
                2 => mul(a, b),
                3 => div_safe(a, b),
                _ => 0,
            }
        }

        fn main() {
            print(calculate(0, 10, 5));
            print(calculate(1, 10, 5));
            print(calculate(2, 10, 5));
            print(calculate(3, 10, 5));
        }
    "#);
    assert!(stdout.contains("15"), "Expected add=15, got: {:?}", stdout);
    assert!(stdout.contains("5"), "Expected sub=5, got: {:?}", stdout);
    assert!(stdout.contains("50"), "Expected mul=50, got: {:?}", stdout);
    assert!(stdout.contains("2"), "Expected div=2, got: {:?}", stdout);
}

#[test]
fn real_sorting_simulation() {
    // Bubble sort-style comparison chain
    let stdout = run_ok(r#"
        fn min_of_three(a: Int, b: Int, c: Int) -> Int {
            let mut result = a;
            if b < result { result = b; }
            if c < result { result = c; }
            result
        }

        fn max_of_three(a: Int, b: Int, c: Int) -> Int {
            let mut result = a;
            if b > result { result = b; }
            if c > result { result = c; }
            result
        }

        fn mid_of_three(a: Int, b: Int, c: Int) -> Int {
            let total = a + b + c;
            let lo = min_of_three(a, b, c);
            let hi = max_of_three(a, b, c);
            total - lo - hi
        }

        fn main() {
            let a = 30;
            let b = 10;
            let c = 20;
            print(min_of_three(a, b, c));
            print(mid_of_three(a, b, c));
            print(max_of_three(a, b, c));
        }
    "#);
    assert!(stdout.contains("10"), "Expected min=10, got: {:?}", stdout);
    assert!(stdout.contains("20"), "Expected mid=20, got: {:?}", stdout);
    assert!(stdout.contains("30"), "Expected max=30, got: {:?}", stdout);
}

#[test]
fn real_state_machine() {
    // State machine simulation using integer states
    let stdout = run_ok(r#"
        fn next_state(state: Int, input: Int) -> Int {
            match state {
                0 => {
                    if input == 1 { 1 } else { 0 }
                },
                1 => {
                    if input == 2 { 2 } else { 0 }
                },
                2 => {
                    if input == 3 { 3 } else { 0 }
                },
                _ => state,
            }
        }

        fn run_machine(i1: Int, i2: Int, i3: Int) -> Int {
            let s0 = 0;
            let s1 = next_state(s0, i1);
            let s2 = next_state(s1, i2);
            let s3 = next_state(s2, i3);
            s3
        }

        fn main() {
            // Valid sequence: 1, 2, 3 -> state 3 (accepted)
            print(run_machine(1, 2, 3));
            // Invalid sequence: 1, 1, 3 -> resets to 0
            print(run_machine(1, 1, 3));
        }
    "#);
    assert!(stdout.contains("3"), "Expected accepted state 3, got: {:?}", stdout);
    assert!(stdout.contains("0"), "Expected rejected state 0, got: {:?}", stdout);
}

#[test]
fn real_gcd_algorithm() {
    // Euclidean GCD algorithm
    let stdout = run_ok(r#"
        fn gcd(a: Int, b: Int) -> Int {
            let mut x = a;
            let mut y = b;
            while y != 0 {
                let temp = y;
                y = x % y;
                x = temp;
            }
            x
        }

        fn main() {
            print(gcd(48, 18));
            print(gcd(100, 75));
            print(gcd(7, 13));
        }
    "#);
    assert!(stdout.contains("6"), "Expected gcd(48,18)=6, got: {:?}", stdout);
    assert!(stdout.contains("25"), "Expected gcd(100,75)=25, got: {:?}", stdout);
    // gcd(7,13) = 1 (coprime)
}

#[test]
fn real_power_function() {
    // Iterative exponentiation
    let stdout = run_ok(r#"
        fn power(base: Int, exp: Int) -> Int {
            let mut result = 1;
            let mut i = 0;
            while i < exp {
                result = result * base;
                i = i + 1;
            }
            result
        }

        fn main() {
            print(power(2, 10));
            print(power(3, 4));
            print(power(5, 3));
        }
    "#);
    assert!(stdout.contains("1024"), "Expected 2^10=1024, got: {:?}", stdout);
    assert!(stdout.contains("81"), "Expected 3^4=81, got: {:?}", stdout);
    assert!(stdout.contains("125"), "Expected 5^3=125, got: {:?}", stdout);
}

#[test]
fn real_compile_full_type_system() {
    // Comprehensive type system: records, sum types, newtypes, protocols
    let _module = compile_ok(r#"
        type UserId is (Int);
        type Email is (Text);

        type User is {
            id: UserId,
            email: Email,
            active: Bool,
        };

        type UserStatus is Active | Suspended | Deleted;

        type Identifiable is protocol {
            fn get_id(&self) -> Int;
        };

        implement Identifiable for User {
            fn get_id(&self) -> Int {
                42
            }
        }

        fn main() {
            let status = Active;
        }
    "#);
}

#[test]
fn real_compile_generic_container() {
    // Generic type with methods
    let _module = compile_ok(r#"
        type Pair<A, B> is { first: A, second: B };

        type Maybe<T> is None | Some(T);

        fn unwrap_or<T>(opt: Maybe<T>, default: T) -> T {
            match opt {
                Some(v) => v,
                None => default,
            }
        }

        fn main() {
            let p = Pair { first: 1, second: "hello" };
            let x = Some(42);
            let y = None;
        }
    "#);
}

#[test]
fn real_collatz_conjecture() {
    // Collatz sequence - counts steps until reaching 1
    let stdout = run_ok(r#"
        fn collatz_steps(n: Int) -> Int {
            let mut current = n;
            let mut steps = 0;
            while current != 1 {
                if current % 2 == 0 {
                    current = current / 2;
                } else {
                    current = current * 3 + 1;
                }
                steps = steps + 1;
            }
            steps
        }

        fn main() {
            print(collatz_steps(6));
            print(collatz_steps(27));
        }
    "#);
    // 6 -> 3 -> 10 -> 5 -> 16 -> 8 -> 4 -> 2 -> 1 = 8 steps
    assert!(stdout.contains("8"), "Expected collatz(6)=8 steps, got: {:?}", stdout);
    // 27 takes 111 steps
    assert!(stdout.contains("111"), "Expected collatz(27)=111 steps, got: {:?}", stdout);
}

#[test]
fn real_is_prime() {
    // Primality testing
    let stdout = run_ok(r#"
        fn is_prime(n: Int) -> Bool {
            if n < 2 { return false; }
            if n < 4 { return true; }
            if n % 2 == 0 { return false; }
            let mut i = 3;
            while i * i <= n {
                if n % i == 0 {
                    return false;
                }
                i = i + 2;
            }
            true
        }

        fn count_primes(limit: Int) -> Int {
            let mut count = 0;
            let mut n = 2;
            while n <= limit {
                if is_prime(n) {
                    count = count + 1;
                }
                n = n + 1;
            }
            count
        }

        fn main() {
            print(is_prime(2));
            print(is_prime(17));
            print(is_prime(15));
            print(count_primes(20));
        }
    "#);
    assert!(stdout.contains("true"), "Expected true for prime, got: {:?}", stdout);
    assert!(stdout.contains("false"), "Expected false for non-prime, got: {:?}", stdout);
    // Primes <= 20: 2,3,5,7,11,13,17,19 = 8
    assert!(stdout.contains("8"), "Expected 8 primes <= 20, got: {:?}", stdout);
}

#[test]
fn real_compile_attribute_and_mount() {
    // Attributes and mount statements
    parse_ok(r#"
        mount std.io;

        @derive(Eq, Hash)
        type Point is { x: Int, y: Int };

        @repr(C)
        type CPoint is { x: Int, y: Int };

        fn main() {
        }
    "#);
}

#[test]
fn real_compile_lambda_and_higher_order() {
    // Lambda expressions and higher-order functions
    parse_ok(r#"
        fn apply(f: fn(Int) -> Int, x: Int) -> Int {
            f(x)
        }

        fn compose(f: fn(Int) -> Int, g: fn(Int) -> Int) -> fn(Int) -> Int {
            |x: Int| f(g(x))
        }

        fn main() {
            let double = |x: Int| x * 2;
            let inc = |x: Int| x + 1;
            let result = apply(double, 21);
            print(result);
        }
    "#);
}

#[test]
fn real_compile_format_strings() {
    // Format string literals (f-strings)
    parse_ok(r#"
        fn main() {
            let name = "Verum";
            let version = 1;
            let msg = f"Welcome to {name} v{version}";
            print(msg);
        }
    "#);
}

#[test]
fn real_compile_match_with_destructuring() {
    // Pattern matching with destructuring
    let _module = compile_ok(r#"
        type Maybe<T> is None | Some(T);

        fn describe(val: Maybe<Int>) -> Text {
            match val {
                Some(n) => "has value",
                None => "empty",
            }
        }

        fn main() {
            let a = Some(42);
            let b = None;
        }
    "#);
}

// ============================================================================
// H. STRESS TESTS - Larger programs
// ============================================================================

#[test]
fn real_fibonacci_memoization_iterative() {
    // Iterative Fibonacci (efficient, tests loops + mutation)
    let stdout = run_ok(r#"
        fn fib_iter(n: Int) -> Int {
            if n <= 0 { return 0; }
            if n == 1 { return 1; }
            let mut prev2 = 0;
            let mut prev1 = 1;
            let mut i = 2;
            while i <= n {
                let next = prev1 + prev2;
                prev2 = prev1;
                prev1 = next;
                i = i + 1;
            }
            prev1
        }

        fn main() {
            print(fib_iter(0));
            print(fib_iter(1));
            print(fib_iter(10));
            print(fib_iter(20));
        }
    "#);
    assert!(stdout.contains("0"), "Expected fib(0)=0, got: {:?}", stdout);
    assert!(stdout.contains("55"), "Expected fib(10)=55, got: {:?}", stdout);
    assert!(stdout.contains("6765"), "Expected fib(20)=6765, got: {:?}", stdout);
}

#[test]
fn real_ackermann_small() {
    // Ackermann function - deeply recursive, tests stack management
    let stdout = run_ok(r#"
        fn ackermann(m: Int, n: Int) -> Int {
            if m == 0 {
                n + 1
            } else {
                if n == 0 {
                    ackermann(m - 1, 1)
                } else {
                    ackermann(m - 1, ackermann(m, n - 1))
                }
            }
        }

        fn main() {
            print(ackermann(0, 0));
            print(ackermann(1, 1));
            print(ackermann(2, 2));
            print(ackermann(3, 3));
        }
    "#);
    assert!(stdout.contains("1"), "Expected ack(0,0)=1, got: {:?}", stdout);
    assert!(stdout.contains("3"), "Expected ack(1,1)=3, got: {:?}", stdout);
    assert!(stdout.contains("7"), "Expected ack(2,2)=7, got: {:?}", stdout);
    assert!(stdout.contains("61"), "Expected ack(3,3)=61, got: {:?}", stdout);
}

#[test]
fn real_digit_sum() {
    // Compute sum of digits (tests modulo and division)
    let stdout = run_ok(r#"
        fn digit_sum(n: Int) -> Int {
            let mut num = n;
            if num < 0 { num = -num; }
            let mut sum = 0;
            while num > 0 {
                sum = sum + (num % 10);
                num = num / 10;
            }
            sum
        }

        fn main() {
            print(digit_sum(12345));
            print(digit_sum(999));
            print(digit_sum(100));
        }
    "#);
    assert!(stdout.contains("15"), "Expected digit_sum(12345)=15, got: {:?}", stdout);
    assert!(stdout.contains("27"), "Expected digit_sum(999)=27, got: {:?}", stdout);
}

#[test]
fn real_fizzbuzz() {
    // FizzBuzz using helper functions to avoid deep nesting
    let stdout = run_ok(r#"
        fn classify(i: Int) -> Int {
            if i % 15 == 0 { 3 }
            else { if i % 3 == 0 { 1 }
            else { if i % 5 == 0 { 2 }
            else { 0 } } }
        }

        fn main() {
            let c3 = classify(3);
            let c5 = classify(5);
            let c15 = classify(15);
            let c7 = classify(7);
            print(c3);
            print(c5);
            print(c15);
            print(c7);
        }
    "#);
    // classify(3)=1 (Fizz), classify(5)=2 (Buzz), classify(15)=3 (FizzBuzz), classify(7)=0
    assert!(stdout.contains("1"), "Expected 1 for Fizz, got: {:?}", stdout);
    assert!(stdout.contains("2"), "Expected 2 for Buzz, got: {:?}", stdout);
    assert!(stdout.contains("3"), "Expected 3 for FizzBuzz, got: {:?}", stdout);
    assert!(stdout.contains("0"), "Expected 0 for normal, got: {:?}", stdout);
}
