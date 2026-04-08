//! Divergence reporting for differential testing
//!
//! This module provides detailed reporting of differences between
//! execution tier outputs, including:
//!
//! - Human-readable diffs with context
//! - Machine-readable reports (JSON, SARIF)
//! - Minimal reproducible examples
//! - Automatic bug report generation

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::semantic_equiv::{DiffKind, DiffLocation, DiffSeverity, Difference, EquivalenceResult};

/// Tier identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tier {
    /// Interpreter (Tier 0)
    Interpreter,
    /// Bytecode VM (Tier 1)
    Bytecode,
    /// JIT compilation (Tier 2)
    Jit,
    /// AOT compilation (Tier 3)
    Aot,
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tier::Interpreter => write!(f, "Tier 0 (Interpreter)"),
            Tier::Bytecode => write!(f, "Tier 1 (Bytecode)"),
            Tier::Jit => write!(f, "Tier 2 (JIT)"),
            Tier::Aot => write!(f, "Tier 3 (AOT)"),
        }
    }
}

/// Execution output from a tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierExecution {
    pub tier: Tier,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub memory_bytes: usize,
    pub success: bool,
}

/// A divergence between tier outputs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Divergence {
    /// Unique identifier for this divergence
    pub id: String,
    /// Source file that caused the divergence
    pub source_file: PathBuf,
    /// Source code content
    pub source_code: String,
    /// Tiers being compared
    pub tier1: Tier,
    pub tier2: Tier,
    /// Execution outputs
    pub execution1: TierExecution,
    pub execution2: TierExecution,
    /// Differences found
    pub differences: Vec<Difference>,
    /// Classification of the divergence
    pub classification: DivergenceClass,
    /// Timestamp when discovered
    pub discovered_at: DateTime<Utc>,
    /// Minimized test case (if available)
    pub minimized: Option<String>,
    /// Related bug reports
    pub related_bugs: Vec<String>,
    /// Tags for filtering
    pub tags: Vec<String>,
}

/// Classification of divergence type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DivergenceClass {
    /// Semantic difference in output
    SemanticOutput,
    /// Exit code mismatch
    ExitCode,
    /// Crash in one tier
    Crash,
    /// Timeout in one tier
    Timeout,
    /// Memory behavior difference
    MemoryBehavior,
    /// Floating-point precision
    FloatPrecision,
    /// Ordering difference (async/parallel)
    Ordering,
    /// Error message difference
    ErrorMessage,
    /// Performance divergence (> 10x difference)
    PerformanceDivergence,
    /// Memory usage divergence (> 5x difference)
    MemoryUsageDivergence,
    /// Signal/exception mismatch
    SignalMismatch,
    /// Resource exhaustion (stack overflow, OOM)
    ResourceExhaustion,
    /// Determinism violation (non-deterministic output)
    DeterminismViolation,
    /// Unknown/Other
    Unknown,
}

impl std::fmt::Display for DivergenceClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DivergenceClass::SemanticOutput => write!(f, "Semantic Output Difference"),
            DivergenceClass::ExitCode => write!(f, "Exit Code Mismatch"),
            DivergenceClass::Crash => write!(f, "Crash"),
            DivergenceClass::Timeout => write!(f, "Timeout"),
            DivergenceClass::MemoryBehavior => write!(f, "Memory Behavior"),
            DivergenceClass::FloatPrecision => write!(f, "Float Precision"),
            DivergenceClass::Ordering => write!(f, "Ordering Difference"),
            DivergenceClass::ErrorMessage => write!(f, "Error Message"),
            DivergenceClass::PerformanceDivergence => write!(f, "Performance Divergence (>10x)"),
            DivergenceClass::MemoryUsageDivergence => write!(f, "Memory Usage Divergence (>5x)"),
            DivergenceClass::SignalMismatch => write!(f, "Signal/Exception Mismatch"),
            DivergenceClass::ResourceExhaustion => write!(f, "Resource Exhaustion"),
            DivergenceClass::DeterminismViolation => write!(f, "Determinism Violation"),
            DivergenceClass::Unknown => write!(f, "Unknown"),
        }
    }
}

