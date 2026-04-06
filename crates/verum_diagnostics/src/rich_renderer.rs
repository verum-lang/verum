//! Rich diagnostic renderer with world-class error messages.
//!
//! Provides Rust/Elm-level error rendering with:
//! - Colored ANSI output
//! - Code snippets with line numbers
//! - Multi-line span support
//! - Diff-style suggestions
//! - Call chain visualization

use crate::colors::{Color, ColorScheme, GlyphSet, Style};
use crate::snippet_extractor::{Snippet, SnippetExtractor};
use crate::{Diagnostic, Label, Severity, Span, SpanLabel};
use std::cmp::max;
use std::io::{self, Write};
use verum_common::{List, Map, Text};

/// Configuration for rich diagnostic rendering
#[derive(Debug, Clone)]
pub struct RichRenderConfig {
    /// Color scheme
    pub color_scheme: ColorScheme,
    /// Glyph set (Unicode vs ASCII)
    pub glyphs: GlyphSet,
    /// Number of context lines before/after error
    pub context_lines: usize,
    /// Show line numbers
    pub show_line_numbers: bool,
    /// Maximum line width before truncation
    pub max_line_width: Option<usize>,
    /// Show source snippets
    pub show_source: bool,
}

impl RichRenderConfig {
    /// Default configuration with auto-detected colors and glyphs
    pub fn default() -> Self {
        Self {
            color_scheme: ColorScheme::auto(),
            glyphs: GlyphSet::auto(),
            context_lines: 2,
            show_line_numbers: true,
            max_line_width: Some(120),
            show_source: true,
        }
    }

    /// Configuration with colors disabled
    pub fn no_color() -> Self {
        Self {
            color_scheme: ColorScheme::no_color(),
            glyphs: GlyphSet::ascii(),
            context_lines: 2,
            show_line_numbers: true,
            max_line_width: Some(120),
            show_source: true,
        }
    }

    /// Minimal configuration for compact output
    pub fn minimal() -> Self {
        Self {
            color_scheme: ColorScheme::auto(),
            glyphs: GlyphSet::auto(),
            context_lines: 0,
            show_line_numbers: false,
            max_line_width: None,
            show_source: false,
        }
    }
}

/// Rich diagnostic renderer
pub struct RichRenderer {
    config: RichRenderConfig,
    extractor: SnippetExtractor,
}

impl RichRenderer {
    /// Create a new rich renderer
    pub fn new(config: RichRenderConfig) -> Self {
        Self {
            config,
            extractor: SnippetExtractor::new(),
        }
    }

    /// Create with default configuration
    pub fn default() -> Self {
        Self::new(RichRenderConfig::default())
    }

    /// Render a diagnostic to a string
    pub fn render(&mut self, diagnostic: &Diagnostic) -> Text {
        let mut output = String::new();
        self.render_diagnostic(diagnostic, &mut output, 0);
        output.into()
    }

    /// Render diagnostic to a writer
    pub fn render_to<W: Write>(
        &mut self,
        diagnostic: &Diagnostic,
        writer: &mut W,
    ) -> io::Result<()> {
        let rendered = self.render(diagnostic);
        write!(writer, "{}", rendered)
    }

    /// Internal: render a diagnostic with indentation
    fn render_diagnostic(&mut self, diagnostic: &Diagnostic, output: &mut String, indent: usize) {
        // 1. Render header (error[E0312]: message)
        self.render_header(diagnostic, output, indent);

        // 2. Render location line (  --> file:line:col)
        if !diagnostic.primary_labels().is_empty() {
            self.render_location_header(diagnostic, output, indent);
        }

        // 3. Render source snippets with underlines
        if self.config.show_source && !diagnostic.primary_labels().is_empty() {
            self.render_source_snippets(diagnostic, output, indent);
        }

        // 4. Render notes
        for note in diagnostic.notes() {
            self.render_note(note, output, indent);
        }

        // 5. Render help messages
        for help in diagnostic.helps() {
            self.render_help(help, output, indent);
        }

        // 6. Render children (nested diagnostics)
        for child in diagnostic.children() {
            output.push('\n');
            self.render_diagnostic(child, output, indent + 2);
        }
    }

