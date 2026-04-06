//! Fuzzing report generation
//!
//! This module provides comprehensive reporting functionality for fuzzing campaigns:
//!
//! - **Text reports**: Human-readable summaries
//! - **JSON reports**: Machine-readable structured data
//! - **HTML reports**: Interactive visualization
//! - **Coverage reports**: Source-level coverage mapping
//!
//! Reports can be generated incrementally during fuzzing or as a final summary.

use crate::coverage::CoverageStats;
use crate::triage::{BugReport, CrashClass, Severity};
use crate::{FuzzStats, Issue, IssueKind};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Report format options
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    /// Plain text format
    Text,
    /// JSON format
    Json,
    /// HTML format with interactive elements
    Html,
    /// Markdown format
    Markdown,
}

impl Default for ReportFormat {
    fn default() -> Self {
        ReportFormat::Text
    }
}

impl std::str::FromStr for ReportFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" | "txt" => Ok(ReportFormat::Text),
            "json" => Ok(ReportFormat::Json),
            "html" => Ok(ReportFormat::Html),
            "markdown" | "md" => Ok(ReportFormat::Markdown),
            _ => Err(format!("Unknown report format: {}", s)),
        }
    }
}

/// Configuration for report generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportConfig {
    /// Output format
    pub format: String,
    /// Output path
    pub output_path: Option<PathBuf>,
    /// Include detailed crash information
    pub include_crashes: bool,
    /// Include coverage information
    pub include_coverage: bool,
    /// Include corpus statistics
    pub include_corpus: bool,
    /// Include performance metrics
    pub include_performance: bool,
    /// Maximum number of issues to include
    pub max_issues: usize,
}

impl Default for ReportConfig {
    fn default() -> Self {
        Self {
            format: "text".to_string(),
            output_path: None,
            include_crashes: true,
            include_coverage: true,
            include_corpus: true,
            include_performance: true,
            max_issues: 100,
        }
    }
}

/// Fuzzing campaign report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzReport {
    /// Report title
    pub title: String,
    /// Timestamp when report was generated
    pub generated_at: String,
    /// Campaign duration
    pub duration_secs: f64,
    /// Summary statistics
    pub summary: ReportSummary,
    /// Issue breakdown by category
    pub issues_by_category: HashMap<String, Vec<IssueReport>>,
    /// Coverage report
    pub coverage: Option<CoverageReport>,
    /// Corpus report
    pub corpus: Option<CorpusReport>,
    /// Performance metrics
    pub performance: Option<PerformanceReport>,
    /// Recommendations
    pub recommendations: Vec<String>,
}

/// Summary section of the report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSummary {
    /// Total iterations executed
    pub total_iterations: usize,
    /// Total unique crashes found
    pub unique_crashes: usize,
    /// Total differential bugs found
    pub differential_bugs: usize,
    /// Total timeouts
    pub timeouts: usize,
    /// Property violations
    pub property_violations: usize,
    /// Total interesting inputs discovered
    pub interesting_inputs: usize,
    /// Overall success rate
    pub success_rate: f64,
}

/// Individual issue report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueReport {
    /// Issue ID
    pub id: String,
    /// Issue severity
    pub severity: String,
    /// Issue category
    pub category: String,
    /// Short description
    pub description: String,
    /// Minimized input (if available)
    pub minimized_input: Option<String>,
    /// File path where crash was saved
    pub crash_file: Option<String>,
    /// Stack trace (if available)
    pub stack_trace: Option<String>,
    /// Time when issue was found
    pub found_at: String,
}

/// Coverage report section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageReport {
    /// Overall coverage percentage
    pub coverage_pct: f64,
    /// Branch coverage percentage
    pub branch_coverage_pct: Option<f64>,
    /// Number of unique branches hit
    pub branches_hit: usize,
    /// Total branches
    pub total_branches: usize,
    /// Coverage by source file
    pub by_file: HashMap<String, f64>,
    /// Coverage by component
    pub by_component: HashMap<String, f64>,
}

/// Corpus report section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusReport {
    /// Total corpus size
    pub size: usize,
    /// Total size in bytes
    pub total_bytes: usize,
    /// Average input size
    pub avg_size: f64,
    /// Number of seed inputs
    pub seed_count: usize,
    /// Number of generated inputs
    pub generated_count: usize,
    /// Number of mutated inputs
    pub mutated_count: usize,
    /// Top inputs by coverage contribution
    pub top_inputs: Vec<CorpusEntry>,
}