impl DivergenceClass {
    /// Check if this divergence class is a critical issue
    pub fn is_critical(&self) -> bool {
        matches!(
            self,
            DivergenceClass::Crash
                | DivergenceClass::SemanticOutput
                | DivergenceClass::SignalMismatch
                | DivergenceClass::ResourceExhaustion
        )
    }

    /// Check if this divergence class is a warning-level issue
    pub fn is_warning(&self) -> bool {
        matches!(
            self,
            DivergenceClass::FloatPrecision
                | DivergenceClass::Ordering
                | DivergenceClass::PerformanceDivergence
                | DivergenceClass::MemoryUsageDivergence
        )
    }

    /// Get severity level as a number (higher = more severe)
    pub fn severity_level(&self) -> u8 {
        match self {
            DivergenceClass::Crash => 10,
            DivergenceClass::SemanticOutput => 9,
            DivergenceClass::SignalMismatch => 9,
            DivergenceClass::ResourceExhaustion => 8,
            DivergenceClass::ExitCode => 7,
            DivergenceClass::Timeout => 6,
            DivergenceClass::DeterminismViolation => 5,
            DivergenceClass::MemoryBehavior => 4,
            DivergenceClass::FloatPrecision => 3,
            DivergenceClass::Ordering => 3,
            DivergenceClass::PerformanceDivergence => 2,
            DivergenceClass::MemoryUsageDivergence => 2,
            DivergenceClass::ErrorMessage => 1,
            DivergenceClass::Unknown => 0,
        }
    }
}

/// Divergence report builder
pub struct DivergenceReporter {
    /// Output directory for reports
    output_dir: PathBuf,
    /// Report format
    format: ReportFormat,
    /// Context lines around differences
    context_lines: usize,
    /// Whether to include source code in reports
    include_source: bool,
    /// Whether to include timestamps
    include_timestamps: bool,
    /// Maximum diff length before truncation
    max_diff_length: usize,
}

/// Report output format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportFormat {
    /// Human-readable text
    Text,
    /// JSON for machine processing
    Json,
    /// SARIF for IDE integration
    Sarif,
    /// Markdown for documentation
    Markdown,
    /// HTML for web viewing
    Html,
}

