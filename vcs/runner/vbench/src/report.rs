//! Report generation for VCS benchmarks.
//!
//! This module provides functionality for generating benchmark reports in
//! various formats: console, JSON, HTML, and CSV.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};

use crate::compare::{ComparisonAssessment, ComparisonResult, RegressionResult};
use crate::metrics::BenchmarkResult;

// ============================================================================
// Report Types
// ============================================================================

/// Complete benchmark report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReport {
    /// Report metadata.
    pub metadata: ReportMetadata,
    /// Summary statistics.
    pub summary: ReportSummary,
    /// Individual benchmark results.
    pub results: Vec<BenchmarkResult>,
    /// Baseline comparisons (if any).
    pub comparisons: Vec<ComparisonResult>,
    /// Regression analysis (if any).
    pub regressions: Vec<RegressionResult>,
    /// Results by category.
    pub by_category: HashMap<String, Vec<BenchmarkResult>>,
}

/// Report metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportMetadata {
    /// Report title.
    pub title: String,
    /// Report timestamp.
    #[serde(with = "chrono::serde::ts_seconds")]
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Verum version.
    pub verum_version: String,
    /// Platform information.
    pub platform: String,
    /// CPU information.
    pub cpu_info: Option<String>,
    /// Memory information.
    pub memory_info: Option<String>,
    /// Custom tags.
    pub tags: HashMap<String, String>,
}

impl ReportMetadata {
    /// Create default metadata.
    pub fn new(title: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            timestamp: chrono::Utc::now(),
            verum_version: version.into(),
            platform: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            cpu_info: get_cpu_info(),
            memory_info: get_memory_info(),
            tags: HashMap::new(),
        }
    }

    /// Add a custom tag.
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }
}

/// Report summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSummary {
    /// Total number of benchmarks.
    pub total: usize,
    /// Number of passed benchmarks.
    pub passed: usize,
    /// Number of failed benchmarks.
    pub failed: usize,
    /// Number of regressions detected.
    pub regressions: usize,
    /// Pass rate (0.0-1.0).
    pub pass_rate: f64,
    /// Total execution time.
    pub total_duration_ms: f64,
    /// Average time per benchmark in nanoseconds.
    pub avg_time_ns: f64,
    /// Fastest benchmark.
    pub fastest: Option<(String, f64)>,
    /// Slowest benchmark.
    pub slowest: Option<(String, f64)>,
}

impl ReportSummary {
    /// Calculate summary from results.
    pub fn from_results(results: &[BenchmarkResult], regressions: &[RegressionResult]) -> Self {
        let total = results.len();
        let passed = results.iter().filter(|r| r.passed).count();
        let failed = total - passed;
        let regression_count = regressions.iter().filter(|r| r.is_regression).count();

        let pass_rate = if total > 0 {
            passed as f64 / total as f64
        } else {
            0.0
        };

        let total_duration_ms: f64 = results
            .iter()
            .map(|r| r.statistics.total_duration.as_secs_f64() * 1000.0)
            .sum();

        let avg_time_ns = if total > 0 {
            results.iter().map(|r| r.statistics.mean_ns).sum::<f64>() / total as f64
        } else {
            0.0
        };

        let fastest = results
            .iter()
            .min_by(|a, b| {
                a.statistics
                    .mean_ns
                    .partial_cmp(&b.statistics.mean_ns)
                    .unwrap()
            })
            .map(|r| (r.name.clone(), r.statistics.mean_ns));

        let slowest = results
            .iter()
            .max_by(|a, b| {
                a.statistics
                    .mean_ns
                    .partial_cmp(&b.statistics.mean_ns)
                    .unwrap()
            })
            .map(|r| (r.name.clone(), r.statistics.mean_ns));

        Self {
            total,
            passed,
            failed,
            regressions: regression_count,
            pass_rate,
            total_duration_ms,
            avg_time_ns,
            fastest,
            slowest,
        }
    }
}

impl BenchmarkReport {
    /// Create a new report from results.
    pub fn new(
        metadata: ReportMetadata,
        results: Vec<BenchmarkResult>,
        comparisons: Vec<ComparisonResult>,
        regressions: Vec<RegressionResult>,
    ) -> Self {
        let summary = ReportSummary::from_results(&results, &regressions);

        // Group by category
        let mut by_category: HashMap<String, Vec<BenchmarkResult>> = HashMap::new();
        for result in &results {
            by_category
                .entry(result.category.to_string())
                .or_insert_with(Vec::new)
                .push(result.clone());
        }

        Self {
            metadata,
            summary,
            results,
            comparisons,
            regressions,
            by_category,
        }
    }
}

// ============================================================================
// Report Formatting
// ============================================================================

