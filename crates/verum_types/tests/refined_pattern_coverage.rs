//! Refinement coverage is about the declared subject and every admissible value.
use verum_ast::{Literal, Pattern, PatternKind, Span};
use verum_common::{Heap, Text};
use verum_fast_parser::Parser;
use verum_types::exhaustiveness::{
    RefinementConstraint, Witness, WitnessLiteral, check_exhaustiveness, extract_refinement,
};
use verum_types::{Type, TypeEnv, refinement::RefinementPredicate};

fn refined(predicate: &str) -> Type {
    Type::refined(
        Type::Int,
        RefinementPredicate::new(
            Parser::new(predicate).parse_expr().unwrap(),
            Text::from("x"),
            Span::dummy(),
        ),
    )
}

fn literal(value: i128) -> Pattern {
    Pattern::literal(Literal::int(value, Span::dummy()))
}

fn range(start: i128, end: i128) -> Pattern {
    Pattern::new(
        PatternKind::Range {
            start: Some(Heap::new(Literal::int(start, Span::dummy()))),
            end: Some(Heap::new(Literal::int(end, Span::dummy()))),
            inclusive: true,
        },
        Span::dummy(),
    )
}

#[test]
fn a_refinement_does_not_change_the_subject_of_a_comparison() {
    for predicate in ["0 < 10", "other < 10", "x + 1 < 10"] {
        assert!(
            extract_refinement(&refined(predicate)).is_none(),
            "{predicate}"
        );
    }
    let constraint = extract_refinement(&refined("10 > x")).expect("reversed comparison");
    assert!(constraint.satisfies(9));
    assert!(!constraint.satisfies(10));
}

#[test]
fn exact_refined_ranges_can_establish_coverage() {
    let result = check_exhaustiveness(
        &[range(0, 100)],
        &refined("x >= 0 && x <= 100"),
        &TypeEnv::default(),
    )
    .unwrap();
    assert!(result.is_exhaustive, "{result:?}");
}

#[test]
fn sampled_points_and_a_false_guard_cannot_prove_coverage() {
    let guard = Pattern::new(
        PatternKind::Guard {
            pattern: Heap::new(Pattern::wildcard(Span::dummy())),
            guard: Heap::new(Parser::new("false").parse_expr().unwrap()),
        },
        Span::dummy(),
    );
    let result = check_exhaustiveness(
        &[guard, literal(0), literal(50), literal(100)],
        &refined("x >= 0 && x <= 100"),
        &TypeEnv::default(),
    )
    .unwrap();
    assert!(!result.is_exhaustive, "{result:?}");
    assert!(result.has_unverified_guards);
}

#[test]
fn refined_witness_is_admissible_and_not_already_covered() {
    let result = check_exhaustiveness(
        &[literal(0), literal(50), literal(100)],
        &refined("x >= 0 && x <= 100"),
        &TypeEnv::default(),
    )
    .unwrap();
    assert!(!result.is_exhaustive);
    for witness in &result.uncovered_witnesses {
        match witness {
            Witness::Literal(WitnessLiteral::Int(value)) => {
                assert!((0..=100).contains(value));
                assert!(![0, 50, 100].contains(value));
            }
            other => panic!("not an admissible concrete witness: {other:?}"),
        }
    }
    assert!(!result.uncovered_witnesses.is_empty());
}

#[test]
fn extreme_strict_constraints_are_empty_without_overflow() {
    assert!(
        RefinementConstraint::GreaterThan(i128::MAX)
            .to_interval_set()
            .is_empty()
    );
    assert!(
        RefinementConstraint::LessThan(i128::MIN)
            .to_interval_set()
            .is_empty()
    );
}

#[test]
fn unknown_predicates_cannot_authorize_a_success_or_a_counterexample() {
    let result =
        check_exhaustiveness(&[literal(0)], &refined("x % 2 == 0"), &TypeEnv::default()).unwrap();
    assert!(!result.is_exhaustive);
    assert!(result.uncovered_witnesses.is_empty(), "{result:?}");
    let all = check_exhaustiveness(
        &[Pattern::wildcard(Span::dummy())],
        &refined("x % 2 == 0"),
        &TypeEnv::default(),
    )
    .unwrap();
    assert!(all.is_exhaustive);
}

#[test]
fn nested_and_empty_refinements_preserve_their_actual_domain() {
    let inner = refined("x >= 10");
    let nested = Type::refined(
        inner,
        RefinementPredicate::new(
            Parser::new("y <= 20").parse_expr().unwrap(),
            Text::from("y"),
            Span::dummy(),
        ),
    );
    assert!(
        check_exhaustiveness(&[range(10, 20)], &nested, &TypeEnv::default())
            .unwrap()
            .is_exhaustive
    );
    assert!(
        check_exhaustiveness(&[], &refined("false"), &TypeEnv::default())
            .unwrap()
            .is_exhaustive
    );
}
