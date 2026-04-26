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
//! Tests for v6.0-BALANCED type system features.
//!
//! This module tests the advanced type system features introduced in v6.0:
//! - TypeKind::Refined with an explicit binder (the canonical sigma form per
//!   VVA §5 — `x: T where P(x)` parses to `Refined` with
//!   `predicate.binding = Some(x)`)
//! - TypeKind::Ownership (mutable/immutable)
//! - WherePredicateKind::Meta
//! - WherePredicateKind::Value
//! - WherePredicateKind::Ensures
//! - Where clause disambiguation
//!
//! Tests for Verum's five refinement binding rules (inline, lambda, sigma, named, bare).
//! Tests for where clause disambiguation (type, meta, value, postcondition).

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::Literal;
use verum_ast::ty::*;
use verum_ast::*;
use verum_common::{Heap, List, Maybe};

/// Helper function to create a test span
fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

/// Helper function to create a test identifier
fn test_ident(name: &str) -> Ident {
    Ident::new(name.to_string(), test_span())
}

// ============================================================================
// Refinement (sigma surface form) Tests — Rule 3: x: T where P(x)
//
// Per VVA §5 the sigma surface form collapses onto `TypeKind::Refined` with
// `predicate.binding = Some(name)`.
// ============================================================================

#[test]
fn test_sigma_type_basic() {
    // Rule 3 (Sigma-type): x: T where pred(x) -- canonical for dependent types
    // x: Int where x > 0
    let span = test_span();

    let predicate = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("x"))),
                span,
            )),
            op: BinOp::Gt,
            right: Heap::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let sigma_type = Type::new(
        TypeKind::Refined {
            base: Heap::new(Type::int(span)),
            predicate: Heap::new(RefinementPredicate::with_binding(
                predicate,
                Maybe::Some(test_ident("x")),
                span,
            )),
        },
        span,
    );

    match &sigma_type.kind {
        TypeKind::Refined { base, predicate } => {
            let binder = match &predicate.binding {
                Maybe::Some(ident) => ident,
                Maybe::None => panic!("Expected explicit binder on sigma refinement"),
            };
            assert_eq!(binder.name.as_str(), "x");
            assert_eq!(base.kind, TypeKind::Int);
            assert!(matches!(predicate.expr.kind, ExprKind::Binary { .. }));
        }
        _ => panic!("Expected Refined (sigma form)"),
    }
}

#[test]
fn test_sigma_type_complex_predicate() {
    // x: Float where x > 0.0 && x < 100.0
    let span = test_span();

    let left_cond = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("x"))),
                span,
            )),
            op: BinOp::Gt,
            right: Heap::new(Expr::literal(Literal::float(0.0, span))),
        },
        span,
    );

    let right_cond = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("x"))),
                span,
            )),
            op: BinOp::Lt,
            right: Heap::new(Expr::literal(Literal::float(100.0, span))),
        },
        span,
    );

    let predicate = Expr::new(
        ExprKind::Binary {
            left: Heap::new(left_cond),
            op: BinOp::And,
            right: Heap::new(right_cond),
        },
        span,
    );

    let sigma_type = Type::new(
        TypeKind::Refined {
            base: Heap::new(Type::float(span)),
            predicate: Heap::new(RefinementPredicate::with_binding(
                predicate,
                Maybe::Some(test_ident("x")),
                span,
            )),
        },
        span,
    );

    match &sigma_type.kind {
        TypeKind::Refined { base, predicate } => {
            let binder = match &predicate.binding {
                Maybe::Some(ident) => ident,
                Maybe::None => panic!("Expected explicit binder on sigma refinement"),
            };
            assert_eq!(binder.name.as_str(), "x");
            assert_eq!(base.kind, TypeKind::Float);
            match &predicate.expr.kind {
                ExprKind::Binary { op, .. } => {
                    assert_eq!(*op, BinOp::And);
                }
                _ => panic!("Expected binary expression"),
            }
        }
        _ => panic!("Expected Refined (sigma form)"),
    }
}

