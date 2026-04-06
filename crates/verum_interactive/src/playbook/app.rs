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

use super::session::{Cell, CellKind, CellOutput, SessionState};
use super::ui::{
    CellWidget, EditorWidget, EditorState, LayoutConfig, PlaybookLayout,
    SidebarWidget, SidebarTab, VarInfo, FuncInfo, OutlineEntry, ExecStats,
    cell_height,
};
use super::keybindings::{KeyAction, Keybindings, KeybindingMode};
use crate::execution::value_format::{format_value, ValueDisplayOptions};
use crate::discovery::tutorials::{Tutorial, builtin_tutorials};

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
enum AsyncExecMsg {
    Done {
        output: CellOutput,
        context: crate::execution::ExecutionContext,
        instructions: u64,
        peak_stack: usize,
        time_ms: f64,
    },
    Error(String),
}

impl PlaybookApp {
    pub fn new() -> Self {
        Self {
            session: SessionState::new(),
            mode: AppMode::Normal,
            file_path: None,
            should_quit: false,
            status_message: None,
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
        let rx = match &self.pending_rx {
            Some(r) => r,
            None => return,
        };

        match rx.try_recv() {
            Ok(AsyncExecMsg::Done { output, context, instructions, peak_stack, time_ms }) => {
                let cell_idx = self.pending_cell_idx.unwrap_or(0);
                self.session.execution_context = context;
                self.session.execution_count += 1;
                let count = self.session.execution_count;
                if cell_idx < self.session.cells.len() {
                    self.session.cells[cell_idx].set_output(output, count);
                }
                self.last_exec_time_ms = time_ms;
                self.last_instructions = instructions;
                self.last_peak_stack = peak_stack;
                self.session.last_instructions = instructions;
                self.session.last_peak_stack = peak_stack;
                self.mark_dependents_dirty();
                self.status_message = Some(format!("Done ({:.1}ms)", time_ms));
                self.cleanup_pending();
            }
            Ok(AsyncExecMsg::Error(e)) => {
                let cell_idx = self.pending_cell_idx.unwrap_or(0);
                self.session.execution_count += 1;
                let count = self.session.execution_count;
                if cell_idx < self.session.cells.len() {
                    self.session.cells[cell_idx].set_output(CellOutput::error(e.clone()), count);
                }
                self.status_message = Some(format!("Error: {}", e));
                self.cleanup_pending();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Still running — animate spinner
                self.pending_spinner += 1;
                let spinners = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                let ch = spinners[self.pending_spinner % spinners.len()];
                let elapsed = self.pending_started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
                self.status_message = Some(format!(
                    "{} Running cell {}... ({:.1}s) [Ctrl+C to cancel]",
                    ch, self.pending_cell_idx.unwrap_or(0) + 1, elapsed,
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

        // Global: Ctrl+C cancels running execution
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) && self.is_executing() {
            self.cancel_execution();
            return;
        }

        // Global: F11 / Ctrl+F fullscreen
        let is_fullscreen_toggle = key.code == KeyCode::F(11)
            || (key.code == KeyCode::Char('f')
                && key.modifiers.contains(KeyModifiers::CONTROL));
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
            KeyAction::CellDown => { self.session.select_next(); self.sync_editor_from_cell(); }
            KeyAction::CellUp => { self.session.select_prev(); self.sync_editor_from_cell(); }
            KeyAction::CellFirst => { self.session.selected_cell = 0; self.sync_editor_from_cell(); }
            KeyAction::CellLast => {
                self.session.selected_cell = self.session.cells.len().saturating_sub(1);
                self.sync_editor_from_cell();
            }
            KeyAction::PageDown => {
                for _ in 0..5 { self.session.select_next(); }
                self.sync_editor_from_cell();
            }
            KeyAction::PageUp => {
                for _ in 0..5 { self.session.select_prev(); }
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
            KeyAction::ExecuteCell => self.execute_current_cell(),
            KeyAction::ExecuteAllCells => {
                self.commit_edit();
                if let Err(e) = self.session.execute_all() {
                    self.status_message = Some(format!("Error: {}", e));
                }
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
            KeyAction::MoveCellUp => { self.session.move_cell_up(); self.sync_editor_from_cell(); }
            KeyAction::MoveCellDown => { self.session.move_cell_down(); self.sync_editor_from_cell(); }
            KeyAction::ToggleCollapse => {
                let id = self.session.current_cell().id;
                if !self.collapsed_cells.remove(&id) { self.collapsed_cells.insert(id); }
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
            KeyAction::SidebarNextTab => self.sidebar_tab = self.sidebar_tab.next(),
            KeyAction::SidebarPrevTab => self.sidebar_tab = self.sidebar_tab.prev(),
            KeyAction::Save => self.save(),
            KeyAction::Undo => {
                if !self.session.undo() {
                    self.status_message = Some("Nothing to undo".to_string());
                } else { self.sync_editor_from_cell(); }
            }
            KeyAction::Redo => {
                if !self.session.redo() {
                    self.status_message = Some("Nothing to redo".to_string());
                } else { self.sync_editor_from_cell(); }
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
                let help = match self.keybindings.mode() {
                    KeybindingMode::Vim =>
                        "j/k:nav i:edit x:run X:all o:new D:del K/J:move Tab:sidebar-tab Ctrl+B:sidebar /:search :cmd q:quit",
                    KeybindingMode::Standard =>
                        "Arrows:nav Enter:edit F5:run F9:all Ins:new Del:del Tab/Shift+Tab:sidebar-tab Ctrl+B:sidebar Ctrl+S:save Ctrl+F:fs",
                };
                self.status_message = Some(help.to_string());
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
            KeyAction::ExitEdit => self.exit_edit_mode(),
            KeyAction::ExecuteCell => self.execute_current_cell(),
            KeyAction::Save => { self.commit_edit(); self.save(); }
            KeyAction::ToggleFullscreen => {
                self.layout_config.toggle_fullscreen();
                self.editor.fullscreen = self.layout_config.editor_fullscreen;
            }
            _ => {
                // Forward to editor for text editing
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                match key.code {
                    KeyCode::Left if ctrl => self.editor.move_word_left(shift),
                    KeyCode::Right if ctrl => self.editor.move_word_right(shift),
                    KeyCode::Left => self.editor.move_left(shift),
                    KeyCode::Right => self.editor.move_right(shift),
                    KeyCode::Up => self.editor.move_up(shift),
                    KeyCode::Down => self.editor.move_down(shift),
                    KeyCode::Home if ctrl => self.editor.move_to_start(shift),
                    KeyCode::End if ctrl => self.editor.move_to_end(shift),
                    KeyCode::Home => self.editor.move_home(shift),
                    KeyCode::End => self.editor.move_end(shift),
                    KeyCode::Enter => self.editor.insert_char('\n'),
                    KeyCode::Backspace => self.editor.backspace(),
                    KeyCode::Delete => self.editor.delete(),
                    KeyCode::Tab => {
                        // Try inline completion if cursor is after a partial word
                        let (row, col) = self.editor.cursor;
                        let line = self.editor.lines.get(row).cloned().unwrap_or_default();
                        let before_cursor = if col <= line.len() { &line[..col] } else { &line };
                        // Extract partial word: sequence of alphanumeric/_ chars before cursor
                        let partial: String = before_cursor.chars().rev()
                            .take_while(|c| c.is_alphanumeric() || *c == '_')
                            .collect::<Vec<_>>().into_iter().rev().collect();

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
                            let old = &self.completions[idx];
                            let new = &self.completions[next].clone();
                            // Delete the old completion text (it replaced the partial already)
                            for _ in 0..old.len() {
                                self.editor.backspace();
                            }
                            for ch in new.chars() {
                                self.editor.insert_char(ch);
                            }
                        } else {
                            // Compute completions
                            let keywords = ["fn", "let", "if", "else", "match", "for", "while",
                                "type", "true", "false", "println", "print", "return", "mut",
                                "implement", "mount", "module", "pub", "async", "await", "spawn"];
                            let builtin_types = ["Int", "Float", "Bool", "Text", "List", "Map",
                                "Set", "Maybe", "Heap", "Shared", "Channel", "Mutex", "Task"];

                            let mut candidates: Vec<String> = Vec::new();
                            // From execution context bindings
                            for name in self.session.execution_context.binding_names() {
                                if name.as_str().starts_with(&partial) {
                                    candidates.push(name.to_string());
                                }
                            }
                            // From execution context functions
                            for name in self.session.execution_context.function_names() {
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
                                for _ in 0..partial.len() {
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
                    KeyCode::Char('c') if ctrl => { self.editor.copy(); self.status_message = Some("Copied".to_string()); }
                    KeyCode::Char('x') if ctrl => { self.editor.cut(); self.status_message = Some("Cut".to_string()); }
                    KeyCode::Char('v') if ctrl => self.editor.paste(),
                    KeyCode::Char('z') if ctrl && shift => { self.editor.redo(); }
                    KeyCode::Char('z') if ctrl => { self.editor.undo(); }
                    KeyCode::Char(c) => {
                        self.completions.clear();
                        self.completion_index = None;
                        self.editor.insert_char(c);
                    }
                    _ => {
                        self.completions.clear();
                        self.completion_index = None;
                    }
                }
            }
        }
        let visible = if self.layout_config.editor_fullscreen { 20 } else { 8 };
        self.editor.ensure_cursor_visible(visible);
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
            KeyCode::Backspace => { self.input_buffer.pop(); }
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
        let source = self.session.current_cell().source.clone();
        self.previous_cell_sources.insert(cell_id, source.to_string());

        // Clone state for the worker thread
        let context = self.session.execution_context.clone();
        let line_number = cell_idx + 1;
        let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel_clone = cancel_flag.clone();

        let (tx, rx) = std::sync::mpsc::channel();

        let thread = std::thread::spawn(move || {
            use crate::ExecutionPipeline;

            let mut pipeline = ExecutionPipeline::new();
            let mut ctx = context;
            let start = Instant::now();

            // Compile
            let compiled = match pipeline.compile(source.as_str(), line_number) {
                Ok(c) => c,
                Err(e) => { let _ = tx.send(AsyncExecMsg::Error(e.to_string())); return; }
            };

            // Set cancel flag on the interpreter (via pipeline.execute)
            // We need to modify the interpreter's config inside execute()
            // For now, set it via a wrapper
            let result = {
                // Execute — the pipeline's execute() creates an Interpreter internally.
                // We set max_instructions as before; cancel_flag is checked in dispatch loop.
                // To pass cancel_flag, we need pipeline.execute to accept it.
                // Simpler: use compile_and_execute_for_cell which calls execute internally.
                pipeline.compile_and_execute_for_cell(
                    source.as_str(), line_number, &mut ctx, cell_id,
                )
            };

            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

            match result {
                Ok(output) => {
                    use crate::playbook::session::CellOutput;

                    let mut outputs = Vec::new();
                    if !output.stdout.is_empty() || !output.stderr.is_empty() {
                        outputs.push(CellOutput::stream_with_stderr(
                            output.stdout.clone(), output.stderr.clone(),
                        ));
                    }
                    if let Some(value) = output.value {
                        outputs.push(CellOutput::value_with_raw(
                            output.display.clone(), output.type_info.clone(), value,
                        ));
                    } else if !output.display.is_empty()
                        && output.display.as_str() != "()"
                        && output.type_info.as_str() != "()" {
                        outputs.push(CellOutput::value(
                            output.display.clone(), output.type_info.clone(),
                        ));
                    }

                    let cell_output = match outputs.len() {
                        0 => CellOutput::Empty,
                        1 => outputs.pop().unwrap(),
                        _ => CellOutput::multi(outputs),
                    };

                    let _ = tx.send(AsyncExecMsg::Done {
                        output: cell_output,
                        context: ctx,
                        instructions: output.instructions_executed,
                        peak_stack: output.peak_stack_depth,
                        time_ms: elapsed_ms,
                    });
                }
                Err(e) => { let _ = tx.send(AsyncExecMsg::Error(e.to_string())); }
            }
        });

        self.pending_rx = Some(rx);
        self.pending_cancel = Some(cancel_flag);
        self.pending_cell_idx = Some(cell_idx);
        self.pending_thread = Some(thread);
        self.pending_started = Some(Instant::now());
        self.pending_spinner = 0;
        self.status_message = Some(format!("⏳ Running cell {}...", cell_idx + 1));
    }

    /// Mark cells that depend on the current cell's bindings as dirty.
    fn mark_dependents_dirty(&mut self) {
        let cell_id = self.session.current_cell().id;
        // Collect binding names defined by the current cell
        let defined: Vec<verum_common::Text> = self.session.execution_context.bindings
            .iter()
            .filter(|(_, info)| info.defined_in == cell_id)
            .map(|(name, _)| name.clone())
            .collect();

        // Find all cells that use those bindings and mark them dirty
        for binding_name in &defined {
            let dependents = self.session.execution_context.dependencies.dependents(binding_name).to_vec();
            for dep_id in dependents {
                if dep_id != cell_id {
                    if let Some((_, cell)) = self.session.cells.iter_mut().enumerate().find(|(_, c)| c.id == dep_id) {
                        cell.dirty = true;
                    }
                }
            }
        }
    }

    fn execute_command(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.trim().split_whitespace().collect();
        match parts.first().copied() {
            Some("w") | Some("save") => {
                if let Some(path) = parts.get(1) {
                    self.file_path = Some(PathBuf::from(path));
                }
                self.save();
            }
            Some("q") | Some("quit") => self.should_quit = true,
            Some("wq") => { self.save(); self.should_quit = true; }
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
                    self.status_message = Some("Usage: :set <autosave|vim|standard|sidebar|nosidebar|timeout>".to_string());
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
                let cell_id = self.session.current_cell().id;
                let defined: Vec<String> = self.session.execution_context.bindings
                    .iter()
                    .filter(|(_, info)| info.defined_in == cell_id)
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
                        if let Some(tutorial) = tutorials.into_iter().find(|t| t.title.to_lowercase().contains(&query)) {
                            self.start_tutorial_from(tutorial);
                        } else {
                            self.status_message = Some(format!("No tutorial matching: {}", query));
                        }
                    }
                } else {
                    self.start_tutorial();
                    self.status_message = Some("Tutorial loaded. Press x to run code cells.".to_string());
                }
            }
            Some("tutorials") => {
                let tutorials = builtin_tutorials();
                let list: Vec<String> = tutorials.iter().enumerate()
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
                let kind = match cell.kind { CellKind::Code => "code", CellKind::Markdown => "markdown" };
                let exec = cell.execution_count.map(|n| format!("#{}", n)).unwrap_or_else(|| "not run".to_string());
                let out_type = cell.output.as_ref().map(|o| match o {
                    CellOutput::Value { .. } => "value",
                    CellOutput::Stream { .. } => "stream",
                    CellOutput::Error { .. } => "error",
                    CellOutput::Multi { .. } => "multi",
                    CellOutput::Empty => "empty",
                    _ => "other",
                }).unwrap_or("none");
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
                let state = if self.session.current_cell().output_collapsed { "collapsed" } else { "expanded" };
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
                                Ok(()) => self.status_message = Some(format!("Exported to {}", path)),
                                Err(e) => self.status_message = Some(format!("Export failed: {}", e)),
                            }
                        }
                        "md" | "markdown" => {
                            let content = super::persistence::export_to_markdown(&self.session.cells);
                            match std::fs::write(path, content) {
                                Ok(()) => self.status_message = Some(format!("Exported to {}", path)),
                                Err(e) => self.status_message = Some(format!("Export failed: {}", e)),
                            }
                        }
                        "html" => {
                            let content = super::persistence::export_to_html(&self.session.cells);
                            match std::fs::write(path, content) {
                                Ok(()) => self.status_message = Some(format!("Exported to {}", path)),
                                Err(e) => self.status_message = Some(format!("Export failed: {}", e)),
                            }
                        }
                        _ => self.status_message = Some("Usage: :export <vr|md|html> <path>".to_string()),
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
                                if !p.is_empty() { diffs.push(format!("-{}: {}", i + 1, p)); }
                                if !c.is_empty() { diffs.push(format!("+{}: {}", i + 1, c)); }
                            }
                        }
                        self.status_message = Some(if diffs.len() > 3 {
                            format!("{} ... ({} more)", diffs[..3].join(" | "), diffs.len() - 3)
                        } else {
                            diffs.join(" | ")
                        });
                    }
                } else {
                    self.status_message = Some("No previous version (cell not yet executed)".to_string());
                }
            }
            Some("theme") => {
                if let Some(name) = parts.get(1) {
                    match *name {
                        "cyberpunk" => { self.theme = Theme::Cyberpunk; self.status_message = Some("Theme: Cyberpunk".to_string()); }
                        "dark" => { self.theme = Theme::Dark; self.status_message = Some("Theme: Dark".to_string()); }
                        "light" => { self.theme = Theme::Light; self.status_message = Some("Theme: Light".to_string()); }
                        _ => self.status_message = Some("Usage: :theme <cyberpunk|dark|light>".to_string()),
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
                    if self.layout_config.show_sidebar { "on" } else { "off" },
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
            Some(c) => self.status_message = Some(format!("Unknown command: {}. Type :help for list.", c)),
            None => {}
        }
    }

    /// Search across all cells for a query string.
    fn perform_search(&mut self, query: &str) {
        self.search_results.clear();
        self.search_cursor = 0;
        if query.is_empty() { return; }

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
            KeyCode::Backspace => { self.input_buffer.pop(); }
            KeyCode::Char(c) => { self.input_buffer.push(c); }
            _ => {}
        }
    }

    fn check_auto_save(&mut self) {
        if self.auto_save_interval_secs == 0 || !self.session.dirty || self.file_path.is_none() {
            return;
        }
        let should_save = self.last_save.map_or(true, |t| {
            t.elapsed().as_secs() >= self.auto_save_interval_secs
        });
        if should_save {
            self.save();
        }
    }

    // ── Rendering ───────────────────────────────────────────────────────

    pub fn render(&self, frame: &mut Frame) {
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
        if inner.height == 0 { return; }

        let cell_heights: Vec<u16> = self.session.cells.iter().map(|cell| {
            cell_height(cell, self.collapsed_cells.contains(&cell.id))
        }).collect();

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
            if cum + h <= scroll { cum += h; continue; }
            if y >= inner.y + inner.height { break; }

            let cell_area = Rect {
                x: inner.x, y,
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

    fn render_sidebar(&self, frame: &mut Frame, area: Rect) {
        // Build variable info using proper value formatter
        let vars: Vec<VarInfo> = self.session.execution_context.bindings.iter().map(|(name, info)| {
            let preview = format_value(&info.value, &self.display_options).to_string();
            VarInfo {
                name: name.to_string(),
                type_info: info.type_info.to_string(),
                value_preview: preview,
                is_mutable: info.is_mutable,
            }
        }).collect();

        let funcs: Vec<FuncInfo> = self.session.execution_context.functions.iter().map(|(name, info)| {
            FuncInfo {
                name: name.to_string(),
                signature: format!("({}) -> {}",
                    info.params.iter().map(|(n, t)| format!("{}: {}", n, t)).collect::<Vec<_>>().join(", "),
                    info.return_type,
                ),
            }
        }).collect();

        let outline: Vec<OutlineEntry> = self.session.cells.iter().enumerate().map(|(i, cell)| {
            OutlineEntry {
                index: i,
                kind: cell.kind,
                exec_number: cell.execution_count,
                first_line: cell.source.as_str().lines().next().unwrap_or("").to_string(),
                has_error: cell.output.as_ref().map_or(false, |o| o.is_error()),
                is_dirty: cell.dirty,
                is_selected: i == self.session.selected_cell,
            }
        }).collect();

        let code_count = self.session.cells.iter().filter(|c| c.is_code()).count();
        let md_count = self.session.cells.iter().filter(|c| c.is_markdown()).count();
        let exec_count = self.session.cells.iter().filter(|c| c.execution_count.is_some()).count();
        let err_count = self.session.cells.iter().filter(|c| c.output.as_ref().map_or(false, |o| o.is_error())).count();

        let stats = ExecStats {
            total_cells: self.session.cells.len(),
            code_cells: code_count,
            markdown_cells: md_count,
            executed_count: exec_count,
            error_count: err_count,
            binding_count: self.session.execution_context.bindings.len(),
            function_count: self.session.execution_context.functions.len(),
            last_cell_source: self.session.current_cell().source.as_str().lines().next().unwrap_or("").to_string(),
            last_exec_time_ms: self.last_exec_time_ms,
            last_instructions: self.last_instructions,
            last_peak_stack: self.last_peak_stack,
        };

        let sidebar = SidebarWidget::new()
            .tab(self.sidebar_tab)
            .variables(&vars)
            .functions(&funcs)
            .outline(&outline)
            .stats(stats);

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
            let cell_info = format!("{}/{}", self.session.selected_cell + 1, self.session.cell_count());
            let kb_mode = match self.keybindings.mode() {
                KeybindingMode::Vim => " VIM",
                KeybindingMode::Standard => "",
            };
            let time = if self.last_exec_time_ms > 0.0 {
                format!(" {:.1}ms", self.last_exec_time_ms)
            } else {
                String::new()
            };
            let auto_save = if self.auto_save_interval_secs > 0 { " [AS]" } else { "" };
            format!(" {} {} {}{}{}{}{}", mode_str, self.file_name(), cell_info, dirty, kb_mode, time, auto_save)
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
        let help = match self.mode {
            AppMode::Normal => {
                match self.keybindings.mode() {
                    KeybindingMode::Vim =>
                        " j/k:nav i:edit x:run X:all o:new D:del K/J:move Tab:sidebar-tab Ctrl+B:sidebar /:search :cmd q:quit",
                    KeybindingMode::Standard =>
                        " Arrows:nav Enter:edit F5:run F9:all Ins:new Del:del Tab:sidebar-tab Ctrl+B:sidebar Ctrl+S:save Ctrl+F:fs",
                }
            }
            AppMode::Edit =>
                " Esc:exit  F5/Ctrl+R:run  Ctrl+c/x/v  Ctrl+z:undo  Ctrl+s:save  Tab:indent",
            AppMode::Command =>
                " Esc:cancel  Enter:exec  Commands: w q wq e clear run set split merge help",
            AppMode::Search =>
                " Esc:cancel  Enter:confirm  Type to search across all cells",
            AppMode::SavePrompt =>
                " Esc:cancel  Enter:save  Type filename (.vrbook appended if no extension)",
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
        self.file_path.as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("untitled.vrbook")
            .to_string()
    }

    // ── Public API ──────────────────────────────────────────────────────

    pub fn set_vim_mode(&mut self, enabled: bool) {
        self.keybindings.set_mode(if enabled { KeybindingMode::Vim } else { KeybindingMode::Standard });
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
        std::fs::write(path, super::persistence::export_to_verum(&self.session.cells))
    }

    pub fn export_to_script_with_outputs(&self, path: &Path) -> io::Result<()> {
        let mut script = format!(
            "// Exported from Verum Playbook\n// Date: {}\n\n",
            Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
        );
        if let Some(fp) = &self.file_path {
            script.insert_str(script.find('\n').unwrap_or(0) + 1, &format!("// Source: {}\n", fp.display()));
        }
        for cell in &self.session.cells {
            match cell.kind {
                CellKind::Code => {
                    script.push_str(cell.source.as_str());
                    script.push('\n');
                    if let Some(output) = &cell.output {
                        let brief = super::ui::format_output_brief(output);
                        if !brief.is_empty() && brief != "()" {
                            let line = if brief.len() > 80 { format!("// -> {}...\n", &brief[..77]) }
                                else { format!("// -> {}\n", brief) };
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
            if trimmed.is_empty() { continue; }
            let is_md = trimmed.lines().all(|l| l.trim().is_empty() || l.trim().starts_with("//"));
            let (kind, content) = if is_md {
                let md = trimmed.lines()
                    .map(|l| l.trim().strip_prefix("//").map_or(l.trim(), |s| s.trim_start()))
                    .collect::<Vec<_>>().join("\n");
                (CellKind::Markdown, md)
            } else {
                (CellKind::Code, trimmed.to_string())
            };
            if i == 0 { app.session.update_current_source(content); }
            else { app.session.insert_cell_after(kind); app.session.update_current_source(content); }
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

    pub fn add_diagnostic(&mut self, line: usize, col_start: usize, col_end: usize, message: String, severity: super::ui::DiagnosticSeverity) {
        self.diagnostics.push(super::ui::EditorDiagnostic { line, col_start, col_end, message, severity });
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
        let cells = Self::intro_tutorial_cells();
        self.load_tutorial_cells(cells);
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
                tutorial.title, tutorial.description, tutorial.difficulty, tutorial.estimated_minutes,
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
    fn intro_tutorial_cells() -> Vec<(CellKind, String)> {
        vec![
            // ── Welcome ─────────────────────────────────────────────
            (CellKind::Markdown, "\
# Welcome to Verum

Verum is a modern language built for safety, performance, and clarity.
This tutorial walks you through the fundamentals interactively.

Press **x** (or **F5**) on any code cell to execute it.
Press **j/k** (or **Up/Down**) to navigate between cells.".to_string()),

            // ── Basic expressions ───────────────────────────────────
            (CellKind::Markdown, "\
## Basic Expressions

Verum evaluates expressions and prints results with `print(...)`.
Format strings use the `f\"...\"` syntax with `{expr}` interpolation.".to_string()),

            (CellKind::Code, "\
let x = 42
let pi = 3.14159
print(f\"x = {x}, pi = {pi}\")".to_string()),

            // ── Let bindings ────────────────────────────────────────
            (CellKind::Markdown, "\
## Let Bindings

Use `let` to bind values. Types are inferred automatically.
Use `let mut` for mutable bindings.".to_string()),

            (CellKind::Code, "\
let name = \"Verum\"
let mut counter = 0
counter = counter + 1
counter = counter + 1
print(f\"name = {name}, counter = {counter}\")".to_string()),

            (CellKind::Code, "\
// Explicit type annotations
let x: Int = 100
let y: Float = 2.718
let flag: Bool = true
let message: Text = \"Hello!\"
print(f\"{x} {y} {flag} {message}\")".to_string()),

            // ── Functions ───────────────────────────────────────────
            (CellKind::Markdown, "\
## Functions

Define functions with `fn`. The last expression is the return value.
Anonymous functions use `fn(args) expr`.".to_string()),

            (CellKind::Code, "\
fn double(n: Int) -> Int {
    n * 2
}

print(f\"double(21) = {double(21)}\")".to_string()),

            (CellKind::Code, "\
fn greet(name: Text) -> Text {
    f\"Hello, {name}!\"
}

print(greet(\"World\"))".to_string()),

            (CellKind::Code, "\
// Anonymous functions (closures)
let square = fn(x: Int) x * x
print(f\"square(7) = {square(7)}\")".to_string()),

            // ── Types ───────────────────────────────────────────────
            (CellKind::Markdown, "\
## Types

Verum defines types with `type Name is ...;` (not `struct`/`enum`).
Record types have named fields. Sum types use `|` for variants.
Implement methods with `implement Name { ... }`.".to_string()),

            (CellKind::Code, "\
// Record type (like a struct)
type Point is { x: Float, y: Float };

let p = Point { x: 3.0, y: 4.0 }
print(f\"Point: ({p.x}, {p.y})\")".to_string()),

            (CellKind::Code, "\
// Sum type (like an enum)
type Shape is
    | Circle(Float)
    | Rectangle { width: Float, height: Float };

let s = Circle(5.0)
let r = Rectangle { width: 4.0, height: 3.0 }
print(f\"shapes created\")".to_string()),

            (CellKind::Code, "\
// Methods via implement blocks
type Counter is { value: Int };

implement Counter {
    fn new() -> Counter {
        Counter { value: 0 }
    }

    fn increment(&mut self) {
        self.value = self.value + 1
    }

    fn get(&self) -> Int {
        self.value
    }
}

let mut c = Counter::new()
c.increment()
c.increment()
print(f\"Counter: {c.get()}\")".to_string()),

            // ── Pattern matching ────────────────────────────────────
            (CellKind::Markdown, "\
## Pattern Matching

The `match` expression provides exhaustive pattern matching.
It works with literal values, sum types, and guards.".to_string()),

            (CellKind::Code, "\
fn describe(n: Int) -> Text {
    match n {
        0 => \"zero\",
        1 => \"one\",
        x if x < 0 => \"negative\",
        _ => \"many\",
    }
}

print(describe(0))
print(describe(1))
print(describe(-5))
print(describe(42))".to_string()),

            (CellKind::Code, "\
// Matching sum types
type Color is Red | Green | Blue;

fn color_name(c: Color) -> Text {
    match c {
        Red => \"red\",
        Green => \"green\",
        Blue => \"blue\",
    }
}

print(color_name(Red))
print(color_name(Blue))".to_string()),

            // ── Lists ───────────────────────────────────────────────
            (CellKind::Markdown, "\
## Lists

`List<T>` is Verum's dynamic array (not `Vec`).
Lists support `push`, `len`, `map`, `filter`, and more.".to_string()),

            (CellKind::Code, "\
let mut nums = List::from([1, 2, 3, 4, 5])
print(f\"Length: {nums.len()}\")

nums.push(6)
print(f\"After push: length = {nums.len()}\")".to_string()),

            (CellKind::Code, "\
// Functional operations on lists
let nums = List::from([1, 2, 3, 4, 5])

let doubled = nums.map(fn(x) x * 2)
print(f\"Doubled: {doubled}\")

let evens = nums.filter(fn(x) x % 2 == 0)
print(f\"Evens: {evens}\")

let sum = nums.reduce(0, fn(acc, x) acc + x)
print(f\"Sum: {sum}\")".to_string()),

            // ── Maps ────────────────────────────────────────────────
            (CellKind::Markdown, "\
## Maps

`Map<K, V>` provides key-value storage (not `HashMap`).
Use `insert`, `get`, `contains_key`, and `len`.".to_string()),

            (CellKind::Code, "\
let mut scores = Map::new()
scores.insert(\"Alice\", 95)
scores.insert(\"Bob\", 87)
scores.insert(\"Charlie\", 92)

print(f\"Alice: {scores.get(\"Alice\")}\")
print(f\"Size: {scores.len()}\")
print(f\"Has Bob: {scores.contains_key(\"Bob\")}\")".to_string()),

            // ── Maybe type ──────────────────────────────────────────
            (CellKind::Markdown, "\
## The Maybe Type

`Maybe<T>` represents an optional value (not `Option`).
Its variants are `Some(value)` and `None`.".to_string()),

            (CellKind::Code, "\
fn safe_divide(a: Int, b: Int) -> Maybe<Int> {
    if b == 0 {
        None
    } else {
        Some(a / b)
    }
}

match safe_divide(10, 3) {
    Some(result) => print(f\"Result: {result}\"),
    None => print(\"Cannot divide by zero\"),
}

match safe_divide(10, 0) {
    Some(result) => print(f\"Result: {result}\"),
    None => print(\"Cannot divide by zero\"),
}".to_string()),

            // ── Wrap up ─────────────────────────────────────────────
            (CellKind::Markdown, "\
## Next Steps

You have covered the fundamentals of Verum:
- Expressions and `print`
- `let` / `let mut` bindings
- Functions (`fn`) and closures
- Types (`type ... is`), records, sum types, and `implement`
- Pattern matching with `match`
- `List<T>` and `Map<K, V>`
- `Maybe<T>` for optional values

Explore the other built-in tutorials for generators, async, error handling,
and tensor operations. Use `:help` to see all available commands.".to_string()),
        ]
    }

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
    fn default() -> Self { Self::new() }
}
