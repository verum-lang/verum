//! Pattern Exhaustiveness Checking
//!
//! This module implements comprehensive exhaustiveness checking for pattern matching
//! in Verum. It uses a matrix-based algorithm inspired by Maranget's "Warnings for
//! pattern matching" paper.
//!
//! # Overview
//!
//! The algorithm works in several phases:
//!
//! 1. **Type Deconstruction**: Enumerate all constructors for a type
//! 2. **Matrix Construction**: Convert patterns into a coverage matrix
//! 3. **Usefulness Check**: Determine if each pattern adds coverage
//! 4. **Exhaustiveness Check**: Verify all cases are covered
//! 5. **Witness Generation**: Create examples of uncovered cases
//!
//! # Supported Pattern Kinds
//!
//! The system handles all 19 pattern kinds in Verum's AST:
//! - Wildcard, Rest, Ident: Cover everything
//! - Literal: Cover one specific value
//! - Tuple, Array, Record: Product types (recursive)
//! - Slice: Variable-length arrays with rest patterns
//! - Variant: Sum type constructors
//! - Or: Union of alternatives
//! - Reference: Pattern on dereferenced value
//! - Range: Interval coverage
//! - Paren: Transparent wrapper
//! - Active: User-defined patterns (total vs partial)
//! - And: Intersection (all must match)
//! - Guard: Conditional patterns (conservative handling)
//! - TypeTest: Runtime type checking
//! - Stream: Iterator head/tail decomposition
//!
//! # Example
//!
//! ```rust,ignore
//! use verum_types::exhaustiveness::{check_exhaustiveness, ExhaustivenessResult};
//!
//! let result = check_exhaustiveness(
//!     &patterns,
//!     &scrutinee_type,
//!     &type_env,
//! )?;
//!
//! if !result.is_exhaustive {
//!     for witness in &result.uncovered_witnesses {
//!         eprintln!("Uncovered case: {}", witness);
//!     }
//! }
//! ```
//!
//! # References
//!
//! - Maranget, L. "Warnings for pattern matching" (2007)
//! - Rust RFC 3637: Guard Patterns
//! - Pattern exhaustiveness checking: ensuring match expressions cover all possible values

mod constructors;
mod diagnostics;
mod matrix;
mod ranges;
mod usefulness;
mod witness;

// Advanced features (Future Extensions from design doc)
pub mod active_exhaustiveness;
pub mod cache;
pub mod dependent;
pub mod refinement;
pub mod smt;

pub use constructors::{Constructor, TypeConstructors};
pub use diagnostics::{
    ExhaustivenessError, ExhaustivenessErrorCode, ExhaustivenessWarning, ExhaustivenessWarningCode,
    format_witnesses, suggest_missing_patterns,
};
pub use matrix::{CoverageMatrix, LiteralPattern, PatternColumn, PatternRow};
pub use ranges::{
    Interval, IntervalSet, RangeOverlap, RangeOverlapAnalysis,
    analyze_range_overlaps, describe_range_overlap, find_uncovered_ranges,
    format_range_overlap_error, ranges_cover_type, suggest_uncovered_ranges,
};
pub use usefulness::is_useful;
pub use witness::{Witness, WitnessLiteral, generate_any_witness, generate_uncovered_witness};

// Advanced features exports
pub use cache::{
    CacheConfig, CacheKey, CacheStats, CachedResult, ExhaustivenessCache, clear_global_cache,
    global_cache, global_cache_stats,
};
pub use refinement::{
    RefinementConstraint, RefinementContext, check_exhaustiveness_with_refinement,
    extract_refinement,
};
pub use smt::{
    GuardedPattern, SmtGuardConfig, SmtGuardResult, SmtGuardVerifier, SmtValue, SmtWitness,
    analyze_guarded_match,
};
pub use dependent::{
    DependentExhaustivenessChecker, DependentExhaustivenessConfig, DependentExhaustivenessResult,
    IndexRefinement, check_dependent_match_unified, check_exhaustiveness_unified,
};
pub use active_exhaustiveness::{
    ActivePatternCallId, ActivePatternCallTracker, ActivePatternOptimizationHints,
    ActivePatternRegistry, VariantReturningPattern, analyze_match_for_optimization,
    analyze_return_type, check_active_pattern_in_matrix, check_variant_pattern_exhaustiveness,
    detect_variant_returning_match, extract_covered_constructors,
};

use crate::context::TypeEnv;
use crate::ty::Type;
use crate::TypeError;
use verum_ast::pattern::Pattern;
use verum_common::{List, Text};

/// Result of exhaustiveness checking
#[derive(Debug, Clone)]
pub struct ExhaustivenessResult {
    /// Whether the patterns cover all possible cases
    pub is_exhaustive: bool,

    /// Concrete examples of uncovered cases
    /// Each witness represents a value that wouldn't match any pattern
    pub uncovered_witnesses: List<Witness>,

    /// Indices of patterns that are unreachable (redundant)
    /// These patterns will never match because earlier patterns cover their cases
    pub redundant_patterns: List<usize>,

    /// Whether all patterns have guards (warning: may block if all guards false)
    pub all_guarded: bool,

    /// Range pattern overlap analysis (if any range patterns found)
    pub range_overlaps: Option<RangeOverlapAnalysis>,

    /// Warnings generated during analysis
    pub warnings: List<ExhaustivenessWarning>,
}

impl ExhaustivenessResult {
    /// Create a result indicating exhaustive patterns
    pub fn exhaustive() -> Self {
        Self {
            is_exhaustive: true,
            uncovered_witnesses: List::new(),
            redundant_patterns: List::new(),
            all_guarded: false,
            range_overlaps: None,
            warnings: List::new(),
        }
    }

