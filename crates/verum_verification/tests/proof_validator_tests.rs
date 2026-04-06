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
//! Proof Validator Tests
//!
//! Comprehensive tests for the proof validation implementations including:
//! - SMT re-checking (recheck_with_smt)
//! - Witness type validation (validate_witness_type)
//! - Type normalization (normalize_types)
//!
//! Implements validation of the formal proof system (Verum 2.0+ planned):
//! - Proof terms are first-class values via Curry-Howard correspondence
//! - Core rules: Axiom, Assumption, ModusPonens, Rewrite, Induction, Lambda, Cases
//! - SMT integration: proofs can be discharged to Z3 (SMT-LIB2 format)
//! - Proof certificates exportable to Dedukti, Coq, Lean, Metamath
//! - Tactics: simp (simplify), ring (normalize ring exprs), omega (linear arith),
//!   blast (tableau prover), auto (proof search with hints database)

use verum_ast::literal::{IntLit, Literal, LiteralKind};
use verum_ast::span::Span;
use verum_ast::{BinOp, Expr, ExprKind, Ident, Path};
use verum_common::Text;
use verum_verification::proof_validator::{
    ExprTypeContext, ProofValidator, TypeVarId, ValidationConfig, ValidationError,
};

// =============================================================================
// Helper Functions for Test Expression Construction
// =============================================================================

fn dummy_span() -> Span {
    Span::dummy()
}

fn make_int_literal(value: i128) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value,
                suffix: None,
            }),
            span: dummy_span(),
        }),
        dummy_span(),
    )
}

fn make_bool_literal(value: bool) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Bool(value),
            span: dummy_span(),
        }),
        dummy_span(),
    )
}

fn make_path_expr(name: &str) -> Expr {
    let path = Path::from_ident(Ident::new(name, dummy_span()));
    Expr::new(ExprKind::Path(path), dummy_span())
}

fn make_binary_expr(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        dummy_span(),
    )
}

fn make_tuple_expr(elements: Vec<Expr>) -> Expr {
    Expr::new(ExprKind::Tuple(elements.into()), dummy_span())
}

// =============================================================================
// Type Normalization Tests
// =============================================================================

#[test]
fn test_normalize_literal_unchanged() {
    let validator = ProofValidator::new();
    let expr = make_int_literal(42);
    let normalized = validator.normalize_expr_for_test(&expr);

    // Literals should remain unchanged
    match &normalized.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Int(i) => assert_eq!(i.value, 42),
            _ => panic!("Expected integer literal"),
        },
        _ => panic!("Expected literal expression"),
    }
}

#[test]
fn test_normalize_path_unchanged() {
    let validator = ProofValidator::new();
    let expr = make_path_expr("x");
    let normalized = validator.normalize_expr_for_test(&expr);

    // Paths should remain unchanged
    match &normalized.kind {
        ExprKind::Path(path) => {
            assert_eq!(path.as_ident().map(|i| i.as_str()), Some("x"));
        }
        _ => panic!("Expected path expression"),
    }
}

#[test]
fn test_normalize_equality_reflexivity() {
    let validator = ProofValidator::new();

    // x == x should normalize to true
    let x = make_path_expr("x");
    let eq_expr = make_binary_expr(BinOp::Eq, x.clone(), x.clone());
    let normalized = validator.normalize_expr_for_test(&eq_expr);

    match &normalized.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Bool(b) => assert!(*b, "x == x should normalize to true"),
            _ => panic!("Expected boolean literal"),
        },
        _ => panic!("Expected literal after reflexivity normalization"),
    }
}

#[test]
fn test_normalize_boolean_and_true_true() {
    let validator = ProofValidator::new();

    // true && true should normalize to true
    let expr = make_binary_expr(BinOp::And, make_bool_literal(true), make_bool_literal(true));
    let normalized = validator.normalize_expr_for_test(&expr);

    match &normalized.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Bool(b) => assert!(*b),
            _ => panic!("Expected boolean literal"),
        },
        _ => panic!("Expected literal after boolean folding"),
    }
}

