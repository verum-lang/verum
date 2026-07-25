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
//! Test cases for implicit 'it' in inline refinement types.
//!
//! The grammar allows implicit `it` in refinements:
//! - `Int{> 0}` should be equivalent to `Int{it > 0}`
//! - `Int{>= 0 && <= 100}` should be equivalent to `Int{it >= 0 && it <= 100}`

use verum_ast::{ExprKind, FileId, TypeKind};
use verum_fast_parser::VerumParser;

fn parse_type(source: &str) -> verum_ast::Type {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_type_str(source, file_id)
        .unwrap_or_else(|_| panic!("Failed to parse type: {}", source))
}

#[test]
fn test_single_gt_implicit_it() {
    // Int{> 0} should parse as Int{it > 0}
    let ty = parse_type("Int{> 0}");

    if let TypeKind::Refined { base, predicate } = ty.kind {
        assert!(matches!(base.kind, TypeKind::Int));
        // Verify the predicate is a binary expression with 'it' as left operand
        assert!(matches!(predicate.expr.kind, ExprKind::Binary { .. }));
    } else {
        panic!("Expected refined type, got {:?}", ty.kind);
    }
}

#[test]
fn test_single_gte_implicit_it() {
    // Int{>= 0} should parse as Int{it >= 0}
    let ty = parse_type("Int{>= 0}");

    if let TypeKind::Refined { base, predicate } = ty.kind {
        assert!(matches!(base.kind, TypeKind::Int));
        assert!(matches!(predicate.expr.kind, ExprKind::Binary { .. }));
    } else {
        panic!("Expected refined type, got {:?}", ty.kind);
    }
}

#[test]
fn test_single_lt_implicit_it() {
    // Int{< 100} should parse as Int{it < 100}
    let ty = parse_type("Int{< 100}");

    if let TypeKind::Refined { base, predicate } = ty.kind {
        assert!(matches!(base.kind, TypeKind::Int));
        assert!(matches!(predicate.expr.kind, ExprKind::Binary { .. }));
    } else {
        panic!("Expected refined type, got {:?}", ty.kind);
    }
}

#[test]
fn test_single_lte_implicit_it() {
    // Int{<= 100} should parse as Int{it <= 100}
    let ty = parse_type("Int{<= 100}");

    if let TypeKind::Refined { base, predicate } = ty.kind {
        assert!(matches!(base.kind, TypeKind::Int));
        assert!(matches!(predicate.expr.kind, ExprKind::Binary { .. }));
    } else {
        panic!("Expected refined type, got {:?}", ty.kind);
    }
}

#[test]
fn test_single_eq_implicit_it() {
    // Int{== 42} should parse as Int{it == 42}
    let ty = parse_type("Int{== 42}");

    if let TypeKind::Refined { base, predicate } = ty.kind {
        assert!(matches!(base.kind, TypeKind::Int));
        assert!(matches!(predicate.expr.kind, ExprKind::Binary { .. }));
    } else {
        panic!("Expected refined type, got {:?}", ty.kind);
    }
}

#[test]
fn test_single_ne_implicit_it() {
    // Int{!= 0} should parse as Int{it != 0}
    let ty = parse_type("Int{!= 0}");

    if let TypeKind::Refined { base, predicate } = ty.kind {
        assert!(matches!(base.kind, TypeKind::Int));
        assert!(matches!(predicate.expr.kind, ExprKind::Binary { .. }));
    } else {
        panic!("Expected refined type, got {:?}", ty.kind);
    }
}

#[test]
fn test_chained_implicit_it() {
    // Int{>= 0 && <= 100} should parse as Int{it >= 0 && it <= 100}
    let ty = parse_type("Int{>= 0 && <= 100}");

    if let TypeKind::Refined { base, predicate } = ty.kind {
        assert!(matches!(base.kind, TypeKind::Int));
        // Verify the predicate is a binary && expression
        assert!(matches!(predicate.expr.kind, ExprKind::Binary { .. }));
    } else {
        panic!("Expected refined type, got {:?}", ty.kind);
    }
}

#[test]
fn test_triple_chained_implicit_it() {
    // Int{>= 0 && <= 100 && != 50} should parse correctly
    let ty = parse_type("Int{>= 0 && <= 100 && != 50}");

    if let TypeKind::Refined { base, predicate } = ty.kind {
        assert!(matches!(base.kind, TypeKind::Int));
        assert!(matches!(predicate.expr.kind, ExprKind::Binary { .. }));
    } else {
        panic!("Expected refined type, got {:?}", ty.kind);
    }
}

#[test]
fn test_explicit_it_still_works() {
    // Int{it > 0 && it < 100} should still work
    let ty = parse_type("Int{it > 0 && it < 100}");

    if let TypeKind::Refined { base, predicate } = ty.kind {
        assert!(matches!(base.kind, TypeKind::Int));
        assert!(matches!(predicate.expr.kind, ExprKind::Binary { .. }));
    } else {
        panic!("Expected refined type, got {:?}", ty.kind);
    }
}

#[test]
fn test_mixed_explicit_implicit() {
    // Int{>= 0 && it * 2 <= 200} should work (first implicit, second explicit)
    let ty = parse_type("Int{>= 0 && it * 2 <= 200}");

    if let TypeKind::Refined { base, predicate } = ty.kind {
        assert!(matches!(base.kind, TypeKind::Int));
        assert!(matches!(predicate.expr.kind, ExprKind::Binary { .. }));
    } else {
        panic!("Expected refined type, got {:?}", ty.kind);
    }
}

#[test]
fn test_complex_rhs_expression() {
    // Int{>= 1 + 1 && <= 10 * 10} should parse correctly
    let ty = parse_type("Int{>= 1 + 1 && <= 10 * 10}");

    if let TypeKind::Refined { base, predicate } = ty.kind {
        assert!(matches!(base.kind, TypeKind::Int));
        assert!(matches!(predicate.expr.kind, ExprKind::Binary { .. }));
    } else {
        panic!("Expected refined type, got {:?}", ty.kind);
    }
}
