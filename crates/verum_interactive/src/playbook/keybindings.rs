//! Keybinding configuration for the Playbook TUI

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Keybinding mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeybindingMode {
    /// Standard keybindings (arrow keys, etc.)
    #[default]
    Standard,
    /// Vim-like keybindings
    Vim,
}

/// Actions that can be triggered by keybindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    // Navigation
    CellUp,
    CellDown,
    CellFirst,
    CellLast,
    PageUp,
    PageDown,

    // Cell operations
    EnterEdit,
    ExitEdit,
    ExecuteCell,
    ExecuteAllCells,
    ExecuteFromCurrent,
    InsertCellAfter,
    InsertCellBefore,
    DeleteCell,
    ClearOutputs,

    // Cell manipulation
    MoveCellUp,
    MoveCellDown,
    ToggleCollapse,
    ToggleCellType,
    SplitCell,
    MergeWithNext,

    // Sidebar
    ToggleSidebar,
    SidebarNextTab,
    SidebarPrevTab,

    // Edit operations
    Undo,
    Redo,

    // Modes
    EnterCommand,
    EnterSearch,
    ToggleFullscreen,

    // File operations
    Save,

    // Application
    Quit,
    ForceQuit,
    ShowHelp,

    // No action
    None,
}

/// Keybinding configuration
#[derive(Debug, Clone)]
pub struct Keybindings {
    mode: KeybindingMode,
}

impl Keybindings {
    pub fn new(mode: KeybindingMode) -> Self {
        Self { mode }
    }

    pub fn mode(&self) -> KeybindingMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: KeybindingMode) {
        self.mode = mode;
    }

    /// Resolve a key event in normal mode.
    pub fn normal_action(&self, key: KeyEvent) -> KeyAction {
        // Global keys
        // Global fullscreen: F11 (Linux/Win) or Ctrl+F (macOS)
        if key.code == KeyCode::F(11) { return KeyAction::ToggleFullscreen; }
        if key.code == KeyCode::Char('f') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return KeyAction::ToggleFullscreen;
        }

        match self.mode {
            KeybindingMode::Standard => self.standard_normal(key),
            KeybindingMode::Vim => self.vim_normal(key),
        }
    }

    /// Resolve a key event in edit mode.
    ///
    /// Execute-cell bindings that work across all terminals:
    /// - F5 (universal)
    /// - Ctrl+R (run — reliable on macOS)
    /// - Alt+Enter (Option+Enter on macOS — transmitted correctly)
    /// - Ctrl+Enter / Shift+Enter (works on Linux/Windows, not macOS Terminal.app)
    pub fn edit_action(&self, key: KeyEvent) -> KeyAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Esc => KeyAction::ExitEdit,
            KeyCode::Enter if ctrl || shift || alt => KeyAction::ExecuteCell,
            KeyCode::Char('r') if ctrl => KeyAction::ExecuteCell,
            KeyCode::Char('s') if ctrl => KeyAction::Save,
            KeyCode::F(5) => KeyAction::ExecuteCell,
            KeyCode::Char('f') if ctrl => KeyAction::ToggleFullscreen,
            KeyCode::F(11) => KeyAction::ToggleFullscreen,
            _ => KeyAction::None,
        }
    }

    fn standard_normal(&self, key: KeyEvent) -> KeyAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Up if ctrl && shift => KeyAction::MoveCellUp,
            KeyCode::Down if ctrl && shift => KeyAction::MoveCellDown,
            KeyCode::Up => KeyAction::CellUp,
            KeyCode::Down => KeyAction::CellDown,
            KeyCode::Home if ctrl => KeyAction::CellFirst,
            KeyCode::End if ctrl => KeyAction::CellLast,
            KeyCode::PageUp => KeyAction::PageUp,
            KeyCode::PageDown => KeyAction::PageDown,
            KeyCode::Enter => KeyAction::EnterEdit,
            KeyCode::F(5) => KeyAction::ExecuteCell,
            KeyCode::F(6) => KeyAction::ExecuteFromCurrent,
            KeyCode::F(9) => KeyAction::ExecuteAllCells,
            KeyCode::Insert => KeyAction::InsertCellAfter,
            KeyCode::Delete => KeyAction::DeleteCell,
            KeyCode::Char('b') if ctrl => KeyAction::ToggleSidebar,
            KeyCode::Tab => KeyAction::SidebarNextTab,
            KeyCode::BackTab => KeyAction::SidebarPrevTab,
            KeyCode::Char('s') if ctrl => KeyAction::Save,
            KeyCode::Char('f') if ctrl => KeyAction::ToggleFullscreen,
            KeyCode::Char('z') if ctrl && shift => KeyAction::Redo,
            KeyCode::Char('z') if ctrl => KeyAction::Undo,
            KeyCode::Char('y') if ctrl => KeyAction::Redo,
            KeyCode::Char('q') if ctrl => KeyAction::Quit,
            KeyCode::F(1) => KeyAction::ShowHelp,
            KeyCode::F(2) => KeyAction::EnterCommand,
            _ => KeyAction::None,
        }
    }

    fn vim_normal(&self, key: KeyEvent) -> KeyAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => KeyAction::CellDown,
            KeyCode::Char('k') | KeyCode::Up => KeyAction::CellUp,
            KeyCode::Char('g') => KeyAction::CellFirst,
            KeyCode::Char('G') => KeyAction::CellLast,
            KeyCode::Char('d') if ctrl => KeyAction::PageDown,
            KeyCode::Char('u') if ctrl => KeyAction::PageUp,
            KeyCode::Char('i') | KeyCode::Enter => KeyAction::EnterEdit,
            KeyCode::Char('x') => KeyAction::ExecuteCell,
            KeyCode::Char('X') => KeyAction::ExecuteAllCells,
            KeyCode::Char('o') => KeyAction::InsertCellAfter,
            KeyCode::Char('O') => KeyAction::InsertCellBefore,
            KeyCode::Char('D') => KeyAction::DeleteCell,
            KeyCode::Char('K') => KeyAction::MoveCellUp,
            KeyCode::Char('J') => KeyAction::MoveCellDown,
            KeyCode::Char(' ') => KeyAction::ToggleCollapse,
            KeyCode::Char('m') => KeyAction::ToggleCellType,
            KeyCode::Char('S') => KeyAction::SplitCell,
            KeyCode::Char('b') if ctrl => KeyAction::ToggleSidebar,
            KeyCode::Tab => KeyAction::SidebarNextTab,
            KeyCode::BackTab => KeyAction::SidebarPrevTab,
            KeyCode::Char(':') => KeyAction::EnterCommand,
            KeyCode::Char('/') => KeyAction::EnterSearch,
            KeyCode::Char('s') if ctrl => KeyAction::Save,
            KeyCode::Char('u') => KeyAction::Undo,
            KeyCode::Char('r') if ctrl => KeyAction::Redo,
            KeyCode::Char('q') => KeyAction::Quit,
            KeyCode::Char('Q') => KeyAction::ForceQuit,
            KeyCode::Char('?') => KeyAction::ShowHelp,
            KeyCode::Char('c') if ctrl => KeyAction::ClearOutputs,
            _ => KeyAction::None,
        }
    }
}

impl Default for Keybindings {
    fn default() -> Self {
        Self::new(KeybindingMode::Standard)
    }
}