    /// Render the diagnostic header: error[E0312]: message
    fn render_header(&self, diagnostic: &Diagnostic, output: &mut String, indent: usize) {
        let indent_str = " ".repeat(indent);

        let (severity_text, severity_color) = match diagnostic.severity() {
            Severity::Error => ("error", &self.config.color_scheme.severity_error),
            Severity::Warning => ("warning", &self.config.color_scheme.severity_warning),
            Severity::Note => ("note", &self.config.color_scheme.severity_note),
            Severity::Help => ("help", &self.config.color_scheme.severity_help),
        };

        let colored_severity = severity_color.wrap(severity_text);

        if let Some(code) = diagnostic.code() {
            let colored_code = self
                .config
                .color_scheme
                .error_code
                .wrap(&format!("[{}]", code));
            output.push_str(&format!(
                "{}{}{}: {}\n",
                indent_str,
                colored_severity,
                colored_code,
                diagnostic.message()
            ));
        } else {
            output.push_str(&format!(
                "{}{}: {}\n",
                indent_str,
                colored_severity,
                diagnostic.message()
            ));
        }
    }

    /// Render location header:   --> file.vr:42:15
    fn render_location_header(&self, diagnostic: &Diagnostic, output: &mut String, indent: usize) {
        if let Some(span) = diagnostic.primary_span() {
            let indent_str = " ".repeat(indent);
            let arrow = self.config.glyphs.arrow_right;
            let colored_arrow = self.config.color_scheme.gutter.wrap(arrow);
            let colored_path = self.config.color_scheme.file_path.wrap(&span.file);

            output.push_str(&format!(
                "{}  {} {}:{}:{}\n",
                indent_str, colored_arrow, colored_path, span.line, span.column
            ));
        }
    }

    /// Render source snippets with line numbers and underlines
    fn render_source_snippets(
        &mut self,
        diagnostic: &Diagnostic,
        output: &mut String,
        indent: usize,
    ) {
        let primary_labels = diagnostic.primary_labels();
        let secondary_labels = diagnostic.secondary_labels();

        if primary_labels.is_empty() {
            return;
        }

        // Group labels by file
        let mut labels_by_file: Map<Text, List<&SpanLabel>> = Map::new();

        for label in primary_labels {
            labels_by_file
                .entry(label.span.file.clone())
                .or_default()
                .push(label);
        }

        for label in secondary_labels {
            labels_by_file
                .entry(label.span.file.clone())
                .or_default()
                .push(label);
        }

        // Sort files to have primary labels first
        let mut files: List<Text> = labels_by_file.keys().cloned().collect();
        files.sort_by(|a: &Text, b: &Text| {
            let a_has_primary = primary_labels.iter().any(|l| &l.span.file == a);
            let b_has_primary = primary_labels.iter().any(|l| &l.span.file == b);
            match (a_has_primary, b_has_primary) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.cmp(b),
            }
        });

