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
// Unit tests for translate.rs
//
// Migrated from src/translate.rs to comply with CLAUDE.md test organization.

use verum_ast::{BinOp, Expr, ExprKind, Literal, Span, Type, UnOp};
use verum_common::Heap;
use verum_smt::context::Context;
use verum_smt::translate::*;
use z3::ast::{Ast, Dynamic, Int};

// Helper to create a verum_smt Context
fn smt_context() -> Context {
    Context::new()
}

#[test]
fn test_translate_literal_int() {
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    let span = Span::dummy();
    let lit = Literal::int(42, span);
    let expr = Expr::literal(lit);

    let result = translator.translate_expr(&expr);
    assert!(result.is_ok());
    let z3_expr = result.unwrap();
    assert!(z3_expr.as_int().is_some());
}

#[test]
fn test_translate_literal_bool() {
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    let span = Span::dummy();
    let lit = Literal::bool(true, span);
    let expr = Expr::literal(lit);

    let result = translator.translate_expr(&expr);
    assert!(result.is_ok());
    let z3_expr = result.unwrap();
    assert!(z3_expr.as_bool().is_some());
}

#[test]
fn test_translate_literal_float() {
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    let span = Span::dummy();
    let lit = Literal::float(2.5, span);
    let expr = Expr::literal(lit);

    let result = translator.translate_expr(&expr);
    assert!(result.is_ok());
    let z3_expr = result.unwrap();
    assert!(z3_expr.as_real().is_some());
}

#[test]
fn test_translate_binary_add() {
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    let span = Span::dummy();
    let left = Heap::new(Expr::literal(Literal::int(1, span)));
    let right = Heap::new(Expr::literal(Literal::int(2, span)));
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            right,
        },
        span,
    );

    let result = translator.translate_expr(&expr);
    assert!(result.is_ok());
}

#[test]
fn test_translate_binary_comparison() {
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    let span = Span::dummy();
    let left = Heap::new(Expr::literal(Literal::int(5, span)));
    let right = Heap::new(Expr::literal(Literal::int(10, span)));
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Lt,
            left,
            right,
        },
        span,
    );

    let result = translator.translate_expr(&expr);
    assert!(result.is_ok());
    let z3_expr = result.unwrap();
    assert!(z3_expr.as_bool().is_some());
}

#[test]
fn test_translate_unary_negation() {
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    let span = Span::dummy();
    let expr = Expr::new(
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: Heap::new(Expr::literal(Literal::int(42, span))),
        },
        span,
    );

    let result = translator.translate_expr(&expr);
    assert!(result.is_ok());
}

#[test]
fn test_translate_unary_not() {
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    let span = Span::dummy();
    let expr = Expr::new(
        ExprKind::Unary {
            op: UnOp::Not,
            expr: Heap::new(Expr::literal(Literal::bool(true, span))),
        },
        span,
    );

    let result = translator.translate_expr(&expr);
    assert!(result.is_ok());
    let z3_expr = result.unwrap();
    assert!(z3_expr.as_bool().is_some());
}

#[test]
fn test_variable_binding() {
    let ctx = smt_context();
    let mut translator = Translator::new(&ctx);

    let int_var = Int::new_const("x");
    translator.bind("x".to_string().into(), Dynamic::from_ast(&int_var));

    assert!(translator.contains("x"));
    assert!(translator.get("x").is_some());
}

#[test]
fn test_create_var_int() {
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    let span = Span::dummy();
    let ty = Type::int(span);

    let result = translator.create_var("test_var", &ty);
    assert!(result.is_ok());
    let var = result.unwrap();
    assert!(var.as_int().is_some());
}

#[test]
fn test_create_var_bool() {
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    let span = Span::dummy();
    let ty = Type::bool(span);

    let result = translator.create_var("test_bool", &ty);
    assert!(result.is_ok());
    let var = result.unwrap();
    assert!(var.as_bool().is_some());
}

#[test]
fn test_translate_logical_and() {
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    let span = Span::dummy();
    let left = Heap::new(Expr::literal(Literal::bool(true, span)));
    let right = Heap::new(Expr::literal(Literal::bool(false, span)));
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::And,
            left,
            right,
        },
        span,
    );

    let result = translator.translate_expr(&expr);
    assert!(result.is_ok());
    let z3_expr = result.unwrap();
    assert!(z3_expr.as_bool().is_some());
}

#[test]
fn test_translate_complex_expr() {
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    let span = Span::dummy();
    // Build: (5 + 3) > 7
    let add_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::literal(Literal::int(5, span))),
            right: Heap::new(Expr::literal(Literal::int(3, span))),
        },
        span,
    );

    let compare_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Gt,
            left: Heap::new(add_expr),
            right: Heap::new(Expr::literal(Literal::int(7, span))),
        },
        span,
    );

    let result = translator.translate_expr(&compare_expr);
    assert!(result.is_ok());
    let z3_expr = result.unwrap();
    assert!(z3_expr.as_bool().is_some());
}

// ==================== TranslatorExt Tests ====================
//
// Tests for the TranslatorExt trait that provides scope management
// for dependent type checking.
//
// Pi type verification: `(x: A) -> B(x)` — return type depends on input value,
//   translated to Z3 forall_const() with pattern-guided instantiation.
// Sigma type verification: `(x: A, B(x))` — second component depends on first,
//   translated to Z3 exists_const() for existential quantification.

use verum_smt::dependent::TranslatorExt;
use verum_common::Text;