/// Output format for reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    Console,
    Json,
    Html,
    Csv,
    Markdown,
}

impl std::str::FromStr for ReportFormat {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "console" | "text" => Ok(Self::Console),
            "json" => Ok(Self::Json),
            "html" => Ok(Self::Html),
            "csv" => Ok(Self::Csv),
            "markdown" | "md" => Ok(Self::Markdown),
            _ => Err(anyhow::anyhow!("Unknown format: {}", s)),
        }
    }
}

/// Generate a report in the specified format.
pub fn generate_report(report: &BenchmarkReport, format: ReportFormat) -> Result<String> {
    match format {
        ReportFormat::Console => generate_console_report(report),
        ReportFormat::Json => generate_json_report(report),
        ReportFormat::Html => generate_html_report(report),
        ReportFormat::Csv => generate_csv_report(report),
        ReportFormat::Markdown => generate_markdown_report(report),
    }
}

/// Write a report to a file.
pub fn write_report(report: &BenchmarkReport, format: ReportFormat, path: &Path) -> Result<()> {
    let content = generate_report(report, format)?;
    std::fs::write(path, content).context("Failed to write report")?;
    Ok(())
}

// ============================================================================
// Console Report
// ============================================================================

fn generate_console_report(report: &BenchmarkReport) -> Result<String> {
    let mut output = String::new();

    // Header
    output.push_str(&"=".repeat(70));
    output.push('\n');
    output.push_str(&format!("{}\n", report.metadata.title.bold().cyan()));
    output.push_str(&"=".repeat(70));
    output.push('\n');
    output.push('\n');

    // Metadata
    output.push_str(&format!(
        "Verum Version: {}\n",
        report.metadata.verum_version
    ));
    output.push_str(&format!("Platform: {}\n", report.metadata.platform));
    if let Some(ref cpu) = report.metadata.cpu_info {
        output.push_str(&format!("CPU: {}\n", cpu));
    }
    output.push_str(&format!(
        "Timestamp: {}\n",
        report.metadata.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
    ));
    output.push('\n');

    // Summary
    output.push_str(&"-".repeat(70));
    output.push('\n');
    output.push_str(&format!("{}\n", "SUMMARY".bold()));
    output.push_str(&"-".repeat(70));
    output.push('\n');

    let _pass_color = if report.summary.pass_rate >= 1.0 {
        "green"
    } else if report.summary.pass_rate >= 0.95 {
        "yellow"
    } else {
        "red"
    };

    output.push_str(&format!(
        "Total: {}  Passed: {}  Failed: {}  Regressions: {}\n",
        report.summary.total,
        format!("{}", report.summary.passed).green(),
        if report.summary.failed > 0 {
            format!("{}", report.summary.failed).red().to_string()
        } else {
            format!("{}", report.summary.failed)
        },
        if report.summary.regressions > 0 {
            format!("{}", report.summary.regressions).red().to_string()
        } else {
            format!("{}", report.summary.regressions)
        },
    ));

    output.push_str(&format!(
        "Pass Rate: {:.1}%\n",
        report.summary.pass_rate * 100.0
    ));
    output.push_str(&format!(
        "Total Duration: {:.2}ms\n",
        report.summary.total_duration_ms
    ));
    output.push('\n');

    // Results by category
    for (category, results) in &report.by_category {
        output.push_str(&"-".repeat(70));
        output.push('\n');
        output.push_str(&format!(
            "{} ({})\n",
            category.to_uppercase().bold(),
            results.len()
        ));
        output.push_str(&"-".repeat(70));
        output.push('\n');

        for result in results {
            let status = if result.passed {
                "PASS".green()
            } else {
                "FAIL".red()
            };

            output.push_str(&format!(
                "  {} {:40} {:>12} {:>12}\n",
                status,
                truncate_string(&result.name, 40),
                format_ns(result.statistics.mean_ns),
                format!("+/- {}", format_ns(result.statistics.std_dev_ns)).dimmed(),
            ));

            if let Some(threshold) = result.threshold_ns {
                let threshold_status = if result.passed { "ok" } else { "exceeded" };
                output.push_str(&format!(
                    "       threshold: {} ({})\n",
                    format_ns(threshold),
                    threshold_status
                ));
            }
        }
        output.push('\n');
    }

    // Comparisons
    if !report.comparisons.is_empty() {
        output.push_str(&"-".repeat(70));
        output.push('\n');
        output.push_str(&format!("{}\n", "BASELINE COMPARISONS".bold()));
        output.push_str(&"-".repeat(70));
        output.push('\n');

        for comparison in &report.comparisons {
            let assessment_str = match comparison.assessment {
                ComparisonAssessment::VerumFaster => "FASTER".green(),
                ComparisonAssessment::Comparable => "COMPARABLE".cyan(),
                ComparisonAssessment::AcceptableSlower => "OK".yellow(),
                ComparisonAssessment::TooSlow => "TOO SLOW".red(),
            };

            output.push_str(&format!(
                "  {} vs {} {}: {:.2}x ({:+.1}%)\n",
                comparison.name,
                comparison.baseline.language,
                assessment_str,
                comparison.ratio,
                comparison.percentage_diff,
            ));
        }
        output.push('\n');
    }

    // Regressions
    let actual_regressions: Vec<_> = report
        .regressions
        .iter()
        .filter(|r| r.is_regression)
        .collect();

    if !actual_regressions.is_empty() {
        output.push_str(&"-".repeat(70));
        output.push('\n');
        output.push_str(&format!("{}\n", "REGRESSIONS DETECTED".bold().red()));
        output.push_str(&"-".repeat(70));
        output.push('\n');

        for regression in actual_regressions {
            output.push_str(&format!(
                "  {} {}: {} -> {} ({:+.1}%)\n",
                "REGRESSION".red(),
                regression.name,
                format_ns(regression.baseline_mean_ns),
                format_ns(regression.current_mean_ns),
                regression.percentage_change,
            ));
        }
        output.push('\n');
    }

    // Footer
    output.push_str(&"=".repeat(70));
    output.push('\n');
    let result_str = if report.summary.failed == 0 && report.summary.regressions == 0 {
        "ALL BENCHMARKS PASSED".green().bold()
    } else {
        format!(
            "{} FAILED, {} REGRESSIONS",
            report.summary.failed, report.summary.regressions
        )
        .red()
        .bold()
    };
    output.push_str(&format!("{}\n", result_str));
    output.push_str(&"=".repeat(70));
    output.push('\n');

    Ok(output)
}

