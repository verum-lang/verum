//! Stability report generation.
//!
//! Generates comprehensive reports on proof stability in various formats:
//! - Console output with colors
//! - JSON for machine consumption
//! - HTML for visual inspection
//! - Markdown for documentation

use crate::{
    metrics::StabilityMetrics,
    regression::RegressionReport,
};
use chrono::{DateTime, Utc};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::time::Duration;
use verum_common::Text;

/// Output format for stability reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StabilityReportFormat {
    /// Console output with ANSI colors
    Console,
    /// JSON format
    Json,
    /// HTML format
    Html,
    /// Markdown format
    Markdown,
}

impl StabilityReportFormat {
    /// Parse format from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "console" | "text" => Some(Self::Console),
            "json" => Some(Self::Json),
            "html" => Some(Self::Html),
            "markdown" | "md" => Some(Self::Markdown),
            _ => None,
        }
    }
}

/// Complete stability report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StabilityReport {
    /// Report title
    pub title: Text,
    /// Generation timestamp
    pub timestamp: DateTime<Utc>,
    /// Compiler version
    pub compiler_version: Option<Text>,
    /// Solver used
    pub solver: Text,
    /// Solver version
    pub solver_version: Option<Text>,
    /// Overall stability metrics
    pub metrics: StabilityMetrics,
    /// Regression report (if available)
    pub regressions: Option<RegressionReport>,
    /// Configuration summary
    pub config_summary: HashMap<Text, Text>,
    /// Test execution duration
    pub execution_time: Duration,
    /// Exit code (0 = pass, 1 = fail)
    pub exit_code: i32,
}

impl StabilityReport {
    /// Create a new report.
    pub fn new(metrics: StabilityMetrics) -> Self {
        Self {
            title: "Proof Stability Report".to_string().into(),
            timestamp: Utc::now(),
            compiler_version: None,
            solver: "z3".to_string().into(),
            solver_version: None,
            metrics,
            regressions: None,
            config_summary: HashMap::new(),
            execution_time: Duration::ZERO,
            exit_code: 0,
        }
    }

    /// Set the title.
    pub fn with_title(mut self, title: Text) -> Self {
        self.title = title;
        self
    }

    /// Set the compiler version.
    pub fn with_compiler_version(mut self, version: Text) -> Self {
        self.compiler_version = Some(version);
        self
    }

    /// Set the solver information.
    pub fn with_solver(mut self, solver: Text, version: Option<Text>) -> Self {
        self.solver = solver;
        self.solver_version = version;
        self
    }

    /// Add regression report.
    pub fn with_regressions(mut self, regressions: RegressionReport) -> Self {
        self.regressions = Some(regressions);
        self
    }

    /// Set execution time.
    pub fn with_execution_time(mut self, time: Duration) -> Self {
        self.execution_time = time;
        self
    }

    /// Set exit code based on thresholds.
    pub fn compute_exit_code(&mut self, stability_threshold: f64, allow_flaky: bool) {
        if self.metrics.overall_stability < stability_threshold {
            self.exit_code = 1;
        } else if !allow_flaky && self.metrics.flaky_count > 0 {
            self.exit_code = 1;
        } else if let Some(ref reg) = self.regressions {
            if reg.total_regressions > 0 {
                self.exit_code = 1;
            }
        }
    }

    /// Generate report to a writer.
    pub fn generate<W: Write>(
        &self,
        writer: &mut W,
        format: StabilityReportFormat,
    ) -> std::io::Result<()> {
        match format {
            StabilityReportFormat::Console => self.generate_console(writer),
            StabilityReportFormat::Json => self.generate_json(writer),
            StabilityReportFormat::Html => self.generate_html(writer),
            StabilityReportFormat::Markdown => self.generate_markdown(writer),
        }
    }

    /// Generate console output.
    fn generate_console<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // Header
        writeln!(writer)?;
        writeln!(writer, "{}", "=".repeat(60).dimmed())?;
        writeln!(writer, "{}", self.title.bold())?;
        writeln!(writer, "{}", "=".repeat(60).dimmed())?;
        writeln!(writer)?;

        // Summary
        let status_color = if self.metrics.overall_stability >= 95.0 {
            "green"
        } else if self.metrics.overall_stability >= 80.0 {
            "yellow"
        } else {
            "red"
        };

