//! UI components for the Playbook TUI

mod cell;
mod editor;
mod output;
mod sidebar;

pub use cell::{CellWidget, cell_height};
pub use editor::{EditorWidget, EditorState, EditorDiagnostic, DiagnosticSeverity, Selection};
pub use output::{OutputWidget, output_height, format_output_lines, format_output_brief, output_line_count};
pub use sidebar::{
    SidebarWidget, SidebarTab, VarInfo, FuncInfo, OutlineEntry, ExecStats,
};

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Layout for the playbook UI
pub struct PlaybookLayout {
    /// Main content area (cells)
    pub content: Rect,
    /// Sidebar area (variables, outline)
    pub sidebar: Rect,
    /// Editor area (bottom input panel)
    pub editor: Rect,
    /// Status bar area
    pub status: Rect,
    /// Help bar area
    pub help: Rect,
}

/// Layout configuration options
#[derive(Debug, Clone, Copy)]
pub struct LayoutConfig {
    /// Editor panel height (lines)
    pub editor_height: u16,
    /// Whether editor is in fullscreen mode
    pub editor_fullscreen: bool,
    /// Whether to show sidebar
    pub show_sidebar: bool,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            editor_height: 10,
            editor_fullscreen: false,
            show_sidebar: true,
        }
    }
}

impl LayoutConfig {
    /// Create a config with fullscreen editor
    pub fn fullscreen() -> Self {
        Self {
            editor_height: 0, // Will use all available space
            editor_fullscreen: true,
            show_sidebar: false,
        }
    }

    /// Toggle fullscreen mode
    pub fn toggle_fullscreen(&mut self) {
        self.editor_fullscreen = !self.editor_fullscreen;
    }
}

impl PlaybookLayout {
    /// Calculate layout from total area with default config
    pub fn from_area(area: Rect) -> Self {
        Self::from_area_with_config(area, LayoutConfig::default())
    }

    /// Calculate layout from total area with custom config
    pub fn from_area_with_config(area: Rect, config: LayoutConfig) -> Self {
        if config.editor_fullscreen {
            // Fullscreen mode: editor takes everything except status/help bars
            let vertical = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(3),      // Editor (fullscreen)
                    Constraint::Length(1),   // Status bar
                    Constraint::Length(1),   // Help bar
                ])
                .split(area);

            Self {
                content: Rect::new(0, 0, 0, 0), // Hidden
                sidebar: Rect::new(0, 0, 0, 0), // Hidden
                editor: vertical[0],
                status: vertical[1],
                help: vertical[2],
            }
        } else {
            // Normal mode: cells on top, editor at bottom, sidebar on right
            let vertical = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(5),                           // Main content + sidebar
                    Constraint::Length(config.editor_height),     // Editor panel
                    Constraint::Length(1),                        // Status bar
                    Constraint::Length(1),                        // Help bar
                ])
                .split(area);

            // Horizontal split for content area: cells | sidebar
            let horizontal = if config.show_sidebar {
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Percentage(70), // Cells
                        Constraint::Percentage(30), // Sidebar
                    ])
                    .split(vertical[0])
            } else {
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Percentage(100), // Cells only
                    ])
                    .split(vertical[0])
            };

            Self {
                content: horizontal[0],
                sidebar: if config.show_sidebar { horizontal[1] } else { Rect::new(0, 0, 0, 0) },
                editor: vertical[1],
                status: vertical[2],
                help: vertical[3],
            }
        }
    }
}
