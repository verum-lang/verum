//! Debug adapter logic.
//!
//! Maps DAP requests to VBC interpreter operations: compiling the program,
//! setting breakpoints, stepping, reading stack traces and variables.

use std::sync::Arc;

use verum_vbc::interpreter::{
    dispatch_loop_table, InterpreterState,
};
use verum_vbc::module::VbcModule;

use crate::session::{DebugSession, StepMode, ThreadState};
use crate::types::*;
use crate::variables;

/// The main thread ID. The VBC interpreter is single-threaded, so we use a
/// constant thread ID of 1.
const MAIN_THREAD_ID: i64 = 1;

/// Core debug adapter that dispatches DAP requests.
pub struct DebugAdapter {
    /// The debug session state.
    pub session: DebugSession,
    /// The interpreter state (created on launch).
    pub interpreter: Option<InterpreterState>,
}

impl DebugAdapter {
    /// Creates a new debug adapter.
    pub fn new() -> Self {
        Self {
            session: DebugSession::new(),
            interpreter: None,
        }
    }

    /// Dispatches a DAP request and returns a response.
    pub fn handle_request(&mut self, request: &Request) -> (Response, Vec<Event>) {
        let mut events = Vec::new();

        let (success, message, body) = match request.command.as_str() {
            "initialize" => self.handle_initialize(request),
            "launch" => {
                let result = self.handle_launch(request);
                // After launch, send initialized event.
                if result.0 {
                    events.push(self.make_event("initialized", Some(serde_json::json!({}))));
                    if self.session.stop_on_entry {
                        self.session.thread_state = ThreadState::Stopped;
                        events.push(self.make_event(
                            "stopped",
                            Some(serde_json::json!({
                                "reason": "entry",
                                "threadId": MAIN_THREAD_ID,
                                "allThreadsStopped": true,
                            })),
                        ));
                    }
                }
                result
            }
            "configurationDone" => self.handle_configuration_done(),
            "setBreakpoints" => self.handle_set_breakpoints(request),
            "threads" => self.handle_threads(),
            "continue" => {
                let result = self.handle_continue(request);
                if self.session.thread_state == ThreadState::Stopped {
                    events.push(self.make_event(
                        "stopped",
                        Some(serde_json::json!({
                            "reason": "breakpoint",
                            "threadId": MAIN_THREAD_ID,
                            "allThreadsStopped": true,
                        })),
                    ));
                } else if self.session.thread_state == ThreadState::Terminated {
                    events.push(self.make_event("terminated", Some(serde_json::json!({}))));
                }
                result
            }
            "next" => {
                let result = self.handle_step(StepMode::Over);
                self.emit_step_events(&mut events);
                result
            }
            "stepIn" => {
                let result = self.handle_step(StepMode::In);
                self.emit_step_events(&mut events);
                result
            }
            "stepOut" => {
                let result = self.handle_step(StepMode::Out);
                self.emit_step_events(&mut events);
                result
            }
            "stackTrace" => self.handle_stack_trace(request),
            "scopes" => self.handle_scopes(request),
            "variables" => self.handle_variables(request),
            "disconnect" => self.handle_disconnect(request),
            other => {
                tracing::warn!("Unhandled DAP command: {}", other);
                (
                    false,
                    Some(format!("Unsupported command: {}", other)),
                    None,
                )
            }
        };

        let response = Response {
            seq: self.session.next_seq(),
            request_seq: request.seq,
            success,
            command: request.command.clone(),
            message,
            body,
        };

        (response, events)
    }

    /// Emit stopped/terminated events after a step operation.
    fn emit_step_events(&mut self, events: &mut Vec<Event>) {
        if self.session.thread_state == ThreadState::Stopped {
            events.push(self.make_event(
                "stopped",
                Some(serde_json::json!({
                    "reason": "step",
                    "threadId": MAIN_THREAD_ID,
                    "allThreadsStopped": true,
                })),
            ));
        } else if self.session.thread_state == ThreadState::Terminated {
            events.push(self.make_event("terminated", Some(serde_json::json!({}))));
        }
    }

