//! Error Context Protocol Implementation
//!
//! The Error Context Protocol provides a standardized, truly zero-cost mechanism for
//! adding rich contextual information to errors as they propagate. On the success path,
//! context operations compile to no-ops (zero allocations, zero closure instantiation).
//! On the error path, context chains capture operational state at each call stack layer,
//! transforming opaque errors into actionable diagnostics.
//!
//! Key guarantees:
//! - SUCCESS PATH: absolutely zero overhead (no allocations, no string formatting)
//! - ERROR PATH: minimal overhead (allocate only on actual errors)
//! - with_context(|| f"...") closures are completely eliminated by dead code elimination on success
//! - Integrates with '?' operator, async/await, and context handlers
//!
//! This module provides the complete Error Context Protocol implementation with:
//! - Zero-cost context on success path
//! - Full stack trace preservation
//! - Result<T,E> integration with ergonomic API
//! - Context chain propagation
//! - Multiple display formats
//! - Backtrace capture (controlled by VERUM_BACKTRACE env var)
//!
//! # Performance Guarantees
//!
//! - **Success Path**: Absolutely zero overhead - no allocations, no closures
//! - **Error Path**: Minimal overhead - only allocate on actual errors
//! - **Backtrace**: Off by default, controlled by VERUM_BACKTRACE env var
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use verum_diagnostics::context_protocol::*;
//!
//! fn load_config(path: &str) -> Result<Config, ErrorWithContext<std::io::Error>> {
//!     std::fs::read_to_string(path)
//!         .context("Failed to read config file")?;
//!     // ... parsing logic
//!     Ok(Config::default())
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Instant;
use verum_common::{List, Map, Text};

#[cfg(feature = "backtrace")]
use backtrace::Backtrace as StdBacktrace;

/// Enhanced error with full context chain
///
/// This type wraps any error with rich contextual information including:
/// - Custom context messages
/// - Source location tracking
/// - Context chain from call stack
/// - Optional backtrace (when VERUM_BACKTRACE=1)
/// - Arbitrary metadata
///
/// # Performance
///
/// - Zero cost on success path (context only added on error)
/// - Lazy evaluation of expensive context via closures
/// - Backtrace capture only when explicitly enabled
#[derive(Clone)]
pub struct ErrorWithContext<E> {
    /// The underlying error
    pub error: E,
    /// Error context information
    pub context: ErrorContext,
    /// Optional backtrace (captured only if VERUM_BACKTRACE=1)
    pub backtrace: Option<Backtrace>,
}

/// Error context information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorContext {
    /// Primary context message
    pub message: Text,
    /// Source code location where context was added
    pub location: SourceLocation,
    /// Chain of context frames showing call stack
    pub context_chain: List<ContextFrame>,
    /// Arbitrary metadata attached to this error
    pub metadata: Map<Text, ContextValue>,
}

/// A single frame in the context chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFrame {
    /// Operation being performed
    pub operation: Text,
    /// Source location
    pub location: SourceLocation,
    /// When this context was created
    pub timestamp: u64, // Unix timestamp in microseconds
    /// Thread that created this context
    pub thread_id: u64,
}

/// Source code location
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceLocation {
    /// File path
    pub file: Text,
    /// Line number
    pub line: u32,
    /// Column number
    pub column: u32,
    /// Function name (if available)
    pub function: Option<Text>,
}

/// Metadata value for context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContextValue {
    Text(Text),
    Int(i64),
    Float(f64),
    Bool(bool),
    List(List<ContextValue>),
    Map(std::collections::HashMap<Text, ContextValue>),
}

/// Backtrace capture and formatting
///
/// Backtrace capture is controlled by VERUM_BACKTRACE environment variable:
/// - VERUM_BACKTRACE=0 or unset: No backtrace (default)
/// - VERUM_BACKTRACE=1: Basic backtrace
/// - VERUM_BACKTRACE=full: Full backtrace with inlined frames
#[derive(Clone)]
pub struct Backtrace {
    frames: List<StackFrame>,
    #[allow(dead_code)]
    captured_at: Instant,
    mode: BacktraceMode,
}