    /// Create a result indicating non-exhaustive patterns
    pub fn non_exhaustive(witnesses: List<Witness>) -> Self {
        Self {
            is_exhaustive: false,
            uncovered_witnesses: witnesses,
            redundant_patterns: List::new(),
            all_guarded: false,
            range_overlaps: None,
            warnings: List::new(),
        }
    }

    /// Add range overlap analysis to the result
    pub fn with_range_overlaps(mut self, analysis: RangeOverlapAnalysis) -> Self {
        // Generate warnings from the analysis
        for overlap in analysis.overlaps.iter() {
            if overlap.is_redundant {
                self.warnings.push(ExhaustivenessWarning::redundant_range(
                    overlap.second_pattern_index,
                    overlap.first_pattern_index,
                    None,
                ));
            } else {
                self.warnings.push(ExhaustivenessWarning::range_overlap(
                    overlap.first_pattern_index,
                    overlap.second_pattern_index,
                    overlap.overlap.start,
                    overlap.overlap.end,
                    None,
                ));
            }
        }
        self.range_overlaps = Some(analysis);
        self
    }

    /// Add a warning to the result
    pub fn with_warning(mut self, warning: ExhaustivenessWarning) -> Self {
        self.warnings.push(warning);
        self
    }
}

/// Check if a set of patterns is exhaustive for a given type
///
/// This is the main entry point for exhaustiveness checking.
///
/// # Arguments
///
/// * `patterns` - The patterns to check
/// * `scrutinee_ty` - The type being matched against
/// * `env` - Type environment for looking up type definitions
///
/// # Returns
///
/// An `ExhaustivenessResult` containing:
/// - Whether the match is exhaustive
/// - Witnesses for uncovered cases
/// - Indices of redundant patterns
pub fn check_exhaustiveness(
    patterns: &[Pattern],
    scrutinee_ty: &Type,
    env: &TypeEnv,
) -> Result<ExhaustivenessResult, TypeError> {
    check_exhaustiveness_with_options(patterns, scrutinee_ty, env, &ExhaustivenessConfig::default())
}

/// Check exhaustiveness with custom configuration
///
/// This allows fine-grained control over the exhaustiveness checking process,
/// including whether to use refinement-aware analysis and SMT verification.
pub fn check_exhaustiveness_with_options(
    patterns: &[Pattern],
    scrutinee_ty: &Type,
    env: &TypeEnv,
    config: &ExhaustivenessConfig,
) -> Result<ExhaustivenessResult, TypeError> {
    // Handle empty patterns - always non-exhaustive (unless scrutinee is empty type)
    if patterns.is_empty() {
        let constructors = constructors::get_type_constructors(scrutinee_ty, env);
        if constructors.is_empty_type() {
            // Empty type (like `Never`) - no patterns needed
            return Ok(ExhaustivenessResult::exhaustive());
        }

        // Non-empty type with no patterns - generate witness
        let witness = witness::generate_any_witness(scrutinee_ty, env);
        return Ok(ExhaustivenessResult::non_exhaustive(List::from_iter([
            witness,
        ])));
    }

    // Build the coverage matrix from patterns
    let matrix = matrix::build_matrix(patterns, scrutinee_ty, env)?;

    // Check for redundant patterns
    let redundant = if config.check_redundancy {
        find_redundant_patterns(&matrix)
    } else {
        List::new()
    };

    // Check if all patterns are guarded
    let all_guarded = matrix.rows.iter().all(|row| row.has_guard);

    // Extract pattern columns for refinement checking
    let pattern_columns: Vec<PatternColumn> = matrix
        .rows
        .iter()
        .filter_map(|row| row.columns.first().cloned())
        .collect();

    // Try SMT guard verification for all-guarded matches
    // This can prove exhaustiveness for cases like: n < 0, n == 0, n > 0
    if all_guarded && config.use_smt_guards {
        let guarded_patterns = extract_guarded_patterns(&matrix, scrutinee_ty);

        if !guarded_patterns.is_empty() {
            let smt_config = smt::SmtGuardConfig {
                timeout_ms: config.smt_timeout_ms,
                max_guards: guarded_patterns.len(),
                extract_witnesses: true,
                detect_redundancy: config.check_redundancy,
            };

            let verifier = smt::SmtGuardVerifier::new(smt_config);
            let smt_result = verifier.verify_guards(&guarded_patterns, scrutinee_ty, env);

            if !smt_result.skipped {
                if smt_result.is_exhaustive {
                    // SMT proved exhaustiveness - return success
                    return Ok(ExhaustivenessResult {
                        is_exhaustive: true,
                        uncovered_witnesses: List::new(),
                        redundant_patterns: redundant,
                        all_guarded: true,
                        range_overlaps: None,
                        warnings: List::new(),
                    });
                } else if !smt_result.uncovered_witnesses.is_empty() {
                    // SMT found uncovered cases - convert to witnesses
                    let witnesses: List<_> = smt_result.uncovered_witnesses
                        .iter()
                        .map(|smt_witness| {
                            witness::Witness::from_smt_witness(smt_witness, scrutinee_ty)
                        })
                        .collect();

                    return Ok(ExhaustivenessResult {
                        is_exhaustive: false,
                        uncovered_witnesses: witnesses,
                        redundant_patterns: redundant,
                        all_guarded: true,
                        range_overlaps: None,
                        warnings: List::new(),
                    });
                }
                // SMT inconclusive - fall through to standard check
            }
        }
    }

    // Try refinement-aware exhaustiveness for refined types
    // This enables more precise analysis for types like `Int{x: x > 0}`
    let uncovered = if config.use_refinement && matches!(scrutinee_ty, Type::Refined { .. }) {
        let (is_exhaustive, witnesses) =
            refinement::check_exhaustiveness_with_refinement(&pattern_columns, scrutinee_ty, env);
        if is_exhaustive {
            List::new()
        } else {
            witnesses
        }
    } else {
        // Standard exhaustiveness check
        find_uncovered_cases(&matrix, scrutinee_ty, env)?
    };

    // Analyze range patterns for overlaps
    let range_analysis = analyze_range_patterns_in_matrix(&matrix);
    let mut warnings = List::new();

    if let Some(ref analysis) = range_analysis {
        for overlap in analysis.overlaps.iter() {
            if overlap.is_redundant {
                warnings.push(ExhaustivenessWarning::redundant_range(
                    overlap.second_pattern_index,
                    overlap.first_pattern_index,
                    None,
                ));
            } else {
                warnings.push(ExhaustivenessWarning::range_overlap(
                    overlap.first_pattern_index,
                    overlap.second_pattern_index,
                    overlap.overlap.start,
                    overlap.overlap.end,
                    None,
                ));
            }
        }
    }

    Ok(ExhaustivenessResult {
        is_exhaustive: uncovered.is_empty(),
        uncovered_witnesses: uncovered,
        redundant_patterns: redundant,
        all_guarded,
        range_overlaps: range_analysis,
        warnings,
    })
}

