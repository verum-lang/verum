//! CI/CD integration for VCS benchmarks.
//!
//! This module provides functionality for integrating benchmark results
//! into continuous integration pipelines, including:
//! - Pass/fail thresholds
//! - GitHub Actions annotations
//! - JUnit XML output
//! - Exit code management

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::compare::{ComparisonAssessment, ComparisonResult, RegressionResult};
use crate::metrics::{BenchmarkCategory, PerformanceTargets};
use crate::report::BenchmarkReport;

// ============================================================================
// CI Configuration
// ============================================================================

/// Configuration for CI integration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiConfig {
    /// Fail the build if any benchmark exceeds its threshold.
    pub fail_on_threshold: bool,
    /// Fail the build if any regression is detected.
    pub fail_on_regression: bool,
    /// Regression threshold as a percentage.
    pub regression_threshold_percent: f64,
    /// Categories that must pass.
    pub required_categories: Vec<BenchmarkCategory>,
    /// Specific benchmarks that must pass.
    pub required_benchmarks: Vec<String>,
    /// Performance targets.
    pub targets: PerformanceTargets,
    /// Output format for CI.
    pub output_format: CiOutputFormat,
    /// Whether to output GitHub Actions annotations.
    pub github_annotations: bool,
    /// Path to baseline file for comparison.
    pub baseline_path: Option<String>,
}

impl Default for CiConfig {
    fn default() -> Self {
        Self {
            fail_on_threshold: true,
            fail_on_regression: true,
            regression_threshold_percent: 5.0,
            required_categories: vec![BenchmarkCategory::Micro],
            required_benchmarks: vec![],
            targets: PerformanceTargets::default(),
            output_format: CiOutputFormat::Console,
            github_annotations: false,
            baseline_path: None,
        }
    }
}

/// Output format for CI results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CiOutputFormat {
    /// Plain console output.
    Console,
    /// GitHub Actions log commands.
    GitHub,
    /// JUnit XML format.
    JUnit,
    /// JSON format.
    Json,
}

// ============================================================================
// CI Result
// ============================================================================

/// Result of CI benchmark validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiResult {
    /// Overall pass/fail status.
    pub passed: bool,
    /// Exit code to use.
    pub exit_code: i32,
    /// Summary of results.
    pub summary: CiSummary,
    /// Individual benchmark statuses.
    pub benchmarks: Vec<BenchmarkStatus>,
    /// Detected regressions.
    pub regressions: Vec<RegressionResult>,
    /// Baseline comparisons.
    pub comparisons: Vec<ComparisonResult>,
    /// Messages/annotations.
    pub messages: Vec<CiMessage>,
}

/// Summary of CI benchmark results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiSummary {
    /// Total number of benchmarks run.
    pub total: usize,
    /// Number of benchmarks that passed.
    pub passed: usize,
    /// Number of benchmarks that failed.
    pub failed: usize,
    /// Number of benchmarks that were skipped.
    pub skipped: usize,
    /// Number of regressions detected.
    pub regressions: usize,
    /// Total execution time in milliseconds.
    pub duration_ms: f64,
}

/// Status of an individual benchmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkStatus {
    /// Benchmark name.
    pub name: String,
    /// Benchmark category.
    pub category: BenchmarkCategory,
    /// Pass/fail status.
    pub status: Status,
    /// Measured mean time in nanoseconds.
    pub mean_ns: f64,
    /// Threshold in nanoseconds (if any).
    pub threshold_ns: Option<f64>,
    /// Reason for failure (if failed).
    pub failure_reason: Option<String>,
}

/// Status of a benchmark or check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Passed,
    Failed,
    Skipped,
    Warning,
}

/// A message or annotation for CI output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiMessage {
    /// Message level.
    pub level: MessageLevel,
    /// Message text.
    pub text: String,
    /// Associated benchmark (if any).
    pub benchmark: Option<String>,
    /// File path (for annotations).
    pub file: Option<String>,
    /// Line number (for annotations).
    pub line: Option<usize>,
}

/// Level of a CI message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageLevel {
    Debug,
    Info,
    Warning,
    Error,
}

// ============================================================================
// CI Runner
// ============================================================================

