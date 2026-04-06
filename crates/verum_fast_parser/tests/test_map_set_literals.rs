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
//! Test parsing of Map and Set literal syntax

use verum_ast::{ExprKind, FileId};
use verum_fast_parser::VerumParser;

fn parse_expr_test(source: &str) -> verum_ast::Expr {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_expr_str(source, file_id)
        .unwrap_or_else(|_| panic!("Failed to parse: {}", source))
}

#[test]
fn test_parse_map_literal() {
    let expr = parse_expr_test(r#"{ "Alice": 30, "Bob": 25 }"#);

    match &expr.kind {
        ExprKind::MapLiteral { entries } => {
            assert_eq!(entries.len(), 2, "Should have 2 map entries");
            println!("✓ Map literal parsed successfully!");
        }
        _ => panic!("Expected MapLiteral, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_set_literal() {
    let expr = parse_expr_test("{2, 3, 5, 7, 11}");

    match &expr.kind {
        ExprKind::SetLiteral { elements } => {
            assert_eq!(elements.len(), 5, "Should have 5 set elements");
            println!("✓ Set literal parsed successfully!");
        }
        _ => panic!("Expected SetLiteral, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_empty_map() {
    let expr = parse_expr_test("{}");

    match &expr.kind {
        ExprKind::MapLiteral { entries } => {
            assert_eq!(entries.len(), 0, "Should be empty map");
            println!("✓ Empty map parsed successfully!");
        }
        _ => panic!("Expected MapLiteral for empty braces, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_block_still_works() {
    let expr = parse_expr_test("{ let a = 1; a + 2 }");

    match &expr.kind {
        ExprKind::Block(_) => {
            println!("✓ Block expression still works!");
        }
        _ => panic!("Expected Block, got {:?}", expr.kind),
    }
}
