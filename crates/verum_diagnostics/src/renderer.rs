//! Pretty-printing and rendering of diagnostics with colors and source context.
//!
//! This module provides beautiful, Rust-like error rendering with:
//! - Color-coded output
//! - Source code snippets with line numbers
//! - Multi-line span support
//! - Gutter decorations
//! - Call chain visualization for context errors

use crate::{Diagnostic, Label, Severity, SpanLabel};
use colored::*;
use std::cmp::{max, min};
use std::fs;
use std::io::{self, Write};
use verum_common::{List, Map, Text};

/// Configuration for rendering diagnostics
#[derive(Debug, Clone)]
pub struct RenderConfig {
    /// Enable colored output
    pub colors: bool,
    /// Number of context lines before/after the error
    pub context_lines: usize,
    /// Show line numbers
    pub show_line_numbers: bool,
    /// Maximum line width before truncation
    pub max_line_width: usize,
    /// Show source snippets
    pub show_source: bool,
    /// Show suggested fixes
    pub show_suggestions: bool,
    /// Show documentation URLs
    pub show_doc_urls: bool,
    /// Enable unicode box drawing characters
    pub unicode_output: bool,
    /// Terminal width for wrapping (0 = no wrapping)
    pub terminal_width: usize,
    /// Show file paths as relative paths when possible
    pub relative_paths: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            colors: true,
            context_lines: 2,
            show_line_numbers: true,
            max_line_width: 120,
            show_source: true,
            show_suggestions: true,
            show_doc_urls: true,
            unicode_output: true,
            terminal_width: 80,
            relative_paths: false,
        }
    }
}

impl RenderConfig {
    /// Create config without colors
    pub fn no_color() -> Self {
        Self {
            colors: false,
            ..Default::default()
        }
    }

    /// Create minimal config (no source, no suggestions)
    pub fn minimal() -> Self {
        Self {
            context_lines: 0,
            show_source: false,
            show_suggestions: false,
            show_doc_urls: false,
            ..Default::default()
        }
    }

    /// Create a config optimized for IDE/LSP output
    pub fn for_ide() -> Self {
        Self {
            colors: false,
            context_lines: 1,
            show_line_numbers: true,
            max_line_width: 200,
            show_source: true,
            show_suggestions: true,
            show_doc_urls: true,
            unicode_output: false,
            terminal_width: 0,
            relative_paths: true,
        }
    }

    /// Create a config for compact terminal output
    pub fn compact() -> Self {
        Self {
            context_lines: 0,
            show_source: true,
            show_suggestions: false,
            show_doc_urls: false,
            ..Default::default()
        }
    }

    /// Create a config for verbose output with more context
    pub fn verbose() -> Self {
        Self {
            context_lines: 4,
            show_source: true,
            show_suggestions: true,
            show_doc_urls: true,
            terminal_width: 100,
            ..Default::default()
        }
    }

    /// Builder: Set terminal width
    pub fn with_terminal_width(mut self, width: usize) -> Self {
        self.terminal_width = width;
        self
    }

    /// Builder: Set relative paths
    pub fn with_relative_paths(mut self, relative: bool) -> Self {
        self.relative_paths = relative;
        self
    }
}

/// Renderer for diagnostics
pub struct Renderer {
    config: RenderConfig,
    /// Cache of loaded source files
    file_cache: Map<Text, List<Text>>,
}

impl Renderer {
    pub fn new(config: RenderConfig) -> Self {
        Self {
            config,
            file_cache: Map::new(),
        }
    }

    pub fn default() -> Self {
        Self::new(RenderConfig::default())
    }

    /// Render a diagnostic to a string
    pub fn render(&mut self, diagnostic: &Diagnostic) -> Text {
        let mut output = Text::new();
        self.render_to_string(diagnostic, &mut output);
        output
    }

    /// Render a diagnostic to a writer
    pub fn render_to<W: Write>(
        &mut self,
        diagnostic: &Diagnostic,
        writer: &mut W,
    ) -> io::Result<()> {
        let rendered = self.render(diagnostic);
        write!(writer, "{}", rendered)
    }

