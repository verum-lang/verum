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
// Unit tests for contract.rs
//
// Migrated from src/contract.rs to comply with CLAUDE.md test organization.

use verum_smt::contract::*;
use verum_smt::rsl_parser::RslClauseKind;

use verum_ast::{Expr, Literal, Span};
use verum_common::Text;
use verum_common::Text as StdText;

#[test]
fn test_parse_simple_contract() {
    let content = StdText::from("requires x > 0");
    let spec = parse_contract_literal(&content, Span::dummy()).unwrap();

    assert_eq!(spec.preconditions.len(), 1);
    assert_eq!(spec.preconditions[0].kind, RslClauseKind::Requires);
}

#[test]
fn test_parse_complex_contract() {
    let content = StdText::from("requires x > 0; ensures result >= 0; invariant len > 0;");
    let spec = parse_contract_literal(&content, Span::dummy()).unwrap();

    assert_eq!(spec.preconditions.len(), 1);
    assert_eq!(spec.postconditions.len(), 1);
    assert_eq!(spec.invariants.len(), 1);
}

#[test]
fn test_extract_contract_from_literal() {
    let lit = Literal::contract(Text::from("requires x > 0"), Span::dummy());
    let expr = Expr::literal(lit);

    let content = extract_contract_from_expr(&expr).unwrap();
    assert_eq!(content, Text::from("requires x > 0"));
}

#[test]
fn test_merge_contracts() {
    let spec1 = parse_contract_literal(&StdText::from("requires x > 0"), Span::dummy()).unwrap();
    let spec2 =
        parse_contract_literal(&StdText::from("ensures result >= 0"), Span::dummy()).unwrap();

    let merged = merge_contracts(&[spec1, spec2]);

    assert_eq!(merged.preconditions.len(), 1);
    assert_eq!(merged.postconditions.len(), 1);
}

#[test]
fn test_validate_contract_result_in_precondition() {
    let content = StdText::from("requires result > 0");
    let spec = parse_contract_literal(&content, Span::dummy()).unwrap();

    let result = validate_contract(&spec);
    assert!(result.is_err());
}

#[test]
fn test_validate_contract_old_in_precondition() {
    let content = StdText::from("requires old(x) > 0");
    let spec = parse_contract_literal(&content, Span::dummy()).unwrap();

    let result = validate_contract(&spec);
    assert!(result.is_err());
}

#[test]
fn test_validate_contract_valid() {
    let content = StdText::from("requires x > 0; ensures result == old(x) + 1");
    let spec = parse_contract_literal(&content, Span::dummy()).unwrap();

    let result = validate_contract(&spec);
    assert!(result.is_ok());
}

#[test]
fn test_contains_result() {
    let lit = Literal::int(42, Span::dummy());
    let expr = Expr::literal(lit);
    assert!(!contains_result(&expr));

    let result_expr = Expr::path(verum_ast::Path::from_ident(verum_ast::Ident::new(
        Text::from("result"),
        Span::dummy(),
    )));
    assert!(contains_result(&result_expr));
}

#[test]
fn test_find_contract_literals() {
    let lit1 = Literal::contract(Text::from("requires x > 0"), Span::dummy());
    let lit2 = Literal::int(42, Span::dummy());
    let lit3 = Literal::contract(Text::from("ensures result >= 0"), Span::dummy());

    let exprs = vec![
        Expr::literal(lit1),
        Expr::literal(lit2),
        Expr::literal(lit3),
    ];

    let contracts = find_contract_literals(&exprs);
    assert_eq!(contracts.len(), 2);
}
