//! Proof regression detection.
//!
//! This module provides infrastructure for detecting regressions in proof
//! behavior across compiler versions, including:
//! - Proofs that previously passed but now fail
//! - Proofs that became flaky
//! - Significant timing regressions

use crate::{
    ProofCategory, ProofId, StabilityError, StabilityStatus,
    config::StabilityThresholds, metrics::ProofMetrics,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use verum_common::{List, Text};

/// Type of regression detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegressionType {
    /// Proof that used to pass now fails
    PassToFail,
    /// Proof that used to fail now passes (might indicate spec change)
    FailToPass,
    /// Proof that was stable is now flaky
    StableToFlaky,
    /// Proof that was flaky is now stable (improvement)
    FlakyToStable,
    /// Significant timing regression
    TimingRegression,
    /// Proof that was stable now times out
    StableToTimeout,
    /// New proof (no baseline)
    NewProof,
}

impl std::fmt::Display for RegressionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::PassToFail => "pass->fail",
            Self::FailToPass => "fail->pass",
            Self::StableToFlaky => "stable->flaky",
            Self::FlakyToStable => "flaky->stable",
            Self::TimingRegression => "timing regression",
            Self::StableToTimeout => "stable->timeout",
            Self::NewProof => "new proof",
        };
        write!(f, "{}", s)
    }
}

/// A single proof regression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofRegression {
    /// Proof identifier
    pub proof_id: ProofId,
    /// Category
    pub category: ProofCategory,
    /// Type of regression
    pub regression_type: RegressionType,
    /// Severity (1-5, 5 being most severe)
    pub severity: u8,
    /// Baseline status
    pub baseline_status: StabilityStatus,
    /// Current status
    pub current_status: StabilityStatus,
    /// Baseline stability percentage
    pub baseline_stability: f64,
    /// Current stability percentage
    pub current_stability: f64,
    /// Baseline average duration
    pub baseline_duration: Duration,
    /// Current average duration
    pub current_duration: Duration,
    /// Duration increase factor
    pub duration_factor: f64,
    /// Baseline compiler version
    pub baseline_version: Option<Text>,
    /// Current compiler version
    pub current_version: Option<Text>,
    /// Detailed message
    pub message: Text,
    /// Detected at
    pub detected_at: DateTime<Utc>,
}

impl ProofRegression {
    /// Check if this is a genuine regression (not an improvement).
    pub fn is_regression(&self) -> bool {
        matches!(
            self.regression_type,
            RegressionType::PassToFail
                | RegressionType::StableToFlaky
                | RegressionType::TimingRegression
                | RegressionType::StableToTimeout
        )
    }

    /// Check if this is an improvement.
    pub fn is_improvement(&self) -> bool {
        matches!(
            self.regression_type,
            RegressionType::FailToPass | RegressionType::FlakyToStable
        )
    }
}

/// Summary of regression analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionReport {
    /// All detected regressions
    pub regressions: List<ProofRegression>,
    /// Regressions by type
    pub by_type: HashMap<Text, usize>,
    /// Regressions by category
    pub by_category: HashMap<ProofCategory, usize>,
    /// Total regression count
    pub total_regressions: usize,
    /// Total improvements
    pub total_improvements: usize,
    /// New proofs without baseline
    pub new_proofs: usize,
    /// Baseline timestamp
    pub baseline_timestamp: Option<DateTime<Utc>>,
    /// Current timestamp
    pub current_timestamp: DateTime<Utc>,
    /// Baseline version
    pub baseline_version: Option<Text>,
    /// Current version
    pub current_version: Option<Text>,
    /// Summary message
    pub summary: Text,
}

impl Default for RegressionReport {
    fn default() -> Self {
        Self {
            regressions: List::new(),
            by_type: HashMap::new(),
            by_category: HashMap::new(),
            total_regressions: 0,
            total_improvements: 0,
            new_proofs: 0,
            baseline_timestamp: None,
            current_timestamp: Utc::now(),
            baseline_version: None,
            current_version: None,
            summary: Text::new(),
        }
    }
}

impl RegressionReport {
    /// Create a new empty report.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a regression to the report.
    pub fn add(&mut self, regression: ProofRegression) {
        let type_key: Text = format!("{}", regression.regression_type).into();
        *self.by_type.entry(type_key).or_default() += 1;
        *self.by_category.entry(regression.category).or_default() += 1;

        if regression.is_regression() {
            self.total_regressions += 1;
        } else if regression.is_improvement() {
            self.total_improvements += 1;
        } else if regression.regression_type == RegressionType::NewProof {
            self.new_proofs += 1;
        }

        self.regressions.push(regression);
    }

