//! Verification Cost Reporting and Decision Making
//!
//! Implements cost tracking and analysis for verification operations:
//! - Track verification time per function
//! - Cost-benefit analysis for verification mode selection
//! - Budget enforcement
//! - Performance regression detection
//!
//! Tracks verification time per function, performs cost-benefit analysis for
//! verification mode selection (runtime vs static vs proof), enforces compile-time
//! budgets, and detects performance regressions across verification runs.

use crate::level::VerificationLevel;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use verum_common::{List, Map, Text};

/// Cost of a verification operation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VerificationCost {
    /// Function being verified
    pub function_name: Text,

    /// Verification level used
    pub level: VerificationLevel,

    /// Time taken for verification
    pub duration: Duration,

    /// Number of SMT queries issued
    pub smt_queries: usize,

    /// Whether verification succeeded
    pub success: bool,

    /// Whether verification timed out
    pub timed_out: bool,

    /// Timestamp when verification occurred
    #[serde(skip)]
    pub timestamp: Instant,

    /// Size of SMT problem (number of constraints)
    pub problem_size: usize,
}

impl Default for VerificationCost {
    fn default() -> Self {
        Self {
            function_name: Text::from(""),
            level: VerificationLevel::Runtime,
            duration: Duration::ZERO,
            smt_queries: 0,
            success: false,
            timed_out: false,
            timestamp: Instant::now(),
            problem_size: 0,
        }
    }
}

impl VerificationCost {
    /// Create a new cost record
    pub fn new(
        function_name: Text,
        level: VerificationLevel,
        duration: Duration,
        smt_queries: usize,
        success: bool,
        timed_out: bool,
        problem_size: usize,
    ) -> Self {
        Self {
            function_name,
            level,
            duration,
            smt_queries,
            success,
            timed_out,
            timestamp: Instant::now(),
            problem_size,
        }
    }

    /// Create a timeout cost record
    pub fn timeout(
        function_name: Text,
        level: VerificationLevel,
        duration: Duration,
        smt_queries: usize,
        problem_size: usize,
    ) -> Self {
        Self {
            function_name,
            level,
            duration,
            smt_queries,
            success: false,
            timed_out: true,
            timestamp: Instant::now(),
            problem_size,
        }
    }

    /// Get cost in milliseconds
    pub fn cost_ms(&self) -> u64 {
        self.duration.as_millis() as u64
    }

    /// Get cost per constraint (ms)
    pub fn cost_per_constraint(&self) -> f64 {
        if self.problem_size == 0 {
            0.0
        } else {
            self.cost_ms() as f64 / self.problem_size as f64
        }
    }
}

/// Model for predicting verification costs
#[derive(Debug, Clone)]
pub struct CostModel {
    /// Base cost per verification level (ms)
    base_costs: Map<VerificationLevel, u64>,

    /// Cost per SMT query (ms)
    smt_query_cost: u64,

    /// Cost per constraint (ms)
    constraint_cost: f64,

    /// Historical costs for calibration
    history: List<VerificationCost>,
}

impl CostModel {
    /// Create a new cost model with default parameters
    pub fn new() -> Self {
        let mut base_costs = Map::new();
        base_costs.insert(VerificationLevel::Runtime, 0);
        base_costs.insert(VerificationLevel::Static, 100);
        base_costs.insert(VerificationLevel::Proof, 1000);

        Self {
            base_costs,
            smt_query_cost: 10,
            constraint_cost: 0.5,
            history: List::new(),
        }
    }

    /// Predict cost of verifying a function
    pub fn predict_cost(
        &self,
        level: VerificationLevel,
        estimated_queries: usize,
        estimated_constraints: usize,
    ) -> Duration {
        let base = *self.base_costs.get(&level).unwrap_or(&0);
        let query_cost = self.smt_query_cost * estimated_queries as u64;
        let constraint_cost = (self.constraint_cost * estimated_constraints as f64) as u64;

        let total_ms = base + query_cost + constraint_cost;
        Duration::from_millis(total_ms)
    }

    /// Record actual cost for model calibration
    pub fn record_cost(&mut self, cost: VerificationCost) {
        self.history.push(cost);
        self.calibrate();
    }

    /// Calibrate model parameters based on historical data
    fn calibrate(&mut self) {
        if self.history.is_empty() {
            return;
        }

        // Recalculate average costs per level
        for level in [
            VerificationLevel::Runtime,
            VerificationLevel::Static,
            VerificationLevel::Proof,
        ] {
            let level_costs: List<_> = self.history.iter().filter(|c| c.level == level).collect();

            if !level_costs.is_empty() {
                let avg =
                    level_costs.iter().map(|c| c.cost_ms()).sum::<u64>() / level_costs.len() as u64;
                self.base_costs.insert(level, avg);
            }
        }

        // Recalculate constraint cost
        let constraint_costs: List<_> = self
            .history
            .iter()
            .filter(|c| c.problem_size > 0)
            .map(|c| c.cost_per_constraint())
            .collect();

        if !constraint_costs.is_empty() {
            self.constraint_cost =
                constraint_costs.iter().sum::<f64>() / constraint_costs.len() as f64;
        }
    }