/// Backtrace capture mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BacktraceMode {
    Disabled,
    Basic,
    Full,
}

/// A single stack frame
#[derive(Debug, Clone)]
pub struct StackFrame {
    /// Function name
    pub function: Text,
    /// File path (if available)
    pub file: Option<Text>,
    /// Line number (if available)
    pub line: Option<usize>,
    /// Column number (if available)
    pub column: Option<usize>,
    /// Instruction pointer
    pub instruction_pointer: usize,
}

impl SourceLocation {
    /// Create a new source location
    pub fn new(file: impl Into<Text>, line: u32, column: u32) -> Self {
        Self {
            file: file.into(),
            line,
            column,
            function: None,
        }
    }

    /// Create source location with function name
    pub fn with_function(mut self, function: impl Into<Text>) -> Self {
        self.function = Some(function.into());
        self
    }

    /// Get the caller's source location
    ///
    /// This uses std::panic::Location to capture the caller's location.
    /// Note: This requires the caller to use #[track_caller]
    #[track_caller]
    pub fn caller() -> Self {
        let location = std::panic::Location::caller();
        Self {
            file: location.file().into(),
            line: location.line(),
            column: location.column(),
            function: None,
        }
    }
}

impl fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref func) = self.function {
            write!(f, "{}:{}:{} in {}", self.file, self.line, self.column, func)
        } else {
            write!(f, "{}:{}:{}", self.file, self.line, self.column)
        }
    }
}

impl Default for ErrorContext {
    fn default() -> Self {
        Self {
            message: Text::from(""),
            location: SourceLocation::new("unknown", 0, 0),
            context_chain: List::new(),
            metadata: Map::new(),
        }
    }
}

impl<E> ErrorWithContext<E> {
    /// Create a new error with context
    pub fn new(error: E, context: ErrorContext) -> Self {
        let backtrace = Backtrace::capture_if_enabled();
        Self {
            error,
            context,
            backtrace,
        }
    }

    /// Get the underlying error
    pub fn error(&self) -> &E {
        &self.error
    }

    /// Get the context
    pub fn context_info(&self) -> &ErrorContext {
        &self.context
    }

    /// Get the backtrace if available
    pub fn backtrace(&self) -> Option<&Backtrace> {
        self.backtrace.as_ref()
    }

    /// Add another layer of context
    #[track_caller]
    pub fn with_additional_context(mut self, message: impl Into<Text>) -> Self
    where
        E: Clone,
    {
        let frame = ContextFrame {
            operation: message.into(),
            location: SourceLocation::caller(),
            timestamp: current_timestamp_micros(),
            thread_id: thread_id_as_u64(),
        };
        self.context.context_chain.push(frame);
        self
    }

    /// Add metadata to this error
    pub fn with_metadata(mut self, key: impl Into<Text>, value: ContextValue) -> Self {
        self.context.metadata.insert(key.into(), value);
        self
    }
}

impl<E: fmt::Display> fmt::Display for ErrorWithContext<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.context.message)?;
        if !self.context.context_chain.is_empty() {
            write!(
                f,
                " (with {} context frames)",
                self.context.context_chain.len()
            )?;
        }
        Ok(())
    }
}

impl<E: std::error::Error + 'static> std::error::Error for ErrorWithContext<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

// Manually implement Debug to avoid recursion
impl<E: fmt::Debug> fmt::Debug for ErrorWithContext<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ErrorWithContext")
            .field("error", &self.error)
            .field("context", &self.context)
            .field("backtrace", &self.backtrace.as_ref().map(|_| "<backtrace>"))
            .finish()
    }
}

/// Extension trait for adding context to Results
///
/// This trait provides ergonomic methods for adding rich contextual information
/// to error values with zero cost on the success path.
pub trait ResultContext<T, E> {
    /// Add static context to error
    ///
    /// # Performance
    /// - Success path: 0 overhead (message not evaluated)
    /// - Error path: 1 string allocation
    ///
    /// # Example
    /// ```rust,ignore
    /// let result = read_file(path).context("Failed to read config file")?;
    /// ```
    #[track_caller]
    fn context<C: Into<Text>>(self, context: C) -> Result<T, ErrorWithContext<E>>;