#[test]
fn test_normalize_boolean_and_false() {
    let validator = ProofValidator::new();

    // false && anything should normalize to false
    let expr = make_binary_expr(BinOp::And, make_bool_literal(false), make_path_expr("x"));
    let normalized = validator.normalize_expr_for_test(&expr);

    match &normalized.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Bool(b) => assert!(!*b),
            _ => panic!("Expected boolean literal"),
        },
        _ => panic!("Expected literal after short-circuit evaluation"),
    }
}

#[test]
fn test_normalize_boolean_or_true() {
    let validator = ProofValidator::new();

    // true || anything should normalize to true
    let expr = make_binary_expr(BinOp::Or, make_bool_literal(true), make_path_expr("x"));
    let normalized = validator.normalize_expr_for_test(&expr);

    match &normalized.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Bool(b) => assert!(*b),
            _ => panic!("Expected boolean literal"),
        },
        _ => panic!("Expected literal after short-circuit evaluation"),
    }
}

#[test]
fn test_normalize_integer_addition() {
    let validator = ProofValidator::new();

    // 2 + 3 should normalize to 5
    let expr = make_binary_expr(BinOp::Add, make_int_literal(2), make_int_literal(3));
    let normalized = validator.normalize_expr_for_test(&expr);

    match &normalized.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Int(i) => assert_eq!(i.value, 5),
            _ => panic!("Expected integer literal"),
        },
        _ => panic!("Expected literal after integer folding"),
    }
}

#[test]
fn test_normalize_integer_comparison() {
    let validator = ProofValidator::new();

    // 5 > 3 should normalize to true
    let expr = make_binary_expr(BinOp::Gt, make_int_literal(5), make_int_literal(3));
    let normalized = validator.normalize_expr_for_test(&expr);

    match &normalized.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Bool(b) => assert!(*b),
            _ => panic!("Expected boolean literal"),
        },
        _ => panic!("Expected literal after comparison folding"),
    }
}

#[test]
fn test_normalize_tuple() {
    let validator = ProofValidator::new();

    // (1 + 2, true && true) should normalize to (3, true)
    let expr = make_tuple_expr(vec![
        make_binary_expr(BinOp::Add, make_int_literal(1), make_int_literal(2)),
        make_binary_expr(BinOp::And, make_bool_literal(true), make_bool_literal(true)),
    ]);
    let normalized = validator.normalize_expr_for_test(&expr);

    match &normalized.kind {
        ExprKind::Tuple(elements) => {
            assert_eq!(elements.len(), 2);

            // First element should be 3
            match &elements[0].kind {
                ExprKind::Literal(lit) => match &lit.kind {
                    LiteralKind::Int(i) => assert_eq!(i.value, 3),
                    _ => panic!("Expected integer in tuple"),
                },
                _ => panic!("Expected literal in tuple"),
            }

            // Second element should be true
            match &elements[1].kind {
                ExprKind::Literal(lit) => match &lit.kind {
                    LiteralKind::Bool(b) => assert!(*b),
                    _ => panic!("Expected boolean in tuple"),
                },
                _ => panic!("Expected literal in tuple"),
            }
        }
        _ => panic!("Expected tuple expression"),
    }
}

#[test]
fn test_normalize_nested_equality() {
    let validator = ProofValidator::new();

    // (x == x) && (y == y) should normalize to true && true = true
    let x_eq_x = make_binary_expr(BinOp::Eq, make_path_expr("x"), make_path_expr("x"));
    let y_eq_y = make_binary_expr(BinOp::Eq, make_path_expr("y"), make_path_expr("y"));
    let expr = make_binary_expr(BinOp::And, x_eq_x, y_eq_y);

    let normalized = validator.normalize_expr_for_test(&expr);

    match &normalized.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Bool(b) => assert!(*b),
            _ => panic!("Expected boolean literal"),
        },
        _ => panic!("Expected literal after nested normalization"),
    }
}

// =============================================================================
// Witness Type Validation Tests
// =============================================================================

#[test]
fn test_witness_type_int_literal() {
    let validator = ProofValidator::new();

    let witness = make_int_literal(42);
    let expected_type = make_path_expr("Int");

    // Should succeed - integer literal has type Int
    let result = validator.validate_witness_type_for_test(&witness, &expected_type);
    assert!(result.is_ok(), "Integer literal should have type Int");
}

