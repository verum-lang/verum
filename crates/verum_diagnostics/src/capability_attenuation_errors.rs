//! Capability Attenuation Error Diagnostics
//!
//! This module provides comprehensive diagnostics for capability attenuation errors in Verum.
//! Capability attenuation is a security mechanism that restricts the capabilities a function
//! can use, ensuring that code can only access the specific sub-contexts it declares.
//!
//! # Error Codes
//!
//! - **E0306**: Capability Violation - Function uses a capability not declared in `using` clause
//! - **E0307**: Sub-Context Not Found - Reference to undefined sub-context in context hierarchy
//! - **E0308**: Capability Not Provided - Required capability not available in environment
//! - **E0309**: Partial Implementation Warning - Context implementation missing some sub-contexts
//!
//! # Design Philosophy
//!
//! These diagnostics emphasize:
//! 1. **Security awareness** - Clear explanation of capability violations
//! 2. **Precise locations** - Show exactly where capabilities are declared vs. used
//! 3. **Actionable fixes** - Multiple suggestions for resolving capability issues
//! 4. **Capability hierarchy** - Visual representation of context/sub-context relationships
//!
//! # Example: E0306 Capability Violation
//!
//! ```verum
//! fn attempt_delete(id: Int) -> Result<()>
//!     using [Database::Query]
//! {
//!     db.execute(f"DELETE FROM users WHERE id = {id}")?;  // ❌ E0306
//! }
//! ```
//!
//! The function declares `Database::Query` but attempts to use `Database::Execute`,
//! which is a capability violation.

use crate::{
    Diagnostic, DiagnosticBuilder, Severity, Span,
    suggestion::{Applicability, CodeSnippet, Suggestion, SuggestionBuilder},
};
use verum_common::{List, Text};

/// Error codes for capability attenuation
pub mod error_codes {
    /// Capability violation - uses undeclared capability
    pub const E0306: &str = "E0306";
    /// Sub-context not found in context hierarchy
    pub const E0307: &str = "E0307";
    /// Required capability not provided in environment
    pub const E0308: &str = "E0308";
    /// Partial implementation of context (warning)
    pub const E0309: &str = "E0309";
}

/// E0306: Capability Violation Error Builder
///
/// This error occurs when a function attempts to use a capability that was not
/// declared in its `using` clause. This is a security violation as it breaks
/// the capability attenuation contract.
///
/// # Example
///
/// ```verum
/// fn process_data() -> Result<()>
///     using [Database::Query]
/// {
///     db.execute("DELETE FROM logs")?;  // E0306: Execute not in using clause
/// }
/// ```
pub struct CapabilityViolationError {
    /// The capability being used (e.g., "Database::Execute")
    used_capability: Text,
    /// The span where the capability is being used
    usage_span: Span,
    /// The capabilities declared in the function's using clause
    declared_capabilities: List<Text>,
    /// The span of the function's using clause
    declaration_span: Option<Span>,
    /// The span of the function signature
    function_span: Option<Span>,
    /// The name of the function
    function_name: Option<Text>,
}

impl CapabilityViolationError {
    /// Create a new capability violation error
    pub fn new(used_capability: impl Into<Text>, usage_span: Span) -> Self {
        Self {
            used_capability: used_capability.into(),
            usage_span,
            declared_capabilities: List::new(),
            declaration_span: None,
            function_span: None,
            function_name: None,
        }
    }

    /// Set the declared capabilities
    pub fn with_declared_capabilities(mut self, capabilities: List<Text>) -> Self {
        self.declared_capabilities = capabilities;
        self
    }

    /// Set the declaration span (where using clause appears)
    pub fn with_declaration_span(mut self, span: Span) -> Self {
        self.declaration_span = Some(span);
        self
    }

    /// Set the function span
    pub fn with_function_span(mut self, span: Span) -> Self {
        self.function_span = Some(span);
        self
    }

    /// Set the function name
    pub fn with_function_name(mut self, name: impl Into<Text>) -> Self {
        self.function_name = Some(name.into());
        self
    }

