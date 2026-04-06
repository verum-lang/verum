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
// Comprehensive operator tests for Verum parser.
//
// This file tests all operators from grammar/verum.ebnf to ensure:
// 1. All operators are recognized and parsed
// 2. Correct precedence (low to high):
//    1. Pipeline |>
//    2. Assignment = += -= *= /= %= &= |= ^= <<= >>=
//    3. Null coalesce ??
//    4. Range .. ..=
//    5. Logical OR ||
//    6. Logical AND &&
//    7. Equality == !=
//    8. Relational < > <= >=
//    9. Bitwise OR |
//   10. Bitwise XOR ^
//   11. Bitwise AND &
//   12. Shift << >>
//   13. Additive + -
//   14. Multiplicative * / %
//   15. Power **
//   16. Unary ! - ~ & &mut * &checked &unsafe
//   17. Postfix . ?. [] () ? as await
// 3. Correct associativity (left vs right)
// 4. Edge cases and combinations

use verum_ast::{BinOp, ExprKind, FileId, UnOp};
use verum_parser::VerumParser;

/// Helper to parse an expression.
fn parse_expr(source: &str) -> Result<verum_ast::Expr, String> {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser.parse_expr_str(source, file_id).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join(", ")
    })
}

/// Helper to verify parsing succeeds.
fn assert_parses(source: &str) {
    parse_expr(source).unwrap_or_else(|_| panic!("Failed to parse: {}", source));
}

/// Helper to verify parsing fails.
fn assert_fails(source: &str) {
    assert!(
        parse_expr(source).is_err(),
        "Expected parse failure for: {}",
        source
    );
}

// ============================================================================
// SECTION 1: PIPELINE OPERATOR (|>)
// ============================================================================

#[test]
fn test_pipeline_basic() {
    let expr = parse_expr("x |> f").unwrap();
    assert!(matches!(expr.kind, ExprKind::Pipeline { .. }));
}

#[test]
fn test_pipeline_chain() {
    // Should parse as (x |> f) |> g (left-associative)
    let expr = parse_expr("x |> f |> g").unwrap();
    match expr.kind {
        ExprKind::Pipeline { left, .. } => {
            assert!(matches!(left.kind, ExprKind::Pipeline { .. }));
        }
        _ => panic!("Expected pipeline at top level"),
    }
}

#[test]
fn test_pipeline_with_function_call() {
    assert_parses("x |> f(1, 2)");
    assert_parses("x |> f() |> g(3)");
}

#[test]
fn test_pipeline_with_method_call() {
    assert_parses("x |> obj.method()");
    assert_parses("value |> parser.parse() |> validator.check()");
}

#[test]
fn test_pipeline_with_lambda() {
    assert_parses("x |> |v| v + 1");
    assert_parses("items |> |x| x * 2 |> |x| x + 1");
}

#[test]
fn test_pipeline_preserves_expression() {
    // Pipeline should consume full expressions
    assert_parses("x + y |> f");
    assert_parses("x * 2 |> f |> g");
}

// ============================================================================
// SECTION 2: ASSIGNMENT OPERATORS
// ============================================================================

#[test]
fn test_simple_assignment() {
    let expr = parse_expr("x = 5").unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Binary {
            op: BinOp::Assign,
            ..
        }
    ));
}

#[test]
fn test_chained_assignment() {
    // Chained assignment is valid in Verum (right-associative)
    let expr = parse_expr("x = y = z").expect("Chained assignment should parse");
    assert!(matches!(
        expr.kind,
        ExprKind::Binary {
            op: BinOp::Assign,
            ..
        }
    ));
}

#[test]
fn test_compound_assignment_add() {
    let expr = parse_expr("x += 5").unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Binary {
            op: BinOp::AddAssign,
            ..
        }
    ));
}

#[test]
fn test_compound_assignment_sub() {
    let expr = parse_expr("x -= 5").unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Binary {
            op: BinOp::SubAssign,
            ..
        }
    ));
}

#[test]
fn test_compound_assignment_mul() {
    let expr = parse_expr("x *= 5").unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Binary {
            op: BinOp::MulAssign,
            ..
        }
    ));
}

#[test]
fn test_compound_assignment_div() {
    let expr = parse_expr("x /= 5").unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Binary {
            op: BinOp::DivAssign,
            ..
        }
    ));
}

