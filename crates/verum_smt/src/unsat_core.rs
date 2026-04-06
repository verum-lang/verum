//! Unsat Core Extraction Module
//!
//! This module provides functionality for extracting minimal unsatisfiable subsets
//! from unsatisfiable constraint sets, crucial for debugging and error reporting.
//!
//! Based on experiments/z3.rs documentation
//! When refinement type verification fails (e.g., cannot prove `Int{> 0}` for a value),
//! unsat core extraction identifies the minimal set of conflicting constraints. This
//! enables precise error messages showing which refinement predicates conflict and why
//! a type constraint cannot be satisfied.

use std::fmt;
use std::time::Instant;

use z3::ast::Ast;
use z3::{SatResult, Solver, ast::Bool};

use verum_common::{List, Map, Maybe, Set, Text};

// ==================== Core Types ====================

/// Tracked assertion with metadata
#[derive(Debug, Clone)]
pub struct TrackedAssertion {
    /// Unique identifier
    pub id: Text,
    /// The assertion itself
    pub assertion: Bool,
    /// Source location (file:line)
    pub source: Maybe<Text>,
    /// Category (e.g., "precondition", "refinement", "invariant")
    pub category: AssertionCategory,
    /// Optional description
    pub description: Maybe<Text>,
}

/// Assertion categories for better organization
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AssertionCategory {
    /// Precondition assertion
    Precondition,
    /// Postcondition assertion
    Postcondition,
    /// Refinement type constraint
    Refinement,
    /// Loop invariant
    Invariant,
    /// User assertion
    UserAssertion,
    /// Generated constraint
    Generated,
    /// Custom category
    Custom(Text),
}

impl fmt::Display for AssertionCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Precondition => write!(f, "precondition"),
            Self::Postcondition => write!(f, "postcondition"),
            Self::Refinement => write!(f, "refinement"),
            Self::Invariant => write!(f, "invariant"),
            Self::UserAssertion => write!(f, "assertion"),
            Self::Generated => write!(f, "generated"),
            Self::Custom(name) => write!(f, "{}", name),
        }
    }
}

/// Unsat core result
#[derive(Debug, Clone)]
pub struct UnsatCore {
    /// Core assertions (minimal unsatisfiable subset)
    pub core: List<TrackedAssertion>,
    /// Original assertions count
    pub total_assertions: usize,
    /// Reduction percentage
    pub reduction_percent: f64,
    /// Core computation time
    pub time_ms: u64,
    /// Whether core is minimal
    pub is_minimal: bool,
}

impl UnsatCore {
    /// Get human-readable explanation
    pub fn explain(&self) -> Text {
        let mut explanation = Text::from("Unsatisfiable core found:\n");

        // Group by category
        let mut by_category: Map<AssertionCategory, List<&TrackedAssertion>> = Map::new();
        for assertion in &self.core {
            by_category
                .entry(assertion.category.clone())
                .or_default()
                .push(assertion);
        }

        // Format each category
        for (category, assertions) in by_category {
            explanation.push_str(&format!("\n{}:\n", category));
            for assertion in assertions {
                let desc = match &assertion.description {
                    Maybe::Some(d) => d.as_str(),
                    Maybe::None => "",
                };
                explanation.push_str(&format!("  - {} {}\n", assertion.id, desc));
                if let Maybe::Some(source) = &assertion.source {
                    explanation.push_str(&format!("    at {}\n", source));
                }
            }
        }

        explanation.push_str(&format!(
            "\nReduced from {} to {} assertions ({:.1}% reduction)\n",
            self.total_assertions,
            self.core.len(),
            self.reduction_percent
        ));

        explanation
    }
}

/// Configuration for unsat core extraction
#[derive(Debug, Clone)]
pub struct UnsatCoreConfig {
    /// Enable core minimization (more expensive)
    pub minimize: bool,
    /// Use quick extraction (less minimal but faster)
    pub quick_extraction: bool,
    /// Maximum iterations for minimization
    pub max_iterations: usize,
    /// Timeout for core extraction
    pub timeout_ms: Maybe<u64>,
    /// Enable proof-based extraction
    pub proof_based: bool,
}

impl Default for UnsatCoreConfig {
    fn default() -> Self {
        Self {
            minimize: true,
            quick_extraction: false,
            max_iterations: 100,
            timeout_ms: Maybe::Some(10000),
            proof_based: false,
        }
    }
}

// ==================== Core Extractor ====================