#[test]
fn test_witness_type_bool_literal() {
    let validator = ProofValidator::new();

    let witness = make_bool_literal(true);
    let expected_type = make_path_expr("Bool");

    // Should succeed - boolean literal has type Bool
    let result = validator.validate_witness_type_for_test(&witness, &expected_type);
    assert!(result.is_ok(), "Boolean literal should have type Bool");
}

#[test]
fn test_witness_type_mismatch() {
    let validator = ProofValidator::new();

    let witness = make_int_literal(42);
    let expected_type = make_path_expr("Bool");

    // Should fail - integer literal does not have type Bool
    let result = validator.validate_witness_type_for_test(&witness, &expected_type);
    assert!(result.is_err(), "Integer literal should not have type Bool");
}

#[test]
fn test_witness_type_comparison_result() {
    let validator = ProofValidator::new();

    // x > 0 has type Bool
    let witness = make_binary_expr(BinOp::Gt, make_path_expr("x"), make_int_literal(0));
    let expected_type = make_path_expr("Bool");

    let result = validator.validate_witness_type_for_test(&witness, &expected_type);
    assert!(result.is_ok(), "Comparison should have type Bool");
}

#[test]
fn test_witness_type_arithmetic_result() {
    let validator = ProofValidator::new();

    // x + 1 has type Int (if x is Int)
    let witness = make_binary_expr(BinOp::Add, make_int_literal(5), make_int_literal(1));
    let expected_type = make_path_expr("Int");

    let result = validator.validate_witness_type_for_test(&witness, &expected_type);
    assert!(result.is_ok(), "Arithmetic on ints should have type Int");
}

#[test]
fn test_witness_type_tuple() {
    let validator = ProofValidator::new();

    // (42, true) should have type (Int, Bool)
    let witness = make_tuple_expr(vec![make_int_literal(42), make_bool_literal(true)]);
    let expected_type = make_tuple_expr(vec![make_path_expr("Int"), make_path_expr("Bool")]);

    let result = validator.validate_witness_type_for_test(&witness, &expected_type);
    assert!(result.is_ok(), "Tuple types should match");
}

#[test]
fn test_witness_type_unknown_compatible() {
    let validator = ProofValidator::new();

    // Unknown type should be compatible with anything
    let witness = make_path_expr("some_var");
    let expected_type = make_path_expr("_Unknown");

    let result = validator.validate_witness_type_for_test(&witness, &expected_type);
    assert!(result.is_ok(), "Unknown type should be compatible");
}

#[test]
fn test_witness_type_numeric_coercion() {
    let validator = ProofValidator::new();

    // Int can coerce to Float in some contexts
    let witness = make_int_literal(42);
    let expected_type = make_path_expr("Float");

    let result = validator.validate_witness_type_for_test(&witness, &expected_type);
    assert!(result.is_ok(), "Int should coerce to Float");
}

// =============================================================================
// SMT Re-checking Tests
// =============================================================================

#[test]
fn test_recheck_with_smt_valid_tautology() {
    let validator = ProofValidator::new();

    // true is always valid
    let formula = make_bool_literal(true);

    let result = validator.recheck_with_smt_for_test("z3", &formula);
    assert!(result.is_ok(), "Tautology should be valid: {:?}", result);
}

#[test]
fn test_recheck_with_smt_invalid_contradiction() {
    let validator = ProofValidator::new();

    // false is never valid
    let formula = make_bool_literal(false);

    let result = validator.recheck_with_smt_for_test("z3", &formula);
    assert!(result.is_err(), "Contradiction should be invalid");
}

#[test]
fn test_recheck_with_smt_simple_equality() {
    let validator = ProofValidator::new();

    // x == x is always valid (reflexivity)
    let x = make_path_expr("x");
    let formula = make_binary_expr(BinOp::Eq, x.clone(), x.clone());

    let result = validator.recheck_with_smt_for_test("z3", &formula);
    assert!(
        result.is_ok(),
        "Reflexive equality should be valid: {:?}",
        result
    );
}

