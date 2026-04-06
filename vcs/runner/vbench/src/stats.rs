//! Statistical analysis module for VBench.
//!
//! This module provides comprehensive statistical analysis capabilities for
//! benchmark results, including:
//!
//! - Descriptive statistics (mean, std dev, percentiles)
//! - Statistical significance testing (Welch's t-test, Mann-Whitney U)
//! - Effect size calculations (Cohen's d, Cliff's delta)
//! - Outlier detection (IQR, modified Z-score)
//! - Confidence intervals and bootstrap resampling
//! - Distribution analysis and normality tests
//!
//! # Example
//!
//! ```
//! use vbench::stats::DescriptiveStats;
//!
//! let samples = vec![10.0, 12.0, 11.0, 13.0, 10.5, 11.5, 12.5, 10.0, 11.0, 12.0];
//! let stats = DescriptiveStats::from_samples(&samples).unwrap();
//!
//! println!("Mean: {:.2}ns +/- {:.2}ns", stats.mean, stats.std_dev);
//! println!("95% CI: [{:.2}, {:.2}]", stats.ci_lower_95, stats.ci_upper_95);
//! ```

use std::cmp::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};

// ============================================================================
// Descriptive Statistics
// ============================================================================

/// Comprehensive descriptive statistics for a sample.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescriptiveStats {
    /// Number of samples.
    pub count: usize,
    /// Minimum value.
    pub min: f64,
    /// Maximum value.
    pub max: f64,
    /// Range (max - min).
    pub range: f64,
    /// Arithmetic mean.
    pub mean: f64,
    /// Geometric mean (if all values positive).
    pub geometric_mean: Option<f64>,
    /// Harmonic mean (if all values positive).
    pub harmonic_mean: Option<f64>,
    /// Median (50th percentile).
    pub median: f64,
    /// Mode (most frequent value, if exists).
    pub mode: Option<f64>,
    /// Sample standard deviation (using n-1).
    pub std_dev: f64,
    /// Population standard deviation (using n).
    pub std_dev_pop: f64,
    /// Sample variance.
    pub variance: f64,
    /// Population variance.
    pub variance_pop: f64,
    /// Standard error of the mean.
    pub std_error: f64,
    /// Coefficient of variation (std_dev / mean).
    pub cv: f64,
    /// Skewness (asymmetry of distribution).
    pub skewness: f64,
    /// Kurtosis (tailedness of distribution).
    pub kurtosis: f64,
    /// Excess kurtosis (kurtosis - 3).
    pub excess_kurtosis: f64,
    /// 1st percentile.
    pub p1: f64,
    /// 5th percentile.
    pub p5: f64,
    /// 10th percentile.
    pub p10: f64,
    /// 25th percentile (Q1).
    pub p25: f64,
    /// 75th percentile (Q3).
    pub p75: f64,
    /// 90th percentile.
    pub p90: f64,
    /// 95th percentile.
    pub p95: f64,
    /// 99th percentile.
    pub p99: f64,
    /// 99.9th percentile.
    pub p999: f64,
    /// Interquartile range (Q3 - Q1).
    pub iqr: f64,
    /// Lower 95% confidence interval bound.
    pub ci_lower_95: f64,
    /// Upper 95% confidence interval bound.
    pub ci_upper_95: f64,
    /// Lower 99% confidence interval bound.
    pub ci_lower_99: f64,
    /// Upper 99% confidence interval bound.
    pub ci_upper_99: f64,
    /// Sum of all values.
    pub sum: f64,
}