    /// Internal rendering to string
    fn render_to_string(&mut self, diagnostic: &Diagnostic, output: &mut Text) {
        // Render header
        self.render_header(diagnostic, output);

        // Render source snippets
        if self.config.show_source {
            self.render_source_snippets(diagnostic, output);
        }

        // Render notes
        for note in diagnostic.notes() {
            self.render_label_line(Severity::Note, note, output);
        }

        // Render help messages
        for help in diagnostic.helps() {
            self.render_label_line(Severity::Help, help, output);
        }

        // Render suggested fixes
        if self.config.show_suggestions {
            self.render_suggested_fixes(diagnostic, output);
        }

        // Render documentation URL
        if self.config.show_doc_urls {
            if let Some(url) = diagnostic.doc_url() {
                output.push_str(&format!(
                    "  {}: for more information, see {}\n",
                    self.colorize("info", Color::Cyan),
                    self.colorize(url, Color::Blue)
                ));
            }
        }

        // Render children
        for child in diagnostic.children() {
            output.push('\n');
            self.render_to_string(child, output);
        }
    }

    /// Render suggested fixes for the diagnostic
    fn render_suggested_fixes(&self, diagnostic: &Diagnostic, output: &mut Text) {
        let fixes = diagnostic.suggested_fixes();
        if fixes.is_empty() {
            return;
        }

        for fix in fixes {
            let applicability = if fix.is_machine_applicable {
                self.colorize("[auto-fix]", Color::Green)
            } else {
                self.colorize("[manual]", Color::Yellow)
            };

            output.push_str(&format!(
                "  {}: {} {}\n",
                self.colorize("suggestion", Color::Cyan),
                fix.message,
                applicability
            ));

            // Show the replacement code if not empty
            if !fix.replacement.is_empty() {
                // Render the replacement with box drawing
                let box_top = if self.config.unicode_output {
                    "    ╭─ replacement"
                } else {
                    "    +-- replacement"
                };
                let box_side = if self.config.unicode_output {
                    "    │ "
                } else {
                    "    | "
                };
                let box_bottom = if self.config.unicode_output {
                    "    ╰─"
                } else {
                    "    +--"
                };

                output.push_str(&self.colorize(box_top, Color::Green));
                output.push('\n');

                for line in fix.replacement.lines() {
                    output.push_str(&self.colorize(box_side, Color::Green));
                    output.push_str(&self.colorize(&line, Color::Green));
                    output.push('\n');
                }

                output.push_str(&self.colorize(box_bottom, Color::Green));
                output.push('\n');
            }
        }
    }

    /// Render the diagnostic header
    fn render_header(&self, diagnostic: &Diagnostic, output: &mut Text) {
        let severity_str = self.format_severity(diagnostic.severity());
        let code_str: Text = if let Some(code) = diagnostic.code() {
            format!("<{}>", code).into()
        } else {
            Text::new()
        };

        output.push_str(&format!(
            "{}{}: {}\n",
            severity_str,
            code_str,
            diagnostic.message()
        ));
    }

    /// Render source snippets with spans
    fn render_source_snippets(&mut self, diagnostic: &Diagnostic, output: &mut Text) {
        let primary_labels = diagnostic.primary_labels();
        let secondary_labels = diagnostic.secondary_labels();

        if primary_labels.is_empty() {
            return;
        }

        // Group labels by file
        let mut by_file: Map<Text, List<&SpanLabel>> = Map::new();
        for label in primary_labels.iter().chain(secondary_labels.iter()) {
            by_file
                .entry(label.span.file.clone())
                .or_default()
                .push(label);
        }

        // Render each file's snippets
        for (file, labels) in by_file {
            self.render_file_snippet(&file, labels, output);
        }
    }

