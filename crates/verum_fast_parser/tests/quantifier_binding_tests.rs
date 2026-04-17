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
//! Tests for enhanced quantifier binding syntax.
//!
//! Grammar Reference:
//!   forall_expr = 'forall' quantifier_binding { ',' quantifier_binding } '.' expression
//!   exists_expr = 'exists' quantifier_binding { ',' quantifier_binding } '.' expression
//!   quantifier_binding = pattern [ ':' type_expr ] [ 'in' expression ] [ 'where' expression ]

use verum_ast::span::FileId;
use verum_ast::{Expr, ExprKind};
use verum_common::{List, Maybe};
use verum_fast_parser::VerumParser;

/// Helper to parse a single expression
fn parse_expr(source: &str) -> Result<Expr, String> {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_expr_str(source, file_id)
        .map_err(|e| format!("{:?}", e))
}

/// Helper to check that parsing succeeds
fn assert_parses(source: &str) {
    match parse_expr(source) {
        Ok(_) => {}
        Err(e) => panic!("Failed to parse '{}': {}", source, e),
    }
}

/// Helper to check that parsing fails
fn assert_parse_fails(source: &str) {
    if let Ok(_) = parse_expr(source) { panic!("Expected parse failure for '{}'", source) }
}

// ============================================================================
// Basic Type-Annotated Quantifiers
// ============================================================================

#[test]
fn test_forall_basic_type() {
    assert_parses("forall x: Int . x > 0");
}

#[test]
fn test_exists_basic_type() {
    assert_parses("exists x: Int . x == 42");
}

#[test]
fn test_forall_complex_body() {
    assert_parses("forall n: Int . n >= 0 || n < 0");
}

// ============================================================================
// Domain-Based Quantification (forall x in collection)
// ============================================================================

#[test]
fn test_forall_in_identifier() {
    assert_parses("forall x in items . x > 0");
}

#[test]
fn test_forall_in_array_literal() {
    assert_parses("forall x in [1, 2, 3] . x > 0");
}

#[test]
fn test_forall_in_range() {
    assert_parses("forall i in 1..10 . i > 0");
}

#[test]
fn test_forall_in_range_inclusive() {
    assert_parses("forall i in 1..=10 . i >= 1");
}

#[test]
fn test_exists_in_collection() {
    assert_parses("exists n in numbers . n == 5");
}

#[test]
fn test_exists_in_method_call() {
    assert_parses("exists x in items.iter() . x > 0");
}

// ============================================================================
// Combined Type and Domain (forall x: T in collection)
// ============================================================================

#[test]
fn test_forall_type_and_domain() {
    assert_parses("forall x: Int in items . x > 0");
}

#[test]
fn test_exists_type_and_domain() {
    assert_parses("exists n: Int in numbers . n == 42");
}

#[test]
fn test_forall_generic_type_and_domain() {
    assert_parses("forall elem: T in collection . elem != null");
}

// ============================================================================
// Guard Clauses (forall x: T where guard)
// ============================================================================

#[test]
fn test_forall_type_with_guard() {
    assert_parses("forall x: Int where x > 0 . x * x > 0");
}

#[test]
fn test_exists_type_with_guard() {
    assert_parses("exists n: Int where n > 0 . n * n == 16");
}

#[test]
fn test_forall_compound_guard() {
    assert_parses("forall x: Int where x >= 0 && x <= 100 . x + 1 > x");
}

// ============================================================================
// Full Syntax (forall x: T in collection where guard)
// ============================================================================

#[test]
fn test_forall_type_domain_guard() {
    assert_parses("forall x: Int in items where x > 0 . x >= 1");
}

#[test]
fn test_exists_type_domain_guard() {
    assert_parses("exists n: Int in data where n > 0 . n % 2 == 0");
}

#[test]
fn test_forall_full_syntax_complex() {
    assert_parses("forall elem: Int in list where elem >= 0 && elem <= 100 . elem * elem <= 10000");
}

// ============================================================================
// Multiple Bindings
// ============================================================================

#[test]
fn test_forall_two_bindings() {
    assert_parses("forall x: Int, y: Int . x + y == y + x");
}

