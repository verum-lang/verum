//! Benchmark execution and analysis for VCS.
//!
//! This module provides statistical analysis for benchmark tests,
//! including performance metrics collection, regression detection,
//! and comparison across tiers and implementations.
//!
//! # Features
//!
//! - Statistical analysis with percentiles and confidence intervals
//! - Performance regression detection with configurable thresholds
//! - Welch's t-test for comparing benchmark results
//! - Integration with criterion-style benchmarks
//! - Historical baseline comparison
//!
//! # Performance Categories
//!
//! Tests can specify performance expectations using the `@expected-performance` directive:
//!
//! ```text
//! // @expected-performance: < 15ns   -- CBGR check latency
//! // @expected-performance: < 100us  -- Simple function call
//! // @expected-performance: < 1ms    -- Complex operation
//! ```

use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, Instant};
use verum_common::{List, Map, Text};

/// Configuration for benchmark execution.
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    /// Number of warmup iterations
    pub warmup_iterations: usize,
    /// Number of measurement iterations
    pub measurement_iterations: usize,
    /// Minimum number of samples
    pub min_samples: usize,
    /// Maximum time per benchmark (ms)
    pub max_time_ms: u64,
    /// Threshold for regression detection (percentage)
    pub regression_threshold: f64,
    /// Whether to collect memory statistics
    pub collect_memory: bool,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            warmup_iterations: 100,
            measurement_iterations: 1000,
            min_samples: 10,
            max_time_ms: 30_000,
            regression_threshold: 10.0,
            collect_memory: true,
        }
    }
}

/// Statistics for a benchmark run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkStats {
    /// Name of the benchmark
    pub name: Text,
    /// Number of samples collected
    pub samples: usize,
    /// Mean duration in nanoseconds
    pub mean_ns: f64,
    /// Median duration in nanoseconds
    pub median_ns: f64,
    /// Standard deviation in nanoseconds
    pub std_dev_ns: f64,
    /// Minimum duration in nanoseconds
    pub min_ns: f64,
    /// Maximum duration in nanoseconds
    pub max_ns: f64,
    /// 5th percentile in nanoseconds
    pub p5_ns: f64,
    /// 95th percentile in nanoseconds
    pub p95_ns: f64,
    /// 99th percentile in nanoseconds
    pub p99_ns: f64,
    /// Throughput (iterations per second)
    pub throughput: f64,
    /// Memory allocations (if collected)
    pub allocations: Option<usize>,
    /// Peak memory usage in bytes (if collected)
    pub peak_memory_bytes: Option<u64>,
}

impl BenchmarkStats {
    /// Calculate statistics from a list of duration samples.
    pub fn from_samples(name: Text, durations: &[Duration]) -> Self {
        let mut nanos: Vec<f64> = durations.iter().map(|d| d.as_nanos() as f64).collect();
        nanos.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let samples = nanos.len();
        if samples == 0 {
            return Self {
                name,
                samples: 0,
                mean_ns: 0.0,
                median_ns: 0.0,
                std_dev_ns: 0.0,
                min_ns: 0.0,
                max_ns: 0.0,
                p5_ns: 0.0,
                p95_ns: 0.0,
                p99_ns: 0.0,
                throughput: 0.0,
                allocations: None,
                peak_memory_bytes: None,
            };
        }

        let sum: f64 = nanos.iter().sum();
        let mean_ns = sum / samples as f64;

        let variance: f64 =
            nanos.iter().map(|x| (x - mean_ns).powi(2)).sum::<f64>() / samples as f64;
        let std_dev_ns = variance.sqrt();

        let median_ns = if samples % 2 == 0 {
            (nanos[samples / 2 - 1] + nanos[samples / 2]) / 2.0
        } else {
            nanos[samples / 2]
        };

        let min_ns = *nanos.first().unwrap_or(&0.0);
        let max_ns = *nanos.last().unwrap_or(&0.0);

        let p5_idx = (samples as f64 * 0.05) as usize;
        let p95_idx = (samples as f64 * 0.95).min((samples - 1) as f64) as usize;
        let p99_idx = (samples as f64 * 0.99).min((samples - 1) as f64) as usize;

        let p5_ns = nanos[p5_idx];
        let p95_ns = nanos[p95_idx];
        let p99_ns = nanos[p99_idx];

        // Throughput = iterations per second
        let throughput = if mean_ns > 0.0 {
            1_000_000_000.0 / mean_ns
        } else {
            0.0
        };

        Self {
            name,
            samples,
            mean_ns,
            median_ns,
            std_dev_ns,
            min_ns,
            max_ns,
            p5_ns,
            p95_ns,
            p99_ns,
            throughput,
            allocations: None,
            peak_memory_bytes: None,
        }
    }