    /// Render a snippet from a specific file
    fn render_file_snippet(&mut self, file: &str, labels: List<&SpanLabel>, output: &mut Text) {
        // Get the first label for location info (always available since labels is non-empty)
        let first_label = labels.first().unwrap();

        // Always render location header first
        output.push_str(&format!(
            "  {} {}:{}:{}\n",
            self.colorize("-->", Color::Blue),
            file,
            first_label.span.line,
            first_label.span.column
        ));

        // Load file content
        let lines_len = match self.load_file(file) {
            Some(lines) => lines.len(),
            None => {
                // File not found - we've already shown the location, just return
                return;
            }
        };

        // Find the range of lines to display
        let (start_line, end_line) = self.compute_line_range(&labels, lines_len);

        // Calculate gutter width (for line numbers)
        let gutter_width = format!("{}", end_line).len();

        // Render empty gutter line (location header was already rendered above)
        self.render_gutter(output, gutter_width, None, GutterStyle::Empty);
        output.push('\n');

        // Get lines again (borrow checker) and clone them
        let lines: List<Text> = self.load_file(file).unwrap().clone();

        // Render source lines
        for line_num in start_line..=end_line {
            let line_idx = line_num - 1;
            if line_idx >= lines.len() {
                break;
            }

            let line_content = &lines[line_idx];

            // Find labels for this line
            let line_labels: List<_> = labels
                .iter()
                .filter(|l| l.span.line == line_num)
                .copied()
                .collect();

            // Render line with gutter
            self.render_gutter(output, gutter_width, Some(line_num), GutterStyle::Line);
            output.push_str(line_content);
            output.push('\n');

            // Render underlines and labels
            if !line_labels.is_empty() {
                self.render_underlines(output, gutter_width, line_content, &line_labels);
            }
        }

        // Render empty gutter line
        self.render_gutter(output, gutter_width, None, GutterStyle::Empty);
        output.push('\n');
    }

    /// Render underlines for labels on a line
    fn render_underlines(
        &self,
        output: &mut Text,
        gutter_width: usize,
        line_content: &str,
        labels: &[&SpanLabel],
    ) {
        for label in labels {
            // Render gutter
            self.render_gutter(output, gutter_width, None, GutterStyle::Annotation);

            // Add spacing to column position
            for (i, ch) in line_content.chars().enumerate() {
                if i + 1 >= label.span.column {
                    break;
                }
                if ch == '\t' {
                    output.push('\t');
                } else {
                    output.push(' ');
                }
            }

            // Render underline
            let length = max(1, label.span.length());
            let underline = if label.is_primary {
                "^".repeat(length)
            } else {
                "-".repeat(length)
            };

            let color = if label.is_primary {
                Color::Red
            } else {
                Color::Blue
            };

            output.push_str(&self.colorize(&underline, color));

            // Add label message if present
            if !label.message.is_empty() {
                output.push_str(&format!(" {}", label.message));
            }

            output.push('\n');
        }
    }

    /// Render the gutter (line numbers and decorations)
    fn render_gutter(
        &self,
        output: &mut Text,
        width: usize,
        line_num: Option<usize>,
        style: GutterStyle,
    ) {
        match style {
            GutterStyle::Line => {
                if let Some(num) = line_num {
                    let num_str = format!("{:>width$}", num, width = width);
                    output.push_str(&self.colorize(&num_str, Color::Blue));
                    output.push_str(&self.colorize(" │ ", Color::Blue));
                }
            }
            GutterStyle::Empty => {
                let spaces = " ".repeat(width);
                output.push_str(&spaces);
                output.push_str(&self.colorize(" │", Color::Blue));
            }
            GutterStyle::Annotation => {
                let spaces = " ".repeat(width);
                output.push_str(&spaces);
                output.push_str(&self.colorize(" │ ", Color::Blue));
            }
        }
    }

    /// Compute the range of lines to display given labels
    fn compute_line_range(&self, labels: &[&SpanLabel], max_line: usize) -> (usize, usize) {
        if labels.is_empty() {
            return (1, 1);
        }

        let min_line = labels.iter().map(|l| l.span.line).min().unwrap_or(1);

        let max_line_in_labels = labels
            .iter()
            .map(|l| l.span.end_line.unwrap_or(l.span.line))
            .max()
            .unwrap_or(min_line);

        let start = min_line.saturating_sub(self.config.context_lines).max(1);
        let end = min(max_line_in_labels + self.config.context_lines, max_line);

        (start, end)
    }