/// Run CI validation on benchmark results.
pub fn validate_ci(report: &BenchmarkReport, config: &CiConfig) -> CiResult {
    let mut benchmarks = Vec::new();
    let mut messages = Vec::new();
    let mut failed_count = 0;
    let mut passed_count = 0;

    // Check each benchmark
    for result in &report.results {
        let status = if !result.passed {
            failed_count += 1;
            messages.push(CiMessage {
                level: MessageLevel::Error,
                text: format!(
                    "Benchmark '{}' exceeded threshold: {:.2}ns > {:.2}ns",
                    result.name,
                    result.statistics.mean_ns,
                    result.threshold_ns.unwrap_or(0.0)
                ),
                benchmark: Some(result.name.clone()),
                file: None,
                line: None,
            });
            Status::Failed
        } else {
            passed_count += 1;
            Status::Passed
        };

        let failure_reason = if status == Status::Failed {
            Some(format!(
                "Mean {:.2}ns exceeded threshold {:.2}ns",
                result.statistics.mean_ns,
                result.threshold_ns.unwrap_or(0.0)
            ))
        } else {
            None
        };

        benchmarks.push(BenchmarkStatus {
            name: result.name.clone(),
            category: result.category,
            status,
            mean_ns: result.statistics.mean_ns,
            threshold_ns: result.threshold_ns,
            failure_reason,
        });
    }

    // Check for regressions
    let regression_count = report
        .regressions
        .iter()
        .filter(|r| r.is_regression)
        .count();

    if regression_count > 0 {
        for regression in &report.regressions {
            if regression.is_regression {
                messages.push(CiMessage {
                    level: MessageLevel::Error,
                    text: format!(
                        "Regression detected in '{}': {:.2}ns -> {:.2}ns ({:+.1}%)",
                        regression.name,
                        regression.baseline_mean_ns,
                        regression.current_mean_ns,
                        regression.percentage_change
                    ),
                    benchmark: Some(regression.name.clone()),
                    file: None,
                    line: None,
                });
            }
        }
    }

    // Check baseline comparisons
    for comparison in &report.comparisons {
        if comparison.assessment == ComparisonAssessment::TooSlow {
            messages.push(CiMessage {
                level: MessageLevel::Warning,
                text: format!(
                    "Benchmark '{}' is too slow vs {}: {:.2}x ({:+.1}%)",
                    comparison.name,
                    comparison.baseline.language,
                    comparison.ratio,
                    comparison.percentage_diff
                ),
                benchmark: Some(comparison.name.clone()),
                file: None,
                line: None,
            });
        }
    }

    // Check required benchmarks
    for required in &config.required_benchmarks {
        if !report.results.iter().any(|r| r.name == *required) {
            messages.push(CiMessage {
                level: MessageLevel::Error,
                text: format!("Required benchmark '{}' was not run", required),
                benchmark: Some(required.clone()),
                file: None,
                line: None,
            });
            failed_count += 1;
        }
    }

    // Determine overall pass/fail
    let threshold_failed = config.fail_on_threshold && failed_count > 0;
    let regression_failed = config.fail_on_regression && regression_count > 0;
    let passed = !threshold_failed && !regression_failed;

    let exit_code = if passed {
        0
    } else if regression_failed {
        2 // Regression detected
    } else {
        1 // Threshold exceeded
    };

    // Calculate total duration
    let duration_ms: f64 = report
        .results
        .iter()
        .map(|r| r.statistics.total_duration.as_secs_f64() * 1000.0)
        .sum();

    CiResult {
        passed,
        exit_code,
        summary: CiSummary {
            total: report.results.len(),
            passed: passed_count,
            failed: failed_count,
            skipped: 0,
            regressions: regression_count,
            duration_ms,
        },
        benchmarks,
        regressions: report.regressions.clone(),
        comparisons: report.comparisons.clone(),
        messages,
    }
}

// ============================================================================
// Output Formatters
// ============================================================================

/// Format CI result for output.
pub fn format_ci_result(result: &CiResult, format: CiOutputFormat) -> String {
    match format {
        CiOutputFormat::Console => format_console(result),
        CiOutputFormat::GitHub => format_github(result),
        CiOutputFormat::JUnit => format_junit(result),
        CiOutputFormat::Json => format_json(result),
    }
}

