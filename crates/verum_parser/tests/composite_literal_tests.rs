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
// Comprehensive test suite for composite literals in Verum.
//
// This test suite covers:
// - Matrix literals: mat#"[[1, 2], [3, 4]]"
// - Vector literals: vec#"<1, 2, 3>" or vec#"[1, 2, 3]"
// - Chemistry literals: chem#"H2O", chem#"C6H12O6"
// - Music literals: music#"C4 D4 E4 F4", music#"Cmaj7"
// - Interval literals: interval#"[0, 100)", interval#"[0..100]"
//
// Tests for composite literal syntax: bool_lit = "true" | "false"

use verum_ast::{Expr, ExprKind, FileId, Literal, LiteralKind};
use verum_parser::VerumParser;

/// Helper function to parse an expression from a string.
fn parse_expr(source: &str) -> Result<Expr, String> {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser.parse_expr_str(source, file_id).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join(", ")
    })
}

/// Helper to check if parsing succeeds.
fn assert_parses(source: &str) {
    parse_expr(source).unwrap_or_else(|_| panic!("Failed to parse: {}", source));
}

/// Helper to check if parsing fails.
fn assert_fails(source: &str) {
    assert!(
        parse_expr(source).is_err(),
        "Expected parse failure for: {}",
        source
    );
}

/// Helper to extract composite literal from parsed expression.
fn extract_composite(expr: &Expr) -> &verum_ast::literal::CompositeLiteral {
    match &expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Composite(comp) => comp,
            _ => panic!("Expected composite literal, got {:?}", lit.kind),
        },
        _ => panic!("Expected literal expression, got {:?}", expr.kind),
    }
}

// ============================================================================
// SECTION 1: MATRIX LITERALS
// ============================================================================

#[test]
fn test_matrix_literal_basic() {
    let expr = parse_expr("mat#\"[[1, 2], [3, 4]]\"").unwrap();
    let comp = extract_composite(&expr);
    assert_eq!(comp.tag.as_str(), "mat");
    assert_eq!(comp.content.as_str(), "[1, 2], [3, 4]");
}

#[test]
fn test_matrix_literal_single_row() {
    assert_parses("mat#\"[[1, 2, 3]]\"");
}

#[test]
fn test_matrix_literal_single_column() {
    assert_parses("mat#\"[[1], [2], [3]]\"");
}

#[test]
fn test_matrix_literal_3x3() {
    assert_parses("mat#\"[[1, 2, 3], [4, 5, 6], [7, 8, 9]]\"");
}

#[test]
fn test_matrix_literal_with_floats() {
    assert_parses("mat#\"[[1.5, 2.3], [3.7, 4.1]]\"");
}

#[test]
fn test_matrix_literal_with_spaces() {
    assert_parses("mat#\"[[ 1 , 2 ], [ 3 , 4 ]]\"");
}

#[test]
fn test_matrix_literal_multiline() {
    assert_parses("mat#\"[[1, 2],\n[3, 4]]\"");
}

#[test]
fn test_matrix_literal_with_parentheses() {
    assert_parses("mat#([[1, 2], [3, 4]])");
}

#[test]
fn test_matrix_literal_with_brackets() {
    assert_parses("mat#[[1, 2], [3, 4]]");
}

#[test]
fn test_matrix_literal_with_braces() {
    assert_parses("mat#{[1, 2], [3, 4]}");
}

#[test]
fn test_matrix_literal_empty() {
    assert_parses("mat#\"[[]]\"");
}

#[test]
fn test_matrix_literal_invalid_missing_inner_brackets() {
    // This should still parse as a tagged literal (validation happens later)
    assert_parses("mat#\"[1, 2, 3, 4]\"");
}

#[test]
fn test_matrix_literal_unmatched_brackets() {
    // This should parse but validation will fail
    assert_parses("mat#\"[[1, 2], [3, 4]\"");
}

// ============================================================================
// SECTION 2: VECTOR LITERALS
// ============================================================================

#[test]
fn test_vector_literal_angle_brackets() {
    let expr = parse_expr("vec#\"<1, 2, 3>\"").unwrap();
    let comp = extract_composite(&expr);
    assert_eq!(comp.tag.as_str(), "vec");
    assert_eq!(comp.content.as_str(), "1, 2, 3");
}

#[test]
fn test_vector_literal_square_brackets() {
    assert_parses("vec#\"[1, 2, 3]\"");
}

#[test]
fn test_vector_literal_floats() {
    assert_parses("vec#\"<1.5, 2.3, 3.7>\"");
}

#[test]
fn test_vector_literal_single_element() {
    assert_parses("vec#\"<42>\"");
}

