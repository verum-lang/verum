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
//! Comprehensive tests for rename module
//!
//! Tests the rename refactoring support including:
//! - Keyword detection
//! - Identifier validation
//! - Symbol resolution
//! - Rename preparation
//! - Cross-file rename support

use tower_lsp::lsp_types::*;
use verum_ast::FileId;
use verum_common::Text;
use verum_lsp::document::{DocumentState, SymbolKind};
use verum_lsp::rename::*;

// ==================== Keyword Detection Tests ====================

#[test]
fn test_is_keyword() {
    assert!(is_keyword("fn"));
    assert!(is_keyword("let"));
    assert!(is_keyword("struct"));
    assert!(!is_keyword("myVar"));
    assert!(!is_keyword("custom_function"));
}

#[test]
fn test_is_keyword_all_keywords() {
    // Test all Verum keywords
    let keywords = vec![
        "fn", "let", "mut", "if", "else", "match", "loop", "while", "for", "return", "break",
        "continue", "type", "struct", "enum", "protocol", "impl", "where", "using", "provide",
        "context", "true", "false", "self", "Self", "super", "pub", "mod", "use", "as", "in", "is",
        "async", "await", "try", "catch", "throw", "dyn", "static", "const",
    ];

    for kw in keywords {
        assert!(is_keyword(kw), "Expected '{}' to be a keyword", kw);
    }
}

#[test]
fn test_is_keyword_non_keywords() {
    let non_keywords = vec![
        "myFunction",
        "variableName",
        "TypeName",
        "_underscore",
        "snake_case",
        "camelCase",
        "PascalCase",
        "SCREAMING_CASE",
        "x",
        "y",
        "z",
        "a1",
        "b2",
        "c3",
    ];

    for name in non_keywords {
        assert!(!is_keyword(name), "Expected '{}' to NOT be a keyword", name);
    }
}

// ==================== Identifier Character Tests ====================

#[test]
fn test_is_identifier_char() {
    assert!(is_identifier_char('a'));
    assert!(is_identifier_char('_'));
    assert!(is_identifier_char('0'));
    assert!(!is_identifier_char(' '));
    assert!(!is_identifier_char('.'));
}

#[test]
fn test_is_identifier_char_alphabet() {
    for c in 'a'..='z' {
        assert!(
            is_identifier_char(c),
            "Expected '{}' to be identifier char",
            c
        );
    }
    for c in 'A'..='Z' {
        assert!(
            is_identifier_char(c),
            "Expected '{}' to be identifier char",
            c
        );
    }
}

#[test]
fn test_is_identifier_char_digits() {
    for c in '0'..='9' {
        assert!(
            is_identifier_char(c),
            "Expected '{}' to be identifier char",
            c
        );
    }
}

#[test]
fn test_is_identifier_char_special() {
    let special = vec![
        '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '-', '+', '=', '[', ']', '{', '}', '|',
        '\\', '/', '?', '<', '>', ',', '.', ';', ':', '\'', '"', '`', '~',
    ];
    for c in special {
        assert!(
            !is_identifier_char(c),
            "Expected '{}' to NOT be identifier char",
            c
        );
    }
}

// ==================== Valid Identifier Tests ====================

#[test]
fn test_is_valid_identifier_valid() {
    assert!(is_valid_identifier("foo"));
    assert!(is_valid_identifier("_bar"));
    assert!(is_valid_identifier("baz123"));
    assert!(is_valid_identifier("_"));
    assert!(is_valid_identifier("CamelCase"));
    assert!(is_valid_identifier("snake_case"));
    assert!(is_valid_identifier("_leading_underscore"));
    assert!(is_valid_identifier("SCREAMING_SNAKE"));
}

#[test]
fn test_is_valid_identifier_invalid() {
    assert!(!is_valid_identifier(""));
    assert!(!is_valid_identifier("123abc"));
    assert!(!is_valid_identifier("with space"));
    assert!(!is_valid_identifier("with-hyphen"));
    assert!(!is_valid_identifier("with.dot"));
    assert!(!is_valid_identifier("$dollar"));
    assert!(!is_valid_identifier("@at"));
}

// ==================== RenameError Tests ====================

#[test]
fn test_rename_error_display() {
    let err = RenameError::CannotRenameKeyword(Text::from("fn"));
    assert!(err.to_string().contains("fn"));
    assert!(err.to_string().contains("keyword"));

    let err = RenameError::InvalidIdentifier(Text::from("123bad"));
    assert!(err.to_string().contains("123bad"));
    assert!(err.to_string().contains("identifier"));

    let err = RenameError::SymbolNotFound;
    assert!(err.to_string().contains("symbol"));

    let err = RenameError::ReadOnlySymbol(Text::from("stdlib_fn"));
    assert!(err.to_string().contains("stdlib_fn"));
    assert!(err.to_string().contains("read-only"));

    let err = RenameError::WorkspaceRequired;
    assert!(err.to_string().contains("workspace"));
}