#[test]
fn test_recheck_with_smt_unsupported_solver() {
    let validator = ProofValidator::new();

    let formula = make_bool_literal(true);

    // Using unsupported solver should fail
    let result = validator.recheck_with_smt_for_test("cvc5", &formula);
    assert!(result.is_err(), "Unsupported solver should fail");

    match result {
        Err(ValidationError::SmtValidationFailed { reason }) => {
            assert!(
                reason.contains("Unsupported solver"),
                "Error should mention unsupported solver"
            );
        }
        _ => panic!("Expected SmtValidationFailed error"),
    }
}

#[test]
fn test_recheck_with_smt_arithmetic_valid() {
    let validator = ProofValidator::new();

    // 2 + 2 == 4 is valid
    let two_plus_two = make_binary_expr(BinOp::Add, make_int_literal(2), make_int_literal(2));
    let four = make_int_literal(4);
    let formula = make_binary_expr(BinOp::Eq, two_plus_two, four);

    let result = validator.recheck_with_smt_for_test("z3", &formula);
    assert!(result.is_ok(), "2 + 2 == 4 should be valid: {:?}", result);
}

#[test]
fn test_recheck_with_smt_arithmetic_invalid() {
    let validator = ProofValidator::new();

    // 2 + 2 == 5 is invalid
    let two_plus_two = make_binary_expr(BinOp::Add, make_int_literal(2), make_int_literal(2));
    let five = make_int_literal(5);
    let formula = make_binary_expr(BinOp::Eq, two_plus_two, five);

    let result = validator.recheck_with_smt_for_test("z3", &formula);
    assert!(result.is_err(), "2 + 2 == 5 should be invalid");
}

#[test]
fn test_recheck_with_smt_logical_and() {
    let validator = ProofValidator::new();

    // true && true is valid
    let formula = make_binary_expr(BinOp::And, make_bool_literal(true), make_bool_literal(true));

    let result = validator.recheck_with_smt_for_test("z3", &formula);
    assert!(result.is_ok(), "true && true should be valid");
}

#[test]
fn test_recheck_with_smt_logical_or() {
    let validator = ProofValidator::new();

    // x || !x is always true (excluded middle)
    // We use true || false as a simpler test
    let formula = make_binary_expr(BinOp::Or, make_bool_literal(true), make_bool_literal(false));

    let result = validator.recheck_with_smt_for_test("z3", &formula);
    assert!(result.is_ok(), "true || false should be valid");
}

// =============================================================================
// Integration Tests
// =============================================================================

#[test]
fn test_validator_config_timeout() {
    let config = ValidationConfig {
        smt_timeout_ms: 10000,
        ..Default::default()
    };
    let validator = ProofValidator::with_config(config);

    // Validator should be created successfully with custom config
    assert!(validator.validator_stats().proofs_validated == 0);
}

#[test]
fn test_validator_hypothesis_context() {
    let mut validator = ProofValidator::new();

    // Register a hypothesis
    validator.register_hypothesis("P", make_bool_literal(true));

    // The hypothesis context should contain P
    // (We can't directly test private fields, but we can verify through behavior)
}

#[test]
fn test_validator_axiom_registration() {
    let mut validator = ProofValidator::new();

    // Register an axiom - using a proper Expr instead of string
    let axiom_formula = make_bool_literal(true);
    validator.register_axiom("reflexivity", axiom_formula);

    // The axiom should be registered
    // (Verified through behavior in other tests)
}

// =============================================================================
// Type Context Tests
// =============================================================================

#[test]
fn test_type_context_creation() {
    let validator = ProofValidator::new();

    // New validator should have empty type context
    let ctx = validator.type_context();
    assert_eq!(ctx.stats().vars_created, 0);
    assert_eq!(ctx.stats().vars_resolved, 0);
    assert_eq!(ctx.stats().fallbacks_used, 0);
}

#[test]
fn test_type_context_stats_after_operations() {
    let mut validator = ProofValidator::new();

    // Access the type context and create some type variables
    let ctx = validator.type_context_mut();

    // Fresh var should increment vars_created
    let var_id = ctx.fresh_var("test origin", dummy_span(), verum_common::Maybe::None);
    assert_eq!(ctx.stats().vars_created, 1);

    // Create an expression to bind
    let bind_expr = make_path_expr("Int");

    // Binding should increment vars_resolved
    let result = ctx.bind(var_id, bind_expr);
    assert!(result.is_ok());
    assert_eq!(ctx.stats().vars_resolved, 1);

    // Record fallback should increment fallbacks_used
    ctx.record_fallback();
    assert_eq!(ctx.stats().fallbacks_used, 1);
}