    /// Format the mean duration in human-readable form.
    pub fn format_duration(&self) -> Text {
        format_ns(self.mean_ns)
    }

    /// Get coefficient of variation (std_dev / mean).
    pub fn cv(&self) -> f64 {
        if self.mean_ns > 0.0 {
            self.std_dev_ns / self.mean_ns
        } else {
            0.0
        }
    }

    /// Check if this benchmark passes a performance threshold.
    ///
    /// threshold_ns is the maximum allowed mean duration in nanoseconds.
    pub fn passes_threshold(&self, threshold_ns: u64) -> bool {
        self.mean_ns <= threshold_ns as f64
    }
}

/// Comparison between two benchmark runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkComparison {
    /// Name of the benchmark
    pub name: Text,
    /// Baseline stats
    pub baseline: BenchmarkStats,
    /// Current stats
    pub current: BenchmarkStats,
    /// Percentage change (positive = regression, negative = improvement)
    pub change_percent: f64,
    /// Whether this is a significant regression
    pub is_regression: bool,
    /// Whether this is a significant improvement
    pub is_improvement: bool,
    /// Confidence level of the comparison
    pub confidence: f64,
}

impl BenchmarkComparison {
    /// Compare two benchmark results.
    pub fn compare(baseline: BenchmarkStats, current: BenchmarkStats, threshold: f64) -> Self {
        let change_percent = if baseline.mean_ns > 0.0 {
            ((current.mean_ns - baseline.mean_ns) / baseline.mean_ns) * 100.0
        } else {
            0.0
        };

        let is_regression = change_percent > threshold;
        let is_improvement = change_percent < -threshold;

        // Simple confidence calculation based on CV
        let confidence = if baseline.cv() < 0.1 && current.cv() < 0.1 {
            0.95 // High confidence if both have low variance
        } else if baseline.cv() < 0.2 && current.cv() < 0.2 {
            0.80
        } else {
            0.60
        };

        Self {
            name: current.name.clone(),
            baseline,
            current,
            change_percent,
            is_regression,
            is_improvement,
            confidence,
        }
    }

    /// Get a human-readable summary of the comparison.
    pub fn summary(&self) -> Text {
        let direction = if self.change_percent > 0.0 {
            "slower"
        } else {
            "faster"
        };

        format!(
            "{}: {:.2}% {} ({} -> {})",
            self.name,
            self.change_percent.abs(),
            direction,
            format_ns(self.baseline.mean_ns),
            format_ns(self.current.mean_ns)
        ).into()
    }
}

/// Complete benchmark report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReport {
    /// Report timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Compiler version
    pub compiler_version: Text,
    /// Configuration used
    pub warmup_iterations: usize,
    /// Measurement iterations
    pub measurement_iterations: usize,
    /// Results by name
    pub results: Map<Text, BenchmarkStats>,
    /// Comparisons (if baseline provided)
    pub comparisons: List<BenchmarkComparison>,
    /// Total duration of benchmark run
    pub total_duration_ms: u64,
}

impl BenchmarkReport {
    /// Create a new empty report.
    pub fn new(compiler_version: Text, config: &BenchmarkConfig) -> Self {
        Self {
            timestamp: chrono::Utc::now(),
            compiler_version,
            warmup_iterations: config.warmup_iterations,
            measurement_iterations: config.measurement_iterations,
            results: Map::new(),
            comparisons: List::new(),
            total_duration_ms: 0,
        }
    }

