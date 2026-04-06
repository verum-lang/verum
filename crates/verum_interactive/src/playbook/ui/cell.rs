//! Cell widget for rendering individual cells with inline output.
//!
//! Each cell renders as:
//! ┌ ▸ [3] ✓ source preview...                        0.2ms ┐
//! │   full source code with syntax highlighting             │
//! │   -> result : Type                                      │
//! │   stdout output here                                    │
//! └────────────────────────────────────────────────────────┘

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Widget};
use crate::playbook::session::{Cell, CellKind, CellOutput};
use super::output::{format_output_lines, output_line_count};

/// Calculate the height needed to render a cell (including borders and output).
pub fn cell_height(cell: &Cell, collapsed: bool) -> u16 {
    let border = 2u16; // top + bottom border

    if collapsed {
        return border + 1; // collapsed: single line
    }

    let source_lines = if cell.source.is_empty() {
        1
    } else {
        cell.source.as_str().lines().count().max(1)
    } as u16;

    let output_lines = if cell.output_collapsed {
        // Collapsed output: just 1-line summary + separator
        cell.output.as_ref().map_or(0, |_| 2) as u16
    } else {
        cell.output.as_ref().map_or(0, |o| {
            let n = output_line_count(o);
            if n > 0 { n + 1 } else { 0 } // +1 for separator line
        }) as u16
    };

    border + source_lines + output_lines
}

/// Status indicator for a cell.
fn status_indicator(cell: &Cell) -> (&'static str, Color) {
    if let Some(output) = &cell.output {
        if output.is_error() {
            ("!!", Color::Red) // error
        } else if cell.dirty {
            ("**", Color::Yellow) // dirty (modified since last run)
        } else {
            ("OK", Color::Green) // success
        }
    } else if cell.dirty && !cell.source.is_empty() {
        ("--", Color::DarkGray) // not executed, has content
    } else {
        ("  ", Color::DarkGray) // not executed
    }
}

/// Extract execution time from output if available.
fn exec_time_str(cell: &Cell) -> String {
    if let Some(output) = &cell.output {
        match output {
            CellOutput::Timing { execution_time_ms, .. } => format!("{}ms", execution_time_ms),
            CellOutput::Multi { outputs } => {
                for o in outputs {
                    if let CellOutput::Timing { execution_time_ms, .. } = o {
                        return format!("{}ms", execution_time_ms);
                    }
                }
                String::new()
            }
            _ => String::new(),
        }
    } else {
        String::new()
    }
}