    /// Add lazy context via closure (true zero-cost abstraction)
    ///
    /// The closure is ONLY called on error - zero overhead on success path.
    /// This is the preferred method for expensive string formatting.
    ///
    /// # Performance
    /// - Success path: 0 overhead (closure never instantiated or called)
    /// - Error path: Closure execution + 1 allocation
    ///
    /// # Example
    /// ```rust,ignore
    /// let result = query_db(sql)
    ///     .with_context(|| format!("Query failed: {}", sql))?;
    /// ```
    #[track_caller]
    fn with_context<C, F>(self, f: F) -> Result<T, ErrorWithContext<E>>
    where
        F: FnOnce() -> C,
        C: Into<Text>;

    /// Add location context
    ///
    /// Explicitly set the source location (useful when #[track_caller] isn't available)
    fn at(self, file: &str, line: u32, column: u32) -> Result<T, ErrorWithContext<E>>;

    /// Add operation context
    ///
    /// Add a context frame describing the operation being performed
    #[track_caller]
    fn operation(self, op: &str) -> Result<T, ErrorWithContext<E>>;

    /// Attach metadata
    ///
    /// Add arbitrary metadata to the error context
    fn meta<K, V>(self, key: K, value: V) -> Result<T, ErrorWithContext<E>>
    where
        K: Into<Text>,
        V: Into<ContextValue>;
}

impl<T, E> ResultContext<T, E> for Result<T, E> {
    #[track_caller]
    fn context<C: Into<Text>>(self, context: C) -> Result<T, ErrorWithContext<E>> {
        self.map_err(|error| {
            let location = SourceLocation::caller();
            ErrorWithContext {
                error,
                context: ErrorContext {
                    message: context.into(),
                    location,
                    context_chain: List::new(),
                    metadata: Map::new(),
                },
                backtrace: Backtrace::capture_if_enabled(),
            }
        })
    }

    #[track_caller]
    fn with_context<C, F>(self, f: F) -> Result<T, ErrorWithContext<E>>
    where
        F: FnOnce() -> C,
        C: Into<Text>,
    {
        self.map_err(|error| {
            let location = SourceLocation::caller();
            ErrorWithContext {
                error,
                context: ErrorContext {
                    message: f().into(),
                    location,
                    context_chain: List::new(),
                    metadata: Map::new(),
                },
                backtrace: Backtrace::capture_if_enabled(),
            }
        })
    }

    fn at(self, file: &str, line: u32, column: u32) -> Result<T, ErrorWithContext<E>> {
        self.map_err(|error| {
            let location = SourceLocation::new(file, line, column);
            ErrorWithContext {
                error,
                context: ErrorContext {
                    message: Text::from(""),
                    location,
                    context_chain: List::new(),
                    metadata: Map::new(),
                },
                backtrace: Backtrace::capture_if_enabled(),
            }
        })
    }

    #[track_caller]
    fn operation(self, op: &str) -> Result<T, ErrorWithContext<E>> {
        self.map_err(|error| {
            let location = SourceLocation::caller();
            let frame = ContextFrame {
                operation: op.into(),
                location: location.clone(),
                timestamp: current_timestamp_micros(),
                thread_id: thread_id_as_u64(),
            };

            let mut context_chain = List::new();
            context_chain.push(frame);

            ErrorWithContext {
                error,
                context: ErrorContext {
                    message: op.into(),
                    location,
                    context_chain,
                    metadata: Map::new(),
                },
                backtrace: Backtrace::capture_if_enabled(),
            }
        })
    }

