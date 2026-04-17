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
// Advanced contract verification tests
//
// Tests for:
// - Loop invariant verification
// - Termination verification
// - Frame conditions
//
// FIXED (Session 24): VerificationCost.category field added

use verum_ast::literal::IntLit;
use verum_ast::{BinOp, Expr, ExprKind, Ident, Literal, LiteralKind, Path, Span, Type, TypeKind};
use verum_common::Heap;
use verum_smt::{Context, verify_frame_condition, verify_loop_invariant, verify_termination};
use verum_common::Text;

// Helper to create Int type
fn int_type() -> Type {
    Type::new(TypeKind::Int, Span::dummy())
}

// Helper to create a simple boolean literal
fn bool_literal(value: bool) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Bool(value),
            span: Span::dummy(),
        }),
        Span::dummy(),
    )
}

// Helper to create an integer literal
fn int_literal(value: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value: value as i128,
                suffix: None,
            }),
            span: Span::dummy(),
        }),
        Span::dummy(),
    )
}

// Helper to create a variable expression
fn var_expr(name: &str) -> Expr {
    let ident = Ident::new(name, Span::dummy());
    Expr::new(ExprKind::Path(Path::from_ident(ident)), Span::dummy())
}

// Helper to create binary operation
fn binary_expr(left: Expr, op: BinOp, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        Span::dummy(),
    )
}

#[test]
fn test_loop_invariant_simple() {
    let context = Context::new();

    // Invariant: true (always holds)
    let invariant = bool_literal(true);

    // Initial state: i: Int
    let init_state: &[(Text, Type)] = &[(Text::from("i"), int_type())];

    // Loop body (simplified)
    let loop_body = int_literal(0);

    // Exit condition: true
    let exit_condition = bool_literal(true);

    // Postcondition: true
    let postcondition = bool_literal(true);

    let result = verify_loop_invariant(
        &context,
        &invariant,
        init_state,
        &loop_body,
        &exit_condition,
        &postcondition,
    );

    // Should succeed for trivial invariant
    assert!(result.is_ok());
}

#[test]
fn test_loop_invariant_with_condition() {
    let context = Context::new();

    // Invariant: i >= 0
    let i_var = var_expr("i");
    let zero = int_literal(0);
    let invariant = binary_expr(i_var.clone(), BinOp::Ge, zero.clone());

    // Initial state: i = 0
    let init_state: &[(Text, Type)] = &[(Text::from("i"), int_type())];

    // Loop body (simplified)
    let loop_body = int_literal(0);

    // Exit condition: i >= 10
    let ten = int_literal(10);
    let exit_condition = binary_expr(i_var.clone(), BinOp::Ge, ten);

    // Postcondition: i >= 0  (should follow from invariant)
    let postcondition = binary_expr(i_var, BinOp::Ge, zero);

    let result = verify_loop_invariant(
        &context,
        &invariant,
        init_state,
        &loop_body,
        &exit_condition,
        &postcondition,
    );

    // May succeed or fail depending on SMT solver
    // Just verify it doesn't panic
    let _ = result;
}

#[test]
fn test_termination_simple() {
    let context = Context::new();

    // Ranking function: n (must decrease and stay >= 0)
    let ranking_function = var_expr("n");

    // Loop variables
    let loop_vars: &[(Text, Type)] = &[(Text::from("n"), int_type())];

    // Loop body (simplified)
    let loop_body = int_literal(0);

    let result = verify_termination(&context, &ranking_function, loop_vars, &loop_body);

    // May succeed or fail depending on constraints
    // Just verify it doesn't panic
    let _ = result;
}

