//! Witness Generation
//!
//! This module generates concrete examples of values not covered by patterns.
//! Witnesses help developers understand what cases are missing from their match.

use super::constructors::{get_type_constructors, Constructor};
use super::matrix::{CoverageMatrix, LiteralPattern, PatternColumn};
use super::smt::{SmtValue, SmtWitness};
use crate::context::TypeEnv;
use crate::ty::Type;
use std::collections::HashSet;
use std::fmt;
use verum_common::{List, Text};

/// A witness represents a concrete value that doesn't match any pattern
#[derive(Debug, Clone)]
pub enum Witness {
    /// A constructor with arguments
    Constructor {
        name: Text,
        args: List<Witness>,
    },

    /// A literal value
    Literal(WitnessLiteral),

    /// A wildcard (any value of this type)
    Wildcard,

    /// A tuple of witnesses
    Tuple(List<Witness>),

    /// A record with named fields
    Record {
        fields: List<(Text, Witness)>,
    },

    /// A range of values
    Range {
        start: Option<i128>,
        end: Option<i128>,
        inclusive: bool,
    },

    /// A stream (head :: tail) witness
    Stream {
        /// Head elements of the stream
        head: List<Witness>,
        /// Tail of the stream (Nil for empty, or another Stream)
        tail: Box<Witness>,
    },
}

/// Literal values in witnesses
#[derive(Debug, Clone)]
pub enum WitnessLiteral {
    Int(i64),
    Float(f64),
    Bool(bool),
    Char(char),
    Text(Text),
}

impl fmt::Display for Witness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Witness::Constructor { name, args } if args.is_empty() => {
                write!(f, "{}", name)
            }
            Witness::Constructor { name, args } => {
                write!(f, "{}(", name)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            Witness::Literal(lit) => match lit {
                WitnessLiteral::Int(n) => write!(f, "{}", n),
                WitnessLiteral::Float(n) => write!(f, "{}", n),
                WitnessLiteral::Bool(b) => write!(f, "{}", b),
                WitnessLiteral::Char(c) => write!(f, "'{}'", c),
                WitnessLiteral::Text(s) => write!(f, "\"{}\"", s),
            },
            Witness::Wildcard => write!(f, "_"),
            Witness::Tuple(elements) => {
                write!(f, "(")?;
                for (i, elem) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", elem)?;
                }
                write!(f, ")")
            }
            Witness::Record { fields } => {
                write!(f, "{{ ")?;
                for (i, (name, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", name, value)?;
                }
                write!(f, " }}")
            }
            Witness::Range {
                start,
                end,
                inclusive,
            } => {
                if let Some(s) = start {
                    write!(f, "{}", s)?;
                }
                if *inclusive {
                    write!(f, "..=")?;
                } else {
                    write!(f, "..")?;
                }
                if let Some(e) = end {
                    write!(f, "{}", e)?;
                }
                Ok(())
            }
            Witness::Stream { head, tail } => {
                if head.is_empty() {
                    // Empty stream
                    write!(f, "[]")
                } else {
                    // Non-empty stream: head :: tail
                    for (i, elem) in head.iter().enumerate() {
                        if i > 0 {
                            write!(f, " :: ")?;
                        }
                        write!(f, "{}", elem)?;
                    }
                    match tail.as_ref() {
                        Witness::Stream { head: h, .. } if h.is_empty() => {
                            // Tail is empty stream - don't print
                            write!(f, " :: []")
                        }
                        Witness::Wildcard => {
                            write!(f, " :: _")
                        }
                        other => {
                            write!(f, " :: {}", other)
                        }
                    }
                }
            }
        }
    }
}

impl Witness {
    /// Create a nullary constructor witness (no arguments)
    pub fn nullary(name: impl Into<Text>) -> Self {
        Witness::Constructor {
            name: name.into(),
            args: List::new(),
        }
    }