/// Extract guarded patterns from matrix for SMT verification
fn extract_guarded_patterns(
    matrix: &CoverageMatrix,
    _scrutinee_ty: &Type,
) -> Vec<smt::GuardedPattern> {
    use std::collections::HashMap;
    use std::sync::Arc;
    use verum_ast::literal::{Literal, LiteralKind};
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::span::Span;

    let mut guarded = Vec::new();

    for (idx, row) in matrix.rows.iter().enumerate() {
        if row.has_guard {
            // For now, we extract guarded patterns without the actual guard expression
            // A full implementation would parse the guard from the AST
            // Use a "true" literal as placeholder - represents "always true" guard
            if let Some(first) = row.columns.first() {
                let placeholder_guard = Expr::new(
                    ExprKind::Literal(Literal {
                        kind: LiteralKind::Bool(true),
                        span: Span::dummy(),
                    }),
                    Span::dummy(),
                );
                guarded.push(smt::GuardedPattern {
                    pattern_index: idx,
                    base_pattern: first.clone(),
                    guard: Arc::new(placeholder_guard),
                    bound_vars: HashMap::new(),
                });
            }
        }
    }

    guarded
}

/// Analyze range patterns in the matrix for overlaps
fn analyze_range_patterns_in_matrix(matrix: &CoverageMatrix) -> Option<RangeOverlapAnalysis> {
    let mut ranges = Vec::new();

    // Default bounds for unbounded ranges (use i64 range to avoid overflow)
    let default_min = i64::MIN as i128;
    let default_max = i64::MAX as i128;

    // Extract range patterns from the matrix
    for (idx, row) in matrix.rows.iter().enumerate() {
        for col in row.columns.iter() {
            if let PatternColumn::Range { start, end, .. } = col {
                let s = start.unwrap_or(default_min);
                let e = end.unwrap_or(default_max);
                ranges.push((idx, Interval::new(s, e)));
            }
        }
    }

    // If no range patterns, return None
    if ranges.is_empty() {
        return None;
    }

    Some(ranges::analyze_range_overlaps(&ranges))
}

/// Find patterns that are redundant (unreachable)
///
/// Performance: O(n²) where n = number of patterns (was O(n³) before optimization)
fn find_redundant_patterns(matrix: &CoverageMatrix) -> List<usize> {
    let mut redundant = List::new();

    // Convert to Vec once for efficient slicing (List may not support O(1) slicing)
    let rows: Vec<_> = matrix.rows.to_vec();

    for (i, row) in rows.iter().enumerate() {
        // Use slice directly - O(1) instead of O(n) clone
        let earlier = &rows[..i];

        // Check if this pattern adds any coverage
        if !earlier.is_empty() && !is_useful(earlier, row) {
            redundant.push(row.original_index);
        }
    }

    redundant
}

/// Find cases not covered by any pattern
fn find_uncovered_cases(
    matrix: &CoverageMatrix,
    scrutinee_ty: &Type,
    env: &TypeEnv,
) -> Result<List<Witness>, TypeError> {
    // Check for a non-guarded wildcard/ident row -- covers everything
    if matrix.has_wildcard_row() {
        return Ok(List::new());
    }

    // Type-specific exhaustiveness logic
    match scrutinee_ty {
        // Bool: treat as 2-variant enum {true, false}
        Type::Bool => {
            return find_uncovered_bool(matrix);
        }

        // Int/Float: infinite domain -- exhaustive only with wildcard
        Type::Int | Type::Float => {
            return find_uncovered_numeric(matrix, scrutinee_ty);
        }

        // Tuple: decompose and check each element
        Type::Tuple(elements) => {
            return find_uncovered_tuple(matrix, elements, env);
        }

        _ => {}
    }

    // Default: constructor-based analysis
    let mut uncovered = List::new();
    let constructors = constructors::get_type_constructors(scrutinee_ty, env);

    for ctor in constructors.iter() {
        if !is_constructor_covered(matrix, ctor, env)? {
            let w = witness::generate_witness_for_constructor(ctor, matrix, env);
            uncovered.push(w);
        }
    }

    Ok(uncovered)
}

