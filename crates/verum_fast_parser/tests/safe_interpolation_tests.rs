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
//! Tests for safe interpolated strings parsing.
//!
//! Tests for safe interpolated string parsing: sql"{}", html"{}", json"{}", url"{}"
//!
//! Safe interpolated strings provide compile-time safety for different domains:
//! - sql"..." - SQL-safe parameterization
//! - html"..." - HTML auto-escaping
//! - json"..." - JSON-safe encoding
//! - url"..." - URL encoding
//! - gql"..." - GraphQL safe queries
//! - xml"..." - XML escaping
//! - yaml"..." - YAML escaping
//! - email"..." - Email-safe formatting
//! - rx"..." - Regex-safe interpolation
//! - contract"..." - Contract literal (separate token)

use verum_ast::{Expr, ExprKind, FileId, Item, ItemKind};
use verum_common::List;
use verum_lexer::Lexer;
use verum_fast_parser::{ParseError, VerumParser};

/// Helper to parse a module from source.
fn parse(source: &str) -> Result<List<Item>, List<ParseError>> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).map(|m| m.items)
}

/// Helper to extract the first expression from a parsed function
fn get_first_expr(items: &List<Item>) -> Option<&Expr> {
    items.first().and_then(|item| {
        if let ItemKind::Function(func) = &item.kind {
            func.body.as_ref().and_then(|body| match body {
                verum_ast::decl::FunctionBody::Block(block) => {
                    block.stmts.first().and_then(|stmt| {
                        if let verum_ast::stmt::StmtKind::Let {
                            pattern: _,
                            ty: _,
                            value,
                        } = &stmt.kind
                        {
                            value.as_ref()
                        } else {
                            None
                        }
                    })
                }
                verum_ast::decl::FunctionBody::Expr(_) => None,
            })
        } else {
            None
        }
    })
}

// ============================================================================
// SQL Safe Interpolation Tests
// ============================================================================

