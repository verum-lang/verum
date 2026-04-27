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
//! Comprehensive tests for complete context system parsing
//!
//! These tests verify all context-related parsing features including:
//! - Context declarations with all variations
//! - Context groups
//! - Provide statements
//! - Using clauses in functions
//! - Sub-contexts (nested contexts)

use verum_ast::{FileId, ItemKind, StmtKind, decl::*};
use verum_lexer::Lexer;
use verum_fast_parser::RecursiveParser;

fn parse_module(source: &str) -> Result<Vec<verum_ast::Item>, String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
    let mut parser = RecursiveParser::new(&tokens, file_id);

    // `RecursiveParser::parse_module` is a recovery parser: it returns
    // `Ok(items)` even when errors were collected during the walk, accumulating
    // them on `parser.errors`. The high-level `FastParser::parse_module`
    // (lib.rs) checks that vector and converts a non-empty list into `Err`.
    // Tests that want "did the source parse cleanly?" must do the same — a
    // bare `parse_module().is_ok()` silently drops every recovered error.
    let items = parser
        .parse_module()
        .map_err(|e| format!("Parse error: {:?}", e))?;
    if !parser.errors.is_empty() {
        return Err(format!(
            "Parse error(s): {:?}",
            parser.errors.into_iter().collect::<Vec<_>>()
        ));
    }
    Ok(items)
}

#[test]
fn test_context_group_declaration() {
    let source = r#"
context group WebApp {
    Database,
    Logger,
    Cache
}
"#;

    let result = parse_module(source);
    assert!(
        result.is_ok(),
        "Failed to parse context group: {:?}",
        result
    );

    let items = result.unwrap();
    assert_eq!(items.len(), 1);

    match &items[0].kind {
        ItemKind::ContextGroup(group) => {
            assert_eq!(group.name.name.as_str(), "WebApp");
            assert_eq!(group.contexts.len(), 3);
        }
        _ => panic!("Expected ContextGroup, got {:?}", items[0].kind),
    }
}

#[test]
fn test_context_group_alias_syntax() {
    let source = r#"
using ServerContext = [Database, Logger, Cache];
"#;

    let result = parse_module(source);
    assert!(
        result.is_ok(),
        "Failed to parse context group alias: {:?}",
        result
    );

    let items = result.unwrap();
    assert_eq!(items.len(), 1);

    match &items[0].kind {
        ItemKind::ContextGroup(group) => {
            assert_eq!(group.name.name.as_str(), "ServerContext");
            assert_eq!(group.contexts.len(), 3);
        }
        _ => panic!("Expected ContextGroup, got {:?}", items[0].kind),
    }
}

#[test]
fn test_function_with_using_clause() {
    let source = r#"
fn process_data(data: List<u8>) -> Result<Text, Error>
    using [Database, Logger]
{
    Logger.info("Processing data");
    Database.save(data)?;
    Ok("Done")
}
"#;

    let result = parse_module(source);
    assert!(
        result.is_ok(),
        "Failed to parse function with using clause: {:?}",
        result
    );

    let items = result.unwrap();
    assert_eq!(items.len(), 1);

    match &items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.contexts.len(), 2);
            // Check that contexts were parsed correctly
            assert!(!func.contexts[0].path.segments.is_empty());
            assert!(!func.contexts[1].path.segments.is_empty());
        }
        _ => panic!("Expected Function, got {:?}", items[0].kind),
    }
}

#[test]
fn test_function_with_single_context_no_brackets() {
    let source = r#"
fn get_config(key: Text) -> Maybe<Text> using Config {
    Config.get(key)
}
"#;

    let result = parse_module(source);
    assert!(
        result.is_ok(),
        "Failed to parse function with single context: {:?}",
        result
    );

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.contexts.len(), 1);
        }
        _ => panic!("Expected Function, got {:?}", items[0].kind),
    }
}

#[test]
fn test_all_async_syntax_variations() {
    let sources = vec![
        // async fn
        r#"async fn fetch() -> Text { "data" }"#,
        // async context
        r#"async context DB { fn query(sql: Text) -> Rows; }"#,
        // context async (alternative)
        r#"context async DB { async fn query(sql: Text) -> Rows; }"#,
    ];

    for source in sources {
        let result = parse_module(source);
        assert!(
            result.is_ok(),
            "Failed to parse: {}\nError: {:?}",
            source,
            result
        );
    }
}