/// Unsat core extractor
///
/// Note: In z3 0.19.4, Context is thread-local and doesn't need to be stored.
pub struct UnsatCoreExtractor {
    /// Configuration
    config: UnsatCoreConfig,
    /// Tracked assertions
    assertions: List<TrackedAssertion>,
    /// Assertion tracking map
    tracking: Map<Text, Bool>,
}

impl UnsatCoreExtractor {
    /// Create new core extractor
    pub fn new(config: UnsatCoreConfig) -> Self {
        Self {
            config,
            assertions: List::new(),
            tracking: Map::new(),
        }
    }

    /// Track an assertion
    pub fn track(&mut self, assertion: TrackedAssertion) {
        // Create tracking literal using thread-local context
        let track_lit = Bool::new_const(assertion.id.as_str());
        self.tracking.insert(assertion.id.clone(), track_lit);
        self.assertions.push(assertion);
    }

    /// Extract unsat core
    pub fn extract_core(&mut self) -> Result<UnsatCore, Text> {
        let start = Instant::now();

        // Create solver with unsat core tracking
        let (solver, status) = self.create_tracked_solver()?;

        // Check satisfiability
        if status != SatResult::Unsat {
            return Err(Text::from("Formula is not unsatisfiable"));
        }

        // Extract core
        let core_ids = if self.config.proof_based {
            self.extract_proof_based_core(&solver)?
        } else {
            self.extract_assumption_based_core(&solver)?
        };

        // Minimize if requested
        let final_core = if self.config.minimize && !self.config.quick_extraction {
            self.minimize_core(core_ids)?
        } else {
            core_ids
        };

        // Build result
        let core_assertions = self.get_assertions_by_ids(&final_core);
        let time_ms = start.elapsed().as_millis() as u64;

        Ok(UnsatCore {
            core: core_assertions,
            total_assertions: self.assertions.len(),
            reduction_percent: (1.0 - (final_core.len() as f64 / self.assertions.len() as f64))
                * 100.0,
            time_ms,
            is_minimal: self.config.minimize,
        })
    }

    /// Create solver with tracked assertions and check with assumptions
    fn create_tracked_solver(&self) -> std::result::Result<(Solver, SatResult), Text> {
        let solver = Solver::new();

        // Enable unsat core generation
        let mut params = z3::Params::new();
        params.set_bool("unsat_core", true);
        solver.set_params(&params);

        // Add tracked assertions
        for assertion in &self.assertions {
            let track_lit = match self.tracking.get(&assertion.id) {
                Maybe::Some(lit) => lit,
                Maybe::None => return Err(Text::from("Missing tracking literal")),
            };

            // Assert: track_lit => assertion
            solver.assert(track_lit.implies(&assertion.assertion));
        }

        // Check with all tracking literals as assumptions
        let assumptions: List<Bool> = self.tracking.values().cloned().collect();
        let status = solver.check_assumptions(&assumptions);

        Ok((solver, status))
    }

    /// Extract core using assumption tracking
    fn extract_assumption_based_core(
        &self,
        solver: &Solver,
    ) -> std::result::Result<Set<Text>, Text> {
        let unsat_core = solver.get_unsat_core();
        let mut core_ids = Set::new();

        for ast in unsat_core {
            // Find which tracking literal this is by string comparison
            // (since Z3 ASTs can't be directly compared for equality in safe code)
            let ast_str = format!("{}", ast);
            for (id, track_lit) in &self.tracking {
                let track_str = format!("{}", track_lit);
                if ast_str == track_str {
                    core_ids.insert(id.clone());
                    break;
                }
            }
        }

        Ok(core_ids)
    }

    /// Extract core using proof
    fn extract_proof_based_core(&self, solver: &Solver) -> std::result::Result<Set<Text>, Text> {
        // This would use Z3's proof API to extract a more precise core
        // For now, fall back to assumption-based
        self.extract_assumption_based_core(solver)
    }

    /// Minimize unsat core using deletion-based minimization
    fn minimize_core(&self, initial_core: Set<Text>) -> std::result::Result<Set<Text>, Text> {
        let mut core = initial_core.clone();
        let mut changed = true;
        let mut iteration = 0;

        while changed && iteration < self.config.max_iterations {
            changed = false;
            iteration += 1;

            // Try removing each assertion from the core
            let current_core = core.clone();
            for id in current_core.iter() {
                let mut test_core = core.clone();
                test_core.remove(id);

                if self.is_unsat_subset(&test_core)? {
                    core = test_core;
                    changed = true;
                }
            }
        }

        Ok(core)
    }

