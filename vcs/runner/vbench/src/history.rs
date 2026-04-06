//! Historical tracking and trend analysis for VCS benchmarks.
//!
//! This module provides functionality for:
//! - Storing benchmark results over time (JSON or SQLite backends)
//! - Detecting performance trends
//! - Automatic regression detection
//! - Statistical significance testing
//! - Baseline management
//!
//! # Storage Backends
//!
//! - **JSON**: Default, human-readable, works without dependencies
//! - **SQLite**: Optional, better for large datasets, concurrent access
//!
//! # Example
//!
//! ```ignore
//! // JSON backend (default)
//! let store = HistoryStore::load_or_create(Path::new("history.json"))?;
//!
//! // SQLite backend
//! let store = SqliteHistoryStore::open(Path::new("history.db"))?;
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::compare::{RegressionConfig, RegressionResult, detect_regression};
use crate::metrics::{BenchmarkCategory, BenchmarkResult, Statistics};

// ============================================================================
// Historical Data Structures
// ============================================================================

/// A single historical data point for a benchmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalPoint {
    /// Unique ID for this data point.
    #[serde(default)]
    pub id: Option<i64>,
    /// Version or commit hash.
    pub version: String,
    /// Git commit hash (if available).
    pub commit: Option<String>,
    /// Git branch (if available).
    #[serde(default)]
    pub branch: Option<String>,
    /// Timestamp when the benchmark was run.
    #[serde(with = "chrono::serde::ts_seconds")]
    pub timestamp: DateTime<Utc>,
    /// Mean execution time in nanoseconds.
    pub mean_ns: f64,
    /// Standard deviation in nanoseconds.
    pub std_dev_ns: f64,
    /// Median execution time in nanoseconds.
    pub median_ns: f64,
    /// 95th percentile in nanoseconds.
    pub p95_ns: f64,
    /// 99th percentile in nanoseconds.
    pub p99_ns: f64,
    /// Number of samples.
    pub sample_count: usize,
    /// Threshold at the time of measurement (if any).
    pub threshold_ns: Option<f64>,
    /// Whether the benchmark passed at the time.
    pub passed: bool,
    /// CI run ID (if applicable).
    #[serde(default)]
    pub ci_run_id: Option<String>,
    /// Machine identifier (for cross-machine comparisons).
    #[serde(default)]
    pub machine_id: Option<String>,
    /// Additional metadata.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl HistoricalPoint {
    /// Create a new historical point from a benchmark result.
    pub fn from_result(result: &BenchmarkResult, version: &str, commit: Option<&str>) -> Self {
        Self {
            id: None,
            version: version.to_string(),
            commit: commit.map(|s| s.to_string()),
            branch: None,
            timestamp: Utc::now(),
            mean_ns: result.statistics.mean_ns,
            std_dev_ns: result.statistics.std_dev_ns,
            median_ns: result.statistics.median_ns,
            p95_ns: result.statistics.p95_ns,
            p99_ns: result.statistics.p99_ns,
            sample_count: result.statistics.count,
            threshold_ns: result.threshold_ns,
            passed: result.passed,
            ci_run_id: None,
            machine_id: None,
            metadata: result.metadata.clone(),
        }
    }

    /// Create a builder for historical points.
    pub fn builder(result: &BenchmarkResult, version: &str) -> HistoricalPointBuilder {
        HistoricalPointBuilder {
            point: Self::from_result(result, version, None),
        }
    }
}

/// Builder for HistoricalPoint.
pub struct HistoricalPointBuilder {
    point: HistoricalPoint,
}

impl HistoricalPointBuilder {
    /// Set the commit hash.
    pub fn commit(mut self, commit: &str) -> Self {
        self.point.commit = Some(commit.to_string());
        self
    }

    /// Set the branch.
    pub fn branch(mut self, branch: &str) -> Self {
        self.point.branch = Some(branch.to_string());
        self
    }

    /// Set the CI run ID.
    pub fn ci_run_id(mut self, run_id: &str) -> Self {
        self.point.ci_run_id = Some(run_id.to_string());
        self
    }

    /// Set the machine ID.
    pub fn machine_id(mut self, machine_id: &str) -> Self {
        self.point.machine_id = Some(machine_id.to_string());
        self
    }

    /// Add metadata.
    pub fn metadata(mut self, key: &str, value: &str) -> Self {
        self.point
            .metadata
            .insert(key.to_string(), value.to_string());
        self
    }

    /// Build the historical point.
    pub fn build(self) -> HistoricalPoint {
        self.point
    }
}

/// Historical data for a single benchmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkHistory {
    /// Benchmark name.
    pub name: String,
    /// Benchmark category.
    pub category: BenchmarkCategory,
    /// Historical data points, ordered by timestamp.
    pub points: Vec<HistoricalPoint>,
    /// Maximum number of points to retain.
    #[serde(default = "default_max_points")]
    pub max_points: usize,
}

