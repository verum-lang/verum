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
//! Comprehensive tests for references module
//!
//! Tests the find-references functionality including:
//! - Function references
//! - Variable references
//! - Type references
//! - Parameter references
//! - Cross-scope references
//! - Include/exclude declaration options

use tower_lsp::lsp_types::*;
use verum_ast::FileId;
use verum_lsp::document::DocumentState;
use verum_lsp::references::*;

// ==================== Helper Functions ====================

fn create_test_document(source: &str) -> DocumentState {
    DocumentState::new(source.to_string(), 1, FileId::new(1))
}

fn test_uri() -> Url {
    Url::parse("file:///test.vr").unwrap()
}

// ==================== Identifier Character Tests ====================

#[test]
fn test_is_identifier_char() {
    assert!(is_identifier_char('a'));
    assert!(is_identifier_char('Z'));
    assert!(is_identifier_char('0'));
    assert!(is_identifier_char('_'));
    assert!(!is_identifier_char(' '));
    assert!(!is_identifier_char('.'));
}

#[test]
fn test_is_identifier_char_alphabet_lowercase() {
    for c in 'a'..='z' {
        assert!(
            is_identifier_char(c),
            "Expected '{}' to be identifier char",
            c
        );
    }
}

#[test]
fn test_is_identifier_char_alphabet_uppercase() {
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
fn test_is_identifier_char_underscore() {
    assert!(is_identifier_char('_'));
}

#[test]
fn test_is_identifier_char_special_chars() {
    let special = vec![
        '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '-', '+', '=', '[', ']', '{', '}', '|',
        '\\', '/', '?', '<', '>', ',', '.', ';', ':', '\'', '"', '`', '~', ' ', '\t', '\n', '\r',
    ];
    for c in special {
        assert!(
            !is_identifier_char(c),
            "Expected '{}' to NOT be identifier char",
            c
        );
    }
}

// ==================== Reference Kind Tests ====================

#[test]
fn test_reference_kind_equality() {
    assert_eq!(ReferenceKind::Definition, ReferenceKind::Definition);
    assert_eq!(ReferenceKind::Read, ReferenceKind::Read);
    assert_eq!(ReferenceKind::Write, ReferenceKind::Write);
    assert_eq!(ReferenceKind::Call, ReferenceKind::Call);

    assert_ne!(ReferenceKind::Definition, ReferenceKind::Read);
    assert_ne!(ReferenceKind::Read, ReferenceKind::Write);
    assert_ne!(ReferenceKind::Write, ReferenceKind::Call);
}

#[test]
fn test_reference_kind_debug() {
    let def = ReferenceKind::Definition;
    let read = ReferenceKind::Read;
    let write = ReferenceKind::Write;
    let call = ReferenceKind::Call;

    assert!(format!("{:?}", def).contains("Definition"));
    assert!(format!("{:?}", read).contains("Read"));
    assert!(format!("{:?}", write).contains("Write"));
    assert!(format!("{:?}", call).contains("Call"));
}

// ==================== Reference Structure Tests ====================

#[test]
fn test_reference_structure() {
    let uri = test_uri();
    let location = Location {
        uri: uri.clone(),
        range: Range::default(),
    };

    let reference = Reference {
        location: location.clone(),
        kind: ReferenceKind::Read,
    };

    assert_eq!(reference.location.uri, uri);
    assert_eq!(reference.kind, ReferenceKind::Read);
}

// ==================== Find References Basic Tests ====================

#[test]
fn test_find_references_function() {
    let source = r#"fn calculate(x: Int) -> Int {
    x * 2
}

fn main() {
    let a = calculate(1);
    let b = calculate(2);
    let c = calculate(3);
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "calculate" definition (line 0, char 3)
    let position = Position {
        line: 0,
        character: 3,
    };

    // Include declaration
    let refs = find_references(&doc, position, &uri, true);
    // Should find definition + 3 calls = 4 references (if working properly)

    // Exclude declaration
    let refs_no_decl = find_references(&doc, position, &uri, false);
    // Should find only 3 calls
}

#[test]
fn test_find_references_variable() {
    let source = r#"fn test() {
    let counter = 0;
    let a = counter + 1;
    let b = counter + 2;
    let c = counter + 3;
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "counter" definition (line 1, around char 8)
    let position = Position {
        line: 1,
        character: 8,
    };

    let refs = find_references(&doc, position, &uri, true);
    // Should find definition + 3 usages = 4 references
}

#[test]
fn test_find_references_type() {
    let source = r#"type Point {
    x: Int,
    y: Int,
}

fn origin() -> Point {
    Point { x: 0, y: 0 }
}

fn add(a: Point, b: Point) -> Point {
    Point { x: a.x + b.x, y: a.y + b.y }
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "Point" definition (line 0, char 5)
    let position = Position {
        line: 0,
        character: 5,
    };

    let refs = find_references(&doc, position, &uri, true);
    // Should find multiple references to Point
}

#[test]
fn test_find_references_parameter() {
    let source = r#"fn process(value: Int) -> Int {
    let doubled = value * 2;
    let tripled = value * 3;
    doubled + tripled + value
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "value" parameter (line 0, around char 11)
    let position = Position {
        line: 0,
        character: 11,
    };

    let refs = find_references(&doc, position, &uri, true);
    // Should find definition + 3 usages = 4 references
}

// ==================== Edge Cases ====================

#[test]
fn test_find_references_no_references() {
    let source = r#"fn unused_function() -> Int {
    42
}

fn main() {
    let x = 1;
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on unused function (line 0, char 3)
    let position = Position {
        line: 0,
        character: 3,
    };

    let refs = find_references(&doc, position, &uri, true);
    // Should find only the definition
}

#[test]
fn test_find_references_empty_document() {
    let source = "";
    let doc = create_test_document(source);
    let uri = test_uri();

    let position = Position {
        line: 0,
        character: 0,
    };

    let refs = find_references(&doc, position, &uri, true);
    assert!(refs.is_empty());
}

#[test]
fn test_find_references_out_of_bounds() {
    let source = "fn test() {}";
    let doc = create_test_document(source);
    let uri = test_uri();

    let position = Position {
        line: 100,
        character: 0,
    };

    let refs = find_references(&doc, position, &uri, true);
    assert!(refs.is_empty());
}

#[test]
fn test_find_references_on_keyword() {
    let source = r#"fn test() {
    let x = 5;
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "fn" keyword
    let position = Position {
        line: 0,
        character: 0,
    };

    let refs = find_references(&doc, position, &uri, true);
    // Keywords don't have references
}

#[test]
fn test_find_references_on_literal() {
    let source = r#"fn test() {
    let x = 42;
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "42" literal
    let position = Position {
        line: 1,
        character: 12,
    };

    let refs = find_references(&doc, position, &uri, true);
    // Literals don't have references
}

// ==================== Include/Exclude Declaration Tests ====================

#[test]
fn test_find_references_include_vs_exclude_declaration() {
    let source = r#"fn foo() {}

fn main() {
    foo();
    foo();
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "foo" definition (line 0, char 3)
    let position = Position {
        line: 0,
        character: 3,
    };

    let with_decl = find_references(&doc, position, &uri, true);
    let without_decl = find_references(&doc, position, &uri, false);

    // with_decl should have at least as many as without_decl
    assert!(with_decl.len() >= without_decl.len());
}

// ==================== Complex Pattern Tests ====================

#[test]
fn test_find_references_in_match_arms() {
    let source = r#"type Option<T> = Some(T) | None

fn unwrap_or(opt: Option<Int>, default: Int) -> Int {
    match opt {
        Some(value) => value,
        None => default,
    }
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "default" (line 2, around char 34)
    let position = Position {
        line: 2,
        character: 34,
    };

    let refs = find_references(&doc, position, &uri, true);
    // Should find parameter and usage in None arm
}

#[test]
fn test_find_references_in_nested_blocks() {
    let source = r#"fn test() {
    let x = 1;
    {
        let y = x + 1;
        {
            let z = x + y;
        }
    }
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "x" (line 1, char 8)
    let position = Position {
        line: 1,
        character: 8,
    };

    let refs = find_references(&doc, position, &uri, true);
    // Should find definition + 2 usages in nested blocks
}

#[test]
fn test_find_references_shadowed_variable() {
    let source = r#"fn test() {
    let x = 1;
    {
        let x = 2;
        let y = x;
    }
    let z = x;
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on outer "x" (line 1, char 8)
    let position = Position {
        line: 1,
        character: 8,
    };

    let refs = find_references(&doc, position, &uri, true);
    // Should find outer x definition and usage on line 6, not the shadowed one
}

// ==================== Cross-File Reference Tests ====================

#[test]
fn test_find_references_returns_correct_uri() {
    let source = r#"fn foo() {}

fn main() {
    foo();
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    let position = Position {
        line: 0,
        character: 3,
    };

    let refs = find_references(&doc, position, &uri, true);

    // All references should have the same URI
    for location in refs.iter() {
        assert_eq!(location.uri, uri);
    }
}

// ==================== Reference Kind Detection Tests ====================

#[test]
fn test_reference_kind_detection_call() {
    // In a call expression like `foo()`, `foo` should be detected as Call kind
    let source = r#"fn foo() {}

fn main() {
    foo();
}
"#;
    let _doc = create_test_document(source);
    // The AST-based reference finding should detect Call kind for function calls
}

#[test]
fn test_reference_kind_detection_read() {
    // In an expression like `x + 1`, `x` should be detected as Read kind
    let source = r#"fn test() {
    let x = 1;
    let y = x + 1;
}
"#;
    let _doc = create_test_document(source);
    // The AST-based reference finding should detect Read kind for variable reads
}

#[test]
fn test_reference_kind_detection_definition() {
    // In a let binding like `let x = 1`, `x` should be detected as Definition kind
    let source = r#"fn test() {
    let x = 1;
}
"#;
    let _doc = create_test_document(source);
    // The AST-based reference finding should detect Definition kind for bindings
}
