//! Dependent Pattern Matching Integration
//!
//! This module bridges the exhaustiveness checking module with dependent type
//! pattern matching. It provides index-aware exhaustiveness checking that
//! understands type indices and can identify absurd patterns.
//!
//! # Overview
//!
//! When pattern matching on indexed types (like `List<T, n>` where `n` is a
//! type-level natural), some patterns may be impossible based on index
//! constraints. For example:
//!
//! ```verum
//! fn head<T, n>(xs: List<T, Succ(n): meta Nat>) -> T =
//!     match xs {
//!         Cons(x, _) => x
//!         // No Nil case needed - type ensures non-empty
//!     }
//! ```
//!
//! This module handles:
//! 1. Filtering absurd constructors based on index constraints
//! 2. Integrating with the matrix-based exhaustiveness algorithm
//! 3. Generating appropriate diagnostics for dependent types
//!
//! # References
//!
//! - Dependent pattern matching: patterns that refine types in branches, with coverage checking
//! - Pattern exhaustiveness checking: ensuring match expressions cover all possible values

use super::constructors::{get_type_constructors, Constructor, TypeConstructors};
use super::matrix::{build_matrix, CoverageMatrix, PatternColumn};
use super::witness::{generate_any_witness, Witness};
use super::diagnostics::ExhaustivenessWarning;
use super::{ExhaustivenessResult, find_redundant_patterns};
use crate::context::TypeEnv;
use crate::dependent_match::{ConstructorRefinement, DependentPatternChecker, Motive};
use crate::ty::{EqTerm, InductiveConstructor, Type, TypeVar};
use crate::unify::Unifier;
use crate::TypeError;
use indexmap::IndexMap;
use verum_ast::pattern::{MatchArm, Pattern, PatternKind};
use verum_ast::span::Span;
use verum_common::{List, Map, Set, Text};

/// Configuration for dependent exhaustiveness checking
#[derive(Debug, Clone)]
pub struct DependentExhaustivenessConfig {
    /// Maximum witnesses to generate
    pub max_witnesses: usize,

    /// Whether to check for redundant patterns
    pub check_redundancy: bool,

    /// Whether to warn about all-guarded matches
    pub warn_all_guarded: bool,

    /// Whether to use SMT solver for guard verification
    pub use_smt_for_guards: bool,

    /// Whether to track index refinements
    pub track_index_refinements: bool,
}

impl Default for DependentExhaustivenessConfig {
    fn default() -> Self {
        Self {
            max_witnesses: 3,
            check_redundancy: true,
            warn_all_guarded: true,
            use_smt_for_guards: false,
            track_index_refinements: true,
        }
    }
}

/// Result of dependent exhaustiveness checking
#[derive(Debug, Clone)]
pub struct DependentExhaustivenessResult {
    /// Base exhaustiveness result
    pub base: ExhaustivenessResult,

    /// Patterns that are absurd (impossible due to index constraints)
    pub absurd_patterns: List<usize>,

    /// Index refinements learned from each pattern
    pub index_refinements: List<IndexRefinement>,

    /// The motive (if dependent result type)
    pub motive: Option<Motive>,
}

/// Information about index refinements from a pattern
#[derive(Debug, Clone)]
pub struct IndexRefinement {
    /// Pattern index
    pub pattern_index: usize,

    /// Substitutions: variable name -> refined type
    pub substitutions: IndexMap<Text, Type>,

    /// Whether this pattern is absurd (impossible)
    pub is_absurd: bool,
}

/// Dependent exhaustiveness checker
///
/// Combines the matrix-based exhaustiveness algorithm with dependent type
/// awareness for index-refined pattern matching.
pub struct DependentExhaustivenessChecker<'a> {
    /// Type environment
    env: &'a TypeEnv,

    /// Configuration
    config: DependentExhaustivenessConfig,

    /// Registry of inductive constructors
    inductive_constructors: &'a Map<Text, List<InductiveConstructor>>,
}