    /// Check if a subset is unsatisfiable
    fn is_unsat_subset(&self, subset: &Set<Text>) -> std::result::Result<bool, Text> {
        let solver = Solver::new();

        for assertion in &self.assertions {
            if subset.contains(&assertion.id) {
                solver.assert(&assertion.assertion);
            }
        }

        Ok(solver.check() == SatResult::Unsat)
    }

    /// Get assertions by their IDs
    fn get_assertions_by_ids(&self, ids: &Set<Text>) -> List<TrackedAssertion> {
        self.assertions
            .iter()
            .filter(|a| ids.contains(&a.id))
            .cloned()
            .collect()
    }
}

// ==================== Core Analyzer ====================

/// Analyzes unsat cores to identify patterns
pub struct UnsatCoreAnalyzer {
    /// Historical cores for pattern detection
    history: List<UnsatCore>,
}

impl Default for UnsatCoreAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl UnsatCoreAnalyzer {
    pub fn new() -> Self {
        Self {
            history: List::new(),
        }
    }

    /// Analyze a core and add to history
    pub fn analyze(&mut self, core: UnsatCore) -> CoreAnalysis {
        // Find common assertions across cores
        let common = self.find_common_assertions(&core);

        // Identify problematic categories
        let problematic_categories = self.find_problematic_categories(&core);

        // Generate suggestion before moving values
        let suggestion = self.generate_suggestion(&common, &problematic_categories);

        // Add to history
        self.history.push(core);

        CoreAnalysis {
            common_assertions: common,
            problematic_categories,
            suggestion,
        }
    }

    /// Find assertions common to multiple cores
    fn find_common_assertions(&self, current: &UnsatCore) -> Set<Text> {
        let mut common = Set::new();

        if self.history.is_empty() {
            return common;
        }

        // Count occurrences across history
        let mut counts: Map<Text, usize> = Map::new();
        for core in &self.history {
            for assertion in &core.core {
                *counts.entry(assertion.id.clone()).or_insert(0) += 1;
            }
        }

        // Find assertions in >50% of cores
        let threshold = self.history.len() / 2;
        for (id, count) in counts {
            if count > threshold {
                common.insert(id);
            }
        }

        common
    }

    /// Find categories that frequently appear in cores
    fn find_problematic_categories(&self, core: &UnsatCore) -> Map<AssertionCategory, usize> {
        let mut categories = Map::new();

        for assertion in &core.core {
            *categories.entry(assertion.category.clone()).or_insert(0) += 1;
        }

        categories
    }

    /// Generate debugging suggestion
    fn generate_suggestion(
        &self,
        common: &Set<Text>,
        categories: &Map<AssertionCategory, usize>,
    ) -> Text {
        let mut suggestion = Text::from("Debugging suggestions:\n");

        if !common.is_empty() {
            suggestion.push_str("Common problematic assertions:\n");
            for id in common.iter() {
                suggestion.push_str(&format!("  - {}\n", id));
            }
        }

        // Find most problematic category
        if let Some((category, count)) = categories.iter().max_by_key(|(_, c)| *c) {
            suggestion.push_str(&format!(
                "\nMost issues in category '{}' ({} assertions)\n",
                category, count
            ));

            match category {
                AssertionCategory::Refinement => {
                    suggestion.push_str("Consider weakening refinement constraints\n");
                }
                AssertionCategory::Precondition => {
                    suggestion.push_str("Check for contradictory preconditions\n");
                }
                AssertionCategory::Invariant => {
                    suggestion.push_str("Review loop invariants for consistency\n");
                }
                _ => {}
            }
        }

        suggestion
    }
}

/// Core analysis result
#[derive(Debug)]
pub struct CoreAnalysis {
    /// Assertions appearing in multiple cores
    pub common_assertions: Set<Text>,
    /// Categories with issue counts
    pub problematic_categories: Map<AssertionCategory, usize>,
    /// Debugging suggestion
    pub suggestion: Text,
}

// ==================== Core Minimizer ====================

/// Advanced core minimization strategies
///
/// Note: In z3 0.19.4, Context is thread-local and doesn't need to be stored.
pub struct CoreMinimizer;

impl Default for CoreMinimizer {
    fn default() -> Self {
        Self::new()
    }
}

impl CoreMinimizer {
    pub fn new() -> Self {
        Self
    }

