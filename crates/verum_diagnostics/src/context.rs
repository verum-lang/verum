//! Error context and chaining for propagating diagnostic information.
//!
//! This module provides types for building error chains and maintaining context
//! as errors propagate through the compiler.

use crate::{Diagnostic, Span};
use serde::{Deserialize, Serialize};
use std::fmt;
use verum_common::{List, Text};

/// Context information that can be attached to diagnostics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticContext {
    /// The stage of compilation where this occurred
    pub stage: CompilerStage,
    /// Source file being processed
    pub file: Option<Text>,
    /// Current function/scope being analyzed
    pub scope: Option<Text>,
    /// Additional metadata
    pub metadata: List<(Text, Text)>,
}

impl DiagnosticContext {
    pub fn new(stage: CompilerStage) -> Self {
        Self {
            stage,
            file: None,
            scope: None,
            metadata: List::new(),
        }
    }

    pub fn with_file(mut self, file: impl Into<Text>) -> Self {
        self.file = Some(file.into());
        self
    }

    pub fn with_scope(mut self, scope: impl Into<Text>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    pub fn add_metadata(mut self, key: impl Into<Text>, value: impl Into<Text>) -> Self {
        self.metadata.push((key.into(), value.into()));
        self
    }

    /// Format the context for display
    pub fn format(&self) -> Text {
        let mut parts = vec![format!("in {}", self.stage)];

        if let Some(file) = &self.file {
            parts.push(format!("file '{}'", file));
        }

        if let Some(scope) = &self.scope {
            parts.push(format!("scope '{}'", scope));
        }

        parts.join(", ").into()
    }
}

/// Compilation stages for context tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompilerStage {
    /// Lexical analysis
    Lexing,
    /// Syntax parsing
    Parsing,
    /// Type checking
    TypeChecking,
    /// Refinement type checking
    RefinementChecking,
    /// SMT verification
    Verification,
    /// Borrow checking
    BorrowChecking,
    /// Effect checking
    EffectChecking,
    /// Optimization
    Optimization,
    /// Code generation
    CodeGeneration,
    /// Linking
    Linking,
}

impl fmt::Display for CompilerStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompilerStage::Lexing => write!(f, "lexing"),
            CompilerStage::Parsing => write!(f, "parsing"),
            CompilerStage::TypeChecking => write!(f, "type checking"),
            CompilerStage::RefinementChecking => write!(f, "refinement checking"),
            CompilerStage::Verification => write!(f, "verification"),
            CompilerStage::BorrowChecking => write!(f, "borrow checking"),
            CompilerStage::EffectChecking => write!(f, "effect checking"),
            CompilerStage::Optimization => write!(f, "optimization"),
            CompilerStage::CodeGeneration => write!(f, "code generation"),
            CompilerStage::Linking => write!(f, "linking"),
        }
    }
}

/// A chain of related errors showing the propagation path
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorChain {
    /// The root diagnostic
    pub root: Diagnostic,
    /// Chain of contexts showing how the error propagated
    pub contexts: List<DiagnosticContext>,
    /// Related diagnostics
    pub related: List<Diagnostic>,
}

impl ErrorChain {
    pub fn new(root: Diagnostic) -> Self {
        Self {
            root,
            contexts: List::new(),
            related: List::new(),
        }
    }

    pub fn add_context(mut self, context: DiagnosticContext) -> Self {
        self.contexts.push(context);
        self
    }

    pub fn add_related(mut self, diagnostic: Diagnostic) -> Self {
        self.related.push(diagnostic);
        self
    }

    /// Get the root diagnostic
    pub fn root(&self) -> &Diagnostic {
        &self.root
    }

    /// Get all contexts
    pub fn contexts(&self) -> &[DiagnosticContext] {
        &self.contexts
    }

    /// Get related diagnostics
    pub fn related(&self) -> &[Diagnostic] {
        &self.related
    }

    /// Format the full error chain for display
    pub fn format(&self) -> Text {
        let mut output = Text::new();

        // Show root error
        output.push_str(&format!(
            "{}: {}\n",
            self.root.severity(),
            self.root.message()
        ));

        // Show contexts
        if !self.contexts.is_empty() {
            output.push_str("\nError propagation:\n");
            for (i, ctx) in self.contexts.iter().enumerate() {
                output.push_str(&format!("  {}. {}\n", i + 1, ctx.format()));
            }
        }

        // Show related errors
        if !self.related.is_empty() {
            output.push_str(&format!("\nRelated errors ({}):\n", self.related.len()));
            for (i, diag) in self.related.iter().enumerate() {
                output.push_str(&format!(
                    "  {}. {}: {}\n",
                    i + 1,
                    diag.severity(),
                    diag.message()
                ));
            }
        }

        output
    }
}

/// Trait for types that can provide diagnostic context
pub trait WithContext {
    /// Add context to this diagnostic
    fn with_context(self, context: DiagnosticContext) -> Self;

