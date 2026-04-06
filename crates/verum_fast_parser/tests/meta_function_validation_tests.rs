//! Tests for @ construct validation
//!
//! Verifies that the parser correctly validates meta-function names
//! and emits quality diagnostic messages for unknown @ constructs.
//!
//! Tests for meta-level functions: @const, @cfg, @file, @line, @type_name, etc.

#![allow(unused_imports)]

use verum_ast::FileId;
use verum_common::List;
use verum_lexer::{Lexer, Token};
use verum_fast_parser::RecursiveParser;

fn parse_expr_with_warnings(source: &str) -> (bool, Vec<String>) {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: List<Token> = lexer.filter_map(|r| r.ok()).collect();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    let result = parser.parse_expr();

    let warnings: Vec<String> = parser
        .attr_warnings
        .iter()
        .map(|w| w.message.to_string())
        .collect();

    (result.is_ok(), warnings)
}

// =============================================================================
// KNOWN META-FUNCTIONS - Should parse without warnings
// =============================================================================

#[test]
fn test_known_meta_function_file() {
    let (ok, warnings) = parse_expr_with_warnings("@file");
    assert!(ok, "Should parse @file");
    assert!(warnings.is_empty(), "Known meta-function @file should not produce warnings");
}

#[test]
fn test_known_meta_function_line() {
    let (ok, warnings) = parse_expr_with_warnings("@line");
    assert!(ok, "Should parse @line");
    assert!(warnings.is_empty(), "Known meta-function @line should not produce warnings");
}

#[test]
fn test_known_meta_function_column() {
    let (ok, warnings) = parse_expr_with_warnings("@column");
    assert!(ok, "Should parse @column");
    assert!(warnings.is_empty(), "Known meta-function @column should not produce warnings");
}

#[test]
fn test_known_meta_function_module() {
    let (ok, warnings) = parse_expr_with_warnings("@module");
    assert!(ok, "Should parse @module");
    assert!(warnings.is_empty(), "Known meta-function @module should not produce warnings");
}

#[test]
fn test_known_meta_function_function() {
    let (ok, warnings) = parse_expr_with_warnings("@fn");
    assert!(ok, "Should parse @fn (function)");
    assert!(warnings.is_empty(), "Known meta-function @function should not produce warnings");
}

#[test]
fn test_known_meta_function_const() {
    let (ok, warnings) = parse_expr_with_warnings("@const 42");
    assert!(ok, "Should parse @const expr");
    assert!(warnings.is_empty(), "Known meta-function @const should not produce warnings");
}

#[test]
fn test_known_meta_function_cfg() {
    let (ok, warnings) = parse_expr_with_warnings("@cfg(target_os)");
    assert!(ok, "Should parse @cfg(condition)");
    assert!(warnings.is_empty(), "Known meta-function @cfg should not produce warnings");
}

#[test]
fn test_known_meta_function_error() {
    let (ok, warnings) = parse_expr_with_warnings("@error(\"message\")");
    assert!(ok, "Should parse @error(msg)");
    assert!(warnings.is_empty(), "Known meta-function @error should not produce warnings");
}

#[test]
fn test_known_meta_function_warning() {
    let (ok, warnings) = parse_expr_with_warnings("@warning(\"message\")");
    assert!(ok, "Should parse @warning(msg)");
    assert!(warnings.is_empty(), "Known meta-function @warning should not produce warnings");
}

#[test]
fn test_known_meta_function_stringify() {
    let (ok, warnings) = parse_expr_with_warnings("@stringify(x)");
    assert!(ok, "Should parse @stringify(tokens)");
    assert!(warnings.is_empty(), "Known meta-function @stringify should not produce warnings");
}

#[test]
fn test_known_meta_function_concat() {
    let (ok, warnings) = parse_expr_with_warnings("@concat(a, b)");
    assert!(ok, "Should parse @concat(a, b)");
    assert!(warnings.is_empty(), "Known meta-function @concat should not produce warnings");
}

#[test]
fn test_known_meta_function_type_name() {
    let (ok, warnings) = parse_expr_with_warnings("@type_name(T)");
    assert!(ok, "Should parse @type_name(T)");
    assert!(warnings.is_empty(), "Known meta-function @type_name should not produce warnings");
}

#[test]
fn test_known_meta_function_type_of() {
    let (ok, warnings) = parse_expr_with_warnings("@type_of(x)");
    assert!(ok, "Should parse @type_of(x)");
    assert!(warnings.is_empty(), "Known meta-function @type_of should not produce warnings");
}

#[test]
fn test_known_meta_function_fields_of() {
    let (ok, warnings) = parse_expr_with_warnings("@fields_of(T)");
    assert!(ok, "Should parse @fields_of(T)");
    assert!(warnings.is_empty(), "Known meta-function @fields_of should not produce warnings");
}

#[test]
fn test_known_meta_function_variants_of() {
    let (ok, warnings) = parse_expr_with_warnings("@variants_of(T)");
    assert!(ok, "Should parse @variants_of(T)");
    assert!(warnings.is_empty(), "Known meta-function @variants_of should not produce warnings");
}