fn default_max_points() -> usize {
    1000
}

impl BenchmarkHistory {
    /// Create a new history for a benchmark.
    pub fn new(name: &str, category: BenchmarkCategory) -> Self {
        Self {
            name: name.to_string(),
            category,
            points: Vec::new(),
            max_points: default_max_points(),
        }
    }

    /// Add a new data point.
    pub fn add_point(&mut self, point: HistoricalPoint) {
        self.points.push(point);

        // Sort by timestamp
        self.points.sort_by_key(|p| p.timestamp);

        // Trim to max points
        if self.points.len() > self.max_points {
            self.points.drain(0..self.points.len() - self.max_points);
        }
    }

    /// Get the most recent point.
    pub fn latest(&self) -> Option<&HistoricalPoint> {
        self.points.last()
    }

    /// Get points within a time range.
    pub fn points_in_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<&HistoricalPoint> {
        self.points
            .iter()
            .filter(|p| p.timestamp >= start && p.timestamp <= end)
            .collect()
    }

    /// Get points for a specific version.
    pub fn points_for_version(&self, version: &str) -> Vec<&HistoricalPoint> {
        self.points
            .iter()
            .filter(|p| p.version == version)
            .collect()
    }

    /// Get points for a specific branch.
    pub fn points_for_branch(&self, branch: &str) -> Vec<&HistoricalPoint> {
        self.points
            .iter()
            .filter(|p| p.branch.as_deref() == Some(branch))
            .collect()
    }

    /// Get the last N points.
    pub fn last_n(&self, n: usize) -> &[HistoricalPoint] {
        let start = self.points.len().saturating_sub(n);
        &self.points[start..]
    }

    /// Calculate statistics over historical points.
    pub fn historical_stats(&self) -> Option<HistoricalStats> {
        if self.points.is_empty() {
            return None;
        }

        let means: Vec<f64> = self.points.iter().map(|p| p.mean_ns).collect();
        let n = means.len();

        let overall_mean = means.iter().sum::<f64>() / n as f64;
        let overall_min = means.iter().cloned().fold(f64::INFINITY, f64::min);
        let overall_max = means.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        let variance = means
            .iter()
            .map(|x| (x - overall_mean).powi(2))
            .sum::<f64>()
            / n as f64;
        let std_dev = variance.sqrt();

        // Calculate trend (simple linear regression)
        let trend = if n > 1 {
            let x_mean = (n - 1) as f64 / 2.0;
            let mut numerator = 0.0;
            let mut denominator = 0.0;

            for (i, &y) in means.iter().enumerate() {
                let x = i as f64;
                numerator += (x - x_mean) * (y - overall_mean);
                denominator += (x - x_mean).powi(2);
            }

            if denominator != 0.0 {
                Some(numerator / denominator)
            } else {
                None
            }
        } else {
            None
        };

        // Calculate R-squared for trend
        let r_squared = trend.and_then(|slope| {
            if n < 3 {
                return None;
            }
            let intercept = overall_mean - slope * (n as f64 - 1.0) / 2.0;
            let ss_tot: f64 = means.iter().map(|y| (y - overall_mean).powi(2)).sum();
            let ss_res: f64 = means
                .iter()
                .enumerate()
                .map(|(i, y)| {
                    let y_pred = intercept + slope * i as f64;
                    (y - y_pred).powi(2)
                })
                .sum();
            if ss_tot > 0.0 {
                Some(1.0 - ss_res / ss_tot)
            } else {
                None
            }
        });

        Some(HistoricalStats {
            count: n,
            mean_of_means: overall_mean,
            min_mean: overall_min,
            max_mean: overall_max,
            std_dev_of_means: std_dev,
            trend_slope: trend,
            trend_r_squared: r_squared,
            first_timestamp: self.points.first().map(|p| p.timestamp),
            last_timestamp: self.points.last().map(|p| p.timestamp),
        })
    }

    /// Calculate statistics over the last N points only.
    pub fn recent_stats(&self, n: usize) -> Option<HistoricalStats> {
        if self.points.is_empty() {
            return None;
        }

        let recent = self.last_n(n);
        let temp_history = BenchmarkHistory {
            name: self.name.clone(),
            category: self.category,
            points: recent.to_vec(),
            max_points: self.max_points,
        };
        temp_history.historical_stats()
    }
}

/// Statistics over historical benchmark data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalStats {
    /// Number of historical points.
    pub count: usize,
    /// Mean of all mean values.
    pub mean_of_means: f64,
    /// Minimum mean value ever recorded.
    pub min_mean: f64,
    /// Maximum mean value ever recorded.
    pub max_mean: f64,
    /// Standard deviation of mean values.
    pub std_dev_of_means: f64,
    /// Trend slope (positive = getting slower, negative = getting faster).
    pub trend_slope: Option<f64>,
    /// R-squared value for trend (goodness of fit).
    #[serde(default)]
    pub trend_r_squared: Option<f64>,
    /// First recorded timestamp.
    pub first_timestamp: Option<DateTime<Utc>>,
    /// Last recorded timestamp.
    pub last_timestamp: Option<DateTime<Utc>>,
}

