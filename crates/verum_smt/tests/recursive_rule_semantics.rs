//! Semantic controls for recursive Horn rules, not construction-only smoke tests.
use verum_common::{List, Text};
use verum_smt::fixedpoint::{
    Atom, DatalogSolver, FixedPointEngine, PredicateBody, PredicateCase, RecursiveCall,
    RecursivePredicate,
};
use z3::{
    Context, FuncDecl, Params, SatResult, Sort,
    ast::{Bool, Int},
};

fn engine() -> FixedPointEngine {
    let mut engine = FixedPointEngine::new(Context::thread_local()).unwrap();
    let mut params = Params::new();
    params.set_symbol("engine", "spacer");
    params.set_u32("timeout", 5000);
    engine.set_params(&params);
    engine
}

fn atom(name: &str) -> Bool {
    FuncDecl::new(name, &[], &Sort::bool())
        .apply(&[])
        .as_bool()
        .unwrap()
}

fn predicate(name: &str, body: PredicateBody) -> RecursivePredicate {
    RecursivePredicate {
        vars: Default::default(),
        name: Text::from(name),
        params: List::new(),
        body,
        well_founded: false,
    }
}

fn call(name: &str) -> RecursiveCall {
    RecursiveCall {
        predicate: name.into(),
        args: List::new(),
    }
}

#[test]
fn a_recursive_premise_cannot_be_assumed() {
    let mut engine = engine();
    engine
        .register_predicate(predicate(
            "looping",
            PredicateBody::Recursive {
                guard: Bool::from_bool(true),
                recursive_calls: [call("looping")].into_iter().collect(),
                conclusion: atom("looping"),
            },
        ))
        .unwrap();
    assert_eq!(engine.query(&atom("looping")).unwrap(), SatResult::Unsat);
}

#[test]
fn a_case_guard_is_required() {
    let mut engine = engine();
    engine
        .register_predicate(predicate(
            "guarded",
            PredicateBody::Cases(
                [PredicateCase {
                    guard: Some(Bool::from_bool(false)),
                    body: atom("guarded"),
                    recursive_calls: List::new(),
                }]
                .into_iter()
                .collect(),
            ),
        ))
        .unwrap();
    assert_eq!(engine.query(&atom("guarded")).unwrap(), SatResult::Unsat);
}

#[test]
fn every_recursive_call_is_required_and_a_real_base_case_enables_derivation() {
    let mut engine = engine();
    let first = engine
        .register_relation(FuncDecl::new("first", &[], &Sort::bool()))
        .unwrap();
    let second = engine
        .register_relation(FuncDecl::new("second", &[], &Sort::bool()))
        .unwrap();
    engine.add_fact(&first, &[]).unwrap();
    engine
        .register_predicate(predicate(
            "both",
            PredicateBody::Cases(
                [PredicateCase {
                    guard: Some(Bool::from_bool(true)),
                    body: atom("both"),
                    recursive_calls: [call("first"), call("second")].into_iter().collect(),
                }]
                .into_iter()
                .collect(),
            ),
        ))
        .unwrap();
    assert_eq!(engine.query(&atom("both")).unwrap(), SatResult::Unsat);
    engine.add_fact(&second, &[]).unwrap();
    assert_eq!(engine.query(&atom("both")).unwrap(), SatResult::Sat);
}

#[test]
fn invalid_definition_does_not_install_a_partial_fact() {
    let mut engine = engine();
    let result = engine.register_predicate(predicate(
        "atomic",
        PredicateBody::Cases(
            [
                PredicateCase {
                    guard: None,
                    body: atom("atomic"),
                    recursive_calls: List::new(),
                },
                PredicateCase {
                    guard: None,
                    body: atom("atomic"),
                    recursive_calls: [call("missing")].into_iter().collect(),
                },
            ]
            .into_iter()
            .collect(),
        ),
    ));
    assert!(result.is_err());
    assert_eq!(engine.get_statistics().num_rules, 0);
    assert_eq!(engine.get_statistics().num_predicates, 0);
    engine
        .register_relation(FuncDecl::new("atomic", &[], &Sort::bool()))
        .unwrap();
    assert_eq!(engine.query(&atom("atomic")).unwrap(), SatResult::Unsat);
}

#[test]
fn boolean_constants_are_not_relation_definitions() {
    let mut engine = engine();
    assert!(
        engine
            .register_predicate(predicate(
                "invented",
                PredicateBody::Base(Bool::from_bool(true))
            ))
            .is_err()
    );
    assert_eq!(engine.get_statistics().num_rules, 0);
    assert_eq!(engine.get_statistics().num_predicates, 0);
}