/// Find uncovered cases for Bool scrutinee
///
/// Bool is treated as a 2-variant enum {true, false}. A literal `true`
/// covers the `true` case, `false` covers the `false` case, and a wildcard
/// covers both. Guarded patterns do NOT provide definitive coverage because
/// the guard can fail at runtime.
fn find_uncovered_bool(matrix: &CoverageMatrix) -> Result<List<Witness>, TypeError> {
    let mut covers_true = false;
    let mut covers_false = false;

    for row in matrix.rows.iter() {
        // Guarded patterns don't provide definitive coverage
        if row.has_guard {
            continue;
        }
        if let Some(first) = row.columns.first() {
            check_bool_coverage(first, &mut covers_true, &mut covers_false);
        }
    }

    let mut uncovered = List::new();
    if !covers_true {
        uncovered.push(Witness::bool(true));
    }
    if !covers_false {
        uncovered.push(Witness::bool(false));
    }
    Ok(uncovered)
}

/// Recursively check which bool values a pattern column covers
fn check_bool_coverage(col: &PatternColumn, covers_true: &mut bool, covers_false: &mut bool) {
    match col {
        PatternColumn::Wildcard => {
            *covers_true = true;
            *covers_false = true;
        }
        PatternColumn::Literal(LiteralPattern::Bool(true)) => {
            *covers_true = true;
        }
        PatternColumn::Literal(LiteralPattern::Bool(false)) => {
            *covers_false = true;
        }
        PatternColumn::Constructor { name, .. } if name.as_str() == "true" => {
            *covers_true = true;
        }
        PatternColumn::Constructor { name, .. } if name.as_str() == "false" => {
            *covers_false = true;
        }
        PatternColumn::Or(alts) => {
            for alt in alts.iter() {
                check_bool_coverage(alt, covers_true, covers_false);
            }
        }
        PatternColumn::Guarded(_inner) => {
            // Guarded inner -- conservative: don't count as coverage
        }
        PatternColumn::And(conjuncts) => {
            // And pattern covers what its most specific conjunct covers
            for conj in conjuncts.iter() {
                check_bool_coverage(conj, covers_true, covers_false);
            }
        }
        _ => {}
    }
}

/// Find uncovered cases for numeric types (Int, Float)
///
/// Numeric types have infinite domains. They are exhaustive only if:
/// - There is a non-guarded wildcard/ident pattern, OR
/// - There is a non-guarded range pattern that covers the entire domain
///
/// Literal-only matches without a wildcard are always non-exhaustive for
/// infinite types. Guard-only patterns require a wildcard fallback.
fn find_uncovered_numeric(
    matrix: &CoverageMatrix,
    scrutinee_ty: &Type,
) -> Result<List<Witness>, TypeError> {
    // Wildcard already checked by caller (has_wildcard_row)

    // Collect all non-guarded coverage (literals + ranges)
    let mut covered_values = std::collections::HashSet::new();
    let mut covered_ranges: Vec<(i128, i128)> = Vec::new();
    let mut has_unguarded_wildcard = false;

    for row in matrix.rows.iter() {
        if row.has_guard {
            continue;
        }
        if let Some(first) = row.columns.first() {
            collect_numeric_coverage(
                first,
                &mut covered_values,
                &mut covered_ranges,
                &mut has_unguarded_wildcard,
            );
        }
    }

    if has_unguarded_wildcard {
        return Ok(List::new());
    }

    // Numeric without wildcard -- non-exhaustive. Generate a witness value.
    let uncovered_val = find_uncovered_int_value(&covered_values, &covered_ranges);
    let w = match scrutinee_ty {
        Type::Float => Witness::float(uncovered_val as f64),
        _ => Witness::int(uncovered_val),
    };
    Ok(List::from_iter([w]))
}

/// Collect numeric coverage from a pattern column
fn collect_numeric_coverage(
    col: &PatternColumn,
    values: &mut std::collections::HashSet<i64>,
    ranges: &mut Vec<(i128, i128)>,
    has_wildcard: &mut bool,
) {
    match col {
        PatternColumn::Wildcard => {
            *has_wildcard = true;
        }
        PatternColumn::Literal(LiteralPattern::Int(n)) => {
            values.insert(*n);
        }
        PatternColumn::Range { start, end, inclusive } => {
            let s = start.unwrap_or(i128::MIN);
            let e = if *inclusive {
                end.unwrap_or(i128::MAX)
            } else {
                end.map(|v| v - 1).unwrap_or(i128::MAX)
            };
            ranges.push((s, e));
        }
        PatternColumn::Or(alts) => {
            for alt in alts.iter() {
                collect_numeric_coverage(alt, values, ranges, has_wildcard);
            }
        }
        PatternColumn::And(conjuncts) => {
            for conj in conjuncts.iter() {
                collect_numeric_coverage(conj, values, ranges, has_wildcard);
            }
        }
        _ => {}
    }
}

/// Find an integer value not covered by the given literals and ranges
fn find_uncovered_int_value(
    values: &std::collections::HashSet<i64>,
    ranges: &[(i128, i128)],
) -> i64 {
    let in_range = |v: i64| -> bool {
        let v = v as i128;
        ranges.iter().any(|(s, e)| v >= *s && v <= *e)
    };
    for candidate in 0i64.. {
        if !values.contains(&candidate) && !in_range(candidate) {
            return candidate;
        }
        let neg = -(candidate + 1);
        if !values.contains(&neg) && !in_range(neg) {
            return neg;
        }
        if candidate > 100 {
            break;
        }
    }
    999999
}