    /// Build the diagnostic
    pub fn build(self) -> Diagnostic {
        let mut builder = DiagnosticBuilder::error()
            .code(error_codes::E0306)
            .message(format!(
                "capability `{}` not declared in [using] clause",
                self.used_capability
            ))
            .span_label(self.usage_span.clone(), "capability violation");

        // Add secondary label for using clause
        if let Some(decl_span) = self.declaration_span {
            let cap_list = if self.declared_capabilities.is_empty() {
                "no capabilities declared".to_string()
            } else {
                format!("{} not in capability list", self.used_capability)
            };
            builder = builder.secondary_span(decl_span, cap_list);
        }

        // Add secondary label for function signature
        if let Some(func_span) = self.function_span {
            let cap_list = if self.declared_capabilities.is_empty() {
                "function requires no capabilities".to_string()
            } else {
                format!(
                    "function requires [{}]",
                    self.declared_capabilities.join(", ")
                )
            };
            builder = builder.secondary_span(func_span, cap_list);
        }

        // Add explanatory notes
        builder = builder
            .add_note({
                let caps = if self.declared_capabilities.is_empty() {
                    "<none>".into()
                } else {
                    self.declared_capabilities.join(", ")
                };
                format!("function signature declares: using [{}]", caps)
            })
            .add_note(format!(
                "this code attempts to use: {}",
                self.used_capability
            ));

        // Add actionable help
        builder = builder.help(format!(
            "add {} to the [using] clause",
            self.used_capability
        ));

        // Suggest creating a context group if many capabilities
        if self.declared_capabilities.len() >= 2 {
            let mut all_caps = self.declared_capabilities.clone();
            all_caps.push(self.used_capability.clone());
            all_caps.sort();
            all_caps.dedup();

            builder = builder.help(format!(
                "or create a context group:\n\
                \n\
                using DatabaseOps = [{}];\n\
                \n\
                fn {}(...) -> ReturnType\n\
                    using DatabaseOps",
                all_caps.join(", "),
                self.function_name.as_deref().unwrap_or("function")
            ));
        }

        // Add security note
        builder = builder
            .add_note(
                "capability attenuation ensures functions only use declared capabilities for security",
            )
            .add_note("capability attenuation restricts functions to only the sub-context capabilities declared in their 'using' clause, enforcing least-privilege security; undeclared capabilities are compile-time errors (E0306)");

        builder.build()
    }
}

/// E0307: Sub-Context Not Found Error Builder
///
/// This error occurs when referencing a sub-context that doesn't exist in the
/// context hierarchy. For example, `Database.Read` when `Database` only defines
/// `Query` and `Execute` sub-contexts.
///
/// # Example
///
/// ```verum
/// using [Database.Read]  // E0307: Read not found in Database
/// ```
pub struct SubContextNotFoundError {
    /// The parent context name (e.g., "Database")
    context_name: Text,
    /// The sub-context that was not found (e.g., "Read")
    sub_context_name: Text,
    /// The span of the invalid sub-context reference
    span: Span,
    /// Available sub-contexts in the parent context
    available_sub_contexts: List<Text>,
    /// The span where the context is defined (if available)
    context_definition_span: Option<Span>,
}

impl SubContextNotFoundError {
    /// Create a new sub-context not found error
    pub fn new(
        context_name: impl Into<Text>,
        sub_context_name: impl Into<Text>,
        span: Span,
    ) -> Self {
        Self {
            context_name: context_name.into(),
            sub_context_name: sub_context_name.into(),
            span,
            available_sub_contexts: List::new(),
            context_definition_span: None,
        }
    }

    /// Set the available sub-contexts
    pub fn with_available_sub_contexts(mut self, sub_contexts: List<Text>) -> Self {
        self.available_sub_contexts = sub_contexts;
        self
    }

    /// Set the context definition span
    pub fn with_context_definition_span(mut self, span: Span) -> Self {
        self.context_definition_span = Some(span);
        self
    }

