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
//! Comprehensive tests for the Verum context system.
//!
//! Tests cover:
//! - Context declarations (sync and async)
//! - Context groups
//! - Using clauses in functions
//! - Provide statements
//! - Context methods
//! - Sub-contexts
//! - Generic contexts
//!
//! Tests for the Verum Context System (capability-based DI, NOT algebraic effects)

use verum_ast::{FileId, ItemKind, Module, stmt::StmtKind, ty::PathSegment};
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

/// Helper to check if parsing succeeds.
fn assert_parses(source: &str) {
    parse_module(source).unwrap_or_else(|_| panic!("Failed to parse: {}", source));
}

/// Helper to parse and extract the first item.
fn parse_first_item(source: &str) -> ItemKind {
    let module = parse_module(source).unwrap_or_else(|_| panic!("Failed to parse: {}", source));
    module
        .items
        .into_iter()
        .next()
        .expect("Expected at least one item")
        .kind
}

// ============================================================================
// SECTION 1: BASIC CONTEXT DECLARATIONS
// ============================================================================

#[test]
fn test_basic_context_declaration() {
    // Context declaration: `context Name { fn method(); }` with optional async modifier
    let source = r#"
        context Database {
            fn query(sql: Text) -> Result<Rows>;
            fn execute(sql: Text) -> Result<Unit>;
        }
    "#;

    let item = parse_first_item(source);
    match item {
        ItemKind::Context(decl) => {
            assert_eq!(decl.name.name.as_str(), "Database");
            assert!(!decl.is_async, "Basic context should be sync");
            assert_eq!(decl.methods.len(), 2);
            assert_eq!(decl.methods[0].name.name.as_str(), "query");
            assert_eq!(decl.methods[1].name.name.as_str(), "execute");
        }
        _ => panic!("Expected Context item"),
    }
}

#[test]
fn test_async_context_declaration() {
    // Async context: `context async Database { async fn query(); }`
    let source = r#"
        async context Database {
            async fn query(sql: Text) -> Result<Rows>;
            async fn execute(sql: Text) -> Result<Unit>;
        }
    "#;

    let item = parse_first_item(source);
    match item {
        ItemKind::Context(decl) => {
            assert_eq!(decl.name.name.as_str(), "Database");
            assert!(decl.is_async, "Async context should have is_async = true");
            assert_eq!(decl.methods.len(), 2);
            assert!(decl.methods[0].is_async, "Methods should be async");
            assert!(decl.methods[1].is_async, "Methods should be async");
        }
        _ => panic!("Expected Context item"),
    }
}

#[test]
fn test_context_with_visibility() {
    assert_parses("public context Logger { fn log(msg: Text) -> Unit; }");
    assert_parses("internal context Cache { fn get(key: Text) -> Maybe<Text>; }");
    assert_parses("protected context Metrics { fn increment(name: Text) -> Unit; }");
}

#[test]
fn test_context_empty_methods() {
    // Empty context should be valid
    assert_parses("context EmptyContext {}");
}

#[test]
fn test_context_single_method() {
    let source = r#"
        context Logger {
            fn log(msg: Text) -> Unit;
        }
    "#;

    let item = parse_first_item(source);
    match item {
        ItemKind::Context(decl) => {
            assert_eq!(decl.methods.len(), 1);
            assert_eq!(decl.methods[0].name.name.as_str(), "log");
        }
        _ => panic!("Expected Context item"),
    }
}

#[test]
fn test_context_mixed_sync_async_methods() {
    // Context can have both sync and async methods
    let source = r#"
        context FileSystem {
            fn read_sync(path: Text) -> Result<Text>;
            async fn read_async(path: Text) -> Result<Text>;
        }
    "#;

    let item = parse_first_item(source);
    match item {
        ItemKind::Context(decl) => {
            assert_eq!(decl.methods.len(), 2);
            assert!(!decl.methods[0].is_async, "First method should be sync");
            assert!(decl.methods[1].is_async, "Second method should be async");
        }
        _ => panic!("Expected Context item"),
    }
}