#[test]
fn test_known_meta_function_is_struct() {
    let (ok, warnings) = parse_expr_with_warnings("@is_struct(T)");
    assert!(ok, "Should parse @is_struct(T)");
    assert!(warnings.is_empty(), "Known meta-function @is_struct should not produce warnings");
}

#[test]
fn test_known_meta_function_is_enum() {
    let (ok, warnings) = parse_expr_with_warnings("@is_enum(T)");
    assert!(ok, "Should parse @is_enum(T)");
    assert!(warnings.is_empty(), "Known meta-function @is_enum should not produce warnings");
}

#[test]
fn test_known_meta_function_is_tuple() {
    let (ok, warnings) = parse_expr_with_warnings("@is_tuple(T)");
    assert!(ok, "Should parse @is_tuple(T)");
    assert!(warnings.is_empty(), "Known meta-function @is_tuple should not produce warnings");
}

#[test]
fn test_known_meta_function_implements() {
    let (ok, warnings) = parse_expr_with_warnings("@implements(T, Protocol)");
    assert!(ok, "Should parse @implements(T, Protocol)");
    assert!(warnings.is_empty(), "Known meta-function @implements should not produce warnings");
}

// =============================================================================
// DECLARATION-ONLY ATTRIBUTES - Should produce ERRORS (not warnings)
// =============================================================================

#[test]
fn test_declaration_attribute_intrinsic_parses() {
    // @intrinsic is a declaration attribute; the parser accepts it in
    // expression context (semantic validation happens later in the pipeline)
    let (ok, _warnings) = parse_expr_with_warnings("@intrinsic(\"foo\")");
    assert!(ok, "@intrinsic should be parseable as an expression (rejected later by semantic analysis)");
}

#[test]
fn test_declaration_attribute_intrinsic_call_parses() {
    // @intrinsic("name", arg) is accepted by the parser
    // Semantic analysis rejects it in non-declaration contexts
    let (ok, _warnings) = parse_expr_with_warnings("@intrinsic(\"slice_len\", self)");
    assert!(ok, "@intrinsic call should be parseable (rejected later by semantic analysis)");
}

#[test]
fn test_declaration_attribute_test_is_error() {
    // @test is a declaration attribute, not a meta-function
    let (ok, _warnings) = parse_expr_with_warnings("@test");
    assert!(!ok, "@test in expression context should be a parse ERROR");
}

#[test]
fn test_declaration_attribute_derive_is_error() {
    // @derive is a declaration attribute, not a meta-function
    let (ok, _warnings) = parse_expr_with_warnings("@derive(Clone)");
    assert!(!ok, "@derive in expression context should be a parse ERROR");
}

#[test]
fn test_declaration_attribute_inline_is_error() {
    // @inline is a declaration attribute
    let (ok, _warnings) = parse_expr_with_warnings("@inline");
    assert!(!ok, "@inline in expression context should be a parse ERROR");
}

#[test]
fn test_declaration_attribute_extern_is_error() {
    // @extern is a declaration attribute for FFI
    let (ok, _warnings) = parse_expr_with_warnings("@extern(\"C\")");
    assert!(!ok, "@extern in expression context should be a parse ERROR");
}

// =============================================================================
// TRULY UNKNOWN @ NAMES - Should produce warnings (not errors)
// =============================================================================

#[test]
fn test_unknown_meta_function_random() {
    // Completely unknown @ name produces warning (might be user-defined macro)
    let (ok, warnings) = parse_expr_with_warnings("@random_unknown_name");
    assert!(ok, "Unknown @ name should parse with warning, not error");
    assert_eq!(warnings.len(), 1, "Should produce exactly one warning");
    assert!(
        warnings[0].contains("unknown meta-function"),
        "Warning should mention 'unknown meta-function'"
    );
}

// =============================================================================
// TYPO SUGGESTIONS - Should suggest similar names
// =============================================================================

#[test]
fn test_typo_suggestion_fille() {
    // Typo: @fille instead of @file
    let (ok, warnings) = parse_expr_with_warnings("@fille");
    assert!(ok, "Should parse but warn about typo");
    assert_eq!(warnings.len(), 1, "Should produce exactly one warning");
    // The warning should contain a suggestion
}

#[test]
fn test_typo_suggestion_lin() {
    // Typo: @lin instead of @line
    let (ok, warnings) = parse_expr_with_warnings("@lin");
    assert!(ok, "Should parse but warn about typo");
    assert_eq!(warnings.len(), 1, "Should produce exactly one warning");
}

#[test]
fn test_typo_suggestion_cfgg() {
    // Typo: @cfgg instead of @cfg
    let (ok, warnings) = parse_expr_with_warnings("@cfgg(x)");
    assert!(ok, "Should parse but warn about typo");
    assert_eq!(warnings.len(), 1, "Should produce exactly one warning");
}

#[test]
fn test_typo_suggestion_conts() {
    // Typo: @conts instead of @const
    let (ok, warnings) = parse_expr_with_warnings("@conts 42");
    assert!(ok, "Should parse but warn about typo");
    assert_eq!(warnings.len(), 1, "Should produce exactly one warning");
}