    /// Add a benchmark result.
    pub fn add_result(&mut self, stats: BenchmarkStats) {
        self.results.insert(stats.name.clone(), stats);
    }

    /// Compare with a baseline report.
    pub fn compare_with(&mut self, baseline: &BenchmarkReport, threshold: f64) {
        for (name, current_stats) in &self.results {
            if let Some(baseline_stats) = baseline.results.get(name) {
                let comparison = BenchmarkComparison::compare(
                    baseline_stats.clone(),
                    current_stats.clone(),
                    threshold,
                );
                self.comparisons.push(comparison);
            }
        }
    }

    /// Check if any benchmark regressed.
    pub fn has_regressions(&self) -> bool {
        self.comparisons.iter().any(|c| c.is_regression)
    }

    /// Get all regressions.
    pub fn regressions(&self) -> List<&BenchmarkComparison> {
        self.comparisons
            .iter()
            .filter(|c| c.is_regression)
            .collect()
    }

    /// Get all improvements.
    pub fn improvements(&self) -> List<&BenchmarkComparison> {
        self.comparisons
            .iter()
            .filter(|c| c.is_improvement)
            .collect()
    }
}

/// Performance metrics collected during test execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    /// Parsing time in microseconds
    pub parse_time_us: Option<u64>,
    /// Type checking time in microseconds
    pub typecheck_time_us: Option<u64>,
    /// Code generation time in microseconds
    pub codegen_time_us: Option<u64>,
    /// Execution time in microseconds
    pub execution_time_us: Option<u64>,
    /// Total time in microseconds
    pub total_time_us: u64,
    /// Memory allocated during compilation (bytes)
    pub compilation_memory_bytes: Option<u64>,
    /// Memory allocated during execution (bytes)
    pub execution_memory_bytes: Option<u64>,
    /// Number of CBGR checks performed
    pub cbgr_checks: Option<u64>,
    /// Average CBGR check time in nanoseconds
    pub cbgr_avg_ns: Option<f64>,
}

impl PerformanceMetrics {
    /// Create new metrics with just total time.
    pub fn new(total_time: Duration) -> Self {
        Self {
            total_time_us: total_time.as_micros() as u64,
            ..Default::default()
        }
    }

    /// Check if metrics meet performance requirements.
    pub fn meets_requirements(&self, requirements: &PerformanceRequirements) -> bool {
        if let Some(max_total) = requirements.max_total_us {
            if self.total_time_us > max_total {
                return false;
            }
        }

        if let (Some(parse_time), Some(max_parse)) = (self.parse_time_us, requirements.max_parse_us)
        {
            if parse_time > max_parse {
                return false;
            }
        }

        if let (Some(typecheck_time), Some(max_typecheck)) =
            (self.typecheck_time_us, requirements.max_typecheck_us)
        {
            if typecheck_time > max_typecheck {
                return false;
            }
        }

        if let (Some(cbgr_avg), Some(max_cbgr)) = (self.cbgr_avg_ns, requirements.max_cbgr_check_ns)
        {
            if cbgr_avg > max_cbgr {
                return false;
            }
        }

        true
    }
}

/// Performance requirements for a test.
#[derive(Debug, Clone, Default)]
pub struct PerformanceRequirements {
    /// Maximum total time in microseconds
    pub max_total_us: Option<u64>,
    /// Maximum parsing time in microseconds
    pub max_parse_us: Option<u64>,
    /// Maximum type checking time in microseconds
    pub max_typecheck_us: Option<u64>,
    /// Maximum CBGR check time in nanoseconds (per check)
    pub max_cbgr_check_ns: Option<f64>,
}

