//! Baseline comparison for VCS benchmarks.
//!
//! This module provides functionality for comparing Verum benchmark results
//! against baselines in other languages (C, Rust, Go) and detecting performance
//! regressions between versions.
//!
//! # Features
//!
//! - Language baseline comparison (C, Rust, Go)
//! - Statistical significance testing:
//!   - Welch's t-test (parametric, for normal distributions)
//!   - Mann-Whitney U test (non-parametric, for any distribution)
//!   - Bootstrap confidence intervals
//! - Regression detection with configurable thresholds
//! - Effect size calculation (Cohen's d)
//!
//! # Example
//!
//! ```ignore
//! let config = RegressionConfig::default()
//!     .with_threshold(5.0)
//!     .with_significance_test(SignificanceTest::MannWhitney);
//!
//! let result = detect_regression(&current, &baseline, &config);
//! if result.is_regression {
//!     println!("Regression detected: {}% slower", result.percentage_change);
//! }
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::metrics::{BenchmarkResult, Statistics};

// ============================================================================
// Language Baselines
// ============================================================================

/// Supported baseline languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BaselineLanguage {
    C,
    Rust,
    Go,
}

impl std::fmt::Display for BaselineLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::C => write!(f, "C"),
            Self::Rust => write!(f, "Rust"),
            Self::Go => write!(f, "Go"),
        }
    }
}

impl BaselineLanguage {
    /// Get the file extension for this language.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::C => "c",
            Self::Rust => "rs",
            Self::Go => "go",
        }
    }

    /// Get the compile command for this language.
    pub fn compile_command(&self, source: &Path, output: &Path) -> Command {
        match self {
            Self::C => {
                let mut cmd = Command::new("cc");
                cmd.args(["-O3", "-march=native", "-o"])
                    .arg(output)
                    .arg(source);
                cmd
            }
            Self::Rust => {
                let mut cmd = Command::new("rustc");
                cmd.args(["-O", "-o"]).arg(output).arg(source);
                cmd
            }
            Self::Go => {
                let mut cmd = Command::new("go");
                cmd.args(["build", "-o"]).arg(output).arg(source);
                cmd
            }
        }
    }
}

/// A baseline benchmark in another language.
#[derive(Debug, Clone)]
pub struct BaselineBenchmark {
    /// Benchmark name.
    pub name: String,
    /// Source language.
    pub language: BaselineLanguage,
    /// Source file path.
    pub source_path: PathBuf,
    /// Compiled binary path (if applicable).
    pub binary_path: Option<PathBuf>,
    /// Number of iterations to run.
    pub iterations: usize,
}

impl BaselineBenchmark {
    /// Create a new baseline benchmark.
    pub fn new(name: impl Into<String>, language: BaselineLanguage, source: PathBuf) -> Self {
        Self {
            name: name.into(),
            language,
            source_path: source,
            binary_path: None,
            iterations: 1000,
        }
    }

    /// Compile the benchmark (for compiled languages).
    pub fn compile(&mut self, output_dir: &Path) -> Result<()> {
        let binary_name = format!("{}_{}", self.name, self.language.to_string().to_lowercase());
        let binary_path = output_dir.join(&binary_name);

        let output = self
            .language
            .compile_command(&self.source_path, &binary_path)
            .output()
            .context("Failed to compile baseline")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Compilation failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        self.binary_path = Some(binary_path);
        Ok(())
    }

    /// Run the benchmark and collect measurements.
    pub fn run(&self, warmup: usize) -> Result<BaselineResult> {
        let binary = self
            .binary_path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Benchmark not compiled"))?;

        // Warmup
        for _ in 0..warmup {
            Command::new(binary)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()?;
        }

        // Measure
        let mut durations = Vec::with_capacity(self.iterations);
        for _ in 0..self.iterations {
            let start = Instant::now();
            let status = Command::new(binary)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()?;
            let duration = start.elapsed();

            if !status.success() {
                return Err(anyhow::anyhow!("Baseline benchmark failed"));
            }

            durations.push(duration);
        }

        let statistics = Statistics::from_durations(&durations)
            .ok_or_else(|| anyhow::anyhow!("No measurements"))?;

        Ok(BaselineResult {
            name: self.name.clone(),
            language: self.language,
            statistics,
            raw_measurements_ns: durations.iter().map(|d| d.as_nanos() as f64).collect(),
        })
    }
}

/// Result of a baseline benchmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineResult {
    /// Benchmark name.
    pub name: String,
    /// Source language.
    pub language: BaselineLanguage,
    /// Statistical summary.
    pub statistics: Statistics,
    /// Raw measurements in nanoseconds (for statistical tests).
    #[serde(default)]
    pub raw_measurements_ns: Vec<f64>,
}

// ============================================================================
// Statistical Significance Tests
// ============================================================================

/// Type of statistical significance test to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignificanceTest {
    /// Welch's t-test (parametric, assumes approximately normal distribution).
    WelchT,
    /// Mann-Whitney U test (non-parametric, robust for any distribution).
    MannWhitney,
    /// Bootstrap confidence interval test.
    Bootstrap,
    /// No significance test (just use threshold).
    None,
}

