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
// Unit tests for affine type tracking module
//
// Higher-kinded types: type constructors parameterized by type-level functions
//
// Tests moved from src/affine.rs per project testing guidelines.

use verum_ast::span::{FileId, Span};
use verum_common::List;
use verum_types::Type;
use verum_types::affine::AffineTracker;

fn dummy_span() -> Span {
    Span::new(0, 1, FileId::dummy())
}

#[test]
fn test_affine_type_registration() {
    let mut tracker = AffineTracker::new();
    tracker.register_affine_type("FileHandle");
    assert!(tracker.is_affine_type("FileHandle"));
    assert!(!tracker.is_affine_type("RegularType"));
}

#[test]
fn test_affine_value_single_use() {
    let mut tracker = AffineTracker::new();
    tracker.register_affine_type("FileHandle");

    let ty = Type::Named {
        path: verum_ast::ty::Path::single(verum_ast::Ident::new("FileHandle", dummy_span())),
        args: List::new(),
    };

    tracker.bind("handle", ty, dummy_span());

    // First use should succeed
    assert!(tracker.use_value("handle", dummy_span()).is_ok());

    // Second use should fail
    assert!(tracker.use_value("handle", dummy_span()).is_err());
}

#[test]
fn test_affine_value_borrow() {
    let mut tracker = AffineTracker::new();
    tracker.register_affine_type("FileHandle");

    let ty = Type::Named {
        path: verum_ast::ty::Path::single(verum_ast::Ident::new("FileHandle", dummy_span())),
        args: List::new(),
    };

    tracker.bind("handle", ty, dummy_span());

    // Multiple borrows should succeed
    assert!(tracker.borrow_value("handle", dummy_span()).is_ok());
    assert!(tracker.borrow_value("handle", dummy_span()).is_ok());

    // After borrowing, we can still use (move) it
    assert!(tracker.use_value("handle", dummy_span()).is_ok());

    // But can't borrow after consuming
    assert!(tracker.borrow_value("handle", dummy_span()).is_err());
}

#[test]
fn test_branch_merge() {
    let mut tracker = AffineTracker::new();
    tracker.register_affine_type("FileHandle");

    let ty = Type::Named {
        path: verum_ast::ty::Path::single(verum_ast::Ident::new("FileHandle", dummy_span())),
        args: List::new(),
    };

    tracker.bind("handle", ty.clone(), dummy_span());

    // Branch 1: consume the value
    let mut branch1 = tracker.enter_scope();
    branch1.use_value("handle", dummy_span()).unwrap();

    // Branch 2: don't use the value
    let branch2 = tracker.enter_scope();

    // After merge, value should NOT be marked as consumed
    // (only consumed in one branch, not both)
    branch1.merge_branch(&branch2);
    assert!(branch1.is_available("handle")); // merge keeps it available (not consumed in both branches)
}

// Test that verifies the full type checking flow with parsing
// NOTE: Requires RUST_MIN_STACK=16777216 in debug builds due to stack requirements.
// Works correctly in release builds as verified by VCS tests.
#[test]
fn test_affine_full_typecheck() {
    use verum_parser::Parser;
    use verum_types::infer::TypeChecker;

    let code = r#"
type affine Handle is {
    id: Int,
};

fn main() {
    let h1 = Handle { id: 1 };
    let h2 = h1;           // h1 is moved here
    let x = h1.id;         // This should error - use after move
}
"#;

    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("Parsing should succeed");

    let mut checker = TypeChecker::new();

    // Verify affine modifier is parsed correctly
    for item in &module.items {
        if let verum_ast::ItemKind::Type(type_decl) = &item.kind
            && type_decl.name.name.as_str() == "Handle" {
                assert!(
                    type_decl.resource_modifier.is_some(),
                    "Handle should have affine modifier"
                );
                println!("Handle has resource_modifier: {:?}", type_decl.resource_modifier);
            }
    }

    // Phase 0: Register all type declarations FIRST (this registers affine types!)
    for item in &module.items {
        if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
            let _ = checker.register_type_declaration(type_decl);
        }
    }

    // Phase 1: Register function signatures
    for item in &module.items {
        if let verum_ast::ItemKind::Function(func) = &item.kind {
            let _ = checker.register_function_signature(func);
        }
    }

    // Phase 2: Type check each item
    let mut result: Result<(), verum_types::TypeError> = Ok(());
    for item in &module.items {
        if let Err(e) = checker.check_item(item) {
            result = Err(e);
            break;
        }
    }

    println!("Type check result: {:?}", result);

    // The check should fail with a MovedValueUsed error
    match result {
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("moved") || err_msg.contains("use after") || err_msg.contains("already"),
                "Expected move-related error, got: {}", err_msg
            );
        }
        Ok(_) => {
            panic!("Type check should have failed due to use after move");
        }
    }
}

// Diagnostic test to understand type propagation
#[test]
fn test_affine_type_lookup() {
    use verum_types::infer::TypeChecker;

    let mut checker = TypeChecker::new();
    let span = dummy_span();

    // Manually register Handle as affine type
    checker.register_affine_type_for_testing("Handle");

    // Check if it's registered
    assert!(checker.is_type_affine_by_name("Handle"), "Handle should be registered as affine");

    // Create Type::Named for Handle
    let handle_named = Type::Named {
        path: verum_ast::ty::Path::single(verum_ast::Ident::new("Handle", span)),
        args: List::new(),
    };

    // Register the type in context
    checker.register_type_for_testing("Handle", handle_named.clone());

    // Verify it's registered
    let looked_up = checker.lookup_type_for_testing("Handle");
    println!("Looked up Handle type: {:?}", looked_up);
    assert!(looked_up.is_some(), "Handle should be in type context");

    // Check if the looked up type is Named
    match looked_up {
        Some(Type::Named { path, .. }) => {
            let name = path.segments.last()
                .and_then(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(id) => Some(id.name.as_str()),
                    _ => None,
                })
                .unwrap_or("");
            println!("Type is Named with name: {}", name);
            assert_eq!(name, "Handle");
        }
        Some(other) => {
            panic!("Expected Type::Named, got: {:?}", other);
        }
        None => panic!("Type not found"),
    }
}

// Integration test with TypeChecker
#[test]
fn test_affine_type_with_type_checker() {
    use verum_types::infer::TypeChecker;
    use verum_types::context::TypeScheme;
    use indexmap::IndexMap;

    let mut checker = TypeChecker::new();
    let span = dummy_span();

    // Manually register an affine type (simulating what register_type_declaration does)
    checker.register_affine_type_for_testing("Handle");

    // Create and register Handle type as Named type
    let handle_ty = Type::Named {
        path: verum_ast::ty::Path::single(verum_ast::Ident::new("Handle", span)),
        args: List::new(),
    };

    // Register in type context
    checker.register_type_for_testing("Handle", handle_ty.clone());

    // Bind variable h1 with affine type
    let pattern = verum_ast::pattern::Pattern::new(
        verum_ast::pattern::PatternKind::Ident {
            mutable: false,
            by_ref: false,
            name: verum_ast::Ident::new("h1", span),
            subpattern: None,
        },
        span,
    );
    checker.bind_pattern(&pattern, &handle_ty).unwrap();

    // First use should succeed
    let path = verum_ast::ty::Path::single(verum_ast::Ident::new("h1", span));
    let first_use = checker.check_path_for_affine(&path, span);
    assert!(first_use.is_ok(), "First use should succeed");

    // Second use should fail (use after move)
    let second_use = checker.check_path_for_affine(&path, span);
    assert!(second_use.is_err(), "Second use should fail - value already moved");
}