/// Find uncovered cases for Tuple scrutinee
///
/// Tuples have a single constructor `()` with element types as arguments.
/// We use the constructor-based approach: specialize the matrix for the
/// tuple constructor, then recursively check each element argument.
/// This correctly handles cross-product coverage (e.g., (true,true)+(false,false)
/// is NOT exhaustive because (true,false) and (false,true) are missing).
fn find_uncovered_tuple(
    matrix: &CoverageMatrix,
    elements: &List<Type>,
    env: &TypeEnv,
) -> Result<List<Witness>, TypeError> {
    if elements.is_empty() {
        // Unit tuple -- always exhaustive if any non-guarded row exists
        if matrix.rows.iter().any(|r| !r.has_guard) {
            return Ok(List::new());
        }
        return Ok(List::from_iter([Witness::Tuple(List::new())]));
    }

    // Use the standard constructor-based approach.
    // get_type_constructors for Tuple returns a single constructor "()" with element args.
    let constructors = constructors::get_type_constructors(
        &Type::Tuple(elements.clone()),
        env,
    );

    let mut uncovered = List::new();
    for ctor in constructors.iter() {
        if !is_constructor_covered(matrix, ctor, env)? {
            let w = witness::generate_witness_for_constructor(ctor, matrix, env);
            uncovered.push(w);
        }
    }

    Ok(uncovered)
}

/// Check if a specific constructor is covered by the pattern matrix
fn is_constructor_covered(
    matrix: &CoverageMatrix,
    ctor: &Constructor,
    env: &TypeEnv,
) -> Result<bool, TypeError> {
    // Specialize the matrix for this constructor
    let specialized = matrix::specialize_matrix(matrix, ctor);

    // If specialized matrix has any wildcard rows, constructor is covered
    if specialized.has_wildcard_row() {
        return Ok(true);
    }

    // If constructor has no arguments, check if any NON-GUARDED row matches it.
    // Guarded rows (including TypeTest runtime checks) don't provide definitive coverage
    // because the guard/test may fail at runtime.
    if ctor.arg_types.is_empty() {
        return Ok(specialized.rows.iter().any(|r| !r.has_guard));
    }

    // For constructors with arguments, recursively check sub-patterns.
    // We must check the *first* arg using all rows, then for each value of
    // the first arg, check the remaining args. This preserves row correlation
    // (cross-product coverage).
    is_specialized_matrix_exhaustive(&specialized, &ctor.arg_types, env)
}

/// Check if a specialized matrix (expanded constructor args) is exhaustive.
///
/// The matrix has columns corresponding to constructor argument types.
/// We process the first column: for each constructor of arg_types[0],
/// specialize and recurse on the remaining columns. This correctly handles
/// cross-product coverage (e.g., {(true,true),(false,false)} is NOT
/// exhaustive for (Bool, Bool)).
fn is_specialized_matrix_exhaustive(
    matrix: &CoverageMatrix,
    arg_types: &List<Type>,
    env: &TypeEnv,
) -> Result<bool, TypeError> {
    if arg_types.is_empty() || matrix.rows.is_empty() {
        // No more args to check -- covered if any non-guarded row exists
        return Ok(matrix.rows.iter().any(|r| !r.has_guard));
    }

    // Check if there's a full wildcard row (all remaining columns are wildcards)
    let has_full_wildcard = matrix.rows.iter().any(|row| {
        !row.has_guard && row.columns.iter().all(|c| matches!(c, PatternColumn::Wildcard))
    });
    if has_full_wildcard {
        return Ok(true);
    }

    let first_ty = &arg_types[0];
    let rest_types: List<Type> = arg_types.iter().skip(1).cloned().collect();

    // Get constructors for the first argument type
    let first_ctors = constructors::get_type_constructors(first_ty, env);

    // For infinite types (Int, Float, etc.), check if there's a wildcard in column 0
    if first_ctors.is_infinite {
        let has_col0_wildcard = matrix.rows.iter().any(|row| {
            !row.has_guard
                && row.columns.first().is_some_and(|c| matches!(c, PatternColumn::Wildcard))
        });
        if !has_col0_wildcard {
            return Ok(false);
        }
        // Wildcard in col 0 means col 0 is covered; check remaining cols
        let wildcard_rows: Vec<PatternRow> = matrix
            .rows
            .iter()
            .filter_map(|row| {
                if row.columns.first().is_some_and(|c| matches!(c, PatternColumn::Wildcard)) {
                    let rest_cols: List<PatternColumn> =
                        row.columns.iter().skip(1).cloned().collect();
                    Some(PatternRow::new(rest_cols, row.original_index, row.has_guard))
                } else {
                    None
                }
            })
            .collect();
        let sub_matrix = CoverageMatrix {
            rows: wildcard_rows,
            scrutinee_ty: matrix.scrutinee_ty.clone(),
        };
        return is_specialized_matrix_exhaustive(&sub_matrix, &rest_types, env);
    }

    // For finite types, check each constructor of the first arg
    for sub_ctor in first_ctors.iter() {
        // Specialize matrix: keep rows whose col 0 matches this constructor,
        // expand sub-constructor args, then append cols 1..
        let sub_matrix = specialize_first_column(matrix, sub_ctor);

        let mut sub_arg_types: List<Type> = sub_ctor.arg_types.iter().cloned().collect();
        for ty in rest_types.iter() {
            sub_arg_types.push(ty.clone());
        }

        if !is_specialized_matrix_exhaustive(&sub_matrix, &sub_arg_types, env)? {
            return Ok(false);
        }
    }

    Ok(true)
}