        // Render each file's snippets
        let mut is_first_file = true;
        for file in &files {
            if !is_first_file {
                // Add spacing between files and show the file path
                output.push('\n');
                let indent_str = " ".repeat(indent);
                let arrow = self.config.glyphs.arrow_right;
                let colored_arrow = self.config.color_scheme.gutter.wrap(arrow);
                let colored_path = self.config.color_scheme.file_path.wrap(file);

                // Get line number of first label in this file
                let labels = labels_by_file.get(file).unwrap();
                if let Some(first_label) = labels.first() {
                    output.push_str(&format!(
                        "{}  {} {}:{}:{}\n",
                        indent_str,
                        colored_arrow,
                        colored_path,
                        first_label.span.line,
                        first_label.span.column
                    ));
                }
            }
            is_first_file = false;

            let labels = labels_by_file.get(file).unwrap();

            // Check if labels are clustered or far apart
            if self.should_merge_snippets(labels) {
                // Render as a single merged snippet
                self.render_merged_snippet(file, labels, output, indent);
            } else {
                // Render as separate snippets
                self.render_separate_snippets(file, labels, output, indent);
            }
        }
    }

    /// Determine if multiple labels should be rendered in a single merged snippet
    fn should_merge_snippets(&self, labels: &[&SpanLabel]) -> bool {
        if labels.len() <= 1 {
            return true;
        }

        // Find min and max line numbers
        let min_line = labels.iter().map(|l| l.span.line).min().unwrap();
        let max_line = labels
            .iter()
            .map(|l| l.span.end_line.unwrap_or(l.span.line))
            .max()
            .unwrap();

        // Merge if within reasonable distance (including context)
        let total_lines = max_line - min_line + 1;
        let max_merged_lines = (self.config.context_lines * 2) + 10;

        total_lines <= max_merged_lines
    }

    /// Render multiple labels in a single merged snippet
    fn render_merged_snippet(
        &mut self,
        file: &str,
        labels: &[&SpanLabel],
        output: &mut String,
        indent: usize,
    ) {
        // Collect all spans
        let spans: List<Span> = labels.iter().map(|l| l.span.clone()).collect();

        match self.extractor.extract_multi_span_snippet(
            std::path::Path::new(file),
            &spans,
            self.config.context_lines,
        ) {
            Ok(snippet) => {
                self.render_multi_span_snippet(&snippet, labels, output, indent);
            }
            Err(_) => {
                let indent_str = " ".repeat(indent);
                output.push_str(&format!(
                    "{}  (source file not available: {})\n",
                    indent_str, file
                ));
            }
        }
    }

    /// Render labels as separate snippets
    fn render_separate_snippets(
        &mut self,
        file: &str,
        labels: &[&SpanLabel],
        output: &mut String,
        indent: usize,
    ) {
        let indent_str = " ".repeat(indent);

        // Render note about multiple locations
        if labels.len() > 1 {
            output.push_str(&format!(
                "{}  {}\n",
                indent_str,
                self.config
                    .color_scheme
                    .severity_note
                    .wrap("(showing multiple locations)")
            ));
        }

        for (idx, label) in labels.iter().enumerate() {
            if idx > 0 {
                // Add separator between snippets
                let gutter_width = label.span.line.to_string().len().max(3);
                output.push_str(&indent_str);
                output.push_str(&" ".repeat(gutter_width));
                output.push(' ');
                output.push_str(&self.config.color_scheme.gutter.wrap("..."));
                output.push('\n');
            }

            match self.extractor.extract_snippet(
                std::path::Path::new(file),
                &label.span,
                self.config.context_lines,
            ) {
                Ok(snippet) => {
                    self.render_snippet(&snippet, label, output, indent);
                }
                Err(_) => {
                    output.push_str(&format!("{}  (could not extract snippet)\n", indent_str));
                }
            }
        }
    }

    /// Render a multi-span snippet (multiple highlights in one view)
    fn render_multi_span_snippet(
        &self,
        snippet: &crate::snippet_extractor::MultiSpanSnippet,
        labels: &[&SpanLabel],
        output: &mut String,
        indent: usize,
    ) {
        let indent_str = " ".repeat(indent);
        let gutter_width = snippet.max_line_number_width();

        // Empty gutter line before snippet
        self.render_gutter_line(output, &indent_str, gutter_width, None);

        // Render each source line
        for line in &snippet.lines {
            // Line with number and content
            self.render_multi_span_source_line(output, &indent_str, gutter_width, line);

            // Render underlines for all spans on this line
            if !line.spans.is_empty() {
                // First, render all span underlines
                for span_on_line in &line.spans {
                    // Find ALL labels that match this span region (not just the first one)
                    for label in labels.iter().filter(|l| {
                        // Check if this label's span is on this line
                        l.span.line <= line.line_number
                            && l.span.end_line.unwrap_or(l.span.line) >= line.line_number
                            // Check if label's column falls within this span region
                            && span_on_line.start_col.is_none_or(|sc| l.span.column >= sc)
                            && span_on_line.end_col.is_none_or(|ec| l.span.column <= ec)
                    }) {
                        self.render_multi_span_underline(
                            output,
                            &indent_str,
                            gutter_width,
                            span_on_line,
                            line,
                            label,
                        );
                    }
                }
            }
        }

        // Empty gutter line after snippet
        self.render_gutter_line(output, &indent_str, gutter_width, None);
    }

    /// Render a source line for multi-span rendering
    fn render_multi_span_source_line(
        &self,
        output: &mut String,
        indent: &str,
        gutter_width: usize,
        line: &crate::snippet_extractor::MultiSpanSourceLine,
    ) {
        output.push_str(indent);

        // Line number
        let num_str = format!("{:>width$}", line.line_number, width = gutter_width);
        output.push_str(&self.config.color_scheme.line_number.wrap(&num_str));
        output.push(' ');

        // Gutter separator
        output.push_str(
            &self
                .config
                .color_scheme
                .gutter
                .wrap(self.config.glyphs.vertical_line),
        );
        output.push(' ');

        // Source content (with optional truncation)
        let content: String = if let Some(max_width) = self.config.max_line_width {
            if line.content.len() > max_width {
                format!("{}...", &line.content[..max_width - 3])
            } else {
                line.content.to_string()
            }
        } else {
            line.content.to_string()
        };

        output.push_str(&content);
        output.push('\n');
    }

    /// Render an underline for a span in multi-span mode
    fn render_multi_span_underline(
        &self,
        output: &mut String,
        indent: &str,
        gutter_width: usize,
        span_on_line: &crate::snippet_extractor::SpanOnLine,
        line: &crate::snippet_extractor::MultiSpanSourceLine,
        label: &SpanLabel,
    ) {
        output.push_str(indent);

        // Empty line number space
        output.push_str(&" ".repeat(gutter_width));
        output.push(' ');

        // Gutter separator
        output.push_str(
            &self
                .config
                .color_scheme
                .gutter
                .wrap(self.config.glyphs.vertical_line),
        );
        output.push(' ');

        // Calculate underline position and length
        let start_col = span_on_line.start_col.unwrap_or(0);
        let end_col = span_on_line.end_col.unwrap_or(line.content.len());
        let length = end_col.saturating_sub(start_col).max(1);

        // Leading spaces to align underline with span start
        output.push_str(&" ".repeat(start_col));

        // Underline characters
        let underline_char = self.config.glyphs.underline_char;
        let underline = underline_char.repeat(length);

        let color = if label.is_primary {
            &self.config.color_scheme.underline_primary
        } else {
            &self.config.color_scheme.underline_secondary
        };

        output.push_str(&color.wrap(&underline));

        // Label message (if present and this is the first line of the span)
        if !label.message.is_empty() && Some(line.line_number) == Some(label.span.line) {
            output.push(' ');
            output.push_str(&label.message);
        }

        output.push('\n');
    }

    /// Render a single snippet with line numbers and underlines
    fn render_snippet(
        &self,
        snippet: &Snippet,
        label: &SpanLabel,
        output: &mut String,
        indent: usize,
    ) {
        let indent_str = " ".repeat(indent);
        let gutter_width = snippet.max_line_number_width();

        // Empty gutter line before snippet
        self.render_gutter_line(output, &indent_str, gutter_width, None);

        // Render each source line
        for line in &snippet.lines {
            // Line with number and content
            self.render_source_line(output, &indent_str, gutter_width, line);

            // Underline if this line is in the span
            if line.is_in_span {
                self.render_underline(output, &indent_str, gutter_width, line, label);
            }
        }

        // Empty gutter line after snippet
        self.render_gutter_line(output, &indent_str, gutter_width, None);
    }

    /// Render a gutter line (empty or with separator)
    fn render_gutter_line(
        &self,
        output: &mut String,
        indent: &str,
        width: usize,
        line_num: Option<usize>,
    ) {
        output.push_str(indent);

        if let Some(num) = line_num {
            let num_str = format!("{:>width$}", num, width = width);
            output.push_str(&self.config.color_scheme.line_number.wrap(&num_str));
        } else {
            output.push_str(&" ".repeat(width));
        }

        output.push(' ');
        output.push_str(
            &self
                .config
                .color_scheme
                .gutter
                .wrap(self.config.glyphs.vertical_line),
        );
        output.push('\n');
    }

    /// Render a source line with line number and content
    fn render_source_line(
        &self,
        output: &mut String,
        indent: &str,
        gutter_width: usize,
        line: &crate::snippet_extractor::SourceLine,
    ) {
        output.push_str(indent);

        // Line number
        let num_str = format!("{:>width$}", line.line_number, width = gutter_width);
        output.push_str(&self.config.color_scheme.line_number.wrap(&num_str));
        output.push(' ');

        // Gutter separator
        output.push_str(
            &self
                .config
                .color_scheme
                .gutter
                .wrap(self.config.glyphs.vertical_line),
        );
        output.push(' ');

        // Source content (with optional truncation)
        let content: String = if let Some(max_width) = self.config.max_line_width {
            if line.content.len() > max_width {
                format!("{}...", &line.content[..max_width - 3])
            } else {
                line.content.to_string()
            }
        } else {
            line.content.to_string()
        };

        output.push_str(&content);
        output.push('\n');
    }

    /// Render an underline for a span on a line
    fn render_underline(
        &self,
        output: &mut String,
        indent: &str,
        gutter_width: usize,
        line: &crate::snippet_extractor::SourceLine,
        label: &SpanLabel,
    ) {
        output.push_str(indent);

        // Empty line number space
        output.push_str(&" ".repeat(gutter_width));
        output.push(' ');

        // Gutter separator
        output.push_str(
            &self
                .config
                .color_scheme
                .gutter
                .wrap(self.config.glyphs.vertical_line),
        );
        output.push(' ');

        // Leading spaces to align underline with span start
        let start_col = line.underline_start();
        output.push_str(&" ".repeat(start_col));

        // Underline characters
        let length = line.underline_length();
        let underline_char = self.config.glyphs.underline_char;
        let underline = underline_char.repeat(length);

        let color = if label.is_primary {
            &self.config.color_scheme.underline_primary
        } else {
            &self.config.color_scheme.underline_secondary
        };

        output.push_str(&color.wrap(&underline));

        // Label message (if present)
        if !label.message.is_empty() {
            output.push(' ');
            output.push_str(&label.message);
        }

        output.push('\n');
    }

    /// Render a note message
    fn render_note(&self, note: &Label, output: &mut String, indent: usize) {
        let indent_str = " ".repeat(indent);
        let prefix = self.config.color_scheme.severity_note.wrap("note");
        output.push_str(&format!("{}  {}: {}\n", indent_str, prefix, note.message));
    }

    /// Render a help message
    fn render_help(&self, help: &Label, output: &mut String, indent: usize) {
        let indent_str = " ".repeat(indent);
        let prefix = self.config.color_scheme.severity_help.wrap("help");
        output.push_str(&format!("{}  {}: {}\n", indent_str, prefix, help.message));
    }

    /// Add test content to the snippet extractor (for testing)
    pub fn add_test_content(&mut self, file: &str, content: &str) {
        self.extractor
            .add_source(std::path::PathBuf::from(file), content);
    }
}

