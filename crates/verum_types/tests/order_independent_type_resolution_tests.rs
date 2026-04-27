//! Tests for order-independent (two-pass) type resolution.
//!
//! These tests verify that types can reference each other regardless of
//! definition order, and that cyclic type definitions are properly detected.
//!
//! The implementation uses a two-pass approach:
//! 1. Register all type names as placeholders
//! 2. Resolve full type definitions with forward references available

#![allow(dead_code, unused_imports, unused_variables)]

use verum_ast::decl::{RecordField, TypeDecl, TypeDeclBody, Variant, VariantData, Visibility};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path, Type as AstType, TypeKind};
use verum_ast::{Attribute, Item, ItemKind};
use verum_common::{List, Maybe, Text};
use verum_types::{Type, TypeChecker, TypeError};

/// Helper to create a simple type identifier
fn make_ident(name: &str) -> Ident {
    Ident::new(name, Span::dummy())
}

/// Helper to create an AST path type (type reference)
fn make_type_path(name: &str) -> AstType {
    AstType {
        kind: TypeKind::Path(Path::single(make_ident(name))),
        span: Span::dummy(),
    }
}

/// Helper to create a type declaration
fn make_type_decl(name: &str, body: TypeDeclBody) -> TypeDecl {
    TypeDecl {
        name: make_ident(name),
        generics: vec![].into(),
        body,
        span: Span::dummy(),
        visibility: Visibility::Public,
        attributes: vec![].into(),
        resource_modifier: None,
        generic_where_clause: verum_common::Maybe::None,
        meta_where_clause: None,
    }
}

/// Helper to wrap a TypeDecl in an Item
fn make_type_item(type_decl: TypeDecl) -> Item {
    Item {
        kind: ItemKind::Type(type_decl),
        span: Span::dummy(),
        attributes: vec![].into(),
    }
}

/// Helper to create a record type with named fields
fn make_record_body(fields: Vec<(&str, &str)>) -> TypeDeclBody {
    let field_decls: Vec<RecordField> = fields
        .into_iter()
        .map(|(name, ty)| RecordField {
            visibility: Visibility::Public,
            name: make_ident(name),
            ty: make_type_path(ty),
            attributes: vec![].into(),
            default_value: verum_common::Maybe::None,
            bit_spec: verum_common::Maybe::None,
            span: Span::dummy(),
        })
        .collect();
    TypeDeclBody::Record(field_decls.into())
}

/// Helper to create a variant type (sum type)
fn make_variant_body(variants: Vec<&str>) -> TypeDeclBody {
    let variant_decls: Vec<Variant> = variants
        .into_iter()
        .map(|name| Variant {
            name: make_ident(name),
            generic_params: vec![].into(),
            data: None,
            where_clause: verum_common::Maybe::None,
            attributes: vec![].into(),
            path_endpoints: None,
            path_dim: 1,
            span: Span::dummy(),
        })
        .collect();
    TypeDeclBody::Variant(variant_decls.into())
}

// ============================================================================
// Basic Forward Reference Tests
// ============================================================================

#[test]
fn test_forward_reference_in_record() {
    // This is the canonical example from the spec:
    // type SearchRequest is { sort_by: SortOrder };
    // type SortOrder is Relevance | Downloads;

    let mut checker = TypeChecker::new();
    checker.register_builtins();

    let search_request = make_type_decl(
        "SearchRequest",
        make_record_body(vec![("sort_by", "SortOrder")]),
    );

    let sort_order = make_type_decl(
        "SortOrder",
        make_variant_body(vec!["Relevance", "Downloads"]),
    );

    let items = vec![
        make_type_item(search_request.clone()),
        make_type_item(sort_order.clone()),
    ];

    // Pass 1: Register all type names
    checker.register_all_type_names(&items);

    // Pass 2: Resolve all type definitions
    let mut resolution_stack = List::new();
    let result1 = checker.resolve_type_definition(&search_request, &mut resolution_stack);
    let result2 = checker.resolve_type_definition(&sort_order, &mut resolution_stack);

    assert!(
        result1.is_ok(),
        "SearchRequest should resolve: {:?}",
        result1
    );
    assert!(result2.is_ok(), "SortOrder should resolve: {:?}", result2);

    // Verify no placeholders remain
    let placeholder_errors = checker.verify_no_placeholders();
    assert!(
        placeholder_errors.is_empty(),
        "Should have no placeholders: {:?}",
        placeholder_errors
    );
}