/// Format as plain console output.
fn format_console(result: &CiResult) -> String {
    let mut output = String::new();

    output.push_str("=== VBench CI Results ===\n\n");

    // Summary
    output.push_str(&format!(
        "Total: {}  Passed: {}  Failed: {}  Regressions: {}\n",
        result.summary.total,
        result.summary.passed,
        result.summary.failed,
        result.summary.regressions
    ));
    output.push_str(&format!(
        "Duration: {:.2}ms\n\n",
        result.summary.duration_ms
    ));

    // Failed benchmarks
    let failed: Vec<_> = result
        .benchmarks
        .iter()
        .filter(|b| b.status == Status::Failed)
        .collect();

    if !failed.is_empty() {
        output.push_str("Failed Benchmarks:\n");
        for bench in failed {
            output.push_str(&format!(
                "  [FAIL] {} - {}\n",
                bench.name,
                bench.failure_reason.as_deref().unwrap_or("unknown")
            ));
        }
        output.push('\n');
    }

    // Regressions
    if !result.regressions.is_empty() {
        output.push_str("Regressions:\n");
        for regression in &result.regressions {
            if regression.is_regression {
                output.push_str(&format!(
                    "  [REGRESSION] {} - {:+.1}%\n",
                    regression.name, regression.percentage_change
                ));
            }
        }
        output.push('\n');
    }

    // Overall status
    if result.passed {
        output.push_str("Status: PASSED\n");
    } else {
        output.push_str(&format!(
            "Status: FAILED (exit code {})\n",
            result.exit_code
        ));
    }

    output
}

/// Format as GitHub Actions log commands.
fn format_github(result: &CiResult) -> String {
    let mut output = String::new();

    // Group: Summary
    output.push_str("::group::VBench Summary\n");
    output.push_str(&format!(
        "Total: {} | Passed: {} | Failed: {} | Regressions: {}\n",
        result.summary.total,
        result.summary.passed,
        result.summary.failed,
        result.summary.regressions
    ));
    output.push_str("::endgroup::\n\n");

    // Output annotations for each message
    for message in &result.messages {
        let level = match message.level {
            MessageLevel::Debug => "debug",
            MessageLevel::Info => "notice",
            MessageLevel::Warning => "warning",
            MessageLevel::Error => "error",
        };

        let location = match (&message.file, message.line) {
            (Some(file), Some(line)) => format!(" file={},line={}", file, line),
            (Some(file), None) => format!(" file={}", file),
            _ => String::new(),
        };

        output.push_str(&format!(
            "::{}{} title={}::{}\n",
            level,
            location,
            message.benchmark.as_deref().unwrap_or("vbench"),
            message.text
        ));
    }

    // Set output variables
    output.push_str(&format!("::set-output name=passed::{}\n", result.passed));
    output.push_str(&format!(
        "::set-output name=total::{}\n",
        result.summary.total
    ));
    output.push_str(&format!(
        "::set-output name=failed::{}\n",
        result.summary.failed
    ));
    output.push_str(&format!(
        "::set-output name=regressions::{}\n",
        result.summary.regressions
    ));

    output
}

/// Format as JUnit XML.
fn format_junit(result: &CiResult) -> String {
    let mut xml = String::new();

    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str(&format!(
        "<testsuite name=\"vbench\" tests=\"{}\" failures=\"{}\" errors=\"0\" skipped=\"{}\" time=\"{:.3}\">\n",
        result.summary.total,
        result.summary.failed,
        result.summary.skipped,
        result.summary.duration_ms / 1000.0
    ));

    for bench in &result.benchmarks {
        let time_seconds = bench.mean_ns / 1_000_000_000.0;

        xml.push_str(&format!(
            "  <testcase name=\"{}\" classname=\"vbench.{}\" time=\"{:.6}\"",
            escape_xml(&bench.name),
            bench.category,
            time_seconds
        ));

        match bench.status {
            Status::Passed => {
                xml.push_str(" />\n");
            }
            Status::Failed => {
                xml.push_str(">\n");
                xml.push_str(&format!(
                    "    <failure message=\"{}\" type=\"ThresholdExceeded\">{}</failure>\n",
                    escape_xml(
                        bench
                            .failure_reason
                            .as_deref()
                            .unwrap_or("Threshold exceeded")
                    ),
                    escape_xml(&format!(
                        "Mean: {:.2}ns, Threshold: {:.2}ns",
                        bench.mean_ns,
                        bench.threshold_ns.unwrap_or(0.0)
                    ))
                ));
                xml.push_str("  </testcase>\n");
            }
            Status::Skipped => {
                xml.push_str(">\n");
                xml.push_str("    <skipped />\n");
                xml.push_str("  </testcase>\n");
            }
            Status::Warning => {
                xml.push_str(" />\n");
            }
        }
    }

    // Add regressions as additional test cases
    for regression in &result.regressions {
        if regression.is_regression {
            xml.push_str(&format!(
                "  <testcase name=\"regression:{}\" classname=\"vbench.regression\" time=\"0\">\n",
                escape_xml(&regression.name)
            ));
            xml.push_str(&format!(
                "    <failure message=\"Regression detected\" type=\"Regression\">{}</failure>\n",
                escape_xml(&format!(
                    "Baseline: {:.2}ns, Current: {:.2}ns, Change: {:+.1}%",
                    regression.baseline_mean_ns,
                    regression.current_mean_ns,
                    regression.percentage_change
                ))
            ));
            xml.push_str("  </testcase>\n");
        }
    }

    xml.push_str("</testsuite>\n");

    xml
}