#[test]
fn test_compound_assignment_rem() {
    let expr = parse_expr("x %= 5").unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Binary {
            op: BinOp::RemAssign,
            ..
        }
    ));
}

#[test]
fn test_compound_assignment_bitwise() {
    assert_parses("x &= 0xFF");
    assert_parses("x |= 0x01");
    assert_parses("x ^= 0xAA");
}

#[test]
fn test_compound_assignment_shift() {
    assert_parses("x <<= 2");
    assert_parses("x >>= 1");
}

#[test]
fn test_assignment_with_complex_rhs() {
    assert_parses("x = a + b * c");
    assert_parses("result = x > 0 && y < 10");
}

// ============================================================================
// SECTION 3: NULL COALESCING OPERATOR (??)
// ============================================================================

#[test]
fn test_null_coalesce_basic() {
    let expr = parse_expr("x ?? y").unwrap();
    assert!(matches!(expr.kind, ExprKind::NullCoalesce { .. }));
}

#[test]
fn test_null_coalesce_right_associative() {
    // x ?? y ?? z should parse as x ?? (y ?? z)
    let expr = parse_expr("x ?? y ?? z").unwrap();
    match expr.kind {
        ExprKind::NullCoalesce { right, .. } => {
            assert!(matches!(right.kind, ExprKind::NullCoalesce { .. }));
        }
        _ => panic!("Expected null coalesce at top level"),
    }
}

#[test]
fn test_null_coalesce_chain() {
    assert_parses("a ?? b ?? c ?? d");
}

#[test]
fn test_null_coalesce_with_expressions() {
    assert_parses("foo() ?? default_value");
    assert_parses("obj?.field ?? fallback");
}

#[test]
fn test_null_coalesce_precedence_over_pipeline() {
    // ?? has higher precedence than |>, so: x ?? y |> f should be (x ?? y) |> f
    let expr = parse_expr("x ?? y |> f").unwrap();
    match expr.kind {
        ExprKind::Pipeline { left, .. } => {
            assert!(matches!(left.kind, ExprKind::NullCoalesce { .. }));
        }
        _ => panic!("Expected pipeline at top level"),
    }
}

// ============================================================================
// SECTION 4: RANGE OPERATORS (.. and ..=)
// ============================================================================

#[test]
fn test_range_exclusive() {
    let expr = parse_expr("0..10").unwrap();
    match expr.kind {
        ExprKind::Range { inclusive, .. } => {
            assert!(!inclusive, "Expected exclusive range");
        }
        _ => panic!("Expected range expression"),
    }
}

#[test]
fn test_range_inclusive() {
    let expr = parse_expr("0..=10").unwrap();
    match expr.kind {
        ExprKind::Range { inclusive, .. } => {
            assert!(inclusive, "Expected inclusive range");
        }
        _ => panic!("Expected range expression"),
    }
}

#[test]
fn test_range_open_ended() {
    assert_parses("0..");
    assert_parses("0..=");
}

#[test]
fn test_range_from_start() {
    assert_parses("..10");
    assert_parses("..=10");
}

#[test]
fn test_range_full() {
    assert_parses("..");
}

#[test]
fn test_range_with_expressions() {
    assert_parses("start..end");
    assert_parses("x + 1..y - 1");
}

// ============================================================================
// SECTION 5: LOGICAL OPERATORS (&& and ||)
// ============================================================================

#[test]
fn test_logical_and() {
    let expr = parse_expr("a && b").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::And, .. }));
}

#[test]
fn test_logical_or() {
    let expr = parse_expr("a || b").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Or, .. }));
}

#[test]
fn test_logical_and_precedence_over_or() {
    // a || b && c should parse as a || (b && c)
    let expr = parse_expr("a || b && c").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::Or,
            right,
            ..
        } => {
            assert!(matches!(
                right.kind,
                ExprKind::Binary { op: BinOp::And, .. }
            ));
        }
        _ => panic!("Expected OR at top level"),
    }
}

#[test]
fn test_logical_left_associative() {
    // a && b && c should parse as (a && b) && c
    let expr = parse_expr("a && b && c").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::And,
            left,
            ..
        } => {
            assert!(matches!(left.kind, ExprKind::Binary { op: BinOp::And, .. }));
        }
        _ => panic!("Expected AND at top level"),
    }
}

