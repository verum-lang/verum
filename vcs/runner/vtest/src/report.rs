//! Report generation for VCS test runner.
//!
//! Generates reports in various formats:
//! - Console (colorized terminal output)
//! - JSON (machine-readable)
//! - HTML (browsable report)
//! - JUnit XML (CI integration)
//! - TAP (Test Anything Protocol)

// Note: Level and Tier are used in the test types but imported from directive
use crate::executor::{TestOutcome, TestResult};
use chrono::{DateTime, Utc};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use std::io::Write;
use std::path::Path;
use thiserror::Error;
use verum_common::{List, Map, Text};

/// Error type for report generation.
#[derive(Debug, Error)]
pub enum ReportError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Template error: {0}")]
    TemplateError(Text),
}

/// Report format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    /// Colorized terminal output
    Console,
    /// Machine-readable JSON
    Json,
    /// Browsable HTML report
    Html,
    /// JUnit XML for CI integration
    Junit,
    /// TAP (Test Anything Protocol) for compatibility
    Tap,
    /// Markdown for documentation and GitHub
    Markdown,
}

impl ReportFormat {
    /// Parse format from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "console" | "text" => Some(Self::Console),
            "json" => Some(Self::Json),
            "html" => Some(Self::Html),
            "junit" | "xml" => Some(Self::Junit),
            "tap" => Some(Self::Tap),
            "markdown" | "md" => Some(Self::Markdown),
            _ => None,
        }
    }

    /// Get the file extension for this format.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Console => "txt",
            Self::Json => "json",
            Self::Html => "html",
            Self::Junit => "xml",
            Self::Tap => "tap",
            Self::Markdown => "md",
        }
    }

    /// Get the MIME type for this format.
    pub fn mime_type(&self) -> &'static str {
        match self {
            Self::Console => "text/plain",
            Self::Json => "application/json",
            Self::Html => "text/html",
            Self::Junit => "application/xml",
            Self::Tap => "text/plain",
            Self::Markdown => "text/markdown",
        }
    }
}

/// Summary statistics for test results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSummary {
    /// Total number of tests
    pub total: usize,
    /// Number of passed tests
    pub passed: usize,
    /// Number of failed tests
    pub failed: usize,
    /// Number of skipped tests
    pub skipped: usize,
    /// Number of errored tests
    pub errored: usize,
    /// Total duration
    pub duration_ms: u64,
    /// Pass rate as percentage
    pub pass_rate: f64,
    /// Summary by level
    pub by_level: Map<Text, LevelSummary>,
    /// Summary by tier
    pub by_tier: Map<Text, TierSummary>,
}

/// Summary for a specific level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub pass_rate: f64,
}

/// Summary for a specific tier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub avg_duration_ms: u64,
}

/// Complete report data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    /// Report timestamp
    pub timestamp: DateTime<Utc>,
    /// Compiler version
    pub compiler_version: Text,
    /// VCS version
    pub vcs_version: Text,
    /// Summary statistics
    pub summary: TestSummary,
    /// Detailed results per test
    pub results: List<TestResultData>,
    /// Failures for easy access
    pub failures: List<FailureData>,
}

/// Serializable test result data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResultData {
    pub name: Text,
    pub path: Text,
    pub test_type: Text,
    pub level: Text,
    pub tags: List<Text>,
    pub outcomes: List<OutcomeData>,
    pub duration_ms: u64,
}

/// Serializable outcome data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomeData {
    pub tier: u8,
    pub status: Text,
    pub reason: Option<Text>,
    pub expected: Option<Text>,
    pub actual: Option<Text>,
    pub duration_ms: u64,
}

/// Failure information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureData {
    pub name: Text,
    pub path: Text,
    pub tier: u8,
    pub reason: Text,
    pub expected: Option<Text>,
    pub actual: Option<Text>,
}

/// Report generator.
pub struct Reporter {
    /// Compiler version string
    compiler_version: Text,
    /// Collected test results
    results: List<TestResult>,
    /// Whether to show colors in console output
    use_colors: bool,
    /// Whether to show verbose output
    verbose: bool,
    /// Whether to show diffs for failures
    show_diff: bool,
    /// Maximum diff context lines
    diff_context_lines: usize,
}

impl Reporter {
    /// Create a new reporter.
    pub fn new(compiler_version: Text) -> Self {
        Self {
            compiler_version,
            results: List::new(),
            use_colors: true,
            verbose: false,
            show_diff: true,
            diff_context_lines: 3,
        }
    }

    /// Set whether to use colors.
    pub fn with_colors(mut self, use_colors: bool) -> Self {
        self.use_colors = use_colors;
        self
    }

    /// Set verbose mode.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Set whether to show diffs for failures.
    pub fn with_diff(mut self, show_diff: bool) -> Self {
        self.show_diff = show_diff;
        self
    }

    /// Set the number of context lines for diffs.
    pub fn with_diff_context(mut self, lines: usize) -> Self {
        self.diff_context_lines = lines;
        self
    }

    /// Add a test result.
    pub fn add_result(&mut self, result: TestResult) {
        self.results.push(result);
    }

    /// Add multiple test results.
    pub fn add_results(&mut self, results: List<TestResult>) {
        self.results.extend(results);
    }