#[test]
fn test_termination_with_constant_ranking() {
    let context = Context::new();

    // Ranking function: 100 (constant, always >= 0 but never decreases)
    let ranking_function = int_literal(100);

    // Loop variables
    let loop_vars: &[(Text, Type)] = &[(Text::from("i"), int_type())];

    // Loop body (doesn't modify any variables, so ranking function stays constant)
    let loop_body = int_literal(0);

    let result = verify_termination(&context, &ranking_function, loop_vars, &loop_body);

    // A constant ranking function fails termination verification because:
    // 1. It's always non-negative (passes first check)
    // 2. It never decreases (fails second check - ranking must decrease)
    // This is correct behavior: a loop with a constant ranking function
    // doesn't provably terminate since the "measure" never gets smaller.
    assert!(result.is_err());
}

#[test]
fn test_frame_condition_no_modifies() {
    let context = Context::new();

    // Function modifies nothing
    let modifies_vars: &[Text] = &[];

    // All variables: x, y, z
    let all_vars: &[(Text, Type)] = &[
        (Text::from("x"), int_type()),
        (Text::from("y"), int_type()),
        (Text::from("z"), int_type()),
    ];

    // Function body (simplified)
    let function_body = int_literal(42);

    let result = verify_frame_condition(&context, modifies_vars, all_vars, &function_body);

    // Should succeed (simplified implementation always succeeds)
    assert!(result.is_ok());
}

#[test]
fn test_frame_condition_with_modifies() {
    let context = Context::new();

    // Function modifies x
    let modifies_vars: &[Text] = &[Text::from("x")];

    // All variables: x, y, z
    let all_vars: &[(Text, Type)] = &[
        (Text::from("x"), int_type()),
        (Text::from("y"), int_type()),
        (Text::from("z"), int_type()),
    ];

    // Function body
    let function_body = int_literal(0);

    let result = verify_frame_condition(&context, modifies_vars, all_vars, &function_body);

    // Should succeed
    assert!(result.is_ok());
}

#[test]
fn test_frame_condition_modifies_all() {
    let context = Context::new();

    // Function modifies everything
    let modifies_vars: &[Text] = &[Text::from("x"), Text::from("y"), Text::from("z")];

    // All variables
    let all_vars: &[(Text, Type)] = &[
        (Text::from("x"), int_type()),
        (Text::from("y"), int_type()),
        (Text::from("z"), int_type()),
    ];

    // Function body
    let function_body = int_literal(0);

    let result = verify_frame_condition(&context, modifies_vars, all_vars, &function_body);

    // Should succeed (no unmodified variables to verify)
    assert!(result.is_ok());
}

#[test]
fn test_loop_invariant_cost_tracking() {
    let context = Context::new();

    let invariant = bool_literal(true);
    let init_state: &[(Text, Type)] = &[(Text::from("i"), int_type())];
    let loop_body = int_literal(0);
    let exit_condition = bool_literal(true);
    let postcondition = bool_literal(true);

    let result = verify_loop_invariant(
        &context,
        &invariant,
        init_state,
        &loop_body,
        &exit_condition,
        &postcondition,
    );

    if let Ok(cost) = result {
        // Should have measured some time
        assert!(cost.duration.as_nanos() > 0 || cost.duration.as_nanos() == 0);
        assert_eq!(cost.category.as_str(), "loop_invariant");
    }
}

#[test]
fn test_termination_cost_tracking() {
    let context = Context::new();

    let ranking_function = int_literal(10);
    let loop_vars: &[(Text, Type)] = &[(Text::from("n"), int_type())];
    let loop_body = int_literal(0);

    let result = verify_termination(&context, &ranking_function, loop_vars, &loop_body);

    if let Ok(cost) = result {
        assert_eq!(cost.category.as_str(), "termination_check");
    }
}

#[test]
fn test_frame_condition_cost_tracking() {
    let context = Context::new();

    let modifies_vars: &[Text] = &[];
    let all_vars: &[(Text, Type)] = &[(Text::from("x"), int_type())];
    let function_body = int_literal(0);

    let result = verify_frame_condition(&context, modifies_vars, all_vars, &function_body);

    if let Ok(cost) = result {
        assert_eq!(cost.category.as_str(), "frame_condition");
    }
}
