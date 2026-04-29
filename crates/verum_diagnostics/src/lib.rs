#![allow(unexpected_cfgs)]
//! Verum Diagnostics System
//!
//! A comprehensive diagnostics system providing world-class error messages for the Verum compiler.
//! This system emphasizes clarity, actionability, and especially excels at refinement type errors.
//!
//! # Separation of Concerns
//!
//! **verum_diagnostics** focuses exclusively on **compiler diagnostics** - beautiful error messages,
//! source spans, suggestions, and rendering for the Verum compiler itself.
//!
//! For **runtime error handling** (Result types, error contexts, recovery strategies), see the
//! **verum_error** crate which implements the 5-Level Error Defense Architecture.
//!
//! # Features
//!
//! - **Rich Error Messages**: Beautiful, colored output with source context
//! - **Refinement Type Specialization**: Shows actual values and constraints that failed
//! - **Actionable Suggestions**: Multiple fix options with code examples
//! - **SMT Trace Integration**: Shows verification paths and counterexamples
//! - **Multi-format Output**: Human-readable text and machine-readable JSON
//! - **Error Chains**: Full context propagation with related diagnostics
//! - **Lint System**: Configurable warnings with allow/warn/deny/forbid levels
//! - **LSP Integration**: Full IDE support with code actions and hover
//!
//! # Example
//!
//! ```rust
//! use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity, Span};
//!
//! let diagnostic = DiagnosticBuilder::error()
//!     .code("E0308")
//!     .message("refinement constraint not satisfied")
//!     .span(Span::new("main.vr", 3, 12, 13))
//!     .label("value `-5` fails constraint `i > 0`")
//!     .help("wrap in runtime check: `PositiveInt::try_from(x)?`")
//!     .help("or use compile-time proof: `@verify x > 0`")
//!     .build();
//! ```

#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(dead_code)]
// Suppress informational clippy lints
#![allow(clippy::result_large_err)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_match)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::if_same_then_else)]
#![allow(clippy::vec_init_then_push)]
#![allow(clippy::should_implement_trait)]

// Re-export semantic types from verum_common (v6.0-BALANCED compliance)
pub use verum_common::{List, Map, Text};

// Core modules
pub mod capability_attenuation_errors;
pub mod colors;
pub mod context;
pub mod context_error;
pub mod context_protocol;
pub mod diagnostic;
pub mod emitter;
pub mod explanations;
pub mod must_handle_errors;
/// Proof-failure repair-suggestion catalogue — typed
/// `ProofFailureKind` + `RepairEngine` trait + reference V0 catalogue
/// + composite engine for adapter chaining.
pub mod proof_repair;
pub mod recovery;
pub mod refinement_error;
pub mod renderer;
pub mod rich_renderer;
pub mod snippet_extractor;
pub mod suggestion;
pub mod try_operator_errors;