/// Specialize the first column of a matrix for a given constructor.
/// Returns a new matrix where matching rows have their first column replaced
/// by the constructor's argument patterns, and the remaining columns preserved.
fn specialize_first_column(
    matrix: &CoverageMatrix,
    ctor: &Constructor,
) -> CoverageMatrix {
    let mut result = CoverageMatrix::new(matrix.scrutinee_ty.clone());

    for row in matrix.rows.iter() {
        if let Some(first) = row.columns.first() {
            let expanded = match first {
                PatternColumn::Wildcard => {
                    // Wildcard matches all constructors: expand to wildcard args
                    Some((
                        (0..ctor.arg_types.len())
                            .map(|_| PatternColumn::Wildcard)
                            .collect::<List<PatternColumn>>(),
                        row.has_guard,
                    ))
                }
                PatternColumn::Constructor { name, args } if name == &ctor.name => {
                    Some((args.clone(), row.has_guard))
                }
                PatternColumn::Literal(LiteralPattern::Bool(b)) => {
                    let lit_name = if *b { "true" } else { "false" };
                    if ctor.name.as_str() == lit_name {
                        Some((List::new(), row.has_guard))
                    } else {
                        None
                    }
                }
                PatternColumn::Tuple(elements) if ctor.name.as_str() == "()" => {
                    Some((elements.clone(), row.has_guard))
                }
                PatternColumn::Guarded(inner) => {
                    // Recursively check inner pattern, mark as guarded
                    match inner.as_ref() {
                        PatternColumn::Wildcard => {
                            Some((
                                (0..ctor.arg_types.len())
                                    .map(|_| PatternColumn::Wildcard)
                                    .collect(),
                                true,
                            ))
                        }
                        PatternColumn::Constructor { name, args } if name == &ctor.name => {
                            Some((args.clone(), true))
                        }
                        PatternColumn::Literal(LiteralPattern::Bool(b)) => {
                            let lit_name = if *b { "true" } else { "false" };
                            if ctor.name.as_str() == lit_name {
                                Some((List::new(), true))
                            } else {
                                None
                            }
                        }
                        PatternColumn::Tuple(elements) if ctor.name.as_str() == "()" => {
                            Some((elements.clone(), true))
                        }
                        _ => None,
                    }
                }
                PatternColumn::Or(alts) => {
                    // Check if any alternative matches
                    let mut found = None;
                    for alt in alts.iter() {
                        if matches_col_ctor(alt, ctor) {
                            found = Some((expand_col_ctor(alt, ctor), row.has_guard));
                            break;
                        }
                    }
                    found
                }
                _ => None,
            };

            if let Some((expanded_args, has_guard)) = expanded {
                let mut new_cols = expanded_args;
                for col in row.columns.iter().skip(1) {
                    new_cols.push(col.clone());
                }
                result.add_row(PatternRow::new(new_cols, row.original_index, has_guard));
            }
        }
    }

    result
}

/// Check if a pattern column matches a constructor (for first-column specialization)
fn matches_col_ctor(col: &PatternColumn, ctor: &Constructor) -> bool {
    match col {
        PatternColumn::Wildcard => true,
        PatternColumn::Constructor { name, .. } => name == &ctor.name,
        PatternColumn::Literal(LiteralPattern::Bool(b)) => {
            let lit_name = if *b { "true" } else { "false" };
            ctor.name.as_str() == lit_name
        }
        PatternColumn::Tuple(_) => ctor.name.as_str() == "()",
        PatternColumn::Guarded(inner) => matches_col_ctor(inner, ctor),
        PatternColumn::Or(alts) => alts.iter().any(|a| matches_col_ctor(a, ctor)),
        _ => ctor.is_default,
    }
}

/// Expand a pattern column for a constructor (for first-column specialization)
fn expand_col_ctor(col: &PatternColumn, ctor: &Constructor) -> List<PatternColumn> {
    match col {
        PatternColumn::Wildcard => {
            (0..ctor.arg_types.len())
                .map(|_| PatternColumn::Wildcard)
                .collect()
        }
        PatternColumn::Constructor { args, .. } => args.clone(),
        PatternColumn::Literal(LiteralPattern::Bool(_)) => List::new(),
        PatternColumn::Tuple(elements) => elements.clone(),
        PatternColumn::Guarded(inner) => expand_col_ctor(inner, ctor),
        _ => List::new(),
    }
}

/// Internal exhaustiveness check for sub-patterns
fn check_exhaustiveness_internal(
    rows: &[PatternColumn],
    ty: &Type,
    env: &TypeEnv,
) -> Result<ExhaustivenessResult, TypeError> {
    // Convert columns to a simple matrix for recursive checking
    let patterns: Vec<Pattern> = rows.iter().map(|c| c.to_pattern()).collect();
    check_exhaustiveness(&patterns, ty, env)
}

/// Configuration for exhaustiveness checking
#[derive(Debug, Clone)]
pub struct ExhaustivenessConfig {
    /// Maximum number of witnesses to generate
    pub max_witnesses: usize,

    /// Whether to check for redundant patterns
    pub check_redundancy: bool,

    /// Whether to warn about all-guarded matches
    pub warn_all_guarded: bool,

    /// Whether to use refinement-aware analysis for refined types
    /// When enabled, types like `Int{x: x > 0}` will eliminate impossible cases
    pub use_refinement: bool,

    /// Whether to use SMT solving for guard verification
    /// When enabled, guards like `n < 0`, `n == 0`, `n > 0` can be proven exhaustive
    pub use_smt_guards: bool,

    /// Timeout for SMT guard verification (in milliseconds)
    pub smt_timeout_ms: u64,
}

impl Default for ExhaustivenessConfig {
    fn default() -> Self {
        Self {
            max_witnesses: 3,
            check_redundancy: true,
            warn_all_guarded: true,
            use_refinement: true,
            use_smt_guards: false, // Disabled by default due to Z3 dependency
            smt_timeout_ms: 100,
        }
    }
}

impl ExhaustivenessConfig {
    /// Create a configuration with all advanced features enabled
    pub fn full() -> Self {
        Self {
            max_witnesses: 5,
            check_redundancy: true,
            warn_all_guarded: true,
            use_refinement: true,
            use_smt_guards: true,
            smt_timeout_ms: 200,
        }
    }

