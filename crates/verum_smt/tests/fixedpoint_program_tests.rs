//! Safety is universal unreachability of violations in the supplied recursive model.
use verum_common::{List, Text};
use verum_smt::{
    Atom, CHC, FixedPointVerificationResult as Verdict, RecursiveProgram, RecursiveProgramVerifier,
    VerificationCondition,
};
use z3::{
    Context, FuncDecl, Params, Sort,
    ast::{Bool, Dynamic, Int},
};

fn atom(name: &str, args: &[Int]) -> Atom {
    Atom {
        predicate: name.into(),
        args: args.iter().cloned().map(Dynamic::from).collect(),
    }
}

fn variables(names: &[&str]) -> List<(Text, Sort)> {
    names
        .iter()
        .map(|n| (Text::from(*n), Sort::int()))
        .collect()
}

fn doubling(base: i64, increment: i64) -> RecursiveProgram {
    let n = Int::new_const("n");
    let r = Int::new_const("r");
    let signature = [&Sort::int(), &Sort::int()];
    RecursiveProgram {
        relations: List::from(vec![FuncDecl::new("double", &signature, &Sort::bool())]),
        rules: List::from(vec![
            CHC {
                vars: List::new(),
                hypothesis: List::new(),
                constraints: List::new(),
                conclusion: atom("double", &[Int::from_i64(0), Int::from_i64(base)]),
            },
            CHC {
                vars: variables(&["n", "r"]),
                hypothesis: List::from(vec![atom("double", &[n.clone(), r.clone()])]),
                constraints: List::from(vec![n.ge(0)]),
                conclusion: atom("double", &[&n + 1, &r + increment]),
            },
        ]),
        verification_conditions: List::from(vec![
            VerificationCondition {
                vars: variables(&["n", "r"]),
                hypothesis: List::from(vec![atom("double", &[n.clone(), r.clone()])]),
                constraints: List::new(),
                assertion: Bool::and(&[n.ge(0), r.ge(0)]),
            },
            VerificationCondition {
                vars: variables(&["n", "r"]),
                hypothesis: List::from(vec![atom("double", &[n.clone(), r.clone()])]),
                constraints: List::new(),
                assertion: r.eq(&n * 2),
            },
        ]),
    }
}

fn verifier() -> RecursiveProgramVerifier {
    RecursiveProgramVerifier::new(Context::thread_local()).unwrap()
}

#[test]
fn checks_actual_base_step_outputs_and_every_safety_condition() {
    let verifier = verifier();
    assert!(matches!(
        verifier.verify_recursive_program(doubling(0, 2)).unwrap(),
        Verdict::Verified
    ));
    for (base, step, expected) in [(1, 2, 1), (0, 3, 1), (-1, 2, 0), (0, -2, 0)] {
        assert!(matches!(
            verifier.verify_recursive_program(doubling(base, step)).unwrap(),
            Verdict::Refuted {condition, ..} if condition == expected
        ));
    }
    let mut altered = doubling(0, 2);
    altered.verification_conditions[1].assertion = Int::new_const("r").eq(Int::new_const("n"));
    assert!(matches!(
        verifier.verify_recursive_program(altered).unwrap(),
        Verdict::Refuted { condition: 1, .. }
    ));
}

#[test]
fn guarded_unreachable_paths_are_not_counterexamples() {
    let mut program = doubling(0, 2);
    let condition = &mut program.verification_conditions[1];
    condition.constraints.push(Int::new_const("n").lt(0));
    condition.assertion = Bool::from_bool(false);
    assert!(matches!(
        verifier().verify_recursive_program(program).unwrap(),
        Verdict::Verified
    ));
}

#[test]
fn mutual_recursion_uses_all_relations_and_explicit_variables() {
    let n = Int::new_const("n");
    let mut model = RecursiveProgram {
        relations: ["even", "odd"]
            .into_iter()
            .map(|name| FuncDecl::new(name, &[&Sort::int()], &Sort::bool()))
            .collect(),
        rules: List::from(vec![CHC {
            vars: List::new(),
            hypothesis: List::new(),
            constraints: List::new(),
            conclusion: atom("even", &[Int::from_i64(0)]),
        }]),
        verification_conditions: List::new(),
    };
    for (before, after, parity) in [("even", "odd", 0), ("odd", "even", 1)] {
        model.rules.push(CHC {
            vars: variables(&["n"]),
            hypothesis: List::from(vec![atom(before, &[n.clone()])]),
            constraints: List::new(),
            conclusion: atom(after, &[&n + 1]),
        });
        model.verification_conditions.push(VerificationCondition {
            vars: variables(&["n"]),
            hypothesis: List::from(vec![atom(before, &[n.clone()])]),
            constraints: List::new(),
            assertion: n.modulo(2).eq(parity),
        });
    }
    assert!(matches!(
        verifier().verify_recursive_program(model).unwrap(),
        Verdict::Verified
    ));
}