    /// Get average cost for a verification level
    pub fn average_cost(&self, level: VerificationLevel) -> Duration {
        Duration::from_millis(*self.base_costs.get(&level).unwrap_or(&0))
    }
}

impl Default for CostModel {
    fn default() -> Self {
        Self::new()
    }
}

/// Report of verification costs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostReport {
    /// Total verification time
    pub total_duration: Duration,

    /// Number of functions verified
    pub functions_verified: usize,

    /// Number of successful verifications
    pub successes: usize,

    /// Number of failed verifications
    pub failures: usize,

    /// Number of timeouts
    pub timeouts: usize,

    /// Costs grouped by verification level
    #[serde(skip)]
    pub by_level: Map<VerificationLevel, LevelCostSummary>,

    /// Top 10 most expensive functions
    pub expensive_functions: List<VerificationCost>,

    /// Functions that exceeded budget
    pub budget_violations: List<BudgetViolation>,
}

impl CostReport {
    /// Create a cost report from a list of costs
    pub fn from_costs(costs: List<VerificationCost>, budget: Option<CostThreshold>) -> Self {
        let total_duration = costs.iter().map(|c| c.duration).sum();
        let functions_verified = costs.len();
        let successes = costs.iter().filter(|c| c.success).count();
        let timeouts = costs.iter().filter(|c| c.timed_out).count();
        let failures = functions_verified - successes - timeouts;

        // Group by level
        let mut by_level = Map::new();
        for level in [
            VerificationLevel::Runtime,
            VerificationLevel::Static,
            VerificationLevel::Proof,
        ] {
            let level_costs: List<_> = costs.iter().filter(|c| c.level == level).cloned().collect();
            if !level_costs.is_empty() {
                by_level.insert(level, LevelCostSummary::from_costs(level_costs));
            }
        }

        // Find expensive functions
        let mut sorted_costs = costs.clone();
        sorted_costs.sort_by(|a, b| b.cost_ms().cmp(&a.cost_ms()));
        let expensive_functions = sorted_costs.into_iter().take(10).collect();

        // Find budget violations
        let budget_violations = if let Some(threshold) = budget {
            costs
                .iter()
                .filter(|c| c.cost_ms() > threshold.max_duration_ms)
                .map(|c| BudgetViolation {
                    function_name: c.function_name.clone(),
                    actual_ms: c.cost_ms(),
                    budget_ms: threshold.max_duration_ms,
                    overage_percent: ((c.cost_ms() as f64 / threshold.max_duration_ms as f64)
                        - 1.0)
                        * 100.0,
                })
                .collect()
        } else {
            List::new()
        };

        Self {
            total_duration,
            functions_verified,
            successes,
            failures,
            timeouts,
            by_level,
            expensive_functions,
            budget_violations,
        }
    }

    /// Format report as human-readable text
    pub fn format(&self) -> Text {
        let mut report = Text::from("Verification Cost Report\n");
        report.push_str("========================\n\n");

        report.push_str(&format!(
            "Total time: {:.2}s\n",
            self.total_duration.as_secs_f64()
        ));
        report.push_str(&format!(
            "Functions verified: {}\n",
            self.functions_verified
        ));
        report.push_str(&format!("  Successes: {}\n", self.successes));
        report.push_str(&format!("  Failures: {}\n", self.failures));
        report.push_str(&format!("  Timeouts: {}\n\n", self.timeouts));

        report.push_str("By Verification Level:\n");
        for (level, summary) in &self.by_level {
            report.push_str(&format!("  {}: {}\n", level, summary.format()));
        }

        if !self.expensive_functions.is_empty() {
            report.push_str("\nMost Expensive Functions:\n");
            for (i, cost) in self.expensive_functions.iter().enumerate().take(5) {
                report.push_str(&format!(
                    "  {}. {} ({:?}): {:.2}s\n",
                    i + 1,
                    cost.function_name,
                    cost.level,
                    cost.duration.as_secs_f64()
                ));
            }
        }

        if !self.budget_violations.is_empty() {
            report.push_str("\nBudget Violations:\n");
            for violation in &self.budget_violations {
                report.push_str(&format!(
                    "  {}: {}ms > {}ms (+{:.1}%)\n",
                    violation.function_name,
                    violation.actual_ms,
                    violation.budget_ms,
                    violation.overage_percent
                ));
            }
        }

        report
    }
}

/// Summary of costs for a specific verification level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelCostSummary {
    /// Total time at this level
    pub total_duration: Duration,

    /// Number of functions
    pub count: usize,

    /// Average time per function
    pub average_duration: Duration,

    /// Minimum time
    pub min_duration: Duration,

    /// Maximum time
    pub max_duration: Duration,
}

impl LevelCostSummary {
    /// Create summary from costs
    pub fn from_costs(costs: List<VerificationCost>) -> Self {
        let count = costs.len();
        let total_duration = costs.iter().map(|c| c.duration).sum();
        let average_duration = if count > 0 {
            total_duration / count as u32
        } else {
            Duration::ZERO
        };

        let min_duration = costs
            .iter()
            .map(|c| c.duration)
            .min()
            .unwrap_or(Duration::ZERO);

        let max_duration = costs
            .iter()
            .map(|c| c.duration)
            .max()
            .unwrap_or(Duration::ZERO);

        Self {
            total_duration,
            count,
            average_duration,
            min_duration,
            max_duration,
        }
    }

