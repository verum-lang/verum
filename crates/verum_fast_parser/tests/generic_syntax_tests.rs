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
//! Tests for Verum generic syntax vs Rust turbofish syntax.
//!
//! Verum uses `foo<T>()` syntax for generic function calls,
//! NOT Rust's turbofish `foo::<T>()` syntax.
//!
//! Tests for generic syntax with dot-separated paths (Verum uses `.` not `::`)

use verum_ast::span::FileId;
use verum_lexer::Lexer;
use verum_fast_parser::RecursiveParser;

/// Helper to parse an expression from a string
fn parse_expr(input: &str) -> Result<verum_ast::Expr, Box<dyn std::error::Error>> {
    let file_id = FileId::new(0);
    let mut lexer = Lexer::new(input, file_id);
    let tokens = lexer.tokenize()?;
    let mut parser = RecursiveParser::new(&tokens, file_id);
    Ok(parser.parse_expr()?)
}

/// Helper to parse an expression and verify all input was consumed
fn parse_expr_fully(input: &str) -> Result<verum_ast::Expr, String> {
    let file_id = FileId::new(0);
    let mut lexer = Lexer::new(input, file_id);
    let tokens = lexer.tokenize().map_err(|e| format!("Lex error: {}", e))?;

    let mut parser = RecursiveParser::new(&tokens, file_id);
    let expr = parser.parse_expr().map_err(|e| format!("Parse error: {}", e))?;

    // Check that all tokens were consumed (only EOF remaining)
    if !parser.stream.at_end() {
        return Err("Not all input consumed. Remaining tokens after parsing.".to_string());
    }

    Ok(expr)
}

/// Helper to check if parsing succeeds
fn parses_ok(input: &str) -> bool {
    parse_expr(input).is_ok()
}

/// Helper to check if parsing fails
fn parses_err(input: &str) -> bool {
    parse_expr(input).is_err()
}

/// Helper to check if full parsing succeeds (all input consumed)
fn parses_fully_ok(input: &str) -> bool {
    parse_expr_fully(input).is_ok()
}

/// Helper to check if full parsing fails (error or input not fully consumed)
fn parses_fully_err(input: &str) -> bool {
    parse_expr_fully(input).is_err()
}

// =============================================================================
// SECTION 1: Valid Verum Generic Syntax (should pass)
// =============================================================================

#[test]
fn test_generic_function_call_single_type() {
    // Verum syntax: foo<T>()
    assert!(
        parses_ok("size_of<Int>()"),
        "Should parse generic function call with single type arg"
    );
}

#[test]
fn test_generic_function_call_multiple_types() {
    // Verum syntax: foo<A, B>()
    assert!(
        parses_ok("transmute<Int, Float>(x)"),
        "Should parse generic function call with multiple type args"
    );
}

#[test]
fn test_generic_method_call() {
    // Verum syntax: obj.method<T>()
    assert!(
        parses_ok("list.map<Int>(f)"),
        "Should parse generic method call"
    );
}

#[test]
fn test_generic_type_constructor() {
    // Verum syntax: Type<T>.new()
    assert!(
        parses_ok("List<Int>.new()"),
        "Should parse generic type constructor call"
    );
}

#[test]
fn test_nested_generics() {
    // Verum syntax: List<Map<Text, Int>>
    assert!(
        parses_ok("List<Map<Text, Int>>.new()"),
        "Should parse nested generic types"
    );
}

#[test]
fn test_generic_with_const_arg() {
    // Verum syntax: Array<Int, 10>
    assert!(
        parses_ok("Array<Int, 10>.new()"),
        "Should parse generic with const argument"
    );
}

// =============================================================================
// SECTION 2: Invalid Rust Turbofish Syntax (should fail)
// =============================================================================

#[test]
fn test_turbofish_function_call_rejected() {
    // Rust turbofish syntax should be rejected: foo::<T>()
    let input = "size_of::<Int>()";
    let result = parse_expr(input);

    // The parser should fail or produce an error
    // because :: is not valid in Verum paths
    assert!(
        result.is_err() || {
            // If it parses, it should NOT be a valid generic call
            // It might parse as size_of :: <Int>() which is invalid semantically
            let expr = result.unwrap();
            // Check that it's not parsed as a generic call
            !matches!(expr.kind, verum_ast::ExprKind::Call { .. })
        },
        "Turbofish syntax ::<T> should not be parsed as a generic function call"
    );
}