    /// Build the report.
    pub fn build_report(&self) -> Report {
        let mut summary = TestSummary {
            total: 0,
            passed: 0,
            failed: 0,
            skipped: 0,
            errored: 0,
            duration_ms: 0,
            pass_rate: 0.0,
            by_level: Map::new(),
            by_tier: Map::new(),
        };

        let mut results_data = List::new();
        let mut failures = List::new();

        for result in &self.results {
            summary.total += 1;
            summary.duration_ms += result.total_duration.as_millis() as u64;

            let mut test_data = TestResultData {
                name: result.directives.display_name(),
                path: result.directives.source_path.clone(),
                test_type: result.directives.test_type.to_string().into(),
                level: result.directives.level.to_string().into(),
                tags: result.directives.tags.iter().cloned().collect(),
                outcomes: List::new(),
                duration_ms: result.total_duration.as_millis() as u64,
            };

            // Track level stats
            let level_key = result.directives.level.to_string();
            let level_summary = summary
                .by_level
                .entry(level_key.clone().into())
                .or_insert(LevelSummary {
                    total: 0,
                    passed: 0,
                    failed: 0,
                    pass_rate: 0.0,
                });
            level_summary.total += 1;

            let mut test_passed = true;
            let mut test_skipped = true;  // Track if ALL outcomes were skipped
            let mut test_errored = false;

            for outcome in &result.outcomes {
                let (status, reason, expected, actual) = match outcome {
                    TestOutcome::Pass { .. } => {
                        test_skipped = false;  // At least one non-skip outcome
                        ("pass".to_string(), None, None, None)
                    }
                    TestOutcome::Fail {
                        reason,
                        expected,
                        actual,
                        ..
                    } => {
                        test_passed = false;
                        test_skipped = false;
                        (
                            "fail".to_string(),
                            Some(reason.clone()),
                            expected.clone(),
                            actual.clone(),
                        )
                    }
                    TestOutcome::Skip { reason, .. } => {
                        // Don't set test_passed = false for skip, only track if ALL are skipped
                        ("skip".to_string(), Some(reason.clone()), None, None)
                    }
                    TestOutcome::Error { error, .. } => {
                        test_passed = false;
                        test_skipped = false;
                        test_errored = true;
                        ("error".to_string(), Some(error.clone()), None, None)
                    }
                };

                let tier = outcome.tier() as u8;
                let duration_ms = outcome
                    .duration()
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);

                // Track tier stats
                let tier_key = format!("tier{}", tier);
                let tier_summary = summary.by_tier.entry(tier_key.into()).or_insert(TierSummary {
                    total: 0,
                    passed: 0,
                    failed: 0,
                    avg_duration_ms: 0,
                });
                tier_summary.total += 1;
                if outcome.is_pass() {
                    tier_summary.passed += 1;
                } else if outcome.is_fail() {
                    tier_summary.failed += 1;
                }

                test_data.outcomes.push(OutcomeData {
                    tier,
                    status: status.clone().into(),
                    reason: reason.clone(),
                    expected: expected.clone(),
                    actual: actual.clone(),
                    duration_ms,
                });

                // Record failure
                if outcome.is_fail() {
                    failures.push(FailureData {
                        name: result.directives.display_name(),
                        path: result.directives.source_path.clone(),
                        tier,
                        reason: reason.unwrap_or_default(),
                        expected,
                        actual,
                    });
                }
            }

            // Update test-level summary (counting unique tests, not outcomes)
            if test_skipped {
                summary.skipped += 1;
            } else if test_errored {
                summary.errored += 1;
                level_summary.failed += 1;
            } else if test_passed {
                summary.passed += 1;
                level_summary.passed += 1;
            } else {
                summary.failed += 1;
                level_summary.failed += 1;
            }

            results_data.push(test_data);
        }

        // Calculate pass rates
        if summary.total > 0 {
            summary.pass_rate = (summary.passed as f64) / (summary.total as f64);
        }

        for level_summary in summary.by_level.values_mut() {
            if level_summary.total > 0 {
                level_summary.pass_rate =
                    (level_summary.passed as f64) / (level_summary.total as f64);
            }
        }

