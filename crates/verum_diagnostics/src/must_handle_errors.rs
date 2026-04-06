//! E0317: Unused Result that must be used
//!
//! The @must_handle annotation provides compile-time enforcement that Result<T, E> values
//! with critical error types are explicitly handled before being dropped. Unlike @must_use
//! (which is a warning on function return values), @must_handle is a compile ERROR that
//! applies to the error TYPE itself -- once a type is marked @must_handle, ALL functions
//! returning Result<T, ThatType> inherit the enforcement.
//!
//! This module provides comprehensive diagnostics for @must_handle annotation violations.
//! When a Result<T, E> has an error type E marked with @must_handle, the compiler enforces
//! that the Result is explicitly handled before being dropped.
//!
//! # Error Code E0317
//!
//! **Severity**: Error (compile-time failure)
//!
//! **Trigger**: A Result<T, E> value where E is marked with @must_handle is dropped
//! without being explicitly handled through one of the allowed operations:
//! - `?` operator (error propagation)
//! - `.unwrap()` (panic on error)
//! - `.expect(msg)` (panic with message)
//! - `match` expression (pattern matching)
//! - `if let Ok/Err` (conditional handling)
//! - `.is_err()` check before drop
//!
//! # Examples
//!
//! ## Example 1: Basic violation
//!
//! ```verum
//! @must_handle
//! type CriticalError is | ConnectionLost | DataCorruption;
//!
//! fn risky() -> Result<Data, CriticalError> { ... }
//!
//! fn bad() {
//!     let result = risky();  // ❌ E0317
//! }
//! ```
//!
//! **Error message**:
//! ```text
//! error[E0317]: unused Result that must be used
//!   --> example.vr:5:5
//!    |
//! 5  |     let result = risky();
//!    |     ^^^^^^^^^^^^^^^^^^^^^ Result with @must_handle error type dropped without handling
//!    |
//!    = note: error type `CriticalError` is marked with @must_handle
//!    = note: this Result must be handled before being dropped
//!    = help: use `risky()?` to propagate the error
//!    = help: use `risky().unwrap()` to panic if error occurs
//!    = help: use `match risky() { Ok(x) => ..., Err(e) => ... }` to handle both cases
//! ```
//!
//! ## Example 2: Wildcard pattern
//!
//! ```verum
//! fn bad() {
//!     let _ = risky();  // ❌ E0317
//! }
//! ```
//!
//! **Error message**:
//! ```text
//! error[E0317]: unused Result that must be used
//!   --> example.vr:2:5
//!    |
//! 2  |     let _ = risky();
//!    |     ^^^^^^^^^^^^^^^^ Result with @must_handle error type intentionally ignored
//!    |
//!    = note: error type `CriticalError` is marked with @must_handle
//!    = note: wildcard pattern `_` explicitly ignores the Result
//!    = help: @must_handle errors represent critical failures that should never be ignored
//!    = help: use `?` to propagate: let data = risky()?;
//!    = help: use pattern matching to handle both cases
//! ```
//!
//! ## Example 3: Binding without use
//!
//! ```verum
//! fn bad() {
//!     let result = risky();
//!     // result never used
//! }  // ❌ E0317
//! ```
//!
//! **Error message**:
//! ```text
//! error[E0317]: unused Result that must be used
//!   --> example.vr:2:5
//!    |
//! 2  |     let result = risky();
//!    |     ^^^^^^^^^^^^^^^^^^^^^ Result created here
//! 3 |     // result never used
//! 4 | }
//!   | - Result dropped here without being handled
//!    |
//!    = note: error type `CriticalError` is marked with @must_handle
//!    = note: variable `result` bound but never checked or handled
//!    = help: add `result?` to propagate the error
//!    = help: or use `result.unwrap()` to panic on error
//!    = help: or match on the Result to handle both Ok and Err cases
//! ```
//!
//! ## Example 4: Conditional branches (partial handling)
//!
//! ```verum
//! fn bad(flag: bool) {
//!     let result = risky();
//!     if flag {
//!         result.unwrap();  // Handled here
//!     }
//!     // Not handled in else branch
//! }  // ❌ E0317
//! ```
//!
//! **Error message**:
//! ```text
//! error[E0317]: unused Result that must be used in some code paths
//!   --> example.vr:2:5
//!    |
//! 2  |     let result = risky();
//!    |     ^^^^^^^^^^^^^^^^^^^^^ Result created here
//! 3  |     if flag {
//! 4  |         result.unwrap();
//!    |         --------------- handled in this branch
//! 5  |     }
//!    |     - not handled when condition is false
//! 6  | }
//!   | - Result may be dropped without handling
//!    |
//!    = note: error type `CriticalError` is marked with @must_handle
//!    = note: Result must be handled in ALL control flow paths
//!    = help: handle in both branches:
//!           if flag {
//!               result.unwrap();
//!           } else {
//!               result.unwrap();  // or handle differently
//!           }
//! ```

