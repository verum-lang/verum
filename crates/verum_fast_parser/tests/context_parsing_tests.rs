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
//! Comprehensive tests for context declaration parsing
//!
//! Tests all forms of context declarations:
//! context syntax, provide statements, using clauses, sub-contexts, aliases

use verum_ast::{FileId, ItemKind, decl::*};
use verum_lexer::Lexer;
use verum_fast_parser::RecursiveParser;

fn parse_context_decl(source: &str) -> Result<ContextDecl, String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
    let mut parser = RecursiveParser::new(&tokens, file_id);

    match parser.parse_item() {
        Ok(item) => match item.kind {
            ItemKind::Context(decl) => Ok(decl),
            _ => Err(format!("Expected context declaration, got {:?}", item.kind)),
        },
        Err(e) => Err(format!("Parse error: {:?}", e)),
    }
}

#[test]
fn test_basic_context() {
    let source = r#"
context Database {
    fn connect() -> Connection;
    fn query(sql: Text) -> Result<Rows, Error>;
}
"#;

    let result = parse_context_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse basic context: {:?}",
        result
    );

    let decl = result.unwrap();
    assert_eq!(decl.name.name.as_str(), "Database");
    assert!(!decl.is_async);
    assert_eq!(decl.methods.len(), 2);
    assert_eq!(decl.methods[0].name.name.as_str(), "connect");
    assert_eq!(decl.methods[1].name.name.as_str(), "query");
}

#[test]
fn test_async_context() {
    let source = r#"
context async Database {
    async fn query(sql: Text) -> Result<Rows, Error>;
    async fn transaction<T>(body: fn() -> T) -> T;
}
"#;

    let result = parse_context_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse async context: {:?}",
        result
    );

    let decl = result.unwrap();
    assert_eq!(decl.name.name.as_str(), "Database");
    assert!(decl.is_async);
    assert_eq!(decl.methods.len(), 2);
    assert!(decl.methods[0].is_async);
    assert!(decl.methods[1].is_async);
}

#[test]
fn test_parameterized_context() {
    let source = r#"
context State<S> {
    fn get() -> S;
    fn put(new_state: S);
    fn modify(f: fn(S) -> S);
}
"#;

    let result = parse_context_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse parameterized context: {:?}",
        result
    );

    let decl = result.unwrap();
    assert_eq!(decl.name.name.as_str(), "State");
    assert_eq!(decl.generics.len(), 1);
    assert_eq!(decl.methods.len(), 3);
}

#[test]
fn test_context_with_multiple_type_params() {
    let source = r#"
context Cache<K, V> {
    fn get(key: K) -> Maybe<V>;
    fn put(key: K, value: V);
    fn invalidate(key: K);
}
"#;

    let result = parse_context_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse multi-param context: {:?}",
        result
    );

    let decl = result.unwrap();
    assert_eq!(decl.name.name.as_str(), "Cache");
    assert_eq!(decl.generics.len(), 2);
    assert_eq!(decl.methods.len(), 3);
}

#[test]
fn test_context_with_using_clause() {
    let source = r#"
context Database {
    fn connect() -> Connection using [Network];
    fn query(sql: Text) -> Result<Rows, Error>;
}
"#;

    let result = parse_context_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse context with using clause: {:?}",
        result
    );

    let decl = result.unwrap();
    assert_eq!(decl.methods.len(), 2);
    assert_eq!(decl.methods[0].contexts.len(), 1);
}

#[test]
fn test_empty_context() {
    let source = r#"
context EmptyContext {
}
"#;

    let result = parse_context_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse empty context: {:?}",
        result
    );

    let decl = result.unwrap();
    assert_eq!(decl.name.name.as_str(), "EmptyContext");
    assert_eq!(decl.methods.len(), 0);
}

#[test]
fn test_public_context() {
    let source = r#"
public context Logger {
    fn info(msg: Text);
    fn error(msg: Text);
}
"#;

    let result = parse_context_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse public context: {:?}",
        result
    );

    let decl = result.unwrap();
    assert_eq!(decl.visibility, Visibility::Public);
}

