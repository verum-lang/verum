//! Diagnostic emission with support for multiple output formats.
//!
//! This module provides the emitter that takes diagnostics and outputs them
//! in various formats (human-readable text, JSON, etc.) for different consumers.

use crate::{
    Severity,
    diagnostic::{Diagnostic, SourceLocation},
    renderer::{RenderConfig, Renderer},
};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use verum_common::{List, Text};

/// Output format for diagnostics
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Human-readable text with colors
    Human,
    /// Human-readable text without colors
    HumanNoColor,
    /// JSON format for machine consumption
    Json,
    /// Compact JSON (single line per diagnostic)
    JsonCompact,
}

/// Configuration for the emitter
#[derive(Debug, Clone)]
pub struct EmitterConfig {
    /// Output format
    pub format: OutputFormat,
    /// Show source snippets (for human format)
    pub show_source: bool,
    /// Number of context lines
    pub context_lines: usize,
    /// Enable colors (for human format)
    pub colors: bool,
}

impl Default for EmitterConfig {
    fn default() -> Self {
        Self {
            format: OutputFormat::Human,
            show_source: true,
            context_lines: 2,
            colors: true,
        }
    }
}

impl EmitterConfig {
    pub fn human() -> Self {
        Self::default()
    }

    pub fn json() -> Self {
        Self {
            format: OutputFormat::Json,
            ..Default::default()
        }
    }

    pub fn no_color() -> Self {
        Self {
            colors: false,
            ..Default::default()
        }
    }

    pub fn minimal() -> Self {
        Self {
            show_source: false,
            context_lines: 0,
            ..Default::default()
        }
    }
}

/// Emitter for diagnostics
pub struct Emitter {
    config: EmitterConfig,
    renderer: Renderer,
    /// Accumulated diagnostics
    diagnostics: List<Diagnostic>,
}

impl Emitter {
    pub fn new(config: EmitterConfig) -> Self {
        let render_config = RenderConfig {
            colors: config.colors,
            context_lines: config.context_lines,
            show_line_numbers: true,
            max_line_width: 120,
            show_source: config.show_source,
            show_suggestions: true,
            show_doc_urls: true,
            unicode_output: true,
            terminal_width: 80,
            relative_paths: false,
        };

        Self {
            config,
            renderer: Renderer::new(render_config),
            diagnostics: List::new(),
        }
    }

    pub fn default() -> Self {
        Self::new(EmitterConfig::default())
    }

    /// Emit a diagnostic immediately
    pub fn emit<W: Write>(&mut self, diagnostic: &Diagnostic, writer: &mut W) -> io::Result<()> {
        match self.config.format {
            OutputFormat::Human | OutputFormat::HumanNoColor => self.emit_human(diagnostic, writer),
            OutputFormat::Json => self.emit_json(diagnostic, writer),
            OutputFormat::JsonCompact => self.emit_json_compact(diagnostic, writer),
        }
    }

    /// Emit to stdout
    pub fn emit_stdout(&mut self, diagnostic: &Diagnostic) -> io::Result<()> {
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        self.emit(diagnostic, &mut handle)
    }

    /// Emit to stderr
    pub fn emit_stderr(&mut self, diagnostic: &Diagnostic) -> io::Result<()> {
        let stderr = io::stderr();
        let mut handle = stderr.lock();
        self.emit(diagnostic, &mut handle)
    }

    /// Add a diagnostic to the accumulator without emitting
    pub fn add(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Emit all accumulated diagnostics
    pub fn emit_all<W: Write>(&mut self, writer: &mut W) -> io::Result<()> {
        match self.config.format {
            OutputFormat::Human | OutputFormat::HumanNoColor => {
                let diagnostics = self.diagnostics.clone();
                for diag in &diagnostics {
                    self.emit_human(diag, writer)?;
                    writeln!(writer)?;
                }
            }
            OutputFormat::Json => {
                let json = self.diagnostics_to_json(&self.diagnostics);
                writeln!(writer, "{}", serde_json::to_string_pretty(&json).unwrap())?;
            }
            OutputFormat::JsonCompact => {
                for diag in &self.diagnostics {
                    let json = self.diagnostic_to_json(diag);
                    writeln!(writer, "{}", serde_json::to_string(&json).unwrap())?;
                }
            }
        }
        Ok(())
    }

    /// Clear accumulated diagnostics
    pub fn clear(&mut self) {
        self.diagnostics.clear();
    }

    /// Get all accumulated diagnostics
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Count errors
    pub fn error_count(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.is_error()).count()
    }

