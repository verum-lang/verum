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
// Comprehensive refinement type tests
//
// Tests the refinement type system including:
// - Basic refinement predicates
// - Numeric constraints (ranges, positive, negative)
// - Boolean predicates
// - String constraints
// - Dependent types
// - Verification condition generation
// - Refinement subtyping

use verum_ast::{
    expr::*,
    literal::Literal,
    span::Span,
    ty::{Ident, Path},
};
use verum_common::{Heap, List, Text};
use verum_types::refinement::*;
use verum_types::ty::Type;

// Helper function to create variable expressions
fn var_expr(name: &str, span: Span) -> Expr {
    Expr::ident(Ident::new(name, span))
}

// ============================================================================
// Basic Refinement Tests
// ============================================================================

#[test]
fn test_simple_refinement_creation() {
    let span = Span::dummy();
    let base = Type::int();
    let predicate = RefinementPredicate::new(
        Expr::literal(Literal::bool(true, span)),
        Text::from("it"),
        span,
    );

    let refined = Type::refined(base, predicate);
    assert!(matches!(refined, Type::Refined { .. }));
}

#[test]
fn test_refinement_base_type_extraction() {
    let span = Span::dummy();
    let base = Type::int();
    let predicate = RefinementPredicate::new(
        Expr::literal(Literal::bool(true, span)),
        Text::from("it"),
        span,
    );

    let refined = Type::refined(base.clone(), predicate);
    assert_eq!(refined.base(), &base);
}

#[test]
fn test_positive_int_refinement() {
    let span = Span::dummy();

    // { x: Int | x > 0 }
    let predicate_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Gt,
            left: Box::new(var_expr("x", span)),
            right: Box::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let predicate = RefinementPredicate::new(predicate_expr, Text::from("x"), span);
    let positive_int = Type::refined(Type::int(), predicate);

    assert!(matches!(positive_int, Type::Refined { .. }));
}

#[test]
fn test_negative_int_refinement() {
    let span = Span::dummy();

    // { x: Int | x < 0 }
    let predicate_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Lt,
            left: Box::new(var_expr("x", span)),
            right: Box::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let predicate = RefinementPredicate::new(predicate_expr, Text::from("x"), span);
    let negative_int = Type::refined(Type::int(), predicate);

    assert!(matches!(negative_int, Type::Refined { .. }));
}

#[test]
fn test_non_zero_refinement() {
    let span = Span::dummy();

    // { x: Int | x != 0 }
    let predicate_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Ne,
            left: Box::new(var_expr("x", span)),
            right: Box::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let predicate = RefinementPredicate::new(predicate_expr, Text::from("x"), span);
    let non_zero = Type::refined(Type::int(), predicate);

    assert!(matches!(non_zero, Type::Refined { .. }));
}

// ============================================================================
// Range Refinement Tests
// ============================================================================

#[test]
fn test_bounded_int_refinement() {
    let span = Span::dummy();

    // { x: Int | 0 <= x && x <= 100 }
    let left_bound = Expr::new(
        ExprKind::Binary {
            op: BinOp::Ge,
            left: Box::new(var_expr("x", span)),
            right: Box::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let right_bound = Expr::new(
        ExprKind::Binary {
            op: BinOp::Le,
            left: Box::new(var_expr("x", span)),
            right: Box::new(Expr::literal(Literal::int(100, span))),
        },
        span,
    );

    let predicate_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::And,
            left: Box::new(left_bound),
            right: Box::new(right_bound),
        },
        span,
    );

    let predicate = RefinementPredicate::new(predicate_expr, Text::from("x"), span);
    let bounded = Type::refined(Type::int(), predicate);

    assert!(matches!(bounded, Type::Refined { .. }));
}

#[test]
fn test_percentage_refinement() {
    let span = Span::dummy();

    // { x: Float | 0.0 <= x && x <= 1.0 }
    let predicate_expr = Expr::literal(Literal::bool(true, span));
    let predicate = RefinementPredicate::new(predicate_expr, Text::from("x"), span);
    let percentage = Type::refined(Type::float(), predicate);

    assert!(matches!(percentage, Type::Refined { .. }));
}

// ============================================================================
// Verification Condition Tests
// ============================================================================