#[test]
fn test_clone_for_scope_empty() {
    // Test that clone_for_scope works with an empty translator
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    let scoped = translator.clone_for_scope();

    // The scoped translator should have no bindings
    assert_eq!(scoped.binding_count(), 0);
}

#[test]
fn test_clone_for_scope_with_bindings() {
    // Test that clone_for_scope copies all bindings
    let ctx = smt_context();
    let mut translator = Translator::new(&ctx);

    // Add some bindings
    let x_var = Int::new_const("x");
    let y_var = Int::new_const("y");
    translator.bind(Text::from("x"), Dynamic::from_ast(&x_var));
    translator.bind(Text::from("y"), Dynamic::from_ast(&y_var));

    // Clone for new scope
    let scoped = translator.clone_for_scope();

    // Both bindings should be present in the cloned translator
    assert!(scoped.contains("x"));
    assert!(scoped.contains("y"));
    assert_eq!(scoped.binding_count(), 2);
}

#[test]
fn test_clone_for_scope_independence() {
    // Test that modifying the scoped translator doesn't affect the original
    let ctx = smt_context();
    let mut translator = Translator::new(&ctx);

    // Add initial binding
    let x_var = Int::new_const("x");
    translator.bind(Text::from("x"), Dynamic::from_ast(&x_var));

    // Clone for new scope and add more bindings
    let mut scoped = translator.clone_for_scope();
    let z_var = Int::new_const("z");
    scoped.bind(Text::from("z"), Dynamic::from_ast(&z_var));

    // Original should only have x
    assert!(translator.contains("x"));
    assert!(!translator.contains("z"));
    assert_eq!(translator.binding_count(), 1);

    // Scoped should have both x and z
    assert!(scoped.contains("x"));
    assert!(scoped.contains("z"));
    assert_eq!(scoped.binding_count(), 2);
}

#[test]
fn test_with_binding() {
    // Test the convenience method for creating a scope with one new binding
    let ctx = smt_context();
    let mut translator = Translator::new(&ctx);

    // Add initial binding
    let x_var = Int::new_const("x");
    translator.bind(Text::from("x"), Dynamic::from_ast(&x_var));

    // Create new scope with additional binding
    let n_var = Int::new_const("n");
    let scoped = translator.with_binding(Text::from("n"), Dynamic::from_ast(&n_var));

    // Original should only have x
    assert!(translator.contains("x"));
    assert!(!translator.contains("n"));

    // Scoped should have both x and n
    assert!(scoped.contains("x"));
    assert!(scoped.contains("n"));
}

#[test]
fn test_with_bindings_multiple() {
    // Test creating a scope with multiple new bindings at once
    let ctx = smt_context();
    let mut translator = Translator::new(&ctx);

    // Add initial binding
    let x_var = Int::new_const("x");
    translator.bind(Text::from("x"), Dynamic::from_ast(&x_var));

    // Create new scope with multiple bindings
    let a_var = Int::new_const("a");
    let b_var = Int::new_const("b");
    let c_var = Int::new_const("c");

    let bindings = vec![
        (Text::from("a"), Dynamic::from_ast(&a_var)),
        (Text::from("b"), Dynamic::from_ast(&b_var)),
        (Text::from("c"), Dynamic::from_ast(&c_var)),
    ];

    let scoped = translator.with_bindings(bindings);

    // Original should only have x
    assert_eq!(translator.binding_count(), 1);

    // Scoped should have x, a, b, c
    assert!(scoped.contains("x"));
    assert!(scoped.contains("a"));
    assert!(scoped.contains("b"));
    assert!(scoped.contains("c"));
    assert_eq!(scoped.binding_count(), 4);
}

#[test]
fn test_binding_shadowing_in_scopes() {
    // Test that bindings can shadow outer bindings in inner scopes
    let ctx = smt_context();
    let mut translator = Translator::new(&ctx);

    // Add initial binding for x with value 10
    let x_outer = Int::from_i64(10);
    translator.bind(Text::from("x"), Dynamic::from_ast(&x_outer));

    // Create new scope and shadow x with a new value
    let x_inner = Int::from_i64(20);
    let scoped = translator.with_binding(Text::from("x"), Dynamic::from_ast(&x_inner));

    // Both should have x, but with different values
    assert!(translator.contains("x"));
    assert!(scoped.contains("x"));

    // Verify values are different (outer still has its value)
    let outer_x = translator.get("x").unwrap();
    let inner_x = scoped.get("x").unwrap();

    // They should both be Int types but reference different Z3 AST nodes
    assert!(outer_x.as_int().is_some());
    assert!(inner_x.as_int().is_some());
}

#[test]
fn test_nested_scopes() {
    // Test multiple levels of scope nesting
    let ctx = smt_context();
    let mut translator = Translator::new(&ctx);

    // Outer scope: x
    let x_var = Int::new_const("x");
    translator.bind(Text::from("x"), Dynamic::from_ast(&x_var));

    // Middle scope: x, y
    let y_var = Int::new_const("y");
    let mut middle_scope = translator.clone_for_scope();
    middle_scope.bind(Text::from("y"), Dynamic::from_ast(&y_var));

    // Inner scope: x, y, z
    let z_var = Int::new_const("z");
    let inner_scope = middle_scope.with_binding(Text::from("z"), Dynamic::from_ast(&z_var));

    // Verify each scope has the correct bindings
    assert_eq!(translator.binding_count(), 1);
    assert!(translator.contains("x"));

    assert_eq!(middle_scope.binding_count(), 2);
    assert!(middle_scope.contains("x"));
    assert!(middle_scope.contains("y"));

    assert_eq!(inner_scope.binding_count(), 3);
    assert!(inner_scope.contains("x"));
    assert!(inner_scope.contains("y"));
    assert!(inner_scope.contains("z"));
}

