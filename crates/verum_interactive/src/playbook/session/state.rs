//! Playbook session state management
//!
//! This module manages the execution state for a playbook session, including:
//! - Cell management (create, delete, reorder)
//! - VBC execution pipeline integration
//! - Cross-cell state preservation via ExecutionContext
//! - Undo/redo history

use verum_ast::FileId;
use verum_common::Text;

use super::cell::{Cell, CellId, CellKind, CellOutput, TensorStats};
use crate::IncrementalScriptParser;
use crate::execution::{ExecutionContext, ExecutionError, ExecutionPipeline};

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
    /// The ONE vocabulary executor (T0858 slice 5): cells run as a
    /// GROWING MODULE through the script engine — the same
    /// hook-installed compiler `verum run` and `core.script` use.
    /// State lives in the question, not in a hidden kernel.
    pub engine: verum_vbc::interpreter::ScriptEngine,
    /// Top-level binding previews from the LAST notebook run:
    /// (name, debug-rendered value). Feeds the Vars lens.
    pub var_previews: Vec<(String, String)>,
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
            // Permissive caps on purpose: the playground is a LOCAL
            // interactive tool, the moral peer of `verum run` in the
            // user's own terminal — sandboxing belongs to script
            // frontmatter grants, not to the notebook.
            engine: verum_vbc::interpreter::ScriptEngine::new()
                .allow_file_io()
                .allow_network()
                .allow_process(),
            var_previews: Vec::new(),
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
        if at_line == 0 || at_line >= lines.len() {
            return;
        }
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

    /// Output-boundary sentinel printed between cells of the grown
    /// module: `\u{1}` cannot come from user `print` text in practice,
    /// and the index after it pins each segment to its cell.
    const CELL_MARK: &'static str = "\u{1}VRNB\u{1}";
    /// Sentinel opening the machine-read VARS tail.
    const VARS_MARK: &'static str = "\u{1}VRVARS\u{1}";

    /// The growing module up to and including cell index `upto`
    /// (code cells only), instrumented with per-cell output marks and
    /// a VARS tail that debug-prints every top-level binding. Returns
    /// (source, code_cell_indices).
    fn chain_source_upto(&self, upto: usize) -> (String, Vec<usize>) {
        let mut source = String::new();
        let mut indices = Vec::new();
        let code_cells: Vec<usize> = self
            .cells
            .iter()
            .enumerate()
            .take(upto + 1)
            .filter(|(_, c)| c.is_code())
            .map(|(i, _)| i)
            .collect();
        let last_pos = code_cells.len().saturating_sub(1);
        for (pos, &i) in code_cells.iter().enumerate() {
            let raw_src = self.cells[i].source.as_str();
            // Notebook idiom: a cell may end in a bare tail
            // EXPRESSION ("answer + 1"). Inside the grown module that
            // is only legal as the script's final tail — normalize:
            // a non-target tail gets `;`; the TARGET tail is bound to
            // `__vrnb_result` so the VARS channel carries the cell's
            // value while the output marks stay the true script tail.
            match split_tail_expression(raw_src) {
                Some((body, tail)) if pos == last_pos => {
                    source.push_str(&body);
                    source.push_str(&format!(
                        "let __vrnb_result = ({tail});\n"
                    ));
                }
                Some((body, tail)) => {
                    source.push_str(&body);
                    source.push_str(&format!("({tail});\n"));
                }
                None => {
                    source.push_str(&normalize_cell_source(raw_src));
                    source.push('\n');
                }
            }
            source.push_str(&format!(
                "print(\"{}{}\");\n",
                Self::CELL_MARK,
                indices.len()
            ));
            indices.push(i);
        }
        // VARS tail: every top-level let binding, debug-rendered from
        // the SAME run (structural Debug never panics — measured on
        // underived records and closures).
        let names = top_level_let_names(&source);
        if !names.is_empty() {
            source.push_str(&format!("print(\"{}\");\n", Self::VARS_MARK));
            for name in names {
                source.push_str(&format!(
                    "print(f\"{name}\x01={{{name}:?}}\");\n"
                ));
            }
        }
        (source, indices)
    }

    /// Run the notebook-as-module up to cell `upto` through the script
    /// engine and distribute outputs: each executed cell gets its
    /// stdout segment; the target cell also gets the run's value and
    /// price; the VARS tail refreshes the Vars lens.
    /// The grown-module question for the SYNC path (tests, replay).
    /// The interactive path prepares the same question, runs it on a
    /// worker engine, and applies the same outcome — one carrier.
    fn run_notebook_upto(&mut self, upto: usize) -> Result<(), Text> {
        let Some((source, indices)) = self.prepare_notebook_run(upto) else {
            return Ok(());
        };
        let started = std::time::Instant::now();
        let outcome = run_grown_module(&mut self.engine, &source);
        let elapsed = started.elapsed();
        self.apply_notebook_outcome(&indices, outcome, elapsed)
    }

    /// The last run's top-level bindings (name, rendered value),
    /// machinery names filtered — the Vars lens / completion feed.
    pub fn var_previews_iter(&self) -> impl Iterator<Item = &(String, String)> {
        self.var_previews
            .iter()
            .filter(|(n, _)| n != "__vrnb_result")
    }

    /// Build the grown module up to `upto`; None when there is
    /// nothing to run.
    pub fn prepare_notebook_run(&self, upto: usize) -> Option<(String, Vec<usize>)> {
        let (source, indices) = self.chain_source_upto(upto);
        if indices.is_empty() {
            return None;
        }
        // Diagnostic tap: VERUM_NB_DEBUG=1 prints the exact grown
        // module the engine will compile — the first question when a
        // notebook run fails is "what source did the law build".
        if std::env::var("VERUM_NB_DEBUG").is_ok() {
            eprintln!("[nb-module]\n{source}[/nb-module]");
        }
        Some((source, indices))
    }

    /// Distribute one run's outcome over the cells (stdout segments,
    /// target value, price, VARS channel).
    pub fn apply_notebook_outcome(
        &mut self,
        indices: &[usize],
        outcome: verum_vbc::interpreter::ScriptOutcome,
        elapsed: std::time::Duration,
    ) -> Result<(), Text> {
        self.execution_count += 1;
        let count = self.execution_count;

        // Split stdout into per-cell segments by the cell marks.
        let mut segments: Vec<String> = vec![String::new(); indices.len()];
        let mut vars_tail = String::new();
        let mut parts = outcome.stdout.split(Self::CELL_MARK);
        if let Some(first) = parts.next() {
            if let Some(seg) = segments.first_mut() {
                seg.push_str(first);
            }
        }
        for part in parts {
            // Each part is "<idx>\n<next cell's stdout...>".
            let Some((idx_str, rest)) = part.split_once('\n') else {
                continue;
            };
            let Ok(seg_idx) = idx_str.trim().parse::<usize>() else {
                continue;
            };
            let rest = match rest.split_once(Self::VARS_MARK) {
                Some((before, tail)) => {
                    vars_tail = tail.to_string();
                    before.to_string()
                }
                None => rest.to_string(),
            };
            if let Some(seg) = segments.get_mut(seg_idx + 1) {
                seg.push_str(&rest);
            } else if seg_idx + 1 == segments.len() {
                // Output after the LAST mark belongs to the vars tail
                // handling above; nothing to assign.
            }
        }

        // Vars lens: parse the tail lines "name\u{1}=repr".
        // (__vrnb_result stays in the list until the value is read
        // below, then the lens filter drops it.)
        self.var_previews = vars_tail
            .lines()
            .filter_map(|l| {
                l.split_once('\u{1}')
                    .and_then(|(n, r)| r.strip_prefix('=').map(|r| (n.to_string(), r.to_string())))
            })
            .collect();

        // Distribute outputs.
        let target = *indices.last().expect("nonempty indices");
        for (seg_pos, &cell_idx) in indices.iter().enumerate() {
            let seg = segments[seg_pos].trim_end_matches('\n');
            let is_target = cell_idx == target;
            let mut outputs: Vec<CellOutput> = Vec::new();
            if !seg.is_empty() {
                outputs.push(CellOutput::stream(Text::from(seg)));
            }
            if is_target {
                if let Some(err) = &outcome.error {
                    outputs.push(CellOutput::error(Text::from(format!("{err:?}"))));
                } else if let Some((_, repr)) = self
                    .var_previews
                    .iter()
                    .find(|(n, _)| n == "__vrnb_result")
                {
                    outputs.push(CellOutput::value(
                        Text::from(repr.as_str()),
                        Text::from(""),
                    ));
                } else if let Some((repr, ty)) = owned_value_repr(&outcome.value) {
                    outputs.push(CellOutput::value(Text::from(repr), Text::from(ty)));
                }
                let ms = elapsed.as_millis() as u64;
                if ms >= 1 {
                    outputs.push(CellOutput::Timing {
                        compile_time_ms: 0,
                        execution_time_ms: ms,
                    });
                }
            }
            let output = match outputs.len() {
                0 => CellOutput::Empty,
                1 => outputs.pop().expect("len checked"),
                _ => CellOutput::multi(outputs),
            };
            self.cells[cell_idx].set_output(output, count);
        }

        match &outcome.error {
            Some(err) => Err(Text::from(format!("{err:?}"))),
            None => Ok(()),
        }
    }

    /// Execute the current cell — by the state law (T0858): the cell
    /// is a question to the GROWN MODULE of cells 1..=k, recomputed
    /// from source through the one vocabulary executor. No hidden
    /// kernel state survives between runs.
    pub fn execute_current(&mut self) -> Result<(), Text> {
        if !self.current_cell().is_code() {
            return Ok(());
        }
        self.run_notebook_upto(self.selected_cell)
    }

    /// Convert an execution error to a CellOutput
    fn execution_error_to_output(&self, error: ExecutionError) -> CellOutput {
        match error {
            ExecutionError::Parse(errors) => {
                // Try to extract line:col from error messages for better display
                let formatted: Vec<String> = errors
                    .iter()
                    .map(|e| {
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
                                let num: String =
                                    after.chars().take_while(|c| c.is_ascii_digit()).collect();
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
                                    if !col_part.is_empty()
                                        && col_part.chars().all(|c| c.is_ascii_digit())
                                    {
                                        return format!(
                                            "[line {}:{}] {}",
                                            line_num,
                                            col_part,
                                            rest[colon2 + 1..].trim_start()
                                        );
                                    }
                                }
                                return format!("[line {}] {}", line_num, rest.trim_start());
                            }
                        }
                        s.to_string()
                    })
                    .collect();
                let message = formatted.join("\n");
                CellOutput::error_with_suggestions(message, None, Vec::new())
            }
            ExecutionError::Codegen(msg) => {
                CellOutput::error(format!("Compilation error: {}", msg))
            }
            ExecutionError::Runtime(msg) => CellOutput::error(format!("Runtime error: {}", msg)),
            ExecutionError::Type(msg) => CellOutput::error(format!("Type error: {}", msg)),
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
        let Some(last_code) = self
            .cells
            .iter()
            .rposition(|c| c.is_code())
        else {
            return Ok(());
        };
        self.run_notebook_upto(last_code)
    }

    /// Execute cells from the current one to the end
    ///
    /// Invalidates the parser cache from the current line and re-executes
    /// all cells from this point forward. Bindings from earlier cells
    /// are preserved.
    /// In the recompute-from-source model "run from here" and "run
    /// all" ask the same question — the whole module. Kept as a
    /// distinct entry for the keybinding.
    pub fn execute_from_current(&mut self) -> Result<(), Text> {
        self.execute_all()
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
        self.cells.iter().enumerate().filter(|(_, c)| c.is_code())
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
        self.cells.iter().enumerate().find(|(_, c)| c.id == id)
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

/// Top-level `let` binding names of a script source, in declaration
/// order (last occurrence wins for shadowing), via the same
/// script-mode parse the compiler uses. Parse failures yield an empty
/// list — the run itself will surface the real diagnostic.
fn top_level_let_names(source: &str) -> Vec<String> {
    use verum_ast::decl::{FunctionBody, ItemKind};
    let parser = verum_fast_parser::FastParser::new();
    let Ok(module) =
        parser.parse_module_script_str(source, verum_ast::FileId::new(0))
    else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    for item in module.items.iter() {
        let ItemKind::Function(f) = &item.kind else { continue };
        if f.name.name.as_str() != "__verum_script_main" {
            continue;
        }
        let verum_common::Maybe::Some(FunctionBody::Block(block)) = &f.body else {
            continue;
        };
        for stmt in block.stmts.iter() {
            if let verum_ast::stmt::StmtKind::Let { pattern, .. } = &stmt.kind {
                collect_pattern_names(pattern, &mut names);
            }
        }
    }
    // Shadowing: keep the LAST occurrence of a repeated name.
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for name in names.into_iter().rev() {
        if seen.insert(name.clone()) {
            out.push(name);
        }
    }
    out.reverse();
    out
}

/// Compile and run a grown module on `engine`: script-mode sources
/// compile to a `__verum_script_main` wrapper; a module-shaped
/// notebook (cells defining their own `fn main`) has no wrapper —
/// try both. Free function so the interactive worker thread can own
/// a throwaway engine.
pub fn run_grown_module(
    engine: &mut verum_vbc::interpreter::ScriptEngine,
    source: &str,
) -> verum_vbc::interpreter::ScriptOutcome {
    match engine.compile(source) {
        Ok(module) => {
            let out = engine.run_to_outcome(module.clone(), "__verum_script_main", &[]);
            match &out.error {
                Some(verum_vbc::interpreter::ScriptError::EntryNotFound(_)) => {
                    engine.run_to_outcome(module, "main", &[])
                }
                _ => out,
            }
        }
        Err(error) => verum_vbc::interpreter::ScriptOutcome {
            value: verum_vbc::interpreter::ScriptValueOwned::Nil,
            error: Some(error),
            stdout: String::new(),
        },
    }
}

/// REPL idiom normalization: a lone `let x = 41` (no trailing `;`)
/// is how people type in a notebook, and the tolerant script parser
/// accepts it at EOF — but the COMPILER's parse does not once more
/// statements follow in the grown module. Purely textual: when the
/// cell has no tail expression and its last meaningful line neither
/// ends a statement nor a block nor trails a comment, append `;`.
fn normalize_cell_source(cell_src: &str) -> String {
    let trimmed = cell_src.trim_end();
    if trimmed.is_empty() || trimmed.ends_with(';') || trimmed.ends_with('}') {
        return cell_src.to_string();
    }
    if trimmed.lines().last().is_some_and(|l| l.contains("//")) {
        // A trailing comment would swallow the `;` — leave the source
        // alone and let the compiler state the real requirement.
        return cell_src.to_string();
    }
    format!("{trimmed};")
}

/// If the cell source ends in a bare tail EXPRESSION, split it:
/// (body-without-tail, tail-text). Uses the same script-mode parse
/// the compiler runs; parse failure or no tail → None (the source
/// goes in verbatim and the real diagnostic surfaces on the run).
fn split_tail_expression(cell_src: &str) -> Option<(String, String)> {
    use verum_ast::decl::{FunctionBody, ItemKind};
    let parser = verum_fast_parser::FastParser::new();
    let module = parser
        .parse_module_script_str(cell_src, verum_ast::FileId::new(0))
        .ok()?;
    for item in module.items.iter() {
        let ItemKind::Function(f) = &item.kind else { continue };
        if f.name.name.as_str() != "__verum_script_main" {
            continue;
        }
        let verum_common::Maybe::Some(FunctionBody::Block(block)) = &f.body else {
            continue;
        };
        let verum_common::Maybe::Some(tail) = &block.expr else {
            continue;
        };
        let start = tail.span.start as usize;
        let end = tail.span.end as usize;
        if start < end && end <= cell_src.len() {
            return Some((
                cell_src[..start].to_string(),
                cell_src[start..end].trim_end().to_string(),
            ));
        }
    }
    None
}

fn collect_pattern_names(pattern: &verum_ast::pattern::Pattern, out: &mut Vec<String>) {
    use verum_ast::pattern::PatternKind;
    match &pattern.kind {
        PatternKind::Ident { name, .. } => out.push(name.as_str().to_string()),
        PatternKind::Tuple(parts) => {
            for p in parts.iter() {
                collect_pattern_names(p, out);
            }
        }
        _ => {}
    }
}

/// Human rendering of the run's final value: (repr, type label).
/// `Nil` renders as nothing — a statement-tail run has no value to
/// show.
fn owned_value_repr(
    value: &verum_vbc::interpreter::ScriptValueOwned,
) -> Option<(String, String)> {
    use verum_vbc::interpreter::ScriptValueOwned as V;
    fn render(v: &V) -> String {
        match v {
            V::Nil => "()".to_string(),
            V::Bool(b) => b.to_string(),
            V::Int(i) => i.to_string(),
            V::Float(f) => format!("{f}"),
            V::Text(t) => t.clone(),
            V::List(xs) => {
                let inner: Vec<String> = xs.iter().map(render).collect();
                format!("[{}]", inner.join(", "))
            }
            V::Map(pairs) => {
                let inner: Vec<String> = pairs
                    .iter()
                    .map(|(k, val)| format!("{}: {}", render(k), render(val)))
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
            V::Other => "<value>".to_string(),
        }
    }
    match value {
        V::Nil => None,
        V::Bool(_) => Some((render(value), "Bool".to_string())),
        V::Int(_) => Some((render(value), "Int".to_string())),
        V::Float(_) => Some((render(value), "Float".to_string())),
        V::Text(_) => Some((render(value), "Text".to_string())),
        V::List(_) => Some((render(value), "List".to_string())),
        V::Map(_) => Some((render(value), "Map".to_string())),
        V::Other => Some(("<value>".to_string(), String::new())),
    }
}
