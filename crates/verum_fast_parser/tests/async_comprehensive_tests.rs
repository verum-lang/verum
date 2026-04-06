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
//! Comprehensive async system tests for Verum parser
//!
//! Tests cover:
//! - Async function declarations
//! - Async blocks
//! - Await expressions (simple, chained, nested)
//! - Spawn expressions with context requirements
//! - Select expressions (basic, biased, with default)
//! - Nursery structured concurrency blocks
//! - Async closures
//! - Async generator functions (fn*)
//! - Channel operations in async context

use verum_ast::{Expr, ExprKind, FileId, ItemKind, Module};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

fn parse_expr(source: &str) -> Expr {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_expr_str(source, file_id)
        .unwrap_or_else(|e| panic!("Failed to parse expr '{}': {:?}", source, e))
}

fn parse_module(source: &str) -> Module {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser
        .parse_module(lexer, file_id)
        .unwrap_or_else(|e| panic!("Failed to parse module: {:?}", e))
}

fn parse_first_item(source: &str) -> ItemKind {
    let module = parse_module(source);
    module
        .items
        .into_iter()
        .next()
        .expect("Expected at least one item")
        .kind
}

fn assert_parses(source: &str) {
    parse_module(source);
}

// ============================================================================
// ASYNC FUNCTION DECLARATIONS
// ============================================================================

#[test]
fn test_async_fn_declaration_basic() {
    let source = r#"
        async fn fetch_data() -> Text {
            let response = http_get("https://example.com").await;
            response.body
        }
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Function(decl) => {
            assert!(decl.is_async, "Function should be async");
            assert_eq!(decl.name.name.as_str(), "fetch_data");
        }
        _ => panic!("Expected Function item, got {:?}", item),
    }
}