    /// Load file content into cache
    fn load_file(&mut self, file: &str) -> Option<&List<Text>> {
        let file_text: Text = file.into();
        if !self.file_cache.contains_key(&file_text) {
            let content = fs::read_to_string(file).ok()?;
            let lines: List<Text> = content.lines().map(Text::from).collect();
            self.file_cache.insert(file_text.clone(), lines);
        }
        self.file_cache.get(&file_text)
    }

    /// Add test content to cache (for testing)
    pub fn add_test_content(&mut self, file: &str, content: &str) {
        let lines: List<Text> = content.lines().map(Text::from).collect();
        self.file_cache.insert(file.into(), lines);
    }

    /// Format severity with color
    fn format_severity(&self, severity: Severity) -> Text {
        let (text, color) = match severity {
            Severity::Error => ("error", Color::Red),
            Severity::Warning => ("warning", Color::Yellow),
            Severity::Note => ("note", Color::Green),
            Severity::Help => ("help", Color::Cyan),
        };
        self.colorize(text, color)
    }

    /// Render a simple label line (for notes/help)
    fn render_label_line(&self, severity: Severity, label: &Label, output: &mut Text) {
        let prefix = self.format_severity(severity);

        // Check if this is a call chain note (contains special markers)
        if label.message.contains("call chain") || label.message.contains("└─>") {
            // Enhanced rendering for call chains
            self.render_call_chain_note(&label.message, output);
        } else {
            // Standard note rendering
            output.push_str(&format!("  {}: {}\n", prefix, label.message));
        }
    }

    /// Render a call chain note with special formatting
    fn render_call_chain_note(&self, message: &str, output: &mut Text) {
        let lines: List<&str> = message.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            if i == 0 {
                // First line - "call chain requiring 'Context':"
                let prefix = self.colorize("note", Color::Green);
                output.push_str(&format!("  {}: {}\n", prefix, line));
            } else if line.contains("└─>") {
                // Chain continuation with arrow
                let parts: List<&str> = line.split("└─>").collect();
                if parts.len() == 2 {
                    let indent = parts[0];
                    let content = parts[1];

                    // Highlight the arrow
                    output.push_str(indent);
                    output.push_str(&self.colorize("└─>", Color::Blue));

                    // Highlight context requirements in brackets
                    if content.contains("[requires") {
                        let parts: List<&str> = content.splitn(2, "[requires").collect();
                        output.push_str(parts[0]);
                        if parts.len() == 2 {
                            output.push_str(
                                &self.colorize(&format!("[requires{}", parts[1]), Color::Cyan),
                            );
                        }
                    } else {
                        output.push_str(content);
                    }
                    output.push('\n');
                } else {
                    output.push_str(line);
                    output.push('\n');
                }
            } else {
                // Regular continuation line
                output.push_str(line);
                output.push('\n');
            }
        }
    }

    /// Apply color if enabled
    fn colorize(&self, text: &str, color: Color) -> Text {
        if !self.config.colors {
            return text.into();
        }

        match color {
            Color::Red => text.red().bold().to_string().into(),
            Color::Yellow => text.yellow().bold().to_string().into(),
            Color::Green => text.green().to_string().into(),
            Color::Blue => text.blue().bold().to_string().into(),
            Color::Cyan => text.cyan().bold().to_string().into(),
        }
    }
}

/// Color options
#[derive(Debug, Clone, Copy)]
enum Color {
    Red,
    Yellow,
    Green,
    Blue,
    Cyan,
}

/// Gutter rendering styles
#[derive(Debug, Clone, Copy)]
enum GutterStyle {
    /// Regular line with number
    Line,
    /// Empty gutter (no number)
    Empty,
    /// Annotation line (for underlines)
    Annotation,
}

/// Diff-style renderer for showing changes
pub struct DiffRenderer {
    config: RenderConfig,
}

impl DiffRenderer {
    /// Create a new diff renderer
    pub fn new(config: RenderConfig) -> Self {
        Self { config }
    }

