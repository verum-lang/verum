//! Strict controls for actual guarded transitions, not existential rank witnesses.
use verum_common::{List, Text};
use verum_smt::ranking::{RankingFunction, RankingTransition};
use verum_smt::{ContextConfig, RecursiveFunction, RecursiveProgramVerifier};
use z3::{
    Context,
    ast::{Bool, Dynamic, Int},
};

fn transition(guard: Bool, arguments: Vec<Dynamic>) -> RankingTransition {
    RankingTransition {
        guard,
        arguments: List::from(arguments),
    }
}
fn verify(
    parameters: Vec<Dynamic>,
    domain: Bool,
    rank: Int,
    transitions: Vec<RankingTransition>,
) -> bool {
    RankingFunction {
        expression: rank,
        well_founded_constraint: Bool::from_bool(true),
    }
    .verify(&parameters, &domain, &transitions, ContextConfig::default())
    .unwrap()
}

#[test]
fn factorial_and_fibonacci_use_actual_guarded_arguments() {
    let n = Int::new_const("n");
    for decrement in [1, 2] {
        assert!(verify(
            vec![n.clone().into()],
            n.ge(0),
            n.clone(),
            vec![transition(n.ge(decrement), vec![(&n - decrement).into()])]
        ));
    }
    let func = RecursiveFunction {
        name: Text::from("fib"),
        parameters: List::from(vec![n.clone().into()]),
        precondition: n.ge(0),
        recursive_calls: List::from(vec![
            transition(n.gt(1), vec![(&n - 1).into()]),
            transition(n.gt(1), vec![(&n - 2).into()]),
        ]),
    };
    assert!(
        RecursiveProgramVerifier::new(Context::thread_local())
            .unwrap()
            .verify_termination(
                func,
                RankingFunction {
                    expression: n.clone(),
                    well_founded_constraint: n.ge(0),
                }
            )
            .unwrap()
    );
}

#[test]
fn unchanged_increasing_and_bad_second_call_refute_candidate() {
    let n = Int::new_const("n");
    for next in [n.clone(), &n + 1] {
        assert!(!verify(
            vec![n.clone().into()],
            n.ge(0),
            n.clone(),
            vec![transition(n.gt(0), vec![next.into()])]
        ));
    }
    assert!(!verify(
        vec![n.clone().into()],
        n.ge(0),
        n.clone(),
        vec![
            transition(n.gt(0), vec![(&n - 1).into()]),
            transition(n.gt(0), vec![n.clone().into()]),
        ]
    ));
}

#[test]
fn domain_and_nonnegativity_are_proved_not_assumed() {
    let n = Int::new_const("n");
    assert!(!verify(
        vec![n.clone().into()],
        Bool::from_bool(true),
        n.clone(),
        vec![]
    ));
    assert!(!verify(
        vec![n.clone().into()],
        n.ge(0),
        n.clone(),
        vec![transition(Bool::from_bool(true), vec![(&n - 1).into()])]
    ));
    // Decrease remains positive, but the successor escapes the supplied invariant.
    assert!(!verify(
        vec![n.clone().into()],
        n.ge(10),
        n.clone(),
        vec![transition(n.ge(10), vec![(&n - 1).into()])]
    ));
    let rank = RankingFunction {
        expression: n.clone(),
        well_founded_constraint: n.gt(100),
    };
    assert!(
        !rank
            .verify(&[n.clone().into()], &n.ge(0), &[], ContextConfig::default())
            .unwrap()
    );
    let contradiction = RankingFunction {
        expression: n.clone(),
        well_founded_constraint: Bool::from_bool(false),
    };
    assert!(
        !contradiction
            .verify(&[n.clone().into()], &n.ge(0), &[], ContextConfig::default())
            .unwrap()
    );
}

#[test]
fn unreachable_calls_are_excluded_only_by_their_guards() {
    let n = Int::new_const("n");
    assert!(verify(
        vec![n.clone().into()],
        n.ge(0),
        n.clone(),
        vec![transition(n.lt(0), vec![(&n + 1).into()])]
    ));
}