use crate::diagnostic::{Diagnostic, DiagnosticBuilder, Label, Severity};
use crate::suggestion::Suggestion;
use std::collections::HashSet;
use verum_ast::span::Span;
use verum_common::span::LineColSpan;
use verum_common::{List, Map, Text};

/// Helper to convert Span to LineColSpan for diagnostics
/// Uses dummy file path since we don't have source file info here
fn span_to_linecol(span: Span) -> LineColSpan {
    LineColSpan::new(
        format!("file_{}", span.file_id.raw()),
        span.start as usize, // Use byte offset as line for now
        0,                   // Column
        span.end as usize,   // End as column
    )
}

/// E0317: Unused Result that must be used
///
/// This error is emitted when a Result<T, E> with a @must_handle error type
/// is dropped without being explicitly handled.
#[derive(Debug, Clone)]
pub struct E0317 {
    /// Span where the Result was created
    pub creation_span: Span,

    /// Optional span where the Result was dropped (if different from creation)
    pub drop_span: Option<Span>,

    /// Variable name (if bound to a variable)
    pub var_name: Option<Text>,

    /// Error type name (E in Result<T, E>)
    pub error_type_name: Text,

    /// Expression that created the Result (for suggestions)
    pub creation_expr: Text,

    /// Violation kind
    pub kind: ViolationKind,

    /// Control flow context (for partial handling)
    pub flow_context: Option<FlowContext>,
}

/// Kind of @must_handle violation
#[derive(Debug, Clone, PartialEq)]
pub enum ViolationKind {
    /// Result bound to variable but never handled
    UnusedBinding,

    /// Result explicitly ignored with wildcard pattern `let _ = ...`
    WildcardIgnored,

    /// Result dropped without binding
    DirectDrop,

    /// Result handled in some branches but not all
    PartialHandling,
}

/// Control flow context for partial handling errors
#[derive(Debug, Clone)]
pub struct FlowContext {
    /// Branches where Result was handled
    pub handled_branches: List<BranchInfo>,

    /// Branches where Result was not handled
    pub unhandled_branches: List<BranchInfo>,
}

impl FlowContext {
    /// Create a new empty flow context
    pub fn new() -> Self {
        Self {
            handled_branches: List::new(),
            unhandled_branches: List::new(),
        }
    }

    /// Add a handled branch
    pub fn add_handled(&mut self, branch: BranchInfo) {
        self.handled_branches.push(branch);
    }

    /// Add an unhandled branch
    pub fn add_unhandled(&mut self, branch: BranchInfo) {
        self.unhandled_branches.push(branch);
    }

    /// Check if all branches are handled
    pub fn is_fully_handled(&self) -> bool {
        self.unhandled_branches.is_empty()
    }

