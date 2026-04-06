//! Output widget for displaying cell execution results
//!
//! Handles rich output rendering including:
//! - Values with type information
//! - Tensors with shape and statistics
//! - Structured data (records, variants)
//! - Collections with truncation
//! - Errors with suggestions
//! - Stream output (stdout/stderr)
//! - Timing information

use ratatui::prelude::*;
use ratatui::widgets::{Paragraph, Widget};
use crate::playbook::session::CellOutput;

/// Widget for displaying cell output
pub struct OutputWidget<'a> {
    output: &'a CellOutput,
    /// Maximum lines to display before truncation
    max_lines: usize,
}

impl<'a> OutputWidget<'a> {
    pub fn new(output: &'a CellOutput) -> Self {
        Self {
            output,
            max_lines: 20,
        }
    }

    /// Set maximum lines to display
    pub fn max_lines(mut self, max: usize) -> Self {
        self.max_lines = max;
        self
    }

    /// Format a single output to lines
    fn format_output(output: &CellOutput) -> Vec<Line<'static>> {
        match output {
            CellOutput::Value { repr, type_info, .. } => {
                vec![Line::from(vec![
                    Span::styled("→ ", Style::default().fg(Color::Green)),
                    Span::styled(repr.to_string(), Style::default().fg(Color::White)),
                    Span::styled(" : ", Style::default().fg(Color::DarkGray)),
                    Span::styled(type_info.to_string(), Style::default().fg(Color::Cyan)),
                ])]
            }
            CellOutput::Tensor { shape, dtype, preview, stats } => {
                let mut lines = vec![];

                // Shape and type header
                let shape_str = format!("[{}]", shape.iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join(", "));
                lines.push(Line::from(vec![
                    Span::styled("→ Tensor<", Style::default().fg(Color::Cyan)),
                    Span::styled(dtype.to_string(), Style::default().fg(Color::Yellow)),
                    Span::styled(", ", Style::default().fg(Color::Cyan)),
                    Span::styled(shape_str, Style::default().fg(Color::White)),
                    Span::styled(">", Style::default().fg(Color::Cyan)),
                ]));

                // Statistics (if available)
                if let Some(stats) = stats {
                    let mut stat_parts = vec![];
                    if let Some(mean) = stats.mean {
                        stat_parts.push(format!("mean={:.4}", mean));
                    }
                    if let Some(std) = stats.std {
                        stat_parts.push(format!("std={:.4}", std));
                    }
                    if let Some(min) = stats.min {
                        stat_parts.push(format!("min={:.4}", min));
                    }
                    if let Some(max) = stats.max {
                        stat_parts.push(format!("max={:.4}", max));
                    }
                    if stats.nan_count > 0 {
                        stat_parts.push(format!("nan={}", stats.nan_count));
                    }
                    if stats.inf_count > 0 {
                        stat_parts.push(format!("inf={}", stats.inf_count));
                    }
                    if !stat_parts.is_empty() {
                        lines.push(Line::from(Span::styled(
                            format!("  {}", stat_parts.join(", ")),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                }

                // Data preview
                if !preview.is_empty() {
                    for line in preview.as_str().lines() {
                        lines.push(Line::from(Span::styled(
                            format!("  {}", line),
                            Style::default().fg(Color::White),
                        )));
                    }
                }

                lines
            }
            CellOutput::Structured { type_name, fields } => {
                let mut lines = vec![];
                lines.push(Line::from(vec![
                    Span::styled("→ ", Style::default().fg(Color::Green)),
                    Span::styled(type_name.to_string(), Style::default().fg(Color::Magenta)),
                    Span::styled(" {", Style::default().fg(Color::DarkGray)),
                ]));

                for (name, value) in fields {
                    let value_str = format_value_brief(value);
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(name.to_string(), Style::default().fg(Color::Blue)),
                        Span::styled(": ", Style::default().fg(Color::DarkGray)),
                        Span::styled(value_str, Style::default().fg(Color::White)),
                    ]));
                }

                lines.push(Line::from(Span::styled("}", Style::default().fg(Color::DarkGray))));
                lines
            }
            CellOutput::Collection { len, element_type, preview, truncated } => {
                let mut lines = vec![];

                // Header with length and type
                lines.push(Line::from(vec![
                    Span::styled("→ [", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{} elements", len),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled("] : List<", Style::default().fg(Color::DarkGray)),
                    Span::styled(element_type.to_string(), Style::default().fg(Color::Cyan)),
                    Span::styled(">", Style::default().fg(Color::DarkGray)),
                ]));

                // Preview elements
                for item in preview {
                    let item_str = format_value_brief(item);
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(item_str, Style::default().fg(Color::White)),
                    ]));
                }

                if *truncated {
                    lines.push(Line::from(Span::styled(
                        format!("  ... ({} more)", len - preview.len()),
                        Style::default().fg(Color::DarkGray),
                    )));
                }