#[test]
fn test_sigma_type_with_text_predicate() {
    // email: Text where is_email(email)
    let span = test_span();

    let predicate = Expr::new(
        ExprKind::Call {
            func: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("is_email"))),
                span,
            )),
            type_args: List::new(),
            args: List::from(vec![Expr::new(
                ExprKind::Path(Path::single(test_ident("email"))),
                span,
            )]),
        },
        span,
    );

    let sigma_type = Type::new(
        TypeKind::Refined {
            base: Heap::new(Type::text(span)),
            predicate: Heap::new(RefinementPredicate::with_binding(
                predicate,
                Maybe::Some(test_ident("email")),
                span,
            )),
        },
        span,
    );

    match &sigma_type.kind {
        TypeKind::Refined { base, predicate } => {
            let binder = match &predicate.binding {
                Maybe::Some(ident) => ident,
                Maybe::None => panic!("Expected explicit binder on sigma refinement"),
            };
            assert_eq!(binder.name.as_str(), "email");
            assert_eq!(base.kind, TypeKind::Text);
            assert!(matches!(predicate.expr.kind, ExprKind::Call { .. }));
        }
        _ => panic!("Expected Refined (sigma form)"),
    }
}

#[test]
fn test_sigma_type_nested_in_function() {
    // fn sqrt(x: Float where x >= 0.0) -> Float
    let span = test_span();

    let predicate = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("x"))),
                span,
            )),
            op: BinOp::Ge,
            right: Heap::new(Expr::literal(Literal::float(0.0, span))),
        },
        span,
    );

    let param_type = Type::new(
        TypeKind::Refined {
            base: Heap::new(Type::float(span)),
            predicate: Heap::new(RefinementPredicate::with_binding(
                predicate,
                Maybe::Some(test_ident("x")),
                span,
            )),
        },
        span,
    );

    // Verify the type can be used as a function parameter type
    assert_eq!(param_type.span, span);
    match &param_type.kind {
        TypeKind::Refined { predicate, .. } => {
            let binder = match &predicate.binding {
                Maybe::Some(ident) => ident,
                Maybe::None => panic!("Expected explicit binder on sigma refinement"),
            };
            assert_eq!(binder.name.as_str(), "x");
        }
        _ => panic!("Expected Refined (sigma form)"),
    }
}

// ============================================================================
// TypeKind::Ownership Tests
// ============================================================================

#[test]
fn test_ownership_immutable_reference() {
    // Ownership references: %T (linear) and %mut T (affine) types
    // %T - immutable ownership reference
    let span = test_span();

    let ownership_type = Type::new(
        TypeKind::Ownership {
            mutable: false,
            inner: Heap::new(Type::int(span)),
        },
        span,
    );

    match &ownership_type.kind {
        TypeKind::Ownership { mutable, inner } => {
            assert!(!mutable);
            assert_eq!(inner.kind, TypeKind::Int);
        }
        _ => panic!("Expected Ownership type"),
    }
}

#[test]
fn test_ownership_mutable_reference() {
    // %mut T - mutable ownership reference
    let span = test_span();

    let ownership_type = Type::new(
        TypeKind::Ownership {
            mutable: true,
            inner: Heap::new(Type::text(span)),
        },
        span,
    );

    match &ownership_type.kind {
        TypeKind::Ownership { mutable, inner } => {
            assert!(mutable);
            assert_eq!(inner.kind, TypeKind::Text);
        }
        _ => panic!("Expected Ownership type"),
    }
}

#[test]
fn test_ownership_complex_inner_type() {
    // %mut List<Int>
    let span = test_span();

    let list_type = Type::new(
        TypeKind::Generic {
            base: Heap::new(Type::new(
                TypeKind::Path(Path::single(test_ident("List"))),
                span,
            )),
            args: List::from(vec![GenericArg::Type(Type::int(span))]),
        },
        span,
    );

    let ownership_type = Type::new(
        TypeKind::Ownership {
            mutable: true,
            inner: Heap::new(list_type),
        },
        span,
    );

    match &ownership_type.kind {
        TypeKind::Ownership { mutable, inner } => {
            assert!(mutable);
            assert!(matches!(inner.kind, TypeKind::Generic { .. }));
        }
        _ => panic!("Expected Ownership type"),
    }
}

