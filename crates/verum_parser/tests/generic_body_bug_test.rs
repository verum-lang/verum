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
// Test for generic function body parsing bug
// This test verifies that generic functions retain their body during parsing

use verum_ast::{FileId, ItemKind, Module, decl::FunctionBody};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

/// Helper to parse a module from source.
fn parse_module(source: &str) -> Result<Module, String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join(", ")
    })
}

#[test]
fn test_generic_function_has_body() {
    let source = "fn identity<T>(x: T) -> T { x }";
    let module = parse_module(source).expect("Failed to parse generic function");

    assert_eq!(module.items.len(), 1, "Should have one item");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "identity");
            assert_eq!(func.generics.len(), 1, "Should have one generic parameter");
            assert!(func.body.is_some(), "Function body should not be None");

            // Verify the body is a block
            if let Some(FunctionBody::Block(_)) = &func.body {
                // Success!
            } else {
                panic!("Expected FunctionBody.Block, got {:?}", func.body);
            }
        }
        _ => panic!("Expected a function item"),
    }
}

#[test]
fn test_non_generic_function_has_body() {
    let source = "fn simple(x: Int) -> Int { x }";
    let module = parse_module(source).expect("Failed to parse non-generic function");

    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "simple");
            assert_eq!(func.generics.len(), 0, "Should have no generic parameters");
            assert!(func.body.is_some(), "Function body should not be None");
        }
        _ => panic!("Expected a function item"),
    }
}

#[test]
fn test_multiple_generic_params_has_body() {
    let source = "fn pair<T, U>(x: T, y: U) -> (T, U) { (x, y) }";
    let module = parse_module(source).expect("Failed to parse multi-generic function");

    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "pair");
            assert_eq!(func.generics.len(), 2, "Should have two generic parameters");
            assert!(func.body.is_some(), "Function body should not be None");
        }
        _ => panic!("Expected a function item"),
    }
}
