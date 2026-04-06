//! Context-specific error types and diagnostics.
//!
//! This module provides world-class error messages for context-related issues,
//! especially E0301 (context used but not declared). It implements:
//! - Call chain propagation visualization
//! - Context requirement tracking
//! - Smart suggestions based on common patterns
//! - "Did you mean" suggestions for typos

use crate::{
    Diagnostic, DiagnosticBuilder, Span,
    suggestion::{Applicability, Suggestion, SuggestionBuilder},
};
use serde::{Deserialize, Serialize};
use verum_common::{List, Text};

// Context error codes for Verum's capability-based dependency injection system.
// E0301: context used but not declared in 'using' clause
// E0302: context declared in signature but no provider installed via 'provide'
// E0303: context type mismatch between declaration and provider
// E0304: context not available in current scope (e.g., nested function without propagation)
// E0305: duplicate context declaration in same 'using' clause
// E0306: (deprecated here, moved to capability_attenuation_errors) capability violation
pub mod error_codes {
    /// Context used but not declared
    pub const E0301: &str = "E0301";
    /// Context declared but not provided
    pub const E0302: &str = "E0302";
    /// Context type mismatch
    pub const E0303: &str = "E0303";
    /// Context not available in this scope
    pub const E0304: &str = "E0304";
    /// Duplicate context declaration
    pub const E0305: &str = "E0305";
    /// Context group undefined (NOTE: E0306 moved to capability_attenuation_errors)
    pub const E0306_DEPRECATED: &str = "E0306";

    // === Async/Sync Context Errors (E08xx) ===

    /// Async context mismatch: async context used in sync function
    ///
    /// This error occurs when an async context (declared with `context async X`)
    /// is used in a synchronous function. Async contexts require an async runtime
    /// and can only be used in async functions.
    ///
    /// # Example
    /// ```verum
    /// context async Database { ... }
    ///
    /// // ERROR E0803: Cannot use async context 'Database' in sync function
    /// fn sync_function() using [Database] { ... }
    /// ```
    pub const E0803: &str = "E0803";

    /// Async context method mismatch
    ///
    /// This error occurs in several scenarios:
    /// 1. Calling async context method from sync function
    /// 2. Calling async context method without `.await` in async function
    /// 3. Using `.await` in sync function
    /// 4. Providing sync implementation for async context
    ///
    /// # Example
    /// ```verum
    /// context async Database {
    ///     async fn query(sql: Text) -> List<Row>;
    /// }
    ///
    /// // ERROR E0804: Async context method 'Database.query' must be awaited
    /// async fn fetch() using [Database] -> List<Row> {
    ///     Database.query("SELECT 1")  // Missing .await
    /// }
    /// ```
    pub const E0804: &str = "E0804";
}

/// A single frame in a call chain showing context requirements
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallFrame {
    /// Function name
    pub function: Text,
    /// Source location
    pub span: Span,
    /// Contexts required by this function
    pub required_contexts: List<Text>,
    /// Whether this is the function where the error originated
    pub is_origin: bool,
}

impl CallFrame {
    pub fn new(function: impl Into<Text>, span: Span) -> Self {
        Self {
            function: function.into(),
            span,
            required_contexts: List::new(),
            is_origin: false,
        }
    }

    pub fn with_contexts(mut self, contexts: List<Text>) -> Self {
        self.required_contexts = contexts;
        self
    }

    pub fn origin(mut self) -> Self {
        self.is_origin = true;
        self
    }

    /// Format this frame for display in call chain
    pub fn format(&self, indent: usize) -> Text {
        let indent_str = "  ".repeat(indent);
        let context_str: Text = if self.required_contexts.is_empty() {
            Text::new()
        } else {
            format!(" [requires {}]", self.required_contexts.join(", ")).into()
        };

        format!(
            "{}{}() @ {}:{}{}",
            indent_str, self.function, self.span.file, self.span.line, context_str
        )
        .into()
    }
}

/// A call chain showing how context requirements propagate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallChain {
    /// Frames in the call chain, from entry point to error site
    pub frames: List<CallFrame>,
    /// The missing context that triggered this chain
    pub missing_context: Text,
}

impl CallChain {
    pub fn new(missing_context: impl Into<Text>) -> Self {
        Self {
            frames: List::new(),
            missing_context: missing_context.into(),
        }
    }

    pub fn add_frame(mut self, frame: CallFrame) -> Self {
        self.frames.push(frame);
        self
    }

    /// Get the entry point (first frame)
    pub fn entry_point(&self) -> Option<&CallFrame> {
        self.frames.first()
    }

    /// Get the error origin (last frame)
    pub fn error_origin(&self) -> Option<&CallFrame> {
        self.frames.last()
    }

