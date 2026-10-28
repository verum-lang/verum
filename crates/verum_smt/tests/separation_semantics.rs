//! Semantic controls for the encoder used by compiler separation obligations.
use verum_ast::literal::{IntLit, IntSuffix, Literal, LiteralKind};
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
    let result = entails(pre, post);
    assert!(
        matches!(result, EntailmentResult::Valid { .. }),
        "{pre:?} |- {post:?}: {result:?}"
    );
}

fn invalid(pre: &SepAssertion, post: &SepAssertion) {
    let result = entails(pre, post);
    assert!(
        matches!(result, EntailmentResult::Invalid { .. }),
        "{pre:?} |- {post:?}: {result:?}"
    );
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

#[test]
fn repeated_free_variables_retain_identity_without_annotations() {
    let p = SepAssertion::points_to(variable("address"), variable("value"));
    valid(&p, &p);
    invalid(
        &p,
        &SepAssertion::points_to(variable("other"), variable("value")),
    );
    let flag = SepAssertion::pure(variable("flag"));
    valid(&flag, &flag);
    invalid(&SepAssertion::Emp, &flag);
}

#[test]
fn named_predicates_preserve_names_arguments_and_heap_authority() {
    let p = SepAssertion::named(Text::from("owned"), [integer(1)].into_iter().collect());
    let different_argument =
        SepAssertion::named(Text::from("owned"), [integer(2)].into_iter().collect());
    let different_name =
        SepAssertion::named(Text::from("ready"), [integer(1)].into_iter().collect());
    valid(&p, &p);
    invalid(&SepAssertion::Emp, &p);
    invalid(&p, &different_argument);
    invalid(&p, &different_name);
    invalid(&p, &SepAssertion::Emp);
}

#[test]
fn unsupported_terms_and_sort_conflicts_are_errors_in_either_polarity() {
    let too_wide = Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit::new(i128::from(i64::MAX) + 1)),
            Span::dummy(),
        )),
        Span::dummy(),
    );
    for unsupported in [
        SepAssertion::pure(integer(1)),
        SepAssertion::points_to(integer(8), too_wide),
        SepAssertion::and(
            SepAssertion::pure(variable("x")),
            SepAssertion::points_to(variable("x"), integer(1)),
        ),
    ] {
        let encoder = SepLogicEncoder::new(SepLogicConfig::default());
        assert!(
            encoder
                .verify_entailment(&SepAssertion::Emp, &unsupported)
                .is_err()
        );
        assert!(
            encoder
                .verify_entailment(&unsupported, &SepAssertion::Emp)
                .is_err()
        );
        // A failed obligation cannot leave variable bindings in the next one.
        assert!(matches!(
            encoder.verify_entailment(&cell(8, 1), &cell(8, 1)),
            Ok(EntailmentResult::Valid { .. })
        ));
    }
}

#[test]
fn unfolding_limits_cannot_manufacture_a_proof() {
    let unknown_layout = SepAssertion::list_segment(
        integer(8),
        integer(24),
        [integer(1), integer(2)].into_iter().collect(),
    );
    for depth in [0, 1, 10] {
        let encoder = SepLogicEncoder::new(SepLogicConfig {
            max_unfolding_depth: depth,
            ..Default::default()
        });
        assert!(
            encoder
                .verify_entailment(&SepAssertion::Emp, &unknown_layout)
                .is_err()
        );
        assert!(
            encoder
                .verify_entailment(&unknown_layout, &SepAssertion::Emp)
                .is_err()
        );
    }
    valid(
        &SepAssertion::Emp,
        &SepAssertion::empty_list_segment(integer(8)),
    );
}

#[test]
fn compiler_dispatch_preserves_invalid_and_unsupported_outcomes() {
    use verum_smt::separation_recognizer::{SepObligationOutcome, verify_separation_obligation};
    fn call(name: &str, args: impl IntoIterator<Item = Expr>) -> Expr {
        Expr::new(
            ExprKind::Call {
                func: Heap::new(variable(name)),
                type_args: Default::default(),
                args: args.into_iter().collect(),
            },
            Span::dummy(),
        )
    }
    let empty = call("emp", []);
    let point = call("points_to", [variable("address"), integer(42)]);
    assert!(matches!(
        verify_separation_obligation(&point, &point),
        SepObligationOutcome::Valid
    ));
    assert!(matches!(
        verify_separation_obligation(&point, &empty),
        SepObligationOutcome::Invalid { .. }
    ));
    let unsupported = call("pure", [call("uninterpreted_user_call", [])]);
    assert!(matches!(
        verify_separation_obligation(&empty, &unsupported),
        SepObligationOutcome::Unknown { .. }
    ));
    assert!(matches!(
        verify_separation_obligation(&unsupported, &empty),
        SepObligationOutcome::Unknown { .. }
    ));
}

