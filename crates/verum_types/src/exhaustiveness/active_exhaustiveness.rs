//! Active Pattern Exhaustiveness
//!
//! This module provides exhaustiveness checking for variant-returning active patterns.
//! When an active pattern returns a sum type instead of Bool/Maybe, we can prove
//! exhaustiveness by checking that all variants of the return type are handled.
//!
//! # Concept
//!
//! Standard active patterns return `Bool` (total) or `Maybe<T>` (partial), which
//! cannot participate in exhaustiveness checking. However, if an active pattern
//! returns a sum type, we can check exhaustiveness:
//!
//! ```verum
//! // Variant-returning active pattern
//! type Parity is Even | Odd;
//!
//! pattern Parity(n: Int) -> Parity =
//!     if n % 2 == 0 { Even } else { Odd };
//!
//! // Can be checked for exhaustiveness!
//! match Parity(x) {
//!     Even => "even",
//!     Odd => "odd",  // Compiler verifies: exhaustive!
//! }
//! ```
//!
//! # Design Rationale
//!
//! This approach leverages existing sum type exhaustiveness infrastructure rather
//! than introducing a new `@complete` annotation system like F#. The key insight
//! is that matching on the *result* of the pattern application uses standard
//! exhaustiveness checking.
//!
//! ## Alternative Considered: @complete Attribute
//!
//! F# uses `@complete(Even, Odd)` annotations. We rejected this because:
//! 1. Requires new attribute infrastructure
//! 2. More error-prone (manual listing of cases)
//! 3. Doesn't leverage existing type system
//!
//! # References
//!
//! - Active pattern exhaustiveness: checking that user-defined active patterns cover all cases
//! - Pattern exhaustiveness checking: ensuring match expressions cover all possible values

use super::constructors::{get_type_constructors, Constructor, TypeConstructors};
use super::matrix::{CoverageMatrix, PatternColumn, PatternRow};
use super::witness::{generate_any_witness, Witness};
use super::ExhaustivenessResult;
use crate::context::TypeEnv;
use crate::ty::Type;
use crate::TypeError;
use std::collections::HashMap;
use verum_common::{List, Map, Text};

/// Information about a variant-returning active pattern
#[derive(Debug, Clone)]
pub struct VariantReturningPattern {
    /// The pattern name (e.g., "Parity")
    pub name: Text,

    /// The return type (must be a variant/sum type)
    pub return_type: Type,

    /// The constructors of the return type
    pub constructors: List<Constructor>,

    /// Whether this pattern is total (always produces a value)
    pub is_total: bool,
}

/// Registry of variant-returning patterns for exhaustiveness checking
#[derive(Debug, Clone, Default)]
pub struct ActivePatternRegistry {
    /// Map from pattern name to its variant-returning info
    patterns: HashMap<Text, VariantReturningPattern>,
}

impl ActivePatternRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            patterns: HashMap::new(),
        }
    }

    /// Register a variant-returning active pattern
    pub fn register(&mut self, pattern: VariantReturningPattern) {
        self.patterns.insert(pattern.name.clone(), pattern);
    }

    /// Look up a pattern by name
    pub fn get(&self, name: &Text) -> Option<&VariantReturningPattern> {
        self.patterns.get(name)
    }

    /// Check if a pattern name is registered as variant-returning
    pub fn is_variant_returning(&self, name: &Text) -> bool {
        self.patterns.contains_key(name)
    }

    /// Get all constructors for a variant-returning pattern
    pub fn get_constructors(&self, name: &Text) -> Option<&List<Constructor>> {
        self.patterns.get(name).map(|p| &p.constructors)
    }
}

/// Analyze whether a type is suitable as a variant-returning pattern result
///
/// Returns `Some(constructors)` if the type is a finite sum type with
/// enumerable constructors. Returns `None` for Bool, Maybe, or non-sum types.
pub fn analyze_return_type(ty: &Type, env: &TypeEnv) -> Option<TypeConstructors> {
    match ty {
        // Bool is handled as total pattern, not variant-returning
        Type::Bool => None,

        // Named types - look up constructors from the type environment
        Type::Generic { .. } | Type::Named { .. } => {
            let ctors = get_type_constructors(ty, env);

            // Only finite, non-infinite types with >1 constructor can be variant-returning.
            // 2-variant types with one nullary (Maybe-like) are handled as partial patterns,
            // not variant-returning. Check structurally instead of by name.
            if ctors.is_infinite || ctors.is_empty_type() || ctors.is_empty() {
                return None;
            }

            // Skip Maybe-like types (exactly 2 constructors, one nullary)
            if ctors.len() == 2
                && ctors.iter().any(|c| c.arg_types.is_empty())
                && ctors.iter().any(|c| !c.arg_types.is_empty())
            {
                return None;
            }

            Some(ctors)
        }

        // Variant types (inline sum types) are directly usable
        Type::Variant(variants) => {
            if variants.is_empty() {
                return None;
            }

            let ctors = get_type_constructors(ty, env);
            if !ctors.is_infinite && !ctors.is_empty() {
                Some(ctors)
            } else {
                None
            }
        }

        // (Generic types handled by the Type::Generic arm above)

        _ => None,
    }
}