#[test]
fn test_sql_simple_interpolation() {
    let source = r#"
        fn test() {
            let query = sql"SELECT * FROM users WHERE id = {user_id}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    let expr = get_first_expr(&items).expect("should have expression");

    match &expr.kind {
        ExprKind::InterpolatedString {
            handler,
            parts,
            exprs,
        } => {
            assert_eq!(handler.as_str(), "sql");
            assert_eq!(parts.len(), 2);
            assert_eq!(exprs.len(), 1);
            assert_eq!(parts[0].as_str(), "SELECT * FROM users WHERE id = ");
            assert_eq!(parts[1].as_str(), "");
        }
        _ => panic!("Expected InterpolatedString, got {:?}", expr.kind),
    }
}

#[test]
fn test_sql_multiple_interpolations() {
    let source = r#"
        fn test() {
            let query = sql"SELECT * FROM users WHERE age BETWEEN {min_age} AND {max_age}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    let expr = get_first_expr(&items).expect("should have expression");

    match &expr.kind {
        ExprKind::InterpolatedString {
            handler,
            parts,
            exprs,
        } => {
            assert_eq!(handler.as_str(), "sql");
            assert_eq!(parts.len(), 3);
            assert_eq!(exprs.len(), 2);
            assert_eq!(parts[0].as_str(), "SELECT * FROM users WHERE age BETWEEN ");
            assert_eq!(parts[1].as_str(), " AND ");
            assert_eq!(parts[2].as_str(), "");
        }
        _ => panic!("Expected InterpolatedString"),
    }
}

#[test]
fn test_sql_no_interpolation() {
    let source = r#"
        fn test() {
            let query = sql"SELECT * FROM users";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    let expr = get_first_expr(&items).expect("should have expression");

    match &expr.kind {
        ExprKind::InterpolatedString {
            handler,
            parts,
            exprs,
        } => {
            assert_eq!(handler.as_str(), "sql");
            assert_eq!(parts.len(), 1);
            assert_eq!(exprs.len(), 0);
            assert_eq!(parts[0].as_str(), "SELECT * FROM users");
        }
        _ => panic!("Expected InterpolatedString"),
    }
}

// ============================================================================
// HTML Safe Interpolation Tests
// ============================================================================

#[test]
fn test_html_simple_interpolation() {
    let source = r#"
        fn test() {
            let html = html"<h1>{title}</h1>";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    let expr = get_first_expr(&items).expect("should have expression");

    match &expr.kind {
        ExprKind::InterpolatedString {
            handler,
            parts,
            exprs,
        } => {
            assert_eq!(handler.as_str(), "html");
            assert_eq!(parts.len(), 2);
            assert_eq!(exprs.len(), 1);
            assert_eq!(parts[0].as_str(), "<h1>");
            assert_eq!(parts[1].as_str(), "</h1>");
        }
        _ => panic!("Expected InterpolatedString"),
    }
}

#[test]
fn test_html_multiple_tags() {
    let source = r#"
        fn test() {
            let html = html"<div>{content}</div><p>{description}</p>";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    let expr = get_first_expr(&items).expect("should have expression");

    match &expr.kind {
        ExprKind::InterpolatedString {
            handler,
            parts,
            exprs,
        } => {
            assert_eq!(handler.as_str(), "html");
            assert_eq!(parts.len(), 3);
            assert_eq!(exprs.len(), 2);
        }
        _ => panic!("Expected InterpolatedString"),
    }
}

// ============================================================================
// JSON Safe Interpolation Tests
// ============================================================================

#[test]
fn test_json_simple_interpolation() {
    let source = r#"
        fn test() {
            let json = json"{{name: {name}}}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    let expr = get_first_expr(&items).expect("should have expression");

    match &expr.kind {
        ExprKind::InterpolatedString {
            handler,
            parts,
            exprs,
        } => {
            assert_eq!(handler.as_str(), "json");
            assert_eq!(parts.len(), 2);
            assert_eq!(exprs.len(), 1);
            // {{ and }} become { and } after parsing (escaped braces)
            assert_eq!(parts[0].as_str(), "{name: ");
            assert_eq!(parts[1].as_str(), "}");
        }
        _ => panic!("Expected InterpolatedString"),
    }
}

// ============================================================================
// URL/URI Safe Interpolation Tests
// ============================================================================

#[test]
fn test_url_simple_interpolation() {
    let source = r#"
        fn test() {
            let url = url"https://api.example.com/users?name={user_name}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    let expr = get_first_expr(&items).expect("should have expression");

    match &expr.kind {
        ExprKind::InterpolatedString {
            handler,
            parts,
            exprs,
        } => {
            assert_eq!(handler.as_str(), "url");
            assert_eq!(parts.len(), 2);
            assert_eq!(exprs.len(), 1);
        }
        _ => panic!("Expected InterpolatedString"),
    }
}

// ============================================================================
// GraphQL Safe Interpolation Tests
// ============================================================================

#[test]
fn test_gql_simple_interpolation() {
    let source = r#"
        fn test() {
            let query = gql"query {{ user(id: {user_id}) {{ name }} }}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    let expr = get_first_expr(&items).expect("should have expression");

    match &expr.kind {
        ExprKind::InterpolatedString {
            handler,
            parts,
            exprs,
        } => {
            assert_eq!(handler.as_str(), "gql");
            assert_eq!(parts.len(), 2);
            assert_eq!(exprs.len(), 1);
        }
        _ => panic!("Expected InterpolatedString"),
    }
}

// ============================================================================
// Format String Tests
// ============================================================================

#[test]
fn test_format_string_simple() {
    let source = r#"
        fn test() {
            let msg = f"Hello, {name}!";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    let expr = get_first_expr(&items).expect("should have expression");

    match &expr.kind {
        ExprKind::InterpolatedString {
            handler,
            parts,
            exprs,
        } => {
            assert_eq!(handler.as_str(), "f");
            assert_eq!(parts.len(), 2);
            assert_eq!(exprs.len(), 1);
            assert_eq!(parts[0].as_str(), "Hello, ");
            assert_eq!(parts[1].as_str(), "!");
        }
        _ => panic!("Expected InterpolatedString"),
    }
}

#[test]
fn test_format_string_with_expressions() {
    let source = r#"
        fn test() {
            let msg = f"x={x}, y={y}, sum={x + y}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    let expr = get_first_expr(&items).expect("should have expression");

    match &expr.kind {
        ExprKind::InterpolatedString {
            handler,
            parts,
            exprs,
        } => {
            assert_eq!(handler.as_str(), "f");
            assert_eq!(parts.len(), 4);
            assert_eq!(exprs.len(), 3);
            assert_eq!(parts[0].as_str(), "x=");
            assert_eq!(parts[1].as_str(), ", y=");
            assert_eq!(parts[2].as_str(), ", sum=");
            assert_eq!(parts[3].as_str(), "");
        }
        _ => panic!("Expected InterpolatedString"),
    }
}

// ============================================================================
// Edge Cases and Error Handling
// ============================================================================

#[test]
fn test_escaped_braces() {
    let source = r#"
        fn test() {
            let msg = f"{{literal}} {var}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    let expr = get_first_expr(&items).expect("should have expression");

    match &expr.kind {
        ExprKind::InterpolatedString {
            handler,
            parts,
            exprs,
        } => {
            assert_eq!(handler.as_str(), "f");
            assert_eq!(parts.len(), 2);
            assert_eq!(exprs.len(), 1);
            assert_eq!(parts[0].as_str(), "{literal} ");
            assert_eq!(parts[1].as_str(), "");
        }
        _ => panic!("Expected InterpolatedString"),
    }
}

#[test]
fn test_nested_braces_in_expression() {
    let source = r#"
        fn test() {
            let msg = f"Data: {data.get(key).unwrap_or(default)}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    let expr = get_first_expr(&items).expect("should have expression");

    match &expr.kind {
        ExprKind::InterpolatedString {
            handler,
            parts,
            exprs,
        } => {
            assert_eq!(handler.as_str(), "f");
            assert_eq!(parts.len(), 2);
            assert_eq!(exprs.len(), 1);
            assert_eq!(parts[0].as_str(), "Data: ");
            assert_eq!(parts[1].as_str(), "");
        }
        _ => panic!("Expected InterpolatedString"),
    }
}

#[test]
fn test_empty_interpolation_string() {
    let source = r#"
        fn test() {
            let empty = sql"";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    let expr = get_first_expr(&items).expect("should have expression");

    match &expr.kind {
        ExprKind::InterpolatedString {
            handler,
            parts,
            exprs,
        } => {
            assert_eq!(handler.as_str(), "sql");
            assert_eq!(parts.len(), 1);
            assert_eq!(exprs.len(), 0);
            assert_eq!(parts[0].as_str(), "");
        }
        _ => panic!("Expected InterpolatedString"),
    }
}

// ============================================================================
// All Semantic Tags
// ============================================================================

#[test]
fn test_all_semantic_tags() {
    // Tags supported by the lexer (from token.rs line 523)
    let tags = vec![
        "sql", "html", "json", "xml", "yaml", "url", "uri", "gql", "rx", "sh", "f",
    ];

    for tag in tags {
        let source = format!(
            r#"
            fn test() {{
                let str = {}"test content {{var}}";
            }}
            "#,
            tag
        );

        let items = parse(&source).unwrap_or_else(|_| panic!("parsing failed for tag: {}", tag));
        let expr = get_first_expr(&items).expect("should have expression");

        match &expr.kind {
            ExprKind::InterpolatedString {
                handler,
                parts,
                exprs,
            } => {
                assert_eq!(handler.as_str(), tag, "handler mismatch for tag: {}", tag);
                assert_eq!(parts.len(), 2, "parts length mismatch for tag: {}", tag);
                assert_eq!(exprs.len(), 1, "exprs length mismatch for tag: {}", tag);
            }
            _ => panic!("Expected InterpolatedString for tag: {}", tag),
        }
    }
}

// ============================================================================
// Complex Real-World Examples
// ============================================================================

#[test]
fn test_multiline_sql_query() {
    let source = r#"
        fn test() {
            let query = sql"
                SELECT u.id, u.name, COUNT(*) as order_count
                FROM users u
                JOIN orders o ON u.id = o.user_id
                WHERE u.status = {status}
                GROUP BY u.id, u.name
                HAVING COUNT(*) > {min_orders}
            ";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    let expr = get_first_expr(&items).expect("should have expression");

    match &expr.kind {
        ExprKind::InterpolatedString {
            handler,
            parts,
            exprs,
        } => {
            assert_eq!(handler.as_str(), "sql");
            assert_eq!(exprs.len(), 2);
            assert_eq!(parts.len(), 3);
        }
        _ => panic!("Expected InterpolatedString"),
    }
}

#[test]
fn test_complex_html_template() {
    let source = r#"
        fn test() {
            let html = html"
                <html>
                    <head><title>{title}</title></head>
                    <body>
                        <h1>{heading}</h1>
                        <p>{content}</p>
                        <footer>{footer}</footer>
                    </body>
                </html>
            ";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    let expr = get_first_expr(&items).expect("should have expression");

    match &expr.kind {
        ExprKind::InterpolatedString {
            handler,
            parts,
            exprs,
        } => {
            assert_eq!(handler.as_str(), "html");
            assert_eq!(exprs.len(), 4);
            assert_eq!(parts.len(), 5);
        }
        _ => panic!("Expected InterpolatedString"),
    }
}
