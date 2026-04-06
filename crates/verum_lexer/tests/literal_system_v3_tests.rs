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
    unused_assignments,
    clippy::approx_constant
)]
// Revolutionary Literal System v3.0 Tests
//
// Comprehensive test suite for Verum's Revolutionary Literal System including:
// - Tagged literals (sql#, html#, regex#, etc.)
// - Interpolated strings (f"", sql"", html"", etc.)
// - Composite literals (matrix#, unit#, interval#)
// - Hex color literals (#FF5733, #RGB)
// - Scientific notation (1.5e10, 3.14E-5)
// - Unit suffixes (100_km, 90_deg, 20_C)
//
// Tests the complete Verum literal system as defined in the lexical grammar.

use verum_ast::span::FileId;
use verum_lexer::{Lexer, TokenKind};
use verum_common::Text;

/// Helper to tokenize a string and extract token kinds.
fn tokenize(source: &str) -> Vec<TokenKind> {
    let file_id = FileId::new(0);
    Lexer::new(source, file_id)
        .map(|r| r.unwrap().kind)
        .collect()
}

/// Helper to get the first token from source.
fn first_token(source: &str) -> TokenKind {
    tokenize(source).into_iter().next().unwrap()
}

// =============================================================================
// Tagged Literals - Compile-Time Parsing
// Grammar: tagged_literal = identifier '#' tagged_content
// Grammar: interpolated_string = identifier '"' { string_char | interpolation } '"'
// =============================================================================