    /// Binary search minimization
    pub fn binary_minimize(
        &self,
        assertions: &List<TrackedAssertion>,
    ) -> Result<List<TrackedAssertion>, Text> {
        if assertions.len() <= 1 {
            return Ok(assertions.clone());
        }

        // Split assertions
        let mid = assertions.len() / 2;
        let mut left = List::new();
        let mut right = List::new();

        for (i, assertion) in assertions.iter().enumerate() {
            if i < mid {
                left.push(assertion.clone());
            } else {
                right.push(assertion.clone());
            }
        }

        // Check if left half is unsat
        if self.is_unsat(&left)? {
            return self.binary_minimize(&left);
        }

        // Check if right half is unsat
        if self.is_unsat(&right)? {
            return self.binary_minimize(&right);
        }

        // Need assertions from both halves
        let mut result = self.binary_minimize(&left)?;
        result.extend(self.binary_minimize(&right)?);
        Ok(result)
    }

    /// Check if assertion set is unsatisfiable
    fn is_unsat(&self, assertions: &List<TrackedAssertion>) -> Result<bool, Text> {
        let solver = Solver::new();
        for assertion in assertions {
            solver.assert(&assertion.assertion);
        }
        Ok(solver.check() == SatResult::Unsat)
    }

    /// QuickXplain algorithm for minimal core
    pub fn quickxplain(
        &self,
        assertions: &List<TrackedAssertion>,
    ) -> Result<List<TrackedAssertion>, Text> {
        if assertions.is_empty() || !self.is_unsat(assertions)? {
            return Ok(List::new());
        }

        self.quickxplain_recursive(&List::new(), assertions.clone(), assertions.clone())
    }

    fn quickxplain_recursive(
        &self,
        delta: &List<TrackedAssertion>,
        c: List<TrackedAssertion>,
        r: List<TrackedAssertion>,
    ) -> Result<List<TrackedAssertion>, Text> {
        if !delta.is_empty() && self.is_unsat(delta)? {
            return Ok(List::new());
        }

        if c.len() == 1 {
            return Ok(c);
        }

        // Split c into two parts
        let k = c.len() / 2;
        let mut c1 = List::new();
        let mut c2 = List::new();

        for (i, assertion) in c.iter().enumerate() {
            if i < k {
                c1.push(assertion.clone());
            } else {
                c2.push(assertion.clone());
            }
        }

        // Recursive calls
        let mut delta2 = delta.clone();
        for item in c2.iter() {
            delta2.push(item.clone());
        }
        let d1 = self.quickxplain_recursive(&delta2, c1, r.clone())?;

        let mut delta1 = delta.clone();
        for item in d1.iter() {
            delta1.push(item.clone());
        }
        let d2 = self.quickxplain_recursive(&delta1, c2, r)?;

        let mut result = d1;
        result.extend(d2);
        Ok(result)
    }
}

// ==================== Unsat Core Simplification ====================

/// Simplifies unsat cores to make them more understandable
///
/// After extracting an unsat core, we can simplify it to make error messages clearer:
/// - Replace complex assertions with simpler equivalents
/// - Merge related assertions
/// - Identify root causes
/// - Generate human-readable explanations
///
/// Simplifies unsat cores for human-readable verification error messages.
/// Identifies root cause constraints, merges related assertions, and generates
/// explanations showing why a refinement predicate or contract could not be proven.
pub struct UnsatCoreSimplifier;

impl Default for UnsatCoreSimplifier {
    fn default() -> Self {
        Self::new()
    }
}

impl UnsatCoreSimplifier {
    pub fn new() -> Self {
        Self
    }

    /// Simplify an unsat core for better readability
    ///
    /// This applies various simplification strategies:
    /// 1. Formula simplification (remove redundant terms)
    /// 2. Grouping related assertions
    /// 3. Identifying minimal conflict sets
    /// 4. Generating explanations
    pub fn simplify_core(&self, core: &UnsatCore) -> SimplifiedCore {
        // Simplify individual assertions
        let simplified_assertions = self.simplify_assertions(&core.core);

        // Group related assertions
        let groups = self.group_assertions(&simplified_assertions);

        // Identify root causes
        let root_causes = self.identify_root_causes(&simplified_assertions);

        // Generate explanation
        let explanation = self.generate_explanation(&simplified_assertions, &groups);

        SimplifiedCore {
            original: (*core).clone(),
            simplified_assertions,
            groups,
            root_causes,
            explanation,
        }
    }