impl PerformanceRequirements {
    /// Parse from a directive string like "< 15ns" or "< 100us".
    pub fn from_directive(s: &str) -> Option<Self> {
        let s = s.trim();

        // Parse operator
        let (value_str, comparator) = if s.starts_with("<=") {
            (&s[2..], "<=")
        } else if s.starts_with('<') {
            (&s[1..], "<")
        } else {
            return None;
        };

        let value_str = value_str.trim();

        // Parse unit and value
        let (value, multiplier) = if value_str.ends_with("ns") {
            let val: f64 = value_str[..value_str.len() - 2].trim().parse().ok()?;
            (val, 0.001) // ns to us
        } else if value_str.ends_with("us") {
            let val: f64 = value_str[..value_str.len() - 2].trim().parse().ok()?;
            (val, 1.0)
        } else if value_str.ends_with("ms") {
            let val: f64 = value_str[..value_str.len() - 2].trim().parse().ok()?;
            (val, 1000.0)
        } else if value_str.ends_with('s')
            && !value_str.ends_with("ns")
            && !value_str.ends_with("us")
            && !value_str.ends_with("ms")
        {
            let val: f64 = value_str[..value_str.len() - 1].trim().parse().ok()?;
            (val, 1_000_000.0)
        } else {
            // Assume microseconds if no unit
            let val: f64 = value_str.parse().ok()?;
            (val, 1.0)
        };

        let us_value = (value * multiplier) as u64;
        let adjusted = if comparator == "<" {
            us_value.saturating_sub(1)
        } else {
            us_value
        };

        Some(Self {
            max_total_us: Some(adjusted),
            ..Default::default()
        })
    }
}

/// Format nanoseconds as human-readable string.
fn format_ns(ns: f64) -> Text {
    if ns < 1000.0 {
        format!("{:.2}ns", ns).into()
    } else if ns < 1_000_000.0 {
        format!("{:.2}us", ns / 1000.0).into()
    } else if ns < 1_000_000_000.0 {
        format!("{:.2}ms", ns / 1_000_000.0).into()
    } else {
        format!("{:.2}s", ns / 1_000_000_000.0).into()
    }
}

/// Tier performance comparison for differential testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierComparison {
    /// Tier 0 (interpreter) stats
    pub tier0: BenchmarkStats,
    /// Tier 3 (AOT) stats
    pub tier3: BenchmarkStats,
    /// Speedup factor (tier0 / tier3)
    pub speedup: f64,
    /// Whether outputs matched
    pub outputs_match: bool,
}

impl TierComparison {
    /// Create a tier comparison from two benchmark results.
    pub fn new(tier0: BenchmarkStats, tier3: BenchmarkStats, outputs_match: bool) -> Self {
        let speedup = if tier3.mean_ns > 0.0 {
            tier0.mean_ns / tier3.mean_ns
        } else {
            0.0
        };

        Self {
            tier0,
            tier3,
            speedup,
            outputs_match,
        }
    }

    /// Check if speedup meets expectations.
    pub fn meets_speedup_target(&self, min_speedup: f64) -> bool {
        self.speedup >= min_speedup
    }
}

/// Benchmark runner for executing and timing test functions.
pub struct BenchmarkRunner {
    config: BenchmarkConfig,
    results: Map<Text, BenchmarkStats>,
}

impl BenchmarkRunner {
    /// Create a new benchmark runner.
    pub fn new(config: BenchmarkConfig) -> Self {
        Self {
            config,
            results: Map::new(),
        }
    }

    /// Run a benchmark with the given name and function.
    pub fn bench<F>(&mut self, name: &str, mut f: F) -> BenchmarkStats
    where
        F: FnMut(),
    {
        // Warmup phase
        for _ in 0..self.config.warmup_iterations {
            f();
        }

        // Measurement phase
        let mut durations = Vec::with_capacity(self.config.measurement_iterations);
        let start_time = Instant::now();

        for _ in 0..self.config.measurement_iterations {
            let iter_start = Instant::now();
            f();
            durations.push(iter_start.elapsed());

            // Check if we've exceeded max time
            if start_time.elapsed().as_millis() as u64 > self.config.max_time_ms {
                break;
            }
        }

        // Ensure minimum samples
        while durations.len() < self.config.min_samples {
            let iter_start = Instant::now();
            f();
            durations.push(iter_start.elapsed());
        }

        let stats = BenchmarkStats::from_samples(name.to_string().into(), &durations);
        self.results.insert(name.to_string().into(), stats.clone());
        stats
    }

