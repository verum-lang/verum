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
//! Test visitor pattern for where clauses
//! Tests for where clause disambiguation (type, meta, value, postcondition).

use verum_ast::ty::{
    Ident, Path, TypeBound, TypeBoundKind, WhereClause, WherePredicate, WherePredicateKind,
};
use verum_common::List;
use verum_ast::visitor::{Visitor, walk_where_predicate};
use verum_ast::{Expr, ExprKind, Span};
use verum_common::Heap;

/// Test visitor that counts where clause components
struct WhereClauseCounter {
    type_constraints: usize,
    meta_constraints: usize,
    value_constraints: usize,
    postconditions: usize,
    expr_count: usize,
}

impl WhereClauseCounter {
    fn new() -> Self {
        Self {
            type_constraints: 0,
            meta_constraints: 0,
            value_constraints: 0,
            postconditions: 0,
            expr_count: 0,
        }
    }
}

impl Visitor for WhereClauseCounter {
    fn visit_where_predicate(&mut self, predicate: &WherePredicate) {
        match &predicate.kind {
            WherePredicateKind::Type { .. } => self.type_constraints += 1,
            WherePredicateKind::Meta { .. } => self.meta_constraints += 1,
            WherePredicateKind::Value { .. } => self.value_constraints += 1,
            WherePredicateKind::Ensures { .. } => self.postconditions += 1,
        }

        // Continue walking to visit expressions
        walk_where_predicate(self, predicate);
    }

    fn visit_expr(&mut self, _expr: &Expr) {
        self.expr_count += 1;
        // Don't walk expr to avoid infinite recursion in this simple test
    }
}

#[test]
fn test_visitor_type_constraint() {
    // Create a simple type constraint: where type T: Display
    let dummy_span = Span::dummy();
    let ty = verum_ast::Type::new(
        verum_ast::ty::TypeKind::Path(Path::single(Ident::new("T", dummy_span))),
        dummy_span,
    );

    let bounds = vec![TypeBound {
        kind: TypeBoundKind::Protocol(Path::single(Ident::new("Display", dummy_span))),
        span: dummy_span,
    }];

    let predicate = WherePredicate {
        kind: WherePredicateKind::Type { ty, bounds: bounds.into() },
        span: dummy_span,
    };

    let mut visitor = WhereClauseCounter::new();
    visitor.visit_where_predicate(&predicate);

    assert_eq!(visitor.type_constraints, 1);
    assert_eq!(visitor.meta_constraints, 0);
    assert_eq!(visitor.value_constraints, 0);
    assert_eq!(visitor.postconditions, 0);
}

#[test]
fn test_visitor_meta_constraint() {
    // Create a meta constraint: where meta N > 0
    let dummy_span = Span::dummy();
    let constraint = Expr::new(
        ExprKind::Binary {
            op: verum_ast::expr::BinOp::Gt,
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(Ident::new("N", dummy_span))),
                dummy_span,
            )),
            right: Heap::new(Expr::new(
                ExprKind::Literal(verum_ast::literal::Literal::int(0, dummy_span)),
                dummy_span,
            )),
        },
        dummy_span,
    );

    let predicate = WherePredicate {
        kind: WherePredicateKind::Meta { constraint },
        span: dummy_span,
    };

    let mut visitor = WhereClauseCounter::new();
    visitor.visit_where_predicate(&predicate);

    assert_eq!(visitor.type_constraints, 0);
    assert_eq!(visitor.meta_constraints, 1);
    assert_eq!(visitor.value_constraints, 0);
    assert_eq!(visitor.postconditions, 0);
    assert_eq!(visitor.expr_count, 1); // Should visit the constraint expression
}

#[test]
fn test_visitor_value_constraint() {
    // Create a value constraint: where value it > 0
    let dummy_span = Span::dummy();
    let predicate_expr = Expr::new(
        ExprKind::Binary {
            op: verum_ast::expr::BinOp::Gt,
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(Ident::new("it", dummy_span))),
                dummy_span,
            )),
            right: Heap::new(Expr::new(
                ExprKind::Literal(verum_ast::literal::Literal::int(0, dummy_span)),
                dummy_span,
            )),
        },
        dummy_span,
    );

    let predicate = WherePredicate {
        kind: WherePredicateKind::Value {
            predicate: predicate_expr,
        },
        span: dummy_span,
    };

    let mut visitor = WhereClauseCounter::new();
    visitor.visit_where_predicate(&predicate);

    assert_eq!(visitor.type_constraints, 0);
    assert_eq!(visitor.meta_constraints, 0);
    assert_eq!(visitor.value_constraints, 1);
    assert_eq!(visitor.postconditions, 0);
    assert_eq!(visitor.expr_count, 1); // Should visit the predicate expression
}

