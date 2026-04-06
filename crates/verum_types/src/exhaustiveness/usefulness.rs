//! Usefulness Algorithm
//!
//! This module implements the usefulness check from Maranget's algorithm.
//! A pattern is "useful" if it covers at least one case not covered by previous patterns.
//!
//! The algorithm works by recursively analyzing the coverage matrix:
//! 1. If the matrix is empty, the pattern is useful (covers new ground)
//! 2. If the pattern starts with a wildcard, check each possible constructor
//! 3. If the pattern starts with a constructor, specialize and recurse

use super::constructors::{get_type_constructors, Constructor};
use super::matrix::{specialize_matrix, CoverageMatrix, PatternColumn, PatternRow};
use crate::context::TypeEnv;
use crate::ty::Type;
use std::collections::HashSet;
use verum_common::{List, Text};

/// Check if a pattern row is useful (adds coverage not provided by earlier patterns)
///
/// # Arguments
///
/// * `earlier_rows` - Rows from patterns that come before
/// * `row` - The row to check for usefulness
///
/// # Returns
///
/// `true` if the row covers at least one case not covered by earlier rows
pub fn is_useful(earlier_rows: &[PatternRow], row: &PatternRow) -> bool {
    // Empty matrix - this pattern is useful
    if earlier_rows.is_empty() {
        return true;
    }

    // Empty row - reached base case, pattern is useful if no earlier rows
    if row.columns.is_empty() {
        return earlier_rows.is_empty();
    }

    // Get the first column of the test row
    // INVARIANT: row.columns.is_empty() returned false above, so first() always succeeds
    let first = row.columns.first().expect("columns verified non-empty");

    match first {
        PatternColumn::Wildcard => {
            // Wildcard matches all constructors
            // Pattern is useful if it's useful for ANY constructor
            is_useful_wildcard(earlier_rows, row)
        }

        PatternColumn::Constructor { name, args } => {
            // Constructor pattern - specialize and recurse
            is_useful_constructor(earlier_rows, row, name, args)
        }

        PatternColumn::Or(alternatives) => {
            // Or pattern is useful if ANY alternative is useful
            alternatives.iter().any(|alt| {
                let modified_row = with_first_column(row, alt.clone());
                is_useful(earlier_rows, &modified_row)
            })
        }

        PatternColumn::And(conjuncts) => {
            // And pattern is useful only if ALL conjuncts are useful
            // This is conservative - we check if any individual conjunct is useful
            conjuncts.iter().any(|conj| {
                let modified_row = with_first_column(row, conj.clone());
                is_useful(earlier_rows, &modified_row)
            })
        }

        PatternColumn::Guarded(inner) => {
            // Guarded patterns are always considered potentially useful
            // because the guard might fail where previous guards succeeded
            // But we still check if the inner pattern is useful
            let modified_row = with_first_column(row, inner.as_ref().clone());
            is_useful(earlier_rows, &modified_row)
        }

        PatternColumn::Literal(lit) => {
            // Literal pattern - check if this specific value is covered
            is_useful_literal(earlier_rows, row, lit)
        }

        PatternColumn::Range { start, end, inclusive } => {
            // Range pattern - check if any part of the range is uncovered
            is_useful_range(earlier_rows, row, *start, *end, *inclusive)
        }

        PatternColumn::Tuple(elements) | PatternColumn::Array(elements) => {
            // Expand tuple/array and recurse
            is_useful_product(earlier_rows, row, elements)
        }

        PatternColumn::Record { fields, .. } => {
            // Expand record fields and recurse
            let elements: List<PatternColumn> =
                fields.iter().map(|(_, col)| col.clone()).collect();
            is_useful_product(earlier_rows, row, &elements)
        }

        PatternColumn::Reference { inner, .. } => {
            // Reference - just check inner pattern
            let modified_row = with_first_column(row, inner.as_ref().clone());
            is_useful(earlier_rows, &modified_row)
        }

        PatternColumn::Stream { head_patterns, tail } => {
            // Stream patterns are like Cons/Nil for lists
            is_useful_stream(earlier_rows, row, head_patterns, tail)
        }

        PatternColumn::TypeTest { type_name, binding } => {
            // TypeTest patterns are runtime type checks
            // They're always potentially useful since they can fail
            is_useful_typetest(earlier_rows, row, type_name, binding)
        }

        PatternColumn::Active { name, bindings, is_total } => {
            // Active patterns are user-defined
            is_useful_active(earlier_rows, row, name, bindings, *is_total)
        }
    }
}

