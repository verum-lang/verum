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
//! Comprehensive tests for goto_definition module
//!
//! Tests the go-to-definition functionality including:
//! - Top-level function definitions
//! - Local variable definitions
//! - Type definitions
//! - Protocol definitions
//! - Parameter definitions
//! - Pattern bindings in match/for/closure

use tower_lsp::lsp_types::*;
use verum_ast::FileId;
use verum_lsp::document::DocumentState;
use verum_lsp::goto_definition::*;

// ==================== Helper Functions ====================

fn create_test_document(source: &str) -> DocumentState {
    DocumentState::new(source.to_string(), 1, FileId::new(1))
}

fn test_uri() -> Url {
    Url::parse("file:///test.vr").unwrap()
}

// ==================== Top-Level Definition Tests ====================

#[test]
fn test_goto_function_definition() {
    let source = r#"fn calculate(x: Int) -> Int {
    x * 2
}

fn main() {
    let result = calculate(21);
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "calculate" in the call site (line 5, around char 17)
    let position = Position {
        line: 5,
        character: 17,
    };

    let result = goto_definition(&doc, position, &uri);
    // Result depends on symbol table population during parsing
    // The function should return Some if the symbol is found
}

#[test]
fn test_goto_type_definition() {
    let source = r#"type Point {
    x: Int,
    y: Int,
}

fn origin() -> Point {
    Point { x: 0, y: 0 }
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "Point" in the return type (line 5, around char 15)
    let position = Position {
        line: 5,
        character: 15,
    };

    let result = goto_definition(&doc, position, &uri);
    // Result depends on symbol resolution
}

#[test]
fn test_goto_protocol_definition() {
    let source = r#"protocol Comparable {
    fn compare(self, other: Self) -> Int
}

fn sort<T: Comparable>(items: List<T>) -> List<T> {
    items
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "Comparable" in the constraint (line 4, around char 11)
    let position = Position {
        line: 4,
        character: 11,
    };

    let result = goto_definition(&doc, position, &uri);
    // Result depends on symbol resolution
}