#[test]
fn test_context_with_generic_methods() {
    let source = r#"
context Logger {
    fn log<T>(value: T) -> Text;
}
"#;

    let result = parse_context_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse context with generic methods: {:?}",
        result
    );

    let decl = result.unwrap();
    assert_eq!(decl.methods.len(), 1);
    assert_eq!(decl.methods[0].generics.len(), 1);
}

#[test]
fn test_context_mixed_sync_async_methods() {
    let source = r#"
context MixedContext {
    fn sync_method() -> Int;
    async fn async_method() -> Int;
}
"#;

    let result = parse_context_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse context with mixed sync/async: {:?}",
        result
    );

    let decl = result.unwrap();
    assert_eq!(decl.methods.len(), 2);
    assert!(!decl.methods[0].is_async);
    assert!(decl.methods[1].is_async);
}

#[test]
fn test_context_spec_example() {
    // The exact example from the spec
    let source = r#"
context Database {
    fn connect() -> Connection using [Network];
    fn query(sql: Text) -> Result<Rows, Error>;
    async fn transaction<T>(body: fn() -> T) -> T;
}
"#;

    let result = parse_context_decl(source);
    assert!(result.is_ok(), "Failed to parse spec example: {:?}", result);

    let decl = result.unwrap();
    assert_eq!(decl.name.name.as_str(), "Database");
    assert_eq!(decl.methods.len(), 3);

    // First method uses Network context
    assert_eq!(decl.methods[0].contexts.len(), 1);

    // Third method is async
    assert!(decl.methods[2].is_async);
    assert_eq!(decl.methods[2].generics.len(), 1);
}

// ============================================================================
// Sub-Context Tests: fine-grained capabilities (e.g., FS.ReadOnly, FS.WriteOnly)
// ============================================================================

#[test]
fn test_sub_contexts_basic() {
    // Basic sub-context example from spec Section 10.1
    let source = r#"
context FileSystem {
    context Read {
        fn read_file(path: Text) -> Result<Text, Error>;
        fn list_files(dir: Text) -> Result<List<Text>, Error>;
    }

    context Write {
        fn write_file(path: Text, content: Text) -> Result<(), Error>;
        fn create_directory(path: Text) -> Result<(), Error>;
    }
}
"#;

    let result = parse_context_decl(source);
    assert!(result.is_ok(), "Failed to parse sub-contexts: {:?}", result);

    let decl = result.unwrap();
    assert_eq!(decl.name.name.as_str(), "FileSystem");
    assert_eq!(decl.methods.len(), 0, "Parent should have no methods");
    assert_eq!(decl.sub_contexts.len(), 2, "Should have 2 sub-contexts");

    // Check Read sub-context
    let read_ctx = &decl.sub_contexts[0];
    assert_eq!(read_ctx.name.name.as_str(), "Read");
    assert_eq!(read_ctx.methods.len(), 2);
    assert_eq!(read_ctx.methods[0].name.name.as_str(), "read_file");
    assert_eq!(read_ctx.methods[1].name.name.as_str(), "list_files");

    // Check Write sub-context
    let write_ctx = &decl.sub_contexts[1];
    assert_eq!(write_ctx.name.name.as_str(), "Write");
    assert_eq!(write_ctx.methods.len(), 2);
    assert_eq!(write_ctx.methods[0].name.name.as_str(), "write_file");
    assert_eq!(write_ctx.methods[1].name.name.as_str(), "create_directory");
}

#[test]
fn test_sub_contexts_with_methods() {
    // Parent context with both methods and sub-contexts
    let source = r#"
context Database {
    fn connect() -> Connection;

    context Query {
        fn select(sql: Text) -> Rows;
    }

    fn close();
}
"#;

    let result = parse_context_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse mixed context: {:?}",
        result
    );

    let decl = result.unwrap();
    assert_eq!(decl.name.name.as_str(), "Database");
    assert_eq!(decl.methods.len(), 2, "Should have 2 parent methods");
    assert_eq!(decl.sub_contexts.len(), 1, "Should have 1 sub-context");

    assert_eq!(decl.methods[0].name.name.as_str(), "connect");
    assert_eq!(decl.methods[1].name.name.as_str(), "close");

    let query_ctx = &decl.sub_contexts[0];
    assert_eq!(query_ctx.name.name.as_str(), "Query");
    assert_eq!(query_ctx.methods.len(), 1);
}