    fn meta<K, V>(self, key: K, value: V) -> Result<T, ErrorWithContext<E>>
    where
        K: Into<Text>,
        V: Into<ContextValue>,
    {
        self.map_err(|error| {
            let mut metadata = Map::new();
            metadata.insert(key.into(), value.into());

            ErrorWithContext {
                error,
                context: ErrorContext {
                    message: Text::from(""),
                    location: SourceLocation::new("unknown", 0, 0),
                    context_chain: List::new(),
                    metadata,
                },
                backtrace: Backtrace::capture_if_enabled(),
            }
        })
    }
}

/// Display trait for error formatting
pub trait DisplayError {
    /// Format error with full context (all details)
    fn display_full(&self) -> Text;

    /// Format error for end user (concise, friendly)
    fn display_user(&self) -> Text;

    /// Format error for developer (verbose, technical)
    fn display_developer(&self) -> Text;

    /// Format error for logging (structured, machine-readable)
    fn display_log(&self) -> Text;
}

impl<E: fmt::Display> DisplayError for ErrorWithContext<E> {
    fn display_full(&self) -> Text {
        let mut output = Text::new();

        // Main error message
        output.push_str(&format!("Error: {}\n", self.error));

        // Context message
        if !self.context.message.is_empty() {
            output.push_str(&format!("Context: {}\n", self.context.message));
        }

        // Location
        output.push_str(&format!("  at {}\n", self.context.location));

        // Context chain
        if !self.context.context_chain.is_empty() {
            output.push_str("\nContext chain:\n");
            for (i, frame) in self.context.context_chain.iter().enumerate() {
                output.push_str(&format!(
                    "  {}: {} at {}\n",
                    i + 1,
                    frame.operation,
                    frame.location
                ));
            }
        }

        // Metadata
        if !self.context.metadata.is_empty() {
            output.push_str("\nMetadata:\n");
            for (key, value) in &self.context.metadata {
                output.push_str(&format!("  {}: {:?}\n", key, value));
            }
        }

        // Backtrace
        if let Some(ref bt) = self.backtrace {
            output.push_str(&format!("\n{}", bt.format()));
        }

        output
    }

    fn display_user(&self) -> Text {
        // Concise user-friendly message
        if !self.context.message.is_empty() {
            format!("{}: {}", self.context.message, self.error).into()
        } else {
            format!("{}", self.error).into()
        }
    }

    fn display_developer(&self) -> Text {
        // Same as full for developers
        self.display_full()
    }