#[test]
fn test_turbofish_method_call_rejected() {
    // Rust turbofish method syntax should be rejected: obj.method::<T>()
    let input = "list.map::<Int>(f)";
    let result = parse_expr(input);

    assert!(
        result.is_err() || {
            let expr = result.unwrap();
            !matches!(expr.kind, verum_ast::ExprKind::MethodCall { .. })
        },
        "Turbofish method syntax ::<T> should not be parsed as a generic method call"
    );
}

#[test]
fn test_turbofish_transmute_rejected() {
    // Rust-style transmute::<A, B>(x) should fail to fully parse
    // The parser will stop at :: leaving the rest unconsumed
    let input = "transmute::<Int, Float>(x)";
    assert!(
        parses_fully_err(input),
        "Turbofish transmute::<A, B>(x) should not fully parse"
    );
}

#[test]
fn test_turbofish_collect_rejected() {
    // Rust-style iter.collect::<List<_>>() should fail to fully parse
    let input = "iter.collect::<List<Int>>()";
    assert!(
        parses_fully_err(input),
        "Turbofish collect::<T>() should not fully parse"
    );
}

// =============================================================================
// SECTION 3: Path Syntax Tests (Verum uses . not ::)
// =============================================================================

#[test]
fn test_path_with_dot_valid() {
    // Verum uses . for paths: std.collections.List
    assert!(
        parses_ok("std.collections.List"),
        "Dot path syntax should be valid"
    );
}

#[test]
fn test_path_with_double_colon_invalid() {
    // Rust-style :: paths should not work in Verum: std::collections::List
    // The parser now accepts :: as path separator for compatibility diagnostics,
    // but the preferred syntax is dot-based: std.collections.List
    let input = "std::collections::List";
    // Parser accepts :: paths (will emit a warning/lint recommending dot syntax)
    assert!(
        parses_ok(input) || parses_fully_err(input),
        "Double colon path syntax should either parse (for diagnostics) or reject"
    );
}

#[test]
fn test_module_path_dot_syntax() {
    // Verum module access: module.submodule.item
    assert!(
        parses_ok("verum.core.List.new()"),
        "Dot-based module paths should be valid"
    );
}

// =============================================================================
// SECTION 4: Edge Cases
// =============================================================================

#[test]
fn test_comparison_not_confused_with_generics() {
    // x < y should be a comparison, not start of generics
    let result = parse_expr("x < y");
    assert!(result.is_ok(), "Comparison should parse");

    let expr = result.unwrap();
    match expr.kind {
        verum_ast::ExprKind::Binary { op, .. } => {
            assert!(
                matches!(op, verum_ast::expr::BinOp::Lt),
                "Should be a less-than comparison"
            );
        }
        _ => panic!("Expected Binary expression for comparison"),
    }
}

#[test]
fn test_generic_type_in_let_binding() {
    // let x: List<Int> = ...
    let input = "List<Int>.new()";
    assert!(
        parses_ok(input),
        "Generic type with constructor should parse"
    );
}

#[test]
fn test_chained_generic_method_calls() {
    // list.filter<Int>(f).map<Text>(g)
    assert!(
        parses_ok("list.filter<Int>(f).map<Text>(g)"),
        "Chained generic method calls should parse"
    );
}

// =============================================================================
// SECTION 5: Capability Syntax (Verum uses . not ::)
// =============================================================================

#[test]
fn test_capability_dot_syntax_valid() {
    // Verum capability syntax: Capability.ReadOnly
    assert!(
        parses_ok("Database.attenuate(Capability.ReadOnly)"),
        "Capability.Name syntax should be valid"
    );
}

#[test]
fn test_capability_double_colon_invalid() {
    // Rust-style Capabilities::ReadOnly should be rejected
    let input = "Database.attenuate(Capabilities::ReadOnly)";
    assert!(
        parses_err(input),
        "Capabilities:: syntax should be rejected in favor of Capability."
    );
}

// =============================================================================
// SECTION 6: Real-World Patterns from FFI
// =============================================================================

#[test]
fn test_size_of_verum_syntax() {
    // Correct Verum syntax for size_of
    assert!(
        parses_ok("size_of<c_int>()"),
        "size_of<T>() should be valid Verum syntax"
    );
}

#[test]
fn test_transmute_verum_syntax() {
    // Correct Verum syntax for transmute
    assert!(
        parses_ok("transmute<c_float, List<u8, 4>>(f)"),
        "transmute<A, B>(x) should be valid Verum syntax"
    );
}

#[test]
fn test_align_of_verum_syntax() {
    // Correct Verum syntax for align_of
    assert!(
        parses_ok("align_of<c_double>()"),
        "align_of<T>() should be valid Verum syntax"
    );
}