    /// Render a before/after diff for a suggested fix
    pub fn render_diff(&self, before: &str, after: &str, context_lines: usize) -> Text {
        let mut output = Text::new();
        let before_lines: List<&str> = before.lines().collect();
        let after_lines: List<&str> = after.lines().collect();

        // Simple line-by-line diff
        let max_len = std::cmp::max(before_lines.len(), after_lines.len());

        for i in 0..max_len {
            let before_line = before_lines.get(i).copied().unwrap_or("");
            let after_line = after_lines.get(i).copied().unwrap_or("");

            if before_line == after_line {
                // Context line
                output.push_str(&format!("  {} | {}\n", i + 1, before_line));
            } else {
                // Changed line
                if !before_line.is_empty() {
                    if self.config.colors {
                        output.push_str(&format!(
                            "{}",
                            format!("- {} | {}\n", i + 1, before_line).red()
                        ));
                    } else {
                        output.push_str(&format!("- {} | {}\n", i + 1, before_line));
                    }
                }
                if !after_line.is_empty() {
                    if self.config.colors {
                        output.push_str(&format!(
                            "{}",
                            format!("+ {} | {}\n", i + 1, after_line).green()
                        ));
                    } else {
                        output.push_str(&format!("+ {} | {}\n", i + 1, after_line));
                    }
                }
            }
        }

        output
    }
}

/// Batch renderer for multiple diagnostics
pub struct BatchRenderer {
    config: RenderConfig,
    renderer: Renderer,
}

impl BatchRenderer {
    /// Create a new batch renderer
    pub fn new(config: RenderConfig) -> Self {
        let renderer = Renderer::new(config.clone());
        Self { config, renderer }
    }

    /// Render multiple diagnostics with a summary
    pub fn render_all(&mut self, diagnostics: &[crate::Diagnostic]) -> Text {
        let mut output = Text::new();
        let mut error_count = 0;
        let mut warning_count = 0;

        for diagnostic in diagnostics {
            output.push_str(&self.renderer.render(diagnostic));
            output.push('\n');

            if diagnostic.is_error() {
                error_count += 1;
            } else if diagnostic.is_warning() {
                warning_count += 1;
            }
        }

        // Add summary
        if error_count > 0 || warning_count > 0 {
            output.push_str(&self.render_summary(error_count, warning_count));
        }

        output
    }

    /// Render a summary line
    fn render_summary(&self, errors: usize, warnings: usize) -> Text {
        let mut parts: List<Text> = List::new();

        if errors > 0 {
            let error_str = format!("{} error{}", errors, if errors == 1 { "" } else { "s" });
            if self.config.colors {
                parts.push(error_str.red().bold().to_string().into());
            } else {
                parts.push(error_str.into());
            }
        }

        if warnings > 0 {
            let warning_str = format!(
                "{} warning{}",
                warnings,
                if warnings == 1 { "" } else { "s" }
            );
            if self.config.colors {
                parts.push(warning_str.yellow().bold().to_string().into());
            } else {
                parts.push(warning_str.into());
            }
        }

        if parts.is_empty() {
            Text::new()
        } else {
            format!(
                "{}: {}\n",
                if self.config.colors {
                    "summary".blue().bold().to_string()
                } else {
                    "summary".to_string()
                },
                parts.join(", ")
            )
            .into()
        }
    }

    /// Render diagnostics grouped by file
    pub fn render_by_file(&mut self, diagnostics: &[crate::Diagnostic]) -> Text {
        let mut by_file: Map<Text, List<&crate::Diagnostic>> = Map::new();

        for diagnostic in diagnostics {
            let file: Text = diagnostic
                .primary_span()
                .map(|s| s.file.clone())
                .unwrap_or_else(|| "<no file>".into());
            by_file.entry(file).or_default().push(diagnostic);
        }

        let mut output = Text::new();

        for (file, file_diagnostics) in by_file {
            // File header
            if self.config.colors {
                output.push_str(&format!("{}\n", file.blue().bold()));
            } else {
                output.push_str(&format!("{}\n", file));
            }
            output.push_str(&"─".repeat(min(file.len(), 60)));
            output.push('\n');

            for diagnostic in file_diagnostics {
                output.push_str(&self.renderer.render(diagnostic));
                output.push('\n');
            }
        }

        output
    }
}
