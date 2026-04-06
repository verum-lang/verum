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
// Unit tests for rsl_parser.rs
//
// Migrated from src/rsl_parser.rs to comply with CLAUDE.md test organization.

use verum_smt::rsl_parser::*;

use verum_ast::{BinOp, ExprKind, Span};
use verum_common::Text;

#[test]
fn test_parse_simple_requires() {
    let input = Text::from("requires x > 0");
    let mut parser = RslParser::new(input, Span::dummy());
    let spec = parser.parse().unwrap();

    assert_eq!(spec.preconditions.len(), 1);
    assert_eq!(spec.preconditions[0].kind, RslClauseKind::Requires);
}

#[test]
fn test_parse_multiple_clauses() {
    let input = Text::from("requires x > 0; ensures result >= 0;");
    let mut parser = RslParser::new(input, Span::dummy());
    let spec = parser.parse().unwrap();

    assert_eq!(spec.preconditions.len(), 1);
    assert_eq!(spec.postconditions.len(), 1);
}

#[test]
fn test_parse_old_function() {
    let input = Text::from("ensures result == old(balance) - amount");
    let mut parser = RslParser::new(input, Span::dummy());
    let spec = parser.parse().unwrap();

    assert_eq!(spec.postconditions.len(), 1);
    // Check that old() is parsed as a function call
    match &spec.postconditions[0].expr.kind {
        ExprKind::Binary { .. } => {
            // Expected structure: result == (old(balance) - amount)
        }
        _ => panic!("Expected binary expression"),
    }
}

#[test]
fn test_parse_result_variable() {
    let input = Text::from("ensures result > 0");
    let mut parser = RslParser::new(input, Span::dummy());
    let spec = parser.parse().unwrap();

    assert_eq!(spec.postconditions.len(), 1);
}

#[test]
fn test_parse_logical_and() {
    let input = Text::from("requires x > 0 && y > 0");
    let mut parser = RslParser::new(input, Span::dummy());
    let spec = parser.parse().unwrap();

    assert_eq!(spec.preconditions.len(), 1);
    match &spec.preconditions[0].expr.kind {
        ExprKind::Binary { op: BinOp::And, .. } => {}
        _ => panic!("Expected AND expression"),
    }
}

// NOTE: This test is ignored until the RSL parser is fixed to properly handle
// implication with comparison operators on both sides
#[test]
fn test_parse_implication() {
    let input = Text::from("requires x > 0 => result > 0;");
    let mut parser = RslParser::new(input, Span::dummy());
    let spec = parser.parse().unwrap();

    assert_eq!(spec.preconditions.len(), 1);
    // Implication is converted to: !left || right
    match &spec.preconditions[0].expr.kind {
        ExprKind::Binary { op: BinOp::Or, .. } => {}
        _ => panic!("Expected OR expression (from implication)"),
    }
}

#[test]
fn test_parse_field_access() {
    let input = Text::from("requires account.balance >= amount");
    let mut parser = RslParser::new(input, Span::dummy());
    let spec = parser.parse().unwrap();

    assert_eq!(spec.preconditions.len(), 1);
}

#[test]
fn test_parse_method_call() {
    let input = Text::from("ensures result.is_valid()");
    let mut parser = RslParser::new(input, Span::dummy());
    let spec = parser.parse().unwrap();

    assert_eq!(spec.postconditions.len(), 1);
    match &spec.postconditions[0].expr.kind {
        ExprKind::MethodCall { .. } => {}
        _ => panic!("Expected method call expression"),
    }
}

#[test]
fn test_parse_array_index() {
    let input = Text::from("requires arr[i] > 0");
    let mut parser = RslParser::new(input, Span::dummy());
    let spec = parser.parse().unwrap();

    assert_eq!(spec.preconditions.len(), 1);
}

#[test]
fn test_empty_contract() {
    let input = Text::from("");
    let mut parser = RslParser::new(input, Span::dummy());
    let spec = parser.parse().unwrap();

    assert!(spec.is_empty());
}

#[test]
fn test_complex_expression() {
    let input = Text::from("ensures (result > 0 && result < 100) || result == -1");
    let mut parser = RslParser::new(input, Span::dummy());
    let spec = parser.parse().unwrap();

    assert_eq!(spec.postconditions.len(), 1);
}
