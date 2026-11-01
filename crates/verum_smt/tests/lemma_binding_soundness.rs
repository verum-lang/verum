use verum_ast::{BinOp, Expr, ExprKind, Ident, Literal, Path, Span};
use verum_common::{Heap, List, Maybe};
use verum_smt::proof_search::{
    HintsDatabase, LemmaHint, ProofGoal, ProofSearchEngine, ProofTactic,
};

fn var(name: &str) -> Expr {
    Expr::path(Path::from_ident(Ident::new(name, Span::dummy())))
}
fn int(value: i128) -> Expr {
    Expr::literal(Literal::int(value, Span::dummy()))
}
fn bin(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        Span::dummy(),
    )
}
fn eq(left: Expr, right: Expr) -> Expr {
    bin(BinOp::Eq, left, right)
}
fn call(name: &str, args: Vec<Expr>) -> Expr {
    Expr::new(
        ExprKind::Call {
            func: Heap::new(var(name)),
            type_args: List::new(),
            args: args.into(),
        },
        Span::dummy(),
    )
}
fn engine(name: &str, params: &[&str], lemma: Expr) -> ProofSearchEngine {
    let mut hints = HintsDatabase::new();
    hints.register_named_lemma(
        name.into(),
        LemmaHint {
            name: name.into(),
            priority: 100,
            params: params.iter().map(|p| (*p).into()).collect(),
            lemma: Heap::new(lemma),
        },
    );
    ProofSearchEngine::with_hints(hints)
}

#[test]
fn exact_and_apply_require_the_same_instantiated_premises() {
    let zero_x = eq(var("x"), int(0));
    let zero_y = eq(var("y"), int(0));
    let mut engine = engine(
        "zero_identity",
        &["x"],
        bin(BinOp::Imply, zero_x.clone(), zero_x.clone()),
    );
    let bad = ProofGoal::with_hypotheses(zero_y.clone(), List::from_iter([zero_x]));
    assert!(
        engine
            .execute_tactic(
                &ProofTactic::Exact {
                    term: "zero_identity".into()
                },
                &bad
            )
            .is_err()
    );
    let obligations = engine
        .execute_tactic(
            &ProofTactic::Apply {
                lemma: "zero_identity".into(),
            },
            &bad,
        )
        .unwrap();
    assert_eq!(obligations.len(), 1);
    assert_eq!(obligations[0].goal, zero_y);
    assert!(
        engine
            .execute_tactic(&ProofTactic::Auto, &obligations[0])
            .is_err()
    );
    let good = ProofGoal::with_hypotheses(zero_y.clone(), List::from_iter([zero_y]));
    assert!(
        engine
            .execute_tactic(
                &ProofTactic::Exact {
                    term: "zero_identity".into()
                },
                &good
            )
            .unwrap()
            .is_empty()
    );
    let backward = engine.hints().backward_reasoning(&good.goal);
    assert_eq!(backward.len(), 1);
    assert_eq!(backward[0].1[0], good.goal);
}

#[test]
fn rigid_function_and_constant_names_cannot_be_inferred_as_parameters() {
    let mut engine = engine("known_fact", &[], call("known", vec![int(0)]));
    let goal = ProofGoal::new(call("unrelated", vec![int(0)]));
    for tactic in [
        ProofTactic::Apply {
            lemma: "known_fact".into(),
        },
        ProofTactic::Exact {
            term: "known_fact".into(),
        },
    ] {
        assert!(engine.execute_tactic(&tactic, &goal).is_err(), "{tactic:?}");
    }
    assert!(engine.hints().backward_reasoning(&goal.goal).is_empty());
}

#[test]
fn explicit_arguments_are_real_expressions_and_cannot_be_reinferred() {
    let mut engine = engine("identity", &["x"], eq(var("x"), var("x")));
    let value = bin(BinOp::Add, var("n"), int(1));
    let args = List::from_iter([value.clone()]);
    let (_, conclusion) = engine.instantiate_lemma(&"identity".into(), &args).unwrap();
    assert_eq!(conclusion, eq(value.clone(), value.clone()));
    let tactic = ProofTactic::ApplyWith {
        lemma: "identity".into(),
        args,
    };
    assert!(
        engine
            .execute_tactic(&tactic, &ProofGoal::new(eq(int(2), int(2))))
            .is_err()
    );
    assert!(
        engine
            .execute_tactic(&tactic, &ProofGoal::new(eq(value.clone(), value)))
            .unwrap()
            .is_empty()
    );
    for args in [List::new(), List::from_iter([int(0), int(1)])] {
        assert!(engine.instantiate_lemma(&"identity".into(), &args).is_err());
    }
}

#[test]
fn repeated_parameter_occurrences_share_one_substitution() {
    let mut engine = engine("reflexive", &["x"], eq(var("x"), var("x")));
    for tactic in [
        ProofTactic::Apply {
            lemma: "reflexive".into(),
        },
        ProofTactic::Exact {
            term: "reflexive".into(),
        },
    ] {
        assert!(
            engine
                .execute_tactic(&tactic, &ProofGoal::new(eq(int(0), int(1))))
                .is_err()
        );
    }
}

#[test]
fn core_schemas_are_actual_laws_with_explicit_parameters() {
    let hints = HintsDatabase::with_core();
    let arithmetic = [
        "plus_comm",
        "mult_comm",
        "plus_assoc",
        "mult_assoc",
        "plus_zero",
        "mult_one",
        "mult_zero",
        "mult_dist_plus",
        "eq_refl",
        "eq_symm",
        "eq_trans",
    ];
    let boolean = [
        "demorgan_and",
        "demorgan_or",
        "and_properties",
        "and_comm",
        "or_comm",
        "and_true",
        "or_false",
        "double_negation",
        "modus_ponens",
        "implication_trans",
    ];
    for name in arithmetic.into_iter().chain(boolean) {
        let hint = hints.lookup_lemma_by_name(&name.into()).unwrap();
        assert!(!hint.params.is_empty(), "{name}");
        assert!(
            matches!(hint.lemma.kind, ExprKind::Binary { .. }),
            "{name} became a true placeholder"
        );
        let mut checker = ProofSearchEngine::with_hints(HintsDatabase::new());
        if boolean.contains(&name) {
            for parameter in &hint.params {
                checker.register_value_sort(parameter.clone(), "Bool".into());
                checker.register_bool_hypothesis(parameter.clone());
            }
        }
        assert!(
            checker
                .execute_tactic(
                    &ProofTactic::Smt {
                        solver: Maybe::None,
                        timeout_ms: Maybe::None
                    },
                    &ProofGoal::new(hint.lemma.as_ref().clone())
                )
                .unwrap()
                .is_empty(),
            "{name}"
        );
    }
    let mut checker = ProofSearchEngine::with_hints(hints);
    let goal = ProofGoal::new(eq(
        bin(BinOp::Add, var("x"), var("y")),
        bin(BinOp::Add, var("y"), var("x")),
    ));
    assert!(
        checker
            .execute_tactic(
                &ProofTactic::Exact {
                    term: "plus_comm".into()
                },
                &goal
            )
            .unwrap()
            .is_empty()
    );
    let false_goal = ProofGoal::new(eq(bin(BinOp::Add, var("x"), var("y")), var("x")));
    assert!(
        checker
            .execute_tactic(
                &ProofTactic::Exact {
                    term: "plus_comm".into()
                },
                &false_goal
            )
            .is_err()
    );
}
