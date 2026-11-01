use verum_ast::{
    Expr,
    expr::{BinOp, ExprKind, UnOp},
    literal::{IntLit, IntSuffix, Literal, LiteralKind},
    span::Span,
    ty::{Ident, Path, PathSegment},
};
use verum_common::List;
use verum_types::{
    refinement::RefinementPredicate,
    refinement_entailment::{
        BoundImplication, IntegerComparison, Relation, entails, entails_with_limits,
    },
    ty::Type,
    type_algebra::Limits,
};

fn integer(value: i128) -> Expr {
    Expr::literal(Literal::int(value, Span::dummy()))
}

fn variable(name: &str) -> Expr {
    Expr::ident(Ident::new(name, Span::dummy()))
}

fn binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        Span::dummy(),
    )
}

fn comparison(name: &str, op: BinOp, bound: i128) -> Expr {
    binary(op, variable(name), integer(bound))
}

fn predicate(name: &str, expression: Expr) -> RefinementPredicate {
    RefinementPredicate::lambda(expression, name.into(), Span::dummy())
}

fn proves(source: Expr, target: Expr) -> bool {
    entails(
        &Type::Int,
        &predicate("x", source),
        &Type::Int,
        &predicate("x", target),
    )
}

#[derive(Clone, Copy, Debug)]
enum Sample {
    BelowLiteralRange,
    Value(i128),
    AboveLiteralRange,
}

fn holds(comparison: IntegerComparison, sample: Sample) -> bool {
    use std::cmp::Ordering;
    let order = match sample {
        Sample::BelowLiteralRange => Ordering::Less,
        Sample::Value(value) => value.cmp(&comparison.bound),
        Sample::AboveLiteralRange => Ordering::Greater,
    };
    match comparison.relation {
        Relation::Eq => order == Ordering::Equal,
        Relation::Ne => order != Ordering::Equal,
        Relation::Lt => order == Ordering::Less,
        Relation::Le => order != Ordering::Greater,
        Relation::Gt => order == Ordering::Greater,
        Relation::Ge => order != Ordering::Less,
    }
}

#[test]
fn arithmetic_witnesses_agree_with_integer_valuations_at_every_boundary() {
    let relations = [
        Relation::Eq,
        Relation::Ne,
        Relation::Lt,
        Relation::Le,
        Relation::Gt,
        Relation::Ge,
    ];
    let bounds = [
        i128::MIN,
        i128::MIN + 1,
        -3,
        -1,
        0,
        1,
        3,
        i128::MAX - 1,
        i128::MAX,
    ];
    for source_relation in relations {
        for target_relation in relations {
            for a in bounds {
                for b in bounds {
                    let witness = BoundImplication {
                        source: IntegerComparison {
                            relation: source_relation,
                            bound: a,
                        },
                        target: IntegerComparison {
                            relation: target_relation,
                            bound: b,
                        },
                    };
                    // Each comparison changes truth only at its bound. Include
                    // both bounds, representable neighbors and the two tails.
                    let mut samples =
                        List::from(vec![Sample::BelowLiteralRange, Sample::AboveLiteralRange]);
                    for bound in [a, b] {
                        samples.push(Sample::Value(bound));
                        if let Some(before) = bound.checked_sub(1) {
                            samples.push(Sample::Value(before));
                        }
                        if let Some(after) = bound.checked_add(1) {
                            samples.push(Sample::Value(after));
                        }
                    }
                    let expected = samples.iter().all(|sample| {
                        !holds(witness.source, *sample) || holds(witness.target, *sample)
                    });
                    assert_eq!(witness.check(), expected, "{witness:?}");
                }
            }
        }
    }
}

#[test]
fn canonical_int_literals_and_strict_bounds_do_not_overflow_or_truncate() {
    let minimum = i128::from(i64::MIN);
    let maximum = i128::from(i64::MAX);
    for bound in [minimum, maximum] {
        assert!(proves(
            comparison("x", BinOp::Gt, bound),
            comparison("x", BinOp::Ge, bound)
        ));
        assert!(proves(
            comparison("x", BinOp::Lt, bound),
            comparison("x", BinOp::Le, bound)
        ));
    }
    assert!(proves(
        comparison("x", BinOp::Gt, maximum - 1),
        comparison("x", BinOp::Ge, maximum)
    ));
    assert!(proves(
        comparison("x", BinOp::Lt, minimum + 1),
        comparison("x", BinOp::Le, minimum)
    ));
    assert!(!proves(
        comparison("x", BinOp::Eq, maximum),
        comparison("x", BinOp::Eq, -1)
    ));

    // The AST encodes the negative endpoint as unary minus applied to a
    // magnitude one beyond Int::MAX. Range validation follows negation.
    let negative_endpoint = Expr::new(
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: Box::new(integer(maximum + 1)),
        },
        Span::dummy(),
    );
    assert!(proves(
        binary(BinOp::Ge, variable("x"), negative_endpoint),
        comparison("x", BinOp::Ge, minimum),
    ));
}

