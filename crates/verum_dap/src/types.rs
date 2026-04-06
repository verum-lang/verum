//! DAP protocol types.
//!
//! Implements the core message types for the Debug Adapter Protocol:
//! requests, responses, events, and supporting data structures.
//!
//! Reference: <https://microsoft.github.io/debug-adapter-protocol/specification>

use serde::{Deserialize, Serialize};

// ============================================================================
// Protocol Messages
// ============================================================================

/// A DAP protocol message (request, response, or event).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProtocolMessage {
    /// A client request.
    #[serde(rename = "request")]
    Request(Request),
    /// A server response.
    #[serde(rename = "response")]
    Response(Response),
    /// A server event.
    #[serde(rename = "event")]
    Event(Event),
}

/// A DAP request from the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// Sequence number (monotonically increasing).
    pub seq: i64,
    /// The command to execute.
    pub command: String,
    /// Command-specific arguments (JSON object).
    #[serde(default)]
    pub arguments: Option<serde_json::Value>,
}

/// A DAP response from the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    /// Sequence number for this response.
    pub seq: i64,
    /// The request sequence number this response is for.
    pub request_seq: i64,
    /// Whether the request was successful.
    pub success: bool,
    /// The command this response is for.
    pub command: String,
    /// Error message if success is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Response body (command-specific).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

/// A DAP event from the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Sequence number for this event.
    pub seq: i64,
    /// The event type.
    pub event: String,
    /// Event body (event-specific).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

// ============================================================================
// Capabilities (Initialize Response)
// ============================================================================

/// Server capabilities reported during initialization.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// The debug adapter supports the `configurationDone` request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_configuration_done_request: Option<bool>,
    /// The debug adapter supports setting breakpoints.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_set_variable: Option<bool>,
    /// The debug adapter supports the `terminate` request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_terminate_request: Option<bool>,
    /// The debug adapter supports stepping granularity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_stepping_granularity: Option<bool>,
}

// ============================================================================
// Request Arguments
// ============================================================================

/// Arguments for the `initialize` request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeArguments {
    /// The ID of the client using this adapter.
    #[serde(default)]
    pub client_id: Option<String>,
    /// The human-readable name of the client.
    #[serde(default)]
    pub client_name: Option<String>,
    /// The ID of the debug adapter.
    #[serde(default)]
    pub adapter_id: Option<String>,
    /// The ISO-639 locale of the client.
    #[serde(default)]
    pub locale: Option<String>,
    /// If true, lines are 1-based (default).
    #[serde(default = "default_true")]
    pub lines_start_at1: bool,
    /// If true, columns are 1-based (default).
    #[serde(default = "default_true")]
    pub columns_start_at1: bool,
    /// Determines in what format paths are specified.
    #[serde(default)]
    pub path_format: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Arguments for the `launch` request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchArguments {
    /// The program (.vr file) to debug.
    pub program: String,
    /// Command-line arguments for the program.
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory for the program.
    #[serde(default)]
    pub cwd: Option<String>,
    /// If true, stop at the entry point.
    #[serde(default)]
    pub stop_on_entry: bool,
    /// If true, do not actually launch (for attach mode).
    #[serde(default)]
    pub no_debug: bool,
}

/// Arguments for the `setBreakpoints` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetBreakpointsArguments {
    /// The source file.
    pub source: Source,
    /// The requested breakpoints.
    #[serde(default)]
    pub breakpoints: Vec<SourceBreakpoint>,
}

/// A source location.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    /// The short name of the source (e.g., file name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The absolute path to the source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Unique source reference for sources without a file path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_reference: Option<i64>,
}

/// A breakpoint location in source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBreakpoint {
    /// The source line of the breakpoint.
    pub line: i64,
    /// Optional column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<i64>,
}

/// Arguments for the `continue` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinueArguments {
    /// Continue execution for this thread.
    pub thread_id: i64,
    /// If true, only continue the specified thread.
    #[serde(default)]
    pub single_thread: bool,
}

/// Arguments for `next`, `stepIn`, `stepOut` requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepArguments {
    /// Execute the step for this thread.
    pub thread_id: i64,
}