impl HistoricalStats {
    /// Check if performance is trending worse.
    pub fn is_trending_worse(&self, threshold: f64) -> bool {
        self.trend_slope.map(|s| s > threshold).unwrap_or(false)
    }

    /// Check if performance is trending better.
    pub fn is_trending_better(&self, threshold: f64) -> bool {
        self.trend_slope.map(|s| s < -threshold).unwrap_or(false)
    }

    /// Get the percentage change from min to max.
    pub fn variability_percent(&self) -> f64 {
        if self.min_mean != 0.0 {
            ((self.max_mean - self.min_mean) / self.min_mean) * 100.0
        } else {
            0.0
        }
    }

    /// Check if the trend is statistically reliable.
    pub fn is_trend_reliable(&self) -> bool {
        // Require at least 10 data points and R-squared > 0.5
        self.count >= 10 && self.trend_r_squared.map(|r| r > 0.5).unwrap_or(false)
    }
}

// ============================================================================
// History Store (JSON Backend)
// ============================================================================

/// Persistent storage for benchmark history (JSON backend).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryStore {
    /// Path to the history file.
    #[serde(skip)]
    path: Option<PathBuf>,
    /// History for each benchmark.
    pub benchmarks: HashMap<String, BenchmarkHistory>,
    /// Store metadata.
    pub metadata: StoreMetadata,
    /// Baselines for comparison.
    #[serde(default)]
    pub baselines: HashMap<String, Baseline>,
}

/// Metadata about the history store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreMetadata {
    /// Store format version.
    pub version: u32,
    /// Creation timestamp.
    #[serde(with = "chrono::serde::ts_seconds")]
    pub created: DateTime<Utc>,
    /// Last update timestamp.
    #[serde(with = "chrono::serde::ts_seconds")]
    pub updated: DateTime<Utc>,
    /// Project name.
    pub project: String,
    /// Machine identifier.
    #[serde(default)]
    pub machine_id: Option<String>,
}

impl Default for StoreMetadata {
    fn default() -> Self {
        Self {
            version: 2,
            created: Utc::now(),
            updated: Utc::now(),
            project: "verum".to_string(),
            machine_id: None,
        }
    }
}

/// A saved baseline for comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Baseline {
    /// Baseline name/identifier.
    pub name: String,
    /// Version or commit that this baseline represents.
    pub version: String,
    /// When this baseline was created.
    #[serde(with = "chrono::serde::ts_seconds")]
    pub created: DateTime<Utc>,
    /// Description of this baseline.
    #[serde(default)]
    pub description: Option<String>,
    /// Benchmark data for this baseline.
    pub benchmarks: HashMap<String, HistoricalPoint>,
}

impl Baseline {
    /// Create a new baseline from current results.
    pub fn from_results(
        name: &str,
        version: &str,
        results: &[BenchmarkResult],
        description: Option<&str>,
    ) -> Self {
        let mut benchmarks = HashMap::new();
        for result in results {
            let point = HistoricalPoint::from_result(result, version, None);
            benchmarks.insert(result.name.clone(), point);
        }

        Self {
            name: name.to_string(),
            version: version.to_string(),
            created: Utc::now(),
            description: description.map(|s| s.to_string()),
            benchmarks,
        }
    }
}

impl HistoryStore {
    /// Create a new empty history store.
    pub fn new() -> Self {
        Self {
            path: None,
            benchmarks: HashMap::new(),
            metadata: StoreMetadata::default(),
            baselines: HashMap::new(),
        }
    }

    /// Load a history store from a file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .context(format!("Failed to read history file: {}", path.display()))?;

        let mut store: Self =
            serde_json::from_str(&content).context("Failed to parse history file")?;