impl DivergenceReporter {
    /// Create a new reporter
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            output_dir,
            format: ReportFormat::Text,
            context_lines: 3,
            include_source: true,
            include_timestamps: true,
            max_diff_length: 10000,
        }
    }

    /// Set report format
    pub fn with_format(mut self, format: ReportFormat) -> Self {
        self.format = format;
        self
    }

    /// Set context lines
    pub fn with_context_lines(mut self, lines: usize) -> Self {
        self.context_lines = lines;
        self
    }

    /// Generate a divergence report
    pub fn report(&self, divergence: &Divergence) -> Result<PathBuf> {
        // Ensure output directory exists
        fs::create_dir_all(&self.output_dir)?;

        let filename = format!(
            "divergence_{}_{}.{}",
            divergence.id,
            chrono::Utc::now().format("%Y%m%d_%H%M%S"),
            self.file_extension()
        );

        let path = self.output_dir.join(&filename);
        let content = self.render(divergence)?;

        let mut file = File::create(&path)?;
        file.write_all(content.as_bytes())?;

        Ok(path)
    }

    /// Get file extension for format
    fn file_extension(&self) -> &'static str {
        match self.format {
            ReportFormat::Text => "txt",
            ReportFormat::Json => "json",
            ReportFormat::Sarif => "sarif.json",
            ReportFormat::Markdown => "md",
            ReportFormat::Html => "html",
        }
    }

    /// Render divergence to string
    fn render(&self, divergence: &Divergence) -> Result<String> {
        match self.format {
            ReportFormat::Text => self.render_text(divergence),
            ReportFormat::Json => self.render_json(divergence),
            ReportFormat::Sarif => self.render_sarif(divergence),
            ReportFormat::Markdown => self.render_markdown(divergence),
            ReportFormat::Html => self.render_html(divergence),
        }
    }

    /// Render as plain text
    fn render_text(&self, divergence: &Divergence) -> Result<String> {
        let mut out = String::new();

        writeln!(out, "{}", "=".repeat(80))?;
        writeln!(out, "DIVERGENCE REPORT")?;
        writeln!(out, "{}", "=".repeat(80))?;
        writeln!(out)?;

        writeln!(out, "ID:            {}", divergence.id)?;
        writeln!(out, "Classification: {}", divergence.classification)?;
        writeln!(out, "Source File:   {}", divergence.source_file.display())?;
        writeln!(out, "Discovered:    {}", divergence.discovered_at)?;
        writeln!(out)?;

        writeln!(out, "TIER COMPARISON")?;
        writeln!(out, "{}", "-".repeat(40))?;
        writeln!(out, "{} vs {}", divergence.tier1, divergence.tier2)?;
        writeln!(out)?;

        // Execution summary
        writeln!(out, "Tier 1 ({}):", divergence.tier1)?;
        writeln!(out, "  Exit code: {:?}", divergence.execution1.exit_code)?;
        writeln!(out, "  Duration:  {}ms", divergence.execution1.duration_ms)?;
        writeln!(
            out,
            "  Memory:    {} bytes",
            divergence.execution1.memory_bytes
        )?;
        writeln!(out)?;

        writeln!(out, "Tier 2 ({}):", divergence.tier2)?;
        writeln!(out, "  Exit code: {:?}", divergence.execution2.exit_code)?;
        writeln!(out, "  Duration:  {}ms", divergence.execution2.duration_ms)?;
        writeln!(
            out,
            "  Memory:    {} bytes",
            divergence.execution2.memory_bytes
        )?;
        writeln!(out)?;

        // Differences
        writeln!(out, "DIFFERENCES")?;
        writeln!(out, "{}", "-".repeat(40))?;

        for (i, diff) in divergence.differences.iter().enumerate() {
            writeln!(out, "Difference #{}:", i + 1)?;
            writeln!(out, "  Location:  {}", diff.location)?;
            writeln!(out, "  Kind:      {:?}", diff.kind)?;
            writeln!(out, "  Severity:  {:?}", diff.severity)?;
            writeln!(out, "  Expected:  {}", truncate(&diff.expected, 200))?;
            writeln!(out, "  Actual:    {}", truncate(&diff.actual, 200))?;
            writeln!(out)?;
        }

        // Unified diff
        writeln!(out, "UNIFIED DIFF")?;
        writeln!(out, "{}", "-".repeat(40))?;
        writeln!(
            out,
            "{}",
            self.generate_unified_diff(
                &divergence.execution1.stdout,
                &divergence.execution2.stdout,
                &format!("{}", divergence.tier1),
                &format!("{}", divergence.tier2),
            )
        )?;

        // Source code
        if self.include_source {
            writeln!(out)?;
            writeln!(out, "SOURCE CODE")?;
            writeln!(out, "{}", "-".repeat(40))?;
            writeln!(out, "{}", &divergence.source_code)?;
        }

        // Minimized case
        if let Some(ref minimized) = divergence.minimized {
            writeln!(out)?;
            writeln!(out, "MINIMIZED TEST CASE")?;
            writeln!(out, "{}", "-".repeat(40))?;
            writeln!(out, "{}", minimized)?;
        }

        Ok(out)
    }

    /// Render as JSON
    fn render_json(&self, divergence: &Divergence) -> Result<String> {
        serde_json::to_string_pretty(divergence).context("Failed to serialize divergence to JSON")
    }

    /// Render as SARIF (Static Analysis Results Interchange Format)
    fn render_sarif(&self, divergence: &Divergence) -> Result<String> {
        let sarif = SarifReport {
            schema: "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json".to_string(),
            version: "2.1.0".to_string(),
            runs: vec![SarifRun {
                tool: SarifTool {
                    driver: SarifToolDriver {
                        name: "verum-differential".to_string(),
                        version: "0.1.0".to_string(),
                        rules: vec![SarifRule {
                            id: format!("DIFF{:03}", divergence.classification as u32),
                            short_description: SarifMessage {
                                text: format!("{}", divergence.classification),
                            },
                            full_description: SarifMessage {
                                text: format!(
                                    "Differential testing found {} between {} and {}",
                                    divergence.classification,
                                    divergence.tier1,
                                    divergence.tier2
                                ),
                            },
                        }],
                    },
                },
                results: divergence
                    .differences
                    .iter()
                    .map(|diff| SarifResult {
                        rule_id: format!("DIFF{:03}", divergence.classification as u32),
                        message: SarifMessage {
                            text: format!(
                                "{:?} at {}: expected '{}', got '{}'",
                                diff.kind,
                                diff.location,
                                truncate(&diff.expected, 100),
                                truncate(&diff.actual, 100)
                            ),
                        },
                        locations: vec![SarifLocation {
                            physical_location: SarifPhysicalLocation {
                                artifact_location: SarifArtifactLocation {
                                    uri: divergence.source_file.to_string_lossy().to_string(),
                                },
                                region: match &diff.location {
                                    DiffLocation::Line(n) => Some(SarifRegion {
                                        start_line: *n,
                                        end_line: *n,
                                    }),
                                    DiffLocation::LineRange { start, end } => Some(SarifRegion {
                                        start_line: *start,
                                        end_line: *end,
                                    }),
                                    _ => None,
                                },
                            },
                        }],
                        level: match diff.severity {
                            DiffSeverity::Info => "note",
                            DiffSeverity::Warning => "warning",
                            DiffSeverity::Error => "error",
                            DiffSeverity::Critical => "error",
                        }
                        .to_string(),
                    })
                    .collect(),
            }],
        };

        serde_json::to_string_pretty(&sarif).context("Failed to serialize SARIF")
    }

    /// Render as Markdown
    fn render_markdown(&self, divergence: &Divergence) -> Result<String> {
        let mut out = String::new();

        writeln!(out, "# Divergence Report: {}", divergence.id)?;
        writeln!(out)?;
        writeln!(out, "**Classification:** {}", divergence.classification)?;
        writeln!(
            out,
            "**Source File:** `{}`",
            divergence.source_file.display()
        )?;
        writeln!(out, "**Discovered:** {}", divergence.discovered_at)?;
        writeln!(out)?;

        writeln!(out, "## Tier Comparison")?;
        writeln!(out)?;
        writeln!(
            out,
            "| Metric | {} | {} |",
            divergence.tier1, divergence.tier2
        )?;
        writeln!(out, "|--------|------|------|")?;
        writeln!(
            out,
            "| Exit Code | {:?} | {:?} |",
            divergence.execution1.exit_code, divergence.execution2.exit_code
        )?;
        writeln!(
            out,
            "| Duration | {}ms | {}ms |",
            divergence.execution1.duration_ms, divergence.execution2.duration_ms
        )?;
        writeln!(
            out,
            "| Memory | {} bytes | {} bytes |",
            divergence.execution1.memory_bytes, divergence.execution2.memory_bytes
        )?;
        writeln!(out)?;

        writeln!(out, "## Differences")?;
        writeln!(out)?;

        for (i, diff) in divergence.differences.iter().enumerate() {
            writeln!(out, "### Difference #{}", i + 1)?;
            writeln!(out)?;
            writeln!(out, "- **Location:** {}", diff.location)?;
            writeln!(out, "- **Kind:** {:?}", diff.kind)?;
            writeln!(out, "- **Severity:** {:?}", diff.severity)?;
            writeln!(out)?;
            writeln!(out, "**Expected:**")?;
            writeln!(out, "```")?;
            writeln!(out, "{}", truncate(&diff.expected, 500))?;
            writeln!(out, "```")?;
            writeln!(out)?;
            writeln!(out, "**Actual:**")?;
            writeln!(out, "```")?;
            writeln!(out, "{}", truncate(&diff.actual, 500))?;
            writeln!(out, "```")?;
            writeln!(out)?;
        }

        writeln!(out, "## Unified Diff")?;
        writeln!(out)?;
        writeln!(out, "```diff")?;
        writeln!(
            out,
            "{}",
            self.generate_unified_diff(
                &divergence.execution1.stdout,
                &divergence.execution2.stdout,
                &format!("{}", divergence.tier1),
                &format!("{}", divergence.tier2),
            )
        )?;
        writeln!(out, "```")?;

        if self.include_source {
            writeln!(out)?;
            writeln!(out, "## Source Code")?;
            writeln!(out)?;
            writeln!(out, "```verum")?;
            writeln!(out, "{}", &divergence.source_code)?;
            writeln!(out, "```")?;
        }

        if let Some(ref minimized) = divergence.minimized {
            writeln!(out)?;
            writeln!(out, "## Minimized Test Case")?;
            writeln!(out)?;
            writeln!(out, "```verum")?;
            writeln!(out, "{}", minimized)?;
            writeln!(out, "```")?;
        }

        Ok(out)
    }

    /// Render as HTML
    fn render_html(&self, divergence: &Divergence) -> Result<String> {
        let mut out = String::new();

        writeln!(out, "<!DOCTYPE html>")?;
        writeln!(out, "<html><head>")?;
        writeln!(out, "<meta charset='utf-8'>")?;
        writeln!(out, "<title>Divergence Report: {}</title>", divergence.id)?;
        writeln!(out, "<style>")?;
        writeln!(
            out,
            "body {{ font-family: system-ui; max-width: 1200px; margin: 0 auto; padding: 20px; }}"
        )?;
        writeln!(
            out,
            "pre {{ background: #f5f5f5; padding: 10px; overflow-x: auto; }}"
        )?;
        writeln!(out, ".diff-add {{ background: #e6ffec; }}")?;
        writeln!(out, ".diff-del {{ background: #ffebe9; }}")?;
        writeln!(out, "table {{ border-collapse: collapse; width: 100%; }}")?;
        writeln!(
            out,
            "th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}"
        )?;
        writeln!(out, "th {{ background: #f5f5f5; }}")?;
        writeln!(out, ".severity-info {{ color: #0969da; }}")?;
        writeln!(out, ".severity-warning {{ color: #9a6700; }}")?;
        writeln!(out, ".severity-error {{ color: #cf222e; }}")?;
        writeln!(
            out,
            ".severity-critical {{ color: #cf222e; font-weight: bold; }}"
        )?;
        writeln!(out, "</style>")?;
        writeln!(out, "</head><body>")?;

        writeln!(out, "<h1>Divergence Report: {}</h1>", divergence.id)?;
        writeln!(
            out,
            "<p><strong>Classification:</strong> {}</p>",
            divergence.classification
        )?;
        writeln!(
            out,
            "<p><strong>Source File:</strong> <code>{}</code></p>",
            divergence.source_file.display()
        )?;
        writeln!(
            out,
            "<p><strong>Discovered:</strong> {}</p>",
            divergence.discovered_at
        )?;

        writeln!(out, "<h2>Tier Comparison</h2>")?;
        writeln!(out, "<table>")?;
        writeln!(
            out,
            "<tr><th>Metric</th><th>{}</th><th>{}</th></tr>",
            divergence.tier1, divergence.tier2
        )?;
        writeln!(
            out,
            "<tr><td>Exit Code</td><td>{:?}</td><td>{:?}</td></tr>",
            divergence.execution1.exit_code, divergence.execution2.exit_code
        )?;
        writeln!(
            out,
            "<tr><td>Duration</td><td>{}ms</td><td>{}ms</td></tr>",
            divergence.execution1.duration_ms, divergence.execution2.duration_ms
        )?;
        writeln!(
            out,
            "<tr><td>Memory</td><td>{} bytes</td><td>{} bytes</td></tr>",
            divergence.execution1.memory_bytes, divergence.execution2.memory_bytes
        )?;
        writeln!(out, "</table>")?;

        writeln!(out, "<h2>Differences</h2>")?;
        for (i, diff) in divergence.differences.iter().enumerate() {
            let severity_class = match diff.severity {
                DiffSeverity::Info => "severity-info",
                DiffSeverity::Warning => "severity-warning",
                DiffSeverity::Error => "severity-error",
                DiffSeverity::Critical => "severity-critical",
            };

            writeln!(out, "<h3>Difference #{}</h3>", i + 1)?;
            writeln!(out, "<ul>")?;
            writeln!(out, "<li><strong>Location:</strong> {}</li>", diff.location)?;
            writeln!(out, "<li><strong>Kind:</strong> {:?}</li>", diff.kind)?;
            writeln!(
                out,
                "<li class='{}'>Severity: {:?}</li>",
                severity_class, diff.severity
            )?;
            writeln!(out, "</ul>")?;
            writeln!(out, "<p><strong>Expected:</strong></p>")?;
            writeln!(
                out,
                "<pre>{}</pre>",
                html_escape(&truncate(&diff.expected, 500))
            )?;
            writeln!(out, "<p><strong>Actual:</strong></p>")?;
            writeln!(
                out,
                "<pre>{}</pre>",
                html_escape(&truncate(&diff.actual, 500))
            )?;
        }

        writeln!(out, "<h2>Unified Diff</h2>")?;
        writeln!(
            out,
            "<pre>{}</pre>",
            html_escape(&self.generate_unified_diff(
                &divergence.execution1.stdout,
                &divergence.execution2.stdout,
                &format!("{}", divergence.tier1),
                &format!("{}", divergence.tier2),
            ))
        )?;

        if self.include_source {
            writeln!(out, "<h2>Source Code</h2>")?;
            writeln!(out, "<pre>{}</pre>", html_escape(&divergence.source_code))?;
        }

        if let Some(ref minimized) = divergence.minimized {
            writeln!(out, "<h2>Minimized Test Case</h2>")?;
            writeln!(out, "<pre>{}</pre>", html_escape(minimized))?;
        }

        writeln!(out, "</body></html>")?;

        Ok(out)
    }

    /// Generate unified diff
    fn generate_unified_diff(&self, a: &str, b: &str, label_a: &str, label_b: &str) -> String {
        let a_lines: Vec<&str> = a.lines().collect();
        let b_lines: Vec<&str> = b.lines().collect();

        let mut output = String::new();
        let _ = writeln!(output, "--- {}", label_a);
        let _ = writeln!(output, "+++ {}", label_b);

        // Simple diff algorithm
        let mut i = 0;
        let mut j = 0;

        while i < a_lines.len() || j < b_lines.len() {
            if i < a_lines.len() && j < b_lines.len() && a_lines[i] == b_lines[j] {
                let _ = writeln!(output, " {}", a_lines[i]);
                i += 1;
                j += 1;
            } else if i < a_lines.len()
                && (j >= b_lines.len() || !b_lines[j..].contains(&a_lines[i]))
            {
                let _ = writeln!(output, "-{}", a_lines[i]);
                i += 1;
            } else if j < b_lines.len() {
                let _ = writeln!(output, "+{}", b_lines[j]);
                j += 1;
            }
        }

        output
    }
}

