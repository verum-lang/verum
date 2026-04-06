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
//! Full compilation pipeline integration tests
//!
//! Tests the complete flow from source code through parsing and AST construction.
//! Each test verifies that valid Verum source successfully parses into an AST.

use verum_ast::FileId;
use verum_parser::VerumParser;

/// Helper: parse source and assert it succeeds
fn parse_ok(source: &str) -> verum_ast::Module {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_module_str(source, file_id);
    match result {
        Ok(module) => module,
        Err(e) => panic!("Parse failed for source:\n{}\nError: {:?}", source, e),
    }
}

// ============================================================================
// BASIC PIPELINE TESTS
// ============================================================================

#[test]
fn test_hello_world_pipeline() {
    let source = r#"
        fn main() {
            print("Hello, World!");
        }
    "#;

    let module = parse_ok(source);
    assert!(!module.items.is_empty(), "Should have at least one item");
}

#[test]
fn test_variable_declaration_pipeline() {
    let source = r#"
        fn main() {
            let x: Int = 42;
            let y = x + 1;
            print(y);
        }
    "#;

    let module = parse_ok(source);
    assert!(!module.items.is_empty(), "Should parse variable declarations");
}

#[test]
fn test_function_call_pipeline() {
    let source = r#"
        fn add(a: Int, b: Int) -> Int {
            a + b
        }

        fn main() {
            let result = add(40, 2);
            print(result);
        }
    "#;

    let module = parse_ok(source);
    assert!(
        module.items.len() >= 2,
        "Should have at least 2 function items"
    );
}