        store.path = Some(path.to_path_buf());
        Ok(store)
    }

    /// Load or create a new history store.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            let mut store = Self::new();
            store.path = Some(path.to_path_buf());
            Ok(store)
        }
    }

    /// Save the history store to its file.
    pub fn save(&mut self) -> Result<()> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No path set for history store"))?;

        self.metadata.updated = Utc::now();

        let content = serde_json::to_string_pretty(self).context("Failed to serialize history")?;

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(path, content)
            .context(format!("Failed to write history file: {}", path.display()))?;

        Ok(())
    }

    /// Add benchmark results to the history.
    pub fn add_results(
        &mut self,
        results: &[BenchmarkResult],
        version: &str,
        commit: Option<&str>,
    ) {
        for result in results {
            let history = self
                .benchmarks
                .entry(result.name.clone())
                .or_insert_with(|| BenchmarkHistory::new(&result.name, result.category));

            let point = HistoricalPoint::from_result(result, version, commit);
            history.add_point(point);
        }
    }

    /// Add results with additional context.
    pub fn add_results_with_context(
        &mut self,
        results: &[BenchmarkResult],
        version: &str,
        commit: Option<&str>,
        branch: Option<&str>,
        ci_run_id: Option<&str>,
        machine_id: Option<&str>,
    ) {
        for result in results {
            let history = self
                .benchmarks
                .entry(result.name.clone())
                .or_insert_with(|| BenchmarkHistory::new(&result.name, result.category));

            let mut builder = HistoricalPoint::builder(result, version);
            if let Some(c) = commit {
                builder = builder.commit(c);
            }
            if let Some(b) = branch {
                builder = builder.branch(b);
            }
            if let Some(ci) = ci_run_id {
                builder = builder.ci_run_id(ci);
            }
            if let Some(m) = machine_id {
                builder = builder.machine_id(m);
            }

            history.add_point(builder.build());
        }
    }

    /// Get history for a specific benchmark.
    pub fn get(&self, name: &str) -> Option<&BenchmarkHistory> {
        self.benchmarks.get(name)
    }

    /// Get all benchmark names.
    pub fn benchmark_names(&self) -> Vec<&str> {
        self.benchmarks.keys().map(|s| s.as_str()).collect()
    }

    /// Create a baseline from current results.
    pub fn create_baseline(
        &mut self,
        name: &str,
        version: &str,
        results: &[BenchmarkResult],
        description: Option<&str>,
    ) {
        let baseline = Baseline::from_results(name, version, results, description);
        self.baselines.insert(name.to_string(), baseline);
    }

    /// Get a baseline by name.
    pub fn get_baseline(&self, name: &str) -> Option<&Baseline> {
        self.baselines.get(name)
    }

    /// List all baseline names.
    pub fn baseline_names(&self) -> Vec<&str> {
        self.baselines.keys().map(|s| s.as_str()).collect()
    }

    /// Set the default baseline.
    pub fn set_default_baseline(&mut self, name: &str) -> Result<()> {
        if !self.baselines.contains_key(name) {
            return Err(anyhow::anyhow!("Baseline '{}' not found", name));
        }
        self.metadata.machine_id = Some(format!("default_baseline:{}", name));
        Ok(())
    }

    /// Detect regressions compared to the baseline.
    pub fn detect_regressions(
        &self,
        current_results: &[BenchmarkResult],
        config: &RegressionConfig,
    ) -> Vec<RegressionResult> {
        let mut regressions = Vec::new();

        for result in current_results {
            if let Some(history) = self.get(&result.name) {
                if let Some(baseline) = history.latest() {
                    // Convert historical point to a BenchmarkResult for comparison
                    let baseline_result = BenchmarkResult::new(
                        result.name.clone(),
                        result.category,
                        Statistics {
                            count: baseline.sample_count,
                            min_ns: baseline.mean_ns * 0.9, // Approximate
                            max_ns: baseline.mean_ns * 1.1, // Approximate
                            mean_ns: baseline.mean_ns,
                            median_ns: baseline.median_ns,
                            std_dev_ns: baseline.std_dev_ns,
                            cv: baseline.std_dev_ns / baseline.mean_ns,
                            p5_ns: baseline.mean_ns * 0.92,
                            p25_ns: baseline.mean_ns * 0.95,
                            p75_ns: baseline.mean_ns * 1.05,
                            p95_ns: baseline.p95_ns,
                            p99_ns: baseline.p99_ns,
                            iqr_ns: baseline.mean_ns * 0.1,
                            total_duration: std::time::Duration::from_nanos(
                                (baseline.mean_ns * baseline.sample_count as f64) as u64,
                            ),
                        },
                        baseline.threshold_ns,
                    );

                    let regression = detect_regression(result, &baseline_result, config);
                    if regression.is_regression {
                        regressions.push(regression);
                    }
                }
            }
        }

        regressions
    }

    /// Detect regressions compared to a named baseline.
    pub fn detect_regressions_vs_baseline(
        &self,
        current_results: &[BenchmarkResult],
        baseline_name: &str,
        config: &RegressionConfig,
    ) -> Result<Vec<RegressionResult>> {
        let baseline = self
            .get_baseline(baseline_name)
            .ok_or_else(|| anyhow::anyhow!("Baseline '{}' not found", baseline_name))?;

        let mut regressions = Vec::new();

        for result in current_results {
            if let Some(baseline_point) = baseline.benchmarks.get(&result.name) {
                let baseline_result = BenchmarkResult::new(
                    result.name.clone(),
                    result.category,
                    Statistics {
                        count: baseline_point.sample_count,
                        min_ns: baseline_point.mean_ns * 0.9,
                        max_ns: baseline_point.mean_ns * 1.1,
                        mean_ns: baseline_point.mean_ns,
                        median_ns: baseline_point.median_ns,
                        std_dev_ns: baseline_point.std_dev_ns,
                        cv: baseline_point.std_dev_ns / baseline_point.mean_ns,
                        p5_ns: baseline_point.mean_ns * 0.92,
                        p25_ns: baseline_point.mean_ns * 0.95,
                        p75_ns: baseline_point.mean_ns * 1.05,
                        p95_ns: baseline_point.p95_ns,
                        p99_ns: baseline_point.p99_ns,
                        iqr_ns: baseline_point.mean_ns * 0.1,
                        total_duration: std::time::Duration::from_nanos(
                            (baseline_point.mean_ns * baseline_point.sample_count as f64) as u64,
                        ),
                    },
                    baseline_point.threshold_ns,
                );

                let regression = detect_regression(result, &baseline_result, config);
                if regression.is_regression {
                    regressions.push(regression);
                }
            }
        }

        Ok(regressions)
    }

    /// Get trend analysis for all benchmarks.
    pub fn analyze_trends(&self) -> Vec<TrendAnalysis> {
        self.benchmarks
            .values()
            .filter_map(|history| {
                history.historical_stats().map(|stats| TrendAnalysis {
                    name: history.name.clone(),
                    category: history.category,
                    stats,
                    data_points: history.points.len(),
                })
            })
            .collect()
    }

    /// Prune old data points.
    pub fn prune(&mut self, max_age_days: i64) {
        let cutoff = Utc::now() - chrono::Duration::days(max_age_days);

        for history in self.benchmarks.values_mut() {
            history.points.retain(|p| p.timestamp >= cutoff);
        }

        // Remove empty histories
        self.benchmarks.retain(|_, h| !h.points.is_empty());
    }

    /// Export to CSV format.
    pub fn export_csv(&self, benchmark_name: Option<&str>) -> String {
        let mut csv = String::from(
            "benchmark,version,timestamp,mean_ns,std_dev_ns,median_ns,p95_ns,p99_ns,sample_count,passed\n",
        );

        let histories: Box<dyn Iterator<Item = &BenchmarkHistory>> = match benchmark_name {
            Some(name) => Box::new(self.benchmarks.get(name).into_iter()),
            None => Box::new(self.benchmarks.values()),
        };

        for history in histories {
            for point in &history.points {
                csv.push_str(&format!(
                    "{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{},{}\n",
                    history.name,
                    point.version,
                    point.timestamp.format("%Y-%m-%d %H:%M:%S"),
                    point.mean_ns,
                    point.std_dev_ns,
                    point.median_ns,
                    point.p95_ns,
                    point.p99_ns,
                    point.sample_count,
                    point.passed,
                ));
            }
        }

        csv
    }

    /// Get statistics summary for all benchmarks.
    pub fn summary(&self) -> HistorySummary {
        let total_benchmarks = self.benchmarks.len();
        let total_data_points: usize = self.benchmarks.values().map(|h| h.points.len()).sum();

        let trends = self.analyze_trends();
        let improving = trends
            .iter()
            .filter(|t| t.stats.is_trending_better(1.0))
            .count();
        let degrading = trends
            .iter()
            .filter(|t| t.stats.is_trending_worse(1.0))
            .count();
        let stable = total_benchmarks - improving - degrading;

        let oldest = self
            .benchmarks
            .values()
            .filter_map(|h| h.points.first())
            .map(|p| p.timestamp)
            .min();

        let newest = self
            .benchmarks
            .values()
            .filter_map(|h| h.points.last())
            .map(|p| p.timestamp)
            .max();

        HistorySummary {
            total_benchmarks,
            total_data_points,
            total_baselines: self.baselines.len(),
            improving_count: improving,
            degrading_count: degrading,
            stable_count: stable,
            oldest_data: oldest,
            newest_data: newest,
        }
    }
}