#[test]
fn test_async_fn_with_params() {
    let source = r#"
        async fn fetch_user(id: Int, timeout: Int) -> Result<User, Error> {
            let user = db_query(id).await;
            user
        }
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Function(decl) => {
            assert!(decl.is_async);
            assert_eq!(decl.params.len(), 2);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_async_fn_with_using_clause() {
    let source = r#"
        async fn query_users() -> List<User> using [Database, Logger] {
            Logger.log("Querying users");
            Database.query("SELECT * FROM users").await
        }
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Function(decl) => {
            assert!(decl.is_async);
            assert_eq!(decl.contexts.len(), 2);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_async_fn_no_return_type() {
    let source = r#"
        async fn fire_and_forget() {
            send_notification().await;
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_public_async_fn() {
    let source = r#"
        public async fn api_handler(req: Request) -> Response {
            process(req).await
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// ASYNC BLOCKS
// ============================================================================

#[test]
fn test_async_block_basic() {
    let expr = parse_expr("async { compute_value() }");
    assert!(
        matches!(expr.kind, ExprKind::Async(_)),
        "Expected Async block, got {:?}",
        expr.kind
    );
}

#[test]
fn test_async_block_with_await() {
    let source = r#"
        fn main() {
            let future = async {
                let data = fetch().await;
                process(data)
            };
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_async_block_in_let_binding() {
    let source = r#"
        fn main() {
            let task = async { 42 };
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// SPAWN EXPRESSIONS
// ============================================================================

#[test]
fn test_spawn_basic() {
    let expr = parse_expr("spawn { compute() }");
    assert!(
        matches!(expr.kind, ExprKind::Spawn { .. }),
        "Expected Spawn expression, got {:?}",
        expr.kind
    );
}

#[test]
fn test_spawn_simple_expr() {
    let expr = parse_expr("spawn compute()");
    assert!(
        matches!(expr.kind, ExprKind::Spawn { .. }),
        "Expected Spawn expression, got {:?}",
        expr.kind
    );
}

#[test]
fn test_spawn_with_using_contexts() {
    // Spawn with context requirements
    let source = r#"
        fn main() using [Database, Logger] {
            let handle = spawn { process() };
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_spawn_await_chain() {
    // spawn { ... }.await parses as Spawn(Await(...)) -- the await is inside
    // Test that the overall expression parses
    let expr = parse_expr("spawn { fetch().await }");
    assert!(
        matches!(expr.kind, ExprKind::Spawn { .. }),
        "Expected Spawn expression, got {:?}",
        expr.kind
    );
}

#[test]
fn test_spawn_in_function() {
    let source = r#"
        async fn parallel_fetch() -> (Data, Data) {
            let a = spawn fetch_a();
            let b = spawn fetch_b();
            (a.await, b.await)
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// SELECT EXPRESSIONS
// ============================================================================

#[test]
fn test_select_basic() {
    let source = r#"
        fn main() {
            let result = select {
                data = channel_a.recv().await => process(data),
                msg = channel_b.recv().await => handle(msg),
            };
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_select_biased() {
    let source = r#"
        fn main() {
            let result = select biased {
                data = priority_channel.recv().await => data,
                msg = normal_channel.recv().await => msg,
            };
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_select_with_default() {
    let source = r#"
        fn main() {
            let result = select {
                data = channel.recv().await => process(data),
                default => fallback_value(),
            };
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_select_single_arm() {
    let source = r#"
        fn main() {
            let result = select {
                value = future.await => value,
            };
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// NURSERY (STRUCTURED CONCURRENCY)
// ============================================================================

#[test]
fn test_nursery_basic() {
    let source = r#"
        fn main() {
            nursery {
                let a = spawn fetch_a();
                let b = spawn fetch_b();
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_nursery_with_on_cancel() {
    let source = r#"
        fn main() {
            nursery {
                let result = spawn long_task();
            } on_cancel {
                cleanup();
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_nursery_with_recover() {
    let source = r#"
        fn main() {
            nursery {
                let a = spawn risky_task();
            } recover {
                TimeoutError => default_value(),
                _ => panic("unexpected"),
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_nursery_with_timeout_option() {
    let source = r#"
        fn main() {
            nursery(timeout: 5000) {
                let result = spawn fetch_data();
            }
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// ASYNC CLOSURES
// ============================================================================

#[test]
fn test_async_closure_basic() {
    let expr = parse_expr("async |x| x + 1");
    match &expr.kind {
        ExprKind::Closure { async_, .. } => {
            assert!(*async_, "Closure should be async");
        }
        _ => panic!("Expected Closure expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_async_closure_with_block() {
    let expr = parse_expr("async |x, y| { let sum = x + y; sum }");
    match &expr.kind {
        ExprKind::Closure { async_, params, .. } => {
            assert!(*async_);
            assert_eq!(params.len(), 2);
        }
        _ => panic!("Expected Closure expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_async_move_closure() {
    let expr = parse_expr("async move |x| x * 2");
    match &expr.kind {
        ExprKind::Closure {
            async_, move_, ..
        } => {
            assert!(*async_, "Should be async");
            assert!(*move_, "Should be move");
        }
        _ => panic!("Expected Closure expression, got {:?}", expr.kind),
    }
}

// ============================================================================
// ASYNC GENERATOR FUNCTIONS (fn*)
// ============================================================================

#[test]
fn test_async_generator_fn() {
    let source = r#"
        async fn* stream_data(url: Text) -> Int {
            yield 1;
            yield 2;
            yield 3;
        }
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Function(decl) => {
            assert!(decl.is_async, "Should be async");
            assert!(decl.is_generator, "Should be generator");
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_generator_fn_basic() {
    let source = r#"
        fn* range(start: Int, end: Int) -> Int {
            let i = start;
            while i < end {
                yield i;
                i = i + 1;
            }
        }
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Function(decl) => {
            assert!(!decl.is_async, "Should not be async");
            assert!(decl.is_generator, "Should be generator");
        }
        _ => panic!("Expected Function item"),
    }
}

// ============================================================================
// COMPLEX ASYNC PATTERNS
// ============================================================================

#[test]
fn test_multiple_await_in_sequence() {
    let source = r#"
        async fn pipeline() -> Result<Data, Error> {
            let raw = fetch_raw().await;
            let parsed = parse(raw).await;
            let validated = validate(parsed).await;
            validated
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_await_in_match_arms() {
    let source = r#"
        async fn handle_event(event: Event) -> Response {
            match event {
                Event.Click(pos) => handle_click(pos).await,
                Event.Key(key) => handle_key(key).await,
                _ => default_handler().await,
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_await_in_if_expression() {
    let source = r#"
        async fn conditional_fetch(use_cache: Bool) -> Data {
            if use_cache {
                cache_get("key").await
            } else {
                remote_fetch("url").await
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_nested_async_blocks() {
    let source = r#"
        fn main() {
            let outer = async {
                let inner = async {
                    42
                };
                inner.await + 1
            };
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_async_fn_with_generics() {
    let source = r#"
        async fn fetch_typed<T>(url: Text) -> Result<T, Error> {
            let response = http_get(url).await;
            deserialize(response.body)
        }
    "#;
    assert_parses(source);
}
