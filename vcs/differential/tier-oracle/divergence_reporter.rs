//! Divergence Reporter for Tier Oracle
//!
//! This module provides comprehensive reporting capabilities for divergences
//! found during differential testing, with multiple output formats and
//! detailed diagnostic information.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A divergence between execution tiers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierDivergence {
    /// Unique identifier for this divergence
    pub id: String,

    /// Source file that caused the divergence
    pub source_file: PathBuf,

    /// Reference tier
    pub reference_tier: u8,

    /// Divergent tier
    pub divergent_tier: u8,

    /// Classification of the divergence
    pub classification: DivergenceClassification,

    /// Severity of the divergence
    pub severity: DivergenceSeverity,

    /// Summary of the divergence
    pub summary: String,

    /// Detailed description
    pub details: String,

    /// Expected output (from reference tier)
    pub expected: String,

    /// Actual output (from divergent tier)
    pub actual: String,

    /// Unified diff
    pub diff: String,

    /// Execution timings
    pub timings: ExecutionTimings,

    /// Timestamp when divergence was detected
    pub timestamp: DateTime<Utc>,

    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

/// Classification of divergence types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DivergenceClassification {
    /// Stdout differs
    StdoutMismatch,

    /// Stderr differs
    StderrMismatch,

    /// Exit code differs
    ExitCodeMismatch,

    /// Floating-point precision differs
    FloatPrecision,

    /// Memory address differs (usually a normalization issue)
    MemoryAddress,

    /// Timing/ordering differs (non-determinism)
    NonDeterministic,

    /// One tier crashed
    Crash,

    /// One tier timed out
    Timeout,

    /// Execution error
    ExecutionError,

    /// Type error at runtime
    TypeError,

    /// Arithmetic error (overflow, divide by zero, etc.)
    ArithmeticError,

    /// Memory safety error
    MemorySafetyError,

    /// Unknown divergence type
    Unknown,
}

impl DivergenceClassification {
    /// Get a human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            DivergenceClassification::StdoutMismatch => "Stdout Mismatch",
            DivergenceClassification::StderrMismatch => "Stderr Mismatch",
            DivergenceClassification::ExitCodeMismatch => "Exit Code Mismatch",
            DivergenceClassification::FloatPrecision => "Float Precision",
            DivergenceClassification::MemoryAddress => "Memory Address",
            DivergenceClassification::NonDeterministic => "Non-Deterministic",
            DivergenceClassification::Crash => "Crash",
            DivergenceClassification::Timeout => "Timeout",
            DivergenceClassification::ExecutionError => "Execution Error",
            DivergenceClassification::TypeError => "Type Error",
            DivergenceClassification::ArithmeticError => "Arithmetic Error",
            DivergenceClassification::MemorySafetyError => "Memory Safety Error",
            DivergenceClassification::Unknown => "Unknown",
        }
    }

    /// Get a color code for terminal output
    pub fn color_code(&self) -> &'static str {
        match self {
            DivergenceClassification::FloatPrecision |
            DivergenceClassification::MemoryAddress => "\x1b[33m",  // Yellow
            DivergenceClassification::Crash |
            DivergenceClassification::MemorySafetyError => "\x1b[31m",  // Red
            DivergenceClassification::Timeout |
            DivergenceClassification::NonDeterministic => "\x1b[35m",  // Magenta
            _ => "\x1b[36m",  // Cyan
        }
    }
}

/// Severity levels for divergences
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DivergenceSeverity {
    /// Informational, may be acceptable
    Info,

    /// Minor divergence, investigate if time permits
    Low,

    /// Moderate divergence, should be investigated
    Medium,

    /// Significant divergence, likely a bug
    High,

    /// Critical divergence, definitely a bug
    Critical,
}

impl DivergenceSeverity {
    /// Get a human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            DivergenceSeverity::Info => "Info",
            DivergenceSeverity::Low => "Low",
            DivergenceSeverity::Medium => "Medium",
            DivergenceSeverity::High => "High",
            DivergenceSeverity::Critical => "Critical",
        }
    }

    /// Get a color code for terminal output
    pub fn color_code(&self) -> &'static str {
        match self {
            DivergenceSeverity::Info => "\x1b[37m",      // White
            DivergenceSeverity::Low => "\x1b[36m",       // Cyan
            DivergenceSeverity::Medium => "\x1b[33m",    // Yellow
            DivergenceSeverity::High => "\x1b[31m",      // Red
            DivergenceSeverity::Critical => "\x1b[35m",  // Magenta
        }
    }
}