impl Default for SignificanceTest {
    fn default() -> Self {
        SignificanceTest::MannWhitney
    }
}

/// Result of a statistical significance test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignificanceResult {
    /// The test used.
    pub test: SignificanceTest,
    /// P-value (probability of observing this result under null hypothesis).
    pub p_value: f64,
    /// Whether the result is statistically significant.
    pub is_significant: bool,
    /// Effect size (Cohen's d for parametric, r for non-parametric).
    pub effect_size: f64,
    /// Effect size interpretation.
    pub effect_interpretation: EffectSizeInterpretation,
    /// Test statistic value.
    pub test_statistic: f64,
    /// Confidence interval for the difference (if applicable).
    pub confidence_interval: Option<(f64, f64)>,
}

/// Interpretation of effect size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectSizeInterpretation {
    Negligible,
    Small,
    Medium,
    Large,
}

impl std::fmt::Display for EffectSizeInterpretation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Negligible => write!(f, "negligible"),
            Self::Small => write!(f, "small"),
            Self::Medium => write!(f, "medium"),
            Self::Large => write!(f, "large"),
        }
    }
}

/// Perform Welch's t-test for two samples with unequal variances.
pub fn welch_t_test(sample1: &[f64], sample2: &[f64], alpha: f64) -> SignificanceResult {
    let n1 = sample1.len() as f64;
    let n2 = sample2.len() as f64;

    if n1 < 2.0 || n2 < 2.0 {
        return SignificanceResult {
            test: SignificanceTest::WelchT,
            p_value: 1.0,
            is_significant: false,
            effect_size: 0.0,
            effect_interpretation: EffectSizeInterpretation::Negligible,
            test_statistic: 0.0,
            confidence_interval: None,
        };
    }

    let mean1 = sample1.iter().sum::<f64>() / n1;
    let mean2 = sample2.iter().sum::<f64>() / n2;

    let var1 = sample1.iter().map(|x| (x - mean1).powi(2)).sum::<f64>() / (n1 - 1.0);
    let var2 = sample2.iter().map(|x| (x - mean2).powi(2)).sum::<f64>() / (n2 - 1.0);

    let _std1 = var1.sqrt();
    let _std2 = var2.sqrt();

    let se = ((var1 / n1) + (var2 / n2)).sqrt();
    let t = if se > 0.0 { (mean1 - mean2) / se } else { 0.0 };

    // Degrees of freedom (Welch-Satterthwaite)
    let num = ((var1 / n1) + (var2 / n2)).powi(2);
    let denom = (var1 / n1).powi(2) / (n1 - 1.0) + (var2 / n2).powi(2) / (n2 - 1.0);
    let df = if denom > 0.0 { num / denom } else { 1.0 };

    // P-value using t-distribution approximation
    let p_value = 2.0 * (1.0 - t_cdf(t.abs(), df));

    // Cohen's d effect size
    let pooled_std = ((var1 + var2) / 2.0).sqrt();
    let cohens_d = if pooled_std > 0.0 {
        (mean1 - mean2).abs() / pooled_std
    } else {
        0.0
    };

    let effect_interpretation = interpret_cohens_d(cohens_d);

    // Confidence interval for the difference
    let t_critical = t_critical_value_for_df(df, alpha / 2.0);
    let diff = mean1 - mean2;
    let ci = (diff - t_critical * se, diff + t_critical * se);

    SignificanceResult {
        test: SignificanceTest::WelchT,
        p_value,
        is_significant: p_value < alpha,
        effect_size: cohens_d,
        effect_interpretation,
        test_statistic: t,
        confidence_interval: Some(ci),
    }
}

/// Perform Mann-Whitney U test (Wilcoxon rank-sum test).
///
/// This is a non-parametric test that doesn't assume normal distribution.
/// It tests whether one sample tends to have larger values than the other.
pub fn mann_whitney_u_test(sample1: &[f64], sample2: &[f64], alpha: f64) -> SignificanceResult {
    let n1 = sample1.len();
    let n2 = sample2.len();

    if n1 == 0 || n2 == 0 {
        return SignificanceResult {
            test: SignificanceTest::MannWhitney,
            p_value: 1.0,
            is_significant: false,
            effect_size: 0.0,
            effect_interpretation: EffectSizeInterpretation::Negligible,
            test_statistic: 0.0,
            confidence_interval: None,
        };
    }

    // Combine samples and rank
    let mut combined: Vec<(f64, usize)> = sample1
        .iter()
        .map(|&x| (x, 0))
        .chain(sample2.iter().map(|&x| (x, 1)))
        .collect();
    combined.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    // Assign ranks (handling ties by averaging)
    let mut ranks: Vec<(f64, usize)> = Vec::with_capacity(combined.len());
    let mut i = 0;
    while i < combined.len() {
        let mut j = i;
        // Find all tied values
        while j < combined.len() && (combined[j].0 - combined[i].0).abs() < 1e-10 {
            j += 1;
        }
        // Average rank for tied values
        let avg_rank = (i + 1..=j).map(|r| r as f64).sum::<f64>() / (j - i) as f64;
        for k in i..j {
            ranks.push((avg_rank, combined[k].1));
        }
        i = j;
    }

    // Sum ranks for sample 1
    let r1: f64 = ranks.iter().filter(|(_, g)| *g == 0).map(|(r, _)| *r).sum();

    // Calculate U statistic
    let u1 = r1 - (n1 * (n1 + 1)) as f64 / 2.0;
    let u2 = (n1 * n2) as f64 - u1;
    let u = u1.min(u2);

    // Normal approximation for large samples
    let n1f = n1 as f64;
    let n2f = n2 as f64;
    let mu = n1f * n2f / 2.0;
    let sigma = ((n1f * n2f * (n1f + n2f + 1.0)) / 12.0).sqrt();

    let z = if sigma > 0.0 { (u - mu) / sigma } else { 0.0 };
    let p_value = 2.0 * (1.0 - normal_cdf(z.abs()));

    // Effect size r = Z / sqrt(N)
    let total_n = n1 + n2;
    let effect_r = z.abs() / (total_n as f64).sqrt();
    let effect_interpretation = interpret_effect_r(effect_r);

    SignificanceResult {
        test: SignificanceTest::MannWhitney,
        p_value,
        is_significant: p_value < alpha,
        effect_size: effect_r,
        effect_interpretation,
        test_statistic: u,
        confidence_interval: None, // Mann-Whitney doesn't provide a natural CI
    }
}