#[test]
fn test_vector_literal_two_elements() {
    assert_parses("vec#\"<1.0, 2.0>\"");
}

#[test]
fn test_vector_literal_many_elements() {
    assert_parses("vec#\"<1, 2, 3, 4, 5, 6, 7, 8, 9, 10>\"");
}

#[test]
fn test_vector_literal_with_spaces() {
    assert_parses("vec#\"< 1 , 2 , 3 >\"");
}

#[test]
fn test_vector_literal_with_parentheses() {
    assert_parses("vec#(1, 2, 3)");
}

#[test]
fn test_vector_literal_with_brackets() {
    assert_parses("vec#[1, 2, 3]");
}

#[test]
fn test_vector_literal_with_braces() {
    assert_parses("vec#{1, 2, 3}");
}

#[test]
fn test_vector_literal_empty() {
    assert_parses("vec#\"<>\"");
}

#[test]
fn test_vector_literal_no_commas() {
    // Single value without comma is still valid
    assert_parses("vec#\"<5>\"");
}

// ============================================================================
// SECTION 3: CHEMISTRY LITERALS
// ============================================================================

#[test]
fn test_chemistry_literal_water() {
    let expr = parse_expr("chem#\"H2O\"").unwrap();
    let comp = extract_composite(&expr);
    assert_eq!(comp.tag.as_str(), "chem");
    assert_eq!(comp.content.as_str(), "H2O");
}

#[test]
fn test_chemistry_literal_glucose() {
    assert_parses("chem#\"C6H12O6\"");
}

#[test]
fn test_chemistry_literal_simple_element() {
    assert_parses("chem#\"H\"");
}

#[test]
fn test_chemistry_literal_with_lowercase() {
    assert_parses("chem#\"Ca\"");
}

#[test]
fn test_chemistry_literal_methane() {
    assert_parses("chem#\"CH4\"");
}

#[test]
fn test_chemistry_literal_ethanol() {
    assert_parses("chem#\"C2H5OH\"");
}

#[test]
fn test_chemistry_literal_complex() {
    assert_parses("chem#\"Ca(OH)2\"");
}

#[test]
fn test_chemistry_literal_with_parentheses() {
    assert_parses("chem#(H2SO4)");
}

#[test]
fn test_chemistry_literal_with_brackets() {
    assert_parses("chem#[NaCl]");
}

#[test]
fn test_chemistry_literal_with_braces() {
    assert_parses("chem#{O2}");
}

#[test]
fn test_chemistry_literal_multiple_words() {
    assert_parses("chem#\"Calcium Carbonate\"");
}

#[test]
fn test_chemistry_literal_invalid_no_elements() {
    // No valid element letters - parsing succeeds but validation fails
    assert_parses("chem#\"123\"");
}

// ============================================================================
// SECTION 4: MUSIC LITERALS
// ============================================================================

#[test]
fn test_music_literal_single_note() {
    let expr = parse_expr("music#\"C\"").unwrap();
    let comp = extract_composite(&expr);
    assert_eq!(comp.tag.as_str(), "music");
    assert_eq!(comp.content.as_str(), "C");
}

#[test]
fn test_music_literal_with_octave() {
    assert_parses("music#\"C4\"");
}

#[test]
fn test_music_literal_melody() {
    assert_parses("music#\"C4 D4 E4 F4\"");
}

#[test]
fn test_music_literal_chord() {
    assert_parses("music#\"Cmaj7\"");
}

#[test]
fn test_music_literal_sharp() {
    assert_parses("music#\"C#\"");
}

#[test]
fn test_music_literal_flat() {
    assert_parses("music#\"Bb\"");
}

#[test]
fn test_music_literal_all_notes() {
    assert_parses("music#\"C D E F G A B\"");
}

#[test]
fn test_music_literal_complex_chord() {
    assert_parses("music#\"Cmaj7/G\"");
}

#[test]
fn test_music_literal_diminished() {
    assert_parses("music#\"Cdim\"");
}

#[test]
fn test_music_literal_augmented() {
    assert_parses("music#\"Caug\"");
}

#[test]
fn test_music_literal_with_parentheses() {
    assert_parses("music#(C4 D4 E4)");
}

#[test]
fn test_music_literal_with_brackets() {
    assert_parses("music#[C major scale]");
}

#[test]
fn test_music_literal_with_braces() {
    assert_parses("music#{G maj7}");
}

#[test]
fn test_music_literal_sus_chord() {
    assert_parses("music#\"Csus4\"");
}

#[test]
fn test_music_literal_invalid_no_notes() {
    // No valid note letters - parsing succeeds but validation fails
    assert_parses("music#\"123\"");
}

// ============================================================================
// SECTION 5: INTERVAL LITERALS
// ============================================================================