#[test]
fn test_async_sub_context() {
    // Async sub-context within parent
    let source = r#"
context FS {
    context Async {
        async fn read_async(path: Text) -> Result<Text, Error>;
        async fn write_async(path: Text, data: Text) -> Result<(), Error>;
    }
}
"#;

    let result = parse_context_decl(source);
    // Note: Currently we parse "context Async" as a regular sub-context named "Async"
    // In the future, we may want syntax like "async context AsyncIO { ... }"
    assert!(result.is_ok(), "Failed to parse: {:?}", result);

    let decl = result.unwrap();
    assert_eq!(decl.sub_contexts.len(), 1);
    let async_ctx = &decl.sub_contexts[0];
    assert_eq!(async_ctx.name.name.as_str(), "Async");
    assert!(async_ctx.methods[0].is_async);
    assert!(async_ctx.methods[1].is_async);
}

#[test]
fn test_nested_sub_contexts() {
    // Deeply nested sub-contexts
    let source = r#"
context Security {
    context Auth {
        fn authenticate(token: Text) -> Bool;

        context Admin {
            fn grant_permission(user: Text, perm: Text);
        }
    }
}
"#;

    let result = parse_context_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse nested sub-contexts: {:?}",
        result
    );

    let decl = result.unwrap();
    assert_eq!(decl.name.name.as_str(), "Security");
    assert_eq!(decl.sub_contexts.len(), 1);

    let auth_ctx = &decl.sub_contexts[0];
    assert_eq!(auth_ctx.name.name.as_str(), "Auth");
    assert_eq!(auth_ctx.methods.len(), 1);
    assert_eq!(auth_ctx.sub_contexts.len(), 1);

    let admin_ctx = &auth_ctx.sub_contexts[0];
    assert_eq!(admin_ctx.name.name.as_str(), "Admin");
    assert_eq!(admin_ctx.methods.len(), 1);
    assert_eq!(admin_ctx.methods[0].name.name.as_str(), "grant_permission");
}

// =============================================================================
// Context Protocol Tests (Dual-Kind: Constraint & Injectable)
// Context kind modifier: context, context type, context protocol
// =============================================================================

/// Helper to parse any item and return error messages
fn parse_item_expecting_error(source: &str) -> String {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
    let mut parser = RecursiveParser::new(&tokens, file_id);

    match parser.parse_item() {
        Ok(_) => "Expected error, but parsing succeeded".to_string(),
        Err(error) => {
            // RecursiveParser returns a single ParseError, not a List
            let mut msg = format!("{}", error);
            if let Some(help) = &error.help {
                msg.push_str(&format!("\nHelp: {}", help));
            }
            msg
        }
    }
}

/// Helper to parse protocol declaration (returns TypeDecl for context protocol)
fn parse_protocol_decl(source: &str) -> Result<TypeDecl, String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
    let mut parser = RecursiveParser::new(&tokens, file_id);

    match parser.parse_item() {
        Ok(item) => match item.kind {
            ItemKind::Type(decl) => Ok(decl),
            ItemKind::Protocol(decl) => {
                // Convert ProtocolDecl to TypeDecl for unified testing
                // This happens for `context protocol Name { }` form
                Ok(TypeDecl {
                    visibility: decl.visibility,
                    name: decl.name,
                    generics: decl.generics,
                    attributes: verum_common::List::new(), // Attributes are on Item, not ProtocolDecl
                    body: TypeDeclBody::Protocol(ProtocolBody {
                        extends: decl.bounds.iter().cloned().collect(), // bounds -> extends
                        items: decl.items,
                        is_context: decl.is_context,
                        generic_where_clause: decl.generic_where_clause,
                    }),
                    resource_modifier: verum_common::Maybe::None,
                    generic_where_clause: verum_common::Maybe::None,
                    meta_where_clause: verum_common::Maybe::None,
                    span: decl.span,
                })
            }
            other => Err(format!("Expected type/protocol declaration, got {:?}", other)),
        },
        Err(e) => Err(format!("Parse error: {:?}", e)),
    }
}