        writeln!(
            writer,
            "Overall Stability: {}",
            match status_color {
                "green" => format!("{:.1}%", self.metrics.overall_stability).green(),
                "yellow" => format!("{:.1}%", self.metrics.overall_stability).yellow(),
                _ => format!("{:.1}%", self.metrics.overall_stability).red(),
            }
        )?;
        writeln!(writer)?;

        // Breakdown
        writeln!(writer, "{}", "Proof Statistics:".bold())?;
        writeln!(
            writer,
            "  Total:      {}",
            self.metrics.total_proofs.to_string().cyan()
        )?;
        writeln!(
            writer,
            "  Stable:     {}",
            self.metrics.stable_count.to_string().green()
        )?;
        writeln!(
            writer,
            "  Flaky:      {}",
            if self.metrics.flaky_count > 0 {
                self.metrics.flaky_count.to_string().red()
            } else {
                self.metrics.flaky_count.to_string().normal()
            }
        )?;
        writeln!(
            writer,
            "  Timeout:    {}",
            if self.metrics.timeout_unstable_count > 0 {
                self.metrics.timeout_unstable_count.to_string().yellow()
            } else {
                self.metrics.timeout_unstable_count.to_string().normal()
            }
        )?;
        writeln!(
            writer,
            "  Unknown:    {}",
            self.metrics.unknown_count.to_string().dimmed()
        )?;
        writeln!(writer)?;

        // Category breakdown
        if !self.metrics.by_category.is_empty() {
            writeln!(writer, "{}", "By Category:".bold())?;
            for (category, cat_metrics) in &self.metrics.by_category {
                let stability = cat_metrics.stability_percentage;
                let stability_str = format!("{:.1}%", stability);
                let colored_stability = if stability >= 95.0 {
                    stability_str.green()
                } else if stability >= 80.0 {
                    stability_str.yellow()
                } else {
                    stability_str.red()
                };
                writeln!(
                    writer,
                    "  {:12} {:>3} proofs, {} stable",
                    format!("{}:", category),
                    cat_metrics.total,
                    colored_stability
                )?;
            }
            writeln!(writer)?;
        }

        // Flaky proofs
        if !self.metrics.flaky_proofs.is_empty() {
            writeln!(writer, "{}", "Flaky Proofs:".bold().red())?;
            for flaky in &self.metrics.flaky_proofs {
                writeln!(
                    writer,
                    "  {} {}",
                    "!".red().bold(),
                    flaky.proof_id.source_path
                )?;
                writeln!(
                    writer,
                    "    {} ({:.1}% stable)",
                    flaky.status.to_string().yellow(),
                    flaky.stability_percentage
                )?;
                writeln!(writer, "    Outcomes: {}", flaky.outcome_distribution)?;
                writeln!(writer, "    Action: {}", flaky.suggested_action.dimmed())?;
            }
            writeln!(writer)?;
        }

        // Regressions
        if let Some(ref regressions) = self.regressions {
            if regressions.has_regressions() {
                writeln!(writer, "{}", "Regressions Detected:".bold().red())?;
                for regression in regressions.most_severe(5) {
                    writeln!(
                        writer,
                        "  {} [severity {}] {}",
                        regression.regression_type.to_string().red(),
                        regression.severity,
                        regression.proof_id.source_path
                    )?;
                    writeln!(writer, "    {}", regression.message.dimmed())?;
                }
                writeln!(writer)?;
            }
        }

        // Footer
        writeln!(writer, "{}", "-".repeat(60).dimmed())?;
        writeln!(
            writer,
            "Execution time: {:.2}s",
            self.execution_time.as_secs_f64()
        )?;
        if let Some(ref v) = self.compiler_version {
            writeln!(writer, "Compiler: {}", v)?;
        }
        writeln!(
            writer,
            "Solver: {} {}",
            self.solver,
            self.solver_version.as_deref().unwrap_or("unknown")
        )?;
        writeln!(
            writer,
            "Timestamp: {}",
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        )?;
        writeln!(writer)?;

        // Exit code indicator
        if self.exit_code == 0 {
            writeln!(writer, "{}", "PASSED".green().bold())?;
        } else {
            writeln!(writer, "{}", "FAILED".red().bold())?;
        }

