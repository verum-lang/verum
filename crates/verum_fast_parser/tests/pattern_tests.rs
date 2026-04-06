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
// Tests for pattern parsing
//
// Tests for pattern syntax: literals, bindings, tuples, records, variants, guards, etc.
// This module tests parsing of all Verum pattern forms including:
// - Literal patterns
// - Identifier patterns (with and without mutable)
// - Wildcard patterns (_)
// - Rest patterns (..)
// - Tuple patterns
// - Array/slice patterns
// - Record patterns
// - Variant patterns (enum patterns)
// - Guard patterns
// - Or patterns
// - Range patterns
// - Reference patterns

use verum_ast::{FileId, Module};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

/// Parse patterns by wrapping code in a function body
/// Patterns appear in let statements, match arms, for loops, function params
fn parse_pattern_in_context(source: &str) -> Module {
    // Wrap in a function body since patterns only appear inside functions
    let wrapped = format!("fn __test__() {{ {} }}", source);
    let file_id = FileId::new(0);
    let lexer = Lexer::new(&wrapped, file_id);
    let parser = VerumParser::new();
    parser
        .parse_module(lexer, file_id)
        .unwrap_or_else(|_| panic!("Failed to parse: {}", source))
}

// === LITERAL PATTERN TESTS ===

#[test]
fn test_parse_pattern_integer_literal() {
    let module = parse_pattern_in_context("let 42 = x;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_float_literal() {
    let module = parse_pattern_in_context("let 3.14 = x;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_string_literal() {
    let module = parse_pattern_in_context(r#"let "hello" = x;"#);
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_char_literal() {
    let module = parse_pattern_in_context("let 'a' = x;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_bool_true() {
    let module = parse_pattern_in_context("let true = x;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_bool_false() {
    let module = parse_pattern_in_context("let false = x;");
    assert_eq!(module.items.len(), 1);
}

// === IDENTIFIER PATTERN TESTS ===

#[test]
fn test_parse_pattern_identifier() {
    let module = parse_pattern_in_context("let x = value;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_identifier_mutable() {
    let module = parse_pattern_in_context("let mut x = value;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_identifier_with_binding() {
    let module = parse_pattern_in_context("let x @ Some(v) = opt;");
    assert_eq!(module.items.len(), 1);
}

// === WILDCARD PATTERN TESTS ===

#[test]
fn test_parse_pattern_wildcard() {
    let module = parse_pattern_in_context("let _ = x;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_wildcard_in_tuple() {
    let module = parse_pattern_in_context("let (_, y) = pair;");
    assert_eq!(module.items.len(), 1);
}

// === REST PATTERN TESTS ===

#[test]
fn test_parse_pattern_rest() {
    let module = parse_pattern_in_context("let [first, ..] = arr;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_rest_middle() {
    let module = parse_pattern_in_context("let [first, .., last] = arr;");
    assert_eq!(module.items.len(), 1);
}

// === TUPLE PATTERN TESTS ===

#[test]
fn test_parse_pattern_tuple_two() {
    let module = parse_pattern_in_context("let (x, y) = pair;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_tuple_three() {
    let module = parse_pattern_in_context("let (x, y, z) = triple;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_tuple_nested() {
    let module = parse_pattern_in_context("let ((x, y), z) = nested;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_tuple_with_wildcard() {
    let module = parse_pattern_in_context("let (x, _, z) = triple;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_tuple_with_rest() {
    let module = parse_pattern_in_context("let (x, ..) = tuple;");
    assert_eq!(module.items.len(), 1);
}

// === ARRAY/SLICE PATTERN TESTS ===

#[test]
fn test_parse_pattern_array_empty() {
    let module = parse_pattern_in_context("let [] = arr;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_array_single() {
    let module = parse_pattern_in_context("let [x] = arr;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_array_multiple() {
    let module = parse_pattern_in_context("let [x, y, z] = arr;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_array_with_rest() {
    let module = parse_pattern_in_context("let [x, ..] = arr;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_array_rest_at_end() {
    let module = parse_pattern_in_context("let [x, y, ..] = arr;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_array_with_wildcards() {
    let module = parse_pattern_in_context("let [_, y, _] = arr;");
    assert_eq!(module.items.len(), 1);
}

// === RECORD PATTERN TESTS ===
// NOTE: Record patterns require a type name prefix in Verum
// Anonymous destructuring like `let { x } = record;` is not supported
// Use `let Point { x } = record;` instead

#[test]
fn test_parse_pattern_record_simple() {
    let module = parse_pattern_in_context("let Point { x } = record;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_record_multiple_fields() {
    let module = parse_pattern_in_context("let Point { x, y, z } = record;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_record_with_rest() {
    let module = parse_pattern_in_context("let Point { x, .. } = record;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_record_renamed() {
    let module = parse_pattern_in_context("let Point { x: a, y: b } = record;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_record_mixed() {
    let module = parse_pattern_in_context("let Point { x, y: b, .. } = record;");
    assert_eq!(module.items.len(), 1);
}

// === ENUM/VARIANT PATTERN TESTS ===

#[test]
fn test_parse_pattern_variant_unit() {
    let module = parse_pattern_in_context("let None = opt;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_variant_with_data() {
    let module = parse_pattern_in_context("let Some(x) = opt;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_variant_nested() {
    let module = parse_pattern_in_context("let Some((x, y)) = opt;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_variant_ok() {
    let module = parse_pattern_in_context("let Ok(value) = result;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_variant_err() {
    let module = parse_pattern_in_context("let Err(err) = result;");
    assert_eq!(module.items.len(), 1);
}

// NOTE: Qualified paths (with ::) in patterns are implemented
#[test]
fn test_parse_pattern_variant_qualified() {
    let module = parse_pattern_in_context("let MyEnum.Variant(x) = val;");
    assert_eq!(module.items.len(), 1);
}

// === OR PATTERN TESTS ===

#[test]
fn test_parse_pattern_or_simple() {
    let module = parse_pattern_in_context("let x | y = val;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_or_three() {
    let module = parse_pattern_in_context("let x | y | z = val;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_or_with_literals() {
    let module = parse_pattern_in_context("let 1 | 2 | 3 = num;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_or_variants() {
    let module = parse_pattern_in_context("let Ok(x) | Err(x) = result;");
    assert_eq!(module.items.len(), 1);
}

// === RANGE PATTERN TESTS ===

#[test]
fn test_parse_pattern_range_inclusive() {
    let module = parse_pattern_in_context("let 1..=10 = x;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_range_exclusive() {
    let module = parse_pattern_in_context("let 1..10 = x;");
    assert_eq!(module.items.len(), 1);
}

// === REFERENCE PATTERN TESTS ===

#[test]
fn test_parse_pattern_reference() {
    let module = parse_pattern_in_context("let &x = reference;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_reference_mutable() {
    let module = parse_pattern_in_context("let &mut x = reference;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_reference_with_pattern() {
    let module = parse_pattern_in_context("let &Some(x) = opt_ref;");
    assert_eq!(module.items.len(), 1);
}

// === COMPLEX NESTED PATTERNS ===

#[test]
fn test_parse_pattern_deeply_nested() {
    let module = parse_pattern_in_context("let ((x, y), (a, b)) = quad;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_mixed_structures() {
    let module = parse_pattern_in_context("let [(x, y), (a, b)] = pairs;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_variant_with_tuple_record() {
    let module = parse_pattern_in_context("let Some(Point { x, y }) = opt;");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_record_with_nested_variant() {
    let module = parse_pattern_in_context("let Outer { field: Some(x), y } = record;");
    assert_eq!(module.items.len(), 1);
}

// === PATTERNS IN MATCH EXPRESSIONS ===

#[test]
fn test_parse_pattern_in_match_literal() {
    let module = parse_pattern_in_context("match x { 1 => {}, _ => {} };");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_in_match_variant() {
    let module = parse_pattern_in_context("match opt { Some(x) => {}, None => {} };");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_in_match_tuple() {
    let module = parse_pattern_in_context("match pair { (x, y) => {}, _ => {} };");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_in_match_or() {
    let module = parse_pattern_in_context("match x { 1 | 2 => {}, _ => {} };");
    assert_eq!(module.items.len(), 1);
}

// === PATTERNS IN FOR LOOPS ===

#[test]
fn test_parse_pattern_in_for_simple() {
    let module = parse_pattern_in_context("for x in items { };");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_in_for_tuple() {
    let module = parse_pattern_in_context("for (k, v) in pairs { };");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_in_for_wildcard() {
    let module = parse_pattern_in_context("for _ in items { };");
    assert_eq!(module.items.len(), 1);
}

// === GUARD PATTERNS ===

#[test]
fn test_parse_pattern_with_guard() {
    let module = parse_pattern_in_context("match x { v if v > 0 => {}, _ => {} };");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn test_parse_pattern_with_complex_guard() {
    let module =
        parse_pattern_in_context("match (x, y) { (a, b) if a > 0 && b < 10 => {}, _ => {} };");
    assert_eq!(module.items.len(), 1);
}