    /// Finalize the report.
    pub fn finalize(&mut self) {
        // Sort regressions by severity (highest first)
        self.regressions.sort_by(|a, b| b.severity.cmp(&a.severity));

        // Generate summary
        self.summary = format!(
            "{} regressions, {} improvements, {} new proofs",
            self.total_regressions, self.total_improvements, self.new_proofs
        ).into();
    }

    /// Check if there are any regressions.
    pub fn has_regressions(&self) -> bool {
        self.total_regressions > 0
    }

    /// Get the most severe regressions.
    pub fn most_severe(&self, count: usize) -> List<&ProofRegression> {
        self.regressions
            .iter()
            .filter(|r| r.is_regression())
            .take(count)
            .collect()
    }

    /// Filter regressions by severity threshold.
    pub fn filter_by_severity(&self, min_severity: u8) -> List<&ProofRegression> {
        self.regressions
            .iter()
            .filter(|r| r.severity >= min_severity)
            .collect()
    }
}

/// Baseline proof data for comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofBaseline {
    /// Proof identifier
    pub proof_id: ProofId,
    /// Category
    pub category: ProofCategory,
    /// Stability status at baseline
    pub status: StabilityStatus,
    /// Stability percentage
    pub stability_percentage: f64,
    /// Average duration
    pub average_duration: Duration,
    /// Compiler version
    pub compiler_version: Option<Text>,
    /// Recorded at
    pub recorded_at: DateTime<Utc>,
}

/// Baseline storage for regression detection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BaselineStorage {
    /// Baselines by proof fingerprint
    pub baselines: HashMap<Text, ProofBaseline>,
    /// Compiler version
    pub compiler_version: Option<Text>,
    /// Created at
    pub created_at: DateTime<Utc>,
    /// Last updated
    pub updated_at: DateTime<Utc>,
}

impl BaselineStorage {
    /// Load baseline from file.
    pub fn load(path: &Path) -> Result<Self, StabilityError> {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content)
            .map_err(|e| StabilityError::SerializationError(e.to_string().into()))
    }

    /// Save baseline to file.
    pub fn save(&self, path: &Path) -> Result<(), StabilityError> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| StabilityError::SerializationError(e.to_string().into()))?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Add or update a baseline.
    pub fn update(&mut self, baseline: ProofBaseline) {
        let key = baseline_key(&baseline.proof_id);
        self.baselines.insert(key, baseline);
        self.updated_at = Utc::now();
    }

    /// Get baseline for a proof.
    pub fn get(&self, proof_id: &ProofId) -> Option<&ProofBaseline> {
        let key = baseline_key(proof_id);
        self.baselines.get(&key)
    }
}

/// Regression detector.
pub struct RegressionDetector {
    /// Thresholds for detection
    thresholds: StabilityThresholds,
    /// Timing regression threshold (e.g., 2.0 = 2x slower)
    timing_regression_factor: f64,
    /// Baseline storage
    baseline: Option<BaselineStorage>,
    /// Current compiler version
    current_version: Option<Text>,
}

impl RegressionDetector {
    /// Create a new regression detector.
    pub fn new(thresholds: StabilityThresholds) -> Self {
        Self {
            thresholds,
            timing_regression_factor: 2.0,
            baseline: None,
            current_version: None,
        }
    }

    /// Set the timing regression factor.
    pub fn with_timing_factor(mut self, factor: f64) -> Self {
        self.timing_regression_factor = factor;
        self
    }

    /// Set the baseline.
    pub fn with_baseline(mut self, baseline: BaselineStorage) -> Self {
        self.baseline = Some(baseline);
        self
    }

    /// Set the current version.
    pub fn with_version(mut self, version: Text) -> Self {
        self.current_version = Some(version);
        self
    }

    /// Load baseline from file.
    pub fn load_baseline(&mut self, path: &Path) -> Result<(), StabilityError> {
        self.baseline = Some(BaselineStorage::load(path)?);
        Ok(())
    }

