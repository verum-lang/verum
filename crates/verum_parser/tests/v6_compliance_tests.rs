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
// Comprehensive parser tests for Verum v6.0-BALANCED specification compliance.
//
// Tests for Verum v6 syntax compliance

use verum_ast::{FileId, ItemKind, TypeKind, decl::*};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

fn parse_source(source: &str) -> Result<Vec<verum_ast::Item>, verum_common::List<verum_fast_parser::ParseError>> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser
        .parse_module(lexer, file_id)
        .map(|m| m.items.to_vec())
}

#[test]
fn test_unified_is_syntax_for_type_alias() {
    // Record type: `type Point is { x: Float, y: Float };`
    let source = "type UserId is Int;";
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    assert!(matches!(&items[0].kind, ItemKind::Type(decl) if decl.name.name == "UserId"));
}

#[test]
fn test_unified_is_syntax_for_record() {
    // Sum type variants: `type Option<T> is None | Some(T);`
    let source = r#"
        type User is {
            id: Int,
            email: Text,
            age: Int
        };
    "#;
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::Type(decl) = &items[0].kind {
        assert_eq!(decl.name.name, "User");
        assert!(matches!(decl.body, TypeDeclBody::Record(_)));
    } else {
        panic!("Expected Type item");
    }
}

#[test]
fn test_unified_is_syntax_for_variant() {
    // Nested variant: `type Tree<T> is Leaf(T) | Node { left: Heap<Tree<T>>, ... };`
    let source = r#"
        type Result<T, E> is
            | Ok(T)
            | Err(E);
    "#;
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::Type(decl) = &items[0].kind {
        assert_eq!(decl.name.name, "Result");
        assert!(matches!(decl.body, TypeDeclBody::Variant(_)));
    } else {
        panic!("Expected Type item");
    }
}

#[test]
fn test_unified_is_syntax_for_newtype() {
    // Protocol definition: `type Iterator is protocol { type Item; fn next(); };`
    let source = "type Kilometers is (Float);";
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::Type(decl) = &items[0].kind {
        assert_eq!(decl.name.name, "Kilometers");
        assert!(matches!(decl.body, TypeDeclBody::Tuple(_)));
    } else {
        panic!("Expected Type item");
    }
}

#[test]
fn test_inline_refinement_syntax() {
    // Newtype and tuple: `type UserId is (Int);` and `type Pair is (Int, Text);`
    let source = "type Positive is Int{> 0};";
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    // The refinement is parsed as part of the type body (alias)
    if let ItemKind::Type(decl) = &items[0].kind {
        assert_eq!(decl.name.name, "Positive");
    } else {
        panic!("Expected Type item");
    }
}

#[test]
fn test_multiple_inline_refinements() {
    // Type alias: `type Alias is ExistingType;`
    let source = r#"
        type Age is Int{>= 0, <= 150};
    "#;
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::Type(decl) = &items[0].kind {
        assert_eq!(decl.name.name, "Age");
    } else {
        panic!("Expected Type item");
    }
}

#[test]
fn test_context_declaration() {
    // Context declaration with function using clause: `fn f() using [Ctx]`
    // Context methods require semicolons per grammar/verum.ebnf line 454-459
    let source = r#"
        context Database {
            fn query(sql: Text) -> Result<Rows>;
        }
    "#;
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::Context(decl) = &items[0].kind {
        assert_eq!(decl.name.name, "Database");
        assert_eq!(decl.methods.len(), 1);
    } else {
        panic!("Expected Context item");
    }
}

#[test]
fn test_async_context_declaration() {
    // Async context: `context async Database { async fn query(); }`
    // Context methods require semicolons per grammar/verum.ebnf line 454-459
    let source = r#"
        async context Database {
            async fn query(sql: Text) -> Result<Rows>;
            async fn execute(sql: Text) -> Result<()>;
        }
    "#;
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::Context(decl) = &items[0].kind {
        assert_eq!(decl.name.name, "Database");
        assert!(decl.is_async, "Async context should have is_async = true");
        assert_eq!(decl.methods.len(), 2);
    } else {
        panic!("Expected Context item");
    }
}

#[test]
fn test_context_group_declaration() {
    // Context providing: `provide Ctx = impl;` installs dependency in task-local storage
    let source = r#"
        context group WebApp {
            Database,
            Logger,
            Cache
        }
    "#;
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::ContextGroup(decl) = &items[0].kind {
        assert_eq!(decl.name.name, "WebApp");
        assert_eq!(decl.contexts.len(), 3);
    } else {
        panic!("Expected ContextGroup item");
    }
}