/// Perform bootstrap confidence interval test.
///
/// Resamples both distributions to estimate the confidence interval
/// of the difference in means.
pub fn bootstrap_test(
    sample1: &[f64],
    sample2: &[f64],
    alpha: f64,
    n_bootstrap: usize,
) -> SignificanceResult {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    if sample1.is_empty() || sample2.is_empty() {
        return SignificanceResult {
            test: SignificanceTest::Bootstrap,
            p_value: 1.0,
            is_significant: false,
            effect_size: 0.0,
            effect_interpretation: EffectSizeInterpretation::Negligible,
            test_statistic: 0.0,
            confidence_interval: None,
        };
    }

    let observed_diff = mean(sample1) - mean(sample2);

    // Simple PRNG using hash
    let mut hasher = DefaultHasher::new();
    let mut seed = 42u64;

    let mut bootstrap_diffs = Vec::with_capacity(n_bootstrap);

    for _ in 0..n_bootstrap {
        // Resample sample1
        let resample1: Vec<f64> = (0..sample1.len())
            .map(|_| {
                seed.hash(&mut hasher);
                seed = hasher.finish();
                let idx = (seed as usize) % sample1.len();
                sample1[idx]
            })
            .collect();

        // Resample sample2
        let resample2: Vec<f64> = (0..sample2.len())
            .map(|_| {
                seed.hash(&mut hasher);
                seed = hasher.finish();
                let idx = (seed as usize) % sample2.len();
                sample2[idx]
            })
            .collect();

        let diff = mean(&resample1) - mean(&resample2);
        bootstrap_diffs.push(diff);
    }

    bootstrap_diffs.sort_by(|a, b| a.partial_cmp(b).unwrap());

    // Calculate confidence interval
    let lower_idx = ((alpha / 2.0) * n_bootstrap as f64) as usize;
    let upper_idx = ((1.0 - alpha / 2.0) * n_bootstrap as f64) as usize;
    let ci_lower = bootstrap_diffs[lower_idx.min(bootstrap_diffs.len() - 1)];
    let ci_upper = bootstrap_diffs[upper_idx.min(bootstrap_diffs.len() - 1)];

    // P-value: proportion of bootstrap samples where difference has opposite sign
    let p_value = if observed_diff > 0.0 {
        bootstrap_diffs.iter().filter(|&&d| d <= 0.0).count() as f64 / n_bootstrap as f64
    } else {
        bootstrap_diffs.iter().filter(|&&d| d >= 0.0).count() as f64 / n_bootstrap as f64
    };
    let p_value = 2.0 * p_value.min(0.5); // Two-tailed

    // Effect size: standardized mean difference
    let pooled_std = ((variance(sample1) + variance(sample2)) / 2.0).sqrt();
    let effect_size = if pooled_std > 0.0 {
        observed_diff.abs() / pooled_std
    } else {
        0.0
    };

    SignificanceResult {
        test: SignificanceTest::Bootstrap,
        p_value,
        is_significant: p_value < alpha,
        effect_size,
        effect_interpretation: interpret_cohens_d(effect_size),
        test_statistic: observed_diff,
        confidence_interval: Some((ci_lower, ci_upper)),
    }
}

/// Interpret Cohen's d effect size.
fn interpret_cohens_d(d: f64) -> EffectSizeInterpretation {
    let d = d.abs();
    if d < 0.2 {
        EffectSizeInterpretation::Negligible
    } else if d < 0.5 {
        EffectSizeInterpretation::Small
    } else if d < 0.8 {
        EffectSizeInterpretation::Medium
    } else {
        EffectSizeInterpretation::Large
    }
}