#[test]
fn test_visitor_postcondition() {
    // Create a postcondition: where ensures result >= 0
    let dummy_span = Span::dummy();
    let postcondition = Expr::new(
        ExprKind::Binary {
            op: verum_ast::expr::BinOp::Ge,
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(Ident::new("result", dummy_span))),
                dummy_span,
            )),
            right: Heap::new(Expr::new(
                ExprKind::Literal(verum_ast::literal::Literal::int(0, dummy_span)),
                dummy_span,
            )),
        },
        dummy_span,
    );

    let predicate = WherePredicate {
        kind: WherePredicateKind::Ensures { postcondition },
        span: dummy_span,
    };

    let mut visitor = WhereClauseCounter::new();
    visitor.visit_where_predicate(&predicate);

    assert_eq!(visitor.type_constraints, 0);
    assert_eq!(visitor.meta_constraints, 0);
    assert_eq!(visitor.value_constraints, 0);
    assert_eq!(visitor.postconditions, 1);
    assert_eq!(visitor.expr_count, 1); // Should visit the postcondition expression
}

#[test]
fn test_visitor_mixed_where_clause() {
    // Create a where clause with all four forms
    let dummy_span = Span::dummy();

    // 1. Type constraint
    let ty = verum_ast::Type::new(
        verum_ast::ty::TypeKind::Path(Path::single(Ident::new("T", dummy_span))),
        dummy_span,
    );
    let type_pred = WherePredicate {
        kind: WherePredicateKind::Type {
            ty,
            bounds: List::from(vec![TypeBound {
                kind: TypeBoundKind::Protocol(Path::single(Ident::new("Display", dummy_span))),
                span: dummy_span,
            }]),
        },
        span: dummy_span,
    };

    // 2. Meta constraint
    let meta_pred = WherePredicate {
        kind: WherePredicateKind::Meta {
            constraint: Expr::new(
                ExprKind::Binary {
                    op: verum_ast::expr::BinOp::Gt,
                    left: Heap::new(Expr::new(
                        ExprKind::Path(Path::single(Ident::new("N", dummy_span))),
                        dummy_span,
                    )),
                    right: Heap::new(Expr::new(
                        ExprKind::Literal(verum_ast::literal::Literal::int(0, dummy_span)),
                        dummy_span,
                    )),
                },
                dummy_span,
            ),
        },
        span: dummy_span,
    };

    // 3. Value constraint
    let value_pred = WherePredicate {
        kind: WherePredicateKind::Value {
            predicate: Expr::new(
                ExprKind::Binary {
                    op: verum_ast::expr::BinOp::Gt,
                    left: Heap::new(Expr::new(
                        ExprKind::Path(Path::single(Ident::new("it", dummy_span))),
                        dummy_span,
                    )),
                    right: Heap::new(Expr::new(
                        ExprKind::Literal(verum_ast::literal::Literal::int(0, dummy_span)),
                        dummy_span,
                    )),
                },
                dummy_span,
            ),
        },
        span: dummy_span,
    };

    // 4. Postcondition
    let ensures_pred = WherePredicate {
        kind: WherePredicateKind::Ensures {
            postcondition: Expr::new(
                ExprKind::Binary {
                    op: verum_ast::expr::BinOp::Ge,
                    left: Heap::new(Expr::new(
                        ExprKind::Path(Path::single(Ident::new("result", dummy_span))),
                        dummy_span,
                    )),
                    right: Heap::new(Expr::new(
                        ExprKind::Literal(verum_ast::literal::Literal::int(0, dummy_span)),
                        dummy_span,
                    )),
                },
                dummy_span,
            ),
        },
        span: dummy_span,
    };

    let where_clause = WhereClause {
        predicates: List::from(vec![type_pred, meta_pred, value_pred, ensures_pred]),
        span: dummy_span,
    };

    let mut visitor = WhereClauseCounter::new();
    visitor.visit_where_clause(&where_clause);

    assert_eq!(visitor.type_constraints, 1);
    assert_eq!(visitor.meta_constraints, 1);
    assert_eq!(visitor.value_constraints, 1);
    assert_eq!(visitor.postconditions, 1);
    assert_eq!(visitor.expr_count, 3); // Three expression-based predicates
}