#[test]
fn test_complex_context_with_all_features() {
    let source = r#"
public context async Database<T> {
    fn connect() -> Connection using [Network];
    async fn query<R>(sql: Text) -> Result<R, Error>;
    fn execute(sql: Text) -> Result<Unit, Error>;
    async fn transaction<F>(body: F) -> Result<Unit, Error>;
}
"#;

    let result = parse_module(source);
    assert!(
        result.is_ok(),
        "Failed to parse complex context: {:?}",
        result
    );

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Context(ctx) => {
            assert_eq!(ctx.visibility, Visibility::Public);
            assert!(ctx.is_async);
            assert_eq!(ctx.generics.len(), 1);
            assert_eq!(ctx.methods.len(), 4);

            // First method uses Network context
            assert_eq!(ctx.methods[0].contexts.len(), 1);

            // Second method is async with generic
            assert!(ctx.methods[1].is_async);
            assert_eq!(ctx.methods[1].generics.len(), 1);
        }
        _ => panic!("Expected Context, got {:?}", items[0].kind),
    }
}

#[test]
fn test_multiple_contexts_in_module() {
    let source = r#"
context Logger {
    fn info(msg: Text);
    fn error(msg: Text);
}

context Database {
    fn query(sql: Text) -> Rows;
}

context group WebContext {
    Logger,
    Database
}

fn handler() using WebContext {
    Logger.info("Handler called");
    Database.query("SELECT * FROM users");
}
"#;

    let result = parse_module(source);
    assert!(
        result.is_ok(),
        "Failed to parse multiple contexts: {:?}",
        result
    );

    let items = result.unwrap();
    assert_eq!(items.len(), 4);

    // Check each item type
    assert!(matches!(items[0].kind, ItemKind::Context(_)));
    assert!(matches!(items[1].kind, ItemKind::Context(_)));
    assert!(matches!(items[2].kind, ItemKind::ContextGroup(_)));
    assert!(matches!(items[3].kind, ItemKind::Function(_)));
}

#[test]
fn test_context_with_visibility_modifiers() {
    let sources = vec![
        (
            r#"public context Logger { fn info(msg: Text); }"#,
            Visibility::Public,
        ),
        (
            r#"context Logger { fn info(msg: Text); }"#,
            Visibility::Private,
        ),
    ];

    for (source, expected_vis) in sources {
        let result = parse_module(source);
        assert!(result.is_ok(), "Failed to parse: {}", source);

        let items = result.unwrap();
        match &items[0].kind {
            ItemKind::Context(ctx) => {
                assert_eq!(ctx.visibility, expected_vis);
            }
            _ => panic!("Expected Context"),
        }
    }
}

#[test]
fn test_error_handling_invalid_context() {
    let invalid_sources = vec![
        // Missing name
        r#"context { fn test(); }"#,
        // Missing body
        r#"context Test"#,
        // Invalid method (missing semicolon)
        r#"context Test { fn method() }"#,
    ];

    for source in invalid_sources {
        let result = parse_module(source);
        assert!(result.is_err(), "Should fail to parse: {}", source);
    }
}

#[test]
fn test_provide_statement_parsing() {
    let source = r#"
fn main() {
    provide Database = PostgresDB.new();
    provide Logger = ConsoleLogger.new();

    run_app();
}
"#;

    let result = parse_module(source);
    assert!(
        result.is_ok(),
        "Failed to parse provide statements: {:?}",
        result
    );

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Function(func) => {
            if let Some(body) = &func.body {
                match body {
                    verum_ast::FunctionBody::Block(block) => {
                        // Should have 3 statements: 2 provides + 1 call
                        assert_eq!(block.stmts.len(), 3);

                        // Check first two are provide statements
                        assert!(matches!(block.stmts[0].kind, StmtKind::Provide { .. }));
                        assert!(matches!(block.stmts[1].kind, StmtKind::Provide { .. }));
                    }
                    _ => panic!("Expected block body"),
                }
            }
        }
        _ => panic!("Expected Function"),
    }
}

#[test]
fn test_real_world_example_from_spec() {
    // Context system pattern: context declaration, provide, using clause
    let source = r#"
context async Database {
    fn connect() -> Connection using [Network];
    async fn query(sql: Text) -> Result<Rows, Error>;
    async fn execute(sql: Text) -> Result<Unit, Error>;
}

context Logger {
    fn info(msg: Text);
    fn error(msg: Text);
}

context group ServerContext {
    Database,
    Logger
}

async fn handle_request(req: Request) -> Response
    using ServerContext
{
    Logger.info("Handling request");
    let users = Database.query("SELECT * FROM users").await?;
    Response.json(users)
}
"#;

    let result = parse_module(source);
    assert!(
        result.is_ok(),
        "Failed to parse real-world example: {:?}",
        result
    );

    let items = result.unwrap();
    assert_eq!(items.len(), 4);
}
