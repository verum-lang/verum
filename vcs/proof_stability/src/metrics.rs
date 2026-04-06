//! Stability metrics and flaky proof detection.
//!
//! This module provides statistical analysis of proof stability including:
//! - Per-proof stability metrics
//! - Aggregated stability statistics
//! - Flaky proof detection and reporting
//! - Timing variance analysis

use crate::{
    ProofAttempt, ProofCategory, ProofId, ProofOutcome, StabilityStatus, cache::ProofCacheEntry,
    config::StabilityThresholds,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use verum_common::{List, Text};

/// Metrics for a single proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofMetrics {
    /// Proof identifier
    pub proof_id: ProofId,
    /// Category
    pub category: ProofCategory,
    /// Stability status
    pub stability_status: StabilityStatus,
    /// Stability percentage (0-100)
    pub stability_percentage: f64,
    /// Number of attempts
    pub attempt_count: usize,
    /// Verified count
    pub verified_count: usize,
    /// Failed count
    pub failed_count: usize,
    /// Timeout count
    pub timeout_count: usize,
    /// Error count
    pub error_count: usize,
    /// Unknown count
    pub unknown_count: usize,
    /// Mean duration
    pub mean_duration: Duration,
    /// Standard deviation of duration
    pub duration_std_dev: Duration,
    /// Coefficient of variation (std_dev / mean)
    pub duration_cv: f64,
    /// Min duration
    pub min_duration: Duration,
    /// Max duration
    pub max_duration: Duration,
    /// Is timing stable?
    pub timing_stable: bool,
}

impl ProofMetrics {
    /// Compute metrics from a list of attempts.
    pub fn from_attempts(
        proof_id: ProofId,
        category: ProofCategory,
        attempts: &[ProofAttempt],
        thresholds: &StabilityThresholds,
    ) -> Self {
        let mut verified_count = 0;
        let mut failed_count = 0;
        let mut timeout_count = 0;
        let mut error_count = 0;
        let mut unknown_count = 0;
        let mut durations: List<Duration> = List::new();

        for attempt in attempts {
            durations.push(attempt.duration);
            match &attempt.outcome {
                ProofOutcome::Verified => verified_count += 1,
                ProofOutcome::Failed { .. } => failed_count += 1,
                ProofOutcome::Timeout { .. } => timeout_count += 1,
                ProofOutcome::Error { .. } => error_count += 1,
                ProofOutcome::Unknown { .. } => unknown_count += 1,
            }
        }

        let attempt_count = attempts.len();

        // Calculate duration statistics
        let (mean_duration, duration_std_dev, duration_cv, min_duration, max_duration) =
            calculate_duration_stats(&durations);

        // Determine stability
        let dominant_count = verified_count.max(failed_count);
        let stability_percentage = if attempt_count > 0 {
            (dominant_count as f64 / attempt_count as f64) * 100.0
        } else {
            0.0
        };

        let stability_status = if attempt_count < thresholds.min_runs {
            StabilityStatus::Unknown
        } else if timeout_count > attempt_count / 2 {
            StabilityStatus::TimeoutUnstable
        } else if stability_percentage >= thresholds.stable_threshold * 100.0 {
            StabilityStatus::Stable
        } else if stability_percentage < thresholds.flaky_threshold * 100.0 {
            StabilityStatus::Flaky
        } else {
            StabilityStatus::Unknown
        };

        let timing_stable = duration_cv <= thresholds.timing_variance_threshold;

        Self {
            proof_id,
            category,
            stability_status,
            stability_percentage,
            attempt_count,
            verified_count,
            failed_count,
            timeout_count,
            error_count,
            unknown_count,
            mean_duration,
            duration_std_dev,
            duration_cv,
            min_duration,
            max_duration,
            timing_stable,
        }
    }

    /// Check if this proof is flaky.
    pub fn is_flaky(&self) -> bool {
        matches!(
            self.stability_status,
            StabilityStatus::Flaky | StabilityStatus::TimeoutUnstable
        )
    }

    /// Check if this proof is stable.
    pub fn is_stable(&self) -> bool {
        self.stability_status == StabilityStatus::Stable
    }

    /// Get a summary description.
    pub fn summary(&self) -> Text {
        format!(
            "{}: {:.1}% stable ({}/{} verified), avg {:.2}ms",
            self.stability_status,
            self.stability_percentage,
            self.verified_count,
            self.attempt_count,
            self.mean_duration.as_secs_f64() * 1000.0
        ).into()
    }
}