#[test]
fn test_vc_generation_simple() {
    use verum_types::context::TypeContext;
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();
    let span = Span::dummy();

    let value = Expr::literal(Literal::int(42, span));
    let pred_expr = Expr::literal(Literal::bool(true, span));
    let predicate = RefinementPredicate::new(pred_expr, Text::from("it"), span);
    let refined_type = RefinementType::refined(verum_types::ty::Type::int(), predicate, span);

    let result = checker.check(&value, &refined_type, &ctx);

    assert!(result.is_ok());
    assert!(result.unwrap().is_valid());
}

#[test]
fn test_vc_generation_comparison() {
    use verum_types::context::TypeContext;
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();
    let span = Span::dummy();

    // Check: 42 > 0
    let value = Expr::literal(Literal::int(42, span));
    let predicate_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Gt,
            left: Box::new(var_expr("it", span)),
            right: Box::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let predicate = RefinementPredicate::new(predicate_expr, Text::from("it"), span);
    let refined_type = RefinementType::refined(verum_types::ty::Type::int(), predicate, span);

    let result = checker.check(&value, &refined_type, &ctx);
    assert!(result.is_ok());
    assert!(result.unwrap().is_valid());
}

#[test]
fn test_multiple_vc_generation() {
    use verum_types::context::TypeContext;
    let mut checker = RefinementChecker::new(RefinementConfig::default());
    let ctx = TypeContext::new();
    let span = Span::dummy();

    let value1 = Expr::literal(Literal::int(42, span));
    let pred1 = RefinementPredicate::new(
        Expr::literal(Literal::bool(true, span)),
        Text::from("it"),
        span,
    );
    let type1 = RefinementType::refined(verum_types::ty::Type::int(), pred1, span);

    let value2 = Expr::literal(Literal::int(100, span));
    let pred2 = RefinementPredicate::new(
        Expr::literal(Literal::bool(true, span)),
        Text::from("it"),
        span,
    );
    let type2 = RefinementType::refined(verum_types::ty::Type::int(), pred2, span);

    let _result1 = checker.check(&value1, &type1, &ctx);
    let _result2 = checker.check(&value2, &type2, &ctx);

    let stats = checker.stats();
    assert_eq!(stats.total_checks, 2);
}

// ============================================================================
// Dependent Type Tests
// ============================================================================

#[test]
fn test_dependent_pair_refinement() {
    let span = Span::dummy();

    // { (x, y): (Int, Int) | x < y }
    // Simplified: just create the structure
    let predicate_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Lt,
            left: Box::new(var_expr("x", span)),
            right: Box::new(var_expr("y", span)),
        },
        span,
    );

    let predicate = RefinementPredicate::new(predicate_expr, Text::from("it"), span);
    let base = Type::tuple(vec![Type::int(), Type::int()].into());
    let refined_pair = Type::refined(base, predicate);

    assert!(matches!(refined_pair, Type::Refined { .. }));
}

#[test]
fn test_dependent_list_refinement() {
    let span = Span::dummy();

    // { xs: List Int | length xs > 0 }
    let predicate_expr = Expr::literal(Literal::bool(true, span));
    let predicate = RefinementPredicate::new(predicate_expr, Text::from("xs"), span);
    let base = Type::Named {
        path: Path::single(Ident::new("List", span)),
        args: vec![Type::int()].into(),
    };
    let non_empty_list = Type::refined(base, predicate);

    assert!(matches!(non_empty_list, Type::Refined { .. }));
}

// ============================================================================
// String Refinement Tests
// ============================================================================

#[test]
fn test_non_empty_string_refinement() {
    let span = Span::dummy();

    // { s: String | length s > 0 }
    let predicate_expr = Expr::literal(Literal::bool(true, span));
    let predicate = RefinementPredicate::new(predicate_expr, Text::from("s"), span);
    let non_empty_string = Type::refined(Type::text(), predicate);

    assert!(matches!(non_empty_string, Type::Refined { .. }));
}

#[test]
fn test_email_format_refinement() {
    let span = Span::dummy();

    // { s: String | is_valid_email s }
    let predicate_expr = Expr::literal(Literal::bool(true, span));
    let predicate = RefinementPredicate::new(predicate_expr, Text::from("s"), span);
    let email = Type::refined(Type::text(), predicate);

    assert!(matches!(email, Type::Refined { .. }));
}

// ============================================================================
// Refinement Subtyping Tests
// ============================================================================

#[test]
fn test_refinement_subtyping_base() {
    let span = Span::dummy();

    // Positive <: Int (refined type is subtype of base)
    let predicate = RefinementPredicate::new(
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Gt,
                left: Box::new(var_expr("x", span)),
                right: Box::new(Expr::literal(Literal::int(0, span))),
            },
            span,
        ),
        Text::from("x"),
        span,
    );

    let positive = Type::refined(Type::int(), predicate);
    let int = Type::int();

    // Positive should be subtype of Int
    assert_eq!(positive.base(), &int);
}

