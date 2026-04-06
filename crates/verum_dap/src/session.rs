//! Debug session state management.
//!
//! Tracks breakpoints, thread state, step mode, and source-to-bytecode mappings
//! for a single debug session.

use std::collections::HashMap;
use std::sync::Arc;

use verum_vbc::module::{FunctionId, SourceMapEntry, VbcModule};

use crate::types::Breakpoint;

// ============================================================================
// Step Mode
// ============================================================================

/// How the interpreter should step when resuming execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepMode {
    /// Step one source line, skipping over function calls.
    Over,
    /// Step one source line, entering function calls.
    In,
    /// Run until the current function returns.
    Out,
}

/// Current state of the debug thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    /// Not yet started.
    NotStarted,
    /// Running (executing instructions).
    Running,
    /// Stopped (at breakpoint, step, or pause).
    Stopped,
    /// Execution finished.
    Terminated,
}

// ============================================================================
// Resolved Breakpoint
// ============================================================================

/// A breakpoint resolved to a VBC instruction index.
#[derive(Debug, Clone)]
pub struct ResolvedBreakpoint {
    /// Unique breakpoint ID.
    pub id: i64,
    /// Source file path.
    pub file: String,
    /// Requested source line.
    pub requested_line: i64,
    /// Actual source line (may differ if requested line has no code).
    pub actual_line: i64,
    /// The function containing this breakpoint.
    pub function_id: FunctionId,
    /// The instruction index within the function's bytecode where execution should stop.
    pub instruction_index: u32,
    /// Whether this breakpoint was successfully resolved.
    pub verified: bool,
}

// ============================================================================
// Debug Session
// ============================================================================

/// State for a single debug session.
pub struct DebugSession {
    /// Breakpoints indexed by source file path.
    pub breakpoints: HashMap<String, Vec<ResolvedBreakpoint>>,
    /// Next breakpoint ID to assign.
    next_breakpoint_id: i64,
    /// Current thread state.
    pub thread_state: ThreadState,
    /// Active step mode (set when stepping, cleared on continue).
    pub step_mode: Option<StepMode>,
    /// The call stack depth when a step-over or step-out was initiated.
    /// Used to determine when to stop after a step.
    pub step_start_depth: usize,
    /// The source line when a step was initiated.
    /// Used for step-over to detect when we've moved to a new line.
    pub step_start_line: Option<u32>,
    /// Whether the client requested stop-on-entry.
    pub stop_on_entry: bool,
    /// Whether the session has been initialized.
    pub initialized: bool,
    /// Whether the session has been configured (configurationDone received).
    pub configured: bool,
    /// The program path being debugged.
    pub program_path: Option<String>,
    /// The compiled VBC module (set after launch/compilation).
    pub module: Option<Arc<VbcModule>>,
    /// Next sequence number for server-initiated messages.
    next_seq: i64,
    /// Whether client uses 1-based lines (default true per DAP spec).
    pub lines_start_at1: bool,
    /// Whether client uses 1-based columns (default true per DAP spec).
    pub columns_start_at1: bool,
}

impl DebugSession {
    /// Creates a new empty debug session.
    pub fn new() -> Self {
        Self {
            breakpoints: HashMap::new(),
            next_breakpoint_id: 1,
            thread_state: ThreadState::NotStarted,
            step_mode: None,
            step_start_depth: 0,
            step_start_line: None,
            stop_on_entry: false,
            initialized: false,
            configured: false,
            program_path: None,
            module: None,
            next_seq: 1,
            lines_start_at1: true,
            columns_start_at1: true,
        }
    }