    /// Format the call chain for display
    pub fn format(&self) -> Text {
        if self.frames.is_empty() {
            return "Call chain empty".into();
        }

        let mut output = Text::new();
        output.push_str(&format!(
            "call chain requiring '{}':\n",
            self.missing_context
        ));

        for (i, frame) in self.frames.iter().enumerate() {
            let is_last = i == self.frames.len() - 1;
            let prefix = if i == 0 {
                "  "
            } else if is_last {
                "    └─> "
            } else {
                "    └─> "
            };

            output.push_str(&format!("{}{}\n", prefix, frame.format(0)));
        }

        output
    }

    /// Generate suggestions for fixing the missing context
    pub fn suggestions(&self) -> List<Suggestion> {
        let mut suggestions = List::new();

        // Find the best place to add the context (usually the entry point)
        if let Some(entry) = self.entry_point() {
            // Suggestion 1: Add context to entry point signature
            suggestions.push(
                SuggestionBuilder::new(format!(
                    "Add '{}' to function signature",
                    self.missing_context
                ))
                .description(format!(
                    "Add 'using [{}]' to the signature of '{}'",
                    self.missing_context, entry.function
                ))
                .code(format!("using [{}]  // <-- add this", self.missing_context))
                .applicability(Applicability::Recommended)
                .build(),
            );

            // Suggestion 2: Provide the context at call site
            suggestions.push(
                SuggestionBuilder::new(format!(
                    "Provide '{}' context before calling '{}'",
                    self.missing_context, entry.function
                ))
                .description("Install the context provider before the function call")
                .code(format!(
                    "provide {} = create_{}();\n{}();",
                    self.missing_context,
                    self.missing_context.to_lowercase(),
                    entry.function
                ))
                .applicability(Applicability::Alternative)
                .build(),
            );
        }

        // Suggestion 3: Create a context group if multiple contexts needed
        if let Some(entry) = self.entry_point()
            && !entry.required_contexts.is_empty()
        {
            let all_contexts = {
                let mut contexts = entry.required_contexts.clone();
                contexts.push(self.missing_context.clone());
                contexts.sort();
                contexts.dedup();
                contexts
            };

            if all_contexts.len() > 2 {
                suggestions.push(
                    SuggestionBuilder::new("Create a context group for reusability")
                        .description("Define a context group to avoid repeating context lists")
                        .code(format!(
                            "using MyContext = [{}];\n\nfn {}() -> ReturnType\n    using MyContext",
                            all_contexts.join(", "),
                            entry.function
                        ))
                        .applicability(Applicability::Alternative)
                        .build(),
                );
            }
        }

        suggestions
    }
}

/// Context error builder for E0301 (context used but not declared)
pub struct ContextNotDeclaredError {
    context_name: Text,
    usage_span: Span,
    call_chain: Option<CallChain>,
    similar_contexts: List<Text>,
}

impl ContextNotDeclaredError {
    pub fn new(context_name: impl Into<Text>, usage_span: Span) -> Self {
        Self {
            context_name: context_name.into(),
            usage_span,
            call_chain: None,
            similar_contexts: List::new(),
        }
    }

    pub fn with_call_chain(mut self, chain: CallChain) -> Self {
        self.call_chain = Some(chain);
        self
    }

    pub fn with_similar_contexts(mut self, similar: List<Text>) -> Self {
        self.similar_contexts = similar;
        self
    }

    /// Build the diagnostic with all visualizations
    pub fn build(self) -> Diagnostic {
        let mut builder = DiagnosticBuilder::error()
            .code(error_codes::E0301)
            .message(format!(
                "context '{}' used but not declared",
                self.context_name
            ))
            .span_label(
                self.usage_span.clone(),
                format!("requires [{}] context", self.context_name),
            );

        // Add call chain visualization
        if let Some(chain) = &self.call_chain {
            builder = builder.add_note(chain.format());

            // Add suggestions from call chain
            for suggestion in chain.suggestions() {
                builder = builder.help(format_suggestion(&suggestion));
            }
        } else {
            // Simple help without call chain
            builder = builder.help(format!(
                "add 'using [{}]' to function signature",
                self.context_name
            ));
        }

        // Add "did you mean" suggestions
        if !self.similar_contexts.is_empty() {
            let mut did_you_mean = Text::from("did you mean one of these contexts?\n");
            for (i, ctx) in self.similar_contexts.iter().enumerate() {
                did_you_mean.push_str(&format!("  {}. {}\n", i + 1, ctx));
            }
            builder = builder.add_note(did_you_mean);
        }

        // Add general context system help
        builder = builder
            .help("contexts must be declared with 'using [Context]' in the function signature")
            .help("then provided with 'provide Context = implementation' before calling");

        builder.build()
    }
}

/// Context not provided error (E0302)
pub struct ContextNotProvidedError {
    context_name: Text,
    declaration_span: Span,
    call_site_span: Span,
}