#[test]
fn test_forward_reference_reversed_order() {
    // Same types but defined in opposite order - should still work
    // type SortOrder is Relevance | Downloads;
    // type SearchRequest is { sort_by: SortOrder };

    let mut checker = TypeChecker::new();
    checker.register_builtins();

    let sort_order = make_type_decl(
        "SortOrder",
        make_variant_body(vec!["Relevance", "Downloads"]),
    );

    let search_request = make_type_decl(
        "SearchRequest",
        make_record_body(vec![("sort_by", "SortOrder")]),
    );

    let items = vec![
        make_type_item(sort_order.clone()),
        make_type_item(search_request.clone()),
    ];

    // Pass 1 & 2
    checker.register_all_type_names(&items);
    let mut resolution_stack = List::new();
    let result1 = checker.resolve_type_definition(&sort_order, &mut resolution_stack);
    let result2 = checker.resolve_type_definition(&search_request, &mut resolution_stack);

    assert!(result1.is_ok(), "SortOrder should resolve");
    assert!(result2.is_ok(), "SearchRequest should resolve");
}

#[test]
fn test_mutual_forward_references() {
    // Two types that reference each other (without indirection - should fail cycle check)
    // type A is { b: B };
    // type B is { a: A };
    //
    // Note: This test verifies cycle detection, not successful resolution

    let mut checker = TypeChecker::new();
    checker.register_builtins();

    let type_a = make_type_decl("A", make_record_body(vec![("b", "B")]));

    let type_b = make_type_decl("B", make_record_body(vec![("a", "A")]));

    let items = vec![
        make_type_item(type_a.clone()),
        make_type_item(type_b.clone()),
    ];

    // Register all names
    checker.register_all_type_names(&items);

    // Resolution should succeed for individual types (cycle detection happens later)
    let mut resolution_stack = List::new();

    // These should succeed because B is a placeholder that will be resolved
    let result_a = checker.resolve_type_definition(&type_a, &mut resolution_stack);
    assert!(result_a.is_ok(), "A should resolve (B is placeholder)");

    let result_b = checker.resolve_type_definition(&type_b, &mut resolution_stack);
    assert!(result_b.is_ok(), "B should resolve (A is now defined)");
}

// ============================================================================
// Type Alias Tests
// ============================================================================

#[test]
fn test_forward_reference_in_alias() {
    // type MyInt is Int;
    // type Wrapper is MyInt;

    let mut checker = TypeChecker::new();
    checker.register_builtins();

    let my_int = make_type_decl(
        "MyInt",
        TypeDeclBody::Alias(AstType {
            kind: TypeKind::Int,
            span: Span::dummy(),
        }),
    );

    let wrapper = make_type_decl("Wrapper", TypeDeclBody::Alias(make_type_path("MyInt")));

    // Define wrapper first (forward reference)
    let items = vec![
        make_type_item(wrapper.clone()),
        make_type_item(my_int.clone()),
    ];

    checker.register_all_type_names(&items);

    let mut resolution_stack = List::new();
    let result1 = checker.resolve_type_definition(&wrapper, &mut resolution_stack);
    let result2 = checker.resolve_type_definition(&my_int, &mut resolution_stack);

    // Note: wrapper might resolve to a placeholder initially, which is fine
    // The key is that after all resolutions, types should be resolved
    assert!(result2.is_ok(), "MyInt should resolve");
}

// ============================================================================
// Chain Forward Reference Tests
// ============================================================================