/// Aggregated stability metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StabilityMetrics {
    /// Total number of proofs
    pub total_proofs: usize,
    /// Stable proofs
    pub stable_count: usize,
    /// Flaky proofs
    pub flaky_count: usize,
    /// Unknown stability proofs
    pub unknown_count: usize,
    /// Timeout unstable proofs
    pub timeout_unstable_count: usize,
    /// Overall stability percentage
    pub overall_stability: f64,
    /// Total attempts
    pub total_attempts: usize,
    /// Total verified
    pub total_verified: usize,
    /// Total failed
    pub total_failed: usize,
    /// Metrics by category
    pub by_category: HashMap<ProofCategory, CategoryMetrics>,
    /// List of flaky proofs
    pub flaky_proofs: List<FlakyProofInfo>,
    /// Average duration across all proofs
    pub average_duration: Duration,
    /// Total test time
    pub total_time: Duration,
}

impl Default for StabilityMetrics {
    fn default() -> Self {
        Self {
            total_proofs: 0,
            stable_count: 0,
            flaky_count: 0,
            unknown_count: 0,
            timeout_unstable_count: 0,
            overall_stability: 0.0,
            total_attempts: 0,
            total_verified: 0,
            total_failed: 0,
            by_category: HashMap::new(),
            flaky_proofs: List::new(),
            average_duration: Duration::ZERO,
            total_time: Duration::ZERO,
        }
    }
}

impl StabilityMetrics {
    /// Create new empty metrics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a proof's metrics to the aggregate.
    pub fn add_proof(&mut self, metrics: &ProofMetrics) {
        self.total_proofs += 1;
        self.total_attempts += metrics.attempt_count;
        self.total_verified += metrics.verified_count;
        self.total_failed += metrics.failed_count;

        match metrics.stability_status {
            StabilityStatus::Stable => self.stable_count += 1,
            StabilityStatus::Flaky => {
                self.flaky_count += 1;
                self.flaky_proofs
                    .push(FlakyProofInfo::from_metrics(metrics));
            }
            StabilityStatus::Unknown => self.unknown_count += 1,
            StabilityStatus::TimeoutUnstable => {
                self.timeout_unstable_count += 1;
                self.flaky_proofs
                    .push(FlakyProofInfo::from_metrics(metrics));
            }
        }

        // Update category metrics
        let cat_metrics = self.by_category.entry(metrics.category).or_default();
        cat_metrics.add(metrics);

        // Update overall stability
        if self.total_proofs > 0 {
            self.overall_stability = (self.stable_count as f64 / self.total_proofs as f64) * 100.0;
        }
    }