    /// Create a constructor witness with arguments
    pub fn constructor(name: impl Into<Text>, args: List<Witness>) -> Self {
        Witness::Constructor {
            name: name.into(),
            args,
        }
    }

    /// Create an integer literal witness
    pub fn int(n: i64) -> Self {
        Witness::Literal(WitnessLiteral::Int(n))
    }

    /// Create a boolean literal witness
    pub fn bool(b: bool) -> Self {
        Witness::Literal(WitnessLiteral::Bool(b))
    }

    /// Create a float literal witness
    pub fn float(f: f64) -> Self {
        Witness::Literal(WitnessLiteral::Float(f))
    }

    /// Create a text literal witness
    pub fn text(s: impl Into<Text>) -> Self {
        Witness::Literal(WitnessLiteral::Text(s.into()))
    }

    /// Convert an SMT witness to a standard Witness
    ///
    /// This is used when the SMT guard verifier finds uncovered cases.
    /// The SMT witness contains variable bindings which we convert to
    /// appropriate witness values based on the scrutinee type.
    ///
    /// # Arguments
    ///
    /// * `smt_witness` - The SMT witness containing variable bindings
    /// * `scrutinee_ty` - The type being matched on
    ///
    /// # Returns
    ///
    /// A `Witness` representing the uncovered case
    pub fn from_smt_witness(smt_witness: &SmtWitness, scrutinee_ty: &Type) -> Witness {
        // If we have bindings, use the first one as the primary witness
        // This handles the common case of a single variable being matched
        if let Some((name, value)) = smt_witness.bindings.iter().next() {
            return Self::from_smt_value(value, scrutinee_ty);
        }

        // Fallback: create a type-appropriate wildcard or default
        Self::default_for_type(scrutinee_ty)
    }

    /// Convert an SMT value to a Witness
    fn from_smt_value(value: &SmtValue, _ty: &Type) -> Witness {
        match value {
            SmtValue::Int(n) => {
                // Convert i128 to i64 with clamping for extreme values
                let clamped = (*n).clamp(i64::MIN as i128, i64::MAX as i128) as i64;
                Witness::int(clamped)
            }
            SmtValue::Float(f) => Witness::float(*f),
            SmtValue::Bool(b) => Witness::bool(*b),
            SmtValue::Unknown => Witness::Wildcard,
        }
    }

    /// Create a default witness for a type
    fn default_for_type(ty: &Type) -> Witness {
        match ty {
            Type::Int => Witness::int(0),
            Type::Float => Witness::float(0.0),
            Type::Bool => Witness::bool(false),
            Type::Char => Witness::Literal(WitnessLiteral::Char('a')),
            Type::Text => Witness::text(""),
            Type::Unit => Witness::Tuple(List::new()),
            _ => Witness::Wildcard,
        }
    }
}

/// Generate any witness for a type (used when no patterns given)
pub fn generate_any_witness(ty: &Type, env: &TypeEnv) -> Witness {
    let ctors = get_type_constructors(ty, env);

    if ctors.is_empty() {
        return Witness::Wildcard;
    }

    // Use the first constructor
    if let Some(first) = ctors.iter().next() {
        generate_witness_for_constructor(first, &CoverageMatrix::new(ty.clone()), env)
    } else {
        Witness::Wildcard
    }
}

/// Generate a witness for a specific constructor
pub fn generate_witness_for_constructor(
    ctor: &Constructor,
    matrix: &CoverageMatrix,
    env: &TypeEnv,
) -> Witness {
    if ctor.arg_types.is_empty() {
        // Nullary constructor
        Witness::Constructor {
            name: ctor.name.clone(),
            args: List::new(),
        }
    } else {
        // Constructor with arguments - generate witnesses for each arg
        let args: List<Witness> = ctor
            .arg_types
            .iter()
            .enumerate()
            .map(|(idx, arg_ty)| {
                // Find uncovered sub-patterns
                let sub_matrix = extract_arg_matrix(matrix, &ctor.name, idx);
                generate_uncovered_witness(arg_ty, &sub_matrix, env)
            })
            .collect();

        Witness::Constructor {
            name: ctor.name.clone(),
            args,
        }
    }
}