#[test]
fn datalog_registration_is_empty_and_facts_have_exact_answers() {
    let mut solver = DatalogSolver::new(Context::thread_local()).unwrap();
    solver.register_relation("node", &[Sort::int()]).unwrap();
    let node = |n| Atom {
        predicate: "node".into(),
        args: [Int::from_i64(n).into()].into_iter().collect(),
    };
    assert!(!solver.query(node(1)).unwrap());
    solver.add_fact(node(1)).unwrap();
    assert!(solver.query(node(1)).unwrap());
    assert!(!solver.query(node(2)).unwrap());
}

#[test]
fn recursive_definition_binds_variables_and_preserves_guards() {
    let mut engine = engine();
    let relation = FuncDecl::new("bounded", &[&Sort::int()], &Sort::bool());
    let x = Int::new_const("x");
    engine
        .register_predicate(RecursivePredicate {
            vars: [(Text::from("x"), Sort::int())].into_iter().collect(),
            name: "bounded".into(),
            params: [Sort::int()].into_iter().collect(),
            body: PredicateBody::Cases(
                [
                    PredicateCase {
                        guard: None,
                        body: relation.apply(&[&Int::from_i64(0)]).as_bool().unwrap(),
                        recursive_calls: List::new(),
                    },
                    PredicateCase {
                        guard: Some(x.lt(Int::from_i64(3))),
                        body: relation
                            .apply(&[&Int::add(&[&x, &Int::from_i64(1)])])
                            .as_bool()
                            .unwrap(),
                        recursive_calls: [RecursiveCall {
                            predicate: "bounded".into(),
                            args: [x.into()].into_iter().collect(),
                        }]
                        .into_iter()
                        .collect(),
                    },
                ]
                .into_iter()
                .collect(),
            ),
            well_founded: false,
        })
        .unwrap();
    for (n, expected) in [
        (-1, SatResult::Unsat),
        (0, SatResult::Sat),
        (3, SatResult::Sat),
        (4, SatResult::Unsat),
    ] {
        assert_eq!(
            engine
                .query(&relation.apply(&[&Int::from_i64(n)]).as_bool().unwrap())
                .unwrap(),
            expected
        );
    }
}

#[test]
fn unbound_rule_variables_are_refused_without_adding_rules() {
    use verum_smt::fixedpoint::CHC;
    let mut engine = engine();
    engine
        .register_relation(FuncDecl::new("p", &[&Sort::int()], &Sort::bool()))
        .unwrap();
    let rule = |vars| CHC {
        vars,
        hypothesis: List::new(),
        constraints: List::new(),
        conclusion: Atom {
            predicate: "p".into(),
            args: [Int::new_const("x").into()].into_iter().collect(),
        },
    };
    assert!(engine.add_chc(rule(List::new())).is_err());
    assert!(
        engine
            .add_chc(rule(
                [(Text::from("x"), Sort::bool())].into_iter().collect()
            ))
            .is_err()
    );
    assert_eq!(engine.get_statistics().num_rules, 0);
    engine
        .add_chc(rule([(Text::from("x"), Sort::int())].into_iter().collect()))
        .unwrap();
    assert_eq!(engine.get_statistics().num_rules, 1);
}

#[test]
fn datalog_transitive_closure_uses_explicit_binders_and_actual_edges() {
    use verum_smt::fixedpoint::DatalogRule;
    let mut solver = DatalogSolver::new(Context::thread_local()).unwrap();
    for name in ["edge", "path"] {
        solver
            .register_relation(name, &[Sort::int(), Sort::int()])
            .unwrap();
    }
    let pair = |name: &str, a: Int, b: Int| Atom {
        predicate: name.into(),
        args: [a.into(), b.into()].into_iter().collect(),
    };
    for (a, b) in [(1, 2), (2, 3), (3, 1), (3, 4)] {
        solver
            .add_fact(pair("edge", Int::from_i64(a), Int::from_i64(b)))
            .unwrap();
    }
    let x = Int::new_const("x");
    let y = Int::new_const("y");
    let z = Int::new_const("z");
    solver
        .add_rule(DatalogRule {
            vars: ["x", "y"]
                .into_iter()
                .map(|name| (Text::from(name), Sort::int()))
                .collect(),
            head: pair("path", x.clone(), y.clone()),
            body: [pair("edge", x.clone(), y.clone())].into_iter().collect(),
            constraints: List::new(),
        })
        .unwrap();
    solver
        .add_rule(DatalogRule {
            vars: ["x", "y", "z"]
                .into_iter()
                .map(|name| (Text::from(name), Sort::int()))
                .collect(),
            head: pair("path", x.clone(), z.clone()),
            body: [pair("edge", x, y.clone()), pair("path", y, z)]
                .into_iter()
                .collect(),
            constraints: List::new(),
        })
        .unwrap();
    for (a, b, expected) in [(1, 4, true), (1, 1, true), (4, 1, false), (1, 5, false)] {
        assert_eq!(
            solver
                .query(pair("path", Int::from_i64(a), Int::from_i64(b)))
                .unwrap(),
            expected
        );
    }
}