/// Interpret effect size r (for Mann-Whitney U).
fn interpret_effect_r(r: f64) -> EffectSizeInterpretation {
    let r = r.abs();
    if r < 0.1 {
        EffectSizeInterpretation::Negligible
    } else if r < 0.3 {
        EffectSizeInterpretation::Small
    } else if r < 0.5 {
        EffectSizeInterpretation::Medium
    } else {
        EffectSizeInterpretation::Large
    }
}

/// Calculate mean of a slice.
fn mean(data: &[f64]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    data.iter().sum::<f64>() / data.len() as f64
}

/// Calculate variance of a slice.
fn variance(data: &[f64]) -> f64 {
    if data.len() < 2 {
        return 0.0;
    }
    let m = mean(data);
    data.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (data.len() - 1) as f64
}

/// Approximate cumulative distribution function for standard normal.
fn normal_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2))
}

/// Approximate error function.
fn erf(x: f64) -> f64 {
    // Horner form coefficients
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

/// Approximate t-distribution CDF.
fn t_cdf(t: f64, df: f64) -> f64 {
    // Use normal approximation for large df
    if df > 30.0 {
        return normal_cdf(t);
    }

    // Simple approximation using beta function
    let x = df / (df + t * t);
    0.5 + 0.5 * (1.0 - incomplete_beta(df / 2.0, 0.5, x)) * t.signum()
}

/// Approximate incomplete beta function.
fn incomplete_beta(_a: f64, _b: f64, x: f64) -> f64 {
    // Simple approximation - for production use a proper implementation
    x.powf(0.5)
}

/// Get t critical value for given degrees of freedom and alpha.
fn t_critical_value_for_df(df: f64, alpha: f64) -> f64 {
    // Approximation using normal distribution for large df
    if df > 30.0 {
        return inverse_normal_cdf(1.0 - alpha);
    }

    // Simple lookup for common values
    if alpha <= 0.005 {
        2.576 * (1.0 + 0.25 / df).sqrt()
    } else if alpha <= 0.025 {
        1.96 * (1.0 + 0.25 / df).sqrt()
    } else if alpha <= 0.05 {
        1.645 * (1.0 + 0.25 / df).sqrt()
    } else {
        1.282 * (1.0 + 0.25 / df).sqrt()
    }
}

/// Approximate inverse normal CDF (quantile function).
fn inverse_normal_cdf(p: f64) -> f64 {
    // Rational approximation
    let p = p.clamp(0.0001, 0.9999);
    let t = if p < 0.5 {
        (-2.0 * p.ln()).sqrt()
    } else {
        (-2.0 * (1.0 - p).ln()).sqrt()
    };

    let c0 = 2.515517;
    let c1 = 0.802853;
    let c2 = 0.010328;
    let d1 = 1.432788;
    let d2 = 0.189269;
    let d3 = 0.001308;

    let z = t - (c0 + c1 * t + c2 * t * t) / (1.0 + d1 * t + d2 * t * t + d3 * t * t * t);

    if p < 0.5 { -z } else { z }
}

// ============================================================================
// Comparison Analysis
// ============================================================================

/// Comparison between Verum and a baseline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonResult {
    /// Benchmark name.
    pub name: String,
    /// Verum result.
    pub verum: Statistics,
    /// Baseline result.
    pub baseline: BaselineResult,
    /// Ratio (Verum / Baseline). < 1.0 means Verum is faster.
    pub ratio: f64,
    /// Percentage difference. Negative means Verum is faster.
    pub percentage_diff: f64,
    /// Assessment of the comparison.
    pub assessment: ComparisonAssessment,
    /// Statistical significance result (if raw data available).
    pub significance: Option<SignificanceResult>,
}

/// Assessment of a comparison result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComparisonAssessment {
    /// Verum is significantly faster (> 10% faster).
    VerumFaster,
    /// Performance is comparable (within 10%).
    Comparable,
    /// Verum is slower but acceptable (within target range).
    AcceptableSlower,
    /// Verum is too slow (outside target range).
    TooSlow,
}

impl ComparisonResult {
    /// Create a comparison from Verum and baseline results.
    pub fn compare(
        verum: &BenchmarkResult,
        baseline: &BaselineResult,
        target_min: f64,
        _target_max: f64,
    ) -> Self {
        let ratio = verum.statistics.mean_ns / baseline.statistics.mean_ns;
        let percentage_diff = (ratio - 1.0) * 100.0;

        let assessment = if ratio < 1.0 - 0.1 {
            ComparisonAssessment::VerumFaster
        } else if ratio <= 1.0 + 0.1 {
            ComparisonAssessment::Comparable
        } else if ratio <= 1.0 / target_min {
            // Within acceptable range (e.g., 0.85-0.95x of C)
            ComparisonAssessment::AcceptableSlower
        } else {
            ComparisonAssessment::TooSlow
        };

        // Perform significance test if raw data is available
        let significance = if !baseline.raw_measurements_ns.is_empty() {
            // Generate synthetic raw measurements for Verum if not available
            // In production, BenchmarkResult should also store raw measurements
            let verum_samples: Vec<f64> = (0..baseline.raw_measurements_ns.len())
                .map(|_| verum.statistics.mean_ns)
                .collect();

            Some(mann_whitney_u_test(
                &verum_samples,
                &baseline.raw_measurements_ns,
                0.05,
            ))
        } else {
            None
        };

        Self {
            name: verum.name.clone(),
            verum: verum.statistics.clone(),
            baseline: baseline.clone(),
            ratio,
            percentage_diff,
            assessment,
            significance,
        }
    }