impl DescriptiveStats {
    /// Calculate descriptive statistics from a slice of samples.
    pub fn from_samples(samples: &[f64]) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }

        let count = samples.len();
        let mut sorted: Vec<f64> = samples.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

        let min = sorted[0];
        let max = sorted[count - 1];
        let range = max - min;

        // Sum and mean
        let sum: f64 = samples.iter().sum();
        let mean = sum / count as f64;

        // Geometric mean (only if all positive)
        let geometric_mean = if samples.iter().all(|&x| x > 0.0) {
            let log_sum: f64 = samples.iter().map(|x| x.ln()).sum();
            Some((log_sum / count as f64).exp())
        } else {
            None
        };

        // Harmonic mean (only if all positive)
        let harmonic_mean = if samples.iter().all(|&x| x > 0.0) {
            let reciprocal_sum: f64 = samples.iter().map(|x| 1.0 / x).sum();
            Some(count as f64 / reciprocal_sum)
        } else {
            None
        };

        // Median
        let median = percentile(&sorted, 50.0);

        // Mode (simple implementation)
        let mode = calculate_mode(samples);

        // Variance and standard deviation
        let variance_sum: f64 = samples.iter().map(|x| (x - mean).powi(2)).sum();
        let variance = if count > 1 {
            variance_sum / (count - 1) as f64
        } else {
            0.0
        };
        let variance_pop = variance_sum / count as f64;
        let std_dev = variance.sqrt();
        let std_dev_pop = variance_pop.sqrt();

        // Standard error
        let std_error = std_dev / (count as f64).sqrt();

        // Coefficient of variation
        let cv = if mean != 0.0 {
            std_dev / mean.abs()
        } else {
            0.0
        };

        // Skewness (Fisher's method)
        let skewness = if std_dev > 0.0 && count > 2 {
            let n = count as f64;
            let skew_sum: f64 = samples.iter().map(|x| ((x - mean) / std_dev).powi(3)).sum();
            (n / ((n - 1.0) * (n - 2.0))) * skew_sum
        } else {
            0.0
        };

        // Kurtosis (Fisher's method)
        let (kurtosis, excess_kurtosis) = if std_dev > 0.0 && count > 3 {
            let n = count as f64;
            let kurt_sum: f64 = samples.iter().map(|x| ((x - mean) / std_dev).powi(4)).sum();
            let k = ((n * (n + 1.0)) / ((n - 1.0) * (n - 2.0) * (n - 3.0))) * kurt_sum;
            let excess = k - (3.0 * (n - 1.0).powi(2)) / ((n - 2.0) * (n - 3.0));
            (k, excess)
        } else {
            (3.0, 0.0)
        };

        // Percentiles
        let p1 = percentile(&sorted, 1.0);
        let p5 = percentile(&sorted, 5.0);
        let p10 = percentile(&sorted, 10.0);
        let p25 = percentile(&sorted, 25.0);
        let p75 = percentile(&sorted, 75.0);
        let p90 = percentile(&sorted, 90.0);
        let p95 = percentile(&sorted, 95.0);
        let p99 = percentile(&sorted, 99.0);
        let p999 = percentile(&sorted, 99.9);
        let iqr = p75 - p25;

        // Confidence intervals (using t-distribution for small samples)
        let t_95 = t_critical_value(count, 0.95);
        let t_99 = t_critical_value(count, 0.99);
        let margin_95 = t_95 * std_error;
        let margin_99 = t_99 * std_error;

        Some(Self {
            count,
            min,
            max,
            range,
            mean,
            geometric_mean,
            harmonic_mean,
            median,
            mode,
            std_dev,
            std_dev_pop,
            variance,
            variance_pop,
            std_error,
            cv,
            skewness,
            kurtosis,
            excess_kurtosis,
            p1,
            p5,
            p10,
            p25,
            p75,
            p90,
            p95,
            p99,
            p999,
            iqr,
            ci_lower_95: mean - margin_95,
            ci_upper_95: mean + margin_95,
            ci_lower_99: mean - margin_99,
            ci_upper_99: mean + margin_99,
            sum,
        })
    }

    /// Calculate descriptive statistics from durations (converted to nanoseconds).
    pub fn from_durations(durations: &[Duration]) -> Option<Self> {
        let samples: Vec<f64> = durations.iter().map(|d| d.as_nanos() as f64).collect();
        Self::from_samples(&samples)
    }

    /// Check if the mean is within a threshold.
    pub fn within_threshold(&self, threshold: f64) -> bool {
        self.mean <= threshold
    }

    /// Calculate the margin of error at a given confidence level.
    pub fn margin_of_error(&self, confidence: f64) -> f64 {
        let t = t_critical_value(self.count, confidence);
        t * self.std_error
    }

    /// Get confidence interval at a given level.
    pub fn confidence_interval(&self, confidence: f64) -> (f64, f64) {
        let margin = self.margin_of_error(confidence);
        (self.mean - margin, self.mean + margin)
    }

    /// Determine if the distribution appears normal based on skewness and kurtosis.
    pub fn appears_normal(&self) -> bool {
        // Rule of thumb: skewness near 0, excess kurtosis near 0
        self.skewness.abs() < 1.0 && self.excess_kurtosis.abs() < 2.0
    }
}

// ============================================================================
// Statistical Significance Testing
// ============================================================================

/// Result of a statistical significance test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignificanceResult {
    /// Name of the test performed.
    pub test_name: String,
    /// The test statistic value.
    pub statistic: f64,
    /// The p-value.
    pub p_value: f64,
    /// Degrees of freedom (if applicable).
    pub df: Option<f64>,
    /// Whether the result is significant at alpha = 0.05.
    pub significant_05: bool,
    /// Whether the result is significant at alpha = 0.01.
    pub significant_01: bool,
    /// Effect size (Cohen's d or Cliff's delta).
    pub effect_size: f64,
    /// Interpretation of effect size.
    pub effect_interpretation: EffectSize,
    /// The confidence interval for the difference.
    pub ci_difference: Option<(f64, f64)>,
}

/// Effect size interpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectSize {
    /// Negligible effect (|d| < 0.2).
    Negligible,
    /// Small effect (0.2 <= |d| < 0.5).
    Small,
    /// Medium effect (0.5 <= |d| < 0.8).
    Medium,
    /// Large effect (|d| >= 0.8).
    Large,
}

impl std::fmt::Display for EffectSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Negligible => write!(f, "negligible"),
            Self::Small => write!(f, "small"),
            Self::Medium => write!(f, "medium"),
            Self::Large => write!(f, "large"),
        }
    }
}

/// Perform Welch's t-test for two independent samples with unequal variances.
///
/// This is the recommended test for comparing benchmark results as it does not
/// assume equal variances between groups.
pub fn welch_t_test(sample1: &[f64], sample2: &[f64]) -> Option<SignificanceResult> {
    if sample1.len() < 2 || sample2.len() < 2 {
        return None;
    }

    let n1 = sample1.len() as f64;
    let n2 = sample2.len() as f64;

    let mean1: f64 = sample1.iter().sum::<f64>() / n1;
    let mean2: f64 = sample2.iter().sum::<f64>() / n2;

    let var1: f64 = sample1.iter().map(|x| (x - mean1).powi(2)).sum::<f64>() / (n1 - 1.0);
    let var2: f64 = sample2.iter().map(|x| (x - mean2).powi(2)).sum::<f64>() / (n2 - 1.0);

    // Welch's t-statistic
    let se = (var1 / n1 + var2 / n2).sqrt();
    if se == 0.0 {
        return None;
    }

    let t = (mean1 - mean2) / se;

    // Welch-Satterthwaite degrees of freedom
    let num = (var1 / n1 + var2 / n2).powi(2);
    let denom = (var1 / n1).powi(2) / (n1 - 1.0) + (var2 / n2).powi(2) / (n2 - 1.0);
    let df = num / denom;

    // Calculate p-value (two-tailed)
    let p_value = 2.0 * (1.0 - t_cdf(t.abs(), df));

    // Cohen's d (pooled standard deviation)
    let pooled_var = ((n1 - 1.0) * var1 + (n2 - 1.0) * var2) / (n1 + n2 - 2.0);
    let cohens_d = (mean1 - mean2) / pooled_var.sqrt();
    let effect_interpretation = interpret_cohens_d(cohens_d);

    // Confidence interval for the difference
    let t_crit = t_critical_value((n1 + n2) as usize, 0.95);
    let ci_margin = t_crit * se;
    let diff = mean1 - mean2;

    Some(SignificanceResult {
        test_name: "Welch's t-test".to_string(),
        statistic: t,
        p_value,
        df: Some(df),
        significant_05: p_value < 0.05,
        significant_01: p_value < 0.01,
        effect_size: cohens_d,
        effect_interpretation,
        ci_difference: Some((diff - ci_margin, diff + ci_margin)),
    })
}