impl<'a> DependentExhaustivenessChecker<'a> {
    /// Create a new dependent exhaustiveness checker
    pub fn new(
        env: &'a TypeEnv,
        inductive_constructors: &'a Map<Text, List<InductiveConstructor>>,
    ) -> Self {
        Self {
            env,
            config: DependentExhaustivenessConfig::default(),
            inductive_constructors,
        }
    }

    /// Create with custom configuration
    pub fn with_config(
        env: &'a TypeEnv,
        inductive_constructors: &'a Map<Text, List<InductiveConstructor>>,
        config: DependentExhaustivenessConfig,
    ) -> Self {
        Self {
            env,
            config,
            inductive_constructors,
        }
    }

    /// Check exhaustiveness with dependent type awareness
    ///
    /// This is the main entry point for dependent exhaustiveness checking.
    /// It combines index-aware filtering with the matrix algorithm.
    pub fn check_exhaustiveness(
        &self,
        patterns: &[Pattern],
        scrutinee_ty: &Type,
    ) -> Result<DependentExhaustivenessResult, TypeError> {
        // Step 1: Filter out impossible constructors based on indices
        let possible_constructors = self.filter_possible_constructors(scrutinee_ty)?;

        // Step 2: Identify absurd patterns
        let (valid_patterns, absurd_patterns) =
            self.partition_patterns(patterns, scrutinee_ty, &possible_constructors)?;

        // Step 3: Run matrix-based exhaustiveness on valid patterns
        let base_result = if valid_patterns.is_empty() && possible_constructors.is_empty() {
            // Empty type - exhaustive with no patterns
            ExhaustivenessResult::exhaustive()
        } else if valid_patterns.is_empty() {
            // Non-empty type with no valid patterns
            let witness = generate_any_witness(scrutinee_ty, self.env);
            ExhaustivenessResult::non_exhaustive(List::from_iter([witness]))
        } else {
            // Run standard exhaustiveness check
            let pattern_refs: Vec<&Pattern> = valid_patterns.iter().collect();
            self.check_filtered_exhaustiveness(&pattern_refs, scrutinee_ty, &possible_constructors)?
        };

        // Step 4: Compute index refinements if configured
        let index_refinements = if self.config.track_index_refinements {
            self.compute_index_refinements(patterns, scrutinee_ty)?
        } else {
            List::new()
        };

        Ok(DependentExhaustivenessResult {
            base: base_result,
            absurd_patterns,
            index_refinements,
            motive: None, // Motive is set by caller if needed
        })
    }

    /// Check exhaustiveness for a dependent match expression
    ///
    /// This handles the full dependent match case with motive inference
    /// and branch type refinement.
    pub fn check_dependent_match(
        &self,
        scrutinee_ty: &Type,
        result_ty: &Type,
        arms: &[MatchArm],
    ) -> Result<DependentExhaustivenessResult, TypeError> {
        // Extract patterns from arms
        let patterns: Vec<Pattern> = arms.iter().map(|arm| arm.pattern.clone()).collect();

        // Run exhaustiveness check
        let mut result = self.check_exhaustiveness(&patterns, scrutinee_ty)?;

        // Infer motive
        let motive = self.infer_motive(scrutinee_ty, result_ty);
        result.motive = Some(motive);

        Ok(result)
    }

    /// Filter constructors to only those possible given the scrutinee type's indices
    fn filter_possible_constructors(
        &self,
        scrutinee_ty: &Type,
    ) -> Result<List<Constructor>, TypeError> {
        let all_constructors = get_type_constructors(scrutinee_ty, self.env);
        let mut possible = List::new();

        for ctor in all_constructors.iter() {
            if self.is_constructor_possible(ctor, scrutinee_ty)? {
                possible.push(ctor.clone());
            }
        }

        Ok(possible)
    }