#[test]
fn test_binding_names_iterator() {
    // Test that binding_names returns all bound variable names
    let ctx = smt_context();
    let mut translator = Translator::new(&ctx);

    let a_var = Int::new_const("a");
    let b_var = Int::new_const("b");
    let c_var = Int::new_const("c");

    translator.bind(Text::from("a"), Dynamic::from_ast(&a_var));
    translator.bind(Text::from("b"), Dynamic::from_ast(&b_var));
    translator.bind(Text::from("c"), Dynamic::from_ast(&c_var));

    let names: Vec<_> = translator.binding_names().collect();

    assert_eq!(names.len(), 3);
    // Check all names are present (order may vary since Map doesn't guarantee order)
    assert!(names.contains(&Text::from("a")));
    assert!(names.contains(&Text::from("b")));
    assert!(names.contains(&Text::from("c")));
}

#[test]
fn test_clear_bindings() {
    // Test that clear_bindings removes all bindings
    let ctx = smt_context();
    let mut translator = Translator::new(&ctx);

    let a_var = Int::new_const("a");
    let b_var = Int::new_const("b");

    translator.bind(Text::from("a"), Dynamic::from_ast(&a_var));
    translator.bind(Text::from("b"), Dynamic::from_ast(&b_var));

    assert_eq!(translator.binding_count(), 2);

    translator.clear_bindings();

    assert_eq!(translator.binding_count(), 0);
    assert!(!translator.contains("a"));
    assert!(!translator.contains("b"));
}

#[test]
fn test_scope_for_pi_type_simulation() {
    // Simulate Pi type verification: (n: Int) -> Int{> n}
    // This tests the typical use case for dependent type checking
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Create a fresh variable for the parameter 'n'
    let n_var = Int::new_const("n");

    // Create a scoped translator with 'n' bound
    let scoped = translator.with_binding(Text::from("n"), Dynamic::from_ast(&n_var));

    // In the scoped context, we can translate expressions that reference 'n'
    assert!(scoped.contains("n"));

    // The outer scope should not have 'n'
    assert!(!translator.contains("n"));
}

#[test]
fn test_scope_for_sigma_type_simulation() {
    // Simulate Sigma type verification: (fst: Int{> 0}, snd: Int{> fst})
    // This tests the pattern of binding first component before checking second
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Create a fresh variable for the first component 'fst'
    let fst_var = Int::new_const("fst");

    // Create a scoped translator with 'fst' bound
    let scoped = translator.with_binding(Text::from("fst"), Dynamic::from_ast(&fst_var));

    // Now in the scoped context, we can verify the second type which references 'fst'
    assert!(scoped.contains("fst"));

    // Create 'snd' and verify it can be checked in context where 'fst' is bound
    let snd_var = Int::new_const("snd");
    let inner_scoped = scoped.with_binding(Text::from("snd"), Dynamic::from_ast(&snd_var));

    assert!(inner_scoped.contains("fst"));
    assert!(inner_scoped.contains("snd"));
}

// ==================== IEEE 754 Floating-Point Theory Tests ====================
//
// Tests for precise FPA (Floating-Point Arithmetic) support.
// These tests verify the IEEE 754 compliant translation using Z3's FPA theory.

use z3::SatResult;

#[test]
fn test_translation_config_default() {
    let config = TranslationConfig::default();
    assert!(!config.precise_floats);
    assert_eq!(config.default_rounding_mode, FloatRoundingMode::NearestTiesToEven);
    assert_eq!(config.float_precision, FloatPrecision::Float64);
}

#[test]
fn test_translation_config_precise_floats() {
    let config = TranslationConfig::with_precise_floats();
    assert!(config.precise_floats);
    assert_eq!(config.default_rounding_mode, FloatRoundingMode::NearestTiesToEven);
    assert_eq!(config.float_precision, FloatPrecision::Float64);
}

#[test]
fn test_translation_config_builder_pattern() {
    let config = TranslationConfig::with_precise_floats()
        .with_rounding_mode(FloatRoundingMode::TowardZero)
        .with_precision(FloatPrecision::Float32);

    assert!(config.precise_floats);
    assert_eq!(config.default_rounding_mode, FloatRoundingMode::TowardZero);
    assert_eq!(config.float_precision, FloatPrecision::Float32);
}

#[test]
fn test_translator_with_config() {
    let ctx = smt_context();
    let config = TranslationConfig::with_precise_floats();
    let translator = Translator::with_config(&ctx, config);

    assert!(translator.uses_precise_floats());
}

#[test]
fn test_translate_float_literal_approximate_mode() {
    // Default mode: floats translated to Real
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    let span = Span::dummy();
    let lit = Literal::float(2.5, span);
    let expr = Expr::literal(lit);

    let result = translator.translate_expr(&expr);
    assert!(result.is_ok());
    let z3_expr = result.unwrap();
    // In approximate mode, floats become Reals
    assert!(z3_expr.as_real().is_some());
}

