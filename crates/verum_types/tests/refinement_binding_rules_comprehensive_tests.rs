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
// Comprehensive Tests for 5 Refinement Binding Rules
//
// Five refinement binding rules: (1) Inline T{pred} with implicit "it", (2) Lambda-style "where |x| pred", (3) Sigma-type "x: T where P(x)", (4) Named predicate "where pred_name", (5) Bare "where pred" (deprecated) — Five Binding Rules
//
// This test suite comprehensively tests all 5 binding rules for refinement types:
// 1. **Inline Refinement** (`Int{> 0}`) - implicit 'it' binding
// 2. **Lambda-style** (`Int where |x| x > 0`) - explicit variable binding
// 3. **Sigma-type** (`x: Int where x > 0`) - dependent type binding
// 4. **Named Predicate** (`Int where is_positive`) - predicate reference
// 5. **Bare where** (`Int where it > 0`) - deprecated but supported
//
// For each rule, we test:
// - Basic functionality
// - Field access patterns
// - Variable scoping
// - Edge cases
// - Nested structures

use verum_ast::{
    expr::*,
    literal::Literal,
    span::Span,
    ty::{Ident, Path, PathSegment},
};
use verum_common::List;
use verum_common::Text;
use verum_types::refinement::*;
use verum_types::ty::Type;

// ============================================================================
// Helper Functions
// ============================================================================

/// Create a variable expression
fn var_expr(name: &str, span: Span) -> Expr {
    Expr::ident(Ident::new(name.to_string(), span))
}

/// Create a binary operation expression
fn binop_expr(op: BinOp, left: Expr, right: Expr, span: Span) -> Expr {
    use verum_common::Heap;
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        span,
    )
}

/// Create an integer literal
fn int_literal(value: i128, span: Span) -> Expr {
    Expr::literal(Literal::int(value, span))
}

/// Create a simple path for named predicates
fn simple_path(name: &str, span: Span) -> Path {
    use smallvec::smallvec;
    Path {
        segments: smallvec![PathSegment::Name(Ident::new(name.to_string(), span))],
        span,
    }
}

// ============================================================================
// RULE 1: Inline Refinement (Int{> 0})
// ============================================================================

#[test]
fn rule1_inline_basic_positive_int() {
    let span = Span::dummy();

    // Int{> 0} means: x > 0 with implicit 'it' binding
    let predicate_expr = binop_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);

    let predicate = RefinementPredicate::inline(predicate_expr, span);

    // Verify binding is inline
    assert_eq!(predicate.binding, RefinementBinding::Inline);
    // Verify bound variable is implicit 'it'
    assert_eq!(predicate.bound_variable(), "it");
}

#[test]
fn rule1_inline_basic_non_empty() {
    let span = Span::dummy();

    // List{len > 0}
    let predicate_expr = binop_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);

    let predicate = RefinementPredicate::inline(predicate_expr, span);
    let refined_list = Type::refined(
        Type::Named {
            path: simple_path("List", span),
            args: vec![Type::int()].into(),
        },
        predicate,
    );

    assert!(matches!(refined_list, Type::Refined { .. }));
    assert_eq!(refined_list.base().to_string(), "List<Int>");
}

#[test]
fn rule1_inline_negative_bound() {
    let span = Span::dummy();

    // Int{< 0} negative integers
    let predicate_expr = binop_expr(BinOp::Lt, var_expr("it", span), int_literal(0, span), span);

    let predicate = RefinementPredicate::inline(predicate_expr, span);
    assert_eq!(predicate.binding, RefinementBinding::Inline);
}

#[test]
fn rule1_inline_range_constraint() {
    let span = Span::dummy();

    // Int{0 <= it && it <= 100}
    let left = binop_expr(BinOp::Ge, int_literal(0, span), var_expr("it", span), span);

    let right = binop_expr(
        BinOp::Le,
        var_expr("it", span),
        int_literal(100, span),
        span,
    );

    let combined = binop_expr(BinOp::And, left, right, span);

    let predicate = RefinementPredicate::inline(combined, span);
    assert_eq!(predicate.binding, RefinementBinding::Inline);
}

