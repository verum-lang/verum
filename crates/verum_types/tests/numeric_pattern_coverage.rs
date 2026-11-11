//! Coverage and witnesses must agree with actual scalar pattern semantics.
use verum_ast::{Literal, Pattern, PatternKind, Span};
use verum_common::{Heap, List};
use verum_types::exhaustiveness::{
    Interval, IntervalSet, Witness, WitnessLiteral, check_exhaustiveness, ranges_cover_type,
};
use verum_types::{Type, TypeEnv};

fn integer(value: i128) -> Pattern {
    Pattern::literal(Literal::int(value, Span::dummy()))
}

fn range(start: Option<i128>, end: Option<i128>, inclusive: bool) -> Pattern {
    Pattern::new(
        PatternKind::Range {
            start: start.map(|n| Heap::new(Literal::int(n, Span::dummy()))),
            end: end.map(|n| Heap::new(Literal::int(n, Span::dummy()))),
            inclusive,
        },
        Span::dummy(),
    )
}

fn conjunction(patterns: impl IntoIterator<Item = Pattern>) -> Pattern {
    Pattern::new(PatternKind::And(List::from_iter(patterns)), Span::dummy())
}

fn integer_witness(patterns: &[Pattern]) -> Option<i64> {
    let result = check_exhaustiveness(patterns, &Type::Int, &TypeEnv::default()).unwrap();
    if result.is_exhaustive {
        assert!(result.uncovered_witnesses.is_empty());
        None
    } else {
        match result.uncovered_witnesses.first().unwrap() {
            Witness::Literal(WitnessLiteral::Int(value)) => Some(*value),
            other => panic!("expected a concrete integer, got {other:?}"),
        }
    }
}

#[test]
fn full_width_integer_partitions_are_exhaustive() {
    assert_eq!(
        integer_witness(&[range(None, Some(0), false), range(Some(0), None, true)]),
        None
    );
    assert_eq!(
        integer_witness(&[range(Some(i64::MIN as i128), Some(i64::MAX as i128), true)]),
        None
    );
}

#[test]
fn witness_is_outside_all_ranges_even_beyond_the_old_sample_window() {
    let patterns = [range(None, Some(1_000_000), true)];
    let witness = integer_witness(&patterns).expect("there are uncovered integers");
    assert!(witness > 1_000_000, "covered witness: {witness}");
    let only_last_missing = [range(None, Some(i64::MAX as i128), false)];
    assert_eq!(integer_witness(&only_last_missing), Some(i64::MAX));
}

#[test]
fn conjunction_intersects_instead_of_erasing_restrictions() {
    assert!(
        integer_witness(&[conjunction([Pattern::wildcard(Span::dummy()), integer(0)])])
            .is_some_and(|n| n != 0)
    );
    let disjoint = conjunction([range(None, Some(0), false), range(Some(0), None, true)]);
    assert!(integer_witness(&[disjoint]).is_some());
    let boolean = conjunction([
        Pattern::literal(Literal::bool(true, Span::dummy())),
        Pattern::literal(Literal::bool(false, Span::dummy())),
    ]);
    let result = check_exhaustiveness(&[boolean], &Type::Bool, &TypeEnv::default()).unwrap();
    assert!(!result.is_exhaustive);
    assert_eq!(result.uncovered_witnesses.len(), 2);
}

#[test]
fn exclusive_minimum_and_interval_extremes_do_not_overflow() {
    assert!(integer_witness(&[range(None, Some(i128::MIN), false)]).is_some());
    let bounds = Interval::new(i128::MIN, i128::MAX);
    let (covered, witness) = ranges_cover_type(&[(None, Some(i128::MIN), false)], bounds);
    assert!(!covered);
    assert_eq!(witness, Some(i128::MIN));
    let mut set = IntervalSet::singleton(Interval::singleton(i128::MAX));
    set.add(Interval::singleton(i128::MIN));
    assert_eq!(set.first_uncovered(bounds), Some(i128::MIN + 1));
}

#[test]
fn float_literal_witness_is_not_the_covered_literal() {
    let patterns = [Pattern::literal(Literal::float(0.0, Span::dummy()))];
    let result = check_exhaustiveness(&patterns, &Type::Float, &TypeEnv::default()).unwrap();
    assert!(!result.is_exhaustive);
    match result.uncovered_witnesses.first().unwrap() {
        Witness::Literal(WitnessLiteral::Float(value)) => assert!(*value != 0.0),
        other => panic!("expected a concrete float, got {other:?}"),
    }
}

#[test]
fn wide_literal_identity_is_preserved_instead_of_wrapping_to_int() {
    assert_eq!(
        integer_witness(&[
            range(Some(i64::MIN as i128 + 1), None, true),
            integer(i64::MAX as i128 + 1),
        ]),
        Some(i64::MIN),
    );
}

#[test]
fn boolean_combinations_agree_with_enumeration_of_a_closed_domain() {
    // Cover everything outside this domain so enumeration is a complete
    // independent oracle, not sampling of an unbounded input space.
    for a in -3..=3 {
        for b in a..=3 {
            for c in -3..=3 {
                for d in c..=3 {
                    for intersect in [false, true] {
                        let parts = List::from_iter([
                            range(Some(a), Some(b), true),
                            range(Some(c), Some(d), true),
                        ]);
                        let kind = if intersect {
                            PatternKind::And(parts)
                        } else {
                            PatternKind::Or(parts)
                        };
                        let patterns = [
                            range(None, Some(-3), false),
                            range(Some(4), None, true),
                            Pattern::new(kind, Span::dummy()),
                        ];
                        let matches = |x| {
                            let left = a <= x && x <= b;
                            let right = c <= x && x <= d;
                            if intersect {
                                left && right
                            } else {
                                left || right
                            }
                        };
                        let missing = (-3..=3).find(|&x| !matches(x));
                        assert_eq!(
                            integer_witness(&patterns).map(i128::from),
                            missing,
                            "[{a}, {b}] [{c}, {d}] intersection={intersect}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn unsupported_range_bounds_are_not_reinterpreted_as_unbounded() {
    let pattern = Pattern::new(
        PatternKind::Range {
            start: Some(Heap::new(Literal::float(0.0, Span::dummy()))),
            end: Some(Heap::new(Literal::float(1.0, Span::dummy()))),
            inclusive: true,
        },
        Span::dummy(),
    );
    assert!(check_exhaustiveness(&[pattern], &Type::Float, &TypeEnv::default()).is_err());
}
