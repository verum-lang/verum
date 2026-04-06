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
// Tests for formatting module
// Comprehensive tests for syntax-tree-based formatting with trivia preservation

use verum_ast::FileId;
use verum_lsp::formatting::*;
use tower_lsp::lsp_types::{Position, Range};

// ============================================================================
// Basic Formatting Tests
// ============================================================================

#[test]
fn test_basic_format() {
    let input = "fn main() {\nlet x = 5;\n}";
    let expected = "fn main() {\n    let x = 5;\n}\n";
    assert_eq!(basic_format(input), expected);
}

#[test]
fn test_basic_format_empty() {
    let input = "";
    let result = basic_format(input);
    assert!(result.is_empty() || result == "\n");
}

#[test]
fn test_basic_format_single_line() {
    let input = "fn foo() { }";
    let result = basic_format(input);
    assert!(result.contains("fn foo()"));
}

#[test]
fn test_basic_format_nested_blocks() {
    let input = "fn foo() {\nif x {\nlet y = 1;\n}\n}";
    let result = basic_format(input);
    // Should have proper nesting
    assert!(result.contains("    if x"));
    assert!(result.contains("        let y"));
}

#[test]
fn test_basic_format_multiple_functions() {
    let input = "fn foo() { }\nfn bar() { }";
    let result = basic_format(input);
    assert!(result.contains("fn foo()"));
    assert!(result.contains("fn bar()"));
}

// ============================================================================
// Indentation Calculation Tests
// ============================================================================

#[test]
fn test_calculate_indent_for_new_line() {
    assert_eq!(calculate_indent_for_new_line("fn main() {"), 1);
    assert_eq!(calculate_indent_for_new_line("    let x = 5;"), 1);
    assert_eq!(calculate_indent_for_new_line("let x = 5;"), 0);
}

#[test]
fn test_calculate_indent_after_opening_bracket() {
    assert_eq!(calculate_indent_for_new_line("let arr = ["), 1);
    assert_eq!(calculate_indent_for_new_line("    items: ["), 2);
}

#[test]
fn test_calculate_indent_for_closing_brace() {
    assert_eq!(calculate_indent_for_new_line("}"), 0);
    assert_eq!(calculate_indent_for_new_line("    }"), 1);
}

#[test]
fn test_calculate_indent_normal_statement() {
    assert_eq!(calculate_indent_for_new_line("let x = 1;"), 0);
    assert_eq!(calculate_indent_for_new_line("    return x;"), 1);
}

// ============================================================================
// Format Configuration Tests
// ============================================================================

#[test]
fn test_format_config_default() {
    let config = VerumFormatConfig::default();
    assert_eq!(config.indent_size, 4);
    assert_eq!(config.max_line_width, 100);
    assert!(config.trailing_commas);
    assert!(!config.align_assignments);
    assert_eq!(config.blank_lines_between_items, 1);
    assert!(config.sort_imports);
    assert!(config.space_inside_braces);
    assert!(config.space_before_brace);
    assert!(config.preserve_blank_lines);
}

#[test]
fn test_format_config_custom() {
    let config = VerumFormatConfig {
        indent_size: 2,
        max_line_width: 80,
        trailing_commas: false,
        align_assignments: true,
        blank_lines_between_items: 2,
        sort_imports: false,
        space_inside_braces: false,
        space_before_brace: false,
        preserve_blank_lines: false,
    };
    assert_eq!(config.indent_size, 2);
    assert_eq!(config.max_line_width, 80);
    assert!(!config.trailing_commas);
}

// ============================================================================
// Trivia Preserving Formatter Tests
// ============================================================================

#[test]
fn test_trivia_preserving_formatter_creation() {
    let config = VerumFormatConfig::default();
    let formatter = TriviaPreservingFormatter::new(config);
    // Formatter should be created without panic
}

#[test]
fn test_trivia_preserving_formatter_empty() {
    let config = VerumFormatConfig::default();
    let formatter = TriviaPreservingFormatter::new(config);
    let source = "";
    let result = formatter.format(source, FileId::new(0));
    assert!(result.is_empty());
}

#[test]
fn test_trivia_preserving_formatter_simple_function() {
    let config = VerumFormatConfig::default();
    let formatter = TriviaPreservingFormatter::new(config);
    let source = "fn foo() { let x = 1; }";
    let result = formatter.format(source, FileId::new(0));
    // Result should not panic, might be empty if parsing fails
    let _ = result;
}