/// Execution timing information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTimings {
    /// Reference tier execution time in milliseconds
    pub reference_ms: u64,

    /// Divergent tier execution time in milliseconds
    pub divergent_ms: u64,

    /// Speedup factor (reference / divergent)
    pub speedup: f64,
}

/// Report format options
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    /// Plain text
    Text,

    /// JSON
    Json,

    /// SARIF (Static Analysis Results Interchange Format)
    Sarif,

    /// Markdown
    Markdown,

    /// HTML
    Html,

    /// JUnit XML (for CI integration)
    JUnit,
}

/// Configuration for the divergence reporter
#[derive(Debug, Clone)]
pub struct ReporterConfig {
    /// Output directory for reports
    pub output_dir: PathBuf,

    /// Default report format
    pub default_format: ReportFormat,

    /// Number of context lines to show in diffs
    pub context_lines: usize,

    /// Whether to colorize text output
    pub colorize: bool,

    /// Whether to include timestamps
    pub include_timestamps: bool,

    /// Maximum diff size before truncation (bytes)
    pub max_diff_size: usize,

    /// Whether to generate individual files per divergence
    pub individual_files: bool,
}

impl Default for ReporterConfig {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("reports"),
            default_format: ReportFormat::Text,
            context_lines: 3,
            colorize: true,
            include_timestamps: true,
            max_diff_size: 10_000,
            individual_files: false,
        }
    }
}

/// The divergence reporter
pub struct DivergenceReporter {
    config: ReporterConfig,
    divergences: Vec<TierDivergence>,
}

impl DivergenceReporter {
    /// Create a new reporter
    pub fn new(config: ReporterConfig) -> Self {
        Self {
            config,
            divergences: Vec::new(),
        }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self::new(ReporterConfig::default())
    }

    /// Add a divergence to the report
    pub fn add_divergence(&mut self, divergence: TierDivergence) {
        self.divergences.push(divergence);
    }

    /// Get all divergences
    pub fn divergences(&self) -> &[TierDivergence] {
        &self.divergences
    }

    /// Get divergence count by classification
    pub fn count_by_classification(&self) -> HashMap<DivergenceClassification, usize> {
        let mut counts = HashMap::new();
        for div in &self.divergences {
            *counts.entry(div.classification).or_insert(0) += 1;
        }
        counts
    }

    /// Get divergence count by severity
    pub fn count_by_severity(&self) -> HashMap<DivergenceSeverity, usize> {
        let mut counts = HashMap::new();
        for div in &self.divergences {
            *counts.entry(div.severity).or_insert(0) += 1;
        }
        counts
    }

    /// Generate report in the specified format
    pub fn generate(&self, format: ReportFormat) -> String {
        match format {
            ReportFormat::Text => self.generate_text(),
            ReportFormat::Json => self.generate_json(),
            ReportFormat::Sarif => self.generate_sarif(),
            ReportFormat::Markdown => self.generate_markdown(),
            ReportFormat::Html => self.generate_html(),
            ReportFormat::JUnit => self.generate_junit(),
        }
    }

    /// Write report to file
    pub fn write_to_file(&self, path: &Path, format: ReportFormat) -> std::io::Result<()> {
        let content = self.generate(format);
        let mut file = fs::File::create(path)?;
        file.write_all(content.as_bytes())?;
        Ok(())
    }

    /// Generate a summary line
    pub fn summary(&self) -> String {
        let total = self.divergences.len();
        let critical = self.divergences.iter()
            .filter(|d| d.severity == DivergenceSeverity::Critical)
            .count();
        let high = self.divergences.iter()
            .filter(|d| d.severity == DivergenceSeverity::High)
            .count();

        format!(
            "{} divergences ({} critical, {} high)",
            total, critical, high
        )
    }