    /// Get percentage of branches that are handled
    pub fn handled_percentage(&self) -> u8 {
        let total = self.handled_branches.len() + self.unhandled_branches.len();
        if total == 0 {
            return 100;
        }
        ((self.handled_branches.len() * 100) / total) as u8
    }

    /// Get a summary description of the flow context
    pub fn summary(&self) -> Text {
        if self.is_fully_handled() {
            return "Result handled in all branches".into();
        }

        let handled_count = self.handled_branches.len();
        let unhandled_count = self.unhandled_branches.len();
        let total = handled_count + unhandled_count;

        format!(
            "Result handled in {}/{} branches ({}%)",
            handled_count,
            total,
            self.handled_percentage()
        )
        .into()
    }
}

impl Default for FlowContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Information about a control flow branch
#[derive(Debug, Clone)]
pub struct BranchInfo {
    /// Description of the branch (e.g., "then branch", "else branch", "match arm 1")
    pub description: Text,

    /// Span of the branch
    pub span: Span,

    /// How the Result was handled (if at all)
    pub handling: Option<Text>,
}

impl BranchInfo {
    /// Create a new handled branch
    pub fn handled(description: impl Into<Text>, span: Span, handling: impl Into<Text>) -> Self {
        Self {
            description: description.into(),
            span,
            handling: Some(handling.into()),
        }
    }

    /// Create a new unhandled branch
    pub fn unhandled(description: impl Into<Text>, span: Span) -> Self {
        Self {
            description: description.into(),
            span,
            handling: None,
        }
    }

    /// Check if this branch handles the Result
    pub fn is_handled(&self) -> bool {
        self.handling.is_some()
    }
}

/// Tracker for @must_handle Results throughout compilation.
///
/// This tracker maintains state about which Result values have @must_handle
/// error types and whether they have been properly handled. It is used during
/// type checking and code generation to ensure compliance with the @must_handle
/// annotation.
#[derive(Debug, Clone)]
pub struct MustHandleTracker {
    /// Active Results that need to be handled (keyed by unique ID)
    active_results: Map<u64, TrackedResult>,
    /// Results that have been properly handled
    handled_results: HashSet<u64>,
    /// Counter for generating unique IDs
    next_id: u64,
    /// Stack of scope IDs for tracking nested scopes
    scope_stack: List<u64>,
    /// Errors detected during tracking
    pub errors: List<E0317>,
}

/// A tracked Result value with @must_handle error type
#[derive(Debug, Clone)]
pub struct TrackedResult {
    /// Unique identifier for this Result
    pub id: u64,
    /// Span where the Result was created
    pub creation_span: Span,
    /// Variable name if bound
    pub var_name: Option<Text>,
    /// Error type name
    pub error_type: Text,
    /// Expression that created the Result
    pub creation_expr: Text,
    /// Scope where the Result was created
    pub scope_id: u64,
    /// How this Result has been handled (if at all)
    pub handling: ResultHandling,
}

/// How a @must_handle Result has been handled
#[derive(Debug, Clone, PartialEq)]
pub enum ResultHandling {
    /// Not yet handled
    NotHandled,
    /// Handled via ? operator
    Propagated,
    /// Handled via match expression
    Matched,
    /// Handled via if let
    IfLet,
    /// Handled via .unwrap() or .expect()
    Unwrapped,
    /// Handled via .is_err() check
    ErrorChecked,
    /// Explicitly ignored with wildcard (error!)
    WildcardIgnored,
}

impl MustHandleTracker {
    /// Create a new tracker
    pub fn new() -> Self {
        Self {
            active_results: Map::new(),
            handled_results: HashSet::new(),
            next_id: 0,
            scope_stack: vec![0].into(),
            errors: List::new(),
        }
    }

    /// Enter a new scope
    pub fn enter_scope(&mut self) -> u64 {
        self.next_id += 1;
        let scope_id = self.next_id;
        self.scope_stack.push(scope_id);
        scope_id
    }

