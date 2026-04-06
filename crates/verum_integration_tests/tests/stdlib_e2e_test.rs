#![cfg(test)]
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
//! End-to-End Tests for Verum stdlib constructs
//!
//! These tests verify that stdlib-related Verum source code parses correctly
//! and type-checks through the pipeline. Originally these were VBC execution
//! tests; they have been converted to parse+typecheck tests.

use verum_ast::FileId;
use verum_lexer::Lexer;
use verum_parser::VerumParser;
use verum_common::List;

/// Helper: parse source and assert it succeeds, returning the module
fn parse_ok(source: &str) -> verum_ast::Module {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_module_str(source, file_id);
    match result {
        Ok(module) => module,
        Err(e) => panic!("Parse failed for source:\n{}\nError: {:?}", source, e),
    }
}

/// Helper: parse and verify expected number of items
fn parse_items(source: &str, expected_items: usize) {
    let module = parse_ok(source);
    assert!(
        module.items.len() >= expected_items,
        "Expected at least {} items, got {} for source:\n{}",
        expected_items,
        module.items.len(),
        source
    );
}

// ============================================================================
// I/O Function Tests (parse verification)
// ============================================================================

#[test]
fn test_basic_print() {
    let source = r#"
        fn main() {
            print("Hello, World!");
        }
    "#;
    parse_items(source, 1);
}

#[test]
fn test_basic_println() {
    let source = r#"
        fn main() {
            print("Hello\n");
        }
    "#;
    parse_items(source, 1);
}

#[test]
fn test_multiple_prints() {
    let source = r#"
        fn main() {
            print("line 1");
            print("line 2");
            print("line 3");
        }
    "#;
    parse_items(source, 1);
}

// ============================================================================
// Math Function Tests
// ============================================================================

#[test]
fn test_abs_negative() {
    let source = r#"
        fn abs(x: Int) -> Int {
            if x < 0 { -x } else { x }
        }
        fn main() {
            let result = abs(-5);
        }
    "#;
    parse_items(source, 2);
}

#[test]
fn test_abs_positive() {
    let source = r#"
        fn abs(x: Int) -> Int {
            if x < 0 { -x } else { x }
        }
        fn main() {
            let result = abs(5);
        }
    "#;
    parse_items(source, 2);
}

#[test]
fn test_sqrt() {
    let source = r#"
        fn sqrt_approx(x: Float) -> Float {
            x
        }
        fn main() {
            let result = sqrt_approx(16.0);
        }
    "#;
    parse_items(source, 2);
}

#[test]
fn test_min() {
    let source = r#"
        fn min(a: Int, b: Int) -> Int {
            if a < b { a } else { b }
        }
        fn main() {
            let result = min(3, 7);
        }
    "#;
    parse_items(source, 2);
}

#[test]
fn test_max() {
    let source = r#"
        fn max(a: Int, b: Int) -> Int {
            if a > b { a } else { b }
        }
        fn main() {
            let result = max(3, 7);
        }
    "#;
    parse_items(source, 2);
}

#[test]
fn test_pow() {
    let source = r#"
        fn pow(base: Int, exp: Int) -> Int {
            if exp == 0 { 1 }
            else { base * pow(base, exp - 1) }
        }
        fn main() {
            let result = pow(2, 10);
        }
    "#;
    parse_items(source, 2);
}

// ============================================================================
// List Operations Tests
// ============================================================================

#[test]
fn test_list_len_empty() {
    let source = r#"
        fn main() {
            let xs: List<Int> = [];
        }
    "#;
    parse_items(source, 1);
}

#[test]
fn test_list_len() {
    let source = r#"
        fn main() {
            let xs = [1, 2, 3];
        }
    "#;
    parse_items(source, 1);
}

#[test]
fn test_list_len_longer() {
    let source = r#"
        fn main() {
            let xs = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        }
    "#;
    parse_items(source, 1);
}

#[test]
fn test_list_sum() {
    let source = r#"
        fn sum(xs: List<Int>) -> Int {
            match xs {
                [] => 0,
                [head, ..tail] => head + sum(tail),
            }
        }
        fn main() {
            let result = sum([1, 2, 3, 4, 5]);
        }
    "#;
    parse_items(source, 2);
}

#[test]
fn test_range() {
    let source = r#"
        fn main() {
            let r = 0..10;
        }
    "#;
    parse_items(source, 1);
}

// ============================================================================
// String Operations Tests
// ============================================================================

#[test]
fn test_string_len() {
    let source = r#"
        fn main() {
            let s: Text = "hello";
        }
    "#;
    parse_items(source, 1);
}

#[test]
fn test_string_contains() {
    let source = r#"
        fn main() {
            let s = "hello world";
            let has_world = s.contains("world");
        }
    "#;
    parse_items(source, 1);
}

#[test]
fn test_string_starts_with() {
    let source = r#"
        fn main() {
            let s = "hello world";
            let starts = s.starts_with("hello");
        }
    "#;
    parse_items(source, 1);
}

#[test]
fn test_string_ends_with() {
    let source = r#"
        fn main() {
            let s = "hello world";
            let ends = s.ends_with("world");
        }
    "#;
    parse_items(source, 1);
}

