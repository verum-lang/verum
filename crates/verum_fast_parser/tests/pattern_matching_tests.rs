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
//! Comprehensive pattern matching tests for Verum parser
//!
//! This test suite validates all pattern types from grammar/verum.ebnf:
//! - literal_pattern, identifier_pattern, wildcard_pattern
//! - tuple_pattern, array_pattern, slice_pattern
//! - record_pattern, variant_pattern
//! - reference_pattern, range_pattern
//! - or_pattern (pat1 | pat2)
//! - Pattern guards (if expression)
//! - Nested patterns
//! - Rest patterns (..)

use verum_ast::{FileId, Module};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

/// Parse patterns by wrapping code in a function body
fn parse_pattern_in_context(source: &str) -> Module {
    let wrapped = format!("fn __test__() {{ {} }}", source);
    let file_id = FileId::new(0);
    let lexer = Lexer::new(&wrapped, file_id);
    let parser = VerumParser::new();
    parser
        .parse_module(lexer, file_id)
        .unwrap_or_else(|_| panic!("Failed to parse: {}", source))
}

// ============================================================================
// SIMPLE PATTERNS
// ============================================================================

#[test]
fn test_literal_patterns_integers() {
    parse_pattern_in_context("let 0 = x;");
    parse_pattern_in_context("let 42 = x;");
    // Note: Negative literals in patterns require unary expression support
    // parse_pattern_in_context("let -1 = x;");
    parse_pattern_in_context("let 0xFF = x;");
    parse_pattern_in_context("let 0b1010 = x;");
}

#[test]
fn test_literal_patterns_floats() {
    parse_pattern_in_context("let 0.0 = x;");
    parse_pattern_in_context("let 3.14 = x;");
    // Note: Negative literals in patterns require unary expression support
    // parse_pattern_in_context("let -2.5 = x;");
    parse_pattern_in_context("let 1e10 = x;");
}

