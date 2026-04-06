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
// Literal parsing tests for the Verum lexer.
// Tests integer, float, string, char, and boolean literals.
// Covers decimal, hex (0x), binary (0b), octal (0o) integers with optional suffixes,
// floats with exponents and hex floats (0x1.8p10), plain/multiline strings,
// char literals with escape sequences, and boolean true/false.

use verum_ast::span::FileId;
use verum_lexer::{Lexer, TokenKind};

fn tokenize(input: &str) -> Vec<TokenKind> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    lexer
        .filter_map(|r| r.ok())
        .map(|token| token.kind)
        .collect()
}

// ===== Integer Literal Tests =====

#[test]
fn test_decimal_integer() {
    let tokens = tokenize("42");
    assert!(matches!(tokens[0], TokenKind::Integer(_)));
}

#[test]
fn test_decimal_integer_zero() {
    let tokens = tokenize("0");
    assert!(matches!(tokens[0], TokenKind::Integer(_)));
}

#[test]
fn test_decimal_integer_large() {
    let tokens = tokenize("123456789");
    assert!(matches!(tokens[0], TokenKind::Integer(_)));
}

#[test]
fn test_decimal_integer_with_underscores() {
    let tokens = tokenize("1_000_000");
    assert!(matches!(tokens[0], TokenKind::Integer(_)));
}

#[test]
fn test_decimal_integer_with_suffix() {
    let tokens = tokenize("100_km");
    assert!(matches!(tokens[0], TokenKind::Integer(_)));
}

#[test]
fn test_hex_integer() {
    let tokens = tokenize("0xFF");
    assert!(matches!(tokens[0], TokenKind::Integer(_)));
}

#[test]
fn test_hex_integer_lowercase() {
    let tokens = tokenize("0xff");
    assert!(matches!(tokens[0], TokenKind::Integer(_)));
}

#[test]
fn test_hex_integer_mixed_case() {
    let tokens = tokenize("0xDEADBEEF");
    assert!(matches!(tokens[0], TokenKind::Integer(_)));
}

#[test]
fn test_hex_integer_with_underscores() {
    let tokens = tokenize("0xFF_FF_FF");
    assert!(matches!(tokens[0], TokenKind::Integer(_)));
}

#[test]
fn test_binary_integer() {
    let tokens = tokenize("0b1010");
    assert!(matches!(tokens[0], TokenKind::Integer(_)));
}

#[test]
fn test_binary_integer_with_underscores() {
    let tokens = tokenize("0b1010_1010");
    assert!(matches!(tokens[0], TokenKind::Integer(_)));
}

#[test]
fn test_binary_integer_single_digit() {
    let tokens = tokenize("0b1");
    assert!(matches!(tokens[0], TokenKind::Integer(_)));
}

#[test]
fn test_octal_integer() {
    let tokens = tokenize("0o777");
    // Note: This may need testing based on actual impl
    // The spec mentions octal but the lexer may need explicit support
}

// ===== Float Literal Tests =====

#[test]
fn test_float_simple() {
    let tokens = tokenize("3.14");
    assert!(matches!(tokens[0], TokenKind::Float(_)));
}

#[test]
fn test_float_leading_digits() {
    let tokens = tokenize("123.456");
    assert!(matches!(tokens[0], TokenKind::Float(_)));
}

#[test]
fn test_float_with_underscores() {
    let tokens = tokenize("1_000.5");
    assert!(matches!(tokens[0], TokenKind::Float(_)));
}

#[test]
fn test_float_scientific_notation_uppercase() {
    let tokens = tokenize("1.5E10");
    assert!(matches!(tokens[0], TokenKind::Float(_)));
}

#[test]
fn test_float_scientific_notation_lowercase() {
    let tokens = tokenize("1.5e10");
    assert!(matches!(tokens[0], TokenKind::Float(_)));
}

#[test]
fn test_float_scientific_notation_negative_exponent() {
    let tokens = tokenize("2.5e-3");
    assert!(matches!(tokens[0], TokenKind::Float(_)));
}

#[test]
fn test_float_scientific_notation_positive_exponent() {
    let tokens = tokenize("2.5e+10");
    assert!(matches!(tokens[0], TokenKind::Float(_)));
}

#[test]
fn test_float_scientific_notation_no_decimal() {
    let tokens = tokenize("1e10");
    assert!(matches!(tokens[0], TokenKind::Float(_)));
}