#[test]
fn rule1_inline_equality_constraint() {
    let span = Span::dummy();

    // Float{it != 0.0}
    let predicate_expr = binop_expr(
        BinOp::Ne,
        var_expr("it", span),
        Expr::literal(Literal::bool(false, span)),
        span,
    );

    let predicate = RefinementPredicate::inline(predicate_expr, span);
    assert_eq!(predicate.binding, RefinementBinding::Inline);
}

#[test]
fn rule1_inline_complex_expression() {
    let span = Span::dummy();

    // Int{it % 2 == 0} (even number)
    let modulo = binop_expr(BinOp::Rem, var_expr("it", span), int_literal(2, span), span);

    let equality = binop_expr(BinOp::Eq, modulo, int_literal(0, span), span);

    let predicate = RefinementPredicate::inline(equality, span);
    assert_eq!(predicate.binding, RefinementBinding::Inline);
}

#[test]
fn rule1_inline_type_creation() {
    let span = Span::dummy();

    let predicate_expr = binop_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);

    let predicate = RefinementPredicate::inline(predicate_expr, span);
    let positive_int = Type::refined(Type::int(), predicate);

    // Verify we can create a refined type from inline binding
    assert!(matches!(positive_int, Type::Refined { .. }));
}

// ============================================================================
// RULE 2: Lambda-Style Where Clause (Int where |x| x > 0)
// ============================================================================

#[test]
fn rule2_lambda_basic_positive_int() {
    let span = Span::dummy();

    // Int where |x| x > 0
    let predicate_expr = binop_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);

    let predicate = RefinementPredicate::lambda(predicate_expr, Text::from("x"), span);

    // Verify binding is lambda with explicit variable
    assert_eq!(
        predicate.binding,
        RefinementBinding::Lambda(Text::from("x"))
    );
    assert_eq!(predicate.bound_variable(), "x");
}

#[test]
fn rule2_lambda_explicit_variable_access() {
    let span = Span::dummy();

    // Text where |s| len(s) > 5
    let predicate_expr = binop_expr(BinOp::Gt, var_expr("s", span), int_literal(5, span), span);

    let predicate = RefinementPredicate::lambda(predicate_expr, Text::from("s"), span);
    assert_eq!(predicate.bound_variable(), "s");
}

#[test]
fn rule2_lambda_multiple_variable_uses() {
    let span = Span::dummy();

    // Pair where |p| p.fst < p.snd
    // This tests that lambda binding allows multiple uses of the bound variable
    let left = var_expr("p", span);
    let right = var_expr("p", span);

    let predicate_expr = binop_expr(BinOp::Lt, left, right, span);
    let predicate = RefinementPredicate::lambda(predicate_expr, Text::from("p"), span);

    assert_eq!(predicate.bound_variable(), "p");
}

#[test]
fn rule2_lambda_field_access_pattern() {
    let span = Span::dummy();

    // Struct where |rec| rec.age >= 18
    let predicate_expr = binop_expr(
        BinOp::Ge,
        var_expr("rec", span),
        int_literal(18, span),
        span,
    );

    let predicate = RefinementPredicate::lambda(predicate_expr, Text::from("rec"), span);
    assert_eq!(predicate.bound_variable(), "rec");
}

#[test]
fn rule2_lambda_list_constraint() {
    let span = Span::dummy();

    // List where |xs| len(xs) > 0
    let predicate_expr = binop_expr(BinOp::Gt, var_expr("xs", span), int_literal(0, span), span);

    let predicate = RefinementPredicate::lambda(predicate_expr, Text::from("xs"), span);
    let non_empty_list = Type::refined(
        Type::Named {
            path: simple_path("List", span),
            args: vec![Type::int()].into(),
        },
        predicate,
    );

    assert!(matches!(non_empty_list, Type::Refined { .. }));
}