/// Format as JSON.
fn format_json(result: &CiResult) -> String {
    serde_json::to_string_pretty(result).unwrap_or_else(|_| "{}".to_string())
}

/// Escape special XML characters.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// ============================================================================
// CI Helper Functions
// ============================================================================

/// Load CI configuration from a file.
pub fn load_ci_config(path: &Path) -> Result<CiConfig> {
    let content = fs::read_to_string(path)
        .context(format!("Failed to read CI config: {}", path.display()))?;

    let config: CiConfig = toml::from_str(&content).context("Failed to parse CI config")?;

    Ok(config)
}

/// Save CI configuration to a file.
pub fn save_ci_config(config: &CiConfig, path: &Path) -> Result<()> {
    let content = toml::to_string_pretty(config).context("Failed to serialize CI config")?;

    fs::write(path, content).context(format!("Failed to write CI config: {}", path.display()))?;

    Ok(())
}

/// Generate a default CI configuration file.
pub fn generate_default_ci_config() -> String {
    let config = CiConfig::default();
    toml::to_string_pretty(&config).unwrap_or_default()
}

/// Check if running in a CI environment.
pub fn is_ci_environment() -> bool {
    std::env::var("CI").is_ok()
        || std::env::var("GITHUB_ACTIONS").is_ok()
        || std::env::var("GITLAB_CI").is_ok()
        || std::env::var("CIRCLECI").is_ok()
        || std::env::var("TRAVIS").is_ok()
        || std::env::var("JENKINS_URL").is_ok()
}

/// Get the current CI provider.
pub fn detect_ci_provider() -> Option<CiProvider> {
    if std::env::var("GITHUB_ACTIONS").is_ok() {
        Some(CiProvider::GitHubActions)
    } else if std::env::var("GITLAB_CI").is_ok() {
        Some(CiProvider::GitLab)
    } else if std::env::var("CIRCLECI").is_ok() {
        Some(CiProvider::CircleCI)
    } else if std::env::var("TRAVIS").is_ok() {
        Some(CiProvider::Travis)
    } else if std::env::var("JENKINS_URL").is_ok() {
        Some(CiProvider::Jenkins)
    } else if std::env::var("CI").is_ok() {
        Some(CiProvider::Unknown)
    } else {
        None
    }
}

/// CI provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiProvider {
    GitHubActions,
    GitLab,
    CircleCI,
    Travis,
    Jenkins,
    Unknown,
}

// ============================================================================
// GitHub Actions Integration
// ============================================================================