impl ContextNotProvidedError {
    pub fn new(
        context_name: impl Into<Text>,
        declaration_span: Span,
        call_site_span: Span,
    ) -> Self {
        Self {
            context_name: context_name.into(),
            declaration_span,
            call_site_span,
        }
    }

    pub fn build(self) -> Diagnostic {
        DiagnosticBuilder::error()
            .code(error_codes::E0302)
            .message(format!(
                "context '{}' declared but not provided",
                self.context_name
            ))
            .span_label(
                self.call_site_span.clone(),
                "called here without providing context",
            )
            .secondary_span(
                self.declaration_span.clone(),
                format!("requires '{}' context", self.context_name),
            )
            .help(format!(
                "provide {} = create_{}() before calling this function",
                self.context_name,
                self.context_name.to_lowercase()
            ))
            .help("contexts must be explicitly provided using 'provide Context = implementation'")
            .add_note("see documentation: https://verum-lang.org/docs/contexts")
            .build()
    }
}

/// Context type mismatch error (E0303)
pub struct ContextTypeMismatchError {
    context_name: Text,
    expected_type: Text,
    actual_type: Text,
    span: Span,
}

impl ContextTypeMismatchError {
    pub fn new(
        context_name: impl Into<Text>,
        expected_type: impl Into<Text>,
        actual_type: impl Into<Text>,
        span: Span,
    ) -> Self {
        Self {
            context_name: context_name.into(),
            expected_type: expected_type.into(),
            actual_type: actual_type.into(),
            span,
        }
    }

    pub fn build(self) -> Diagnostic {
        DiagnosticBuilder::error()
            .code(error_codes::E0303)
            .message(format!("context '{}' type mismatch", self.context_name))
            .span_label(
                self.span.clone(),
                format!(
                    "expected type '{}', found '{}'",
                    self.expected_type, self.actual_type
                ),
            )
            .add_note(format!(
                "context '{}' expects interface '{}'",
                self.context_name, self.expected_type
            ))
            .help(format!(
                "ensure the provided implementation matches the '{}' interface",
                self.expected_type
            ))
            .build()
    }
}

/// Context group undefined error (E0306 - DEPRECATED, kept for backward compatibility)
pub struct ContextGroupUndefinedError {
    group_name: Text,
    usage_span: Span,
    available_groups: List<Text>,
}

impl ContextGroupUndefinedError {
    pub fn new(group_name: impl Into<Text>, usage_span: Span) -> Self {
        Self {
            group_name: group_name.into(),
            usage_span,
            available_groups: List::new(),
        }
    }

    pub fn with_available_groups(mut self, groups: List<Text>) -> Self {
        self.available_groups = groups;
        self
    }

    pub fn build(self) -> Diagnostic {
        let mut builder = DiagnosticBuilder::error()
            .code(error_codes::E0306_DEPRECATED)
            .message(format!(
                "context group '{}' is not defined",
                self.group_name
            ))
            .span_label(self.usage_span.clone(), "undefined context group")
            .help(format!(
                "define the context group: using {} = [Context1, Context2]",
                self.group_name
            ));

        if !self.available_groups.is_empty() {
            let mut note = Text::from("available context groups:\n");
            for group in &self.available_groups {
                note.push_str(&format!("  - {}\n", group));
            }
            builder = builder.add_note(note);
        }

        builder.build()
    }
}

/// Compute Levenshtein distance for "did you mean" suggestions
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: List<char> = a.chars().collect();
    let b_chars: List<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut matrix = vec![vec![0; b_len + 1]; a_len + 1];

    for i in 0..=a_len {
        matrix[i][0] = i;
    }
    for j in 0..=b_len {
        matrix[0][j] = j;
    }

    for i in 1..=a_len {
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            matrix[i][j] = (matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1)
                .min(matrix[i - 1][j - 1] + cost);
        }
    }

    matrix[a_len][b_len]
}

/// Find similar context names for "did you mean" suggestions
pub fn find_similar_contexts(target: &str, available: &[Text]) -> List<Text> {
    let mut candidates: List<(Text, usize)> = available
        .iter()
        .map(|ctx| {
            let distance = levenshtein_distance(&target.to_lowercase(), &ctx.to_lowercase());
            (ctx.clone(), distance)
        })
        .collect();

    // Sort by distance
    candidates.sort_by_key(|(_, dist)| *dist);

    // Return contexts within reasonable edit distance (≤3 for suggestions)
    candidates
        .into_iter()
        .filter(|(_, dist)| *dist <= 3)
        .take(5) // Limit to 5 suggestions
        .map(|(ctx, _)| ctx)
        .collect()
}