/// Create a divergence from execution results
pub fn create_divergence(
    source_file: PathBuf,
    source_code: String,
    tier1: Tier,
    exec1: TierExecution,
    tier2: Tier,
    exec2: TierExecution,
    equiv_result: EquivalenceResult,
) -> Option<Divergence> {
    let differences = match equiv_result {
        EquivalenceResult::Equivalent => return None,
        EquivalenceResult::Different(diffs) => diffs,
    };

    // Classify the divergence
    let classification = classify_divergence(&exec1, &exec2, &differences);

    // Generate unique ID
    let id = format!(
        "{:x}",
        md5::compute(format!(
            "{}{}{:?}{:?}",
            source_code,
            source_file.display(),
            tier1,
            tier2
        ))
    );

    Some(Divergence {
        id,
        source_file,
        source_code,
        tier1,
        tier2,
        execution1: exec1,
        execution2: exec2,
        differences,
        classification,
        discovered_at: Utc::now(),
        minimized: None,
        related_bugs: vec![],
        tags: vec![],
    })
}

/// Divergence classification configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DivergenceThresholds {
    /// Performance ratio threshold (default: 10.0 = 10x slower)
    pub performance_ratio: f64,
    /// Memory ratio threshold (default: 5.0 = 5x more memory)
    pub memory_ratio: f64,
    /// Timeout threshold in ms (default: 30000)
    pub timeout_ms: u64,
    /// Minimum duration for performance comparison (default: 10ms)
    pub min_duration_ms: u64,
    /// Minimum memory for memory comparison (default: 1024 bytes)
    pub min_memory_bytes: usize,
}