    /// Simplify individual assertions using Z3's simplifier
    fn simplify_assertions(&self, assertions: &List<TrackedAssertion>) -> List<TrackedAssertion> {
        assertions
            .iter()
            .map(|assertion| {
                // Apply Z3 simplification to formula
                let simplified_formula = assertion.assertion.simplify();

                TrackedAssertion {
                    id: assertion.id.clone(),
                    assertion: simplified_formula,
                    source: assertion.source.clone(),
                    category: assertion.category.clone(),
                    description: assertion.description.clone(),
                }
            })
            .collect()
    }

    /// Group related assertions by category and dependencies
    fn group_assertions(&self, assertions: &List<TrackedAssertion>) -> List<AssertionGroup> {
        // Group by category
        let mut by_category: Map<AssertionCategory, List<TrackedAssertion>> = Map::new();

        for assertion in assertions {
            by_category
                .entry(assertion.category.clone())
                .or_default()
                .push(assertion.clone());
        }

        // Convert to groups
        by_category
            .into_iter()
            .map(|(category, assertions_in_group)| {
                let conflict_description = self.describe_conflict(&category);
                AssertionGroup {
                    category,
                    assertions: assertions_in_group,
                    conflict_description,
                }
            })
            .collect()
    }

    /// Describe the type of conflict for a category
    fn describe_conflict(&self, category: &AssertionCategory) -> Text {
        match category {
            AssertionCategory::Precondition => {
                Text::from("Preconditions are mutually contradictory")
            }
            AssertionCategory::Postcondition => Text::from("Postcondition cannot be satisfied"),
            AssertionCategory::Refinement => Text::from("Refinement type constraints conflict"),
            AssertionCategory::Invariant => Text::from("Loop invariant cannot be maintained"),
            AssertionCategory::UserAssertion => Text::from("User assertions are inconsistent"),
            AssertionCategory::Generated => Text::from("Generated constraints conflict"),
            AssertionCategory::Custom(name) => format!("{} constraints conflict", name).into(),
        }
    }

    /// Identify root causes of unsatisfiability
    ///
    /// Attempts to find the "core of the core" - the minimal set of assertions
    /// that explain the conflict.
    fn identify_root_causes(&self, assertions: &List<TrackedAssertion>) -> List<RootCause> {
        let mut root_causes = List::new();

        // Try to find minimal subsets that are still unsat
        // This is a heuristic approach - full analysis would be expensive
        for i in 0..assertions.len() {
            for j in (i + 1)..assertions.len() {
                let mut pair = List::new();
                pair.push(assertions[i].clone());
                pair.push(assertions[j].clone());

                if self.check_if_unsat_pair(&assertions[i], &assertions[j]) {
                    root_causes.push(RootCause {
                        assertions: pair,
                        explanation: self.explain_conflict(&assertions[i], &assertions[j]),
                        severity: self.assess_severity(&assertions[i], &assertions[j]),
                    });
                }
            }
        }

        // Sort by severity
        root_causes.sort_by(|a, b| b.severity.cmp(&a.severity));

        root_causes
    }

    /// Check if a pair of assertions is unsatisfiable
    fn check_if_unsat_pair(&self, a1: &TrackedAssertion, a2: &TrackedAssertion) -> bool {
        let solver = Solver::new();
        solver.assert(&a1.assertion);
        solver.assert(&a2.assertion);
        solver.check() == SatResult::Unsat
    }

    /// Explain why two assertions conflict
    fn explain_conflict(&self, a1: &TrackedAssertion, a2: &TrackedAssertion) -> Text {
        // This is a simplified explanation - could be enhanced with SMT analysis
        format!(
            "Conflict between '{}' and '{}': The constraints are mutually exclusive",
            a1.description.clone().unwrap_or_else(|| a1.id.clone()),
            a2.description.clone().unwrap_or_else(|| a2.id.clone())
        )
        .into()
    }

    /// Assess the severity of a conflict
    fn assess_severity(&self, a1: &TrackedAssertion, a2: &TrackedAssertion) -> u8 {
        // Preconditions and postconditions are critical
        let severity1 = match a1.category {
            AssertionCategory::Precondition | AssertionCategory::Postcondition => 10,
            AssertionCategory::Invariant => 8,
            AssertionCategory::Refinement => 7,
            AssertionCategory::UserAssertion => 6,
            _ => 5,
        };

        let severity2 = match a2.category {
            AssertionCategory::Precondition | AssertionCategory::Postcondition => 10,
            AssertionCategory::Invariant => 8,
            AssertionCategory::Refinement => 7,
            AssertionCategory::UserAssertion => 6,
            _ => 5,
        };

        severity1.max(severity2)
    }

