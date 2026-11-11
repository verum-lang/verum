//! Syntactic level projection has no authority to invent a lexical level.
use verum_ast::expr::{BinOp, Block, ConditionKind, IfCondition};
use verum_ast::{
    Expr, ExprKind, GenericArg, Ident, Literal, Path, Span, Type, TypeKind, UniverseLevelExpr,
};
use verum_common::{Heap, List};
fn integer(value: i128) -> Expr {
    Expr::literal(Literal::int(value, Span::default()))
}
fn binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        Span::default(),
    )
}
#[test]
fn concrete_levels_preserve_the_full_checked_u32_domain() {
    for value in [0, 1, u32::MAX] {
        assert_eq!(
            GenericArg::Const(integer(i128::from(value)).into()).universe_for_declared_slot(),
            Ok(Some(UniverseLevelExpr::Concrete(value)))
        );
    }
    for value in [-1, i128::from(u32::MAX) + 1] {
        assert!(
            GenericArg::Const(integer(value).into())
                .universe_for_declared_slot()
                .is_err()
        );
    }
}
#[test]
fn only_an_actual_inference_hole_projects_to_no_level() {
    assert_eq!(
        GenericArg::Type(Type::new(TypeKind::Inferred, Span::default())).universe_for_declared_slot(),
        Ok(None)
    );
    for argument in [
        GenericArg::Type(Type::new(TypeKind::Int, Span::default())),
        GenericArg::Const(Expr::literal(Literal::bool(true, Span::default())).into()),
        GenericArg::Const(Expr::literal(Literal::float(1.0, Span::default())).into()),
    ] {
        assert!(argument.universe_for_declared_slot().is_err());
    }
}
#[test]
fn an_unqualified_name_remains_a_symbolic_level_request() {
    let name = Ident::new("u", Span::default());
    for argument in [
        GenericArg::Const(Expr::ident(name.clone()).into()),
        GenericArg::Type(Type::new(
            TypeKind::Path(Path::single(name.clone())),
            Span::default(),
        )),
        GenericArg::Const(Expr::new(
            ExprKind::Paren(Heap::new(Expr::ident(name.clone()))),
            Span::default(),
        ).into()),
    ] {
        assert_eq!(
            argument.universe_for_declared_slot(),
            Ok(Some(UniverseLevelExpr::Variable(name.clone())))
        );
    }
}
#[test]
fn qualified_paths_are_not_resolved_by_their_leaf_spelling() {
    let span = Span::default();
    let path = Path::new(
        List::from_iter([
            verum_ast::ty::PathSegment::Name(Ident::new("other", span)),
            verum_ast::ty::PathSegment::Name(Ident::new("u", span)),
        ]),
        span,
    );
    assert!(
        GenericArg::Const(Expr::path(path.clone()).into())
            .universe_for_declared_slot()
            .is_err()
    );
    assert!(
        GenericArg::Type(Type::new(TypeKind::Path(path), span))
            .universe_for_declared_slot()
            .is_err()
    );
}

#[test]
fn closed_arithmetic_refuses_invalid_result_and_invalid_operations() {
    assert_eq!(
        GenericArg::Const(binary(BinOp::Add, integer(1), integer(1)).into()).universe_for_declared_slot(),
        Ok(Some(UniverseLevelExpr::Concrete(2)))
    );
    for expression in [
        binary(BinOp::Add, integer(i128::from(u32::MAX)), integer(1)),
        binary(BinOp::Div, integer(1), integer(0)),
        binary(BinOp::Sub, integer(0), integer(1)),
    ] {
        assert!(
            GenericArg::Const(expression.into())
                .universe_for_declared_slot()
                .is_err()
        );
    }
}
#[test]
fn a_dead_branch_cannot_hide_a_free_name_in_closed_arithmetic() {
    let span = Span::default();
    let conditional = Expr::new(
        ExprKind::If {
            condition: Heap::new(IfCondition {
                conditions: [ConditionKind::Expr(Expr::literal(Literal::bool(
                    true, span,
                )))]
                .into_iter()
                .collect(),
                span,
            }),
            then_branch: Block::new(List::new(), Some(Heap::new(integer(1))), span),
            else_branch: Some(Heap::new(Expr::ident(Ident::new("foreign", span)))),
        },
        span,
    );
    assert!(
        GenericArg::Const(conditional.into())
            .universe_for_declared_slot()
            .is_err()
    );
}

#[test]
fn foreign_integer_sorts_and_unbounded_expression_trees_are_refused() {
    let mut expression = integer(2);
    let ExprKind::Literal(Literal {
        kind: verum_ast::literal::LiteralKind::Int(integer),
        ..
    }) = &mut expression.kind
    else {
        unreachable!()
    };
    integer.suffix = Some(verum_ast::literal::IntSuffix::U64);
    assert!(
        GenericArg::Const(expression.into())
            .universe_for_declared_slot()
            .is_err()
    );
    let mut nested = Expr::path(Path::single(Ident::new("u", Span::default())));
    for _ in 0..65 {
        nested = Expr::new(ExprKind::Paren(Heap::new(nested)), Span::default());
    }
    assert!(
        GenericArg::Const(nested.into())
            .universe_for_declared_slot()
            .is_err()
    );
}