    /// Count warnings
    pub fn warning_count(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.is_warning()).count()
    }

    /// Check if there are any errors
    pub fn has_errors(&self) -> bool {
        self.error_count() > 0
    }

    /// Emit summary statistics
    pub fn emit_summary<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let errors = self.error_count();
        let warnings = self.warning_count();

        if errors == 0 && warnings == 0 {
            return Ok(());
        }

        writeln!(writer)?;

        if errors > 0 {
            writeln!(
                writer,
                "error: compilation failed with {} error{}",
                errors,
                if errors == 1 { "" } else { "s" }
            )?;
        }

        if warnings > 0 {
            writeln!(
                writer,
                "warning: {} warning{} emitted",
                warnings,
                if warnings == 1 { "" } else { "s" }
            )?;
        }

        Ok(())
    }

    // Internal: Emit human-readable format
    fn emit_human<W: Write>(&mut self, diagnostic: &Diagnostic, writer: &mut W) -> io::Result<()> {
        let rendered = self.renderer.render(diagnostic);
        write!(writer, "{}", rendered)
    }

    // Internal: Emit JSON format
    fn emit_json<W: Write>(&self, diagnostic: &Diagnostic, writer: &mut W) -> io::Result<()> {
        let json = self.diagnostic_to_json(diagnostic);
        writeln!(writer, "{}", serde_json::to_string_pretty(&json).unwrap())
    }

    // Internal: Emit compact JSON format
    fn emit_json_compact<W: Write>(
        &self,
        diagnostic: &Diagnostic,
        writer: &mut W,
    ) -> io::Result<()> {
        let json = self.diagnostic_to_json(diagnostic);
        writeln!(writer, "{}", serde_json::to_string(&json).unwrap())
    }

    // Internal: Convert diagnostic to JSON representation
    fn diagnostic_to_json(&self, diagnostic: &Diagnostic) -> JsonDiagnostic {
        JsonDiagnostic {
            level: match diagnostic.severity() {
                Severity::Error => "error".into(),
                Severity::Warning => "warning".into(),
                Severity::Note => "note".into(),
                Severity::Help => "help".into(),
            },
            code: diagnostic.code().map(|s| s.into()),
            message: diagnostic.message().to_string(),
            spans: diagnostic
                .primary_labels()
                .iter()
                .map(|l| JsonSpan {
                    location: SourceLocation::from(&l.span),
                    label: l.message.clone(),
                    is_primary: l.is_primary,
                })
                .collect(),
            children: diagnostic
                .children()
                .iter()
                .map(|c| self.diagnostic_to_json(c))
                .collect(),
            notes: diagnostic
                .notes()
                .iter()
                .map(|n| n.message.clone())
                .collect(),
            helps: diagnostic
                .helps()
                .iter()
                .map(|h| h.message.clone())
                .collect(),
            suggested_fixes: diagnostic
                .suggested_fixes()
                .iter()
                .map(|fix| JsonSuggestedFix {
                    message: fix.message.clone(),
                    span: JsonSpan {
                        location: SourceLocation::from(&fix.span),
                        label: fix.message.clone(),
                        is_primary: true,
                    },
                    replacement: fix.replacement.clone(),
                    is_machine_applicable: fix.is_machine_applicable,
                })
                .collect(),
            doc_url: diagnostic.doc_url().map(|s| s.into()),
            is_fixable: diagnostic.is_fixable(),
            related_files: diagnostic.related_files().to_vec().into(),
        }
    }

    // Internal: Convert multiple diagnostics to JSON
    fn diagnostics_to_json(&self, diagnostics: &[Diagnostic]) -> JsonOutput {
        let error_count = diagnostics.iter().filter(|d| d.is_error()).count();
        let warning_count = diagnostics.iter().filter(|d| d.is_warning()).count();
        let note_count = diagnostics
            .iter()
            .filter(|d| d.is_note() || d.is_help())
            .count();
        let fixable_count = diagnostics.iter().filter(|d| d.is_fixable()).count();

        JsonOutput {
            diagnostics: diagnostics
                .iter()
                .map(|d| self.diagnostic_to_json(d))
                .collect(),
            summary: JsonSummary {
                error_count,
                warning_count,
                note_count,
                fixable_count,
            },
        }
    }
}

/// LSP-compatible diagnostic output
#[derive(Debug, Serialize, Deserialize)]
pub struct LspDiagnostic {
    /// Diagnostic range
    pub range: LspRange,
    /// Severity (1 = error, 2 = warning, 3 = info, 4 = hint)
    pub severity: u32,
    /// Error code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Text>,
    /// Source (usually "verum")
    pub source: Text,
    /// Diagnostic message
    pub message: Text,
    /// Related information
    #[serde(skip_serializing_if = "list_is_empty")]
    pub related_information: List<LspRelatedInformation>,
    /// Code actions (for suggested fixes)
    #[serde(skip_serializing_if = "list_is_empty")]
    pub code_actions: List<LspCodeAction>,
}

/// LSP range
#[derive(Debug, Serialize, Deserialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

/// LSP position (0-indexed)
#[derive(Debug, Serialize, Deserialize)]
pub struct LspPosition {
    pub line: usize,
    pub character: usize,
}

/// LSP related information
#[derive(Debug, Serialize, Deserialize)]
pub struct LspRelatedInformation {
    pub location: LspLocation,
    pub message: Text,
}

/// LSP location
#[derive(Debug, Serialize, Deserialize)]
pub struct LspLocation {
    pub uri: Text,
    pub range: LspRange,
}

/// LSP code action for suggested fixes
#[derive(Debug, Serialize, Deserialize)]
pub struct LspCodeAction {
    pub title: Text,
    pub kind: Text,
    pub is_preferred: bool,
    pub edit: LspWorkspaceEdit,
}