/// Render a diff-style suggestion
pub struct DiffRenderer {
    color_scheme: ColorScheme,
    glyphs: GlyphSet,
}

impl DiffRenderer {
    pub fn new(color_scheme: ColorScheme, glyphs: GlyphSet) -> Self {
        Self {
            color_scheme,
            glyphs,
        }
    }

    /// Render a suggestion as a diff
    pub fn render_suggestion(&self, original: &str, suggested: &str, message: &str) -> String {
        let mut output = String::new();

        // Help header
        let prefix = self.color_scheme.severity_help.wrap("help");
        output.push_str(&format!("  {}: {}\n", prefix, message));

        // Show diff
        output.push_str("     ");
        output.push_str(&self.color_scheme.gutter.wrap(self.glyphs.vertical_line));
        output.push('\n');

        // Original line (red with -)
        output.push_str("     ");
        output.push_str(&self.color_scheme.gutter.wrap(self.glyphs.vertical_line));
        output.push(' ');
        output.push_str(
            &self
                .color_scheme
                .suggestion_remove
                .wrap(&format!("- {}", original)),
        );
        output.push('\n');

        // Suggested line (green with +)
        output.push_str("     ");
        output.push_str(&self.color_scheme.gutter.wrap(self.glyphs.vertical_line));
        output.push(' ');
        output.push_str(
            &self
                .color_scheme
                .suggestion_add
                .wrap(&format!("+ {}", suggested)),
        );
        output.push('\n');

        output
    }

