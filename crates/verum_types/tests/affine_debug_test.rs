//! Integration tests for affine type checking through the full pipeline
//!
//! These tests verify that affine type checking works correctly when
//! going through parsing -> type registration -> type checking.

#![allow(dead_code)]

use verum_parser::Parser;
use verum_types::infer::TypeChecker;

#[test]
fn test_affine_pipeline_debug() {
    // Test case 1: Simple affine struct
    let code = r#"
type affine Handle is {
    id: Int,
};

fn main() {
    let h1 = Handle { id: 42 };
    let h2 = h1;           // Move h1
    let x = h1.id;         // Use after move - SHOULD FAIL
}
"#;

    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("Parsing should succeed");

    // Step 1: Verify parser captured affine modifier
    let mut handle_has_affine = false;
    for item in &module.items {
        if let verum_ast::ItemKind::Type(type_decl) = &item.kind
            && type_decl.name.name.as_str() == "Handle" {
                println!("DEBUG: Handle resource_modifier = {:?}", type_decl.resource_modifier);
                handle_has_affine = type_decl.resource_modifier.is_some();
            }
    }
    assert!(handle_has_affine, "Parser should capture affine modifier");

    // Step 2: Run type checker
    let mut checker = TypeChecker::new();

    // Phase 0: Register type declarations
    for item in &module.items {
        if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
            let _ = checker.register_type_declaration(type_decl);
        }
    }

    // Verify Handle is registered as affine
    assert!(checker.is_type_affine_by_name("Handle"),
            "Handle should be registered as affine type");

    // Phase 1: Register function signatures
    for item in &module.items {
        if let verum_ast::ItemKind::Function(func) = &item.kind {
            let _ = checker.register_function_signature(func);
        }
    }

    // Phase 2: Type check
    let mut result = Ok(());
    for item in &module.items {
        if let Err(e) = checker.check_item(item) {
            println!("DEBUG: Type error = {:?}", e);
            result = Err(e);
            break;
        }
    }

    // The check SHOULD fail due to use after move
    assert!(result.is_err(), "Type check should fail due to use-after-move of h1");

    let err = result.unwrap_err();
    let err_msg = format!("{}", err);
    println!("DEBUG: Error message = {}", err_msg);
    assert!(
        err_msg.contains("moved") || err_msg.contains("use after") || err_msg.contains("already"),
        "Expected move-related error, got: {}", err_msg
    );
}

#[test]
fn test_affine_double_field_access() {
    // Field access BORROWS the affine value, doesn't consume it.
    // Multiple field accesses on the same affine value are allowed:
    //   let x = h.id;   // borrow h, extract field
    //   let y = h.id;   // borrow h again, still valid
    // Only full value transfer consumes: let h2 = h;
    let code = r#"
type affine Handle is {
    id: Int,
};

fn main() {
    let h = Handle { id: 42 };
    let x = h.id;   // First field access - borrows h
    let y = h.id;   // Second field access - h still valid (just borrowed)
}
"#;

    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("Parsing should succeed");

    let mut checker = TypeChecker::new();

    // Register types
    for item in &module.items {
        if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
            let _ = checker.register_type_declaration(type_decl);
        }
    }

    // Register functions
    for item in &module.items {
        if let verum_ast::ItemKind::Function(func) = &item.kind {
            let _ = checker.register_function_signature(func);
        }
    }

    // Type check
    let mut result = Ok(());
    for item in &module.items {
        if let Err(e) = checker.check_item(item) {
            println!("DEBUG double access: Type error = {:?}", e);
            result = Err(e);
            break;
        }
    }

    // Field access borrows, doesn't consume - so this should pass
    assert!(result.is_ok(), "Double field access on affine type should succeed (field access borrows)");
}
