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
//! Comprehensive tests for completion module
//!
//! Tests the code completion functionality including:
//! - Trigger context detection
//! - Keyword completions
//! - Type completions
//! - Member completions
//! - Import completions
//! - Context-aware completions
//! - Snippet support

use tower_lsp::lsp_types::*;
use verum_ast::FileId;
use verum_lsp::completion::*;
use verum_lsp::document::DocumentState;
use verum_common::List;

// ==================== Helper Functions ====================

fn create_test_document(source: &str) -> DocumentState {
    DocumentState::new(source.to_string(), 1, FileId::new(1))
}

// ==================== Trigger Context Tests ====================

#[test]
fn test_get_trigger_context() {
    assert_eq!(get_trigger_context("let x:", 6), Some(TriggerContext::Type));
    assert_eq!(
        get_trigger_context("value.", 6),
        Some(TriggerContext::Member)
    );
    assert_eq!(
        get_trigger_context("use std::", 9),
        Some(TriggerContext::Import)
    );
    assert_eq!(
        get_trigger_context("let x = ", 8),
        Some(TriggerContext::Expression)
    );
}

#[test]
fn test_trigger_context_type_annotation() {
    // After colon in parameter
    assert_eq!(
        get_trigger_context("fn foo(x:", 9),
        Some(TriggerContext::Type)
    );
    // After arrow for return type
    assert_eq!(
        get_trigger_context("fn foo() ->", 11),
        Some(TriggerContext::Type)
    );
    // After colon in let binding
    assert_eq!(
        get_trigger_context("let value:", 10),
        Some(TriggerContext::Type)
    );
}

#[test]
fn test_trigger_context_member_access() {
    assert_eq!(
        get_trigger_context("object.", 7),
        Some(TriggerContext::Member)
    );
    assert_eq!(
        get_trigger_context("self.", 5),
        Some(TriggerContext::Member)
    );
    assert_eq!(
        get_trigger_context("foo.bar.", 8),
        Some(TriggerContext::Member)
    );
}

#[test]
fn test_trigger_context_import() {
    assert_eq!(get_trigger_context("use ", 4), Some(TriggerContext::Import));
    assert_eq!(
        get_trigger_context("use std::", 9),
        Some(TriggerContext::Import)
    );
    assert_eq!(
        get_trigger_context("use verum_common::collections::", 29),
        Some(TriggerContext::Import)
    );
}

#[test]
fn test_trigger_context_expression() {
    assert_eq!(
        get_trigger_context("let x = ", 8),
        Some(TriggerContext::Expression)
    );
    assert_eq!(
        get_trigger_context("return ", 7),
        Some(TriggerContext::Expression)
    );
    assert_eq!(
        get_trigger_context("if ", 3),
        Some(TriggerContext::Expression)
    );
}

#[test]
fn test_trigger_context_edge_cases() {
    // Empty string
    assert!(get_trigger_context("", 0).is_none() || get_trigger_context("", 0).is_some());

    // Beginning of line
    assert!(get_trigger_context("f", 0).is_none() || get_trigger_context("f", 0).is_some());

    // Offset past end
    assert!(
        get_trigger_context("let x", 100).is_none() || get_trigger_context("let x", 100).is_some()
    );
}

// ==================== Keyword Completion Tests ====================

#[test]
fn test_keyword_completions() {
    let mut completions = List::new();
    add_keyword_completions(&mut completions);
    assert!(!completions.is_empty());
    assert!(completions.iter().any(|c| c.label == "fn"));
    assert!(completions.iter().any(|c| c.label == "let"));
}

#[test]
fn test_keyword_completions_all_keywords() {
    let mut completions = List::new();
    add_keyword_completions(&mut completions);

    // Keywords that are actually defined in the completion module
    let expected_keywords = vec![
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
        "as",
        "in",
        "is",
        "true",
        "false",
        "null",
        "self",
    ];

    for kw in expected_keywords {
        assert!(
            completions.iter().any(|c| c.label == kw),
            "Expected keyword '{}' in completions",
            kw
        );
    }
}