/// Perform Mann-Whitney U test (Wilcoxon rank-sum test).
///
/// A non-parametric alternative to the t-test that doesn't assume normal distribution.
/// Recommended when sample sizes are small or distributions are skewed.
pub fn mann_whitney_u_test(sample1: &[f64], sample2: &[f64]) -> Option<SignificanceResult> {
    if sample1.is_empty() || sample2.is_empty() {
        return None;
    }

    let n1 = sample1.len();
    let n2 = sample2.len();

    // Combine and rank all values
    let mut combined: Vec<(f64, usize)> = Vec::with_capacity(n1 + n2);
    for &x in sample1 {
        combined.push((x, 0)); // 0 = sample1
    }
    for &x in sample2 {
        combined.push((x, 1)); // 1 = sample2
    }
    combined.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));

    // Assign ranks (handling ties with average rank)
    let ranks = calculate_ranks(&combined);

    // Sum of ranks for sample1
    let r1: f64 = combined
        .iter()
        .zip(ranks.iter())
        .filter(|((_, group), _)| *group == 0)
        .map(|(_, rank)| *rank)
        .sum();

    // Calculate U statistic
    let u1 = r1 - (n1 as f64 * (n1 as f64 + 1.0)) / 2.0;
    let u2 = (n1 as f64 * n2 as f64) - u1;
    let u = u1.min(u2);

    // Normal approximation for p-value (works well for n1, n2 >= 10)
    let mean_u = (n1 as f64 * n2 as f64) / 2.0;
    let std_u = ((n1 as f64 * n2 as f64 * (n1 as f64 + n2 as f64 + 1.0)) / 12.0).sqrt();

    let z = if std_u > 0.0 {
        (u - mean_u) / std_u
    } else {
        0.0
    };

    let p_value = 2.0 * (1.0 - normal_cdf(z.abs()));

    // Cliff's delta (effect size for rank-based tests)
    let cliffs_delta = (u1 - u2) / (n1 as f64 * n2 as f64);
    let effect_interpretation = interpret_cliffs_delta(cliffs_delta);

    Some(SignificanceResult {
        test_name: "Mann-Whitney U test".to_string(),
        statistic: u,
        p_value,
        df: None,
        significant_05: p_value < 0.05,
        significant_01: p_value < 0.01,
        effect_size: cliffs_delta,
        effect_interpretation,
        ci_difference: None,
    })
}

/// Perform a paired t-test for dependent samples.
///
/// Use this when comparing before/after measurements on the same subjects.
pub fn paired_t_test(sample1: &[f64], sample2: &[f64]) -> Option<SignificanceResult> {
    if sample1.len() != sample2.len() || sample1.len() < 2 {
        return None;
    }

    let n = sample1.len() as f64;
    let differences: Vec<f64> = sample1
        .iter()
        .zip(sample2.iter())
        .map(|(a, b)| a - b)
        .collect();

    let mean_diff: f64 = differences.iter().sum::<f64>() / n;
    let var_diff: f64 = differences
        .iter()
        .map(|d| (d - mean_diff).powi(2))
        .sum::<f64>()
        / (n - 1.0);
    let se = var_diff.sqrt() / n.sqrt();

    if se == 0.0 {
        return None;
    }

    let t = mean_diff / se;
    let df = n - 1.0;
    let p_value = 2.0 * (1.0 - t_cdf(t.abs(), df));

    // Effect size (Cohen's d for paired samples)
    let cohens_d = mean_diff / var_diff.sqrt();
    let effect_interpretation = interpret_cohens_d(cohens_d);

    // Confidence interval
    let t_crit = t_critical_value(sample1.len(), 0.95);
    let ci_margin = t_crit * se;

    Some(SignificanceResult {
        test_name: "Paired t-test".to_string(),
        statistic: t,
        p_value,
        df: Some(df),
        significant_05: p_value < 0.05,
        significant_01: p_value < 0.01,
        effect_size: cohens_d,
        effect_interpretation,
        ci_difference: Some((mean_diff - ci_margin, mean_diff + ci_margin)),
    })
}

// ============================================================================
// Bootstrap Resampling
// ============================================================================

/// Result of bootstrap resampling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapResult {
    /// Bootstrap estimate of the mean.
    pub mean: f64,
    /// Bootstrap estimate of the standard error.
    pub std_error: f64,
    /// Bootstrap confidence interval (lower bound).
    pub ci_lower: f64,
    /// Bootstrap confidence interval (upper bound).
    pub ci_upper: f64,
    /// Confidence level used.
    pub confidence: f64,
    /// Number of bootstrap samples.
    pub n_bootstrap: usize,
    /// Bias (difference between bootstrap mean and sample mean).
    pub bias: f64,
}