impl Default for HistoryStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary statistics for the history store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistorySummary {
    /// Total number of benchmarks tracked.
    pub total_benchmarks: usize,
    /// Total number of data points.
    pub total_data_points: usize,
    /// Total number of saved baselines.
    pub total_baselines: usize,
    /// Number of benchmarks with improving trend.
    pub improving_count: usize,
    /// Number of benchmarks with degrading trend.
    pub degrading_count: usize,
    /// Number of benchmarks with stable performance.
    pub stable_count: usize,
    /// Oldest data point timestamp.
    pub oldest_data: Option<DateTime<Utc>>,
    /// Newest data point timestamp.
    pub newest_data: Option<DateTime<Utc>>,
}

// ============================================================================
// Trend Analysis
// ============================================================================

/// Analysis of performance trends for a benchmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendAnalysis {
    /// Benchmark name.
    pub name: String,
    /// Benchmark category.
    pub category: BenchmarkCategory,
    /// Historical statistics.
    pub stats: HistoricalStats,
    /// Number of data points.
    pub data_points: usize,
}

impl TrendAnalysis {
    /// Get a human-readable trend description.
    pub fn trend_description(&self) -> String {
        match self.stats.trend_slope {
            Some(slope) if slope > 1.0 => format!("Degrading ({:+.2}ns/run)", slope),
            Some(slope) if slope < -1.0 => format!("Improving ({:+.2}ns/run)", slope),
            Some(_) => "Stable".to_string(),
            None => "Insufficient data".to_string(),
        }
    }