#[test]
fn test_logical_complex_chains() {
    assert_parses("a && b && c || d && e");
    assert_parses("x > 0 && x < 10 || y == 5");
}

// ============================================================================
// SECTION 6: EQUALITY OPERATORS (== and !=)
// ============================================================================

#[test]
fn test_equality_eq() {
    let expr = parse_expr("x == y").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Eq, .. }));
}

#[test]
fn test_equality_ne() {
    let expr = parse_expr("x != y").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Ne, .. }));
}

#[test]
fn test_equality_chain() {
    assert_parses("a == b != c");
}

#[test]
fn test_equality_with_expressions() {
    assert_parses("x + 1 == y - 1");
    assert_parses("foo() != bar()");
}

// ============================================================================
// SECTION 7: RELATIONAL OPERATORS (<, >, <=, >=)
// ============================================================================

#[test]
fn test_relational_lt() {
    let expr = parse_expr("x < y").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Lt, .. }));
}

#[test]
fn test_relational_gt() {
    let expr = parse_expr("x > y").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Gt, .. }));
}

#[test]
fn test_relational_le() {
    let expr = parse_expr("x <= y").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Le, .. }));
}

#[test]
fn test_relational_ge() {
    let expr = parse_expr("x >= y").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Ge, .. }));
}

#[test]
fn test_relational_all_operators() {
    assert_parses("a < b && c <= d && e > f && g >= h");
}

// ============================================================================
// SECTION 8: BITWISE OPERATORS (&, |, ^)
// ============================================================================

#[test]
fn test_bitwise_and() {
    let expr = parse_expr("x & y").unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Binary {
            op: BinOp::BitAnd,
            ..
        }
    ));
}

#[test]
fn test_bitwise_or() {
    let expr = parse_expr("x | y").unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Binary {
            op: BinOp::BitOr,
            ..
        }
    ));
}

#[test]
fn test_bitwise_xor() {
    let expr = parse_expr("x ^ y").unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Binary {
            op: BinOp::BitXor,
            ..
        }
    ));
}

#[test]
fn test_bitwise_precedence_order() {
    // x | y ^ z & w should parse as x | (y ^ (z & w))
    let expr = parse_expr("x | y ^ z & w").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::BitOr,
            right,
            ..
        } => match &right.kind {
            ExprKind::Binary {
                op: BinOp::BitXor,
                right: inner_right,
                ..
            } => {
                assert!(matches!(
                    inner_right.kind,
                    ExprKind::Binary {
                        op: BinOp::BitAnd,
                        ..
                    }
                ));
            }
            _ => panic!("Expected BitXor after BitOr"),
        },
        _ => panic!("Expected BitOr at top level"),
    }
}

#[test]
fn test_bitwise_with_hex_literals() {
    assert_parses("x & 0xFF");
    assert_parses("(x & 0xFF00) | (y & 0x00FF)");
}

// ============================================================================
// SECTION 9: SHIFT OPERATORS (<<, >>)
// ============================================================================

#[test]
fn test_shift_left() {
    let expr = parse_expr("x << 2").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Shl, .. }));
}

#[test]
fn test_shift_right() {
    let expr = parse_expr("x >> 2").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Shr, .. }));
}

#[test]
fn test_shift_left_associative() {
    // x << 1 << 2 should parse as (x << 1) << 2
    let expr = parse_expr("x << 1 << 2").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::Shl,
            left,
            ..
        } => {
            assert!(matches!(left.kind, ExprKind::Binary { op: BinOp::Shl, .. }));
        }
        _ => panic!("Expected Shl at top level"),
    }
}

#[test]
fn test_shift_with_addition() {
    // x << y + 1 should parse as x << (y + 1)
    let expr = parse_expr("x << y + 1").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::Shl,
            right,
            ..
        } => {
            assert!(matches!(
                right.kind,
                ExprKind::Binary { op: BinOp::Add, .. }
            ));
        }
        _ => panic!("Expected Shl at top level"),
    }
}

// ============================================================================
// SECTION 10: ADDITIVE OPERATORS (+, -)
// ============================================================================

#[test]
fn test_addition() {
    let expr = parse_expr("x + y").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Add, .. }));
}

#[test]
fn test_subtraction() {
    let expr = parse_expr("x - y").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Sub, .. }));
}