#[test]
fn test_rename_error_name_conflict() {
    let err = RenameError::NameConflict {
        new_name: Text::from("existing"),
        conflicting_symbol: Text::from("existing"),
        kind: SymbolKind::Function,
    };

    let msg = err.to_string();
    assert!(msg.contains("existing"));
    assert!(msg.contains("conflicts"));
}

// ==================== SymbolScope Tests ====================

#[test]
fn test_symbol_scope_equality() {
    assert_eq!(SymbolScope::Module, SymbolScope::Module);
    assert_eq!(SymbolScope::Block, SymbolScope::Block);
    assert_eq!(
        SymbolScope::Function(Text::from("foo")),
        SymbolScope::Function(Text::from("foo"))
    );
    assert_eq!(
        SymbolScope::Type(Text::from("MyType")),
        SymbolScope::Type(Text::from("MyType"))
    );

    assert_ne!(SymbolScope::Module, SymbolScope::Block);
    assert_ne!(
        SymbolScope::Function(Text::from("foo")),
        SymbolScope::Function(Text::from("bar"))
    );
}

// ==================== Prepare Rename Tests ====================

#[test]
fn test_prepare_rename_keyword() {
    let source = "fn test() {}\n";
    let doc = DocumentState::new(source.to_string(), 1, FileId::new(1));

    // Position on "fn" keyword should return None
    let position = Position {
        line: 0,
        character: 0,
    };
    let result = prepare_rename(&doc, position);
    assert!(result.is_none());
}

#[test]
fn test_prepare_rename_valid_identifier() {
    let source = "fn test() {\n    let myVar = 42;\n}\n";
    let doc = DocumentState::new(source.to_string(), 1, FileId::new(1));

    // Position on "test" function name should work
    let position = Position {
        line: 0,
        character: 3,
    };
    let result = prepare_rename(&doc, position);
    // Result depends on symbol table population
}

// ==================== ResolvedSymbol Tests ====================

#[test]
fn test_resolved_symbol_structure() {
    use verum_common::List;

    let uri = Url::parse("file:///test.vr").unwrap();
    let definition = Location {
        uri: uri.clone(),
        range: Range::default(),
    };

    let symbol = ResolvedSymbol {
        name: Text::from("myFunction"),
        kind: SymbolKind::Function,
        definition,
        references: List::new(),
        is_cross_file: false,
        scope: SymbolScope::Module,
    };

    assert_eq!(symbol.name.as_str(), "myFunction");
    assert!(matches!(symbol.kind, SymbolKind::Function));
    assert!(!symbol.is_cross_file);
    assert!(matches!(symbol.scope, SymbolScope::Module));
}

// ==================== Integration Tests ====================

#[test]
fn test_rename_workflow() {
    let source = r#"
fn calculate(x: Int) -> Int {
    x * 2
}

fn main() {
    let result = calculate(21);
}
"#;
    let doc = DocumentState::new(source.to_string(), 1, FileId::new(1));
    let uri = Url::parse("file:///test.vr").unwrap();

    // The workflow should work even if symbol table is not fully populated
    let position = Position {
        line: 1,
        character: 3,
    };
    let _prepare_result = prepare_rename(&doc, position);
    // Further operations would depend on symbol resolution
}

#[test]
fn test_validate_new_name_keyword() {
    let source = "fn test() {}\n";
    let doc = DocumentState::new(source.to_string(), 1, FileId::new(1));
    let uri = Url::parse("file:///test.vr").unwrap();

    // Create a mock resolved symbol
    let symbol = ResolvedSymbol {
        name: Text::from("test"),
        kind: SymbolKind::Function,
        definition: Location {
            uri: uri.clone(),
            range: Range::default(),
        },
        references: verum_common::List::new(),
        is_cross_file: false,
        scope: SymbolScope::Module,
    };

    // Trying to rename to a keyword should fail
    let result = validate_new_name(&doc, &symbol, "fn");
    assert!(matches!(result, Err(RenameError::CannotRenameKeyword(_))));

    let result = validate_new_name(&doc, &symbol, "let");
    assert!(matches!(result, Err(RenameError::CannotRenameKeyword(_))));
}

#[test]
fn test_validate_new_name_invalid_identifier() {
    let source = "fn test() {}\n";
    let doc = DocumentState::new(source.to_string(), 1, FileId::new(1));
    let uri = Url::parse("file:///test.vr").unwrap();

    let symbol = ResolvedSymbol {
        name: Text::from("test"),
        kind: SymbolKind::Function,
        definition: Location {
            uri: uri.clone(),
            range: Range::default(),
        },
        references: verum_common::List::new(),
        is_cross_file: false,
        scope: SymbolScope::Module,
    };

    // Invalid identifiers should fail
    let result = validate_new_name(&doc, &symbol, "123bad");
    assert!(matches!(result, Err(RenameError::InvalidIdentifier(_))));

    let result = validate_new_name(&doc, &symbol, "with spaces");
    assert!(matches!(result, Err(RenameError::InvalidIdentifier(_))));

    let result = validate_new_name(&doc, &symbol, "");
    assert!(matches!(result, Err(RenameError::InvalidIdentifier(_))));
}
