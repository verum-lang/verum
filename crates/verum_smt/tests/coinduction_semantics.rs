//! Coinduction requires seed membership and closure, not a type or depth limit.
use verum_ast::{
    ContextList, Expr, ExprKind, IntLit, Literal, LiteralKind, Type, TypeKind,
    span::Span,
    ty::{Ident, Path},
};
use verum_common::{Heap, List};
use verum_smt::{
    coinductive::{
        BisimulationChecker, BisimulationConfig, CoinductiveChecker, CoinductiveType, Destructor,
        stream_type,
    },
    verify::VerificationError,
};

fn ty(name: &str) -> Type {
    Type::new(
        TypeKind::Path(Path::from_ident(Ident::new(name, Span::dummy()))),
        Span::dummy(),
    )
}
fn literal(n: i128) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit::new(n)),
            Span::dummy(),
        )),
        Span::dummy(),
    )
}
fn negative_recursion() -> CoinductiveType {
    CoinductiveType {
        name: "Bad".into(),
        params: List::new(),
        destructors: [Destructor {
            name: "observe".into(),
            return_type: Heap::new(Type::new(
                TypeKind::Function {
                    params: [ty("Bad")].into_iter().collect(),
                    return_type: Box::new(ty("Bool")),
                    contexts: ContextList::empty(),
                    calling_convention: None,
                },
                Span::dummy(),
            )),
        }]
        .into_iter()
        .collect(),
    }
}

#[test]
fn a_type_schema_cannot_certify_greatest_fixed_point_or_productivity() {
    assert!(matches!(
        CoinductiveChecker::new().verify_greatest_fixpoint(&negative_recursion()),
        Err(VerificationError::Unknown(_))
    ));
}

#[test]
fn reaching_an_unfolding_limit_is_inconclusive() {
    let checker = CoinductiveChecker::new();
    let mut bounded = BisimulationChecker::new(
        &checker,
        BisimulationConfig {
            max_depth: 0,
            ..Default::default()
        },
    );
    assert!(matches!(
        bounded.check_bisimulation(&literal(1), &literal(2), &stream_type(ty("Int"))),
        Err(VerificationError::Unknown(_))
    ));
}

#[test]
fn recursive_observations_require_a_closed_candidate_relation() {
    assert!(matches!(
        CoinductiveChecker::new().verify_bisimulation(
            &literal(1),
            &literal(2),
            &negative_recursion()
        ),
        Err(VerificationError::Unknown(_))
    ));
}

use verum_smt::{
    CoinductionModel, CoinductionObligation, CoinductionResult, context::ContextConfig,
};
use z3::ast::{Bool, Dynamic, Int};

fn parity_model() -> CoinductionModel {
    let x = Int::new_const("left_state");
    let y = Int::new_const("right_state");
    let left = x.modulo(Int::from_i64(2));
    let right = y.modulo(Int::from_i64(2));
    CoinductionModel {
        variables: [x.clone().into(), y.clone().into()],
        initial: [Int::from_i64(0).into(), Int::from_i64(2).into()],
        relation: left.eq(&right),
        observations: [(left.into(), right.into())].into_iter().collect(),
        successors: [(
            Int::add(&[&x, &Int::from_i64(1)]).into(),
            Int::add(&[&y, &Int::from_i64(1)]).into(),
        )]
        .into_iter()
        .collect(),
    }
}

#[test]
fn an_infinite_observation_sequence_has_a_closed_candidate() {
    assert!(matches!(
        parity_model().verify(ContextConfig::fast()).unwrap(),
        CoinductionResult::Verified
    ));
}

#[test]
fn initial_membership_cannot_be_assumed() {
    let mut model = parity_model();
    model.initial[1] = Int::from_i64(1).into();
    assert!(matches!(
        model.verify(ContextConfig::fast()).unwrap(),
        CoinductionResult::CandidateRefuted {
            obligation: CoinductionObligation::InitialPair,
            ..
        }
    ));
}

#[test]
fn every_observation_must_agree_for_all_related_states() {
    let mut model = parity_model();
    model
        .observations
        .push((model.variables[0].clone(), model.variables[1].clone()));
    assert!(matches!(
        model.verify(ContextConfig::fast()).unwrap(),
        CoinductionResult::CandidateRefuted {
            obligation: CoinductionObligation::Observation(1),
            ..
        }
    ));
}

