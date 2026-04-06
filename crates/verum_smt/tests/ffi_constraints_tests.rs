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
//! Comprehensive test suite for FFI constraint translation
//!
//! This test suite validates the SMT translation of FFI boundary contracts
//! per the FFI boundary contract specification. FFI boundaries are compile-time
//! specifications (not types) with 7 mandatory components: signature (@extern("C")),
//! preconditions (requires), postconditions (ensures), memory effects, thread safety,
//! error protocol, and ownership semantics. Only C ABI is supported.
//!
//! Test coverage:
//! - 10 tests for precondition encoding
//! - 10 tests for postcondition encoding
//! - 8 tests for memory effects
//! - 5 tests for full verification
//! Total: 33 tests

use verum_ast::ffi::{
    CallingConvention, ErrorProtocol, FFIBoundary, FFIFunction, FFISignature, MemoryEffects,
    Ownership,
};
use verum_ast::literal::{FloatLit, IntLit};
use verum_ast::ty::{Ident, Type, TypeKind};
use verum_ast::{BinOp, Expr, ExprKind, Literal, LiteralKind, Span, UnOp};
use verum_common::span::FileId;
use verum_common::{Heap, List, Maybe, Text};
use verum_smt::Context;
use verum_smt::ffi_constraints::{ConstraintCategory, FFIConstraintEncoder};

// ============================================================================
// Helper Functions
// ============================================================================

fn create_span() -> Span {
    Span::new(0, 0, FileId::dummy())
}

fn create_ident(name: &str) -> Ident {
    Ident::new(Text::from(name), create_span())
}

fn create_int_type() -> Type {
    Type::int(create_span())
}

fn create_float_type() -> Type {
    Type::float(create_span())
}

fn create_bool_type() -> Type {
    Type::bool(create_span())
}

fn create_ptr_type() -> Type {
    Type::new(
        TypeKind::Pointer {
            mutable: false,
            inner: Heap::new(create_int_type()),
        },
        create_span(),
    )
}

fn create_int_literal(value: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit::new(value as i128)),
            create_span(),
        )),
        create_span(),
    )
}

fn create_float_literal(value: f64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Float(FloatLit::new(value)),
            create_span(),
        )),
        create_span(),
    )
}

fn create_bool_literal(value: bool) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::new(LiteralKind::Bool(value), create_span())),
        create_span(),
    )
}

fn create_var_expr(name: &str) -> Expr {
    Expr::new(
        ExprKind::Path(verum_ast::ty::Path::single(create_ident(name))),
        create_span(),
    )
}

fn create_binary_expr(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        create_span(),
    )
}

fn create_unary_expr(op: UnOp, expr: Expr) -> Expr {
    Expr::new(
        ExprKind::Unary {
            op,
            expr: Heap::new(expr),
        },
        create_span(),
    )
}

fn create_simple_function(
    name: &str,
    params: List<(&str, Type)>,
    return_type: Type,
    requires: List<Expr>,
    ensures: List<Expr>,
    memory_effects: MemoryEffects,
) -> FFIFunction {
    let param_list: List<(Ident, Type)> = params
        .into_iter()
        .map(|(n, t)| (create_ident(n), t))
        .collect();

    FFIFunction {
        name: create_ident(name),
        signature: FFISignature {
            params: param_list,
            return_type,
            calling_convention: CallingConvention::C,
            is_variadic: false,
            span: create_span(),
        },
        requires,
        ensures,
        memory_effects,
        thread_safe: true,
        error_protocol: ErrorProtocol::None,
        ownership: Ownership::Borrow,
        span: create_span(),
    }
}

// ============================================================================
// Precondition Encoding Tests (10 tests)
// ============================================================================

