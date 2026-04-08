//! Divergence Report - Generate comprehensive reports of tier divergences
//!
//! This module provides facilities for generating detailed reports about
//! divergences found during differential testing. Reports can be generated
//! in multiple formats including Markdown, HTML, JSON, and SARIF.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    OracleResult, OracleSummary, Tier, TierDivergence, DivergenceCategory,
};

/// Report format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportFormat {
    /// Plain text
    Text,
    /// Markdown
    Markdown,
    /// HTML
    Html,
    /// JSON
    Json,
    /// SARIF (for IDE integration)
    Sarif,
}

impl Default for ReportFormat {
    fn default() -> Self {
        ReportFormat::Markdown
    }
}

impl std::fmt::Display for ReportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReportFormat::Text => write!(f, "text"),
            ReportFormat::Markdown => write!(f, "markdown"),
            ReportFormat::Html => write!(f, "html"),
            ReportFormat::Json => write!(f, "json"),
            ReportFormat::Sarif => write!(f, "sarif"),
        }
    }
}

/// A section in a report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSection {
    /// Section title
    pub title: String,
    /// Section content
    pub content: String,
    /// Subsections
    pub subsections: Vec<ReportSection>,
    /// Code blocks in this section
    pub code_blocks: Vec<CodeBlock>,
    /// Tables in this section
    pub tables: Vec<ReportTable>,
}

impl ReportSection {
    /// Create a new section
    pub fn new(title: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            content: content.into(),
            subsections: Vec::new(),
            code_blocks: Vec::new(),
            tables: Vec::new(),
        }
    }

    /// Add a subsection
    pub fn with_subsection(mut self, section: ReportSection) -> Self {
        self.subsections.push(section);
        self
    }

    /// Add a code block
    pub fn with_code(mut self, language: impl Into<String>, code: impl Into<String>) -> Self {
        self.code_blocks.push(CodeBlock {
            language: language.into(),
            code: code.into(),
        });
        self
    }

    /// Add a table
    pub fn with_table(mut self, table: ReportTable) -> Self {
        self.tables.push(table);
        self
    }
}

/// A code block in a report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBlock {
    /// Language for syntax highlighting
    pub language: String,
    /// Code content
    pub code: String,
}

/// A table in a report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportTable {
    /// Table headers
    pub headers: Vec<String>,
    /// Table rows
    pub rows: Vec<Vec<String>>,
}

impl ReportTable {
    /// Create a new table
    pub fn new(headers: Vec<String>) -> Self {
        Self {
            headers,
            rows: Vec::new(),
        }
    }

    /// Add a row
    pub fn add_row(&mut self, row: Vec<String>) {
        self.rows.push(row);
    }
}

/// Complete divergence report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DivergenceReport {
    /// Report ID
    pub id: String,
    /// Report title
    pub title: String,
    /// Generation timestamp
    pub timestamp: DateTime<Utc>,
    /// Summary statistics
    pub summary: OracleSummary,
    /// Detailed results
    pub results: Vec<OracleResult>,
    /// Report sections
    pub sections: Vec<ReportSection>,
    /// Metadata
    pub metadata: ReportMetadata,
}

/// Report metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportMetadata {
    /// Version of the differential testing tool
    pub tool_version: String,
    /// Tiers that were tested
    pub tiers_tested: Vec<Tier>,
    /// Test directory
    pub test_directory: Option<PathBuf>,
    /// Total execution time
    pub total_duration_ms: u64,
    /// Environment information
    pub environment: HashMap<String, String>,
}

impl Default for ReportMetadata {
    fn default() -> Self {
        Self {
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            tiers_tested: vec![Tier::Interpreter, Tier::Aot],
            test_directory: None,
            total_duration_ms: 0,
            environment: HashMap::new(),
        }
    }
}