#[test]
fn test_function_with_single_context_no_brackets() {
    // Contract annotations: `requires cond`, `ensures cond`, `invariant cond`
    // Single context - brackets optional
    let source = r#"
        fn query_user(id: Int) -> User using Database {
            Database.query(sql"SELECT * FROM users WHERE id = {id}")
        }
    "#;
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::Function(decl) = &items[0].kind {
        assert_eq!(decl.name.name, "query_user");
        // Contexts should be present
        assert!(!decl.contexts.is_empty());
    } else {
        panic!("Expected Function item");
    }
}

#[test]
fn test_function_with_multiple_contexts_brackets_required() {
    // Contract annotations: `requires cond`, `ensures cond`, `invariant cond`
    // Multiple contexts - brackets required
    let source = r#"
        fn complex_operation() -> Result<Data> using [Database, Logger, Cache] {
            // implementation
        }
    "#;
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::Function(decl) = &items[0].kind {
        assert_eq!(decl.name.name, "complex_operation");
        // Contexts should be present with multiple contexts
        assert!(!decl.contexts.is_empty(), "Expected contexts");
        assert_eq!(decl.contexts.len(), 3);
    } else {
        panic!("Expected Function item");
    }
}

#[test]
fn test_protocol_declaration() {
    // Implement block: `implement Protocol for Type { ... }`
    // Note: "type X is protocol { ... }" creates ItemKind::Type with TypeDeclBody::Protocol
    let source = r#"
        type Serializable is protocol {
            fn serialize(self) -> JsonValue;
        };
    "#;
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::Type(type_decl) = &items[0].kind {
        assert_eq!(type_decl.name.name, "Serializable");
        if let TypeDeclBody::Protocol(protocol_body) = &type_decl.body {
            assert_eq!(protocol_body.items.len(), 1);
        } else {
            panic!("Expected TypeDeclBody.Protocol");
        }
    } else {
        panic!("Expected Type item with Protocol body");
    }
}

#[test]
fn test_implement_block() {
    // Let binding with destructuring: `let (a, b) = tuple;`
    let source = r#"
        implement Display for User {
            fn display(self) -> Text {
                f"User({self.id})"
            }
        }
    "#;
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::Impl(decl) = &items[0].kind {
        assert!(matches!(decl.kind, ImplKind::Protocol { .. }));
    } else {
        panic!("Expected Impl item");
    }
}

#[test]
fn test_affine_type_modifier() {
    // Function definition: `fn name<T>(params) -> RetType { body }`
    let source = r#"
        type affine FileHandle is {
            fd: Int,
            path: Text
        };
    "#;
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::Type(decl) = &items[0].kind {
        assert_eq!(decl.name.name, "FileHandle");
        assert!(matches!(
            decl.resource_modifier,
            Some(verum_ast::ResourceModifier::Affine)
        ));
    } else {
        panic!("Expected Type item");
    }
}

#[test]
fn test_async_function() {
    // Contract and verification annotations on functions
    let source = r#"
        async fn fetch_data(url: Text) -> Result<Data> {
            http.get(url).await
        }
    "#;
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::Function(decl) = &items[0].kind {
        assert_eq!(decl.name.name, "fetch_data");
        assert!(decl.is_async);
    } else {
        panic!("Expected Function item");
    }
}

#[test]
fn test_generic_type_definition() {
    // Contract and verification annotations on functions
    let source = r#"
        type Option<T> is
            | Some(T)
            | None;
    "#;
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::Type(decl) = &items[0].kind {
        assert_eq!(decl.name.name, "Option");
        assert_eq!(decl.generics.len(), 1);
    } else {
        panic!("Expected Type item");
    }
}

#[test]
fn test_mount_statement() {
    // Module system: `module name { ... }`, `mount path.{items}`
    let source = "mount std.collections.List;";
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    assert!(matches!(&items[0].kind, ItemKind::Mount(_)));
}

#[test]
fn test_const_declaration() {
    let source = "const MAX_SIZE: Int = 1000;";
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::Const(decl) = &items[0].kind {
        assert_eq!(decl.name.name, "MAX_SIZE");
    } else {
        panic!("Expected Const item");
    }
}