#[test]
fn test_context_protocol_primary_form() {
    // Primary form: context protocol Name { ... }
    let source = r#"
context protocol Serializable {
    fn serialize(&self) -> Text;
    fn deserialize(s: &Text) -> Result<Self, Error>;
}
"#;

    let result = parse_protocol_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse context protocol (primary form): {:?}",
        result
    );

    let decl = result.unwrap();
    assert_eq!(decl.name.name.as_str(), "Serializable");

    // Verify it's marked as a context protocol
    if let TypeDeclBody::Protocol(body) = &decl.body {
        assert!(body.is_context, "Should be marked as context protocol");
        assert_eq!(body.items.len(), 2, "Should have 2 methods");
    } else {
        panic!("Expected Protocol body");
    }
}

#[test]
fn test_context_protocol_with_extends() {
    // Context protocol with extends clause
    let source = r#"
context protocol Cacheable extends Serializable + Clone {
    fn cache_key(&self) -> Text;
}
"#;

    let result = parse_protocol_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse context protocol with extends: {:?}",
        result
    );

    let decl = result.unwrap();
    if let TypeDeclBody::Protocol(body) = &decl.body {
        assert!(body.is_context);
        assert_eq!(body.extends.len(), 2, "Should extend 2 protocols");
    } else {
        panic!("Expected Protocol body");
    }
}

#[test]
fn test_context_type_protocol_alternative_form() {
    // Alternative form: context type Name is protocol { ... };
    let source = r#"
context type Validator is protocol {
    fn validate(&self) -> Result<(), ValidationError>;
    fn is_valid(&self) -> Bool;
};
"#;

    let result = parse_protocol_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse context type protocol (alternative form): {:?}",
        result
    );

    let decl = result.unwrap();
    assert_eq!(decl.name.name.as_str(), "Validator");

    if let TypeDeclBody::Protocol(body) = &decl.body {
        assert!(body.is_context, "Should be marked as context protocol");
        assert_eq!(body.items.len(), 2);
    } else {
        panic!("Expected Protocol body");
    }
}

#[test]
fn test_context_type_without_protocol_gives_helpful_error() {
    // ERROR CASE: context type Name is { ... } (missing 'protocol')
    let source = r#"
context type Logger is {
    fn log(msg: Text);
};
"#;

    let error_msg = parse_item_expecting_error(source);
    
    // Verify error mentions the missing 'protocol' keyword
    assert!(
        error_msg.contains("protocol"),
        "Error should mention 'protocol' keyword. Got: {}",
        error_msg
    );
    
    // Verify helpful suggestions are present
    assert!(
        error_msg.contains("context protocol") || error_msg.contains("is protocol"),
        "Error should suggest correct syntax. Got: {}",
        error_msg
    );
    
    // Verify semantic explanation is present
    assert!(
        error_msg.contains("Contexts are NOT types") || error_msg.contains("capability"),
        "Error should explain why contexts aren't types. Got: {}",
        error_msg
    );
}

#[test]
fn test_context_type_with_record_body_gives_helpful_error() {
    // ERROR CASE: context type Name is { field: Type } (record body, not protocol)
    let source = r#"
context type Config is {
    timeout: Int,
    retries: Int,
};
"#;

    let error_msg = parse_item_expecting_error(source);
    
    // Should fail with helpful error
    assert!(
        error_msg.contains("protocol") || error_msg.contains("missing"),
        "Error should indicate protocol is required. Got: {}",
        error_msg
    );
}

#[test]
fn test_public_context_protocol() {
    // Public visibility on context protocol
    let source = r#"
public context protocol Authenticator {
    fn authenticate(credentials: Credentials) -> Result<User, AuthError>;
}
"#;

    let result = parse_protocol_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse public context protocol: {:?}",
        result
    );

    let decl = result.unwrap();
    assert!(matches!(decl.visibility, Visibility::Public));
}

#[test]
fn test_generic_context_protocol() {
    // Generic context protocol
    let source = r#"
context protocol Repository<T, Id> {
    fn find(id: Id) -> Maybe<T>;
    fn save(item: &T) -> Result<Id, Error>;
    fn delete(id: Id) -> Result<(), Error>;
}
"#;

    let result = parse_protocol_decl(source);
    assert!(
        result.is_ok(),
        "Failed to parse generic context protocol: {:?}",
        result
    );

    let decl = result.unwrap();
    assert_eq!(decl.generics.len(), 2, "Should have 2 type parameters");
}
