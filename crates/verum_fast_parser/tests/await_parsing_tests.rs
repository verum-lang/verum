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
//! Comprehensive tests for .await postfix operator parsing
//!
//! This test suite verifies that the `.await` postfix operator is properly
//! implemented according to the grammar specification in grammar/verum.ebnf:594
//!
//! postfix_op = '.' , identifier , [ call_args ]
//!            | '?.' , identifier , [ call_args ]
//!            | '.' , integer_lit
//!            | '.' , 'await'      // <-- THIS IS TESTED HERE
//!            | '[' , expression , ']'
//!            | call_args
//!            | '?'
//!            | 'as' , type_expr ;

use verum_ast::{Expr, ExprKind, FileId};
use verum_fast_parser::VerumParser;

fn parse_expr(source: &str) -> Result<Expr, String> {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_expr_str(source, file_id)
        .map_err(|e| format!("{:?}", e))
}

#[test]
fn test_simple_await() {
    let expr = parse_expr("future.await").unwrap();
    assert!(
        matches!(expr.kind, ExprKind::Await(_)),
        "Expected Await expression, got {:?}",
        expr.kind
    );
}

#[test]
fn test_await_on_function_call() {
    let expr = parse_expr("async_fn().await").unwrap();
    assert!(
        matches!(expr.kind, ExprKind::Await(_)),
        "Expected Await expression, got {:?}",
        expr.kind
    );
}

#[test]
fn test_await_on_method_call() {
    let expr = parse_expr("obj.method().await").unwrap();
    assert!(
        matches!(expr.kind, ExprKind::Await(_)),
        "Expected Await expression, got {:?}",
        expr.kind
    );
}

#[test]
fn test_chained_await() {
    let expr = parse_expr("first().await.then().await").unwrap();
    // Outer is Await
    assert!(
        matches!(expr.kind, ExprKind::Await(_)),
        "Expected outer Await expression, got {:?}",
        expr.kind
    );
}

#[test]
fn test_await_with_question_mark() {
    let expr = parse_expr("async_call().await?").unwrap();
    // Should parse as Try(Await(...))
    match expr.kind {
        ExprKind::Try(inner) => {
            assert!(
                matches!(inner.kind, ExprKind::Await(_)),
                "Expected Await inside Try, got {:?}",
                inner.kind
            );
        }
        _ => panic!("Expected Try expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_await_in_binary_expression() {
    let expr = parse_expr("a.await + b.await").unwrap();
    // Should parse as Binary(Await(a), Add, Await(b))
    match expr.kind {
        ExprKind::Binary { left, right, .. } => {
            assert!(
                matches!(left.kind, ExprKind::Await(_)),
                "Expected left to be Await, got {:?}",
                left.kind
            );
            assert!(
                matches!(right.kind, ExprKind::Await(_)),
                "Expected right to be Await, got {:?}",
                right.kind
            );
        }
        _ => panic!("Expected Binary expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_await_in_assignment() {
    let expr = parse_expr("result = fetch().await").unwrap();
    match expr.kind {
        ExprKind::Binary { right, .. } => {
            assert!(
                matches!(right.kind, ExprKind::Await(_)),
                "Expected right side to be Await, got {:?}",
                right.kind
            );
        }
        _ => panic!(
            "Expected Binary (assignment) expression, got {:?}",
            expr.kind
        ),
    }
}

#[test]
fn test_await_with_field_access() {
    let expr = parse_expr("response.await.data").unwrap();
    // Should parse as Field(Await(response), "data")
    match expr.kind {
        ExprKind::Field { expr: inner, .. } => {
            assert!(
                matches!(inner.kind, ExprKind::Await(_)),
                "Expected Field on Await, got {:?}",
                inner.kind
            );
        }
        _ => panic!("Expected Field expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_await_with_index() {
    let expr = parse_expr("array_future.await[0]").unwrap();
    // Should parse as Index(Await(array_future), 0)
    match expr.kind {
        ExprKind::Index { expr: inner, .. } => {
            assert!(
                matches!(inner.kind, ExprKind::Await(_)),
                "Expected Index on Await, got {:?}",
                inner.kind
            );
        }
        _ => panic!("Expected Index expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_await_with_cast() {
    let expr = parse_expr("future.await as Int").unwrap();
    // Should parse as Cast(Await(future), Int)
    match expr.kind {
        ExprKind::Cast { expr: inner, .. } => {
            assert!(
                matches!(inner.kind, ExprKind::Await(_)),
                "Expected Cast on Await, got {:?}",
                inner.kind
            );
        }
        _ => panic!("Expected Cast expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_await_in_match() {
    let source = r#"
        match fetch().await {
            Some(x) => x,
            None => 0
        }
    "#;
    let expr = parse_expr(source).unwrap();
    match expr.kind {
        ExprKind::Match {
            expr: scrutinee, ..
        } => {
            assert!(
                matches!(scrutinee.kind, ExprKind::Await(_)),
                "Expected Match on Await, got {:?}",
                scrutinee.kind
            );
        }
        _ => panic!("Expected Match expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_await_in_if_condition() {
    let source = r#"
        if check().await {
            1
        } else {
            0
        }
    "#;
    let expr = parse_expr(source).unwrap();
    assert!(
        matches!(expr.kind, ExprKind::If { .. }),
        "Expected If expression, got {:?}",
        expr.kind
    );
}

#[test]
fn test_await_in_let_binding() {
    let source = r#"{
        let x = fetch().await;
        x
    }"#;
    let expr = parse_expr(source).unwrap();
    assert!(
        matches!(expr.kind, ExprKind::Block(_)),
        "Expected Block expression with let statement"
    );
}

#[test]
fn test_multiple_postfix_operators_with_await() {
    // Test: obj.method1().await.method2().await
    let expr = parse_expr("obj.method1().await.method2().await").unwrap();
    assert!(
        matches!(expr.kind, ExprKind::Await(_)),
        "Expected outermost Await, got {:?}",
        expr.kind
    );
}

#[test]
fn test_await_with_pipeline() {
    let expr = parse_expr("fetch() |> process.await").unwrap();
    match expr.kind {
        ExprKind::Pipeline { right, .. } => {
            assert!(
                matches!(right.kind, ExprKind::Await(_)),
                "Expected Pipeline with Await on right, got {:?}",
                right.kind
            );
        }
        _ => panic!("Expected Pipeline expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_complex_async_expression() {
    let source = r#"
        async fn fetch_and_process(url: Text) -> Result<Int> using [IO] {
            let data = http_get(url).await?;
            let parsed = parse_json(data).await?;
            let result = process(parsed).await?;
            Ok(result)
        }
    "#;

    use verum_lexer::Lexer;
    use verum_fast_parser::VerumParser;
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse async function with multiple awaits: {:?}",
        result.err()
    );
}

#[test]
fn test_await_preserves_span_info() {
    let expr = parse_expr("future.await").unwrap();
    // Just verify the expression has valid span information
    assert!(expr.span.start < expr.span.end, "Span should be valid");
}