    /// Allocates the next sequence number for a server-initiated message.
    pub fn next_seq(&mut self) -> i64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    /// Sets breakpoints for a source file, resolving them against the VBC source map.
    ///
    /// Returns the list of breakpoints with verified status.
    pub fn set_breakpoints(
        &mut self,
        file_path: &str,
        requested_lines: &[i64],
    ) -> Vec<Breakpoint> {
        // Clear existing breakpoints for this file.
        self.breakpoints.remove(file_path);

        let mut resolved = Vec::new();
        let mut dap_breakpoints = Vec::new();

        for &line in requested_lines {
            let bp_id = self.next_breakpoint_id;
            self.next_breakpoint_id += 1;

            // Try to resolve the breakpoint against the source map.
            let resolution = self.resolve_line_to_instruction(file_path, line);

            let (actual_line, func_id, instr_idx, verified) = match resolution {
                Some((resolved_line, fid, idx)) => (resolved_line, fid, idx, true),
                None => (line, FunctionId(0), 0, false),
            };

            resolved.push(ResolvedBreakpoint {
                id: bp_id,
                file: file_path.to_string(),
                requested_line: line,
                actual_line,
                function_id: func_id,
                instruction_index: instr_idx,
                verified,
            });

            dap_breakpoints.push(Breakpoint {
                id: Some(bp_id),
                verified,
                message: if verified {
                    None
                } else {
                    Some("No code at this line".to_string())
                },
                source: None,
                line: Some(actual_line),
                column: None,
            });
        }

        self.breakpoints.insert(file_path.to_string(), resolved);
        dap_breakpoints
    }

    /// Checks if the given function/instruction index hits a breakpoint.
    ///
    /// Returns the breakpoint ID if hit.
    pub fn check_breakpoint(&self, function_id: FunctionId, instruction_index: u32) -> Option<i64> {
        for bps in self.breakpoints.values() {
            for bp in bps {
                if bp.verified
                    && bp.function_id == function_id
                    && bp.instruction_index == instruction_index
                {
                    return Some(bp.id);
                }
            }
        }
        None
    }

    /// Resolves a source line to a VBC instruction index using the source map.
    ///
    /// Returns `(actual_line, function_id, instruction_index)` or None if no mapping exists.
    fn resolve_line_to_instruction(
        &self,
        file_path: &str,
        line: i64,
    ) -> Option<(i64, FunctionId, u32)> {
        let module = self.module.as_ref()?;
        let source_map = module.source_map.as_ref()?;

        // Find the file index in the source map.
        let file_idx = source_map.files.iter().position(|sid| {
            module
                .get_string(*sid)
                .is_some_and(|name| file_path.ends_with(name) || name == file_path)
        })? as u16;

        // Find the closest source map entry at or after the requested line.
        let target_line = line as u32;
        let mut best_entry: Option<&SourceMapEntry> = None;

        for entry in &source_map.entries {
            if entry.file_idx != file_idx {
                continue;
            }
            if entry.line == target_line {
                // Exact match — prefer the first one.
                if best_entry.is_none_or(|b| entry.bytecode_offset < b.bytecode_offset) {
                    best_entry = Some(entry);
                }
            } else if entry.line > target_line {
                // No exact match; snap forward to the next available line.
                if best_entry.is_none_or(|b| entry.line < b.line) {
                    best_entry = Some(entry);
                }
            }
        }

        let entry = best_entry?;

        // Find which function contains this bytecode offset.
        let func = module.functions.iter().find(|f| {
            entry.bytecode_offset >= f.bytecode_offset
                && entry.bytecode_offset < f.bytecode_offset + f.bytecode_length
        })?;

        // The instruction index is relative to the function's start.
        let instr_idx = entry.bytecode_offset - func.bytecode_offset;

        Some((entry.line as i64, func.id, instr_idx))
    }

    /// Looks up the source location for a given bytecode offset.
    ///
    /// Returns `(file_path, line, column)` if found.
    pub fn lookup_source_location(
        &self,
        bytecode_offset: u32,
    ) -> Option<(String, i64, i64)> {
        let module = self.module.as_ref()?;
        let source_map = module.source_map.as_ref()?;

        // Find the entry with the largest offset <= bytecode_offset.
        let mut best: Option<&SourceMapEntry> = None;
        for entry in &source_map.entries {
            if entry.bytecode_offset <= bytecode_offset {
                if best.is_none_or(|b| entry.bytecode_offset > b.bytecode_offset) {
                    best = Some(entry);
                }
            }
        }

        let entry = best?;
        let file_name = module.get_string(source_map.files[entry.file_idx as usize])?;

        Some((
            file_name.to_string(),
            entry.line as i64,
            entry.column as i64,
        ))
    }
}

impl Default for DebugSession {
    fn default() -> Self {
        Self::new()
    }
}