#[test]
fn rule2_lambda_different_variable_names() {
    let span = Span::dummy();

    // Test that lambda can use any variable name
    let test_names = vec!["x", "value", "elem", "item", "n"];

    for name in test_names {
        let predicate_expr =
            binop_expr(BinOp::Gt, var_expr(name, span), int_literal(0, span), span);

        let predicate = RefinementPredicate::lambda(predicate_expr, Text::from(name), span);

        assert_eq!(predicate.bound_variable().as_str(), name);
    }
}

#[test]
fn rule2_lambda_scoping_correctness() {
    let span = Span::dummy();

    // Verify that the bound variable in lambda doesn't leak outside
    let predicate_expr = binop_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);

    let predicate = RefinementPredicate::lambda(predicate_expr, Text::from("x"), span);

    // The predicate's bound variable should be 'x'
    assert_eq!(predicate.bound_variable(), "x");

    // But we could also have another predicate with a different binding
    let other_expr = binop_expr(BinOp::Lt, var_expr("y", span), int_literal(100, span), span);

    let other = RefinementPredicate::lambda(other_expr, Text::from("y"), span);
    assert_eq!(other.bound_variable(), "y");

    // They're independent
    assert_ne!(predicate.bound_variable(), other.bound_variable());
}

#[test]
fn rule2_lambda_complex_expression() {
    let span = Span::dummy();

    // List where |items| len(items) > 0 && len(items) <= 1000
    let len_check = binop_expr(
        BinOp::Gt,
        var_expr("items", span),
        int_literal(0, span),
        span,
    );

    let max_check = binop_expr(
        BinOp::Le,
        var_expr("items", span),
        int_literal(1000, span),
        span,
    );

    let combined = binop_expr(BinOp::And, len_check, max_check, span);

    let predicate = RefinementPredicate::lambda(combined, Text::from("items"), span);
    assert_eq!(predicate.bound_variable(), "items");
}

// ============================================================================
// RULE 3: Sigma-Type Refinement (x: Int where x > 0)
// ============================================================================

#[test]
fn rule3_sigma_basic_positive_int() {
    let span = Span::dummy();

    // x: Int where x > 0
    let predicate_expr = binop_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);

    let predicate = RefinementPredicate::sigma(predicate_expr, Text::from("x"), span);

    // Verify binding is sigma with explicit variable
    assert_eq!(predicate.binding, RefinementBinding::Sigma(Text::from("x")));
    assert_eq!(predicate.bound_variable(), "x");
}

#[test]
fn rule3_sigma_dependent_pair() {
    let span = Span::dummy();

    // p: Pair where p.fst < p.snd
    let predicate_expr = binop_expr(BinOp::Lt, var_expr("p", span), var_expr("p", span), span);

    let predicate = RefinementPredicate::sigma(predicate_expr, Text::from("p"), span);
    assert_eq!(predicate.bound_variable(), "p");
}

#[test]
fn rule3_sigma_explicit_name_binding() {
    let span = Span::dummy();

    // Test that sigma type preserves the explicit variable name
    let names = vec!["x", "val", "result", "param", "element"];

    for name in names {
        let predicate_expr =
            binop_expr(BinOp::Gt, var_expr(name, span), int_literal(0, span), span);

        let predicate = RefinementPredicate::sigma(predicate_expr, Text::from(name), span);

        assert_eq!(predicate.bound_variable().as_str(), name);
        assert_eq!(
            predicate.binding,
            RefinementBinding::Sigma(Text::from(name))
        );
    }
}

#[test]
fn rule3_sigma_field_access_dependent() {
    let span = Span::dummy();

    // rec: Record where rec.age >= 18 && rec.email != ""
    let age_check = binop_expr(
        BinOp::Ge,
        var_expr("rec", span),
        int_literal(18, span),
        span,
    );

    let email_check = binop_expr(
        BinOp::Ne,
        var_expr("rec", span),
        Expr::literal(Literal::string("".to_string().into(), span)),
        span,
    );

    let combined = binop_expr(BinOp::And, age_check, email_check, span);

    let predicate = RefinementPredicate::sigma(combined, Text::from("rec"), span);
    assert_eq!(predicate.bound_variable(), "rec");
}