/// Corpus entry for reporting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusEntry {
    /// Input hash/ID
    pub id: String,
    /// Input size in bytes
    pub size: usize,
    /// Coverage contribution
    pub coverage_contribution: f64,
    /// Number of times selected
    pub selection_count: usize,
}

/// Performance report section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceReport {
    /// Executions per second
    pub execs_per_sec: f64,
    /// Average execution time in microseconds
    pub avg_exec_time_us: f64,
    /// Peak memory usage in MB
    pub peak_memory_mb: f64,
    /// Time spent in different phases
    pub phase_breakdown: HashMap<String, f64>,
    /// Throughput over time
    pub throughput_history: Vec<(f64, f64)>,
}

/// Report generator
pub struct ReportGenerator {
    config: ReportConfig,
}

impl ReportGenerator {
    /// Create a new report generator
    pub fn new(config: ReportConfig) -> Self {
        Self { config }
    }

    /// Generate a report from fuzzing statistics and issues
    pub fn generate(
        &self,
        stats: &FuzzStats,
        issues: &[Issue],
        coverage: Option<&CoverageStats>,
    ) -> FuzzReport {
        let summary = self.generate_summary(stats, issues);
        let issues_by_category = self.categorize_issues(issues);
        let coverage_report = coverage.map(|c| self.generate_coverage_report(c));
        let corpus_report = if self.config.include_corpus {
            Some(self.generate_corpus_report(stats))
        } else {
            None
        };
        let performance_report = if self.config.include_performance {
            Some(self.generate_performance_report(stats))
        } else {
            None
        };
        let recommendations = self.generate_recommendations(stats, issues);

        FuzzReport {
            title: "VCS Fuzzing Campaign Report".to_string(),
            generated_at: chrono::Utc::now().to_rfc3339(),
            duration_secs: stats.duration_secs,
            summary,
            issues_by_category,
            coverage: coverage_report,
            corpus: corpus_report,
            performance: performance_report,
            recommendations,
        }
    }

    /// Generate summary section
    fn generate_summary(&self, stats: &FuzzStats, issues: &[Issue]) -> ReportSummary {
        let property_violations = issues
            .iter()
            .filter(|i| {
                matches!(
                    i.kind,
                    IssueKind::TypeUnsoundness | IssueKind::VerificationFailure
                )
            })
            .count();

        ReportSummary {
            total_iterations: stats.iterations,
            unique_crashes: stats.unique_crashes,
            differential_bugs: stats.differential_bugs,
            timeouts: stats.timeouts,
            property_violations,
            interesting_inputs: stats.interesting_inputs,
            success_rate: if stats.iterations > 0 {
                1.0 - (stats.issues_found as f64 / stats.iterations as f64)
            } else {
                1.0
            },
        }
    }

    /// Categorize issues by type
    fn categorize_issues(&self, issues: &[Issue]) -> HashMap<String, Vec<IssueReport>> {
        let mut by_category: HashMap<String, Vec<IssueReport>> = HashMap::new();

        for issue in issues.iter().take(self.config.max_issues) {
            let category = match &issue.kind {
                IssueKind::Crash(_) => "crashes",
                IssueKind::DifferentialMismatch => "differential",
                IssueKind::Timeout => "timeouts",
                IssueKind::MemorySafety => "memory_safety",
                IssueKind::TypeUnsoundness => "type_errors",
                IssueKind::VerificationFailure => "verification",
            };

            let report = IssueReport {
                id: issue.id.clone(),
                severity: self.severity_for_issue(&issue.kind).to_string(),
                category: category.to_string(),
                description: issue.message.clone(),
                minimized_input: issue.minimized.clone(),
                crash_file: None,
                stack_trace: None,
                found_at: issue.timestamp.clone(),
            };

            by_category
                .entry(category.to_string())
                .or_default()
                .push(report);
        }

        by_category
    }