#[test]
fn literals_outside_the_canonical_int_domain_are_unproved() {
    for bound in [
        i128::MIN,
        i128::MAX,
        i128::from(i64::MIN) - 1,
        i128::from(i64::MAX) + 1,
    ] {
        let outside = || comparison("x", BinOp::Eq, bound);
        assert!(!proves(outside(), outside()));
        assert!(!proves(outside(), comparison("x", BinOp::Ne, 0)));
        assert!(!proves(comparison("x", BinOp::Eq, 0), outside()));
    }
}

#[test]
fn recorded_binders_can_be_alpha_renamed_but_free_variables_cannot() {
    let source = predicate("x", comparison("x", BinOp::Gt, 10));
    let target = predicate("y", comparison("y", BinOp::Ge, 0));
    assert!(entails(&Type::Int, &source, &Type::Int, &target));
    let wrong = predicate("y", comparison("x", BinOp::Gt, 10));
    assert!(!entails(&Type::Int, &wrong, &Type::Int, &source));
    assert!(!entails(&Type::Int, &source, &Type::Int, &wrong));
    assert!(!entails(&Type::Int, &wrong, &Type::Int, &wrong));
    let sigma =
        RefinementPredicate::sigma(comparison("z", BinOp::Gt, 0), "z".into(), Span::dummy());
    assert!(entails(&Type::Int, &source, &Type::Int, &sigma));
}

#[test]
fn implicit_subject_aliases_respect_explicit_binder_shadowing() {
    for name in ["it", "self"] {
        let source = RefinementPredicate::inline(comparison(name, BinOp::Gt, 10), Span::dummy());
        let target = RefinementPredicate::bare(comparison(name, BinOp::Gt, 0), Span::dummy());
        assert!(entails(&Type::Int, &source, &Type::Int, &target));
        let shadowed = predicate("x", comparison(name, BinOp::Gt, 10));
        assert!(!entails(&Type::Int, &shadowed, &Type::Int, &shadowed));
    }
    let self_path = Expr::path(Path::new(
        vec![PathSegment::SelfValue].into(),
        Span::dummy(),
    ));
    let source =
        RefinementPredicate::inline(binary(BinOp::Gt, self_path, integer(10)), Span::dummy());
    let target = RefinementPredicate::bare(comparison("it", BinOp::Gt, 0), Span::dummy());
    assert!(entails(&Type::Int, &source, &Type::Int, &target));
    let free = RefinementPredicate::inline(comparison("x", BinOp::Gt, 10), Span::dummy());
    assert!(!entails(&Type::Int, &free, &Type::Int, &free));
    let explicit_it = predicate("it", comparison("it", BinOp::Gt, 10));
    assert!(explicit_it.binds_subject("it"));
    assert!(!explicit_it.binds_subject("self"));
    let collision = predicate("it", comparison("self", BinOp::Gt, 10));
    assert!(!entails(&Type::Int, &collision, &Type::Int, &explicit_it));
    let qualified_it = Expr::path(Path::new(
        vec![
            PathSegment::Name(Ident::new("other", Span::dummy())),
            PathSegment::Name(Ident::new("it", Span::dummy())),
        ]
        .into(),
        Span::dummy(),
    ));
    let qualified =
        RefinementPredicate::inline(binary(BinOp::Gt, qualified_it, integer(0)), Span::dummy());
    assert!(!entails(&Type::Int, &qualified, &Type::Int, &qualified));
}

#[test]
fn integer_rules_cannot_discharge_float_or_overloaded_comparisons() {
    let source = predicate("x", comparison("x", BinOp::Gt, 0));
    let target = predicate("x", comparison("x", BinOp::Ge, 1));
    let custom = Type::Named {
        path: Path::single(Ident::new("OrderedValue", Span::dummy())),
        args: List::new(),
    };
    for base in [Type::Float, Type::Bool, custom] {
        assert!(!entails(&base, &source, &base, &target));
        assert!(!entails(&base, &source, &Type::Int, &target));
        assert!(!entails(&Type::Int, &source, &base, &target));
    }
    assert!(entails(&Type::Int, &source, &Type::Int, &target));
}

