//! Playbook session state management
//!
//! This module manages the execution state for a playbook session, including:
//! - Cell management (create, delete, reorder)
//! - VBC execution pipeline integration
//! - Cross-cell state preservation via ExecutionContext
//! - Undo/redo history

use verum_common::Text;
use verum_ast::FileId;

use super::cell::{Cell, CellId, CellKind, CellOutput, TensorStats};
use crate::execution::{ExecutionContext, ExecutionError, ExecutionPipeline};
use crate::IncrementalScriptParser;

/// Session state for a playbook
///
/// Manages the complete state of a playbook session including:
/// - Cell contents and outputs
/// - VBC execution pipeline for actual code execution
/// - Cross-cell state preservation (variables, functions)
/// - Undo/redo history
pub struct SessionState {
    /// All cells in order
    pub cells: Vec<Cell>,
    /// Currently selected cell index
    pub selected_cell: usize,
    /// Incremental parser for the session (provides caching and dependency tracking)
    pub parser: IncrementalScriptParser,
    /// Execution pipeline for VBC compilation and execution
    pub pipeline: ExecutionPipeline,
    /// Cross-cell execution context (bindings, functions)
    pub execution_context: ExecutionContext,
    /// Execution counter
    pub execution_count: u32,
    /// Undo history
    undo_stack: Vec<SessionSnapshot>,
    /// Redo history
    redo_stack: Vec<SessionSnapshot>,
    /// Maximum undo history size
    max_undo: usize,
    /// File ID for the session
    pub file_id: FileId,
    /// Whether the session has unsaved changes
    pub dirty: bool,
    /// Execution timeout in milliseconds (default 5000)
    pub execution_timeout_ms: u64,
    /// Instructions executed in last run.
    pub last_instructions: u64,
    /// Peak stack depth in last run.
    pub last_peak_stack: usize,
}

impl SessionState {
    /// Create a new empty session
    pub fn new() -> Self {
        let file_id = FileId::new(1);
        Self {
            cells: vec![Cell::new_code("")],
            selected_cell: 0,
            parser: IncrementalScriptParser::new(),
            pipeline: ExecutionPipeline::with_file_id(file_id),
            execution_context: ExecutionContext::new(),
            execution_count: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_undo: 100,
            file_id,
            dirty: false,
            execution_timeout_ms: 5000,
            last_instructions: 0,
            last_peak_stack: 0,
        }
    }

    /// Create a session with initial cells
    pub fn with_cells(cells: Vec<Cell>) -> Self {
        let mut session = Self::new();
        session.cells = if cells.is_empty() {
            vec![Cell::new_code("")]
        } else {
            cells
        };
        session
    }

    /// Get the currently selected cell
    pub fn current_cell(&self) -> &Cell {
        &self.cells[self.selected_cell]
    }

    /// Get the currently selected cell mutably
    pub fn current_cell_mut(&mut self) -> &mut Cell {
        &mut self.cells[self.selected_cell]
    }

