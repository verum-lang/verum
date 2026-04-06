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
//! Tests for rank-2 polymorphic function types: fn<R>(...) -> R
//!
//! Rank-2 types allow universally quantified type parameters scoped
//! within a function type. The caller cannot choose R; the function
//! must work for ALL R.

use verum_ast::span::FileId;
use verum_ast::ty::TypeKind;
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

fn parse_module_ok(input: &str) {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(
        result.is_ok(),
        "Failed to parse:\n{}\nError: {:?}",
        input,
        result.err()
    );
}

fn parse_type_str(input: &str) -> TypeKind {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_type_str(input, file_id);
    assert!(
        result.is_ok(),
        "Failed to parse type:\n{}\nError: {:?}",
        input,
        result.err()
    );
    result.unwrap().kind
}

// ============================================================================
// Basic Rank-2 Type Parsing
// ============================================================================

#[test]
fn test_rank2_identity_type() {
    // fn<T>(T) -> T : a function that works for all T
    let kind = parse_type_str("fn<T>(T) -> T");
    assert!(
        matches!(kind, TypeKind::Rank2Function { .. }),
        "Expected Rank2Function, got {:?}",
        kind
    );
}

#[test]
fn test_rank2_single_param() {
    let kind = parse_type_str("fn<R>(R) -> R");
    if let TypeKind::Rank2Function { type_params, .. } = kind {
        assert_eq!(type_params.len(), 1);
    } else {
        panic!("Expected Rank2Function");
    }
}

#[test]
fn test_rank2_two_params() {
    let kind = parse_type_str("fn<A, B>(A, B) -> A");
    if let TypeKind::Rank2Function { type_params, params, .. } = kind {
        assert_eq!(type_params.len(), 2);
        assert_eq!(params.len(), 2);
    } else {
        panic!("Expected Rank2Function");
    }
}

#[test]
fn test_rank2_no_return() {
    // fn<T>(T) - void return
    let kind = parse_type_str("fn<T>(T)");
    assert!(matches!(kind, TypeKind::Rank2Function { .. }));
}

// ============================================================================
// Rank-2 in Record Types
// ============================================================================

#[test]
fn test_rank2_in_record_field() {
    parse_module_ok("type Container is { transform: fn<R>(R) -> R };");
}

#[test]
fn test_rank2_transducer_pattern() {
    // The classic transducer type from Clojure/Haskell
    parse_module_ok(
        "type Transducer<A, B> is { transform: fn<R>(fn(R, B) -> R) -> fn(R, A) -> R };"
    );
}

#[test]
fn test_rank2_multiple_fields() {
    parse_module_ok(
        "type Codec<A, B> is { encode: fn<R>(A, fn(B) -> R) -> R, decode: fn<R>(B, fn(A) -> R) -> R };"
    );
}

// ============================================================================
// Rank-2 as Function Parameters
// ============================================================================

#[test]
fn test_rank2_as_function_param() {
    parse_module_ok("fn apply(f: fn<T>(T) -> T, x: Int) -> Int { f(x) }");
}

#[test]
fn test_rank2_multiple_params() {
    parse_module_ok(
        "fn combine(f: fn<T>(T) -> T, g: fn<U>(U) -> U) -> Int { f(g(42)) }"
    );
}

// ============================================================================
// Rank-2 with Contexts
// ============================================================================

#[test]
fn test_rank2_with_context() {
    parse_module_ok(
        "type Handler is { handle: fn<R>(fn() -> R using [Logger]) -> R };"
    );
}

// ============================================================================
// Rank-2 vs Regular Function Types
// ============================================================================

#[test]
fn test_regular_function_type_is_not_rank2() {
    let kind = parse_type_str("fn(Int) -> Int");
    assert!(
        matches!(kind, TypeKind::Function { .. }),
        "Regular function type should be Function, not Rank2Function"
    );
}

#[test]
fn test_generic_function_decl_is_not_rank2() {
    // A generic function declaration (fn foo<T>(...)) is NOT a rank-2 type.
    // Rank-2 is specifically for function TYPES used as values.
    parse_module_ok("fn identity<T>(x: T) -> T { x }");
}

// ============================================================================
// Rank-2 in Type Aliases
// ============================================================================

#[test]
fn test_rank2_type_alias() {
    parse_module_ok("type Fold is fn<R>(R, Int) -> R;");
}

#[test]
fn test_rank2_in_sum_type() {
    parse_module_ok(
        "type Transform is Identity | Custom(fn<R>(R) -> R);"
    );
}

// ============================================================================
// Nested Rank-2 Types
// ============================================================================

#[test]
fn test_rank2_returning_rank2() {
    // A rank-2 function that returns a regular function
    parse_module_ok(
        "type Builder is { build: fn<R>(fn(R) -> R) -> fn(Int) -> Int };"
    );
}

// ============================================================================
// Rank-2 with Type Bounds (where clause)
// ============================================================================

#[test]
fn test_rank2_with_bounded_params() {
    // fn<T: Eq>(T, T) -> Bool
    parse_module_ok("fn compare(cmp: fn<T: Eq>(T, T) -> Bool) -> Bool { cmp(1, 2) }");
}