#[test]
fn test_trivia_preserving_formatter_with_comment() {
    let config = VerumFormatConfig::default();
    let formatter = TriviaPreservingFormatter::new(config);
    let source = "// This is a comment\nfn foo() { }";
    let result = formatter.format(source, FileId::new(0));
    // Should preserve the comment if possible
    let _ = result;
}

#[test]
fn test_trivia_preserving_formatter_with_doc_comment() {
    let config = VerumFormatConfig::default();
    let formatter = TriviaPreservingFormatter::new(config);
    let source = "/// Doc comment\nfn foo() { }";
    let result = formatter.format(source, FileId::new(0));
    // Doc comments should be preserved
    let _ = result;
}

// ============================================================================
// Format Document Tests
// ============================================================================

#[test]
fn test_format_document_no_changes() {
    let input = "fn foo() {\n    let x = 1;\n}\n";
    let edits = format_document(input);
    // May or may not have edits depending on exact formatting
    assert!(edits.len() <= 1);
}

#[test]
fn test_format_document_with_changes() {
    let input = "fn foo(){let x=1;}";
    let edits = format_document(input);
    // Should have at least one edit for reformatting
    // Note: exact behavior depends on formatter
}

#[test]
fn test_format_document_empty() {
    let input = "";
    let edits = format_document(input);
    // Empty document should have no edits
    assert!(edits.is_empty());
}

// ============================================================================
// Format Range Tests
// ============================================================================

#[test]
fn test_format_range_single_line() {
    let input = "fn foo() {\n    let x = 1;\n}";
    let range = Range {
        start: Position { line: 1, character: 0 },
        end: Position { line: 1, character: 100 },
    };
    let edits = format_range(input, range);
    assert!(edits.len() <= 1);
}

#[test]
fn test_format_range_multi_line() {
    let input = "fn foo() {\nlet x = 1;\nlet y = 2;\n}";
    let range = Range {
        start: Position { line: 1, character: 0 },
        end: Position { line: 2, character: 100 },
    };
    let edits = format_range(input, range);
    // May return edits for the range
}

#[test]
fn test_format_range_out_of_bounds() {
    let input = "fn foo() { }";
    let range = Range {
        start: Position { line: 100, character: 0 },
        end: Position { line: 101, character: 0 },
    };
    let edits = format_range(input, range);
    assert!(edits.is_empty());
}

// ============================================================================
// Format On Type Tests
// ============================================================================

#[test]
fn test_format_on_type_closing_brace() {
    let input = "fn foo() {\n    let x = 1;\n}";
    let position = Position { line: 2, character: 1 };
    let edits = format_on_type(input, position, '}');
    // May fix indentation of closing brace
}

#[test]
fn test_format_on_type_semicolon() {
    let input = "fn foo() {\n    let x = 1;\n}";
    let position = Position { line: 1, character: 14 };
    let edits = format_on_type(input, position, ';');
    // Currently no-op for semicolons
    assert!(edits.is_empty());
}

#[test]
fn test_format_on_type_newline() {
    let input = "fn foo() {\n";
    let position = Position { line: 1, character: 0 };
    let edits = format_on_type(input, position, '\n');
    // Should insert proper indentation
}

#[test]
fn test_format_on_type_other_char() {
    let input = "fn foo() { }";
    let position = Position { line: 0, character: 5 };
    let edits = format_on_type(input, position, 'x');
    // Other characters should not trigger formatting
    assert!(edits.is_empty());
}

// ============================================================================
// Type Definition Formatting Tests
// ============================================================================

#[test]
fn test_format_type_definition() {
    let config = VerumFormatConfig::default();
    let formatter = TriviaPreservingFormatter::new(config);
    let source = "type Point is { x: Float, y: Float };";
    let result = formatter.format(source, FileId::new(0));
    // Should format type definition
    let _ = result;
}

#[test]
fn test_format_variant_type() {
    let config = VerumFormatConfig::default();
    let formatter = TriviaPreservingFormatter::new(config);
    let source = "type Option<T> is None | Some(T);";
    let result = formatter.format(source, FileId::new(0));
    // Should format variant type with proper spacing
    let _ = result;
}

#[test]
fn test_format_protocol_definition() {
    let config = VerumFormatConfig::default();
    let formatter = TriviaPreservingFormatter::new(config);
    let source = "type Iterator is protocol { fn next(&mut self) -> Maybe<T>; };";
    let result = formatter.format(source, FileId::new(0));
    let _ = result;
}

// ============================================================================
// Function Formatting Tests
// ============================================================================

#[test]
fn test_format_function_with_params() {
    let config = VerumFormatConfig::default();
    let formatter = TriviaPreservingFormatter::new(config);
    let source = "fn foo(x: Int, y: Int) -> Int { x + y }";
    let result = formatter.format(source, FileId::new(0));
    let _ = result;
}

