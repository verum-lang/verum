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
// Integration tests for the Verum LSP server
//
// Tests the main LSP features end-to-end.

use tower_lsp::lsp_types::*;
use verum_ast::FileId;
use verum_lsp::completion;
use verum_lsp::document::DocumentState;
use verum_lsp::hover;

#[test]
fn test_document_state_creation() {
    let source = r#"
        fn factorial(n: Int) -> Int {
            match n {
                0 => 1,
                n => n * factorial(n - 1)
            }
        }
    "#;

    let file_id = FileId::new(1);
    let state = DocumentState::new(source.to_string(), 1, file_id);

    assert_eq!(state.version, 1);
    assert_eq!(state.file_id, file_id);
    assert_eq!(state.text, source);
}

#[test]
fn test_document_state_update() {
    let source = "fn main() {}";
    let file_id = FileId::new(1);
    let mut state = DocumentState::new(source.to_string(), 1, file_id);

    let new_source = "fn main() { let x = 5; }";
    state.update(new_source.to_string(), 2);

    assert_eq!(state.version, 2);
    assert_eq!(state.text, new_source);
}

#[test]
fn test_word_at_position() {
    let source = "fn factorial(n: Int) -> Int";
    let file_id = FileId::new(1);
    let state = DocumentState::new(source.to_string(), 1, file_id);

    // Test getting "factorial"
    let word = state.word_at_position(Position {
        line: 0,
        character: 5,
    });
    assert_eq!(word, Some("factorial".to_string()));

    // Test getting "Int"
    let word = state.word_at_position(Position {
        line: 0,
        character: 17,
    });
    assert_eq!(word, Some("Int".to_string()));
}

#[test]
fn test_completion_keywords() {
    let source = "fn main() {}";
    let file_id = FileId::new(1);
    let state = DocumentState::new(source.to_string(), 1, file_id);

    let completions = completion::complete_at_position(
        &state,
        Position {
            line: 0,
            character: 0,
        },
    );

    assert!(!completions.is_empty());

    // Should contain keyword completions
    let has_fn = completions.iter().any(|c| c.label == "fn");
    let has_let = completions.iter().any(|c| c.label == "let");

    assert!(has_fn, "Should have 'fn' keyword");
    assert!(has_let, "Should have 'let' keyword");
}

#[test]
fn test_completion_types() {
    let source = "let x: ";
    let file_id = FileId::new(1);
    let state = DocumentState::new(source.to_string(), 1, file_id);

    let completions = completion::complete_at_position(
        &state,
        Position {
            line: 0,
            character: 7,
        },
    );

    assert!(!completions.is_empty());

    // Should contain type completions
    let has_int = completions.iter().any(|c| c.label == "Int");
    let has_text = completions.iter().any(|c| c.label == "Text");

    assert!(has_int, "Should have 'Int' type");
    assert!(has_text, "Should have 'Text' type");
}

#[test]
fn test_hover_builtin_type() {
    let source = "let x: Int = 5";
    let file_id = FileId::new(1);
    let state = DocumentState::new(source.to_string(), 1, file_id);

    let hover_info = hover::hover_at_position(
        &state,
        Position {
            line: 0,
            character: 8,
        },
    );

    assert!(hover_info.is_some());
    let info = hover_info.unwrap();

    if let HoverContents::Markup(content) = info.contents {
        assert!(content.value.contains("Integer"));
    } else {
        panic!("Expected Markup hover content");
    }
}

#[test]
fn test_hover_keyword() {
    let source = "fn main() {}";
    let file_id = FileId::new(1);
    let state = DocumentState::new(source.to_string(), 1, file_id);

    let hover_info = hover::hover_at_position(
        &state,
        Position {
            line: 0,
            character: 1,
        },
    );

    assert!(hover_info.is_some());
    let info = hover_info.unwrap();

    if let HoverContents::Markup(content) = info.contents {
        assert!(content.value.contains("fn"));
    } else {
        panic!("Expected Markup hover content");
    }
}

#[test]
fn test_document_parsing_errors() {
    let source = "fn main( { }"; // Invalid syntax
    let file_id = FileId::new(1);
    let state = DocumentState::new(source.to_string(), 1, file_id);

    // Should have diagnostics for parsing errors
    assert!(!state.diagnostics.is_empty());
}

#[test]
fn test_document_get_line() {
    let source = "line 1\nline 2\nline 3";
    let file_id = FileId::new(1);
    let state = DocumentState::new(source.to_string(), 1, file_id);

    assert_eq!(state.get_line(0), Some("line 1"));
    assert_eq!(state.get_line(1), Some("line 2"));
    assert_eq!(state.get_line(2), Some("line 3"));
    assert_eq!(state.get_line(10), None);
}

#[cfg(test)]
mod position_conversion_tests {
    use super::*;

    #[test]
    fn test_position_to_offset() {
        let source = "abc\ndef\nghi";
        let file_id = FileId::new(1);
        let state = DocumentState::new(source.to_string(), 1, file_id);

        // Line 0, char 0 = offset 0
        let offset = state.position_to_offset(Position {
            line: 0,
            character: 0,
        });
        assert_eq!(offset, 0);

        // Line 1, char 0 = offset 4 (after "abc\n")
        let offset = state.position_to_offset(Position {
            line: 1,
            character: 0,
        });
        assert_eq!(offset, 4);

        // Line 2, char 2 = offset 10 (after "abc\ndef\ngh")
        let offset = state.position_to_offset(Position {
            line: 2,
            character: 2,
        });
        assert_eq!(offset, 10);
    }
}