#[test]
fn missing_definitions_conditions_and_unbound_symbols_are_errors() {
    for change in [
        "rules",
        "conditions",
        "symbol",
        "arity",
        "unknown",
        "duplicate",
        "sort",
    ] {
        let mut program = doubling(0, 2);
        match change {
            "rules" => program.rules.clear(),
            "conditions" => program.verification_conditions.clear(),
            "symbol" => {
                program.verification_conditions[1].assertion = Int::new_const("hidden").ge(0)
            }
            "arity" => program.rules[1].conclusion.args.pop().map(|_| ()).unwrap(),
            "unknown" => {
                program.verification_conditions[1].hypothesis[0].predicate = "missing".into()
            }
            "duplicate" => program.relations.push(FuncDecl::new(
                "double",
                &[&Sort::int(), &Sort::int()],
                &Sort::bool(),
            )),
            "sort" => program.rules[1].conclusion.args[0] = Bool::from_bool(true).into(),
            _ => unreachable!(),
        }
        assert!(
            verifier().verify_recursive_program(program).is_err(),
            "{change}"
        );
    }
}

#[test]
fn later_malformed_conditions_are_not_hidden_by_an_early_refutation() {
    let mut program = doubling(-1, 2);
    program.verification_conditions[1].assertion = Int::new_const("unbound").gt(0);
    assert!(
        verifier()
            .verify_recursive_program(program)
            .unwrap_err()
            .contains("Unbound Horn variable")
    );
}

#[test]
fn calls_and_failed_registrations_are_isolated() {
    let verifier = verifier();
    for _ in 0..2 {
        let mut bad = doubling(0, 2);
        bad.verification_conditions[1].vars.clear();
        assert!(verifier.verify_recursive_program(bad).is_err());
        assert!(matches!(
            verifier.verify_recursive_program(doubling(0, 3)).unwrap(),
            Verdict::Refuted { .. }
        ));
        assert!(matches!(
            verifier.verify_recursive_program(doubling(0, 2)).unwrap(),
            Verdict::Verified
        ));
    }
}

#[test]
fn solver_exhaustion_is_an_error_with_a_reason() {
    let mut params = Params::new();
    params.set_symbol("engine", "spacer");
    params.set_u32("spacer.max_level", 0);
    let error = verifier()
        .verify_recursive_program_with_params(doubling(0, 2), &params)
        .unwrap_err();
    assert!(
        error.starts_with("Recursive safety condition 0 unknown: "),
        "{error}"
    );
    assert!(error.len() > "Recursive safety condition 0 unknown: ".len());
}

#[test]
fn theory_only_conditions_are_universally_proved() {
    for correct in [true, false] {
        let n = Int::new_const("n");
        let condition = VerificationCondition {
            vars: variables(&["n"]),
            hypothesis: List::new(),
            constraints: List::from(vec![n.ge(0)]),
            assertion: if correct { (&n + 1).gt(0) } else { n.gt(0) },
        };
        let result = verifier()
            .verify_recursive_program(RecursiveProgram {
                relations: List::new(),
                rules: List::new(),
                verification_conditions: List::from(vec![condition]),
            })
            .unwrap();
        assert_eq!(matches!(result, Verdict::Verified), correct);
    }
}

#[test]
fn foreign_assertions_are_refused_before_negation_or_lowering() {
    use std::cell::RefCell;
    thread_local! { static FOREIGN: RefCell<Option<Bool>> = const { RefCell::new(None) }; }
    z3::with_z3_config(&z3::Config::new(), || {
        FOREIGN.with(|slot| *slot.borrow_mut() = Some(Bool::from_bool(true)));
    });
    let mut program = doubling(0, 2);
    program.verification_conditions[1].assertion =
        FOREIGN.with(|slot| slot.borrow_mut().take().unwrap());
    assert!(
        verifier()
            .verify_recursive_program(program)
            .unwrap_err()
            .contains("another Z3 context")
    );
}