    /// Check if a constructor is possible given the scrutinee type's indices
    fn is_constructor_possible(
        &self,
        ctor: &Constructor,
        scrutinee_ty: &Type,
    ) -> Result<bool, TypeError> {
        // Extract indices from scrutinee type
        let scrutinee_indices = match scrutinee_ty {
            Type::Generic { args, .. } | Type::Named { args, .. } => args,
            _ => return Ok(true), // Non-indexed type
        };

        if scrutinee_indices.is_empty() {
            return Ok(true); // No indices to constrain
        }

        // Look up constructor return type indices
        let ctor_indices = self.get_constructor_indices(&ctor.name)?;

        // Check compatibility
        for (scr_idx, ctor_idx) in scrutinee_indices.iter().zip(ctor_indices.iter()) {
            if self.are_indices_incompatible(scr_idx, ctor_idx) {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Get constructor indices from the registry
    fn get_constructor_indices(&self, ctor_name: &Text) -> Result<List<Type>, TypeError> {
        // Search for constructor in all types
        for (_type_name, constructors) in self.inductive_constructors.iter() {
            for ctor in constructors {
                if &ctor.name == ctor_name {
                    // Extract indices from return type
                    return Ok(match ctor.return_type.as_ref() {
                        Type::Generic { args, .. } | Type::Named { args, .. } => args.clone(),
                        _ => List::new(),
                    });
                }
            }
        }

        Ok(List::new())
    }

    /// Check if two type indices are incompatible
    fn are_indices_incompatible(&self, idx1: &Type, idx2: &Type) -> bool {
        match (idx1, idx2) {
            // Zero vs Succ - incompatible
            (Type::Generic { name: n1, .. }, Type::Generic { name: n2, .. }) => {
                (n1.as_str() == "Zero" && n2.as_str() == "Succ")
                    || (n1.as_str() == "Succ" && n2.as_str() == "Zero")
            }
            // Different meta values - incompatible
            (Type::Meta { name: n1, .. }, Type::Meta { name: n2, .. }) => {
                // Only incompatible if both are concrete values
                let n1_concrete = n1.as_str().chars().all(|c| c.is_numeric());
                let n2_concrete = n2.as_str().chars().all(|c| c.is_numeric());
                n1_concrete && n2_concrete && n1 != n2
            }
            _ => false,
        }
    }

    /// Partition patterns into valid and absurd
    fn partition_patterns(
        &self,
        patterns: &[Pattern],
        scrutinee_ty: &Type,
        possible_constructors: &[Constructor],
    ) -> Result<(Vec<Pattern>, List<usize>), TypeError> {
        let mut valid = Vec::new();
        let mut absurd = List::new();

        for (i, pattern) in patterns.iter().enumerate() {
            if self.is_pattern_absurd(pattern, scrutinee_ty, possible_constructors)? {
                absurd.push(i);
            } else {
                valid.push(pattern.clone());
            }
        }

        Ok((valid, absurd))
    }

    /// Check if a pattern is absurd (impossible due to index constraints)
    fn is_pattern_absurd(
        &self,
        pattern: &Pattern,
        scrutinee_ty: &Type,
        possible_constructors: &[Constructor],
    ) -> Result<bool, TypeError> {
        match &pattern.kind {
            PatternKind::Variant { path, .. } => {
                // Get constructor name
                if let Some(segment) = path.segments.last() {
                    let ctor_name = match segment {
                        verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                        _ => return Ok(false),
                    };

                    // Check if this constructor is in the possible set
                    let is_possible = possible_constructors
                        .iter()
                        .any(|c| c.name.as_str() == ctor_name);

                    Ok(!is_possible)
                } else {
                    Ok(false)
                }
            }
            PatternKind::Wildcard | PatternKind::Ident { .. } => {
                // Wildcards are never absurd
                Ok(false)
            }
            PatternKind::Or(alternatives) => {
                // Or is absurd only if ALL alternatives are absurd
                for alt in alternatives {
                    if !self.is_pattern_absurd(alt, scrutinee_ty, possible_constructors)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            PatternKind::And(conjuncts) => {
                // And is absurd if ANY conjunct is absurd
                for conj in conjuncts {
                    if self.is_pattern_absurd(conj, scrutinee_ty, possible_constructors)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    /// Check exhaustiveness with filtered constructors
    fn check_filtered_exhaustiveness(
        &self,
        patterns: &[&Pattern],
        scrutinee_ty: &Type,
        possible_constructors: &[Constructor],
    ) -> Result<ExhaustivenessResult, TypeError> {
        // Convert to owned patterns for matrix building
        let owned_patterns: Vec<Pattern> = patterns.iter().map(|p| (*p).clone()).collect();

        // Build matrix
        let matrix = build_matrix(&owned_patterns, scrutinee_ty, self.env)?;

        // Check redundancy
        let redundant = if self.config.check_redundancy {
            find_redundant_patterns(&matrix)
        } else {
            List::new()
        };

        // Check if all patterns are guarded
        let all_guarded = matrix.rows.iter().all(|row| row.has_guard);

        // Find uncovered cases among possible constructors
        let uncovered = self.find_uncovered_in_possible(&matrix, possible_constructors)?;

        // `warn_all_guarded` gate: when true (default) and every
        // pattern in the match carries a guard, surface a typed
        // `AllGuarded` warning so callers know that an unsatisfied
        // guard set could leave the match unmatched. Before this
        // wire-up the field was inert — the warning was never
        // emitted regardless of the flag.
        let mut warnings: List<ExhaustivenessWarning> = List::new();
        if all_guarded
            && !matrix.rows.is_empty()
            && self.config.warn_all_guarded
        {
            warnings.push(ExhaustivenessWarning::all_guarded(None));
        }

        Ok(ExhaustivenessResult {
            is_exhaustive: uncovered.is_empty(),
            uncovered_witnesses: uncovered,
            redundant_patterns: redundant,
            all_guarded,
            range_overlaps: None,
            warnings,
        })
    }

    /// Whether SMT-backed guard verification is enabled for this
    /// dependent exhaustiveness pass. Mirrors
    /// `DependentExhaustivenessConfig.use_smt_for_guards`.
    /// Surfaced so downstream orchestrators that wrap this checker
    /// can decide whether to feed guarded patterns to an
    /// `SmtGuardVerifier` before declaring the match
    /// exhaustive — without re-reading the config struct.
    /// Before this accessor existed the field was inert from the
    /// orchestrator's perspective.
    #[must_use]
    pub fn use_smt_for_guards_enabled(&self) -> bool {
        self.config.use_smt_for_guards
    }

    /// Whether the all-guarded diagnostic is emitted. Mirrors
    /// `DependentExhaustivenessConfig.warn_all_guarded`.
    #[must_use]
    pub fn warn_all_guarded_enabled(&self) -> bool {
        self.config.warn_all_guarded
    }

    /// Find uncovered cases among the possible constructors
    fn find_uncovered_in_possible(
        &self,
        matrix: &CoverageMatrix,
        possible_constructors: &[Constructor],
    ) -> Result<List<Witness>, TypeError> {
        let mut uncovered = List::new();

        for ctor in possible_constructors {
            if !self.is_constructor_covered_by_matrix(matrix, ctor) {
                let witness = super::witness::generate_witness_for_constructor(ctor, matrix, self.env);
                uncovered.push(witness);

                if uncovered.len() >= self.config.max_witnesses {
                    break;
                }
            }
        }

        Ok(uncovered)
    }

    /// Check if a constructor is covered by the matrix
    fn is_constructor_covered_by_matrix(
        &self,
        matrix: &CoverageMatrix,
        ctor: &Constructor,
    ) -> bool {
        // Check if any row covers this constructor
        for row in matrix.rows.iter() {
            if let Some(first) = row.columns.first() {
                if self.column_covers_constructor(first, ctor) && !row.has_guard {
                    return true;
                }
            }
        }

        // Check for wildcard coverage
        matrix.has_wildcard_row()
    }

    /// Check if a pattern column covers a constructor
    fn column_covers_constructor(&self, col: &PatternColumn, ctor: &Constructor) -> bool {
        match col {
            PatternColumn::Wildcard => true,
            PatternColumn::Constructor { name, .. } => name.as_str() == ctor.name.as_str(),
            PatternColumn::Or(alts) => alts
                .iter()
                .any(|alt| self.column_covers_constructor(alt, ctor)),
            PatternColumn::Guarded(inner) => self.column_covers_constructor(inner, ctor),
            _ => false,
        }
    }

    /// Compute index refinements for each pattern
    fn compute_index_refinements(
        &self,
        patterns: &[Pattern],
        scrutinee_ty: &Type,
    ) -> Result<List<IndexRefinement>, TypeError> {
        let mut refinements = List::new();

        for (i, pattern) in patterns.iter().enumerate() {
            let refinement = self.compute_pattern_refinement(pattern, scrutinee_ty, i)?;
            refinements.push(refinement);
        }

        Ok(refinements)
    }

    /// Compute index refinement for a single pattern
    fn compute_pattern_refinement(
        &self,
        pattern: &Pattern,
        scrutinee_ty: &Type,
        pattern_index: usize,
    ) -> Result<IndexRefinement, TypeError> {
        let mut substitutions = IndexMap::new();
        let mut is_absurd = false;

        match &pattern.kind {
            PatternKind::Variant { path, .. } => {
                // Get constructor name
                if let Some(segment) = path.segments.last() {
                    if let verum_ast::ty::PathSegment::Name(id) = segment {
                        let ctor_name = Text::from(id.name.as_str());

                        // Look up constructor
                        if let Some(ctor_info) = self.find_constructor(&ctor_name) {
                            // Compute substitutions from matching return type with scrutinee
                            self.compute_substitutions(
                                scrutinee_ty,
                                &ctor_info.return_type,
                                &mut substitutions,
                            );

                            // Check if this matching is absurd
                            is_absurd = self.is_matching_absurd(scrutinee_ty, &ctor_info);
                        }
                    }
                }
            }
            _ => {}
        }

        Ok(IndexRefinement {
            pattern_index,
            substitutions,
            is_absurd,
        })
    }

    /// Find a constructor by name
    fn find_constructor(&self, name: &Text) -> Option<InductiveConstructor> {
        for (_type_name, constructors) in self.inductive_constructors.iter() {
            for ctor in constructors {
                if &ctor.name == name {
                    return Some(ctor.clone());
                }
            }
        }
        None
    }

    /// Compute type substitutions from matching two types
    fn compute_substitutions(
        &self,
        scrutinee_ty: &Type,
        ctor_return_ty: &Type,
        out: &mut IndexMap<Text, Type>,
    ) {
        // Extract type arguments
        let (scr_args, ctor_args) = match (scrutinee_ty, ctor_return_ty) {
            (Type::Generic { args: a1, .. }, Type::Generic { args: a2, .. }) => (a1, a2),
            (Type::Named { args: a1, .. }, Type::Named { args: a2, .. }) => (a1, a2),
            (Type::Generic { args: a1, .. }, Type::Named { args: a2, .. }) => (a1, a2),
            (Type::Named { args: a1, .. }, Type::Generic { args: a2, .. }) => (a1, a2),
            _ => return,
        };

        // Match up arguments
        for (scr_arg, ctor_arg) in scr_args.iter().zip(ctor_args.iter()) {
            // If scrutinee argument is a variable, record substitution
            if let Type::Generic { name, args } = scr_arg {
                if args.is_empty() && name.as_str().len() == 1 {
                    out.insert(name.clone(), ctor_arg.clone());
                }
            }
            if let Type::Meta { name, .. } = scr_arg {
                out.insert(name.clone(), ctor_arg.clone());
            }
        }
    }

    /// Check if matching a constructor against scrutinee is absurd
    fn is_matching_absurd(&self, scrutinee_ty: &Type, ctor: &InductiveConstructor) -> bool {
        let (scr_args, ctor_args) = match (scrutinee_ty, ctor.return_type.as_ref()) {
            (Type::Generic { args: a1, .. }, Type::Generic { args: a2, .. }) => (a1, a2),
            (Type::Named { args: a1, .. }, Type::Named { args: a2, .. }) => (a1, a2),
            _ => return false,
        };

        for (scr_arg, ctor_arg) in scr_args.iter().zip(ctor_args.iter()) {
            if self.are_indices_incompatible(scr_arg, ctor_arg) {
                return true;
            }
        }

        false
    }

    /// Infer motive from scrutinee and result types
    fn infer_motive(&self, scrutinee_ty: &Type, result_ty: &Type) -> Motive {
        Motive::simple(
            Text::from("scrutinee"),
            scrutinee_ty.clone(),
            result_ty.clone(),
        )
    }
}

/// Unified exhaustiveness check that handles both dependent and non-dependent cases
///
/// This is the recommended entry point for exhaustiveness checking in Verum.
/// It automatically detects whether dependent type features are needed and
/// uses the appropriate algorithm.
pub fn check_exhaustiveness_unified(
    patterns: &[Pattern],
    scrutinee_ty: &Type,
    env: &TypeEnv,
    inductive_constructors: &Map<Text, List<InductiveConstructor>>,
) -> Result<DependentExhaustivenessResult, TypeError> {
    let checker = DependentExhaustivenessChecker::new(env, inductive_constructors);
    checker.check_exhaustiveness(patterns, scrutinee_ty)
}

/// Check a dependent match expression
///
/// This handles the full dependent match case including motive inference
/// and index refinement.
pub fn check_dependent_match_unified(
    scrutinee_ty: &Type,
    result_ty: &Type,
    arms: &[MatchArm],
    env: &TypeEnv,
    inductive_constructors: &Map<Text, List<InductiveConstructor>>,
) -> Result<DependentExhaustivenessResult, TypeError> {
    let checker = DependentExhaustivenessChecker::new(env, inductive_constructors);
    checker.check_dependent_match(scrutinee_ty, result_ty, arms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_incompatibility() {
        let env = TypeEnv::new();
        let ctors = Map::new();
        let checker = DependentExhaustivenessChecker::new(
            &env,
            &ctors,
        );

        // Zero vs Succ should be incompatible
        let zero = Type::Generic {
            name: Text::from("Zero"),
            args: List::new(),
        };
        let succ = Type::Generic {
            name: Text::from("Succ"),
            args: List::from_iter([Type::Var(TypeVar::fresh())]),
        };

        assert!(checker.are_indices_incompatible(&zero, &succ));
        assert!(checker.are_indices_incompatible(&succ, &zero));

        // Same should not be incompatible
        assert!(!checker.are_indices_incompatible(&zero, &zero));
    }

    #[test]
    fn config_accessors_mirror_construction_values() {
        // Pin: `warn_all_guarded` and `use_smt_for_guards` reach
        // the checker via accessors. Before the wire-up the two
        // fields had no public read surface — orchestrators that
        // wanted to drive SMT-backed guard verification or
        // suppress the all-guarded diagnostic had no way to
        // observe the configured stance.
        let env = TypeEnv::new();
        let ctors = Map::new();

        for &warn in &[true, false] {
            for &smt in &[true, false] {
                let cfg = DependentExhaustivenessConfig {
                    warn_all_guarded: warn,
                    use_smt_for_guards: smt,
                    ..DependentExhaustivenessConfig::default()
                };
                let checker = DependentExhaustivenessChecker::with_config(
                    &env,
                    &ctors,
                    cfg,
                );
                assert_eq!(checker.warn_all_guarded_enabled(), warn);
                assert_eq!(checker.use_smt_for_guards_enabled(), smt);
            }
        }
    }
}

// =============================================================================
// HOU strategy.
// =============================================================================

/// higher-order unification strategy
/// for dependent pattern-match coverage checking.
///
/// Q#11 picks **`MillerPatternFragment`** as the
/// production default. Rationale:
///
/// * **Decidable** — Miller's pattern fragment is decidable by
///   first-order unification on the linear-pattern subset.
///   Termination is guaranteed; the checker never diverges.
/// * **Sufficient in practice** — Coq, Lean, and Agda all use
///   Miller-pattern style for their dependent-pattern coverage.
///   Real-world index-dependent patterns (length-indexed lists,
///   dimension-indexed vectors, depth-indexed trees) fall in
///   the pattern fragment.
/// * **Aligned with VVA philosophy** — `Zero-Cost Abstractions`
///   and `No Magic`: an undecidable HOU would surprise users
///   with non-termination on innocent-looking patterns.
/// * **Forward-compat path** — `RestrictedHigherOrderMatching`
///   reserved for future extensions that need patterns slightly
///   outside Miller's fragment (linearity-violating but
///   structurally-decidable). `FullHigherOrderUnification` is
///   exposed for explicit opt-in only — turning it on means the
///   checker may diverge on adversarial patterns and the kernel
///   refuses to admit such programs without `@verify(thorough)`
///   or higher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HouStrategy {
    /// Decidable: Miller's pattern fragment. Production default.
    /// First-order unification on the linear-pattern subset.
    MillerPatternFragment,
    /// Decidable but more permissive: structurally-decidable
    /// matching beyond strict Miller patterns. Reserved for
    /// future extensions; not yet used by the V2 K-Elim per-case
    /// typing pass.
    RestrictedHigherOrderMatching,
    /// **Undecidable**: full HOU (Huet's algorithm). Opt-in only;
    /// the kernel refuses to admit programs requiring this
    /// strategy without `@verify(thorough)` or higher.
    FullHigherOrderUnification,
}

impl HouStrategy {
    /// Production default Q#11.
    pub const DEFAULT: HouStrategy = HouStrategy::MillerPatternFragment;

    /// `true` when this strategy is guaranteed to terminate. The
    /// V2 K-Elim per-case typing pass refuses to admit user
    /// programs whose dependent-pattern coverage requires a
    /// non-terminating strategy without an explicit
    /// `@verify(thorough)` or stronger annotation.
    pub fn is_decidable(&self) -> bool {
        match self {
            Self::MillerPatternFragment | Self::RestrictedHigherOrderMatching => true,
            Self::FullHigherOrderUnification => false,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MillerPatternFragment => "miller-pattern",
            Self::RestrictedHigherOrderMatching => "restricted-higher-order-matching",
            Self::FullHigherOrderUnification => "full-hou",
        }
    }
}

impl Default for HouStrategy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl std::fmt::Display for HouStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod hou_strategy_tests {
    use super::HouStrategy;

    #[test]
    fn default_is_miller_pattern_fragment() {
        assert_eq!(HouStrategy::default(), HouStrategy::MillerPatternFragment);
        assert_eq!(HouStrategy::DEFAULT, HouStrategy::MillerPatternFragment);
    }

    #[test]
    fn miller_pattern_is_decidable() {
        assert!(HouStrategy::MillerPatternFragment.is_decidable());
    }

    #[test]
    fn restricted_higher_order_matching_is_decidable() {
        assert!(HouStrategy::RestrictedHigherOrderMatching.is_decidable());
    }

    #[test]
    fn full_hou_is_undecidable() {
        assert!(!HouStrategy::FullHigherOrderUnification.is_decidable());
    }

    #[test]
    fn display_round_trips_canonical_names() {
        assert_eq!(format!("{}", HouStrategy::MillerPatternFragment), "miller-pattern");
        assert_eq!(
            format!("{}", HouStrategy::RestrictedHigherOrderMatching),
            "restricted-higher-order-matching"
        );
        assert_eq!(format!("{}", HouStrategy::FullHigherOrderUnification), "full-hou");
    }
}
