//! Inductive candidates must satisfy the complete original relation system.
use std::time::Duration;
use verum_common::{List, Map, Text};
use verum_smt::context::ContextConfig;
use verum_smt::fixedpoint::{
    FixedPointEngine, FixedPointQuery, FixedPointSolution, PredicateInterpretation,
    extract_invariants,
};
use z3::{
    Context, FuncDecl, SatResult, Sort,
    ast::{Ast, Bool, Dynamic, Int},
};

fn engine() -> FixedPointEngine {
    FixedPointEngine::new(Context::thread_local()).unwrap()
}

fn candidate(name: &str, parameters: &[Dynamic], formula: Bool) -> FixedPointSolution {
    FixedPointSolution {
        interpretations: [(
            Text::from(name),
            PredicateInterpretation {
                predicate: name.into(),
                parameters: parameters.iter().cloned().collect(),
                formula,
            },
        )]
        .into_iter()
        .collect(),
        invariants: List::new(),
    }
}

fn naturals() -> FixedPointEngine {
    let mut engine = engine();
    let reachable = engine
        .register_relation(FuncDecl::new("reachable", &[&Sort::int()], &Sort::bool()))
        .unwrap();
    engine
        .add_fact(&reachable, &[Int::from_i64(0).into()])
        .unwrap();
    let x = Int::new_const("n");
    let step = reachable
        .apply(&[&x])
        .as_bool()
        .unwrap()
        .implies(reachable.apply(&[&(&x + 1)]).as_bool().unwrap());
    engine
        .add_rule(&z3::ast::forall_const(&[&x], &[], &step), None)
        .unwrap();
    engine
}

#[test]
fn every_registered_relation_needs_an_interpretation() {
    let engine = naturals();
    let missing = FixedPointSolution {
        interpretations: Map::new(),
        invariants: List::new(),
    };
    assert!(
        engine
            .validate_solution(&missing)
            .unwrap_err()
            .contains("Missing")
    );
    assert!(
        engine
            .validate_solution(&candidate("unknown", &[], Bool::from_bool(true)))
            .unwrap_err()
            .contains("Unknown candidate")
    );
    let mut wrong_key = candidate(
        "reachable",
        &[Int::new_const("x").into()],
        Bool::from_bool(true),
    );
    wrong_key
        .interpretations
        .get_mut(&Text::from("reachable"))
        .unwrap()
        .predicate = "other".into();
    assert!(
        engine
            .validate_solution(&wrong_key)
            .unwrap_err()
            .contains("key differs")
    );
}

#[test]
fn base_and_recursive_rules_are_both_checked_universally() {
    let engine = naturals();
    let x = Int::new_const("candidate_x");
    let parameters = [Dynamic::from_ast(&x)];
    assert!(
        engine
            .validate_solution(&candidate("reachable", &parameters, x.ge(0)))
            .unwrap()
    );
    assert!(
        !engine
            .validate_solution(&candidate("reachable", &parameters, x.gt(0)))
            .unwrap()
    );
    assert!(
        !engine
            .validate_solution(&candidate("reachable", &parameters, x.le(0)))
            .unwrap()
    );
    // A closed overapproximation is valid; this does not claim minimality.
    assert!(
        engine
            .validate_solution(&candidate("reachable", &parameters, Bool::from_bool(true)))
            .unwrap()
    );
}

#[test]
fn binary_parameter_order_and_simultaneous_calls_are_preserved() {
    let mut engine = engine();
    let pair = engine
        .register_relation(FuncDecl::new(
            "pair",
            &[&Sort::int(), &Sort::int()],
            &Sort::bool(),
        ))
        .unwrap();
    engine
        .add_fact(&pair, &[Int::from_i64(1).into(), Int::from_i64(2).into()])
        .unwrap();
    let x = Int::new_const("x");
    let y = Int::new_const("y");
    let swap = pair
        .apply(&[&x, &y])
        .as_bool()
        .unwrap()
        .implies(pair.apply(&[&y, &x]).as_bool().unwrap());
    engine
        .add_rule(&z3::ast::forall_const(&[&x, &y], &[], &swap), None)
        .unwrap();
    let parameters = [Dynamic::from_ast(&x), Dynamic::from_ast(&y)];
    assert!(
        engine
            .validate_solution(&candidate("pair", &parameters, (&x + &y).eq(3)))
            .unwrap()
    );
    assert!(
        !engine
            .validate_solution(&candidate("pair", &parameters, x.le(&y)))
            .unwrap()
    );
    assert!(
        engine
            .validate_solution(&candidate(
                "pair",
                &[x.clone().into(), x.into()],
                Bool::from_bool(true)
            ))
            .unwrap_err()
            .contains("distinct symbolic")
    );
}

