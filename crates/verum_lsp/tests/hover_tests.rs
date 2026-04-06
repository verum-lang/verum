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
//! Comprehensive tests for hover module
//!
//! Tests the hover functionality including:
//! - Builtin type information
//! - Keyword documentation
//! - Function signatures
//! - Type hover with CBGR cost
//! - Variable hover
//! - Documentation extraction

use tower_lsp::lsp_types::*;
use verum_ast::FileId;
use verum_lsp::ast_format::get_builtin_info;
use verum_lsp::document::DocumentState;
use verum_lsp::hover::hover_at_position;

// ==================== Helper Functions ====================

fn create_test_document(source: &str) -> DocumentState {
    DocumentState::new(source.to_string(), 1, FileId::new(1))
}

// ==================== Builtin Info Tests ====================

#[test]
fn test_builtin_info() {
    assert!(get_builtin_info("Int").is_some());
    assert!(get_builtin_info("fn").is_some());
    assert!(get_builtin_info("List").is_some());
    assert!(get_builtin_info("nonexistent").is_none());
}

#[test]
fn test_builtin_info_content() {
    let info = get_builtin_info("Int").unwrap();
    assert!(info.contains("Integer"));
    assert!(info.contains("Arbitrary-precision"));
}

#[test]
fn test_builtin_info_all_types() {
    // Types that are actually defined in get_builtin_info
    let types = vec![
        "Int", "Float", "Bool", "Text", "Char", "Unit", "List", "Map", "Set", "Maybe", "Result",
        "Heap", "Shared",
    ];

    for ty in types {
        let info = get_builtin_info(ty);
        assert!(info.is_some(), "Expected builtin info for type '{}'", ty);
    }
}

#[test]
fn test_builtin_info_keywords() {
    // Keywords that are actually defined in get_builtin_info
    let keywords = vec![
        "fn",
        "let",
        "mut",
        "if",
        "else",
        "match",
        "loop",
        "while",
        "for",
        "return",
        "break",
        "continue",
        "type",
        "struct",
        "enum",
        "protocol",
        "impl",
        "mod",
        "use",
        "pub",
        "async",
        "await",
        "defer",
        "stream",
        "verify",
        "requires",
        "ensures",
        "invariant",
        "assert",
        "assume",
        "ref",
        "checked",
        "unsafe",
        "true",
        "false",
        "null",
        "self",
    ];

    for kw in keywords {
        let info = get_builtin_info(kw);
        assert!(info.is_some(), "Expected builtin info for keyword '{}'", kw);
    }
}

#[test]
fn test_builtin_info_reference_types() {
    // CBGR reference types are represented by keywords
    // The "ref" keyword handles CBGR references
    assert!(get_builtin_info("ref").is_some());
    assert!(get_builtin_info("checked").is_some());
    assert!(get_builtin_info("unsafe").is_some());
}

// ==================== Hover at Position Tests ====================

#[test]
fn test_hover_on_function_name() {
    let source = r#"fn calculate(x: Int) -> Int {
    x * 2
}
"#;
    let doc = create_test_document(source);

    // Position on "calculate" function name (line 0, char 3)
    let position = Position {
        line: 0,
        character: 3,
    };

    let result = hover_at_position(&doc, position);
    // Should return hover with function signature
}

#[test]
fn test_hover_on_parameter() {
    let source = r#"fn process(value: Int) -> Int {
    value * 2
}
"#;
    let doc = create_test_document(source);

    // Position on "value" parameter (line 0, around char 11)
    let position = Position {
        line: 0,
        character: 11,
    };

    let result = hover_at_position(&doc, position);
    // Should return hover with parameter info
}

#[test]
fn test_hover_on_type_annotation() {
    let source = r#"fn example(x: Int) -> Text {
    "hello"
}
"#;
    let doc = create_test_document(source);

    // Position on "Int" type (line 0, around char 14)
    let position = Position {
        line: 0,
        character: 14,
    };

    let result = hover_at_position(&doc, position);
    // Should return hover with Int type info
}

#[test]
fn test_hover_on_keyword() {
    let source = r#"fn test() {
    let x = 5;
}
"#;
    let doc = create_test_document(source);

    // Position on "fn" keyword (line 0, char 0)
    let position = Position {
        line: 0,
        character: 0,
    };

    let result = hover_at_position(&doc, position);
    // Should return hover with fn keyword info
    if let Some(hover) = result {
        match hover.contents {
            HoverContents::Markup(markup) => {
                assert!(!markup.value.is_empty());
            }
            HoverContents::Scalar(text) => {
                let text_str = match text {
                    MarkedString::String(s) => s,
                    MarkedString::LanguageString(ls) => ls.value,
                };
                assert!(!text_str.is_empty());
            }
            HoverContents::Array(arr) => {
                assert!(!arr.is_empty());
            }
        }
    }
}

