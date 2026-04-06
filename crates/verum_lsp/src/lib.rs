#![allow(unexpected_cfgs)]
//! Verum Language Server Protocol (LSP) Library
//!
//! This crate provides a Language Server Protocol implementation for the Verum language.
//! It can be used as a library or run as a standalone server binary.
//!
//! # Features
//!
//! - **Diagnostics**: Real-time syntax and type error reporting
//! - **Completion**: Context-aware code completion
//! - **Hover**: Type information and documentation
//! - **Navigation**: Go to definition, find references
//! - **Refactoring**: Rename symbols, extract functions
//! - **Formatting**: Code formatting according to style guidelines
//!
//! # Example
//!
//! ```rust,no_run
//! use verum_lsp::backend::Backend;
//! use tower_lsp::{LspService, Server};
//!
//! #[tokio::main]
//! async fn main() {
//!     let stdin = tokio::io::stdin();
//!     let stdout = tokio::io::stdout();
//!
//!     let (service, socket) = LspService::new(|client| Backend::new(client));
//!     Server::new(stdin, stdout, socket).serve(service).await;
//! }
//! ```

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
#![allow(clippy::filter_map_identity)]
#![allow(clippy::if_same_then_else)]
#![allow(clippy::format_in_format_args)]
#![allow(clippy::option_as_ref_deref)]
#![allow(clippy::single_char_add_str)]
#![allow(clippy::redundant_pattern_matching)]
#![allow(clippy::manual_filter_map)]
#![allow(clippy::redundant_pattern)]
#![allow(clippy::unnecessary_filter_map)]
#![allow(clippy::redundant_guards)]

pub mod ast_format; // Shared AST formatting utilities
pub mod backend;
pub mod backend_incremental;
pub mod cbgr_hints;
pub mod code_actions;
pub mod completion;
pub mod debouncer;
pub mod diagnostics;
pub mod document;
pub mod exhaustiveness; // Exhaustiveness checking integration
pub mod document_cache;
pub mod formatting;
pub mod goto_definition;
pub mod hover;
pub mod incremental;
pub mod position_utils;
pub mod quick_fixes; // Comprehensive refinement violation quick fixes
pub mod references;
pub mod refinement_validation;
pub mod rename;
pub mod script; // Script/REPL parsing with incremental caching
pub mod selection_range;
pub mod semantic_tokens;
pub mod type_hierarchy;
pub mod inline_values;
pub mod workspace_index;

// Re-export key types for convenience
pub use backend::Backend;
pub use backend_incremental::IncrementalBackend;
pub use cbgr_hints::CbgrHintProvider;
pub use diagnostics::IncrementalDiagnosticsProvider;
pub use document::{DocumentState, DocumentStore};
pub use document_cache::{DocumentCache, ParsedDocument};
pub use formatting::TriviaPreservingFormatter;
pub use goto_definition::SyntaxTreeDefinitionProvider;
pub use incremental::IncrementalState;
pub use refinement_validation::{
    InferRefinementParams, InferRefinementResult, PromoteToCheckedParams, PromoteToCheckedResult,
    RefinementValidator, ValidateRefinementParams, ValidateRefinementResult,
};
pub use rename::SyntaxTreeRenameProvider;
pub use script::{
    detect_dependencies, explain_error, needs_continuation, suggest_autocompletion,
    suggest_completion, CachedLine, DependencyGraph, IncrementalScriptParser, IncrementalStats,
    ParseMode, RecoveryResult, ScriptContext, ScriptParseResult, ScriptParser, ScriptRecovery,
};
pub use semantic_tokens::SemanticTokenProvider;
pub use exhaustiveness::{
    ExhaustivenessDiagnostic, ExhaustivenessLspConfig, ExhaustivenessProvider, MatchCoverageInfo,
    create_add_patterns_fix, create_add_wildcard_fix, create_redundant_pattern_diagnostic,
    create_remove_redundant_fix,
};
