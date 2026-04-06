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
//
// Safe interpolation handlers: sql"...{expr}..." uses parameterized $1,$2 (prevents injection),
// html"...{expr}..." auto-escapes content (prevents XSS), url"...{expr}..." URL-encodes params.
// Desugars to meta-system calls that generate safe parameterized queries/templates.

use verum_ast::expr::Expr;
use verum_ast::literal::Literal;
use verum_ast::{FileId, Span};
use verum_compiler::interpolation::sql::SqlInterpolationHandler;
use verum_common::Text;

fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

fn dummy_expr() -> Expr {
    Expr::literal(Literal::int(0, test_span()))
}

// Basic Interpolation Tests

#[test]
fn test_sql_no_interpolation() {
    let template = Text::from("SELECT * FROM users");
    let exprs = vec![];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_ok());
    let query = result.unwrap();
    assert_eq!(query.template.as_str(), "SELECT * FROM users");
    assert_eq!(query.params.len(), 0);
}

#[test]
fn test_sql_single_interpolation() {
    let template = Text::from("SELECT * FROM users WHERE id = {user_id}");
    let exprs = vec![dummy_expr()];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_ok());
    let query = result.unwrap();
    assert_eq!(query.template.as_str(), "SELECT * FROM users WHERE id = ?");
    assert_eq!(query.params.len(), 1);
}

#[test]
fn test_sql_multiple_interpolations() {
    let template = Text::from("SELECT * FROM users WHERE id = {id} AND name = {name}");
    let exprs = vec![dummy_expr(), dummy_expr()];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_ok());
    let query = result.unwrap();
    assert_eq!(
        query.template.as_str(),
        "SELECT * FROM users WHERE id = ? AND name = ?"
    );
    assert_eq!(query.params.len(), 2);
}

#[test]
fn test_sql_three_interpolations() {
    let template =
        Text::from("SELECT * FROM users WHERE id = {id} AND name = {name} AND email = {email}");
    let exprs = vec![dummy_expr(), dummy_expr(), dummy_expr()];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_ok());
    let query = result.unwrap();
    assert_eq!(
        query.template.as_str(),
        "SELECT * FROM users WHERE id = ? AND name = ? AND email = ?"
    );
    assert_eq!(query.params.len(), 3);
}

// Escaped Braces Tests

#[test]
fn test_sql_escaped_braces() {
    let template = Text::from("SELECT '{{literal}}' WHERE id = {id}");
    let exprs = vec![dummy_expr()];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_ok());
    let query = result.unwrap();
    assert_eq!(query.template.as_str(), "SELECT '{literal}' WHERE id = ?");
    assert_eq!(query.params.len(), 1);
}

#[test]
fn test_sql_multiple_escaped_braces() {
    let template = Text::from("SELECT '{{a}}' WHERE id = {id} AND name = '{{b}}'");
    let exprs = vec![dummy_expr()];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_ok());
    let query = result.unwrap();
    assert_eq!(
        query.template.as_str(),
        "SELECT '{a}' WHERE id = ? AND name = '{b}'"
    );
}

#[test]
fn test_sql_escaped_closing_brace() {
    let template = Text::from("SELECT '}}' WHERE id = {id}");
    let exprs = vec![dummy_expr()];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_ok());
    let query = result.unwrap();
    assert_eq!(query.template.as_str(), "SELECT '}' WHERE id = ?");
}

// Error Cases

#[test]
fn test_sql_unclosed_interpolation() {
    let template = Text::from("SELECT * FROM users WHERE id = {user_id");
    let exprs = vec![dummy_expr()];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_err());
}

#[test]
fn test_sql_wrong_number_of_expressions_too_few() {
    let template = Text::from("SELECT * FROM users WHERE id = {id} AND name = {name}");
    let exprs = vec![dummy_expr()]; // Only 1 expression, but template needs 2
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_err());
}

#[test]
fn test_sql_wrong_number_of_expressions_too_many() {
    let template = Text::from("SELECT * FROM users WHERE id = {id}");
    let exprs = vec![dummy_expr(), dummy_expr()]; // 2 expressions, but template only needs 1
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_err());
}

#[test]
fn test_sql_unexpected_closing_brace() {
    let template = Text::from("SELECT * FROM users WHERE id = }");
    let exprs = vec![];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_err());
}

// Complex Query Tests

#[test]
fn test_sql_insert_query() {
    let template = Text::from("INSERT INTO users (id, name, email) VALUES ({id}, {name}, {email})");
    let exprs = vec![dummy_expr(), dummy_expr(), dummy_expr()];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_ok());
    let query = result.unwrap();
    assert_eq!(
        query.template.as_str(),
        "INSERT INTO users (id, name, email) VALUES (?, ?, ?)"
    );
}

#[test]
fn test_sql_update_query() {
    let template = Text::from("UPDATE users SET name = {name}, email = {email} WHERE id = {id}");
    let exprs = vec![dummy_expr(), dummy_expr(), dummy_expr()];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_ok());
    let query = result.unwrap();
    assert_eq!(
        query.template.as_str(),
        "UPDATE users SET name = ?, email = ? WHERE id = ?"
    );
}

#[test]
fn test_sql_delete_query() {
    let template = Text::from("DELETE FROM users WHERE id = {id}");
    let exprs = vec![dummy_expr()];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_ok());
    let query = result.unwrap();
    assert_eq!(query.template.as_str(), "DELETE FROM users WHERE id = ?");
}

// Template Validation Tests

#[test]
fn test_validate_safe_template() {
    let template = Text::from("SELECT * FROM users WHERE id = ?");
    let result = SqlInterpolationHandler::validate_template(&template, test_span());
    assert!(result.is_ok());
}

#[test]
fn test_validate_template_with_comment() {
    let template = Text::from("SELECT * FROM users -- comment");
    let result = SqlInterpolationHandler::validate_template(&template, test_span());
    // Validation passes but logs warning
    assert!(result.is_ok());
}

// Edge Cases

#[test]
fn test_sql_adjacent_interpolations() {
    let template = Text::from("SELECT {a}{b}");
    let exprs = vec![dummy_expr(), dummy_expr()];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_ok());
    let query = result.unwrap();
    assert_eq!(query.template.as_str(), "SELECT ??");
}

#[test]
fn test_sql_interpolation_at_start() {
    let template = Text::from("{table} WHERE id = {id}");
    let exprs = vec![dummy_expr(), dummy_expr()];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_ok());
    let query = result.unwrap();
    assert_eq!(query.template.as_str(), "? WHERE id = ?");
}

#[test]
fn test_sql_interpolation_at_end() {
    let template = Text::from("SELECT * FROM users WHERE id = {id}");
    let exprs = vec![dummy_expr()];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_ok());
    let query = result.unwrap();
    assert_eq!(query.template.as_str(), "SELECT * FROM users WHERE id = ?");
}

#[test]
fn test_sql_empty_template() {
    let template = Text::from("");
    let exprs = vec![];
    let result = SqlInterpolationHandler::handle(&template, &exprs, test_span());

    assert!(result.is_ok());
    let query = result.unwrap();
    assert_eq!(query.template.as_str(), "");
    assert_eq!(query.params.len(), 0);
}