#[test]
fn simultaneous_substitution_handles_permutation_and_compound_ranks() {
    let a = Int::new_const("a");
    let b = Int::new_const("b");
    let domain = Bool::and(&[&a.ge(0), &b.ge(0)]);
    // A swap preserves the sum. Sequential replacement would corrupt this check.
    assert!(!verify(
        vec![a.clone().into(), b.clone().into()],
        domain.clone(),
        &a + &b,
        vec![transition(
            a.gt(&b),
            vec![b.clone().into(), a.clone().into()]
        )]
    ));
    assert!(verify(
        vec![a.clone().into(), b.clone().into()],
        domain.clone(),
        &a + &b,
        vec![transition(a.gt(0), vec![b.clone().into(), (&a - 1).into()])]
    ));
    // Euclid: b is the rank, and actual arguments are (b, a mod b).
    assert!(verify(
        vec![a.clone().into(), b.clone().into()],
        domain,
        b.clone(),
        vec![transition(
            b.gt(0),
            vec![b.clone().into(), a.modulo(&b).into()]
        )]
    ));
}

#[test]
fn binary_search_and_split_recursion_decrease_actual_sizes() {
    let lo = Int::new_const("lo");
    let hi = Int::new_const("hi");
    let mid = (&lo + &hi).div(2);
    assert!(verify(
        vec![lo.clone().into(), hi.clone().into()],
        lo.le(&hi),
        &hi - &lo,
        vec![transition(lo.lt(&hi), vec![lo.clone().into(), mid.into()])]
    ));
    let n = Int::new_const("n");
    assert!(verify(
        vec![n.clone().into()],
        n.ge(0),
        n.clone(),
        vec![
            transition(n.gt(1), vec![n.div(2).into()]),
            transition(n.gt(1), vec![(&n - n.div(2)).into()]),
        ]
    ));
}

#[test]
fn collatz_rank_is_refuted_without_claiming_nontermination() {
    let n = Int::new_const("n");
    assert!(!verify(
        vec![n.clone().into()],
        n.gt(0),
        n.clone(),
        vec![
            transition(
                Bool::and(&[&n.gt(1), &n.modulo(2).eq(0)]),
                vec![n.div(2).into()]
            ),
            transition(
                Bool::and(&[&n.gt(1), &n.modulo(2).eq(1)]),
                vec![(&n * 3 + 1).into()]
            ),
        ]
    ));
}

#[test]
fn malformed_transitions_are_errors_even_on_an_unreachable_domain() {
    let n = Int::new_const("n");
    let b = Bool::new_const("b");
    let rank = RankingFunction {
        expression: n.clone(),
        well_founded_constraint: n.ge(0),
    };
    for parameters in [
        vec![n.clone().into(), n.clone().into()],
        vec![(&n + 1).into()],
        vec![Int::from_i64(0).into()],
    ] {
        assert!(
            rank.verify(
                &parameters,
                &Bool::from_bool(false),
                &[],
                ContextConfig::default()
            )
            .is_err()
        );
    }
    for args in [
        vec![],
        vec![b.clone().into()],
        vec![n.clone().into(), n.clone().into()],
    ] {
        assert!(
            rank.verify(
                &[n.clone().into()],
                &Bool::from_bool(false),
                &[transition(Bool::from_bool(false), args)],
                ContextConfig::default()
            )
            .is_err()
        );
    }
}

#[test]
fn foreign_context_is_rejected_before_any_ast_operation() {
    use std::cell::RefCell;
    thread_local! { static FOREIGN: RefCell<Option<Dynamic>> = const { RefCell::new(None) }; }
    z3::with_z3_config(&z3::Config::new(), || {
        FOREIGN.with(|slot| *slot.borrow_mut() = Some(Int::from_i64(2).into()));
    });
    let n = Int::new_const("n");
    let rank = RankingFunction {
        expression: n.clone(),
        well_founded_constraint: n.ge(0),
    };
    let foreign = FOREIGN.with(|slot| slot.borrow_mut().take().unwrap());
    assert!(
        rank.verify(
            &[n.clone().into()],
            &n.ge(0),
            &[transition(n.gt(0), vec![foreign])],
            ContextConfig::default()
        )
        .is_err()
    );
}