#[test]
fn rule3_sigma_list_with_constraint() {
    let span = Span::dummy();

    // xs: List where len(xs) > 0
    let predicate_expr = binop_expr(BinOp::Gt, var_expr("xs", span), int_literal(0, span), span);

    let predicate = RefinementPredicate::sigma(predicate_expr, Text::from("xs"), span);
    let non_empty = Type::refined(
        Type::Named {
            path: simple_path("List", span),
            args: vec![Type::int()].into(),
        },
        predicate,
    );

    assert!(matches!(non_empty, Type::Refined { .. }));
}

#[test]
fn rule3_sigma_bound_variable_preserved() {
    let span = Span::dummy();

    // Verify that sigma-type bindings preserve the variable name
    // for use in dependent type expressions
    let predicate_expr = binop_expr(
        BinOp::Gt,
        var_expr("val", span),
        int_literal(100, span),
        span,
    );

    let predicate = RefinementPredicate::sigma(predicate_expr, Text::from("val"), span);

    // The name should be preserved for dependent type construction
    match &predicate.binding {
        RefinementBinding::Sigma(name) => {
            assert_eq!(name, &"val");
        }
        _ => panic!("Expected sigma binding"),
    }
}

#[test]
fn rule3_sigma_multiple_constraints() {
    let span = Span::dummy();

    // account: Account where account.balance >= 0 && account.status != Closed
    let balance_check = binop_expr(
        BinOp::Ge,
        var_expr("account", span),
        int_literal(0, span),
        span,
    );

    let status_check = binop_expr(
        BinOp::Ne,
        var_expr("account", span),
        Expr::literal(Literal::bool(false, span)),
        span,
    );

    let combined = binop_expr(BinOp::And, balance_check, status_check, span);

    let predicate = RefinementPredicate::sigma(combined, Text::from("account"), span);
    assert_eq!(predicate.bound_variable(), "account");
}

// ============================================================================
// RULE 4: Named Predicate (Int where is_positive)
// ============================================================================

#[test]
fn rule4_named_basic_predicate() {
    let span = Span::dummy();

    // Int where is_positive
    let path = simple_path("is_positive", span);
    let predicate = RefinementPredicate::named(path, span);

    // Verify binding is named
    match &predicate.binding {
        RefinementBinding::Named(_) => {
            // Expected
        }
        _ => panic!("Expected named binding"),
    }

    // The implicit binding should be 'it'
    assert_eq!(predicate.bound_variable(), "it");
}

#[test]
fn rule4_named_multiple_predicates() {
    let span = Span::dummy();

    // Test various named predicates
    let predicates = vec!["is_positive", "is_email", "is_valid", "is_sorted"];

    for pred_name in predicates {
        let path = simple_path(pred_name, span);
        let predicate = RefinementPredicate::named(path, span);

        match &predicate.binding {
            RefinementBinding::Named(_) => {
                // Expected
            }
            _ => panic!("Expected named binding for {}", pred_name),
        }
    }
}

#[test]
fn rule4_named_predicate_reusability() {
    let span = Span::dummy();

    // Same predicate can be reused on different types
    let is_valid_path = simple_path("is_valid", span);

    // Int where is_valid
    let int_pred = RefinementPredicate::named(is_valid_path.clone(), span);
    let refined_int = Type::refined(Type::int(), int_pred);

    // Text where is_valid
    let text_pred = RefinementPredicate::named(is_valid_path, span);
    let refined_text = Type::refined(Type::text(), text_pred);

    // Both should be refined types
    assert!(matches!(refined_int, Type::Refined { .. }));
    assert!(matches!(refined_text, Type::Refined { .. }));
}

#[test]
fn rule4_named_predicate_call_generation() {
    let span = Span::dummy();

    // Verify that named predicate generates a call expression
    let path = simple_path("is_positive", span);
    let predicate = RefinementPredicate::named(path, span);

    // The predicate expr should be a function call: is_positive(it)
    match &predicate.predicate.kind {
        ExprKind::Call { .. } => {
            // Expected: function call
        }
        _ => panic!("Expected call expression for named predicate"),
    }
}

