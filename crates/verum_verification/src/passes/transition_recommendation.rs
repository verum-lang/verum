//! Transition-recommendation verification pass.
//!
//! Analyses each function's metrics (complexity, test coverage,
//! change frequency) and recommends a transition between
//! verification levels (e.g., runtime → static → proof) per the
//! configured `TransitionStrategy`.

use std::time::Instant;

use verum_ast::{Module, decl::ItemKind};
use verum_common::{List, Text};

use crate::context::VerificationContext;
use crate::level::VerificationLevel;
use crate::transition::{CodeMetrics, TransitionAnalyzer, TransitionStrategy};

use super::{VerificationError, VerificationPass, VerificationResult};

/// Transition recommendation pass.
///
/// Analyses functions for transition opportunities between
/// verification levels using real code metrics collection.
#[derive(Debug)]
pub struct TransitionRecommendationPass {
    /// Transition analyzer with strategy
    analyzer: TransitionAnalyzer,
    /// Metrics collector for real metrics
    metrics_collector: crate::metrics::CodeMetricsCollector,
    /// Generated recommendations
    recommendations: List<TransitionRecommendation>,
}

/// A transition recommendation for a specific function.
#[derive(Debug, Clone)]
pub struct TransitionRecommendation {
    /// Function name
    pub function_name: Text,
    /// Current verification level
    pub current_level: VerificationLevel,
    /// Recommended target level
    pub recommended_level: VerificationLevel,
    /// Confidence in recommendation (0.0 to 1.0)
    pub confidence: f64,
    /// Expected benefit (percentage)
    pub expected_benefit: f64,
    /// Collected metrics
    pub metrics: CodeMetrics,
    /// Human-readable reason
    pub reason: Text,
    /// Risk score for transition
    pub risk_score: f64,
}

impl TransitionRecommendationPass {
    /// Create a new transition recommendation pass.
    pub fn new(strategy: TransitionStrategy) -> Self {
        Self {
            analyzer: TransitionAnalyzer::new(strategy),
            metrics_collector: crate::metrics::CodeMetricsCollector::new(),
            recommendations: List::new(),
        }
    }

    /// Load coverage data for metrics collection.
    pub fn load_coverage(
        &mut self,
        path: &std::path::Path,
    ) -> Result<(), crate::metrics::MetricsError> {
        self.metrics_collector.load_coverage(path)
    }

    /// Load profiling data for metrics collection.
    pub fn load_profiling(
        &mut self,
        path: &std::path::Path,
    ) -> Result<(), crate::metrics::MetricsError> {
        self.metrics_collector.load_profiling(path)
    }

    /// Load git history for change frequency analysis.
    pub fn load_git_history(
        &mut self,
        repo_path: &std::path::Path,
    ) -> Result<(), crate::metrics::MetricsError> {
        self.metrics_collector.load_git_history(repo_path)
    }

    /// Get all generated recommendations.
    pub fn recommendations(&self) -> &List<TransitionRecommendation> {
        &self.recommendations
    }

    /// Get recommendations for functions that should transition.
    pub fn actionable_recommendations(&self) -> List<&TransitionRecommendation> {
        self.recommendations
            .iter()
            .filter(|r| r.current_level != r.recommended_level && r.confidence >= 0.7)
            .collect()
    }
}

impl VerificationPass for TransitionRecommendationPass {
    fn run(
        &mut self,
        module: &Module,
        ctx: &mut VerificationContext,
    ) -> Result<VerificationResult, VerificationError> {
        let start = Instant::now();
        self.recommendations = List::new();

        // Analyze each function for transition opportunities
        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                let current_level = ctx.current_level();

                // Collect real metrics from AST analysis
                let enhanced_metrics = self.metrics_collector.analyze_function(func);
                let metrics = enhanced_metrics.to_code_metrics();

                // Analyze transition using real metrics
                let decision = self.analyzer.analyze_function(
                    &Text::from(func.name.as_str()),
                    current_level,
                    &metrics,
                );

                // Generate reason based on metrics
                let reason = if decision.recommend {
                    if metrics.is_stable() && metrics.has_good_tests() {
                        Text::from(format!(
                            "Code is stable (change freq: {:.2}/week) with good test coverage ({:.1}%)",
                            metrics.change_frequency_per_week,
                            metrics.test_coverage * 100.0
                        ))
                    } else if metrics.is_critical() {
                        Text::from("Critical code path requiring stronger guarantees")
                    } else {
                        Text::from(format!(
                            "Based on complexity ({}) and coverage ({:.1}%)",
                            metrics.cyclomatic_complexity,
                            metrics.test_coverage * 100.0
                        ))
                    }
                } else {
                    Text::from("Transition not recommended at this time")
                };

                // Calculate risk score
                let risk_score = metrics.transition_risk_score();

                // Create recommendation
                let recommendation = TransitionRecommendation {
                    function_name: Text::from(func.name.as_str()),
                    current_level,
                    recommended_level: decision.to,
                    confidence: decision.confidence,
                    expected_benefit: decision.expected_benefit_percent,
                    metrics: metrics.clone(),
                    reason,
                    risk_score,
                };

                self.recommendations.push(recommendation);
            }
        }

        let duration = start.elapsed();
        let mut result =
            VerificationResult::success(VerificationLevel::Runtime, duration, List::new());
        result.functions_verified = self.recommendations.len();

        Ok(result)
    }

    fn name(&self) -> &str {
        "transition_recommendation"
    }

    /// V8 (#208, B7) — transition recommendations are advisory.
    /// A "failure" here means the recommender produced no output
    /// for some functions, which is informational, not unsound.
    fn classification(&self) -> super::PassClassification {
        super::PassClassification::Informational
    }
}