// ============================================================================
// SECTION 2: GENERIC CONTEXTS
// ============================================================================

#[test]
fn test_generic_context_single_param() {
    let source = r#"
        context State<S> {
            fn get() -> S;
            fn set(value: S) -> Unit;
        }
    "#;

    let item = parse_first_item(source);
    match item {
        ItemKind::Context(decl) => {
            assert_eq!(decl.name.name.as_str(), "State");
            assert_eq!(decl.generics.len(), 1);
            assert_eq!(decl.methods.len(), 2);
        }
        _ => panic!("Expected Context item"),
    }
}

#[test]
fn test_generic_context_multiple_params() {
    let source = r#"
        context Cache<K, V> {
            fn get(key: K) -> Maybe<V>;
            fn set(key: K, value: V) -> Unit;
        }
    "#;

    let item = parse_first_item(source);
    match item {
        ItemKind::Context(decl) => {
            assert_eq!(decl.generics.len(), 2);
            assert_eq!(decl.methods.len(), 2);
        }
        _ => panic!("Expected Context item"),
    }
}

// ============================================================================
// SECTION 3: CONTEXT GROUPS
// ============================================================================

#[test]
fn test_context_group_basic() {
    // Context providing: `provide Ctx = impl;` installs dependency in task-local storage
    let source = r#"
        context group WebApp {
            Database,
            Logger,
            Cache
        }
    "#;

    let item = parse_first_item(source);
    match item {
        ItemKind::ContextGroup(decl) => {
            assert_eq!(decl.name.name.as_str(), "WebApp");
            assert_eq!(decl.contexts.len(), 3);
            assert_eq!(decl.contexts[0].path.to_string(), "Database");
            assert_eq!(decl.contexts[1].path.to_string(), "Logger");
            assert_eq!(decl.contexts[2].path.to_string(), "Cache");
        }
        _ => panic!("Expected ContextGroup item"),
    }
}

#[test]
fn test_context_group_single_context() {
    let source = r#"
        context group SingleContext {
            Database
        }
    "#;

    let item = parse_first_item(source);
    match item {
        ItemKind::ContextGroup(decl) => {
            assert_eq!(decl.contexts.len(), 1);
            assert_eq!(decl.contexts[0].path.to_string(), "Database");
        }
        _ => panic!("Expected ContextGroup item"),
    }
}

#[test]
fn test_context_group_empty() {
    let source = "context group EmptyGroup {}";

    let item = parse_first_item(source);
    match item {
        ItemKind::ContextGroup(decl) => {
            assert_eq!(decl.contexts.len(), 0);
        }
        _ => panic!("Expected ContextGroup item"),
    }
}

#[test]
fn test_context_group_with_visibility() {
    assert_parses("public context group WebApp { Database, Logger }");
    assert_parses("internal context group ServerContext { Database, Metrics }");
}

#[test]
fn test_context_group_trailing_comma() {
    let source = r#"
        context group WebApp {
            Database,
            Logger,
            Cache,
        }
    "#;

    assert_parses(source);
}

// ============================================================================
// SECTION 4: CONTEXT GROUP ALIASES (using Name = [...])
// ============================================================================

#[test]
fn test_context_group_alias_basic() {
    let source = "using WebContext = [Database, Logger];";

    let item = parse_first_item(source);
    match item {
        ItemKind::ContextGroup(decl) => {
            assert_eq!(decl.name.name.as_str(), "WebContext");
            assert_eq!(decl.contexts.len(), 2);
            assert_eq!(decl.contexts[0].path.to_string(), "Database");
            assert_eq!(decl.contexts[1].path.to_string(), "Logger");
        }
        _ => panic!("Expected ContextGroup item"),
    }
}

