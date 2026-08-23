//! Main Playbook application: state, event loop, and rendering.
//!
//! Cyberpunk-inspired research environment for Verum language exploration.
//! All keybinding dispatch uses the centralized `Keybindings` module.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::keybindings::{KeyAction, KeybindingMode, Keybindings};
use super::session::{Cell, CellKind, CellOutput, SessionState};
use super::ui::{
    CellWidget, EditorState, EditorWidget, ExecStats, FuncInfo, LayoutConfig, OutlineEntry,
    PlaybookLayout, SidebarTab, SidebarWidget, VarInfo, cell_height,
};
use crate::discovery::tutorials::{Tutorial, builtin_tutorials};
use crate::value_format::{ValueDisplayOptions, format_value};

/// Visual theme for the playbook UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Cyberpunk,
    Dark,
    Light,
}

/// Application mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Normal,
    Edit,
    Command,
    Search,
    /// Prompts for a filename when saving without a path.
    SavePrompt,
}

/// Main Playbook application.
pub struct PlaybookApp {
    pub session: SessionState,
    pub mode: AppMode,
    pub file_path: Option<PathBuf>,
    pub should_quit: bool,
    pub status_message: Option<String>,
    pub editor: EditorState,
    pub layout_config: LayoutConfig,
    pub sidebar_tab: SidebarTab,
    diagnostics: Vec<super::ui::EditorDiagnostic>,
    /// Scroll offset for the cell list.
    scroll_offset: u16,
    /// Command/search buffer.
    input_buffer: String,
    /// Set of collapsed cell IDs.
    collapsed_cells: HashSet<super::session::CellId>,
    /// Full-screen help overlay (T0858 «видимая карта действий»):
    /// `?` toggles it; any key closes it. The one-line contextual
    /// hint stays in the footer — this is the COMPLETE map.
    show_help_overlay: bool,
    /// The launch gallery (Playground Reborn §3, «вход»): an empty
    /// `verum play` opens a chooser — guided tours, recent books in
    /// the working directory, a blank sheet — instead of a bare
    /// buffer the newcomer must already understand.
    gallery: Option<GalleryState>,
    /// VBC lens (T0858 slice 3): disassembly of the
    /// notebook-as-module, from the same artifact path the
    /// interpreter runs.
    vbc_lens_text: String,
    /// Tiers lens: verdict lines from the LAST `diff-tiers` judgment.
    tiers_lens_lines: Vec<String>,
    /// In-flight tier judgment — result channel, its temp source
    /// file (kept alive until the child reads it), start time.
    tiers_rx: Option<std::sync::mpsc::Receiver<Vec<String>>>,
    tiers_tmp: Option<tempfile::NamedTempFile>,
    tiers_started: Option<Instant>,
    /// The session journal (T0858 slice 5): every question this
    /// session asked — runs, lens queries, tier judgments — one line
    /// each, append-only, with the chain address where one exists.
    /// The TUI twin of the protocol's frame journal.
    journal: Vec<String>,
    /// Arch-lens content (T0858 slice 2): rendered lines mirrored
    /// from `verum arch query --json` over the notebook-as-module
    /// (the accepted state law: cells concatenate into one growing
    /// module; the lens asks the SAME vocabulary agents use).
    arch_lens_lines: Vec<String>,
    /// Centralized keybinding dispatch.
    keybindings: Keybindings,
    /// Last execution time for stats display.
    last_exec_time_ms: f64,
    /// Auto-save: last save timestamp.
    last_save: Option<Instant>,
    /// Auto-save interval in seconds (0 = disabled).
    auto_save_interval_secs: u64,
    /// Search results: list of (cell_index, line_index) matches.
    search_results: Vec<(usize, usize)>,
    /// Current search result index.
    search_cursor: usize,
    /// Value display options for sidebar.
    display_options: ValueDisplayOptions,
    /// Snapshot of cell sources before execution (for :diff).
    previous_cell_sources: std::collections::HashMap<super::session::CellId, String>,
    /// VBC instructions executed in last run.
    last_instructions: u64,
    /// Peak stack depth in last run.
    last_peak_stack: usize,
    /// Visual theme.
    theme: Theme,
    /// Tab completions for the current partial word.
    completions: Vec<String>,
    /// Current index in completion list.
    completion_index: Option<usize>,
    /// Background execution state (None = idle).
    pending_rx: Option<std::sync::mpsc::Receiver<AsyncExecMsg>>,
    /// Cancellation flag shared with interpreter dispatch loop.
    pending_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Index of the cell being executed in background.
    pending_cell_idx: Option<usize>,
    /// Worker thread handle.
    pending_thread: Option<std::thread::JoinHandle<()>>,
    /// When background execution started.
    pending_started: Option<Instant>,
    /// Spinner frame counter.
    pending_spinner: usize,
}

/// Message from worker thread back to UI.
#[allow(clippy::large_enum_variant)]
enum AsyncExecMsg {
    /// One grown-module run finished (T0858): the outcome distributes
    /// over cells via SessionState::apply_notebook_outcome.
    NotebookDone {
        indices: Vec<usize>,
        outcome: verum_vbc::interpreter::ScriptOutcome,
        elapsed: std::time::Duration,
    },
}

impl PlaybookApp {
    pub fn new() -> Self {
        Self {
            session: SessionState::new(),
            mode: AppMode::Normal,
            file_path: None,
            should_quit: false,
            status_message: None,
            show_help_overlay: false,
            gallery: None,
            vbc_lens_text: String::new(),
            tiers_lens_lines: Vec::new(),
            tiers_rx: None,
            tiers_tmp: None,
            tiers_started: None,
            journal: Vec::new(),
            arch_lens_lines: Vec::new(),
            editor: EditorState::new(),
            layout_config: LayoutConfig::default(),
            sidebar_tab: SidebarTab::Variables,
            diagnostics: Vec::new(),
            scroll_offset: 0,
            input_buffer: String::new(),
            collapsed_cells: HashSet::new(),
            keybindings: Keybindings::new(KeybindingMode::Standard),
            last_exec_time_ms: 0.0,
            last_save: None,
            auto_save_interval_secs: 0,
            search_results: Vec::new(),
            search_cursor: 0,
            display_options: ValueDisplayOptions::compact(),
            previous_cell_sources: std::collections::HashMap::new(),
            last_instructions: 0,
            last_peak_stack: 0,
            theme: Theme::Cyberpunk,
            completions: Vec::new(),
            completion_index: None,
            pending_rx: None,
            pending_cancel: None,
            pending_cell_idx: None,
            pending_thread: None,
            pending_started: None,
            pending_spinner: 0,
        }
    }

    /// True if a cell is executing in background.
    pub fn is_executing(&self) -> bool {
        self.pending_rx.is_some()
    }

    /// Poll for background execution results. Called each UI tick.
    pub fn poll_execution(&mut self) {
        self.poll_tiers_judge();
        let rx = match &self.pending_rx {
            Some(r) => r,
            None => return,
        };

        match rx.try_recv() {
            Ok(AsyncExecMsg::NotebookDone {
                indices,
                outcome,
                elapsed,
            }) => {
                let cell_idx = self.pending_cell_idx.unwrap_or(0);
                let time_ms = elapsed.as_secs_f64() * 1000.0;
                let verdict = self
                    .session
                    .apply_notebook_outcome(&indices, outcome, elapsed);
                self.last_exec_time_ms = time_ms;
                match verdict {
                    Ok(()) => {
                        self.status_message = Some(format!("Done ({time_ms:.1}ms)"));
                        self.journal_push(format!(
                            "run cell {} ({time_ms:.1}ms)",
                            cell_idx + 1
                        ));
                    }
                    Err(e) => {
                        self.status_message = Some(format!("Error: {e}"));
                        self.journal_push(format!("run cell {} error", cell_idx + 1));
                    }
                }
                self.cleanup_pending();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Still running — animate spinner
                self.pending_spinner += 1;
                let spinners = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                let ch = spinners[self.pending_spinner % spinners.len()];
                let elapsed = self
                    .pending_started
                    .map(|s| s.elapsed().as_secs_f64())
                    .unwrap_or(0.0);
                self.status_message = Some(format!(
                    "{} Running cell {}... ({:.1}s) [Ctrl+C to cancel]",
                    ch,
                    self.pending_cell_idx.unwrap_or(0) + 1,
                    elapsed,
                ));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.status_message = Some("Worker thread crashed".to_string());
                self.cleanup_pending();
            }
        }
    }

    /// Cancel the background execution via atomic flag.
    pub fn cancel_execution(&mut self) {
        if let Some(flag) = &self.pending_cancel {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
            self.status_message = Some("Cancelling...".to_string());
        }
    }

    fn cleanup_pending(&mut self) {
        if let Some(handle) = self.pending_thread.take() {
            let _ = handle.join();
        }
        self.pending_rx = None;
        self.pending_cancel = None;
        self.pending_cell_idx = None;
        self.pending_started = None;
    }

    pub fn from_file(path: PathBuf) -> io::Result<Self> {
        let mut app = Self::new();
        app.file_path = Some(path.clone());
        if path.exists() {
            match super::persistence::load_playbook(&path) {
                Ok((cells, settings)) => {
                    app.session = SessionState::with_cells(cells);
                    if let Some(s) = settings {
                        app.auto_save_interval_secs = s.auto_save_interval_secs;
                        app.layout_config.show_sidebar = s.show_sidebar;
                        app.session.execution_timeout_ms = s.execution_timeout_ms;
                        match s.keybinding_mode.as_str() {
                            "vim" => app.keybindings.set_mode(KeybindingMode::Vim),
                            _ => app.keybindings.set_mode(KeybindingMode::Standard),
                        }
                    }
                    app.status_message = Some(format!("Loaded: {}", path.display()));
                }
                Err(e) => app.status_message = Some(format!("Error loading: {}", e)),
            }
        }
        app.sync_editor_from_cell();
        Ok(app)
    }

