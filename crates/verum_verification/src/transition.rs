//! Gradual Transition Between Verification Levels
//!
//! Implements the gradual transition system that allows code to smoothly
//! migrate from runtime checking to compile-time verification.
//!
//! Key features:
//! - Automated transition recommendation based on code stability
//! - Migration path planning (Runtime -> Static -> Proof)
//! - Cost-benefit analysis for transitions
//! - Safety guarantees during transitions
//!
//! The transition system enables seamless migration between verification levels:
//! start with @verify(runtime) for rapid prototyping, gradually add @verify(static)
//! for performance-critical code, then use @verify(proof) for critical safety
//! requirements. Transitions are recommended based on code stability metrics,
//! change frequency, test coverage, and cyclomatic complexity.

use crate::Error;
use crate::cost::{CostModel, VerificationCost};
use crate::level::{VerificationLevel, VerificationMode};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use verum_common::{List, Map, Maybe, Text, ToText};

/// Strategy for transitioning between verification levels
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TransitionStrategy {
    /// Conservative: Only transition when absolutely safe
    Conservative,

    /// Balanced: Transition when likely beneficial (default)
    #[default]
    Balanced,

    /// Aggressive: Maximize static verification
    Aggressive,

    /// Manual: Only transition with explicit user annotation
    Manual,
}

impl TransitionStrategy {
    /// Get confidence threshold for this strategy (0.0 to 1.0)
    pub fn confidence_threshold(&self) -> f64 {
        match self {
            TransitionStrategy::Conservative => 0.95,
            TransitionStrategy::Balanced => 0.80,
            TransitionStrategy::Aggressive => 0.60,
            TransitionStrategy::Manual => 1.0,
        }
    }

    /// Get maximum acceptable cost increase (percentage)
    pub fn max_cost_increase_percent(&self) -> f64 {
        match self {
            TransitionStrategy::Conservative => 10.0,
            TransitionStrategy::Balanced => 20.0,
            TransitionStrategy::Aggressive => 50.0,
            TransitionStrategy::Manual => f64::INFINITY,
        }
    }
}

/// Decision about whether to transition verification levels
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransitionDecision {
    /// Whether transition is recommended
    pub recommend: bool,

    /// Source verification level
    pub from: VerificationLevel,

    /// Target verification level
    pub to: VerificationLevel,

    /// Confidence in recommendation (0.0 to 1.0)
    pub confidence: f64,

    /// Expected benefit (percentage improvement)
    pub expected_benefit_percent: f64,

    /// Expected cost increase (percentage)
    pub expected_cost_percent: f64,

    /// Reason for recommendation
    pub reason: Text,

    /// Required steps for transition
    pub steps: List<TransitionStep>,
}

impl TransitionDecision {
    /// Create a decision to stay at current level
    pub fn stay(level: VerificationLevel, reason: Text) -> Self {
        Self {
            recommend: false,
            from: level,
            to: level,
            confidence: 1.0,
            expected_benefit_percent: 0.0,
            expected_cost_percent: 0.0,
            reason,
            steps: List::new(),
        }
    }

    /// Create a decision to transition
    pub fn transition(
        from: VerificationLevel,
        to: VerificationLevel,
        confidence: f64,
        benefit: f64,
        cost: f64,
        reason: Text,
        steps: List<TransitionStep>,
    ) -> Self {
        Self {
            recommend: true,
            from,
            to,
            confidence,
            expected_benefit_percent: benefit,
            expected_cost_percent: cost,
            reason,
            steps,
        }
    }

    /// Check if this decision passes the strategy's threshold
    pub fn passes_threshold(&self, strategy: &TransitionStrategy) -> bool {
        if !self.recommend {
            return false;
        }

        self.confidence >= strategy.confidence_threshold()
            && self.expected_cost_percent <= strategy.max_cost_increase_percent()
    }
}

/// A step in the transition process
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionStep {
    /// Description of this step
    pub description: Text,

    /// Whether this step is automated
    pub automated: bool,

    /// Whether this step is required
    pub required: bool,

    /// Estimated time to complete (for manual steps)
    pub estimated_time: Option<Duration>,
}

impl TransitionStep {
    /// Create an automated step
    pub fn automated(description: Text) -> Self {
        Self {
            description,
            automated: true,
            required: true,
            estimated_time: None,
        }
    }

    /// Create a manual step
    pub fn manual(description: Text, estimated_time: Duration) -> Self {
        Self {
            description,
            automated: false,
            required: true,
            estimated_time: Some(estimated_time),
        }
    }

    /// Create an optional step
    pub fn optional(description: Text) -> Self {
        Self {
            description,
            automated: false,
            required: false,
            estimated_time: None,
        }
    }
}

