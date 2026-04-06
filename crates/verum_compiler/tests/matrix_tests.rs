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
// Per CLAUDE.md standards - tests in tests/ directory

use verum_ast::{FileId, Span};
use verum_compiler::literal_parsers::parse_matrix;
use verum_compiler::literal_registry::ParsedLiteral;
use verum_common::Text;

fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

#[test]
fn test_parse_2x2_matrix() {
    let result = parse_matrix(&Text::from("[[1, 2], [3, 4]]"), test_span(), None);
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Matrix { rows, cols, data }) = result {
        assert_eq!(rows, 2);
        assert_eq!(cols, 2);
        assert_eq!(data.len(), 4);
    }
}

#[test]
fn test_parse_3x3_matrix() {
    let result = parse_matrix(
        &Text::from("[[1, 2, 3], [4, 5, 6], [7, 8, 9]]"),
        test_span(),
        None,
    );
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Matrix { rows, cols, data }) = result {
        assert_eq!(rows, 3);
        assert_eq!(cols, 3);
        assert_eq!(data.len(), 9);
    }
}

#[test]
fn test_parse_2x3_matrix() {
    let result = parse_matrix(&Text::from("[[1, 2, 3], [4, 5, 6]]"), test_span(), None);
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Matrix { rows, cols, .. }) = result {
        assert_eq!(rows, 2);
        assert_eq!(cols, 3);
    }
}

#[test]
fn test_parse_floating_point() {
    let result = parse_matrix(&Text::from("[[1.5, 2.5], [3.5, 4.5]]"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_negative_values() {
    let result = parse_matrix(&Text::from("[[-1, -2], [-3, -4]]"), test_span(), None);
    assert!(result.is_ok());
}

#[test]
fn test_parse_uneven_rows() {
    let result = parse_matrix(&Text::from("[[1, 2], [3, 4, 5]]"), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_invalid_format() {
    let result = parse_matrix(&Text::from("[1, 2, 3]"), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_empty_matrix() {
    let result = parse_matrix(&Text::from("[[]]"), test_span(), None);
    assert!(result.is_err());
}

#[test]
fn test_parse_single_element_matrix() {
    let result = parse_matrix(&Text::from("[[42]]"), test_span(), None);
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Matrix { rows, cols, data }) = result {
        assert_eq!(rows, 1);
        assert_eq!(cols, 1);
        assert_eq!(data.len(), 1);
        assert_eq!(data[0], 42.0);
    }
}

#[test]
fn test_parse_row_vector() {
    let result = parse_matrix(&Text::from("[[1, 2, 3, 4, 5]]"), test_span(), None);
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Matrix { rows, cols, .. }) = result {
        assert_eq!(rows, 1);
        assert_eq!(cols, 5);
    }
}

#[test]
fn test_parse_column_vector() {
    let result = parse_matrix(&Text::from("[[1], [2], [3], [4], [5]]"), test_span(), None);
    assert!(result.is_ok());
    if let Ok(ParsedLiteral::Matrix { rows, cols, .. }) = result {
        assert_eq!(rows, 5);
        assert_eq!(cols, 1);
    }
}

#[test]
fn test_parse_matrix_with_spaces() {
    let result = parse_matrix(
        &Text::from("[[ 1 , 2 , 3 ], [ 4 , 5 , 6 ]]"),
        test_span(),
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn test_parse_matrix_scientific_notation() {
    let result = parse_matrix(
        &Text::from("[[1e-5, 2e-5], [3e-5, 4e-5]]"),
        test_span(),
        None,
    );
    assert!(result.is_ok());
}