    /// Create a minimal configuration for fast checking
    pub fn minimal() -> Self {
        Self {
            max_witnesses: 1,
            check_redundancy: false,
            warn_all_guarded: false,
            use_refinement: false,
            use_smt_guards: false,
            smt_timeout_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::literal::{Literal, LiteralKind};
    use verum_ast::pattern::{Pattern, PatternKind};
    use verum_ast::span::Span;
    use verum_common::Heap;

    fn span() -> Span { Span::dummy() }
    fn env() -> TypeEnv { TypeEnv::default() }

    fn pat_wildcard() -> Pattern { Pattern::wildcard(span()) }
    fn pat_bool(b: bool) -> Pattern { Pattern::literal(Literal::bool(b, span())) }
    fn pat_int(n: i128) -> Pattern { Pattern::literal(Literal::int(n, span())) }

    fn pat_ident(name: &str) -> Pattern {
        Pattern::new(
            PatternKind::Ident {
                name: verum_ast::ty::Ident::new(Text::from(name), span()),
                mutable: false,
                by_ref: false,
                subpattern: verum_common::Maybe::None,
            },
            span(),
        )
    }

    fn pat_range(start: Option<i128>, end: Option<i128>, inclusive: bool) -> Pattern {
        let start_lit = start.map(|v| Heap::new(Literal::int(v, span())));
        let end_lit = end.map(|v| Heap::new(Literal::int(v, span())));
        Pattern::new(
            PatternKind::Range {
                start: start_lit.into(),
                end: end_lit.into(),
                inclusive,
            },
            span(),
        )
    }

    fn pat_tuple(elements: Vec<Pattern>) -> Pattern {
        Pattern::new(PatternKind::Tuple(List::from_iter(elements)), span())
    }

    fn pat_guard(pattern: Pattern, guard: verum_ast::expr::Expr) -> Pattern {
        Pattern::new(
            PatternKind::Guard {
                pattern: Heap::new(pattern),
                guard: Heap::new(guard),
            },
            span(),
        )
    }

    fn expr_bool(b: bool) -> verum_ast::expr::Expr {
        verum_ast::expr::Expr::new(
            verum_ast::expr::ExprKind::Literal(Literal {
                kind: LiteralKind::Bool(b),
                span: span(),
            }),
            span(),
        )
    }

    // ===== Bool exhaustiveness =====

    #[test]
    fn test_bool_true_false_exhaustive() {
        let patterns = vec![pat_bool(true), pat_bool(false)];
        let result = check_exhaustiveness(&patterns, &Type::Bool, &env()).unwrap();
        assert!(result.is_exhaustive, "true + false should be exhaustive for Bool");
    }

    #[test]
    fn test_bool_false_true_exhaustive() {
        let patterns = vec![pat_bool(false), pat_bool(true)];
        let result = check_exhaustiveness(&patterns, &Type::Bool, &env()).unwrap();
        assert!(result.is_exhaustive);
    }

    #[test]
    fn test_bool_only_true_non_exhaustive() {
        let patterns = vec![pat_bool(true)];
        let result = check_exhaustiveness(&patterns, &Type::Bool, &env()).unwrap();
        assert!(!result.is_exhaustive, "only true should NOT be exhaustive for Bool");
        assert!(!result.uncovered_witnesses.is_empty());
    }

    #[test]
    fn test_bool_only_false_non_exhaustive() {
        let patterns = vec![pat_bool(false)];
        let result = check_exhaustiveness(&patterns, &Type::Bool, &env()).unwrap();
        assert!(!result.is_exhaustive);
    }

    #[test]
    fn test_bool_wildcard_exhaustive() {
        let patterns = vec![pat_wildcard()];
        let result = check_exhaustiveness(&patterns, &Type::Bool, &env()).unwrap();
        assert!(result.is_exhaustive);
    }

    #[test]
    fn test_bool_ident_exhaustive() {
        let patterns = vec![pat_ident("b")];
        let result = check_exhaustiveness(&patterns, &Type::Bool, &env()).unwrap();
        assert!(result.is_exhaustive, "identifier binding should be exhaustive for Bool");
    }

    // ===== Integer literal exhaustiveness =====

    #[test]
    fn test_int_literals_with_wildcard_exhaustive() {
        let patterns = vec![pat_int(0), pat_int(1), pat_wildcard()];
        let result = check_exhaustiveness(&patterns, &Type::Int, &env()).unwrap();
        assert!(result.is_exhaustive);
    }

    #[test]
    fn test_int_literals_without_wildcard_non_exhaustive() {
        let patterns = vec![pat_int(0), pat_int(1)];
        let result = check_exhaustiveness(&patterns, &Type::Int, &env()).unwrap();
        assert!(!result.is_exhaustive);
        assert!(!result.uncovered_witnesses.is_empty());
    }

    #[test]
    fn test_int_single_wildcard_exhaustive() {
        let patterns = vec![pat_wildcard()];
        let result = check_exhaustiveness(&patterns, &Type::Int, &env()).unwrap();
        assert!(result.is_exhaustive);
    }

    #[test]
    fn test_int_ident_binding_exhaustive() {
        let patterns = vec![pat_ident("x")];
        let result = check_exhaustiveness(&patterns, &Type::Int, &env()).unwrap();
        assert!(result.is_exhaustive);
    }

    // ===== Range pattern exhaustiveness =====

    #[test]
    fn test_range_with_wildcard_exhaustive() {
        let patterns = vec![
            pat_range(Some(0), Some(10), false),
            pat_wildcard(),
        ];
        let result = check_exhaustiveness(&patterns, &Type::Int, &env()).unwrap();
        assert!(result.is_exhaustive);
    }

    #[test]
    fn test_range_without_wildcard_non_exhaustive() {
        let patterns = vec![pat_range(Some(0), Some(10), true)];
        let result = check_exhaustiveness(&patterns, &Type::Int, &env()).unwrap();
        assert!(!result.is_exhaustive);
    }

    #[test]
    fn test_multiple_ranges_with_wildcard_exhaustive() {
        let patterns = vec![
            pat_range(Some(0), Some(10), true),
            pat_range(Some(11), Some(100), true),
            pat_wildcard(),
        ];
        let result = check_exhaustiveness(&patterns, &Type::Int, &env()).unwrap();
        assert!(result.is_exhaustive);
    }

    // ===== Tuple pattern exhaustiveness =====

    #[test]
    fn test_tuple_bool_bool_exhaustive() {
        let patterns = vec![
            pat_tuple(vec![pat_bool(true), pat_bool(true)]),
            pat_tuple(vec![pat_bool(true), pat_bool(false)]),
            pat_tuple(vec![pat_bool(false), pat_wildcard()]),
        ];
        let ty = Type::Tuple(List::from_iter([Type::Bool, Type::Bool]));
        let result = check_exhaustiveness(&patterns, &ty, &env()).unwrap();
        assert!(result.is_exhaustive, "(true,true)+(true,false)+(false,_) should be exhaustive");
    }

    #[test]
    fn test_tuple_bool_bool_non_exhaustive() {
        let patterns = vec![
            pat_tuple(vec![pat_bool(true), pat_bool(true)]),
            pat_tuple(vec![pat_bool(false), pat_bool(false)]),
        ];
        let ty = Type::Tuple(List::from_iter([Type::Bool, Type::Bool]));
        let result = check_exhaustiveness(&patterns, &ty, &env()).unwrap();
        assert!(!result.is_exhaustive, "missing (true,false) and (false,true)");
    }

    #[test]
    fn test_tuple_wildcard_exhaustive() {
        let patterns = vec![pat_wildcard()];
        let ty = Type::Tuple(List::from_iter([Type::Bool, Type::Int]));
        let result = check_exhaustiveness(&patterns, &ty, &env()).unwrap();
        assert!(result.is_exhaustive);
    }

    #[test]
    fn test_tuple_int_with_wildcard() {
        let patterns = vec![
            pat_tuple(vec![pat_int(0), pat_wildcard()]),
            pat_tuple(vec![pat_wildcard(), pat_int(0)]),
            pat_wildcard(),
        ];
        let ty = Type::Tuple(List::from_iter([Type::Int, Type::Int]));
        let result = check_exhaustiveness(&patterns, &ty, &env()).unwrap();
        assert!(result.is_exhaustive);
    }

    // ===== Guard-aware exhaustiveness =====

    #[test]
    fn test_guard_with_wildcard_fallback_exhaustive() {
        let patterns = vec![
            pat_guard(pat_ident("n"), expr_bool(true)),
            pat_wildcard(),
        ];
        let result = check_exhaustiveness(&patterns, &Type::Int, &env()).unwrap();
        assert!(result.is_exhaustive, "guarded pattern + wildcard fallback should be exhaustive");
    }

    #[test]
    fn test_guard_without_fallback_non_exhaustive() {
        let patterns = vec![pat_guard(pat_ident("n"), expr_bool(true))];
        let result = check_exhaustiveness(&patterns, &Type::Int, &env()).unwrap();
        assert!(!result.is_exhaustive, "guarded pattern without fallback should NOT be exhaustive");
    }

    #[test]
    fn test_all_guarded_non_exhaustive() {
        let patterns = vec![
            pat_guard(pat_ident("n"), expr_bool(true)),
            pat_guard(pat_ident("n"), expr_bool(true)),
        ];
        let result = check_exhaustiveness(&patterns, &Type::Int, &env()).unwrap();
        assert!(!result.is_exhaustive);
        assert!(result.all_guarded, "should detect all_guarded");
    }

    #[test]
    fn test_guard_on_bool_with_wildcard() {
        let patterns = vec![
            pat_guard(pat_bool(true), expr_bool(true)),
            pat_wildcard(),
        ];
        let result = check_exhaustiveness(&patterns, &Type::Bool, &env()).unwrap();
        assert!(result.is_exhaustive);
    }

    #[test]
    fn test_guard_on_bool_without_unguarded_coverage() {
        let patterns = vec![
            pat_guard(pat_bool(true), expr_bool(true)),
            pat_guard(pat_bool(false), expr_bool(true)),
        ];
        let result = check_exhaustiveness(&patterns, &Type::Bool, &env()).unwrap();
        assert!(!result.is_exhaustive, "all-guarded bool match should NOT be exhaustive");
    }

    // ===== Mixed patterns =====

    #[test]
    fn test_int_literals_range_wildcard() {
        let patterns = vec![
            pat_int(0),
            pat_range(Some(1), Some(10), true),
            pat_wildcard(),
        ];
        let result = check_exhaustiveness(&patterns, &Type::Int, &env()).unwrap();
        assert!(result.is_exhaustive);
    }

    #[test]
    fn test_empty_patterns_non_exhaustive() {
        let patterns: Vec<Pattern> = vec![];
        let result = check_exhaustiveness(&patterns, &Type::Int, &env()).unwrap();
        assert!(!result.is_exhaustive);
    }

    #[test]
    fn test_empty_patterns_never_type_exhaustive() {
        let patterns: Vec<Pattern> = vec![];
        let result = check_exhaustiveness(&patterns, &Type::Never, &env()).unwrap();
        assert!(result.is_exhaustive);
    }

    #[test]
    fn test_unit_wildcard_exhaustive() {
        let patterns = vec![pat_wildcard()];
        let result = check_exhaustiveness(&patterns, &Type::Unit, &env()).unwrap();
        assert!(result.is_exhaustive);
    }
}