#[test]
fn every_successor_pair_must_preserve_the_relation() {
    let mut model = parity_model();
    model.successors.push((
        Int::add(&[&model.variables[0].as_int().unwrap(), &Int::from_i64(1)]).into(),
        model.variables[1].clone(),
    ));
    assert!(matches!(
        model.verify(ContextConfig::fast()).unwrap(),
        CoinductionResult::CandidateRefuted {
            obligation: CoinductionObligation::Successor(1),
            ..
        }
    ));
}

#[test]
fn checking_only_the_initial_step_cannot_prove_infinite_closure() {
    let mut model = parity_model();
    let x = model.variables[0].as_int().unwrap();
    let y = model.variables[1].as_int().unwrap();
    model.relation = Bool::and(&[&x.eq(Int::from_i64(0)), &y.eq(Int::from_i64(2))]);
    assert!(matches!(
        model.verify(ContextConfig::fast()).unwrap(),
        CoinductionResult::CandidateRefuted {
            obligation: CoinductionObligation::Successor(0),
            ..
        }
    ));
}

#[test]
fn different_state_representations_can_have_identical_observations() {
    let x = Int::new_const("integer_state");
    let y = Bool::new_const("boolean_state");
    let even = x.modulo(Int::from_i64(2)).eq(Int::from_i64(0));
    let model = CoinductionModel {
        variables: [x.clone().into(), y.clone().into()],
        initial: [Int::from_i64(0).into(), Bool::from_bool(true).into()],
        relation: even.eq(&y),
        observations: [(even.into(), y.clone().into())].into_iter().collect(),
        successors: [(Int::add(&[&x, &Int::from_i64(1)]).into(), y.not().into())]
            .into_iter()
            .collect(),
    };
    assert!(matches!(
        model.verify(ContextConfig::fast()).unwrap(),
        CoinductionResult::Verified
    ));
}

#[test]
fn a_bad_candidate_does_not_prove_inequivalence() {
    let mut model = parity_model();
    model.initial[1] = model.initial[0].clone();
    model.relation = Bool::from_bool(false);
    assert!(matches!(
        model.verify(ContextConfig::fast()).unwrap(),
        CoinductionResult::CandidateRefuted {
            obligation: CoinductionObligation::InitialPair,
            ..
        }
    ));
}

#[test]
fn model_validation_rejects_sort_changes_and_aliasing() {
    let valid = parity_model();
    let mut model = valid.clone();
    model.initial[1] = Bool::from_bool(true).into();
    assert!(model.verify(ContextConfig::fast()).is_err());
    let mut model = valid.clone();
    model.variables[1] = model.variables[0].clone();
    assert!(model.verify(ContextConfig::fast()).is_err());
    let mut model = valid.clone();
    model.successors[0].0 = Bool::from_bool(true).into();
    assert!(model.verify(ContextConfig::fast()).is_err());
    let mut model = valid;
    model.observations[0].0 = Bool::from_bool(true).into();
    assert!(model.verify(ContextConfig::fast()).is_err());
}

#[test]
fn foreign_contexts_are_rejected_before_native_operations() {
    use std::cell::RefCell;
    thread_local! { static FOREIGN: RefCell<Option<Dynamic>> = const { RefCell::new(None) }; }
    z3::with_z3_config(&z3::Config::new(), || {
        FOREIGN.with(|slot| *slot.borrow_mut() = Some(Int::from_i64(2).into()))
    });
    let mut model = parity_model();
    model.initial[1] = FOREIGN.with(|slot| slot.borrow_mut().take().unwrap());
    assert!(model.verify(ContextConfig::fast()).is_err());
}

#[test]
fn no_expression_strategy_can_invent_an_infinite_proof() {
    use verum_smt::coinductive::InfiniteStrategy;
    let checker = CoinductiveChecker::new();
    for strategy in [
        InfiniteStrategy::BoundedUnfolding,
        InfiniteStrategy::CoinductiveHypothesis,
        InfiniteStrategy::GreatestFixpoint,
        InfiniteStrategy::Hybrid,
    ] {
        let mut verifier = BisimulationChecker::new(
            &checker,
            BisimulationConfig {
                max_depth: 2,
                infinite_strategy: strategy,
                ..Default::default()
            },
        );
        assert!(
            matches!(
                verifier.check_bisimulation(&literal(1), &literal(2), &negative_recursion()),
                Err(VerificationError::Unknown(_))
            ),
            "{strategy:?}"
        );
    }
}

#[test]
fn a_model_without_observations_or_successors_has_a_universal_fixed_point() {
    let mut model = parity_model();
    model.observations.clear();
    model.successors.clear();
    model.relation = Bool::from_bool(true);
    assert!(matches!(
        model.verify(ContextConfig::fast()).unwrap(),
        CoinductionResult::Verified
    ));
}
