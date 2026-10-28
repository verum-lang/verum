//! Exact declaration and value preservation through the supported Z3 API.
use verum_common::{List, Text};
use verum_smt::fixedpoint::{Atom, DatalogRule, FixedPointEngine, FixedPointQuery};
use z3::{
    Context, FuncDecl, Params, SatResult, Sort, Symbol,
    ast::{BV, Bool, Dynamic, Int},
};

fn engine() -> FixedPointEngine {
    let mut engine = FixedPointEngine::new(Context::thread_local()).unwrap();
    let mut parameters = Params::new();
    parameters.set_symbol("engine", "spacer");
    parameters.set_u32("timeout", 5000);
    engine.set_params(&parameters);
    engine
}

fn query(engine: &mut FixedPointEngine, name: &str, value: Dynamic) -> SatResult {
    engine
        .query_predicate(FixedPointQuery {
            predicate: Text::from(name),
            args: [value].into_iter().collect(),
        })
        .unwrap()
        .status
}

#[test]
fn integer_facts_preserve_sign_and_values_above_u32() {
    let mut engine = engine();
    let relation = FuncDecl::new("exact_integer", &[&Sort::int()], &Sort::bool());
    let relation = engine.register_relation(relation).unwrap();
    engine
        .add_fact(&relation, &[Int::from_i64(-7).into()])
        .unwrap();
    engine
        .add_fact(&relation, &[Int::from_i64(1_i64 << 40).into()])
        .unwrap();
    assert_eq!(
        query(&mut engine, "exact_integer", Int::from_i64(-7).into()),
        SatResult::Sat
    );
    assert_eq!(
        query(
            &mut engine,
            "exact_integer",
            Int::from_i64(1_i64 << 40).into()
        ),
        SatResult::Sat
    );
    assert_eq!(
        query(&mut engine, "exact_integer", Int::from_i64(7).into()),
        SatResult::Unsat
    );
    assert_eq!(
        query(&mut engine, "exact_integer", Int::from_i64(0).into()),
        SatResult::Unsat
    );
}

#[test]
fn stored_relations_preserve_bitvector_widths() {
    for width in [8, 64, 128] {
        let mut engine = engine();
        let relation = FuncDecl::new("exact_word", &[&Sort::bitvector(width)], &Sort::bool());
        let relation = engine.register_relation(relation).unwrap();
        let value = if width == 8 { 42 } else { (1_u64 << 40) + 42 };
        engine
            .add_fact(&relation, &[BV::from_u64(value, width).into()])
            .unwrap();
        assert_eq!(
            query(&mut engine, "exact_word", BV::from_u64(value, width).into()),
            SatResult::Sat
        );
        assert_eq!(
            query(
                &mut engine,
                "exact_word",
                BV::from_u64(value + 1, width).into()
            ),
            SatResult::Unsat
        );
        if width > 32 {
            assert_eq!(
                query(&mut engine, "exact_word", BV::from_u64(42, width).into()),
                SatResult::Unsat
            );
        }
    }
}

#[test]
fn named_domains_survive_registration_and_rule_construction() {
    let mut engine = engine();
    let domain = Sort::uninterpreted(Symbol::String("OwnedObject".into()));
    let relation = FuncDecl::new("owns_object", &[&domain], &Sort::bool());
    engine.register_relation(relation).unwrap();
    let object = Dynamic::fresh_const("object", &domain);
    engine
        .add_datalog_rule(DatalogRule {
            head: Atom {
                predicate: "owns_object".into(),
                args: [object].into_iter().collect(),
            },
            body: List::new(),
            constraints: List::new(),
        })
        .unwrap();
    let state = engine.to_smtlib();
    assert!(
        state
            .as_str()
            .contains("(declare-rel owns_object (OwnedObject))"),
        "{state}"
    );
}