    /// Detect regressions by comparing current metrics to baseline.
    pub fn detect(&self, current_metrics: &[ProofMetrics]) -> RegressionReport {
        let mut report = RegressionReport::new();
        report.current_version = self.current_version.clone();

        let baseline = match &self.baseline {
            Some(b) => b,
            None => {
                // No baseline - all proofs are "new"
                for m in current_metrics {
                    report.add(ProofRegression {
                        proof_id: m.proof_id.clone(),
                        category: m.category,
                        regression_type: RegressionType::NewProof,
                        severity: 1,
                        baseline_status: StabilityStatus::Unknown,
                        current_status: m.stability_status,
                        baseline_stability: 0.0,
                        current_stability: m.stability_percentage,
                        baseline_duration: Duration::ZERO,
                        current_duration: m.mean_duration,
                        duration_factor: 1.0,
                        baseline_version: None,
                        current_version: self.current_version.clone(),
                        message: "No baseline available".to_string().into(),
                        detected_at: Utc::now(),
                    });
                }
                report.finalize();
                return report;
            }
        };

        report.baseline_version = baseline.compiler_version.clone();
        report.baseline_timestamp = Some(baseline.created_at);

        for current in current_metrics {
            if let Some(base) = baseline.get(&current.proof_id) {
                if let Some(regression) = self.compare(base, current) {
                    report.add(regression);
                }
            } else {
                // New proof
                report.add(ProofRegression {
                    proof_id: current.proof_id.clone(),
                    category: current.category,
                    regression_type: RegressionType::NewProof,
                    severity: 1,
                    baseline_status: StabilityStatus::Unknown,
                    current_status: current.stability_status,
                    baseline_stability: 0.0,
                    current_stability: current.stability_percentage,
                    baseline_duration: Duration::ZERO,
                    current_duration: current.mean_duration,
                    duration_factor: 1.0,
                    baseline_version: baseline.compiler_version.clone(),
                    current_version: self.current_version.clone(),
                    message: "New proof not in baseline".to_string().into(),
                    detected_at: Utc::now(),
                });
            }
        }

        report.finalize();
        report
    }

    /// Compare a single proof to its baseline.
    fn compare(&self, baseline: &ProofBaseline, current: &ProofMetrics) -> Option<ProofRegression> {
        let duration_factor = if baseline.average_duration.as_nanos() > 0 {
            current.mean_duration.as_nanos() as f64 / baseline.average_duration.as_nanos() as f64
        } else {
            1.0
        };

        let (regression_type, severity, message) =
            self.determine_regression_type(baseline, current, duration_factor)?;

        Some(ProofRegression {
            proof_id: current.proof_id.clone(),
            category: current.category,
            regression_type,
            severity,
            baseline_status: baseline.status,
            current_status: current.stability_status,
            baseline_stability: baseline.stability_percentage,
            current_stability: current.stability_percentage,
            baseline_duration: baseline.average_duration,
            current_duration: current.mean_duration,
            duration_factor,
            baseline_version: baseline.compiler_version.clone(),
            current_version: self.current_version.clone(),
            message,
            detected_at: Utc::now(),
        })
    }

    /// Determine regression type and severity.
    fn determine_regression_type(
        &self,
        baseline: &ProofBaseline,
        current: &ProofMetrics,
        duration_factor: f64,
    ) -> Option<(RegressionType, u8, Text)> {
        use StabilityStatus::*;

        match (baseline.status, current.stability_status) {
            // Pass to fail is most severe
            (Stable, Flaky) if current.verified_count == 0 => Some((
                RegressionType::PassToFail,
                5,
                format!(
                    "Proof was stable at {:.1}% but now fails",
                    baseline.stability_percentage
                ).into(),
            )),

            // Stable to flaky
            (Stable, Flaky) | (Stable, TimeoutUnstable) => Some((
                RegressionType::StableToFlaky,
                4,
                format!(
                    "Proof stability dropped from {:.1}% to {:.1}%",
                    baseline.stability_percentage, current.stability_percentage
                ).into(),
            )),

            // Timing regression (even if still stable)
            (Stable, Stable) if duration_factor > self.timing_regression_factor => Some((
                RegressionType::TimingRegression,
                3,
                format!(
                    "Duration increased {:.1}x ({:.2}ms -> {:.2}ms)",
                    duration_factor,
                    baseline.average_duration.as_secs_f64() * 1000.0,
                    current.mean_duration.as_secs_f64() * 1000.0
                ).into(),
            )),

            // Improvements (not really regressions but worth noting)
            (Flaky, Stable) | (TimeoutUnstable, Stable) => Some((
                RegressionType::FlakyToStable,
                1,
                format!(
                    "Proof stability improved from {:.1}% to {:.1}%",
                    baseline.stability_percentage, current.stability_percentage
                ).into(),
            )),

            // No significant change
            _ => None,
        }
    }

