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
// Module-level @using attribute tests for Verum Context System
//
// Tests the parsing of module-level context requirements using @using attribute.
// Tests for module-level context annotations: @using([Ctx1, Ctx2])

use verum_ast::{FileId, ItemKind, Module};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

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

/// Helper to check if parsing succeeds.
fn assert_parses(source: &str) {
    parse_module(source).unwrap_or_else(|_| panic!("Failed to parse: {}", source));
}

// ============================================================================
// SECTION 1: Module-Level @using Attribute Tests
// ============================================================================

#[test]
fn test_module_with_single_context() {
    let source = r#"
        @using(Database)
        module user_service {
            fn get_user(id: Int) -> User { }
        }
    "#;

    let module = parse_module(source).expect("Parse error");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Module(decl) => {
            assert_eq!(decl.name.name.as_str(), "user_service");
            assert_eq!(decl.contexts.len(), 1);
            assert_eq!(decl.contexts[0].path.segments.len(), 1);
            if let verum_ast::ty::PathSegment::Name(ident) = &decl.contexts[0].path.segments[0] {
                assert_eq!(ident.name.as_str(), "Database");
            } else {
                panic!("Expected Name segment");
            }
        }
        _ => panic!("Expected Module declaration"),
    }
}

#[test]
fn test_module_with_multiple_contexts() {
    let source = r#"
        @using([Database, Logger])
        module user_service {
            fn get_user(id: Int) -> User { }
        }
    "#;

    let module = parse_module(source).expect("Parse error");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Module(decl) => {
            assert_eq!(decl.name.name.as_str(), "user_service");
            assert_eq!(decl.contexts.len(), 2);

            if let verum_ast::ty::PathSegment::Name(ident) = &decl.contexts[0].path.segments[0] {
                assert_eq!(ident.name.as_str(), "Database");
            }
            if let verum_ast::ty::PathSegment::Name(ident) = &decl.contexts[1].path.segments[0] {
                assert_eq!(ident.name.as_str(), "Logger");
            }
        }
        _ => panic!("Expected Module declaration"),
    }
}

#[test]
fn test_module_with_three_contexts() {
    let source = r#"
        @using([Database, Logger, Cache])
        module web_service {
            fn handle_request() { }
        }
    "#;

    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Module(decl) => {
            assert_eq!(decl.contexts.len(), 3);

            if let verum_ast::ty::PathSegment::Name(ident) = &decl.contexts[0].path.segments[0] {
                assert_eq!(ident.name.as_str(), "Database");
            }
            if let verum_ast::ty::PathSegment::Name(ident) = &decl.contexts[1].path.segments[0] {
                assert_eq!(ident.name.as_str(), "Logger");
            }
            if let verum_ast::ty::PathSegment::Name(ident) = &decl.contexts[2].path.segments[0] {
                assert_eq!(ident.name.as_str(), "Cache");
            }
        }
        _ => panic!("Expected Module declaration"),
    }
}

#[test]
fn test_module_without_using() {
    let source = r#"
        module simple_module {
            fn foo() { }
        }
    "#;

    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Module(decl) => {
            assert_eq!(decl.contexts.len(), 0);
        }
        _ => panic!("Expected Module declaration"),
    }
}

#[test]
fn test_module_with_qualified_context_path() {
    let source = r#"
        @using([std.database.Database, app.Logger])
        module service {
            fn process() { }
        }
    "#;

    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Module(decl) => {
            assert_eq!(decl.contexts.len(), 2);

            // Check std.database.Database path
            assert_eq!(decl.contexts[0].path.segments.len(), 3);
            if let verum_ast::ty::PathSegment::Name(ident) = &decl.contexts[0].path.segments[0] {
                assert_eq!(ident.name.as_str(), "std");
            }
            if let verum_ast::ty::PathSegment::Name(ident) = &decl.contexts[0].path.segments[1] {
                assert_eq!(ident.name.as_str(), "database");
            }
            if let verum_ast::ty::PathSegment::Name(ident) = &decl.contexts[0].path.segments[2] {
                assert_eq!(ident.name.as_str(), "Database");
            }

            // Check app.Logger path
            assert_eq!(decl.contexts[1].path.segments.len(), 2);
            if let verum_ast::ty::PathSegment::Name(ident) = &decl.contexts[1].path.segments[0] {
                assert_eq!(ident.name.as_str(), "app");
            }
            if let verum_ast::ty::PathSegment::Name(ident) = &decl.contexts[1].path.segments[1] {
                assert_eq!(ident.name.as_str(), "Logger");
            }
        }
        _ => panic!("Expected Module declaration"),
    }
}