    /// Get trend severity (for CI integration).
    pub fn severity(&self) -> TrendSeverity {
        match self.stats.trend_slope {
            Some(slope) if slope > 10.0 => TrendSeverity::Critical,
            Some(slope) if slope > 5.0 => TrendSeverity::Warning,
            Some(slope) if slope > 1.0 => TrendSeverity::Minor,
            Some(slope) if slope < -1.0 => TrendSeverity::Improvement,
            _ => TrendSeverity::Stable,
        }
    }

    /// Check if trend is reliable (enough data, good R-squared).
    pub fn is_reliable(&self) -> bool {
        self.stats.is_trend_reliable()
    }
}

/// Severity level for trend analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrendSeverity {
    /// Performance is improving.
    Improvement,
    /// Performance is stable.
    Stable,
    /// Minor degradation (< 5ns/run).
    Minor,
    /// Warning level degradation (5-10ns/run).
    Warning,
    /// Critical degradation (> 10ns/run).
    Critical,
}

impl std::fmt::Display for TrendSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Improvement => write!(f, "improvement"),
            Self::Stable => write!(f, "stable"),
            Self::Minor => write!(f, "minor"),
            Self::Warning => write!(f, "warning"),
            Self::Critical => write!(f, "critical"),
        }
    }
}

// ============================================================================
// Moving Average
// ============================================================================

/// Calculate exponential moving average for smoothing.
pub fn exponential_moving_average(values: &[f64], alpha: f64) -> Vec<f64> {
    if values.is_empty() {
        return Vec::new();
    }

    let mut ema = Vec::with_capacity(values.len());
    ema.push(values[0]);

    for i in 1..values.len() {
        let prev = ema[i - 1];
        ema.push(alpha * values[i] + (1.0 - alpha) * prev);
    }

    ema
}

/// Calculate simple moving average.
pub fn simple_moving_average(values: &[f64], window: usize) -> Vec<f64> {
    if values.is_empty() || window == 0 {
        return Vec::new();
    }

    values
        .windows(window.min(values.len()))
        .map(|w| w.iter().sum::<f64>() / w.len() as f64)
        .collect()
}

/// Calculate weighted moving average.
pub fn weighted_moving_average(values: &[f64], window: usize) -> Vec<f64> {
    if values.is_empty() || window == 0 {
        return Vec::new();
    }

    let weights: Vec<f64> = (1..=window).map(|i| i as f64).collect();
    let weight_sum: f64 = weights.iter().sum();

    values
        .windows(window.min(values.len()))
        .map(|w| {
            w.iter()
                .zip(weights.iter())
                .map(|(v, wt)| v * wt)
                .sum::<f64>()
                / weight_sum
        })
        .collect()
}

// ============================================================================
// Anomaly Detection
// ============================================================================

/// Detect anomalies in benchmark results.
pub struct AnomalyDetector {
    /// Number of standard deviations for anomaly threshold.
    pub threshold_sigma: f64,
    /// Minimum data points required.
    pub min_samples: usize,
    /// Use robust statistics (median/MAD instead of mean/std).
    pub use_robust: bool,
}

impl Default for AnomalyDetector {
    fn default() -> Self {
        Self {
            threshold_sigma: 3.0,
            min_samples: 10,
            use_robust: true,
        }
    }
}

impl AnomalyDetector {
    /// Create a new anomaly detector.
    pub fn new(threshold_sigma: f64, min_samples: usize) -> Self {
        Self {
            threshold_sigma,
            min_samples,
            use_robust: true,
        }
    }

    /// Use non-robust statistics (mean/std).
    pub fn with_non_robust(mut self) -> Self {
        self.use_robust = false;
        self
    }