    /// Format as string
    pub fn format(&self) -> Text {
        Text::from(format!(
            "{} functions, avg {:.2}s, total {:.2}s",
            self.count,
            self.average_duration.as_secs_f64(),
            self.total_duration.as_secs_f64()
        ))
    }
}

/// Budget violation record
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetViolation {
    /// Function that exceeded budget
    pub function_name: Text,

    /// Actual time taken (ms)
    pub actual_ms: u64,

    /// Budget limit (ms)
    pub budget_ms: u64,

    /// Overage percentage
    pub overage_percent: f64,
}

/// Cost threshold for verification decisions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostThreshold {
    /// Maximum duration allowed (ms)
    pub max_duration_ms: u64,

    /// Maximum SMT queries allowed
    pub max_smt_queries: usize,

    /// Maximum problem size allowed
    pub max_problem_size: usize,
}

impl CostThreshold {
    /// Create a new cost threshold
    pub fn new(max_duration_ms: u64, max_smt_queries: usize, max_problem_size: usize) -> Self {
        Self {
            max_duration_ms,
            max_smt_queries,
            max_problem_size,
        }
    }

    /// Check if a cost exceeds this threshold
    pub fn exceeds(&self, cost: &VerificationCost) -> bool {
        cost.cost_ms() > self.max_duration_ms
            || cost.smt_queries > self.max_smt_queries
            || cost.problem_size > self.max_problem_size
    }

    /// Default threshold for static verification
    pub fn static_default() -> Self {
        Self::new(5000, 100, 1000)
    }

    /// Default threshold for proof verification
    pub fn proof_default() -> Self {
        Self::new(30000, 1000, 10000)
    }
}

/// Decision about which verification mode to use
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationDecision {
    /// Recommended verification level
    pub recommended_level: VerificationLevel,

    /// Reason for recommendation
    pub reason: Text,

    /// Expected cost
    pub expected_cost: Duration,

    /// Expected benefit (performance improvement)
    pub expected_benefit_percent: f64,

    /// Confidence in recommendation (0.0 to 1.0)
    pub confidence: f64,
}

impl VerificationDecision {
    /// Create a new decision
    pub fn new(
        level: VerificationLevel,
        reason: Text,
        cost: Duration,
        benefit: f64,
        confidence: f64,
    ) -> Self {
        Self {
            recommended_level: level,
            reason,
            expected_cost: cost,
            expected_benefit_percent: benefit,
            confidence,
        }
    }

    /// Recommend runtime verification
    pub fn runtime(reason: Text) -> Self {
        Self::new(VerificationLevel::Runtime, reason, Duration::ZERO, 0.0, 1.0)
    }

    /// Recommend static verification
    pub fn static_verification(reason: Text, cost: Duration, benefit: f64) -> Self {
        Self::new(VerificationLevel::Static, reason, cost, benefit, 0.8)
    }

    /// Recommend proof verification
    pub fn proof(reason: Text, cost: Duration, benefit: f64) -> Self {
        Self::new(VerificationLevel::Proof, reason, cost, benefit, 0.9)
    }
}

/// Criteria for making verification decisions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionCriteria {
    /// Maximum acceptable cost (ms)
    pub max_cost_ms: u64,

    /// Minimum required benefit (percentage)
    pub min_benefit_percent: f64,

    /// Minimum required confidence (0.0 to 1.0)
    pub min_confidence: f64,

    /// Whether to prefer faster compilation
    pub prefer_fast_compilation: bool,

    /// Whether to prefer runtime performance
    pub prefer_runtime_performance: bool,
}

impl DecisionCriteria {
    /// Create default criteria
    pub fn new() -> Self {
        Self {
            max_cost_ms: 10000,
            min_benefit_percent: 5.0,
            min_confidence: 0.75,
            prefer_fast_compilation: false,
            prefer_runtime_performance: true,
        }
    }

    /// Check if a decision meets these criteria
    pub fn meets_criteria(&self, decision: &VerificationDecision) -> bool {
        decision.expected_cost.as_millis() as u64 <= self.max_cost_ms
            && decision.expected_benefit_percent >= self.min_benefit_percent
            && decision.confidence >= self.min_confidence
    }

    /// Development criteria (favor fast iteration)
    pub fn development() -> Self {
        Self {
            max_cost_ms: 1000,
            min_benefit_percent: 0.0,
            min_confidence: 0.5,
            prefer_fast_compilation: true,
            prefer_runtime_performance: false,
        }
    }

    /// Production criteria (favor runtime performance)
    pub fn production() -> Self {
        Self {
            max_cost_ms: 60000,
            min_benefit_percent: 10.0,
            min_confidence: 0.9,
            prefer_fast_compilation: false,
            prefer_runtime_performance: true,
        }
    }
}

impl Default for DecisionCriteria {
    fn default() -> Self {
        Self::new()
    }
}
