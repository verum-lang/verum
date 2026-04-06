//! Sidebar widget with tabbed panels: Variables, Outline, Stats.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Tabs, Widget};
use crate::playbook::session::CellKind;

/// Sidebar tab selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarTab {
    #[default]
    Variables,
    Outline,
    DevTools,
}

impl SidebarTab {
    pub fn next(self) -> Self {
        match self {
            Self::Variables => Self::Outline,
            Self::Outline => Self::DevTools,
            Self::DevTools => Self::Variables,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            Self::Variables => Self::DevTools,
            Self::Outline => Self::Variables,
            Self::DevTools => Self::Outline,
        }
    }
    pub fn index(self) -> usize {
        match self {
            Self::Variables => 0,
            Self::Outline => 1,
            Self::DevTools => 2,
        }
    }
}

/// Variable info for sidebar display.
#[derive(Debug, Clone)]
pub struct VarInfo {
    pub name: String,
    pub type_info: String,
    pub value_preview: String,
    pub is_mutable: bool,
}

/// Function info for sidebar display.
#[derive(Debug, Clone)]
pub struct FuncInfo {
    pub name: String,
    pub signature: String,
}

/// Cell outline entry.
#[derive(Debug, Clone)]
pub struct OutlineEntry {
    pub index: usize,
    pub kind: CellKind,
    pub exec_number: Option<u32>,
    pub first_line: String,
    pub has_error: bool,
    pub is_dirty: bool,
    pub is_selected: bool,
}

/// Execution stats for DevTools tab.
#[derive(Debug, Clone, Default)]
pub struct ExecStats {
    pub total_cells: usize,
    pub code_cells: usize,
    pub markdown_cells: usize,
    pub executed_count: usize,
    pub error_count: usize,
    pub binding_count: usize,
    pub function_count: usize,
    pub last_cell_source: String,
    pub last_exec_time_ms: f64,
    pub last_instructions: u64,
    pub last_peak_stack: usize,
}

/// Sidebar widget with tabbed panels.
pub struct SidebarWidget<'a> {
    tab: SidebarTab,
    variables: &'a [VarInfo],
    functions: &'a [FuncInfo],
    outline: &'a [OutlineEntry],
    stats: ExecStats,
}

impl<'a> SidebarWidget<'a> {
    pub fn new() -> Self {
        Self {
            tab: SidebarTab::Variables,
            variables: &[],
            functions: &[],
            outline: &[],
            stats: ExecStats::default(),
        }
    }