#[test]
fn recursive_reachability_uses_real_solver_results() {
    let mut engine = engine();
    let reachable = FuncDecl::new("reachable", &[&Sort::int()], &Sort::bool());
    let reachable = engine.register_relation(reachable).unwrap();
    engine
        .add_fact(&reachable, &[Int::from_i64(0).into()])
        .unwrap();
    let x = Int::fresh_const("x");
    let next = Int::add(&[&x, &Int::from_i64(1)]);
    let rule = reachable
        .apply(&[&x])
        .as_bool()
        .unwrap()
        .implies(reachable.apply(&[&next]).as_bool().unwrap());
    engine
        .add_rule(&z3::ast::forall_const(&[&x], &[], &rule), Some("successor"))
        .unwrap();
    assert_eq!(
        query(&mut engine, "reachable", Int::from_i64(5).into()),
        SatResult::Sat
    );
    assert_eq!(
        query(&mut engine, "reachable", Int::from_i64(-1).into()),
        SatResult::Unsat
    );
    assert_eq!(
        engine.query_relations(&[&reachable]).unwrap(),
        SatResult::Sat
    );
    assert!(!engine.get_help().is_empty());
}

#[test]
fn facts_refuse_symbolic_values_and_nonrelations() {
    let mut engine = engine();
    let relation = FuncDecl::new("ground", &[&Sort::int()], &Sort::bool());
    let relation = engine.register_relation(relation).unwrap();
    assert!(
        engine
            .add_fact(&relation, &[Int::fresh_const("x").into()])
            .is_err()
    );
    assert!(engine.add_fact(&relation, &[]).is_err());
    let function = FuncDecl::new("function", &[&Sort::int()], &Sort::int());
    assert!(engine.register_relation(function).is_err());
    assert_eq!(
        query(&mut engine, "ground", Int::from_i64(0).into()),
        SatResult::Unsat
    );
}

#[test]
fn a_ground_rule_cannot_drop_its_constraint() {
    let mut engine = engine();
    let relation = FuncDecl::new("guarded", &[&Sort::int()], &Sort::bool());
    engine.register_relation(relation).unwrap();
    engine
        .add_datalog_rule(DatalogRule {
            head: Atom {
                predicate: "guarded".into(),
                args: [Int::from_i64(1).into()].into_iter().collect(),
            },
            body: List::new(),
            constraints: [Bool::from_bool(false)].into_iter().collect(),
        })
        .unwrap();
    assert_eq!(
        query(&mut engine, "guarded", Int::from_i64(1).into()),
        SatResult::Unsat
    );
}

#[test]
fn horn_clause_variables_are_universally_bound() {
    let mut engine = engine();
    let relation = FuncDecl::new("nonnegative", &[&Sort::int()], &Sort::bool());
    engine.register_relation(relation).unwrap();
    let x = Int::new_const("bound_x");
    engine
        .add_chc(verum_smt::fixedpoint::CHC {
            vars: [(Text::from("bound_x"), Sort::int())].into_iter().collect(),
            hypothesis: List::new(),
            constraints: [x.ge(Int::from_i64(0))].into_iter().collect(),
            conclusion: Atom {
                predicate: "nonnegative".into(),
                args: [x.into()].into_iter().collect(),
            },
        })
        .unwrap();
    assert_eq!(
        query(&mut engine, "nonnegative", Int::from_i64(5).into()),
        SatResult::Sat
    );
    assert_eq!(
        query(&mut engine, "nonnegative", Int::from_i64(-1).into()),
        SatResult::Unsat
    );
}

#[test]
fn malformed_applications_are_errors_without_mutating_the_rules() {
    let mut engine = engine();
    let relation = engine
        .register_relation(FuncDecl::new(
            "word",
            &[&Sort::bitvector(64)],
            &Sort::bool(),
        ))
        .unwrap();
    assert!(
        engine
            .add_fact(&relation, &[BV::from_u64(1, 8).into()])
            .is_err()
    );
    assert!(
        engine
            .add_fact(&relation, &[Bool::from_bool(true).into()])
            .is_err()
    );
    assert!(
        engine
            .query_predicate(FixedPointQuery {
                predicate: "word".into(),
                args: [Int::from_i64(1).into()].into_iter().collect(),
            })
            .is_err()
    );
    assert!(
        engine
            .query_predicate(FixedPointQuery {
                predicate: "word".into(),
                args: List::new(),
            })
            .is_err()
    );
    assert!(
        engine
            .register_relation(FuncDecl::new("word", &[&Sort::int()], &Sort::bool(),))
            .is_err()
    );
    assert_eq!(engine.get_statistics().num_rules, 0);
    assert_eq!(
        query(&mut engine, "word", BV::from_u64(1, 64).into()),
        SatResult::Unsat
    );
}