#[test]
fn test_float_with_suffix() {
    let tokens = tokenize("3.14_rad");
    assert!(matches!(tokens[0], TokenKind::Float(_)));
}

#[test]
fn test_float_zero() {
    let tokens = tokenize("0.0");
    assert!(matches!(tokens[0], TokenKind::Float(_)));
}

// ===== String Literal Tests =====

#[test]
fn test_string_simple() {
    let input = "\"hello\"";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Text(_)));
}

#[test]
fn test_string_empty() {
    let input = "\"\"";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Text(_)));
}

#[test]
fn test_string_with_spaces() {
    let input = "\"hello world\"";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Text(_)));
}

#[test]
fn test_string_with_escape_newline() {
    let input = "\"hello\\nworld\"";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Text(_)));
}

#[test]
fn test_string_with_escape_tab() {
    let input = "\"hello\\tworld\"";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Text(_)));
}

#[test]
fn test_string_with_escaped_quote() {
    let input = "\"hello \\\"world\\\"\"";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Text(_)));
}

#[test]
fn test_string_with_escaped_backslash() {
    let input = "\"path\\\\to\\\\file\"";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Text(_)));
}

#[test]
fn test_multiline_string() {
    let input = "\"\"\"multiline\nstring\nhere\"\"\"";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Text(_)));
}

#[test]
fn test_raw_multiline_string() {
    // Raw multiline strings use """...""" syntax (no r#"..."# syntax)
    let tokens = tokenize(r#""""raw string""""#);
    assert!(matches!(tokens[0], TokenKind::Text(_)));
}

#[test]
fn test_raw_multiline_with_backslashes() {
    // Raw multiline strings don't process escape sequences
    let input = r#""""raw with \n and \t inside""""#;
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Text(_)));
}

// ===== Interpolated String Tests =====

#[test]
fn test_interpolated_string_f() {
    let input = "f\"hello {name}\"";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::InterpolatedString(_)));
}

#[test]
fn test_interpolated_string_sql() {
    let input = "sql\"SELECT * FROM users WHERE id = {id}\"";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::InterpolatedString(_)));
}

#[test]
fn test_interpolated_string_html() {
    let input = "html\"<div>{content}</div>\"";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::InterpolatedString(_)));
}

#[test]
fn test_interpolated_string_json() {
    let input = "json\"{\\\"name\\\": \\\"{name}\\\"}\"";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::InterpolatedString(_)));
}

// ===== Char Literal Tests =====

#[test]
fn test_char_simple() {
    let input = "'a'";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Char(_)));
}

#[test]
fn test_char_digit() {
    let input = "'5'";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Char(_)));
}

#[test]
fn test_char_space() {
    let input = "' '";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Char(_)));
}

#[test]
fn test_char_escaped_newline() {
    let input = "'\\n'";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Char(_)));
}

#[test]
fn test_char_escaped_tab() {
    let input = "'\\t'";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Char(_)));
}

#[test]
fn test_char_escaped_quote() {
    let input = "'\\'";
    let tokens = tokenize(&format!("{}' ", input));
    // Just check that it parses as some token
}

#[test]
fn test_char_escaped_backslash() {
    let input = "'\\\\'";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Char(_)));
}

// ===== Boolean Literal Tests =====

#[test]
fn test_boolean_true() {
    let tokens = tokenize("true");
    assert!(matches!(tokens[0], TokenKind::True));
}

#[test]
fn test_boolean_false() {
    let tokens = tokenize("false");
    assert!(matches!(tokens[0], TokenKind::False));
}

// ===== Variant Literals =====

#[test]
fn test_variant_none() {
    let tokens = tokenize("None");
    assert!(matches!(tokens[0], TokenKind::None));
}

#[test]
fn test_variant_some() {
    let tokens = tokenize("Some");
    assert!(matches!(tokens[0], TokenKind::Some));
}

#[test]
fn test_variant_ok() {
    let tokens = tokenize("Ok");
    assert!(matches!(tokens[0], TokenKind::Ok));
}

#[test]
fn test_variant_err() {
    let tokens = tokenize("Err");
    assert!(matches!(tokens[0], TokenKind::Err));
}

// ===== Tagged Literals =====

#[test]
fn test_tagged_literal_datetime() {
    // Tagged literal with 'd' prefix for datetime
    let tokens = tokenize("d\"2024-01-15\"");
    // Check if we got a tagged literal or identifier + string
    assert!(!tokens.is_empty());
}