    pub fn tab(mut self, tab: SidebarTab) -> Self { self.tab = tab; self }
    pub fn variables(mut self, vars: &'a [VarInfo]) -> Self { self.variables = vars; self }
    pub fn functions(mut self, funcs: &'a [FuncInfo]) -> Self { self.functions = funcs; self }
    pub fn outline(mut self, entries: &'a [OutlineEntry]) -> Self { self.outline = entries; self }
    pub fn stats(mut self, stats: ExecStats) -> Self { self.stats = stats; self }
    pub fn cell_info(self, _count: usize, _selected: usize) -> Self { self }

    fn render_variables(&self, area: Rect, buf: &mut Buffer) {
        let mut lines: Vec<Line> = Vec::new();

        if self.variables.is_empty() && self.functions.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no bindings yet)",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Run a cell with `let x = 1`",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(Span::styled(
                "  to see variables here.",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            // Variables section
            if !self.variables.is_empty() {
                for var in self.variables {
                    // Name : Type
                    let mut name_spans = vec![];
                    if var.is_mutable {
                        name_spans.push(Span::styled("mut ", Style::default().fg(Color::Yellow)));
                    }
                    name_spans.push(Span::styled(&var.name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));
                    name_spans.push(Span::styled(" : ", Style::default().fg(Color::DarkGray)));
                    name_spans.push(Span::styled(&var.type_info, Style::default().fg(Color::Cyan)));
                    lines.push(Line::from(name_spans));

                    // Value preview — allow up to sidebar width
                    if !var.value_preview.is_empty() {
                        let max_width = area.width.saturating_sub(4) as usize;
                        let preview = if var.value_preview.len() > max_width {
                            format!("  = {}…", &var.value_preview[..max_width.saturating_sub(4)])
                        } else {
                            format!("  = {}", &var.value_preview)
                        };
                        lines.push(Line::from(Span::styled(
                            preview,
                            Style::default().fg(Color::Green),
                        )));
                    }
                }
            }

            // Functions section
            if !self.functions.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "╌╌ functions ╌╌",
                    Style::default().fg(Color::Magenta),
                )));
                for func in self.functions {
                    lines.push(Line::from(vec![
                        Span::styled("fn ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
                        Span::styled(&func.name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                        Span::styled(&func.signature, Style::default().fg(Color::DarkGray)),
                    ]));
                }
            }
        }

        Paragraph::new(lines).render(area, buf);
    }

    fn render_outline(&self, area: Rect, buf: &mut Buffer) {
        let mut lines: Vec<Line> = Vec::new();

        if self.outline.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no cells)",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
            )));
        } else {
            for entry in self.outline {
                let icon = match entry.kind {
                    CellKind::Code => "▸",
                    CellKind::Markdown => "≡",
                };
                let num = format!("{:>2}", entry.index + 1);

                let status = if entry.has_error {
                    Span::styled(" ✗ ", Style::default().fg(Color::Red))
                } else if entry.is_dirty {
                    Span::styled(" ● ", Style::default().fg(Color::Yellow))
                } else if entry.exec_number.is_some() {
                    Span::styled(" ✓ ", Style::default().fg(Color::Green))
                } else {
                    Span::styled(" · ", Style::default().fg(Color::DarkGray))
                };

                let max_preview = area.width.saturating_sub(12) as usize;
                let fl = if entry.first_line.len() > max_preview {
                    format!("{}…", &entry.first_line[..max_preview.saturating_sub(1)])
                } else {
                    entry.first_line.clone()
                };

                let row_style = if entry.is_selected {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                lines.push(Line::from(vec![
                    Span::styled(format!("{} {} ", icon, num), row_style),
                    status,
                    Span::styled(fl, row_style),
                ]));
            }
        }

        Paragraph::new(lines).render(area, buf);
    }

    fn render_devtools(&self, area: Rect, buf: &mut Buffer) {
        let s = &self.stats;
        let mut lines = vec![
            // Session overview
            Line::from(Span::styled(
                "╌╌ session ╌╌",
                Style::default().fg(Color::Magenta),
            )),
            Line::from(vec![
                Span::styled("  Cells     ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{}", s.total_cells), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("  ({}↓ {}≡)", s.code_cells, s.markdown_cells),
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Executed  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}/{}", s.executed_count, s.code_cells),
                    Style::default().fg(if s.executed_count == s.code_cells && s.code_cells > 0 {
                        Color::Green
                    } else {
                        Color::White
                    }),
                ),
            ]),
        ];

        if s.error_count > 0 {
            lines.push(Line::from(vec![
                Span::styled("  Errors    ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", s.error_count),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "╌╌ context ╌╌",
            Style::default().fg(Color::Magenta),
        )));
        lines.push(Line::from(vec![
            Span::styled("  Bindings  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", s.binding_count),
                Style::default().fg(Color::Cyan),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Functions ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", s.function_count),
                Style::default().fg(Color::Cyan),
            ),
        ]));

        if s.last_exec_time_ms > 0.0 {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "╌╌ last run ╌╌",
                Style::default().fg(Color::Magenta),
            )));
            let time_str = if s.last_exec_time_ms < 1.0 {
                format!("{:.1}ms", s.last_exec_time_ms)
            } else if s.last_exec_time_ms > 1000.0 {
                format!("{:.2}s", s.last_exec_time_ms / 1000.0)
            } else {
                format!("{:.0}ms", s.last_exec_time_ms)
            };
            lines.push(Line::from(vec![
                Span::styled("  Time  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("  {}", time_str),
                    Style::default().fg(Color::White),
                ),
            ]));

            if s.last_instructions > 0 {
                let instr_str = if s.last_instructions >= 1_000_000 {
                    format!("{:.1}M", s.last_instructions as f64 / 1_000_000.0)
                } else if s.last_instructions >= 1_000 {
                    format!("{:.1}K", s.last_instructions as f64 / 1_000.0)
                } else {
                    format!("{}", s.last_instructions)
                };
                lines.push(Line::from(vec![
                    Span::styled("  Instrs", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("  {}", instr_str),
                        Style::default().fg(Color::White),
                    ),
                ]));
            }

            if s.last_peak_stack > 0 {
                lines.push(Line::from(vec![
                    Span::styled("  Stack ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("  {} deep", s.last_peak_stack),
                        Style::default().fg(Color::White),
                    ),
                ]));
            }
        }

        // Keybinding hints at bottom
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "╌╌ shortcuts ╌╌",
            Style::default().fg(Color::Magenta),
        )));
        lines.push(Line::from(Span::styled(
            "  F5       run cell",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "  Ctrl+R   run cell",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "  F9       run all",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "  Ctrl+S   save",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "  Ctrl+B   sidebar",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "  Tab      next tab",
            Style::default().fg(Color::DarkGray),
        )));

        Paragraph::new(lines).render(area, buf);
    }
}

impl<'a> Default for SidebarWidget<'a> {
    fn default() -> Self { Self::new() }
}

impl<'a> Widget for SidebarWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 5 || area.height < 3 {
            return;
        }

        let tab_label = match self.tab {
            SidebarTab::Variables => " Variables ",
            SidebarTab::Outline => " Cells ",
            SidebarTab::DevTools => " Session ",
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(tab_label, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));

        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height < 2 {
            return;
        }

        // Tab bar — rename to meaningful labels
        let tabs = Tabs::new(vec!["Vars", "Cells", "Session"])
            .select(self.tab.index())
            .style(Style::default().fg(Color::DarkGray))
            .highlight_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .divider(Span::styled("│", Style::default().fg(Color::DarkGray)));
        let tab_area = Rect { height: 1, ..inner };
        tabs.render(tab_area, buf);

        let content = Rect {
            y: inner.y + 1,
            height: inner.height.saturating_sub(1),
            ..inner
        };

        match self.tab {
            SidebarTab::Variables => self.render_variables(content, buf),
            SidebarTab::Outline => self.render_outline(content, buf),
            SidebarTab::DevTools => self.render_devtools(content, buf),
        }
    }
}