/// Verum syntax highlighting for a single line (simplified, reusable).
fn highlight_verum_line(source: &str) -> Vec<Span<'_>> {
    // Keywords per grammar/verum.ebnf (reserved + primary + control + async + modifiers + ffi + module + additional + proof)
    let keywords = [
        // Reserved (3)
        "fn", "let", "is",
        // Primary (3)
        "type", "where", "using",
        // Control flow (9)
        "if", "else", "match", "return", "for", "while", "loop", "break", "continue",
        // Async/context (10)
        "async", "await", "spawn", "defer", "errdefer", "try", "yield", "throws", "select", "nursery",
        // Modifiers (5)
        "pub", "mut", "const", "unsafe", "pure",
        // FFI (1)
        "ffi",
        // Module (6)
        "module", "mount", "implement", "context", "protocol", "extends",
        // Additional (21)
        "self", "super", "cog", "static", "meta", "provide", "finally", "recover",
        "invariant", "decreases", "stream", "tensor", "affine", "linear",
        "public", "internal", "protected", "ensures", "requires", "result", "some",
        // Proof (17)
        "theorem", "lemma", "axiom", "corollary", "proof", "calc",
        "have", "show", "suffices", "obtain", "by", "qed",
        "induction", "cases", "contradiction", "forall", "exists",
    ];
    // Boolean literals (highlighted as keywords for visibility)
    let literals = ["true", "false"];
    // Primitive + stdlib types per grammar line 1032 + semantic types
    let types = [
        "Int", "Float", "Bool", "Char", "Text",
        "List", "Map", "Set", "Maybe", "Heap", "Shared",
        "Deque", "Channel", "Mutex", "Task", "Result",
        "Tensor", "Future", "Duration",
    ];

    let mut spans = Vec::new();
    let mut chars = source.char_indices().peekable();
    let mut current_start = 0;

    while let Some((i, c)) = chars.next() {
        if c == '@' {
            let start = i;
            let mut end = i + 1;
            while let Some(&(ni, nc)) = chars.peek() {
                if nc.is_alphanumeric() || nc == '_' { end = ni + nc.len_utf8(); chars.next(); } else { break; }
            }
            if start > current_start { spans.push(Span::raw(&source[current_start..start])); }
            spans.push(Span::styled(&source[start..end], Style::default().fg(Color::LightYellow)));
            current_start = end;
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            let mut end = i + c.len_utf8();
            while let Some(&(ni, nc)) = chars.peek() {
                if nc.is_alphanumeric() || nc == '_' { end = ni + nc.len_utf8(); chars.next(); } else { break; }
            }
            if start > current_start { spans.push(Span::raw(&source[current_start..start])); }
            let word = &source[start..end];
            let style = if keywords.contains(&word) {
                Style::default().fg(Color::Magenta).bold()
            } else if literals.contains(&word) {
                Style::default().fg(Color::LightBlue).bold()
            } else if types.contains(&word) {
                Style::default().fg(Color::Cyan)
            } else if word.starts_with(char::is_uppercase) {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };
            spans.push(Span::styled(word, style));
            current_start = end;
        } else if c == '"' {
            let start = i;
            let mut end = i + 1;
            let mut escaped = false;
            while let Some(&(ni, nc)) = chars.peek() {
                end = ni + nc.len_utf8(); chars.next();
                if escaped { escaped = false; } else if nc == '\\' { escaped = true; } else if nc == '"' { break; }
            }
            if start > current_start { spans.push(Span::raw(&source[current_start..start])); }
            spans.push(Span::styled(&source[start..end], Style::default().fg(Color::Yellow)));
            current_start = end;
        } else if c == '/' && source.get(i+1..i+2) == Some("/") {
            if i > current_start { spans.push(Span::raw(&source[current_start..i])); }
            spans.push(Span::styled(&source[i..], Style::default().fg(Color::DarkGray).italic()));
            current_start = source.len();
            while chars.next().is_some() {}
        } else if c.is_ascii_digit() {
            let start = i;
            let mut end = i + 1;
            while let Some(&(ni, nc)) = chars.peek() {
                if nc.is_ascii_digit() || nc == '.' || nc == '_' || nc == 'x' || nc == 'b' {
                    end = ni + nc.len_utf8(); chars.next();
                } else { break; }
            }
            if start > current_start { spans.push(Span::raw(&source[current_start..start])); }
            spans.push(Span::styled(&source[start..end], Style::default().fg(Color::LightBlue)));
            current_start = end;
        }
    }
    if current_start < source.len() { spans.push(Span::raw(&source[current_start..])); }
    if spans.is_empty() { spans.push(Span::raw(source)); }
    spans
}