#[test]
fn test_translate_float_literal_precise_mode() {
    // Precise mode: floats translated to FPA
    let ctx = smt_context();
    let config = TranslationConfig::with_precise_floats();
    let translator = Translator::with_config(&ctx, config);

    let span = Span::dummy();
    let lit = Literal::float(2.5, span);
    let expr = Expr::literal(lit);

    let result = translator.translate_expr(&expr);
    assert!(result.is_ok());

    // The result should be a Float (FPA) in precise mode
    // We check this by verifying the sort kind
    let z3_expr = result.unwrap();
    let sort = z3_expr.get_sort();
    assert_eq!(sort.kind(), z3::SortKind::FloatingPoint);
}

#[test]
fn test_float_binary_operations_approximate() {
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    let span = Span::dummy();
    let left = Heap::new(Expr::literal(Literal::float(1.5, span)));
    let right = Heap::new(Expr::literal(Literal::float(2.5, span)));

    let add_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            right,
        },
        span,
    );

    let result = translator.translate_expr(&add_expr);
    assert!(result.is_ok());
    // In approximate mode, result is Real
    assert!(result.unwrap().as_real().is_some());
}

#[test]
fn test_float_binary_operations_precise() {
    let ctx = smt_context();
    let config = TranslationConfig::with_precise_floats();
    let translator = Translator::with_config(&ctx, config);

    let span = Span::dummy();
    let left = Heap::new(Expr::literal(Literal::float(1.5, span)));
    let right = Heap::new(Expr::literal(Literal::float(2.5, span)));

    // Test all arithmetic operations
    for op in [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div] {
        let expr = Expr::new(
            ExprKind::Binary {
                op,
                left: left.clone(),
                right: right.clone(),
            },
            span,
        );

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed for operation: {:?}", op);

        // In precise mode, result should be FPA
        let z3_expr = result.unwrap();
        let sort = z3_expr.get_sort();
        assert_eq!(sort.kind(), z3::SortKind::FloatingPoint, "Wrong sort for operation: {:?}", op);
    }
}

#[test]
fn test_float_comparison_operations_precise() {
    let ctx = smt_context();
    let config = TranslationConfig::with_precise_floats();
    let translator = Translator::with_config(&ctx, config);

    let span = Span::dummy();
    let left = Heap::new(Expr::literal(Literal::float(1.5, span)));
    let right = Heap::new(Expr::literal(Literal::float(2.5, span)));

    // Test all comparison operations
    for op in [BinOp::Lt, BinOp::Le, BinOp::Gt, BinOp::Ge, BinOp::Eq, BinOp::Ne] {
        let expr = Expr::new(
            ExprKind::Binary {
                op,
                left: left.clone(),
                right: right.clone(),
            },
            span,
        );

        let result = translator.translate_expr(&expr);
        assert!(result.is_ok(), "Failed for comparison: {:?}", op);

        // Comparisons always return Bool
        let z3_expr = result.unwrap();
        assert!(z3_expr.as_bool().is_some(), "Comparison should return Bool for: {:?}", op);
    }
}

#[test]
fn test_float_negation_precise() {
    let ctx = smt_context();
    let config = TranslationConfig::with_precise_floats();
    let translator = Translator::with_config(&ctx, config);

    let span = Span::dummy();
    let expr = Expr::new(
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: Heap::new(Expr::literal(Literal::float(42.0, span))),
        },
        span,
    );

    let result = translator.translate_expr(&expr);
    assert!(result.is_ok());

    // Negation of float should return float in precise mode
    let z3_expr = result.unwrap();
    let sort = z3_expr.get_sort();
    assert_eq!(sort.kind(), z3::SortKind::FloatingPoint);
}

#[test]
fn test_float_const_creation_precise() {
    let ctx = smt_context();
    let config = TranslationConfig::with_precise_floats();
    let translator = Translator::with_config(&ctx, config);

    let result = translator.new_float_const("x");
    assert!(result.is_ok());

    // Check that the constant has the correct sort
    let float_const = result.unwrap();
    let sort = float_const.get_sort();
    assert_eq!(sort.kind(), z3::SortKind::FloatingPoint);
}

#[test]
fn test_float_const_creation_fails_in_approximate_mode() {
    let ctx = smt_context();
    let translator = Translator::new(&ctx); // Default: approximate mode

    let result = translator.new_float_const("x");
    assert!(result.is_err());
}

#[test]
fn test_float_nan_creation() {
    let ctx = smt_context();
    let config = TranslationConfig::with_precise_floats();
    let translator = Translator::with_config(&ctx, config);

    let nan = translator.float_nan();
    assert!(nan.is_ok());
}

#[test]
fn test_float_infinity_creation() {
    let ctx = smt_context();
    let config = TranslationConfig::with_precise_floats();
    let translator = Translator::with_config(&ctx, config);

    let pos_inf = translator.float_positive_infinity();
    let neg_inf = translator.float_negative_infinity();

    assert!(pos_inf.is_ok());
    assert!(neg_inf.is_ok());
}

#[test]
fn test_float_precision_settings() {
    // Test Float32 precision
    let config32 = TranslationConfig::with_precise_floats()
        .with_precision(FloatPrecision::Float32);

    let (ebits, sbits) = config32.float_precision.bit_widths();
    assert_eq!(ebits, 8);
    assert_eq!(sbits, 24);

    // Test Float64 precision
    let config64 = TranslationConfig::with_precise_floats()
        .with_precision(FloatPrecision::Float64);

    let (ebits, sbits) = config64.float_precision.bit_widths();
    assert_eq!(ebits, 11);
    assert_eq!(sbits, 53);
}