#[test]
fn all_registered_relations_and_cross_relation_rules_are_checked() {
    let mut engine = engine();
    let p = engine
        .register_relation(FuncDecl::new("p", &[], &Sort::bool()))
        .unwrap();
    let q = engine
        .register_relation(FuncDecl::new("q", &[], &Sort::bool()))
        .unwrap();
    engine.add_fact(&p, &[]).unwrap();
    engine
        .add_rule(
            &p.apply(&[])
                .as_bool()
                .unwrap()
                .implies(q.apply(&[]).as_bool().unwrap()),
            None,
        )
        .unwrap();
    let mut model = candidate("p", &[], Bool::from_bool(true));
    assert!(
        engine
            .validate_solution(&model)
            .unwrap_err()
            .contains("Missing")
    );
    model
        .interpretations
        .extend(candidate("q", &[], Bool::from_bool(false)).interpretations);
    assert!(!engine.validate_solution(&model).unwrap());
    model
        .interpretations
        .get_mut(&Text::from("q"))
        .unwrap()
        .formula = Bool::from_bool(true);
    assert!(engine.validate_solution(&model).unwrap());
}

#[test]
fn false_background_axioms_are_obligations_not_vacuous_assumptions() {
    let mut engine = engine();
    engine
        .register_relation(FuncDecl::new("p", &[], &Sort::bool()))
        .unwrap();
    engine.assert(&Bool::from_bool(false)).unwrap();
    assert!(
        !engine
            .validate_solution(&candidate("p", &[], Bool::from_bool(true)))
            .unwrap()
    );
}

#[test]
fn global_properties_cannot_be_smuggled_in_with_a_valid_model() {
    let engine = naturals();
    let x = Int::new_const("x");
    let mut model = candidate("reachable", &[Dynamic::from_ast(&x)], x.ge(0));
    assert!(
        extract_invariants(&model).is_empty(),
        "a predicate body is not a global invariant"
    );
    model.invariants.push(x.ge(0));
    assert!(!engine.validate_solution(&model).unwrap());
    model.invariants.clear();
    let reachable = FuncDecl::new("reachable", &[&Sort::int()], &Sort::bool());
    let conditional = reachable.apply(&[&x]).as_bool().unwrap().implies(x.ge(0));
    model
        .invariants
        .push(z3::ast::forall_const(&[&x], &[], &conditional));
    assert!(engine.validate_solution(&model).unwrap());
}

#[test]
fn circular_or_hidden_function_definitions_are_refused() {
    let mut engine = engine();
    let p = engine
        .register_relation(FuncDecl::new("p", &[&Sort::int()], &Sort::bool()))
        .unwrap();
    let x = Int::new_const("x");
    let args = [Dynamic::from_ast(&x)];
    let circular = p.apply(&[&x]).as_bool().unwrap().not();
    assert!(
        engine
            .validate_solution(&candidate("p", &args, circular))
            .unwrap_err()
            .contains("Unbound symbol")
    );
    let hidden = p.as_array().select(&x).as_bool().unwrap().not();
    assert!(
        engine
            .validate_solution(&candidate("p", &args, hidden))
            .unwrap_err()
            .contains("function-valued")
    );
    let free = Int::new_const("not_a_parameter");
    assert!(
        engine
            .validate_solution(&candidate("p", &args, x.eq(free)))
            .unwrap_err()
            .contains("Unbound symbol")
    );
    let quantified = z3::ast::forall_const(&[&x], &[], &x.eq(&x));
    assert!(
        engine
            .validate_solution(&candidate("p", &args, quantified))
            .unwrap_err()
            .contains("quantifier-free")
    );
}

#[test]
fn parameter_arity_sort_and_symbol_identity_are_checked_before_solving() {
    let engine = naturals();
    assert!(
        engine
            .validate_solution(&candidate("reachable", &[], Bool::from_bool(true)))
            .unwrap_err()
            .contains("argument count")
    );
    assert!(
        engine
            .validate_solution(&candidate(
                "reachable",
                &[Bool::new_const("b").into()],
                Bool::from_bool(true)
            ))
            .unwrap_err()
            .contains("wrong sort")
    );
    assert!(
        engine
            .validate_solution(&candidate(
                "reachable",
                &[Int::from_i64(0).into()],
                Bool::from_bool(true)
            ))
            .unwrap_err()
            .contains("symbolic constants")
    );
}

#[test]
fn a_raw_reachability_answer_is_not_fabricated_into_a_solution() {
    let mut engine = naturals();
    let result = engine
        .query_predicate(FixedPointQuery {
            predicate: "reachable".into(),
            args: [Int::from_i64(1).into()].into_iter().collect(),
        })
        .unwrap();
    assert_eq!(result.status, SatResult::Sat);
    assert!(result.answer.is_some());
    assert!(result.solution.is_none());
}

#[test]
fn resource_exhaustion_is_an_error_not_an_inductiveness_verdict() {
    let mut engine = engine();
    engine
        .register_relation(FuncDecl::new("p", &[], &Sort::bool()))
        .unwrap();
    let pigeons: List<Int> = (0..100).map(|_| Int::fresh_const("pigeon")).collect();
    let mut clauses: List<Bool> = pigeons.iter().flat_map(|x| [x.ge(0), x.lt(99)]).collect();
    clauses.push(Int::distinct(&pigeons));
    engine.assert(&Bool::and(&clauses).not()).unwrap();
    let config = ContextConfig {
        timeout: Some(Duration::from_millis(1)),
        ..Default::default()
    };
    let error = engine
        .validate_solution_with_config(&candidate("p", &[], Bool::from_bool(true)), config)
        .unwrap_err();
    assert!(
        error.starts_with("Inductive solution obligation unknown:"),
        "{error}"
    );
}