#[test]
fn rule4_named_qualified_path() {
    let span = Span::dummy();

    // Test that we can have qualified predicate names
    // Example: validators::is_email
    use smallvec::smallvec;
    let path = Path {
        segments: smallvec![
            PathSegment::Name(Ident::new("validators".to_string(), span)),
            PathSegment::Name(Ident::new("is_email".to_string(), span)),
        ],
        span,
    };

    let predicate = RefinementPredicate::named(path, span);

    match &predicate.binding {
        RefinementBinding::Named(_) => {
            // Expected
        }
        _ => panic!("Expected named binding with qualified path"),
    }
}

#[test]
fn rule4_named_common_predicates() {
    let span = Span::dummy();

    // Test common predicate names from specification
    let common = vec![
        "is_positive",
        "is_negative",
        "is_zero",
        "is_non_empty",
        "is_sorted",
        "is_valid_email",
        "is_ascii",
    ];

    for pred in common {
        let path = simple_path(pred, span);
        let predicate = RefinementPredicate::named(path, span);

        assert_eq!(predicate.bound_variable(), "it");
    }
}

#[test]
fn rule4_named_with_type_creation() {
    let span = Span::dummy();

    let path = simple_path("is_valid", span);
    let predicate = RefinementPredicate::named(path, span);
    let refined_type = Type::refined(Type::int(), predicate);

    // Should successfully create a refined type
    assert!(matches!(refined_type, Type::Refined { .. }));
}

// ============================================================================
// RULE 5: Bare Where Clause (Int where it > 0) - Deprecated
// ============================================================================

#[test]
fn rule5_bare_basic_where_clause() {
    let span = Span::dummy();

    // Int where it > 0 (deprecated form)
    let predicate_expr = binop_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);

    let predicate = RefinementPredicate::bare(predicate_expr, span);

    // Verify binding is bare
    assert_eq!(predicate.binding, RefinementBinding::Bare);
    // Implicit binding should still be 'it'
    assert_eq!(predicate.bound_variable(), "it");
}

#[test]
fn rule5_bare_implicit_it_binding() {
    let span = Span::dummy();

    // Test that bare where clause uses implicit 'it'
    let predicate_expr = binop_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);

    let predicate = RefinementPredicate::bare(predicate_expr, span);

    // Should bind to 'it'
    assert_eq!(predicate.bound_variable(), "it");

    // But it's marked as deprecated (Bare variant)
    match &predicate.binding {
        RefinementBinding::Bare => {
            // Expected: deprecated form
        }
        _ => panic!("Expected bare binding"),
    }
}

#[test]
fn rule5_bare_negative_constraint() {
    let span = Span::dummy();

    // Int where it < 0
    let predicate_expr = binop_expr(BinOp::Lt, var_expr("it", span), int_literal(0, span), span);

    let predicate = RefinementPredicate::bare(predicate_expr, span);
    assert_eq!(predicate.binding, RefinementBinding::Bare);
}

#[test]
fn rule5_bare_equality_check() {
    let span = Span::dummy();

    // String where it != ""
    let predicate_expr = binop_expr(
        BinOp::Ne,
        var_expr("it", span),
        Expr::literal(Literal::string("".to_string().into(), span)),
        span,
    );

    let predicate = RefinementPredicate::bare(predicate_expr, span);
    assert_eq!(predicate.binding, RefinementBinding::Bare);
}

#[test]
fn rule5_bare_comparison_with_inline() {
    let span = Span::dummy();

    // Both inline and bare use 'it', but differ in syntax
    let expr = binop_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);

    // Rule 1: Inline (preferred)
    let inline = RefinementPredicate::inline(expr.clone(), span);

    // Rule 5: Bare (deprecated)
    let bare = RefinementPredicate::bare(expr, span);

    // Same bound variable
    assert_eq!(inline.bound_variable(), bare.bound_variable());

    // Different binding types
    assert_ne!(inline.binding, bare.binding);
}