/// Check exhaustiveness for a match on a variant-returning active pattern
///
/// This is called when we detect that a match expression matches on the result
/// of an active pattern application where the pattern returns a sum type.
///
/// # Arguments
///
/// * `pattern_info` - Information about the variant-returning pattern
/// * `covered_constructors` - The constructors covered by match arms
/// * `env` - Type environment
///
/// # Returns
///
/// `ExhaustivenessResult` indicating whether all variants are covered
pub fn check_variant_pattern_exhaustiveness(
    pattern_info: &VariantReturningPattern,
    covered_constructors: &[Text],
    env: &TypeEnv,
) -> ExhaustivenessResult {
    // Find uncovered constructors
    let mut uncovered = List::new();

    for ctor in pattern_info.constructors.iter() {
        if !covered_constructors.iter().any(|c| c.as_str() == ctor.name.as_str()) {
            // Generate witness for this uncovered constructor
            let witness = Witness::constructor(
                ctor.name.clone(),
                ctor.arg_types.iter().map(|ty| {
                    generate_any_witness(ty, env)
                }).collect(),
            );
            uncovered.push(witness);
        }
    }

    if uncovered.is_empty() {
        ExhaustivenessResult::exhaustive()
    } else {
        ExhaustivenessResult::non_exhaustive(uncovered)
    }
}

/// Detect and extract variant-returning pattern from a match scrutinee
///
/// Analyzes a match expression to determine if it's matching on the result
/// of a variant-returning active pattern application.
///
/// # Example
///
/// For `match Parity(x) { Even => ..., Odd => ... }`:
/// - Detects that `Parity` is a variant-returning pattern
/// - Returns the pattern info for exhaustiveness checking
pub fn detect_variant_returning_match(
    scrutinee_ty: &Type,
    active_pattern_name: Option<&Text>,
    registry: &ActivePatternRegistry,
    env: &TypeEnv,
) -> Option<VariantReturningPattern> {
    // If we have an explicit active pattern name, check registry
    if let Some(name) = active_pattern_name {
        if let Some(info) = registry.get(name) {
            return Some(info.clone());
        }
    }

    // Otherwise, analyze the scrutinee type directly
    // This handles cases where the match is on a variant type value
    // that was produced by an active pattern
    if let Some(ctors) = analyze_return_type(scrutinee_ty, env) {
        // Create a synthetic pattern info for direct variant matching
        let name = match scrutinee_ty {
            Type::Named { path, .. } => path.segments.last().map(|s| match s {
                verum_ast::ty::PathSegment::Name(id) => Text::from(id.name.as_str()),
                _ => Text::from("_variant"),
            }).unwrap_or_else(|| Text::from("_variant")),
            Type::Variant(_) => Text::from("_inline_variant"),
            _ => Text::from("_unknown"),
        };

        Some(VariantReturningPattern {
            name,
            return_type: scrutinee_ty.clone(),
            constructors: ctors.iter().cloned().collect(),
            is_total: true,
        })
    } else {
        None
    }
}

/// Extract constructor names from active pattern bindings in a match arm
///
/// When matching on `Parity(x)`, the arm `Even => ...` has `Even` as the constructor.
/// This extracts those constructor names for exhaustiveness checking.
pub fn extract_covered_constructors(rows: &[PatternRow]) -> List<Text> {
    let mut covered = List::new();

    for row in rows {
        if let Some(first) = row.columns.first() {
            collect_constructor_names(first, &mut covered);
        }
    }

    covered
}