impl Default for DivergenceThresholds {
    fn default() -> Self {
        Self {
            performance_ratio: 10.0,
            memory_ratio: 5.0,
            timeout_ms: 30000,
            min_duration_ms: 10,
            min_memory_bytes: 1024,
        }
    }
}

/// Classify the type of divergence
fn classify_divergence(
    exec1: &TierExecution,
    exec2: &TierExecution,
    differences: &[Difference],
) -> DivergenceClass {
    classify_divergence_with_thresholds(exec1, exec2, differences, &DivergenceThresholds::default())
}

/// Classify the type of divergence with custom thresholds
pub fn classify_divergence_with_thresholds(
    exec1: &TierExecution,
    exec2: &TierExecution,
    differences: &[Difference],
    thresholds: &DivergenceThresholds,
) -> DivergenceClass {
    // Check for resource exhaustion (stack overflow, OOM)
    if is_resource_exhaustion(&exec1.stderr) || is_resource_exhaustion(&exec2.stderr) {
        return DivergenceClass::ResourceExhaustion;
    }

    // Check for crashes
    if !exec1.success || !exec2.success {
        if exec1.stderr.contains("panic")
            || exec2.stderr.contains("panic")
            || exec1.stderr.contains("crash")
            || exec2.stderr.contains("crash")
            || exec1.stderr.contains("SIGSEGV")
            || exec2.stderr.contains("SIGSEGV")
            || exec1.stderr.contains("SIGABRT")
            || exec2.stderr.contains("SIGABRT")
        {
            return DivergenceClass::Crash;
        }
    }

    // Check for signal mismatches
    if is_signal_mismatch(exec1, exec2) {
        return DivergenceClass::SignalMismatch;
    }

    // Check for exit code mismatch
    if exec1.exit_code != exec2.exit_code {
        return DivergenceClass::ExitCode;
    }

    // Check for timeout (one tier took much longer than the other)
    let (min_dur, max_dur) = if exec1.duration_ms < exec2.duration_ms {
        (exec1.duration_ms, exec2.duration_ms)
    } else {
        (exec2.duration_ms, exec1.duration_ms)
    };

    if max_dur > thresholds.timeout_ms {
        return DivergenceClass::Timeout;
    }

    // Check for performance divergence (> 10x difference by default)
    if min_dur >= thresholds.min_duration_ms && max_dur > 0 {
        let ratio = max_dur as f64 / min_dur.max(1) as f64;
        if ratio > thresholds.performance_ratio {
            return DivergenceClass::PerformanceDivergence;
        }
    }

    // Check for memory usage divergence (> 5x difference by default)
    let (min_mem, max_mem) = if exec1.memory_bytes < exec2.memory_bytes {
        (exec1.memory_bytes, exec2.memory_bytes)
    } else {
        (exec2.memory_bytes, exec1.memory_bytes)
    };

    if min_mem >= thresholds.min_memory_bytes && max_mem > 0 {
        let ratio = max_mem as f64 / min_mem.max(1) as f64;
        if ratio > thresholds.memory_ratio {
            return DivergenceClass::MemoryUsageDivergence;
        }
    }

    // Check differences for classification
    for diff in differences {
        match &diff.kind {
            DiffKind::FloatMismatch { .. } => return DivergenceClass::FloatPrecision,
            DiffKind::OrderMismatch => return DivergenceClass::Ordering,
            _ => {}
        }
    }

    // Check if it's error message difference
    if !exec1.stderr.is_empty() && !exec2.stderr.is_empty() {
        return DivergenceClass::ErrorMessage;
    }

    // Default to semantic output difference
    DivergenceClass::SemanticOutput
}

