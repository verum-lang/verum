//! Exact scalar coverage, shared by checking and witness construction.
use super::{Interval, IntervalSet, LiteralPattern, PatternColumn};

pub(super) const INT_BOUNDS: Interval = Interval {
    start: i64::MIN as i128,
    end: i64::MAX as i128,
};

fn integer_coverage(pattern: &PatternColumn) -> IntervalSet {
    match pattern {
        PatternColumn::Wildcard => IntervalSet::singleton(INT_BOUNDS),
        PatternColumn::Literal(LiteralPattern::Int(value)) => {
            IntervalSet::singleton(Interval::singleton(*value))
                .intersect(&IntervalSet::singleton(INT_BOUNDS))
        }
        PatternColumn::Range {
            start,
            end,
            inclusive,
        } => Interval::from_range(*start, *end, *inclusive, INT_BOUNDS)
            .map(IntervalSet::singleton)
            .unwrap_or_else(IntervalSet::empty),
        PatternColumn::Or(alternatives) => {
            let mut result = IntervalSet::empty();
            for alternative in alternatives {
                result.union_with(&integer_coverage(alternative));
            }
            result
        }
        PatternColumn::And(conjuncts) => conjuncts
            .iter()
            .fold(IntervalSet::singleton(INT_BOUNDS), |covered, pattern| {
                covered.intersect(&integer_coverage(pattern))
            }),
        // A condition contributes no unconditional coverage.
        _ => IntervalSet::empty(),
    }
}

pub(super) fn covered_integers<'a>(
    patterns: impl IntoIterator<Item = &'a PatternColumn>,
) -> IntervalSet {
    let mut covered = IntervalSet::empty();
    for pattern in patterns {
        covered.union_with(&integer_coverage(pattern));
    }
    covered
}

pub(super) fn uncovered_integer<'a>(
    patterns: impl IntoIterator<Item = &'a PatternColumn>,
) -> Option<i64> {
    covered_integers(patterns)
        .first_uncovered(INT_BOUNDS)
        .map(|value| i64::try_from(value).expect("an integer witness lies within the Int domain"))
}

pub(super) fn covers_bool(pattern: &PatternColumn, value: bool) -> bool {
    match pattern {
        PatternColumn::Wildcard => true,
        PatternColumn::Literal(LiteralPattern::Bool(literal)) => *literal == value,
        PatternColumn::Constructor { name, .. } => {
            name.as_str() == if value { "true" } else { "false" }
        }
        PatternColumn::Or(alternatives) => alternatives.iter().any(|p| covers_bool(p, value)),
        PatternColumn::And(conjuncts) => conjuncts.iter().all(|p| covers_bool(p, value)),
        _ => false,
    }
}

/// IEEE equality and bounded ordering never match NaN. For scalar patterns,
/// any unconditional combination that matches NaN also matches every Float.
pub(super) fn covers_nan(pattern: &PatternColumn) -> bool {
    match pattern {
        PatternColumn::Wildcard => true,
        PatternColumn::Range {
            start: None,
            end: None,
            ..
        } => true,
        PatternColumn::Or(alternatives) => alternatives.iter().any(covers_nan),
        PatternColumn::And(conjuncts) => conjuncts.iter().all(covers_nan),
        _ => false,
    }
}