    fn display_log(&self) -> Text {
        // Production-grade structured JSON output for machine-readable logging
        // Compatible with log aggregation systems (ELK, Splunk, Datadog, etc.)
        let error_escaped = escape_json_string(&self.error.to_string());

        let mut json: Text = format!(r#"{{"error": "{}"#, error_escaped).into();

        // Add context message if present
        if !self.context.message.is_empty() {
            let context_escaped = escape_json_string(&self.context.message);
            json.push_str(&format!(r#", "context": "{}"#, context_escaped));
        }

        // Add structured location information
        json.push_str(&format!(
            r#", "location": {{"file": "{}", "line": {}, "column": {}"#,
            escape_json_string(&self.context.location.file),
            self.context.location.line,
            self.context.location.column
        ));

        if let Some(ref func) = self.context.location.function {
            json.push_str(&format!(r#", "function": "{}"#, escape_json_string(func)));
        }
        json.push('}');

        // Add context chain as array
        if !self.context.context_chain.is_empty() {
            json.push_str(r#", "context_chain": ["#);
            for (i, frame) in self.context.context_chain.iter().enumerate() {
                if i > 0 {
                    json.push_str(", ");
                }
                json.push_str(&format!(
                    r#"{{"operation": "{}", "file": "{}", "line": {}, "column": {}, "timestamp_us": {}, "thread_id": {}}}"#,
                    escape_json_string(&frame.operation),
                    escape_json_string(&frame.location.file),
                    frame.location.line,
                    frame.location.column,
                    frame.timestamp,
                    frame.thread_id
                ));
            }
            json.push(']');
        }

        // Add metadata as object
        if !self.context.metadata.is_empty() {
            json.push_str(r#", "metadata": {"#);
            for (i, (key, value)) in self.context.metadata.iter().enumerate() {
                if i > 0 {
                    json.push_str(", ");
                }
                let value_json = context_value_to_json(value);
                json.push_str(&format!(r#""{}": {}"#, escape_json_string(key), value_json));
            }
            json.push('}');
        }

        // Add backtrace presence indicator
        if self.backtrace.is_some() {
            json.push_str(r#", "has_backtrace": true"#);
        }

        json.push('}');
        json
    }
}

impl Backtrace {
    /// Capture backtrace if enabled via VERUM_BACKTRACE environment variable
    ///
    /// Default: No backtrace capture (VERUM_BACKTRACE=0 or unset)
    /// Enable: VERUM_BACKTRACE=1 (basic backtrace)
    /// Full: VERUM_BACKTRACE=full (full backtrace with inlined frames)
    pub fn capture_if_enabled() -> Option<Self> {
        use std::sync::OnceLock;

        static MODE: OnceLock<BacktraceMode> = OnceLock::new();
        let mode = MODE.get_or_init(|| {
            match std::env::var("VERUM_BACKTRACE").as_deref() {
                Ok("1") | Ok("true") => BacktraceMode::Basic,
                Ok("full") => BacktraceMode::Full,
                _ => BacktraceMode::Disabled, // DEFAULT
            }
        });

        match mode {
            BacktraceMode::Disabled => None,
            BacktraceMode::Basic => Some(Self::capture_basic()),
            BacktraceMode::Full => Some(Self::capture_full()),
        }
    }

    /// Capture basic backtrace
    #[cfg(feature = "backtrace")]
    fn capture_basic() -> Self {
        let bt = StdBacktrace::new();
        let frames = Self::convert_backtrace(&bt, false);
        Self {
            frames,
            captured_at: Instant::now(),
            mode: BacktraceMode::Basic,
        }
    }

    /// Capture basic backtrace (no-op without backtrace feature)
    #[cfg(not(feature = "backtrace"))]
    fn capture_basic() -> Self {
        Self {
            frames: List::new(),
            captured_at: Instant::now(),
            mode: BacktraceMode::Basic,
        }
    }

    /// Capture full backtrace with inlined frames
    #[cfg(feature = "backtrace")]
    fn capture_full() -> Self {
        let bt = StdBacktrace::new();
        let frames = Self::convert_backtrace(&bt, true);
        Self {
            frames,
            captured_at: Instant::now(),
            mode: BacktraceMode::Full,
        }
    }

    /// Capture full backtrace (no-op without backtrace feature)
    #[cfg(not(feature = "backtrace"))]
    fn capture_full() -> Self {
        Self {
            frames: List::new(),
            captured_at: Instant::now(),
            mode: BacktraceMode::Full,
        }
    }

    #[cfg(feature = "backtrace")]
    fn convert_backtrace(bt: &StdBacktrace, _full: bool) -> List<StackFrame> {
        let mut frames = List::new();

        for frame in bt.frames() {
            for symbol in frame.symbols() {
                let function = symbol
                    .name()
                    .map(|n| n.to_string().into())
                    .unwrap_or_else(|| Text::from("<unknown>"));

                let file = symbol.filename().and_then(|p| p.to_str()).map(|s| s.into());

                let line = symbol.lineno().map(|l| l as usize);
                let column = symbol.colno().map(|c| c as usize);

                frames.push(StackFrame {
                    function,
                    file,
                    line,
                    column,
                    instruction_pointer: frame.ip() as usize,
                });
            }
        }

        frames
    }

    /// Format backtrace for display
    pub fn format(&self) -> Text {
        let mut output = Text::from("Stack trace:\n");

        for (i, frame) in self.frames.iter().enumerate() {
            let location: Text = if let Some(ref file) = frame.file {
                format!(
                    "{}:{}:{}",
                    file,
                    frame.line.unwrap_or(0),
                    frame.column.unwrap_or(0)
                )
                .into()
            } else {
                Text::from("<unknown>")
            };

            output.push_str(&format!("  {}: {} at {}\n", i, frame.function, location));
        }

        output
    }

    /// Get the frames
    pub fn frames(&self) -> &List<StackFrame> {
        &self.frames
    }
}

impl fmt::Debug for Backtrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Backtrace {{ frames: {} frames, mode: {:?} }}",
            self.frames.len(),
            self.mode
        )
    }
}

// Conversion implementations for ContextValue
impl From<&str> for ContextValue {
    fn from(s: &str) -> Self {
        ContextValue::Text(s.into())
    }
}

impl From<Text> for ContextValue {
    fn from(t: Text) -> Self {
        ContextValue::Text(t)
    }
}

impl From<i32> for ContextValue {
    fn from(i: i32) -> Self {
        ContextValue::Int(i as i64)
    }
}

impl From<i64> for ContextValue {
    fn from(i: i64) -> Self {
        ContextValue::Int(i)
    }
}

impl From<f64> for ContextValue {
    fn from(f: f64) -> Self {
        ContextValue::Float(f)
    }
}

impl From<bool> for ContextValue {
    fn from(b: bool) -> Self {
        ContextValue::Bool(b)
    }
}

/// Convenient macro for adding context
///
/// # Example
/// ```rust,ignore
/// let result = read_file(path);
/// let content = context!(result, "Failed to read config file")?;
/// ```
#[macro_export]
macro_rules! context {
    ($result:expr, $msg:expr) => {
        $result.context($msg)
    };
}

/// Macro for try with context
///
/// # Example
/// ```rust,ignore
/// let content = try_context!(read_file(path), "Failed to read config file");
/// ```
#[macro_export]
macro_rules! try_context {
    ($result:expr, $msg:expr) => {
        match $result {
            Ok(val) => val,
            Err(e) => {
                return Err(
                    $crate::context_protocol::ResultContext::context(Err(e), $msg).unwrap_err(),
                )
            }
        }
    };
}

// Helper functions

/// Convert a ContextValue to its JSON string representation.
///
/// This function recursively serializes all ContextValue variants:
/// - Text: Escaped JSON string
/// - Int: JSON number
/// - Float: JSON number (with special handling for NaN/Infinity)
/// - Bool: JSON boolean (true/false)
/// - List: JSON array with recursively serialized elements
/// - Map: JSON object with recursively serialized values
fn context_value_to_json(value: &ContextValue) -> Text {
    match value {
        ContextValue::Text(s) => format!(r#""{}""#, escape_json_string(s)).into(),
        ContextValue::Int(n) => n.to_string().into(),
        ContextValue::Float(f) => {
            if f.is_nan() {
                r#""NaN""#.into()
            } else if f.is_infinite() {
                if *f > 0.0 {
                    r#""Infinity""#.into()
                } else {
                    r#""-Infinity""#.into()
                }
            } else {
                f.to_string().into()
            }
        }
        ContextValue::Bool(b) => b.to_string().into(),
        ContextValue::List(items) => {
            let mut json = Text::from("[");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    json.push_str(", ");
                }
                json.push_str(&context_value_to_json(item));
            }
            json.push(']');
            json
        }
        ContextValue::Map(map) => {
            let mut json = Text::from("{");
            for (i, (key, val)) in map.iter().enumerate() {
                if i > 0 {
                    json.push_str(", ");
                }
                json.push_str(&format!(
                    r#""{}": {}"#,
                    escape_json_string(key),
                    context_value_to_json(val)
                ));
            }
            json.push('}');
            json
        }
    }
}

/// Escape a string for JSON output.
///
/// Handles all special characters that need escaping in JSON strings:
/// - Backslash (`\`) -> `\\`
/// - Double quote (`"`) -> `\"`
/// - Newline -> `\n`
/// - Carriage return -> `\r`
/// - Tab -> `\t`
/// - Form feed -> `\f`
/// - Backspace -> `\b`
/// - Control characters (U+0000 to U+001F) -> `\uXXXX`
fn escape_json_string(s: &str) -> Text {
    let mut result = Text::new();

    for c in s.chars() {
        match c {
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\u{08}' => result.push_str("\\b"), // Backspace
            '\u{0C}' => result.push_str("\\f"), // Form feed
            c if c.is_control() => {
                // Escape other control characters as \uXXXX
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => result.push(c),
        }
    }

    result
}

/// Get current timestamp in microseconds
fn current_timestamp_micros() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Get thread ID as u64
fn thread_id_as_u64() -> u64 {
    // Convert ThreadId to u64 (best effort)
    // This is a hack since ThreadId doesn't expose its internal value
    let thread_id = std::thread::current().id();
    let debug_str = format!("{:?}", thread_id);
    // ThreadId debug format is "ThreadId(N)"
    debug_str
        .trim_start_matches("ThreadId(")
        .trim_end_matches(')')
        .parse()
        .unwrap_or(0)
}

/// Integration helpers for verum_error types
#[cfg(feature = "verum-error-integration")]
pub mod verum_error_integration {
    use super::*;

    /// Convert ErrorWithContext to verum_error::VerumError
    ///
    /// This preserves the context chain by building it into the error message
    pub fn to_verum_error<E: fmt::Display>(err: ErrorWithContext<E>) -> verum_error::VerumError {
        let full_message = err.display_full();
        verum_error::VerumError::new(full_message.to_string(), verum_error::ErrorKind::Other)
    }

    /// Extension trait for converting Results to verum_error
    pub trait ToVerumError<T, E> {
        fn to_verum_error(self) -> Result<T, verum_error::VerumError>;
    }

    impl<T, E: fmt::Display> ToVerumError<T, E> for Result<T, ErrorWithContext<E>> {
        fn to_verum_error(self) -> Result<T, verum_error::VerumError> {
            self.map_err(to_verum_error)
        }
    }
}

/// Integration helpers for CBGR error types (placeholder for future CBGR runtime)
pub mod cbgr_integration {
    /// Placeholder -- CBGR runtime integration will be added when the runtime is implemented.
    pub fn example_usage() {
        // This is just documentation, no actual code
    }
}

/// Integration helpers for SMT verification errors
pub mod smt_integration {
    #[cfg(feature = "smt-integration")]
    use super::ErrorWithContext;

    /// Type alias for SMT operations with context
    #[cfg(feature = "smt-integration")]
    pub type SmtResult<T> = Result<T, ErrorWithContext<verum_smt::Error>>;

    /// Example usage pattern for SMT operations
    ///
    /// ```rust,ignore
    /// use verum_diagnostics::context_protocol::smt_integration::*;
    ///
    /// fn verify_with_context(constraint: &Constraint) -> SmtResult<Proof> {
    ///     smt_verify(constraint)
    ///         .with_context(|| format!(
    ///             "Failed to verify constraint: {}\nSolver: {}\nTimeout: {}ms",
    ///             constraint,
    ///             solver_name(),
    ///             timeout_ms()
    ///         ))?
    /// }
    /// ```
    pub fn example_usage() {
        // This is just documentation, no actual code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_basic() {
        let result: Result<(), &str> = Err("base error");
        let err = result.context("operation failed").unwrap_err();

        assert_eq!(err.context.message.as_str(), "operation failed");
    }

    #[test]
    fn test_with_context_lazy() {
        let expensive_value = "expensive";
        let result: Result<(), &str> = Err("base error");

        let err = result
            .with_context(|| format!("computed: {}", expensive_value))
            .unwrap_err();
        assert!(err.context.message.as_str().contains("computed"));
    }

    #[test]
    fn test_display_formats() {
        let result: Result<(), &str> = Err("base error");
        let err = result.context("operation failed").unwrap_err();

        let full = err.display_full();
        assert!(full.as_str().contains("base error"));
        assert!(full.as_str().contains("operation failed"));

        let user = err.display_user();
        assert!(!user.as_str().is_empty());

        let log = err.display_log();
        // Updated for structured JSON format
        assert!(log.as_str().contains("\"error\":"));
        assert!(log.as_str().contains("\"location\":"));
        assert!(log.as_str().starts_with("{"));
        assert!(log.as_str().ends_with("}"));
    }
}