                lines
            }
            CellOutput::Error { message, suggestions, .. } => {
                let mut lines = vec![Line::from(vec![
                    Span::styled("✗ ", Style::default().fg(Color::Red).bold()),
                    Span::styled(message.to_string(), Style::default().fg(Color::Red)),
                ])];

                // Add suggestions if available
                for suggestion in suggestions {
                    lines.push(Line::from(vec![
                        Span::styled("  💡 ", Style::default().fg(Color::Yellow)),
                        Span::styled(suggestion.to_string(), Style::default().fg(Color::Yellow)),
                    ]));
                }

                lines
            }
            CellOutput::Stream { stdout, stderr } => {
                let mut lines = vec![];

                // Stdout — green prefix, white text
                if !stdout.is_empty() {
                    for line in stdout.as_str().lines() {
                        lines.push(Line::from(vec![
                            Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                line.to_string(),
                                Style::default().fg(Color::Green),
                            ),
                        ]));
                    }
                }

                // Stderr — red
                if !stderr.is_empty() {
                    for line in stderr.as_str().lines() {
                        lines.push(Line::from(vec![
                            Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                line.to_string(),
                                Style::default().fg(Color::Red),
                            ),
                        ]));
                    }
                }

                lines
            }
            CellOutput::Timing { compile_time_ms, execution_time_ms } => {
                let time_str = if *execution_time_ms == 0 {
                    "< 1ms".to_string()
                } else if *execution_time_ms > 1000 {
                    format!("{:.1}s", *execution_time_ms as f64 / 1000.0)
                } else {
                    format!("{}ms", execution_time_ms)
                };
                vec![Line::from(Span::styled(
                    format!("⏱ {}", time_str),
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                ))]
            }
            CellOutput::Multi { outputs } => {
                let mut lines = vec![];
                for output in outputs {
                    lines.extend(Self::format_output(output));
                }
                lines
            }
            CellOutput::Empty => vec![],
        }
    }
}

/// Format a CellOutput value briefly (for nested display)
fn format_value_brief(output: &CellOutput) -> String {
    match output {
        CellOutput::Value { repr, .. } => repr.to_string(),
        CellOutput::Tensor { shape, dtype, .. } => {
            let shape_str = format!("[{}]", shape.iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join(", "));
            format!("Tensor<{}, {}>", dtype, shape_str)
        }
        CellOutput::Structured { type_name, fields } => {
            format!("{} {{ {} fields }}", type_name, fields.len())
        }
        CellOutput::Collection { len, element_type, .. } => {
            format!("[{} × {}]", len, element_type)
        }
        CellOutput::Error { message, .. } => format!("Error: {}", message),
        CellOutput::Stream { stdout, .. } => {
            let preview = stdout.as_str().lines().next().unwrap_or("");
            if preview.len() > 30 {
                format!("{}...", &preview[..30])
            } else {
                preview.to_string()
            }
        }
        CellOutput::Timing { execution_time_ms, .. } => format!("({}ms)", execution_time_ms),
        CellOutput::Multi { outputs } => format!("[{} outputs]", outputs.len()),
        CellOutput::Empty => "()".to_string(),
    }
}

impl<'a> Widget for OutputWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let lines = Self::format_output(self.output);

        if lines.is_empty() {
            return; // Don't render anything for empty output
        }

        // Truncate if too many lines
        let display_lines: Vec<Line> = if lines.len() > self.max_lines {
            let mut truncated = lines.into_iter().take(self.max_lines - 1).collect::<Vec<_>>();
            truncated.push(Line::from(Span::styled(
                "... (output truncated)",
                Style::default().fg(Color::DarkGray),
            )));
            truncated
        } else {
            lines
        };

        let text = Text::from(display_lines);
        let paragraph = Paragraph::new(text);
        paragraph.render(area, buf);
    }
}

/// Format output into styled lines (public API for cell widget inline rendering).
pub fn format_output_lines(output: &CellOutput) -> Vec<Line<'static>> {
    OutputWidget::format_output(output)
}

/// Format output as a brief single-line summary (for collapsed cells, sidebar).
pub fn format_output_brief(output: &CellOutput) -> String {
    format_value_brief(output)
}

/// Calculate the number of lines an output will use.
pub fn output_line_count(output: &CellOutput) -> usize {
    output_height(output)
}

/// Calculate the number of lines an output will use
pub fn output_height(output: &CellOutput) -> usize {
    match output {
        CellOutput::Empty => 0,
        CellOutput::Value { .. } => 1,
        CellOutput::Tensor { stats, preview, .. } => {
            let mut height = 1; // header
            if stats.is_some() {
                height += 1;
            }
            height += preview.as_str().lines().count();
            height
        }
        CellOutput::Structured { fields, .. } => 2 + fields.len(), // header + fields + closing
        CellOutput::Collection { preview, truncated, .. } => {
            1 + preview.len() + if *truncated { 1 } else { 0 }
        }
        CellOutput::Error { suggestions, .. } => 1 + suggestions.len(),
        CellOutput::Stream { stdout, stderr } => {
            stdout.as_str().lines().count() + stderr.as_str().lines().count()
        }
        CellOutput::Timing { .. } => 1,
        CellOutput::Multi { outputs } => outputs.iter().map(output_height).sum(),
    }
}