#[test]
fn test_type_context_unification_same_path() {
    let mut validator = ProofValidator::new();
    let ctx = validator.type_context_mut();

    let int1 = make_path_expr("Int");
    let int2 = make_path_expr("Int");

    // Same paths should unify successfully
    let result = ctx.unify(&int1, &int2, dummy_span());
    assert!(result.is_ok());
}

#[test]
fn test_type_context_unification_tuples() {
    let mut validator = ProofValidator::new();
    let ctx = validator.type_context_mut();

    let tuple1 = make_tuple_expr(vec![make_path_expr("Int"), make_path_expr("Bool")]);
    let tuple2 = make_tuple_expr(vec![make_path_expr("Int"), make_path_expr("Bool")]);

    // Same tuples should unify
    let result = ctx.unify(&tuple1, &tuple2, dummy_span());
    assert!(result.is_ok());
}

#[test]
fn test_type_context_unification_tuple_size_mismatch() {
    let mut validator = ProofValidator::new();
    let ctx = validator.type_context_mut();

    let tuple2 = make_tuple_expr(vec![make_path_expr("Int"), make_path_expr("Bool")]);
    let tuple3 = make_tuple_expr(vec![
        make_path_expr("Int"),
        make_path_expr("Bool"),
        make_path_expr("Char"),
    ]);

    // Different sized tuples should fail to unify
    let result = ctx.unify(&tuple2, &tuple3, dummy_span());
    assert!(result.is_err());
}

#[test]
fn test_type_context_variable_binding() {
    let mut validator = ProofValidator::new();
    let ctx = validator.type_context_mut();

    // Create a fresh type variable
    let var_id = ctx.fresh_var("test var", dummy_span(), verum_common::Maybe::Some(0));

    // Initially unbound
    assert!(ctx.resolve(var_id).is_none());

    // Bind it to Int
    let int_expr = make_path_expr("Int");
    let result = ctx.bind(var_id, int_expr.clone());
    assert!(result.is_ok());

    // Now it should resolve to Int
    let resolved = ctx.resolve(var_id);
    assert!(resolved.is_some());
}

#[test]
fn test_type_context_double_bind_same_type() {
    let mut validator = ProofValidator::new();
    let ctx = validator.type_context_mut();

    // Create and bind a type variable
    let var_id = ctx.fresh_var("test var", dummy_span(), verum_common::Maybe::None);
    let int_expr = make_path_expr("Int");

    // First bind should succeed
    let result1 = ctx.bind(var_id, int_expr.clone());
    assert!(result1.is_ok());

    // Second bind with same type should also succeed
    let result2 = ctx.bind(var_id, int_expr);
    assert!(result2.is_ok());
}

#[test]
fn test_type_context_double_bind_different_type() {
    let mut validator = ProofValidator::new();
    let ctx = validator.type_context_mut();

    // Create and bind a type variable
    let var_id = ctx.fresh_var("test var", dummy_span(), verum_common::Maybe::None);
    let int_expr = make_path_expr("Int");
    let bool_expr = make_path_expr("Bool");

    // First bind should succeed
    let result1 = ctx.bind(var_id, int_expr);
    assert!(result1.is_ok());

    // Second bind with different type should fail
    let result2 = ctx.bind(var_id, bool_expr);
    assert!(result2.is_err());
}

#[test]
fn test_type_var_expr_format() {
    let mut validator = ProofValidator::new();
    let ctx = validator.type_context_mut();

    // Create a type variable
    let var_id = ctx.fresh_var("test", dummy_span(), verum_common::Maybe::None);

    // Create an expression for it
    let expr = ctx.make_type_var_expr(var_id, dummy_span());

    // Should be a path expression
    match &expr.kind {
        ExprKind::Path(path) => {
            if let Some(ident) = path.as_ident() {
                let name = ident.as_str();
                // Name should start with ?T
                assert!(
                    name.starts_with("?T"),
                    "Type var name should start with ?T, got: {}",
                    name
                );
            } else {
                panic!("Expected simple identifier path");
            }
        }
        _ => panic!("Expected path expression for type variable"),
    }
}