#[test]
fn test_rounding_mode_conversion() {
    // Test all rounding mode conversions compile and work
    let modes = [
        FloatRoundingMode::NearestTiesToEven,
        FloatRoundingMode::NearestTiesToAway,
        FloatRoundingMode::TowardPositive,
        FloatRoundingMode::TowardNegative,
        FloatRoundingMode::TowardZero,
    ];

    for mode in modes {
        // Just verify conversion doesn't panic
        let _ = mode.to_z3_rounding_mode();
    }
}

// ============================================================================
// Quantifier Pattern Extraction Tests
// ============================================================================
//
// Tests for pattern extraction and generation for Z3 quantifier instantiation.
// These patterns guide Z3's MBQI (Model-Based Quantifier Instantiation) to
// find relevant ground instances of quantified formulas.

use verum_smt::translate::{PatternGenConfig, PatternTrigger};
use verum_ast::ty::{Ident, Path, PathSegment};

/// Helper to create a variable reference expression
fn make_var_expr(name: &str) -> Expr {
    let ident = Ident::new(name, Span::dummy());
    let segment = PathSegment::Name(ident);
    let path = Path {
        segments: vec![segment].into(),
        span: Span::dummy(),
    };
    Expr::new(ExprKind::Path(path), Span::dummy())
}

/// Helper to create a function call expression
fn make_call_expr(func_name: &str, args: Vec<Expr>) -> Expr {
    let func = make_var_expr(func_name);
    Expr::new(
        ExprKind::Call {
            func: Heap::new(func),
            type_args: Vec::new().into(),
            args: args.into(),
        },
        Span::dummy(),
    )
}

/// Helper to create an index expression: base[index]
fn make_index_expr(base: Expr, index: Expr) -> Expr {
    Expr::new(
        ExprKind::Index {
            expr: Heap::new(base),
            index: Heap::new(index),
        },
        Span::dummy(),
    )
}

/// Helper to create a field access expression: base.field
fn make_field_expr(base: Expr, field_name: &str) -> Expr {
    Expr::new(
        ExprKind::Field {
            expr: Heap::new(base),
            field: Ident::new(field_name, Span::dummy()),
        },
        Span::dummy(),
    )
}

/// Helper to create a binary operation expression
fn make_binary_expr(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        Span::dummy(),
    )
}

/// Helper to create a method call expression: receiver.method(args)
fn make_method_call_expr(receiver: Expr, method_name: &str, args: Vec<Expr>) -> Expr {
    Expr::new(
        ExprKind::MethodCall {
            receiver: Heap::new(receiver),
            method: Ident::new(method_name, Span::dummy()),
            args: args.into(),
            type_args: vec![].into(),
        },
        Span::dummy(),
    )
}

// ==================== Pattern Extraction Tests ====================

#[test]
fn test_extract_function_app_pattern() {
    // Test: forall x => f(x) > 0
    // Expected pattern: f(x)
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Build: f(x) > 0
    let x = make_var_expr("x");
    let f_x = make_call_expr("f", vec![x]);
    let zero = Expr::literal(Literal::int(0, Span::dummy()));
    let body = make_binary_expr(BinOp::Gt, f_x, zero);

    // Extract patterns with bound variable "x"
    let bound_vars = vec![Text::from("x")];
    let triggers = translator.extract_pattern_triggers(&body, &bound_vars);

    // Should find the f(x) trigger
    assert!(!triggers.is_empty(), "Should extract at least one trigger");

    // First trigger should be the function application
    let first = &triggers[0];
    match first {
        PatternTrigger::FunctionApp { func_name, .. } => {
            assert_eq!(func_name.as_str(), "f");
        }
        _ => panic!("Expected FunctionApp trigger, got {:?}", first),
    }
}

#[test]
fn test_extract_index_access_pattern() {
    // Test: forall i => arr[i] > 0
    // Expected pattern: arr[i]
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Build: arr[i] > 0
    let arr = make_var_expr("arr");
    let i = make_var_expr("i");
    let arr_i = make_index_expr(arr, i);
    let zero = Expr::literal(Literal::int(0, Span::dummy()));
    let body = make_binary_expr(BinOp::Gt, arr_i, zero);

    let bound_vars = vec![Text::from("i")];
    let triggers = translator.extract_pattern_triggers(&body, &bound_vars);

    // Should find the index access trigger
    let has_index_trigger = triggers.iter().any(|t| {
        matches!(t, PatternTrigger::IndexAccess { .. })
    });
    assert!(has_index_trigger, "Should extract IndexAccess trigger");
}

#[test]
fn test_extract_method_call_pattern() {
    // Test: forall x => x.len() > 0
    // Expected pattern: len(x)
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Build: x.len() > 0
    let x = make_var_expr("x");
    let x_len = make_method_call_expr(x, "len", vec![]);
    let zero = Expr::literal(Literal::int(0, Span::dummy()));
    let body = make_binary_expr(BinOp::Gt, x_len, zero);

    let bound_vars = vec![Text::from("x")];
    let triggers = translator.extract_pattern_triggers(&body, &bound_vars);

    // Should find the method call trigger
    let has_method_trigger = triggers.iter().any(|t| {
        matches!(t, PatternTrigger::MethodCall { method, .. } if method.as_str() == "len")
    });
    assert!(has_method_trigger, "Should extract MethodCall trigger for len");
}