/// Perform bootstrap resampling to estimate confidence intervals.
///
/// Bootstrap is particularly useful for small samples or non-normal distributions.
pub fn bootstrap_ci(
    samples: &[f64],
    n_bootstrap: usize,
    confidence: f64,
) -> Option<BootstrapResult> {
    if samples.is_empty() {
        return None;
    }

    let n = samples.len();
    let sample_mean: f64 = samples.iter().sum::<f64>() / n as f64;

    // Generate bootstrap samples using a simple PRNG
    let mut rng = SimpleRng::new(42);
    let mut bootstrap_means: Vec<f64> = Vec::with_capacity(n_bootstrap);

    for _ in 0..n_bootstrap {
        let mut sum = 0.0;
        for _ in 0..n {
            let idx = rng.next_usize() % n;
            sum += samples[idx];
        }
        bootstrap_means.push(sum / n as f64);
    }

    bootstrap_means.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

    let mean: f64 = bootstrap_means.iter().sum::<f64>() / n_bootstrap as f64;
    let variance: f64 = bootstrap_means
        .iter()
        .map(|x| (x - mean).powi(2))
        .sum::<f64>()
        / n_bootstrap as f64;
    let std_error = variance.sqrt();

    // Percentile method for CI
    let alpha = 1.0 - confidence;
    let lower_idx = ((alpha / 2.0) * n_bootstrap as f64) as usize;
    let upper_idx = ((1.0 - alpha / 2.0) * n_bootstrap as f64) as usize;

    Some(BootstrapResult {
        mean,
        std_error,
        ci_lower: bootstrap_means[lower_idx.min(n_bootstrap - 1)],
        ci_upper: bootstrap_means[upper_idx.min(n_bootstrap - 1)],
        confidence,
        n_bootstrap,
        bias: mean - sample_mean,
    })
}

/// Compare two samples using bootstrap resampling.
pub fn bootstrap_compare(
    sample1: &[f64],
    sample2: &[f64],
    n_bootstrap: usize,
) -> Option<BootstrapComparisonResult> {
    if sample1.is_empty() || sample2.is_empty() {
        return None;
    }

    let n1 = sample1.len();
    let n2 = sample2.len();

    let mean1: f64 = sample1.iter().sum::<f64>() / n1 as f64;
    let mean2: f64 = sample2.iter().sum::<f64>() / n2 as f64;
    let observed_diff = mean1 - mean2;

    // Generate bootstrap differences
    let mut rng = SimpleRng::new(42);
    let mut bootstrap_diffs: Vec<f64> = Vec::with_capacity(n_bootstrap);

    for _ in 0..n_bootstrap {
        let mut sum1 = 0.0;
        for _ in 0..n1 {
            let idx = rng.next_usize() % n1;
            sum1 += sample1[idx];
        }

        let mut sum2 = 0.0;
        for _ in 0..n2 {
            let idx = rng.next_usize() % n2;
            sum2 += sample2[idx];
        }

        bootstrap_diffs.push(sum1 / n1 as f64 - sum2 / n2 as f64);
    }

    bootstrap_diffs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

    // Calculate p-value (proportion of bootstrap diffs with opposite sign)
    let p_value = if observed_diff >= 0.0 {
        bootstrap_diffs.iter().filter(|&&d| d <= 0.0).count() as f64 / n_bootstrap as f64
    } else {
        bootstrap_diffs.iter().filter(|&&d| d >= 0.0).count() as f64 / n_bootstrap as f64
    };

    // 95% CI
    let lower_idx = (0.025 * n_bootstrap as f64) as usize;
    let upper_idx = (0.975 * n_bootstrap as f64) as usize;

    Some(BootstrapComparisonResult {
        observed_difference: observed_diff,
        ci_lower: bootstrap_diffs[lower_idx.min(n_bootstrap - 1)],
        ci_upper: bootstrap_diffs[upper_idx.min(n_bootstrap - 1)],
        p_value: 2.0 * p_value.min(1.0 - p_value), // Two-tailed
        n_bootstrap,
    })
}

/// Result of bootstrap comparison between two samples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapComparisonResult {
    /// Observed difference in means.
    pub observed_difference: f64,
    /// Lower bound of 95% CI for the difference.
    pub ci_lower: f64,
    /// Upper bound of 95% CI for the difference.
    pub ci_upper: f64,
    /// Two-tailed p-value.
    pub p_value: f64,
    /// Number of bootstrap samples.
    pub n_bootstrap: usize,
}

// ============================================================================
// Outlier Detection
// ============================================================================

/// Outlier detection result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlierAnalysis {
    /// Number of outliers detected.
    pub count: usize,
    /// Indices of outlier values.
    pub indices: Vec<usize>,
    /// The outlier values.
    pub values: Vec<f64>,
    /// Lower threshold for outliers.
    pub lower_threshold: f64,
    /// Upper threshold for outliers.
    pub upper_threshold: f64,
    /// Method used for detection.
    pub method: OutlierMethod,
}

/// Method for outlier detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutlierMethod {
    /// Interquartile range method (Q1 - 1.5*IQR, Q3 + 1.5*IQR).
    Iqr,
    /// Modified Z-score using median absolute deviation.
    ModifiedZScore,
    /// Tukey's fences (Q1 - k*IQR, Q3 + k*IQR with customizable k).
    TukeyFences,
}

impl std::fmt::Display for OutlierMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Iqr => write!(f, "IQR"),
            Self::ModifiedZScore => write!(f, "Modified Z-Score"),
            Self::TukeyFences => write!(f, "Tukey's Fences"),
        }
    }
}

/// Detect outliers using the IQR method.
pub fn detect_outliers_iqr(samples: &[f64], k: f64) -> OutlierAnalysis {
    let mut sorted: Vec<f64> = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

    let q1 = percentile(&sorted, 25.0);
    let q3 = percentile(&sorted, 75.0);
    let iqr = q3 - q1;

    let lower = q1 - k * iqr;
    let upper = q3 + k * iqr;

    let mut indices = Vec::new();
    let mut values = Vec::new();

    for (i, &v) in samples.iter().enumerate() {
        if v < lower || v > upper {
            indices.push(i);
            values.push(v);
        }
    }

    OutlierAnalysis {
        count: indices.len(),
        indices,
        values,
        lower_threshold: lower,
        upper_threshold: upper,
        method: OutlierMethod::Iqr,
    }
}