    /// Detect anomalies in a benchmark history.
    pub fn detect(&self, history: &BenchmarkHistory) -> Vec<Anomaly> {
        if history.points.len() < self.min_samples {
            return Vec::new();
        }

        let means: Vec<f64> = history.points.iter().map(|p| p.mean_ns).collect();

        let (center, spread) = if self.use_robust {
            // Use median and MAD (Median Absolute Deviation)
            let median = Self::median(&means);
            let deviations: Vec<f64> = means.iter().map(|x| (x - median).abs()).collect();
            let mad = Self::median(&deviations) * 1.4826; // Scale factor for normal distribution
            (median, mad)
        } else {
            // Use mean and standard deviation
            let n = means.len();
            let mean = means.iter().sum::<f64>() / n as f64;
            let variance = means.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
            (mean, variance.sqrt())
        };

        if spread == 0.0 {
            return Vec::new();
        }

        let lower_bound = center - self.threshold_sigma * spread;
        let upper_bound = center + self.threshold_sigma * spread;

        history
            .points
            .iter()
            .enumerate()
            .filter_map(|(i, point)| {
                let z_score = (point.mean_ns - center) / spread;
                if point.mean_ns < lower_bound {
                    Some(Anomaly {
                        index: i,
                        point: point.clone(),
                        kind: AnomalyKind::UnexpectedlyFast,
                        z_score,
                        threshold_sigma: self.threshold_sigma,
                    })
                } else if point.mean_ns > upper_bound {
                    Some(Anomaly {
                        index: i,
                        point: point.clone(),
                        kind: AnomalyKind::UnexpectedlySlow,
                        z_score,
                        threshold_sigma: self.threshold_sigma,
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// Calculate median.
    fn median(data: &[f64]) -> f64 {
        if data.is_empty() {
            return 0.0;
        }
        let mut sorted = data.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mid = sorted.len() / 2;
        if sorted.len() % 2 == 0 {
            (sorted[mid - 1] + sorted[mid]) / 2.0
        } else {
            sorted[mid]
        }
    }
}

/// A detected anomaly in benchmark data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anomaly {
    /// Index in the history.
    pub index: usize,
    /// The anomalous data point.
    pub point: HistoricalPoint,
    /// Kind of anomaly.
    pub kind: AnomalyKind,
    /// Z-score (number of standard deviations from center).
    pub z_score: f64,
    /// Threshold used for detection.
    pub threshold_sigma: f64,
}

impl Anomaly {
    /// Get a human-readable description.
    pub fn description(&self) -> String {
        let change = match self.kind {
            AnomalyKind::UnexpectedlyFast => "unexpectedly fast",
            AnomalyKind::UnexpectedlySlow => "unexpectedly slow",
        };
        format!(
            "Version {} was {} ({:.1} sigma, {:.2}ns)",
            self.point.version,
            change,
            self.z_score.abs(),
            self.point.mean_ns
        )
    }
}

/// Kind of anomaly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnomalyKind {
    /// Performance was unexpectedly fast.
    UnexpectedlyFast,
    /// Performance was unexpectedly slow.
    UnexpectedlySlow,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn create_test_result(name: &str, mean_ns: f64) -> BenchmarkResult {
        BenchmarkResult::new(
            name.to_string(),
            BenchmarkCategory::Micro,
            Statistics {
                count: 100,
                min_ns: mean_ns * 0.9,
                max_ns: mean_ns * 1.1,
                mean_ns,
                median_ns: mean_ns,
                std_dev_ns: mean_ns * 0.05,
                cv: 0.05,
                p5_ns: mean_ns * 0.92,
                p25_ns: mean_ns * 0.95,
                p75_ns: mean_ns * 1.05,
                p95_ns: mean_ns * 1.08,
                p99_ns: mean_ns * 1.09,
                iqr_ns: mean_ns * 0.1,
                total_duration: Duration::from_nanos((mean_ns * 100.0) as u64),
            },
            Some(mean_ns * 1.2),
        )
    }

    #[test]
    fn test_historical_point_creation() {
        let result = create_test_result("test", 100.0);
        let point = HistoricalPoint::from_result(&result, "1.0.0", Some("abc123"));

        assert_eq!(point.version, "1.0.0");
        assert_eq!(point.commit, Some("abc123".to_string()));
        assert_eq!(point.mean_ns, 100.0);
    }

    #[test]
    fn test_historical_point_builder() {
        let result = create_test_result("test", 100.0);
        let point = HistoricalPoint::builder(&result, "1.0.0")
            .commit("abc123")
            .branch("main")
            .ci_run_id("run-123")
            .machine_id("machine-1")
            .metadata("key", "value")
            .build();

        assert_eq!(point.commit, Some("abc123".to_string()));
        assert_eq!(point.branch, Some("main".to_string()));
        assert_eq!(point.ci_run_id, Some("run-123".to_string()));
        assert_eq!(point.machine_id, Some("machine-1".to_string()));
        assert_eq!(point.metadata.get("key"), Some(&"value".to_string()));
    }

    #[test]
    fn test_benchmark_history() {
        let mut history = BenchmarkHistory::new("test", BenchmarkCategory::Micro);

        for i in 0..10 {
            let result = create_test_result("test", 100.0 + i as f64);
            let point = HistoricalPoint::from_result(&result, &format!("1.0.{}", i), None);
            history.add_point(point);
        }

        assert_eq!(history.points.len(), 10);
        assert!(history.latest().is_some());
        assert_eq!(history.latest().unwrap().mean_ns, 109.0);
    }

    #[test]
    fn test_historical_stats() {
        let mut history = BenchmarkHistory::new("test", BenchmarkCategory::Micro);

        for i in 0..10 {
            let result = create_test_result("test", 100.0 + i as f64);
            let point = HistoricalPoint::from_result(&result, &format!("1.0.{}", i), None);
            history.add_point(point);
        }

        let stats = history.historical_stats().unwrap();
        assert_eq!(stats.count, 10);
        assert!(stats.mean_of_means > 100.0);
        assert!(stats.trend_slope.is_some());
        assert!(stats.trend_slope.unwrap() > 0.0); // Values increasing
    }

    #[test]
    fn test_history_store() {
        let mut store = HistoryStore::new();

        let results = vec![
            create_test_result("bench1", 100.0),
            create_test_result("bench2", 200.0),
        ];

        store.add_results(&results, "1.0.0", Some("abc123"));
        store.add_results(&results, "1.0.1", Some("def456"));

        assert_eq!(store.benchmarks.len(), 2);
        assert_eq!(store.get("bench1").unwrap().points.len(), 2);
    }

    #[test]
    fn test_baseline_creation() {
        let mut store = HistoryStore::new();
        let results = vec![
            create_test_result("bench1", 100.0),
            create_test_result("bench2", 200.0),
        ];

        store.create_baseline("release-1.0", "1.0.0", &results, Some("Initial release"));

        let baseline = store.get_baseline("release-1.0").unwrap();
        assert_eq!(baseline.name, "release-1.0");
        assert_eq!(baseline.benchmarks.len(), 2);
        assert_eq!(baseline.description, Some("Initial release".to_string()));
    }

    #[test]
    fn test_trend_analysis() {
        let mut store = HistoryStore::new();

        for i in 0..10 {
            let results = vec![create_test_result("bench", 100.0 + i as f64 * 5.0)];
            store.add_results(&results, &format!("1.0.{}", i), None);
        }

        let trends = store.analyze_trends();
        assert_eq!(trends.len(), 1);
        assert!(trends[0].stats.trend_slope.is_some());
    }

    #[test]
    fn test_summary() {
        let mut store = HistoryStore::new();

        for i in 0..5 {
            let results = vec![
                create_test_result("bench1", 100.0 + i as f64),
                create_test_result("bench2", 200.0 - i as f64),
            ];
            store.add_results(&results, &format!("1.0.{}", i), None);
        }

        let summary = store.summary();
        assert_eq!(summary.total_benchmarks, 2);
        assert_eq!(summary.total_data_points, 10);
    }

    #[test]
    fn test_csv_export() {
        let mut store = HistoryStore::new();
        let results = vec![create_test_result("bench1", 100.0)];
        store.add_results(&results, "1.0.0", None);

        let csv = store.export_csv(None);
        assert!(csv.contains("bench1"));
        assert!(csv.contains("1.0.0"));
        assert!(csv.contains("100.00"));
    }

    #[test]
    fn test_moving_average() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];

        let ema = exponential_moving_average(&values, 0.5);
        assert_eq!(ema.len(), 5);

        let sma = simple_moving_average(&values, 3);
        assert_eq!(sma.len(), 3);
        assert!((sma[0] - 2.0).abs() < 0.001); // (1+2+3)/3 = 2

        let wma = weighted_moving_average(&values, 3);
        assert_eq!(wma.len(), 3);
    }

    #[test]
    fn test_anomaly_detection() {
        let mut history = BenchmarkHistory::new("test", BenchmarkCategory::Micro);

        // Add normal points
        for i in 0..20 {
            let result = create_test_result("test", 100.0 + (i % 3) as f64);
            let point = HistoricalPoint::from_result(&result, &format!("1.0.{}", i), None);
            history.add_point(point);
        }

        // Add an anomalous point
        let anomaly_result = create_test_result("test", 500.0); // Way above normal
        let anomaly_point = HistoricalPoint::from_result(&anomaly_result, "1.0.20", None);
        history.add_point(anomaly_point);

        let detector = AnomalyDetector::default();
        let anomalies = detector.detect(&history);

        assert!(!anomalies.is_empty());
        assert_eq!(anomalies[0].kind, AnomalyKind::UnexpectedlySlow);
    }

    #[test]
    fn test_anomaly_description() {
        let point = HistoricalPoint {
            id: None,
            version: "1.0.0".to_string(),
            commit: None,
            branch: None,
            timestamp: Utc::now(),
            mean_ns: 500.0,
            std_dev_ns: 10.0,
            median_ns: 500.0,
            p95_ns: 520.0,
            p99_ns: 530.0,
            sample_count: 100,
            threshold_ns: None,
            passed: true,
            ci_run_id: None,
            machine_id: None,
            metadata: HashMap::new(),
        };

        let anomaly = Anomaly {
            index: 0,
            point,
            kind: AnomalyKind::UnexpectedlySlow,
            z_score: 4.5,
            threshold_sigma: 3.0,
        };

        let desc = anomaly.description();
        assert!(desc.contains("1.0.0"));
        assert!(desc.contains("unexpectedly slow"));
    }
}