#[test]
fn resource_exhaustion_is_inconclusive_and_keeps_its_reason() {
    let mut engine = engine();
    let relation = engine
        .register_relation(FuncDecl::new("limited", &[&Sort::int()], &Sort::bool()))
        .unwrap();
    engine
        .add_fact(&relation, &[Int::from_i64(1).into()])
        .unwrap();
    let mut params = Params::new();
    params.set_symbol("engine", "spacer");
    params.set_u32("rlimit", 1);
    engine.set_params(&params);
    let result = engine
        .query_predicate(FixedPointQuery {
            predicate: "limited".into(),
            args: [Int::from_i64(1).into()].into_iter().collect(),
        })
        .unwrap();
    assert_eq!(result.status, SatResult::Unknown);
    assert!(result.solution.is_none());
    assert!(!engine.get_reason_unknown().is_empty());
}

#[test]
fn zero_argument_relations_keep_their_identity() {
    let mut engine = engine();
    let relation = engine
        .register_relation(FuncDecl::new("flag", &[], &Sort::bool()))
        .unwrap();
    let atom = relation.apply(&[]).as_bool().unwrap();
    assert_eq!(engine.query(&atom).unwrap(), SatResult::Unsat);
    engine.add_fact(&relation, &[]).unwrap();
    assert_eq!(engine.query(&atom).unwrap(), SatResult::Sat);
}

#[test]
fn foreign_context_declarations_and_arguments_are_refused() {
    use std::cell::RefCell;
    thread_local! {
        static FOREIGN: RefCell<Option<(FuncDecl, FuncDecl, Dynamic)>> = const { RefCell::new(None) };
    }
    z3::with_z3_config(&z3::Config::new(), || {
        FOREIGN.with(|slot| {
            *slot.borrow_mut() = Some((
                FuncDecl::new("foreign", &[&Sort::int()], &Sort::bool()),
                FuncDecl::new("foreign_flag", &[], &Sort::bool()),
                Int::from_i64(1).into(),
            ));
        });
    });
    let (foreign, flag, argument) = FOREIGN.with(|slot| slot.borrow_mut().take().unwrap());
    let mut engine = engine();
    assert!(engine.query_relations(&[&foreign]).is_err());
    assert!(engine.register_relation(foreign).is_err());
    assert!(engine.register_relation(flag).is_err());
    let relation = engine
        .register_relation(FuncDecl::new("local", &[&Sort::int()], &Sort::bool()))
        .unwrap();
    assert!(engine.add_fact(&relation, &[argument]).is_err());
    assert_eq!(engine.get_statistics().num_rules, 0);
    assert_eq!(engine.get_statistics().num_predicates, 1);
}

#[test]
fn distinct_named_sorts_cannot_be_interchanged() {
    let mut engine = engine();
    let owner = Sort::uninterpreted(Symbol::String("Owner".into()));
    let other = Sort::uninterpreted(Symbol::String("Other".into()));
    engine
        .register_relation(FuncDecl::new("owns", &[&owner], &Sort::bool()))
        .unwrap();
    assert!(
        engine
            .add_datalog_rule(DatalogRule {
                head: Atom {
                    predicate: "owns".into(),
                    args: [Dynamic::fresh_const("other", &other)]
                        .into_iter()
                        .collect(),
                },
                body: List::new(),
                constraints: List::new(),
            })
            .is_err()
    );
    assert_eq!(engine.get_statistics().num_rules, 0);
}