        Report {
            timestamp: Utc::now(),
            compiler_version: self.compiler_version.clone(),
            vcs_version: env!("CARGO_PKG_VERSION").to_string().into(),
            summary,
            results: results_data,
            failures,
        }
    }

    /// Generate console output.
    pub fn generate_console<W: Write>(&self, writer: &mut W) -> Result<(), ReportError> {
        let report = self.build_report();

        writeln!(writer)?;
        writeln!(
            writer,
            "{}",
            "═══════════════════════════════════════════════════════════".dimmed()
        )?;
        writeln!(writer, "            {}", "VTEST EXECUTION REPORT".bold())?;
        writeln!(
            writer,
            "{}",
            "═══════════════════════════════════════════════════════════".dimmed()
        )?;
        writeln!(writer)?;
        writeln!(writer, "  Verum Compliance Suite v{}", report.vcs_version)?;
        writeln!(
            writer,
            "  Running on: verum-compiler v{}",
            report.compiler_version
        )?;
        writeln!(
            writer,
            "  Date: {}",
            report.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        )?;
        writeln!(writer)?;
        writeln!(
            writer,
            "{}",
            "───────────────────────────────────────────────────────────".dimmed()
        )?;

        // Print results by level
        for (level, level_summary) in &report.summary.by_level {
            writeln!(writer)?;
            writeln!(
                writer,
                "  {} ({}):",
                level.bold(),
                match level.as_str() {
                    "L0" => "Critical",
                    "L1" => "Core",
                    "L2" => "Standard",
                    "L3" => "Extended",
                    "L4" => "Performance",
                    _ => "",
                }
            )?;

            // Find tests for this level
            for result_data in &report.results {
                if result_data.level == *level {
                    let all_skip = result_data.outcomes.iter().all(|o| o.status == "skip");
                    let any_fail = result_data.outcomes.iter().any(|o| o.status == "fail" || o.status == "error");

                    let status_icon = if all_skip {
                        "  SKIP  ".on_yellow().black()
                    } else if any_fail {
                        "  FAIL  ".on_red().white()
                    } else {
                        "  PASS  ".on_green().black()
                    };

                    let duration = format!("[{:>4}ms]", result_data.duration_ms);

                    writeln!(
                        writer,
                        "    {} {} {}",
                        status_icon,
                        result_data.name,
                        duration.dimmed()
                    )?;

                    // Show failure details in verbose mode or if show_diff is enabled
                    if self.verbose || self.show_diff {
                        for outcome in &result_data.outcomes {
                            if outcome.status == "fail" {
                                if let Some(ref reason) = outcome.reason {
                                    writeln!(writer, "      {} {}", "Reason:".red(), reason)?;
                                }

                                // Show diff if both expected and actual are available
                                if let (Some(expected), Some(actual)) =
                                    (&outcome.expected, &outcome.actual)
                                {
                                    if self.show_diff {
                                        writeln!(writer, "      {}:", "Diff".yellow())?;
                                        let diff_config = DiffConfig {
                                            context_lines: self.diff_context_lines,
                                            use_colors: self.use_colors,
                                            ..Default::default()
                                        };
                                        let diff = diff_config.generate(expected, actual);
                                        for line in diff.lines() {
                                            writeln!(writer, "        {}", line)?;
                                        }
                                    } else {
                                        // Just show expected/actual without diff
                                        writeln!(
                                            writer,
                                            "      {} {}",
                                            "Expected:".yellow(),
                                            expected
                                        )?;
                                        writeln!(
                                            writer,
                                            "      {} {}",
                                            "Actual:".yellow(),
                                            actual
                                        )?;
                                    }
                                } else {
                                    // Only one is available
                                    if let Some(ref expected) = outcome.expected {
                                        writeln!(
                                            writer,
                                            "      {} {}",
                                            "Expected:".yellow(),
                                            expected
                                        )?;
                                    }
                                    if let Some(ref actual) = outcome.actual {
                                        writeln!(
                                            writer,
                                            "      {} {}",
                                            "Actual:".yellow(),
                                            actual
                                        )?;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            writeln!(
                writer,
                "    {}: {}/{} ({:.1}%)",
                level,
                level_summary.passed,
                level_summary.total,
                level_summary.pass_rate * 100.0
            )?;
        }

        writeln!(writer)?;
        writeln!(
            writer,
            "{}",
            "═══════════════════════════════════════════════════════════".dimmed()
        )?;
        writeln!(writer)?;
        writeln!(writer, "  {}", "SUMMARY".bold())?;
        writeln!(
            writer,
            "{}",
            "───────────────────────────────────────────────────────────".dimmed()
        )?;
        writeln!(writer, "  Total:     {} tests", report.summary.total)?;
        writeln!(
            writer,
            "  Passed:    {} ({:.1}%)",
            report.summary.passed.to_string().green(),
            report.summary.pass_rate * 100.0
        )?;
        writeln!(
            writer,
            "  Failed:    {} ({:.1}%)",
            if report.summary.failed > 0 {
                report.summary.failed.to_string().red()
            } else {
                report.summary.failed.to_string().normal()
            },
            (report.summary.failed as f64) / (report.summary.total.max(1) as f64) * 100.0
        )?;
        writeln!(
            writer,
            "  Skipped:   {} ({:.1}%)",
            report.summary.skipped,
            (report.summary.skipped as f64) / (report.summary.total.max(1) as f64) * 100.0
        )?;
        writeln!(writer)?;
        writeln!(writer, "  By Level:")?;
        for (level, level_summary) in &report.summary.by_level {
            let status = if level_summary.pass_rate >= 1.0 {
                "OK".green()
            } else if level_summary.pass_rate >= 0.95 {
                "OK".yellow()
            } else {
                "FAIL".red()
            };
            writeln!(
                writer,
                "    {}: {}/{} ({:.1}%) {}",
                level,
                level_summary.passed,
                level_summary.total,
                level_summary.pass_rate * 100.0,
                status
            )?;
        }
        writeln!(writer)?;
        writeln!(writer, "  Duration: {}ms", report.summary.duration_ms)?;
        writeln!(writer)?;
        writeln!(
            writer,
            "{}",
            "═══════════════════════════════════════════════════════════".dimmed()
        )?;

        // Final result
        if report.summary.failed > 0 || report.summary.errored > 0 {
            writeln!(
                writer,
                "  RESULT: {} ({} failures)",
                "FAILED".red().bold(),
                report.summary.failed + report.summary.errored
            )?;
        } else {
            writeln!(writer, "  RESULT: {}", "PASSED".green().bold())?;
        }
        writeln!(
            writer,
            "{}",
            "═══════════════════════════════════════════════════════════".dimmed()
        )?;
        writeln!(writer)?;

        Ok(())
    }

    /// Generate JSON output.
    pub fn generate_json<W: Write>(&self, writer: &mut W) -> Result<(), ReportError> {
        let report = self.build_report();
        let json = serde_json::to_string_pretty(&report)?;
        writeln!(writer, "{}", json)?;
        Ok(())
    }

    /// Generate HTML output.
    pub fn generate_html<W: Write>(&self, writer: &mut W) -> Result<(), ReportError> {
        let report = self.build_report();

        writeln!(
            writer,
            r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>VCS Test Report</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            max-width: 1200px;
            margin: 0 auto;
            padding: 20px;
            background: #f5f5f5;
        }}
        .header {{
            background: #1a1a1a;
            color: white;
            padding: 20px;
            border-radius: 8px;
            margin-bottom: 20px;
        }}
        .summary {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 20px;
            margin-bottom: 20px;
        }}
        .stat-card {{
            background: white;
            padding: 20px;
            border-radius: 8px;
            box-shadow: 0 1px 3px rgba(0,0,0,0.1);
        }}
        .stat-value {{
            font-size: 2em;
            font-weight: bold;
        }}
        .pass {{ color: #22c55e; }}
        .fail {{ color: #ef4444; }}
        .skip {{ color: #f59e0b; }}
        .results {{
            background: white;
            border-radius: 8px;
            padding: 20px;
            box-shadow: 0 1px 3px rgba(0,0,0,0.1);
        }}
        table {{
            width: 100%;
            border-collapse: collapse;
        }}
        th, td {{
            padding: 12px;
            text-align: left;
            border-bottom: 1px solid #e5e5e5;
        }}
        th {{
            background: #f9fafb;
            font-weight: 600;
        }}
        .badge {{
            display: inline-block;
            padding: 4px 8px;
            border-radius: 4px;
            font-size: 0.85em;
        }}
        .badge-pass {{ background: #dcfce7; color: #166534; }}
        .badge-fail {{ background: #fee2e2; color: #991b1b; }}
        .badge-skip {{ background: #fef3c7; color: #92400e; }}
        .level {{ font-weight: 600; }}
    </style>
</head>
<body>
    <div class="header">
        <h1>VCS Test Report</h1>
        <p>Verum Compliance Suite v{vcs_version}</p>
        <p>Compiler: v{compiler_version}</p>
        <p>Generated: {timestamp}</p>
    </div>

    <div class="summary">
        <div class="stat-card">
            <div class="stat-value">{total}</div>
            <div>Total Tests</div>
        </div>
        <div class="stat-card">
            <div class="stat-value pass">{passed}</div>
            <div>Passed</div>
        </div>
        <div class="stat-card">
            <div class="stat-value fail">{failed}</div>
            <div>Failed</div>
        </div>
        <div class="stat-card">
            <div class="stat-value skip">{skipped}</div>
            <div>Skipped</div>
        </div>
        <div class="stat-card">
            <div class="stat-value">{pass_rate:.1}%</div>
            <div>Pass Rate</div>
        </div>
        <div class="stat-card">
            <div class="stat-value">{duration}ms</div>
            <div>Duration</div>
        </div>
    </div>

    <div class="results">
        <h2>Test Results</h2>
        <table>
            <thead>
                <tr>
                    <th>Test</th>
                    <th>Level</th>
                    <th>Type</th>
                    <th>Status</th>
                    <th>Duration</th>
                </tr>
            </thead>
            <tbody>
"#,
            vcs_version = report.vcs_version,
            compiler_version = report.compiler_version,
            timestamp = report.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
            total = report.summary.total,
            passed = report.summary.passed,
            failed = report.summary.failed,
            skipped = report.summary.skipped,
            pass_rate = report.summary.pass_rate * 100.0,
            duration = report.summary.duration_ms,
        )?;

        for result in &report.results {
            let status = if result.outcomes.iter().all(|o| o.status == "pass") {
                "pass"
            } else if result.outcomes.iter().any(|o| o.status == "fail") {
                "fail"
            } else {
                "skip"
            };

            writeln!(
                writer,
                r#"                <tr>
                    <td>{name}</td>
                    <td class="level">{level}</td>
                    <td>{test_type}</td>
                    <td><span class="badge badge-{status}">{status_upper}</span></td>
                    <td>{duration}ms</td>
                </tr>"#,
                name = html_escape(&result.name),
                level = result.level,
                test_type = result.test_type,
                status = status,
                status_upper = status.to_uppercase(),
                duration = result.duration_ms,
            )?;
        }

        writeln!(
            writer,
            r#"            </tbody>
        </table>
    </div>
</body>
</html>"#
        )?;

        Ok(())
    }

    /// Generate JUnit XML output.
    pub fn generate_junit<W: Write>(&self, writer: &mut W) -> Result<(), ReportError> {
        let report = self.build_report();

        writeln!(writer, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
        writeln!(
            writer,
            r#"<testsuites name="VCS" tests="{}" failures="{}" errors="{}" time="{:.3}">"#,
            report.summary.total,
            report.summary.failed,
            report.summary.errored,
            report.summary.duration_ms as f64 / 1000.0
        )?;

        // Group by level
        let mut by_level: Map<Text, List<&TestResultData>> = Map::new();
        for result in &report.results {
            by_level
                .entry(result.level.clone())
                .or_insert_with(List::new)
                .push(result);
        }

        for (level, results) in by_level {
            let level_passed = results
                .iter()
                .filter(|r| {
                    r.outcomes
                        .iter()
                        .all(|o| o.status == "pass" || o.status == "skip")
                })
                .count();
            let level_failed = results.len() - level_passed;
            let level_time: u64 = results.iter().map(|r| r.duration_ms).sum();

            writeln!(
                writer,
                r#"  <testsuite name="{}" tests="{}" failures="{}" time="{:.3}">"#,
                level,
                results.len(),
                level_failed,
                level_time as f64 / 1000.0
            )?;

            for result in results {
                let test_time = result.duration_ms as f64 / 1000.0;

                writeln!(
                    writer,
                    r#"    <testcase name="{}" classname="{}" time="{:.3}">"#,
                    xml_escape(&result.name),
                    xml_escape(&result.path),
                    test_time
                )?;

                for outcome in &result.outcomes {
                    if outcome.status == "fail" {
                        writeln!(
                            writer,
                            r#"      <failure message="{}">"#,
                            xml_escape(outcome.reason.as_deref().unwrap_or(""))
                        )?;
                        if let Some(ref expected) = outcome.expected {
                            writeln!(writer, "Expected: {}", xml_escape(expected))?;
                        }
                        if let Some(ref actual) = outcome.actual {
                            writeln!(writer, "Actual: {}", xml_escape(actual))?;
                        }
                        writeln!(writer, r#"      </failure>"#)?;
                    } else if outcome.status == "error" {
                        writeln!(
                            writer,
                            r#"      <error message="{}" />"#,
                            xml_escape(outcome.reason.as_deref().unwrap_or(""))
                        )?;
                    } else if outcome.status == "skip" {
                        writeln!(
                            writer,
                            r#"      <skipped message="{}" />"#,
                            xml_escape(outcome.reason.as_deref().unwrap_or(""))
                        )?;
                    }
                }

                writeln!(writer, r#"    </testcase>"#)?;
            }

            writeln!(writer, r#"  </testsuite>"#)?;
        }

        writeln!(writer, r#"</testsuites>"#)?;

        Ok(())
    }

    /// Generate TAP (Test Anything Protocol) output.
    ///
    /// TAP is a simple text-based interface between testing modules.
    /// See https://testanything.org/ for specification.
    pub fn generate_tap<W: Write>(&self, writer: &mut W) -> Result<(), ReportError> {
        let report = self.build_report();

        // TAP version header
        writeln!(writer, "TAP version 14")?;

        // Plan line (1..N where N is total tests)
        writeln!(writer, "1..{}", report.summary.total)?;

        let mut test_number = 0;

        for result in &report.results {
            test_number += 1;

            // Determine overall test status
            let all_pass = result
                .outcomes
                .iter()
                .all(|o| o.status == "pass" || o.status == "skip");
            let is_skip = result.outcomes.iter().all(|o| o.status == "skip");

            // TAP test line format: (ok|not ok) N - description [# directive]
            let status = if all_pass { "ok" } else { "not ok" };
            let description = tap_escape(&result.name);

            if is_skip {
                // SKIP directive
                let skip_reason = result
                    .outcomes
                    .first()
                    .and_then(|o| o.reason.as_ref())
                    .map(|r| r.as_str())
                    .unwrap_or("");
                writeln!(
                    writer,
                    "{} {} - {} # SKIP {}",
                    status, test_number, description, skip_reason
                )?;
            } else {
                writeln!(writer, "{} {} - {}", status, test_number, description)?;
            }

            // Add YAML diagnostics for failures
            if !all_pass {
                writeln!(writer, "  ---")?;
                writeln!(writer, "  file: \"{}\"", tap_escape(&result.path))?;
                writeln!(writer, "  level: {}", result.level)?;
                writeln!(writer, "  type: {}", result.test_type)?;
                writeln!(writer, "  duration_ms: {}", result.duration_ms)?;

                // Show failure details per tier
                writeln!(writer, "  outcomes:")?;
                for outcome in &result.outcomes {
                    writeln!(writer, "    - tier: {}", outcome.tier)?;
                    writeln!(writer, "      status: {}", outcome.status)?;
                    if let Some(ref reason) = outcome.reason {
                        writeln!(writer, "      reason: \"{}\"", tap_escape(reason))?;
                    }
                    if let Some(ref expected) = outcome.expected {
                        writeln!(writer, "      expected: \"{}\"", tap_escape(expected))?;
                    }
                    if let Some(ref actual) = outcome.actual {
                        writeln!(writer, "      actual: \"{}\"", tap_escape(actual))?;
                    }
                }

                writeln!(writer, "  ...")?;
            }
        }

        // TAP footer with summary as a comment
        writeln!(writer)?;
        writeln!(writer, "# Tests: {}", report.summary.total)?;
        writeln!(writer, "# Passed: {}", report.summary.passed)?;
        writeln!(writer, "# Failed: {}", report.summary.failed)?;
        writeln!(writer, "# Skipped: {}", report.summary.skipped)?;
        writeln!(writer, "# Duration: {}ms", report.summary.duration_ms)?;

        Ok(())
    }

    /// Generate report to a file.
    pub fn generate_to_file(&self, path: &Path, format: ReportFormat) -> Result<(), ReportError> {
        let mut file = std::fs::File::create(path)?;
        self.generate(&mut file, format)
    }

    /// Generate report to a writer.
    pub fn generate<W: Write>(
        &self,
        writer: &mut W,
        format: ReportFormat,
    ) -> Result<(), ReportError> {
        match format {
            ReportFormat::Console => self.generate_console(writer),
            ReportFormat::Json => self.generate_json(writer),
            ReportFormat::Html => self.generate_html(writer),
            ReportFormat::Junit => self.generate_junit(writer),
            ReportFormat::Tap => self.generate_tap(writer),
            ReportFormat::Markdown => self.generate_markdown(writer),
        }
    }

    /// Generate Markdown output.
    ///
    /// Produces a clean Markdown report suitable for GitHub README files,
    /// documentation, or issue tracking.
    pub fn generate_markdown<W: Write>(&self, writer: &mut W) -> Result<(), ReportError> {
        let report = self.build_report();

        // Title and metadata
        writeln!(writer, "# VCS Test Report")?;
        writeln!(writer)?;
        writeln!(writer, "**Verum Compliance Suite** v{}", report.vcs_version)?;
        writeln!(writer)?;

        // Summary badges (GitHub-style)
        let pass_badge = if report.summary.pass_rate >= 1.0 {
            "![Pass](https://img.shields.io/badge/status-pass-brightgreen)"
        } else if report.summary.pass_rate >= 0.9 {
            "![Partial](https://img.shields.io/badge/status-partial-yellow)"
        } else {
            "![Fail](https://img.shields.io/badge/status-fail-red)"
        };

        writeln!(writer, "{}", pass_badge)?;
        writeln!(writer)?;

        // Metadata table
        writeln!(writer, "| Property | Value |")?;
        writeln!(writer, "|----------|-------|")?;
        writeln!(writer, "| Compiler | v{} |", report.compiler_version)?;
        writeln!(
            writer,
            "| Date | {} |",
            report.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        )?;
        writeln!(writer, "| Duration | {}ms |", report.summary.duration_ms)?;
        writeln!(writer)?;

        // Summary section
        writeln!(writer, "## Summary")?;
        writeln!(writer)?;
        writeln!(writer, "| Metric | Count | Percentage |")?;
        writeln!(writer, "|--------|------:|------------|")?;
        writeln!(writer, "| Total | {} | 100% |", report.summary.total)?;
        writeln!(
            writer,
            "| Passed | {} | {:.1}% |",
            report.summary.passed,
            report.summary.pass_rate * 100.0
        )?;
        writeln!(
            writer,
            "| Failed | {} | {:.1}% |",
            report.summary.failed,
            (report.summary.failed as f64) / (report.summary.total.max(1) as f64) * 100.0
        )?;
        writeln!(
            writer,
            "| Skipped | {} | {:.1}% |",
            report.summary.skipped,
            (report.summary.skipped as f64) / (report.summary.total.max(1) as f64) * 100.0
        )?;
        writeln!(writer)?;

        // Results by level
        writeln!(writer, "## Results by Level")?;
        writeln!(writer)?;
        writeln!(writer, "| Level | Passed | Total | Rate | Status |")?;
        writeln!(writer, "|-------|-------:|------:|-----:|--------|")?;

        for (level, summary) in &report.summary.by_level {
            let status = if summary.pass_rate >= 1.0 {
                ":white_check_mark:"
            } else if summary.pass_rate >= 0.95 {
                ":warning:"
            } else {
                ":x:"
            };

            let level_desc = match level.as_str() {
                "L0" => "L0 (Critical)",
                "L1" => "L1 (Core)",
                "L2" => "L2 (Standard)",
                "L3" => "L3 (Extended)",
                "L4" => "L4 (Performance)",
                _ => level.as_str(),
            };

            writeln!(
                writer,
                "| {} | {} | {} | {:.1}% | {} |",
                level_desc,
                summary.passed,
                summary.total,
                summary.pass_rate * 100.0,
                status
            )?;
        }
        writeln!(writer)?;

        // Results by tier
        if !report.summary.by_tier.is_empty() {
            writeln!(writer, "## Results by Tier")?;
            writeln!(writer)?;
            writeln!(writer, "| Tier | Passed | Total | Avg Duration |")?;
            writeln!(writer, "|------|-------:|------:|-------------:|")?;

            for (tier, summary) in &report.summary.by_tier {
                let tier_desc = match tier.as_str() {
                    "tier0" => "Tier 0 (Interpreter)",
                    "tier1" => "Tier 1 (JIT Base)",
                    "tier2" => "Tier 2 (JIT Opt)",
                    "tier3" => "Tier 3 (AOT)",
                    _ => tier.as_str(),
                };

                let avg_duration = if summary.total > 0 {
                    summary.avg_duration_ms / summary.total as u64
                } else {
                    0
                };

                writeln!(
                    writer,
                    "| {} | {} | {} | {}ms |",
                    tier_desc, summary.passed, summary.total, avg_duration
                )?;
            }
            writeln!(writer)?;
        }

        // Failures section
        if !report.failures.is_empty() {
            writeln!(writer, "## Failures")?;
            writeln!(writer)?;

            for failure in &report.failures {
                writeln!(writer, "### {} (Tier {})", failure.name, failure.tier)?;
                writeln!(writer)?;
                writeln!(writer, "**File:** `{}`", failure.path)?;
                writeln!(writer)?;
                writeln!(writer, "**Reason:** {}", failure.reason)?;
                writeln!(writer)?;

                if let Some(ref expected) = failure.expected {
                    writeln!(writer, "<details>")?;
                    writeln!(writer, "<summary>Expected</summary>")?;
                    writeln!(writer)?;
                    writeln!(writer, "```")?;
                    writeln!(writer, "{}", expected)?;
                    writeln!(writer, "```")?;
                    writeln!(writer, "</details>")?;
                    writeln!(writer)?;
                }

                if let Some(ref actual) = failure.actual {
                    writeln!(writer, "<details>")?;
                    writeln!(writer, "<summary>Actual</summary>")?;
                    writeln!(writer)?;
                    writeln!(writer, "```")?;
                    writeln!(writer, "{}", actual)?;
                    writeln!(writer, "```")?;
                    writeln!(writer, "</details>")?;
                    writeln!(writer)?;
                }

                writeln!(writer, "---")?;
                writeln!(writer)?;
            }
        }

        // Detailed results (collapsible)
        writeln!(writer, "## Detailed Results")?;
        writeln!(writer)?;
        writeln!(writer, "<details>")?;
        writeln!(
            writer,
            "<summary>Click to expand all test results</summary>"
        )?;
        writeln!(writer)?;
        writeln!(writer, "| Test | Level | Type | Status | Duration |")?;
        writeln!(writer, "|------|-------|------|--------|----------|")?;

        for result in &report.results {
            let status = if result.outcomes.iter().all(|o| o.status == "pass") {
                ":white_check_mark:"
            } else if result.outcomes.iter().any(|o| o.status == "fail") {
                ":x:"
            } else {
                ":fast_forward:"
            };

            writeln!(
                writer,
                "| {} | {} | {} | {} | {}ms |",
                result.name, result.level, result.test_type, status, result.duration_ms
            )?;
        }

        writeln!(writer)?;
        writeln!(writer, "</details>")?;
        writeln!(writer)?;

        // Footer
        writeln!(writer, "---")?;
        writeln!(writer)?;
        writeln!(
            writer,
            "*Generated by VCS Test Runner v{} on {}*",
            report.vcs_version,
            report.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        )?;

        Ok(())
    }

    /// Get the overall exit code (0 = all pass, 1 = failures).
    pub fn exit_code(&self) -> i32 {
        let report = self.build_report();
        if report.summary.failed > 0 || report.summary.errored > 0 {
            1
        } else {
            0
        }
    }
}

/// Escape HTML special characters.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Escape XML special characters.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Escape TAP output (escape newlines and special chars for YAML).
fn tap_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Generate a colored diff between expected and actual values.
pub fn generate_diff(expected: &str, actual: &str, use_colors: bool) -> String {
    let diff = TextDiff::from_lines(expected, actual);
    let mut output = String::new();

    for change in diff.iter_all_changes() {
        let (sign, line) = match change.tag() {
            ChangeTag::Delete => ("-", change.value()),
            ChangeTag::Insert => ("+", change.value()),
            ChangeTag::Equal => (" ", change.value()),
        };

        let formatted_line = if use_colors {
            match change.tag() {
                ChangeTag::Delete => format!("{}{}", sign.red(), line.red()),
                ChangeTag::Insert => format!("{}{}", sign.green(), line.green()),
                ChangeTag::Equal => format!("{}{}", sign.dimmed(), line),
            }
        } else {
            format!("{}{}", sign, line)
        };

        output.push_str(&formatted_line);
        if !line.ends_with('\n') {
            output.push('\n');
        }
    }

    output
}

/// Generate a unified diff with context.
pub fn generate_unified_diff(
    expected: &str,
    actual: &str,
    context_lines: usize,
    use_colors: bool,
) -> String {
    let diff = TextDiff::from_lines(expected, actual);
    let mut output = String::new();

    // Header
    let header = format!("--- expected\n+++ actual\n");
    if use_colors {
        output.push_str(&header.bold().to_string());
    } else {
        output.push_str(&header);
    }

    for hunk in diff
        .unified_diff()
        .context_radius(context_lines)
        .iter_hunks()
    {
        // Hunk header - use Display trait of the hunk header
        let hunk_header = format!("{}\n", hunk.header());
        if use_colors {
            output.push_str(&hunk_header.cyan().to_string());
        } else {
            output.push_str(&hunk_header);
        }

        // Hunk content
        for change in hunk.iter_changes() {
            let (sign, line) = match change.tag() {
                ChangeTag::Delete => ("-", change.value()),
                ChangeTag::Insert => ("+", change.value()),
                ChangeTag::Equal => (" ", change.value()),
            };

            let formatted_line = if use_colors {
                match change.tag() {
                    ChangeTag::Delete => format!("{}{}", sign.red(), line.on_red().black()),
                    ChangeTag::Insert => format!("{}{}", sign.green(), line.on_green().black()),
                    ChangeTag::Equal => format!("{}{}", sign.dimmed(), line),
                }
            } else {
                format!("{}{}", sign, line)
            };

            output.push_str(&formatted_line);
            if !line.ends_with('\n') {
                output.push('\n');
            }
        }
    }

    output
}

/// Generate an inline diff highlighting character-level changes.
pub fn generate_inline_diff(expected: &str, actual: &str, use_colors: bool) -> String {
    // For short strings, show character-level diff
    if expected.len() < 200
        && actual.len() < 200
        && !expected.contains('\n')
        && !actual.contains('\n')
    {
        let diff = TextDiff::from_chars(expected, actual);
        let mut expected_out = String::new();
        let mut actual_out = String::new();

        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Delete => {
                    if use_colors {
                        expected_out
                            .push_str(&change.value().to_string().on_red().black().to_string());
                    } else {
                        expected_out.push_str(&format!("[{}]", change.value()));
                    }
                }
                ChangeTag::Insert => {
                    if use_colors {
                        actual_out
                            .push_str(&change.value().to_string().on_green().black().to_string());
                    } else {
                        actual_out.push_str(&format!("[{}]", change.value()));
                    }
                }
                ChangeTag::Equal => {
                    expected_out.push_str(change.value());
                    actual_out.push_str(change.value());
                }
            }
        }

        format!("Expected: {}\nActual:   {}\n", expected_out, actual_out)
    } else {
        // Fall back to line-level diff for longer strings
        generate_unified_diff(expected, actual, 3, use_colors)
    }
}

/// Diff configuration options.
#[derive(Debug, Clone)]
pub struct DiffConfig {
    /// Number of context lines around changes
    pub context_lines: usize,
    /// Use inline diff for short strings
    pub inline_threshold: usize,
    /// Maximum lines to show (truncate if larger)
    pub max_lines: usize,
    /// Use colors
    pub use_colors: bool,
}

impl Default for DiffConfig {
    fn default() -> Self {
        Self {
            context_lines: 3,
            inline_threshold: 200,
            max_lines: 50,
            use_colors: true,
        }
    }
}

impl DiffConfig {
    /// Generate a diff with the configured options.
    pub fn generate(&self, expected: &str, actual: &str) -> String {
        // Choose diff strategy based on input
        let is_short = expected.len() < self.inline_threshold
            && actual.len() < self.inline_threshold
            && !expected.contains('\n')
            && !actual.contains('\n');

        let diff = if is_short {
            generate_inline_diff(expected, actual, self.use_colors)
        } else {
            generate_unified_diff(expected, actual, self.context_lines, self.use_colors)
        };

        // Truncate if too long
        let lines: Vec<&str> = diff.lines().collect();
        if lines.len() > self.max_lines {
            let mut truncated: Vec<String> = lines[..self.max_lines]
                .iter()
                .map(|s| s.to_string())
                .collect();
            truncated.push(format!("... ({} more lines)", lines.len() - self.max_lines));
            truncated.join("\n")
        } else {
            diff
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<test>"), "&lt;test&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
    }

    #[test]
    fn test_xml_escape() {
        assert_eq!(xml_escape("<test>"), "&lt;test&gt;");
        assert_eq!(xml_escape("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(xml_escape("it's"), "it&apos;s");
    }

    #[test]
    fn test_tap_escape() {
        assert_eq!(tap_escape("line1\nline2"), "line1\\nline2");
        assert_eq!(tap_escape("quoted \"value\""), "quoted \\\"value\\\"");
        assert_eq!(tap_escape("tab\there"), "tab\\there");
    }

    #[test]
    fn test_report_format_parse() {
        assert_eq!(
            ReportFormat::from_str("console"),
            Some(ReportFormat::Console)
        );
        assert_eq!(ReportFormat::from_str("text"), Some(ReportFormat::Console));
        assert_eq!(ReportFormat::from_str("JSON"), Some(ReportFormat::Json));
        assert_eq!(ReportFormat::from_str("html"), Some(ReportFormat::Html));
        assert_eq!(ReportFormat::from_str("junit"), Some(ReportFormat::Junit));
        assert_eq!(ReportFormat::from_str("xml"), Some(ReportFormat::Junit));
        assert_eq!(ReportFormat::from_str("tap"), Some(ReportFormat::Tap));
        assert_eq!(
            ReportFormat::from_str("markdown"),
            Some(ReportFormat::Markdown)
        );
        assert_eq!(ReportFormat::from_str("md"), Some(ReportFormat::Markdown));
        assert_eq!(ReportFormat::from_str("unknown"), None);
    }

    #[test]
    fn test_report_format_extension() {
        assert_eq!(ReportFormat::Console.extension(), "txt");
        assert_eq!(ReportFormat::Json.extension(), "json");
        assert_eq!(ReportFormat::Html.extension(), "html");
        assert_eq!(ReportFormat::Junit.extension(), "xml");
        assert_eq!(ReportFormat::Tap.extension(), "tap");
        assert_eq!(ReportFormat::Markdown.extension(), "md");
    }

    #[test]
    fn test_report_format_mime_type() {
        assert_eq!(ReportFormat::Console.mime_type(), "text/plain");
        assert_eq!(ReportFormat::Json.mime_type(), "application/json");
        assert_eq!(ReportFormat::Html.mime_type(), "text/html");
        assert_eq!(ReportFormat::Junit.mime_type(), "application/xml");
        assert_eq!(ReportFormat::Tap.mime_type(), "text/plain");
        assert_eq!(ReportFormat::Markdown.mime_type(), "text/markdown");
    }

    #[test]
    fn test_empty_report() {
        let reporter = Reporter::new("1.0.0".to_string().into());
        let report = reporter.build_report();

        assert_eq!(report.summary.total, 0);
        assert_eq!(report.summary.passed, 0);
        assert_eq!(report.summary.failed, 0);
    }

    #[test]
    fn test_tap_output_format() {
        let reporter = Reporter::new("1.0.0".to_string().into());
        let mut output = Vec::new();
        reporter.generate_tap(&mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();

        // TAP output should start with version and plan
        assert!(output_str.starts_with("TAP version 14\n1..0"));
    }

    #[test]
    fn test_generate_diff_simple() {
        let expected = "hello\nworld\n";
        let actual = "hello\nverum\n";

        let diff = generate_diff(expected, actual, false);
        assert!(diff.contains("-world"));
        assert!(diff.contains("+verum"));
        assert!(diff.contains(" hello"));
    }

    #[test]
    fn test_generate_unified_diff() {
        let expected = "line1\nline2\nline3\nline4\nline5\n";
        let actual = "line1\nchanged\nline3\nline4\nline5\n";

        let diff = generate_unified_diff(expected, actual, 1, false);
        assert!(diff.contains("--- expected"));
        assert!(diff.contains("+++ actual"));
        assert!(diff.contains("-line2"));
        assert!(diff.contains("+changed"));
    }

    #[test]
    fn test_generate_inline_diff() {
        let expected = "hello";
        let actual = "hallo";

        let diff = generate_inline_diff(expected, actual, false);
        assert!(diff.contains("Expected:"));
        assert!(diff.contains("Actual:"));
    }

    #[test]
    fn test_diff_config() {
        let config = DiffConfig {
            context_lines: 2,
            inline_threshold: 100,
            max_lines: 10,
            use_colors: false,
        };

        let expected = "short";
        let actual = "shot";
        let diff = config.generate(expected, actual);

        // Should use inline diff for short strings
        assert!(diff.contains("Expected:"));
    }

    #[test]
    fn test_diff_config_long_strings() {
        let config = DiffConfig {
            context_lines: 2,
            inline_threshold: 100,
            max_lines: 10,
            use_colors: false,
        };

        let expected = "line1\nline2\nline3\nline4\nline5\n";
        let actual = "line1\nchanged\nline3\nline4\nline5\n";
        let diff = config.generate(expected, actual);

        // Should use unified diff for multi-line strings
        assert!(diff.contains("---"));
        assert!(diff.contains("+++"));
    }
}