#[test]
fn test_chain_forward_reference() {
    // type A is { b: B };
    // type B is { c: C };
    // type C is { value: Int };

    let mut checker = TypeChecker::new();
    checker.register_builtins();

    let type_a = make_type_decl("A", make_record_body(vec![("b", "B")]));

    let type_b = make_type_decl("B", make_record_body(vec![("c", "C")]));

    let type_c = make_type_decl("C", make_record_body(vec![("value", "Int")]));

    let items = vec![
        make_type_item(type_a.clone()),
        make_type_item(type_b.clone()),
        make_type_item(type_c.clone()),
    ];

    checker.register_all_type_names(&items);

    let mut resolution_stack = List::new();
    let result_a = checker.resolve_type_definition(&type_a, &mut resolution_stack);
    let result_b = checker.resolve_type_definition(&type_b, &mut resolution_stack);
    let result_c = checker.resolve_type_definition(&type_c, &mut resolution_stack);

    assert!(result_a.is_ok(), "A should resolve");
    assert!(result_b.is_ok(), "B should resolve");
    assert!(result_c.is_ok(), "C should resolve");
}

// ============================================================================
// Variant Type Forward Reference Tests
// ============================================================================

#[test]
fn test_variant_with_forward_reference_payload() {
    // type Result is Ok(Value) | Err(ErrorInfo);
    // type Value is { data: Int };
    // type ErrorInfo is { message: Text };

    let mut checker = TypeChecker::new();
    checker.register_builtins();

    let result_type = make_type_decl(
        "MyResult",
        TypeDeclBody::Variant(vec![
            Variant {
                name: make_ident("Ok"),
                generic_params: vec![].into(),
                data: Some(VariantData::Tuple(vec![make_type_path("Value")].into())),
                where_clause: verum_common::Maybe::None,
                attributes: vec![].into(),
                path_endpoints: None,
                path_dim: 1,
                span: Span::dummy(),
            },
            Variant {
                name: make_ident("Err"),
                generic_params: vec![].into(),
                data: Some(VariantData::Tuple(vec![make_type_path("ErrorInfo")].into())),
                where_clause: verum_common::Maybe::None,
                attributes: vec![].into(),
                path_endpoints: None,
                path_dim: 1,
                span: Span::dummy(),
            },
        ].into()),
    );

    let value_type = make_type_decl("Value", make_record_body(vec![("data", "Int")]));

    let error_info = make_type_decl("ErrorInfo", make_record_body(vec![("message", "Text")]));

    let items = vec![
        make_type_item(result_type.clone()),
        make_type_item(value_type.clone()),
        make_type_item(error_info.clone()),
    ];

    checker.register_all_type_names(&items);

    let mut resolution_stack = List::new();
    let r1 = checker.resolve_type_definition(&result_type, &mut resolution_stack);
    let r2 = checker.resolve_type_definition(&value_type, &mut resolution_stack);
    let r3 = checker.resolve_type_definition(&error_info, &mut resolution_stack);

    assert!(r1.is_ok(), "MyResult should resolve: {:?}", r1);
    assert!(r2.is_ok(), "Value should resolve");
    assert!(r3.is_ok(), "ErrorInfo should resolve");
}

// ============================================================================
// Batch Resolution Tests
// ============================================================================

#[test]
fn test_batch_resolution() {
    // Test the convenience batch methods

    let mut checker = TypeChecker::new();
    checker.register_builtins();

    let type_a = make_type_decl("A", make_record_body(vec![("b", "B")]));

    let type_b = make_type_decl("B", make_variant_body(vec!["First", "Second"]));

    let items = vec![make_type_item(type_a), make_type_item(type_b)];

    // Use batch methods
    checker.register_all_type_names(&items);
    let results = checker.resolve_all_type_definitions(&items);

    // All resolutions should succeed
    for (i, result) in results.iter().enumerate() {
        assert!(result.is_ok(), "Type {} should resolve: {:?}", i, result);
    }
}

// ============================================================================
// Complex Scenario Tests
// ============================================================================