#[test]
fn test_refinement_strengthening() {
    let span = Span::dummy();

    // { x | x > 10 } <: { x | x > 0 }
    let stronger_pred = RefinementPredicate::new(
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Gt,
                left: Box::new(var_expr("x", span)),
                right: Box::new(Expr::literal(Literal::int(10, span))),
            },
            span,
        ),
        Text::from("x"),
        span,
    );

    let weaker_pred = RefinementPredicate::new(
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Gt,
                left: Box::new(var_expr("x", span)),
                right: Box::new(Expr::literal(Literal::int(0, span))),
            },
            span,
        ),
        Text::from("x"),
        span,
    );

    let stronger = Type::refined(Type::int(), stronger_pred);
    let weaker = Type::refined(Type::int(), weaker_pred);

    // Both should have same base
    assert_eq!(stronger.base(), weaker.base());
}

// ============================================================================
// Complex Refinement Tests
// ============================================================================

#[test]
fn test_nested_refinement() {
    let span = Span::dummy();

    // List of positive integers
    let elem_pred = RefinementPredicate::new(
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Gt,
                left: Box::new(var_expr("x", span)),
                right: Box::new(Expr::literal(Literal::int(0, span))),
            },
            span,
        ),
        Text::from("x"),
        span,
    );

    let positive_int = Type::refined(Type::int(), elem_pred);
    // In full implementation: List<positive_int>
    assert!(matches!(positive_int, Type::Refined { .. }));
}

#[test]
fn test_function_refinement() {
    let span = Span::dummy();

    // Function that returns positive: fn(Int) -> { x | x > 0 }
    let return_pred = RefinementPredicate::new(
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Gt,
                left: Box::new(var_expr("x", span)),
                right: Box::new(Expr::literal(Literal::int(0, span))),
            },
            span,
        ),
        Text::from("x"),
        span,
    );

    let positive_int = Type::refined(Type::int(), return_pred);
    let func_type = Type::function(vec![Type::int()].into(), positive_int);

    assert!(matches!(func_type, Type::Function { .. }));
}

// ============================================================================
// Division by Zero Prevention
// ============================================================================

#[test]
fn test_division_safety_refinement() {
    let span = Span::dummy();

    // Safe divisor: { x: Int | x != 0 }
    let predicate = RefinementPredicate::new(
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Ne,
                left: Box::new(var_expr("x", span)),
                right: Box::new(Expr::literal(Literal::int(0, span))),
            },
            span,
        ),
        Text::from("x"),
        span,
    );

    let safe_divisor = Type::refined(Type::int(), predicate);
    assert!(matches!(safe_divisor, Type::Refined { .. }));
}

// ============================================================================
// Array Bounds Safety
// ============================================================================

#[test]
fn test_array_index_refinement() {
    let span = Span::dummy();

    // Valid index: { i: Int | 0 <= i && i < length }
    let predicate = RefinementPredicate::new(
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Ge,
                left: Box::new(var_expr("i", span)),
                right: Box::new(Expr::literal(Literal::int(0, span))),
            },
            span,
        ),
        Text::from("i"),
        span,
    );

    let valid_index = Type::refined(Type::int(), predicate);
    assert!(matches!(valid_index, Type::Refined { .. }));
}

// ============================================================================
// Null Safety Refinements
// ============================================================================

#[test]
fn test_non_null_refinement() {
    let span = Span::dummy();

    // { x: Maybe T | is_some x }
    let predicate = RefinementPredicate::new(
        Expr::literal(Literal::bool(true, span)),
        Text::from("x"),
        span,
    );

    let base = Type::Named {
        path: Path::single(Ident::new("Maybe", span)),
        args: vec![Type::Var(verum_types::ty::TypeVar::fresh())].into(),
    };
    let non_null = Type::refined(base, predicate);

    assert!(matches!(non_null, Type::Refined { .. }));
}

// ============================================================================
// Numeric Invariants
// ============================================================================

#[test]
fn test_even_number_refinement() {
    let span = Span::dummy();

    // { x: Int | x % 2 == 0 }
    let predicate = RefinementPredicate::new(
        Expr::literal(Literal::bool(true, span)),
        Text::from("x"),
        span,
    );

    let even = Type::refined(Type::int(), predicate);
    assert!(matches!(even, Type::Refined { .. }));
}

