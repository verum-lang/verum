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
//! Test for pipeline operator with qualified path expressions bug
//!
//! Issue: Parser incorrectly expects array indexing `[` after identifier in pipeline,
//! instead of allowing `::` for qualified paths like `stream::map`.

use verum_ast::{Expr, FileId};
use verum_fast_parser::VerumParser;

fn parse_expr_test(source: &str) -> Result<Expr, String> {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_expr_str(source, file_id)
        .map_err(|e| format!("{:?}", e))
}

#[test]
fn test_simple_qualified_path() {
    // This should work fine - just parsing a qualified path
    let result = parse_expr_test("stream.map");
    assert!(
        result.is_ok(),
        "Failed to parse simple qualified path: {:?}",
        result
    );
}

#[test]
fn test_pipeline_with_simple_ident() {
    // This should work - pipeline with simple identifier
    let result = parse_expr_test("data |> transform");
    assert!(
        result.is_ok(),
        "Failed to parse pipeline with simple ident: {:?}",
        result
    );
}

#[test]
fn test_pipeline_with_qualified_path() {
    // Qualified paths use . (dot) as the path separator
    let result = parse_expr_test("data |> stream.map");
    assert!(
        result.is_ok(),
        "Failed to parse pipeline with qualified path: {:?}",
        result
    );
}

#[test]
fn test_pipeline_chain_with_qualified_paths() {
    let result = parse_expr_test("data |> stream.filter(pred) |> stream.map(transform)");
    assert!(
        result.is_ok(),
        "Failed to parse pipeline chain with qualified paths: {:?}",
        result
    );
}

#[test]
fn test_pipeline_multiline() {
    let result = parse_expr_test(
        r#"data
            |> stream.filter(|x| x > 0)
            |> stream.map(|x| x * 2)
            |> stream.collect()"#,
    );
    assert!(
        result.is_ok(),
        "Failed to parse multiline pipeline with qualified paths: {:?}",
        result
    );
}