#[test]
fn literal_suffixes_cannot_silently_change_the_scalar_model() {
    for suffix in [
        IntSuffix::I8,
        IntSuffix::I16,
        IntSuffix::I32,
        IntSuffix::I128,
        IntSuffix::Isize,
        IntSuffix::U8,
        IntSuffix::U16,
        IntSuffix::U32,
        IntSuffix::U64,
        IntSuffix::U128,
        IntSuffix::Usize,
        IntSuffix::Custom("km".into()),
    ] {
        let expression = Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit::with_suffix(1, suffix)),
                Span::dummy(),
            )),
            Span::dummy(),
        );
        let assertion = equal(expression, integer(1));
        let encoder = SepLogicEncoder::new(SepLogicConfig::default());
        assert!(encoder.verify_entailment(&assertion, &assertion).is_err());
        assert!(
            encoder
                .verify_entailment(&SepAssertion::Emp, &assertion)
                .is_err()
        );
        assert!(
            encoder
                .verify_entailment(&assertion, &SepAssertion::Emp)
                .is_err()
        );
    }
    let signed = Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit::with_suffix(1, IntSuffix::I64)),
            Span::dummy(),
        )),
        Span::dummy(),
    );
    valid(&SepAssertion::Emp, &equal(signed, integer(1)));
}

#[test]
fn public_encoder_rejects_malformed_heap_sorts() {
    use z3::{Sort, ast::Array};
    let word = Sort::bitvector(64);
    let heap = Array::fresh_const("heap", &word, &word);
    let allocated = Array::fresh_const("allocated", &word, &Sort::bool());
    let bad_index = Array::fresh_const("bad_index", &Sort::int(), &word);
    let bad_value = Array::fresh_const("bad_value", &word, &Sort::int());
    let encoder = SepLogicEncoder::new(SepLogicConfig::default());
    for (heap, allocated) in [
        (&bad_index, &allocated),
        (&bad_value, &allocated),
        (&heap, &heap),
    ] {
        assert!(
            encoder
                .encode_assertion(&SepAssertion::Emp, heap, allocated)
                .is_err()
        );
        assert!(
            encoder
                .encode_assertion(&cell(8, 1), heap, allocated)
                .is_err()
        );
    }
}

#[test]
fn separating_implication_preserves_owned_values_and_empty_extension() {
    valid(
        &cell(16, 2),
        &SepAssertion::wand(cell(8, 1), SepAssertion::sep(cell(8, 1), cell(16, 2))),
    );
    invalid(&cell(16, 2), &SepAssertion::wand(cell(8, 1), cell(8, 1)));
    let p = SepAssertion::named("owned", Default::default());
    let extended = SepAssertion::wand(SepAssertion::Emp, p.clone());
    valid(&p, &extended);
    valid(&extended, &p);
}

#[test]
fn quantified_bindings_do_not_escape_success_or_failure() {
    valid(
        &SepAssertion::Emp,
        &SepAssertion::forall(Text::from("x"), equal(variable("x"), variable("x"))),
    );
    invalid(
        &SepAssertion::exists(Text::from("x"), equal(variable("x"), integer(8))),
        &equal(variable("x"), integer(8)),
    );

    let encoder = SepLogicEncoder::new(SepLogicConfig::default());
    let model = verum_smt::separation_logic::Z3HeapModel::new();
    let free = SepAssertion::pure(variable("x"));
    let before = encoder
        .encode_assertion(&free, model.heap(), model.allocated())
        .unwrap();
    let failing = SepAssertion::exists(
        Text::from("x"),
        SepAssertion::and(
            equal(variable("x"), integer(8)),
            SepAssertion::pure(integer(1)),
        ),
    );
    assert!(
        encoder
            .encode_assertion(&failing, model.heap(), model.allocated())
            .is_err()
    );
    let after = encoder
        .encode_assertion(&free, model.heap(), model.allocated())
        .unwrap();
    assert_eq!(
        before, after,
        "failed quantification must restore the free Boolean binding"
    );
}