#[test]
fn test_odd_number_refinement() {
    let span = Span::dummy();

    // { x: Int | x % 2 == 1 }
    let predicate = RefinementPredicate::new(
        Expr::literal(Literal::bool(true, span)),
        Text::from("x"),
        span,
    );

    let odd = Type::refined(Type::int(), predicate);
    assert!(matches!(odd, Type::Refined { .. }));
}

// ============================================================================
// Capacity and Resource Refinements
// ============================================================================

#[test]
fn test_buffer_size_refinement() {
    let span = Span::dummy();

    // { size: Int | size > 0 && size <= MAX_BUFFER }
    let predicate = RefinementPredicate::new(
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Gt,
                left: Box::new(var_expr("size", span)),
                right: Box::new(Expr::literal(Literal::int(0, span))),
            },
            span,
        ),
        Text::from("size"),
        span,
    );

    let buffer_size = Type::refined(Type::int(), predicate);
    assert!(matches!(buffer_size, Type::Refined { .. }));
}

// ============================================================================
// SmtBackend timeout wire-up
// ============================================================================

#[test]
fn smt_backend_set_timeout_ms_default_is_no_op() {
    // Pin: the new `SmtBackend::set_timeout_ms` trait method has
    // a no-op default impl. Legacy backends compile without
    // modification; only backends that need the timeout override
    // it. The trait extension is therefore source-compatible with
    // every existing backend.
    use verum_common::{Map, Maybe};
    use verum_types::refinement::{
        RefinementError, SmtBackend, SmtResult, VerificationResult,
    };

    /// Legacy-style backend that doesn't override `set_timeout_ms`.
    /// Compiling this is the test — if the default impl were
    /// removed or changed to a non-default-providing form, this
    /// would fail to compile.
    struct LegacyBackend;
    impl SmtBackend for LegacyBackend {
        fn check(&mut self, _expr: &Expr) -> Result<SmtResult, RefinementError> {
            Ok(SmtResult::Unsat)
        }
        fn get_model(&mut self) -> Result<Map<Text, Text>, RefinementError> {
            Ok(Map::new())
        }
        fn verify_refinement(
            &mut self,
            _predicate: &Expr,
            _value: &Expr,
            _assumptions: &[Expr],
        ) -> Result<VerificationResult, RefinementError> {
            Ok(VerificationResult::Valid)
        }
        // No `set_timeout_ms` override — relies on the trait default.
    }

    // Calling the default method should not panic and should not
    // observably mutate anything (since the default impl is empty).
    let mut backend = LegacyBackend;
    backend.set_timeout_ms(12_345);
    backend.set_timeout_ms(0);
    backend.set_timeout_ms(u64::MAX);

    // Sanity: regular method dispatch still works.
    let dummy_expr = Expr::literal(Literal::int(0, Span::dummy()));
    assert!(matches!(backend.check(&dummy_expr), Ok(SmtResult::Unsat)));
}

#[test]
fn smt_backend_set_timeout_ms_can_be_overridden() {
    // Pin: backends that need the timeout can override
    // `set_timeout_ms` and observe the calls. This is how the
    // production Z3 backend forwards the limit to the solver.
    use std::sync::{Arc, Mutex};
    use verum_common::{Map, Maybe};
    use verum_types::refinement::{
        RefinementError, SmtBackend, SmtResult, VerificationResult,
    };

    struct SpyBackend {
        seen: Arc<Mutex<Vec<u64>>>,
    }
    impl SmtBackend for SpyBackend {
        fn check(&mut self, _expr: &Expr) -> Result<SmtResult, RefinementError> {
            Ok(SmtResult::Unsat)
        }
        fn get_model(&mut self) -> Result<Map<Text, Text>, RefinementError> {
            Ok(Map::new())
        }
        fn verify_refinement(
            &mut self,
            _predicate: &Expr,
            _value: &Expr,
            _assumptions: &[Expr],
        ) -> Result<VerificationResult, RefinementError> {
            Ok(VerificationResult::Valid)
        }
        fn set_timeout_ms(&mut self, ms: u64) {
            self.seen.lock().unwrap().push(ms);
        }
    }

    let seen: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let mut backend = SpyBackend { seen: seen.clone() };

    backend.set_timeout_ms(100);
    backend.set_timeout_ms(500);
    backend.set_timeout_ms(0);

    let recorded = seen.lock().unwrap().clone();
    assert_eq!(recorded, vec![100, 500, 0]);
}