// Re-export main types
pub use capability_attenuation_errors::{
    CapabilityNotProvidedError, CapabilityViolationError, PartialImplementationWarning,
    SubContextNotFoundError,
};
pub use colors::{Color, ColorScheme, GlyphSet, Style};
pub use context::{DiagnosticContext, ErrorChain};
pub use context_error::{
    CallChain, CallFrame, ContextGroupUndefinedError, ContextNotDeclaredError,
    ContextNotProvidedError, ContextTypeMismatchError,
};
pub use context_protocol::{
    Backtrace, BacktraceMode, ContextFrame, ContextValue, DisplayError, ErrorContext,
    ErrorWithContext, ResultContext, SourceLocation as ContextSourceLocation, StackFrame,
};
pub use diagnostic::{
    Diagnostic, DiagnosticBuilder, DiagnosticCollector, Label, MessageFormatter, Severity,
    SourceLocation, Span, SpanLabel, SuggestedFix,
};
pub use emitter::{
    Emitter, EmitterConfig, LspCodeAction, LspDiagnostic, LspLocation, LspPosition, LspRange,
    LspRelatedInformation, LspTextEdit, LspWorkspaceEdit, OutputFormat,
};
pub use explanations::{
    ErrorExplanation, get_explanation, list_error_codes, render_explanation, search_errors,
};
pub use must_handle_errors::{
    BranchInfo, E0317, FlowContext, MustHandleTracker, ResultHandling, TrackedResult, ViolationKind,
};
pub use recovery::{
    ErrorKind, ErrorRecovery, NameContext, PartialCompilation, RecoveryAction, RecoverySeverity,
    RecoveryState, SyntaxErrorContext, SyntaxErrorKind, TypeConversion,
    find_closest_name, rust_keyword_suggestion, rust_macro_suggestion, rust_type_suggestion,
    RUST_KEYWORD_MAP, RUST_MACRO_MAP, RUST_TYPE_MAP,
};
pub use refinement_error::{
    Constraint, ConstraintViolation, CounterExample, RefinementError, RefinementErrorBuilder,
    SMTTrace, VerificationStep,
};
pub use renderer::{BatchRenderer, DiffRenderer, RenderConfig, Renderer};
pub use rich_renderer::{DiffRenderer as RichDiffRenderer, RichRenderConfig, RichRenderer};
pub use snippet_extractor::{Snippet, SnippetExtractor, SourceLine};
pub use suggestion::{
    Applicability, CodeSnippet, ErrorHandlingSuggestionTemplates, PerformanceSuggestionTemplates,
    Suggestion, SuggestionBuilder, SyntaxSuggestionTemplates, TypeSuggestionTemplates,
};
pub use try_operator_errors::{
    e0203_result_type_mismatch, e0204_multiple_conversion_paths, e0205_nested_try_operator,
    e0205_try_in_non_result_context,
};

/// Error code constants for quick reference
///
/// For full error code information with explanations, use the `error_codes` module:
/// ```rust,ignore
/// use verum_diagnostics::error_codes::lookup_error;
/// if let Some(info) = lookup_error("E0312") {
///     println!("{}", info.explanation);
/// }
/// ```
pub mod codes {
    // Try operator errors (E0203-E0205)
    /// Result type mismatch in '?' operator
    pub const E0203: &str = "E0203";
    /// Missing From implementation or multiple conversion paths
    pub const E0204: &str = "E0204";
    /// Cannot use '?' in non-Result context
    pub const E0205: &str = "E0205";

    // Context-related errors (E0301-E0305)
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

    // Capability attenuation errors (E0306-E0309)
    /// Capability violation - uses undeclared capability
    pub const E0306: &str = "E0306";
    /// Sub-context not found in context hierarchy
    pub const E0307: &str = "E0307";
    /// Required capability not provided in environment
    pub const E0308: &str = "E0308";
    /// Partial implementation of context (warning)
    pub const E0309: &str = "E0309";

    // Type and verification errors (E0310-E0320)
    /// Unsafe array access
    pub const E0310: &str = "E0310";
    /// Type mismatch
    pub const E0311: &str = "E0311";
    /// Refinement constraint not satisfied
    pub const E0312: &str = "E0312";
    /// Integer overflow
    pub const E0313: &str = "E0313";
    /// Division by zero
    pub const E0314: &str = "E0314";
    /// Null pointer dereference
    pub const E0315: &str = "E0315";
    /// Resource already consumed
    pub const E0316: &str = "E0316";
    /// Unused Result that must be used (@must_handle annotation)
    pub const E0317: &str = "E0317";
}

/// Warning codes for Verum diagnostics
pub mod warning_codes {
    /// Unused variable
    pub const W0101: &str = "W0101";
    /// Unused import
    pub const W0102: &str = "W0102";
    /// Dead code
    pub const W0103: &str = "W0103";
    /// Unnecessary refinement
    pub const W0104: &str = "W0104";
    /// False positive detected
    pub const W0105: &str = "W0105";
    /// Async placeholder future generated (incomplete async resolution)
    pub const W0042: &str = "W0042";
}