#[test]
fn test_context_group_alias_single() {
    let source = "using SimpleContext = [Database];";

    let item = parse_first_item(source);
    match item {
        ItemKind::ContextGroup(decl) => {
            assert_eq!(decl.contexts.len(), 1);
        }
        _ => panic!("Expected ContextGroup item"),
    }
}

#[test]
fn test_context_group_alias_empty() {
    let source = "using EmptyContext = [];";

    let item = parse_first_item(source);
    match item {
        ItemKind::ContextGroup(decl) => {
            assert_eq!(decl.contexts.len(), 0);
        }
        _ => panic!("Expected ContextGroup item"),
    }
}

#[test]
fn test_context_group_alias_many() {
    let source = "using ServerContext = [Database, Logger, Cache, Metrics];";

    let item = parse_first_item(source);
    match item {
        ItemKind::ContextGroup(decl) => {
            assert_eq!(decl.contexts.len(), 4);
        }
        _ => panic!("Expected ContextGroup item"),
    }
}

#[test]
fn test_context_group_alias_with_visibility() {
    assert_parses("public using WebContext = [Database, Logger];");
    assert_parses("internal using ServerContext = [Database];");
}

#[test]
fn test_context_group_alias_trailing_comma() {
    let source = "using WebContext = [Database, Logger,];";
    assert_parses(source);
}

#[test]
fn test_context_group_alias_vs_traditional() {
    // Both syntaxes should produce equivalent results
    let source1 = "using WebContext = [Database, Logger];";
    let source2 = "context group WebContext { Database, Logger }";

    let item1 = parse_first_item(source1);
    let item2 = parse_first_item(source2);

    match (item1, item2) {
        (ItemKind::ContextGroup(cg1), ItemKind::ContextGroup(cg2)) => {
            assert_eq!(cg1.name.name.as_str(), cg2.name.name.as_str());
            assert_eq!(cg1.contexts.len(), cg2.contexts.len());
            assert_eq!(cg1.contexts[0].path.to_string(), cg2.contexts[0].path.to_string());
            assert_eq!(cg1.contexts[1].path.to_string(), cg2.contexts[1].path.to_string());
        }
        _ => panic!("Expected ContextGroup items"),
    }
}

// ============================================================================
// SECTION 5: USING CLAUSES IN FUNCTIONS
// ============================================================================