    /// Exit the current scope, checking for unhandled Results
    pub fn exit_scope(&mut self) {
        let scope_id = self.scope_stack.pop().unwrap_or(0);

        // Find all unhandled Results in this scope
        let unhandled: List<TrackedResult> = self
            .active_results
            .values()
            .filter(|r| r.scope_id == scope_id && !self.handled_results.contains(&r.id))
            .cloned()
            .collect();

        // Generate errors for unhandled Results
        for result in unhandled {
            match result.handling {
                ResultHandling::NotHandled => {
                    if let Some(var_name) = &result.var_name {
                        self.errors.push(E0317::unused_binding(
                            result.creation_span,
                            var_name.clone(),
                            result.error_type.clone(),
                            result.creation_expr.clone(),
                        ));
                    } else {
                        self.errors.push(E0317::direct_drop(
                            result.creation_span,
                            result.error_type.clone(),
                            result.creation_expr.clone(),
                        ));
                    }
                }
                ResultHandling::WildcardIgnored => {
                    self.errors.push(E0317::wildcard_ignored(
                        result.creation_span,
                        result.error_type.clone(),
                        result.creation_expr.clone(),
                    ));
                }
                _ => {
                    // Handled in some way, not an error
                }
            }

            // Remove from active tracking
            self.active_results.remove(&result.id);
        }
    }

    /// Register a new @must_handle Result
    pub fn register_result(
        &mut self,
        creation_span: Span,
        var_name: Option<Text>,
        error_type: Text,
        creation_expr: Text,
    ) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let scope_id = *self.scope_stack.last().unwrap_or(&0);

        let tracked = TrackedResult {
            id,
            creation_span,
            var_name,
            error_type,
            creation_expr,
            scope_id,
            handling: ResultHandling::NotHandled,
        };

        self.active_results.insert(id, tracked);
        id
    }

    /// Mark a Result as handled via a specific method
    pub fn mark_handled(&mut self, id: u64, handling: ResultHandling) {
        if let Some(result) = self.active_results.get_mut(&id) {
            result.handling = handling.clone();
            if handling != ResultHandling::WildcardIgnored {
                self.handled_results.insert(id);
            }
        }
    }

    /// Mark a Result as handled by variable name
    pub fn mark_handled_by_name(&mut self, var_name: &str, handling: ResultHandling) {
        let ids: List<u64> = self
            .active_results
            .values()
            .filter(|r| r.var_name.as_deref() == Some(var_name))
            .map(|r| r.id)
            .collect();

        for id in ids {
            self.mark_handled(id, handling.clone());
        }
    }

    /// Check if a Result is currently being tracked
    pub fn is_tracked(&self, id: u64) -> bool {
        self.active_results.contains_key(&id)
    }

    /// Get all currently unhandled Results
    pub fn get_unhandled(&self) -> List<&TrackedResult> {
        self.active_results
            .values()
            .filter(|r| !self.handled_results.contains(&r.id))
            .collect()
    }

    /// Get all errors detected during tracking
    pub fn get_errors(&self) -> &[E0317] {
        &self.errors
    }

    /// Convert all errors to diagnostics
    pub fn to_diagnostics(&self) -> List<Diagnostic> {
        self.errors.iter().map(|e| e.to_diagnostic()).collect()
    }

    /// Clear all state
    pub fn clear(&mut self) {
        self.active_results.clear();
        self.handled_results.clear();
        self.errors.clear();
        self.scope_stack = vec![0].into();
        self.next_id = 0;
    }
}

impl Default for MustHandleTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl E0317 {
    /// Create a new E0317 error for an unused binding
    pub fn unused_binding(
        creation_span: Span,
        var_name: Text,
        error_type_name: Text,
        creation_expr: Text,
    ) -> Self {
        Self {
            creation_span,
            drop_span: None,
            var_name: Some(var_name),
            error_type_name,
            creation_expr,
            kind: ViolationKind::UnusedBinding,
            flow_context: None,
        }
    }