/// Generate a GitHub Actions workflow step summary.
pub fn generate_github_summary(result: &CiResult) -> String {
    let mut summary = String::new();

    summary.push_str("## VBench Benchmark Results\n\n");

    // Status badge
    if result.passed {
        summary.push_str("![Status](https://img.shields.io/badge/status-passed-success)\n\n");
    } else {
        summary.push_str("![Status](https://img.shields.io/badge/status-failed-critical)\n\n");
    }

    // Summary table
    summary.push_str("| Metric | Value |\n");
    summary.push_str("|--------|-------|\n");
    summary.push_str(&format!("| Total | {} |\n", result.summary.total));
    summary.push_str(&format!("| Passed | {} |\n", result.summary.passed));
    summary.push_str(&format!("| Failed | {} |\n", result.summary.failed));
    summary.push_str(&format!(
        "| Regressions | {} |\n",
        result.summary.regressions
    ));
    summary.push_str(&format!(
        "| Duration | {:.2}ms |\n\n",
        result.summary.duration_ms
    ));

    // Failed benchmarks table
    let failed: Vec<_> = result
        .benchmarks
        .iter()
        .filter(|b| b.status == Status::Failed)
        .collect();

    if !failed.is_empty() {
        summary.push_str("### Failed Benchmarks\n\n");
        summary.push_str("| Benchmark | Mean | Threshold | Reason |\n");
        summary.push_str("|-----------|------|-----------|--------|\n");

        for bench in failed {
            summary.push_str(&format!(
                "| {} | {:.2}ns | {:.2}ns | {} |\n",
                bench.name,
                bench.mean_ns,
                bench.threshold_ns.unwrap_or(0.0),
                bench.failure_reason.as_deref().unwrap_or("-")
            ));
        }
        summary.push('\n');
    }

    // Regressions table
    let regressions: Vec<_> = result
        .regressions
        .iter()
        .filter(|r| r.is_regression)
        .collect();

    if !regressions.is_empty() {
        summary.push_str("### Regressions Detected\n\n");
        summary.push_str("| Benchmark | Baseline | Current | Change |\n");
        summary.push_str("|-----------|----------|---------|--------|\n");

        for regression in regressions {
            summary.push_str(&format!(
                "| {} | {:.2}ns | {:.2}ns | {:+.1}% |\n",
                regression.name,
                regression.baseline_mean_ns,
                regression.current_mean_ns,
                regression.percentage_change
            ));
        }
    }

    summary
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{BenchmarkResult, Statistics};
    use std::time::Duration;

    fn create_test_report() -> BenchmarkReport {
        use crate::report::ReportMetadata;

        let results = vec![
            BenchmarkResult::new(
                "passing_bench".to_string(),
                BenchmarkCategory::Micro,
                Statistics {
                    count: 100,
                    min_ns: 10.0,
                    max_ns: 20.0,
                    mean_ns: 14.0, // Under threshold
                    median_ns: 14.0,
                    std_dev_ns: 2.0,
                    cv: 0.14,
                    p5_ns: 11.0,
                    p25_ns: 12.0,
                    p75_ns: 16.0,
                    p95_ns: 18.0,
                    p99_ns: 19.0,
                    iqr_ns: 4.0,
                    total_duration: Duration::from_nanos(1400),
                },
                Some(15.0),
            ),
            BenchmarkResult::new(
                "failing_bench".to_string(),
                BenchmarkCategory::Micro,
                Statistics {
                    count: 100,
                    min_ns: 15.0,
                    max_ns: 25.0,
                    mean_ns: 20.0, // Over threshold
                    median_ns: 20.0,
                    std_dev_ns: 2.0,
                    cv: 0.1,
                    p5_ns: 16.0,
                    p25_ns: 18.0,
                    p75_ns: 22.0,
                    p95_ns: 24.0,
                    p99_ns: 24.5,
                    iqr_ns: 4.0,
                    total_duration: Duration::from_nanos(2000),
                },
                Some(15.0),
            ),
        ];

        let metadata = ReportMetadata::new("Test Report", "1.0.0");
        BenchmarkReport::new(metadata, results, vec![], vec![])
    }

    #[test]
    fn test_ci_validation() {
        let report = create_test_report();
        let config = CiConfig::default();
        let result = validate_ci(&report, &config);

        assert!(!result.passed);
        assert_eq!(result.summary.total, 2);
        assert_eq!(result.summary.passed, 1);
        assert_eq!(result.summary.failed, 1);
    }

    #[test]
    fn test_console_format() {
        let report = create_test_report();
        let config = CiConfig::default();
        let result = validate_ci(&report, &config);

        let output = format_ci_result(&result, CiOutputFormat::Console);
        assert!(output.contains("VBench CI Results"));
        assert!(output.contains("FAILED"));
    }

    #[test]
    fn test_github_format() {
        let report = create_test_report();
        let config = CiConfig::default();
        let result = validate_ci(&report, &config);

        let output = format_ci_result(&result, CiOutputFormat::GitHub);
        assert!(output.contains("::group::"));
        assert!(output.contains("::error"));
    }

    #[test]
    fn test_junit_format() {
        let report = create_test_report();
        let config = CiConfig::default();
        let result = validate_ci(&report, &config);

        let output = format_ci_result(&result, CiOutputFormat::JUnit);
        assert!(output.contains("<?xml"));
        assert!(output.contains("<testsuite"));
        assert!(output.contains("<failure"));
    }

    #[test]
    fn test_json_format() {
        let report = create_test_report();
        let config = CiConfig::default();
        let result = validate_ci(&report, &config);

        let output = format_ci_result(&result, CiOutputFormat::Json);
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert!(parsed["passed"].as_bool() == Some(false));
    }

    #[test]
    fn test_github_summary() {
        let report = create_test_report();
        let config = CiConfig::default();
        let result = validate_ci(&report, &config);

        let summary = generate_github_summary(&result);
        assert!(summary.contains("## VBench Benchmark Results"));
        assert!(summary.contains("| Total |"));
    }

    #[test]
    fn test_escape_xml() {
        assert_eq!(escape_xml("a < b"), "a &lt; b");
        assert_eq!(escape_xml("a & b"), "a &amp; b");
        assert_eq!(escape_xml("a > b"), "a &gt; b");
    }
}