    /// Determine severity for an issue kind
    fn severity_for_issue(&self, kind: &IssueKind) -> &'static str {
        match kind {
            IssueKind::Crash(_) => "critical",
            IssueKind::MemorySafety => "critical",
            IssueKind::TypeUnsoundness => "high",
            IssueKind::DifferentialMismatch => "high",
            IssueKind::VerificationFailure => "medium",
            IssueKind::Timeout => "low",
        }
    }

    /// Generate coverage report section
    fn generate_coverage_report(&self, stats: &CoverageStats) -> CoverageReport {
        CoverageReport {
            coverage_pct: stats.coverage_pct,
            branch_coverage_pct: None,
            branches_hit: stats.discovered_edges,
            total_branches: stats.total_edges,
            by_file: HashMap::new(),
            by_component: HashMap::new(),
        }
    }

    /// Generate corpus report section
    fn generate_corpus_report(&self, stats: &FuzzStats) -> CorpusReport {
        CorpusReport {
            size: stats.corpus_size,
            total_bytes: 0, // Would need to calculate from corpus
            avg_size: 0.0,
            seed_count: 0,
            generated_count: 0,
            mutated_count: 0,
            top_inputs: Vec::new(),
        }
    }

    /// Generate performance report section
    fn generate_performance_report(&self, stats: &FuzzStats) -> PerformanceReport {
        let execs_per_sec = if stats.duration_secs > 0.0 {
            stats.iterations as f64 / stats.duration_secs
        } else {
            0.0
        };

        PerformanceReport {
            execs_per_sec,
            avg_exec_time_us: if stats.iterations > 0 {
                (stats.duration_secs * 1_000_000.0) / stats.iterations as f64
            } else {
                0.0
            },
            peak_memory_mb: stats.peak_memory as f64 / (1024.0 * 1024.0),
            phase_breakdown: HashMap::new(),
            throughput_history: Vec::new(),
        }
    }

    /// Generate recommendations based on findings
    fn generate_recommendations(&self, stats: &FuzzStats, issues: &[Issue]) -> Vec<String> {
        let mut recommendations = Vec::new();

        if stats.unique_crashes > 0 {
            recommendations.push(format!(
                "Found {} unique crashes. Prioritize fixing crash bugs before other issues.",
                stats.unique_crashes
            ));
        }

        if stats.differential_bugs > 0 {
            recommendations.push(format!(
                "Found {} differential bugs between Tier 0 and Tier 3. \
                 These indicate potential optimizer bugs or semantic mismatches.",
                stats.differential_bugs
            ));
        }

        if stats.timeouts > 10 {
            recommendations.push(
                "High number of timeouts detected. Consider investigating \
                 potential infinite loops or performance regressions."
                    .to_string(),
            );
        }

        if let Some(cov_pct) = stats.coverage_pct {
            if cov_pct < 50.0 {
                recommendations.push(format!(
                    "Coverage is only {:.1}%. Consider adding more seed inputs \
                     or adjusting generator weights to explore more code paths.",
                    cov_pct
                ));
            }
        }

        if stats.interesting_inputs < stats.iterations / 100 {
            recommendations.push(
                "Low rate of interesting inputs. Consider adjusting mutation \
                 strategies or adding more diverse seeds."
                    .to_string(),
            );
        }

        if recommendations.is_empty() {
            recommendations
                .push("No major issues detected. Fuzzing campaign appears healthy.".to_string());
        }

        recommendations
    }

    /// Render report as text
    pub fn render_text(&self, report: &FuzzReport) -> String {
        let mut output = String::new();

        output.push_str(&format!("=== {} ===\n\n", report.title));
        output.push_str(&format!("Generated: {}\n", report.generated_at));
        output.push_str(&format!("Duration: {:.2}s\n\n", report.duration_secs));

        output.push_str("--- Summary ---\n");
        output.push_str(&format!(
            "Total Iterations: {}\n",
            report.summary.total_iterations
        ));
        output.push_str(&format!(
            "Unique Crashes: {}\n",
            report.summary.unique_crashes
        ));
        output.push_str(&format!(
            "Differential Bugs: {}\n",
            report.summary.differential_bugs
        ));
        output.push_str(&format!("Timeouts: {}\n", report.summary.timeouts));
        output.push_str(&format!(
            "Property Violations: {}\n",
            report.summary.property_violations
        ));
        output.push_str(&format!(
            "Interesting Inputs: {}\n",
            report.summary.interesting_inputs
        ));
        output.push_str(&format!(
            "Success Rate: {:.2}%\n\n",
            report.summary.success_rate * 100.0
        ));

        if !report.issues_by_category.is_empty() {
            output.push_str("--- Issues ---\n");
            for (category, issues) in &report.issues_by_category {
                output.push_str(&format!(
                    "\n{} ({}):\n",
                    category.to_uppercase(),
                    issues.len()
                ));
                for issue in issues.iter().take(5) {
                    output.push_str(&format!(
                        "  [{}] {} - {}\n",
                        issue.severity.to_uppercase(),
                        issue.id,
                        issue.description.chars().take(60).collect::<String>()
                    ));
                }
                if issues.len() > 5 {
                    output.push_str(&format!("  ... and {} more\n", issues.len() - 5));
                }
            }
            output.push('\n');
        }

        if let Some(ref cov) = report.coverage {
            output.push_str("--- Coverage ---\n");
            output.push_str(&format!("Overall: {:.2}%\n", cov.coverage_pct));
            if let Some(branch) = cov.branch_coverage_pct {
                output.push_str(&format!("Branch: {:.2}%\n", branch));
            }
            output.push_str(&format!(
                "Branches: {}/{}\n\n",
                cov.branches_hit, cov.total_branches
            ));
        }

        if let Some(ref perf) = report.performance {
            output.push_str("--- Performance ---\n");
            output.push_str(&format!("Executions/sec: {:.1}\n", perf.execs_per_sec));
            output.push_str(&format!("Avg exec time: {:.2}us\n", perf.avg_exec_time_us));
            output.push_str(&format!("Peak memory: {:.2}MB\n\n", perf.peak_memory_mb));
        }

        if !report.recommendations.is_empty() {
            output.push_str("--- Recommendations ---\n");
            for (i, rec) in report.recommendations.iter().enumerate() {
                output.push_str(&format!("{}. {}\n", i + 1, rec));
            }
        }

        output
    }

    /// Render report as JSON
    pub fn render_json(&self, report: &FuzzReport) -> String {
        serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".to_string())
    }

    /// Render report as Markdown
    pub fn render_markdown(&self, report: &FuzzReport) -> String {
        let mut output = String::new();

        output.push_str(&format!("# {}\n\n", report.title));
        output.push_str(&format!("**Generated:** {}  \n", report.generated_at));
        output.push_str(&format!("**Duration:** {:.2}s\n\n", report.duration_secs));

        output.push_str("## Summary\n\n");
        output.push_str("| Metric | Value |\n");
        output.push_str("|--------|-------|\n");
        output.push_str(&format!(
            "| Total Iterations | {} |\n",
            report.summary.total_iterations
        ));
        output.push_str(&format!(
            "| Unique Crashes | {} |\n",
            report.summary.unique_crashes
        ));
        output.push_str(&format!(
            "| Differential Bugs | {} |\n",
            report.summary.differential_bugs
        ));
        output.push_str(&format!("| Timeouts | {} |\n", report.summary.timeouts));
        output.push_str(&format!(
            "| Property Violations | {} |\n",
            report.summary.property_violations
        ));
        output.push_str(&format!(
            "| Interesting Inputs | {} |\n",
            report.summary.interesting_inputs
        ));
        output.push_str(&format!(
            "| Success Rate | {:.2}% |\n\n",
            report.summary.success_rate * 100.0
        ));

        if !report.issues_by_category.is_empty() {
            output.push_str("## Issues\n\n");
            for (category, issues) in &report.issues_by_category {
                output.push_str(&format!("### {} ({})\n\n", category, issues.len()));
                output.push_str("| ID | Severity | Description |\n");
                output.push_str("|----|----------|-------------|\n");
                for issue in issues.iter().take(10) {
                    let desc: String = issue.description.chars().take(50).collect();
                    output.push_str(&format!(
                        "| {} | {} | {}... |\n",
                        issue.id, issue.severity, desc
                    ));
                }
                output.push('\n');
            }
        }

        if let Some(ref cov) = report.coverage {
            output.push_str("## Coverage\n\n");
            output.push_str(&format!("- **Overall:** {:.2}%\n", cov.coverage_pct));
            if let Some(branch) = cov.branch_coverage_pct {
                output.push_str(&format!("- **Branch:** {:.2}%\n", branch));
            }
            output.push_str(&format!(
                "- **Branches Hit:** {}/{}\n\n",
                cov.branches_hit, cov.total_branches
            ));
        }

        if !report.recommendations.is_empty() {
            output.push_str("## Recommendations\n\n");
            for rec in &report.recommendations {
                output.push_str(&format!("- {}\n", rec));
            }
        }

        output
    }

    /// Render report as HTML
    pub fn render_html(&self, report: &FuzzReport) -> String {
        let mut output = String::new();

        output.push_str("<!DOCTYPE html>\n<html>\n<head>\n");
        output.push_str(&format!("<title>{}</title>\n", report.title));
        output.push_str(HTML_STYLES);
        output.push_str("</head>\n<body>\n");

        output.push_str(&format!("<h1>{}</h1>\n", report.title));
        output.push_str(&format!(
            "<p class=\"metadata\">Generated: {} | Duration: {:.2}s</p>\n",
            report.generated_at, report.duration_secs
        ));

        // Summary section
        output.push_str("<div class=\"section\">\n<h2>Summary</h2>\n");
        output.push_str("<div class=\"stats-grid\">\n");
        output.push_str(&format!(
            "<div class=\"stat-card\"><span class=\"stat-value\">{}</span><span class=\"stat-label\">Iterations</span></div>\n",
            report.summary.total_iterations
        ));
        output.push_str(&format!(
            "<div class=\"stat-card critical\"><span class=\"stat-value\">{}</span><span class=\"stat-label\">Crashes</span></div>\n",
            report.summary.unique_crashes
        ));
        output.push_str(&format!(
            "<div class=\"stat-card warning\"><span class=\"stat-value\">{}</span><span class=\"stat-label\">Differential Bugs</span></div>\n",
            report.summary.differential_bugs
        ));
        output.push_str(&format!(
            "<div class=\"stat-card\"><span class=\"stat-value\">{:.1}%</span><span class=\"stat-label\">Success Rate</span></div>\n",
            report.summary.success_rate * 100.0
        ));
        output.push_str("</div>\n</div>\n");

        // Issues section
        if !report.issues_by_category.is_empty() {
            output.push_str("<div class=\"section\">\n<h2>Issues</h2>\n");
            for (category, issues) in &report.issues_by_category {
                output.push_str(&format!("<h3>{} ({})</h3>\n", category, issues.len()));
                output.push_str(
                    "<table>\n<tr><th>ID</th><th>Severity</th><th>Description</th></tr>\n",
                );
                for issue in issues.iter().take(10) {
                    let severity_class = match issue.severity.as_str() {
                        "critical" => "severity-critical",
                        "high" => "severity-high",
                        "medium" => "severity-medium",
                        _ => "severity-low",
                    };
                    let desc: String = issue.description.chars().take(80).collect();
                    output.push_str(&format!(
                        "<tr><td>{}</td><td class=\"{}\">{}</td><td>{}</td></tr>\n",
                        issue.id,
                        severity_class,
                        issue.severity.to_uppercase(),
                        desc
                    ));
                }
                output.push_str("</table>\n");
            }
            output.push_str("</div>\n");
        }

        // Coverage section
        if let Some(ref cov) = report.coverage {
            output.push_str("<div class=\"section\">\n<h2>Coverage</h2>\n");
            output.push_str(&format!(
                "<div class=\"progress-bar\"><div class=\"progress-fill\" style=\"width: {}%\"></div></div>\n",
                cov.coverage_pct
            ));
            output.push_str(&format!(
                "<p>Overall Coverage: {:.2}%</p>\n",
                cov.coverage_pct
            ));
            output.push_str(&format!(
                "<p>Branches Hit: {} / {}</p>\n",
                cov.branches_hit, cov.total_branches
            ));
            output.push_str("</div>\n");
        }

        // Recommendations section
        if !report.recommendations.is_empty() {
            output.push_str("<div class=\"section\">\n<h2>Recommendations</h2>\n<ul>\n");
            for rec in &report.recommendations {
                output.push_str(&format!("<li>{}</li>\n", rec));
            }
            output.push_str("</ul>\n</div>\n");
        }

        output.push_str("</body>\n</html>\n");
        output
    }

    /// Save report to file
    pub fn save(&self, report: &FuzzReport, path: &Path) -> std::io::Result<()> {
        let format = path
            .extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| ext.parse::<ReportFormat>().ok())
            .unwrap_or(ReportFormat::Text);

        let content = match format {
            ReportFormat::Text => self.render_text(report),
            ReportFormat::Json => self.render_json(report),
            ReportFormat::Html => self.render_html(report),
            ReportFormat::Markdown => self.render_markdown(report),
        };

        std::fs::write(path, content)
    }
}