    /// Create a comparison with statistical test.
    pub fn compare_with_test(
        verum: &BenchmarkResult,
        verum_raw: &[f64],
        baseline: &BaselineResult,
        target_min: f64,
        target_max: f64,
        test: SignificanceTest,
        alpha: f64,
    ) -> Self {
        let mut result = Self::compare(verum, baseline, target_min, target_max);

        if !baseline.raw_measurements_ns.is_empty() && !verum_raw.is_empty() {
            result.significance = Some(match test {
                SignificanceTest::WelchT => {
                    welch_t_test(verum_raw, &baseline.raw_measurements_ns, alpha)
                }
                SignificanceTest::MannWhitney => {
                    mann_whitney_u_test(verum_raw, &baseline.raw_measurements_ns, alpha)
                }
                SignificanceTest::Bootstrap => {
                    bootstrap_test(verum_raw, &baseline.raw_measurements_ns, alpha, 10000)
                }
                SignificanceTest::None => return result,
            });
        }

        result
    }

    /// Check if the comparison passed.
    pub fn passed(&self) -> bool {
        matches!(
            self.assessment,
            ComparisonAssessment::VerumFaster
                | ComparisonAssessment::Comparable
                | ComparisonAssessment::AcceptableSlower
        )
    }

    /// Check if the difference is statistically significant.
    pub fn is_significant(&self) -> bool {
        self.significance
            .as_ref()
            .map(|s| s.is_significant)
            .unwrap_or(false)
    }
}

// ============================================================================
// Regression Detection
// ============================================================================

/// Historical benchmark data for regression detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkHistory {
    /// Benchmark name.
    pub name: String,
    /// Historical data points.
    pub data_points: Vec<HistoricalDataPoint>,
}

/// A single historical data point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalDataPoint {
    /// Version or commit hash.
    pub version: String,
    /// Timestamp.
    #[serde(with = "chrono::serde::ts_seconds")]
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Mean time in nanoseconds.
    pub mean_ns: f64,
    /// Standard deviation.
    pub std_dev_ns: f64,
    /// Raw measurements (for statistical tests).
    #[serde(default)]
    pub raw_measurements_ns: Vec<f64>,
}

/// Regression detection result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionResult {
    /// Benchmark name.
    pub name: String,
    /// Current version.
    pub current_version: String,
    /// Baseline version.
    pub baseline_version: String,
    /// Current mean in nanoseconds.
    pub current_mean_ns: f64,
    /// Baseline mean in nanoseconds.
    pub baseline_mean_ns: f64,
    /// Percentage change (positive = slower).
    pub percentage_change: f64,
    /// Whether this is a regression.
    pub is_regression: bool,
    /// Statistical significance result.
    pub significance: Option<SignificanceResult>,
    /// Confidence that this is a true regression (0-1).
    pub confidence: f64,
}

impl RegressionResult {
    /// Get a human-readable summary.
    pub fn summary(&self) -> String {
        if self.is_regression {
            format!(
                "REGRESSION: {} is {:.1}% slower ({:.2}ns -> {:.2}ns) with {:.0}% confidence",
                self.name,
                self.percentage_change,
                self.baseline_mean_ns,
                self.current_mean_ns,
                self.confidence * 100.0
            )
        } else if self.percentage_change < -5.0 {
            format!(
                "IMPROVEMENT: {} is {:.1}% faster ({:.2}ns -> {:.2}ns)",
                self.name, -self.percentage_change, self.baseline_mean_ns, self.current_mean_ns
            )
        } else {
            format!(
                "STABLE: {} changed by {:.1}% ({:.2}ns -> {:.2}ns)",
                self.name, self.percentage_change, self.baseline_mean_ns, self.current_mean_ns
            )
        }
    }
}

/// Configuration for regression detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionConfig {
    /// Threshold for regression detection (percentage).
    pub threshold_percent: f64,
    /// Minimum number of samples for comparison.
    pub min_samples: usize,
    /// Significance level (alpha).
    pub significance_level: f64,
    /// Type of significance test to use.
    pub significance_test: SignificanceTest,
    /// Require statistical significance for regression.
    pub require_significance: bool,
    /// Number of bootstrap iterations (if using bootstrap test).
    pub bootstrap_iterations: usize,
}

impl Default for RegressionConfig {
    fn default() -> Self {
        Self {
            threshold_percent: 5.0, // 5% regression threshold
            min_samples: 30,
            significance_level: 0.05,
            significance_test: SignificanceTest::MannWhitney,
            require_significance: true,
            bootstrap_iterations: 10000,
        }
    }
}

impl RegressionConfig {
    /// Set the regression threshold.
    pub fn with_threshold(mut self, percent: f64) -> Self {
        self.threshold_percent = percent;
        self
    }