    /// Render an inline suggestion (single-line change)
    pub fn render_inline_suggestion(
        &self,
        line: &str,
        span_start: usize,
        span_end: usize,
        replacement: &str,
        message: &str,
    ) -> String {
        let mut output = String::new();

        // Help header
        let prefix = self.color_scheme.severity_help.wrap("help");
        output.push_str(&format!("  {}: {}\n", prefix, message));

        // Show the modified line
        output.push_str("     ");
        output.push_str(&self.color_scheme.gutter.wrap(self.glyphs.vertical_line));
        output.push(' ');

        // Before span
        output.push_str(&line[..span_start]);

        // Replacement (highlighted)
        output.push_str(&self.color_scheme.suggestion_add.wrap(replacement));

        // After span
        output.push_str(&line[span_end..]);
        output.push('\n');

        // Underline the replacement
        output.push_str("     ");
        output.push_str(&self.color_scheme.gutter.wrap(self.glyphs.vertical_line));
        output.push(' ');
        output.push_str(&" ".repeat(span_start));
        let underline = self.glyphs.underline_char.repeat(replacement.len());
        output.push_str(&self.color_scheme.suggestion_add.wrap(&underline));
        output.push_str(" add this\n");

        output
    }
}

/// Render multi-line spans with connecting lines
pub struct MultiLineRenderer {
    color_scheme: ColorScheme,
    glyphs: GlyphSet,
}