const HTML_STYLES: &str = r#"
<style>
body {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    line-height: 1.6;
    max-width: 1200px;
    margin: 0 auto;
    padding: 20px;
    background: #f5f5f5;
}
h1 { color: #333; border-bottom: 2px solid #0066cc; padding-bottom: 10px; }
h2 { color: #555; margin-top: 30px; }
.section { background: white; padding: 20px; margin: 20px 0; border-radius: 8px; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }
.metadata { color: #666; font-size: 0.9em; }
.stats-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 15px; }
.stat-card { background: #f8f9fa; padding: 20px; text-align: center; border-radius: 8px; }
.stat-card.critical { background: #ffe6e6; }
.stat-card.warning { background: #fff3cd; }
.stat-value { display: block; font-size: 2em; font-weight: bold; color: #333; }
.stat-label { color: #666; font-size: 0.9em; }
table { width: 100%; border-collapse: collapse; margin: 10px 0; }
th, td { padding: 12px; text-align: left; border-bottom: 1px solid #ddd; }
th { background: #f8f9fa; font-weight: 600; }
.severity-critical { color: #dc3545; font-weight: bold; }
.severity-high { color: #fd7e14; font-weight: bold; }
.severity-medium { color: #ffc107; }
.severity-low { color: #28a745; }
.progress-bar { height: 30px; background: #e9ecef; border-radius: 15px; overflow: hidden; }
.progress-fill { height: 100%; background: linear-gradient(90deg, #28a745, #20c997); }
ul { padding-left: 20px; }
li { margin: 8px 0; }
</style>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_report_format_parsing() {
        assert_eq!("text".parse::<ReportFormat>().unwrap(), ReportFormat::Text);
        assert_eq!("json".parse::<ReportFormat>().unwrap(), ReportFormat::Json);
        assert_eq!("html".parse::<ReportFormat>().unwrap(), ReportFormat::Html);
        assert_eq!(
            "md".parse::<ReportFormat>().unwrap(),
            ReportFormat::Markdown
        );
    }

    #[test]
    fn test_generate_report() {
        let config = ReportConfig::default();
        let generator = ReportGenerator::new(config);

        let stats = FuzzStats {
            iterations: 1000,
            duration_secs: 60.0,
            unique_crashes: 2,
            differential_bugs: 1,
            timeouts: 5,
            interesting_inputs: 50,
            ..Default::default()
        };

        let issues = vec![Issue::new(
            "fn main() { panic!() }",
            IssueKind::Crash(crate::CrashKind::Panic),
            "Test crash",
        )];

        let report = generator.generate(&stats, &issues, None);

        assert_eq!(report.summary.total_iterations, 1000);
        assert_eq!(report.summary.unique_crashes, 2);
        assert!(!report.issues_by_category.is_empty());
    }

    #[test]
    fn test_render_text() {
        let config = ReportConfig::default();
        let generator = ReportGenerator::new(config);

        let stats = FuzzStats::default();
        let report = generator.generate(&stats, &[], None);
        let text = generator.render_text(&report);

        assert!(text.contains("Summary"));
        assert!(text.contains("Recommendations"));
    }

    #[test]
    fn test_render_json() {
        let config = ReportConfig::default();
        let generator = ReportGenerator::new(config);

        let stats = FuzzStats::default();
        let report = generator.generate(&stats, &[], None);
        let json = generator.render_json(&report);

        // Should be valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("title").is_some());
    }

    #[test]
    fn test_render_html() {
        let config = ReportConfig::default();
        let generator = ReportGenerator::new(config);

        let stats = FuzzStats::default();
        let report = generator.generate(&stats, &[], None);
        let html = generator.render_html(&report);

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("</html>"));
    }
}