/// Detect outliers using the modified Z-score method.
///
/// Uses median absolute deviation (MAD) instead of standard deviation,
/// making it more robust to outliers.
pub fn detect_outliers_mad(samples: &[f64], threshold: f64) -> OutlierAnalysis {
    let mut sorted: Vec<f64> = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

    let median = percentile(&sorted, 50.0);

    // Calculate MAD
    let mut deviations: Vec<f64> = samples.iter().map(|x| (x - median).abs()).collect();
    deviations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let mad = percentile(&deviations, 50.0);

    // Modified Z-scores
    let k = 0.6745; // consistency constant for normal distribution
    let modified_mad = mad / k;

    let mut indices = Vec::new();
    let mut values = Vec::new();

    if modified_mad > 0.0 {
        for (i, &v) in samples.iter().enumerate() {
            let z = (v - median).abs() / modified_mad;
            if z > threshold {
                indices.push(i);
                values.push(v);
            }
        }
    }

    // Calculate thresholds for reporting
    let lower = median - threshold * modified_mad;
    let upper = median + threshold * modified_mad;

    OutlierAnalysis {
        count: indices.len(),
        indices,
        values,
        lower_threshold: lower,
        upper_threshold: upper,
        method: OutlierMethod::ModifiedZScore,
    }
}

/// Remove outliers from a sample and return the filtered data.
pub fn remove_outliers(samples: &[f64], method: OutlierMethod) -> Vec<f64> {
    let analysis = match method {
        OutlierMethod::Iqr => detect_outliers_iqr(samples, 1.5),
        OutlierMethod::ModifiedZScore => detect_outliers_mad(samples, 3.5),
        OutlierMethod::TukeyFences => detect_outliers_iqr(samples, 1.5),
    };

    samples
        .iter()
        .enumerate()
        .filter(|(i, _)| !analysis.indices.contains(i))
        .map(|(_, &v)| v)
        .collect()
}

// ============================================================================
// Regression Detection
// ============================================================================

/// Result of comparing two benchmark runs for regression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionAnalysis {
    /// Name of the benchmark.
    pub name: String,
    /// Whether a regression was detected.
    pub is_regression: bool,
    /// Whether an improvement was detected.
    pub is_improvement: bool,
    /// Percentage change (positive = regression, negative = improvement).
    pub percentage_change: f64,
    /// Absolute change in mean.
    pub absolute_change: f64,
    /// Statistical significance result.
    pub significance: Option<SignificanceResult>,
    /// Current mean value.
    pub current_mean: f64,
    /// Baseline mean value.
    pub baseline_mean: f64,
    /// Confidence that this is a real change.
    pub confidence: ChangeConfidence,
}

/// Confidence level for detected change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeConfidence {
    /// High confidence (p < 0.01, large effect).
    High,
    /// Medium confidence (p < 0.05, medium effect).
    Medium,
    /// Low confidence (p >= 0.05 or small effect).
    Low,
    /// No significant change detected.
    None,
}

impl std::fmt::Display for ChangeConfidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::High => write!(f, "high"),
            Self::Medium => write!(f, "medium"),
            Self::Low => write!(f, "low"),
            Self::None => write!(f, "none"),
        }
    }
}

/// Analyze potential regression between current and baseline samples.
pub fn analyze_regression(
    name: &str,
    current: &[f64],
    baseline: &[f64],
    threshold_percent: f64,
) -> RegressionAnalysis {
    let current_mean: f64 = current.iter().sum::<f64>() / current.len() as f64;
    let baseline_mean: f64 = baseline.iter().sum::<f64>() / baseline.len() as f64;

    let absolute_change = current_mean - baseline_mean;
    let percentage_change = if baseline_mean != 0.0 {
        (absolute_change / baseline_mean) * 100.0
    } else {
        0.0
    };

    // Perform statistical test
    let significance = welch_t_test(current, baseline);

    // Determine confidence based on p-value and effect size
    let confidence = match &significance {
        Some(sig) => {
            if sig.significant_01 && matches!(sig.effect_interpretation, EffectSize::Large) {
                ChangeConfidence::High
            } else if sig.significant_05
                && matches!(
                    sig.effect_interpretation,
                    EffectSize::Medium | EffectSize::Large
                )
            {
                ChangeConfidence::Medium
            } else if sig.significant_05 {
                ChangeConfidence::Low
            } else {
                ChangeConfidence::None
            }
        }
        None => ChangeConfidence::None,
    };

    // Determine if regression (significant slowdown above threshold)
    let is_regression = percentage_change > threshold_percent
        && confidence != ChangeConfidence::None
        && matches!(
            &significance,
            Some(s) if s.significant_05
        );

    // Determine if improvement (significant speedup)
    let is_improvement = percentage_change < -threshold_percent
        && confidence != ChangeConfidence::None
        && matches!(
            &significance,
            Some(s) if s.significant_05
        );

    RegressionAnalysis {
        name: name.to_string(),
        is_regression,
        is_improvement,
        percentage_change,
        absolute_change,
        significance,
        current_mean,
        baseline_mean,
        confidence,
    }
}

// ============================================================================
// Distribution Analysis
// ============================================================================

/// Result of distribution analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributionAnalysis {
    /// Whether the distribution appears normal.
    pub appears_normal: bool,
    /// Shapiro-Wilk W statistic (approximation).
    pub normality_statistic: f64,
    /// P-value for normality test.
    pub normality_p_value: f64,
    /// Detected distribution type.
    pub distribution_type: DistributionType,
    /// Histogram bin counts.
    pub histogram: Vec<(f64, f64, usize)>, // (lower, upper, count)
}