/// Analyzer for gradual verification transitions
///
/// Analyzes code to recommend when and how to transition between
/// verification levels, based on:
/// - Code stability (test coverage, change frequency)
/// - Verification cost
/// - Expected performance benefit
/// - Proof complexity
#[derive(Debug)]
pub struct TransitionAnalyzer {
    /// Transition strategy
    strategy: TransitionStrategy,

    /// Cost model for verification
    cost_model: CostModel,

    /// Historical verification costs
    history: Map<Text, List<VerificationCost>>,
}

impl TransitionAnalyzer {
    /// Create a new transition analyzer
    pub fn new(strategy: TransitionStrategy) -> Self {
        Self {
            strategy,
            cost_model: CostModel::default(),
            history: Map::new(),
        }
    }

    /// Analyze whether to transition a function's verification level
    pub fn analyze_function(
        &self,
        _function_name: &Text,
        current_level: VerificationLevel,
        code_metrics: &CodeMetrics,
    ) -> TransitionDecision {
        // Check if already at highest level
        if current_level == VerificationLevel::Proof {
            return TransitionDecision::stay(
                current_level,
                Text::from("Already at highest verification level"),
            );
        }

        // Determine target level based on metrics
        let target_level = self.recommend_target_level(current_level, code_metrics);

        if target_level == current_level {
            return TransitionDecision::stay(current_level, Text::from("Current level is optimal"));
        }

        // Calculate confidence based on code stability
        let confidence = self.calculate_confidence(code_metrics);

        // Estimate benefit (runtime improvement)
        let benefit = self.estimate_benefit(current_level, target_level, code_metrics);

        // Estimate cost (compile-time increase)
        let cost = self.estimate_cost(current_level, target_level, code_metrics);

        // Generate transition steps
        let steps = self.generate_steps(current_level, target_level, code_metrics);

        // Create reason
        let reason = self.generate_reason(current_level, target_level, code_metrics);

        TransitionDecision::transition(
            current_level,
            target_level,
            confidence,
            benefit,
            cost,
            reason,
            steps,
        )
    }

    /// Recommend target verification level based on code metrics
    fn recommend_target_level(
        &self,
        current: VerificationLevel,
        metrics: &CodeMetrics,
    ) -> VerificationLevel {
        use VerificationLevel::*;

        match current {
            Runtime => {
                // Consider moving to Static if code is stable
                if metrics.is_stable() && metrics.has_good_tests() {
                    if metrics.is_critical() { Proof } else { Static }
                } else {
                    Runtime
                }
            }
            Static => {
                // Consider moving to Proof if critical
                if metrics.is_critical() && metrics.is_very_stable() {
                    Proof
                } else {
                    Static
                }
            }
            Proof => Proof,
        }
    }

    /// Calculate confidence in the recommendation
    fn calculate_confidence(&self, metrics: &CodeMetrics) -> f64 {
        let mut confidence = 0.5; // Base confidence

        // Increase confidence based on stability
        if metrics.change_frequency_per_week < 0.1 {
            confidence += 0.2;
        }

        // Increase confidence based on test coverage
        if metrics.test_coverage > 0.95 {
            confidence += 0.2;
        }

        // Increase confidence based on proof simplicity
        if metrics.proof_complexity < 100 {
            confidence += 0.1;
        }

        // Cap at 1.0
        if confidence > 1.0 { 1.0 } else { confidence }
    }

    /// Estimate performance benefit of transition
    fn estimate_benefit(
        &self,
        from: VerificationLevel,
        to: VerificationLevel,
        metrics: &CodeMetrics,
    ) -> f64 {
        use VerificationLevel::*;

        let base_benefit = match (from, to) {
            (Runtime, Static) => 15.0, // Remove runtime checks
            (Runtime, Proof) => 15.0,  // Remove runtime checks
            (Static, Proof) => 5.0,    // Better optimization opportunities
            _ => 0.0,
        };

        // Scale by hotness
        base_benefit * metrics.execution_frequency_multiplier()
    }

    /// Estimate cost increase of transition
    fn estimate_cost(
        &self,
        from: VerificationLevel,
        to: VerificationLevel,
        metrics: &CodeMetrics,
    ) -> f64 {
        use VerificationLevel::*;

        let base_cost = match (from, to) {
            (Runtime, Static) => 15.0, // +15% compile time
            (Runtime, Proof) => 400.0, // +400% compile time
            (Static, Proof) => 300.0,  // +300% compile time
            _ => 0.0,
        };

        // Scale by proof complexity
        base_cost * (metrics.proof_complexity as f64 / 100.0)
    }