    /// Generate text report
    fn generate_text(&self) -> String {
        let mut output = String::new();
        let reset = if self.config.colorize { "\x1b[0m" } else { "" };

        output.push_str("=== Differential Test Report ===\n");
        output.push_str(&format!("Total divergences: {}\n\n", self.divergences.len()));

        // Summary by classification
        output.push_str("By Classification:\n");
        for (class, count) in self.count_by_classification() {
            let color = if self.config.colorize { class.color_code() } else { "" };
            output.push_str(&format!("  {}{}: {}{}\n", color, class.name(), count, reset));
        }
        output.push('\n');

        // Summary by severity
        output.push_str("By Severity:\n");
        for (sev, count) in self.count_by_severity() {
            let color = if self.config.colorize { sev.color_code() } else { "" };
            output.push_str(&format!("  {}{}: {}{}\n", color, sev.name(), count, reset));
        }
        output.push('\n');

        // Individual divergences
        output.push_str("--- Divergences ---\n\n");
        for div in &self.divergences {
            output.push_str(&self.format_divergence_text(div));
            output.push('\n');
        }

        output
    }

    /// Format a single divergence as text
    fn format_divergence_text(&self, div: &TierDivergence) -> String {
        let mut output = String::new();
        let reset = if self.config.colorize { "\x1b[0m" } else { "" };
        let sev_color = if self.config.colorize { div.severity.color_code() } else { "" };
        let class_color = if self.config.colorize { div.classification.color_code() } else { "" };

        output.push_str(&format!("ID: {}\n", div.id));
        output.push_str(&format!("File: {}\n", div.source_file.display()));
        output.push_str(&format!("Classification: {}{}{}\n",
            class_color, div.classification.name(), reset));
        output.push_str(&format!("Severity: {}{}{}\n",
            sev_color, div.severity.name(), reset));
        output.push_str(&format!("Tiers: {} vs {}\n", div.reference_tier, div.divergent_tier));
        output.push_str(&format!("Summary: {}\n", div.summary));

        if self.config.include_timestamps {
            output.push_str(&format!("Timestamp: {}\n", div.timestamp));
        }

        output.push_str(&format!("\nTiming: {}ms (ref) vs {}ms (test) [{:.2}x]\n",
            div.timings.reference_ms, div.timings.divergent_ms, div.timings.speedup));

        output.push_str("\n--- Diff ---\n");
        if div.diff.len() > self.config.max_diff_size {
            output.push_str(&div.diff[..self.config.max_diff_size]);
            output.push_str("\n... (truncated)\n");
        } else {
            output.push_str(&div.diff);
        }

        output.push_str("\n-----------\n");
        output
    }

    /// Generate JSON report
    fn generate_json(&self) -> String {
        serde_json::to_string_pretty(&self.divergences).unwrap_or_default()
    }

    /// Generate SARIF report (for IDE integration)
    fn generate_sarif(&self) -> String {
        let runs: Vec<serde_json::Value> = vec![serde_json::json!({
            "tool": {
                "driver": {
                    "name": "vcs-differential",
                    "version": "0.1.0",
                    "informationUri": "https://github.com/verum-lang/verum/tree/main/vcs"
                }
            },
            "results": self.divergences.iter().map(|d| {
                serde_json::json!({
                    "ruleId": format!("DIFF-{:?}", d.classification),
                    "level": match d.severity {
                        DivergenceSeverity::Critical => "error",
                        DivergenceSeverity::High => "error",
                        DivergenceSeverity::Medium => "warning",
                        DivergenceSeverity::Low => "note",
                        DivergenceSeverity::Info => "note",
                    },
                    "message": {
                        "text": d.summary
                    },
                    "locations": [{
                        "physicalLocation": {
                            "artifactLocation": {
                                "uri": d.source_file.to_string_lossy()
                            }
                        }
                    }]
                })
            }).collect::<Vec<_>>()
        })];

        serde_json::to_string_pretty(&serde_json::json!({
            "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
            "version": "2.1.0",
            "runs": runs
        })).unwrap_or_default()
    }