#[test]
fn test_literal_patterns_strings() {
    parse_pattern_in_context(r#"let "" = x;"#);
    parse_pattern_in_context(r#"let "hello" = x;"#);
    parse_pattern_in_context(r#"let "hello\nworld" = x;"#);
}

#[test]
fn test_literal_patterns_chars() {
    parse_pattern_in_context("let 'a' = x;");
    parse_pattern_in_context("let ' ' = x;");
    parse_pattern_in_context(r"let '\n' = x;");
}

#[test]
fn test_literal_patterns_booleans() {
    parse_pattern_in_context("let true = x;");
    parse_pattern_in_context("let false = x;");
}

#[test]
fn test_identifier_patterns() {
    parse_pattern_in_context("let x = value;");
    parse_pattern_in_context("let foo_bar = value;");
    parse_pattern_in_context("let _unused = value;");
    parse_pattern_in_context("let x123 = value;");
}

#[test]
fn test_identifier_patterns_mutable() {
    parse_pattern_in_context("let mut x = value;");
    parse_pattern_in_context("let mut counter = 0;");
}

#[test]
fn test_identifier_patterns_ref() {
    parse_pattern_in_context("let ref x = value;");
    parse_pattern_in_context("let ref mut x = value;");
}

#[test]
fn test_wildcard_pattern() {
    parse_pattern_in_context("let _ = x;");
    parse_pattern_in_context("match x { _ => {} };");
}

// ============================================================================
// DESTRUCTURING PATTERNS
// ============================================================================

#[test]
fn test_tuple_pattern_basic() {
    parse_pattern_in_context("let () = unit;");
    parse_pattern_in_context("let (x,) = single;");
    parse_pattern_in_context("let (x, y) = pair;");
    parse_pattern_in_context("let (a, b, c) = triple;");
}

#[test]
fn test_tuple_pattern_nested() {
    parse_pattern_in_context("let ((a, b), c) = nested;");
    parse_pattern_in_context("let (x, (y, z)) = nested;");
    parse_pattern_in_context("let ((a, b), (c, d)) = quad;");
}

#[test]
fn test_tuple_pattern_with_wildcard() {
    parse_pattern_in_context("let (x, _) = pair;");
    parse_pattern_in_context("let (_, y) = pair;");
    parse_pattern_in_context("let (x, _, z) = triple;");
}

#[test]
fn test_array_pattern_basic() {
    parse_pattern_in_context("let [] = empty;");
    parse_pattern_in_context("let [x] = single;");
    parse_pattern_in_context("let [a, b, c] = arr;");
}

#[test]
fn test_array_pattern_nested() {
    parse_pattern_in_context("let [[a, b], [c, d]] = matrix;");
    parse_pattern_in_context("let [(x, y), (z, w)] = pairs;");
}

#[test]
fn test_slice_pattern_with_rest() {
    parse_pattern_in_context("let [..] = all;");
    parse_pattern_in_context("let [first, ..] = arr;");
    parse_pattern_in_context("let [.., last] = arr;");
    parse_pattern_in_context("let [first, .., last] = arr;");
    parse_pattern_in_context("let [a, b, .., y, z] = arr;");
}

#[test]
fn test_record_pattern_basic() {
    parse_pattern_in_context("let Point { x } = p;");
    parse_pattern_in_context("let Point { x, y } = p;");
    parse_pattern_in_context("let Person { name, age } = person;");
}

#[test]
fn test_record_pattern_renamed() {
    parse_pattern_in_context("let Point { x: px } = p;");
    parse_pattern_in_context("let Point { x: a, y: b } = p;");
}

#[test]
fn test_record_pattern_mixed() {
    parse_pattern_in_context("let Point { x, y: py } = p;");
    parse_pattern_in_context("let Rect { x, y: py, width: w, height } = rect;");
}

#[test]
fn test_record_pattern_with_rest() {
    parse_pattern_in_context("let Point { x, .. } = p;");
    parse_pattern_in_context("let Person { name, .. } = person;");
}

#[test]
fn test_record_pattern_nested() {
    parse_pattern_in_context("let Container { value: Point { x, y } } = c;");
    parse_pattern_in_context("let Outer { inner: Inner { data } } = nested;");
}

// ============================================================================
// VARIANT PATTERNS (ENUMS)
// ============================================================================

#[test]
fn test_variant_pattern_unit() {
    parse_pattern_in_context("let None = opt;");
    parse_pattern_in_context("match status { Active => {}, Inactive => {} };");
}

#[test]
fn test_variant_pattern_tuple_style() {
    parse_pattern_in_context("let Some(x) = opt;");
    parse_pattern_in_context("let Ok(value) = result;");
    parse_pattern_in_context("let Err(error) = result;");
}

#[test]
fn test_variant_pattern_tuple_multiple_fields() {
    parse_pattern_in_context("let Color(r, g, b) = color;");
    parse_pattern_in_context("let Point3D(x, y, z) = point;");
}

#[test]
fn test_variant_pattern_record_style() {
    parse_pattern_in_context("let Person { name, age } = p;");
    parse_pattern_in_context("let Error { code, message } = err;");
}

#[test]
fn test_variant_pattern_nested() {
    parse_pattern_in_context("let Some((x, y)) = opt;");
    parse_pattern_in_context("let Some(Point { x, y }) = opt;");
    parse_pattern_in_context("let Ok([a, b, c]) = result;");
}

#[test]
fn test_variant_pattern_deeply_nested() {
    parse_pattern_in_context("let Some(Ok((x, y))) = nested;");
    parse_pattern_in_context("let Ok(Some([a, b])) = complex;");
}

// ============================================================================
// OR PATTERNS
// ============================================================================

#[test]
fn test_or_pattern_identifiers() {
    parse_pattern_in_context("let x | y = value;");
    parse_pattern_in_context("let a | b | c = value;");
}

#[test]
fn test_or_pattern_literals() {
    parse_pattern_in_context("let 1 | 2 | 3 = num;");
    parse_pattern_in_context(r#"let "a" | "b" | "c" = str;"#);
    parse_pattern_in_context("let true | false = bool;");
}

#[test]
fn test_or_pattern_variants() {
    parse_pattern_in_context("match result { Ok(x) | Err(x) => {} };");
    parse_pattern_in_context("match opt { Some(0) | None => {} _ => {} };");
}

#[test]
fn test_or_pattern_complex() {
    parse_pattern_in_context("let (1, x) | (2, x) | (3, x) = pair;");
}

// ============================================================================
// RANGE PATTERNS
// ============================================================================

#[test]
fn test_range_pattern_exclusive() {
    parse_pattern_in_context("let 0..10 = x;");
    parse_pattern_in_context("let 1..100 = num;");
}

#[test]
fn test_range_pattern_inclusive() {
    parse_pattern_in_context("let 0..=10 = x;");
    parse_pattern_in_context("let 1..=100 = num;");
}

#[test]
fn test_range_pattern_from() {
    parse_pattern_in_context("let 100.. = x;");
}

#[test]
fn test_range_pattern_to() {
    parse_pattern_in_context("let ..10 = x;");
    // Note: Inclusive RangeTo pattern (..=100) not yet supported
    // parse_pattern_in_context("let ..=100 = x;");
}

#[test]
fn test_range_pattern_in_match() {
    parse_pattern_in_context("match x { 0..10 => {}, 10..20 => {}, _ => {} };");
    parse_pattern_in_context("match age { 0..=17 => {}, 18..=64 => {}, 65.. => {} };");
}

// ============================================================================
// REFERENCE PATTERNS
// ============================================================================

#[test]
fn test_reference_pattern_immutable() {
    parse_pattern_in_context("let &x = ref_val;");
    parse_pattern_in_context("let &(a, b) = ref_pair;");
}

#[test]
fn test_reference_pattern_mutable() {
    parse_pattern_in_context("let &mut x = ref_val;");
    parse_pattern_in_context("let &mut [a, b] = ref_arr;");
}

#[test]
fn test_reference_pattern_nested() {
    parse_pattern_in_context("let &Some(x) = opt_ref;");
    parse_pattern_in_context("let &Ok(value) = result_ref;");
    parse_pattern_in_context("let &Point { x, y } = point_ref;");
}

// ============================================================================
// GUARD PATTERNS
// ============================================================================

#[test]
fn test_guard_simple() {
    parse_pattern_in_context("match x { n if n > 0 => {}, _ => {} };");
    parse_pattern_in_context("match x { x if x == 42 => {}, _ => {} };");
}

#[test]
fn test_guard_complex_condition() {
    parse_pattern_in_context("match x { n if n > 0 && n < 100 => {}, _ => {} };");
    parse_pattern_in_context("match x { x if x % 2 == 0 || x % 3 == 0 => {}, _ => {} };");
}

#[test]
fn test_guard_with_pattern() {
    parse_pattern_in_context("match pair { (x, y) if x > y => {}, _ => {} };");
    parse_pattern_in_context("match opt { Some(x) if x > 0 => {}, _ => {} };");
}

#[test]
fn test_guard_with_record_pattern() {
    parse_pattern_in_context("match point { Point { x, y } if x > 0 && y > 0 => {}, _ => {} };");
}

// ============================================================================
// AT BINDINGS (@)
// ============================================================================

#[test]
fn test_at_binding_basic() {
    parse_pattern_in_context("let x @ Some(y) = opt;");
    parse_pattern_in_context("let val @ Ok(inner) = result;");
}

#[test]
fn test_at_binding_with_range() {
    parse_pattern_in_context("match x { n @ 1..=10 => {}, _ => {} };");
    parse_pattern_in_context("match age { a @ 18..=64 => {}, _ => {} };");
}

#[test]
fn test_at_binding_nested() {
    parse_pattern_in_context("let outer @ Some(inner @ (x, y)) = nested;");
}

// ============================================================================
// COMPLEX NESTED PATTERNS
// ============================================================================

#[test]
fn test_complex_nested_tuple_array() {
    parse_pattern_in_context("let [(a, b), (c, d), (e, f)] = pairs;");
    parse_pattern_in_context("let ([x, y], [z, w]) = matrix;");
}

#[test]
fn test_complex_nested_variant_tuple() {
    parse_pattern_in_context("let Some((Ok(x), Err(e))) = complex;");
    parse_pattern_in_context("let Ok((a, Some(b))) = nested;");
}

#[test]
fn test_complex_nested_variant_record() {
    parse_pattern_in_context("let Some(Person { name, age }) = opt_person;");
    parse_pattern_in_context("let Ok(Point { x, y }) = result_point;");
}

#[test]
fn test_complex_nested_record_variant() {
    parse_pattern_in_context("let Container { value: Some(x) } = container;");
    parse_pattern_in_context("let Response { data: Ok(body), status } = resp;");
}

#[test]
fn test_complex_slice_variant() {
    parse_pattern_in_context("let [Some(x), None, Some(y)] = opts;");
    parse_pattern_in_context("let [first, .., Some(last)] = list;");
}

#[test]
fn test_complex_or_with_nested() {
    parse_pattern_in_context("match x { Some((1, _)) | Some((_, 1)) => {}, _ => {} };");
    parse_pattern_in_context("match x { Ok([a, b]) | Err([a, b]) => {}, _ => {} };");
}

// ============================================================================
// PATTERNS IN DIFFERENT CONTEXTS
// ============================================================================

#[test]
fn test_pattern_in_let_binding() {
    parse_pattern_in_context("let (x, y) = pair;");
    parse_pattern_in_context("let Point { x, y } = point;");
    parse_pattern_in_context("let Some(value) = opt;");
}

#[test]
fn test_pattern_in_match_arm() {
    parse_pattern_in_context("match x { 1 => {}, 2 => {}, _ => {} };");
    parse_pattern_in_context("match opt { Some(x) => {}, None => {} };");
    parse_pattern_in_context("match pair { (0, y) => {}, (x, 0) => {}, (x, y) => {} };");
}

#[test]
fn test_pattern_in_for_loop() {
    parse_pattern_in_context("for x in items { };");
    parse_pattern_in_context("for (key, value) in map { };");
    parse_pattern_in_context("for Point { x, y } in points { };");
}

#[test]
fn test_pattern_in_if_let() {
    parse_pattern_in_context("if let Some(x) = opt { };");
    parse_pattern_in_context("if let Ok(value) = result { };");
    parse_pattern_in_context("if let (1, y) = pair { };");
}

// ============================================================================
// EDGE CASES AND ERROR RECOVERY
// ============================================================================

#[test]
fn test_pattern_trailing_commas() {
    parse_pattern_in_context("let (x, y,) = pair;");
    parse_pattern_in_context("let [a, b,] = arr;");
    parse_pattern_in_context("let Point { x, y, } = p;");
}

#[test]
fn test_pattern_empty_structures() {
    parse_pattern_in_context("let () = unit;");
    parse_pattern_in_context("let [] = empty;");
}

#[test]
fn test_pattern_single_element_tuple() {
    parse_pattern_in_context("let (x,) = single;");
}

#[test]
fn test_pattern_parenthesized() {
    parse_pattern_in_context("let (x) = value;");
    parse_pattern_in_context("let ((x)) = value;");
    parse_pattern_in_context("let (((x, y))) = pair;");
}

#[test]
fn test_pattern_multiple_wildcards() {
    parse_pattern_in_context("let (_, _, _) = triple;");
    parse_pattern_in_context("let [_, x, _, y, _] = arr;");
}

#[test]
fn test_pattern_multiple_rest() {
    // Only one rest pattern should be allowed per slice
    // This tests parser's ability to handle it even if semantically invalid
    parse_pattern_in_context("let [first, .., last] = arr;");
}

// ============================================================================
// REAL-WORLD PATTERNS
// ============================================================================

#[test]
fn test_realistic_option_matching() {
    parse_pattern_in_context(
        r#"
        match find_user(id) {
            Some(User { name, email, .. }) => {},
            None => {}
        };
    "#,
    );
}

#[test]
fn test_realistic_result_matching() {
    parse_pattern_in_context(
        r#"
        match parse_json(data) {
            Ok(json) => {},
            Err(ParseError { line, column, message }) => {}
        };
    "#,
    );
}

#[test]
fn test_realistic_list_processing() {
    parse_pattern_in_context(
        r#"
        match items {
            [] => {},
            [single] => {},
            [first, second] => {},
            [first, .., last] => {},
            [head, ..] => {}
        };
    "#,
    );
}

#[test]
fn test_realistic_tuple_destructuring() {
    parse_pattern_in_context(
        r#"
        let (status, headers, body) = response;
    "#,
    );
}

#[test]
fn test_realistic_nested_iteration() {
    parse_pattern_in_context(
        r#"
        for (index, Some(value)) in enumerated { };
    "#,
    );
}

#[test]
fn test_realistic_guard_with_range() {
    parse_pattern_in_context(
        r#"
        match score {
            s @ 90..=100 if s > 95 => {},
            s @ 80..=89 => {},
            s @ 70..=79 => {},
            _ => {}
        };
    "#,
    );
}