#[test]
fn test_function_using_single_context_no_brackets() {
    // Single context shorthand: `using Database` (brackets optional for single context/group)
    let source = r#"
        fn query_user(id: Int) -> User using Database {
            Database.query("SELECT * FROM users WHERE id = ?", id)
        }
    "#;

    let item = parse_first_item(source);
    match item {
        ItemKind::Function(decl) => {
            assert_eq!(decl.contexts.len(), 1);
            match &decl.contexts[0].path.segments[0] {
                PathSegment::Name(ident) => {
                    assert_eq!(ident.name.as_str(), "Database");
                }
                _ => panic!("Expected Name segment"),
            }
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_function_using_single_context_with_brackets() {
    let source = r#"
        fn query_user(id: Int) -> User using [Database] {
            Database.query("SELECT * FROM users WHERE id = ?", id)
        }
    "#;

    let item = parse_first_item(source);
    match item {
        ItemKind::Function(decl) => {
            assert_eq!(decl.contexts.len(), 1);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_function_using_multiple_contexts_brackets_required() {
    // Multiple contexts: `using [Database, Logger]` (brackets required for multiple)
    let source = r#"
        fn complex_operation() -> Result<Data> using [Database, Logger, Cache] {
            Logger.log("Starting operation");
            let data = Database.query("...");
            Cache.set("key", data);
            Ok(data)
        }
    "#;

    let item = parse_first_item(source);
    match item {
        ItemKind::Function(decl) => {
            assert_eq!(decl.contexts.len(), 3);
            match &decl.contexts[0].path.segments[0] {
                PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "Database"),
                _ => panic!("Expected Name segment"),
            }
            match &decl.contexts[1].path.segments[0] {
                PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "Logger"),
                _ => panic!("Expected Name segment"),
            }
            match &decl.contexts[2].path.segments[0] {
                PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "Cache"),
                _ => panic!("Expected Name segment"),
            }
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_function_using_context_group() {
    let source = r#"
        fn handle_request(req: Request) -> Response using WebApp {
            // WebApp is a context group containing Database, Logger, Cache
            Logger.log("Handling request");
            let data = Database.query("...");
            Ok(Response.new(data))
        }
    "#;

    let item = parse_first_item(source);
    match item {
        ItemKind::Function(decl) => {
            assert_eq!(decl.contexts.len(), 1);
            match &decl.contexts[0].path.segments[0] {
                PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "WebApp"),
                _ => panic!("Expected Name segment"),
            }
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_function_using_generic_context() {
    let source = r#"
        fn get_state<S>() -> S using State<S> {
            State.get()
        }
    "#;

    let item = parse_first_item(source);
    match item {
        ItemKind::Function(decl) => {
            assert_eq!(decl.contexts.len(), 1);
            match &decl.contexts[0].path.segments[0] {
                PathSegment::Name(ident) => assert_eq!(ident.name.as_str(), "State"),
                _ => panic!("Expected Name segment"),
            }
            assert_eq!(decl.contexts[0].args.len(), 1);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_async_function_using_async_context() {
    // Tests async function with context requirements
    // .await keyword parsing is fully implemented and tested in await_parsing_tests.rs
    let source = r#"
        async fn fetch_user(id: Int) -> Result<User> using Database {
            Database.query("SELECT * FROM users WHERE id = ?", id)
        }
    "#;

    let item = parse_first_item(source);
    match item {
        ItemKind::Function(decl) => {
            assert!(decl.is_async, "Function should be async");
            assert_eq!(decl.contexts.len(), 1);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_function_using_trailing_comma() {
    let source = r#"
        fn foo() using [Database, Logger,] {
            42
        }
    "#;

    assert_parses(source);
}

// ============================================================================
// SECTION 6: PROVIDE STATEMENTS
// ============================================================================

#[test]
fn test_provide_statement_basic() {
    let source = r#"
        fn main() {
            provide Database = PostgresDatabase.new();
        }
    "#;

    // Parse the function
    let item = parse_first_item(source);
    match item {
        ItemKind::Function(decl) => {
            // Check that the function body contains a provide statement
            if let Some(body) = decl.body {
                match body {
                    verum_ast::decl::FunctionBody::Block(block) => {
                        assert_eq!(block.stmts.len(), 1);
                        match &block.stmts[0].kind {
                            StmtKind::Provide { context, .. } => {
                                assert_eq!(context.as_str(), "Database");
                            }
                            _ => panic!("Expected Provide statement"),
                        }
                    }
                    _ => panic!("Expected Block body"),
                }
            }
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_provide_statement_multiple() {
    let source = r#"
        fn setup() {
            provide Database = PostgresDatabase.new();
            provide Logger = ConsoleLogger.new();
            provide Cache = RedisCache.new();
        }
    "#;

    assert_parses(source);
}

#[test]
fn test_provide_statement_with_complex_expression() {
    let source = r#"
        fn setup() {
            provide Database = if is_production() {
                ProductionDB.new()
            } else {
                TestDB.new()
            };
        }
    "#;

    assert_parses(source);
}

// ============================================================================
// SECTION 7: CONTEXT METHODS WITH PARAMETERS
// ============================================================================

#[test]
fn test_context_method_no_params() {
    let source = r#"
        context Time {
            fn now() -> Timestamp;
        }
    "#;

    assert_parses(source);
}

#[test]
fn test_context_method_one_param() {
    let source = r#"
        context Logger {
            fn log(msg: Text) -> Unit;
        }
    "#;

    assert_parses(source);
}

#[test]
fn test_context_method_multiple_params() {
    let source = r#"
        context Database {
            fn query(sql: Text, params: List<Value>) -> Result<Rows>;
        }
    "#;

    assert_parses(source);
}

#[test]
fn test_context_method_generic() {
    let source = r#"
        context Serializer {
            fn serialize<T>(value: T) -> Result<Bytes>;
            fn deserialize<T>(bytes: Bytes) -> Result<T>;
        }
    "#;

    assert_parses(source);
}

#[test]
fn test_context_method_with_using_clause() {
    // Context methods can also require contexts
    let source = r#"
        context HighLevel {
            fn complex_operation(data: Data) -> Result<Output> using [Database, Logger];
        }
    "#;

    assert_parses(source);
}

// ============================================================================
// SECTION 8: COMPLETE EXAMPLES
// ============================================================================

#[test]
fn test_complete_web_app_example() {
    let source = r#"
        // Define contexts
        context Database {
            fn query(sql: Text) -> Result<Rows>;
            fn execute(sql: Text) -> Result<Unit>;
        }

        context Logger {
            fn log(msg: Text) -> Unit;
            fn error(msg: Text) -> Unit;
        }

        context Cache {
            fn get(key: Text) -> Maybe<Text>;
            fn set(key: Text, value: Text) -> Unit;
        }

        // Define context group
        context group WebApp {
            Database,
            Logger,
            Cache
        }

        // Function using context group
        fn handle_request(req: Request) -> Response using WebApp {
            Logger.log("Handling request");
            let cached = Cache.get(req.url);

            let data = match cached {
                Some(d) => d,
                None => {
                    let d = Database.query("SELECT * FROM data");
                    Cache.set(req.url, d);
                    d
                }
            };

            Response.new(data)
        }

        // Main function that provides contexts
        fn main() {
            provide Database = PostgresDatabase.new();
            provide Logger = ConsoleLogger.new();
            provide Cache = RedisCache.new();

            let response = handle_request(Request.new("/api/data"));
        }
    "#;

    assert_parses(source);
}

#[test]
fn test_complete_async_example() {
    // NOTE: await keyword parsing not yet implemented, using alternative syntax
    let source = r#"
        async context AsyncDatabase {
            async fn query(sql: Text) -> Result<Rows>;
            async fn execute(sql: Text) -> Result<Unit>;
        }

        async fn fetch_user(id: Int) -> Result<User> using AsyncDatabase {
            AsyncDatabase.query("SELECT * FROM users WHERE id = ?", id)
        }

        async fn main() {
            provide AsyncDatabase = AsyncPostgres.new();
            let user = fetch_user(42);
        }
    "#;

    assert_parses(source);
}

#[test]
fn test_complete_generic_context_example() {
    let source = r#"
        context State<S> {
            fn get() -> S;
            fn set(value: S) -> Unit;
            fn update(f: fn(S) -> S) -> Unit;
        }

        fn increment_counter() using State<Int> {
            let current = State.get();
            State.set(current + 1);
        }

        fn main() {
            provide State = MutableState.new(0);
            increment_counter();
            let value = State.get();
        }
    "#;

    assert_parses(source);
}

// ============================================================================
// SECTION 9: ERROR CASES (should fail to parse)
// ============================================================================

#[test]
fn test_error_using_at_top_level_without_alias() {
    // using at top-level without = syntax should fail
    let source = "using Database";

    assert!(
        parse_module(source).is_err(),
        "Expected error: 'using' at top-level requires alias syntax"
    );
}

#[test]
fn test_error_context_group_missing_group_keyword() {
    // "context Name" without "group" should be parsed as regular context
    // and fail because it expects { methods }
    let source = "context WebApp Database, Logger";

    assert!(
        parse_module(source).is_err(),
        "Expected error: invalid context declaration"
    );
}