/// Recursively collect constructor names from a pattern column
fn collect_constructor_names(col: &PatternColumn, out: &mut List<Text>) {
    match col {
        PatternColumn::Constructor { name, .. } => {
            if !out.iter().any(|n| n == name) {
                out.push(name.clone());
            }
        }
        PatternColumn::Or(alts) => {
            for alt in alts.iter() {
                collect_constructor_names(alt, out);
            }
        }
        PatternColumn::And(conjs) => {
            // For And patterns, any constructor in any conjunct counts
            for conj in conjs.iter() {
                collect_constructor_names(conj, out);
            }
        }
        PatternColumn::Guarded(inner) => {
            collect_constructor_names(inner, out);
        }
        PatternColumn::Active { bindings, .. } => {
            // If active pattern has constructor bindings, extract them
            for binding in bindings.iter() {
                collect_constructor_names(binding, out);
            }
        }
        // Wildcard covers all constructors - don't add to explicit list
        // but signal that everything is covered
        PatternColumn::Wildcard => {
            // We handle wildcards separately in the caller
        }
        _ => {}
    }
}

/// Check if a match has a wildcard that covers all remaining cases
pub fn has_wildcard_coverage(rows: &[PatternRow]) -> bool {
    rows.iter().any(|row| {
        row.columns.first()
            .map(|col| matches!(col, PatternColumn::Wildcard) && !row.has_guard)
            .unwrap_or(false)
    })
}

/// Integration point: Check active pattern exhaustiveness within the main checker
///
/// This function is called by the main exhaustiveness checker when it detects
/// that the scrutinee involves a variant-returning active pattern.
pub fn check_active_pattern_in_matrix(
    matrix: &CoverageMatrix,
    pattern_info: &VariantReturningPattern,
    env: &TypeEnv,
) -> ExhaustivenessResult {
    // Check for wildcard coverage first
    if has_wildcard_coverage(&matrix.rows) {
        return ExhaustivenessResult::exhaustive();
    }

    // Extract covered constructors
    let covered: Vec<Text> = extract_covered_constructors(&matrix.rows)
        .iter()
        .cloned()
        .collect();

    // Check exhaustiveness
    check_variant_pattern_exhaustiveness(pattern_info, &covered, env)
}

// ============================================================
// ACTIVE PATTERN DOUBLE-CALL OPTIMIZATION
// ============================================================
//
// When the same active pattern is called multiple times on the same
// value in a match expression, we want to optimize by caching the result.
//
// Example:
// ```verum
// match x {
//     Parity(Even) & Positive() => "positive even",
//     Parity(Odd) & Positive() => "positive odd",
//     Parity(Even) & Negative() => "negative even",
//     Parity(Odd) & Negative() => "negative odd",
// }
// ```
//
// Without optimization: Parity(x) is called 4 times, Positive/Negative 4 times
// With optimization: Parity(x) is called once, Positive/Negative called once
//
// This module tracks pattern calls and provides optimization hints to codegen.

/// A unique identifier for an active pattern call site
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActivePatternCallId {
    /// The pattern name
    pub pattern_name: Text,
    /// Hash of the arguments (for value equality)
    pub args_hash: u64,
}

impl ActivePatternCallId {
    /// Create a new call ID
    pub fn new(pattern_name: impl Into<Text>, args_hash: u64) -> Self {
        Self {
            pattern_name: pattern_name.into(),
            args_hash,
        }
    }
}

/// Tracks active pattern calls within a match expression for optimization
#[derive(Debug, Clone, Default)]
pub struct ActivePatternCallTracker {
    /// Map from call ID to list of arm indices using this call
    call_sites: HashMap<ActivePatternCallId, List<usize>>,
    /// Total number of pattern calls
    total_calls: usize,
    /// Number of unique calls (after deduplication)
    unique_calls: usize,
}

impl ActivePatternCallTracker {
    /// Create a new tracker
    pub fn new() -> Self {
        Self {
            call_sites: HashMap::new(),
            total_calls: 0,
            unique_calls: 0,
        }
    }

    /// Register an active pattern call from a match arm
    ///
    /// Returns `true` if this is a duplicate call (optimization opportunity)
    pub fn register_call(&mut self, call_id: ActivePatternCallId, arm_index: usize) -> bool {
        self.total_calls += 1;

        if let Some(arms) = self.call_sites.get_mut(&call_id) {
            arms.push(arm_index);
            true // Duplicate
        } else {
            self.call_sites.insert(call_id, List::from_iter([arm_index]));
            self.unique_calls += 1;
            false // First occurrence
        }
    }

    /// Check if a call has duplicates
    pub fn has_duplicates(&self, call_id: &ActivePatternCallId) -> bool {
        self.call_sites
            .get(call_id)
            .map(|arms| arms.len() > 1)
            .unwrap_or(false)
    }