    /// Run a benchmark with black_box to prevent optimization.
    pub fn bench_with_input<T, F>(&mut self, name: &str, input: T, mut f: F) -> BenchmarkStats
    where
        T: Clone,
        F: FnMut(T),
    {
        self.bench(name, || {
            f(std::hint::black_box(input.clone()));
        })
    }

    /// Get all benchmark results.
    pub fn results(&self) -> &Map<Text, BenchmarkStats> {
        &self.results
    }

    /// Build a report from the results.
    pub fn build_report(&self, compiler_version: Text) -> BenchmarkReport {
        let mut report = BenchmarkReport::new(compiler_version, &self.config);
        for (_, stats) in &self.results {
            report.add_result(stats.clone());
        }
        report
    }
}

/// Result of a Welch's t-test comparison.
#[derive(Debug, Clone)]
pub struct TTestResult {
    /// t-statistic
    pub t_statistic: f64,
    /// Degrees of freedom (Welch-Satterthwaite)
    pub degrees_of_freedom: f64,
    /// p-value (two-tailed)
    pub p_value: f64,
    /// Effect size (Cohen's d)
    pub effect_size: f64,
    /// Whether the difference is statistically significant at alpha=0.05
    pub is_significant: bool,
}

impl TTestResult {
    /// Perform Welch's t-test comparing two samples.
    pub fn welch_test(sample1: &[f64], sample2: &[f64]) -> Self {
        let n1 = sample1.len() as f64;
        let n2 = sample2.len() as f64;

        let mean1 = sample1.iter().sum::<f64>() / n1;
        let mean2 = sample2.iter().sum::<f64>() / n2;

        let var1 = sample1.iter().map(|x| (x - mean1).powi(2)).sum::<f64>() / (n1 - 1.0);
        let var2 = sample2.iter().map(|x| (x - mean2).powi(2)).sum::<f64>() / (n2 - 1.0);

        let se1 = var1 / n1;
        let se2 = var2 / n2;
        let se_diff = (se1 + se2).sqrt();

        let t_statistic = (mean1 - mean2) / se_diff;

        // Welch-Satterthwaite degrees of freedom
        let df = (se1 + se2).powi(2) / ((se1.powi(2) / (n1 - 1.0)) + (se2.powi(2) / (n2 - 1.0)));

        // Approximate p-value using normal distribution for large samples
        // For small samples, would need proper t-distribution
        let p_value = 2.0 * (1.0 - normal_cdf(t_statistic.abs()));

        // Cohen's d effect size
        let pooled_std = ((var1 + var2) / 2.0).sqrt();
        let effect_size = (mean1 - mean2).abs() / pooled_std;

        Self {
            t_statistic,
            degrees_of_freedom: df,
            p_value,
            effect_size,
            is_significant: p_value < 0.05,
        }
    }

    /// Interpret the effect size.
    pub fn effect_interpretation(&self) -> &'static str {
        if self.effect_size < 0.2 {
            "negligible"
        } else if self.effect_size < 0.5 {
            "small"
        } else if self.effect_size < 0.8 {
            "medium"
        } else {
            "large"
        }
    }
}

/// Approximate normal CDF using error function approximation.
fn normal_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2))
}