#[test]
fn test_ownership_nested_in_tuple() {
    // (%T, %mut U)
    let span = test_span();

    let tuple_type = Type::new(
        TypeKind::Tuple(List::from(vec![
            Type::new(
                TypeKind::Ownership {
                    mutable: false,
                    inner: Heap::new(Type::int(span)),
                },
                span,
            ),
            Type::new(
                TypeKind::Ownership {
                    mutable: true,
                    inner: Heap::new(Type::float(span)),
                },
                span,
            ),
        ])),
        span,
    );

    match &tuple_type.kind {
        TypeKind::Tuple(elements) => {
            assert_eq!(elements.len(), 2);
            assert!(matches!(elements[0].kind, TypeKind::Ownership { .. }));
            assert!(matches!(elements[1].kind, TypeKind::Ownership { .. }));
        }
        _ => panic!("Expected Tuple type"),
    }
}

#[test]
fn test_ownership_reference_vs_cbgr_reference() {
    // Verify distinction between %T (ownership) and &T (CBGR)
    let span = test_span();

    let ownership = Type::new(
        TypeKind::Ownership {
            mutable: false,
            inner: Heap::new(Type::int(span)),
        },
        span,
    );

    let cbgr = Type::new(
        TypeKind::Reference {
            mutable: false,
            inner: Heap::new(Type::int(span)),
        },
        span,
    );

    // They should be different
    assert_ne!(ownership.kind, cbgr.kind);
}

// ============================================================================
// WherePredicateKind::Meta Tests
// ============================================================================

#[test]
fn test_where_meta_basic() {
    // Where clause disambiguation: type constraints vs meta refinements vs value predicates
    // where meta N > 0
    let span = test_span();

    let constraint = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("N"))),
                span,
            )),
            op: BinOp::Gt,
            right: Heap::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let predicate = WherePredicate {
        kind: WherePredicateKind::Meta { constraint },
        span,
    };

    match &predicate.kind {
        WherePredicateKind::Meta { constraint } => {
            assert!(matches!(constraint.kind, ExprKind::Binary { .. }));
        }
        _ => panic!("Expected Meta predicate"),
    }
}

#[test]
fn test_where_meta_complex_constraint() {
    // where meta N > 0 && N < 100
    let span = test_span();

    let left_cond = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("N"))),
                span,
            )),
            op: BinOp::Gt,
            right: Heap::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let right_cond = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("N"))),
                span,
            )),
            op: BinOp::Lt,
            right: Heap::new(Expr::literal(Literal::int(100, span))),
        },
        span,
    );

    let constraint = Expr::new(
        ExprKind::Binary {
            left: Heap::new(left_cond),
            op: BinOp::And,
            right: Heap::new(right_cond),
        },
        span,
    );

    let predicate = WherePredicate {
        kind: WherePredicateKind::Meta { constraint },
        span,
    };

    match &predicate.kind {
        WherePredicateKind::Meta { constraint } => match &constraint.kind {
            ExprKind::Binary { op, .. } => {
                assert_eq!(*op, BinOp::And);
            }
            _ => panic!("Expected binary expression"),
        },
        _ => panic!("Expected Meta predicate"),
    }
}

#[test]
fn test_where_meta_equality() {
    // where meta M == N * 2
    let span = test_span();

    let constraint = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("M"))),
                span,
            )),
            op: BinOp::Eq,
            right: Heap::new(Expr::new(
                ExprKind::Binary {
                    left: Heap::new(Expr::new(
                        ExprKind::Path(Path::single(test_ident("N"))),
                        span,
                    )),
                    op: BinOp::Mul,
                    right: Heap::new(Expr::literal(Literal::int(2, span))),
                },
                span,
            )),
        },
        span,
    );

    let predicate = WherePredicate {
        kind: WherePredicateKind::Meta { constraint },
        span,
    };

    assert!(matches!(predicate.kind, WherePredicateKind::Meta { .. }));
}

// ============================================================================
// WherePredicateKind::Value Tests
// ============================================================================