    /// Generate transition steps
    fn generate_steps(
        &self,
        from: VerificationLevel,
        to: VerificationLevel,
        metrics: &CodeMetrics,
    ) -> List<TransitionStep> {
        let mut steps = List::new();

        use VerificationLevel::*;

        match (from, to) {
            (Runtime, Static) => {
                steps.push(TransitionStep::automated(Text::from(
                    "Enable static verification with @verify(static)",
                )));
                steps.push(TransitionStep::automated(Text::from(
                    "Run escape analysis on all references",
                )));
                if metrics.has_complex_predicates() {
                    steps.push(TransitionStep::manual(
                        Text::from("Simplify complex refinement predicates"),
                        Duration::from_secs(3600), // 1 hour
                    ));
                }
            }
            (Runtime, Proof) | (Static, Proof) => {
                steps.push(TransitionStep::automated(Text::from(
                    "Enable proof verification with @verify(proof)",
                )));
                steps.push(TransitionStep::automated(Text::from(
                    "Add preconditions using contract# literals",
                )));
                steps.push(TransitionStep::automated(Text::from(
                    "Add postconditions using contract# literals",
                )));
                if metrics.has_loops() {
                    steps.push(TransitionStep::manual(
                        Text::from("Add loop invariants"),
                        Duration::from_secs(7200), // 2 hours
                    ));
                }
                steps.push(TransitionStep::automated(Text::from(
                    "Run SMT solver to generate proof",
                )));
            }
            _ => {}
        }

        steps
    }

    /// Generate human-readable reason for recommendation
    fn generate_reason(
        &self,
        from: VerificationLevel,
        to: VerificationLevel,
        _metrics: &CodeMetrics,
    ) -> Text {
        use VerificationLevel::*;

        match (from, to) {
            (Runtime, Static) => "Code is stable with good test coverage. \
                 Static verification will eliminate runtime checks and improve performance."
                .to_text(),
            (Runtime, Proof) | (Static, Proof) => "Code is critical and very stable. \
                 Formal proofs will provide mathematical guarantees of correctness."
                .to_text(),
            _ => "No transition recommended".to_text(),
        }
    }

    /// Record verification cost for future analysis
    pub fn record_cost(&mut self, function_name: Text, cost: VerificationCost) {
        self.history
            .entry(function_name)
            .or_insert_with(List::new)
            .push(cost);
    }

    /// Get historical costs for a function
    pub fn get_history(&self, function_name: &Text) -> Option<&List<VerificationCost>> {
        self.history.get(function_name)
    }
}

/// Code metrics for transition analysis
///
/// Captures code stability, complexity, and quality indicators used to recommend
/// verification level transitions (runtime -> static -> proof).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeMetrics {
    /// Test coverage (0.0 to 1.0)
    pub test_coverage: f64,

    /// Change frequency (changes per week)
    pub change_frequency_per_week: f64,

    /// Lines of code
    pub lines_of_code: usize,

    /// Cyclomatic complexity (number of linearly independent paths)
    pub cyclomatic_complexity: usize,

    /// Proof complexity estimate (SMT query size)
    pub proof_complexity: usize,

    /// Execution frequency (calls per second in production)
    pub execution_frequency: f64,

    /// Criticality score (0 to 10)
    pub criticality_score: u8,

    /// Whether code has loops
    pub has_loops: bool,

    /// Whether code has complex predicates
    pub has_complex_predicates: bool,

    // === NEW fields per spec ===
    /// Number of dependencies (imported modules, called functions)
    pub dependency_count: u32,

    /// Number of invariants/contracts in the function
    pub invariant_count: u32,

    /// Whether the function contains unsafe blocks
    pub has_unsafe_blocks: bool,

    /// Maximum loop nesting depth
    pub loop_nesting_depth: u32,

    /// Assertion density (asserts per LOC)
    pub assertion_density: f64,
}

impl CodeMetrics {
    /// Check if code is stable
    pub fn is_stable(&self) -> bool {
        self.change_frequency_per_week < 1.0
    }

    /// Check if code is very stable
    pub fn is_very_stable(&self) -> bool {
        self.change_frequency_per_week < 0.1
    }

    /// Check if code has good test coverage
    pub fn has_good_tests(&self) -> bool {
        self.test_coverage > 0.90
    }

    /// Check if code is critical
    pub fn is_critical(&self) -> bool {
        self.criticality_score >= 8
    }

    /// Check if code has loops
    pub fn has_loops(&self) -> bool {
        self.has_loops
    }

    /// Check if code has complex predicates
    pub fn has_complex_predicates(&self) -> bool {
        self.has_complex_predicates
    }

