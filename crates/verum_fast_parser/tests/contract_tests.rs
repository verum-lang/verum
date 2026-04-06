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
// Tests for contract literal parsing.
//
// Tests for contract annotations: requires, ensures, invariant, decreases
//
// Contract literals in various contexts:
// - Function preconditions and postconditions
// - Refinement types
// - Loop invariants
// - Type constraints

use verum_ast::span::FileId;
use verum_ast::{Expr, ExprKind, Item, ItemKind, Literal, LiteralKind};
use verum_lexer::{Lexer, Token};
use verum_fast_parser::VerumParser;
use verum_common::Text;

/// Helper to parse a single expression
fn parse_expr(source: &str) -> Expr {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser.parse_expr_str(source, file_id).expect("Parse error")
}

/// Helper to parse module
fn parse_test_module(source: &str) -> verum_common::List<Item> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let module = parser.parse_module(lexer, file_id).expect("Parse error");
    module.items
}

#[test]
fn test_contract_literal_as_expression() {
    let source = r#"contract#"requires x > 0""#;
    let expr = parse_expr(source);

    match expr.kind {
        ExprKind::Literal(lit) => match lit.kind {
            LiteralKind::Contract(content) => {
                assert_eq!(content, Text::from("requires x > 0"));
            }
            _ => panic!("Expected Contract literal, got {:?}", lit.kind),
        },
        _ => panic!("Expected Literal expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_contract_literal_ensures() {
    let source = r#"contract#"ensures result >= 0""#;
    let expr = parse_expr(source);

    match expr.kind {
        ExprKind::Literal(lit) => match lit.kind {
            LiteralKind::Contract(content) => {
                assert_eq!(content, Text::from("ensures result >= 0"));
            }
            _ => panic!("Expected Contract literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

#[test]
fn test_contract_literal_invariant() {
    let source = r#"contract#"invariant total == arr[0..i].sum()""#;
    let expr = parse_expr(source);

    match expr.kind {
        ExprKind::Literal(lit) => match lit.kind {
            LiteralKind::Contract(content) => {
                assert_eq!(content, Text::from("invariant total == arr[0..i].sum()"));
            }
            _ => panic!("Expected Contract literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

#[test]
fn test_function_with_single_contract() {
    let source = r#"
        fn divide(a: Int, b: Int) -> Int
            contract#"requires b != 0"
        {
            a / b
        }
    "#;

    let items = parse_test_module(source);
    assert_eq!(items.len(), 1);

    // Note: Current parser doesn't have explicit contract field in FunctionDecl
    // Contracts would need to be parsed as part of the function body or attributes
    // This test validates that the contract literal parses successfully
    match &items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "divide");
            assert_eq!(func.params.len(), 2);
            // Contract handling in function context would be implementation-specific
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_function_with_multiple_contracts() {
    let source = r#"
        fn withdraw(balance: Float, amount: Float) -> Float
            contract#"requires amount > 0"
            contract#"requires amount <= balance"
            contract#"ensures result >= 0"
            contract#"ensures result == old(balance) - amount"
        {
            balance - amount
        }
    "#;

    let items = parse_test_module(source);
    assert_eq!(items.len(), 1);

    match &items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "withdraw");
            assert_eq!(func.params.len(), 2);
            // Multiple contracts would be tracked differently
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_contract_literal_with_old_expression() {
    let source = r#"contract#"ensures result == old(value) + 1""#;
    let expr = parse_expr(source);

    match expr.kind {
        ExprKind::Literal(lit) => match lit.kind {
            LiteralKind::Contract(content) => {
                assert!(content.contains("old(value)"));
            }
            _ => panic!("Expected Contract literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

#[test]
fn test_contract_literal_with_quantifier() {
    let source = r#"contract#"requires forall(i: 0..n, arr[i] > 0)""#;
    let expr = parse_expr(source);

    match expr.kind {
        ExprKind::Literal(lit) => match lit.kind {
            LiteralKind::Contract(content) => {
                assert!(content.contains("forall"));
                assert!(content.contains("arr[i] > 0"));
            }
            _ => panic!("Expected Contract literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

#[test]
fn test_contract_literal_complex_logic() {
    let source = r#"contract#"requires (x > 0 && y > 0) || (x < 0 && y < 0)""#;
    let expr = parse_expr(source);

    match expr.kind {
        ExprKind::Literal(lit) => match lit.kind {
            LiteralKind::Contract(content) => {
                assert!(content.contains("&&"));
                assert!(content.contains("||"));
            }
            _ => panic!("Expected Contract literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

#[test]
fn test_contract_literal_with_method_calls() {
    let source = r#"contract#"ensures result.is_valid() && result.len() > 0""#;
    let expr = parse_expr(source);

    match expr.kind {
        ExprKind::Literal(lit) => match lit.kind {
            LiteralKind::Contract(content) => {
                assert!(content.contains("is_valid()"));
                assert!(content.contains("len()"));
            }
            _ => panic!("Expected Contract literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

#[test]
fn test_contract_literal_empty_string() {
    let source = r#"contract#"""#;
    let expr = parse_expr(source);

    match expr.kind {
        ExprKind::Literal(lit) => match lit.kind {
            LiteralKind::Contract(content) => {
                assert_eq!(content, Text::from(""));
            }
            _ => panic!("Expected Contract literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

#[test]
fn test_contract_literal_multiline() {
    let source = r#"contract#"
        requires x > 0;
        requires y > 0;
        ensures result == x + y;
    ""#;
    let expr = parse_expr(source);

    match expr.kind {
        ExprKind::Literal(lit) => match lit.kind {
            LiteralKind::Contract(content) => {
                assert!(content.contains("requires x > 0"));
                assert!(content.contains("requires y > 0"));
                assert!(content.contains("ensures result == x + y"));
            }
            _ => panic!("Expected Contract literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

#[test]
fn test_contract_literal_with_implication() {
    let source = r#"contract#"requires x > 0 ==> result > 0""#;
    let expr = parse_expr(source);

    match expr.kind {
        ExprKind::Literal(lit) => match lit.kind {
            LiteralKind::Contract(content) => {
                assert!(content.contains("==>"));
            }
            _ => panic!("Expected Contract literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

#[test]
fn test_type_with_contract_refinement() {
    let source = r#"
        type Positive is Int where contract#"it > 0";
    "#;

    let items = parse_test_module(source);
    assert_eq!(items.len(), 1);

    match &items[0].kind {
        ItemKind::Type(ty_decl) => {
            assert_eq!(ty_decl.name.name.as_str(), "Positive");
            // Refinement constraint would contain the contract literal
        }
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_contract_literal_with_array_operations() {
    let source = r#"contract#"ensures result == arr.iter().sum()""#;
    let expr = parse_expr(source);

    match expr.kind {
        ExprKind::Literal(lit) => match lit.kind {
            LiteralKind::Contract(content) => {
                assert!(content.contains("iter()"));
                assert!(content.contains("sum()"));
            }
            _ => panic!("Expected Contract literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

#[test]
fn test_contract_literal_with_range() {
    let source = r#"contract#"requires i in 0..arr.len()""#;
    let expr = parse_expr(source);

    match expr.kind {
        ExprKind::Literal(lit) => match lit.kind {
            LiteralKind::Contract(content) => {
                assert!(content.contains("0..arr.len()"));
            }
            _ => panic!("Expected Contract literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

#[test]
fn test_contract_literal_exists_quantifier() {
    let source = r#"contract#"ensures exists(i: 0..n, arr[i] == target)""#;
    let expr = parse_expr(source);

    match expr.kind {
        ExprKind::Literal(lit) => match lit.kind {
            LiteralKind::Contract(content) => {
                assert!(content.contains("exists"));
            }
            _ => panic!("Expected Contract literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

#[test]
fn test_contract_literal_with_numeric_comparison() {
    let source = r#"contract#"requires 0 <= index && index < len""#;
    let expr = parse_expr(source);

    match expr.kind {
        ExprKind::Literal(lit) => match lit.kind {
            LiteralKind::Contract(content) => {
                assert!(content.contains("0 <= index"));
                assert!(content.contains("index < len"));
            }
            _ => panic!("Expected Contract literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

#[test]
fn test_contract_literal_preserves_whitespace() {
    let source = r#"contract#"requires    x    >    0""#;
    let expr = parse_expr(source);

    match expr.kind {
        ExprKind::Literal(lit) => match lit.kind {
            LiteralKind::Contract(content) => {
                // Content should preserve internal whitespace
                assert!(content.contains("    "));
            }
            _ => panic!("Expected Contract literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}