    /// Build the diagnostic
    pub fn build(self) -> Diagnostic {
        let mut builder = DiagnosticBuilder::error()
            .code(error_codes::E0307)
            .message(format!(
                "sub-context `{}` not found in context `{}`",
                self.sub_context_name, self.context_name
            ))
            .span_label(self.span.clone(), "unknown sub-context");

        // Add context definition location if available
        if let Some(def_span) = self.context_definition_span {
            builder = builder.secondary_span(
                def_span,
                format!("context `{}` defined here", self.context_name),
            );
        }

        // List available sub-contexts
        if !self.available_sub_contexts.is_empty() {
            let mut note = format!(
                "context `{}` defines these sub-contexts:\n",
                self.context_name
            );
            for sub_ctx in &self.available_sub_contexts {
                note.push_str(&format!("        - {}\n", sub_ctx));
            }
            builder = builder.add_note(note.trim_end());
        } else {
            builder = builder.add_note(format!(
                "context `{}` does not define any sub-contexts",
                self.context_name
            ));
        }

        // Add help
        builder = builder.help("check the context definition or use a valid sub-context");

        // Suggest similar sub-contexts if available
        if !self.available_sub_contexts.is_empty() {
            let similar = find_similar_names(&self.sub_context_name, &self.available_sub_contexts);
            if !similar.is_empty() {
                let mut help = Text::from("did you mean one of these?\n");
                for (i, name) in similar.iter().enumerate() {
                    help.push_str(&format!(
                        "        {}. {}::{}\n",
                        i + 1,
                        self.context_name,
                        name
                    ));
                }
                builder = builder.help(help.trim_end());
            }
        }

        builder.build()
    }
}

/// E0308: Capability Not Provided Error Builder
///
/// This error occurs when a function requires a capability that has not been
/// provided in the environment. The capability is declared in the `using` clause
/// but no provider has been installed.
///
/// # Example
///
/// ```verum
/// fn main() {
///     let data = read_file("data.txt")?;  // E0308: FileSystem::Read not provided
/// }
/// ```
pub struct CapabilityNotProvidedError {
    /// The required capability (e.g., "FileSystem::Read")
    required_capability: Text,
    /// The span where the capability is required
    usage_span: Span,
    /// The function requiring the capability
    function_name: Option<Text>,
    /// The span of the function declaration
    function_declaration_span: Option<Span>,
}

impl CapabilityNotProvidedError {
    /// Create a new capability not provided error
    pub fn new(required_capability: impl Into<Text>, usage_span: Span) -> Self {
        Self {
            required_capability: required_capability.into(),
            usage_span,
            function_name: None,
            function_declaration_span: None,
        }
    }

    /// Set the function name
    pub fn with_function_name(mut self, name: impl Into<Text>) -> Self {
        self.function_name = Some(name.into());
        self
    }

    /// Set the function declaration span
    pub fn with_function_declaration_span(mut self, span: Span) -> Self {
        self.function_declaration_span = Some(span);
        self
    }

    /// Build the diagnostic
    pub fn build(self) -> Diagnostic {
        let mut builder = DiagnosticBuilder::error()
            .code(error_codes::E0308)
            .message(format!(
                "capability `{}` required but not provided",
                self.required_capability
            ))
            .span_label(
                self.usage_span.clone(),
                "requires capability not in environment",
            );

        // Add function declaration context if available
        if let Some(decl_span) = self.function_declaration_span {
            builder = builder.secondary_span(
                decl_span,
                format!(
                    "function `{}` requires {}",
                    self.function_name.as_deref().unwrap_or("this"),
                    self.required_capability
                ),
            );
        }

        // Add explanatory note
        builder = builder.add_note(format!("required capability: {}", self.required_capability));

        // Add help for installing provider
        let parts: Vec<Text> = self.required_capability.split("::").into();
        let context_name = parts.first().map(|s| s.as_str()).unwrap_or("Context");
        let impl_name = format!("Real{}", context_name);

        builder = builder.help(format!(
            "install provider before calling:\n\
            \n\
            provide {} = {}.new();",
            context_name, impl_name
        ));

        // Add alternative suggestion
        builder = builder.help(format!(
            "or pass the capability as a parameter:\n\
            \n\
            fn {}(ctx: &{}) -> Result<...> {{",
            self.function_name.as_deref().unwrap_or("function"),
            context_name
        ));

        // Add documentation reference
        builder = builder
            .add_note("contexts must be provided before functions requiring them can be called")
            .add_note("contexts must be installed via 'provide ContextName = ProviderImpl.new()' before any function requiring that context can be called; providers implement context protocols and are resolved at runtime (~5-30ns lookup)");

        builder.build()
    }
}