/// Render a markdown line with basic inline formatting:
/// `**bold**`, `*italic*`, `` `code` ``, `# headings`, `- lists`, `> quotes`
fn render_markdown_line(line: &str) -> Line<'_> {
    // Headers
    if let Some(rest) = line.strip_prefix("### ") {
        return Line::from(Span::styled(
            rest,
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(rest) = line.strip_prefix("## ") {
        return Line::from(Span::styled(
            rest,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ));
    }
    if let Some(rest) = line.strip_prefix("# ") {
        return Line::from(Span::styled(
            rest,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ));
    }

    // Blockquote
    if let Some(rest) = line.strip_prefix("> ") {
        return Line::from(vec![
            Span::styled("▎ ", Style::default().fg(Color::DarkGray)),
            Span::styled(rest, Style::default().fg(Color::Gray).add_modifier(Modifier::ITALIC)),
        ]);
    }

    // Bullet list
    if line.starts_with("- ") || line.starts_with("* ") {
        return Line::from(vec![
            Span::styled("  • ", Style::default().fg(Color::Cyan)),
            Span::styled(&line[2..], Style::default().fg(Color::White)),
        ]);
    }

    // Numbered list (e.g. "1. ", "12. ")
    if let Some(dot_pos) = line.find(". ")
        && dot_pos <= 3 && line[..dot_pos].chars().all(|c| c.is_ascii_digit()) {
            return Line::from(vec![
                Span::styled(
                    format!("  {}. ", &line[..dot_pos]),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(&line[dot_pos + 2..], Style::default().fg(Color::White)),
            ]);
        }

    // Horizontal rule
    if line.trim() == "---" || line.trim() == "***" || line.trim() == "___" {
        return Line::from(Span::styled(
            "────────────────────────────────────",
            Style::default().fg(Color::DarkGray),
        ));
    }

    // Inline formatting: parse **bold**, *italic*, `code`
    let mut spans = Vec::new();
    let mut chars = line.char_indices().peekable();
    let mut current_start = 0;

    while let Some((i, c)) = chars.next() {
        if c == '`' {
            // Inline code
            if i > current_start {
                spans.push(Span::raw(&line[current_start..i]));
            }
            let code_start = i + 1;
            let mut code_end = code_start;
            for (j, c2) in chars.by_ref() {
                if c2 == '`' {
                    code_end = j;
                    break;
                }
                code_end = j + c2.len_utf8();
            }
            if code_end > code_start {
                spans.push(Span::styled(
                    &line[code_start..code_end],
                    Style::default().fg(Color::Yellow).bg(Color::Black),
                ));
            }
            current_start = code_end + 1;
        } else if c == '*' && line.get(i + 1..i + 2) == Some("*") {
            // Bold **text**
            if i > current_start {
                spans.push(Span::raw(&line[current_start..i]));
            }
            chars.next(); // skip second *
            let bold_start = i + 2;
            let mut bold_end = bold_start;
            while let Some((j, c2)) = chars.next() {
                if c2 == '*' && line.get(j + 1..j + 2) == Some("*") {
                    bold_end = j;
                    chars.next(); // skip second *
                    break;
                }
                bold_end = j + c2.len_utf8();
            }
            if bold_end > bold_start {
                spans.push(Span::styled(
                    &line[bold_start..bold_end],
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            }
            current_start = bold_end + 2;
        } else if c == '*' {
            // Italic *text*
            if i > current_start {
                spans.push(Span::raw(&line[current_start..i]));
            }
            let italic_start = i + 1;
            let mut italic_end = italic_start;
            for (j, c2) in chars.by_ref() {
                if c2 == '*' {
                    italic_end = j;
                    break;
                }
                italic_end = j + c2.len_utf8();
            }
            if italic_end > italic_start {
                spans.push(Span::styled(
                    &line[italic_start..italic_end],
                    Style::default().add_modifier(Modifier::ITALIC),
                ));
            }
            current_start = italic_end + 1;
        }
    }
    if current_start < line.len() {
        spans.push(Span::styled(
            &line[current_start..],
            Style::default().fg(Color::White),
        ));
    }
    if spans.is_empty() {
        Line::from(Span::styled(line, Style::default().fg(Color::White)))
    } else {
        Line::from(spans)
    }
}

/// Widget for rendering a single cell with inline output.
pub struct CellWidget<'a> {
    cell: &'a Cell,
    selected: bool,
    collapsed: bool,
    execution_number: Option<u32>,
}

impl<'a> CellWidget<'a> {
    pub fn new(cell: &'a Cell) -> Self {
        Self {
            cell,
            selected: false,
            collapsed: false,
            execution_number: None,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    pub fn execution_number(mut self, num: Option<u32>) -> Self {
        self.execution_number = num;
        self
    }
}

impl<'a> Widget for CellWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 2 || area.width < 10 {
            return;
        }

        // --- Border style based on state ---
        let border_style = if self.selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        // --- Build title ---
        let kind_icon = match self.cell.kind {
            CellKind::Code => "▸",
            CellKind::Markdown => "≡",
        };
        let exec_str = self.execution_number
            .map(|n| format!("[{}]", n))
            .unwrap_or_else(|| "[ ]".to_string());
        let (status, status_color) = status_indicator(self.cell);
        let time_str = exec_time_str(self.cell);

        // Title: "▸ [3] OK first_line_preview..."
        let first_line = self.cell.source.as_str().lines().next().unwrap_or("");
        let preview_max = (area.width as usize).saturating_sub(20);
        let preview = if first_line.len() > preview_max {
            format!("{}...", &first_line[..preview_max.saturating_sub(3)])
        } else {
            first_line.to_string()
        };

        let title_line = Line::from(vec![
            Span::styled(format!(" {} {} ", kind_icon, exec_str), border_style),
            Span::styled(format!("{} ", status), Style::default().fg(status_color)),
            Span::styled(preview, Style::default().fg(Color::White)),
        ]);

        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title_line);

        // Timing on the right side of top border
        if !time_str.is_empty() {
            block = block.title(
                Line::from(Span::styled(
                    format!(" {} ", time_str),
                    Style::default().fg(Color::DarkGray),
                ))
                .right_aligned(),
            );
        }

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        // --- Collapsed mode: show just the brief output summary ---
        if self.collapsed {
            if let Some(output) = &self.cell.output {
                let brief = super::output::format_output_brief(output);
                let line = Line::from(vec![
                    Span::styled("  -> ", Style::default().fg(Color::Cyan)),
                    Span::styled(brief, Style::default().fg(Color::DarkGray)),
                ]);
                buf.set_line(inner.x, inner.y, &line, inner.width);
            }
            return;
        }

        // --- Full mode: source + output ---
        let mut y = inner.y;
        let max_y = inner.y + inner.height;

        // Render source code with line numbers, or markdown with rich formatting
        let source = self.cell.source.as_str();
        let source_line_count = source.lines().count().max(1);
        let gutter_width = if self.cell.is_code() {
            // Gutter: "  1 │ " — adapt width to line count
            (source_line_count.to_string().len() + 3) as u16
        } else {
            2 // markdown gets simple indent
        };
        let code_width = inner.width.saturating_sub(gutter_width);

        if self.cell.is_code() {
            for (line_num, line) in source.lines().enumerate() {
                if y >= max_y { break; }
                // Line number gutter
                let num_str = format!(
                    "{:>width$} │ ",
                    line_num + 1,
                    width = (source_line_count.to_string().len())
                );
                buf.set_string(
                    inner.x,
                    y,
                    &num_str,
                    Style::default().fg(Color::DarkGray),
                );
                // Syntax-highlighted source
                let spans = highlight_verum_line(line);
                let styled_line = Line::from(spans);
                buf.set_line(inner.x + gutter_width, y, &styled_line, code_width);
                y += 1;
            }
            if source.is_empty() && y < max_y {
                buf.set_string(
                    inner.x,
                    y,
                    "  1 │ ",
                    Style::default().fg(Color::DarkGray),
                );
                y += 1;
            }
        } else {
            // Markdown rendering with inline formatting
            for line in source.lines() {
                if y >= max_y { break; }
                let styled_line = render_markdown_line(line);
                buf.set_line(inner.x + 1, y, &styled_line, inner.width.saturating_sub(1));
                y += 1;
            }
        }

        // Render output below the source, separated by a dim rule
        if let Some(output) = &self.cell.output {
            let output_lines = format_output_lines(output);
            if !output_lines.is_empty() && y < max_y {
                // ── separator with collapse indicator ──
                let indicator = if self.cell.output_collapsed { "[+]" } else { "[-]" };
                let rule_width = inner.width.saturating_sub(6) as usize;
                let separator = Line::from(vec![
                    Span::styled(
                        format!(" {}", "─".repeat(rule_width)),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!(" {} ", indicator),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
                buf.set_line(inner.x, y, &separator, inner.width);
                y += 1;

                if self.cell.output_collapsed {
                    // Single-line summary
                    if y < max_y {
                        let brief = super::output::format_output_brief(output);
                        let summary = Line::from(Span::styled(
                            format!(" {} ", brief),
                            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                        ));
                        buf.set_line(inner.x, y, &summary, inner.width);
                        let _ = y + 1; // height already accounted for in cell_height
                    }
                } else {
                    for line in output_lines {
                        if y >= max_y { break; }
                        buf.set_line(inner.x + 1, y, &line, inner.width.saturating_sub(1));
                        y += 1;
                    }
                }
            }
        }
    }
}