/// LSP workspace edit
#[derive(Debug, Serialize, Deserialize)]
pub struct LspWorkspaceEdit {
    pub changes: std::collections::HashMap<Text, List<LspTextEdit>>,
}

/// LSP text edit
#[derive(Debug, Serialize, Deserialize)]
pub struct LspTextEdit {
    pub range: LspRange,
    pub new_text: Text,
}

impl Diagnostic {
    /// Convert to LSP-compatible diagnostic
    pub fn to_lsp(&self) -> LspDiagnostic {
        let (range, uri): (LspRange, Text) = if let Some(span) = self.primary_span() {
            (
                LspRange {
                    start: LspPosition {
                        line: span.line.saturating_sub(1),
                        character: span.column.saturating_sub(1),
                    },
                    end: LspPosition {
                        line: span.end_line.unwrap_or(span.line).saturating_sub(1),
                        character: span.end_column.saturating_sub(1),
                    },
                },
                format!("file://{}", span.file).into(),
            )
        } else {
            (
                LspRange {
                    start: LspPosition {
                        line: 0,
                        character: 0,
                    },
                    end: LspPosition {
                        line: 0,
                        character: 0,
                    },
                },
                Text::new(),
            )
        };

        let severity = match self.severity() {
            Severity::Error => 1,
            Severity::Warning => 2,
            Severity::Note => 3,
            Severity::Help => 4,
        };

        let related_information = self
            .secondary_labels()
            .iter()
            .map(|label| LspRelatedInformation {
                location: LspLocation {
                    uri: format!("file://{}", label.span.file).into(),
                    range: LspRange {
                        start: LspPosition {
                            line: label.span.line.saturating_sub(1),
                            character: label.span.column.saturating_sub(1),
                        },
                        end: LspPosition {
                            line: label
                                .span
                                .end_line
                                .unwrap_or(label.span.line)
                                .saturating_sub(1),
                            character: label.span.end_column.saturating_sub(1),
                        },
                    },
                },
                message: label.message.clone(),
            })
            .collect();

        let code_actions: List<LspCodeAction> = self
            .suggested_fixes()
            .iter()
            .filter(|fix| fix.is_machine_applicable)
            .map(|fix| {
                let mut changes = std::collections::HashMap::new();
                changes.insert(
                    format!("file://{}", fix.span.file).into(),
                    vec![LspTextEdit {
                        range: LspRange {
                            start: LspPosition {
                                line: fix.span.line.saturating_sub(1),
                                character: fix.span.column.saturating_sub(1),
                            },
                            end: LspPosition {
                                line: fix.span.end_line.unwrap_or(fix.span.line).saturating_sub(1),
                                character: fix.span.end_column.saturating_sub(1),
                            },
                        },
                        new_text: fix.replacement.clone(),
                    }]
                    .into(),
                );

                LspCodeAction {
                    title: fix.message.clone(),
                    kind: "quickfix".into(),
                    is_preferred: true,
                    edit: LspWorkspaceEdit { changes },
                }
            })
            .collect();

        LspDiagnostic {
            range,
            severity,
            code: self.code().map(|s| s.into()),
            source: "verum".into(),
            message: self.message().into(),
            related_information,
            code_actions,
        }
    }
}

/// Helper function for serde to check if a List is empty
fn list_is_empty<T>(list: &List<T>) -> bool {
    list.is_empty()
}

/// JSON representation of a diagnostic
#[derive(Debug, Serialize, Deserialize)]
struct JsonDiagnostic {
    level: Text,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<Text>,
    message: String,
    spans: List<JsonSpan>,
    #[serde(skip_serializing_if = "list_is_empty")]
    children: List<JsonDiagnostic>,
    #[serde(skip_serializing_if = "list_is_empty")]
    notes: List<Text>,
    #[serde(skip_serializing_if = "list_is_empty")]
    helps: List<Text>,
    #[serde(skip_serializing_if = "list_is_empty")]
    suggested_fixes: List<JsonSuggestedFix>,
    #[serde(skip_serializing_if = "Option::is_none")]
    doc_url: Option<Text>,
    is_fixable: bool,
    #[serde(skip_serializing_if = "list_is_empty")]
    related_files: List<Text>,
}

/// JSON representation of a suggested fix
#[derive(Debug, Serialize, Deserialize)]
struct JsonSuggestedFix {
    message: Text,
    span: JsonSpan,
    replacement: Text,
    is_machine_applicable: bool,
}

/// JSON representation of a span
#[derive(Debug, Serialize, Deserialize)]
struct JsonSpan {
    location: SourceLocation,
    label: Text,
    is_primary: bool,
}

/// JSON output containing multiple diagnostics
#[derive(Debug, Serialize, Deserialize)]
struct JsonOutput {
    diagnostics: List<JsonDiagnostic>,
    summary: JsonSummary,
}

/// Summary statistics for JSON output
#[derive(Debug, Serialize, Deserialize)]
struct JsonSummary {
    error_count: usize,
    warning_count: usize,
    note_count: usize,
    fixable_count: usize,
}