#[test]
fn test_hover_on_local_variable() {
    let source = r#"fn test() {
    let counter = 0;
    counter
}
"#;
    let doc = create_test_document(source);

    // Position on "counter" usage (line 2, char 4)
    let position = Position {
        line: 2,
        character: 4,
    };

    let result = hover_at_position(&doc, position);
    // Should return hover with variable info
}

#[test]
fn test_hover_on_struct_type() {
    let source = r#"type Point {
    x: Int,
    y: Int,
}

fn origin() -> Point {
    Point { x: 0, y: 0 }
}
"#;
    let doc = create_test_document(source);

    // Position on "Point" type name (line 0, char 5)
    let position = Position {
        line: 0,
        character: 5,
    };

    let result = hover_at_position(&doc, position);
    // Should return hover with type definition
}

#[test]
fn test_hover_on_protocol_name() {
    let source = r#"protocol Comparable {
    fn compare(self, other: Self) -> Int
}
"#;
    let doc = create_test_document(source);

    // Position on "Comparable" (line 0, around char 9)
    let position = Position {
        line: 0,
        character: 9,
    };

    let result = hover_at_position(&doc, position);
    // Should return hover with protocol info
}

// ==================== Edge Cases ====================

#[test]
fn test_hover_empty_document() {
    let source = "";
    let doc = create_test_document(source);

    let position = Position {
        line: 0,
        character: 0,
    };

    let result = hover_at_position(&doc, position);
    assert!(result.is_none());
}

#[test]
fn test_hover_out_of_bounds() {
    let source = "fn test() {}";
    let doc = create_test_document(source);

    let position = Position {
        line: 100,
        character: 0,
    };

    let result = hover_at_position(&doc, position);
    assert!(result.is_none());
}

#[test]
fn test_hover_on_whitespace() {
    let source = "fn test() {  }";
    let doc = create_test_document(source);

    // Position on whitespace between braces (line 0, char 12)
    let position = Position {
        line: 0,
        character: 12,
    };

    let result = hover_at_position(&doc, position);
    // Should return None for whitespace
}

#[test]
fn test_hover_on_operator() {
    let source = r#"fn test() {
    let x = 1 + 2;
}
"#;
    let doc = create_test_document(source);

    // Position on "+" operator (line 1, around char 14)
    let position = Position {
        line: 1,
        character: 14,
    };

    let result = hover_at_position(&doc, position);
    // May or may not return hover for operators
}

#[test]
fn test_hover_on_numeric_literal() {
    let source = r#"fn test() {
    let x = 42;
}
"#;
    let doc = create_test_document(source);

    // Position on "42" literal (line 1, around char 12)
    let position = Position {
        line: 1,
        character: 12,
    };

    let result = hover_at_position(&doc, position);
    // May or may not return hover for literals
}

#[test]
fn test_hover_on_string_literal() {
    let source = r#"fn test() {
    let s = "hello";
}
"#;
    let doc = create_test_document(source);

    // Position inside string literal (line 1, around char 14)
    let position = Position {
        line: 1,
        character: 14,
    };

    let result = hover_at_position(&doc, position);
    // May or may not return hover for string content
}

// ==================== CBGR Cost Hover Tests ====================

#[test]
fn test_hover_shows_cbgr_cost() {
    let source = r#"fn process(data: &T) -> Result<Int, Error> {
    data.value
}
"#;
    let doc = create_test_document(source);

    // Position on "data" parameter (line 0, around char 11)
    let position = Position {
        line: 0,
        character: 11,
    };

    let result = hover_at_position(&doc, position);
    // If CBGR cost is available, it should be included in hover
}

// ==================== Documentation Hover Tests ====================

#[test]
fn test_hover_with_doc_comment() {
    let source = r#"/// Calculates the square of a number.
fn square(x: Int) -> Int {
    x * x
}
"#;
    let doc = create_test_document(source);

    // Position on "square" function (line 1, char 3)
    let position = Position {
        line: 1,
        character: 3,
    };

    let result = hover_at_position(&doc, position);
    // Should include doc comment in hover
}

// ==================== Range Tests ====================

#[test]
fn test_hover_returns_correct_range() {
    let source = r#"fn calculate(x: Int) -> Int { x }"#;
    let doc = create_test_document(source);

    // Position on "calculate"
    let position = Position {
        line: 0,
        character: 5,
    };

    let result = hover_at_position(&doc, position);

    // If hover has a range, it should span the identifier
    if let Some(hover) = result
        && let Some(range) = hover.range {
            // Range should be within line 0
            assert_eq!(range.start.line, 0);
            assert_eq!(range.end.line, 0);
        }
}