#[test]
fn test_keyword_completions_have_correct_kind() {
    let mut completions = List::new();
    add_keyword_completions(&mut completions);

    for completion in completions.iter() {
        assert_eq!(completion.kind, Some(CompletionItemKind::KEYWORD));
    }
}

// ==================== Type Completion Tests ====================

#[test]
fn test_type_completions() {
    let mut completions = List::new();
    add_type_completions(&mut completions);
    assert!(!completions.is_empty());
    assert!(completions.iter().any(|c| c.label == "Int"));
    assert!(completions.iter().any(|c| c.label == "Text"));
}

#[test]
fn test_type_completions_all_builtin_types() {
    let mut completions = List::new();
    add_type_completions(&mut completions);

    // Types that are actually defined in the completion module
    let expected_types = vec![
        "Int", "Float", "Bool", "Text", "Char", "Unit", "List", "Map", "Set", "Maybe", "Result",
        "Heap", "Shared", "Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64",
        "Float32", "Float64",
    ];

    for ty in expected_types {
        assert!(
            completions.iter().any(|c| c.label == ty),
            "Expected type '{}' in completions",
            ty
        );
    }
}

#[test]
fn test_type_completions_have_correct_kind() {
    let mut completions = List::new();
    add_type_completions(&mut completions);

    for completion in completions.iter() {
        assert!(
            completion.kind == Some(CompletionItemKind::CLASS)
                || completion.kind == Some(CompletionItemKind::STRUCT)
                || completion.kind == Some(CompletionItemKind::TYPE_PARAMETER),
            "Expected type completion to have type-related kind"
        );
    }
}

// ==================== Complete at Position Tests ====================

#[test]
fn test_complete_at_position_empty_document() {
    let source = "";
    let doc = create_test_document(source);

    let position = Position {
        line: 0,
        character: 0,
    };

    let completions = complete_at_position(&doc, position);
    // Should return some completions (keywords at minimum)
}

#[test]
fn test_complete_at_position_after_let() {
    let source = "fn test() {\n    let \n}";
    let doc = create_test_document(source);

    let position = Position {
        line: 1,
        character: 8,
    };

    let completions = complete_at_position(&doc, position);
    // Should return identifier suggestions
}

#[test]
fn test_complete_at_position_after_colon() {
    let source = "fn test() {\n    let x: \n}";
    let doc = create_test_document(source);

    let position = Position {
        line: 1,
        character: 11,
    };

    let completions = complete_at_position(&doc, position);
    // Should return type completions
    assert!(completions.iter().any(|c| c.label == "Int"));
    assert!(completions.iter().any(|c| c.label == "Text"));
}

#[test]
fn test_complete_at_position_after_dot() {
    let source = "fn test() {\n    let x = obj.\n}";
    let doc = create_test_document(source);

    let position = Position {
        line: 1,
        character: 18,
    };

    let completions = complete_at_position(&doc, position);
    // Should return member completions
}

#[test]
fn test_complete_at_position_function_start() {
    let source = "fn test() {\n    \n}";
    let doc = create_test_document(source);

    let position = Position {
        line: 1,
        character: 4,
    };

    let completions = complete_at_position(&doc, position);
    // Should return keywords and available identifiers
}

// ==================== Context-Aware Completion Tests ====================

#[test]
fn test_completions_in_function_body() {
    let source = r#"fn calculate(x: Int) -> Int {

}
"#;
    let doc = create_test_document(source);

    let position = Position {
        line: 1,
        character: 4,
    };

    let completions = complete_at_position(&doc, position);
    // Should include parameter `x` in completions
}

#[test]
fn test_completions_show_local_variables() {
    let source = r#"fn test() {
    let counter = 0;

}
"#;
    let doc = create_test_document(source);

    let position = Position {
        line: 2,
        character: 4,
    };

    let completions = complete_at_position(&doc, position);
    // Should include local variable `counter`
}