    /// Create a new E0317 error for wildcard ignored
    pub fn wildcard_ignored(
        creation_span: Span,
        error_type_name: Text,
        creation_expr: Text,
    ) -> Self {
        Self {
            creation_span,
            drop_span: None,
            var_name: None,
            error_type_name,
            creation_expr,
            kind: ViolationKind::WildcardIgnored,
            flow_context: None,
        }
    }

    /// Create a new E0317 error for direct drop
    pub fn direct_drop(creation_span: Span, error_type_name: Text, creation_expr: Text) -> Self {
        Self {
            creation_span,
            drop_span: None,
            var_name: None,
            error_type_name,
            creation_expr,
            kind: ViolationKind::DirectDrop,
            flow_context: None,
        }
    }

    /// Create a new E0317 error for partial handling
    pub fn partial_handling(
        creation_span: Span,
        var_name: Option<Text>,
        error_type_name: Text,
        creation_expr: Text,
        flow_context: FlowContext,
    ) -> Self {
        Self {
            creation_span,
            drop_span: None,
            var_name,
            error_type_name,
            creation_expr,
            kind: ViolationKind::PartialHandling,
            flow_context: Some(flow_context),
        }
    }

    /// Convert to a Diagnostic for emission
    pub fn to_diagnostic(&self) -> Diagnostic {
        // DiagnosticBuilder::error() already sets severity to Error
        let mut builder = DiagnosticBuilder::error().code("E0317");

        // Primary message
        let primary_msg = match &self.kind {
            ViolationKind::UnusedBinding => "unused Result that must be used".to_string(),
            ViolationKind::WildcardIgnored => {
                "Result with @must_handle error type intentionally ignored".to_string()
            }
            ViolationKind::DirectDrop => {
                "Result with @must_handle error type dropped without handling".to_string()
            }
            ViolationKind::PartialHandling => {
                "unused Result that must be used in some code paths".to_string()
            }
        };

        builder = builder.message(primary_msg);

        // Primary label
        let primary_label = match &self.kind {
            ViolationKind::UnusedBinding => {
                if let Some(var_name) = &self.var_name {
                    format!("Result bound to `{}` but never handled", var_name)
                } else {
                    "Result created but never handled".to_string()
                }
            }
            ViolationKind::WildcardIgnored => {
                "Result with @must_handle error type intentionally ignored".to_string()
            }
            ViolationKind::DirectDrop => {
                "Result with @must_handle error type dropped here".to_string()
            }
            ViolationKind::PartialHandling => "Result created here".to_string(),
        };

        builder = builder.span_label(span_to_linecol(self.creation_span), primary_label);

        // Add drop label if different from creation
        if let Some(drop_span) = self.drop_span {
            builder = builder.secondary_span(
                span_to_linecol(drop_span),
                "Result dropped here without handling",
            );
        }

        // Add flow context labels for partial handling
        if let Some(flow_ctx) = &self.flow_context {
            for branch in flow_ctx.handled_branches.iter() {
                let label = if let Some(handling) = &branch.handling {
                    format!("handled in {}: {}", branch.description, handling)
                } else {
                    format!("handled in {}", branch.description)
                };
                builder = builder.secondary_span(span_to_linecol(branch.span), label);
            }

            for branch in flow_ctx.unhandled_branches.iter() {
                let label = format!("not handled in {}", branch.description);
                builder = builder.secondary_span(span_to_linecol(branch.span), label);
            }
        }

        // Notes
        builder = builder.add_note(format!(
            "error type `{}` is marked with @must_handle",
            self.error_type_name
        ));

        match &self.kind {
            ViolationKind::UnusedBinding => {
                if let Some(var_name) = &self.var_name {
                    builder = builder.add_note(format!(
                        "variable `{}` bound but never checked or handled",
                        var_name
                    ));
                }
                builder = builder.add_note("this Result must be handled before being dropped");
            }
            ViolationKind::WildcardIgnored => {
                builder = builder.add_note("wildcard pattern `_` explicitly ignores the Result");
                builder = builder.add_note(
                    "@must_handle errors represent critical failures that should never be ignored",
                );
            }
            ViolationKind::DirectDrop => {
                builder = builder.add_note("this Result must be handled before being dropped");
            }
            ViolationKind::PartialHandling => {
                builder = builder.add_note("Result must be handled in ALL control flow paths");
            }
        }

        // Suggestions
        builder = self.add_suggestions(builder);

        builder.build()
    }