/// Builder for divergence reports
pub struct ReportBuilder {
    format: ReportFormat,
    output_dir: PathBuf,
    include_passing: bool,
    include_performance: bool,
    include_source: bool,
    max_divergences_per_test: usize,
    context_lines: usize,
}

impl ReportBuilder {
    /// Create a new report builder
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            format: ReportFormat::Markdown,
            output_dir,
            include_passing: false,
            include_performance: true,
            include_source: true,
            max_divergences_per_test: 10,
            context_lines: 3,
        }
    }

    /// Set the report format
    pub fn format(mut self, format: ReportFormat) -> Self {
        self.format = format;
        self
    }

    /// Include passing tests in the report
    pub fn include_passing(mut self, include: bool) -> Self {
        self.include_passing = include;
        self
    }

    /// Include performance metrics
    pub fn include_performance(mut self, include: bool) -> Self {
        self.include_performance = include;
        self
    }

    /// Include source code
    pub fn include_source(mut self, include: bool) -> Self {
        self.include_source = include;
        self
    }

    /// Set max divergences per test
    pub fn max_divergences(mut self, max: usize) -> Self {
        self.max_divergences_per_test = max;
        self
    }

    /// Build the report from results
    pub fn build(&self, results: &[OracleResult], summary: &OracleSummary) -> Result<DivergenceReport> {
        let id = format!("{}", Utc::now().format("%Y%m%d_%H%M%S"));
        let timestamp = Utc::now();

        let mut sections = Vec::new();

        // Summary section
        sections.push(self.build_summary_section(summary));

        // Divergences section
        if summary.failed > 0 {
            sections.push(self.build_divergences_section(results));
        }

        // Category breakdown
        if !summary.divergence_counts.is_empty() {
            sections.push(self.build_category_section(summary));
        }

        // Performance section
        if self.include_performance {
            sections.push(self.build_performance_section(results));
        }

        // Passing tests (if enabled)
        if self.include_passing {
            let passing: Vec<_> = results.iter().filter(|r| r.success).collect();
            if !passing.is_empty() {
                sections.push(self.build_passing_section(&passing));
            }
        }

        Ok(DivergenceReport {
            id,
            title: format!("Differential Testing Report - {}", timestamp.format("%Y-%m-%d %H:%M:%S")),
            timestamp,
            summary: summary.clone(),
            results: results.to_vec(),
            sections,
            metadata: ReportMetadata {
                total_duration_ms: summary.duration.as_millis() as u64,
                ..Default::default()
            },
        })
    }

    /// Build summary section
    fn build_summary_section(&self, summary: &OracleSummary) -> ReportSection {
        let mut table = ReportTable::new(vec![
            "Metric".to_string(),
            "Value".to_string(),
        ]);

        table.add_row(vec!["Total Tests".to_string(), summary.total.to_string()]);
        table.add_row(vec![
            "Passed".to_string(),
            format!("{} ({:.1}%)", summary.passed, 100.0 * summary.passed as f64 / summary.total.max(1) as f64),
        ]);
        table.add_row(vec![
            "Failed".to_string(),
            format!("{} ({:.1}%)", summary.failed, 100.0 * summary.failed as f64 / summary.total.max(1) as f64),
        ]);
        table.add_row(vec![
            "Duration".to_string(),
            format!("{:?}", summary.duration),
        ]);

        ReportSection::new("Summary", "Overview of differential test results")
            .with_table(table)
    }

    /// Build divergences section
    fn build_divergences_section(&self, results: &[OracleResult]) -> ReportSection {
        let mut section = ReportSection::new(
            "Divergences",
            "Detailed information about failed tests",
        );

        for result in results.iter().filter(|r| !r.success) {
            let mut test_section = ReportSection::new(
                &result.test_name,
                format!("File: {}", result.test_path.display()),
            );

            for (i, div) in result.divergences.iter().take(self.max_divergences_per_test).enumerate() {
                let div_section = self.build_divergence_detail(div, i + 1);
                test_section.subsections.push(div_section);
            }

            if result.divergences.len() > self.max_divergences_per_test {
                test_section.subsections.push(ReportSection::new(
                    "...",
                    format!(
                        "({} more divergences not shown)",
                        result.divergences.len() - self.max_divergences_per_test
                    ),
                ));
            }

            section.subsections.push(test_section);
        }

        section
    }

    /// Build a divergence detail section
    fn build_divergence_detail(&self, div: &TierDivergence, index: usize) -> ReportSection {
        let mut section = ReportSection::new(
            format!("Divergence #{} - {}", index, div.category),
            &div.summary,
        );

        let mut table = ReportTable::new(vec![
            "Aspect".to_string(),
            "Expected".to_string(),
            "Actual".to_string(),
        ]);

        table.add_row(vec![
            "Tier".to_string(),
            format!("{}", div.tier1),
            format!("{}", div.tier2),
        ]);

        for detail in &div.details {
            table.add_row(vec![
                detail.location.clone(),
                truncate(&detail.expected, 50),
                truncate(&detail.actual, 50),
            ]);
        }

        section = section.with_table(table);

        // Add context if available
        if let Some(detail) = div.details.first() {
            if !detail.context.is_empty() {
                section = section.with_code("text", detail.context.join("\n"));
            }
        }

        // Add suggested fix if available
        if let Some(fix) = &div.suggested_fix {
            section.content.push_str(&format!("\n\n**Suggested Fix:** {}", fix));
        }

        section
    }

    /// Build category breakdown section
    fn build_category_section(&self, summary: &OracleSummary) -> ReportSection {
        let mut table = ReportTable::new(vec![
            "Category".to_string(),
            "Count".to_string(),
            "Percentage".to_string(),
        ]);

        let total_divs: usize = summary.divergence_counts.values().sum();

        for (category, count) in &summary.divergence_counts {
            table.add_row(vec![
                format!("{}", category),
                count.to_string(),
                format!("{:.1}%", 100.0 * *count as f64 / total_divs.max(1) as f64),
            ]);
        }

        ReportSection::new("Divergence Categories", "Breakdown by divergence type")
            .with_table(table)
    }

    /// Build performance section
    fn build_performance_section(&self, results: &[OracleResult]) -> ReportSection {
        let mut table = ReportTable::new(vec![
            "Test".to_string(),
            "Tier 0 (ms)".to_string(),
            "Tier 3 (ms)".to_string(),
            "Ratio".to_string(),
        ]);

        for result in results {
            let tier0 = result.tier_results.get(&Tier::Interpreter);
            let tier3 = result.tier_results.get(&Tier::Aot);

            if let (Some(t0), Some(t3)) = (tier0, tier3) {
                let ratio = if t0.duration_ms > 0 {
                    t3.duration_ms as f64 / t0.duration_ms as f64
                } else {
                    0.0
                };

                table.add_row(vec![
                    result.test_name.clone(),
                    t0.duration_ms.to_string(),
                    t3.duration_ms.to_string(),
                    format!("{:.2}x", ratio),
                ]);
            }
        }

        ReportSection::new("Performance Comparison", "Execution time by tier")
            .with_table(table)
    }

    /// Build passing tests section
    fn build_passing_section(&self, results: &[&OracleResult]) -> ReportSection {
        let mut table = ReportTable::new(vec![
            "Test".to_string(),
            "Duration (ms)".to_string(),
            "Level".to_string(),
        ]);

        for result in results {
            table.add_row(vec![
                result.test_name.clone(),
                result.duration.as_millis().to_string(),
                result.annotations.level.clone().unwrap_or_else(|| "L1".to_string()),
            ]);
        }

        ReportSection::new("Passing Tests", "Tests where all tiers agree")
            .with_table(table)
    }

    /// Write report to file
    pub fn write(&self, report: &DivergenceReport) -> Result<PathBuf> {
        fs::create_dir_all(&self.output_dir)?;

        let filename = format!(
            "differential_report_{}.{}",
            report.id,
            self.format_extension()
        );
        let path = self.output_dir.join(&filename);

        let content = match self.format {
            ReportFormat::Text => self.render_text(report),
            ReportFormat::Markdown => self.render_markdown(report),
            ReportFormat::Html => self.render_html(report),
            ReportFormat::Json => self.render_json(report)?,
            ReportFormat::Sarif => self.render_sarif(report)?,
        };

        let mut file = File::create(&path)?;
        file.write_all(content.as_bytes())?;

        Ok(path)
    }

    /// Get file extension for format
    fn format_extension(&self) -> &'static str {
        match self.format {
            ReportFormat::Text => "txt",
            ReportFormat::Markdown => "md",
            ReportFormat::Html => "html",
            ReportFormat::Json => "json",
            ReportFormat::Sarif => "sarif.json",
        }
    }

    /// Render as plain text
    fn render_text(&self, report: &DivergenceReport) -> String {
        let mut output = String::new();

        output.push_str(&format!("= {} =\n\n", report.title));
        output.push_str(&format!("Generated: {}\n\n", report.timestamp));

        for section in &report.sections {
            self.render_text_section(&mut output, section, 0);
        }

        output
    }

    /// Render a text section
    fn render_text_section(&self, output: &mut String, section: &ReportSection, depth: usize) {
        let indent = "  ".repeat(depth);
        let header_char = if depth == 0 { '=' } else { '-' };

        output.push_str(&format!(
            "{}{} {}\n",
            indent,
            header_char.to_string().repeat(3),
            section.title
        ));
        output.push_str(&format!("{}{}\n\n", indent, section.content));

        for table in &section.tables {
            self.render_text_table(output, table, &indent);
        }

        for block in &section.code_blocks {
            output.push_str(&format!("{}```{}\n", indent, block.language));
            for line in block.code.lines() {
                output.push_str(&format!("{}{}\n", indent, line));
            }
            output.push_str(&format!("{}```\n\n", indent));
        }

        for subsection in &section.subsections {
            self.render_text_section(output, subsection, depth + 1);
        }
    }

    /// Render a text table
    fn render_text_table(&self, output: &mut String, table: &ReportTable, indent: &str) {
        // Calculate column widths
        let mut widths: Vec<usize> = table.headers.iter().map(|h| h.len()).collect();
        for row in &table.rows {
            for (i, cell) in row.iter().enumerate() {
                if i < widths.len() {
                    widths[i] = widths[i].max(cell.len());
                }
            }
        }

        // Print headers
        output.push_str(indent);
        for (i, header) in table.headers.iter().enumerate() {
            output.push_str(&format!("{:width$}  ", header, width = widths[i]));
        }
        output.push('\n');

        // Print separator
        output.push_str(indent);
        for width in &widths {
            output.push_str(&format!("{:-<width$}  ", "", width = width));
        }
        output.push('\n');

        // Print rows
        for row in &table.rows {
            output.push_str(indent);
            for (i, cell) in row.iter().enumerate() {
                let width = widths.get(i).copied().unwrap_or(0);
                output.push_str(&format!("{:width$}  ", cell, width = width));
            }
            output.push('\n');
        }

        output.push('\n');
    }

    /// Render as Markdown
    fn render_markdown(&self, report: &DivergenceReport) -> String {
        let mut output = String::new();

        output.push_str(&format!("# {}\n\n", report.title));
        output.push_str(&format!("*Generated: {}*\n\n", report.timestamp));

        for section in &report.sections {
            self.render_markdown_section(&mut output, section, 2);
        }

        output
    }

    /// Render a Markdown section
    fn render_markdown_section(&self, output: &mut String, section: &ReportSection, depth: usize) {
        let header = "#".repeat(depth.min(6));
        output.push_str(&format!("{} {}\n\n", header, section.title));
        output.push_str(&format!("{}\n\n", section.content));

        for table in &section.tables {
            self.render_markdown_table(output, table);
        }

        for block in &section.code_blocks {
            output.push_str(&format!("```{}\n{}\n```\n\n", block.language, block.code));
        }

        for subsection in &section.subsections {
            self.render_markdown_section(output, subsection, depth + 1);
        }
    }

    /// Render a Markdown table
    fn render_markdown_table(&self, output: &mut String, table: &ReportTable) {
        // Headers
        output.push_str("| ");
        output.push_str(&table.headers.join(" | "));
        output.push_str(" |\n");

        // Separator
        output.push_str("| ");
        output.push_str(
            &table
                .headers
                .iter()
                .map(|_| "---")
                .collect::<Vec<_>>()
                .join(" | "),
        );
        output.push_str(" |\n");

        // Rows
        for row in &table.rows {
            output.push_str("| ");
            output.push_str(&row.join(" | "));
            output.push_str(" |\n");
        }

        output.push('\n');
    }

    /// Render as HTML
    fn render_html(&self, report: &DivergenceReport) -> String {
        let mut output = String::new();

        output.push_str("<!DOCTYPE html>\n<html>\n<head>\n");
        output.push_str(&format!("<title>{}</title>\n", report.title));
        output.push_str(r#"<style>
body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; margin: 2em; }
h1 { color: #333; }
h2 { color: #555; border-bottom: 1px solid #ddd; }
h3 { color: #666; }
table { border-collapse: collapse; width: 100%; margin: 1em 0; }
th, td { border: 1px solid #ddd; padding: 8px; text-align: left; }
th { background-color: #f2f2f2; }
tr:nth-child(even) { background-color: #f9f9f9; }
pre { background-color: #f5f5f5; padding: 1em; overflow-x: auto; }
code { font-family: "SFMono-Regular", Consolas, monospace; }
.success { color: #28a745; }
.failure { color: #dc3545; }
.warning { color: #ffc107; }
</style>
"#);
        output.push_str("</head>\n<body>\n");

        output.push_str(&format!("<h1>{}</h1>\n", report.title));
        output.push_str(&format!("<p><em>Generated: {}</em></p>\n", report.timestamp));

        for section in &report.sections {
            self.render_html_section(&mut output, section, 2);
        }

        output.push_str("</body>\n</html>\n");

        output
    }

    /// Render an HTML section
    fn render_html_section(&self, output: &mut String, section: &ReportSection, depth: usize) {
        let tag = format!("h{}", depth.min(6));
        output.push_str(&format!("<{}>{}</{}>\n", tag, section.title, tag));
        output.push_str(&format!("<p>{}</p>\n", section.content));

        for table in &section.tables {
            self.render_html_table(output, table);
        }

        for block in &section.code_blocks {
            output.push_str(&format!(
                "<pre><code class=\"language-{}\">{}</code></pre>\n",
                block.language,
                html_escape(&block.code)
            ));
        }

        for subsection in &section.subsections {
            output.push_str("<div class=\"subsection\">\n");
            self.render_html_section(output, subsection, depth + 1);
            output.push_str("</div>\n");
        }
    }

    /// Render an HTML table
    fn render_html_table(&self, output: &mut String, table: &ReportTable) {
        output.push_str("<table>\n<thead>\n<tr>\n");
        for header in &table.headers {
            output.push_str(&format!("<th>{}</th>\n", html_escape(header)));
        }
        output.push_str("</tr>\n</thead>\n<tbody>\n");

        for row in &table.rows {
            output.push_str("<tr>\n");
            for cell in row {
                output.push_str(&format!("<td>{}</td>\n", html_escape(cell)));
            }
            output.push_str("</tr>\n");
        }

        output.push_str("</tbody>\n</table>\n");
    }

    /// Render as JSON
    fn render_json(&self, report: &DivergenceReport) -> Result<String> {
        serde_json::to_string_pretty(report).context("Failed to serialize report to JSON")
    }

    /// Render as SARIF
    fn render_sarif(&self, report: &DivergenceReport) -> Result<String> {
        // SARIF 2.1.0 format for IDE integration
        let sarif = SarifReport {
            schema: "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json".to_string(),
            version: "2.1.0".to_string(),
            runs: vec![SarifRun {
                tool: SarifTool {
                    driver: SarifDriver {
                        name: "Verum Differential Tester".to_string(),
                        version: report.metadata.tool_version.clone(),
                        information_uri: "https://verumlang.org".to_string(),
                    },
                },
                results: report
                    .results
                    .iter()
                    .filter(|r| !r.success)
                    .flat_map(|r| {
                        r.divergences.iter().map(|d| SarifResult {
                            rule_id: format!("DIFF-{:?}", d.category),
                            level: match d.category {
                                DivergenceCategory::Crash => "error",
                                DivergenceCategory::ExitCode => "error",
                                DivergenceCategory::FloatPrecision => "warning",
                                DivergenceCategory::Ordering => "warning",
                                _ => "error",
                            }
                            .to_string(),
                            message: SarifMessage {
                                text: d.summary.clone(),
                            },
                            locations: vec![SarifLocation {
                                physical_location: SarifPhysicalLocation {
                                    artifact_location: SarifArtifactLocation {
                                        uri: r.test_path.to_string_lossy().to_string(),
                                    },
                                },
                            }],
                        })
                    })
                    .collect(),
            }],
        };

        serde_json::to_string_pretty(&sarif).context("Failed to serialize SARIF report")
    }
}

/// Helper to truncate strings
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// Helper to escape HTML
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// SARIF types for IDE integration
#[derive(Serialize)]
struct SarifReport {
    #[serde(rename = "$schema")]
    schema: String,
    version: String,
    runs: Vec<SarifRun>,
}

#[derive(Serialize)]
struct SarifRun {
    tool: SarifTool,
    results: Vec<SarifResult>,
}

#[derive(Serialize)]
struct SarifTool {
    driver: SarifDriver,
}

#[derive(Serialize)]
struct SarifDriver {
    name: String,
    version: String,
    #[serde(rename = "informationUri")]
    information_uri: String,
}

#[derive(Serialize)]
struct SarifResult {
    #[serde(rename = "ruleId")]
    rule_id: String,
    level: String,
    message: SarifMessage,
    locations: Vec<SarifLocation>,
}

#[derive(Serialize)]
struct SarifMessage {
    text: String,
}

#[derive(Serialize)]
struct SarifLocation {
    #[serde(rename = "physicalLocation")]
    physical_location: SarifPhysicalLocation,
}

#[derive(Serialize)]
struct SarifPhysicalLocation {
    #[serde(rename = "artifactLocation")]
    artifact_location: SarifArtifactLocation,
}

#[derive(Serialize)]
struct SarifArtifactLocation {
    uri: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_report_format_display() {
        assert_eq!(format!("{}", ReportFormat::Markdown), "markdown");
        assert_eq!(format!("{}", ReportFormat::Json), "json");
    }

    #[test]
    fn test_report_section() {
        let section = ReportSection::new("Title", "Content")
            .with_code("rust", "fn main() {}")
            .with_subsection(ReportSection::new("Sub", "SubContent"));

        assert_eq!(section.title, "Title");
        assert_eq!(section.content, "Content");
        assert_eq!(section.code_blocks.len(), 1);
        assert_eq!(section.subsections.len(), 1);
    }

    #[test]
    fn test_report_table() {
        let mut table = ReportTable::new(vec!["A".to_string(), "B".to_string()]);
        table.add_row(vec!["1".to_string(), "2".to_string()]);

        assert_eq!(table.headers.len(), 2);
        assert_eq!(table.rows.len(), 1);
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello...");
    }
}