    /// Set the significance test.
    pub fn with_significance_test(mut self, test: SignificanceTest) -> Self {
        self.significance_test = test;
        self
    }

    /// Set whether to require statistical significance.
    pub fn with_require_significance(mut self, require: bool) -> Self {
        self.require_significance = require;
        self
    }

    /// Set the significance level.
    pub fn with_significance_level(mut self, alpha: f64) -> Self {
        self.significance_level = alpha;
        self
    }
}

/// Detect regressions between two benchmark results.
pub fn detect_regression(
    current: &BenchmarkResult,
    baseline: &BenchmarkResult,
    config: &RegressionConfig,
) -> RegressionResult {
    let percentage_change = ((current.statistics.mean_ns - baseline.statistics.mean_ns)
        / baseline.statistics.mean_ns)
        * 100.0;

    // Perform significance test if we have enough samples
    let significance = if current.statistics.count >= config.min_samples
        && baseline.statistics.count >= config.min_samples
    {
        // Generate synthetic samples based on statistics
        // In production, use actual raw measurements
        let current_samples = generate_samples(
            current.statistics.mean_ns,
            current.statistics.std_dev_ns,
            config.min_samples,
        );
        let baseline_samples = generate_samples(
            baseline.statistics.mean_ns,
            baseline.statistics.std_dev_ns,
            config.min_samples,
        );

        Some(match config.significance_test {
            SignificanceTest::WelchT => welch_t_test(
                &current_samples,
                &baseline_samples,
                config.significance_level,
            ),
            SignificanceTest::MannWhitney => mann_whitney_u_test(
                &current_samples,
                &baseline_samples,
                config.significance_level,
            ),
            SignificanceTest::Bootstrap => bootstrap_test(
                &current_samples,
                &baseline_samples,
                config.significance_level,
                config.bootstrap_iterations,
            ),
            SignificanceTest::None => {
                // No test, just use threshold
                SignificanceResult {
                    test: SignificanceTest::None,
                    p_value: 0.0,
                    is_significant: true,
                    effect_size: 0.0,
                    effect_interpretation: EffectSizeInterpretation::Negligible,
                    test_statistic: 0.0,
                    confidence_interval: None,
                }
            }
        })
    } else {
        None
    };

    // Determine if this is a regression
    let is_significant = significance
        .as_ref()
        .map(|s| s.is_significant)
        .unwrap_or(true);
    let exceeds_threshold = percentage_change > config.threshold_percent;

    let is_regression = if config.require_significance {
        exceeds_threshold && is_significant
    } else {
        exceeds_threshold
    };

    // Calculate confidence score
    let confidence = calculate_regression_confidence(
        percentage_change,
        config.threshold_percent,
        significance.as_ref().map(|s| s.p_value).unwrap_or(0.5),
    );

    RegressionResult {
        name: current.name.clone(),
        current_version: current
            .metadata
            .get("version")
            .cloned()
            .unwrap_or_else(|| "current".to_string()),
        baseline_version: baseline
            .metadata
            .get("version")
            .cloned()
            .unwrap_or_else(|| "baseline".to_string()),
        current_mean_ns: current.statistics.mean_ns,
        baseline_mean_ns: baseline.statistics.mean_ns,
        percentage_change,
        is_regression,
        significance,
        confidence,
    }
}

/// Detect regression with raw measurement data.
pub fn detect_regression_with_raw(
    current: &BenchmarkResult,
    current_raw: &[f64],
    baseline: &BenchmarkResult,
    baseline_raw: &[f64],
    config: &RegressionConfig,
) -> RegressionResult {
    let percentage_change = ((current.statistics.mean_ns - baseline.statistics.mean_ns)
        / baseline.statistics.mean_ns)
        * 100.0;

    let significance =
        if current_raw.len() >= config.min_samples && baseline_raw.len() >= config.min_samples {
            Some(match config.significance_test {
                SignificanceTest::WelchT => {
                    welch_t_test(current_raw, baseline_raw, config.significance_level)
                }
                SignificanceTest::MannWhitney => {
                    mann_whitney_u_test(current_raw, baseline_raw, config.significance_level)
                }
                SignificanceTest::Bootstrap => bootstrap_test(
                    current_raw,
                    baseline_raw,
                    config.significance_level,
                    config.bootstrap_iterations,
                ),
                SignificanceTest::None => return detect_regression(current, baseline, config),
            })
        } else {
            None
        };

    let is_significant = significance
        .as_ref()
        .map(|s| s.is_significant)
        .unwrap_or(true);
    let exceeds_threshold = percentage_change > config.threshold_percent;

    let is_regression = if config.require_significance {
        exceeds_threshold && is_significant
    } else {
        exceeds_threshold
    };

    let confidence = calculate_regression_confidence(
        percentage_change,
        config.threshold_percent,
        significance.as_ref().map(|s| s.p_value).unwrap_or(0.5),
    );

    RegressionResult {
        name: current.name.clone(),
        current_version: current
            .metadata
            .get("version")
            .cloned()
            .unwrap_or_else(|| "current".to_string()),
        baseline_version: baseline
            .metadata
            .get("version")
            .cloned()
            .unwrap_or_else(|| "baseline".to_string()),
        current_mean_ns: current.statistics.mean_ns,
        baseline_mean_ns: baseline.statistics.mean_ns,
        percentage_change,
        is_regression,
        significance,
        confidence,
    }
}