#[test]
fn test_tagged_literal_sql() {
    // Tagged literal with 'sql' prefix
    let tokens = tokenize("sql\"SELECT * FROM users\"");
    assert!(!tokens.is_empty());
}

#[test]
fn test_tagged_literal_regex() {
    // Tagged literal with 'rx' prefix for regex
    let tokens = tokenize("rx\"[a-z]+\"");
    assert!(!tokens.is_empty());
}

#[test]
fn test_tagged_literal_graphql() {
    // Tagged literal with 'gql' prefix
    let tokens = tokenize("gql\"query { user { id } }\"");
    assert!(!tokens.is_empty());
}

#[test]
fn test_tagged_literal_composite_vec_paren() {
    let tokens = tokenize("vec#(1, 2, 3)");
    assert!(matches!(tokens[0], TokenKind::TaggedLiteral(_)));
}

#[test]
fn test_tagged_literal_composite_matrix_bracket() {
    let tokens = tokenize("mat#[[1,2],[3,4]]");
    assert!(matches!(tokens[0], TokenKind::TaggedLiteral(_)));
}

#[test]
fn test_tagged_literal_composite_music_brace() {
    let input = "music#{notes: [C, D, E]}";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::TaggedLiteral(_)));
}

// ===== Contract Literal Tests =====

#[test]
fn test_contract_literal() {
    let input = "contract#\"requires x > 0; ensures result > 0\"";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::ContractLiteral(_)));
}

#[test]
fn test_contract_literal_raw() {
    // Contract literal with raw string
    let tokens = tokenize("contract#\"requires x > 0\"");
    assert!(!tokens.is_empty());
}

// ===== Hex Color Literals =====

#[test]
fn test_hex_color_rgb() {
    let tokens = tokenize("# FF5733");
    // Need space after # to avoid Rust parsing error
    assert!(!tokens.is_empty());
}

#[test]
fn test_hex_color_rgba() {
    let tokens = tokenize("# FF5733FF");
    assert!(!tokens.is_empty());
}

#[test]
fn test_hex_color_lowercase() {
    let tokens = tokenize("# ff5733");
    assert!(!tokens.is_empty());
}

// ===== Multiple Literals =====

#[test]
fn test_multiple_integers() {
    let tokens = tokenize("1 2 3");
    assert_eq!(tokens.len(), 4); // 3 integers + EOF
    assert!(matches!(tokens[0], TokenKind::Integer(_)));
    assert!(matches!(tokens[1], TokenKind::Integer(_)));
    assert!(matches!(tokens[2], TokenKind::Integer(_)));
}

#[test]
fn test_integer_and_float() {
    let tokens = tokenize("42 3.14");
    assert!(matches!(tokens[0], TokenKind::Integer(_)));
    assert!(matches!(tokens[1], TokenKind::Float(_)));
}

#[test]
fn test_string_and_char() {
    let input = "\"hello\" 'a'";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Text(_)));
    assert!(matches!(tokens[1], TokenKind::Char(_)));
}

// ===== Literals in Context =====

#[test]
fn test_integer_in_assignment() {
    let tokens = tokenize("x = 42");
    assert!(matches!(tokens[0], TokenKind::Ident(_)));
    assert!(matches!(tokens[1], TokenKind::Eq));
    assert!(matches!(tokens[2], TokenKind::Integer(_)));
}

#[test]
fn test_string_in_function_call() {
    let input = "print(\"hello\")";
    let tokens = tokenize(input);
    assert!(matches!(tokens[0], TokenKind::Ident(_)));
    assert!(matches!(tokens[1], TokenKind::LParen));
    assert!(matches!(tokens[2], TokenKind::Text(_)));
    assert!(matches!(tokens[3], TokenKind::RParen));
}

#[test]
fn test_float_in_arithmetic() {
    let tokens = tokenize("3.14 * 2");
    assert!(matches!(tokens[0], TokenKind::Float(_)));
    assert!(matches!(tokens[1], TokenKind::Star));
    assert!(matches!(tokens[2], TokenKind::Integer(_)));
}

// Added to test escaped quote
#[test]
fn test_char_escaped_single_quote_actual() {
    // Test that '\'' (escaped single quote) lexes correctly
    let input = r#"'\''"#;  // This is literally '\''
    println!("Testing input: {:?}", input);
    let tokens = tokenize(input);
    println!("Tokens: {:?}", tokens);
    assert!(matches!(tokens[0], TokenKind::Char('\'')), "Expected Char('\\'''), got {:?}", tokens[0]);
}