/// Approximation of the error function using Abramowitz and Stegun approximation.
fn erf(x: f64) -> f64 {
    let a1 = 0.254829592;
    let a2 = -0.284496736;
    let a3 = 1.421413741;
    let a4 = -1.453152027;
    let a5 = 1.061405429;
    let p = 0.3275911;

    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();

    let t = 1.0 / (1.0 + p * x);
    let y = 1.0 - (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t * (-x * x).exp();

    sign * y
}

/// Regression detection result.
#[derive(Debug, Clone)]
pub struct RegressionDetection {
    /// Name of the benchmark
    pub name: Text,
    /// Baseline mean
    pub baseline_mean: f64,
    /// Current mean
    pub current_mean: f64,
    /// Percentage change
    pub change_percent: f64,
    /// Statistical test result
    pub test_result: TTestResult,
    /// Whether this is a regression
    pub is_regression: bool,
    /// Severity level
    pub severity: RegressionSeverity,
}

/// Severity of a performance regression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegressionSeverity {
    /// No regression
    None,
    /// Minor regression (<5%)
    Minor,
    /// Moderate regression (5-15%)
    Moderate,
    /// Major regression (15-50%)
    Major,
    /// Critical regression (>50%)
    Critical,
}

impl RegressionSeverity {
    /// Get severity from percentage change.
    pub fn from_percent(change: f64) -> Self {
        if change <= 0.0 {
            Self::None
        } else if change < 5.0 {
            Self::Minor
        } else if change < 15.0 {
            Self::Moderate
        } else if change < 50.0 {
            Self::Major
        } else {
            Self::Critical
        }
    }

    /// Get a display string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minor => "minor",
            Self::Moderate => "moderate",
            Self::Major => "major",
            Self::Critical => "critical",
        }
    }

    /// Get colored display string.
    pub fn colored_str(&self) -> String {
        match self {
            Self::None => "none".green().to_string(),
            Self::Minor => "minor".yellow().to_string(),
            Self::Moderate => "moderate".yellow().bold().to_string(),
            Self::Major => "major".red().to_string(),
            Self::Critical => "CRITICAL".red().bold().to_string(),
        }
    }
}

impl RegressionDetection {
    /// Detect regression between baseline and current samples.
    pub fn detect(
        name: &str,
        baseline_samples: &[f64],
        current_samples: &[f64],
        threshold: f64,
    ) -> Self {
        let baseline_mean = baseline_samples.iter().sum::<f64>() / baseline_samples.len() as f64;
        let current_mean = current_samples.iter().sum::<f64>() / current_samples.len() as f64;

        let change_percent = if baseline_mean > 0.0 {
            ((current_mean - baseline_mean) / baseline_mean) * 100.0
        } else {
            0.0
        };

        let test_result = TTestResult::welch_test(baseline_samples, current_samples);

        // Consider it a regression if:
        // 1. Change exceeds threshold
        // 2. Difference is statistically significant
        // 3. Effect size is at least small
        let is_regression = change_percent > threshold
            && test_result.is_significant
            && test_result.effect_size >= 0.2;

        let severity = if is_regression {
            RegressionSeverity::from_percent(change_percent)
        } else {
            RegressionSeverity::None
        };

        Self {
            name: name.to_string().into(),
            baseline_mean,
            current_mean,
            change_percent,
            test_result,
            is_regression,
            severity,
        }
    }

    /// Format as human-readable report.
    pub fn report(&self) -> String {
        let direction = if self.change_percent > 0.0 {
            "slower"
        } else {
            "faster"
        };

        format!(
            "{}: {:.2}% {} ({} -> {}) [p={:.4}, d={:.2} ({})]",
            self.name,
            self.change_percent.abs(),
            direction,
            format_ns(self.baseline_mean),
            format_ns(self.current_mean),
            self.test_result.p_value,
            self.test_result.effect_size,
            self.test_result.effect_interpretation()
        )
    }
}

/// Baseline storage for benchmark comparisons.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkBaseline {
    /// Creation timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Git commit hash (if available)
    pub commit_hash: Option<Text>,
    /// Compiler version
    pub compiler_version: Text,
    /// Results keyed by benchmark name
    pub results: Map<Text, BaselineEntry>,
}

/// Single baseline entry with samples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineEntry {
    /// Sample values in nanoseconds
    pub samples_ns: List<f64>,
    /// Summary statistics
    pub stats: BenchmarkStats,
}

impl BenchmarkBaseline {
    /// Create a new baseline.
    pub fn new(compiler_version: Text) -> Self {
        Self {
            timestamp: chrono::Utc::now(),
            commit_hash: None,
            compiler_version,
            results: Map::new(),
        }
    }