    /// Get all arms that use a specific pattern call
    pub fn get_arms_for_call(&self, call_id: &ActivePatternCallId) -> Option<&List<usize>> {
        self.call_sites.get(call_id)
    }

    /// Calculate the optimization potential (saved calls)
    pub fn optimization_potential(&self) -> usize {
        self.total_calls.saturating_sub(self.unique_calls)
    }

    /// Get optimization hints for codegen
    pub fn get_optimization_hints(&self) -> ActivePatternOptimizationHints {
        let mut cacheable_patterns = List::new();
        let mut call_counts = HashMap::new();

        for (call_id, arms) in &self.call_sites {
            if arms.len() > 1 {
                cacheable_patterns.push(call_id.clone());
            }
            call_counts.insert(call_id.pattern_name.clone(), arms.len());
        }

        ActivePatternOptimizationHints {
            cacheable_patterns,
            call_counts,
            total_savings: self.optimization_potential(),
        }
    }
}

/// Optimization hints for codegen to use when generating match code
#[derive(Debug, Clone, Default)]
pub struct ActivePatternOptimizationHints {
    /// Pattern calls that should be cached (called more than once)
    pub cacheable_patterns: List<ActivePatternCallId>,
    /// Map from pattern name to total call count
    pub call_counts: HashMap<Text, usize>,
    /// Total number of pattern evaluations saved by caching
    pub total_savings: usize,
}

impl ActivePatternOptimizationHints {
    /// Check if any optimization is possible
    pub fn has_optimizations(&self) -> bool {
        !self.cacheable_patterns.is_empty()
    }

    /// Check if a specific pattern should be cached
    pub fn should_cache(&self, pattern_name: &Text, args_hash: u64) -> bool {
        let call_id = ActivePatternCallId::new(pattern_name.clone(), args_hash);
        self.cacheable_patterns.iter().any(|c| c == &call_id)
    }
}

/// Analyze a match expression for active pattern optimization opportunities
pub fn analyze_match_for_optimization(
    rows: &[PatternRow],
) -> ActivePatternCallTracker {
    let mut tracker = ActivePatternCallTracker::new();

    for (arm_idx, row) in rows.iter().enumerate() {
        // Extract active pattern calls from each column
        for col in row.columns.iter() {
            extract_active_calls(col, arm_idx, &mut tracker);
        }
    }

    tracker
}