/// Generate a witness for an uncovered case
pub fn generate_uncovered_witness(
    ty: &Type,
    covered_patterns: &[PatternColumn],
    env: &TypeEnv,
) -> Witness {
    // Check specific types
    match ty {
        Type::Bool => {
            // Find which bool value isn't covered
            let covers_true = covered_patterns
                .iter()
                .any(|p| covers_bool_value(p, true));
            let covers_false = covered_patterns
                .iter()
                .any(|p| covers_bool_value(p, false));

            if !covers_true {
                Witness::Literal(WitnessLiteral::Bool(true))
            } else if !covers_false {
                Witness::Literal(WitnessLiteral::Bool(false))
            } else {
                Witness::Wildcard
            }
        }

        Type::Int => {
            // Find an uncovered integer value
            let uncovered = find_uncovered_integer(covered_patterns);
            Witness::Literal(WitnessLiteral::Int(uncovered))
        }

        Type::Unit => Witness::Literal(WitnessLiteral::Text(Text::from("()"))),

        Type::Tuple(elements) => {
            // Generate witnesses for each tuple element
            let elem_witnesses: List<Witness> = elements
                .iter()
                .enumerate()
                .map(|(idx, elem_ty)| {
                    let sub_patterns = extract_tuple_element(covered_patterns, idx);
                    generate_uncovered_witness(elem_ty, &sub_patterns, env)
                })
                .collect();
            Witness::Tuple(elem_witnesses)
        }

        Type::Variant(variants) => {
            // Find an uncovered variant
            for (name, variant_ty) in variants.iter() {
                let covers_variant = covered_patterns.iter().any(|p| covers_constructor(p, name));
                if !covers_variant {
                    if *variant_ty == Type::Unit {
                        return Witness::Constructor {
                            name: name.clone(),
                            args: List::new(),
                        };
                    } else {
                        return Witness::Constructor {
                            name: name.clone(),
                            args: List::from_iter([Witness::Wildcard]),
                        };
                    }
                }
            }
            Witness::Wildcard
        }

        Type::Generic { name: _, args } => {
            let ctors = get_type_constructors(ty, env);

            // Structural stream detection: 2 constructors (one nullary, one with args)
            // and patterns include stream patterns
            let is_stream_like = ctors.len() == 2
                && ctors.iter().any(|c| c.arg_types.is_empty())
                && ctors.iter().any(|c| !c.arg_types.is_empty())
                && covered_patterns.iter().any(|p| matches!(p, PatternColumn::Stream { .. }));

            if is_stream_like {
                return generate_stream_witness(covered_patterns, args, env);
            }

            for ctor in ctors.iter() {
                let covers_ctor = covered_patterns
                    .iter()
                    .any(|p| covers_constructor(p, &ctor.name));
                if !covers_ctor && !ctor.is_default {
                    return generate_witness_for_constructor(ctor, &CoverageMatrix::new(ty.clone()), env);
                }
            }
            Witness::Wildcard
        }

        Type::Named { path: _, args } => {
            let ctors = get_type_constructors(ty, env);

            let is_stream_like = ctors.len() == 2
                && ctors.iter().any(|c| c.arg_types.is_empty())
                && ctors.iter().any(|c| !c.arg_types.is_empty())
                && covered_patterns.iter().any(|p| matches!(p, PatternColumn::Stream { .. }));

            if is_stream_like {
                return generate_stream_witness(covered_patterns, args, env);
            }

            for ctor in ctors.iter() {
                let covers_ctor = covered_patterns
                    .iter()
                    .any(|p| covers_constructor(p, &ctor.name));
                if !covers_ctor && !ctor.is_default {
                    return generate_witness_for_constructor(ctor, &CoverageMatrix::new(ty.clone()), env);
                }
            }
            Witness::Wildcard
        }

        _ => Witness::Wildcard,
    }
}