    /// Finalize metrics (compute averages, etc.).
    pub fn finalize(&mut self) {
        if self.total_attempts > 0 {
            let total_ms: f64 = self
                .by_category
                .values()
                .map(|c| c.total_duration.as_secs_f64() * 1000.0)
                .sum();
            self.average_duration =
                Duration::from_secs_f64(total_ms / self.total_attempts as f64 / 1000.0);
        }

        // Sort flaky proofs by stability percentage (most unstable first)
        self.flaky_proofs.sort_by(|a, b| {
            a.stability_percentage
                .partial_cmp(&b.stability_percentage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// Check if the test suite meets the stability threshold.
    pub fn meets_threshold(&self, threshold: f64) -> bool {
        self.overall_stability >= threshold
    }

    /// Get a summary description.
    pub fn summary(&self) -> Text {
        format!(
            "{}/{} proofs stable ({:.1}%), {} flaky, {} timeout-unstable, {} unknown",
            self.stable_count,
            self.total_proofs,
            self.overall_stability,
            self.flaky_count,
            self.timeout_unstable_count,
            self.unknown_count
        ).into()
    }
}

/// Metrics for a specific proof category.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CategoryMetrics {
    /// Total proofs in category
    pub total: usize,
    /// Stable proofs
    pub stable: usize,
    /// Flaky proofs
    pub flaky: usize,
    /// Total attempts
    pub attempts: usize,
    /// Total verified
    pub verified: usize,
    /// Total duration
    pub total_duration: Duration,
    /// Stability percentage
    pub stability_percentage: f64,
}

impl CategoryMetrics {
    /// Add a proof's metrics.
    pub fn add(&mut self, metrics: &ProofMetrics) {
        self.total += 1;
        self.attempts += metrics.attempt_count;
        self.verified += metrics.verified_count;
        self.total_duration += metrics.mean_duration * metrics.attempt_count as u32;

        if metrics.is_stable() {
            self.stable += 1;
        } else if metrics.is_flaky() {
            self.flaky += 1;
        }

        if self.total > 0 {
            self.stability_percentage = (self.stable as f64 / self.total as f64) * 100.0;
        }
    }
}

/// Information about a flaky proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlakyProofInfo {
    /// Proof identifier
    pub proof_id: ProofId,
    /// Category
    pub category: ProofCategory,
    /// Stability status
    pub status: StabilityStatus,
    /// Stability percentage
    pub stability_percentage: f64,
    /// Number of attempts
    pub attempt_count: usize,
    /// Outcome distribution
    pub outcome_distribution: Text,
    /// Average duration
    pub average_duration: Duration,
    /// Timing coefficient of variation
    pub timing_cv: f64,
    /// Suggested action
    pub suggested_action: Text,
}

impl FlakyProofInfo {
    /// Create from proof metrics.
    pub fn from_metrics(metrics: &ProofMetrics) -> Self {
        let outcome_distribution: Text = format!(
            "V:{} F:{} T:{} E:{} U:{}",
            metrics.verified_count,
            metrics.failed_count,
            metrics.timeout_count,
            metrics.error_count,
            metrics.unknown_count
        ).into();

        let suggested_action: Text = match metrics.stability_status {
            StabilityStatus::TimeoutUnstable => {
                "Consider increasing timeout or simplifying proof".to_string().into()
            }
            StabilityStatus::Flaky => {
                if metrics.timeout_count > 0 {
                    "Mixed timeouts - investigate solver behavior".to_string().into()
                } else if metrics.verified_count > 0 && metrics.failed_count > 0 {
                    "Inconsistent results - possible quantifier instability".to_string().into()
                } else {
                    "Review proof strategy".to_string().into()
                }
            }
            _ => "No action needed".to_string().into(),
        };

        Self {
            proof_id: metrics.proof_id.clone(),
            category: metrics.category,
            status: metrics.stability_status,
            stability_percentage: metrics.stability_percentage,
            attempt_count: metrics.attempt_count,
            outcome_distribution,
            average_duration: metrics.mean_duration,
            timing_cv: metrics.duration_cv,
            suggested_action,
        }
    }
}

/// Calculate duration statistics.
fn calculate_duration_stats(
    durations: &[Duration],
) -> (Duration, Duration, f64, Duration, Duration) {
    if durations.is_empty() {
        return (
            Duration::ZERO,
            Duration::ZERO,
            0.0,
            Duration::ZERO,
            Duration::ZERO,
        );
    }

    let n = durations.len() as f64;
    let nanos: List<f64> = durations.iter().map(|d| d.as_nanos() as f64).collect();

    let mean = nanos.iter().sum::<f64>() / n;
    let variance = nanos.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    let std_dev = variance.sqrt();
    let cv = if mean > 0.0 { std_dev / mean } else { 0.0 };

    let min = *nanos
        .iter()
        .min_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap_or(&0.0);
    let max = *nanos
        .iter()
        .max_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap_or(&0.0);

    (
        Duration::from_nanos(mean as u64),
        Duration::from_nanos(std_dev as u64),
        cv,
        Duration::from_nanos(min as u64),
        Duration::from_nanos(max as u64),
    )
}

/// Analyze proof stability from cache entries.
pub fn analyze_stability(
    entries: &[&ProofCacheEntry],
    thresholds: &StabilityThresholds,
) -> StabilityMetrics {
    let mut metrics = StabilityMetrics::new();

    for entry in entries {
        // Convert cache entry attempts to ProofAttempts
        let attempts: List<ProofAttempt> = entry
            .attempts
            .iter()
            .map(|a| ProofAttempt {
                proof_id: entry.proof_id.clone(),
                category: entry.category,
                seed: a.seed,
                solver: a.solver.clone(),
                solver_version: a.solver_version.clone(),
                outcome: a.outcome.clone(),
                duration: a.duration,
                timestamp: a.timestamp,
                metadata: HashMap::new(),
            })
            .collect();

        let proof_metrics = ProofMetrics::from_attempts(
            entry.proof_id.clone(),
            entry.category,
            &attempts,
            thresholds,
        );

        metrics.add_proof(&proof_metrics);
    }

    metrics.finalize();
    metrics
}

/// Detect flaky proofs from a list of metrics.
pub fn detect_flaky_proofs(metrics: &[ProofMetrics]) -> List<FlakyProofInfo> {
    metrics
        .iter()
        .filter(|m| m.is_flaky())
        .map(FlakyProofInfo::from_metrics)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_attempt(seed: u64, outcome: ProofOutcome, duration_ms: u64) -> ProofAttempt {
        ProofAttempt {
            proof_id: ProofId::new("test.vr".to_string().into(), "main".to_string().into(), 10, "test".to_string().into()),
            category: ProofCategory::Arithmetic,
            seed,
            solver: "z3".to_string().into(),
            solver_version: "4.12".to_string().into(),
            outcome,
            duration: Duration::from_millis(duration_ms),
            timestamp: Utc::now(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn test_stable_proof_metrics() {
        let thresholds = StabilityThresholds::default();
        let attempts = vec![
            make_attempt(1, ProofOutcome::Verified, 100),
            make_attempt(2, ProofOutcome::Verified, 110),
            make_attempt(3, ProofOutcome::Verified, 105),
            make_attempt(4, ProofOutcome::Verified, 95),
            make_attempt(5, ProofOutcome::Verified, 102),
        ];

        let proof_id = ProofId::new("test.vr".to_string().into(), "main".to_string().into(), 10, "test".to_string().into());
        let metrics = ProofMetrics::from_attempts(
            proof_id,
            ProofCategory::Arithmetic,
            &attempts,
            &thresholds,
        );

        assert_eq!(metrics.stability_status, StabilityStatus::Stable);
        assert_eq!(metrics.stability_percentage, 100.0);
        assert!(metrics.is_stable());
    }

    #[test]
    fn test_flaky_proof_metrics() {
        let thresholds = StabilityThresholds::default();
        let attempts = vec![
            make_attempt(1, ProofOutcome::Verified, 100),
            make_attempt(
                2,
                ProofOutcome::Failed {
                    counterexample: None,
                },
                110,
            ),
            make_attempt(3, ProofOutcome::Verified, 105),
            make_attempt(4, ProofOutcome::Timeout { timeout_ms: 30000 }, 30000),
            make_attempt(5, ProofOutcome::Verified, 102),
        ];

        let proof_id = ProofId::new("test.vr".to_string().into(), "main".to_string().into(), 10, "test".to_string().into());
        let metrics = ProofMetrics::from_attempts(
            proof_id,
            ProofCategory::Quantifier,
            &attempts,
            &thresholds,
        );

        assert!(metrics.is_flaky());
        assert!(metrics.stability_percentage < 100.0);
    }

    #[test]
    fn test_stability_metrics_aggregation() {
        let thresholds = StabilityThresholds::default();
        let mut agg = StabilityMetrics::new();

        // Add a stable proof
        let stable_attempts = vec![
            make_attempt(1, ProofOutcome::Verified, 100),
            make_attempt(2, ProofOutcome::Verified, 100),
            make_attempt(3, ProofOutcome::Verified, 100),
        ];
        let stable_metrics = ProofMetrics::from_attempts(
            ProofId::new("stable.vr".into(), "main".into(), 10, "test".into()),
            ProofCategory::Arithmetic,
            &stable_attempts,
            &thresholds,
        );
        agg.add_proof(&stable_metrics);

        // Add a flaky proof
        let flaky_attempts = vec![
            make_attempt(1, ProofOutcome::Verified, 100),
            make_attempt(
                2,
                ProofOutcome::Failed {
                    counterexample: None,
                },
                100,
            ),
            make_attempt(3, ProofOutcome::Timeout { timeout_ms: 30000 }, 30000),
        ];
        let flaky_metrics = ProofMetrics::from_attempts(
            ProofId::new("flaky.vr".into(), "main".into(), 10, "test".into()),
            ProofCategory::Quantifier,
            &flaky_attempts,
            &thresholds,
        );
        agg.add_proof(&flaky_metrics);

        agg.finalize();

        assert_eq!(agg.total_proofs, 2);
        assert_eq!(agg.stable_count, 1);
        assert_eq!(agg.flaky_proofs.len(), 1);
        assert_eq!(agg.overall_stability, 50.0);
    }
}