/// Check usefulness for a stream pattern
fn is_useful_stream(
    earlier_rows: &[PatternRow],
    row: &PatternRow,
    head_patterns: &List<PatternColumn>,
    tail: &Option<Box<PatternColumn>>,
) -> bool {
    if head_patterns.is_empty() {
        // Empty stream pattern (Nil) - check if Nil is covered
        let nil_covered = earlier_rows.iter().any(|earlier| {
            if let Some(first) = earlier.columns.first() {
                matches_nil_pattern(first) && !earlier.has_guard
            } else {
                false
            }
        });

        if nil_covered {
            // Nil already covered
            let specialized = specialize_for_wildcard(earlier_rows);
            let rest_row = remove_first_column(row);
            return is_useful(&specialized, &rest_row);
        }
        return true;
    }

    // Non-empty stream (Cons head :: tail)
    // Check if this specific head/tail combination is useful
    let cons_covered = earlier_rows.iter().any(|earlier| {
        if let Some(first) = earlier.columns.first() {
            matches_cons_pattern(first) && !earlier.has_guard
        } else {
            false
        }
    });

    if cons_covered {
        // Expand and recurse
        let expanded_earlier: Vec<_> = earlier_rows
            .iter()
            .filter_map(expand_stream_row)
            .collect();

        let mut new_columns = List::new();
        if let Some(first_head) = head_patterns.first() {
            new_columns.push(first_head.clone());
        }
        // Remaining heads become nested stream
        if head_patterns.len() > 1 {
            new_columns.push(PatternColumn::Stream {
                head_patterns: head_patterns.iter().skip(1).cloned().collect(),
                tail: tail.clone(),
            });
        } else if let Some(t) = tail {
            new_columns.push(t.as_ref().clone());
        } else {
            new_columns.push(PatternColumn::Wildcard);
        }
        for col in row.columns.iter().skip(1) {
            new_columns.push(col.clone());
        }

        let expanded_row = PatternRow::new(new_columns, row.original_index, row.has_guard);
        return is_useful(&expanded_earlier, &expanded_row);
    }

    // Not covered - useful
    true
}

/// Check if a pattern matches the empty/nil case of a stream or list
fn matches_nil_pattern(col: &PatternColumn) -> bool {
    match col {
        PatternColumn::Wildcard => true,
        PatternColumn::Stream { head_patterns, .. } => head_patterns.is_empty(),
        // Structural: a nullary constructor (no args) is the empty case
        PatternColumn::Constructor { args, .. } => args.is_empty(),
        PatternColumn::Guarded(inner) => matches_nil_pattern(inner),
        PatternColumn::Or(alts) => alts.iter().any(matches_nil_pattern),
        _ => false,
    }
}

/// Check if a pattern matches the non-empty/cons case of a stream or list
fn matches_cons_pattern(col: &PatternColumn) -> bool {
    match col {
        PatternColumn::Wildcard => true,
        PatternColumn::Stream { head_patterns, .. } => !head_patterns.is_empty(),
        // Structural: a constructor with args is the non-empty case
        PatternColumn::Constructor { args, .. } => !args.is_empty(),
        PatternColumn::Guarded(inner) => matches_cons_pattern(inner),
        PatternColumn::Or(alts) => alts.iter().any(matches_cons_pattern),
        _ => false,
    }
}