/// E0309: Partial Implementation Warning Builder
///
/// This warning occurs when a context implementation only provides some of the
/// sub-contexts, not all of them. This is allowed but should be documented.
///
/// # Example
///
/// ```verum
/// implement FileSystem for ReadOnlyFS {
///     // Only implements Read, not Write or Admin
/// }
/// ```
pub struct PartialImplementationWarning {
    /// The context being implemented
    context_name: Text,
    /// The implementation name
    implementation_name: Text,
    /// The span of the implementation
    span: Span,
    /// Sub-contexts that are implemented
    implemented_sub_contexts: List<Text>,
    /// Sub-contexts that are missing
    missing_sub_contexts: List<Text>,
}

impl PartialImplementationWarning {
    /// Create a new partial implementation warning
    pub fn new(
        context_name: impl Into<Text>,
        implementation_name: impl Into<Text>,
        span: Span,
    ) -> Self {
        Self {
            context_name: context_name.into(),
            implementation_name: implementation_name.into(),
            span,
            implemented_sub_contexts: List::new(),
            missing_sub_contexts: List::new(),
        }
    }

    /// Set the implemented sub-contexts
    pub fn with_implemented_sub_contexts(mut self, sub_contexts: List<Text>) -> Self {
        self.implemented_sub_contexts = sub_contexts;
        self
    }

    /// Set the missing sub-contexts
    pub fn with_missing_sub_contexts(mut self, sub_contexts: List<Text>) -> Self {
        self.missing_sub_contexts = sub_contexts;
        self
    }

    /// Build the diagnostic
    pub fn build(self) -> Diagnostic {
        let mut builder = DiagnosticBuilder::warning()
            .code(error_codes::E0309)
            .message(format!(
                "partial implementation of context `{}`",
                self.context_name
            ))
            .span_label(self.span.clone(), "incomplete sub-contexts");

        // Add note about implemented sub-contexts
        if !self.implemented_sub_contexts.is_empty() {
            builder = builder.add_note(format!(
                "implemented sub-contexts: {}",
                self.implemented_sub_contexts.join(", ")
            ));
        }

        // Add note about missing sub-contexts
        if !self.missing_sub_contexts.is_empty() {
            builder = builder.add_note(format!(
                "missing sub-contexts: {}",
                self.missing_sub_contexts.join(", ")
            ));
        }

        // Add help
        builder = builder
            .help("document the partial implementation")
            .help(format!(
                "add a comment explaining why {} doesn't implement all sub-contexts",
                self.implementation_name
            ));

        // Suggest implementing missing sub-contexts
        if !self.missing_sub_contexts.is_empty() && self.missing_sub_contexts.len() <= 3 {
            let mut suggestion = Text::from("implement missing sub-contexts:\n\n");
            for sub_ctx in &self.missing_sub_contexts {
                suggestion.push_str(&format!(
                    "fn {}(...) -> ... {{\n    // implement {} for {}\n}}\n\n",
                    sub_ctx.to_lowercase(),
                    sub_ctx,
                    self.implementation_name
                ));
            }
            builder = builder.help(suggestion.trim_end());
        }

        // Add note about partial implementations
        builder = builder
            .add_note("partial implementations are allowed but should be clearly documented")
            .add_note("code using this implementation should be aware of missing capabilities");

        builder.build()
    }
}

/// Compute Levenshtein distance for "did you mean" suggestions
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
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