    /// Add contextual suggestions based on violation kind
    fn add_suggestions(&self, mut builder: DiagnosticBuilder) -> DiagnosticBuilder {
        match &self.kind {
            ViolationKind::UnusedBinding | ViolationKind::DirectDrop => {
                // Suggest ? operator
                builder = builder.help(format!(
                    "use `{}?` to propagate the error",
                    self.creation_expr
                ));

                // Suggest unwrap
                builder = builder.help(format!(
                    "use `{}.unwrap()` to panic if error occurs",
                    self.creation_expr
                ));

                // Suggest match
                builder = builder.help(format!(
                    "use `match {} {{ Ok(x) => ..., Err(e) => ... }}` to handle both cases",
                    self.creation_expr
                ));

                // Suggest is_err check if binding exists
                if let Some(var_name) = &self.var_name {
                    builder = builder.help(format!(
                        "check with `if {}.is_err() {{ ... }}` before dropping",
                        var_name
                    ));
                }
            }

            ViolationKind::WildcardIgnored => {
                // Suggest ? operator
                builder = builder.help(format!(
                    "use `?` to propagate: let data = {}?;",
                    self.creation_expr
                ));

                // Suggest pattern matching
                builder = builder.help("use pattern matching to handle both cases");

                // Suggest explicit handling
                builder = builder.help(format!(
                    "or use `{}.unwrap()` to explicitly panic on error",
                    self.creation_expr
                ));
            }

            ViolationKind::PartialHandling => {
                // Suggest handling in all branches
                builder = builder.help("handle the Result in all control flow paths");

                if let Some(flow_ctx) = &self.flow_context
                    && !flow_ctx.unhandled_branches.is_empty()
                {
                    builder = builder.help(format!(
                        "add handling in: {}",
                        flow_ctx
                            .unhandled_branches
                            .iter()
                            .map(|b| b.description.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }

        builder
    }

    /// Create suggestion for fixing the error
    pub fn create_fix_suggestions(&self) -> List<Suggestion> {
        let mut suggestions = Vec::new();

        match &self.kind {
            ViolationKind::UnusedBinding => {
                // Suggest adding ?
                if let Some(var_name) = &self.var_name {
                    use crate::suggestion::{Applicability, CodeSnippet};

                    suggestions.push(Suggestion {
                        title: "add `?` to propagate the error".into(),
                        description: Some(
                            "Use the `?` operator to propagate errors up the call stack"
                                .into(),
                        ),
                        snippet: Some(CodeSnippet::with_span(
                            format!("let {} = {}?", var_name, self.creation_expr),
                            span_to_linecol(self.creation_span),
                        )),
                        applicability: Applicability::Recommended,
                        snippets: List::new(),
                    });

                    // Suggest match expression
                    suggestions.push(Suggestion {
                        title: "use match to handle both cases".into(),
                        description: Some("Pattern match on the Result to handle Ok and Err cases explicitly".into()),
                        snippet: Some(CodeSnippet::with_span(
                            format!(
                                "match {} {{\n    Ok({}) => {{ /* use {} */ }},\n    Err(e) => {{ /* handle error */ }},\n}}",
                                self.creation_expr,
                                var_name,
                                var_name
                            ),
                            span_to_linecol(self.creation_span)
                        )),
                        applicability: Applicability::Alternative,
                        snippets: List::new(),
                    });
                }
            }

            ViolationKind::WildcardIgnored => {
                use crate::suggestion::{Applicability, CodeSnippet};

                // Suggest replacing _ with explicit handling
                suggestions.push(Suggestion {
                    title: "use `?` to propagate instead of ignoring".into(),
                    description: Some("Replace the wildcard pattern with error propagation".into()),
                    snippet: Some(CodeSnippet::with_span(
                        format!("let _result = {}?", self.creation_expr),
                        span_to_linecol(self.creation_span),
                    )),
                    applicability: Applicability::Recommended,
                    snippets: List::new(),
                });
            }

            ViolationKind::DirectDrop => {
                use crate::suggestion::{Applicability, CodeSnippet};

                // Suggest match expression
                suggestions.push(Suggestion {
                    title: "use match to handle the Result".into(),
                    description: Some("Pattern match on the Result to handle both cases".into()),
                    snippet: Some(CodeSnippet::with_span(
                        format!(
                            "match {} {{\n    Ok(data) => {{ /* use data */ }},\n    Err(e) => {{ /* handle error */ }},\n}}",
                            self.creation_expr
                        ),
                        span_to_linecol(self.creation_span)
                    )),
                    applicability: Applicability::Recommended,
                    snippets: List::new(),
                });
            }

            ViolationKind::PartialHandling => {
                use crate::suggestion::{Applicability, CodeSnippet};

                // Suggest handling in all branches
                suggestions.push(Suggestion {
                    title: "ensure Result is handled in all branches".into(),
                    description: Some("Add handling in all control flow paths".into()),
                    snippet: Some(CodeSnippet::with_span(
                        "// Add handling in all control flow paths",
                        span_to_linecol(self.creation_span),
                    )),
                    applicability: Applicability::HasPlaceholders,
                    snippets: List::new(),
                });
            }
        }

        suggestions.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::span::FileId;

    fn make_span(line: usize, col: usize) -> Span {
        // Create a byte-offset span for testing
        // Using arbitrary offsets that correspond to the line/col positions
        let start = (line * 100 + col) as u32;
        let end = start + 10;
        Span::new(start, end, FileId::dummy())
    }

    #[test]
    fn test_unused_binding_error() {
        let error = E0317::unused_binding(
            make_span(5, 5),
            "result".into(),
            "CriticalError".into(),
            "risky()".into(),
        );

        let diagnostic = error.to_diagnostic();
        assert!(diagnostic.message().contains("unused Result"));
        assert_eq!(diagnostic.severity(), Severity::Error);
    }

    #[test]
    fn test_wildcard_ignored_error() {
        let error =
            E0317::wildcard_ignored(make_span(2, 5), "CriticalError".into(), "risky()".into());

        let diagnostic = error.to_diagnostic();
        assert!(diagnostic.message().contains("intentionally ignored"));
    }

    #[test]
    fn test_partial_handling_error() {
        let flow_context = FlowContext {
            handled_branches: vec![BranchInfo {
                description: "then branch".into(),
                span: make_span(3, 5),
                handling: Some("unwrap()".into()),
            }]
            .into(),
            unhandled_branches: vec![BranchInfo {
                description: "else branch".into(),
                span: make_span(5, 5),
                handling: None,
            }]
            .into(),
        };

        let error = E0317::partial_handling(
            make_span(2, 5),
            Some("result".into()),
            "CriticalError".into(),
            "risky()".into(),
            flow_context,
        );

        let diagnostic = error.to_diagnostic();
        assert!(diagnostic.message().contains("some code paths"));
    }

    #[test]
    fn test_fix_suggestions() {
        let error = E0317::unused_binding(
            make_span(5, 5),
            "result".into(),
            "CriticalError".into(),
            "risky()".into(),
        );

        let suggestions = error.create_fix_suggestions();
        assert!(!suggestions.is_empty());
    }
}