/// Arguments for the `stackTrace` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackTraceArguments {
    /// Retrieve the stack trace for this thread.
    pub thread_id: i64,
    /// Index of the first frame to return.
    #[serde(default)]
    pub start_frame: Option<i64>,
    /// Maximum number of frames to return.
    #[serde(default)]
    pub levels: Option<i64>,
}

/// Arguments for the `scopes` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopesArguments {
    /// Retrieve the scopes for this stack frame.
    pub frame_id: i64,
}

/// Arguments for the `variables` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariablesArguments {
    /// The variable container to expand.
    pub variables_reference: i64,
    /// Optional start index for paging.
    #[serde(default)]
    pub start: Option<i64>,
    /// Optional count for paging.
    #[serde(default)]
    pub count: Option<i64>,
}

/// Arguments for the `disconnect` request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisconnectArguments {
    /// If true, terminate the debuggee.
    #[serde(default)]
    pub terminate_debuggee: Option<bool>,
    /// If true, the disconnect request is part of a restart sequence.
    #[serde(default)]
    pub restart: Option<bool>,
}

// ============================================================================
// Response Bodies
// ============================================================================

/// Response body for `setBreakpoints`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetBreakpointsResponseBody {
    /// Breakpoints with verified status.
    pub breakpoints: Vec<Breakpoint>,
}

/// Response body for `continue`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinueResponseBody {
    /// If true, all threads were continued.
    #[serde(default = "default_true")]
    pub all_threads_continued: bool,
}

/// Response body for `threads`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsResponseBody {
    /// All threads.
    pub threads: Vec<Thread>,
}

/// Response body for `stackTrace`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackTraceResponseBody {
    /// The stack frames.
    pub stack_frames: Vec<StackFrame>,
    /// Total number of frames available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_frames: Option<i64>,
}

/// Response body for `scopes`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopesResponseBody {
    /// The scopes.
    pub scopes: Vec<Scope>,
}

/// Response body for `variables`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariablesResponseBody {
    /// The variables.
    pub variables: Vec<Variable>,
}

// ============================================================================
// Data Types
// ============================================================================

/// Information about a breakpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Breakpoint {
    /// Unique breakpoint identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// Whether the breakpoint could be set.
    pub verified: bool,
    /// An error message if the breakpoint could not be set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The source location.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// The actual line where the breakpoint was set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<i64>,
    /// The actual column where the breakpoint was set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<i64>,
}

/// A thread in the debuggee.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    /// Unique thread identifier.
    pub id: i64,
    /// Human-readable thread name.
    pub name: String,
}

/// A stack frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackFrame {
    /// Unique frame identifier.
    pub id: i64,
    /// The name of the frame (typically the function name).
    pub name: String,
    /// The source location.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// The line within the source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<i64>,
    /// The column within the line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<i64>,
}

/// A scope for variable grouping.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    /// Name of the scope (e.g., "Locals", "Arguments").
    pub name: String,
    /// Reference to the variables in this scope.
    pub variables_reference: i64,
    /// If true, the number of variables is large and should be loaded lazily.
    #[serde(default)]
    pub expensive: bool,
}

/// A variable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variable {
    /// The variable name.
    pub name: String,
    /// The variable value as a string.
    pub value: String,
    /// The type of the variable.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    /// If > 0, the variable has children that can be retrieved with a variables request.
    #[serde(default)]
    pub variables_reference: i64,
}

// ============================================================================
// Event Bodies
// ============================================================================

/// Event body for `stopped` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoppedEventBody {
    /// The reason for the event: "breakpoint", "step", "pause", "entry", "exception".
    pub reason: String,
    /// The thread which was stopped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<i64>,
    /// Additional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Full text of the stopped event (for display).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// If true, all threads are stopped.
    #[serde(default = "default_true")]
    pub all_threads_stopped: bool,
}

/// Event body for `terminated` event.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminatedEventBody {
    /// If true, the debuggee can be restarted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart: Option<bool>,
}

/// Event body for `output` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputEventBody {
    /// The output category: "console", "stdout", "stderr", "telemetry".
    #[serde(default = "default_console_category")]
    pub category: String,
    /// The output to report.
    pub output: String,
}

fn default_console_category() -> String {
    "console".to_string()
}

/// Event body for `initialized` event (empty body).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InitializedEventBody {}
