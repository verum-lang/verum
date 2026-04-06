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
//! SQL interpolation handler tests
//! Per CLAUDE.md standards - tests in tests/ directory

use verum_ast::Span;
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::literal::{IntLit, Literal, LiteralKind};
use verum_compiler::interpolation::SqlInterpolationHandler;
use verum_common::Text;

fn make_dummy_expr() -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit::new(42)),
            span: Span::default(),
        }),
        Span::default(),
    )
}

#[test]
fn test_simple_interpolation() {
    let template = Text::from("SELECT * FROM users WHERE id = {id}");
    let interpolations = vec![make_dummy_expr()];

    let result = SqlInterpolationHandler::handle(&template, &interpolations, Span::default());
    assert!(result.is_ok());

    let query = result.unwrap();
    assert_eq!(query.template.as_str(), "SELECT * FROM users WHERE id = ?");
    assert_eq!(query.params.len(), 1);
}

#[test]
fn test_multiple_interpolations() {
    let template = Text::from("SELECT * FROM users WHERE id = {id} AND name = {name}");
    let interpolations = vec![make_dummy_expr(), make_dummy_expr()];

    let result = SqlInterpolationHandler::handle(&template, &interpolations, Span::default());
    assert!(result.is_ok());

    let query = result.unwrap();
    assert_eq!(
        query.template.as_str(),
        "SELECT * FROM users WHERE id = ? AND name = ?"
    );
    assert_eq!(query.params.len(), 2);
}

#[test]
fn test_escaped_braces() {
    let template = Text::from("SELECT {{literal}} FROM users");
    let interpolations: Vec<Expr> = vec![];

    let result = SqlInterpolationHandler::handle(&template, &interpolations, Span::default());
    assert!(result.is_ok());

    let query = result.unwrap();
    assert_eq!(query.template.as_str(), "SELECT {literal} FROM users");
}

#[test]
fn test_no_interpolations() {
    let template = Text::from("SELECT * FROM users");
    let interpolations: Vec<Expr> = vec![];

    let result = SqlInterpolationHandler::handle(&template, &interpolations, Span::default());
    assert!(result.is_ok());

    let query = result.unwrap();
    assert_eq!(query.template.as_str(), "SELECT * FROM users");
    assert!(query.params.is_empty());
}

#[test]
fn test_unclosed_interpolation() {
    let template = Text::from("SELECT * FROM users WHERE id = {id");
    let interpolations = vec![make_dummy_expr()];

    let result = SqlInterpolationHandler::handle(&template, &interpolations, Span::default());
    assert!(result.is_err());
}

#[test]
fn test_wrong_number_of_expressions() {
    let template = Text::from("SELECT * FROM users WHERE id = {id} AND name = {name}");
    let interpolations = vec![make_dummy_expr()]; // Only one, need two

    let result = SqlInterpolationHandler::handle(&template, &interpolations, Span::default());
    assert!(result.is_err());
}

#[test]
fn test_validate_template() {
    let template = Text::from("SELECT * FROM users WHERE id = ?");
    let result = SqlInterpolationHandler::validate_template(&template, Span::default());
    assert!(result.is_ok());
}