    /// Add a scope context
    fn in_scope(self, scope: impl Into<Text>) -> Self;

    /// Add a file context
    fn in_file(self, file: impl Into<Text>) -> Self;
}

impl WithContext for Diagnostic {
    fn with_context(self, context: DiagnosticContext) -> Self {
        // Add context as a child note
        let note = format!("in {}", context.format());
        let child = crate::DiagnosticBuilder::note_diag().message(note).build();

        // Use builder to add child
        crate::DiagnosticBuilder::new(self.severity())
            .code(self.code().unwrap_or("").to_string())
            .message(self.message().to_string())
            .child(child)
            .build()
    }

    fn in_scope(self, scope: impl Into<Text>) -> Self {
        let context = DiagnosticContext::new(CompilerStage::TypeChecking).with_scope(scope);
        self.with_context(context)
    }

    fn in_file(self, file: impl Into<Text>) -> Self {
        let context = DiagnosticContext::new(CompilerStage::TypeChecking).with_file(file);
        self.with_context(context)
    }
}

/// Builder for error chains
pub struct ErrorChainBuilder {
    root: Option<Diagnostic>,
    contexts: List<DiagnosticContext>,
    related: List<Diagnostic>,
}

impl ErrorChainBuilder {
    pub fn new(root: Diagnostic) -> Self {
        Self {
            root: Some(root),
            contexts: List::new(),
            related: List::new(),
        }
    }

    pub fn add_context(mut self, context: DiagnosticContext) -> Self {
        self.contexts.push(context);
        self
    }

    pub fn add_related(mut self, diagnostic: Diagnostic) -> Self {
        self.related.push(diagnostic);
        self
    }

    pub fn build(self) -> ErrorChain {
        ErrorChain {
            root: self.root.expect("root diagnostic must be set"),
            contexts: self.contexts,
            related: self.related,
        }
    }
}

/// Parse a location string like "/path/to/file.rs:123:45" into a Span
fn parse_location_string(location: &str) -> Option<Span> {
    // Find the last two colons which should be line:column
    // This handles paths with colons (e.g., Windows C:\path\...)
    let mut colon_positions: Vec<usize> = location.match_indices(':').map(|(i, _)| i).collect();

    if colon_positions.len() < 2 {
        return None;
    }

    // Try to parse line and column from the last two segments
    let col_pos = colon_positions.pop()?;
    let line_pos = colon_positions.pop()?;

    let file = &location[..line_pos];
    let line_str = &location[line_pos + 1..col_pos];
    let col_str = &location[col_pos + 1..];

    let line: usize = line_str.parse().ok()?;
    let column: usize = col_str.parse().ok()?;

    Some(Span::new(file, line, column, column))
}

/// A backtrace showing the call stack at the point of error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backtrace {
    /// Stack frames
    pub frames: List<StackFrame>,
    /// Whether the backtrace was captured
    pub captured: bool,
}

impl Backtrace {
    pub fn new() -> Self {
        Self {
            frames: List::new(),
            captured: false,
        }
    }

    /// Capture the current call stack.
    ///
    /// This uses std::backtrace::Backtrace to capture the current call stack
    /// and converts each frame into our StackFrame representation.
    ///
    /// Note: Backtrace capture respects the RUST_BACKTRACE environment variable.
    /// Set RUST_BACKTRACE=1 to enable backtrace capture.
    pub fn capture() -> Self {
        use std::backtrace::Backtrace as StdBacktrace;
        use std::backtrace::BacktraceStatus;

        let bt = StdBacktrace::capture();

        // Check if backtrace was actually captured
        if bt.status() != BacktraceStatus::Captured {
            return Self {
                frames: List::new(),
                captured: false,
            };
        }

        // Parse the backtrace output to extract frames
        // std::backtrace::Backtrace doesn't provide direct frame access,
        // so we parse its Display output
        let bt_string = format!("{}", bt);
        let mut frames: List<StackFrame> = List::new();

        for line in bt_string.lines() {
            let line = line.trim();

            // Skip empty lines and header/footer lines
            if line.is_empty() || line.starts_with("stack backtrace:") {
                continue;
            }

            // Parse lines like:
            // "   0: std::backtrace::Backtrace::capture"
            // "             at /rustc/.../src/backtrace.rs:234:18"
            if let Some(stripped) = line.strip_prefix("at ") {
                // This is a location line, update the last frame if exists
                if let Some(last_frame) = frames.last_mut() {
                    // Parse "path/to/file.rs:line:column"
                    // Format is typically: /path/to/file.rs:123:45
                    // We need to handle paths that may contain colons (e.g., Windows paths)
                    if let Some(parsed_span) = parse_location_string(stripped) {
                        last_frame.span = Some(parsed_span);
                    }
                }
            } else if let Some(idx_end) = line.find(':') {
                // This is a frame line like "   0: function_name"
                let function_part = &line[idx_end + 1..].trim();

                // Extract module and function name
                let (module, function) = if let Some(last_sep) = function_part.rfind("::") {
                    (
                        Some(Text::from(&function_part[..last_sep])),
                        Text::from(&function_part[last_sep + 2..]),
                    )
                } else {
                    (None, Text::from(*function_part))
                };

                frames.push(StackFrame {
                    function,
                    span: None,
                    module,
                });
            }
        }

        Self {
            frames,
            captured: true,
        }
    }