impl MultiLineRenderer {
    pub fn new(color_scheme: ColorScheme, glyphs: GlyphSet) -> Self {
        Self {
            color_scheme,
            glyphs,
        }
    }

    /// Render a multi-line span with brackets
    pub fn render_multi_line_span(&self, snippet: &Snippet, message: &str) -> String {
        let mut output = String::new();
        let gutter_width = snippet.max_line_number_width();

        for (idx, line) in snippet.lines.iter().enumerate() {
            let is_first = idx == 0;
            let is_last = idx == snippet.lines.len() - 1;

            // Line number
            let num_str = format!("{:>width$}", line.line_number, width = gutter_width);
            output.push_str(&self.color_scheme.line_number.wrap(&num_str));
            output.push(' ');

            // Bracket
            if line.is_in_span {
                if is_first && is_last {
                    // Single line span
                    output.push_str(
                        &self
                            .color_scheme
                            .underline_primary
                            .wrap(self.glyphs.vertical_line),
                    );
                } else if is_first {
                    // Start of multi-line span
                    output.push_str(&self.color_scheme.underline_primary.wrap("/"));
                } else if is_last {
                    // End of multi-line span
                    output.push_str(
                        &self
                            .color_scheme
                            .underline_primary
                            .wrap(self.glyphs.vertical_line),
                    );
                } else {
                    // Middle of multi-line span
                    output.push_str(
                        &self
                            .color_scheme
                            .underline_primary
                            .wrap(self.glyphs.vertical_line),
                    );
                }
            } else {
                output.push_str(&self.color_scheme.gutter.wrap(self.glyphs.vertical_line));
            }

            output.push(' ');
            output.push_str(&line.content);
            output.push('\n');

            // Add underline for last line
            if is_last && line.is_in_span {
                output.push_str(&" ".repeat(gutter_width + 1));
                output.push_str(
                    &self
                        .color_scheme
                        .underline_primary
                        .wrap(self.glyphs.vertical_line),
                );
                let underline = self.glyphs.underline_char.repeat(line.underline_length());
                output.push(' ');
                output.push_str(&self.color_scheme.underline_primary.wrap(&underline));
                output.push(' ');
                output.push_str(message);
                output.push('\n');
            }
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Diagnostic, DiagnosticBuilder, Span, SpanLabel};

    fn create_test_span() -> Span {
        Span {
            file: "test.vr".into(),
            line: 2,
            column: 19,
            end_line: Some(2),
            end_column: 21,
        }
    }

    fn create_test_diagnostic() -> Diagnostic {
        DiagnosticBuilder::error()
            .code("E0312")
            .message("refinement constraint not satisfied")
            .span_label(create_test_span(), "value `-5` fails constraint `> 0`")
            .add_note("value has type `Int` but requires `Positive`")
            .help("use runtime check: `Positive::try_from(-5)?`")
            .build()
    }

    #[test]
    fn test_render_basic_diagnostic() {
        let mut renderer = RichRenderer::new(RichRenderConfig::no_color());
        renderer.add_test_content(
            "test.vr",
            "fn main() {\n    let x: Positive = -5;\n    println!(x);\n}\n",
        );

        let diagnostic = create_test_diagnostic();
        let output = renderer.render(&diagnostic);

        assert!(output.contains("error[E0312]"));
        assert!(output.contains("refinement constraint not satisfied"));
        assert!(output.contains("test.vr:2:19"));
    }

    #[test]
    fn test_render_with_colors() {
        // Force colors on (don't auto-detect, since tests run without TTY)
        let config = RichRenderConfig {
            color_scheme: ColorScheme::default_colors(),
            ..RichRenderConfig::default()
        };
        let mut renderer = RichRenderer::new(config);
        renderer.add_test_content("test.vr", "fn main() {\n    let x: Positive = -5;\n}\n");

        let diagnostic = create_test_diagnostic();
        let output = renderer.render(&diagnostic);

        // Should contain ANSI escape codes (forced on)
        assert!(
            output.contains("\x1b["),
            "Expected ANSI escape codes in output: {}",
            output
        );
    }

    #[test]
    fn test_render_without_source() {
        let config = RichRenderConfig {
            show_source: false,
            ..RichRenderConfig::no_color()
        };
        let mut renderer = RichRenderer::new(config);

        let diagnostic = create_test_diagnostic();
        let output = renderer.render(&diagnostic);

        assert!(output.contains("error[E0312]"));
        assert!(!output.contains("let x: Positive"));
    }

    #[test]
    fn test_diff_renderer() {
        let renderer = DiffRenderer::new(ColorScheme::no_color(), GlyphSet::ascii());

        let output = renderer.render_suggestion(
            "let x: Positive = -5;",
            "let x = Positive::try_from(-5)?;",
            "convert to checked type",
        );

        assert!(output.contains("help"));
        assert!(output.contains("- let x: Positive = -5;"));
        assert!(output.contains("+ let x = Positive::try_from(-5)?;"));
    }

    #[test]
    fn test_multi_line_rendering() {
        let mut renderer = RichRenderer::new(RichRenderConfig::no_color());
        let content =
            "fn test() {\nlet result = if condition {\n    value_a\n} else {\n    value_b\n};\n}";
        renderer.add_test_content("test.vr", content);

        let span = Span {
            file: "test.vr".into(),
            line: 2,
            column: 14,
            end_line: Some(6),
            end_column: 2,
        };

        let diagnostic = DiagnosticBuilder::error()
            .code("E0308")
            .message("type mismatch")
            .span_label(span, "expected `Int`, found `Float`")
            .build();

        let output = renderer.render(&diagnostic);

        assert!(output.contains("error[E0308]"));
        assert!(output.contains("type mismatch"));
    }

    #[test]
    fn test_minimal_config() {
        let config = RichRenderConfig::minimal();
        let mut renderer = RichRenderer::new(config);

        let diagnostic = create_test_diagnostic();
        let output = renderer.render(&diagnostic);

        assert!(output.contains("error[E0312]"));
        // Should be more compact
        assert!(output.len() < 500);
    }
}