/// Expand a stream pattern row for Cons matching
fn expand_stream_row(row: &PatternRow) -> Option<PatternRow> {
    if let Some(first) = row.columns.first() {
        match first {
            PatternColumn::Wildcard => {
                // Wildcard matches everything - expand to wildcard head and tail
                let mut new_cols = List::from_iter([
                    PatternColumn::Wildcard,
                    PatternColumn::Wildcard,
                ]);
                for col in row.columns.iter().skip(1) {
                    new_cols.push(col.clone());
                }
                Some(PatternRow::new(new_cols, row.original_index, row.has_guard))
            }
            PatternColumn::Stream { head_patterns, tail } if !head_patterns.is_empty() => {
                let mut new_cols = List::new();
                if let Some(head) = head_patterns.first() {
                    new_cols.push(head.clone());
                }
                if head_patterns.len() > 1 {
                    new_cols.push(PatternColumn::Stream {
                        head_patterns: head_patterns.iter().skip(1).cloned().collect(),
                        tail: tail.clone(),
                    });
                } else if let Some(t) = tail {
                    new_cols.push(t.as_ref().clone());
                } else {
                    new_cols.push(PatternColumn::Wildcard);
                }
                for col in row.columns.iter().skip(1) {
                    new_cols.push(col.clone());
                }
                Some(PatternRow::new(new_cols, row.original_index, row.has_guard))
            }
            PatternColumn::Guarded(inner) => {
                let inner_row = PatternRow::new(
                    List::from_iter([inner.as_ref().clone()]),
                    row.original_index,
                    true,
                );
                expand_stream_row(&inner_row).map(|mut r| {
                    for col in row.columns.iter().skip(1) {
                        r.columns.push(col.clone());
                    }
                    r
                })
            }
            _ => None,
        }
    } else {
        None
    }
}

/// Check usefulness for TypeTest pattern
fn is_useful_typetest(
    earlier_rows: &[PatternRow],
    row: &PatternRow,
    type_name: &verum_common::Text,
    binding: &Option<Box<PatternColumn>>,
) -> bool {
    // TypeTest patterns are runtime type checks that may fail
    // They're always potentially useful unless the exact same type test exists earlier

    // Check if an earlier row has the same type test
    let same_typetest_covered = earlier_rows.iter().any(|earlier| {
        if let Some(first) = earlier.columns.first() {
            if let PatternColumn::TypeTest { type_name: other_name, .. } = first {
                other_name == type_name && !earlier.has_guard
            } else {
                matches!(first, PatternColumn::Wildcard) && !earlier.has_guard
            }
        } else {
            false
        }
    });

    if same_typetest_covered {
        // Check if binding adds usefulness
        if let Some(bind) = binding {
            let inner_row = with_first_column(row, bind.as_ref().clone());
            let earlier_bindings: Vec<_> = earlier_rows
                .iter()
                .filter_map(|r| {
                    if let Some(PatternColumn::TypeTest { binding: Some(b), .. }) = r.columns.first() {
                        Some(PatternRow::new(
                            List::from_iter([b.as_ref().clone()]),
                            r.original_index,
                            r.has_guard,
                        ))
                    } else {
                        None
                    }
                })
                .collect();
            return is_useful(&earlier_bindings, &inner_row);
        }
        return false;
    }

    // Not covered - useful
    true
}