/// Generate a witness for an uncovered stream case
fn generate_stream_witness(
    covered_patterns: &[PatternColumn],
    args: &List<Type>,
    env: &TypeEnv,
) -> Witness {
    let elem_ty = args.first().cloned().unwrap_or(Type::Unknown);

    // Check if Nil (empty stream) is covered
    let covers_nil = covered_patterns.iter().any(covers_nil_stream);

    // Check if Cons (non-empty stream) is covered
    let covers_cons = covered_patterns.iter().any(covers_cons_stream);

    if !covers_nil {
        // Empty stream not covered
        Witness::Stream {
            head: List::new(),
            tail: Box::new(Witness::Stream {
                head: List::new(),
                tail: Box::new(Witness::Wildcard),
            }),
        }
    } else if !covers_cons {
        // Non-empty stream not covered
        let elem_witness = generate_uncovered_witness(&elem_ty, &[], env);
        Witness::Stream {
            head: List::from_iter([elem_witness]),
            tail: Box::new(Witness::Wildcard),
        }
    } else {
        // Both covered - need to find uncovered sub-patterns
        // This would require more sophisticated analysis
        Witness::Wildcard
    }
}

/// Check if a pattern covers the empty/nil case of a stream
fn covers_nil_stream(pattern: &PatternColumn) -> bool {
    match pattern {
        PatternColumn::Wildcard => true,
        PatternColumn::Stream { head_patterns, .. } => head_patterns.is_empty(),
        // Structural: a nullary constructor covers the empty case
        PatternColumn::Constructor { args, .. } => args.is_empty(),
        PatternColumn::Or(alts) => alts.iter().any(covers_nil_stream),
        PatternColumn::Guarded(inner) => covers_nil_stream(inner),
        _ => false,
    }
}

/// Check if a pattern covers the non-empty/cons case of a stream
fn covers_cons_stream(pattern: &PatternColumn) -> bool {
    match pattern {
        PatternColumn::Wildcard => true,
        PatternColumn::Stream { head_patterns, .. } => !head_patterns.is_empty(),
        // Structural: a constructor with args covers the non-empty case
        PatternColumn::Constructor { args, .. } => !args.is_empty(),
        PatternColumn::Or(alts) => alts.iter().any(covers_cons_stream),
        PatternColumn::Guarded(inner) => covers_cons_stream(inner),
        _ => false,
    }
}

/// Check if a pattern covers a specific bool value
fn covers_bool_value(pattern: &PatternColumn, value: bool) -> bool {
    match pattern {
        PatternColumn::Wildcard => true,
        PatternColumn::Literal(LiteralPattern::Bool(b)) => *b == value,
        PatternColumn::Or(alts) => alts.iter().any(|a| covers_bool_value(a, value)),
        PatternColumn::Guarded(inner) => covers_bool_value(inner, value),
        _ => false,
    }
}

/// Check if a pattern covers a specific constructor
fn covers_constructor(pattern: &PatternColumn, name: &Text) -> bool {
    match pattern {
        PatternColumn::Wildcard => true,
        PatternColumn::Constructor {
            name: ctor_name, ..
        } => ctor_name == name,
        PatternColumn::Or(alts) => alts.iter().any(|a| covers_constructor(a, name)),
        PatternColumn::Guarded(inner) => covers_constructor(inner, name),
        _ => false,
    }
}