#[test]
fn test_forall_three_bindings() {
    assert_parses("forall a: Int, b: Int, c: Int . (a + b) + c == a + (b + c)");
}

#[test]
fn test_exists_two_bindings() {
    assert_parses("exists x: Int, y: Int . x + y == 10");
}

#[test]
fn test_multiple_bindings_with_domains() {
    assert_parses("forall x in xs, y in ys . x < y");
}

#[test]
fn test_multiple_bindings_mixed() {
    assert_parses("forall x: Int, y in ys . x + y > 0");
}

#[test]
fn test_multiple_bindings_with_guards() {
    assert_parses("forall x: Int where x > 0, y: Int where y > 0 . x + y > 0");
}

// ============================================================================
// Nested Quantifiers
// ============================================================================

#[test]
fn test_nested_forall() {
    assert_parses("forall x: Int . forall y: Int . x + y == y + x");
}

#[test]
fn test_nested_forall_exists() {
    assert_parses("forall x: Int . exists y: Int . y > x");
}

#[test]
fn test_nested_with_domains() {
    assert_parses("forall x in xs . forall y in ys . x < y");
}

// ============================================================================
// Quantifiers in Expressions
// ============================================================================

#[test]
fn test_quantifier_in_let() {
    assert_parses("(forall x: Int . x == x)");
}

#[test]
fn test_quantifier_conjunction() {
    assert_parses("(forall x: Int . x > 0) && (exists y: Int . y < 0)");
}

#[test]
fn test_quantifier_in_function_call() {
    assert_parses("assert(forall x: Int . x == x)");
}

// ============================================================================
// Error Cases - Should Fail to Parse
// ============================================================================

#[test]
fn test_fail_missing_type_and_domain() {
    // Should fail: quantifier binding needs either type or domain
    assert_parse_fails("forall x . x > 0");
}

#[test]
fn test_fail_missing_dot() {
    // Should fail: missing dot before body
    assert_parse_fails("forall x: Int x > 0");
}

#[test]
fn test_fail_empty_binding() {
    // Should fail: no binding at all
    assert_parse_fails("forall . true");
}

#[test]
fn test_where_with_in_containment() {
    // Note: This actually parses successfully because `in` is also a binary operator
    // (containment check). So `where x > 0 in items` parses as `where (x > 0) in items`
    // which checks if the boolean `x > 0` is contained in `items`.
    // This is syntactically valid, though semantically unusual.
    assert_parses("forall x: Int where x > 0 in items . x >= 0");
}

#[test]
fn test_fail_double_in() {
    // Should fail: cannot have two 'in' clauses
    assert_parse_fails("forall x in a in b . x > 0");
}

#[test]
fn test_fail_double_where() {
    // Should fail: cannot have two 'where' clauses
    assert_parse_fails("forall x: Int where x > 0 where x < 100 . x >= 0");
}

#[test]
fn test_fail_in_without_expression() {
    // Should fail: 'in' without domain expression
    assert_parse_fails("forall x: Int in . x > 0");
}

#[test]
fn test_fail_where_without_expression() {
    // Should fail: 'where' without guard expression
    assert_parse_fails("forall x: Int where . x > 0");
}

#[test]
fn test_fail_trailing_comma_bindings() {
    // Should fail: trailing comma after last binding
    assert_parse_fails("forall x: Int, y: Int, . x + y > 0");
}

// ============================================================================
// Pattern Bindings
// ============================================================================

#[test]
fn test_forall_tuple_pattern() {
    assert_parses("forall (a, b): (Int, Int) . a + b >= a");
}

#[test]
fn test_forall_pattern_in_domain() {
    assert_parses("forall (x, y) in pairs . x < y");
}

// ============================================================================
// Complex Domain Expressions
// ============================================================================

#[test]
fn test_domain_filter_chain() {
    assert_parses("forall x in items.filter(|n| n > 0) . x > 0");
}

#[test]
fn test_domain_map_chain() {
    assert_parses("forall x in items.map(|n| n * 2) . x % 2 == 0");
}

#[test]
fn test_domain_enumerate() {
    assert_parses("forall (i, v) in items.enumerate() . i >= 0");
}