    /// Create a baseline from current metrics.
    pub fn create_baseline(
        &self,
        metrics: &[ProofMetrics],
        version: Option<Text>,
    ) -> BaselineStorage {
        let now = Utc::now();
        let mut storage = BaselineStorage {
            baselines: HashMap::new(),
            compiler_version: version,
            created_at: now,
            updated_at: now,
        };

        for m in metrics {
            let baseline = ProofBaseline {
                proof_id: m.proof_id.clone(),
                category: m.category,
                status: m.stability_status,
                stability_percentage: m.stability_percentage,
                average_duration: m.mean_duration,
                compiler_version: storage.compiler_version.clone(),
                recorded_at: now,
            };
            storage.update(baseline);
        }

        storage
    }
}

/// Generate a baseline key from proof ID.
fn baseline_key(proof_id: &ProofId) -> Text {
    format!(
        "{}:{}:{}",
        proof_id.source_path, proof_id.scope, proof_id.line
    ).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_baseline(stability: f64, status: StabilityStatus, duration_ms: u64) -> ProofBaseline {
        ProofBaseline {
            proof_id: ProofId::new("test.vr".into(), "main".into(), 10, "test".into()),
            category: ProofCategory::Arithmetic,
            status,
            stability_percentage: stability,
            average_duration: Duration::from_millis(duration_ms),
            compiler_version: Some("v1.0".into()),
            recorded_at: Utc::now(),
        }
    }

    fn make_metrics(stability: f64, status: StabilityStatus, duration_ms: u64) -> ProofMetrics {
        ProofMetrics {
            proof_id: ProofId::new("test.vr".into(), "main".into(), 10, "test".into()),
            category: ProofCategory::Arithmetic,
            stability_status: status,
            stability_percentage: stability,
            attempt_count: 5,
            verified_count: (stability / 20.0) as usize,
            failed_count: 5 - (stability / 20.0) as usize,
            timeout_count: 0,
            error_count: 0,
            unknown_count: 0,
            mean_duration: Duration::from_millis(duration_ms),
            duration_std_dev: Duration::from_millis(10),
            duration_cv: 0.1,
            min_duration: Duration::from_millis(duration_ms - 10),
            max_duration: Duration::from_millis(duration_ms + 10),
            timing_stable: true,
        }
    }

    #[test]
    fn test_stable_to_flaky_regression() {
        let detector = RegressionDetector::new(StabilityThresholds::default());
        let baseline = make_baseline(100.0, StabilityStatus::Stable, 100);
        let current = make_metrics(60.0, StabilityStatus::Flaky, 100);

        let regression = detector.compare(&baseline, &current).unwrap();
        assert_eq!(regression.regression_type, RegressionType::StableToFlaky);
        assert!(regression.severity >= 3);
    }

    #[test]
    fn test_timing_regression() {
        let detector =
            RegressionDetector::new(StabilityThresholds::default()).with_timing_factor(2.0);

        let baseline = make_baseline(100.0, StabilityStatus::Stable, 100);
        let current = make_metrics(100.0, StabilityStatus::Stable, 300);

        let regression = detector.compare(&baseline, &current).unwrap();
        assert_eq!(regression.regression_type, RegressionType::TimingRegression);
        assert!(regression.duration_factor > 2.0);
    }

    #[test]
    fn test_flaky_to_stable_improvement() {
        let detector = RegressionDetector::new(StabilityThresholds::default());
        let baseline = make_baseline(60.0, StabilityStatus::Flaky, 100);
        let current = make_metrics(100.0, StabilityStatus::Stable, 100);

        let regression = detector.compare(&baseline, &current).unwrap();
        assert_eq!(regression.regression_type, RegressionType::FlakyToStable);
        assert!(regression.is_improvement());
    }

    #[test]
    fn test_regression_report() {
        let thresholds = StabilityThresholds::default();
        let baseline = ProofBaseline {
            proof_id: ProofId::new("test.vr".into(), "main".into(), 10, "test".into()),
            category: ProofCategory::Arithmetic,
            status: StabilityStatus::Stable,
            stability_percentage: 100.0,
            average_duration: Duration::from_millis(100),
            compiler_version: Some("v1.0".into()),
            recorded_at: Utc::now(),
        };

        let mut baseline_storage = BaselineStorage::default();
        baseline_storage.update(baseline);

        let current_metrics = vec![make_metrics(60.0, StabilityStatus::Flaky, 100)];

        let detector = RegressionDetector::new(thresholds)
            .with_baseline(baseline_storage)
            .with_version("v2.0".into());

        let report = detector.detect(&current_metrics);
        assert!(report.has_regressions());
        assert_eq!(report.total_regressions, 1);
    }
}