    /// Generate Markdown report
    fn generate_markdown(&self) -> String {
        let mut output = String::new();

        output.push_str("# Differential Test Report\n\n");
        output.push_str(&format!("**Total divergences:** {}\n\n", self.divergences.len()));

        // Summary table
        output.push_str("## Summary\n\n");
        output.push_str("| Classification | Count |\n");
        output.push_str("|----------------|-------|\n");
        for (class, count) in self.count_by_classification() {
            output.push_str(&format!("| {} | {} |\n", class.name(), count));
        }
        output.push('\n');

        // Divergences
        output.push_str("## Divergences\n\n");
        for div in &self.divergences {
            output.push_str(&format!("### {}\n\n", div.id));
            output.push_str(&format!("- **File:** `{}`\n", div.source_file.display()));
            output.push_str(&format!("- **Classification:** {}\n", div.classification.name()));
            output.push_str(&format!("- **Severity:** {}\n", div.severity.name()));
            output.push_str(&format!("- **Tiers:** {} vs {}\n", div.reference_tier, div.divergent_tier));
            output.push_str(&format!("- **Summary:** {}\n\n", div.summary));

            output.push_str("```diff\n");
            output.push_str(&div.diff);
            output.push_str("```\n\n");
        }

        output
    }

    /// Generate HTML report
    fn generate_html(&self) -> String {
        let mut output = String::new();

        output.push_str(r#"<!DOCTYPE html>
<html>
<head>
    <title>Differential Test Report</title>
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; margin: 40px; }
        h1 { color: #333; }
        .summary { background: #f5f5f5; padding: 20px; border-radius: 8px; margin-bottom: 20px; }
        .divergence { border: 1px solid #ddd; padding: 20px; margin: 20px 0; border-radius: 8px; }
        .critical { border-left: 4px solid #dc3545; }
        .high { border-left: 4px solid #fd7e14; }
        .medium { border-left: 4px solid #ffc107; }
        .low { border-left: 4px solid #17a2b8; }
        .info { border-left: 4px solid #6c757d; }
        .diff { background: #f8f9fa; padding: 15px; font-family: monospace; white-space: pre-wrap; overflow-x: auto; }
        .diff-add { background: #d4edda; }
        .diff-del { background: #f8d7da; }
        table { border-collapse: collapse; width: 100%; }
        th, td { border: 1px solid #ddd; padding: 12px; text-align: left; }
        th { background: #f5f5f5; }
    </style>
</head>
<body>
    <h1>Differential Test Report</h1>
"#);

        // Summary
        output.push_str(&format!(r#"
    <div class="summary">
        <h2>Summary</h2>
        <p><strong>Total divergences:</strong> {}</p>
        <table>
            <tr><th>Classification</th><th>Count</th></tr>
"#, self.divergences.len()));

        for (class, count) in self.count_by_classification() {
            output.push_str(&format!("            <tr><td>{}</td><td>{}</td></tr>\n",
                class.name(), count));
        }

        output.push_str("        </table>\n    </div>\n\n    <h2>Divergences</h2>\n");

        // Divergences
        for div in &self.divergences {
            let sev_class = match div.severity {
                DivergenceSeverity::Critical => "critical",
                DivergenceSeverity::High => "high",
                DivergenceSeverity::Medium => "medium",
                DivergenceSeverity::Low => "low",
                DivergenceSeverity::Info => "info",
            };

            output.push_str(&format!(r#"
    <div class="divergence {}">
        <h3>{}</h3>
        <p><strong>File:</strong> <code>{}</code></p>
        <p><strong>Classification:</strong> {}</p>
        <p><strong>Severity:</strong> {}</p>
        <p><strong>Tiers:</strong> {} vs {}</p>
        <p><strong>Summary:</strong> {}</p>
        <div class="diff">{}</div>
    </div>
"#,
                sev_class,
                div.id,
                html_escape(&div.source_file.to_string_lossy()),
                div.classification.name(),
                div.severity.name(),
                div.reference_tier,
                div.divergent_tier,
                html_escape(&div.summary),
                html_escape(&div.diff)
            ));
        }

        output.push_str("</body>\n</html>\n");
        output
    }

    /// Generate JUnit XML report (for CI integration)
    fn generate_junit(&self) -> String {
        let mut output = String::new();

        output.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites>
    <testsuite name="differential-tests"
"#);
        output.push_str(&format!(r#"               tests="{}"
               failures="{}"
               errors="0">
"#,
            self.divergences.len(),
            self.divergences.iter()
                .filter(|d| d.severity >= DivergenceSeverity::Medium)
                .count()
        ));

        for div in &self.divergences {
            let status = if div.severity >= DivergenceSeverity::Medium { "failure" } else { "passed" };
            output.push_str(&format!(r#"        <testcase name="{}" classname="differential.{:?}">
"#, xml_escape(&div.source_file.to_string_lossy()), div.classification));

            if status == "failure" {
                output.push_str(&format!(r#"            <failure message="{}">
{}
            </failure>
"#, xml_escape(&div.summary), xml_escape(&div.diff)));
            }

            output.push_str("        </testcase>\n");
        }

        output.push_str("    </testsuite>\n</testsuites>\n");
        output
    }
}

/// Escape HTML special characters
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Escape XML special characters
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Create a divergence with automatic ID generation
pub fn create_divergence(
    source_file: PathBuf,
    reference_tier: u8,
    divergent_tier: u8,
    classification: DivergenceClassification,
    severity: DivergenceSeverity,
    summary: String,
    expected: String,
    actual: String,
    timings: ExecutionTimings,
) -> TierDivergence {
    use md5::{Md5, Digest};

    // Generate ID from content hash
    let mut hasher = Md5::new();
    hasher.update(source_file.to_string_lossy().as_bytes());
    hasher.update(&[reference_tier, divergent_tier]);
    hasher.update(expected.as_bytes());
    hasher.update(actual.as_bytes());
    let hash = hasher.finalize();
    let id = format!("{:x}", hash)[..12].to_string();

    // Compute diff
    let diff = compute_unified_diff(&expected, &actual, 3);

    TierDivergence {
        id,
        source_file,
        reference_tier,
        divergent_tier,
        classification,
        severity,
        summary,
        details: String::new(),
        expected,
        actual,
        diff,
        timings,
        timestamp: Utc::now(),
        metadata: HashMap::new(),
    }
}

/// Compute a unified diff between two strings
fn compute_unified_diff(expected: &str, actual: &str, context: usize) -> String {
    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();

    let mut diff = String::new();
    diff.push_str("--- Expected (Reference Tier)\n");
    diff.push_str("+++ Actual (Test Tier)\n");

    let max_lines = expected_lines.len().max(actual_lines.len());

    for i in 0..max_lines {
        let exp = expected_lines.get(i);
        let act = actual_lines.get(i);

        match (exp, act) {
            (Some(e), Some(a)) if e == a => {
                diff.push_str(&format!(" {}\n", e));
            }
            (Some(e), Some(a)) => {
                diff.push_str(&format!("-{}\n", e));
                diff.push_str(&format!("+{}\n", a));
            }
            (Some(e), None) => {
                diff.push_str(&format!("-{}\n", e));
            }
            (None, Some(a)) => {
                diff.push_str(&format!("+{}\n", a));
            }
            (None, None) => {}
        }
    }

    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_divergence() {
        let div = create_divergence(
            PathBuf::from("test.vr"),
            0,
            3,
            DivergenceClassification::StdoutMismatch,
            DivergenceSeverity::High,
            "Output differs".to_string(),
            "expected\n".to_string(),
            "actual\n".to_string(),
            ExecutionTimings {
                reference_ms: 100,
                divergent_ms: 10,
                speedup: 10.0,
            },
        );

        assert!(!div.id.is_empty());
        assert_eq!(div.reference_tier, 0);
        assert_eq!(div.divergent_tier, 3);
    }

    #[test]
    fn test_reporter_summary() {
        let mut reporter = DivergenceReporter::with_defaults();

        reporter.add_divergence(create_divergence(
            PathBuf::from("test1.vr"),
            0, 3,
            DivergenceClassification::StdoutMismatch,
            DivergenceSeverity::Critical,
            "Critical issue".to_string(),
            "a".to_string(),
            "b".to_string(),
            ExecutionTimings { reference_ms: 1, divergent_ms: 1, speedup: 1.0 },
        ));

        let summary = reporter.summary();
        assert!(summary.contains("1 divergences"));
        assert!(summary.contains("1 critical"));
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
    }
}