#[test]
fn test_interval_literal_closed() {
    let expr = parse_expr("interval#\"[0, 100]\"").unwrap();
    let comp = extract_composite(&expr);
    assert_eq!(comp.tag.as_str(), "interval");
    assert_eq!(comp.content.as_str(), "0, 100");
}

#[test]
fn test_interval_literal_open() {
    assert_parses("interval#\"(0, 100)\"");
}

#[test]
fn test_interval_literal_half_open_right() {
    assert_parses("interval#\"[0, 100)\"");
}

#[test]
fn test_interval_literal_half_open_left() {
    assert_parses("interval#\"(0, 100]\"");
}

#[test]
fn test_interval_literal_dotdot_notation() {
    assert_parses("interval#\"[0..100]\"");
}

#[test]
fn test_interval_literal_floats() {
    assert_parses("interval#\"[0.5, 99.5]\"");
}

#[test]
fn test_interval_literal_dates() {
    assert_parses("interval#\"[2025-01-01, 2025-12-31]\"");
}

#[test]
fn test_interval_literal_dates_with_dotdot() {
    assert_parses("interval#\"2025-01-01..2025-12-31\"");
}

#[test]
fn test_interval_literal_with_spaces() {
    assert_parses("interval#\"[ 0 , 100 ]\"");
}

#[test]
fn test_interval_literal_with_parentheses() {
    assert_parses("interval#(0, 100)");
}

#[test]
fn test_interval_literal_with_brackets() {
    assert_parses("interval#[0, 100]");
}

#[test]
fn test_interval_literal_with_braces() {
    assert_parses("interval#{0, 100}");
}

#[test]
fn test_interval_literal_negative_numbers() {
    assert_parses("interval#\"[-100, 100]\"");
}

#[test]
fn test_interval_literal_same_endpoints() {
    assert_parses("interval#\"[5, 5]\"");
}

#[test]
fn test_interval_literal_invalid_missing_separator() {
    // No comma or .. - parsing succeeds but validation fails
    assert_parses("interval#\"[0 100]\"");
}

#[test]
fn test_interval_literal_invalid_wrong_brackets() {
    // Wrong bracket types - parsing succeeds but validation fails
    assert_parses("interval#\"{0, 100}\"");
}

// ============================================================================
// SECTION 6: RECOGNITION AND VALIDATION TESTS
// ============================================================================

#[test]
fn test_composite_types_recognized() {
    let tags = vec!["mat", "vec", "chem", "music", "interval"];

    for tag in tags {
        let source = format!("{}#\"test\"", tag);
        let expr = parse_expr(&source).unwrap();
        let comp = extract_composite(&expr);
        assert_eq!(comp.tag.as_str(), tag);
        assert!(comp.is_recognized());
    }
}

#[test]
fn test_unknown_composite_not_recognized() {
    let expr = parse_expr("unknown#\"test\"").unwrap();
    let comp = extract_composite(&expr);
    assert_eq!(comp.tag.as_str(), "unknown");
    assert!(!comp.is_recognized());
}

#[test]
fn test_composite_type_detection() {
    use verum_ast::literal::CompositeType;

    let expr = parse_expr("mat#\"[[1, 2]]\"").unwrap();
    let comp = extract_composite(&expr);
    assert_eq!(comp.composite_type(), Some(CompositeType::Matrix));

    let expr = parse_expr("vec#\"<1, 2>\"").unwrap();
    let comp = extract_composite(&expr);
    assert_eq!(comp.composite_type(), Some(CompositeType::Vector));

    let expr = parse_expr("chem#\"H2O\"").unwrap();
    let comp = extract_composite(&expr);
    assert_eq!(comp.composite_type(), Some(CompositeType::Chemistry));

    let expr = parse_expr("music#\"C4\"").unwrap();
    let comp = extract_composite(&expr);
    assert_eq!(comp.composite_type(), Some(CompositeType::Music));

    let expr = parse_expr("interval#\"[0, 100]\"").unwrap();
    let comp = extract_composite(&expr);
    assert_eq!(comp.composite_type(), Some(CompositeType::Interval));
}

#[test]
fn test_composite_validation_matrix() {
    let expr = parse_expr("mat#\"[[1, 2], [3, 4]]\"").unwrap();
    let comp = extract_composite(&expr);
    assert!(
        comp.validate().is_ok(),
        "Valid matrix should pass validation"
    );

    let expr = parse_expr("mat#\"[1, 2, 3]\"").unwrap();
    let comp = extract_composite(&expr);
    assert!(
        comp.validate().is_err(),
        "Matrix without [[ ]] should fail validation"
    );
}