/// Type of distribution detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistributionType {
    /// Normal/Gaussian distribution.
    Normal,
    /// Right-skewed distribution.
    RightSkewed,
    /// Left-skewed distribution.
    LeftSkewed,
    /// Heavy-tailed distribution.
    HeavyTailed,
    /// Uniform distribution.
    Uniform,
    /// Bimodal distribution.
    Bimodal,
    /// Unknown distribution.
    Unknown,
}

impl std::fmt::Display for DistributionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "normal"),
            Self::RightSkewed => write!(f, "right-skewed"),
            Self::LeftSkewed => write!(f, "left-skewed"),
            Self::HeavyTailed => write!(f, "heavy-tailed"),
            Self::Uniform => write!(f, "uniform"),
            Self::Bimodal => write!(f, "bimodal"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Analyze the distribution of samples.
pub fn analyze_distribution(samples: &[f64], n_bins: usize) -> Option<DistributionAnalysis> {
    if samples.len() < 8 {
        return None;
    }

    let stats = DescriptiveStats::from_samples(samples)?;

    // Simple normality check based on skewness and kurtosis
    let appears_normal = stats.skewness.abs() < 1.0 && stats.excess_kurtosis.abs() < 2.0;

    // Determine distribution type
    let distribution_type = if appears_normal {
        DistributionType::Normal
    } else if stats.skewness > 1.0 {
        DistributionType::RightSkewed
    } else if stats.skewness < -1.0 {
        DistributionType::LeftSkewed
    } else if stats.excess_kurtosis > 3.0 {
        DistributionType::HeavyTailed
    } else if stats.excess_kurtosis < -1.5 {
        DistributionType::Uniform
    } else {
        DistributionType::Unknown
    };

    // Create histogram
    let histogram = create_histogram(samples, n_bins);

    // Approximate Shapiro-Wilk (using D'Agostino-Pearson approximation)
    let z_skew = stats.skewness / (6.0 / samples.len() as f64).sqrt();
    let z_kurt = stats.excess_kurtosis / (24.0 / samples.len() as f64).sqrt();
    let k2 = z_skew.powi(2) + z_kurt.powi(2);
    let normality_p_value = 1.0 - chi_squared_cdf(k2, 2.0);

    Some(DistributionAnalysis {
        appears_normal,
        normality_statistic: k2,
        normality_p_value,
        distribution_type,
        histogram,
    })
}

/// Create a histogram from samples.
fn create_histogram(samples: &[f64], n_bins: usize) -> Vec<(f64, f64, usize)> {
    if samples.is_empty() || n_bins == 0 {
        return Vec::new();
    }

    let min = samples.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = samples.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let bin_width = (max - min) / n_bins as f64;

    if bin_width == 0.0 {
        return vec![(min, max, samples.len())];
    }

    let mut bins: Vec<(f64, f64, usize)> = (0..n_bins)
        .map(|i| {
            let lower = min + i as f64 * bin_width;
            let upper = lower + bin_width;
            (lower, upper, 0)
        })
        .collect();

    for &sample in samples {
        let idx = ((sample - min) / bin_width) as usize;
        let idx = idx.min(n_bins - 1);
        bins[idx].2 += 1;
    }

    bins
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Calculate percentile from sorted values.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }

    let idx = (p / 100.0) * (sorted.len() - 1) as f64;
    let lower = idx.floor() as usize;
    let upper = idx.ceil() as usize;
    let frac = idx - lower as f64;

    if lower == upper || upper >= sorted.len() {
        sorted[lower.min(sorted.len() - 1)]
    } else {
        sorted[lower] * (1.0 - frac) + sorted[upper] * frac
    }
}

/// Calculate mode from samples.
fn calculate_mode(samples: &[f64]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }

    // Simple binning approach for continuous data
    let mut counts: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    let precision = 1e-10;

    for &x in samples {
        let key = (x / precision).round() as i64;
        *counts.entry(key).or_insert(0) += 1;
    }

    let max_count = counts.values().max()?;
    if *max_count == 1 {
        return None; // No repeated values
    }

    counts
        .iter()
        .filter(|&(_, c)| *c == *max_count)
        .map(|(&k, _)| k as f64 * precision)
        .next()
}

/// Calculate ranks with tie handling.
fn calculate_ranks(sorted_with_groups: &[(f64, usize)]) -> Vec<f64> {
    let n = sorted_with_groups.len();
    let mut ranks = vec![0.0; n];
    let mut i = 0;

    while i < n {
        let val = sorted_with_groups[i].0;
        let mut j = i;

        // Find all tied values
        while j < n && (sorted_with_groups[j].0 - val).abs() < 1e-10 {
            j += 1;
        }

        // Average rank for ties
        let avg_rank = (i + j + 1) as f64 / 2.0;
        for rank in ranks.iter_mut().take(j).skip(i) {
            *rank = avg_rank;
        }

        i = j;
    }

    ranks
}

/// T-distribution critical value (approximation).
fn t_critical_value(n: usize, confidence: f64) -> f64 {
    let df = (n - 1) as f64;
    let _alpha = 1.0 - confidence;

    // Use normal approximation for large df
    if df > 30.0 {
        // Z critical values for common confidence levels
        if confidence >= 0.99 {
            return 2.576;
        } else if confidence >= 0.95 {
            return 1.96;
        } else if confidence >= 0.90 {
            return 1.645;
        }
    }

    // Approximation for smaller df
    let z: f64 = if confidence >= 0.99 {
        2.576
    } else if confidence >= 0.95 {
        1.96
    } else {
        1.645
    };

    z + (z + z.powi(3)) / (4.0 * df)
        + (5.0 * z.powi(5) + 16.0 * z.powi(3) + 3.0 * z) / (96.0 * df * df)
}