/// Format a suggestion for help text
fn format_suggestion(suggestion: &Suggestion) -> Text {
    let mut output: Text = suggestion.title().into();

    if let Some(desc) = suggestion.description() {
        output.push_str(&format!("\n  {}", desc));
    }

    if let Some(snippet) = suggestion.snippet() {
        output.push_str(&format!(
            "\n  Example:\n    {}",
            snippet.code.replace("\n", "\n    ")
        ));
    }

    output
}

// ============================================================================
// Async/Sync Context Mismatch Diagnostics
// ============================================================================

/// Error for using async context in sync function (E0803)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncContextInSyncFunction {
    /// The async context being used
    pub context_name: Text,
    /// Location where the async context is used
    pub usage_span: Span,
    /// The sync function where it's being used
    pub function_name: Text,
    /// Span of the function signature
    pub function_span: Span,
}

impl AsyncContextInSyncFunction {
    pub fn new(
        context_name: impl Into<Text>,
        usage_span: Span,
        function_name: impl Into<Text>,
        function_span: Span,
    ) -> Self {
        Self {
            context_name: context_name.into(),
            usage_span,
            function_name: function_name.into(),
            function_span,
        }
    }

    /// Build the diagnostic message
    pub fn build(&self) -> Diagnostic {
        DiagnosticBuilder::error()
            .code(error_codes::E0803)
            .message(format!(
                "async context '{}' cannot be used in synchronous function '{}'",
                self.context_name, self.function_name
            ))
            .span_label(self.usage_span.clone(), "async context used here")
            .span_label(
                self.function_span.clone(),
                "in this synchronous function",
            )
            .help(format!(
                "make the function async: `async fn {}(...)`",
                self.function_name
            ))
            .add_note(
                "Async contexts require an async runtime and can only be called \
                 from async functions. Consider making this function async, or \
                 use a synchronous alternative if available."
            )
            .build()
    }
}

/// Error for async context method called without await (E0804)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncContextMethodNotAwaited {
    /// The context containing the async method
    pub context_name: Text,
    /// The async method name
    pub method_name: Text,
    /// Location of the method call
    pub call_span: Span,
    /// Whether the calling function is async
    pub caller_is_async: bool,
}

impl AsyncContextMethodNotAwaited {
    pub fn new(
        context_name: impl Into<Text>,
        method_name: impl Into<Text>,
        call_span: Span,
        caller_is_async: bool,
    ) -> Self {
        Self {
            context_name: context_name.into(),
            method_name: method_name.into(),
            call_span,
            caller_is_async,
        }
    }

    /// Build the diagnostic message
    pub fn build(&self) -> Diagnostic {
        let mut builder = DiagnosticBuilder::error()
            .code(error_codes::E0804)
            .message(format!(
                "async context method '{}.{}' must be awaited",
                self.context_name, self.method_name
            ))
            .span_label(self.call_span.clone(), "async method call not awaited");

        if self.caller_is_async {
            builder = builder
                .help(format!(
                    "add `.await` to the call: `{}.{}(...).await`",
                    self.context_name, self.method_name
                ))
                .add_note(
                    "Async context methods return a Future that must be awaited \
                     to execute the actual operation."
                );
        } else {
            builder = builder
                .help("make the calling function async to use `.await`")
                .add_note(
                    "Async context methods can only be called from async functions. \
                     Either make this function async, or use a synchronous alternative."
                );
        }

        builder.build()
    }
}

/// Error for providing sync implementation for async context (E0804 variant)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncProviderForAsyncContext {
    /// The async context
    pub context_name: Text,
    /// Location of the provide statement
    pub provide_span: Span,
    /// Whether the context is async
    pub context_is_async: bool,
    /// Whether the provider is async
    pub provider_is_async: bool,
}

impl SyncProviderForAsyncContext {
    pub fn new(
        context_name: impl Into<Text>,
        provide_span: Span,
        context_is_async: bool,
        provider_is_async: bool,
    ) -> Self {
        Self {
            context_name: context_name.into(),
            provide_span,
            context_is_async,
            provider_is_async,
        }
    }

    /// Build the diagnostic message
    pub fn build(&self) -> Diagnostic {
        let (ctx_kind, prov_kind) = if self.context_is_async {
            ("async", "sync")
        } else {
            ("sync", "async")
        };

        DiagnosticBuilder::error()
            .code(error_codes::E0804)
            .message(format!(
                "async/sync mismatch: {} context '{}' cannot use {} provider",
                ctx_kind, self.context_name, prov_kind
            ))
            .span_label(
                self.provide_span.clone(),
                format!("{} provider for {} context", prov_kind, ctx_kind),
            )
            .help(if self.context_is_async {
                format!("provide an async implementation for context '{}'", self.context_name)
            } else {
                format!("provide a sync implementation for context '{}'", self.context_name)
            })
            .add_note(
                "Context providers must match the async/sync nature of the context. \
                 Async contexts require async providers, and sync contexts require sync providers."
            )
            .build()
    }
}