/// Recursively extract active pattern calls from a pattern column
fn extract_active_calls(
    col: &PatternColumn,
    arm_idx: usize,
    tracker: &mut ActivePatternCallTracker,
) {
    match col {
        PatternColumn::Active { name, is_total, bindings } => {
            // Create a call ID (using pattern name and a simple hash of position)
            // In a real implementation, this would hash the actual arguments
            let call_id = ActivePatternCallId::new(name.clone(), 0);
            tracker.register_call(call_id, arm_idx);

            // Recurse into bindings
            for binding in bindings.iter() {
                extract_active_calls(binding, arm_idx, tracker);
            }
        }
        PatternColumn::Constructor { args, .. } => {
            for arg in args.iter() {
                extract_active_calls(arg, arm_idx, tracker);
            }
        }
        PatternColumn::Or(alts) => {
            for alt in alts.iter() {
                extract_active_calls(alt, arm_idx, tracker);
            }
        }
        PatternColumn::And(conjs) => {
            for conj in conjs.iter() {
                extract_active_calls(conj, arm_idx, tracker);
            }
        }
        PatternColumn::Guarded(inner) => {
            extract_active_calls(inner, arm_idx, tracker);
        }
        PatternColumn::Tuple(elems) => {
            for elem in elems.iter() {
                extract_active_calls(elem, arm_idx, tracker);
            }
        }
        PatternColumn::Array(elems) => {
            for elem in elems.iter() {
                extract_active_calls(elem, arm_idx, tracker);
            }
        }
        PatternColumn::Record { fields, .. } => {
            for (_, field) in fields.iter() {
                extract_active_calls(field, arm_idx, tracker);
            }
        }
        PatternColumn::Reference { inner, .. } => {
            extract_active_calls(inner, arm_idx, tracker);
        }
        PatternColumn::Stream { head_patterns, tail, .. } => {
            for head in head_patterns.iter() {
                extract_active_calls(head, arm_idx, tracker);
            }
            if let Some(tail_col) = tail {
                extract_active_calls(tail_col, arm_idx, tracker);
            }
        }
        // No active patterns in these
        PatternColumn::Wildcard
        | PatternColumn::Literal(_)
        | PatternColumn::Range { .. }
        | PatternColumn::TypeTest { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    fn make_parity_pattern() -> VariantReturningPattern {
        let mut variants = IndexMap::new();
        variants.insert(Text::from("Even"), Type::Unit);
        variants.insert(Text::from("Odd"), Type::Unit);

        VariantReturningPattern {
            name: Text::from("Parity"),
            return_type: Type::Variant(variants),
            constructors: List::from_iter([
                Constructor::nullary("Even"),
                Constructor::nullary("Odd"),
            ]),
            is_total: true,
        }
    }

    #[test]
    fn test_registry_operations() {
        let mut registry = ActivePatternRegistry::new();
        let parity = make_parity_pattern();

        registry.register(parity.clone());

        assert!(registry.is_variant_returning(&Text::from("Parity")));
        assert!(!registry.is_variant_returning(&Text::from("Unknown")));

        let info = registry.get(&Text::from("Parity"));
        assert!(info.is_some());
        assert_eq!(info.unwrap().constructors.len(), 2);
    }

    #[test]
    fn test_exhaustive_coverage() {
        let env = TypeEnv::new();
        let parity = make_parity_pattern();

        // Both Even and Odd covered
        let covered = vec![Text::from("Even"), Text::from("Odd")];
        let result = check_variant_pattern_exhaustiveness(&parity, &covered, &env);

        assert!(result.is_exhaustive);
        assert!(result.uncovered_witnesses.is_empty());
    }

    #[test]
    fn test_non_exhaustive_missing_odd() {
        let env = TypeEnv::new();
        let parity = make_parity_pattern();

        // Only Even covered
        let covered = vec![Text::from("Even")];
        let result = check_variant_pattern_exhaustiveness(&parity, &covered, &env);

        assert!(!result.is_exhaustive);
        assert_eq!(result.uncovered_witnesses.len(), 1);
    }

    #[test]
    fn test_analyze_return_type_bool() {
        let env = TypeEnv::new();

        // Bool should not be treated as variant-returning
        let result = analyze_return_type(&Type::Bool, &env);
        assert!(result.is_none());
    }

    #[test]
    fn test_analyze_return_type_variant() {
        let env = TypeEnv::new();

        // Inline variant type should be variant-returning
        let mut variants = IndexMap::new();
        variants.insert(Text::from("A"), Type::Unit);
        variants.insert(Text::from("B"), Type::Int);
        let variant = Type::Variant(variants);

        let result = analyze_return_type(&variant, &env);
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 2);
    }

    #[test]
    fn test_call_tracker_basic() {
        let mut tracker = ActivePatternCallTracker::new();

        let call1 = ActivePatternCallId::new("Parity", 0);
        let call2 = ActivePatternCallId::new("Sign", 0);

        // First call to Parity - not a duplicate
        assert!(!tracker.register_call(call1.clone(), 0));
        // Second call to Parity - is a duplicate
        assert!(tracker.register_call(call1.clone(), 1));
        // First call to Sign - not a duplicate
        assert!(!tracker.register_call(call2.clone(), 0));

        assert_eq!(tracker.total_calls, 3);
        assert_eq!(tracker.unique_calls, 2);
        assert_eq!(tracker.optimization_potential(), 1);
    }

    #[test]
    fn test_optimization_hints() {
        let mut tracker = ActivePatternCallTracker::new();

        let call = ActivePatternCallId::new("Parity", 0);

        // Call Parity from 4 arms
        tracker.register_call(call.clone(), 0);
        tracker.register_call(call.clone(), 1);
        tracker.register_call(call.clone(), 2);
        tracker.register_call(call.clone(), 3);

        let hints = tracker.get_optimization_hints();

        assert!(hints.has_optimizations());
        assert!(hints.should_cache(&Text::from("Parity"), 0));
        assert_eq!(hints.total_savings, 3); // 4 calls - 1 unique = 3 saved
    }

    #[test]
    fn test_no_optimization_single_calls() {
        let mut tracker = ActivePatternCallTracker::new();

        // Each pattern called only once
        tracker.register_call(ActivePatternCallId::new("Parity", 0), 0);
        tracker.register_call(ActivePatternCallId::new("Sign", 0), 1);
        tracker.register_call(ActivePatternCallId::new("Magnitude", 0), 2);

        let hints = tracker.get_optimization_hints();

        assert!(!hints.has_optimizations());
        assert_eq!(hints.total_savings, 0);
    }
}