    /// Move selection up
    pub fn select_prev(&mut self) {
        if self.selected_cell > 0 {
            self.selected_cell -= 1;
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if self.selected_cell < self.cells.len() - 1 {
            self.selected_cell += 1;
        }
    }

    /// Insert a new cell after the current one
    pub fn insert_cell_after(&mut self, kind: CellKind) {
        self.save_undo_state();
        let cell = match kind {
            CellKind::Code => Cell::new_code(""),
            CellKind::Markdown => Cell::new_markdown(""),
        };
        self.cells.insert(self.selected_cell + 1, cell);
        self.selected_cell += 1;
        self.dirty = true;
    }

    /// Insert a new cell before the current one
    pub fn insert_cell_before(&mut self, kind: CellKind) {
        self.save_undo_state();
        let cell = match kind {
            CellKind::Code => Cell::new_code(""),
            CellKind::Markdown => Cell::new_markdown(""),
        };
        self.cells.insert(self.selected_cell, cell);
        self.dirty = true;
    }

    /// Move current cell up.
    pub fn move_cell_up(&mut self) {
        if self.selected_cell > 0 {
            self.save_undo_state();
            self.cells.swap(self.selected_cell, self.selected_cell - 1);
            self.selected_cell -= 1;
            self.dirty = true;
        }
    }

    /// Move current cell down.
    pub fn move_cell_down(&mut self) {
        if self.selected_cell < self.cells.len() - 1 {
            self.save_undo_state();
            self.cells.swap(self.selected_cell, self.selected_cell + 1);
            self.selected_cell += 1;
            self.dirty = true;
        }
    }

    /// Toggle collapse state of the current cell.
    pub fn toggle_collapse(&mut self) {
        // Collapse is tracked externally by app, this is a no-op placeholder
    }

    /// Check if a cell is collapsed (tracked externally, always false here).
    pub fn is_collapsed(&self, _id: CellId) -> bool {
        false
    }

    /// Toggle cell type between Code and Markdown.
    pub fn toggle_cell_type(&mut self) {
        self.save_undo_state();
        let cell = &mut self.cells[self.selected_cell];
        cell.kind = match cell.kind {
            CellKind::Code => CellKind::Markdown,
            CellKind::Markdown => CellKind::Code,
        };
        cell.output = None;
        cell.dirty = true;
        self.dirty = true;
    }

    /// Delete the current cell
    pub fn delete_current_cell(&mut self) {
        if self.cells.len() > 1 {
            self.save_undo_state();
            self.cells.remove(self.selected_cell);
            if self.selected_cell >= self.cells.len() {
                self.selected_cell = self.cells.len() - 1;
            }
            self.dirty = true;
        }
    }

    /// Split current cell at a given line index.
    pub fn split_cell(&mut self, at_line: usize) {
        let source = self.current_cell().source.clone();
        let lines: Vec<&str> = source.as_str().lines().collect();
        if at_line == 0 || at_line >= lines.len() { return; }
        self.save_undo_state();
        let first: String = lines[..at_line].join("\n");
        let second: String = lines[at_line..].join("\n");
        let kind = self.current_cell().kind;
        self.current_cell_mut().set_source(first.as_str());
        self.current_cell_mut().output = None;
        let new_cell = match kind {
            CellKind::Code => Cell::new_code(second.as_str()),
            CellKind::Markdown => Cell::new_markdown(second.as_str()),
        };
        self.cells.insert(self.selected_cell + 1, new_cell);
        self.dirty = true;
    }

    /// Merge current cell with the next one.
    pub fn merge_with_next(&mut self) {
        if self.selected_cell < self.cells.len() - 1 {
            self.save_undo_state();
            let next_source = self.cells[self.selected_cell + 1].source.clone();
            let current_source = self.current_cell().source.clone();
            let merged = format!("{}\n{}", current_source, next_source);
            self.current_cell_mut().set_source(merged.as_str());
            self.current_cell_mut().output = None;
            self.cells.remove(self.selected_cell + 1);
            self.dirty = true;
        }
    }

    /// Update the source of the current cell
    pub fn update_current_source(&mut self, source: impl Into<Text>) {
        self.save_undo_state();
        self.current_cell_mut().set_source(source);
        self.dirty = true;
    }

    /// Execute the current cell and return any diagnostics (line, message pairs).
    ///
    /// Diagnostics are extracted from parse errors to enable editor underlines.
    pub fn execute_current_with_diagnostics(&mut self) -> (Result<(), Text>, Vec<(usize, String)>) {
        let result = self.execute_current();
        let mut diagnostics = Vec::new();

        // Extract diagnostics from the output if it's an error
        if let Some(CellOutput::Error { message, .. }) = &self.current_cell().output {
            // Parse errors often contain line info like "line 3: unexpected token"
            for line in message.as_str().lines() {
                diagnostics.push((0, line.to_string()));
            }
        }

        (result, diagnostics)
    }

    /// Execute the current cell.
    pub fn execute_current(&mut self) -> Result<(), Text> {
        if !self.current_cell().is_code() {
            return Ok(()); // Markdown cells don't execute
        }

        let source = self.current_cell().source.clone();
        let cell_id = self.current_cell().id;
        self.execution_count += 1;
        let count = self.execution_count;

        // Clear any previous bindings from this cell (for re-execution)
        self.execution_context.clear_cell_bindings(cell_id);

        // Use incremental parser line number
        let line_number = self.selected_cell + 1;

        // Compile and execute using the VBC pipeline, tracking bindings for sidebar
        match self.pipeline.compile_and_execute_for_cell(
            source.as_str(),
            line_number,
            &mut self.execution_context,
            cell_id,
        ) {
            Ok(exec_output) => {
                // Capture VBC execution stats
                self.last_instructions = exec_output.instructions_executed;
                self.last_peak_stack = exec_output.peak_stack_depth;

                // Build the output combining value, streams, and timing
                let mut outputs = Vec::new();

                // Add stream output if there was stdout/stderr
                if !exec_output.stdout.is_empty() || !exec_output.stderr.is_empty() {
                    outputs.push(CellOutput::stream_with_stderr(
                        exec_output.stdout.clone(),
                        exec_output.stderr.clone(),
                    ));
                }

                // Add the value output only if it carries meaningful information.
                // Unit "()" is NOT a meaningful result — suppress it, especially
                // when the cell already produced stdout (e.g. print("ok")).
                if let Some(value) = exec_output.value {
                    outputs.push(CellOutput::value_with_raw(
                        exec_output.display.clone(),
                        exec_output.type_info.clone(),
                        value,
                    ));
                } else if !exec_output.display.is_empty()
                    && exec_output.display.as_str() != "()"
                    && exec_output.type_info.as_str() != "()"
                {
                    outputs.push(CellOutput::value(
                        exec_output.display.clone(),
                        exec_output.type_info.clone(),
                    ));
                }

                // Add execution timing (shown inline below output)
                let exec_us = exec_output.execution_time.as_micros() as u64;
                if exec_us >= 100 {
                    outputs.push(CellOutput::Timing {
                        compile_time_ms: 0,
                        execution_time_ms: exec_us / 1000,
                    });
                }

                // Create the final output
                let output = match outputs.len() {
                    0 => CellOutput::Empty,
                    1 => outputs.pop().unwrap(),
                    _ => CellOutput::multi(outputs),
                };

                self.current_cell_mut().set_output(output, count);
                Ok(())
            }
            Err(error) => {
                // Convert execution error to cell output
                let output = self.execution_error_to_output(error);
                self.current_cell_mut().set_output(output.clone(), count);

                // Extract error message for return
                if let CellOutput::Error { message, .. } = &output {
                    Err(message.clone())
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Convert an execution error to a CellOutput
    fn execution_error_to_output(&self, error: ExecutionError) -> CellOutput {
        match error {
            ExecutionError::Parse(errors) => {
                // Try to extract line:col from error messages for better display
                let formatted: Vec<String> = errors.iter().map(|e| {
                    // Match patterns like "line 5" or "5:10" or "at line 5, column 10"
                    let s = e.as_str();
                    // Already has line info — pass through
                    if s.starts_with("[line") {
                        return s.to_string();
                    }
                    // Try "line N" pattern
                    if let Some(pos) = s.find("line ") {
                        let after = &s[pos + 5..];
                        if after.starts_with(|c: char| c.is_ascii_digit()) {
                            let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
                            return format!("[line {}] {}", num, s);
                        }
                    }
                    // Try "N:M" pattern at start (e.g. "3:10: unexpected token")
                    let trimmed = s.trim_start();
                    if let Some(colon_pos) = trimmed.find(':') {
                        let before = &trimmed[..colon_pos];
                        if !before.is_empty() && before.chars().all(|c| c.is_ascii_digit()) {
                            let line_num = before;
                            // Check for col after first colon
                            let rest = &trimmed[colon_pos + 1..];
                            if let Some(colon2) = rest.find(':') {
                                let col_part = &rest[..colon2];
                                if !col_part.is_empty() && col_part.chars().all(|c| c.is_ascii_digit()) {
                                    return format!("[line {}:{}] {}", line_num, col_part, rest[colon2 + 1..].trim_start());
                                }
                            }
                            return format!("[line {}] {}", line_num, rest.trim_start());
                        }
                    }
                    s.to_string()
                }).collect();
                let message = formatted.join("\n");
                CellOutput::error_with_suggestions(
                    message,
                    None,
                    Vec::new(),
                )
            }
            ExecutionError::Codegen(msg) => {
                CellOutput::error(format!("Compilation error: {}", msg))
            }
            ExecutionError::Runtime(msg) => {
                CellOutput::error(format!("Runtime error: {}", msg))
            }
            ExecutionError::Type(msg) => {
                CellOutput::error(format!("Type error: {}", msg))
            }
            ExecutionError::InvalidState(msg) => {
                CellOutput::error(format!("Invalid state: {}", msg))
            }
        }
    }

    /// Execute all cells from the beginning
    ///
    /// Resets the execution context and re-executes all cells in order.
    /// This ensures a clean state and is useful when bindings may have
    /// become inconsistent.
    pub fn execute_all(&mut self) -> Result<(), Text> {
        // Reset all state
        self.parser.reset();
        self.pipeline.reset_parser();
        self.pipeline.clear_cache();
        self.execution_context.reset();
        self.execution_count = 0;

        let original_selected = self.selected_cell;
        let cell_count = self.cells.len();

        for i in 0..cell_count {
            self.selected_cell = i;
            if self.current_cell().is_code() {
                self.execute_current()?;
            }
        }

        self.selected_cell = original_selected;
        Ok(())
    }

    /// Execute cells from the current one to the end
    ///
    /// Invalidates the parser cache from the current line and re-executes
    /// all cells from this point forward. Bindings from earlier cells
    /// are preserved.
    pub fn execute_from_current(&mut self) -> Result<(), Text> {
        // Invalidate cache from current line
        let line_number = self.selected_cell + 1;
        self.parser.invalidate_from_line(line_number);
        self.pipeline.invalidate_from(line_number);

        // Clear bindings from cells being re-executed
        let cells_to_reexecute: Vec<CellId> = self.cells[self.selected_cell..]
            .iter()
            .filter(|c| c.is_code())
            .map(|c| c.id)
            .collect();

        for cell_id in cells_to_reexecute {
            self.execution_context.clear_cell_bindings(cell_id);
        }

        let original_selected = self.selected_cell;
        let cell_count = self.cells.len();

        for i in self.selected_cell..cell_count {
            self.selected_cell = i;
            if self.current_cell().is_code() {
                self.execute_current()?;
            }
        }

        self.selected_cell = original_selected;
        Ok(())
    }

    /// Clear all outputs
    ///
    /// Clears all cell outputs and resets the execution context.
    /// The next execution will start fresh.
    pub fn clear_all_outputs(&mut self) {
        self.save_undo_state();
        for cell in &mut self.cells {
            cell.clear_output();
        }
        self.parser.reset();
        self.pipeline.reset_parser();
        self.pipeline.clear_cache();
        self.execution_context.reset();
        self.execution_count = 0;
        self.dirty = true;
    }

    /// Get the current execution context (for completions, hover, etc.)
    pub fn context(&self) -> &ExecutionContext {
        &self.execution_context
    }

    /// Get all available variable names for completion
    pub fn available_bindings(&self) -> impl Iterator<Item = &Text> {
        self.execution_context.binding_names()
    }

    /// Get all available function names for completion
    pub fn available_functions(&self) -> impl Iterator<Item = &Text> {
        self.execution_context.function_names()
    }

    /// Save state for undo
    fn save_undo_state(&mut self) {
        let snapshot = SessionSnapshot {
            cells: self.cells.clone(),
            selected_cell: self.selected_cell,
        };
        self.undo_stack.push(snapshot);
        if self.undo_stack.len() > self.max_undo {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    /// Undo last change
    pub fn undo(&mut self) -> bool {
        if let Some(snapshot) = self.undo_stack.pop() {
            let current = SessionSnapshot {
                cells: self.cells.clone(),
                selected_cell: self.selected_cell,
            };
            self.redo_stack.push(current);
            self.cells = snapshot.cells;
            self.selected_cell = snapshot.selected_cell;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Redo last undone change
    pub fn redo(&mut self) -> bool {
        if let Some(snapshot) = self.redo_stack.pop() {
            let current = SessionSnapshot {
                cells: self.cells.clone(),
                selected_cell: self.selected_cell,
            };
            self.undo_stack.push(current);
            self.cells = snapshot.cells;
            self.selected_cell = snapshot.selected_cell;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Get all code cells
    pub fn code_cells(&self) -> impl Iterator<Item = (usize, &Cell)> {
        self.cells
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_code())
    }

    /// Get cell count
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// Get cell by index
    pub fn get_cell(&self, index: usize) -> Option<&Cell> {
        self.cells.get(index)
    }

    /// Find cell by ID
    pub fn find_cell(&self, id: CellId) -> Option<(usize, &Cell)> {
        self.cells
            .iter()
            .enumerate()
            .find(|(_, c)| c.id == id)
    }
}

impl Default for SessionState {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot for undo/redo
#[derive(Debug, Clone)]
struct SessionSnapshot {
    cells: Vec<Cell>,
    selected_cell: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_navigation() {
        let mut session = SessionState::new();
        session.insert_cell_after(CellKind::Code);
        session.insert_cell_after(CellKind::Code);

        assert_eq!(session.cell_count(), 3);
        assert_eq!(session.selected_cell, 2);

        session.select_prev();
        assert_eq!(session.selected_cell, 1);

        session.select_next();
        assert_eq!(session.selected_cell, 2);
    }

    #[test]
    fn test_undo_redo() {
        let mut session = SessionState::new();
        session.update_current_source("let x = 1");
        session.update_current_source("let x = 2");

        assert!(session.undo());
        assert_eq!(session.current_cell().source.as_str(), "let x = 1");

        assert!(session.redo());
        assert_eq!(session.current_cell().source.as_str(), "let x = 2");
    }
}