#[test]
fn test_precond_simple_comparison() {
    // requires x >= 0
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let precond = create_binary_expr(BinOp::Ge, create_var_expr("x"), create_int_literal(0));

    let function = create_simple_function(
        "test_fn",
        vec![("x", create_int_type())].into_iter().collect(),
        create_int_type(),
        vec![precond.clone()].into_iter().collect(),
        List::new(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_precondition(&precond, &function);
    assert!(result.is_ok());

    let constraint = result.unwrap();
    assert_eq!(constraint.category, ConstraintCategory::Precondition);
    assert!(constraint.description.contains("Precondition"));
}

#[test]
fn test_precond_float_nonzero() {
    // requires b != 0.0
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let precond = create_binary_expr(BinOp::Ne, create_var_expr("b"), create_float_literal(0.0));

    let function = create_simple_function(
        "divide",
        vec![("a", create_float_type()), ("b", create_float_type())]
            .into_iter()
            .collect(),
        create_float_type(),
        vec![precond.clone()].into_iter().collect(),
        List::new(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_precondition(&precond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_precond_pointer_nonnull() {
    // requires ptr != null (represented as 0)
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let precond = create_binary_expr(BinOp::Ne, create_var_expr("ptr"), create_int_literal(0));

    let function = create_simple_function(
        "use_ptr",
        vec![("ptr", create_ptr_type())].into_iter().collect(),
        create_int_type(),
        vec![precond.clone()].into_iter().collect(),
        List::new(),
        MemoryEffects::Reads(Maybe::None),
    );

    let result = encoder.encode_precondition(&precond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_precond_logical_and() {
    // requires x > 0 && x < 100
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let left = create_binary_expr(BinOp::Gt, create_var_expr("x"), create_int_literal(0));

    let right = create_binary_expr(BinOp::Lt, create_var_expr("x"), create_int_literal(100));

    let precond = create_binary_expr(BinOp::And, left, right);

    let function = create_simple_function(
        "bounded_fn",
        vec![("x", create_int_type())].into_iter().collect(),
        create_int_type(),
        vec![precond.clone()].into_iter().collect(),
        List::new(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_precondition(&precond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_precond_logical_or() {
    // requires x == 0 || x == 1
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let left = create_binary_expr(BinOp::Eq, create_var_expr("x"), create_int_literal(0));

    let right = create_binary_expr(BinOp::Eq, create_var_expr("x"), create_int_literal(1));

    let precond = create_binary_expr(BinOp::Or, left, right);

    let function = create_simple_function(
        "binary_fn",
        vec![("x", create_int_type())].into_iter().collect(),
        create_int_type(),
        vec![precond.clone()].into_iter().collect(),
        List::new(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_precondition(&precond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_precond_negation() {
    // requires !(x < 0)
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let inner = create_binary_expr(BinOp::Lt, create_var_expr("x"), create_int_literal(0));

    let precond = create_unary_expr(UnOp::Not, inner);

    let function = create_simple_function(
        "nonneg_fn",
        vec![("x", create_int_type())].into_iter().collect(),
        create_int_type(),
        vec![precond.clone()].into_iter().collect(),
        List::new(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_precondition(&precond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_precond_multiple_params() {
    // requires a > 0 && b > 0
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let left = create_binary_expr(BinOp::Gt, create_var_expr("a"), create_int_literal(0));

    let right = create_binary_expr(BinOp::Gt, create_var_expr("b"), create_int_literal(0));

    let precond = create_binary_expr(BinOp::And, left, right);

    let function = create_simple_function(
        "mul",
        vec![("a", create_int_type()), ("b", create_int_type())]
            .into_iter()
            .collect(),
        create_int_type(),
        vec![precond.clone()].into_iter().collect(),
        List::new(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_precondition(&precond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_precond_arithmetic_in_comparison() {
    // requires x + y > 10
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let sum = create_binary_expr(BinOp::Add, create_var_expr("x"), create_var_expr("y"));

    let precond = create_binary_expr(BinOp::Gt, sum, create_int_literal(10));

    let function = create_simple_function(
        "sum_fn",
        vec![("x", create_int_type()), ("y", create_int_type())]
            .into_iter()
            .collect(),
        create_int_type(),
        vec![precond.clone()].into_iter().collect(),
        List::new(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_precondition(&precond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_precond_implication() {
    // requires x > 0 -> y > 0
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let left = create_binary_expr(BinOp::Gt, create_var_expr("x"), create_int_literal(0));

    let right = create_binary_expr(BinOp::Gt, create_var_expr("y"), create_int_literal(0));

    let precond = create_binary_expr(BinOp::Imply, left, right);

    let function = create_simple_function(
        "impl_fn",
        vec![("x", create_int_type()), ("y", create_int_type())]
            .into_iter()
            .collect(),
        create_int_type(),
        vec![precond.clone()].into_iter().collect(),
        List::new(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_precondition(&precond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_precond_complex_nested() {
    // requires (x > 0 && y > 0) || (x == 0 && y == 0)
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let x_pos = create_binary_expr(BinOp::Gt, create_var_expr("x"), create_int_literal(0));
    let y_pos = create_binary_expr(BinOp::Gt, create_var_expr("y"), create_int_literal(0));
    let both_pos = create_binary_expr(BinOp::And, x_pos, y_pos);

    let x_zero = create_binary_expr(BinOp::Eq, create_var_expr("x"), create_int_literal(0));
    let y_zero = create_binary_expr(BinOp::Eq, create_var_expr("y"), create_int_literal(0));
    let both_zero = create_binary_expr(BinOp::And, x_zero, y_zero);

    let precond = create_binary_expr(BinOp::Or, both_pos, both_zero);

    let function = create_simple_function(
        "complex_fn",
        vec![("x", create_int_type()), ("y", create_int_type())]
            .into_iter()
            .collect(),
        create_int_type(),
        vec![precond.clone()].into_iter().collect(),
        List::new(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_precondition(&precond, &function);
    assert!(result.is_ok());
}

// ============================================================================
// Postcondition Encoding Tests (10 tests)
// ============================================================================

#[test]
fn test_postcond_result_positive() {
    // ensures result >= 0
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let postcond = create_binary_expr(BinOp::Ge, create_var_expr("result"), create_int_literal(0));

    let function = create_simple_function(
        "abs",
        vec![("x", create_int_type())].into_iter().collect(),
        create_int_type(),
        List::new(),
        vec![postcond.clone()].into_iter().collect(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_postcondition(&postcond, &function);
    assert!(result.is_ok());

    let constraint = result.unwrap();
    assert_eq!(constraint.category, ConstraintCategory::Postcondition);
}

#[test]
fn test_postcond_result_bounded() {
    // ensures result >= 0 && result <= 100
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let left = create_binary_expr(BinOp::Ge, create_var_expr("result"), create_int_literal(0));

    let right = create_binary_expr(
        BinOp::Le,
        create_var_expr("result"),
        create_int_literal(100),
    );

    let postcond = create_binary_expr(BinOp::And, left, right);

    let function = create_simple_function(
        "clamp",
        vec![("x", create_int_type())].into_iter().collect(),
        create_int_type(),
        List::new(),
        vec![postcond.clone()].into_iter().collect(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_postcondition(&postcond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_postcond_result_computation() {
    // ensures result == x + y
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let sum = create_binary_expr(BinOp::Add, create_var_expr("x"), create_var_expr("y"));

    let postcond = create_binary_expr(BinOp::Eq, create_var_expr("result"), sum);

    let function = create_simple_function(
        "add",
        vec![("x", create_int_type()), ("y", create_int_type())]
            .into_iter()
            .collect(),
        create_int_type(),
        List::new(),
        vec![postcond.clone()].into_iter().collect(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_postcondition(&postcond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_postcond_sqrt_property() {
    // ensures result >= 0.0 (for sqrt)
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let postcond = create_binary_expr(
        BinOp::Ge,
        create_var_expr("result"),
        create_float_literal(0.0),
    );

    let function = create_simple_function(
        "sqrt",
        vec![("x", create_float_type())].into_iter().collect(),
        create_float_type(),
        List::new(),
        vec![postcond.clone()].into_iter().collect(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_postcondition(&postcond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_postcond_boolean_result() {
    // ensures result == true || result == false
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let left = create_binary_expr(
        BinOp::Eq,
        create_var_expr("result"),
        create_bool_literal(true),
    );

    let right = create_binary_expr(
        BinOp::Eq,
        create_var_expr("result"),
        create_bool_literal(false),
    );

    let postcond = create_binary_expr(BinOp::Or, left, right);

    let function = create_simple_function(
        "is_valid",
        vec![("x", create_int_type())].into_iter().collect(),
        create_bool_type(),
        List::new(),
        vec![postcond.clone()].into_iter().collect(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_postcondition(&postcond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_postcond_result_implies() {
    // ensures x > 0 -> result > 0
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let left = create_binary_expr(BinOp::Gt, create_var_expr("x"), create_int_literal(0));

    let right = create_binary_expr(BinOp::Gt, create_var_expr("result"), create_int_literal(0));

    let postcond = create_binary_expr(BinOp::Imply, left, right);

    let function = create_simple_function(
        "identity",
        vec![("x", create_int_type())].into_iter().collect(),
        create_int_type(),
        List::new(),
        vec![postcond.clone()].into_iter().collect(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_postcondition(&postcond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_postcond_result_less_than_input() {
    // ensures result < x
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let postcond = create_binary_expr(BinOp::Lt, create_var_expr("result"), create_var_expr("x"));

    let function = create_simple_function(
        "decrement",
        vec![("x", create_int_type())].into_iter().collect(),
        create_int_type(),
        List::new(),
        vec![postcond.clone()].into_iter().collect(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_postcondition(&postcond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_postcond_division_property() {
    // ensures result * b == a (approximately)
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let product = create_binary_expr(BinOp::Mul, create_var_expr("result"), create_var_expr("b"));

    let postcond = create_binary_expr(BinOp::Eq, product, create_var_expr("a"));

    let function = create_simple_function(
        "divide",
        vec![("a", create_float_type()), ("b", create_float_type())]
            .into_iter()
            .collect(),
        create_float_type(),
        List::new(),
        vec![postcond.clone()].into_iter().collect(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_postcondition(&postcond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_postcond_multiple_constraints() {
    // ensures result >= 0 && result <= x
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let left = create_binary_expr(BinOp::Ge, create_var_expr("result"), create_int_literal(0));

    let right = create_binary_expr(BinOp::Le, create_var_expr("result"), create_var_expr("x"));

    let postcond = create_binary_expr(BinOp::And, left, right);

    let function = create_simple_function(
        "bounded_abs",
        vec![("x", create_int_type())].into_iter().collect(),
        create_int_type(),
        List::new(),
        vec![postcond.clone()].into_iter().collect(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_postcondition(&postcond, &function);
    assert!(result.is_ok());
}

#[test]
fn test_postcond_negation_property() {
    // ensures !(result < 0)
    let ctx = Context::new();
    let encoder = FFIConstraintEncoder::new(&ctx);

    let inner = create_binary_expr(BinOp::Lt, create_var_expr("result"), create_int_literal(0));

    let postcond = create_unary_expr(UnOp::Not, inner);

    let function = create_simple_function(
        "nonneg",
        vec![("x", create_int_type())].into_iter().collect(),
        create_int_type(),
        List::new(),
        vec![postcond.clone()].into_iter().collect(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_postcondition(&postcond, &function);
    assert!(result.is_ok());
}

// ============================================================================
// Memory Effects Tests (8 tests)
// ============================================================================

#[test]
fn test_memory_pure_function() {
    let ctx = Context::new();
    let mut encoder = FFIConstraintEncoder::new(&ctx);

    let function = create_simple_function(
        "pure_fn",
        vec![("x", create_int_type())].into_iter().collect(),
        create_int_type(),
        List::new(),
        List::new(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_memory_effects(&function.memory_effects, &function);
    assert!(result.is_ok());

    let constraints = result.unwrap();
    assert!(!constraints.is_empty());
    assert!(constraints[0].category == ConstraintCategory::FrameCondition);
}

#[test]
fn test_memory_reads_only() {
    let ctx = Context::new();
    let mut encoder = FFIConstraintEncoder::new(&ctx);

    let function = create_simple_function(
        "read_fn",
        vec![("ptr", create_ptr_type())].into_iter().collect(),
        create_int_type(),
        List::new(),
        List::new(),
        MemoryEffects::Reads(Maybe::None),
    );

    let result = encoder.encode_memory_effects(&function.memory_effects, &function);
    assert!(result.is_ok());

    let constraints = result.unwrap();
    assert!(!constraints.is_empty());
}

#[test]
fn test_memory_writes_unrestricted() {
    let ctx = Context::new();
    let mut encoder = FFIConstraintEncoder::new(&ctx);

    let function = create_simple_function(
        "write_fn",
        vec![("ptr", create_ptr_type())].into_iter().collect(),
        create_int_type(),
        List::new(),
        List::new(),
        MemoryEffects::Writes(Maybe::None),
    );

    let result = encoder.encode_memory_effects(&function.memory_effects, &function);
    assert!(result.is_ok());

    let constraints = result.unwrap();
    assert!(!constraints.is_empty());
}

#[test]
fn test_memory_writes_specific_range() {
    let ctx = Context::new();
    let mut encoder = FFIConstraintEncoder::new(&ctx);

    let ranges = vec![Text::from("ptr")].into_iter().collect();

    let function = create_simple_function(
        "write_range_fn",
        vec![("ptr", create_ptr_type())].into_iter().collect(),
        create_int_type(),
        List::new(),
        List::new(),
        MemoryEffects::Writes(Maybe::Some(ranges)),
    );

    let result = encoder.encode_memory_effects(&function.memory_effects, &function);
    assert!(result.is_ok());

    let constraints = result.unwrap();
    assert!(!constraints.is_empty());
}

#[test]
fn test_memory_allocates() {
    let ctx = Context::new();
    let mut encoder = FFIConstraintEncoder::new(&ctx);

    let function = create_simple_function(
        "alloc_fn",
        vec![("size", create_int_type())].into_iter().collect(),
        create_ptr_type(),
        List::new(),
        List::new(),
        MemoryEffects::Allocates,
    );

    let result = encoder.encode_memory_effects(&function.memory_effects, &function);
    assert!(result.is_ok());

    let constraints = result.unwrap();
    // Allocates doesn't generate frame conditions
    assert_eq!(constraints.len(), 0);
}

#[test]
fn test_memory_deallocates() {
    let ctx = Context::new();
    let mut encoder = FFIConstraintEncoder::new(&ctx);

    let function = create_simple_function(
        "free_fn",
        vec![("ptr", create_ptr_type())].into_iter().collect(),
        create_int_type(),
        List::new(),
        List::new(),
        MemoryEffects::Deallocates(Maybe::Some(Text::from("ptr"))),
    );

    let result = encoder.encode_memory_effects(&function.memory_effects, &function);
    assert!(result.is_ok());

    let constraints = result.unwrap();
    assert_eq!(constraints.len(), 0);
}

#[test]
fn test_memory_combined_effects() {
    let ctx = Context::new();
    let mut encoder = FFIConstraintEncoder::new(&ctx);

    let combined = vec![
        MemoryEffects::Reads(Maybe::None),
        MemoryEffects::Writes(Maybe::None),
    ]
    .into_iter()
    .collect();

    let function = create_simple_function(
        "rw_fn",
        vec![("ptr", create_ptr_type())].into_iter().collect(),
        create_int_type(),
        List::new(),
        List::new(),
        MemoryEffects::Combined(combined),
    );

    let result = encoder.encode_memory_effects(&function.memory_effects, &function);
    assert!(result.is_ok());

    let constraints = result.unwrap();
    assert!(!constraints.is_empty());
}

#[test]
fn test_memory_multiple_ranges() {
    let ctx = Context::new();
    let mut encoder = FFIConstraintEncoder::new(&ctx);

    let ranges = vec![Text::from("ptr1"), Text::from("ptr2")]
        .into_iter()
        .collect();

    let function = create_simple_function(
        "write_multi_fn",
        vec![("ptr1", create_ptr_type()), ("ptr2", create_ptr_type())]
            .into_iter()
            .collect(),
        create_int_type(),
        List::new(),
        List::new(),
        MemoryEffects::Writes(Maybe::Some(ranges)),
    );

    let result = encoder.encode_memory_effects(&function.memory_effects, &function);
    assert!(result.is_ok());

    let constraints = result.unwrap();
    assert!(!constraints.is_empty());
}

// ============================================================================
// Full Verification Tests (5 tests)
// ============================================================================

#[test]
fn test_verify_sqrt_contract() {
    // sqrt: requires x >= 0.0, ensures result >= 0.0
    let ctx = Context::new();
    let mut encoder = FFIConstraintEncoder::new(&ctx);

    let precond = create_binary_expr(BinOp::Ge, create_var_expr("x"), create_float_literal(0.0));

    let postcond = create_binary_expr(
        BinOp::Ge,
        create_var_expr("result"),
        create_float_literal(0.0),
    );

    let function = create_simple_function(
        "sqrt",
        vec![("x", create_float_type())].into_iter().collect(),
        create_float_type(),
        vec![precond].into_iter().collect(),
        vec![postcond].into_iter().collect(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_function(&function);
    assert!(result.is_ok());

    let constraints = result.unwrap();
    assert!(constraints.len() >= 2); // At least precond and postcond

    // Check we have both types
    let has_precond = constraints
        .iter()
        .any(|c| c.category == ConstraintCategory::Precondition);
    let has_postcond = constraints
        .iter()
        .any(|c| c.category == ConstraintCategory::Postcondition);
    let has_frame = constraints
        .iter()
        .any(|c| c.category == ConstraintCategory::FrameCondition);

    assert!(has_precond);
    assert!(has_postcond);
    assert!(has_frame);
}

#[test]
fn test_verify_division_contract() {
    // divide: requires b != 0.0
    let ctx = Context::new();
    let mut encoder = FFIConstraintEncoder::new(&ctx);

    let precond = create_binary_expr(BinOp::Ne, create_var_expr("b"), create_float_literal(0.0));

    let function = create_simple_function(
        "divide",
        vec![("a", create_float_type()), ("b", create_float_type())]
            .into_iter()
            .collect(),
        create_float_type(),
        vec![precond].into_iter().collect(),
        List::new(),
        MemoryEffects::Pure,
    );

    let result = encoder.encode_function(&function);
    assert!(result.is_ok());

    let constraints = result.unwrap();
    let has_precond = constraints
        .iter()
        .any(|c| c.category == ConstraintCategory::Precondition);
    assert!(has_precond);
}

#[test]
fn test_verify_boundary_multiple_functions() {
    // Test entire boundary with multiple functions
    let ctx = Context::new();
    let mut encoder = FFIConstraintEncoder::new(&ctx);

    let func1 = create_simple_function(
        "abs",
        vec![("x", create_int_type())].into_iter().collect(),
        create_int_type(),
        List::new(),
        vec![create_binary_expr(
            BinOp::Ge,
            create_var_expr("result"),
            create_int_literal(0),
        )]
        .into_iter()
        .collect(),
        MemoryEffects::Pure,
    );

    let func2 = create_simple_function(
        "max",
        vec![("a", create_int_type()), ("b", create_int_type())]
            .into_iter()
            .collect(),
        create_int_type(),
        List::new(),
        List::new(),
        MemoryEffects::Pure,
    );

    let boundary = FFIBoundary {
        name: create_ident("MathLib"),
        extends: Maybe::None,
        functions: vec![func1, func2].into_iter().collect(),
        visibility: verum_ast::decl::Visibility::Public,
        attributes: List::new(),
        span: create_span(),
    };

    let result = encoder.encode_boundary(&boundary);
    assert!(result.is_ok());

    let constraints = result.unwrap();
    assert!(constraints.len() >= 2); // At least one constraint per function
}

#[test]
fn test_verify_with_memory_effects() {
    // Function with writes and preconditions
    let ctx = Context::new();
    let mut encoder = FFIConstraintEncoder::new(&ctx);

    let precond = create_binary_expr(BinOp::Ne, create_var_expr("ptr"), create_int_literal(0));

    let ranges = vec![Text::from("ptr")].into_iter().collect();

    let function = create_simple_function(
        "write_val",
        vec![("ptr", create_ptr_type()), ("val", create_int_type())]
            .into_iter()
            .collect(),
        create_int_type(),
        vec![precond].into_iter().collect(),
        List::new(),
        MemoryEffects::Writes(Maybe::Some(ranges)),
    );

    let result = encoder.encode_function(&function);
    assert!(result.is_ok());

    let constraints = result.unwrap();
    let has_precond = constraints
        .iter()
        .any(|c| c.category == ConstraintCategory::Precondition);
    let has_frame = constraints
        .iter()
        .any(|c| c.category == ConstraintCategory::FrameCondition);

    assert!(has_precond);
    assert!(has_frame);
}

#[test]
fn test_verify_ownership_transfer() {
    // Function with ownership transfer
    let ctx = Context::new();
    let mut encoder = FFIConstraintEncoder::new(&ctx);

    let mut function = create_simple_function(
        "take_ownership",
        vec![("ptr", create_ptr_type())].into_iter().collect(),
        create_int_type(),
        List::new(),
        List::new(),
        MemoryEffects::Pure,
    );

    function.ownership = Ownership::TransferTo(Text::from("ptr"));

    let result = encoder.encode_function(&function);
    assert!(result.is_ok());

    let constraints = result.unwrap();
    let has_alloc = constraints
        .iter()
        .any(|c| c.category == ConstraintCategory::AllocationConstraint);

    assert!(has_alloc);
}