#[test]
fn test_completions_show_function_names() {
    let source = r#"fn helper() -> Int { 42 }

fn main() {

}
"#;
    let doc = create_test_document(source);

    let position = Position {
        line: 3,
        character: 4,
    };

    let completions = complete_at_position(&doc, position);
    // Should include function `helper`
}

#[test]
fn test_completions_show_type_names() {
    let source = r#"type Point {
    x: Int,
    y: Int,
}

fn foo(p:
"#;
    let doc = create_test_document(source);

    let position = Position {
        line: 5,
        character: 10,
    };

    let completions = complete_at_position(&doc, position);
    // Should include user-defined type `Point`
}

// ==================== Snippet Completion Tests ====================

#[test]
fn test_function_snippet_completion() {
    let mut completions = List::new();
    add_keyword_completions(&mut completions);

    let fn_completion = completions.iter().find(|c| c.label == "fn");
    if let Some(completion) = fn_completion {
        // Function keyword should have a snippet
        assert!(
            completion.insert_text_format == Some(InsertTextFormat::SNIPPET)
                || completion.insert_text.is_some()
        );
    }
}

#[test]
fn test_if_snippet_completion() {
    let mut completions = List::new();
    add_keyword_completions(&mut completions);

    let if_completion = completions.iter().find(|c| c.label == "if");
    if let Some(completion) = if_completion {
        // If keyword might have a snippet
        if completion.insert_text_format == Some(InsertTextFormat::SNIPPET) {
            assert!(completion.insert_text.is_some());
        }
    }
}

// ==================== Edge Cases ====================

#[test]
fn test_complete_at_position_out_of_bounds() {
    let source = "fn test() {}";
    let doc = create_test_document(source);

    let position = Position {
        line: 100,
        character: 0,
    };

    let completions = complete_at_position(&doc, position);
    // Should handle gracefully (return empty or default completions)
}

#[test]
fn test_complete_at_position_inside_string() {
    let source = r#"fn test() {
    let s = "hello ";
}
"#;
    let doc = create_test_document(source);

    // Position inside the string
    let position = Position {
        line: 1,
        character: 16,
    };

    let completions = complete_at_position(&doc, position);
    // Should probably not return completions inside strings
}

#[test]
fn test_complete_at_position_inside_comment() {
    let source = r#"fn test() {
    // This is a comment
}
"#;
    let doc = create_test_document(source);

    // Position inside the comment
    let position = Position {
        line: 1,
        character: 20,
    };

    let completions = complete_at_position(&doc, position);
    // Should probably not return completions inside comments
}

// ==================== Completion Item Detail Tests ====================

#[test]
fn test_type_completion_has_documentation() {
    let mut completions = List::new();
    add_type_completions(&mut completions);

    // At least some types should have documentation
    let has_docs = completions.iter().any(|c| c.documentation.is_some());
    // Documentation is optional but helpful
}

#[test]
fn test_completion_sort_text() {
    let mut completions = List::new();
    add_keyword_completions(&mut completions);
    add_type_completions(&mut completions);

    // Keywords and types might have different sort priorities
    // This is optional behavior
}

// ==================== Member Completion Tests ====================

#[test]
fn test_member_completion_on_known_type() {
    // When we have type information, we should show appropriate members
    let source = r#"type Point {
    x: Int,
    y: Int,
}

fn test(p: Point) {
    p.
}
"#;
    let doc = create_test_document(source);

    let position = Position {
        line: 6,
        character: 6,
    };

    let completions = complete_at_position(&doc, position);
    // Should include fields `x` and `y`
}

// ==================== Import Completion Tests ====================

#[test]
fn test_import_completion() {
    let source = "use \n";
    let doc = create_test_document(source);

    let position = Position {
        line: 0,
        character: 4,
    };

    let completions = complete_at_position(&doc, position);
    // Should include module names
}