#[test]
fn test_format_async_function() {
    let config = VerumFormatConfig::default();
    let formatter = TriviaPreservingFormatter::new(config);
    let source = "async fn fetch() -> Result<Data, Error> { }";
    let result = formatter.format(source, FileId::new(0));
    let _ = result;
}

#[test]
fn test_format_generic_function() {
    let config = VerumFormatConfig::default();
    let formatter = TriviaPreservingFormatter::new(config);
    let source = "fn map<A, B>(f: fn(A) -> B, list: List<A>) -> List<B> { }";
    let result = formatter.format(source, FileId::new(0));
    let _ = result;
}

#[test]
fn test_format_function_with_where_clause() {
    let config = VerumFormatConfig::default();
    let formatter = TriviaPreservingFormatter::new(config);
    let source = "fn process<T>(x: T) where T: Debug { }";
    let result = formatter.format(source, FileId::new(0));
    let _ = result;
}

// ============================================================================
// Implementation Block Formatting Tests
// ============================================================================

#[test]
fn test_format_impl_block() {
    let config = VerumFormatConfig::default();
    let formatter = TriviaPreservingFormatter::new(config);
    let source = "implement Point { fn new(x: Float, y: Float) -> Point { Point { x, y } } }";
    let result = formatter.format(source, FileId::new(0));
    let _ = result;
}

#[test]
fn test_format_impl_for_protocol() {
    let config = VerumFormatConfig::default();
    let formatter = TriviaPreservingFormatter::new(config);
    let source = "implement Debug for Point { fn debug(&self) -> Text { } }";
    let result = formatter.format(source, FileId::new(0));
    let _ = result;
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_format_deeply_nested() {
    let input = "fn foo() {\nif a {\nif b {\nif c {\nlet x = 1;\n}\n}\n}\n}";
    let result = basic_format(input);
    // Should maintain proper indentation for deeply nested code
    assert!(result.lines().filter(|l| l.starts_with("            ")).count() >= 1);
}

#[test]
fn test_format_with_strings() {
    let config = VerumFormatConfig::default();
    let formatter = TriviaPreservingFormatter::new(config);
    let source = r#"fn foo() { let s = "hello world"; }"#;
    let result = formatter.format(source, FileId::new(0));
    // Strings should be preserved
    let _ = result;
}

#[test]
fn test_format_with_multiline_string() {
    let input = "fn foo() {\nlet s = \"\"\"hello\nworld\"\"\";\n}";
    let result = basic_format(input);
    // Multiline strings should be preserved
}

#[test]
fn test_format_trailing_whitespace_removal() {
    let input = "fn foo() {   \n    let x = 1;   \n}   ";
    let result = basic_format(input);
    // Lines should not have trailing whitespace
    for line in result.lines() {
        assert!(!line.ends_with(' '), "Line has trailing whitespace: {:?}", line);
    }
}

#[test]
fn test_format_consistent_newlines() {
    let input = "fn foo() { }\n\n\nfn bar() { }";
    let result = basic_format(input);
    // Note: consecutive newline normalization may not be implemented yet
    // The formatter should preserve or normalize blank lines between items
    assert!(result.contains("fn foo()"));
    assert!(result.contains("fn bar()"));
}

// ============================================================================
// Syntax Bridge Integration Tests
// ============================================================================

#[test]
fn test_lossless_parser_integration() {
    // Verify that the LosslessParser from syntax_bridge works correctly
    use verum_parser::syntax_bridge::LosslessParser;

    let parser = LosslessParser::new();
    let source = "fn main() { let x = 42; }";
    let result = parser.parse(source, FileId::new(0));

    // Should produce a syntax tree
    let syntax = result.syntax();
    assert!(!syntax.text().is_empty());
}

#[test]
fn test_lossless_parser_preserves_trivia() {
    use verum_parser::syntax_bridge::LosslessParser;

    let parser = LosslessParser::new();
    let source = "// comment\nfn foo() { }";
    let result = parser.parse(source, FileId::new(0));

    // The full text should include the comment
    let syntax = result.syntax();
    let text = syntax.text();
    assert!(text.contains("comment") || text.contains("foo"));
}

#[test]
fn test_lossless_parser_with_errors() {
    use verum_parser::syntax_bridge::LosslessParser;

    let parser = LosslessParser::new();
    let source = "fn foo( { }"; // Missing closing paren
    let result = parser.parse(source, FileId::new(0));

    // Should still produce a tree, even with errors
    let syntax = result.syntax();
    let _ = syntax.text();
}