    /// Capture backtrace using the backtrace crate (when feature enabled)
    ///
    /// This provides more detailed backtrace information than the standard library
    /// implementation, including inlined frames and symbol resolution.
    #[cfg(feature = "backtrace")]
    pub fn capture_with_backtrace_crate() -> Self {
        use backtrace::Backtrace as ExternalBacktrace;

        let bt = ExternalBacktrace::new();
        let mut frames = List::new();

        for frame in bt.frames() {
            for symbol in frame.symbols() {
                let function = symbol
                    .name()
                    .map(|n| Text::from(n.to_string()))
                    .unwrap_or_else(|| Text::from("<unknown>"));

                let span = symbol.filename().and_then(|path| {
                    path.to_str().map(|file| {
                        let line = symbol.lineno().unwrap_or(0) as usize;
                        let column = symbol.colno().unwrap_or(0) as usize;
                        Span::new(file, line, column, column)
                    })
                });

                // Extract module from function name
                let module = function.rfind("::").map(|idx| Text::from(&function[..idx]));

                frames.push(StackFrame {
                    function: if let Some(idx) = function.rfind("::") {
                        Text::from(&function[idx + 2..])
                    } else {
                        function
                    },
                    span,
                    module,
                });
            }
        }

        Self {
            frames,
            captured: true,
        }
    }

    pub fn add_frame(mut self, frame: StackFrame) -> Self {
        self.frames.push(frame);
        self
    }

    /// Check if the backtrace was successfully captured
    pub fn is_captured(&self) -> bool {
        self.captured && !self.frames.is_empty()
    }

    /// Get the number of frames in the backtrace
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Filter out internal frames (std library, backtrace crate, etc.)
    pub fn filter_internal_frames(&self) -> Self {
        let filtered_frames: List<StackFrame> = self
            .frames
            .iter()
            .filter(|frame| {
                // Filter out internal Rust/stdlib frames
                let func = frame.function.as_str();
                !func.starts_with("std::")
                    && !func.starts_with("core::")
                    && !func.starts_with("backtrace::")
                    && !func.starts_with("verum_diagnostics::context::Backtrace")
                    && !func.contains("__rust_")
                    && !func.contains("_ZN") // Mangled C++ names
            })
            .cloned()
            .collect();

        Self {
            frames: filtered_frames,
            captured: self.captured,
        }
    }

    /// Format the backtrace for display
    pub fn format(&self) -> Text {
        if !self.captured {
            return "Backtrace not captured (set RUST_BACKTRACE=1 to enable)".into();
        }

        if self.frames.is_empty() {
            return "Backtrace captured but no frames available".into();
        }

        let mut output = Text::from("Backtrace:\n");
        for (i, frame) in self.frames.iter().enumerate() {
            output.push_str(&format!("  {}: {}\n", i, frame.format()));
        }
        output
    }

    /// Format the backtrace with a maximum number of frames
    pub fn format_limited(&self, max_frames: usize) -> Text {
        if !self.captured {
            return "Backtrace not captured (set RUST_BACKTRACE=1 to enable)".into();
        }

        if self.frames.is_empty() {
            return "Backtrace captured but no frames available".into();
        }

        let mut output = Text::from("Backtrace:\n");
        let display_count = std::cmp::min(self.frames.len(), max_frames);

        for (i, frame) in self.frames.iter().take(display_count).enumerate() {
            output.push_str(&format!("  {}: {}\n", i, frame.format()));
        }

        if self.frames.len() > max_frames {
            output.push_str(&format!(
                "  ... and {} more frames\n",
                self.frames.len() - max_frames
            ));
        }

        output
    }
}

impl Default for Backtrace {
    fn default() -> Self {
        Self::new()
    }
}

/// A single stack frame in a backtrace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackFrame {
    /// Function name
    pub function: Text,
    /// Source location
    pub span: Option<Span>,
    /// Module path
    pub module: Option<Text>,
}

impl StackFrame {
    pub fn new(function: impl Into<Text>) -> Self {
        Self {
            function: function.into(),
            span: None,
            module: None,
        }
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    pub fn with_module(mut self, module: impl Into<Text>) -> Self {
        self.module = Some(module.into());
        self
    }

    /// Format the frame for display
    pub fn format(&self) -> Text {
        let mut output = Text::new();

        if let Some(module) = &self.module {
            output.push_str(&format!("{}::", module));
        }

        output.push_str(&self.function);

        if let Some(span) = &self.span {
            output.push_str(&format!(" at {}:{}:{}", span.file, span.line, span.column));
        }

        output
    }
}