    // ── Event Dispatch ──────────────────────────────────────────────────

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.status_message = None;

        // Help overlay swallows every key: any press closes it. It
        // must come before all other dispatch — a map you cannot
        // close from is worse than no map.
        if self.show_help_overlay {
            self.show_help_overlay = false;
            return;
        }

        // The gallery swallows navigation until a choice is made —
        // it IS the first screen, not a dialog over one.
        if self.gallery.is_some() {
            self.handle_gallery_key(key);
            return;
        }

        // The Tiers lens answers only on demand: `t` with the lens on
        // screen starts the judgment (it builds BOTH tiers — an
        // expensive question deserves an explicit ask).
        if self.mode == AppMode::Normal
            && key.code == KeyCode::Char('t')
            && key.modifiers.is_empty()
            && self.layout_config.show_sidebar
            && self.sidebar_tab == SidebarTab::Tiers
        {
            self.start_tiers_judge();
            return;
        }

        // Global: Ctrl+C cancels running execution
        if key.code == KeyCode::Char('c')
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && self.is_executing()
        {
            self.cancel_execution();
            return;
        }

        // Global: F11 / Ctrl+F fullscreen
        let is_fullscreen_toggle = key.code == KeyCode::F(11)
            || (key.code == KeyCode::Char('f') && key.modifiers.contains(KeyModifiers::CONTROL));
        if is_fullscreen_toggle {
            self.layout_config.toggle_fullscreen();
            self.editor.fullscreen = self.layout_config.editor_fullscreen;
            return;
        }

        match self.mode {
            AppMode::Normal => self.dispatch_normal(key),
            AppMode::Edit => self.dispatch_edit(key),
            AppMode::Command => self.dispatch_input(key, false),
            AppMode::Search => self.dispatch_input(key, true),
            AppMode::SavePrompt => self.dispatch_save_prompt(key),
        }