/// T-distribution CDF (approximation).
fn t_cdf(t: f64, df: f64) -> f64 {
    // Use normal approximation for large df
    if df > 30.0 {
        return normal_cdf(t);
    }

    // Hill's algorithm approximation
    let x = df / (df + t * t);
    let a = df / 2.0;
    let b = 0.5;

    let result = incomplete_beta(x, a, b);
    if t >= 0.0 {
        1.0 - 0.5 * result
    } else {
        0.5 * result
    }
}

/// Normal distribution CDF (approximation using error function).
fn normal_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2))
}

/// Chi-squared CDF (approximation).
fn chi_squared_cdf(x: f64, df: f64) -> f64 {
    if x < 0.0 {
        return 0.0;
    }
    incomplete_gamma(df / 2.0, x / 2.0)
}

/// Error function (approximation).
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

/// Incomplete beta function (approximation).
fn incomplete_beta(x: f64, a: f64, b: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    if x == 1.0 {
        return 1.0;
    }

    // Lentz's continued fraction algorithm (simplified)
    let bt = if x == 0.0 || x == 1.0 {
        0.0
    } else {
        (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp()
    };

    if x < (a + 1.0) / (a + b + 2.0) {
        bt * beta_cf(x, a, b) / a
    } else {
        1.0 - bt * beta_cf(1.0 - x, b, a) / b
    }
}

/// Continued fraction for incomplete beta.
fn beta_cf(x: f64, a: f64, b: f64) -> f64 {
    const ITMAX: usize = 100;
    const EPS: f64 = 3.0e-7;
    const FPMIN: f64 = 1.0e-30;

    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;

    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < FPMIN {
        d = FPMIN;
    }
    d = 1.0 / d;
    let mut h = d;

    for m in 1..=ITMAX {
        let m = m as f64;
        let m2 = 2.0 * m;

        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        h *= d * c;

        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;

        if (del - 1.0).abs() < EPS {
            return h;
        }
    }

    h
}

/// Log gamma function (Lanczos approximation).
fn ln_gamma(x: f64) -> f64 {
    let g = 7.0;
    let c = [
        0.99999999999980993,
        676.5203681218851,
        -1259.1392167224028,
        771.32342877765313,
        -176.61502916214059,
        12.507343278686905,
        -0.13857109526572012,
        9.9843695780195716e-6,
        1.5056327351493116e-7,
    ];

    if x < 0.5 {
        std::f64::consts::PI.ln() - (std::f64::consts::PI * x).sin().ln() - ln_gamma(1.0 - x)
    } else {
        let x = x - 1.0;
        let mut a = c[0];
        for (i, &ci) in c.iter().enumerate().skip(1) {
            a += ci / (x + i as f64);
        }
        let t = x + g + 0.5;
        0.5 * (2.0 * std::f64::consts::PI).ln() + (t.ln() * (x + 0.5)) - t + a.ln()
    }
}

/// Incomplete gamma function (series approximation).
fn incomplete_gamma(a: f64, x: f64) -> f64 {
    if x < 0.0 || a <= 0.0 {
        return 0.0;
    }

    if x < a + 1.0 {
        // Series representation
        let mut sum = 1.0 / a;
        let mut term = 1.0 / a;
        for n in 1..100 {
            term *= x / (a + n as f64);
            sum += term;
            if term.abs() < sum.abs() * 1e-10 {
                break;
            }
        }
        sum * (-x + a * x.ln() - ln_gamma(a)).exp()
    } else {
        // Continued fraction representation
        1.0 - incomplete_gamma_cf(a, x)
    }
}

/// Continued fraction for incomplete gamma.
fn incomplete_gamma_cf(a: f64, x: f64) -> f64 {
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / 1e-30;
    let mut d = 1.0 / b;
    let mut h = d;

    for i in 1..100 {
        let an = -((i as f64) * ((i as f64) - a));
        b += 2.0;
        d = an * d + b;
        if d.abs() < 1e-30 {
            d = 1e-30;
        }
        c = b + an / c;
        if c.abs() < 1e-30 {
            c = 1e-30;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-10 {
            break;
        }
    }

    (-x + a * x.ln() - ln_gamma(a)).exp() * h
}

/// Interpret Cohen's d effect size.
fn interpret_cohens_d(d: f64) -> EffectSize {
    let abs_d = d.abs();
    if abs_d < 0.2 {
        EffectSize::Negligible
    } else if abs_d < 0.5 {
        EffectSize::Small
    } else if abs_d < 0.8 {
        EffectSize::Medium
    } else {
        EffectSize::Large
    }
}

/// Interpret Cliff's delta effect size.
fn interpret_cliffs_delta(delta: f64) -> EffectSize {
    let abs_delta = delta.abs();
    if abs_delta < 0.147 {
        EffectSize::Negligible
    } else if abs_delta < 0.33 {
        EffectSize::Small
    } else if abs_delta < 0.474 {
        EffectSize::Medium
    } else {
        EffectSize::Large
    }
}

/// Simple PRNG for bootstrap resampling.
struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        // xorshift64
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    fn next_usize(&mut self) -> usize {
        self.next_u64() as usize
    }
}

// ============================================================================
// Summary Statistics Comparison
// ============================================================================

/// Compare two sets of statistics and provide a summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonSummary {
    /// Name of the comparison.
    pub name: String,
    /// Current statistics.
    pub current: DescriptiveStats,
    /// Baseline statistics.
    pub baseline: DescriptiveStats,
    /// Percentage change in mean.
    pub mean_change_percent: f64,
    /// Absolute change in mean.
    pub mean_change_absolute: f64,
    /// Significance test result.
    pub significance: Option<SignificanceResult>,
    /// Regression analysis.
    pub regression: RegressionAnalysis,
}