/// Check usefulness for Active pattern
fn is_useful_active(
    earlier_rows: &[PatternRow],
    row: &PatternRow,
    name: &verum_common::Text,
    bindings: &List<PatternColumn>,
    is_total: bool,
) -> bool {
    // Active patterns are user-defined pattern functions
    // Total patterns (Bool) are like guards - always potentially useful
    // Partial patterns (Maybe<T>) extract values

    if is_total {
        // Total active patterns can always fail, so they're always potentially useful
        // unless an earlier wildcard covers everything
        let wildcard_covered = earlier_rows.iter().any(|earlier| {
            earlier.columns.first()
                .map(|first| matches!(first, PatternColumn::Wildcard) && !earlier.has_guard)
                .unwrap_or(false)
        });
        return !wildcard_covered;
    }

    // Partial active pattern - check if same active pattern with same bindings exists
    let same_active_covered = earlier_rows.iter().any(|earlier| {
        if let Some(first) = earlier.columns.first() {
            match first {
                PatternColumn::Active { name: other_name, .. } if other_name == name => {
                    !earlier.has_guard
                }
                PatternColumn::Wildcard => !earlier.has_guard,
                _ => false,
            }
        } else {
            false
        }
    });

    if same_active_covered && !bindings.is_empty() {
        // Check if bindings add usefulness
        let earlier_bindings: Vec<_> = earlier_rows
            .iter()
            .filter_map(|r| {
                if let Some(PatternColumn::Active { bindings: other_bindings, .. }) = r.columns.first() {
                    if !other_bindings.is_empty() {
                        Some(PatternRow::new(
                            other_bindings.clone(),
                            r.original_index,
                            r.has_guard,
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        let binding_row = PatternRow::new(bindings.clone(), row.original_index, row.has_guard);
        return is_useful(&earlier_bindings, &binding_row);
    }

    !same_active_covered
}

/// Check usefulness when the first column is a wildcard
///
/// Performance: Uses HashSet<Text> for O(1) constructor lookup instead of O(n) list search
fn is_useful_wildcard(earlier_rows: &[PatternRow], row: &PatternRow) -> bool {
    // Check if any earlier row has a wildcard in the first column (without a guard)
    // A wildcard covers everything, so a later wildcard would be redundant
    let has_earlier_wildcard = earlier_rows.iter().any(|earlier| {
        if let Some(first) = earlier.columns.first() {
            matches_wildcard_pattern(first) && !earlier.has_guard
        } else {
            false
        }
    });

    if has_earlier_wildcard {
        // An earlier wildcard covers everything
        // Check if the rest of the pattern adds anything
        let specialized = specialize_for_wildcard(earlier_rows);
        let rest_row = remove_first_column(row);
        return is_useful(&specialized, &rest_row);
    }

    // Collect all constructors from earlier rows using HashSet for O(1) lookup
    let mut seen_constructors: HashSet<Text> = HashSet::new();

    for earlier in earlier_rows {
        if let Some(first) = earlier.columns.first() {
            collect_constructors_fast(first, &mut seen_constructors);
        }
    }

    // If no constructors seen and no wildcards, this wildcard is useful
    if seen_constructors.is_empty() {
        return true;
    }

    // Check if the wildcard adds any coverage
    // A wildcard after seeing some constructors is useful if:
    // 1. Not all constructors are covered
    // 2. The pattern is useful for at least one remaining case

    // For now, use a conservative check:
    // If we've seen any constructors, check each one
    for ctor_name in &seen_constructors {
        let ctor = Constructor::nullary(ctor_name.to_string());
        let specialized = specialize_for_constructor(earlier_rows, &ctor);
        let rest_row = remove_first_column(row);

        if is_useful(&specialized, &rest_row) {
            return true;
        }
    }

    // Also check for uncovered constructors (wildcard default case)
    let default_ctor = Constructor::default_ctor("_");
    let specialized = specialize_for_constructor(earlier_rows, &default_ctor);
    let rest_row = remove_first_column(row);

    is_useful(&specialized, &rest_row)
}

/// Check if a pattern column is a wildcard-like pattern
fn matches_wildcard_pattern(col: &PatternColumn) -> bool {
    match col {
        PatternColumn::Wildcard => true,
        PatternColumn::Guarded(inner) => matches_wildcard_pattern(inner),
        _ => false,
    }
}

/// Specialize rows for wildcard - remove first column from rows with wildcards
fn specialize_for_wildcard(earlier_rows: &[PatternRow]) -> Vec<PatternRow> {
    earlier_rows
        .iter()
        .filter_map(|row| {
            if let Some(first) = row.columns.first() {
                if matches_wildcard_pattern(first) {
                    Some(remove_first_column(row))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect()
}

/// Check usefulness when the first column is a specific constructor
fn is_useful_constructor(
    earlier_rows: &[PatternRow],
    row: &PatternRow,
    name: &verum_common::Text,
    args: &List<PatternColumn>,
) -> bool {
    let ctor = Constructor::with_args(name.clone(), List::new());

    // Specialize earlier rows for this constructor
    let specialized = specialize_for_constructor(earlier_rows, &ctor);

    // Expand arguments and remaining columns into new row
    let mut new_columns = args.clone();
    for col in row.columns.iter().skip(1) {
        new_columns.push(col.clone());
    }

    let expanded_row = PatternRow::new(new_columns, row.original_index, row.has_guard);

    is_useful(&specialized, &expanded_row)
}

/// Check usefulness for a literal pattern
fn is_useful_literal(
    earlier_rows: &[PatternRow],
    row: &PatternRow,
    lit: &super::matrix::LiteralPattern,
) -> bool {
    // Check if this exact literal is covered by any earlier pattern
    for earlier in earlier_rows {
        if let Some(first) = earlier.columns.first() {
            if literal_covered_by(lit, first) && !earlier.has_guard {
                // This literal is covered by an earlier non-guarded pattern
                let rest_row = remove_first_column(row);
                let earlier_rest = earlier_rows
                    .iter()
                    .map(remove_first_column)
                    .collect::<Vec<_>>();
                return is_useful(&earlier_rest, &rest_row);
            }
        }
    }

    // Literal not covered - useful
    true
}

/// Check if a literal is covered by a pattern column
fn literal_covered_by(
    lit: &super::matrix::LiteralPattern,
    col: &PatternColumn,
) -> bool {
    match col {
        PatternColumn::Wildcard => true,
        PatternColumn::Literal(other) => {
            // Compare literals
            match (lit, other) {
                (super::matrix::LiteralPattern::Int(a), super::matrix::LiteralPattern::Int(b)) => {
                    a == b
                }
                (
                    super::matrix::LiteralPattern::Float(a),
                    super::matrix::LiteralPattern::Float(b),
                ) => (a - b).abs() < f64::EPSILON,
                (
                    super::matrix::LiteralPattern::Bool(a),
                    super::matrix::LiteralPattern::Bool(b),
                ) => a == b,
                (
                    super::matrix::LiteralPattern::Char(a),
                    super::matrix::LiteralPattern::Char(b),
                ) => a == b,
                (
                    super::matrix::LiteralPattern::Text(a),
                    super::matrix::LiteralPattern::Text(b),
                ) => a == b,
                _ => false,
            }
        }
        PatternColumn::Range {
            start,
            end,
            inclusive,
        } => {
            // Check if literal is in range
            if let super::matrix::LiteralPattern::Int(n) = lit {
                let n = *n as i128;
                let in_start = start.is_none_or(|s| n >= s);
                let in_end = if *inclusive {
                    end.is_none_or(|e| n <= e)
                } else {
                    end.is_none_or(|e| n < e)
                };
                in_start && in_end
            } else {
                false
            }
        }
        PatternColumn::Or(alts) => alts.iter().any(|a| literal_covered_by(lit, a)),
        PatternColumn::Guarded(inner) => literal_covered_by(lit, inner),
        _ => false,
    }
}

/// Check usefulness for a range pattern
fn is_useful_range(
    earlier_rows: &[PatternRow],
    row: &PatternRow,
    start: Option<i128>,
    end: Option<i128>,
    inclusive: bool,
) -> bool {
    // For range patterns, check if any part of the range is uncovered
    // This is a simplified check - a full implementation would do interval analysis

    // Check if any earlier pattern fully covers this range
    for earlier in earlier_rows {
        if let Some(first) = earlier.columns.first() {
            if range_covered_by(start, end, inclusive, first) && !earlier.has_guard {
                // Range fully covered
                return false;
            }
        }
    }

    true
}

/// Check if a range is covered by a pattern column
fn range_covered_by(
    start: Option<i128>,
    end: Option<i128>,
    inclusive: bool,
    col: &PatternColumn,
) -> bool {
    match col {
        PatternColumn::Wildcard => true,
        PatternColumn::Range {
            start: s2,
            end: e2,
            inclusive: inc2,
        } => {
            // Check if [start, end] is subset of [s2, e2]
            let covers_start = s2.is_none_or(|s| start.is_some_and(|st| st >= s));
            let covers_end = e2.is_none_or(|e| {
                end.is_some_and(|en| {
                    if *inc2 { en <= e } else { en < e }
                })
            });
            covers_start && covers_end
        }
        PatternColumn::Or(alts) => {
            // Would need to check if union of alternatives covers the range
            // Conservative: only if any single alternative covers it
            alts.iter()
                .any(|a| range_covered_by(start, end, inclusive, a))
        }
        _ => false,
    }
}

/// Check usefulness for a product type (tuple, array, record)
fn is_useful_product(
    earlier_rows: &[PatternRow],
    row: &PatternRow,
    elements: &List<PatternColumn>,
) -> bool {
    // Expand elements into the row
    let mut new_columns = elements.clone();
    for col in row.columns.iter().skip(1) {
        new_columns.push(col.clone());
    }

    // Expand earlier rows similarly
    let expanded_earlier: Vec<_> = earlier_rows
        .iter()
        .filter_map(|r| {
            if let Some(first) = r.columns.first() {
                match first {
                    PatternColumn::Wildcard => {
                        let mut new_cols: List<PatternColumn> = (0..elements.len())
                            .map(|_| PatternColumn::Wildcard)
                            .collect();
                        for col in r.columns.iter().skip(1) {
                            new_cols.push(col.clone());
                        }
                        Some(PatternRow::new(new_cols, r.original_index, r.has_guard))
                    }
                    PatternColumn::Tuple(elems) | PatternColumn::Array(elems) => {
                        let mut new_cols = elems.clone();
                        for col in r.columns.iter().skip(1) {
                            new_cols.push(col.clone());
                        }
                        Some(PatternRow::new(new_cols, r.original_index, r.has_guard))
                    }
                    _ => None,
                }
            } else {
                None
            }
        })
        .collect();

    let expanded_row = PatternRow::new(new_columns, row.original_index, row.has_guard);

    is_useful(&expanded_earlier, &expanded_row)
}

/// Create a new row with a different first column
fn with_first_column(row: &PatternRow, new_first: PatternColumn) -> PatternRow {
    let mut new_columns = List::from_iter([new_first]);
    for col in row.columns.iter().skip(1) {
        new_columns.push(col.clone());
    }
    PatternRow::new(new_columns, row.original_index, row.has_guard)
}

/// Remove the first column from a row
fn remove_first_column(row: &PatternRow) -> PatternRow {
    let new_columns: List<PatternColumn> = row.columns.iter().skip(1).cloned().collect();
    PatternRow::new(new_columns, row.original_index, row.has_guard)
}

/// Specialize rows for a specific constructor
fn specialize_for_constructor(rows: &[PatternRow], ctor: &Constructor) -> Vec<PatternRow> {
    let mut specialized = Vec::new();

    for row in rows {
        if let Some(first) = row.columns.first() {
            match first {
                PatternColumn::Wildcard => {
                    // Wildcard matches all constructors
                    specialized.push(remove_first_column(row));
                }
                PatternColumn::Constructor { name, args } if name == &ctor.name => {
                    // Matching constructor
                    let mut new_cols = args.clone();
                    for col in row.columns.iter().skip(1) {
                        new_cols.push(col.clone());
                    }
                    specialized.push(PatternRow::new(
                        new_cols,
                        row.original_index,
                        row.has_guard,
                    ));
                }
                PatternColumn::Or(alts) => {
                    // Check alternatives
                    for alt in alts.iter() {
                        if constructor_matches(alt, ctor) {
                            let expanded = expand_constructor(alt, ctor);
                            let mut new_cols = expanded;
                            for col in row.columns.iter().skip(1) {
                                new_cols.push(col.clone());
                            }
                            specialized.push(PatternRow::new(
                                new_cols,
                                row.original_index,
                                row.has_guard,
                            ));
                        }
                    }
                }
                PatternColumn::Guarded(inner) if constructor_matches(inner, ctor) => {
                    let expanded = expand_constructor(inner, ctor);
                    let mut new_cols = expanded;
                    for col in row.columns.iter().skip(1) {
                        new_cols.push(col.clone());
                    }
                    specialized.push(PatternRow::new(new_cols, row.original_index, true));
                }
                _ => {}
            }
        }
    }

    specialized
}

/// Check if a pattern column matches a constructor
fn constructor_matches(col: &PatternColumn, ctor: &Constructor) -> bool {
    match col {
        PatternColumn::Wildcard => true,
        PatternColumn::Constructor { name, .. } => name == &ctor.name || ctor.is_default,
        PatternColumn::Guarded(inner) => constructor_matches(inner, ctor),
        _ => ctor.is_default,
    }
}

/// Expand a pattern column for a constructor
fn expand_constructor(col: &PatternColumn, _ctor: &Constructor) -> List<PatternColumn> {
    match col {
        PatternColumn::Wildcard => List::new(),
        PatternColumn::Constructor { args, .. } => args.clone(),
        PatternColumn::Guarded(inner) => expand_constructor(inner, _ctor),
        _ => List::new(),
    }
}

/// Collect constructor names from a pattern column (optimized with HashSet)
///
/// Performance: O(1) insertion vs O(n) for List-based version
fn collect_constructors_fast(col: &PatternColumn, out: &mut HashSet<Text>) {
    match col {
        PatternColumn::Constructor { name, .. } => {
            // HashSet.insert is O(1) amortized
            out.insert(name.clone());
        }
        PatternColumn::Or(alts) => {
            for alt in alts.iter() {
                collect_constructors_fast(alt, out);
            }
        }
        PatternColumn::Guarded(inner) => {
            collect_constructors_fast(inner, out);
        }
        _ => {}
    }
}

/// Collect constructor names from a pattern column (legacy List-based)
#[allow(dead_code)]
fn collect_constructors(col: &PatternColumn, out: &mut List<String>) {
    match col {
        PatternColumn::Constructor { name, .. } => {
            let name_str = name.to_string();
            if !out.iter().any(|n| n == &name_str) {
                out.push(name_str);
            }
        }
        PatternColumn::Or(alts) => {
            for alt in alts.iter() {
                collect_constructors(alt, out);
            }
        }
        PatternColumn::Guarded(inner) => {
            collect_constructors(inner, out);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_wildcard_row(idx: usize) -> PatternRow {
        PatternRow::new(List::from_iter([PatternColumn::Wildcard]), idx, false)
    }

    fn make_ctor_row(name: &str, idx: usize) -> PatternRow {
        PatternRow::new(
            List::from_iter([PatternColumn::Constructor {
                name: verum_common::Text::from(name),
                args: List::new(),
            }]),
            idx,
            false,
        )
    }

    #[test]
    fn test_first_pattern_useful() {
        let row = make_wildcard_row(0);
        assert!(is_useful(&[], &row));
    }

    #[test]
    fn test_wildcard_after_wildcard_not_useful() {
        let earlier = vec![make_wildcard_row(0)];
        let row = make_wildcard_row(1);
        // Wildcard after wildcard is not useful (redundant)
        assert!(!is_useful(&earlier, &row));
    }

    #[test]
    fn test_constructor_after_different_constructor_useful() {
        let earlier = vec![make_ctor_row("Some", 0)];
        let row = make_ctor_row("None", 1);
        // Different constructor is useful
        assert!(is_useful(&earlier, &row));
    }

    #[test]
    fn test_constructor_after_same_constructor_not_useful() {
        let earlier = vec![make_ctor_row("Some", 0)];
        let row = make_ctor_row("Some", 1);
        // Same constructor is not useful (redundant)
        assert!(!is_useful(&earlier, &row));
    }
}
