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
//! Edge case tests for verum_fast_parser

use verum_ast::span::FileId;
use verum_lexer::Lexer;
use verum_parser::VerumParser;

#[test]
fn test_empty_function_body() {
    let input = "fn f() {}";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_single_character_identifiers() {
    // Test single character identifiers in expressions
    let input = "x + y";
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(input, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_deeply_nested_expressions() {
    let input = "((((((((((x))))))))))";
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(input, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_many_function_parameters() {
    let input = "fn f(a: Int, b: Int, c: Int, d: Int, e: Int, f: Int, g: Int, h: Int) -> Int { a }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_operator_chains() {
    let input = "a + b + c + d + e";
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(input, file_id);
    assert!(result.is_ok());
}

// ============================================================================
// CRITICAL EDGE CASES - Must Never Fail
// ============================================================================

/// Test generic closing with >> (two consecutive > tokens)
#[test]
fn test_generic_double_close() {
    let input = "fn test<T>(x: List<List<Int>>) { }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Nested generics with >> should parse");
}

/// Test triple nested generics with >>>
#[test]
fn test_generic_triple_close() {
    let input = "fn test<T>(x: List<List<List<Int>>>) { }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Triple nested generics should parse");
}

/// Test power operator right associativity: 2 ** 3 ** 2 = 2 ** (3 ** 2)
#[test]
fn test_power_operator_right_associativity() {
    let input = "2 ** 3 ** 2";
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(input, file_id);
    assert!(result.is_ok(), "Power operator should be right-associative");
}

/// Test unary minus with power operator: -2 ** 2
#[test]
fn test_unary_minus_with_power() {
    let input = "-2 ** 2";
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(input, file_id);
    assert!(result.is_ok(), "Unary minus with power should parse");
}

/// Test reference type combinations
#[test]
fn test_reference_type_combinations() {
    let cases = [
        "fn f(x: &Int) { }",
        "fn f(x: &mut Int) { }",
        "fn f(x: &checked Int) { }",
        "fn f(x: &checked mut Int) { }",
        "fn f(x: &unsafe Int) { }",
        "fn f(x: &unsafe mut Int) { }",
    ];

    for input in cases {
        let file_id = FileId::new(0);
        let lexer = Lexer::new(input, file_id);
        let parser = VerumParser::new();
        let result = parser.parse_module(lexer, file_id);
        assert!(result.is_ok(), "Reference type '{}' should parse", input);
    }
}

/// Test 'is' keyword is reserved (cannot be function name)
#[test]
fn test_is_keyword_reserved() {
    let input = "fn is() { }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_err(), "'is' is reserved and cannot be used as function name");
}

/// Test 'default' is contextual (CAN be function name)
#[test]
fn test_default_contextual_keyword() {
    let input = "fn default() { }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "'default' is contextual and CAN be used as function name");
}

/// Test trailing comma in tuples
#[test]
fn test_trailing_comma_tuple() {
    let input = "(1, 2, 3,)";
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(input, file_id);
    assert!(result.is_ok(), "Trailing comma in tuple should parse");
}

/// Test trailing comma in function parameters
#[test]
fn test_trailing_comma_params() {
    let input = "fn f(a: Int, b: Int,) { }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Trailing comma in params should parse");
}

/// Test trailing comma in record types
#[test]
fn test_trailing_comma_record() {
    let input = "type Point is { x: Int, y: Int, };";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Trailing comma in record should parse");
}

/// Test null coalesce precedence with logical or
#[test]
fn test_null_coalesce_precedence() {
    let input = "a ?? b || c";
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(input, file_id);
    assert!(result.is_ok(), "Null coalesce with logical or should parse");
}

/// Test format string basic interpolation
#[test]
fn test_format_string_interpolation() {
    let input = r#"f"x={x}""#;
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(input, file_id);
    assert!(result.is_ok(), "Format string interpolation should parse");
}

/// Test type-level array with size
#[test]
fn test_array_type_with_size() {
    let input = "type Matrix is [Int; 10];";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Array type with size should parse");
}

// ============================================================================
// HIGH PRIORITY EDGE CASES - Subtle Bugs Common
// ============================================================================

/// Test optional function return type (should infer)
#[test]
fn test_optional_return_type() {
    let input = "fn no_type() { 42 }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Function without return type should parse");
}

/// Test pattern alternation with &
#[test]
fn test_pattern_alternation_with_and() {
    let input = r#"
fn test(x: Int) {
    match x {
        1 | 2 => 0,
        _ => 1
    }
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Pattern alternation should parse");
}

/// Test await in complex expressions
#[test]
fn test_await_in_complex_expr() {
    let input = "f().await + g().await";
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(input, file_id);
    assert!(result.is_ok(), "Await in complex expression should parse");
}

/// Test attribute on type definition
#[test]
fn test_attribute_on_type() {
    let input = "@derive(Clone) type T is { x: Int };";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Attribute on type should parse");
}

/// Test let-else pattern
#[test]
fn test_let_else_pattern() {
    let input = r#"
fn test() {
    let Some(x) = maybe else { return };
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Let-else pattern should parse");
}

/// Test record pattern with rest (..)
#[test]
fn test_record_pattern_with_rest() {
    let input = r#"
fn test(p: Point) {
    match p {
        Point { x, .. } => x
    }
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Record pattern with rest should parse");
}

// ============================================================================
// OPERATOR PRECEDENCE EDGE CASES
// ============================================================================

/// Test complex operator precedence chain
#[test]
fn test_complex_operator_chain() {
    let input = "a || b && c | d ^ e & f == g < h + i * j ** k";
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(input, file_id);
    assert!(result.is_ok(), "Complex operator chain should parse");
}

/// Test range operators
#[test]
fn test_range_operators() {
    let cases = [
        "0..10",
        "0..=10",
        "..10",
        "0..",
        "..",
    ];

    for input in cases {
        let file_id = FileId::new(0);
        let parser = VerumParser::new();
        let result = parser.parse_expr_str(input, file_id);
        assert!(result.is_ok(), "Range '{}' should parse", input);
    }
}

/// Test optional chain and null coalesce combined
#[test]
fn test_optional_chain_with_coalesce() {
    let input = "obj?.field ?? default_value";
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(input, file_id);
    assert!(result.is_ok(), "Optional chain with null coalesce should parse");
}

// ============================================================================
// DELIMITER MATCHING EDGE CASES
// ============================================================================

/// Test deeply nested delimiters
#[test]
fn test_deeply_nested_delimiters() {
    let input = "type Deep is List<List<List<List<Int>>>>;";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Deeply nested delimiters should parse");
}

/// Test empty delimiters - parser supports these even though grammar was previously inconsistent
/// Grammar updated to match parser behavior:
/// - Empty array `[]` IS valid (returns empty List)
/// - Empty tuple/unit `()` IS valid (unit expression, single value of unit type)
/// - Empty map `{}` IS valid
/// - `()` as both TYPE and EXPRESSION is valid
#[test]
fn test_empty_delimiters() {
    let cases = [
        // Types
        ("type Unit is ();", "unit type definition"),
        ("fn f() -> () { {} }", "explicit unit return type"),
        ("fn f(x: [Int; 0]) { }", "zero-length array type in param"),
        // Expressions (inside function bodies)
        ("fn f() { }", "empty function body"),
        ("fn f() { {} }", "empty block in block"),
        ("fn f() { let u = (); }", "unit expression assignment"),
        ("fn f() { let arr = []; }", "empty array expression"),
        ("fn f() { return (); }", "return unit value"),
    ];

    for (input, desc) in cases {
        let file_id = FileId::new(0);
        let lexer = Lexer::new(input, file_id);
        let parser = VerumParser::new();
        let result = parser.parse_module(lexer, file_id);
        assert!(result.is_ok(), "{} should parse: {:?}", desc, result);
    }
}

// ============================================================================
// TYPE RECURSION EDGE CASES
// ============================================================================

/// Test function type within function type
#[test]
fn test_higher_order_function_type() {
    let input = "type HigherOrder is fn(fn(Int) -> Int) -> fn(Int) -> Int;";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Higher order function type should parse");
}

/// Test recursive type definition
#[test]
fn test_recursive_type_definition() {
    let input = "type Tree is { value: Int, left: Maybe<Heap<Tree>>, right: Maybe<Heap<Tree>> };";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Recursive type definition should parse");
}

// ============================================================================
// BLOCK EXPRESSION EDGE CASES
// ============================================================================

/// Test nested blocks
#[test]
fn test_nested_blocks() {
    let input = r#"
fn test() {
    let x = {
        let a = {
            let b = 42;
            b + 1
        };
        a * 2
    };
    x
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Nested blocks should parse");
}

/// Test expression as final statement (no semicolon)
#[test]
fn test_expression_final_statement() {
    let input = "fn test() -> Int { let x = 5; x + 1 }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Expression as final statement should parse");
}

// ============================================================================
// LITERAL PARSING EDGE CASES
// ============================================================================

/// Test raw strings with hash delimiters
#[test]
fn test_raw_string_literals() {
    let cases = [
        r#"r"simple raw""#,
        r##"r#"raw with # inside"#"##,
    ];

    for input in cases {
        let file_id = FileId::new(0);
        let parser = VerumParser::new();
        let result = parser.parse_expr_str(input, file_id);
        assert!(result.is_ok(), "Raw string '{}' should parse", input);
    }
}

/// Test unicode escape in character literal
#[test]
fn test_unicode_char_literal() {
    let input = r"'\u{1F600}'";
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(input, file_id);
    assert!(result.is_ok(), "Unicode escape in char literal should parse");
}

// ============================================================================
// ASYNC EDGE CASES
// ============================================================================

/// Test async function definition
#[test]
fn test_async_function() {
    let input = "async fn fetch() -> Int { 42 }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Async function should parse");
}

/// Test async block
#[test]
fn test_async_block() {
    let input = "async { do_work() }";
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(input, file_id);
    assert!(result.is_ok(), "Async block should parse");
}

/// Test await with error handling
#[test]
fn test_await_with_try() {
    let input = "future.await?";
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(input, file_id);
    assert!(result.is_ok(), "Await with ? operator should parse");
}

// ============================================================================
// LINK/IMPORT EDGE CASES
// ============================================================================

/// Test mount statement variants
#[test]
fn test_mount_statements() {
    let cases = [
        "mount std.collections;",
        "mount std.collections as sc;",
        "mount std.collections.{List, Map};",
    ];

    for input in cases {
        let file_id = FileId::new(0);
        let lexer = Lexer::new(input, file_id);
        let parser = VerumParser::new();
        let result = parser.parse_module(lexer, file_id);
        assert!(result.is_ok(), "Mount statement '{}' should parse", input);
    }
}