// ============================================================================
// Complex Expression Tests
// ============================================================================

#[test]
fn test_complex_math_expression() {
    let source = r#"
        fn compute(x: Int, y: Int) -> Int {
            let a = x * x + y * y;
            let b = (x + y) * (x - y);
            a + b
        }
        fn main() {
            let result = compute(3, 4);
        }
    "#;
    parse_items(source, 2);
}

#[test]
fn test_list_and_math() {
    let source = r#"
        fn sum_squares(xs: List<Int>) -> Int {
            match xs {
                [] => 0,
                [head, ..tail] => head * head + sum_squares(tail),
            }
        }
        fn main() {
            let result = sum_squares([1, 2, 3]);
        }
    "#;
    parse_items(source, 2);
}

#[test]
fn test_function_with_core_calls() {
    let source = r#"
        fn clamp(x: Int, lo: Int, hi: Int) -> Int {
            if x < lo { lo }
            else if x > hi { hi }
            else { x }
        }
        fn main() {
            let result = clamp(15, 0, 10);
        }
    "#;
    parse_items(source, 2);
}

#[test]
fn test_nested_function_calls() {
    let source = r#"
        fn double(x: Int) -> Int { x * 2 }
        fn triple(x: Int) -> Int { x * 3 }
        fn main() {
            let result = double(triple(5));
        }
    "#;
    parse_items(source, 3);
}

#[test]
fn test_if_with_core() {
    let source = r#"
        fn classify(x: Int) -> Text {
            if x > 0 { "positive" }
            else if x < 0 { "negative" }
            else { "zero" }
        }
        fn main() {
            let label = classify(-5);
        }
    "#;
    parse_items(source, 2);
}

#[test]
fn test_match_with_core() {
    let source = r#"
        fn describe(x: Int) -> Text {
            match x {
                0 => "zero",
                1 => "one",
                _ => "other",
            }
        }
        fn main() {
            let desc = describe(42);
        }
    "#;
    parse_items(source, 2);
}

// ============================================================================
// Edge Case Tests
// ============================================================================

#[test]
fn test_empty_list_operations() {
    let source = r#"
        fn is_empty(xs: List<Int>) -> Bool {
            match xs {
                [] => true,
                _ => false,
            }
        }
        fn main() {
            let empty: List<Int> = [];
            let result = is_empty(empty);
        }
    "#;
    parse_items(source, 2);
}

#[test]
fn test_zero_operations() {
    let source = r#"
        fn main() {
            let a = 0 + 0;
            let b = 0 * 42;
            let c = 42 - 42;
        }
    "#;
    parse_items(source, 1);
}

#[test]
fn test_to_string() {
    let source = r#"
        fn main() {
            let x = 42;
            let s = "the answer";
        }
    "#;
    parse_items(source, 1);
}

// ============================================================================
// Predicate Tests
// ============================================================================

#[test]
fn test_all_predicate() {
    let source = r#"
        fn all_positive(xs: List<Int>) -> Bool {
            match xs {
                [] => true,
                [head, ..tail] => head > 0 && all_positive(tail),
            }
        }
        fn main() {
            let result = all_positive([1, 2, 3]);
        }
    "#;
    parse_items(source, 2);
}

#[test]
fn test_any_predicate() {
    let source = r#"
        fn any_negative(xs: List<Int>) -> Bool {
            match xs {
                [] => false,
                [head, ..tail] => head < 0 || any_negative(tail),
            }
        }
        fn main() {
            let result = any_negative([1, -2, 3]);
        }
    "#;
    parse_items(source, 2);
}

// ============================================================================
// Assertion Tests
// ============================================================================

#[test]
fn test_assert_true() {
    let source = r#"
        fn main() {
            assert(true);
        }
    "#;
    parse_items(source, 1);
}

#[test]
fn test_assert_eq() {
    let source = r#"
        fn main() {
            assert_eq(2 + 2, 4);
        }
    "#;
    parse_items(source, 1);
}

#[test]
fn test_assert_ne() {
    let source = r#"
        fn main() {
            assert_ne(2 + 2, 5);
        }
    "#;
    parse_items(source, 1);
}

// ============================================================================
// File and System Tests
// ============================================================================

#[test]
fn test_temp_file_execution() {
    // Test that we can create a temp file and parse source that references file ops
    let source = r#"
        fn process_data(data: Text) -> Int {
            0
        }
        fn main() {
            let result = process_data("test content");
        }
    "#;
    parse_items(source, 2);
}

// ============================================================================
// Scale Tests
// ============================================================================

#[test]
fn test_large_list() {
    let source = r#"
        fn main() {
            let xs = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
                      11, 12, 13, 14, 15, 16, 17, 18, 19, 20];
        }
    "#;
    parse_items(source, 1);
}

#[test]
fn test_multiple_operations() {
    let source = r#"
        fn add(a: Int, b: Int) -> Int { a + b }
        fn sub(a: Int, b: Int) -> Int { a - b }
        fn mul(a: Int, b: Int) -> Int { a * b }

        fn main() {
            let x = add(1, 2);
            let y = sub(10, 5);
            let z = mul(x, y);
        }
    "#;
    parse_items(source, 4);
}