    /// Load baseline from file.
    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Save baseline to file.
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, content)
    }

    /// Add a benchmark result.
    pub fn add_result(&mut self, name: Text, samples_ns: List<f64>, stats: BenchmarkStats) {
        self.results
            .insert(name, BaselineEntry { samples_ns, stats });
    }

    /// Compare with current results and detect regressions.
    pub fn compare_with_current(
        &self,
        current: &BenchmarkReport,
        threshold: f64,
    ) -> List<RegressionDetection> {
        let mut detections = List::new();

        for (name, current_stats) in &current.results {
            if let Some(baseline_entry) = self.results.get(name) {
                // Convert current stats to samples (approximate from stats)
                let current_samples: Vec<f64> = vec![current_stats.mean_ns; current_stats.samples];

                let detection = RegressionDetection::detect(
                    name,
                    &baseline_entry.samples_ns,
                    &current_samples,
                    threshold,
                );

                detections.push(detection);
            }
        }

        detections
    }
}

/// Format nanoseconds with units for display.
pub fn format_duration_ns(ns: f64) -> String {
    format_ns(ns).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_benchmark_stats_from_samples() {
        let durations: Vec<Duration> = vec![
            Duration::from_nanos(100),
            Duration::from_nanos(110),
            Duration::from_nanos(105),
            Duration::from_nanos(95),
            Duration::from_nanos(102),
        ];

        let stats = BenchmarkStats::from_samples("test".to_string().into(), &durations);

        assert_eq!(stats.samples, 5);
        assert!((stats.mean_ns - 102.4).abs() < 0.1);
        assert!((stats.median_ns - 102.0).abs() < 0.1);
        assert_eq!(stats.min_ns, 95.0);
        assert_eq!(stats.max_ns, 110.0);
    }

    #[test]
    fn test_benchmark_comparison() {
        let baseline =
            BenchmarkStats::from_samples("test".to_string().into(), &vec![Duration::from_nanos(100); 10]);
        let current =
            BenchmarkStats::from_samples("test".to_string().into(), &vec![Duration::from_nanos(120); 10]);

        let comparison = BenchmarkComparison::compare(baseline, current, 10.0);

        assert!((comparison.change_percent - 20.0).abs() < 0.1);
        assert!(comparison.is_regression);
        assert!(!comparison.is_improvement);
    }

    #[test]
    fn test_performance_requirements_parsing() {
        let req = PerformanceRequirements::from_directive("< 15ns").unwrap();
        // 15ns = 0.015us, rounded down to 0 for strict less than
        assert!(req.max_total_us.is_some());

        let req = PerformanceRequirements::from_directive("<= 100us").unwrap();
        assert_eq!(req.max_total_us, Some(100));

        let req = PerformanceRequirements::from_directive("< 1ms").unwrap();
        assert_eq!(req.max_total_us, Some(999));
    }

    #[test]
    fn test_format_ns() {
        assert_eq!(format_ns(500.0), "500.00ns");
        assert_eq!(format_ns(1500.0), "1.50us");
        assert_eq!(format_ns(1_500_000.0), "1.50ms");
        assert_eq!(format_ns(1_500_000_000.0), "1.50s");
    }

    #[test]
    fn test_tier_comparison() {
        let tier0 =
            BenchmarkStats::from_samples("test".to_string().into(), &vec![Duration::from_nanos(1000); 10]);
        let tier3 =
            BenchmarkStats::from_samples("test".to_string().into(), &vec![Duration::from_nanos(100); 10]);

        let comparison = TierComparison::new(tier0, tier3, true);

        assert!((comparison.speedup - 10.0).abs() < 0.1);
        assert!(comparison.meets_speedup_target(5.0));
        assert!(!comparison.meets_speedup_target(15.0));
    }

    #[test]
    fn test_benchmark_runner() {
        let config = BenchmarkConfig {
            warmup_iterations: 5,
            measurement_iterations: 10,
            min_samples: 5,
            max_time_ms: 1000,
            regression_threshold: 10.0,
            collect_memory: false,
        };

        let mut runner = BenchmarkRunner::new(config);

        let stats = runner.bench("simple_add", || {
            let _ = std::hint::black_box(1 + 2);
        });

        assert!(stats.samples >= 5);
        assert!(stats.mean_ns > 0.0);
        assert!(runner.results().contains_key(&Text::from("simple_add")));
    }

    #[test]
    fn test_welch_t_test() {
        // Two clearly different samples
        let sample1: Vec<f64> = vec![100.0, 102.0, 98.0, 101.0, 99.0];
        let sample2: Vec<f64> = vec![150.0, 148.0, 152.0, 149.0, 151.0];

        let result = TTestResult::welch_test(&sample1, &sample2);

        assert!(result.t_statistic < 0.0); // sample1 < sample2
        assert!(result.is_significant);
        assert!(result.effect_size > 0.8); // Large effect
    }

    #[test]
    fn test_welch_t_test_no_difference() {
        // Two similar samples
        let sample1: Vec<f64> = vec![100.0, 102.0, 98.0, 101.0, 99.0];
        let sample2: Vec<f64> = vec![100.0, 99.0, 101.0, 100.0, 100.0];

        let result = TTestResult::welch_test(&sample1, &sample2);

        assert!(!result.is_significant);
        assert!(result.effect_size < 0.2); // Negligible effect
    }

    #[test]
    fn test_regression_detection() {
        let baseline = vec![
            100.0, 102.0, 98.0, 101.0, 99.0, 100.0, 101.0, 99.0, 100.0, 101.0,
        ];
        let regressed = vec![
            120.0, 122.0, 118.0, 121.0, 119.0, 120.0, 121.0, 119.0, 120.0, 121.0,
        ];

        let detection = RegressionDetection::detect("test", &baseline, &regressed, 10.0);

        assert!(detection.is_regression);
        assert!(detection.change_percent > 15.0);
        assert!(matches!(detection.severity, RegressionSeverity::Major));
    }

    #[test]
    fn test_regression_severity() {
        assert_eq!(
            RegressionSeverity::from_percent(-5.0),
            RegressionSeverity::None
        );
        assert_eq!(
            RegressionSeverity::from_percent(3.0),
            RegressionSeverity::Minor
        );
        assert_eq!(
            RegressionSeverity::from_percent(10.0),
            RegressionSeverity::Moderate
        );
        assert_eq!(
            RegressionSeverity::from_percent(30.0),
            RegressionSeverity::Major
        );
        assert_eq!(
            RegressionSeverity::from_percent(75.0),
            RegressionSeverity::Critical
        );
    }

    #[test]
    fn test_normal_cdf() {
        // Known values
        assert!((normal_cdf(0.0) - 0.5).abs() < 0.01);
        assert!((normal_cdf(1.96) - 0.975).abs() < 0.01);
        assert!((normal_cdf(-1.96) - 0.025).abs() < 0.01);
    }

    #[test]
    fn test_erf() {
        // Known values
        assert!((erf(0.0) - 0.0).abs() < 0.01);
        assert!((erf(1.0) - 0.8427).abs() < 0.01);
        assert!((erf(-1.0) - (-0.8427)).abs() < 0.01);
    }

    #[test]
    fn test_benchmark_baseline_serialization() {
        let mut baseline = BenchmarkBaseline::new("1.0.0".to_string().into());
        baseline.add_result(
            "test".to_string().into(),
            vec![100.0, 101.0, 99.0].into(),
            BenchmarkStats::from_samples(
                "test".to_string().into(),
                &[
                    Duration::from_nanos(100),
                    Duration::from_nanos(101),
                    Duration::from_nanos(99),
                ],
            ),
        );

        let json = serde_json::to_string(&baseline).unwrap();
        let parsed: BenchmarkBaseline = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.compiler_version, Text::from("1.0.0"));
        assert!(parsed.results.contains_key(&Text::from("test")));
    }
}