// ============================================================================
// JSON Report
// ============================================================================

fn generate_json_report(report: &BenchmarkReport) -> Result<String> {
    serde_json::to_string_pretty(report).context("Failed to serialize report to JSON")
}

// ============================================================================
// HTML Report
// ============================================================================

fn generate_html_report(report: &BenchmarkReport) -> Result<String> {
    let mut html = String::new();

    html.push_str(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>"#,
    );
    html.push_str(&report.metadata.title);
    html.push_str(
        r#"</title>
    <style>
        :root {
            --bg-primary: #1a1a2e;
            --bg-secondary: #16213e;
            --text-primary: #eee;
            --text-secondary: #aaa;
            --accent: #0f4c75;
            --success: #00b894;
            --warning: #fdcb6e;
            --error: #e74c3c;
        }
        body {
            font-family: 'Segoe UI', system-ui, sans-serif;
            background: var(--bg-primary);
            color: var(--text-primary);
            margin: 0;
            padding: 20px;
            line-height: 1.6;
        }
        .container {
            max-width: 1200px;
            margin: 0 auto;
        }
        h1, h2, h3 {
            color: var(--text-primary);
        }
        .header {
            background: var(--bg-secondary);
            padding: 20px;
            border-radius: 8px;
            margin-bottom: 20px;
        }
        .summary {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
            gap: 15px;
            margin-bottom: 20px;
        }
        .stat-card {
            background: var(--bg-secondary);
            padding: 15px;
            border-radius: 8px;
            text-align: center;
        }
        .stat-value {
            font-size: 2em;
            font-weight: bold;
        }
        .stat-label {
            color: var(--text-secondary);
            font-size: 0.9em;
        }
        .passed { color: var(--success); }
        .failed { color: var(--error); }
        .warning { color: var(--warning); }
        table {
            width: 100%;
            border-collapse: collapse;
            background: var(--bg-secondary);
            border-radius: 8px;
            overflow: hidden;
            margin-bottom: 20px;
        }
        th, td {
            padding: 12px;
            text-align: left;
            border-bottom: 1px solid var(--bg-primary);
        }
        th {
            background: var(--accent);
            color: white;
        }
        tr:hover {
            background: rgba(255,255,255,0.05);
        }
        .status-pass {
            color: var(--success);
            font-weight: bold;
        }
        .status-fail {
            color: var(--error);
            font-weight: bold;
        }
        .bar-container {
            width: 100px;
            height: 8px;
            background: #333;
            border-radius: 4px;
            overflow: hidden;
        }
        .bar {
            height: 100%;
            background: var(--accent);
        }
    </style>
</head>
<body>
    <div class="container">
"#,
    );

    // Header
    html.push_str(&format!(
        r#"<div class="header">
    <h1>{}</h1>
    <p>Verum {} | {} | {}</p>
</div>"#,
        report.metadata.title,
        report.metadata.verum_version,
        report.metadata.platform,
        report.metadata.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
    ));

    // Summary cards
    html.push_str(r#"<div class="summary">"#);
    html.push_str(&format!(
        r#"<div class="stat-card"><div class="stat-value">{}</div><div class="stat-label">Total</div></div>"#,
        report.summary.total
    ));
    html.push_str(&format!(
        r#"<div class="stat-card"><div class="stat-value passed">{}</div><div class="stat-label">Passed</div></div>"#,
        report.summary.passed
    ));
    html.push_str(&format!(
        r#"<div class="stat-card"><div class="stat-value failed">{}</div><div class="stat-label">Failed</div></div>"#,
        report.summary.failed
    ));
    html.push_str(&format!(
        r#"<div class="stat-card"><div class="stat-value">{:.1}%</div><div class="stat-label">Pass Rate</div></div>"#,
        report.summary.pass_rate * 100.0
    ));
    html.push_str(r#"</div>"#);

    // Results table
    html.push_str(
        r#"<h2>Benchmark Results</h2>
<table>
    <thead>
        <tr>
            <th>Status</th>
            <th>Name</th>
            <th>Category</th>
            <th>Mean</th>
            <th>Std Dev</th>
            <th>Min</th>
            <th>Max</th>
            <th>Threshold</th>
        </tr>
    </thead>
    <tbody>"#,
    );

    for result in &report.results {
        let status_class = if result.passed {
            "status-pass"
        } else {
            "status-fail"
        };
        let status_text = if result.passed { "PASS" } else { "FAIL" };

        html.push_str(&format!(
            r#"<tr>
    <td class="{}">{}</td>
    <td>{}</td>
    <td>{}</td>
    <td>{}</td>
    <td>{}</td>
    <td>{}</td>
    <td>{}</td>
    <td>{}</td>
</tr>"#,
            status_class,
            status_text,
            result.name,
            result.category,
            format_ns(result.statistics.mean_ns),
            format_ns(result.statistics.std_dev_ns),
            format_ns(result.statistics.min_ns),
            format_ns(result.statistics.max_ns),
            result
                .threshold_ns
                .map(|t| format_ns(t))
                .unwrap_or_else(|| "-".to_string()),
        ));
    }

    html.push_str(r#"</tbody></table>"#);

    // Comparisons
    if !report.comparisons.is_empty() {
        html.push_str(
            r#"<h2>Baseline Comparisons</h2>
<table>
    <thead>
        <tr>
            <th>Benchmark</th>
            <th>Language</th>
            <th>Ratio</th>
            <th>Difference</th>
            <th>Assessment</th>
        </tr>
    </thead>
    <tbody>"#,
        );

        for comparison in &report.comparisons {
            let assessment_class = match comparison.assessment {
                ComparisonAssessment::VerumFaster => "passed",
                ComparisonAssessment::Comparable => "",
                ComparisonAssessment::AcceptableSlower => "warning",
                ComparisonAssessment::TooSlow => "failed",
            };

            html.push_str(&format!(
                r#"<tr>
    <td>{}</td>
    <td>{}</td>
    <td>{:.2}x</td>
    <td>{:+.1}%</td>
    <td class="{}">{:?}</td>
</tr>"#,
                comparison.name,
                comparison.baseline.language,
                comparison.ratio,
                comparison.percentage_diff,
                assessment_class,
                comparison.assessment,
            ));
        }

        html.push_str(r#"</tbody></table>"#);
    }

    // Performance Distribution Chart (SVG)
    html.push_str(r#"<h2>Performance Distribution</h2>"#);
    html.push_str(&generate_bar_chart_svg(&report.results));

    // Timing Histogram (if we have enough data)
    if report.results.len() >= 3 {
        html.push_str(r#"<h2>Benchmark Timing Comparison</h2>"#);
        html.push_str(&generate_timing_chart_svg(&report.results));
    }

    // Category breakdown pie chart
    if !report.by_category.is_empty() {
        html.push_str(r#"<h2>Benchmarks by Category</h2>"#);
        html.push_str(&generate_category_pie_svg(&report.by_category));
    }

    // Threshold compliance chart
    let results_with_threshold: Vec<_> = report
        .results
        .iter()
        .filter(|r| r.threshold_ns.is_some())
        .collect();
    if !results_with_threshold.is_empty() {
        html.push_str(r#"<h2>Threshold Compliance</h2>"#);
        html.push_str(&generate_threshold_chart_svg(&results_with_threshold));
    }

    // Footer
    html.push_str(
        r#"
    </div>
</body>
</html>"#,
    );

    Ok(html)
}

/// Generate an SVG bar chart for benchmark results.
fn generate_bar_chart_svg(results: &[BenchmarkResult]) -> String {
    if results.is_empty() {
        return String::new();
    }

    let max_value = results
        .iter()
        .map(|r| r.statistics.mean_ns)
        .fold(0.0f64, |a, b| a.max(b));

    let bar_height = 25;
    let bar_gap = 5;
    let label_width = 250;
    let chart_width = 600;
    let total_width = label_width + chart_width + 100;
    let total_height = (bar_height + bar_gap) * results.len() + 50;

    let mut svg = format!(
        r##"<svg width="{}" height="{}" xmlns="http://www.w3.org/2000/svg">
        <style>
            .bar {{ fill: #0f4c75; }}
            .bar-pass {{ fill: #00b894; }}
            .bar-fail {{ fill: #e74c3c; }}
            .label {{ fill: #eee; font-family: monospace; font-size: 12px; }}
            .value {{ fill: #aaa; font-family: monospace; font-size: 11px; }}
            .axis {{ stroke: #444; stroke-width: 1; }}
        </style>
        <rect width="100%" height="100%" fill="#1a1a2e"/>
        <line x1="{}" y1="10" x2="{}" y2="{}" class="axis"/>
        "##,
        total_width,
        total_height,
        label_width,
        label_width,
        total_height - 20
    );

    for (i, result) in results.iter().enumerate() {
        let y = (i * (bar_height + bar_gap)) + 15;
        let bar_width = if max_value > 0.0 {
            ((result.statistics.mean_ns / max_value) * chart_width as f64) as usize
        } else {
            0
        };

        let bar_class = if result.passed {
            "bar-pass"
        } else {
            "bar-fail"
        };

        // Truncate name for display
        let display_name = if result.name.len() > 30 {
            format!("{}...", &result.name[..27])
        } else {
            result.name.clone()
        };

        svg.push_str(&format!(
            r#"<text x="5" y="{}" class="label">{}</text>
            <rect x="{}" y="{}" width="{}" height="{}" class="{}"/>
            <text x="{}" y="{}" class="value">{}</text>
            "#,
            y + bar_height / 2 + 4,
            display_name,
            label_width,
            y,
            bar_width.max(2),
            bar_height,
            bar_class,
            label_width + bar_width + 5,
            y + bar_height / 2 + 4,
            format_ns(result.statistics.mean_ns)
        ));
    }

    svg.push_str("</svg>");
    svg
}

/// Generate SVG timing comparison chart.
fn generate_timing_chart_svg(results: &[BenchmarkResult]) -> String {
    let display_results: Vec<_> = results.iter().take(15).collect();
    if display_results.is_empty() {
        return String::new();
    }

    let max_value = display_results
        .iter()
        .map(|r| r.statistics.max_ns)
        .fold(0.0f64, |a, b| a.max(b));

    let chart_width = 800;
    let chart_height = 300;
    let margin_left = 80;
    let margin_bottom = 100;
    let margin_top = 30;
    let bar_width = (chart_width - margin_left) / display_results.len() - 10;

    let mut svg = format!(
        r#"<svg width="{}" height="{}" xmlns="http://www.w3.org/2000/svg">
        <style>
            .chart-bg {{ fill: #1a1a2e; }}
            .bar-mean {{ fill: #0f4c75; }}
            .bar-std {{ fill: rgba(15, 76, 117, 0.3); }}
            .axis-line {{ stroke: #444; stroke-width: 1; }}
            .axis-label {{ fill: #aaa; font-family: sans-serif; font-size: 10px; }}
            .title {{ fill: #eee; font-family: sans-serif; font-size: 12px; font-weight: bold; }}
            .grid {{ stroke: #333; stroke-width: 0.5; stroke-dasharray: 2,2; }}
        </style>
        <rect width="100%" height="100%" class="chart-bg"/>
        "#,
        chart_width,
        chart_height + margin_bottom
    );

    // Y-axis
    svg.push_str(&format!(
        r#"<line x1="{}" y1="{}" x2="{}" y2="{}" class="axis-line"/>"#,
        margin_left, margin_top, margin_left, chart_height
    ));

    // X-axis
    svg.push_str(&format!(
        r#"<line x1="{}" y1="{}" x2="{}" y2="{}" class="axis-line"/>"#,
        margin_left, chart_height, chart_width, chart_height
    ));

    // Y-axis labels and grid
    for i in 0..=5 {
        let y = margin_top + (chart_height - margin_top) * i / 5;
        let value = max_value * (5 - i) as f64 / 5.0;
        svg.push_str(&format!(
            r#"<text x="{}" y="{}" class="axis-label" text-anchor="end">{}</text>
            <line x1="{}" y1="{}" x2="{}" y2="{}" class="grid"/>"#,
            margin_left - 5,
            y + 4,
            format_ns(value),
            margin_left,
            y,
            chart_width,
            y
        ));
    }

    // Bars
    for (i, result) in display_results.iter().enumerate() {
        let x = margin_left + i * (bar_width + 10) + 5;
        let height_scale = (chart_height - margin_top) as f64;

        // Mean bar
        let mean_height = if max_value > 0.0 {
            (result.statistics.mean_ns / max_value * height_scale) as usize
        } else {
            0
        };
        let mean_y = chart_height - mean_height;

        // Std dev overlay
        let std_height = if max_value > 0.0 {
            (result.statistics.std_dev_ns / max_value * height_scale) as usize
        } else {
            0
        };

        svg.push_str(&format!(
            r#"<rect x="{}" y="{}" width="{}" height="{}" class="bar-mean"/>
            <rect x="{}" y="{}" width="{}" height="{}" class="bar-std"/>"#,
            x,
            mean_y,
            bar_width,
            mean_height,
            x,
            mean_y - std_height / 2,
            bar_width,
            std_height
        ));

        // X-axis label (rotated)
        let label = if result.name.len() > 15 {
            format!("{}...", &result.name[..12])
        } else {
            result.name.clone()
        };
        svg.push_str(&format!(
            r#"<text x="{}" y="{}" class="axis-label" transform="rotate(45, {}, {})">{}</text>"#,
            x + bar_width / 2,
            chart_height + 15,
            x + bar_width / 2,
            chart_height + 15,
            label
        ));
    }

    svg.push_str("</svg>");
    svg
}

/// Generate SVG pie chart for category breakdown.
fn generate_category_pie_svg(
    by_category: &std::collections::HashMap<String, Vec<BenchmarkResult>>,
) -> String {
    let total: usize = by_category.values().map(|v| v.len()).sum();
    if total == 0 {
        return String::new();
    }

    let chart_size = 300;
    let center = chart_size / 2;
    let radius = 100;

    let colors = [
        "#0f4c75", "#00b894", "#e74c3c", "#fdcb6e", "#6c5ce7", "#00cec9",
    ];

    let mut svg = format!(
        r##"<svg width="{}" height="{}" xmlns="http://www.w3.org/2000/svg">
        <style>
            .legend-text {{ fill: #eee; font-family: sans-serif; font-size: 12px; }}
        </style>
        <rect width="100%" height="100%" fill="#1a1a2e"/>
        "##,
        chart_size + 150,
        chart_size
    );

    let mut start_angle = 0.0f64;
    let mut legend_y = 30;

    for (i, (category, results)) in by_category.iter().enumerate() {
        let percentage = results.len() as f64 / total as f64;
        let angle = percentage * 360.0;
        let end_angle = start_angle + angle;

        let large_arc = if angle > 180.0 { 1 } else { 0 };

        let start_x =
            center as f64 + radius as f64 * (start_angle * std::f64::consts::PI / 180.0).cos();
        let start_y =
            center as f64 + radius as f64 * (start_angle * std::f64::consts::PI / 180.0).sin();
        let end_x =
            center as f64 + radius as f64 * (end_angle * std::f64::consts::PI / 180.0).cos();
        let end_y =
            center as f64 + radius as f64 * (end_angle * std::f64::consts::PI / 180.0).sin();

        let color = colors[i % colors.len()];

        svg.push_str(&format!(
            r#"<path d="M {} {} L {} {} A {} {} 0 {} 1 {} {} Z" fill="{}"/>"#,
            center, center, start_x, start_y, radius, radius, large_arc, end_x, end_y, color
        ));

        // Legend
        svg.push_str(&format!(
            r#"<rect x="{}" y="{}" width="15" height="15" fill="{}"/>
            <text x="{}" y="{}" class="legend-text">{}: {} ({:.0}%)</text>"#,
            chart_size + 10,
            legend_y,
            color,
            chart_size + 30,
            legend_y + 12,
            category,
            results.len(),
            percentage * 100.0
        ));

        legend_y += 25;
        start_angle = end_angle;
    }

    svg.push_str("</svg>");
    svg
}

/// Generate SVG threshold compliance chart.
fn generate_threshold_chart_svg(results: &[&BenchmarkResult]) -> String {
    let display_results: Vec<_> = results.iter().take(10).collect();
    if display_results.is_empty() {
        return String::new();
    }

    let chart_width = 700;
    let row_height = 40;
    let total_height = row_height * display_results.len() + 60;
    let bar_start = 200;
    let bar_width = 400;

    let max_threshold = display_results
        .iter()
        .filter_map(|r| r.threshold_ns)
        .fold(0.0f64, |a, b| a.max(b));

    let mut svg = format!(
        r#"<svg width="{}" height="{}" xmlns="http://www.w3.org/2000/svg">
        <style>
            .bg {{ fill: #1a1a2e; }}
            .label {{ fill: #eee; font-family: monospace; font-size: 11px; }}
            .bar-actual {{ fill: #0f4c75; }}
            .bar-threshold {{ fill: none; stroke: #fdcb6e; stroke-width: 2; stroke-dasharray: 4,2; }}
            .pass-indicator {{ fill: #00b894; }}
            .fail-indicator {{ fill: #e74c3c; }}
            .threshold-line {{ stroke: #e74c3c; stroke-width: 2; }}
        </style>
        <rect width="100%" height="100%" class="bg"/>
        <text x="{}" y="25" class="label" font-weight="bold">Actual (bar) vs Threshold (dashed line)</text>
        "#,
        chart_width, total_height, bar_start
    );

    for (i, result) in display_results.iter().enumerate() {
        let y = i * row_height + 40;
        let threshold = result.threshold_ns.unwrap_or(0.0);
        let actual = result.statistics.mean_ns;

        let actual_width = if max_threshold > 0.0 {
            ((actual / max_threshold) * bar_width as f64).min(bar_width as f64) as usize
        } else {
            0
        };

        let threshold_x = if max_threshold > 0.0 {
            bar_start + ((threshold / max_threshold) * bar_width as f64) as usize
        } else {
            bar_start
        };

        let indicator_class = if result.passed {
            "pass-indicator"
        } else {
            "fail-indicator"
        };

        let display_name = if result.name.len() > 25 {
            format!("{}...", &result.name[..22])
        } else {
            result.name.clone()
        };

        svg.push_str(&format!(
            r#"<text x="5" y="{}" class="label">{}</text>
            <rect x="{}" y="{}" width="{}" height="20" class="bar-actual"/>
            <line x1="{}" y1="{}" x2="{}" y2="{}" class="threshold-line"/>
            <circle cx="{}" cy="{}" r="5" class="{}"/>
            <text x="{}" y="{}" class="label" font-size="10">{} / {}</text>
            "#,
            y + 15,
            display_name,
            bar_start,
            y,
            actual_width,
            threshold_x,
            y,
            threshold_x,
            y + 20,
            chart_width - 15,
            y + 10,
            indicator_class,
            bar_start + bar_width + 10,
            y + 15,
            format_ns(actual),
            format_ns(threshold)
        ));
    }

    svg.push_str("</svg>");
    svg
}

// ============================================================================
// CSV Report
// ============================================================================

fn generate_csv_report(report: &BenchmarkReport) -> Result<String> {
    let mut output = String::new();

    // Header
    output.push_str("name,category,tier,passed,mean_ns,std_dev_ns,min_ns,max_ns,median_ns,p95_ns,p99_ns,threshold_ns\n");

    // Data rows
    for result in &report.results {
        output.push_str(&format!(
            "{},{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}\n",
            escape_csv(&result.name),
            result.category,
            result.tier.map(|t| t.to_string()).unwrap_or_default(),
            result.passed,
            result.statistics.mean_ns,
            result.statistics.std_dev_ns,
            result.statistics.min_ns,
            result.statistics.max_ns,
            result.statistics.median_ns,
            result.statistics.p95_ns,
            result.statistics.p99_ns,
            result
                .threshold_ns
                .map(|t| format!("{:.2}", t))
                .unwrap_or_default(),
        ));
    }

    Ok(output)
}

fn escape_csv(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// ============================================================================
// Markdown Report
// ============================================================================

fn generate_markdown_report(report: &BenchmarkReport) -> Result<String> {
    let mut md = String::new();

    // Header
    md.push_str(&format!("# {}\n\n", report.metadata.title));
    md.push_str(&format!(
        "**Verum Version:** {}  \n",
        report.metadata.verum_version
    ));
    md.push_str(&format!("**Platform:** {}  \n", report.metadata.platform));
    md.push_str(&format!(
        "**Timestamp:** {}  \n\n",
        report.metadata.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
    ));

    // Summary
    md.push_str("## Summary\n\n");
    md.push_str(&format!("| Metric | Value |\n|--------|-------|\n"));
    md.push_str(&format!("| Total | {} |\n", report.summary.total));
    md.push_str(&format!("| Passed | {} |\n", report.summary.passed));
    md.push_str(&format!("| Failed | {} |\n", report.summary.failed));
    md.push_str(&format!(
        "| Pass Rate | {:.1}% |\n",
        report.summary.pass_rate * 100.0
    ));
    md.push_str(&format!(
        "| Regressions | {} |\n\n",
        report.summary.regressions
    ));

    // Results table
    md.push_str("## Results\n\n");
    md.push_str("| Status | Name | Category | Mean | Std Dev | Threshold |\n");
    md.push_str("|--------|------|----------|------|---------|----------|\n");

    for result in &report.results {
        let status = if result.passed { "PASS" } else { "FAIL" };
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            status,
            result.name,
            result.category,
            format_ns(result.statistics.mean_ns),
            format_ns(result.statistics.std_dev_ns),
            result
                .threshold_ns
                .map(|t| format_ns(t))
                .unwrap_or_else(|| "-".to_string()),
        ));
    }

    // Comparisons
    if !report.comparisons.is_empty() {
        md.push_str("\n## Baseline Comparisons\n\n");
        md.push_str("| Benchmark | vs Language | Ratio | Difference | Assessment |\n");
        md.push_str("|-----------|-------------|-------|------------|------------|\n");

        for comparison in &report.comparisons {
            md.push_str(&format!(
                "| {} | {} | {:.2}x | {:+.1}% | {:?} |\n",
                comparison.name,
                comparison.baseline.language,
                comparison.ratio,
                comparison.percentage_diff,
                comparison.assessment,
            ));
        }
    }

    // Regressions
    let regressions: Vec<_> = report
        .regressions
        .iter()
        .filter(|r| r.is_regression)
        .collect();

    if !regressions.is_empty() {
        md.push_str("\n## Regressions\n\n");
        md.push_str("| Benchmark | Previous | Current | Change |\n");
        md.push_str("|-----------|----------|---------|--------|\n");

        for regression in regressions {
            md.push_str(&format!(
                "| {} | {} | {} | {:+.1}% |\n",
                regression.name,
                format_ns(regression.baseline_mean_ns),
                format_ns(regression.current_mean_ns),
                regression.percentage_change,
            ));
        }
    }

    Ok(md)
}

// ============================================================================
// Utilities
// ============================================================================

/// Format nanoseconds for display.
fn format_ns(ns: f64) -> String {
    if ns >= 1_000_000_000.0 {
        format!("{:.2}s", ns / 1_000_000_000.0)
    } else if ns >= 1_000_000.0 {
        format!("{:.2}ms", ns / 1_000_000.0)
    } else if ns >= 1_000.0 {
        format!("{:.2}us", ns / 1_000.0)
    } else {
        format!("{:.2}ns", ns)
    }
}

/// Truncate a string to max length.
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

/// Get CPU information.
fn get_cpu_info() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
    }

    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|content| {
                content
                    .lines()
                    .find(|l| l.starts_with("model name"))
                    .and_then(|l| l.split(':').nth(1))
                    .map(|s| s.trim().to_string())
            })
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// Get memory information.
fn get_memory_info() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|bytes| format!("{} GB", bytes / 1_073_741_824))
    }

    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|content| {
                content
                    .lines()
                    .find(|l| l.starts_with("MemTotal"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|kb| format!("{} GB", kb / 1_048_576))
            })
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{BenchmarkCategory, Statistics};
    use std::time::Duration;

    fn create_test_result(name: &str, mean_ns: f64, _passed: bool) -> BenchmarkResult {
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
                total_duration: Duration::from_nanos(mean_ns as u64 * 100),
            },
            Some(mean_ns * 1.2),
        )
    }

    #[test]
    fn test_report_summary() {
        let results = vec![
            create_test_result("test1", 10.0, true),
            create_test_result("test2", 20.0, true),
            create_test_result("test3", 30.0, false),
        ];

        let summary = ReportSummary::from_results(&results, &[]);

        assert_eq!(summary.total, 3);
        assert_eq!(summary.passed, 3); // All pass threshold
    }

    #[test]
    fn test_format_ns() {
        assert_eq!(format_ns(5.0), "5.00ns");
        assert_eq!(format_ns(5000.0), "5.00us");
        assert_eq!(format_ns(5_000_000.0), "5.00ms");
        assert_eq!(format_ns(5_000_000_000.0), "5.00s");
    }

    #[test]
    fn test_json_report() {
        let metadata = ReportMetadata::new("Test Report", "1.0.0");
        let results = vec![create_test_result("test", 15.0, true)];
        let report = BenchmarkReport::new(metadata, results, vec![], vec![]);

        let json = generate_json_report(&report).unwrap();
        assert!(json.contains("Test Report"));
        assert!(json.contains("test"));
    }

    #[test]
    fn test_csv_report() {
        let metadata = ReportMetadata::new("Test Report", "1.0.0");
        let results = vec![create_test_result("test", 15.0, true)];
        let report = BenchmarkReport::new(metadata, results, vec![], vec![]);

        let csv = generate_csv_report(&report).unwrap();
        assert!(csv.contains("name,category"));
        assert!(csv.contains("test,micro"));
    }
}