#[test]
fn test_extract_field_access_pattern() {
    // Test: forall p => p.x > 0
    // Expected pattern: field_x(p)
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Build: p.x > 0
    let p = make_var_expr("p");
    let p_x = make_field_expr(p, "x");
    let zero = Expr::literal(Literal::int(0, Span::dummy()));
    let body = make_binary_expr(BinOp::Gt, p_x, zero);

    let bound_vars = vec![Text::from("p")];
    let triggers = translator.extract_pattern_triggers(&body, &bound_vars);

    // Should find the field access trigger
    let has_field_trigger = triggers.iter().any(|t| {
        matches!(t, PatternTrigger::FieldAccess { field, .. } if field.as_str() == "x")
    });
    assert!(has_field_trigger, "Should extract FieldAccess trigger for field x");
}

#[test]
fn test_extract_multiple_patterns() {
    // Test: forall x => f(x) > 0 && g(x) < 10
    // Expected patterns: f(x), g(x)
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Build: f(x) > 0 && g(x) < 10
    let x1 = make_var_expr("x");
    let x2 = make_var_expr("x");
    let f_x = make_call_expr("f", vec![x1]);
    let g_x = make_call_expr("g", vec![x2]);
    let zero = Expr::literal(Literal::int(0, Span::dummy()));
    let ten = Expr::literal(Literal::int(10, Span::dummy()));
    let f_x_gt_0 = make_binary_expr(BinOp::Gt, f_x, zero);
    let g_x_lt_10 = make_binary_expr(BinOp::Lt, g_x, ten);
    let body = make_binary_expr(BinOp::And, f_x_gt_0, g_x_lt_10);

    let bound_vars = vec![Text::from("x")];
    let triggers = translator.extract_pattern_triggers(&body, &bound_vars);

    // Should find both function applications
    let func_names: Vec<_> = triggers.iter()
        .filter_map(|t| match t {
            PatternTrigger::FunctionApp { func_name, .. } => Some(func_name.as_str()),
            _ => None,
        })
        .collect();

    assert!(func_names.contains(&"f"), "Should extract f(x) trigger");
    assert!(func_names.contains(&"g"), "Should extract g(x) trigger");
}

#[test]
fn test_pattern_priority_ordering() {
    // Test that triggers are sorted by priority
    // Priority: FunctionApp (100) > MethodCall (90) > IndexAccess (80) > FieldAccess (70) > BinaryOp (50)
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Build expression with multiple trigger types
    let x1 = make_var_expr("x");
    let x2 = make_var_expr("x");
    let x3 = make_var_expr("x");

    // f(x) - function call
    let f_x = make_call_expr("f", vec![x1]);
    // x.len() - method call
    let x_len = make_method_call_expr(x2, "len", vec![]);
    // x.field - field access
    let x_field = make_field_expr(x3, "field");

    // Combine: f(x) > 0 && x.len() > 0 && x.field > 0
    let zero1 = Expr::literal(Literal::int(0, Span::dummy()));
    let zero2 = Expr::literal(Literal::int(0, Span::dummy()));
    let zero3 = Expr::literal(Literal::int(0, Span::dummy()));

    let cmp1 = make_binary_expr(BinOp::Gt, f_x, zero1);
    let cmp2 = make_binary_expr(BinOp::Gt, x_len, zero2);
    let cmp3 = make_binary_expr(BinOp::Gt, x_field, zero3);

    let and1 = make_binary_expr(BinOp::And, cmp1, cmp2);
    let body = make_binary_expr(BinOp::And, and1, cmp3);

    let bound_vars = vec![Text::from("x")];
    let triggers = translator.extract_pattern_triggers(&body, &bound_vars);

    // First trigger should be FunctionApp (highest priority)
    if !triggers.is_empty() {
        assert!(
            matches!(&triggers[0], PatternTrigger::FunctionApp { .. }),
            "First trigger should be FunctionApp (highest priority)"
        );
    }
}

#[test]
fn test_no_pattern_for_unbound_variables() {
    // Test: forall x => f(y) > 0
    // y is not bound, so f(y) should not be a pattern
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Build: f(y) > 0
    let y = make_var_expr("y");
    let f_y = make_call_expr("f", vec![y]);
    let zero = Expr::literal(Literal::int(0, Span::dummy()));
    let body = make_binary_expr(BinOp::Gt, f_y, zero);

    // Only "x" is bound, not "y"
    let bound_vars = vec![Text::from("x")];
    let triggers = translator.extract_pattern_triggers(&body, &bound_vars);

    // Should not find any triggers (y is not bound)
    assert!(triggers.is_empty(), "Should not extract triggers for unbound variables");
}

#[test]
fn test_pattern_bound_var_refs() {
    // Test that triggers correctly track which bound variables they reference
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Build: f(x, y)
    let x = make_var_expr("x");
    let y = make_var_expr("y");
    let f_xy = make_call_expr("f", vec![x, y]);
    let zero = Expr::literal(Literal::int(0, Span::dummy()));
    let body = make_binary_expr(BinOp::Gt, f_xy, zero);

    let bound_vars = vec![Text::from("x"), Text::from("y")];
    let triggers = translator.extract_pattern_triggers(&body, &bound_vars);

    // The trigger should reference both x and y
    if let Some(trigger) = triggers.first() {
        let refs = trigger.bound_var_refs();
        assert!(refs.iter().any(|r| r.as_str() == "x"), "Should reference x");
        assert!(refs.iter().any(|r| r.as_str() == "y"), "Should reference y");
    } else {
        panic!("Expected at least one trigger");
    }
}