impl ComparisonSummary {
    /// Create a comparison summary from two sample sets.
    pub fn from_samples(
        name: &str,
        current: &[f64],
        baseline: &[f64],
        threshold_percent: f64,
    ) -> Option<Self> {
        let current_stats = DescriptiveStats::from_samples(current)?;
        let baseline_stats = DescriptiveStats::from_samples(baseline)?;

        let mean_change_absolute = current_stats.mean - baseline_stats.mean;
        let mean_change_percent = if baseline_stats.mean != 0.0 {
            (mean_change_absolute / baseline_stats.mean) * 100.0
        } else {
            0.0
        };

        let significance = welch_t_test(current, baseline);
        let regression = analyze_regression(name, current, baseline, threshold_percent);

        Some(Self {
            name: name.to_string(),
            current: current_stats,
            baseline: baseline_stats,
            mean_change_percent,
            mean_change_absolute,
            significance,
            regression,
        })
    }

    /// Check if there's a statistically significant change.
    pub fn is_significant(&self) -> bool {
        self.significance
            .as_ref()
            .map(|s| s.significant_05)
            .unwrap_or(false)
    }

    /// Format the change for display.
    pub fn format_change(&self) -> String {
        let arrow = if self.mean_change_percent > 0.0 {
            "+"
        } else {
            ""
        };
        format!(
            "{}{:.2}% ({}{:.2}ns)",
            arrow, self.mean_change_percent, arrow, self.mean_change_absolute
        )
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_descriptive_stats() {
        let samples: Vec<f64> = vec![10.0, 12.0, 11.0, 13.0, 10.5, 11.5, 12.5, 10.0, 11.0, 12.0];
        let stats = DescriptiveStats::from_samples(&samples).unwrap();

        assert_eq!(stats.count, 10);
        assert!((stats.mean - 11.35).abs() < 0.01);
        assert!((stats.min - 10.0).abs() < 0.001);
        assert!((stats.max - 13.0).abs() < 0.001);
    }

    #[test]
    fn test_welch_t_test() {
        let sample1: Vec<f64> = vec![10.0, 11.0, 12.0, 11.0, 10.5];
        let sample2: Vec<f64> = vec![20.0, 21.0, 22.0, 21.0, 20.5];

        let result = welch_t_test(&sample1, &sample2).unwrap();

        assert!(result.significant_01);
        assert_eq!(result.effect_interpretation, EffectSize::Large);
    }

    #[test]
    fn test_mann_whitney() {
        let sample1: Vec<f64> = vec![10.0, 11.0, 12.0, 11.0, 10.5];
        let sample2: Vec<f64> = vec![20.0, 21.0, 22.0, 21.0, 20.5];

        let result = mann_whitney_u_test(&sample1, &sample2).unwrap();

        assert!(result.significant_05);
    }

    #[test]
    fn test_bootstrap_ci() {
        let samples: Vec<f64> = (0..100).map(|i| 100.0 + (i as f64 * 0.1)).collect();
        let result = bootstrap_ci(&samples, 1000, 0.95).unwrap();

        assert!(result.ci_lower < result.mean);
        assert!(result.ci_upper > result.mean);
    }

    #[test]
    fn test_outlier_detection_iqr() {
        let samples: Vec<f64> = vec![10.0, 11.0, 12.0, 11.0, 10.5, 100.0]; // 100.0 is outlier
        let analysis = detect_outliers_iqr(&samples, 1.5);

        assert_eq!(analysis.count, 1);
        assert!(analysis.values.contains(&100.0));
    }

    #[test]
    fn test_outlier_detection_mad() {
        let samples: Vec<f64> = vec![10.0, 11.0, 12.0, 11.0, 10.5, 100.0]; // 100.0 is outlier
        let analysis = detect_outliers_mad(&samples, 3.5);

        assert_eq!(analysis.count, 1);
        assert!(analysis.values.contains(&100.0));
    }

    #[test]
    fn test_regression_analysis() {
        let current: Vec<f64> = vec![15.0, 16.0, 15.5, 15.2, 15.8];
        let baseline: Vec<f64> = vec![10.0, 11.0, 10.5, 10.2, 10.8];

        let analysis = analyze_regression("test", &current, &baseline, 5.0);

        assert!(analysis.is_regression);
        assert!(analysis.percentage_change > 40.0);
    }

    #[test]
    fn test_distribution_analysis() {
        let samples: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let analysis = analyze_distribution(&samples, 10).unwrap();

        assert!(!analysis.histogram.is_empty());
    }

    #[test]
    fn test_comparison_summary() {
        let current: Vec<f64> = vec![15.0, 16.0, 15.5, 15.2, 15.8];
        let baseline: Vec<f64> = vec![10.0, 11.0, 10.5, 10.2, 10.8];

        let summary = ComparisonSummary::from_samples("test", &current, &baseline, 5.0).unwrap();

        assert!(summary.is_significant());
        assert!(summary.mean_change_percent > 0.0);
    }

    #[test]
    fn test_percentile_calculation() {
        let sorted: Vec<f64> = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];

        assert!((percentile(&sorted, 0.0) - 1.0).abs() < 0.001);
        assert!((percentile(&sorted, 50.0) - 5.5).abs() < 0.001);
        assert!((percentile(&sorted, 100.0) - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_confidence_interval() {
        let samples: Vec<f64> = vec![10.0, 12.0, 11.0, 13.0, 10.5, 11.5, 12.5, 10.0, 11.0, 12.0];
        let stats = DescriptiveStats::from_samples(&samples).unwrap();

        let (lower, upper) = stats.confidence_interval(0.95);
        assert!(lower < stats.mean);
        assert!(upper > stats.mean);
    }
}