/// Generate synthetic samples from mean and std dev (for testing without raw data).
fn generate_samples(mean: f64, std_dev: f64, n: usize) -> Vec<f64> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    let mut seed = 42u64;

    (0..n)
        .map(|i| {
            i.hash(&mut hasher);
            seed.hash(&mut hasher);
            seed = hasher.finish();

            // Box-Muller transform for normal distribution
            let u1 = (seed % 10000) as f64 / 10000.0;
            seed.hash(&mut hasher);
            seed = hasher.finish();
            let u2 = (seed % 10000) as f64 / 10000.0;

            let z = (-2.0 * u1.max(0.0001).ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
            mean + z * std_dev
        })
        .collect()
}

/// Calculate confidence score for regression detection.
fn calculate_regression_confidence(percentage_change: f64, threshold: f64, p_value: f64) -> f64 {
    // Confidence based on how much the change exceeds threshold
    let threshold_factor = if percentage_change > 0.0 {
        (percentage_change / threshold).min(2.0) / 2.0
    } else {
        0.0
    };

    // Confidence based on p-value (lower p = higher confidence)
    let p_factor = (1.0 - p_value).max(0.0);

    // Combine factors
    (threshold_factor * 0.4 + p_factor * 0.6).min(1.0)
}

// ============================================================================
// Baseline Manager
// ============================================================================

/// Manages baseline benchmarks and comparisons.
pub struct BaselineManager {
    /// Directory containing baseline source files.
    baselines_dir: PathBuf,
    /// Output directory for compiled binaries.
    output_dir: PathBuf,
    /// Discovered baselines.
    baselines: HashMap<String, Vec<BaselineBenchmark>>,
}

impl BaselineManager {
    /// Create a new baseline manager.
    pub fn new(baselines_dir: PathBuf, output_dir: PathBuf) -> Self {
        Self {
            baselines_dir,
            output_dir,
            baselines: HashMap::new(),
        }
    }

    /// Discover baselines in the directory.
    pub fn discover(&mut self) -> Result<()> {
        for lang in [
            BaselineLanguage::C,
            BaselineLanguage::Rust,
            BaselineLanguage::Go,
        ] {
            let lang_dir = self
                .baselines_dir
                .join(format!("vs_{}", lang.to_string().to_lowercase()));
            if !lang_dir.exists() {
                continue;
            }

            for entry in std::fs::read_dir(&lang_dir)? {
                let entry = entry?;
                let path = entry.path();

                if path.extension().and_then(|e| e.to_str()) == Some(lang.extension()) {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();

                    let baseline = BaselineBenchmark::new(&name, lang, path);

                    self.baselines
                        .entry(name.clone())
                        .or_insert_with(Vec::new)
                        .push(baseline);
                }
            }
        }

        Ok(())
    }

    /// Compile all baselines.
    pub fn compile_all(&mut self) -> Result<()> {
        std::fs::create_dir_all(&self.output_dir)?;

        for baselines in self.baselines.values_mut() {
            for baseline in baselines.iter_mut() {
                baseline.compile(&self.output_dir)?;
            }
        }

        Ok(())
    }

    /// Run all baselines.
    pub fn run_all(&self, warmup: usize) -> Result<Vec<BaselineResult>> {
        let mut results = Vec::new();

        for baselines in self.baselines.values() {
            for baseline in baselines {
                results.push(baseline.run(warmup)?);
            }
        }

        Ok(results)
    }

    /// Get baselines for a specific benchmark name.
    pub fn get_baselines(&self, name: &str) -> Option<&Vec<BaselineBenchmark>> {
        self.baselines.get(name)
    }