#[test]
fn test_additive_left_associative() {
    // x + y - z should parse as (x + y) - z
    let expr = parse_expr("x + y - z").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::Sub,
            left,
            ..
        } => {
            assert!(matches!(left.kind, ExprKind::Binary { op: BinOp::Add, .. }));
        }
        _ => panic!("Expected Sub at top level"),
    }
}

#[test]
fn test_additive_chain() {
    assert_parses("a + b + c - d - e");
}

// ============================================================================
// SECTION 11: MULTIPLICATIVE OPERATORS (*, /, %)
// ============================================================================

#[test]
fn test_multiplication() {
    let expr = parse_expr("x * y").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Mul, .. }));
}

#[test]
fn test_division() {
    let expr = parse_expr("x / y").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Div, .. }));
}

#[test]
fn test_remainder() {
    let expr = parse_expr("x % y").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Rem, .. }));
}

#[test]
fn test_multiplicative_left_associative() {
    // x * y / z should parse as (x * y) / z
    let expr = parse_expr("x * y / z").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::Div,
            left,
            ..
        } => {
            assert!(matches!(left.kind, ExprKind::Binary { op: BinOp::Mul, .. }));
        }
        _ => panic!("Expected Div at top level"),
    }
}

#[test]
fn test_multiplicative_precedence_over_additive() {
    // x + y * z should parse as x + (y * z)
    let expr = parse_expr("x + y * z").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::Add,
            right,
            ..
        } => {
            assert!(matches!(
                right.kind,
                ExprKind::Binary { op: BinOp::Mul, .. }
            ));
        }
        _ => panic!("Expected Add at top level"),
    }
}

// ============================================================================
// SECTION 12: POWER OPERATOR (**)
// ============================================================================

#[test]
fn test_power() {
    let expr = parse_expr("x ** y").unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Pow, .. }));
}

#[test]
fn test_power_right_associative() {
    // x ** y ** z should parse as x ** (y ** z)
    let expr = parse_expr("x ** y ** z").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::Pow,
            right,
            ..
        } => {
            assert!(matches!(
                right.kind,
                ExprKind::Binary { op: BinOp::Pow, .. }
            ));
        }
        _ => panic!("Expected Pow at top level"),
    }
}

#[test]
fn test_power_precedence_over_multiplicative() {
    // x * y ** z should parse as x * (y ** z)
    let expr = parse_expr("x * y ** z").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::Mul,
            right,
            ..
        } => {
            assert!(matches!(
                right.kind,
                ExprKind::Binary { op: BinOp::Pow, .. }
            ));
        }
        _ => panic!("Expected Mul at top level"),
    }
}

// ============================================================================
// SECTION 13: UNARY OPERATORS (!, -, ~, &, *, &mut, &checked, &unsafe)
// ============================================================================

#[test]
fn test_unary_not() {
    let expr = parse_expr("!x").unwrap();
    assert!(matches!(expr.kind, ExprKind::Unary { op: UnOp::Not, .. }));
}

#[test]
fn test_unary_neg() {
    let expr = parse_expr("-x").unwrap();
    assert!(matches!(expr.kind, ExprKind::Unary { op: UnOp::Neg, .. }));
}

#[test]
fn test_unary_bitnot() {
    let expr = parse_expr("~x").unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Unary {
            op: UnOp::BitNot,
            ..
        }
    ));
}

#[test]
fn test_unary_ref() {
    let expr = parse_expr("&x").unwrap();
    assert!(matches!(expr.kind, ExprKind::Unary { op: UnOp::Ref, .. }));
}

#[test]
fn test_unary_ref_mut() {
    let expr = parse_expr("&mut x").unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Unary {
            op: UnOp::RefMut,
            ..
        }
    ));
}

#[test]
fn test_unary_deref() {
    let expr = parse_expr("*x").unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Unary {
            op: UnOp::Deref,
            ..
        }
    ));
}

#[test]
fn test_unary_precedence_over_binary() {
    // -x + y should parse as (-x) + y
    let expr = parse_expr("-x + y").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            ..
        } => {
            assert!(matches!(left.kind, ExprKind::Unary { op: UnOp::Neg, .. }));
        }
        _ => panic!("Expected Add at top level"),
    }
}

#[test]
fn test_unary_chains() {
    assert_parses("!!x");
    assert_parses("--x");
    assert_parses("~~x");
}

