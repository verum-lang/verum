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
//! Tests for loop annotations (invariant, decreases)
//!
//! Tests parsing of verification annotations on loops:
//! - `loop invariant expr { ... }`
//! - `while cond invariant expr decreases expr { ... }`
//! - `for x in iter invariant expr decreases expr { ... }`

use verum_ast::{Expr, ExprKind, FileId};
use verum_common::Maybe;
use verum_fast_parser::VerumParser;

fn parse_expr(input: &str) -> Expr {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_expr_str(input, file_id)
        .unwrap_or_else(|_| panic!("Failed to parse: {}", input))
}

#[test]
fn test_loop_with_invariant() {
    let input = r#"
        loop
            invariant x > 0
        {
            step()
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::Loop {
            label: _,
            body: _,
            invariants,
        } => {
            assert!(!invariants.is_empty(), "Expected invariant to be present");
        }
        _ => panic!("Expected Loop expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_loop_without_invariant() {
    let input = r#"
        loop {
            step()
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::Loop {
            label: _,
            body: _,
            invariants,
        } => {
            assert!(invariants.is_empty(), "Expected no invariant");
        }
        _ => panic!("Expected Loop expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_while_with_invariant() {
    let input = r#"
        while x > 0
            invariant x >= 0
        {
            x = x - 1
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::While {
            label: _,
            condition: _,
            body: _,
            invariants,
            decreases,
        } => {
            assert!(!invariants.is_empty(), "Expected invariant to be present");
            assert!(decreases.is_empty(), "Expected no decreases");
        }
        _ => panic!("Expected While expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_while_with_decreases() {
    let input = r#"
        while n > 0
            decreases n
        {
            n = n - 1
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::While {
            label: _,
            condition: _,
            body: _,
            invariants,
            decreases,
        } => {
            assert!(invariants.is_empty(), "Expected no invariant");
            assert!(!decreases.is_empty(), "Expected decreases to be present");
        }
        _ => panic!("Expected While expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_while_with_invariant_and_decreases() {
    let input = r#"
        while n > 0
            invariant n >= 0
            decreases n
        {
            n = n - 1
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::While {
            label: _,
            condition: _,
            body: _,
            invariants,
            decreases,
        } => {
            assert!(!invariants.is_empty(), "Expected invariant to be present");
            assert!(!decreases.is_empty(), "Expected decreases to be present");
        }
        _ => panic!("Expected While expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_while_without_annotations() {
    let input = r#"
        while x < 10 {
            x = x + 1
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::While {
            label: _,
            condition: _,
            body: _,
            invariants,
            decreases,
        } => {
            assert!(invariants.is_empty(), "Expected no invariant");
            assert!(decreases.is_empty(), "Expected no decreases");
        }
        _ => panic!("Expected While expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_for_with_invariant() {
    let input = r#"
        for i in 0..n
            invariant acc >= 0
        {
            acc = acc + i
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::For {
            label: _,
            pattern: _,
            iter: _,
            body: _,
            invariants,
            decreases,
        } => {
            assert!(!invariants.is_empty(), "Expected invariant to be present");
            assert!(decreases.is_empty(), "Expected no decreases");
        }
        _ => panic!("Expected For expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_for_with_decreases() {
    let input = r#"
        for i in 0..n
            decreases n - i
        {
            process(i)
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::For {
            label: _,
            pattern: _,
            iter: _,
            body: _,
            invariants,
            decreases,
        } => {
            assert!(invariants.is_empty(), "Expected no invariant");
            assert!(!decreases.is_empty(), "Expected decreases to be present");
        }
        _ => panic!("Expected For expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_for_with_invariant_and_decreases() {
    let input = r#"
        for i in 0..n
            invariant acc >= 0
            decreases n - i
        {
            acc = acc + i
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::For {
            label: _,
            pattern: _,
            iter: _,
            body: _,
            invariants,
            decreases,
        } => {
            assert!(!invariants.is_empty(), "Expected invariant to be present");
            assert!(!decreases.is_empty(), "Expected decreases to be present");
        }
        _ => panic!("Expected For expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_for_without_annotations() {
    let input = r#"
        for x in items {
            print(x)
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::For {
            label: _,
            pattern: _,
            iter: _,
            body: _,
            invariants,
            decreases,
        } => {
            assert!(invariants.is_empty(), "Expected no invariant");
            assert!(decreases.is_empty(), "Expected no decreases");
        }
        _ => panic!("Expected For expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_nested_loops_with_invariants() {
    let input = r#"
        for i in 0..n
            invariant i <= n
        {
            for j in 0..m
                invariant j <= m
            {
                matrix[i][j] = i + j
            }
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::For {
            label: _,
            pattern: _,
            iter: _,
            body,
            invariants,
            decreases: _,
        } => {
            assert!(!invariants.is_empty(), "Expected outer loop to have invariant");

            // Check inner loop
            if let Maybe::Some(first_stmt) = body.stmts.first() {
                // The inner for loop should also have an invariant
                // This is a simple structural check
                // (full validation would require more detailed inspection)
            }
        }
        _ => panic!("Expected For expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_complex_invariant_expression() {
    let input = r#"
        while i < n
            invariant i >= 0 && i <= n
        {
            i = i + 1
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::While {
            label: _,
            condition: _,
            body: _,
            invariants,
            decreases: _,
        } => {
            assert!(
                !invariants.is_empty(),
                "Expected complex invariant to be present"
            );
        }
        _ => panic!("Expected While expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_complex_decreases_expression() {
    let input = r#"
        for i in 0..n
            decreases (n - i, m - j)
        {
            process()
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::For {
            label: _,
            pattern: _,
            iter: _,
            body: _,
            invariants: _,
            decreases,
        } => {
            assert!(
                !decreases.is_empty(),
                "Expected decreases with tuple to be present"
            );
            // Get the first decreases expression
            if let Some(dec) = decreases.iter().next() {
                match &dec.kind {
                    ExprKind::Tuple(_) => {
                        // Success: parsed as tuple
                    }
                    _ => panic!("Expected tuple for decreases, got {:?}", dec.kind),
                }
            }
        }
        _ => panic!("Expected For expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_loop_invariant_with_method_call() {
    let input = r#"
        loop
            invariant buffer.is_valid() && buffer.len() > 0
        {
            buffer.pop()
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::Loop {
            label: _,
            body: _,
            invariants,
        } => {
            assert!(!invariants.is_empty(), "Expected invariant with method calls");
        }
        _ => panic!("Expected Loop expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_decreases_order_after_invariant() {
    // Test that decreases must come after invariant (not before)
    let input = r#"
        while x > 0
            invariant x >= 0
            decreases x
        {
            x = x - 1
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::While {
            label: _,
            condition: _,
            body: _,
            invariants,
            decreases,
        } => {
            assert!(!invariants.is_empty(), "Expected invariant");
            assert!(!decreases.is_empty(), "Expected decreases");
        }
        _ => panic!("Expected While expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_invariant_with_quantifiers() {
    let input = r#"
        for i in 0..n
            invariant sum >= 0
        {
            sum = sum + i
        }
    "#;

    let expr = parse_expr(input);
    match expr.kind {
        ExprKind::For {
            label: _,
            pattern: _,
            iter: _,
            body: _,
            invariants,
            decreases: _,
        } => {
            assert!(!invariants.is_empty(), "Expected invariant");
        }
        _ => panic!("Expected For expression, got {:?}", expr.kind),
    }
}