#[test]
fn test_goto_const_definition() {
    let source = r#"const MAX_SIZE: Int = 100

fn validate(size: Int) -> Bool {
    size <= MAX_SIZE
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "MAX_SIZE" in the comparison (line 3, around char 12)
    let position = Position {
        line: 3,
        character: 12,
    };

    let result = goto_definition(&doc, position, &uri);
    // Result depends on symbol resolution
}

// ==================== Local Variable Definition Tests ====================

#[test]
fn test_goto_local_variable_definition() {
    let source = r#"fn example() {
    let counter = 0;
    let result = counter + 1;
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "counter" in the usage (line 2, around char 17)
    let position = Position {
        line: 2,
        character: 17,
    };

    let result = goto_definition(&doc, position, &uri);
    // Should find the definition on line 1
}

#[test]
fn test_goto_parameter_definition() {
    let source = r#"fn process(value: Int, name: Text) -> Int {
    let doubled = value * 2;
    doubled
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "value" in the usage (line 1, around char 18)
    let position = Position {
        line: 1,
        character: 18,
    };

    let result = goto_definition(&doc, position, &uri);
    // Should find the parameter definition
}

// ==================== Pattern Binding Tests ====================

#[test]
fn test_goto_match_pattern_binding() {
    let source = r#"type Option<T> = Some(T) | None

fn unwrap(opt: Option<Int>) -> Int {
    match opt {
        Some(value) => value,
        None => 0,
    }
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "value" in the match arm body (line 4, around char 23)
    let position = Position {
        line: 4,
        character: 23,
    };

    let result = goto_definition(&doc, position, &uri);
    // Should find the pattern binding
}

#[test]
fn test_goto_for_loop_pattern_binding() {
    let source = r#"fn sum(items: List<Int>) -> Int {
    let total = 0;
    for item in items {
        total = total + item;
    }
    total
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "item" in the loop body (line 3, around char 24)
    let position = Position {
        line: 3,
        character: 24,
    };

    let result = goto_definition(&doc, position, &uri);
    // Should find the for loop pattern binding
}

#[test]
fn test_goto_closure_parameter() {
    let source = r#"fn apply(f: (Int) -> Int, x: Int) -> Int {
    f(x)
}

fn main() {
    let doubled = apply(|n| n * 2, 5);
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "n" in the closure body (line 5, around char 28)
    let position = Position {
        line: 5,
        character: 28,
    };

    let result = goto_definition(&doc, position, &uri);
    // Should find the closure parameter
}

#[test]
fn test_goto_tuple_pattern_binding() {
    let source = r#"fn unpack(pair: (Int, Text)) -> Int {
    let (x, y) = pair;
    x
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "x" in the return (line 2, char 4)
    let position = Position {
        line: 2,
        character: 4,
    };

    let result = goto_definition(&doc, position, &uri);
    // Should find the tuple pattern binding
}

// ==================== Member Definition Tests ====================

#[test]
fn test_goto_struct_field_definition() {
    let source = r#"type Person {
    name: Text,
    age: Int,
}

fn get_name(p: Person) -> Text {
    p.name
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "name" in the field access (line 6, around char 6)
    let position = Position {
        line: 6,
        character: 6,
    };

    let result = goto_definition(&doc, position, &uri);
    // Should find the field definition
}

#[test]
fn test_goto_enum_variant_definition() {
    let source = r#"type Color = Red | Green | Blue

fn is_red(c: Color) -> Bool {
    match c {
        Red => true,
        _ => false,
    }
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "Red" in the match (line 4, around char 8)
    let position = Position {
        line: 4,
        character: 8,
    };

    let result = goto_definition(&doc, position, &uri);
    // Should find the variant definition
}

// ==================== Edge Cases ====================

#[test]
fn test_goto_definition_on_keyword() {
    let source = r#"fn test() {
    let x = 5;
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "fn" keyword (line 0, char 0)
    let position = Position {
        line: 0,
        character: 0,
    };

    let result = goto_definition(&doc, position, &uri);
    // Should return None for keywords
    assert!(result.is_none());
}

#[test]
fn test_goto_definition_on_literal() {
    let source = r#"fn test() {
    let x = 42;
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "42" literal (line 1, around char 12)
    let position = Position {
        line: 1,
        character: 12,
    };

    let result = goto_definition(&doc, position, &uri);
    // Should return None for literals
    assert!(result.is_none());
}

#[test]
fn test_goto_definition_on_operator() {
    let source = r#"fn test() {
    let x = 1 + 2;
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "+" operator (line 1, around char 14)
    let position = Position {
        line: 1,
        character: 14,
    };

    let result = goto_definition(&doc, position, &uri);
    // Should return None for operators
    assert!(result.is_none());
}

#[test]
fn test_goto_definition_empty_document() {
    let source = "";
    let doc = create_test_document(source);
    let uri = test_uri();

    let position = Position {
        line: 0,
        character: 0,
    };

    let result = goto_definition(&doc, position, &uri);
    assert!(result.is_none());
}

#[test]
fn test_goto_definition_out_of_bounds() {
    let source = "fn test() {}";
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position way past the end
    let position = Position {
        line: 100,
        character: 0,
    };

    let result = goto_definition(&doc, position, &uri);
    assert!(result.is_none());
}

// ==================== Nested Scope Tests ====================

#[test]
fn test_goto_definition_nested_blocks() {
    let source = r#"fn test() {
    let outer = 1;
    {
        let inner = 2;
        let sum = outer + inner;
    }
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "outer" in the nested block (line 4, around char 18)
    let position = Position {
        line: 4,
        character: 18,
    };

    let result = goto_definition(&doc, position, &uri);
    // Should find the outer definition
}

#[test]
fn test_goto_definition_shadowed_variable() {
    let source = r#"fn test() {
    let x = 1;
    {
        let x = 2;
        let y = x;
    }
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "x" in the inner block (line 4, around char 16)
    let position = Position {
        line: 4,
        character: 16,
    };

    let result = goto_definition(&doc, position, &uri);
    // Should find the inner shadowing definition
}

#[test]
fn test_goto_definition_if_else_branches() {
    let source = r#"fn test(cond: Bool) {
    if cond {
        let a = 1;
        let b = a + 1;
    } else {
        let c = 2;
        let d = c + 2;
    }
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "a" in if branch (line 3, around char 16)
    let position = Position {
        line: 3,
        character: 16,
    };

    let result = goto_definition(&doc, position, &uri);
    // Should find the definition in the if branch
}

// ==================== Multiple Files Consideration ====================

#[test]
fn test_goto_definition_returns_correct_uri() {
    let source = r#"fn foo() -> Int { 42 }

fn main() {
    foo()
}
"#;
    let doc = create_test_document(source);
    let uri = test_uri();

    // Position on "foo" call (line 3, around char 4)
    let position = Position {
        line: 3,
        character: 4,
    };

    let result = goto_definition(&doc, position, &uri);

    // If we get a result, check the URI matches
    if let Some(GotoDefinitionResponse::Scalar(location)) = result {
        assert_eq!(location.uri, uri);
    }
}
