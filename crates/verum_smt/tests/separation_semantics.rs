//! Semantic controls for the encoder used by compiler separation obligations.
use verum_ast::literal::{IntLit, Literal, LiteralKind};
use verum_ast::{BinOp, Expr, ExprKind, Ident, Path, Span};
use verum_common::{Heap, Text};
use verum_smt::separation_logic::{
    EntailmentResult, SepAssertion, SepLogicConfig, SepLogicEncoder,
};

fn integer(value: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit::new(i128::from(value))),
            Span::dummy(),
        )),
        Span::dummy(),
    )
}

fn cell(address: i64, value: i64) -> SepAssertion {
    SepAssertion::points_to(integer(address), integer(value))
}

fn variable(name: &str) -> Expr {
    Expr::new(
        ExprKind::Path(Path::from_ident(Ident::new(name, Span::dummy()))),
        Span::dummy(),
    )
}

fn equal(left: Expr, right: Expr) -> SepAssertion {
    SepAssertion::Pure(Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        Span::dummy(),
    ))
}

fn entails(pre: &SepAssertion, post: &SepAssertion) -> EntailmentResult {
    SepLogicEncoder::new(SepLogicConfig::default())
        .verify_entailment(pre, post)
        .unwrap()
}

fn valid(pre: &SepAssertion, post: &SepAssertion) {
    assert!(matches!(entails(pre, post), EntailmentResult::Valid { .. }));
}

fn invalid(pre: &SepAssertion, post: &SepAssertion) {
    assert!(matches!(
        entails(pre, post),
        EntailmentResult::Invalid { .. }
    ));
}

#[test]
fn empty_and_singleton_heaps_are_distinct() {
    valid(&SepAssertion::Emp, &SepAssertion::Emp);
    valid(&cell(8, 42), &cell(8, 42));
    invalid(&cell(8, 42), &SepAssertion::Emp);
    invalid(&SepAssertion::Emp, &cell(8, 42));
    invalid(&cell(8, 42), &cell(8, 43));
}

#[test]
fn separating_witnesses_are_local_even_in_conclusions() {
    let pair = SepAssertion::sep(cell(8, 1), cell(16, 2));
    valid(&pair, &pair);
    valid(&pair, &SepAssertion::sep(cell(16, 2), cell(8, 1)));
    valid(
        &cell(8, 1),
        &SepAssertion::sep(cell(8, 1), SepAssertion::Emp),
    );
    valid(
        &SepAssertion::sep(SepAssertion::Emp, cell(8, 1)),
        &cell(8, 1),
    );
}

#[test]
fn exact_ownership_cannot_discard_or_duplicate_cells() {
    let pair = SepAssertion::sep(cell(8, 1), cell(16, 2));
    invalid(&pair, &cell(8, 1));
    invalid(&cell(8, 1), &pair);
    invalid(&cell(8, 1), &SepAssertion::sep(cell(8, 1), cell(8, 1)));
    // Inconsistent ownership entails false; identical values do not allow
    // the same cell to be allocated twice.
    let falsehood = SepAssertion::Pure(Expr::new(
        ExprKind::Literal(Literal::new(LiteralKind::Bool(false), Span::dummy())),
        Span::dummy(),
    ));
    valid(&SepAssertion::sep(cell(8, 1), cell(8, 1)), &falsehood);
}

#[test]
fn existential_witnesses_are_bound_and_shadowing_restores_the_outer_owner() {
    let any_value = SepAssertion::exists(
        Text::from("x"),
        SepAssertion::points_to(integer(8), variable("x")),
    );
    valid(&cell(8, 42), &any_value);
    invalid(&any_value, &cell(8, 42));

    let nested = SepAssertion::exists(
        Text::from("x"),
        SepAssertion::and(
            equal(variable("x"), integer(8)),
            SepAssertion::and(
                SepAssertion::exists(Text::from("x"), equal(variable("x"), integer(16))),
                SepAssertion::points_to(variable("x"), integer(1)),
            ),
        ),
    );
    valid(&nested, &cell(8, 1));
    valid(&cell(8, 1), &nested);
    invalid(&nested, &cell(16, 1));
}