        Ok(())
    }

    /// Generate JSON output.
    fn generate_json<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        writeln!(writer, "{}", json)
    }

    /// Generate HTML output.
    fn generate_html<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        writeln!(writer, "<!DOCTYPE html>")?;
        writeln!(writer, "<html><head>")?;
        writeln!(writer, "<title>{}</title>", self.title)?;
        writeln!(writer, "<style>")?;
        writeln!(
            writer,
            r#"
            body {{ font-family: -apple-system, BlinkMacSystemFont, sans-serif; margin: 40px; }}
            h1 {{ color: #333; }}
            .stable {{ color: #28a745; }}
            .flaky {{ color: #dc3545; }}
            .unknown {{ color: #6c757d; }}
            table {{ border-collapse: collapse; width: 100%; margin: 20px 0; }}
            th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
            th {{ background-color: #f2f2f2; }}
            .summary {{ background: #f8f9fa; padding: 20px; border-radius: 8px; margin: 20px 0; }}
            .metric {{ font-size: 2em; font-weight: bold; }}
        "#
        )?;
        writeln!(writer, "</style></head><body>")?;

        writeln!(writer, "<h1>{}</h1>", self.title)?;

        // Summary box
        let stability_class = if self.metrics.overall_stability >= 95.0 {
            "stable"
        } else if self.metrics.overall_stability >= 80.0 {
            "unknown"
        } else {
            "flaky"
        };
        writeln!(writer, "<div class='summary'>")?;
        writeln!(
            writer,
            "<div class='metric {}'>{:.1}%</div>",
            stability_class, self.metrics.overall_stability
        )?;
        writeln!(writer, "<div>Overall Proof Stability</div>")?;
        writeln!(
            writer,
            "<div>{} stable / {} total proofs</div>",
            self.metrics.stable_count, self.metrics.total_proofs
        )?;
        writeln!(writer, "</div>")?;

        // Category table
        writeln!(writer, "<h2>By Category</h2>")?;
        writeln!(
            writer,
            "<table><tr><th>Category</th><th>Total</th><th>Stable</th><th>Flaky</th><th>Stability</th></tr>"
        )?;
        for (category, cat_metrics) in &self.metrics.by_category {
            let class = if cat_metrics.stability_percentage >= 95.0 {
                "stable"
            } else if cat_metrics.stability_percentage >= 80.0 {
                "unknown"
            } else {
                "flaky"
            };
            writeln!(
                writer,
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class='{}'>{:.1}%</td></tr>",
                category,
                cat_metrics.total,
                cat_metrics.stable,
                cat_metrics.flaky,
                class,
                cat_metrics.stability_percentage
            )?;
        }
        writeln!(writer, "</table>")?;

        // Flaky proofs
        if !self.metrics.flaky_proofs.is_empty() {
            writeln!(writer, "<h2>Flaky Proofs</h2>")?;
            writeln!(
                writer,
                "<table><tr><th>Proof</th><th>Category</th><th>Stability</th><th>Outcomes</th><th>Action</th></tr>"
            )?;
            for flaky in &self.metrics.flaky_proofs {
                writeln!(
                    writer,
                    "<tr><td>{}</td><td>{}</td><td class='flaky'>{:.1}%</td><td>{}</td><td>{}</td></tr>",
                    flaky.proof_id.source_path,
                    flaky.category,
                    flaky.stability_percentage,
                    flaky.outcome_distribution,
                    flaky.suggested_action
                )?;
            }
            writeln!(writer, "</table>")?;
        }

        // Footer
        writeln!(writer, "<hr>")?;
        writeln!(
            writer,
            "<p>Generated: {}</p>",
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        )?;
        if let Some(ref v) = self.compiler_version {
            writeln!(writer, "<p>Compiler: {}</p>", v)?;
        }

        writeln!(writer, "</body></html>")?;
        Ok(())
    }

    /// Generate Markdown output.
    fn generate_markdown<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        writeln!(writer, "# {}", self.title)?;
        writeln!(writer)?;
        writeln!(
            writer,
            "**Generated:** {}",
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        )?;
        if let Some(ref v) = self.compiler_version {
            writeln!(writer, "**Compiler:** {}", v)?;
        }
        writeln!(
            writer,
            "**Solver:** {} {}",
            self.solver,
            self.solver_version.as_deref().unwrap_or("unknown")
        )?;
        writeln!(writer)?;

        writeln!(writer, "## Summary")?;
        writeln!(writer)?;
        writeln!(writer, "| Metric | Value |")?;
        writeln!(writer, "|--------|-------|")?;
        writeln!(
            writer,
            "| Overall Stability | {:.1}% |",
            self.metrics.overall_stability
        )?;
        writeln!(writer, "| Total Proofs | {} |", self.metrics.total_proofs)?;
        writeln!(writer, "| Stable | {} |", self.metrics.stable_count)?;
        writeln!(writer, "| Flaky | {} |", self.metrics.flaky_count)?;
        writeln!(
            writer,
            "| Timeout Unstable | {} |",
            self.metrics.timeout_unstable_count
        )?;
        writeln!(writer, "| Unknown | {} |", self.metrics.unknown_count)?;
        writeln!(writer)?;

        writeln!(writer, "## By Category")?;
        writeln!(writer)?;
        writeln!(writer, "| Category | Total | Stable | Flaky | Stability |")?;
        writeln!(writer, "|----------|-------|--------|-------|-----------|")?;
        for (category, cat_metrics) in &self.metrics.by_category {
            writeln!(
                writer,
                "| {} | {} | {} | {} | {:.1}% |",
                category,
                cat_metrics.total,
                cat_metrics.stable,
                cat_metrics.flaky,
                cat_metrics.stability_percentage
            )?;
        }
        writeln!(writer)?;

        if !self.metrics.flaky_proofs.is_empty() {
            writeln!(writer, "## Flaky Proofs")?;
            writeln!(writer)?;
            for flaky in &self.metrics.flaky_proofs {
                writeln!(writer, "### `{}`", flaky.proof_id.source_path)?;
                writeln!(writer)?;
                writeln!(writer, "- **Category:** {}", flaky.category)?;
                writeln!(
                    writer,
                    "- **Stability:** {:.1}%",
                    flaky.stability_percentage
                )?;
                writeln!(writer, "- **Outcomes:** {}", flaky.outcome_distribution)?;
                writeln!(writer, "- **Action:** {}", flaky.suggested_action)?;
                writeln!(writer)?;
            }
        }

        if let Some(ref regressions) = self.regressions {
            if regressions.has_regressions() {
                writeln!(writer, "## Regressions")?;
                writeln!(writer)?;
                writeln!(writer, "| Type | Proof | Severity | Message |")?;
                writeln!(writer, "|------|-------|----------|---------|")?;
                for regression in &regressions.regressions {
                    if regression.is_regression() {
                        writeln!(
                            writer,
                            "| {} | {} | {} | {} |",
                            regression.regression_type,
                            regression.proof_id.source_path,
                            regression.severity,
                            regression.message
                        )?;
                    }
                }
                writeln!(writer)?;
            }
        }

        writeln!(writer, "---")?;
        writeln!(
            writer,
            "*Execution time: {:.2}s*",
            self.execution_time.as_secs_f64()
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_from_str() {
        assert_eq!(
            StabilityReportFormat::from_str("console"),
            Some(StabilityReportFormat::Console)
        );
        assert_eq!(
            StabilityReportFormat::from_str("json"),
            Some(StabilityReportFormat::Json)
        );
        assert_eq!(
            StabilityReportFormat::from_str("md"),
            Some(StabilityReportFormat::Markdown)
        );
    }

    #[test]
    fn test_report_json_generation() {
        let metrics = StabilityMetrics::default();
        let report = StabilityReport::new(metrics);

        let mut output = Vec::new();
        report
            .generate(&mut output, StabilityReportFormat::Json)
            .unwrap();

        let json = String::from_utf8(output).unwrap();
        assert!(json.contains("overall_stability"));
    }

    #[test]
    fn test_report_exit_code() {
        let mut metrics = StabilityMetrics::default();
        metrics.overall_stability = 90.0;
        metrics.flaky_count = 2;

        let mut report = StabilityReport::new(metrics);
        report.compute_exit_code(95.0, false);
        assert_eq!(report.exit_code, 1);

        let mut metrics2 = StabilityMetrics::default();
        metrics2.overall_stability = 98.0;
        metrics2.flaky_count = 0;

        let mut report2 = StabilityReport::new(metrics2);
        report2.compute_exit_code(95.0, false);
        assert_eq!(report2.exit_code, 0);
    }
}