/// Check if output indicates resource exhaustion
fn is_resource_exhaustion(stderr: &str) -> bool {
    let patterns = [
        "stack overflow",
        "out of memory",
        "OutOfMemoryError",
        "StackOverflowError",
        "memory allocation failed",
        "cannot allocate memory",
        "fatal runtime error: stack overflow",
        "thread 'main' has overflowed its stack",
    ];

    let stderr_lower = stderr.to_lowercase();
    patterns
        .iter()
        .any(|p| stderr_lower.contains(&p.to_lowercase()))
}

/// Check if there's a signal mismatch between executions
fn is_signal_mismatch(exec1: &TierExecution, exec2: &TierExecution) -> bool {
    let get_signal = |stderr: &str| -> Option<&str> {
        for signal in [
            "SIGSEGV", "SIGABRT", "SIGFPE", "SIGILL", "SIGBUS", "SIGTRAP",
        ] {
            if stderr.contains(signal) {
                return Some(signal);
            }
        }
        None
    };

    let sig1 = get_signal(&exec1.stderr);
    let sig2 = get_signal(&exec2.stderr);

    // Mismatch if one has a signal and the other doesn't, or different signals
    match (sig1, sig2) {
        (Some(s1), Some(s2)) => s1 != s2,
        (Some(_), None) | (None, Some(_)) => true,
        (None, None) => false,
    }
}