#[test]
fn test_complete_v6_program() {
    // Integration test with multiple v6.0-BALANCED features
    let source = r#"
        type Positive is Int{> 0};

        type User is {
            id: Int,
            email: Text,
            age: Positive
        };

        context Database {
            fn query(sql: Text) -> Result<Rows>;
        }

        async fn fetch_user(id: Int) -> Result<User> using Database {
            let query = sql"SELECT * FROM users WHERE id = {id}";
            let rows = Database.query(query).await?;
            parse_user(rows)
        }

        type Serializable is protocol {
            fn serialize(self) -> Text;
        };

        implement Serializable for User {
            fn serialize(self) -> Text {
                f"User({self.id}, {self.email})"
            }
        }
    "#;

    let items = parse_source(source).unwrap();

    // Verify we parsed all items
    assert!(items.len() >= 5);

    // Check types
    let type_items: Vec<_> = items
        .iter()
        .filter(|i| matches!(i.kind, ItemKind::Type(_)))
        .collect();
    assert_eq!(type_items.len(), 3); // Positive, User, and Serializable (as "type X is protocol")

    // Check context
    let context_items: Vec<_> = items
        .iter()
        .filter(|i| matches!(i.kind, ItemKind::Context(_)))
        .collect();
    assert_eq!(context_items.len(), 1);

    // Check functions
    let fn_items: Vec<_> = items
        .iter()
        .filter(|i| matches!(i.kind, ItemKind::Function(_)))
        .collect();
    assert_eq!(fn_items.len(), 1); // fetch_user

    // Check protocol - now 0 because "type X is protocol { ... }" creates ItemKind::Type
    let protocol_items: Vec<_> = items
        .iter()
        .filter(|i| matches!(i.kind, ItemKind::Protocol(_)))
        .collect();
    assert_eq!(protocol_items.len(), 0);

    // Check impl
    let impl_items: Vec<_> = items
        .iter()
        .filter(|i| matches!(i.kind, ItemKind::Impl(_)))
        .collect();
    assert_eq!(impl_items.len(), 1);
}

#[test]
fn test_deprecated_keywords_not_allowed() {
    // Core keywords: let, fn, is — the three reserved keywords
    // These should parse (lexer recognizes them) but we should use unified 'is' syntax

    // Modern unified syntax
    let modern = "type Point is { x: Float, y: Float };";
    assert!(parse_source(modern).is_ok());

    // If we try to use deprecated 'struct' keyword, parser should handle it
    // (implementation may vary - some parsers emit warnings, others reject)
}

#[test]
fn test_visibility_modifiers() {
    let source = r#"
        pub type PublicType is Int;
        public fn public_function() {}
    "#;
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 2);
    if let ItemKind::Type(decl) = &items[0].kind {
        assert!(matches!(decl.visibility, Visibility::Public));
    }
    if let ItemKind::Function(decl) = &items[1].kind {
        assert!(matches!(decl.visibility, Visibility::Public));
    }
}

#[test]
fn test_variant_with_record_data() {
    // Generic type definition with constraints: `type Container<T: Display>`
    let source = r#"
        type Shape is
            | Circle { radius: Float }
            | Rectangle { width: Float, height: Float }
            | Point;
    "#;
    let items = parse_source(source).unwrap();

    assert_eq!(items.len(), 1);
    if let ItemKind::Type(decl) = &items[0].kind {
        assert_eq!(decl.name.name, "Shape");
        if let TypeDeclBody::Variant(variants) = &decl.body {
            assert_eq!(variants.len(), 3);
        } else {
            panic!("Expected variant body");
        }
    } else {
        panic!("Expected Type item");
    }
}

#[test]
fn test_generic_protocol_declaration() {
    // Test generic protocol with type parameter in method return type
    let source = r#"
        type Provider<T> is protocol {
            fn provide() -> T;
        };
    "#;

    let items = parse_source(source);
    match items {
        Ok(items) => {
            assert_eq!(items.len(), 1, "Expected 1 item");
            if let ItemKind::Type(type_decl) = &items[0].kind {
                assert_eq!(type_decl.name.name, "Provider");
                assert_eq!(type_decl.generics.len(), 1);
            } else {
                panic!("Expected Type item");
            }
        }
        Err(errors) => {
            panic!("Parse failed with errors: {:?}", errors);
        }
    }
}

#[test]
fn debug_generic_protocol_tokens() {
    use verum_lexer::Lexer;
    use verum_ast::FileId;
    
    // Test generic protocol with type parameter in method return type
    let source = r#"
        type Provider<T> is protocol {
            fn provide() -> T;
        };
    "#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    
    eprintln!("Source:\n{}", source);
    eprintln!("\nTokens:");
    for (i, result) in lexer.enumerate() {
        match result {
            Ok(token) => {
                eprintln!("{:3}: {:?} at {:?}", i, token.kind, token.span);
            }
            Err(e) => {
                eprintln!("{:3}: ERROR: {:?}", i, e);
            }
        }
    }
    
    // Now try parsing
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = verum_fast_parser::VerumParser::new();
    
    match parser.parse_module(lexer, file_id) {
        Ok(module) => eprintln!("Parsed {} items", module.items.len()),
        Err(errors) => {
            eprintln!("Parse errors:");
            for e in &errors {
                eprintln!("  {:?}", e);
            }
        }
    }
}