#[test]
fn boolean_distribution_uses_checked_numeric_consequences() {
    let source = binary(
        BinOp::Or,
        binary(
            BinOp::And,
            comparison("x", BinOp::Gt, 10),
            comparison("x", BinOp::Lt, 20),
        ),
        comparison("x", BinOp::Eq, 100),
    );
    let target = binary(
        BinOp::And,
        binary(
            BinOp::Or,
            comparison("x", BinOp::Gt, 0),
            comparison("x", BinOp::Eq, 100),
        ),
        binary(
            BinOp::Or,
            comparison("x", BinOp::Lt, 30),
            comparison("x", BinOp::Eq, 100),
        ),
    );
    assert!(proves(source.clone(), target.clone()));
    assert!(!proves(target, source));
    for value in -5..=105 {
        let source_holds = (value > 10 && value < 20) || value == 100;
        let target_holds = (value > 0 || value == 100) && (value < 30 || value == 100);
        assert!(!source_holds || target_holds);
    }
}

#[test]
fn booleans_preserve_vacuity_without_arbitrary_strengthening() {
    let truth = || Expr::literal(Literal::bool(true, Span::dummy()));
    let falsity = || Expr::literal(Literal::bool(false, Span::dummy()));
    let positive = || comparison("x", BinOp::Gt, 0);
    assert!(proves(falsity(), positive()));
    assert!(proves(positive(), truth()));
    assert!(!proves(truth(), positive()));
    assert!(!proves(truth(), falsity()));
    assert!(proves(binary(BinOp::And, positive(), truth()), positive()));
}

#[test]
fn stateful_partial_and_unresolved_atoms_are_not_assumed_stable() {
    let call = Expr::new(
        ExprKind::Call {
            func: Box::new(variable("observe")),
            type_args: List::new(),
            args: vec![variable("x")].into(),
        },
        Span::dummy(),
    );
    let divided = binary(
        BinOp::Gt,
        binary(BinOp::Div, variable("x"), integer(0)),
        integer(0),
    );
    let arithmetic = binary(
        BinOp::Gt,
        binary(BinOp::Add, variable("x"), integer(1)),
        integer(0),
    );
    for unsupported in [call, divided, arithmetic, variable("external_bool")] {
        assert!(!proves(unsupported.clone(), unsupported));
    }
    let named = RefinementPredicate::named(
        Path::single(Ident::new("positive", Span::dummy())),
        Span::dummy(),
    );
    assert!(!entails(&Type::Int, &named, &Type::Int, &named));
}

#[test]
fn literal_suffixes_and_unrepresentable_negation_are_refused() {
    let suffixed = Expr::literal(Literal::new(
        LiteralKind::Int(IntLit {
            value: 1,
            suffix: Some(IntSuffix::U8),
        }),
        Span::dummy(),
    ));
    let overflowing_negation = Expr::new(
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: Box::new(integer(i128::MIN)),
        },
        Span::dummy(),
    );
    for unsupported in [
        suffixed,
        overflowing_negation,
        Expr::literal(Literal::float(1.0, Span::dummy())),
    ] {
        let comparison = binary(BinOp::Gt, variable("x"), unsupported);
        assert!(!proves(comparison.clone(), comparison));
    }
    let negative = Expr::new(
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: Box::new(integer(10)),
        },
        Span::dummy(),
    );
    assert!(proves(
        binary(BinOp::Gt, variable("x"), negative),
        comparison("x", BinOp::Gt, -11)
    ));
}

#[test]
fn reversing_comparison_operands_preserves_the_relation() {
    assert!(proves(
        binary(BinOp::Lt, integer(10), variable("x")),
        comparison("x", BinOp::Gt, 0)
    ));
    assert!(!proves(
        binary(BinOp::Gt, integer(10), variable("x")),
        comparison("x", BinOp::Gt, 0)
    ));
}

#[test]
fn resource_exhaustion_is_unproved() {
    let predicate = predicate("x", comparison("x", BinOp::Gt, 0));
    let zero_nodes = Limits {
        max_expression_nodes: 0,
        ..Limits::default()
    };
    let zero_search = Limits {
        max_sequents: 0,
        ..Limits::default()
    };
    for limits in [zero_nodes, zero_search] {
        assert!(!entails_with_limits(
            &Type::Int,
            &predicate,
            &Type::Int,
            &predicate,
            limits
        ));
    }
}