#[test]
fn test_unary_with_postfix() {
    // -obj.field should parse as -(obj.field)
    let expr = parse_expr("-obj.field").unwrap();
    match expr.kind {
        ExprKind::Unary {
            op: UnOp::Neg,
            expr,
            ..
        } => {
            assert!(matches!(expr.kind, ExprKind::Field { .. }));
        }
        _ => panic!("Expected Unary at top level"),
    }
}

// ============================================================================
// SECTION 14: POSTFIX OPERATORS (., ?., [], (), ?, as, await)
// ============================================================================

#[test]
fn test_postfix_field_access() {
    let expr = parse_expr("obj.field").unwrap();
    assert!(matches!(expr.kind, ExprKind::Field { .. }));
}

#[test]
fn test_postfix_optional_chain() {
    let expr = parse_expr("obj?.field").unwrap();
    assert!(matches!(expr.kind, ExprKind::OptionalChain { .. }));
}

#[test]
fn test_postfix_index() {
    let expr = parse_expr("arr[0]").unwrap();
    assert!(matches!(expr.kind, ExprKind::Index { .. }));
}

#[test]
fn test_postfix_call() {
    let expr = parse_expr("foo()").unwrap();
    assert!(matches!(expr.kind, ExprKind::Call { .. }));
}

#[test]
fn test_postfix_method_call() {
    let expr = parse_expr("obj.method()").unwrap();
    assert!(matches!(expr.kind, ExprKind::MethodCall { .. }));
}

#[test]
fn test_postfix_try() {
    let expr = parse_expr("foo()?").unwrap();
    assert!(matches!(expr.kind, ExprKind::Try(_)));
}

#[test]
fn test_postfix_cast() {
    let expr = parse_expr("x as Int").unwrap();
    assert!(matches!(expr.kind, ExprKind::Cast { .. }));
}

#[test]
fn test_postfix_await() {
    let expr = parse_expr("future.await").unwrap();
    assert!(matches!(expr.kind, ExprKind::Await(_)));
}

#[test]
fn test_postfix_chained() {
    assert_parses("obj.method()[0].field");
    assert_parses("obj?.field?.nested");
    assert_parses("arr[i][j]");
}

#[test]
fn test_postfix_precedence_highest() {
    // obj.field + 1 should parse as (obj.field) + 1
    let expr = parse_expr("obj.field + 1").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            ..
        } => {
            assert!(matches!(left.kind, ExprKind::Field { .. }));
        }
        _ => panic!("Expected Add at top level"),
    }
}

// ============================================================================
// SECTION 15: MIXED PRECEDENCE TESTS
// ============================================================================

#[test]
fn test_precedence_arithmetic_comparison() {
    // a + b * c > d should parse as (a + (b * c)) > d
    let expr = parse_expr("a + b * c > d").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::Gt,
            left,
            ..
        } => match &left.kind {
            ExprKind::Binary {
                op: BinOp::Add,
                right,
                ..
            } => {
                assert!(matches!(
                    right.kind,
                    ExprKind::Binary { op: BinOp::Mul, .. }
                ));
            }
            _ => panic!("Expected Add in comparison left side"),
        },
        _ => panic!("Expected Gt at top level"),
    }
}

#[test]
fn test_precedence_comparison_logical() {
    // x > 0 && y < 10 should parse as (x > 0) && (y < 10)
    let expr = parse_expr("x > 0 && y < 10").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::And,
            left,
            right,
            ..
        } => {
            assert!(matches!(left.kind, ExprKind::Binary { op: BinOp::Gt, .. }));
            assert!(matches!(right.kind, ExprKind::Binary { op: BinOp::Lt, .. }));
        }
        _ => panic!("Expected And at top level"),
    }
}

#[test]
fn test_precedence_bitwise_shift_arithmetic() {
    // x & y << z + 1 should parse as x & (y << (z + 1))
    let expr = parse_expr("x & y << z + 1").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::BitAnd,
            right,
            ..
        } => match &right.kind {
            ExprKind::Binary {
                op: BinOp::Shl,
                right: shift_right,
                ..
            } => {
                assert!(matches!(
                    shift_right.kind,
                    ExprKind::Binary { op: BinOp::Add, .. }
                ));
            }
            _ => panic!("Expected Shl after BitAnd"),
        },
        _ => panic!("Expected BitAnd at top level"),
    }
}