/// Find similar names for "did you mean" suggestions
fn find_similar_names(target: &str, available: &[Text]) -> List<Text> {
    let mut candidates: Vec<(Text, usize)> = available
        .iter()
        .map(|name| {
            let distance = levenshtein_distance(&target.to_lowercase(), &name.to_lowercase());
            (name.clone(), distance)
        })
        .collect();

    // Sort by distance
    candidates.sort_by_key(|(_, dist)| *dist);

    // Return names within reasonable edit distance (≤3 for suggestions)
    candidates
        .into_iter()
        .filter(|(_, dist)| *dist <= 3)
        .take(3) // Limit to 3 suggestions
        .map(|(name, _)| name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::span::LineColSpan;

    fn dummy_span(line: usize, col: usize) -> Span {
        LineColSpan::new("test.vr", line, col, 10)
    }

    #[test]
    fn test_capability_violation_error() {
        let error = CapabilityViolationError::new("Database::Execute", dummy_span(6, 5))
            .with_declared_capabilities(vec!["Database::Query".into()].into())
            .with_declaration_span(dummy_span(4, 5))
            .with_function_span(dummy_span(3, 1))
            .with_function_name("attempt_delete");

        let diagnostic = error.build();
        assert_eq!(diagnostic.code(), Some(error_codes::E0306));
        assert_eq!(diagnostic.severity(), Severity::Error);
        assert!(diagnostic.message().contains("Database::Execute"));
        assert!(!diagnostic.helps().is_empty());
    }

    #[test]
    fn test_sub_context_not_found_error() {
        let error = SubContextNotFoundError::new("Database", "Read", dummy_span(2, 12))
            .with_available_sub_contexts(vec!["Query".into(), "Execute".into()].into());

        let diagnostic = error.build();
        assert_eq!(diagnostic.code(), Some(error_codes::E0307));
        assert_eq!(diagnostic.severity(), Severity::Error);
        assert!(diagnostic.message().contains("Read"));
        assert!(diagnostic.message().contains("Database"));
    }

    #[test]
    fn test_capability_not_provided_error() {
        let error = CapabilityNotProvidedError::new("FileSystem::Read", dummy_span(9, 16))
            .with_function_name("read_file");

        let diagnostic = error.build();
        assert_eq!(diagnostic.code(), Some(error_codes::E0308));
        assert_eq!(diagnostic.severity(), Severity::Error);
        assert!(diagnostic.message().contains("FileSystem::Read"));
        assert!(!diagnostic.helps().is_empty());
    }

    #[test]
    fn test_partial_implementation_warning() {
        let error = PartialImplementationWarning::new("FileSystem", "ReadOnlyFS", dummy_span(5, 1))
            .with_implemented_sub_contexts(vec!["Read".into()].into())
            .with_missing_sub_contexts(vec!["Write".into(), "Admin".into()].into());

        let diagnostic = error.build();
        assert_eq!(diagnostic.code(), Some(error_codes::E0309));
        assert_eq!(diagnostic.severity(), Severity::Warning);
        assert!(diagnostic.message().contains("partial implementation"));
        assert!(!diagnostic.helps().is_empty());
    }

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein_distance("Read", "Read"), 0);
        assert_eq!(levenshtein_distance("Read", "read"), 1);
        assert_eq!(levenshtein_distance("Read", "Rea"), 1);
        assert_eq!(levenshtein_distance("Read", "Write"), 5);
    }

    #[test]
    fn test_find_similar_names() {
        let available = vec!["Query".into(), "Execute".into(), "Admin".into()];
        let similar = find_similar_names("Execut", &available);
        assert!(similar.contains(&"Execute".into()));
    }

    #[test]
    fn test_capability_violation_with_no_declared_capabilities() {
        let error = CapabilityViolationError::new("Database::Execute", dummy_span(6, 5))
            .with_function_name("some_function");

        let diagnostic = error.build();
        assert!(diagnostic.message().contains("Database::Execute"));
        assert!(
            diagnostic
                .notes()
                .iter()
                .any(|n| n.message.contains("<none>"))
        );
    }

    #[test]
    fn test_sub_context_not_found_with_similar_suggestions() {
        let error = SubContextNotFoundError::new("Database", "Execut", dummy_span(2, 12))
            .with_available_sub_contexts(
                vec!["Query".into(), "Execute".into(), "Admin".into()].into(),
            );

        let diagnostic = error.build();
        // Should suggest "Execute" since it's similar to "Execut"
        assert!(
            diagnostic
                .helps()
                .iter()
                .any(|h| h.message.contains("Execute"))
        );
    }
}