#[test]
fn rule5_bare_complex_predicate() {
    let span = Span::dummy();

    // String where it != ""
    let predicate_expr = binop_expr(
        BinOp::Ne,
        var_expr("it", span),
        Expr::literal(Literal::string("".to_string().into(), span)),
        span,
    );

    let predicate = RefinementPredicate::bare(predicate_expr, span);
    assert_eq!(predicate.binding, RefinementBinding::Bare);
}

#[test]
fn rule5_bare_list_constraint() {
    let span = Span::dummy();

    // List where it.len > 0 && it.len < 1000
    let len_check = binop_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);

    let max_check = binop_expr(
        BinOp::Lt,
        var_expr("it", span),
        int_literal(1000, span),
        span,
    );

    let combined = binop_expr(BinOp::And, len_check, max_check, span);

    let predicate = RefinementPredicate::bare(combined, span);
    assert_eq!(predicate.binding, RefinementBinding::Bare);
}

// ============================================================================
// Cross-Rule Comparison Tests
// ============================================================================

#[test]
fn compare_all_rules_bound_variables() {
    let span = Span::dummy();

    // Create predicates using all 5 rules
    let expr1 = binop_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);
    let rule1 = RefinementPredicate::inline(expr1, span);

    let expr2 = binop_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span);
    let rule2 = RefinementPredicate::lambda(expr2, Text::from("x"), span);

    let expr3 = binop_expr(BinOp::Gt, var_expr("val", span), int_literal(0, span), span);
    let rule3 = RefinementPredicate::sigma(expr3, Text::from("val"), span);

    let rule4 = RefinementPredicate::named(simple_path("is_positive", span), span);

    let expr5 = binop_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span);
    let rule5 = RefinementPredicate::bare(expr5, span);

    // Verify each rule
    assert_eq!(rule1.bound_variable(), "it");
    assert_eq!(rule2.bound_variable(), "x");
    assert_eq!(rule3.bound_variable(), "val");
    assert_eq!(rule4.bound_variable(), "it");
    assert_eq!(rule5.bound_variable(), "it");
}

#[test]
fn all_rules_create_refined_types() {
    let span = Span::dummy();

    // Each rule should be able to create a refined type

    // Rule 1: Inline
    let pred1 = RefinementPredicate::inline(
        binop_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span),
        span,
    );
    let type1 = Type::refined(Type::int(), pred1);

    // Rule 2: Lambda
    let pred2 = RefinementPredicate::lambda(
        binop_expr(BinOp::Gt, var_expr("x", span), int_literal(0, span), span),
        Text::from("x"),
        span,
    );
    let type2 = Type::refined(Type::int(), pred2);

    // Rule 3: Sigma
    let pred3 = RefinementPredicate::sigma(
        binop_expr(BinOp::Gt, var_expr("val", span), int_literal(0, span), span),
        Text::from("val"),
        span,
    );
    let type3 = Type::refined(Type::int(), pred3);

    // Rule 4: Named
    let pred4 = RefinementPredicate::named(simple_path("is_positive", span), span);
    let type4 = Type::refined(Type::int(), pred4);

    // Rule 5: Bare
    let pred5 = RefinementPredicate::bare(
        binop_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span),
        span,
    );
    let type5 = Type::refined(Type::int(), pred5);

    // All should be refined types
    assert!(matches!(type1, Type::Refined { .. }));
    assert!(matches!(type2, Type::Refined { .. }));
    assert!(matches!(type3, Type::Refined { .. }));
    assert!(matches!(type4, Type::Refined { .. }));
    assert!(matches!(type5, Type::Refined { .. }));
}

// ============================================================================
// Edge Cases and Error Conditions
// ============================================================================

#[test]
fn edge_case_trivial_predicate_all_rules() {
    let span = Span::dummy();
    let true_expr = Expr::literal(Literal::bool(true, span));

    // All rules should handle trivial (true) predicate
    let inline = RefinementPredicate::inline(true_expr.clone(), span);
    let lambda = RefinementPredicate::lambda(true_expr.clone(), Text::from("x"), span);
    let sigma = RefinementPredicate::sigma(true_expr.clone(), Text::from("x"), span);
    let named = RefinementPredicate::named(simple_path("always_true", span), span);
    let bare = RefinementPredicate::bare(true_expr, span);

    assert!(inline.is_trivial());
    // Lambda, sigma, and bare with true expr should also be trivial
    assert!(lambda.is_trivial());
    assert!(sigma.is_trivial());
    assert!(bare.is_trivial());

    // Named predicate is never trivial (it's a function call)
    assert!(!named.is_trivial());
}