// ==================== Trigger Grouping Tests ====================

#[test]
fn test_group_triggers_by_shared_vars() {
    // Test that triggers sharing bound variables are grouped together
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Build: f(x) > 0 && g(x) < 10 && h(y) > 5
    // f(x) and g(x) share x, so should be grouped
    // h(y) doesn't share vars with them, separate group
    let x1 = make_var_expr("x");
    let x2 = make_var_expr("x");
    let y = make_var_expr("y");

    let f_x = make_call_expr("f", vec![x1]);
    let g_x = make_call_expr("g", vec![x2]);
    let h_y = make_call_expr("h", vec![y]);

    let zero = Expr::literal(Literal::int(0, Span::dummy()));
    let five = Expr::literal(Literal::int(5, Span::dummy()));
    let ten = Expr::literal(Literal::int(10, Span::dummy()));

    let cmp1 = make_binary_expr(BinOp::Gt, f_x, zero);
    let cmp2 = make_binary_expr(BinOp::Lt, g_x, ten);
    let cmp3 = make_binary_expr(BinOp::Gt, h_y, five);

    let and1 = make_binary_expr(BinOp::And, cmp1, cmp2);
    let body = make_binary_expr(BinOp::And, and1, cmp3);

    let bound_vars = vec![Text::from("x"), Text::from("y")];
    let triggers = translator.extract_pattern_triggers(&body, &bound_vars);

    // Verify we extracted at least 3 function call triggers
    let func_triggers: Vec<_> = triggers.iter()
        .filter(|t| matches!(t, PatternTrigger::FunctionApp { .. }))
        .collect();
    assert!(func_triggers.len() >= 3, "Should extract at least 3 function triggers");

    // Now group them
    let groups = translator.group_triggers(&triggers);

    // Should have at least 2 groups (x-related and y-related)
    assert!(groups.len() >= 2, "Should have at least 2 groups");

    // Verify the total number of triggers across all groups matches
    let total_in_groups: usize = groups.iter().map(|g| g.len()).sum();
    assert_eq!(total_in_groups, triggers.len(), "All triggers should be in groups");

    // Check that x-related triggers (f and g) are grouped together
    // Find the group containing "f"
    let f_group = groups.iter().find(|g| {
        g.iter().any(|t| {
            matches!(t, PatternTrigger::FunctionApp { func_name, .. } if func_name.as_str() == "f")
        })
    });
    assert!(f_group.is_some(), "Should find group containing f");

    // Check that g is in the same group as f (since both reference x)
    if let Some(group) = f_group {
        let has_g = group.iter().any(|t| {
            matches!(t, PatternTrigger::FunctionApp { func_name, .. } if func_name.as_str() == "g")
        });
        assert!(has_g, "f and g should be in the same group (both reference x)");
    }
}

// ==================== Z3 Pattern Generation Tests ====================

#[test]
fn test_triggers_to_z3_patterns() {
    // Test converting triggers to Z3 patterns
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Build: f(x) > 0
    let x = make_var_expr("x");
    let f_x = make_call_expr("f", vec![x]);
    let zero = Expr::literal(Literal::int(0, Span::dummy()));
    let body = make_binary_expr(BinOp::Gt, f_x, zero);

    let bound_vars = vec![Text::from("x")];
    let triggers = translator.extract_pattern_triggers(&body, &bound_vars);

    // Create Z3 variable mapping
    let x_var = Int::new_const("x");
    let mut z3_vars = verum_common::Map::new();
    z3_vars.insert(Text::from("x"), Dynamic::from_ast(&x_var));

    let config = PatternGenConfig::default();
    let patterns = translator.triggers_to_z3_patterns(&triggers, &z3_vars, &config);

    assert!(patterns.is_ok(), "Pattern generation should succeed");
    let patterns = patterns.unwrap();

    // Should generate at least one pattern
    assert!(!patterns.is_empty(), "Should generate at least one Z3 pattern");
}

#[test]
fn test_generate_quantifier_patterns_with_config() {
    // Test the full pattern generation pipeline with config
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Build: f(x) > 0
    let x = make_var_expr("x");
    let f_x = make_call_expr("f", vec![x]);
    let zero = Expr::literal(Literal::int(0, Span::dummy()));
    let body = make_binary_expr(BinOp::Gt, f_x, zero);

    let x_var = Int::new_const("x");

    let config = PatternGenConfig {
        max_patterns: 3,
        min_priority: 50,
        enable_multi_patterns: false,
        include_arithmetic: false,
    };

    let patterns = translator.generate_quantifier_patterns_with_config(
        "x",
        &Dynamic::from_ast(&x_var),
        &body,
        &config,
    );

    assert!(patterns.is_ok(), "Pattern generation should succeed");
}

#[test]
fn test_generate_multi_var_patterns() {
    // Test pattern generation for multiple bound variables
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Build: f(x, y) > 0
    let x = make_var_expr("x");
    let y = make_var_expr("y");
    let f_xy = make_call_expr("f", vec![x, y]);
    let zero = Expr::literal(Literal::int(0, Span::dummy()));
    let body = make_binary_expr(BinOp::Gt, f_xy, zero);

    let x_var = Int::new_const("x");
    let y_var = Int::new_const("y");

    let var_names = vec![Text::from("x"), Text::from("y")];
    let bound_vars = vec![Dynamic::from_ast(&x_var), Dynamic::from_ast(&y_var)];

    let patterns = translator.generate_multi_var_patterns(&var_names, &bound_vars, &body);

    assert!(patterns.is_ok(), "Multi-var pattern generation should succeed");
}