    /// Compare Verum results against baselines.
    pub fn compare_all(
        &self,
        verum_results: &[BenchmarkResult],
        baseline_results: &[BaselineResult],
        target_min: f64,
        target_max: f64,
    ) -> Vec<ComparisonResult> {
        let mut comparisons = Vec::new();

        for verum in verum_results {
            // Find matching baseline by name
            for baseline in baseline_results {
                if verum.name.contains(&baseline.name) {
                    comparisons.push(ComparisonResult::compare(
                        verum, baseline, target_min, target_max,
                    ));
                }
            }
        }

        comparisons
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::BenchmarkCategory;
    use std::time::Duration;

    fn make_stats(mean: f64, std_dev: f64, count: usize) -> Statistics {
        Statistics {
            count,
            min_ns: mean - 2.0 * std_dev,
            max_ns: mean + 2.0 * std_dev,
            mean_ns: mean,
            median_ns: mean,
            std_dev_ns: std_dev,
            cv: std_dev / mean,
            p5_ns: mean - 1.5 * std_dev,
            p25_ns: mean - 0.5 * std_dev,
            p75_ns: mean + 0.5 * std_dev,
            p95_ns: mean + 1.5 * std_dev,
            p99_ns: mean + 2.0 * std_dev,
            iqr_ns: std_dev,
            total_duration: Duration::from_nanos((mean * count as f64) as u64),
        }
    }

    #[test]
    fn test_comparison_assessment() {
        let verum_stats = make_stats(15.0, 2.0, 100);
        let baseline = BaselineResult {
            name: "test".to_string(),
            language: BaselineLanguage::C,
            statistics: make_stats(14.0, 2.0, 100),
            raw_measurements_ns: vec![],
        };

        let verum_result = BenchmarkResult::new(
            "test".to_string(),
            BenchmarkCategory::Micro,
            verum_stats,
            None,
        );

        let comparison = ComparisonResult::compare(&verum_result, &baseline, 0.85, 0.95);

        assert!(comparison.ratio > 1.0); // Verum is slower
        assert!(comparison.ratio < 1.1); // But within 10%
        assert_eq!(comparison.assessment, ComparisonAssessment::Comparable);
    }

    #[test]
    fn test_regression_detection() {
        let current = BenchmarkResult::new(
            "test".to_string(),
            BenchmarkCategory::Micro,
            make_stats(25.0, 2.0, 100), // 25% slower
            None,
        );

        let baseline = BenchmarkResult::new(
            "test".to_string(),
            BenchmarkCategory::Micro,
            make_stats(20.0, 2.0, 100),
            None,
        );

        let config = RegressionConfig::default().with_require_significance(false); // Don't require significance for this test
        let result = detect_regression(&current, &baseline, &config);

        assert!(result.is_regression);
        assert!(result.percentage_change > 20.0);
    }

    #[test]
    fn test_welch_t_test() {
        // Test with identical distributions
        let sample1: Vec<f64> = (0..100).map(|i| 100.0 + (i as f64 * 0.1)).collect();
        let sample2: Vec<f64> = (0..100).map(|i| 100.0 + (i as f64 * 0.1)).collect();
        let result = welch_t_test(&sample1, &sample2, 0.05);
        assert!(result.p_value > 0.5); // Should be high (no difference)
        assert!(!result.is_significant);

        // Test with different distributions (with some variance)
        let sample1: Vec<f64> = (0..100).map(|i| 100.0 + (i % 10) as f64).collect();
        let sample2: Vec<f64> = (0..100).map(|i| 150.0 + (i % 10) as f64).collect();
        let result = welch_t_test(&sample1, &sample2, 0.05);
        // With a 50 unit difference in means and variance of ~8.25, t should be very large
        assert!(
            result.p_value < 0.05,
            "p_value={} should be < 0.05",
            result.p_value
        );
    }

    #[test]
    fn test_mann_whitney_u() {
        // Test with identical distributions
        let sample1: Vec<f64> = (0..50).map(|i| i as f64).collect();
        let sample2: Vec<f64> = (0..50).map(|i| i as f64).collect();
        let result = mann_whitney_u_test(&sample1, &sample2, 0.05);
        assert!(result.p_value > 0.5);

        // Test with clearly different distributions
        let sample1: Vec<f64> = (0..50).map(|i| i as f64).collect();
        let sample2: Vec<f64> = (50..100).map(|i| i as f64).collect();
        let result = mann_whitney_u_test(&sample1, &sample2, 0.05);
        assert!(result.p_value < 0.05);
        assert!(result.is_significant);
    }

    #[test]
    fn test_bootstrap() {
        let sample1: Vec<f64> = (0..50).map(|i| 100.0 + i as f64).collect();
        let sample2: Vec<f64> = (0..50).map(|i| 100.0 + i as f64).collect();
        let result = bootstrap_test(&sample1, &sample2, 0.05, 1000);
        assert!(!result.is_significant);
    }

    #[test]
    fn test_effect_size_interpretation() {
        assert_eq!(
            interpret_cohens_d(0.1),
            EffectSizeInterpretation::Negligible
        );
        assert_eq!(interpret_cohens_d(0.3), EffectSizeInterpretation::Small);
        assert_eq!(interpret_cohens_d(0.6), EffectSizeInterpretation::Medium);
        assert_eq!(interpret_cohens_d(1.0), EffectSizeInterpretation::Large);
    }

    #[test]
    fn test_regression_config_builder() {
        let config = RegressionConfig::default()
            .with_threshold(10.0)
            .with_significance_test(SignificanceTest::WelchT)
            .with_require_significance(true);

        assert_eq!(config.threshold_percent, 10.0);
        assert_eq!(config.significance_test, SignificanceTest::WelchT);
        assert!(config.require_significance);
    }

    #[test]
    fn test_regression_summary() {
        let result = RegressionResult {
            name: "test_bench".to_string(),
            current_version: "v2".to_string(),
            baseline_version: "v1".to_string(),
            current_mean_ns: 120.0,
            baseline_mean_ns: 100.0,
            percentage_change: 20.0,
            is_regression: true,
            significance: None,
            confidence: 0.85,
        };

        let summary = result.summary();
        assert!(summary.contains("REGRESSION"));
        assert!(summary.contains("20.0%"));
    }
}