#[test]
fn test_where_value_basic() {
    // Where clause disambiguation: type constraints vs meta refinements vs value predicates
    // where value it > 0
    let span = test_span();

    let predicate_expr = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("it"))),
                span,
            )),
            op: BinOp::Gt,
            right: Heap::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let predicate = WherePredicate {
        kind: WherePredicateKind::Value {
            predicate: predicate_expr,
        },
        span,
    };

    match &predicate.kind {
        WherePredicateKind::Value { predicate } => {
            assert!(matches!(predicate.kind, ExprKind::Binary { .. }));
        }
        _ => panic!("Expected Value predicate"),
    }
}

#[test]
fn test_where_value_complex_predicate() {
    // where value it.len() > 0 && it.len() < 100
    let span = test_span();

    let left_cond = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::MethodCall {
                    receiver: Heap::new(Expr::new(
                        ExprKind::Path(Path::single(test_ident("it"))),
                        span,
                    )),
                    method: test_ident("len"),
                    type_args: List::new(),
                    args: List::new(),
                },
                span,
            )),
            op: BinOp::Gt,
            right: Heap::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let right_cond = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::MethodCall {
                    receiver: Heap::new(Expr::new(
                        ExprKind::Path(Path::single(test_ident("it"))),
                        span,
                    )),
                    method: test_ident("len"),
                    type_args: List::new(),
                    args: List::new(),
                },
                span,
            )),
            op: BinOp::Lt,
            right: Heap::new(Expr::literal(Literal::int(100, span))),
        },
        span,
    );

    let predicate_expr = Expr::new(
        ExprKind::Binary {
            left: Heap::new(left_cond),
            op: BinOp::And,
            right: Heap::new(right_cond),
        },
        span,
    );

    let predicate = WherePredicate {
        kind: WherePredicateKind::Value {
            predicate: predicate_expr,
        },
        span,
    };

    match &predicate.kind {
        WherePredicateKind::Value { predicate } => match &predicate.kind {
            ExprKind::Binary { op, .. } => {
                assert_eq!(*op, BinOp::And);
            }
            _ => panic!("Expected binary expression"),
        },
        _ => panic!("Expected Value predicate"),
    }
}

// ============================================================================
// WherePredicateKind::Ensures Tests
// ============================================================================

#[test]
fn test_where_ensures_basic() {
    // Where clause disambiguation: type constraints vs meta refinements vs value predicates
    // where ensures result >= 0
    let span = test_span();

    let postcondition = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("result"))),
                span,
            )),
            op: BinOp::Ge,
            right: Heap::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let predicate = WherePredicate {
        kind: WherePredicateKind::Ensures { postcondition },
        span,
    };

    match &predicate.kind {
        WherePredicateKind::Ensures { postcondition } => {
            assert!(matches!(postcondition.kind, ExprKind::Binary { .. }));
        }
        _ => panic!("Expected Ensures predicate"),
    }
}

#[test]
fn test_where_ensures_complex_postcondition() {
    // where ensures result > 0 && result < input
    let span = test_span();

    let left_cond = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("result"))),
                span,
            )),
            op: BinOp::Gt,
            right: Heap::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let right_cond = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("result"))),
                span,
            )),
            op: BinOp::Lt,
            right: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("input"))),
                span,
            )),
        },
        span,
    );

    let postcondition = Expr::new(
        ExprKind::Binary {
            left: Heap::new(left_cond),
            op: BinOp::And,
            right: Heap::new(right_cond),
        },
        span,
    );

    let predicate = WherePredicate {
        kind: WherePredicateKind::Ensures { postcondition },
        span,
    };

    match &predicate.kind {
        WherePredicateKind::Ensures { postcondition } => match &postcondition.kind {
            ExprKind::Binary { op, .. } => {
                assert_eq!(*op, BinOp::And);
            }
            _ => panic!("Expected binary expression"),
        },
        _ => panic!("Expected Ensures predicate"),
    }
}

#[test]
fn test_where_ensures_method_call() {
    // where ensures result.is_valid()
    let span = test_span();

    let postcondition = Expr::new(
        ExprKind::MethodCall {
            receiver: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("result"))),
                span,
            )),
            method: test_ident("is_valid"),
            type_args: List::new(),
            args: List::new(),
        },
        span,
    );

    let predicate = WherePredicate {
        kind: WherePredicateKind::Ensures { postcondition },
        span,
    };

    match &predicate.kind {
        WherePredicateKind::Ensures { postcondition } => {
            assert!(matches!(postcondition.kind, ExprKind::MethodCall { .. }));
        }
        _ => panic!("Expected Ensures predicate"),
    }
}