#[test]
fn test_pattern_config_max_patterns() {
    // Test that max_patterns is respected
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Build expression with many triggers
    let x1 = make_var_expr("x");
    let x2 = make_var_expr("x");
    let x3 = make_var_expr("x");
    let x4 = make_var_expr("x");

    let f_x = make_call_expr("f", vec![x1]);
    let g_x = make_call_expr("g", vec![x2]);
    let h_x = make_call_expr("h", vec![x3]);
    let i_x = make_call_expr("i", vec![x4]);

    let zero = Expr::literal(Literal::int(0, Span::dummy()));

    let cmp1 = make_binary_expr(BinOp::Gt, f_x, zero.clone());
    let cmp2 = make_binary_expr(BinOp::Gt, g_x, zero.clone());
    let cmp3 = make_binary_expr(BinOp::Gt, h_x, zero.clone());
    let cmp4 = make_binary_expr(BinOp::Gt, i_x, zero);

    let and1 = make_binary_expr(BinOp::And, cmp1, cmp2);
    let and2 = make_binary_expr(BinOp::And, cmp3, cmp4);
    let body = make_binary_expr(BinOp::And, and1, and2);

    let bound_vars = vec![Text::from("x")];
    let triggers = translator.extract_pattern_triggers(&body, &bound_vars);

    let x_var = Int::new_const("x");
    let mut z3_vars = verum_common::Map::new();
    z3_vars.insert(Text::from("x"), Dynamic::from_ast(&x_var));

    // Limit to 2 patterns
    let config = PatternGenConfig {
        max_patterns: 2,
        min_priority: 50,
        enable_multi_patterns: false,
        include_arithmetic: false,
    };

    let patterns = translator.triggers_to_z3_patterns(&triggers, &z3_vars, &config);
    assert!(patterns.is_ok());
    let patterns = patterns.unwrap();

    assert!(patterns.len() <= 2, "Should respect max_patterns limit");
}

#[test]
fn test_pattern_config_min_priority() {
    // Test that min_priority filters out low-priority triggers
    let ctx = smt_context();
    let translator = Translator::new(&ctx);

    // Build: x + 1 > 0 (binary op is low priority)
    let x = make_var_expr("x");
    let one = Expr::literal(Literal::int(1, Span::dummy()));
    let x_plus_1 = make_binary_expr(BinOp::Add, x, one);
    let zero = Expr::literal(Literal::int(0, Span::dummy()));
    let body = make_binary_expr(BinOp::Gt, x_plus_1, zero);

    let bound_vars = vec![Text::from("x")];
    let triggers = translator.extract_pattern_triggers(&body, &bound_vars);

    let x_var = Int::new_const("x");
    let mut z3_vars = verum_common::Map::new();
    z3_vars.insert(Text::from("x"), Dynamic::from_ast(&x_var));

    // Set high min_priority to filter out binary ops
    let config = PatternGenConfig {
        max_patterns: 10,
        min_priority: 60, // BinaryOp has priority 50
        enable_multi_patterns: false,
        include_arithmetic: false,
    };

    let patterns = translator.triggers_to_z3_patterns(&triggers, &z3_vars, &config);
    assert!(patterns.is_ok());
    let patterns = patterns.unwrap();

    // Should filter out the binary op trigger
    assert!(patterns.is_empty(), "Should filter out low-priority triggers");
}

#[test]
fn test_pattern_trigger_references_var() {
    // Test the references_var helper method
    let trigger = PatternTrigger::FunctionApp {
        func_name: Text::from("f"),
        args: vec![].into(),
        bound_var_refs: vec![Text::from("x"), Text::from("y")].into(),
    };

    assert!(trigger.references_var("x"), "Should reference x");
    assert!(trigger.references_var("y"), "Should reference y");
    assert!(!trigger.references_var("z"), "Should not reference z");
}

#[test]
fn test_pattern_trigger_priority() {
    // Test priority values for each trigger type
    let func_app = PatternTrigger::FunctionApp {
        func_name: Text::from("f"),
        args: vec![].into(),
        bound_var_refs: vec![].into(),
    };
    assert_eq!(func_app.priority(), 100);

    let method_call = PatternTrigger::MethodCall {
        receiver: Box::new(make_var_expr("x")),
        method: Text::from("len"),
        args: vec![].into(),
        bound_var_refs: vec![].into(),
    };
    assert_eq!(method_call.priority(), 90);

    let index_access = PatternTrigger::IndexAccess {
        base: Box::new(make_var_expr("arr")),
        index: Box::new(make_var_expr("i")),
        bound_var_refs: vec![].into(),
    };
    assert_eq!(index_access.priority(), 80);

    let field_access = PatternTrigger::FieldAccess {
        base: Box::new(make_var_expr("p")),
        field: Text::from("x"),
        bound_var_refs: vec![].into(),
    };
    assert_eq!(field_access.priority(), 70);

    let binary_op = PatternTrigger::BinaryOp {
        op: BinOp::Add,
        left: Box::new(make_var_expr("x")),
        right: Box::new(Expr::literal(Literal::int(1, Span::dummy()))),
        bound_var_refs: vec![].into(),
    };
    assert_eq!(binary_op.priority(), 50);
}