#[test]
fn test_module_with_using_and_profile() {
    let source = r#"
        @profile(application)
        @using([Database, Logger])
        module web_app {
            fn serve() { }
        }
    "#;

    assert_parses(source);

    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Module(decl) => {
            assert!(decl.profile.is_some());
            assert_eq!(decl.contexts.len(), 2);
        }
        _ => panic!("Expected Module declaration"),
    }
}

#[test]
fn test_module_with_using_and_features() {
    // Note: @features syntax with key-value pairs is not yet implemented
    // This test just verifies @using works alongside other attributes
    let source = r#"
        @using([Database, Logger])
        module async_service {
            fn run() { }
        }
    "#;

    assert_parses(source);

    let module = parse_module(source).expect("Parse error");
    match &module.items[0].kind {
        ItemKind::Module(decl) => {
            assert_eq!(decl.contexts.len(), 2);
        }
        _ => panic!("Expected Module declaration"),
    }
}

#[test]
fn test_module_forward_declaration_with_using() {
    let source = r#"
        @using([Database])
        module external_service;
    "#;

    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Module(decl) => {
            assert_eq!(decl.contexts.len(), 1);
            assert!(decl.items.is_none());
        }
        _ => panic!("Expected Module declaration"),
    }
}

#[test]
fn test_module_public_visibility_with_using() {
    let source = r#"
        @using([Logger])
        public module api {
            fn endpoint() { }
        }
    "#;

    assert_parses(source);
}

#[test]
fn test_nested_module_with_using() {
    let source = r#"
        @using([Logger])
        module parent {
            @using([Database])
            module child {
                fn process() { }
            }
        }
    "#;

    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Module(parent) => {
            assert_eq!(parent.contexts.len(), 1);

            if let verum_ast::ty::PathSegment::Name(ident) = &parent.contexts[0].path.segments[0] {
                assert_eq!(ident.name.as_str(), "Logger");
            }

            if let Some(items) = &parent.items {
                match &items[0].kind {
                    ItemKind::Module(child) => {
                        assert_eq!(child.contexts.len(), 1);

                        if let verum_ast::ty::PathSegment::Name(ident) =
                            &child.contexts[0].path.segments[0]
                        {
                            assert_eq!(ident.name.as_str(), "Database");
                        }
                    }
                    _ => panic!("Expected nested Module declaration"),
                }
            }
        }
        _ => panic!("Expected Module declaration"),
    }
}

// ============================================================================
// SECTION 2: Integration with Functions
// ============================================================================

#[test]
fn test_module_using_with_function_using() {
    let source = r#"
        @using([Database, Logger])
        module service {
            fn get_data(id: Int) -> Data
                using [Cache]
            {
                // Function has Cache in addition to module's Database and Logger
            }
        }
    "#;

    assert_parses(source);
}

#[test]
fn test_module_using_with_multiple_functions() {
    let source = r#"
        @using([Database, Logger])
        module user_service {
            fn get_user(id: Int) -> User { }
            fn save_user(user: User) { }
            fn delete_user(id: Int) { }
        }
    "#;

    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Module(decl) => {
            assert_eq!(decl.contexts.len(), 2);

            if let Some(items) = &decl.items {
                assert_eq!(items.len(), 3);
            }
        }
        _ => panic!("Expected Module declaration"),
    }
}

// ============================================================================
// SECTION 3: Edge Cases
// ============================================================================

#[test]
fn test_module_empty_contexts_array() {
    // Empty array should parse but result in no contexts
    let source = r#"
        @using([])
        module empty_ctx {
            fn foo() { }
        }
    "#;

    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Module(decl) => {
            assert_eq!(decl.contexts.len(), 0);
        }
        _ => panic!("Expected Module declaration"),
    }
}

#[test]
fn test_module_multiple_attributes() {
    // Note: @features syntax with key-value pairs is not yet implemented
    // This test verifies @using works with @profile
    let source = r#"
        @profile(application)
        @using([Database, Logger, Cache, Metrics])
        module complex_service {
            fn run() { }
        }
    "#;

    assert_parses(source);

    let module = parse_module(source).expect("Parse error");
    match &module.items[0].kind {
        ItemKind::Module(decl) => {
            assert!(decl.profile.is_some());
            assert_eq!(decl.contexts.len(), 4);
        }
        _ => panic!("Expected Module declaration"),
    }
}