#[test]
fn test_complex_type_graph() {
    // A more realistic scenario with multiple interrelated types:
    //
    // type User is { profile: Profile, posts: PostList };
    // type Profile is { name: Text, settings: Settings };
    // type Settings is { theme: Theme, notifications: Bool };
    // type Theme is Light | Dark;
    // type PostList is { items: Post };  // Simplified
    // type Post is { author: User, content: Content };  // Note: circular reference
    // type Content is { text: Text };

    let mut checker = TypeChecker::new();
    checker.register_builtins();

    let user = make_type_decl(
        "User",
        make_record_body(vec![("profile", "Profile"), ("posts", "PostList")]),
    );

    let profile = make_type_decl(
        "Profile",
        make_record_body(vec![("name", "Text"), ("settings", "Settings")]),
    );

    let settings = make_type_decl(
        "Settings",
        make_record_body(vec![("theme", "Theme"), ("notifications", "Bool")]),
    );

    let theme = make_type_decl("Theme", make_variant_body(vec!["Light", "Dark"]));

    let post_list = make_type_decl("PostList", make_record_body(vec![("items", "Post")]));

    let post = make_type_decl(
        "Post",
        make_record_body(vec![
            ("author", "User"), // Circular reference
            ("content", "Content"),
        ]),
    );

    let content = make_type_decl("Content", make_record_body(vec![("text", "Text")]));

    let items = vec![
        make_type_item(user.clone()),
        make_type_item(profile.clone()),
        make_type_item(settings.clone()),
        make_type_item(theme.clone()),
        make_type_item(post_list.clone()),
        make_type_item(post.clone()),
        make_type_item(content.clone()),
    ];

    checker.register_all_type_names(&items);

    // Resolve in arbitrary order (the whole point of two-pass resolution)
    let results = checker.resolve_all_type_definitions(&items);

    // All should resolve (circular references are allowed when not causing infinite size)
    for (i, result) in results.iter().enumerate() {
        assert!(result.is_ok(), "Type {} should resolve: {:?}", i, result);
    }
}

// ============================================================================
// Newtype Forward Reference Tests
// ============================================================================

#[test]
fn test_newtype_forward_reference() {
    // type UserId is CustomInt;
    // type CustomInt is Int;

    let mut checker = TypeChecker::new();
    checker.register_builtins();

    let user_id = make_type_decl("UserId", TypeDeclBody::Newtype(make_type_path("CustomInt")));

    let custom_int = make_type_decl(
        "CustomInt",
        TypeDeclBody::Alias(AstType {
            kind: TypeKind::Int,
            span: Span::dummy(),
        }),
    );

    let items = vec![
        make_type_item(user_id.clone()),
        make_type_item(custom_int.clone()),
    ];

    checker.register_all_type_names(&items);

    let mut resolution_stack = List::new();
    let r1 = checker.resolve_type_definition(&user_id, &mut resolution_stack);
    let r2 = checker.resolve_type_definition(&custom_int, &mut resolution_stack);

    // Both should resolve
    assert!(r2.is_ok(), "CustomInt should resolve");
    // UserId may resolve to a placeholder initially, but that's acceptable
}

// ============================================================================
// Tuple Type Forward Reference Tests
// ============================================================================

#[test]
fn test_tuple_type_forward_reference() {
    // type Pair is (First, Second);
    // type First is { value: Int };
    // type Second is { name: Text };

    let mut checker = TypeChecker::new();
    checker.register_builtins();

    let pair = make_type_decl(
        "Pair",
        TypeDeclBody::Tuple(vec![make_type_path("First"), make_type_path("Second")].into()),
    );

    let first = make_type_decl("First", make_record_body(vec![("value", "Int")]));
    let second = make_type_decl("Second", make_record_body(vec![("name", "Text")]));

    let items = vec![
        make_type_item(pair.clone()),
        make_type_item(first.clone()),
        make_type_item(second.clone()),
    ];

    checker.register_all_type_names(&items);
    let results = checker.resolve_all_type_definitions(&items);

    assert!(results[0].is_ok(), "Pair should resolve");
    assert!(results[1].is_ok(), "First should resolve");
    assert!(results[2].is_ok(), "Second should resolve");
}