        // Auto-save check
        self.check_auto_save();
    }

    /// Handle mouse events (scroll wheel for cell list).
    pub fn handle_mouse(&mut self, event: MouseEvent) {
        match event.kind {
            MouseEventKind::ScrollUp => {
                self.session.select_prev();
                self.sync_editor_from_cell();
            }
            MouseEventKind::ScrollDown => {
                self.session.select_next();
                self.sync_editor_from_cell();
            }
            _ => {}
        }
    }

    /// Normal mode: dispatch via Keybindings module.
    fn dispatch_normal(&mut self, key: KeyEvent) {
        let action = self.keybindings.normal_action(key);
        match action {
            KeyAction::Quit => self.should_quit = true,
            KeyAction::ForceQuit => self.should_quit = true,
            KeyAction::CellDown => {
                self.session.select_next();
                self.sync_editor_from_cell();
            }
            KeyAction::CellUp => {
                self.session.select_prev();
                self.sync_editor_from_cell();
            }
            KeyAction::CellFirst => {
                self.session.selected_cell = 0;
                self.sync_editor_from_cell();
            }
            KeyAction::CellLast => {
                self.session.selected_cell = self.session.cells.len().saturating_sub(1);
                self.sync_editor_from_cell();
            }
            KeyAction::PageDown => {
                for _ in 0..5 {
                    self.session.select_next();
                }
                self.sync_editor_from_cell();
            }
            KeyAction::PageUp => {
                for _ in 0..5 {
                    self.session.select_prev();
                }
                self.sync_editor_from_cell();
            }
            KeyAction::EnterEdit => self.enter_edit_mode(),
            KeyAction::InsertCellAfter => {
                self.session.insert_cell_after(CellKind::Code);
                self.sync_editor_from_cell();
                self.enter_edit_mode();
            }
            KeyAction::InsertCellBefore => {
                self.session.insert_cell_before(CellKind::Code);
                self.sync_editor_from_cell();
                self.enter_edit_mode();
            }
            KeyAction::ExecuteCell => {
                self.execute_current_cell();
                self.refresh_lenses_if_active();
            }
            KeyAction::ExecuteAllCells => {
                self.commit_edit();
                if let Err(e) = self.session.execute_all() {
                    self.status_message = Some(format!("Error: {}", e));
                }
                self.refresh_lenses_if_active();
            }
            KeyAction::ExecuteFromCurrent => {
                self.commit_edit();
                if let Err(e) = self.session.execute_from_current() {
                    self.status_message = Some(format!("Error: {}", e));
                }
            }
            KeyAction::DeleteCell => {
                self.session.delete_current_cell();
                self.sync_editor_from_cell();
            }
            KeyAction::MoveCellUp => {
                self.session.move_cell_up();
                self.sync_editor_from_cell();
            }
            KeyAction::MoveCellDown => {
                self.session.move_cell_down();
                self.sync_editor_from_cell();
            }
            KeyAction::ToggleCollapse => {
                let id = self.session.current_cell().id;
                if !self.collapsed_cells.remove(&id) {
                    self.collapsed_cells.insert(id);
                }
            }
            KeyAction::ToggleCellType => {
                self.session.toggle_cell_type();
                self.sync_editor_from_cell();
            }
            KeyAction::SplitCell => {
                let cursor_line = self.editor.cursor.0;
                self.commit_edit();
                self.session.split_cell(cursor_line);
                self.sync_editor_from_cell();
            }
            KeyAction::MergeWithNext => {
                self.session.merge_with_next();
                self.sync_editor_from_cell();
            }
            KeyAction::ToggleSidebar => {
                self.layout_config.show_sidebar = !self.layout_config.show_sidebar;
            }
            KeyAction::SidebarNextTab => {
                self.sidebar_tab = self.sidebar_tab.next();
                self.refresh_lenses_if_active();
            }
            KeyAction::SidebarPrevTab => {
                self.sidebar_tab = self.sidebar_tab.prev();
                self.refresh_lenses_if_active();
            }
            KeyAction::Save => self.save(),
            KeyAction::Undo => {
                if !self.session.undo() {
                    self.status_message = Some("Nothing to undo".to_string());
                } else {
                    self.sync_editor_from_cell();
                }
            }
            KeyAction::Redo => {
                if !self.session.redo() {
                    self.status_message = Some("Nothing to redo".to_string());
                } else {
                    self.sync_editor_from_cell();
                }
            }
            KeyAction::ClearOutputs => {
                self.session.clear_all_outputs();
                self.status_message = Some("Outputs cleared".to_string());
            }
            KeyAction::EnterCommand => {
                self.mode = AppMode::Command;
                self.input_buffer.clear();
            }
            KeyAction::EnterSearch => {
                self.mode = AppMode::Search;
                self.input_buffer.clear();
                self.search_results.clear();
            }
            KeyAction::ShowHelp => {
                self.show_help_overlay = true;
            }
            KeyAction::ToggleFullscreen => {
                self.layout_config.toggle_fullscreen();
                self.editor.fullscreen = self.layout_config.editor_fullscreen;
            }
            _ => {}
        }
    }

    /// Edit mode: keybinding dispatch + editor input.
    fn dispatch_edit(&mut self, key: KeyEvent) {
        let action = self.keybindings.edit_action(key);
        match action {
            KeyAction::ExitEdit => {
                // In the modal fullscreen editor, the first Esc collapses
                // the modal back into the notebook; the second leaves
                // edit mode — matching every editor's modal convention.
                if self.layout_config.editor_fullscreen {
                    self.layout_config.toggle_fullscreen();
                    self.editor.fullscreen = false;
                } else {
                    self.exit_edit_mode();
                }
            }
            KeyAction::ExecuteCell => {
                self.execute_current_cell();
                self.refresh_lenses_if_active();
            }
            KeyAction::Save => {
                self.commit_edit();
                self.save();
            }
            KeyAction::ToggleFullscreen => {
                self.layout_config.toggle_fullscreen();
                self.editor.fullscreen = self.layout_config.editor_fullscreen;
            }
            _ => {
                // Forward to editor for text editing
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                let alt = key.modifiers.contains(KeyModifiers::ALT);
                match key.code {
                    KeyCode::Left if ctrl || alt => self.editor.move_word_left(shift),
                    KeyCode::Right if ctrl || alt => self.editor.move_word_right(shift),
                    KeyCode::Left => self.editor.move_left(shift),
                    KeyCode::Right => self.editor.move_right(shift),
                    KeyCode::Up if alt => self.editor.move_lines_up(),
                    KeyCode::Down if alt => self.editor.move_lines_down(),
                    KeyCode::Up => self.editor.move_up(shift),
                    KeyCode::Down => self.editor.move_down(shift),
                    KeyCode::PageUp => self.editor.page_move(false, self.editor_page(), shift),
                    KeyCode::PageDown => self.editor.page_move(true, self.editor_page(), shift),
                    KeyCode::Home if ctrl => self.editor.move_to_start(shift),
                    KeyCode::End if ctrl => self.editor.move_to_end(shift),
                    KeyCode::Home => self.editor.move_home_smart(shift),
                    KeyCode::End => self.editor.move_end(shift),
                    KeyCode::Enter => self.editor.insert_newline_auto_indent(),
                    KeyCode::Backspace if ctrl || alt => self.editor.delete_word_left(),
                    KeyCode::Backspace => self.editor.backspace(),
                    KeyCode::Delete if ctrl || alt => self.editor.delete_word_right(),
                    KeyCode::Delete => self.editor.delete(),
                    KeyCode::BackTab => self.editor.dedent_lines(),
                    // Editing power keys (VS Code / JetBrains muscle memory):
                    // Ctrl+D duplicate, Ctrl+Shift+K delete line,
                    // Ctrl+/ (arrives as '/' or '_' per terminal) comment,
                    // Ctrl+K kill-to-eol, Ctrl+J join lines.
                    KeyCode::Char('d') if ctrl => self.editor.duplicate_lines(),
                    KeyCode::Char('k') if ctrl && shift => self.editor.delete_lines(),
                    KeyCode::Char('/') if ctrl => self.editor.toggle_comment(),
                    KeyCode::Char('_') if ctrl => self.editor.toggle_comment(),
                    KeyCode::Char('7') if ctrl => self.editor.toggle_comment(),
                    KeyCode::Char('k') if ctrl => self.editor.kill_to_eol(),
                    KeyCode::Char('j') if ctrl => self.editor.join_lines(),
                    KeyCode::Tab if self
                        .editor
                        .selection
                        .map(|s| {
                            let n = s.normalize();
                            !n.is_empty() && n.start.0 != n.end.0
                        })
                        .unwrap_or(false) =>
                    {
                        // A multi-line selection: Tab indents the block
                        // (BackTab dedents) — completion only fires on a
                        // bare cursor.
                        self.editor.indent_lines();
                    }
                    KeyCode::Tab => {
                        // Try inline completion if cursor is after a partial word
                        let (row, col) = self.editor.cursor;
                        let line = self.editor.lines.get(row).cloned().unwrap_or_default();
                        // `col` is a CHAR index (editor cursor convention) —
                        // slicing the String needs the BYTE offset, or any
                        // non-ASCII line panics/garbles.
                        let byte_col = line
                            .char_indices()
                            .nth(col)
                            .map(|(i, _)| i)
                            .unwrap_or(line.len());
                        let before_cursor = &line[..byte_col];
                        // Extract partial word: sequence of alphanumeric/_ chars before cursor
                        let partial: String = before_cursor
                            .chars()
                            .rev()
                            .take_while(|c| c.is_alphanumeric() || *c == '_')
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect();

                        if partial.is_empty() {
                            // No partial word — just insert tab as spaces
                            self.completions.clear();
                            self.completion_index = None;
                            self.editor.insert_tab();
                        } else if self.completion_index.is_some() && !self.completions.is_empty() {
                            // Cycle to next completion
                            let idx = self.completion_index.unwrap();
                            let next = (idx + 1) % self.completions.len();
                            self.completion_index = Some(next);
                            // Replace: delete old completion, insert new one
                            // (backspace steps are CHARS, so count chars,
                            // not bytes).
                            let old_chars = self.completions[idx].chars().count();
                            let new = self.completions[next].clone();
                            for _ in 0..old_chars {
                                self.editor.backspace();
                            }
                            for ch in new.chars() {
                                self.editor.insert_char(ch);
                            }
                        } else {
                            // Compute completions
                            let keywords = [
                                "fn",
                                "let",
                                "if",
                                "else",
                                "match",
                                "for",
                                "while",
                                "type",
                                "true",
                                "false",
                                "println",
                                "print",
                                "return",
                                "mut",
                                "implement",
                                "mount",
                                "module",
                                "pub",
                                "async",
                                "await",
                                "spawn",
                            ];
                            let builtin_types = [
                                "Int", "Float", "Bool", "Text", "List", "Map", "Set", "Maybe",
                                "Heap", "Shared", "Channel", "Mutex", "Task",
                            ];

                            let mut candidates: Vec<String> = Vec::new();
                            // From execution context bindings
                            for (name, _) in self.session.var_previews_iter() {
                                if name.as_str().starts_with(&partial) {
                                    candidates.push(name.to_string());
                                }
                            }
                            // Keywords
                            for kw in &keywords {
                                if kw.starts_with(&partial) {
                                    candidates.push(kw.to_string());
                                }
                            }
                            // Builtin types
                            for bt in &builtin_types {
                                if bt.starts_with(&partial) {
                                    candidates.push(bt.to_string());
                                }
                            }
                            candidates.sort();
                            candidates.dedup();

                            if candidates.is_empty() {
                                // No completions, just insert tab
                                self.editor.insert_tab();
                            } else {
                                // Replace partial with first completion
                                for _ in 0..partial.chars().count() {
                                    self.editor.backspace();
                                }
                                let first = candidates[0].clone();
                                for ch in first.chars() {
                                    self.editor.insert_char(ch);
                                }
                                self.completions = candidates;
                                self.completion_index = Some(0);
                            }
                        }
                    }
                    KeyCode::Char('a') if ctrl => self.editor.select_all(),
                    KeyCode::Char('c') if ctrl => {
                        self.editor.copy();
                        self.status_message = Some("Copied".to_string());
                    }
                    KeyCode::Char('x') if ctrl => {
                        self.editor.cut();
                        self.status_message = Some("Cut".to_string());
                    }
                    KeyCode::Char('v') if ctrl => self.editor.paste(),
                    KeyCode::Char('z') if ctrl && shift => {
                        self.editor.redo();
                    }
                    KeyCode::Char('z') if ctrl => {
                        self.editor.undo();
                    }
                    // A PLAIN character — the guard matters: without it,
                    // every unbound Ctrl/Alt chord fell through here and
                    // typed its letter into the buffer (Ctrl+G inserted a
                    // literal 'g').
                    KeyCode::Char(c) if !ctrl && !alt => {
                        self.completions.clear();
                        self.completion_index = None;
                        self.editor.insert_char_smart(c);
                    }
                    _ => {
                        self.completions.clear();
                        self.completion_index = None;
                    }
                }
            }
        }
        let (rows, cols) = self.editor_viewport();
        self.editor.ensure_cursor_visible(rows);
        self.editor.ensure_cursor_visible_h(cols);
    }

    /// The editor viewport in (rows, text columns), measured from the
    /// real terminal size so fullscreen scrolling tracks the actual
    /// window instead of a hard-coded guess.
    fn editor_viewport(&self) -> (usize, usize) {
        let (w, h) = crossterm::terminal::size().unwrap_or((100, 30));
        let rows = if self.layout_config.editor_fullscreen {
            (h as usize).saturating_sub(4).max(3)
        } else {
            8
        };
        let gutter = self.editor.lines.len().to_string().len() + 3;
        let cols = (w as usize).saturating_sub(gutter).max(10);
        (rows, cols)
    }

    /// Page size for PageUp/PageDown inside the editor.
    fn editor_page(&self) -> usize {
        self.editor_viewport().0.saturating_sub(1).max(1)
    }

    /// Command/Search input mode.
    fn dispatch_input(&mut self, key: KeyEvent, is_search: bool) {
        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
                self.input_buffer.clear();
            }
            KeyCode::Enter => {
                let buf = self.input_buffer.clone();
                self.mode = AppMode::Normal;
                if is_search {
                    self.perform_search(&buf);
                } else {
                    self.execute_command(&buf);
                }
                self.input_buffer.clear();
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Char(c) => {
                self.input_buffer.push(c);
                // Live search preview
                if is_search {
                    self.perform_search(&self.input_buffer.clone());
                }
            }
            _ => {}
        }
    }

    // ── Core Operations ─────────────────────────────────────────────────

    fn execute_current_cell(&mut self) {
        if self.is_executing() {
            self.status_message = Some("Already executing. Ctrl+C to cancel.".to_string());
            return;
        }

        self.commit_edit();
        self.diagnostics.clear();

        let cell_idx = self.session.selected_cell;
        let cell_id = self.session.current_cell().id;
        self.previous_cell_sources
            .insert(cell_id, self.session.current_cell().source.to_string());

        // The grown-module question, prepared on the UI thread; a
        // throwaway engine runs it on the worker (the compiler and
        // archive caches are process-wide, so per-run engines are
        // cheap), and its interrupt handle IS the cancel flag.
        let Some((source, indices)) = self.session.prepare_notebook_run(cell_idx) else {
            return;
        };
        let mut engine = verum_vbc::interpreter::ScriptEngine::new()
            .allow_file_io()
            .allow_network()
            .allow_process();
        let cancel_flag = engine.interrupt_handle();

        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let started = Instant::now();
            let outcome =
                crate::playbook::session::run_grown_module(&mut engine, &source);
            let _ = tx.send(AsyncExecMsg::NotebookDone {
                indices,
                outcome,
                elapsed: started.elapsed(),
            });
        });

        self.pending_rx = Some(rx);
        self.pending_cancel = Some(cancel_flag);
        self.pending_cell_idx = Some(cell_idx);
        self.pending_thread = Some(thread);
        self.pending_started = Some(Instant::now());
        self.pending_spinner = 0;
        self.status_message = Some(format!("⏳ Running cell {}...", cell_idx + 1));
    }

    /// In the recompute-from-source model every code cell AFTER the
    /// executed one shows output from an older question — mark them
    /// stale wholesale (the old binding-dependency graph belonged to
    /// the retired execution bridge).
    fn mark_dependents_dirty(&mut self) {
        let after = self.session.selected_cell + 1;
        for cell in self.session.cells.iter_mut().skip(after) {
            if cell.output.is_some() {
                cell.dirty = true;
            }
        }
    }

    fn execute_command(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        match parts.first().copied() {
            Some("w") | Some("save") => {
                if let Some(path) = parts.get(1) {
                    self.file_path = Some(PathBuf::from(path));
                }
                self.save();
            }
            Some("q") | Some("quit") => self.should_quit = true,
            Some("wq") => {
                self.save();
                self.should_quit = true;
            }
            Some("e") => {
                if let Some(path) = parts.get(1) {
                    match self.export_to_script(Path::new(path)) {
                        Ok(()) => self.status_message = Some(format!("Exported to {}", path)),
                        Err(e) => self.status_message = Some(format!("Export failed: {}", e)),
                    }
                } else {
                    self.status_message = Some("Usage: :e <path>".to_string());
                }
            }
            Some("clear") => {
                self.session.clear_all_outputs();
                self.status_message = Some("Outputs cleared".to_string());
            }
            Some("run") | Some("runall") => {
                self.commit_edit();
                if let Err(e) = self.session.execute_all() {
                    self.status_message = Some(format!("Error: {}", e));
                }
            }
            Some("set") => {
                if let Some(setting) = parts.get(1) {
                    match *setting {
                        "autosave" => {
                            let secs = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(60);
                            self.auto_save_interval_secs = secs;
                            self.status_message = Some(format!("Auto-save: {}s", secs));
                        }
                        "noautosave" => {
                            self.auto_save_interval_secs = 0;
                            self.status_message = Some("Auto-save disabled".to_string());
                        }
                        "vim" => {
                            self.keybindings.set_mode(KeybindingMode::Vim);
                            self.status_message = Some("Vim mode".to_string());
                        }
                        "standard" => {
                            self.keybindings.set_mode(KeybindingMode::Standard);
                            self.status_message = Some("Standard mode".to_string());
                        }
                        "sidebar" => {
                            self.layout_config.show_sidebar = true;
                            self.status_message = Some("Sidebar on".to_string());
                        }
                        "nosidebar" => {
                            self.layout_config.show_sidebar = false;
                            self.status_message = Some("Sidebar off".to_string());
                        }
                        "timeout" => {
                            let ms = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(5000u64);
                            self.session.execution_timeout_ms = ms;
                            self.status_message = Some(format!("Execution timeout: {}ms", ms));
                        }
                        _ => self.status_message = Some(format!("Unknown setting: {}", setting)),
                    }
                } else {
                    self.status_message = Some(
                        "Usage: :set <autosave|vim|standard|sidebar|nosidebar|timeout>".to_string(),
                    );
                }
            }
            Some("split") => {
                let line = self.editor.cursor.0;
                self.commit_edit();
                self.session.split_cell(line);
                self.sync_editor_from_cell();
                self.status_message = Some("Cell split".to_string());
            }
            Some("merge") => {
                self.session.merge_with_next();
                self.sync_editor_from_cell();
                self.status_message = Some("Cells merged".to_string());
            }
            Some("deps") => {
                // Recompute model: a cell's effective inputs are ALL
                // bindings of the module above it — list the run's
                // top-level bindings instead of a per-cell graph.
                let defined: Vec<String> = self
                    .session
                    .var_previews_iter()
                    .map(|(name, _)| name.to_string())
                    .collect();
                if defined.is_empty() {
                    self.status_message = Some("No bindings defined by this cell".to_string());
                } else {
                    self.status_message = Some(format!("Defines: {}", defined.join(", ")));
                }
            }
            Some("tutorial") => {
                if let Some(idx_str) = parts.get(1) {
                    if let Ok(idx) = idx_str.parse::<usize>() {
                        self.start_tutorial_by_index(idx);
                    } else {
                        // Search by name
                        let query = parts[1..].join(" ").to_lowercase();
                        let tutorials = builtin_tutorials();
                        if let Some(tutorial) = tutorials
                            .into_iter()
                            .find(|t| t.title.to_lowercase().contains(&query))
                        {
                            self.start_tutorial_from(tutorial);
                        } else {
                            self.status_message = Some(format!("No tutorial matching: {}", query));
                        }
                    }
                } else {
                    self.start_tutorial();
                    self.status_message =
                        Some("Tutorial loaded. Press x to run code cells.".to_string());
                }
            }
            Some("tutorials") => {
                let tutorials = builtin_tutorials();
                let list: Vec<String> = tutorials
                    .iter()
                    .enumerate()
                    .map(|(i, t)| format!("{}: {}", i, t.title))
                    .collect();
                self.status_message = Some(format!("Tutorials: {}", list.join(", ")));
            }
            Some("goto") | Some("g") => {
                if let Some(idx_str) = parts.get(1) {
                    if let Ok(idx) = idx_str.parse::<usize>() {
                        let target = idx.saturating_sub(1); // 1-based to 0-based
                        if target < self.session.cells.len() {
                            self.session.selected_cell = target;
                            self.sync_editor_from_cell();
                            self.status_message = Some(format!("Cell {}", idx));
                        } else {
                            self.status_message = Some(format!(
                                "Cell {} out of range (1-{})",
                                idx,
                                self.session.cells.len()
                            ));
                        }
                    }
                } else {
                    self.status_message = Some("Usage: :goto <cell_number>".to_string());
                }
            }
            Some("clearc") | Some("cc") => {
                // Clear current cell's output only
                self.session.current_cell_mut().clear_output();
                self.status_message = Some(format!(
                    "Cell {} output cleared",
                    self.session.selected_cell + 1
                ));
            }
            Some("info") | Some("i") => {
                let cell = self.session.current_cell();
                let lines = cell.source.as_str().lines().count();
                let kind = match cell.kind {
                    CellKind::Code => "code",
                    CellKind::Markdown => "markdown",
                };
                let exec = cell
                    .execution_count
                    .map(|n| format!("#{}", n))
                    .unwrap_or_else(|| "not run".to_string());
                let out_type = cell
                    .output
                    .as_ref()
                    .map(|o| match o {
                        CellOutput::Value { .. } => "value",
                        CellOutput::Stream { .. } => "stream",
                        CellOutput::Error { .. } => "error",
                        CellOutput::Multi { .. } => "multi",
                        CellOutput::Empty => "empty",
                        _ => "other",
                    })
                    .unwrap_or("none");
                self.status_message = Some(format!(
                    "Cell {} | {} | {} lines | exec {} | output: {}",
                    self.session.selected_cell + 1,
                    kind,
                    lines,
                    exec,
                    out_type,
                ));
            }
            Some("expand") => {
                let cell_id = self.session.current_cell().id;
                self.collapsed_cells.remove(&cell_id);
                self.session.current_cell_mut().output_collapsed = false;
                self.status_message = Some("Cell expanded".to_string());
            }
            Some("collapse") => {
                let cell_id = self.session.current_cell().id;
                self.collapsed_cells.insert(cell_id);
                self.status_message = Some("Cell collapsed".to_string());
            }
            Some("toggleoutput") => {
                self.session.current_cell_mut().toggle_output_collapse();
                let state = if self.session.current_cell().output_collapsed {
                    "collapsed"
                } else {
                    "expanded"
                };
                self.status_message = Some(format!("Output {}", state));
            }
            Some("export") => {
                if parts.len() >= 3 {
                    let format = parts[1];
                    let path = parts[2];
                    match format {
                        "vr" => {
                            let content = super::persistence::export_to_verum(&self.session.cells);
                            match std::fs::write(path, content) {
                                Ok(()) => {
                                    self.status_message = Some(format!("Exported to {}", path))
                                }
                                Err(e) => {
                                    self.status_message = Some(format!("Export failed: {}", e))
                                }
                            }
                        }
                        "md" | "markdown" => {
                            let content =
                                super::persistence::export_to_markdown(&self.session.cells);
                            match std::fs::write(path, content) {
                                Ok(()) => {
                                    self.status_message = Some(format!("Exported to {}", path))
                                }
                                Err(e) => {
                                    self.status_message = Some(format!("Export failed: {}", e))
                                }
                            }
                        }
                        "html" => {
                            let content = super::persistence::export_to_html(&self.session.cells);
                            match std::fs::write(path, content) {
                                Ok(()) => {
                                    self.status_message = Some(format!("Exported to {}", path))
                                }
                                Err(e) => {
                                    self.status_message = Some(format!("Export failed: {}", e))
                                }
                            }
                        }
                        _ => {
                            self.status_message =
                                Some("Usage: :export <vr|md|html> <path>".to_string())
                        }
                    }
                } else {
                    self.status_message = Some("Usage: :export <vr|md|html> <path>".to_string());
                }
            }
            Some("diff") => {
                let cell_id = self.session.current_cell().id;
                let current = self.session.current_cell().source.to_string();
                if let Some(previous) = self.previous_cell_sources.get(&cell_id) {
                    if *previous == current {
                        self.status_message = Some("No changes since last execution".to_string());
                    } else {
                        let mut diffs = Vec::new();
                        let prev_lines: Vec<&str> = previous.lines().collect();
                        let curr_lines: Vec<&str> = current.lines().collect();
                        let max = prev_lines.len().max(curr_lines.len());
                        for i in 0..max {
                            let p = prev_lines.get(i).copied().unwrap_or("");
                            let c = curr_lines.get(i).copied().unwrap_or("");
                            if p != c {
                                if !p.is_empty() {
                                    diffs.push(format!("-{}: {}", i + 1, p));
                                }
                                if !c.is_empty() {
                                    diffs.push(format!("+{}: {}", i + 1, c));
                                }
                            }
                        }
                        self.status_message = Some(if diffs.len() > 3 {
                            format!("{} ... ({} more)", diffs[..3].join(" | "), diffs.len() - 3)
                        } else {
                            diffs.join(" | ")
                        });
                    }
                } else {
                    self.status_message =
                        Some("No previous version (cell not yet executed)".to_string());
                }
            }
            Some("theme") => {
                if let Some(name) = parts.get(1) {
                    match *name {
                        "cyberpunk" => {
                            self.theme = Theme::Cyberpunk;
                            self.status_message = Some("Theme: Cyberpunk".to_string());
                        }
                        "dark" => {
                            self.theme = Theme::Dark;
                            self.status_message = Some("Theme: Dark".to_string());
                        }
                        "light" => {
                            self.theme = Theme::Light;
                            self.status_message = Some("Theme: Light".to_string());
                        }
                        _ => {
                            self.status_message =
                                Some("Usage: :theme <cyberpunk|dark|light>".to_string())
                        }
                    }
                } else {
                    self.status_message = Some("Usage: :theme <cyberpunk|dark|light>".to_string());
                }
            }
            Some("settings") | Some("config") => {
                let mode = match self.keybindings.mode() {
                    KeybindingMode::Vim => "vim",
                    KeybindingMode::Standard => "standard",
                };
                let theme_str = match self.theme {
                    Theme::Cyberpunk => "cyberpunk",
                    Theme::Dark => "dark",
                    Theme::Light => "light",
                };
                let settings = format!(
                    "mode:{} | sidebar:{} | autosave:{}s | timeout:{}ms | theme:{}",
                    mode,
                    if self.layout_config.show_sidebar {
                        "on"
                    } else {
                        "off"
                    },
                    self.auto_save_interval_secs,
                    self.session.execution_timeout_ms,
                    theme_str,
                );
                self.status_message = Some(settings);
            }
            Some("help") | Some("h") => {
                self.status_message = Some(
                    "w/save q/quit wq e export<vr|md|html> clear clearc/cc run goto/g info/i set split merge deps expand collapse toggleoutput diff theme settings/config tutorial tutorials".to_string()
                );
            }
            Some(c) => {
                self.status_message = Some(format!("Unknown command: {}. Type :help for list.", c))
            }
            None => {}
        }
    }

    /// Search across all cells for a query string.
    fn perform_search(&mut self, query: &str) {
        self.search_results.clear();
        self.search_cursor = 0;
        if query.is_empty() {
            return;
        }

        let query_lower = query.to_lowercase();
        for (ci, cell) in self.session.cells.iter().enumerate() {
            for (li, line) in cell.source.as_str().lines().enumerate() {
                if line.to_lowercase().contains(&query_lower) {
                    self.search_results.push((ci, li));
                }
            }
        }

        if let Some(&(ci, _)) = self.search_results.first() {
            self.session.selected_cell = ci;
            self.sync_editor_from_cell();
            self.status_message = Some(format!("{} matches", self.search_results.len()));
        } else {
            self.status_message = Some("No matches".to_string());
        }
    }

    fn enter_edit_mode(&mut self) {
        // The editor may be STALE relative to the cell: navigation
        // syncs it, but programmatic source changes (open_book onto the
        // same index, replay, update_current_source from code) do not.
        // Entering edit with a stale buffer meant commit_edit on exit
        // OVERWROTE the cell with the old text — sync on entry unless
        // the buffer already matches.
        if self.editor.content() != self.session.current_cell().source.as_str() {
            self.sync_editor_from_cell();
        }
        self.mode = AppMode::Edit;
        self.editor.move_to_end(false);
    }

    fn exit_edit_mode(&mut self) {
        self.commit_edit();
        self.mode = AppMode::Normal;
    }

    fn commit_edit(&mut self) {
        let content = self.editor.content();
        if content != self.session.current_cell().source.as_str() {
            self.session.update_current_source(content);
        }
    }

    fn sync_editor_from_cell(&mut self) {
        let source = self.session.current_cell().source.to_string();
        self.editor.set_content(&source);
    }

    fn current_settings(&self) -> super::persistence::PlaybookSettings {
        super::persistence::PlaybookSettings {
            auto_save_interval_secs: self.auto_save_interval_secs,
            keybinding_mode: match self.keybindings.mode() {
                KeybindingMode::Vim => "vim".to_string(),
                KeybindingMode::Standard => "standard".to_string(),
            },
            show_sidebar: self.layout_config.show_sidebar,
            execution_timeout_ms: self.session.execution_timeout_ms,
        }
    }

    fn save(&mut self) {
        if let Some(path) = &self.file_path {
            let settings = self.current_settings();
            match super::persistence::save_playbook(path, &self.session.cells, Some(&settings)) {
                Ok(()) => {
                    self.session.dirty = false;
                    self.last_save = Some(Instant::now());
                    self.status_message = Some(format!("Saved: {}", path.display()));
                }
                Err(e) => self.status_message = Some(format!("Error saving: {}", e)),
            }
        } else {
            // No path yet — prompt for one
            self.mode = AppMode::SavePrompt;
            self.input_buffer.clear();
        }
    }

    /// Collect a filename for save-as, then write.
    fn dispatch_save_prompt(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
                self.input_buffer.clear();
                self.status_message = None;
            }
            KeyCode::Enter => {
                let path = self.input_buffer.trim().to_string();
                self.input_buffer.clear();
                self.mode = AppMode::Normal;
                if path.is_empty() {
                    self.status_message = Some("Save cancelled".to_string());
                } else {
                    // Append .vrbook if no extension given
                    let path = if !path.contains('.') {
                        format!("{}.vrbook", path)
                    } else {
                        path
                    };
                    self.file_path = Some(PathBuf::from(&path));
                    self.save();
                }
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Char(c) => {
                self.input_buffer.push(c);
            }
            _ => {}
        }
    }

    fn check_auto_save(&mut self) {
        if self.auto_save_interval_secs == 0 || !self.session.dirty || self.file_path.is_none() {
            return;
        }
        let should_save = self
            .last_save
            .is_none_or(|t| t.elapsed().as_secs() >= self.auto_save_interval_secs);
        if should_save {
            self.save();
        }
    }

    // ── Rendering ───────────────────────────────────────────────────────

    pub fn render(&self, frame: &mut Frame) {
        if let Some(gallery) = &self.gallery {
            self.render_gallery(frame, gallery);
            return;
        }
        let config = if self.layout_config.editor_fullscreen {
            LayoutConfig::fullscreen()
        } else {
            self.layout_config
        };
        let layout = PlaybookLayout::from_area_with_config(frame.area(), config);

        if !self.layout_config.editor_fullscreen {
            self.render_cells(frame, layout.content);
            if layout.sidebar.width > 0 {
                self.render_sidebar(frame, layout.sidebar);
            }
        }

        self.render_editor(frame, layout.editor);
        self.render_status(frame, layout.status);
        self.render_help(frame, layout.help);
        if self.show_help_overlay {
            self.render_help_overlay(frame);
        }
    }

    /// Open the launch gallery: guided tours from
    /// `builtin_tutorials()`, the working directory's recent books
    /// (newest first), and a blank sheet.
    pub fn open_gallery(&mut self) {
        let mut items: Vec<GalleryItem> = vec![GalleryItem::BlankSheet];
        for (index, t) in builtin_tutorials().iter().enumerate() {
            items.push(GalleryItem::Tour {
                index,
                title: t.title.clone(),
                description: t.description.clone(),
                minutes: t.estimated_minutes,
            });
        }
        let mut books: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(".")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                if path.extension().and_then(|x| x.to_str()) == Some("vrbook") {
                    let modified = e.metadata().ok()?.modified().ok()?;
                    Some((modified, path))
                } else {
                    None
                }
            })
            .collect();
        books.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, path) in books.into_iter().take(5) {
            items.push(GalleryItem::RecentBook { path });
        }
        self.gallery = Some(GalleryState { items, selected: 0 });
    }

    fn handle_gallery_key(&mut self, key: crossterm::event::KeyEvent) {
        let Some(gallery) = &mut self.gallery else { return };
        let len = gallery.items.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                gallery.selected = gallery.selected.checked_sub(1).unwrap_or(len - 1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                gallery.selected = (gallery.selected + 1) % len;
            }
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc => self.gallery = None,
            KeyCode::Enter => {
                let choice = gallery.items[gallery.selected].clone();
                self.gallery = None;
                match choice {
                    GalleryItem::BlankSheet => {}
                    GalleryItem::Tour { index, .. } => {
                        self.start_tutorial_by_index(index);
                    }
                    GalleryItem::RecentBook { path } => self.open_book(path),
                }
            }
            _ => {}
        }
    }

    /// Load a book into THIS app in place (the gallery's open path —
    /// `from_file` builds a fresh app, which would discard settings
    /// already applied by CLI flags).
    fn open_book(&mut self, path: PathBuf) {
        self.file_path = Some(path.clone());
        match super::persistence::load_playbook(&path) {
            Ok((cells, settings)) => {
                self.session = SessionState::with_cells(cells);
                if let Some(s) = settings {
                    self.auto_save_interval_secs = s.auto_save_interval_secs;
                    self.layout_config.show_sidebar = s.show_sidebar;
                    self.session.execution_timeout_ms = s.execution_timeout_ms;
                }
                self.status_message = Some(format!("Loaded: {}", path.display()));
            }
            Err(e) => self.status_message = Some(format!("Error loading: {}", e)),
        }
        self.sync_editor_from_cell();
    }

    fn render_gallery(&self, frame: &mut Frame, gallery: &GalleryState) {
        use ratatui::widgets::Clear;
        let area = frame.area();
        frame.render_widget(Clear, area);
        let mut lines: Vec<Line> = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  VERUM PLAYGROUND",
                Style::default().fg(Color::Cyan).bold(),
            )),
            Line::from(Span::styled(
                "  choose where to begin",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
        ];
        let mut last_section = "";
        for (i, item) in gallery.items.iter().enumerate() {
            let section = match item {
                GalleryItem::BlankSheet => "START",
                GalleryItem::Tour { .. } => "GUIDED TOURS",
                GalleryItem::RecentBook { .. } => "RECENT BOOKS",
            };
            if section != last_section {
                lines.push(Line::from(Span::styled(
                    format!("  {section}"),
                    Style::default().fg(Color::DarkGray).bold(),
                )));
                last_section = section;
            }
            let selected = i == gallery.selected;
            let marker = if selected { "▸ " } else { "  " };
            let style = if selected {
                Style::default().fg(Color::White).bold()
            } else {
                Style::default().fg(Color::Gray)
            };
            let text = match item {
                GalleryItem::BlankSheet => "blank sheet — an empty notebook".to_string(),
                GalleryItem::Tour {
                    title,
                    description,
                    minutes,
                    ..
                } => format!("{title} — {description} (~{minutes} min)"),
                GalleryItem::RecentBook { path } => {
                    format!("{}", path.display())
                }
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(marker, Style::default().fg(Color::Cyan)),
                Span::styled(text, style),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ↑/↓ select   Enter open   Esc blank sheet   q quit",
            Style::default().fg(Color::DarkGray),
        )));
        frame.render_widget(Paragraph::new(lines), area);
    }

    /// The COMPLETE key map, centered over everything (`?` opens,
    /// any key closes). The footer hint teaches the next three
    /// moves; this teaches the whole game.
    fn render_help_overlay(&self, frame: &mut Frame) {
        use ratatui::widgets::Clear;
        let area = frame.area();
        let w = area.width.min(78);
        let h = area.height.min(28);
        let rect = Rect {
            x: area.x + (area.width.saturating_sub(w)) / 2,
            y: area.y + (area.height.saturating_sub(h)) / 2,
            width: w,
            height: h,
        };
        let vim = matches!(self.keybindings.mode(), KeybindingMode::Vim);
        let nav = if vim { "j / k" } else { "↑ / ↓" };
        let edit = if vim { "i" } else { "Enter" };
        let run = if vim { "x" } else { "F5" };
        let run_all = if vim { "X" } else { "F9" };
        let new_cell = if vim { "o" } else { "Ins" };
        let del_cell = if vim { "D" } else { "Del" };
        let text = format!(
            "
  NAVIGATE   {nav}  cells      g / G  first / last cell
  EDIT       {edit}  edit cell    Esc  leave editor / modal
  RUN        {run}  run cell     {run_all}  run all    Ctrl+C  cancel
  CELLS      {new_cell}  new    {del_cell}  delete    K / J  move cell
  PANELS     Tab  sidebar tab   Ctrl+B  toggle   F11/Ctrl+F  modal editor
  FILES      Ctrl+S  save       :w / :e  command forms
  MODES      /  search    :  command    q  quit

  ── EDITOR (inside a cell) ──────────────────────────────────
  MOVE       Ctrl+←/→  by word     Home  smart home   PgUp/PgDn  page
  SELECT     Shift+move    Ctrl+A  all    type ( [ {{ \"  wraps selection
  LINES      Ctrl+D  duplicate     Ctrl+Shift+K  delete line
             Alt+↑/↓  move line    Ctrl+J  join    Ctrl+K  kill to eol
  BLOCKS     Tab / Shift+Tab  indent / dedent selection
             Ctrl+/  toggle // comment
  PAIRS      auto-close ( [ {{ \"    Enter inside {{}}  opens a block
  DELETE     Ctrl+Backspace / Ctrl+Del  word left / right
  UNDO       Ctrl+Z / Ctrl+Shift+Z (word-level coalescing)
  CLIPBOARD  Ctrl+C / X / V (system clipboard)
  RUN        F5 / Ctrl+R / Alt+Enter    Tab  complete word

                                  press any key to close"
        );
        frame.render_widget(Clear, rect);
        frame.render_widget(
            Paragraph::new(text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" KEY MAP — ? "),
                )
                .style(Style::default().bg(Color::Black).fg(Color::White)),
            rect,
        );
    }

    fn render_cells(&self, frame: &mut Frame, area: Rect) {
        let cells_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                format!(" {} ", self.title()),
                Style::default().fg(Color::Cyan).bold(),
            ));

        let inner = cells_block.inner(area);
        frame.render_widget(cells_block, area);
        if inner.height == 0 {
            return;
        }

        let cell_heights: Vec<u16> = self
            .session
            .cells
            .iter()
            .map(|cell| cell_height(cell, self.collapsed_cells.contains(&cell.id)))
            .collect();

        let selected = self.session.selected_cell;
        let height_before: u16 = cell_heights[..selected].iter().sum();
        let sel_h = cell_heights[selected];
        let available = inner.height;

        let scroll = if height_before < self.scroll_offset {
            height_before
        } else if height_before + sel_h > self.scroll_offset + available {
            (height_before + sel_h).saturating_sub(available)
        } else {
            self.scroll_offset
        };

        let mut y = inner.y;
        let mut cum: u16 = 0;

        for (idx, cell) in self.session.cells.iter().enumerate() {
            let h = cell_heights[idx];
            if cum + h <= scroll {
                cum += h;
                continue;
            }
            if y >= inner.y + inner.height {
                break;
            }

            let cell_area = Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: h.min(inner.y + inner.height - y),
            };

            let widget = CellWidget::new(cell)
                .selected(idx == selected)
                .collapsed(self.collapsed_cells.contains(&cell.id))
                .execution_number(cell.execution_count);

            frame.render_widget(widget, cell_area);
            y += h;
            cum += h;
        }
    }

    /// Ask `verum arch query --json` about the notebook-as-module and
    /// mirror the answer into lens lines. Subprocess on purpose: the
    /// lens speaks the same vocabulary as agents and the CLI — one
    /// derivation, three transports (accepted T0858 design).
    /// A cheap lens mirrors the notebook, so it refreshes exactly
    /// when its subject may have changed WHILE it is on screen:
    /// landing on the tab, and executing cells with the tab open. No
    /// manual refresh key exists to forget. (The Tiers lens is NOT
    /// here on purpose — it is expensive and runs only on demand.)
    fn refresh_lenses_if_active(&mut self) {
        if !self.layout_config.show_sidebar {
            return;
        }
        match self.sidebar_tab {
            SidebarTab::Arch => self.refresh_arch_lens(),
            SidebarTab::Vbc => self.refresh_vbc_lens(),
            _ => {}
        }
    }

    /// Compile the notebook-as-module through the SAME pipeline the
    /// interpreter uses and disassemble the resulting `VbcModule` —
    /// the lens shows the artifact, not a re-derivation. A fresh
    /// pipeline on purpose: the growing module carries its own
    /// bindings, and the live incremental parser must not be
    /// disturbed by an out-of-band compile.
    fn refresh_vbc_lens(&mut self) {
        let source = self.notebook_module_source();
        if source.trim().is_empty() {
            self.vbc_lens_text = String::new();
            return;
        }
        // The engine's compile IS the artifact path cells execute on
        // (T0858 slice 5) — the lens disassembles that module, not a
        // second derivation.
        self.vbc_lens_text = match self.session.engine.compile(&source) {
            Ok(module) => verum_vbc::disassemble::disassemble_module(&module),
            Err(e) => format!("; compile failed:\n; {e:?}"),
        };
    }

    /// Start a background `verum diff-tiers --json` over the growing
    /// module. One judgment at a time; the result lands in the lens
    /// via `poll_execution`.
    fn start_tiers_judge(&mut self) {
        use std::io::Write as _;
        if self.tiers_rx.is_some() {
            self.status_message = Some("Tier judgment already running".to_string());
            return;
        }
        let source = self.notebook_module_source();
        if source.trim().is_empty() {
            self.tiers_lens_lines = vec!["(empty notebook — nothing to judge)".to_string()];
            return;
        }
        let tmp = match tempfile::Builder::new().suffix(".vr").tempfile() {
            Ok(mut t) => match t.write_all(source.as_bytes()) {
                Ok(()) => t,
                Err(e) => {
                    self.tiers_lens_lines = vec![format!("judge failed: {e}")];
                    return;
                }
            },
            Err(e) => {
                self.tiers_lens_lines = vec![format!("judge failed: {e}")];
                return;
            }
        };
        let path = tmp.path().to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let lines = run_tiers_judge(&path);
            let _ = tx.send(lines);
        });
        self.tiers_tmp = Some(tmp);
        self.tiers_rx = Some(rx);
        self.tiers_started = Some(Instant::now());
        self.tiers_lens_lines = vec!["judging… (builds both tiers)".to_string()];
        self.journal_push("tiers.diff started");
    }

    /// Poll the in-flight tier judgment; called from the UI tick.
    fn poll_tiers_judge(&mut self) {
        let Some(rx) = &self.tiers_rx else { return };
        match rx.try_recv() {
            Ok(mut lines) => {
                if let Some(started) = self.tiers_started.take() {
                    lines.push(format!(
                        "# cost: {:.1}s",
                        started.elapsed().as_secs_f64()
                    ));
                }
                let verdict = lines
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "?".to_string());
                self.tiers_lens_lines = lines;
                self.tiers_rx = None;
                self.tiers_tmp = None;
                self.journal_push(format!("tiers.diff {verdict}"));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if let Some(started) = self.tiers_started {
                    self.tiers_lens_lines = vec![format!(
                        "judging… ({:.0}s — builds both tiers)",
                        started.elapsed().as_secs_f64()
                    )];
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.tiers_lens_lines = vec!["judge thread crashed".to_string()];
                self.tiers_rx = None;
                self.tiers_tmp = None;
                self.tiers_started = None;
            }
        }
    }

    /// Append a journal line: wall time, the chain address of the
    /// module the question was about (when it has one), the event.
    fn journal_push(&mut self, event: impl AsRef<str>) {
        let at = chrono::Local::now().format("%H:%M:%S");
        let address = super::persistence::chain_of_cells(&self.session.cells)
            .last()
            .map(|h| h[..8.min(h.len())].to_string())
            .unwrap_or_else(|| "--------".to_string());
        self.journal.push(format!("{at} {address} {}", event.as_ref()));
    }

    /// The growing module: every code cell in order (the accepted
    /// state law — state lives in the QUESTION).
    fn notebook_module_source(&self) -> String {
        let mut source = String::new();
        for cell in self.session.cells.iter().filter(|c| c.is_code()) {
            source.push_str(cell.source.as_str());
            source.push('\n');
        }
        source
    }

    fn refresh_arch_lens(&mut self) {
        let source = self.notebook_module_source();
        if source.trim().is_empty() {
            // An empty module is not a question — no subprocess, no
            // journal entry (same law as the VBC lens).
            self.arch_lens_lines = Vec::new();
            return;
        }
        self.arch_lens_lines = match arch_query_subprocess(&source) {
            Ok(lines) => lines,
            Err(e) => vec![format!("query failed: {e}")],
        };
        self.journal_push("arch.query");
    }

    fn render_sidebar(&self, frame: &mut Frame, area: Rect) {
        // Vars lens: top-level bindings of the LAST notebook run,
        // debug-rendered from that same run (the vocabulary
        // executor's VARS channel — T0858 slice 5). The
        // `__vrnb_result` capture cell is machinery, not a binding.
        let vars: Vec<VarInfo> = self
            .session
            .var_previews
            .iter()
            .filter(|(name, _)| name != "__vrnb_result")
            .map(|(name, preview)| VarInfo {
                name: name.clone(),
                type_info: String::new(),
                value_preview: preview.clone(),
                is_mutable: false,
            })
            .collect();

        // Function signatures come from the notebook's own source —
        // the parse the arch/VBC lenses already rely on.
        let funcs: Vec<FuncInfo> = Vec::new();

        let outline: Vec<OutlineEntry> = self
            .session
            .cells
            .iter()
            .enumerate()
            .map(|(i, cell)| OutlineEntry {
                index: i,
                kind: cell.kind,
                exec_number: cell.execution_count,
                first_line: cell
                    .source
                    .as_str()
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string(),
                has_error: cell.output.as_ref().is_some_and(|o| o.is_error()),
                is_dirty: cell.dirty,
                is_selected: i == self.session.selected_cell,
            })
            .collect();

        let code_count = self.session.cells.iter().filter(|c| c.is_code()).count();
        let md_count = self
            .session
            .cells
            .iter()
            .filter(|c| c.is_markdown())
            .count();
        let exec_count = self
            .session
            .cells
            .iter()
            .filter(|c| c.execution_count.is_some())
            .count();
        let err_count = self
            .session
            .cells
            .iter()
            .filter(|c| c.output.as_ref().is_some_and(|o| o.is_error()))
            .count();

        let stats = ExecStats {
            total_cells: self.session.cells.len(),
            code_cells: code_count,
            markdown_cells: md_count,
            executed_count: exec_count,
            error_count: err_count,
            binding_count: self.session.var_previews_iter().count(),
            function_count: 0,
            last_cell_source: self
                .session
                .current_cell()
                .source
                .as_str()
                .lines()
                .next()
                .unwrap_or("")
                .to_string(),
            last_exec_time_ms: self.last_exec_time_ms,
            last_instructions: self.last_instructions,
            last_peak_stack: self.last_peak_stack,
        };

        let sidebar = SidebarWidget::new()
            .tab(self.sidebar_tab)
            .variables(&vars)
            .functions(&funcs)
            .outline(&outline)
            .stats(stats)
            .arch_lines(&self.arch_lens_lines)
            .vbc_text(&self.vbc_lens_text)
            .tiers_lines(&self.tiers_lens_lines)
            .journal_lines(&self.journal);

        frame.render_widget(sidebar, area);
    }

    fn render_editor(&self, frame: &mut Frame, area: Rect) {
        let title = match self.mode {
            AppMode::Edit => format!("Cell {} [EDIT]", self.session.selected_cell + 1),
            AppMode::Command => format!(":{}", self.input_buffer),
            AppMode::Search => format!("/{}", self.input_buffer),
            AppMode::SavePrompt => format!("Save as: {}", self.input_buffer),
            _ => format!("Cell {}", self.session.selected_cell + 1),
        };

        let widget = EditorWidget::new(&self.editor)
            .title(title)
            .diagnostics(&self.diagnostics);

        frame.render_widget(widget, area);
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        let status = if let Some(msg) = &self.status_message {
            msg.clone()
        } else {
            let mode_str = match self.mode {
                AppMode::Normal => "NRM",
                AppMode::Edit => "EDT",
                AppMode::Command => "CMD",
                AppMode::Search => "SRC",
                AppMode::SavePrompt => "SAV",
            };
            let dirty = if self.session.dirty { " [+]" } else { "" };
            let cell_info = format!(
                "{}/{}",
                self.session.selected_cell + 1,
                self.session.cell_count()
            );
            let kb_mode = match self.keybindings.mode() {
                KeybindingMode::Vim => " VIM",
                KeybindingMode::Standard => "",
            };
            let time = if self.last_exec_time_ms > 0.0 {
                format!(" {:.1}ms", self.last_exec_time_ms)
            } else {
                String::new()
            };
            let auto_save = if self.auto_save_interval_secs > 0 {
                " [AS]"
            } else {
                ""
            };
            format!(
                " {} {} {}{}{}{}{}",
                mode_str,
                self.file_name(),
                cell_info,
                dirty,
                kb_mode,
                time,
                auto_save
            )
        };

        let style = match self.mode {
            AppMode::Normal => Style::default().bg(Color::DarkGray).fg(Color::White),
            AppMode::Edit => Style::default().bg(Color::Cyan).fg(Color::Black),
            AppMode::Command => Style::default().bg(Color::Magenta).fg(Color::White),
            AppMode::Search => Style::default().bg(Color::Yellow).fg(Color::Black),
            AppMode::SavePrompt => Style::default().bg(Color::Green).fg(Color::Black),
        };

        frame.render_widget(Paragraph::new(status).style(style), area);
    }

    fn render_help(&self, frame: &mut Frame, area: Rect) {
        // The visible map of next actions: 3-5 contextual keys, and
        // always the door to the full map — a wall of ten hints
        // teaches nothing (Playground Reborn §3, visible action map).
        let help = match self.mode {
            AppMode::Normal => match self.keybindings.mode() {
                KeybindingMode::Vim => {
                    " j/k:nav  i:edit  x:run  o:new cell  ?:all keys"
                }
                KeybindingMode::Standard => {
                    " ↑/↓:nav  Enter:edit  F5:run  Ins:new cell  ?:all keys"
                }
            },
            AppMode::Edit => {
                if self.layout_config.editor_fullscreen {
                    " Esc:back to notebook  Ctrl+R:run  Ctrl+D:dup  Ctrl+/:comment  ?-map via Esc"
                } else {
                    " Esc:done  Ctrl+R:run  Ctrl+F:modal editor  Ctrl+D:dup  Ctrl+/:comment  Ctrl+Z:undo"
                }
            }
            AppMode::Command => {
                " Esc:cancel  Enter:exec  Commands: w q wq e clear run set split merge help"
            }
            AppMode::Search => " Esc:cancel  Enter:confirm  Type to search across all cells",
            AppMode::SavePrompt => {
                " Esc:cancel  Enter:save  Type filename (.vrbook appended if no extension)"
            }
        };
        frame.render_widget(
            Paragraph::new(help).style(Style::default().fg(Color::DarkGray)),
            area,
        );
    }

    fn title(&self) -> String {
        format!("VERUM PLAYBOOK // {}", self.file_name().to_uppercase())
    }

    fn file_name(&self) -> String {
        self.file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("untitled.vrbook")
            .to_string()
    }

    // ── Public API ──────────────────────────────────────────────────────

    pub fn set_vim_mode(&mut self, enabled: bool) {
        self.keybindings.set_mode(if enabled {
            KeybindingMode::Vim
        } else {
            KeybindingMode::Standard
        });
    }

    pub fn set_profiling(&mut self, _enabled: bool) {
        // Profiling data now tracked via last_exec_time_ms and displayed in status bar
    }

    pub fn preload_file(&mut self, path: &str) -> io::Result<()> {
        let source = std::fs::read_to_string(path)?;
        self.session.insert_cell_after(CellKind::Code);
        self.session.update_current_source(source);
        if let Err(e) = self.session.execute_current() {
            return Err(io::Error::other(format!("Preload failed: {}", e)));
        }
        self.status_message = Some(format!("Preloaded: {}", path));
        self.sync_editor_from_cell();
        Ok(())
    }

    pub fn export_to_script(&self, path: &Path) -> io::Result<()> {
        std::fs::write(
            path,
            super::persistence::export_to_verum(&self.session.cells),
        )
    }

    pub fn export_to_script_with_outputs(&self, path: &Path) -> io::Result<()> {
        let mut script = format!(
            "// Exported from Verum Playbook\n// Date: {}\n\n",
            Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
        );
        if let Some(fp) = &self.file_path {
            script.insert_str(
                script.find('\n').unwrap_or(0) + 1,
                &format!("// Source: {}\n", fp.display()),
            );
        }
        for cell in &self.session.cells {
            match cell.kind {
                CellKind::Code => {
                    script.push_str(cell.source.as_str());
                    script.push('\n');
                    if let Some(output) = &cell.output {
                        let brief = super::ui::format_output_brief(output);
                        if !brief.is_empty() && brief != "()" {
                            let line = if brief.len() > 80 {
                                // Char-count truncation; `&brief[..77]` panics on
                                // any non-ASCII content in the cell's brief output.
                                let preview = verum_common::text_utf8::truncate_chars(&brief, 77);
                                format!("// -> {}...\n", preview)
                            } else {
                                format!("// -> {}\n", brief)
                            };
                            script.push_str(&line);
                        }
                    }
                    script.push('\n');
                }
                CellKind::Markdown => {
                    for line in cell.source.as_str().lines() {
                        script.push_str("// ");
                        script.push_str(line);
                        script.push('\n');
                    }
                    script.push('\n');
                }
            }
        }
        std::fs::write(path, script)
    }

    pub fn from_source(source: &str) -> Self {
        let mut app = Self::new();
        for (i, chunk) in source.split("\n\n").enumerate() {
            let trimmed = chunk.trim();
            if trimmed.is_empty() {
                continue;
            }
            let is_md = trimmed
                .lines()
                .all(|l| l.trim().is_empty() || l.trim().starts_with("//"));
            let (kind, content) = if is_md {
                let md = trimmed
                    .lines()
                    .map(|l| {
                        l.trim()
                            .strip_prefix("//")
                            .map_or(l.trim(), |s| s.trim_start())
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (CellKind::Markdown, md)
            } else {
                (CellKind::Code, trimmed.to_string())
            };
            if i == 0 {
                app.session.update_current_source(content);
            } else {
                app.session.insert_cell_after(kind);
                app.session.update_current_source(content);
            }
        }
        app.session.selected_cell = 0;
        app.sync_editor_from_cell();
        app
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        let settings = self.current_settings();
        super::persistence::save_playbook(path, &self.session.cells, Some(&settings))
            .map_err(|e| io::Error::other(e.to_string()))
    }

    pub fn add_diagnostic(
        &mut self,
        line: usize,
        col_start: usize,
        col_end: usize,
        message: String,
        severity: super::ui::DiagnosticSeverity,
    ) {
        self.diagnostics.push(super::ui::EditorDiagnostic {
            line,
            col_start,
            col_end,
            message,
            severity,
        });
    }

    pub fn clear_diagnostics(&mut self) {
        self.diagnostics.clear();
    }

    // ── Tutorial System ─────────────────────────────────────────────────

    /// Start the default introductory tutorial.
    ///
    /// Populates the playbook with a sequence of markdown and code cells
    /// that walk the user through fundamental Verum concepts: expressions,
    /// let bindings, functions, types, pattern matching, lists, and maps.
    pub fn start_tutorial(&mut self) {
        // The first by-example tour IS the intro — the hardcoded
        // welcome deck it replaced was a builtin copy of teaching
        // content, forbidden by the one-truth-of-examples law.
        self.start_tutorial_by_index(0);
    }

    /// Start a built-in tutorial by index (from `builtin_tutorials()`).
    ///
    /// Returns `false` if the index is out of range.
    pub fn start_tutorial_by_index(&mut self, index: usize) -> bool {
        let tutorials = builtin_tutorials();
        if let Some(tutorial) = tutorials.into_iter().nth(index) {
            self.start_tutorial_from(tutorial);
            true
        } else {
            self.status_message = Some(format!("Tutorial index {} out of range", index));
            false
        }
    }

    /// Start a tutorial from a `Tutorial` value, converting its steps into
    /// playbook cells (alternating markdown explanation and code example).
    pub fn start_tutorial_from(&mut self, tutorial: Tutorial) {
        let mut cells: Vec<(CellKind, String)> = Vec::new();

        // Title cell
        cells.push((
            CellKind::Markdown,
            format!(
                "# {}\n\n{}\n\nDifficulty: {}/5 | Estimated time: {} min",
                tutorial.title,
                tutorial.description,
                tutorial.difficulty,
                tutorial.estimated_minutes,
            ),
        ));

        for step in &tutorial.steps {
            // Explanation cell
            cells.push((
                CellKind::Markdown,
                format!("## {}\n\n{}", step.title, step.explanation),
            ));

            // Code example cell
            if let Some(code) = &step.example_code {
                cells.push((CellKind::Code, code.clone()));
            }

            // Exercise prompt cell (if any)
            if let Some(prompt) = &step.exercise_prompt {
                let mut exercise_md = format!("### Try it yourself\n\n{}", prompt);
                if let Some(hint) = &step.hint {
                    exercise_md.push_str(&format!("\n\n> Hint: {}", hint));
                }
                cells.push((CellKind::Markdown, exercise_md));

                // Empty code cell for the user to type in
                cells.push((CellKind::Code, "// Your answer here".to_string()));
            }
        }

        self.load_tutorial_cells(cells);
        self.status_message = Some(format!("Tutorial loaded: {}", tutorial.title));
    }

    /// Build the cells for the built-in introductory tutorial.
    ///
    /// This is the default tutorial shown when the user runs `:tutorial`
    /// or calls `start_tutorial()`. It covers the essential Verum concepts
    /// using correct Verum syntax (not Rust).
    /// Replace the current session with the given sequence of cells.
    fn load_tutorial_cells(&mut self, cells: Vec<(CellKind, String)>) {
        // Build a fresh session with the tutorial cells
        let mut new_cells: Vec<Cell> = Vec::with_capacity(cells.len());
        for (kind, content) in cells {
            let cell = match kind {
                CellKind::Code => Cell::new_code(content),
                CellKind::Markdown => Cell::new_markdown(content),
            };
            new_cells.push(cell);
        }

        self.session = SessionState::with_cells(new_cells);
        self.session.selected_cell = 0;
        self.sync_editor_from_cell();
    }
}

impl Default for PlaybookApp {
    fn default() -> Self {
        Self::new()
    }
}

/// Run `verum arch query --json` on `source` (via a temp file) and
/// flatten the report into display lines. Lives outside the app so
/// the TUI layer stays free of compiler dependencies — the subprocess
/// IS the dependency, and it is the same one agents use.
fn arch_query_subprocess(source: &str) -> anyhow::Result<Vec<String>> {
    use std::io::Write as _;
    let exe = std::env::current_exe()?;
    let mut tmp = tempfile::NamedTempFile::with_suffix(".vr")?;
    tmp.write_all(source.as_bytes())?;
    let out = std::process::Command::new(exe)
        .args(["arch", "query", "--json", "--at"])
        .arg(tmp.path())
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "{}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .next()
                .unwrap_or("arch query failed")
        );
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let mut lines = Vec::new();
    lines.push("# inferred surface".to_string());
    match v["inferred"].as_array() {
        Some(a) if !a.is_empty() => {
            for atom in a {
                lines.push(format!(
                    "{}  [{}]",
                    atom["atom"].as_str().unwrap_or("?"),
                    atom["evidence"].as_str().unwrap_or("?"),
                ));
            }
        }
        _ => lines.push("(empty)".to_string()),
    }
    if let Some(pinned) = v["pinned"].as_array() {
        lines.push(String::new());
        lines.push("# pinned".to_string());
        for p in pinned {
            lines.push(p.as_str().unwrap_or("?").to_string());
        }
        for e in v["escalations"].as_array().into_iter().flatten() {
            lines.push(format!(
                "ESCALATION {}",
                e["atom"].as_str().unwrap_or("?")
            ));
        }
        for d in v["dead_rights"].as_array().into_iter().flatten() {
            lines.push(format!("DEAD RIGHT {}", d.as_str().unwrap_or("?")));
        }
    }
    if let Some(unres) = v["unresolved_calls"].as_array() {
        if !unres.is_empty() {
            lines.push(String::new());
            lines.push(format!("# unresolved calls ({})", unres.len()));
            for u in unres.iter().take(12) {
                lines.push(u.as_str().unwrap_or("?").to_string());
            }
        }
    }
    Ok(lines)
}

/// Run `verum diff-tiers --json` on `path` and flatten the report
/// into lens lines. Free function: it runs on the judge thread and
/// touches no app state.
fn run_tiers_judge(path: &std::path::Path) -> Vec<String> {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return vec![format!("judge failed: {e}")],
    };
    let out = match std::process::Command::new(exe)
        .args(["diff-tiers", "--json"])
        .arg(path)
        .output()
    {
        Ok(o) => o,
        Err(e) => return vec![format!("judge failed: {e}")],
    };
    let report: serde_json::Value = match serde_json::from_slice(&out.stdout) {
        Ok(v) => v,
        Err(_) => {
            let err = String::from_utf8_lossy(&out.stderr);
            return vec![format!(
                "judge failed: {}",
                err.lines().next().unwrap_or("no JSON report")
            )];
        }
    };
    let mut lines = Vec::new();
    lines.push(format!(
        "# verdict: {}",
        report["verdict"].as_str().unwrap_or("?")
    ));
    for tier in ["tier0", "tier1"] {
        lines.push(String::new());
        lines.push(format!(
            "# {tier} (exit {})",
            report[tier]["exit"].as_str().unwrap_or("?")
        ));
        for l in report[tier]["stdout"]
            .as_str()
            .unwrap_or("")
            .lines()
            .take(12)
        {
            lines.push(l.to_string());
        }
    }
    if let Some(line) = report["first_divergence_line"].as_u64() {
        lines.push(String::new());
        lines.push(format!("DIVERGENT at program-output line {line}"));
    }
    lines
}

/// One choice on the launch gallery.
#[derive(Clone)]
enum GalleryItem {
    BlankSheet,
    Tour {
        index: usize,
        title: String,
        description: String,
        minutes: u32,
    },
    RecentBook {
        path: PathBuf,
    },
}

struct GalleryState {
    items: Vec<GalleryItem>,
    selected: usize,
}