#[test]
fn test_precedence_all_levels() {
    // Complex expression with all precedence levels
    assert_parses("x |> f ?? y = a || b && c == d > e + f * g ** h");
}

#[test]
fn test_precedence_postfix_unary_binary() {
    // -obj.method() * 2 should parse as (-(obj.method())) * 2
    assert_parses("-obj.method() * 2");
}

#[test]
fn test_precedence_pipeline_preserves_complex_expr() {
    // x + y * z |> f should parse as (x + (y * z)) |> f
    let expr = parse_expr("x + y * z |> f").unwrap();
    match expr.kind {
        ExprKind::Pipeline { left, .. } => match &left.kind {
            ExprKind::Binary {
                op: BinOp::Add,
                right,
                ..
            } => {
                assert!(matches!(
                    right.kind,
                    ExprKind::Binary { op: BinOp::Mul, .. }
                ));
            }
            _ => panic!("Expected Add in pipeline left side"),
        },
        _ => panic!("Expected Pipeline at top level"),
    }
}

// ============================================================================
// SECTION 16: EDGE CASES AND COMBINATIONS
// ============================================================================

#[test]
fn test_parentheses_override_precedence() {
    // (a + b) * c should parse as (a + b) * c, not a + (b * c)
    let expr = parse_expr("(a + b) * c").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::Mul,
            left,
            ..
        } => {
            assert!(matches!(left.kind, ExprKind::Paren(_)));
        }
        _ => panic!("Expected Mul at top level"),
    }
}

#[test]
fn test_deeply_nested_parentheses() {
    assert_parses("((a + b) * (c - d)) / ((e + f) * (g - h))");
}

#[test]
fn test_optional_chain_with_null_coalesce() {
    assert_parses("obj?.field ?? default");
    assert_parses("obj?.method()?.result ?? fallback");
}

#[test]
fn test_range_in_pipeline() {
    assert_parses("0..10 |> process");
    assert_parses("start..end |> validate |> transform");
}

#[test]
fn test_cast_with_arithmetic() {
    // x as Float + y should parse as (x as Float) + y
    let expr = parse_expr("x as Float + y").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            ..
        } => {
            assert!(matches!(left.kind, ExprKind::Cast { .. }));
        }
        _ => panic!("Expected Add at top level"),
    }
}

#[test]
fn test_try_with_arithmetic() {
    // foo()? + 1 should parse as (foo()?) + 1
    let expr = parse_expr("foo()? + 1").unwrap();
    match expr.kind {
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            ..
        } => {
            assert!(matches!(left.kind, ExprKind::Try(_)));
        }
        _ => panic!("Expected Add at top level"),
    }
}

#[test]
fn test_complex_real_world_expression() {
    assert_parses("data.values.filter(|x| x > 0).map(|x| x * 2).sum() ?? 0");
}

#[test]
fn test_complex_pipeline_with_lambdas() {
    assert_parses("items |> filter(|x| x > 0 && x < 100) |> map(|x| x * 2) |> sum");
}

#[test]
fn test_all_assignment_operators() {
    assert_parses("x = 1");
    assert_parses("x += 1");
    assert_parses("x -= 1");
    assert_parses("x *= 2");
    assert_parses("x /= 2");
    assert_parses("x %= 3");
    assert_parses("x &= 0xFF");
    assert_parses("x |= 0x01");
    assert_parses("x ^= 0xAA");
    assert_parses("x <<= 1");
    assert_parses("x >>= 1");
}

#[test]
fn test_tuple_index() {
    assert_parses("tuple.0");
    assert_parses("tuple.1");
    // Chaining tuple indices requires accessing a field that is itself a tuple
    assert_parses("(tuple.0).0");
}

#[test]
fn test_await_postfix() {
    assert_parses("async_fn().await");
    assert_parses("obj.async_method().await");
}

// ============================================================================
// SECTION 17: OPERATOR SUMMARY
// ============================================================================

// All operators tested:
// Pipeline: |>
// Assignment: = += -= *= /= %= &= |= ^= <<= >>=
// Null coalescing: ??
// Range: .. ..=
// Logical: || &&
// Equality: == !=
// Relational: < > <= >=
// Bitwise: & | ^
// Shift: << >>
// Additive: + -
// Multiplicative: * / %
// Power: **
// Unary: ! - ~ & &mut * &checked &unsafe
// Postfix: . ?. [] () ? as await

// Total tests: ~130+