    /// Generate human-readable explanation of the unsat core
    fn generate_explanation(
        &self,
        assertions: &List<TrackedAssertion>,
        groups: &List<AssertionGroup>,
    ) -> Text {
        let mut explanation = String::new();
        explanation.push_str("Verification failed due to conflicting constraints:\n\n");

        // Summarize by group
        for group in groups {
            explanation.push_str(&format!(
                "{}:\n  {} assertions conflict\n  {}\n\n",
                group.category,
                group.assertions.len(),
                group.conflict_description
            ));
        }

        // List individual assertions
        explanation.push_str("Conflicting assertions:\n");
        for assertion in assertions {
            let desc = assertion
                .description
                .clone()
                .unwrap_or_else(|| assertion.id.clone());
            let location = assertion
                .source
                .clone()
                .unwrap_or_else(|| Text::from("unknown"));

            explanation.push_str(&format!(
                "  - [{}] {} at {}\n",
                assertion.category, desc, location
            ));
        }

        explanation.into()
    }

    /// Merge redundant assertions in a core
    ///
    /// If multiple assertions are logically equivalent or one implies another,
    /// keep only the strongest one.
    pub fn merge_redundant(&self, assertions: &List<TrackedAssertion>) -> List<TrackedAssertion> {
        let mut result = List::new();
        let mut processed = Set::new();

        for (i, assertion) in assertions.iter().enumerate() {
            if processed.contains(&i) {
                continue;
            }

            // Check if this assertion is implied by others
            let mut is_redundant = false;

            for (j, other) in assertions.iter().enumerate() {
                if i == j || processed.contains(&j) {
                    continue;
                }

                // Check if other implies assertion
                if self.check_implication(&other.assertion, &assertion.assertion) {
                    // other is stronger, skip assertion
                    is_redundant = true;
                    break;
                }
            }

            if !is_redundant {
                result.push(assertion.clone());
            }

            processed.insert(i);
        }

        result
    }

    /// Check if formula1 implies formula2
    fn check_implication(&self, formula1: &Bool, formula2: &Bool) -> bool {
        let solver = Solver::new();
        // Check if formula1 ∧ ¬formula2 is UNSAT
        // If so, then formula1 ⇒ formula2
        solver.assert(formula1);
        solver.assert(formula2.not());
        solver.check() == SatResult::Unsat
    }
}

/// Simplified unsat core with explanation
#[derive(Debug)]
pub struct SimplifiedCore {
    /// Original unsat core
    pub original: UnsatCore,
    /// Simplified assertions
    pub simplified_assertions: List<TrackedAssertion>,
    /// Grouped assertions
    pub groups: List<AssertionGroup>,
    /// Identified root causes
    pub root_causes: List<RootCause>,
    /// Human-readable explanation
    pub explanation: Text,
}

impl SimplifiedCore {
    /// Get summary for error reporting
    pub fn summary(&self) -> Text {
        let mut summary = String::new();

        // Start with overall stats
        summary.push_str(&format!(
            "Found {} conflicting constraints in {} groups\n\n",
            self.simplified_assertions.len(),
            self.groups.len()
        ));

        // Show top root causes
        if !self.root_causes.is_empty() {
            summary.push_str("Key conflicts:\n");
            for (i, cause) in self.root_causes.iter().take(3).enumerate() {
                summary.push_str(&format!("{}. {}\n", i + 1, cause.explanation));
            }
            summary.push('\n');
        }

        // Add full explanation
        summary.push_str(self.explanation.as_str());

        summary.into()
    }
}

/// Group of related assertions
#[derive(Debug)]
pub struct AssertionGroup {
    /// Category of assertions in this group
    pub category: AssertionCategory,
    /// Assertions in this group
    pub assertions: List<TrackedAssertion>,
    /// Description of the conflict
    pub conflict_description: Text,
}

/// Root cause of unsatisfiability
#[derive(Debug)]
pub struct RootCause {
    /// Minimal set of assertions causing conflict
    pub assertions: List<TrackedAssertion>,
    /// Explanation of why they conflict
    pub explanation: Text,
    /// Severity (0-10, higher = more severe)
    pub severity: u8,
}