/// Detect determinism violations by running the same test multiple times
pub fn detect_determinism_violation(outputs: &[TierExecution]) -> Option<DivergenceClass> {
    if outputs.len() < 2 {
        return None;
    }

    let first_output = &outputs[0].stdout;
    for output in &outputs[1..] {
        if output.stdout != *first_output {
            return Some(DivergenceClass::DeterminismViolation);
        }
    }
    None
}

/// Truncate a string to max length
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// HTML escape
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// SARIF types for serialization
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
    driver: SarifToolDriver,
}

#[derive(Serialize)]
struct SarifToolDriver {
    name: String,
    version: String,
    rules: Vec<SarifRule>,
}

#[derive(Serialize)]
struct SarifRule {
    id: String,
    #[serde(rename = "shortDescription")]
    short_description: SarifMessage,
    #[serde(rename = "fullDescription")]
    full_description: SarifMessage,
}

#[derive(Serialize)]
struct SarifResult {
    #[serde(rename = "ruleId")]
    rule_id: String,
    message: SarifMessage,
    locations: Vec<SarifLocation>,
    level: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    region: Option<SarifRegion>,
}

#[derive(Serialize)]
struct SarifArtifactLocation {
    uri: String,
}

#[derive(Serialize)]
struct SarifRegion {
    #[serde(rename = "startLine")]
    start_line: usize,
    #[serde(rename = "endLine")]
    end_line: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier_display() {
        assert_eq!(format!("{}", Tier::Interpreter), "Tier 0 (Interpreter)");
        assert_eq!(format!("{}", Tier::Aot), "Tier 3 (AOT)");
    }

    #[test]
    fn test_divergence_classification() {
        let exec1 = TierExecution {
            tier: Tier::Interpreter,
            stdout: "42".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: 100,
            memory_bytes: 1024,
            success: true,
        };

        let exec2 = TierExecution {
            tier: Tier::Aot,
            stdout: "42".to_string(),
            stderr: String::new(),
            exit_code: Some(1),
            duration_ms: 50,
            memory_bytes: 2048,
            success: true,
        };

        let class = classify_divergence(&exec1, &exec2, &[]);
        assert_eq!(class, DivergenceClass::ExitCode);
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello...");
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
    }
}