// ============================================================================
// Where Clause Disambiguation Tests
// ============================================================================

#[test]
fn test_where_clause_all_predicates() {
    // Test that all four predicate kinds can coexist in a WhereClause
    let span = test_span();

    let predicates = List::from(vec![
        // where type T: Ord
        WherePredicate {
            kind: WherePredicateKind::Type {
                ty: Type::new(TypeKind::Path(Path::single(test_ident("T"))), span),
                bounds: List::from(vec![TypeBound {
                    kind: TypeBoundKind::Protocol(Path::single(test_ident("Ord"))),
                    span,
                }]),
            },
            span,
        },
        // where meta N > 0
        WherePredicate {
            kind: WherePredicateKind::Meta {
                constraint: Expr::new(
                    ExprKind::Binary {
                        left: Heap::new(Expr::new(
                            ExprKind::Path(Path::single(test_ident("N"))),
                            span,
                        )),
                        op: BinOp::Gt,
                        right: Heap::new(Expr::literal(Literal::int(0, span))),
                    },
                    span,
                ),
            },
            span,
        },
        // where value it > 0
        WherePredicate {
            kind: WherePredicateKind::Value {
                predicate: Expr::new(
                    ExprKind::Binary {
                        left: Heap::new(Expr::new(
                            ExprKind::Path(Path::single(test_ident("it"))),
                            span,
                        )),
                        op: BinOp::Gt,
                        right: Heap::new(Expr::literal(Literal::int(0, span))),
                    },
                    span,
                ),
            },
            span,
        },
        // where ensures result >= 0
        WherePredicate {
            kind: WherePredicateKind::Ensures {
                postcondition: Expr::new(
                    ExprKind::Binary {
                        left: Heap::new(Expr::new(
                            ExprKind::Path(Path::single(test_ident("result"))),
                            span,
                        )),
                        op: BinOp::Ge,
                        right: Heap::new(Expr::literal(Literal::int(0, span))),
                    },
                    span,
                ),
            },
            span,
        },
    ]);

    let where_clause = WhereClause { predicates, span };

    assert_eq!(where_clause.predicates.len(), 4);
    assert!(matches!(
        where_clause.predicates[0].kind,
        WherePredicateKind::Type { .. }
    ));
    assert!(matches!(
        where_clause.predicates[1].kind,
        WherePredicateKind::Meta { .. }
    ));
    assert!(matches!(
        where_clause.predicates[2].kind,
        WherePredicateKind::Value { .. }
    ));
    assert!(matches!(
        where_clause.predicates[3].kind,
        WherePredicateKind::Ensures { .. }
    ));
}

#[test]
fn test_where_type_predicate_multiple_bounds() {
    // where type T: Ord + Display + Clone
    let span = test_span();

    let predicate = WherePredicate {
        kind: WherePredicateKind::Type {
            ty: Type::new(TypeKind::Path(Path::single(test_ident("T"))), span),
            bounds: List::from(vec![
                TypeBound {
                    kind: TypeBoundKind::Protocol(Path::single(test_ident("Ord"))),
                    span,
                },
                TypeBound {
                    kind: TypeBoundKind::Protocol(Path::single(test_ident("Display"))),
                    span,
                },
                TypeBound {
                    kind: TypeBoundKind::Protocol(Path::single(test_ident("Clone"))),
                    span,
                },
            ]),
        },
        span,
    };

    match &predicate.kind {
        WherePredicateKind::Type { ty, bounds } => {
            assert!(matches!(ty.kind, TypeKind::Path(_)));
            assert_eq!(bounds.len(), 3);
        }
        _ => panic!("Expected Type predicate"),
    }
}

#[test]
fn test_where_clause_empty() {
    // Empty where clause is valid
    let span = test_span();

    let where_clause = WhereClause {
        predicates: List::new(),
        span,
    };

    assert!(where_clause.predicates.is_empty());
}