    /// Get execution frequency multiplier for benefit calculation
    pub fn execution_frequency_multiplier(&self) -> f64 {
        if self.execution_frequency > 1000.0 {
            2.0 // Hot path
        } else if self.execution_frequency > 100.0 {
            1.5 // Warm path
        } else {
            1.0 // Cold path
        }
    }
}

impl CodeMetrics {
    /// Create new metrics with the given function name
    pub fn new() -> Self {
        Self::default()
    }

    /// Calculate maintainability index (0-100)
    pub fn maintainability_index(&self) -> f64 {
        let loc = self.lines_of_code.max(1) as f64;
        let cc = self.cyclomatic_complexity.max(1) as f64;

        let halstead_approx = loc * loc.ln().max(1.0);
        let mi = 171.0 - 5.2 * halstead_approx.ln() - 0.23 * cc - 16.2 * loc.ln();

        (mi.max(0.0) / 171.0 * 100.0).min(100.0)
    }

    /// Calculate risk score for verification transition (0-10)
    pub fn transition_risk_score(&self) -> f64 {
        let mut risk = 0.0;

        if self.cyclomatic_complexity > 20 {
            risk += 3.0;
        } else if self.cyclomatic_complexity > 10 {
            risk += 1.5;
        }

        if self.loop_nesting_depth > 4 {
            risk += 2.0;
        } else if self.loop_nesting_depth > 2 {
            risk += 1.0;
        }

        if self.has_unsafe_blocks {
            risk += 2.0;
        }

        if self.has_complex_predicates {
            risk += 1.5;
        }

        if self.test_coverage < 0.5 {
            risk += 2.0;
        } else if self.test_coverage < 0.8 {
            risk += 1.0;
        }

        if self.change_frequency_per_week > 5.0 {
            risk += 2.0;
        } else if self.change_frequency_per_week > 1.0 {
            risk += 1.0;
        }

        if risk > 10.0 { 10.0 } else { risk }
    }

    /// Check if code has deep nesting
    pub fn has_deep_nesting(&self) -> bool {
        self.loop_nesting_depth > 3
    }

    /// Check if code is suitable for formal verification
    pub fn is_verification_candidate(&self) -> bool {
        self.is_stable()
            && self.has_good_tests()
            && self.cyclomatic_complexity <= 20
            && !self.has_deep_nesting()
    }
}

impl Default for CodeMetrics {
    fn default() -> Self {
        Self {
            test_coverage: 0.0,
            change_frequency_per_week: 10.0,
            lines_of_code: 0,
            cyclomatic_complexity: 1,
            proof_complexity: 50,
            execution_frequency: 0.0,
            criticality_score: 0,
            has_loops: false,
            has_complex_predicates: false,
            dependency_count: 0,
            invariant_count: 0,
            has_unsafe_blocks: false,
            loop_nesting_depth: 0,
            assertion_density: 0.0,
        }
    }
}

/// Migration path from one verification level to another
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MigrationPath {
    /// Starting verification level
    pub start: VerificationLevel,

    /// Target verification level
    pub target: VerificationLevel,

    /// Steps in the migration
    pub steps: List<MigrationStep>,

    /// Total estimated time
    pub estimated_time: Duration,

    /// Whether migration is fully automated
    pub fully_automated: bool,
}

impl MigrationPath {
    /// Create a migration path
    pub fn new(
        start: VerificationLevel,
        target: VerificationLevel,
        steps: List<MigrationStep>,
    ) -> Self {
        let estimated_time = steps
            .iter()
            .filter_map(|s| s.estimated_time)
            .sum::<Duration>();

        let fully_automated = steps.iter().all(|s| s.automated);

        Self {
            start,
            target,
            steps,
            estimated_time,
            fully_automated,
        }
    }

    /// Get the next step to execute
    pub fn next_step(&self) -> Option<&MigrationStep> {
        self.steps.iter().find(|s| !s.completed)
    }

    /// Check if migration is complete
    pub fn is_complete(&self) -> bool {
        self.steps.iter().all(|s| s.completed)
    }
}

/// A step in a migration path
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationStep {
    /// Description of this step
    pub description: Text,

    /// Whether this step is automated
    pub automated: bool,

    /// Whether this step has been completed
    pub completed: bool,

    /// Estimated time to complete
    pub estimated_time: Option<Duration>,
}

impl MigrationStep {
    /// Create an automated migration step
    pub fn automated(description: Text) -> Self {
        Self {
            description,
            automated: true,
            completed: false,
            estimated_time: None,
        }
    }

    /// Create a manual migration step
    pub fn manual(description: Text, estimated_time: Duration) -> Self {
        Self {
            description,
            automated: false,
            completed: false,
            estimated_time: Some(estimated_time),
        }
    }

    /// Mark this step as completed
    pub fn complete(&mut self) {
        self.completed = true;
    }
}