#[test]
fn test_tagged_literal_sql() {
    let source = r#"sql#"SELECT * FROM users""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("sql"));
            assert_eq!(data.content, Text::from("SELECT * FROM users"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_regex() {
    let source = r#"rx#"[a-zA-Z0-9]+""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("rx"));
            assert_eq!(data.content, Text::from("[a-zA-Z0-9]+"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_datetime() {
    let source = r#"d#"2025-11-15""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("d"));
            assert_eq!(data.content, Text::from("2025-11-15"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_graphql() {
    let source = r#"gql#"query { user { name } }""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("gql"));
            assert_eq!(data.content, Text::from("query { user { name } }"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_json() {
    let source = r#"json#"{\"key\": \"value\"}""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("json"));
            assert_eq!(data.content, Text::from(r#"{"key": "value"}"#));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_xml() {
    let source = r#"xml#"<root><child>text</child></root>""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("xml"));
            assert_eq!(data.content, Text::from("<root><child>text</child></root>"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_yaml() {
    let source = r#"yaml#"key: value\n  nested: item""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("yaml"));
            assert!(data.content.contains("key: value"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_uri() {
    let source = r#"uri#"https://example.com/path?query=value""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("uri"));
            assert_eq!(
                data.content,
                Text::from("https://example.com/path?query=value")
            );
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_url() {
    let source = r#"url#"https://api.example.com/v1/users""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("url"));
            assert_eq!(data.content, Text::from("https://api.example.com/v1/users"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_email() {
    let source = r#"email#"user@example.com""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("email"));
            assert_eq!(data.content, Text::from("user@example.com"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_interval() {
    let source = r#"interval#"[0, 100)""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("interval"));
            assert_eq!(data.content, Text::from("[0, 100)"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_matrix() {
    let source = r#"mat#"[[1, 2], [3, 4]]""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("mat"));
            assert_eq!(data.content, Text::from("[[1, 2], [3, 4]]"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_vector() {
    let source = r#"vec#"<1, 2, 3>""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("vec"));
            assert_eq!(data.content, Text::from("<1, 2, 3>"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_chemistry() {
    let source = r#"chem#"H2O""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("chem"));
            assert_eq!(data.content, Text::from("H2O"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_music() {
    let source = r#"music#"Cmaj7""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("music"));
            assert_eq!(data.content, Text::from("Cmaj7"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_with_escapes() {
    let source = r#"sql#"SELECT * FROM \"users\"\\n""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("sql"));
            assert!(data.content.contains("SELECT"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_multiline_raw() {
    // Tagged literal with multiline raw syntax: tag#"""..."""
    // NOTE: tag#r#"..."# syntax removed in simplified literal architecture
    let source = r#"sql#"""SELECT * FROM users""""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("sql"));
            assert_eq!(data.content, Text::from("SELECT * FROM users"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

#[test]
fn test_tagged_literal_multiline_raw_with_special_chars() {
    // Tagged literal with special characters preserved (raw - no escapes)
    let source = r#"rx#"""[a-z]+\d*""""#;
    let token = first_token(source);

    match token {
        TokenKind::TaggedLiteral(data) => {
            assert_eq!(data.tag, Text::from("rx"));
            // Backslash-d is preserved literally (no escape processing)
            assert!(data.content.contains("\\d"));
        }
        _ => panic!("Expected TaggedLiteral, got {:?}", token),
    }
}

// =============================================================================
// Interpolated Strings - Safe Runtime Interpolation
// Grammar: interpolated_string = identifier '"' { string_char | interpolation } '"'
// Prefixes: f (format), sql (injection-safe), html (XSS-safe), url (encoding), gql, etc.
// =============================================================================

#[test]
fn test_interpolated_string_format() {
    let source = r#"f"Hello {name}""#;
    let token = first_token(source);

    match token {
        TokenKind::InterpolatedString(data) => {
            assert_eq!(data.prefix, Text::from("f"));
            assert_eq!(data.content, Text::from("Hello {name}"));
        }
        _ => panic!("Expected InterpolatedString, got {:?}", token),
    }
}

#[test]
fn test_interpolated_string_sql() {
    let source = r#"sql"SELECT * FROM users WHERE id = {user_id}""#;
    let token = first_token(source);

    match token {
        TokenKind::InterpolatedString(data) => {
            assert_eq!(data.prefix, Text::from("sql"));
            assert!(data.content.contains("{user_id}"));
        }
        _ => panic!("Expected InterpolatedString, got {:?}", token),
    }
}

#[test]
fn test_interpolated_string_html() {
    let source = r#"html"<h1>{title}</h1><p>{content}</p>""#;
    let token = first_token(source);

    match token {
        TokenKind::InterpolatedString(data) => {
            assert_eq!(data.prefix, Text::from("html"));
            assert!(data.content.contains("{title}"));
            assert!(data.content.contains("{content}"));
        }
        _ => panic!("Expected InterpolatedString, got {:?}", token),
    }
}

#[test]
fn test_interpolated_string_url() {
    let source = r#"url"https://api.example.com/users/{id}""#;
    let token = first_token(source);

    match token {
        TokenKind::InterpolatedString(data) => {
            assert_eq!(data.prefix, Text::from("url"));
            assert!(data.content.contains("{id}"));
        }
        _ => panic!("Expected InterpolatedString, got {:?}", token),
    }
}

#[test]
fn test_interpolated_string_json() {
    let source = r#"json"{\"name\": \"{name}\", \"age\": {age}}""#;
    let token = first_token(source);

    match token {
        TokenKind::InterpolatedString(data) => {
            assert_eq!(data.prefix, Text::from("json"));
            assert!(data.content.contains("{name}"));
            assert!(data.content.contains("{age}"));
        }
        _ => panic!("Expected InterpolatedString, got {:?}", token),
    }
}

#[test]
fn test_interpolated_string_xml() {
    let source = r#"xml"<user><name>{name}</name><id>{id}</id></user>""#;
    let token = first_token(source);

    match token {
        TokenKind::InterpolatedString(data) => {
            assert_eq!(data.prefix, Text::from("xml"));
            assert!(data.content.contains("{name}"));
        }
        _ => panic!("Expected InterpolatedString, got {:?}", token),
    }
}

#[test]
fn test_interpolated_string_yaml() {
    let source = r#"yaml"name: {name}\nage: {age}""#;
    let token = first_token(source);

    match token {
        TokenKind::InterpolatedString(data) => {
            assert_eq!(data.prefix, Text::from("yaml"));
            assert!(data.content.contains("{name}"));
        }
        _ => panic!("Expected InterpolatedString, got {:?}", token),
    }
}

#[test]
fn test_interpolated_string_multiple_expressions() {
    let source = r#"f"User {user.name} is {user.age} years old and lives in {user.city}""#;
    let token = first_token(source);

    match token {
        TokenKind::InterpolatedString(data) => {
            assert_eq!(data.prefix, Text::from("f"));
            assert!(data.content.contains("{user.name}"));
            assert!(data.content.contains("{user.age}"));
            assert!(data.content.contains("{user.city}"));
        }
        _ => panic!("Expected InterpolatedString, got {:?}", token),
    }
}

#[test]
fn test_interpolated_string_with_escapes() {
    let source = r#"f"Hello\n{name}\t{age}""#;
    let token = first_token(source);

    match token {
        TokenKind::InterpolatedString(data) => {
            assert_eq!(data.prefix, Text::from("f"));
            assert!(data.content.contains("{name}"));
        }
        _ => panic!("Expected InterpolatedString, got {:?}", token),
    }
}

#[test]
fn test_interpolated_string_nested_braces() {
    let source = r#"f"Result: {compute({x, y})}""#;
    let token = first_token(source);

    match token {
        TokenKind::InterpolatedString(data) => {
            assert_eq!(data.prefix, Text::from("f"));
            assert!(data.content.contains("compute"));
        }
        _ => panic!("Expected InterpolatedString, got {:?}", token),
    }
}

#[test]
fn test_interpolated_string_gql() {
    let source = r#"gql"query GetUser($id: ID!) { user(id: {userId}) { name } }""#;
    let token = first_token(source);

    match token {
        TokenKind::InterpolatedString(data) => {
            assert_eq!(data.prefix, Text::from("gql"));
            assert!(data.content.contains("{userId}"));
        }
        _ => panic!("Expected InterpolatedString, got {:?}", token),
    }
}

// =============================================================================
// Hex Color Literals - Context-Adaptive
// Grammar: hex_color_literal = '#' hex_digit{6} [hex_digit{2}]
// =============================================================================

#[test]
fn test_hex_color_rgb() {
    let source = "#FF5733";
    let token = first_token(source);

    match token {
        TokenKind::HexColor(ref color) => {
            assert_eq!(color, &Text::from("FF5733"));
        }
        _ => panic!("Expected HexColor, got {:?}", token),
    }
}

#[test]
fn test_hex_color_rgba() {
    let source = "#FF5733AA";
    let token = first_token(source);

    match token {
        TokenKind::HexColor(ref color) => {
            assert_eq!(color, &Text::from("FF5733AA"));
        }
        _ => panic!("Expected HexColor, got {:?}", token),
    }
}

#[test]
fn test_hex_color_lowercase() {
    let source = "#ff5733";
    let token = first_token(source);

    match token {
        TokenKind::HexColor(ref color) => {
            assert_eq!(color, &Text::from("ff5733"));
        }
        _ => panic!("Expected HexColor, got {:?}", token),
    }
}

#[test]
fn test_hex_color_mixed_case() {
    let source = "#Ff5733Aa";
    let token = first_token(source);

    match token {
        TokenKind::HexColor(ref color) => {
            assert_eq!(color, &Text::from("Ff5733Aa"));
        }
        _ => panic!("Expected HexColor, got {:?}", token),
    }
}

#[test]
fn test_hex_color_black() {
    let source = "#000000";
    let token = first_token(source);

    match token {
        TokenKind::HexColor(ref color) => {
            assert_eq!(color, &Text::from("000000"));
        }
        _ => panic!("Expected HexColor, got {:?}", token),
    }
}

#[test]
fn test_hex_color_white() {
    let source = "#FFFFFF";
    let token = first_token(source);

    match token {
        TokenKind::HexColor(ref color) => {
            assert_eq!(color, &Text::from("FFFFFF"));
        }
        _ => panic!("Expected HexColor, got {:?}", token),
    }
}

#[test]
fn test_hex_color_transparent() {
    let source = "#00000000";
    let token = first_token(source);

    match token {
        TokenKind::HexColor(ref color) => {
            assert_eq!(color, &Text::from("00000000"));
        }
        _ => panic!("Expected HexColor, got {:?}", token),
    }
}

#[test]
fn test_multiple_hex_colors() {
    let source = "#FF0000 #00FF00 #0000FF";
    let tokens = tokenize(source);

    assert_eq!(tokens.len(), 4); // 3 colors + EOF

    match &tokens[0] {
        TokenKind::HexColor(color) => assert_eq!(color, &Text::from("FF0000")),
        _ => panic!("Expected HexColor"),
    }

    match &tokens[1] {
        TokenKind::HexColor(color) => assert_eq!(color, &Text::from("00FF00")),
        _ => panic!("Expected HexColor"),
    }

    match &tokens[2] {
        TokenKind::HexColor(color) => assert_eq!(color, &Text::from("0000FF")),
        _ => panic!("Expected HexColor"),
    }
}

// =============================================================================
// Scientific Notation - Float Literals
// Grammar: float_lit = decimal '.' decimal [exponent] ['_' identifier]
// exponent = ('e' | 'E') ['+' | '-'] decimal
// =============================================================================

#[test]
fn test_scientific_notation_positive_exponent() {
    let source = "1.5e10";
    let token = first_token(source);

    match token {
        TokenKind::Float(ref lit) => {
            assert_eq!(lit.value, 1.5e10);
            assert!(lit.suffix.is_none());
        }
        _ => panic!("Expected Float, got {:?}", token),
    }
}

#[test]
fn test_scientific_notation_negative_exponent() {
    let source = "3.14E-5";
    let token = first_token(source);

    match token {
        TokenKind::Float(ref lit) => {
            assert!((lit.value - 3.14e-5).abs() < 1e-10);
            assert!(lit.suffix.is_none());
        }
        _ => panic!("Expected Float, got {:?}", token),
    }
}

#[test]
fn test_scientific_notation_uppercase_e() {
    let source = "2.5E8";
    let token = first_token(source);

    match token {
        TokenKind::Float(ref lit) => {
            assert_eq!(lit.value, 2.5e8);
        }
        _ => panic!("Expected Float, got {:?}", token),
    }
}

#[test]
fn test_scientific_notation_lowercase_e() {
    let source = "6.022e23";
    let token = first_token(source);

    match token {
        TokenKind::Float(ref lit) => {
            assert_eq!(lit.value, 6.022e23);
        }
        _ => panic!("Expected Float, got {:?}", token),
    }
}

#[test]
fn test_scientific_notation_with_plus() {
    let source = "1.23e+4";
    let token = first_token(source);

    match token {
        TokenKind::Float(ref lit) => {
            assert_eq!(lit.value, 1.23e4);
        }
        _ => panic!("Expected Float, got {:?}", token),
    }
}

#[test]
fn test_scientific_notation_zero_exponent() {
    let source = "5.0e0";
    let token = first_token(source);

    match token {
        TokenKind::Float(ref lit) => {
            assert_eq!(lit.value, 5.0);
        }
        _ => panic!("Expected Float, got {:?}", token),
    }
}

#[test]
fn test_scientific_notation_large_exponent() {
    let source = "1.0e308";
    let token = first_token(source);

    match token {
        TokenKind::Float(ref lit) => {
            assert_eq!(lit.value, 1.0e308);
        }
        _ => panic!("Expected Float, got {:?}", token),
    }
}

#[test]
fn test_scientific_notation_small_exponent() {
    let source = "1.0e-308";
    let token = first_token(source);

    match token {
        TokenKind::Float(ref lit) => {
            assert_eq!(lit.value, 1.0e-308);
        }
        _ => panic!("Expected Float, got {:?}", token),
    }
}

#[test]
fn test_scientific_notation_with_underscores() {
    let source = "1_000.5e1_0";
    let token = first_token(source);

    match token {
        TokenKind::Float(ref lit) => {
            assert_eq!(lit.value, 1000.5e10);
        }
        _ => panic!("Expected Float, got {:?}", token),
    }
}

// =============================================================================
// Unit Suffixes - Type-Safe Units of Measure
// Grammar: integer_lit = (decimal_lit | hex_lit | bin_lit) ['_' identifier]
// Examples: 100_km, 90_deg, 20_C, 1024_MB (suffix specifies unit type)
// =============================================================================

#[test]
fn test_unit_suffix_kilometers() {
    let source = "100_km";
    let token = first_token(source);

    match token {
        TokenKind::Integer(ref lit) => {
            assert_eq!(lit.as_i64(), Some(100));
            assert_eq!(lit.suffix.as_deref(), Some("km"));
        }
        _ => panic!("Expected Integer with suffix, got {:?}", token),
    }
}

#[test]
fn test_unit_suffix_degrees() {
    let source = "90_deg";
    let token = first_token(source);

    match token {
        TokenKind::Integer(ref lit) => {
            assert_eq!(lit.as_i64(), Some(90));
            assert_eq!(lit.suffix.as_deref(), Some("deg"));
        }
        _ => panic!("Expected Integer with suffix, got {:?}", token),
    }
}

#[test]
fn test_unit_suffix_celsius() {
    let source = "20_C";
    let token = first_token(source);

    match token {
        TokenKind::Integer(ref lit) => {
            assert_eq!(lit.as_i64(), Some(20));
            assert_eq!(lit.suffix.as_deref(), Some("C"));
        }
        _ => panic!("Expected Integer with suffix, got {:?}", token),
    }
}

#[test]
fn test_unit_suffix_megabytes() {
    let source = "1024_MB";
    let token = first_token(source);

    match token {
        TokenKind::Integer(ref lit) => {
            assert_eq!(lit.as_i64(), Some(1024));
            assert_eq!(lit.suffix.as_deref(), Some("MB"));
        }
        _ => panic!("Expected Integer with suffix, got {:?}", token),
    }
}

#[test]
fn test_unit_suffix_float() {
    let source = "3.14_rad";
    let token = first_token(source);

    match token {
        TokenKind::Float(ref lit) => {
            assert!((lit.value - 3.14).abs() < 0.001);
            assert_eq!(lit.suffix.as_deref(), Some("rad"));
        }
        _ => panic!("Expected Float with suffix, got {:?}", token),
    }
}

#[test]
fn test_unit_suffix_meters_per_second() {
    // Note: Multiple underscores in suffix may not be supported
    // Testing simple compound unit instead
    let source = "9.8_mps";
    let token = first_token(source);

    match token {
        TokenKind::Float(ref lit) => {
            assert!((lit.value - 9.8).abs() < 0.001);
            assert_eq!(lit.suffix.as_deref(), Some("mps"));
        }
        _ => panic!("Expected Float with suffix, got {:?}", token),
    }
}

#[test]
fn test_unit_suffix_seconds() {
    let source = "60_sec";
    let token = first_token(source);

    match token {
        TokenKind::Integer(ref lit) => {
            assert_eq!(lit.as_i64(), Some(60));
            assert_eq!(lit.suffix.as_deref(), Some("sec"));
        }
        _ => panic!("Expected Integer with suffix, got {:?}", token),
    }
}

#[test]
fn test_unit_suffix_hours() {
    let source = "24_hours";
    let token = first_token(source);

    match token {
        TokenKind::Integer(ref lit) => {
            assert_eq!(lit.as_i64(), Some(24));
            assert_eq!(lit.suffix.as_deref(), Some("hours"));
        }
        _ => panic!("Expected Integer with suffix, got {:?}", token),
    }
}

#[test]
fn test_unit_suffix_with_underscores() {
    let source = "1_000_000_bytes";
    let token = first_token(source);

    match token {
        TokenKind::Integer(ref lit) => {
            assert_eq!(lit.as_i64(), Some(1_000_000));
            assert_eq!(lit.suffix.as_deref(), Some("bytes"));
        }
        _ => panic!("Expected Integer with suffix, got {:?}", token),
    }
}

#[test]
fn test_unit_suffix_scientific_notation() {
    let source = "1.5e10_Hz";
    let token = first_token(source);

    match token {
        TokenKind::Float(ref lit) => {
            assert_eq!(lit.value, 1.5e10);
            assert_eq!(lit.suffix.as_deref(), Some("Hz"));
        }
        _ => panic!("Expected Float with suffix, got {:?}", token),
    }
}

// =============================================================================
// Raw String Literals - Deep Nesting
// Grammar: multiline_string = '"""' { any_char } '"""' (no escape processing)
// NOTE: r#"..."# syntax removed - use """...""" for raw strings
// =============================================================================

#[test]
fn test_raw_multiline_basic() {
    // Triple-quote strings are raw (no escape processing)
    let source = r#""""hello world""""#;
    let token = first_token(source);

    match token {
        TokenKind::Text(ref s) => {
            assert_eq!(s, &Text::from("hello world"));
        }
        _ => panic!("Expected String, got {:?}", token),
    }
}

#[test]
fn test_raw_multiline_with_quotes() {
    // Single/double quotes are fine inside triple-quote strings
    // Just need to avoid three consecutive quotes which ends the string
    let source = r#""""She said 'hello' today""""#;
    let token = first_token(source);

    match token {
        TokenKind::Text(ref s) => {
            assert_eq!(s, &Text::from("She said 'hello' today"));
        }
        _ => panic!("Expected String, got {:?}", token),
    }
}

#[test]
fn test_raw_multiline_with_hash_chars() {
    // Hash characters are just regular content in raw strings
    let source = r#""""String with # and ## inside""""#;
    let token = first_token(source);

    match token {
        TokenKind::Text(ref s) => {
            assert!(s.contains("#"));
            assert!(s.contains("##"));
        }
        _ => panic!("Expected String, got {:?}", token),
    }
}

#[test]
fn test_raw_multiline_deeply_nested_content() {
    // Complex content with various special chars
    let source = r#""""Nested "quotes" and 'apostrophes' work""""#;
    let token = first_token(source);

    match token {
        TokenKind::Text(ref s) => {
            assert!(s.contains("\"quotes\""));
            assert!(s.contains("'apostrophes'"));
        }
        _ => panic!("Expected String, got {:?}", token),
    }
}

#[test]
fn test_raw_multiline_no_escapes() {
    // Escape sequences are NOT processed in triple-quote strings
    let source = r#""""No \n escapes \t here""""#;
    let token = first_token(source);

    match token {
        TokenKind::Text(ref s) => {
            // Backslash-n stays as literal \n
            assert_eq!(s, &Text::from(r"No \n escapes \t here"));
        }
        _ => panic!("Expected String, got {:?}", token),
    }
}

#[test]
fn test_raw_multiline_actual_multiline() {
    // Triple-quote strings can span multiple lines
    let source = r#""""Line 1
Line 2
Line 3""""#;
    let token = first_token(source);

    match token {
        TokenKind::Text(ref s) => {
            assert!(s.contains("Line 1"));
            assert!(s.contains("Line 2"));
            assert!(s.contains("Line 3"));
        }
        _ => panic!("Expected String, got {:?}", token),
    }
}

// =============================================================================
// Multiline String Literals
// Grammar: multiline_string = '"""' { char_except_newline | '\n' | '\r\n' } '"""'
// =============================================================================

#[test]
fn test_multiline_string_basic() {
    let source = r#""""Hello
World""""#;
    let token = first_token(source);

    match token {
        TokenKind::Text(ref s) => {
            assert!(s.contains("Hello"));
            assert!(s.contains("World"));
        }
        _ => panic!("Expected String, got {:?}", token),
    }
}

#[test]
fn test_multiline_string_preserve_formatting() {
    let source = r#""""    Indented
  Less indented
No indent""""#;
    let token = first_token(source);

    match token {
        TokenKind::Text(ref s) => {
            assert!(s.contains("    Indented"));
            assert!(s.contains("  Less indented"));
            assert!(s.contains("No indent"));
        }
        _ => panic!("Expected String, got {:?}", token),
    }
}

#[test]
fn test_multiline_string_empty_lines() {
    let source = r#""""Line 1

Line 3""""#;
    let token = first_token(source);

    match token {
        TokenKind::Text(ref s) => {
            assert!(s.contains("Line 1"));
            assert!(s.contains("Line 3"));
        }
        _ => panic!("Expected String, got {:?}", token),
    }
}

// =============================================================================
// Integration Tests - Complex Literal Combinations
// =============================================================================

#[test]
fn test_mixed_literals_in_expression() {
    let source = r#"let distance = 100_km + 5_mi"#;
    let tokens = tokenize(source);

    // Should parse: let, distance, =, 100_km, +, 5_mi, EOF
    assert!(tokens.len() >= 6);

    match &tokens[3] {
        TokenKind::Integer(lit) => {
            assert_eq!(lit.as_i64(), Some(100));
            assert_eq!(lit.suffix.as_deref(), Some("km"));
        }
        _ => panic!("Expected Integer with km suffix"),
    }

    match &tokens[5] {
        TokenKind::Integer(lit) => {
            assert_eq!(lit.as_i64(), Some(5));
            assert_eq!(lit.suffix.as_deref(), Some("mi"));
        }
        _ => panic!("Expected Integer with mi suffix"),
    }
}

#[test]
fn test_tagged_and_interpolated_together() {
    let source = r#"let query = sql#"SELECT * FROM users" let msg = f"Found {count} users""#;
    let tokens = tokenize(source);

    // Should have both tagged literal and interpolated string
    let has_tagged = tokens
        .iter()
        .any(|t| matches!(t, TokenKind::TaggedLiteral(_)));
    let has_interpolated = tokens
        .iter()
        .any(|t| matches!(t, TokenKind::InterpolatedString(_)));

    assert!(has_tagged, "Should have tagged literal");
    assert!(has_interpolated, "Should have interpolated string");
}

#[test]
fn test_hex_color_in_expression() {
    let source = "let bg = #FF5733 let fg = #FFFFFF";
    let tokens = tokenize(source);

    let color_count = tokens
        .iter()
        .filter(|t| matches!(t, TokenKind::HexColor(_)))
        .count();
    assert_eq!(color_count, 2);
}

#[test]
fn test_scientific_notation_with_units() {
    // Note: Scientific notation with compound suffixes may not parse correctly
    // Testing scientific notation with simple suffix
    let source = "let speed = 3.0e8_Hz";
    let tokens = tokenize(source);

    // Find the float token in the result
    let float_token = tokens
        .iter()
        .find(|t| matches!(t, TokenKind::Float(_)))
        .expect("Should have float token");

    match float_token {
        TokenKind::Float(lit) => {
            assert_eq!(lit.value, 3.0e8);
            assert_eq!(lit.suffix.as_deref(), Some("Hz"));
        }
        _ => panic!("Expected Float with unit suffix"),
    }
}