#[test]
fn test_composite_validation_vector() {
    let expr = parse_expr("vec#\"<1, 2, 3>\"").unwrap();
    let comp = extract_composite(&expr);
    assert!(
        comp.validate().is_ok(),
        "Valid vector should pass validation"
    );
}

#[test]
fn test_composite_validation_chemistry() {
    let expr = parse_expr("chem#\"H2O\"").unwrap();
    let comp = extract_composite(&expr);
    assert!(
        comp.validate().is_ok(),
        "Valid chemistry formula should pass validation"
    );

    let expr = parse_expr("chem#\"123\"").unwrap();
    let comp = extract_composite(&expr);
    assert!(
        comp.validate().is_err(),
        "Chemistry without elements should fail validation"
    );
}

#[test]
fn test_composite_validation_music() {
    let expr = parse_expr("music#\"C4\"").unwrap();
    let comp = extract_composite(&expr);
    assert!(
        comp.validate().is_ok(),
        "Valid music note should pass validation"
    );

    let expr = parse_expr("music#\"123\"").unwrap();
    let comp = extract_composite(&expr);
    assert!(
        comp.validate().is_err(),
        "Music without notes should fail validation"
    );
}

#[test]
fn test_composite_validation_interval() {
    let expr = parse_expr("interval#\"[0, 100]\"").unwrap();
    let comp = extract_composite(&expr);
    assert!(
        comp.validate().is_ok(),
        "Valid interval should pass validation"
    );

    let expr = parse_expr("interval#\"[0 100]\"").unwrap();
    let comp = extract_composite(&expr);
    assert!(
        comp.validate().is_err(),
        "Interval without separator should fail validation"
    );
}

// ============================================================================
// SECTION 7: DELIMITER TESTS
// ============================================================================

#[test]
fn test_composite_delimiter_quote() {
    let expr = parse_expr("mat#\"[[1, 2]]\"").unwrap();
    let comp = extract_composite(&expr);
    assert_eq!(
        comp.delimiter,
        verum_ast::literal::CompositeDelimiter::Quote
    );
}

#[test]
fn test_composite_delimiter_paren() {
    let expr = parse_expr("mat#([[1, 2]])").unwrap();
    let comp = extract_composite(&expr);
    assert_eq!(
        comp.delimiter,
        verum_ast::literal::CompositeDelimiter::Paren
    );
}

#[test]
fn test_composite_delimiter_bracket() {
    let expr = parse_expr("mat#[[1, 2]]").unwrap();
    let comp = extract_composite(&expr);
    assert_eq!(
        comp.delimiter,
        verum_ast::literal::CompositeDelimiter::Bracket
    );
}

#[test]
fn test_composite_delimiter_brace() {
    let expr = parse_expr("mat#{[1, 2]}").unwrap();
    let comp = extract_composite(&expr);
    assert_eq!(
        comp.delimiter,
        verum_ast::literal::CompositeDelimiter::Brace
    );
}

// ============================================================================
// SECTION 8: DISPLAY AND FORMATTING TESTS
// ============================================================================

#[test]
fn test_composite_literal_display() {
    use std::fmt::Display;

    let expr = parse_expr("mat#\"[[1, 2]]\"").unwrap();
    let comp = extract_composite(&expr);
    let displayed = format!("{}", comp);
    assert_eq!(displayed, "mat#\"[1, 2]\"");
}

// ============================================================================
// SECTION 9: EDGE CASES AND CORNER CASES
// ============================================================================

#[test]
fn test_composite_with_special_characters_in_content() {
    assert_parses("music#\"C#-dim-7/G\"");
}

#[test]
fn test_composite_with_unicode() {
    assert_parses("chem#\"H₂O\"");
}

#[test]
fn test_composite_very_long_content() {
    let long_vector = "vec#\"<".to_string()
        + &(1..100)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(", ")
        + ">\"";
    assert_parses(&long_vector);
}

#[test]
fn test_composite_multiple_in_expression() {
    assert_parses("mat#\"[[1, 2]]\" + vec#\"<3, 4>\"");
}

#[test]
fn test_composite_in_function_call() {
    assert_parses("process(mat#\"[[1, 2], [3, 4]]\")");
}

#[test]
fn test_composite_in_array_literal() {
    assert_parses("[mat#\"[[1, 2]]\", vec#\"<1, 2>\"]");
}

#[test]
fn test_composite_in_tuple() {
    assert_parses("(mat#\"[[1, 2]]\", chem#\"H2O\")");
}

#[test]
fn test_composite_literal_empty_content() {
    assert_parses("mat#\"\"");
}

#[test]
fn test_composite_as_nested_expression() {
    // Block expression with let binding
    assert_parses("{ let x = mat#\"[[1, 2], [3, 4]]\"; x }");
}