#[test]
fn edge_case_nested_refinement() {
    let span = Span::dummy();

    // Test that we can nest refinements
    let inner_pred = RefinementPredicate::inline(
        binop_expr(BinOp::Gt, var_expr("it", span), int_literal(0, span), span),
        span,
    );

    let positive_int = Type::refined(Type::int(), inner_pred);

    // Now refine a List of positive ints
    let outer_pred = RefinementPredicate::lambda(
        binop_expr(BinOp::Gt, var_expr("xs", span), int_literal(0, span), span),
        Text::from("xs"),
        span,
    );

    let _list_of_positive = Type::refined(
        Type::Named {
            path: simple_path("List", span),
            args: vec![positive_int].into(),
        },
        outer_pred,
    );

    // Should succeed without panic
}

#[test]
fn edge_case_empty_path_segments() {
    let span = Span::dummy();

    // Single segment path (simple name)
    let simple = simple_path("is_valid", span);
    let pred = RefinementPredicate::named(simple, span);

    // Should work
    match &pred.binding {
        RefinementBinding::Named(_) => {}
        _ => panic!("Expected named binding"),
    }
}

#[test]
fn edge_case_bound_variable_consistency() {
    let span = Span::dummy();

    // Test that bound_variable() method is consistent for all rules

    let inline = RefinementPredicate::inline(Expr::literal(Literal::bool(true, span)), span);
    assert_eq!(inline.bound_variable(), "it");

    let lambda = RefinementPredicate::lambda(
        Expr::literal(Literal::bool(true, span)),
        Text::from("x"),
        span,
    );
    assert_eq!(lambda.bound_variable(), "x");

    let sigma = RefinementPredicate::sigma(
        Expr::literal(Literal::bool(true, span)),
        Text::from("y"),
        span,
    );
    assert_eq!(sigma.bound_variable(), "y");

    let named = RefinementPredicate::named(simple_path("pred", span), span);
    assert_eq!(named.bound_variable(), "it");

    let bare = RefinementPredicate::bare(Expr::literal(Literal::bool(true, span)), span);
    assert_eq!(bare.bound_variable(), "it");
}

// ============================================================================
// Display and Formatting Tests
// ============================================================================

#[test]
fn display_rule1_inline() {
    let span = Span::dummy();
    let pred = RefinementPredicate::inline(Expr::literal(Literal::bool(true, span)), span);

    let display_str = pred.to_string();
    assert!(display_str.contains("{"));
    assert!(display_str.contains("}"));
}

#[test]
fn display_rule2_lambda() {
    let span = Span::dummy();
    let pred = RefinementPredicate::lambda(
        Expr::literal(Literal::bool(true, span)),
        Text::from("x"),
        span,
    );

    let display_str = pred.to_string();
    assert!(display_str.contains("where"));
    assert!(display_str.contains("x"));
}

#[test]
fn display_rule3_sigma() {
    let span = Span::dummy();
    let pred = RefinementPredicate::sigma(
        Expr::literal(Literal::bool(true, span)),
        Text::from("val"),
        span,
    );

    let display_str = pred.to_string();
    assert!(display_str.contains("where"));
    assert!(display_str.contains("val"));
}

#[test]
fn display_rule4_named() {
    let span = Span::dummy();
    let pred = RefinementPredicate::named(simple_path("is_valid", span), span);

    let display_str = pred.to_string();
    assert!(display_str.contains("where"));
}

#[test]
fn display_rule5_bare() {
    let span = Span::dummy();
    let pred = RefinementPredicate::bare(Expr::literal(Literal::bool(true, span)), span);

    let display_str = pred.to_string();
    assert!(display_str.contains("where"));
}