/// Find an uncovered integer value
///
/// Performance: Uses HashSet for O(1) value lookup instead of O(n) list search
fn find_uncovered_integer(patterns: &[PatternColumn]) -> i64 {
    // Collect all covered integers using HashSet for O(1) lookup
    let mut covered_values: HashSet<i64> = HashSet::new();
    let mut covered_ranges: Vec<(i128, i128)> = Vec::new();

    for pattern in patterns {
        collect_integer_coverage_fast(pattern, &mut covered_values, &mut covered_ranges);
    }

    // Helper to check if value is in any range
    let in_any_range = |val: i64| -> bool {
        let val = val as i128;
        covered_ranges
            .iter()
            .any(|(start, end)| val >= *start && val <= *end)
    };

    // Find a value not covered
    for candidate in 0i64.. {
        // O(1) HashSet lookup instead of O(n) list search
        if !covered_values.contains(&candidate) && !in_any_range(candidate) {
            return candidate;
        }

        // Also try negative values
        let neg_candidate = -(candidate + 1);
        if !covered_values.contains(&neg_candidate) && !in_any_range(neg_candidate) {
            return neg_candidate;
        }

        // Limit search to prevent infinite loop
        if candidate > 100 {
            break;
        }
    }

    // Default: use a large number
    999999
}

/// Collect integer coverage from a pattern (optimized with HashSet)
fn collect_integer_coverage_fast(
    pattern: &PatternColumn,
    values: &mut HashSet<i64>,
    ranges: &mut Vec<(i128, i128)>,
) {
    match pattern {
        PatternColumn::Literal(LiteralPattern::Int(n)) => {
            values.insert(*n);
        }
        PatternColumn::Range {
            start,
            end,
            inclusive,
        } => {
            let s = start.unwrap_or(i128::MIN);
            let e = if *inclusive {
                end.unwrap_or(i128::MAX)
            } else {
                end.map(|e| e - 1).unwrap_or(i128::MAX)
            };
            ranges.push((s, e));
        }
        PatternColumn::Or(alts) => {
            for alt in alts.iter() {
                collect_integer_coverage_fast(alt, values, ranges);
            }
        }
        _ => {}
    }
}

/// Extract patterns for a specific tuple element
fn extract_tuple_element(patterns: &[PatternColumn], idx: usize) -> Vec<PatternColumn> {
    patterns
        .iter()
        .filter_map(|p| match p {
            PatternColumn::Tuple(elements) => elements.get(idx).cloned(),
            PatternColumn::Wildcard => Some(PatternColumn::Wildcard),
            _ => None,
        })
        .collect()
}

/// Extract patterns for constructor arguments
fn extract_arg_matrix(
    matrix: &CoverageMatrix,
    ctor_name: &Text,
    arg_idx: usize,
) -> Vec<PatternColumn> {
    matrix
        .rows
        .iter()
        .filter_map(|row| {
            row.columns.first().and_then(|col| match col {
                PatternColumn::Constructor { name, args } if name == ctor_name => {
                    args.get(arg_idx).cloned()
                }
                PatternColumn::Wildcard => Some(PatternColumn::Wildcard),
                _ => None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_witness_display() {
        let w = Witness::Constructor {
            name: Text::from("Some"),
            args: List::from_iter([Witness::Literal(WitnessLiteral::Int(42))]),
        };
        assert_eq!(format!("{}", w), "Some(42)");
    }

    #[test]
    fn test_find_uncovered_integer() {
        let patterns = vec![
            PatternColumn::Literal(LiteralPattern::Int(0)),
            PatternColumn::Literal(LiteralPattern::Int(1)),
        ];
        let uncovered = find_uncovered_integer(&patterns);
        assert!(uncovered != 0 && uncovered != 1);
    }

    #[test]
    fn test_bool_coverage() {
        let true_pattern = PatternColumn::Literal(LiteralPattern::Bool(true));
        assert!(covers_bool_value(&true_pattern, true));
        assert!(!covers_bool_value(&true_pattern, false));

        let wildcard = PatternColumn::Wildcard;
        assert!(covers_bool_value(&wildcard, true));
        assert!(covers_bool_value(&wildcard, false));
    }
}
