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
// Tests for contract literal lexing.
//
// Contract literals are compiler intrinsics for formal verification (NOT in @tagged_literal registry).
// - Syntax: contract#"..." (plain) or contract#"""...""" (raw multiline)
// - Used for preconditions, postconditions, invariants
// - Deep integration with type system and SMT solvers
// NOTE: r#"..."# syntax removed in simplified literal architecture

use verum_ast::span::FileId;
use verum_lexer::{Lexer, TokenKind};
use verum_common::Text;

/// Helper to lex a single token (excluding EOF)
fn lex_single(source: &str) -> TokenKind {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<_> = lexer.map(|r| r.unwrap()).collect();
    assert!(tokens.len() >= 2, "Expected at least one token plus EOF");
    tokens[0].kind.clone()
}

/// Helper to lex multiple tokens
fn lex_all(source: &str) -> Vec<TokenKind> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    lexer
        .map(|r| r.unwrap())
        .filter(|t| !matches!(t.kind, TokenKind::Eof))
        .map(|t| t.kind)
        .collect()
}

#[test]
fn test_contract_literal_simple_requires() {
    let source = r#"contract#"requires x > 0""#;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(content, Text::from("requires x > 0"));
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}

#[test]
fn test_contract_literal_simple_ensures() {
    let source = r#"contract#"ensures result >= 0""#;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(content, Text::from("ensures result >= 0"));
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}

#[test]
fn test_contract_literal_simple_invariant() {
    let source = r#"contract#"invariant total >= 0""#;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(content, Text::from("invariant total >= 0"));
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}

#[test]
fn test_contract_literal_with_logical_operators() {
    let source = r#"contract#"requires amount > 0 && balance >= amount""#;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(
                content,
                Text::from("requires amount > 0 && balance >= amount")
            );
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}

#[test]
fn test_contract_literal_with_arithmetic() {
    let source = r#"contract#"ensures result == old(balance) - amount""#;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(
                content,
                Text::from("ensures result == old(balance) - amount")
            );
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}

#[test]
fn test_contract_literal_with_escapes() {
    let source = r#"contract#"requires \"valid\" input""#;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            // Escapes should be processed: \" becomes "
            assert_eq!(content, Text::from(r#"requires "valid" input"#));
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}

#[test]
fn test_contract_literal_multiline_raw() {
    // Test contract#"""...""" syntax for raw multiline contracts
    // NOTE: r#"..."# syntax is no longer supported, use """...""" instead
    let source = r#"contract#"""requires x > 0 && y < 100""""#;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(content, Text::from("requires x > 0 && y < 100"));
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}

#[test]
fn test_contract_literal_multiline() {
    let source = r#"contract#"
        requires amount > 0;
        ensures result >= 0;
    ""#;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            assert!(content.contains("requires amount > 0"));
            assert!(content.contains("ensures result >= 0"));
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}

#[test]
fn test_multiple_contract_literals() {
    let source = r#"
        contract#"requires amount > 0"
        contract#"ensures result >= 0"
    "#;
    let tokens = lex_all(source);

    assert_eq!(tokens.len(), 2, "Expected 2 contract literals");

    match &tokens[0] {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(content, &Text::from("requires amount > 0"));
        }
        _ => panic!("Expected ContractLiteral for first token"),
    }

    match &tokens[1] {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(content, &Text::from("ensures result >= 0"));
        }
        _ => panic!("Expected ContractLiteral for second token"),
    }
}

#[test]
fn test_contract_literal_with_complex_expression() {
    let source = r#"contract#"ensures arr.iter().sum() == result""#;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(content, Text::from("ensures arr.iter().sum() == result"));
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}

#[test]
fn test_contract_literal_with_quantifiers() {
    let source = r#"contract#"requires forall(i: 0..arr.len(), arr[i] >= 0)""#;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(
                content,
                Text::from("requires forall(i: 0..arr.len(), arr[i] >= 0)")
            );
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}

#[test]
fn test_contract_literal_in_context() {
    // Test lexing contract literal alongside function syntax
    let source = r#"fn divide(a: Int, b: Int) contract#"requires b != 0""#;
    let tokens = lex_all(source);

    // Find the contract literal
    let contract_token = tokens
        .iter()
        .find(|t| matches!(t, TokenKind::ContractLiteral(_)));

    assert!(
        contract_token.is_some(),
        "Expected to find contract literal in function context"
    );

    if let Some(TokenKind::ContractLiteral(content)) = contract_token {
        assert_eq!(content, &Text::from("requires b != 0"));
    }
}

#[test]
fn test_contract_literal_empty() {
    let source = r#"contract#"""#;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(content, Text::from(""));
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}

#[test]
fn test_contract_literal_with_semicolons() {
    let source = r#"contract#"requires x > 0; ensures result > 0;""#;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(content, Text::from("requires x > 0; ensures result > 0;"));
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}

#[test]
fn test_contract_literal_refinement_type() {
    // Contract literal in refinement type context
    let source = r#"type Positive is Int where contract#"it > 0""#;
    let tokens = lex_all(source);

    let contract_token = tokens
        .iter()
        .find(|t| matches!(t, TokenKind::ContractLiteral(_)));

    assert!(
        contract_token.is_some(),
        "Expected to find contract literal in refinement type"
    );

    if let Some(TokenKind::ContractLiteral(content)) = contract_token {
        assert_eq!(content, &Text::from("it > 0"));
    }
}

#[test]
fn test_contract_literal_with_special_chars() {
    let source = r#"contract#"requires x != null && y?.value == 42""#;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(content, Text::from("requires x != null && y?.value == 42"));
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}

#[test]
fn test_contract_not_identifier() {
    // "contract" alone should be an identifier, not start a contract literal
    let source = "contract";
    let token = lex_single(source);

    match token {
        TokenKind::Ident(s) => {
            assert_eq!(s, Text::from("contract"));
        }
        _ => panic!("Expected Ident, got {:?}", token),
    }
}

#[test]
fn test_contract_literal_with_unicode() {
    let source = r#"contract#"requires x ≥ 0 ∧ y ≤ 100""#;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(content, Text::from("requires x ≥ 0 ∧ y ≤ 100"));
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}

#[test]
fn test_contract_raw_string_literal() {
    // contract#r#"..."# raw string syntax (no escape processing)
    let source = r####"contract#r#"requires x > 0 && (x + y) < MAX"#"####;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(
                content,
                Text::from("requires x > 0 && (x + y) < MAX")
            );
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}

#[test]
fn test_contract_raw_string_with_quotes() {
    // Raw string can contain unescaped quotes
    let source = r####"contract#r#"requires name != "admin""#"####;
    let token = lex_single(source);

    match token {
        TokenKind::ContractLiteral(content) => {
            assert_eq!(content, Text::from(r#"requires name != "admin""#));
        }
        _ => panic!("Expected ContractLiteral, got {:?}", token),
    }
}