    // ========================================================================
    // Request Handlers
    // ========================================================================

    fn handle_initialize(
        &mut self,
        request: &Request,
    ) -> (bool, Option<String>, Option<serde_json::Value>) {
        if let Some(args) = &request.arguments
            && let Ok(init_args) = serde_json::from_value::<InitializeArguments>(args.clone()) {
                self.session.lines_start_at1 = init_args.lines_start_at1;
                self.session.columns_start_at1 = init_args.columns_start_at1;
            }

        self.session.initialized = true;

        let capabilities = Capabilities {
            supports_configuration_done_request: Some(true),
            supports_set_variable: Some(false),
            supports_terminate_request: Some(true),
            supports_stepping_granularity: Some(false),
        };

        (
            true,
            None,
            Some(serde_json::to_value(capabilities).unwrap_or_default()),
        )
    }

    fn handle_launch(
        &mut self,
        request: &Request,
    ) -> (bool, Option<String>, Option<serde_json::Value>) {
        let args = match &request.arguments {
            Some(v) => match serde_json::from_value::<LaunchArguments>(v.clone()) {
                Ok(a) => a,
                Err(e) => {
                    return (
                        false,
                        Some(format!("Invalid launch arguments: {}", e)),
                        None,
                    );
                }
            },
            None => {
                return (false, Some("Missing launch arguments".to_string()), None);
            }
        };

        self.session.stop_on_entry = args.stop_on_entry;
        self.session.program_path = Some(args.program.clone());

        // Compile the program to VBC.
        let module = match compile_program(&args.program) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                return (
                    false,
                    Some(format!("Compilation failed: {}", e)),
                    None,
                );
            }
        };

        // Create the interpreter state.
        let state = InterpreterState::new(module.clone());

        self.session.module = Some(module);
        self.interpreter = Some(state);
        self.session.thread_state = ThreadState::Stopped;

        (true, None, None)
    }

    fn handle_configuration_done(
        &mut self,
    ) -> (bool, Option<String>, Option<serde_json::Value>) {
        self.session.configured = true;
        (true, None, None)
    }

    fn handle_set_breakpoints(
        &mut self,
        request: &Request,
    ) -> (bool, Option<String>, Option<serde_json::Value>) {
        let args = match &request.arguments {
            Some(v) => match serde_json::from_value::<SetBreakpointsArguments>(v.clone()) {
                Ok(a) => a,
                Err(e) => {
                    return (
                        false,
                        Some(format!("Invalid setBreakpoints arguments: {}", e)),
                        None,
                    );
                }
            },
            None => {
                return (false, Some("Missing arguments".to_string()), None);
            }
        };

        let file_path = args
            .source
            .path
            .as_deref()
            .unwrap_or("");

        let lines: Vec<i64> = args.breakpoints.iter().map(|bp| bp.line).collect();

        let breakpoints = self.session.set_breakpoints(file_path, &lines);

        let body = SetBreakpointsResponseBody { breakpoints };

        (
            true,
            None,
            Some(serde_json::to_value(body).unwrap_or_default()),
        )
    }

    fn handle_threads(&self) -> (bool, Option<String>, Option<serde_json::Value>) {
        // VBC interpreter is single-threaded.
        let body = ThreadsResponseBody {
            threads: vec![Thread {
                id: MAIN_THREAD_ID,
                name: "main".to_string(),
            }],
        };

        (
            true,
            None,
            Some(serde_json::to_value(body).unwrap_or_default()),
        )
    }

    fn handle_continue(
        &mut self,
        _request: &Request,
    ) -> (bool, Option<String>, Option<serde_json::Value>) {
        self.session.step_mode = None;
        self.run_until_stop();

        let body = ContinueResponseBody {
            all_threads_continued: true,
        };

        (
            true,
            None,
            Some(serde_json::to_value(body).unwrap_or_default()),
        )
    }

    fn handle_step(
        &mut self,
        mode: StepMode,
    ) -> (bool, Option<String>, Option<serde_json::Value>) {
        if let Some(state) = &self.interpreter {
            self.session.step_start_depth = state.call_stack.depth();
            // Record current source line for step-over detection.
            let current_offset = self.current_bytecode_offset();
            self.session.step_start_line = current_offset
                .and_then(|off| self.session.lookup_source_location(off))
                .map(|(_, line, _)| line as u32);
        }
        self.session.step_mode = Some(mode);
        self.run_until_stop();

        (true, None, None)
    }

    fn handle_stack_trace(
        &self,
        request: &Request,
    ) -> (bool, Option<String>, Option<serde_json::Value>) {
        let _args = request
            .arguments
            .as_ref()
            .and_then(|v| serde_json::from_value::<StackTraceArguments>(v.clone()).ok());

        let empty_response = || {
            (
                true,
                None,
                Some(serde_json::to_value(StackTraceResponseBody {
                    stack_frames: vec![],
                    total_frames: Some(0),
                }).unwrap_or_default()),
            )
        };

        let state = match &self.interpreter {
            Some(s) => s,
            None => return empty_response(),
        };

        let module = match &self.session.module {
            Some(m) => m,
            None => return empty_response(),
        };

        let mut frames = Vec::new();

        // Use iter_rev() to get frames from top (most recent) to bottom.
        for (idx, frame) in state.call_stack.iter_rev().enumerate() {
            let func_name = module
                .functions
                .iter()
                .find(|f| f.id == frame.function)
                .and_then(|f| module.get_string(f.name))
                .unwrap_or("<unknown>")
                .to_string();

            let bytecode_offset = frame.pc;
            let source_loc = self.session.lookup_source_location(bytecode_offset);

            let (source, line, column) = match source_loc {
                Some((file, l, c)) => (
                    Some(Source {
                        name: Some(
                            std::path::Path::new(&file)
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or(&file)
                                .to_string(),
                        ),
                        path: Some(file),
                        source_reference: None,
                    }),
                    Some(l),
                    Some(c),
                ),
                None => (None, None, None),
            };

            frames.push(StackFrame {
                id: idx as i64,
                name: func_name,
                source,
                line,
                column,
            });
        }

        let total = frames.len() as i64;

        let body = StackTraceResponseBody {
            stack_frames: frames,
            total_frames: Some(total),
        };

        (
            true,
            None,
            Some(serde_json::to_value(body).unwrap_or_default()),
        )
    }

    fn handle_scopes(
        &self,
        request: &Request,
    ) -> (bool, Option<String>, Option<serde_json::Value>) {
        let args = match &request.arguments {
            Some(v) => match serde_json::from_value::<ScopesArguments>(v.clone()) {
                Ok(a) => a,
                Err(_) => {
                    return (false, Some("Invalid scopes arguments".to_string()), None);
                }
            },
            None => {
                return (false, Some("Missing arguments".to_string()), None);
            }
        };

        let frame_id = args.frame_id;

        let body = ScopesResponseBody {
            scopes: vec![
                Scope {
                    name: "Locals".to_string(),
                    variables_reference: variables::encode_variables_reference(frame_id, 0),
                    expensive: false,
                },
                Scope {
                    name: "Arguments".to_string(),
                    variables_reference: variables::encode_variables_reference(frame_id, 1),
                    expensive: false,
                },
            ],
        };

        (
            true,
            None,
            Some(serde_json::to_value(body).unwrap_or_default()),
        )
    }

    fn handle_variables(
        &self,
        request: &Request,
    ) -> (bool, Option<String>, Option<serde_json::Value>) {
        let args = match &request.arguments {
            Some(v) => match serde_json::from_value::<VariablesArguments>(v.clone()) {
                Ok(a) => a,
                Err(_) => {
                    return (false, Some("Invalid variables arguments".to_string()), None);
                }
            },
            None => {
                return (false, Some("Missing arguments".to_string()), None);
            }
        };

        let (frame_idx, scope_kind) =
            variables::decode_variables_reference(args.variables_reference);

        let state = match &self.interpreter {
            Some(s) => s,
            None => {
                return (
                    true,
                    None,
                    Some(
                        serde_json::to_value(VariablesResponseBody {
                            variables: vec![],
                        })
                        .unwrap_or_default(),
                    ),
                );
            }
        };

        let module = match &self.session.module {
            Some(m) => m,
            None => {
                return (
                    true,
                    None,
                    Some(
                        serde_json::to_value(VariablesResponseBody {
                            variables: vec![],
                        })
                        .unwrap_or_default(),
                    ),
                );
            }
        };

        // Get the call frame. Frames are indexed from the top of the stack (0 = most recent).
        let frame = state.call_stack.iter_rev().nth(frame_idx as usize);

        let vars = match frame {
            Some(f) => variables::read_frame_variables(state, module, f, scope_kind),
            None => vec![],
        };

        let body = VariablesResponseBody { variables: vars };

        (
            true,
            None,
            Some(serde_json::to_value(body).unwrap_or_default()),
        )
    }

    fn handle_disconnect(
        &mut self,
        _request: &Request,
    ) -> (bool, Option<String>, Option<serde_json::Value>) {
        self.session.thread_state = ThreadState::Terminated;
        self.interpreter = None;
        (true, None, None)
    }

    // ========================================================================
    // Execution Control
    // ========================================================================

    /// Runs the interpreter until a breakpoint is hit, a step completes, or
    /// the program terminates.
    fn run_until_stop(&mut self) {
        let state = match self.interpreter.as_mut() {
            Some(s) => s,
            None => {
                self.session.thread_state = ThreadState::Terminated;
                return;
            }
        };

        self.session.thread_state = ThreadState::Running;

        // Execute the program using the table-based dispatch loop.
        //
        // In a production implementation this would use the interpreter's debug hook
        // callback mechanism for per-instruction breakpoint and step checking.
        //
        // For the initial implementation, we run the interpreter to completion.
        // The breakpoint and stepping state is tracked and ready for integration
        // when the per-instruction hook API is exposed.
        match dispatch_loop_table(state) {
            Ok(_) => {
                self.session.thread_state = ThreadState::Terminated;
            }
            Err(e) => {
                tracing::error!("Interpreter error: {:?}", e);
                self.session.thread_state = ThreadState::Terminated;
            }
        }
    }

    /// Returns the current bytecode offset (PC) of the topmost call frame.
    fn current_bytecode_offset(&self) -> Option<u32> {
        let state = self.interpreter.as_ref()?;
        state.call_stack.current().map(|f| f.pc)
    }

    // ========================================================================
    // Event Helpers
    // ========================================================================

    /// Creates a DAP event.
    fn make_event(&mut self, event_type: &str, body: Option<serde_json::Value>) -> Event {
        Event {
            seq: self.session.next_seq(),
            event: event_type.to_string(),
            body,
        }
    }
}

impl Default for DebugAdapter {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Compilation Helper
// ============================================================================

/// Compiles a `.vr` file to a VBC module.
///
/// Uses the verum_compiler pipeline (parse, type check, VBC codegen).
fn compile_program(path: &str) -> Result<VbcModule, String> {
    use std::path::Path;

    let source_path = Path::new(path);
    if !source_path.exists() {
        return Err(format!("Source file not found: {}", path));
    }

    let source = std::fs::read_to_string(source_path)
        .map_err(|e| format!("Failed to read source: {}", e))?;

    verum_compiler::api::compile_to_vbc(&source)
        .map_err(|e| format!("Compilation error: {}", e))
}
